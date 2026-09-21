use std::collections::HashMap;
use std::collections::VecDeque;
use std::fs::{OpenOptions, remove_file};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
use std::sync::atomic::AtomicBool;
#[cfg(any(windows, unix))]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread;
use std::time::{Duration, Instant};

use image::{ColorType, ImageEncoder, codecs::png::PngEncoder};
use toyoterm_api::PaneId;
use toyoterm_terminal::{
    NotificationIcon, NotificationReporting, NotificationSound, NotificationUrgency,
};

const MAX_TRACKED_NOTIFICATIONS: usize = 128;
#[cfg(any(windows, unix))]
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

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum NotificationKey {
    Identified(u32),
    Anonymous(u64),
}

#[cfg(all(unix, not(target_os = "macos")))]
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
    let sound_name = notification_sound_name(request.sound);
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

#[cfg(all(unix, not(target_os = "macos")))]
struct PreparedNotification {
    notification: notify_rust::Notification,
    icon_path: Option<PathBuf>,
}

#[cfg(all(unix, not(target_os = "macos")))]
fn prepare_notification(request: &DesktopNotification) -> PreparedNotification {
    let mut notification = build_notification(request);
    let named_icon = request
        .icon_name
        .as_deref()
        .and_then(standard_notification_icon);
    #[cfg(all(unix, not(target_os = "macos")))]
    let icon = request.icon.as_ref().or(named_icon.as_ref());
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    let icon = named_icon.as_ref().or(request.icon.as_ref());
    let icon_path = icon.and_then(|icon| {
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

#[cfg(windows)]
fn notification_sound_name(sound: NotificationSound) -> Option<&'static str> {
    match sound {
        NotificationSound::System => Some("Default"),
        NotificationSound::Silent => None,
        NotificationSound::Error => Some("Alarm"),
        NotificationSound::Warning => Some("Reminder"),
        NotificationSound::Info => Some("IM"),
        NotificationSound::Question => Some("SMS"),
    }
}

#[cfg(target_os = "macos")]
fn notification_sound_name(sound: NotificationSound) -> Option<&'static str> {
    match sound {
        NotificationSound::System | NotificationSound::Error => Some("Basso"),
        NotificationSound::Silent => None,
        NotificationSound::Warning => Some("Sosumi"),
        NotificationSound::Info => Some("Glass"),
        NotificationSound::Question => Some("Ping"),
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn notification_sound_name(sound: NotificationSound) -> Option<&'static str> {
    match sound {
        NotificationSound::System => None,
        NotificationSound::Silent => Some("silent"),
        NotificationSound::Error => Some("error"),
        NotificationSound::Warning => Some("warning"),
        NotificationSound::Info => Some("info"),
        NotificationSound::Question => Some("question"),
    }
}

#[cfg(not(any(windows, unix)))]
fn notification_sound_name(sound: NotificationSound) -> Option<&'static str> {
    match sound {
        NotificationSound::System => None,
        NotificationSound::Silent => Some("silent"),
        NotificationSound::Error => Some("error"),
        NotificationSound::Warning => Some("warning"),
        NotificationSound::Info => Some("info"),
        NotificationSound::Question => Some("question"),
    }
}

fn standard_notification_icon(name: &str) -> Option<NotificationIcon> {
    let mut rgba = vec![0_u8; STANDARD_ICON_SIDE * STANDARD_ICON_SIDE * 4];
    let white = [255, 255, 255, 255];

    match name {
        "dialog-error" => {
            icon_circle(&mut rgba, [204, 45, 55, 255]);
            icon_line(&mut rgba, (16, 16), (32, 32), 2, white);
            icon_line(&mut rgba, (32, 16), (16, 32), 2, white);
        }
        "dialog-warning" => {
            for y in 5..44 {
                let half_width = (y - 5) / 2;
                for x in (24 - half_width)..=(24 + half_width) {
                    icon_pixel(&mut rgba, x, y, [230, 158, 25, 255]);
                }
            }
            icon_line(&mut rgba, (24, 16), (24, 30), 2, white);
            icon_line(&mut rgba, (24, 36), (24, 36), 2, white);
        }
        "dialog-information" => {
            icon_circle(&mut rgba, [45, 117, 204, 255]);
            icon_line(&mut rgba, (24, 21), (24, 35), 2, white);
            icon_line(&mut rgba, (24, 13), (24, 14), 2, white);
        }
        "dialog-question" | "help-browser" => {
            icon_circle(
                &mut rgba,
                if name == "help-browser" {
                    [45, 154, 92, 255]
                } else {
                    [126, 76, 180, 255]
                },
            );
            icon_line(&mut rgba, (17, 17), (21, 13), 2, white);
            icon_line(&mut rgba, (21, 13), (28, 13), 2, white);
            icon_line(&mut rgba, (28, 13), (32, 17), 2, white);
            icon_line(&mut rgba, (32, 17), (32, 21), 2, white);
            icon_line(&mut rgba, (32, 21), (24, 27), 2, white);
            icon_line(&mut rgba, (24, 27), (24, 30), 2, white);
            icon_line(&mut rgba, (24, 36), (24, 36), 2, white);
        }
        "system-file-manager" => {
            icon_fill_rect(&mut rgba, 4, 13, 23, 20, [232, 175, 56, 255]);
            icon_fill_rect(&mut rgba, 4, 18, 44, 40, [244, 194, 72, 255]);
            icon_line(&mut rgba, (9, 25), (39, 25), 1, [255, 225, 145, 255]);
        }
        "utilities-system-monitor" => {
            icon_fill_rect(&mut rgba, 4, 7, 44, 38, [47, 55, 65, 255]);
            icon_line(&mut rgba, (9, 29), (16, 23), 1, [70, 214, 120, 255]);
            icon_line(&mut rgba, (16, 23), (21, 27), 1, [70, 214, 120, 255]);
            icon_line(&mut rgba, (21, 27), (29, 14), 1, [70, 214, 120, 255]);
            icon_line(&mut rgba, (29, 14), (39, 20), 1, [70, 214, 120, 255]);
            icon_fill_rect(&mut rgba, 18, 38, 30, 43, [103, 113, 126, 255]);
        }
        "accessories-text-editor" => {
            icon_fill_rect(&mut rgba, 8, 4, 40, 44, [245, 247, 250, 255]);
            for y in [14, 21, 28, 35] {
                icon_line(&mut rgba, (14, y), (34, y), 1, [62, 116, 184, 255]);
            }
        }
        _ => return None,
    }

    Some(NotificationIcon {
        width: STANDARD_ICON_SIDE as u16,
        height: STANDARD_ICON_SIDE as u16,
        rgba: Arc::from(rgba),
    })
}

const STANDARD_ICON_SIDE: usize = 48;

fn icon_pixel(rgba: &mut [u8], x: i32, y: i32, color: [u8; 4]) {
    if (0..STANDARD_ICON_SIDE as i32).contains(&x) && (0..STANDARD_ICON_SIDE as i32).contains(&y) {
        let offset = (y as usize * STANDARD_ICON_SIDE + x as usize) * 4;
        rgba[offset..offset + 4].copy_from_slice(&color);
    }
}

fn icon_line(rgba: &mut [u8], from: (i32, i32), to: (i32, i32), width: i32, color: [u8; 4]) {
    let (mut x, mut y) = from;
    let dx = (to.0 - x).abs();
    let sx = if x < to.0 { 1 } else { -1 };
    let dy = -(to.1 - y).abs();
    let sy = if y < to.1 { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        for offset_y in -width..=width {
            for offset_x in -width..=width {
                icon_pixel(rgba, x + offset_x, y + offset_y, color);
            }
        }
        if (x, y) == to {
            break;
        }
        let doubled = error * 2;
        if doubled >= dy {
            error += dy;
            x += sx;
        }
        if doubled <= dx {
            error += dx;
            y += sy;
        }
    }
}

fn icon_fill_rect(
    rgba: &mut [u8],
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
    color: [u8; 4],
) {
    for y in top..bottom {
        for x in left..right {
            let offset = (y * STANDARD_ICON_SIDE + x) * 4;
            rgba[offset..offset + 4].copy_from_slice(&color);
        }
    }
}

fn icon_circle(rgba: &mut [u8], color: [u8; 4]) {
    for y in 3..45 {
        for x in 3..45 {
            let dx = x as i32 - 24;
            let dy = y as i32 - 24;
            if dx * dx + dy * dy <= 21 * 21 {
                let offset = (y * STANDARD_ICON_SIDE + x) * 4;
                rgba[offset..offset + 4].copy_from_slice(&color);
            }
        }
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
struct PreparedWindowsNotification {
    notifier: windows::UI::Notifications::ToastNotifier,
    toast: windows::UI::Notifications::ToastNotification,
    icon_path: Option<PathBuf>,
}

#[cfg(windows)]
fn prepare_windows_notification(
    request: &DesktopNotification,
) -> windows::core::Result<PreparedWindowsNotification> {
    use windows::{
        Data::Xml::Dom::XmlDocument,
        UI::Notifications::{ToastNotification, ToastNotificationManager},
        core::HSTRING,
    };

    const POWERSHELL_APP_ID: &str =
        "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";

    let named_icon = request
        .icon_name
        .as_deref()
        .and_then(standard_notification_icon);
    let icon = named_icon.as_ref().or(request.icon.as_ref());
    let icon_path = icon.and_then(|icon| {
        write_notification_icon(icon)
            .inspect_err(|error| {
                tracing::warn!(target: "toyoterm::notification", %error, "prepare OSC notification icon failed");
            })
            .ok()
    });
    let xml = windows_notification_xml(request, icon_path.as_deref());
    let document = XmlDocument::new()?;
    if let Err(error) = document.LoadXml(&HSTRING::from(xml)) {
        if let Some(path) = icon_path.as_deref() {
            remove_icon_file(path);
        }
        return Err(error);
    }
    let toast = ToastNotification::CreateToastNotification(&document)?;
    let notifier =
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(POWERSHELL_APP_ID))?;
    Ok(PreparedWindowsNotification {
        notifier,
        toast,
        icon_path,
    })
}

#[cfg(windows)]
fn windows_notification_xml(request: &DesktopNotification, icon_path: Option<&Path>) -> String {
    use std::fmt::Write;

    let duration = if request.timeout_ms.is_some_and(|timeout| timeout > 7_000) {
        "long"
    } else {
        "short"
    };
    let scenario = if request.urgency == NotificationUrgency::Critical {
        " scenario=\"reminder\""
    } else {
        ""
    };
    let mut xml = format!(
        "<toast duration=\"{duration}\"{scenario}><visual><binding template=\"ToastGeneric\">"
    );
    let title = request.title.as_deref().unwrap_or("toyoterm");
    let _ = write!(
        xml,
        "<text>{}</text><text>{}</text>",
        xml_escape(title),
        xml_escape(&request.body)
    );
    if let Some(path) = icon_path {
        let source = format!("file:///{}", path.to_string_lossy().replace('\\', "/"));
        let _ = write!(
            xml,
            "<image placement=\"appLogoOverride\" src=\"{}\"/>",
            xml_escape(&source)
        );
    }
    xml.push_str("</binding></visual>");
    if !request.buttons.is_empty() {
        xml.push_str("<actions>");
        for (index, label) in request.buttons.iter().enumerate() {
            let _ = write!(
                xml,
                "<action content=\"{}\" arguments=\"{}\"/>",
                xml_escape(label),
                index + 1
            );
        }
        xml.push_str("</actions>");
    }
    match request.sound {
        NotificationSound::Silent => xml.push_str("<audio silent=\"true\"/>"),
        sound => {
            if let Some(name) = notification_sound_name(sound) {
                let _ = write!(xml, "<audio src=\"ms-winsoundevent:Notification.{name}\"/>");
            }
        }
    }
    xml.push_str("</toast>");
    xml
}

#[cfg(windows)]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(windows)]
fn start_windows_notification_listener(
    toast: &windows::UI::Notifications::ToastNotification,
    request: &DesktopNotification,
    icon_path: Option<PathBuf>,
    response_suppressed: Arc<AtomicBool>,
    feedback: FeedbackHandler,
    response_listeners: Arc<AtomicUsize>,
) -> Option<SyncSender<Option<NotificationResponseKind>>> {
    use windows::{
        Foundation::TypedEventHandler,
        UI::Notifications::{ToastActivatedEventArgs, ToastDismissedEventArgs, ToastNotification},
        core::{IInspectable, Interface},
    };

    if !request.reporting.activation && !request.reporting.close {
        return None;
    }
    if response_listeners
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
            (count < MAX_NOTIFICATION_RESPONSE_LISTENERS).then_some(count + 1)
        })
        .is_err()
    {
        tracing::warn!(
            target: "toyoterm::notification",
            limit = MAX_NOTIFICATION_RESPONSE_LISTENERS,
            "OSC notification response listener limit reached"
        );
        return None;
    }

    let (sender, receiver) = sync_channel(1);
    let activated_sender = sender.clone();
    let activated = TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, args| {
        let response = args
            .as_ref()
            .and_then(|args| args.cast::<ToastActivatedEventArgs>().ok())
            .and_then(|args| args.Arguments().ok())
            .map(|arguments| arguments.to_string())
            .filter(|arguments| !arguments.is_empty())
            .map_or(
                NotificationResponseKind::Default,
                NotificationResponseKind::Action,
            );
        let _ = activated_sender.try_send(Some(response));
        Ok(())
    });
    let dismissed_sender = sender.clone();
    let dismissed =
        TypedEventHandler::<ToastNotification, ToastDismissedEventArgs>::new(move |_, _| {
            let _ = dismissed_sender.try_send(Some(NotificationResponseKind::Closed));
            Ok(())
        });
    if let Err(error) = toast.Activated(&activated) {
        response_listeners.fetch_sub(1, Ordering::Relaxed);
        tracing::warn!(target: "toyoterm::notification", %error, "register OSC notification activation listener failed");
        return None;
    }
    if let Err(error) = toast.Dismissed(&dismissed) {
        response_listeners.fetch_sub(1, Ordering::Relaxed);
        tracing::warn!(target: "toyoterm::notification", %error, "register OSC notification close listener failed");
        return None;
    }

    let pane = request.pane;
    let id = request.protocol_id.clone();
    let reporting = request.reporting;
    let completion_counter = Arc::clone(&response_listeners);
    let spawn = thread::Builder::new()
        .name("toyoterm-notification-response".into())
        .spawn(move || {
            if let Ok(Some(response)) = receiver.recv()
                && !response_suppressed.load(Ordering::Acquire)
            {
                for event in notification_feedbacks(pane, &id, reporting, response) {
                    feedback(event);
                }
            }
            if let Some(path) = icon_path {
                remove_icon_file(&path);
            }
            completion_counter.fetch_sub(1, Ordering::Relaxed);
        });
    if let Err(error) = spawn {
        response_listeners.fetch_sub(1, Ordering::Relaxed);
        tracing::warn!(target: "toyoterm::notification", %error, "start OSC notification response listener failed");
        return None;
    }
    Some(sender)
}

