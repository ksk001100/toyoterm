#[cfg(not(windows))]
use std::collections::HashMap;
use std::collections::VecDeque;
use std::fs::{OpenOptions, remove_file};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(windows)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(windows))]
use std::sync::mpsc::RecvTimeoutError;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread;
#[cfg(not(windows))]
use std::time::{Duration, Instant};

use image::{ColorType, ImageEncoder, codecs::png::PngEncoder};
use toyoterm_api::PaneId;
use toyoterm_terminal::{
    NotificationIcon, NotificationReporting, NotificationSound, NotificationUrgency,
};

#[cfg(not(windows))]
const MAX_TRACKED_NOTIFICATIONS: usize = 128;
#[cfg(windows)]
const MAX_RETAINED_ICON_FILES: usize = 128;
#[cfg(windows)]
const MAX_NOTIFICATION_RESPONSE_LISTENERS: usize = 128;
static NEXT_ICON_FILE: AtomicU64 = AtomicU64::new(0);

#[allow(dead_code)]
pub(super) struct DesktopNotification {
    pub pane: PaneId,
    pub protocol_id: String,
    pub id: Option<u32>,
    pub title: Option<String>,
    pub body: String,
    pub urgency: NotificationUrgency,
    pub timeout_ms: Option<u32>,
    pub sound: NotificationSound,
    pub icon_name: Option<String>,
    pub icon: Option<NotificationIcon>,
    pub buttons: Vec<String>,
    pub reporting: NotificationReporting,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct NotificationFeedback {
    pub pane: PaneId,
    pub id: String,
    pub kind: NotificationFeedbackKind,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NotificationFeedbackKind {
    Activated,
    Button(usize),
    Closed,
}

#[allow(dead_code)]
type FeedbackHandler = Arc<dyn Fn(NotificationFeedback) + Send + Sync>;

enum NotificationRequest {
    Show(DesktopNotification),
    Close(u32),
}

#[cfg(not(windows))]
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum NotificationKey {
    Identified(u32),
    Anonymous(u64),
}

fn build_notification(request: &DesktopNotification) -> notify_rust::Notification {
    let title = request.title.as_deref().unwrap_or("toyoterm");
    let mut notification = notify_rust::Notification::new();
    notification
        .appname("toyoterm")
        .summary(title)
        .body(&request.body)
        .urgency(match request.urgency {
            NotificationUrgency::Low => notify_rust::Urgency::Low,
            NotificationUrgency::Normal => notify_rust::Urgency::Normal,
            NotificationUrgency::Critical => notify_rust::Urgency::Critical,
        });
    if let Some(timeout_ms) = request.timeout_ms {
        notification.timeout(if timeout_ms == 0 {
            notify_rust::Timeout::Never
        } else {
            notify_rust::Timeout::Milliseconds(timeout_ms)
        });
    }
    let sound_name = match request.sound {
        NotificationSound::System => None,
        NotificationSound::Silent => Some("silent"),
        NotificationSound::Error => Some("error"),
        NotificationSound::Warning => Some("warning"),
        NotificationSound::Info => Some("info"),
        NotificationSound::Question => Some("question"),
    };
    if let Some(sound_name) = sound_name {
        notification.sound_name(sound_name);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(icon_name) = request.icon_name.as_deref() {
        notification.icon(icon_name);
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    let _ = &request.icon_name;
    if let Some(id) = request.id {
        notification.id(id);
    }
    for (index, label) in request.buttons.iter().enumerate() {
        notification.action(&(index + 1).to_string(), label);
    }
    notification
}

struct PreparedNotification {
    notification: notify_rust::Notification,
    icon_path: Option<PathBuf>,
}

fn prepare_notification(request: &DesktopNotification) -> PreparedNotification {
    let mut notification = build_notification(request);
    let icon_path = request.icon.as_ref().and_then(|icon| {
        write_notification_icon(icon)
            .inspect_err(|error| {
                tracing::warn!(target: "toyoterm::notification", %error, "prepare OSC notification icon failed");
            })
            .ok()
    });
    if let Some(path) = icon_path.as_deref() {
        notification.image_path(path.to_string_lossy().as_ref());
    }
    PreparedNotification {
        notification,
        icon_path,
    }
}

fn write_notification_icon(icon: &NotificationIcon) -> Result<PathBuf, String> {
    let expected_len = usize::from(icon.width)
        .checked_mul(usize::from(icon.height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "notification icon dimensions overflow".to_owned())?;
    if icon.width == 0 || icon.height == 0 || icon.rgba.len() != expected_len {
        return Err("notification icon has invalid RGBA dimensions".to_owned());
    }

    let directory = std::env::temp_dir();
    for _ in 0..16 {
        let sequence = NEXT_ICON_FILE.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(
            "toyoterm-notification-{}-{sequence}.png",
            std::process::id()
        ));
        let file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("create {}: {error}", path.display())),
        };
        let result = PngEncoder::new(BufWriter::new(file)).write_image(
            icon.rgba.as_ref(),
            u32::from(icon.width),
            u32::from(icon.height),
            ColorType::Rgba8.into(),
        );
        if let Err(error) = result {
            remove_icon_file(&path);
            return Err(format!("encode {}: {error}", path.display()));
        }
        return Ok(path);
    }
    Err("allocate a unique notification icon path".to_owned())
}

fn remove_icon_file(path: &Path) {
    if let Err(error) = remove_file(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(target: "toyoterm::notification", %error, path = %path.display(), "remove OSC notification icon failed");
    }
}

#[cfg(windows)]
fn run_notification_worker(receiver: Receiver<NotificationRequest>, feedback: FeedbackHandler) {
    let mut retained_icons = VecDeque::new();
    let response_listeners = Arc::new(AtomicUsize::new(0));
    for request in receiver {
        match request {
            NotificationRequest::Show(request) => {
                let prepared = prepare_notification(&request);
                match prepared.notification.show() {
                    Ok(handle) => {
                        if (request.reporting.activation || request.reporting.close)
                            && response_listeners
                                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                                    (count < MAX_NOTIFICATION_RESPONSE_LISTENERS)
                                        .then_some(count + 1)
                                })
                                .is_ok()
                        {
                            let pane = request.pane;
                            let id = request.protocol_id;
                            let reporting = request.reporting;
                            let icon_path = prepared.icon_path;
                            let feedback = Arc::clone(&feedback);
                            let response_listeners = Arc::clone(&response_listeners);
                            thread::spawn(move || {
                                let _ = handle.wait_for_response(
                                    move |response: &notify_rust::NotificationResponse| {
                                        for event in
                                            notification_feedbacks(pane, &id, reporting, response)
                                        {
                                            feedback(event);
                                        }
                                    },
                                );
                                if let Some(path) = icon_path {
                                    remove_icon_file(&path);
                                }
                                response_listeners.fetch_sub(1, Ordering::Relaxed);
                            });
                        } else if let Some(path) = prepared.icon_path {
                            retained_icons.push_back(path);
                            if retained_icons.len() > MAX_RETAINED_ICON_FILES
                                && let Some(path) = retained_icons.pop_front()
                            {
                                remove_icon_file(&path);
                            }
                        }
                    }
                    Err(error) => {
                        if let Some(path) = prepared.icon_path {
                            remove_icon_file(&path);
                        }
                        tracing::warn!(target: "toyoterm::notification", %error, "show OSC notification failed");
                    }
                }
            }
            NotificationRequest::Close(id) => {
                let _ = id;
            }
        }
    }
    for path in retained_icons {
        remove_icon_file(&path);
    }
}

