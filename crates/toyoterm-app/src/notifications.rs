#[cfg(not(windows))]
use std::collections::{HashMap, VecDeque};
#[cfg(not(windows))]
use std::sync::mpsc::RecvTimeoutError;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread;
#[cfg(not(windows))]
use std::time::{Duration, Instant};

use toyoterm_terminal::{NotificationSound, NotificationUrgency};

#[cfg(not(windows))]
const MAX_TRACKED_NOTIFICATIONS: usize = 128;

pub(super) struct DesktopNotification {
    pub id: Option<u32>,
    pub title: Option<String>,
    pub body: String,
    pub urgency: NotificationUrgency,
    pub timeout_ms: Option<u32>,
    pub sound: NotificationSound,
    pub icon_name: Option<String>,
}

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
    notification
}

#[cfg(windows)]
fn run_notification_worker(receiver: Receiver<NotificationRequest>) {
    for request in receiver {
        match request {
            NotificationRequest::Show(request) => {
                if let Err(error) = build_notification(&request).show() {
                    tracing::warn!(target: "toyoterm::notification", %error, "show OSC notification failed");
                }
            }
            NotificationRequest::Close(id) => {
                let _ = id;
            }
        }
    }
}

#[cfg(not(windows))]
fn run_notification_worker(receiver: Receiver<NotificationRequest>) {
    let mut active: HashMap<NotificationKey, notify_rust::NotificationHandle> = HashMap::new();
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
            if let Some(handle) = active.remove(&key) {
                handle.close();
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
                if let Some(handle) = active.remove(&key) {
                    handle.close();
                }
            }
            NotificationRequest::Show(request) => {
                let id = request.id;
                let timeout_ms = request.timeout_ms;
                match build_notification(&request).show() {
                    Ok(handle) => {
                        let key = id.map_or_else(
                            || {
                                let key = NotificationKey::Anonymous(next_anonymous);
                                next_anonymous = next_anonymous.wrapping_add(1);
                                key
                            },
                            NotificationKey::Identified,
                        );
                        active_order.retain(|tracked| *tracked != key);
                        expirations.remove(&key);
                        if active.len() >= MAX_TRACKED_NOTIFICATIONS
                            && !active.contains_key(&key)
                            && let Some(oldest) = active_order.pop_front()
                        {
                            expirations.remove(&oldest);
                            if let Some(handle) = active.remove(&oldest) {
                                handle.close();
                            }
                        }
                        active.insert(key, handle);
                        active_order.push_back(key);
                        if let Some(timeout_ms) = timeout_ms.filter(|timeout| *timeout > 0)
                            && let Some(deadline) = Instant::now()
                                .checked_add(Duration::from_millis(u64::from(timeout_ms)))
                        {
                            expirations.insert(key, deadline);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(target: "toyoterm::notification", %error, "show OSC notification failed");
                    }
                }
            }
        }
    }

    for (_, handle) in active {
        handle.close();
    }
}

pub(super) struct NotificationSender(SyncSender<NotificationRequest>);

impl NotificationSender {
    pub(super) fn start() -> Result<Self, String> {
        let (sender, receiver) = sync_channel::<NotificationRequest>(32);
        thread::Builder::new()
            .name("toyoterm-notifications".into())
            .spawn(move || run_notification_worker(receiver))
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