#[cfg(windows)]
fn run_notification_worker(receiver: Receiver<NotificationRequest>, feedback: FeedbackHandler) {
    use windows::UI::Notifications::{ToastNotification, ToastNotifier};

    struct ActiveNotification {
        notifier: ToastNotifier,
        toast: ToastNotification,
        icon_path: Option<PathBuf>,
        response_suppressed: Arc<AtomicBool>,
        response_cancel: Option<SyncSender<Option<NotificationResponseKind>>>,
    }

    impl ActiveNotification {
        fn close(self, suppress_response: bool) {
            if suppress_response {
                self.response_suppressed.store(true, Ordering::Release);
            }
            if let Some(cancel) = self.response_cancel {
                let response = (!suppress_response).then_some(NotificationResponseKind::Closed);
                let _ = cancel.try_send(response);
            }
            if let Err(error) = self.notifier.Hide(&self.toast) {
                tracing::warn!(target: "toyoterm::notification", %error, "close OSC notification failed");
            }
            if let Some(path) = self.icon_path {
                remove_icon_file(&path);
            }
        }
    }

    let mut active: HashMap<NotificationKey, ActiveNotification> = HashMap::new();
    let mut active_order: VecDeque<NotificationKey> = VecDeque::new();
    let mut expirations: HashMap<NotificationKey, Instant> = HashMap::new();
    let mut next_anonymous = 0_u64;
    let response_listeners = Arc::new(AtomicUsize::new(0));

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
                notification.close(false);
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
                    notification.close(false);
                }
            }
            NotificationRequest::Show(request) => {
                let id = request.id;
                let timeout_ms = request.timeout_ms;
                if let Some(key) = id.map(NotificationKey::Identified) {
                    expirations.remove(&key);
                    active_order.retain(|tracked| *tracked != key);
                    if let Some(notification) = active.remove(&key) {
                        notification.close(true);
                    }
                }
                match prepare_windows_notification(&request) {
                    Ok(prepared) => {
                        let response_suppressed = Arc::new(AtomicBool::new(false));
                        let response_cancel = start_windows_notification_listener(
                            &prepared.toast,
                            &request,
                            prepared.icon_path.clone(),
                            Arc::clone(&response_suppressed),
                            Arc::clone(&feedback),
                            Arc::clone(&response_listeners),
                        );
                        if let Err(error) = prepared.notifier.Show(&prepared.toast) {
                            response_suppressed.store(true, Ordering::Release);
                            if let Some(cancel) = response_cancel {
                                let _ = cancel.try_send(None);
                            }
                            if let Some(path) = prepared.icon_path {
                                remove_icon_file(&path);
                            }
                            tracing::warn!(target: "toyoterm::notification", %error, "show OSC notification failed");
                            continue;
                        }
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
                                notification.close(false);
                            }
                        }
                        active.insert(
                            key,
                            ActiveNotification {
                                notifier: prepared.notifier,
                                toast: prepared.toast,
                                icon_path: prepared.icon_path,
                                response_suppressed,
                                response_cancel,
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
                        tracing::warn!(target: "toyoterm::notification", %error, "show OSC notification failed");
                    }
                }
            }
        }
    }

    for (_, notification) in active {
        notification.close(true);
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn run_notification_worker(receiver: Receiver<NotificationRequest>, feedback: FeedbackHandler) {
    struct ActiveNotification {
        handle: notify_rust::NotificationHandle,
        icon_path: Option<PathBuf>,
        response_suppressed: Arc<AtomicBool>,
    }

    impl ActiveNotification {
        fn close(self, suppress_response: bool) {
            if suppress_response {
                self.response_suppressed.store(true, Ordering::Release);
            }
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
    let response_listeners = Arc::new(AtomicUsize::new(0));

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
                notification.close(false);
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
                    notification.close(false);
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
                        notification.close(true);
                    }
                }
                let prepared = prepare_notification(&request);
                match prepared.notification.show() {
                    Ok(handle) => {
                        let response_suppressed = Arc::new(AtomicBool::new(false));
                        start_linux_notification_listener(
                            handle.id(),
                            &request,
                            Arc::clone(&response_suppressed),
                            Arc::clone(&feedback),
                            Arc::clone(&response_listeners),
                        );
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
                                notification.close(false);
                            }
                        }
                        active.insert(
                            key,
                            ActiveNotification {
                                handle,
                                icon_path: prepared.icon_path,
                                response_suppressed,
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
        notification.close(true);
    }
}

#[cfg(target_os = "macos")]
fn run_notification_worker(receiver: Receiver<NotificationRequest>, feedback: FeedbackHandler) {
    use futures_channel::oneshot;

    struct ActiveNotification {
        native_id: String,
        cancellation: Option<oneshot::Sender<bool>>,
        icon_path: Option<PathBuf>,
    }

    impl ActiveNotification {
        fn close(self, suppress_response: bool) {
            let listener_closed = self
                .cancellation
                .is_some_and(|sender| sender.send(suppress_response).is_ok());
            if !listener_closed {
                mac_usernotifications::blocking::close_delivered(&self.native_id);
            }
            if let Some(path) = self.icon_path {
                remove_icon_file(&path);
            }
        }
    }

    let mut active: HashMap<NotificationKey, ActiveNotification> = HashMap::new();
    let mut active_order: VecDeque<NotificationKey> = VecDeque::new();
    let mut expirations: HashMap<NotificationKey, Instant> = HashMap::new();
    let mut next_anonymous = 0_u64;
    let response_listeners = Arc::new(AtomicUsize::new(0));

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
                notification.close(false);
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
                    notification.close(false);
                }
            }
            NotificationRequest::Show(request) => {
                let id = request.id;
                let timeout_ms = request.timeout_ms;
                if let Some(key) = id.map(NotificationKey::Identified) {
                    expirations.remove(&key);
                    active_order.retain(|tracked| *tracked != key);
                    if let Some(notification) = active.remove(&key) {
                        notification.close(true);
                    }
                }
                let prepared = prepare_macos_notification(&request);
                match prepared.notification.send_blocking() {
                    Ok(handle) => {
                        let native_id = handle.notification_id().to_owned();
                        let cancellation = start_macos_notification_listener(
                            handle,
                            &request,
                            prepared.icon_path.clone(),
                            Arc::clone(&feedback),
                            Arc::clone(&response_listeners),
                        );
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
                                notification.close(false);
                            }
                        }
                        active.insert(
                            key,
                            ActiveNotification {
                                native_id,
                                cancellation,
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
        notification.close(true);
    }
}

#[cfg(target_os = "macos")]
struct PreparedMacNotification {
    notification: mac_usernotifications::Notification,
    icon_path: Option<PathBuf>,
}

#[cfg(target_os = "macos")]
fn prepare_macos_notification(request: &DesktopNotification) -> PreparedMacNotification {
    use mac_usernotifications::{Action, InterruptionLevel};

    let title = request.title.as_deref().unwrap_or("toyoterm");
    let mut notification = mac_usernotifications::Notification::new()
        .title(title)
        .message(&request.body)
        .interruption_level(match request.urgency {
            NotificationUrgency::Low => InterruptionLevel::Passive,
            NotificationUrgency::Normal => InterruptionLevel::Active,
            NotificationUrgency::Critical => InterruptionLevel::TimeSensitive,
        });
    if let Some(sound_name) = notification_sound_name(request.sound) {
        notification = notification.sound(sound_name);
    }
    if let Some(id) = request.id {
        notification = notification.id(&id.to_string());
    }
    if let Some(timeout_ms) = request.timeout_ms.filter(|timeout| *timeout > 0) {
        notification = notification.timeout(Duration::from_millis(u64::from(timeout_ms)));
    }
    for (index, label) in request.buttons.iter().enumerate() {
        notification = notification.action(Action::button((index + 1).to_string(), label));
    }

    let named_icon = request
        .icon_name
        .as_deref()
        .and_then(standard_notification_icon);
    let icon = named_icon.as_ref().or(request.icon.as_ref());
    let icon_path = icon.and_then(|icon| {
        write_notification_icon(icon)
            .inspect_err(|error| {
                tracing::warn!(target: "toyoterm::notification", %error, "prepare OSC notification icon failed");
            })
            .ok()
    });
    if let Some(path) = icon_path.as_deref() {
        notification = notification.image_path(path.to_string_lossy());
    }
    PreparedMacNotification {
        notification,
        icon_path,
    }
}

#[cfg(target_os = "macos")]
fn start_macos_notification_listener(
    handle: mac_usernotifications::NotificationHandle,
    request: &DesktopNotification,
    icon_path: Option<PathBuf>,
    feedback: FeedbackHandler,
    response_listeners: Arc<AtomicUsize>,
) -> Option<futures_channel::oneshot::Sender<bool>> {
    use futures_lite::future;

    if !request.reporting.activation && !request.reporting.close {
        return None;
    }
    if response_listeners
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
            (count < MAX_NOTIFICATION_RESPONSE_LISTENERS).then_some(count + 1)
        })
        .is_err()
    {
        tracing::warn!(
            target: "toyoterm::notification",
            limit = MAX_NOTIFICATION_RESPONSE_LISTENERS,
            "OSC notification response listener limit reached"
        );
        return None;
    }

    enum Outcome {
        Response(Result<mac_usernotifications::NotificationResponse, mac_usernotifications::Error>),
        Cancelled(bool),
    }

    let pane = request.pane;
    let id = request.protocol_id.clone();
    let reporting = request.reporting;
    let native_id = handle.notification_id().to_owned();
    let completion_counter = Arc::clone(&response_listeners);
    let (cancel, cancelled) = futures_channel::oneshot::channel();
    let spawn = thread::Builder::new()
        .name("toyoterm-notification-response".into())
        .spawn(move || {
            let outcome = future::block_on(future::or(
                async move { Outcome::Response(handle.response().await) },
                async move {
                    let suppress = cancelled.await.unwrap_or(true);
                    mac_usernotifications::close_delivered(&native_id).await;
                    Outcome::Cancelled(suppress)
                },
            ));
            let response = match outcome {
                Outcome::Response(Ok(response)) if response.is_default_action() => {
                    Some(NotificationResponseKind::Default)
                }
                Outcome::Response(Ok(response)) if response.close_reason.is_some() => {
                    Some(NotificationResponseKind::Closed)
                }
                Outcome::Response(Ok(response)) => {
                    Some(NotificationResponseKind::Action(response.action_identifier))
                }
                Outcome::Response(Err(error)) => {
                    tracing::warn!(target: "toyoterm::notification", %error, "wait for OSC notification response failed");
                    None
                }
                Outcome::Cancelled(false) => Some(NotificationResponseKind::Closed),
                Outcome::Cancelled(true) => None,
            };
            if let Some(response) = response {
                for event in notification_feedbacks(pane, &id, reporting, response) {
                    feedback(event);
                }
            }
            if let Some(path) = icon_path {
                remove_icon_file(&path);
            }
            completion_counter.fetch_sub(1, Ordering::Relaxed);
        });
    if let Err(error) = spawn {
        response_listeners.fetch_sub(1, Ordering::Relaxed);
        tracing::warn!(target: "toyoterm::notification", %error, "start OSC notification response listener failed");
        return None;
    }
    Some(cancel)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn start_linux_notification_listener(
    native_id: u32,
    request: &DesktopNotification,
    response_suppressed: Arc<AtomicBool>,
    feedback: FeedbackHandler,
    response_listeners: Arc<AtomicUsize>,
) {
    if !request.reporting.activation && !request.reporting.close {
        return;
    }
    if response_listeners
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
            (count < MAX_NOTIFICATION_RESPONSE_LISTENERS).then_some(count + 1)
        })
        .is_err()
    {
        tracing::warn!(
            target: "toyoterm::notification",
            limit = MAX_NOTIFICATION_RESPONSE_LISTENERS,
            "OSC notification response listener limit reached"
        );
        return;
    }

    let pane = request.pane;
    let id = request.protocol_id.clone();
    let reporting = request.reporting;
    let completion_counter = Arc::clone(&response_listeners);
    let spawn = thread::Builder::new()
        .name("toyoterm-notification-response".into())
        .spawn(move || {
            let result = notify_rust::handle_action(native_id, |response| {
                if response_suppressed.load(Ordering::Acquire) {
                    return;
                }
                let response = match response {
                    notify_rust::ActionResponse::Custom("default") => {
                        NotificationResponseKind::Default
                    }
                    notify_rust::ActionResponse::Custom(action) => {
                        NotificationResponseKind::Action((*action).to_owned())
                    }
                    notify_rust::ActionResponse::Closed(_) => NotificationResponseKind::Closed,
                };
                for event in notification_feedbacks(pane, &id, reporting, response) {
                    feedback(event);
                }
            });
            if let Err(error) = result {
                tracing::warn!(target: "toyoterm::notification", %error, "wait for OSC notification response failed");
            }
            completion_counter.fetch_sub(1, Ordering::Relaxed);
        });
    if let Err(error) = spawn {
        response_listeners.fetch_sub(1, Ordering::Relaxed);
        tracing::warn!(target: "toyoterm::notification", %error, "start OSC notification response listener failed");
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum NotificationResponseKind {
    Default,
    Action(String),
    Closed,
}

fn notification_feedbacks(
    pane: PaneId,
    id: &str,
    reporting: NotificationReporting,
    response: NotificationResponseKind,
) -> Vec<NotificationFeedback> {
    let mut events = Vec::with_capacity(2);
    match response {
        NotificationResponseKind::Default if reporting.activation => {
            events.push(NotificationFeedback {
                pane,
                id: id.to_owned(),
                kind: NotificationFeedbackKind::Activated,
            });
        }
        NotificationResponseKind::Action(action) if reporting.activation => {
            if let Ok(button) = action.parse::<usize>() {
                events.push(NotificationFeedback {
                    pane,
                    id: id.to_owned(),
                    kind: NotificationFeedbackKind::Button(button),
                });
            }
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
    fn provides_bounded_fallbacks_for_every_standard_notification_icon() {
        for name in [
            "dialog-error",
            "dialog-warning",
            "dialog-information",
            "dialog-question",
            "help-browser",
            "system-file-manager",
            "utilities-system-monitor",
            "accessories-text-editor",
        ] {
            let icon = standard_notification_icon(name).unwrap();
            assert_eq!(icon.width, STANDARD_ICON_SIDE as u16);
            assert_eq!(icon.height, STANDARD_ICON_SIDE as u16);
            assert_eq!(icon.rgba.len(), STANDARD_ICON_SIDE * STANDARD_ICON_SIDE * 4);
            assert!(icon.rgba.iter().skip(3).step_by(4).any(|alpha| *alpha != 0));
        }
        assert!(standard_notification_icon("untrusted-application").is_none());
    }

    #[test]
    fn maps_every_standard_notification_sound_to_the_platform_backend() {
        for sound in [
            NotificationSound::System,
            NotificationSound::Silent,
            NotificationSound::Error,
            NotificationSound::Warning,
            NotificationSound::Info,
            NotificationSound::Question,
        ] {
            let mapped = notification_sound_name(sound);
            if cfg!(any(windows, target_os = "macos")) {
                assert_eq!(mapped.is_none(), sound == NotificationSound::Silent);
            } else {
                assert!(sound == NotificationSound::System || mapped.is_some());
            }
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
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
    fn builds_escaped_windows_notification_xml_with_actions() {
        let request = DesktopNotification {
            pane: PaneId(1),
            protocol_id: "build".into(),
            id: Some(7),
            title: Some("Build & test".into()),
            body: "Finished <successfully>".into(),
            urgency: NotificationUrgency::Critical,
            timeout_ms: Some(8_000),
            sound: NotificationSound::Warning,
            icon_name: None,
            icon: None,
            buttons: vec!["Open \"log\"".into(), "Dismiss".into()],
            reporting: NotificationReporting::default(),
        };

        let xml = windows_notification_xml(&request, None);
        assert!(xml.contains("duration=\"long\" scenario=\"reminder\""));
        assert!(xml.contains("Build &amp; test"));
        assert!(xml.contains("Finished &lt;successfully&gt;"));
        assert!(xml.contains("content=\"Open &quot;log&quot;\" arguments=\"1\""));
        assert!(xml.contains("ms-winsoundevent:Notification.Reminder"));
    }

    #[test]
    fn maps_notification_responses_to_requested_feedback() {
        let reporting = NotificationReporting {
            activation: true,
            close: true,
        };
        assert_eq!(
            notification_feedbacks(
                PaneId(3),
                "job",
                reporting,
                NotificationResponseKind::Action("2".into()),
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
                NotificationResponseKind::Default,
            )[0]
            .kind,
            NotificationFeedbackKind::Activated
        );
    }
}