#[cfg(not(windows))]
fn run_notification_worker(receiver: Receiver<NotificationRequest>, _feedback: FeedbackHandler) {
    struct ActiveNotification {
        handle: notify_rust::NotificationHandle,
        icon_path: Option<PathBuf>,
    }

    impl ActiveNotification {
        fn close(self) {
            self.handle.close();
            if let Some(path) = self.icon_path {
                remove_icon_file(&path);
            }
        }
    }

    let mut active: HashMap<NotificationKey, ActiveNotification> = HashMap::new();
    let mut active_order: VecDeque<NotificationKey> = VecDeque::new();
    let mut expirations: HashMap<NotificationKey, Instant> = HashMap::new();
    let mut next_anonymous = 0_u64;

    loop {
        let now = Instant::now();
        let expired = expirations
            .iter()
            .filter_map(|(key, deadline)| (*deadline <= now).then_some(*key))
            .collect::<Vec<_>>();
        for key in expired {
            expirations.remove(&key);
            active_order.retain(|tracked| *tracked != key);
            if let Some(notification) = active.remove(&key) {
                notification.close();
            }
        }

        let wait = expirations
            .values()
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
            .min()
            .unwrap_or(Duration::from_secs(60 * 60));
        let request = match receiver.recv_timeout(wait) {
            Ok(request) => request,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        match request {
            NotificationRequest::Close(id) => {
                let key = NotificationKey::Identified(id);
                expirations.remove(&key);
                active_order.retain(|tracked| *tracked != key);
                if let Some(notification) = active.remove(&key) {
                    notification.close();
                }
            }
            NotificationRequest::Show(request) => {
                let id = request.id;
                let timeout_ms = request.timeout_ms;
                // Some notification services ignore replacement IDs. Close an
                // existing identified notification first so an update cannot
                // leave two visible notifications behind on those backends.
                if let Some(key) = id.map(NotificationKey::Identified) {
                    expirations.remove(&key);
                    active_order.retain(|tracked| *tracked != key);
                    if let Some(notification) = active.remove(&key) {
                        notification.close();
                    }
                }
                let prepared = prepare_notification(&request);
                match prepared.notification.show() {
                    Ok(handle) => {
                        let key = id.map_or_else(
                            || {
                                let key = NotificationKey::Anonymous(next_anonymous);
                                next_anonymous = next_anonymous.wrapping_add(1);
                                key
                            },
                            NotificationKey::Identified,
                        );
                        if active.len() >= MAX_TRACKED_NOTIFICATIONS
                            && !active.contains_key(&key)
                            && let Some(oldest) = active_order.pop_front()
                        {
                            expirations.remove(&oldest);
                            if let Some(notification) = active.remove(&oldest) {
                                notification.close();
                            }
                        }
                        active.insert(
                            key,
                            ActiveNotification {
                                handle,
                                icon_path: prepared.icon_path,
                            },
                        );
                        active_order.push_back(key);
                        if let Some(timeout_ms) = timeout_ms.filter(|timeout| *timeout > 0)
                            && let Some(deadline) = Instant::now()
                                .checked_add(Duration::from_millis(u64::from(timeout_ms)))
                        {
                            expirations.insert(key, deadline);
                        }
                    }
                    Err(error) => {
                        if let Some(path) = prepared.icon_path {
                            remove_icon_file(&path);
                        }
                        tracing::warn!(target: "toyoterm::notification", %error, "show OSC notification failed");
                    }
                }
            }
        }
    }

    for (_, notification) in active {
        notification.close();
    }
}

#[cfg(windows)]
fn notification_feedbacks(
    pane: PaneId,
    id: &str,
    reporting: NotificationReporting,
    response: &notify_rust::NotificationResponse,
) -> Vec<NotificationFeedback> {
    let mut events = Vec::with_capacity(2);
    match response {
        notify_rust::NotificationResponse::Default if reporting.activation => {
            events.push(NotificationFeedback {
                pane,
                id: id.to_owned(),
                kind: NotificationFeedbackKind::Activated,
            });
        }
        notify_rust::NotificationResponse::Action(action) if reporting.activation => {
            if let Ok(button) = action.parse::<usize>() {
                events.push(NotificationFeedback {
                    pane,
                    id: id.to_owned(),
                    kind: NotificationFeedbackKind::Button(button),
                });
            }
        }
        notify_rust::NotificationResponse::Reply(_) if reporting.activation => {
            events.push(NotificationFeedback {
                pane,
                id: id.to_owned(),
                kind: NotificationFeedbackKind::Activated,
            });
        }
        _ => {}
    }
    if reporting.close {
        events.push(NotificationFeedback {
            pane,
            id: id.to_owned(),
            kind: NotificationFeedbackKind::Closed,
        });
    }
    events
}

pub(super) struct NotificationSender(SyncSender<NotificationRequest>);

impl NotificationSender {
    pub(super) fn start(
        feedback: impl Fn(NotificationFeedback) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let (sender, receiver) = sync_channel::<NotificationRequest>(32);
        let feedback = Arc::new(feedback);
        thread::Builder::new()
            .name("toyoterm-notifications".into())
            .spawn(move || run_notification_worker(receiver, feedback))
            .map_err(|error| format!("start notification worker: {error}"))?;
        Ok(Self(sender))
    }

    pub(super) fn send(&self, request: DesktopNotification) -> Result<(), String> {
        self.send_request(NotificationRequest::Show(request))
    }

    pub(super) fn close(&self, id: u32) -> Result<(), String> {
        self.send_request(NotificationRequest::Close(id))
    }

    fn send_request(&self, request: NotificationRequest) -> Result<(), String> {
        self.0.try_send(request).map_err(|error| match error {
            TrySendError::Full(_) => "notification queue is full".to_owned(),
            TrySendError::Disconnected(_) => "notification worker stopped".to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn writes_and_removes_rgba_notification_icon() {
        let icon = NotificationIcon {
            width: 2,
            height: 1,
            rgba: Arc::from([255, 0, 0, 255, 0, 255, 0, 128]),
        };

        let path = write_notification_icon(&icon).unwrap();
        let decoded = image::open(&path).unwrap().into_rgba8();
        assert_eq!(decoded.dimensions(), (2, 1));
        assert_eq!(decoded.into_raw(), icon.rgba.as_ref());

        remove_icon_file(&path);
        assert!(!path.exists());
    }

    #[test]
    fn rejects_invalid_notification_icon_dimensions() {
        let icon = NotificationIcon {
            width: 2,
            height: 1,
            rgba: Arc::from([0_u8; 4]),
        };

        assert!(write_notification_icon(&icon).is_err());
    }

    #[test]
    fn maps_notification_buttons_to_platform_actions() {
        let request = DesktopNotification {
            pane: PaneId(1),
            protocol_id: String::new(),
            id: None,
            title: Some("Build".into()),
            body: "Finished".into(),
            urgency: NotificationUrgency::Normal,
            timeout_ms: None,
            sound: NotificationSound::System,
            icon_name: None,
            icon: None,
            buttons: vec!["Open".into(), "Dismiss".into()],
            reporting: NotificationReporting::default(),
        };

        let notification = build_notification(&request);
        assert_eq!(notification.actions, ["1", "Open", "2", "Dismiss"]);
    }

    #[cfg(windows)]
    #[test]
    fn maps_windows_notification_responses_to_requested_feedback() {
        let reporting = NotificationReporting {
            activation: true,
            close: true,
        };
        assert_eq!(
            notification_feedbacks(
                PaneId(3),
                "job",
                reporting,
                &notify_rust::NotificationResponse::Action("2".into()),
            ),
            vec![
                NotificationFeedback {
                    pane: PaneId(3),
                    id: "job".into(),
                    kind: NotificationFeedbackKind::Button(2),
                },
                NotificationFeedback {
                    pane: PaneId(3),
                    id: "job".into(),
                    kind: NotificationFeedbackKind::Closed,
                },
            ]
        );
        assert_eq!(
            notification_feedbacks(
                PaneId(3),
                "job",
                reporting,
                &notify_rust::NotificationResponse::Default,
            )[0]
            .kind,
            NotificationFeedbackKind::Activated
        );
    }
}
