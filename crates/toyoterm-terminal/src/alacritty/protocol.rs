use std::collections::{HashMap, VecDeque};
use std::io::Cursor;
use std::sync::Arc;

use alacritty_terminal::vte::ansi::{NamedColor, Rgb};
use base64::Engine;
use cursor_icon::CursorIcon;

use super::{
    ITERM_COPY_PREFIX, MAX_ITERM_COPY_BASE64_BYTES, MAX_OSC_BADGE_FORMAT_BYTES,
    MAX_OSC_ICON_TITLE_BYTES, MAX_OSC_NOTIFICATION_BYTES, MAX_OSC_NOTIFICATION_ICON_BYTES,
    MAX_OSC_REMOTE_HOST_BYTES, MAX_OSC_REPORT_VARIABLE_NAME_BYTES, MAX_OSC_SESSION_STATUS_BYTES,
    MAX_OSC_SHELL_NAME_BYTES, MAX_OSC_URL_BYTES, MAX_OSC_USER_VAR_NAME_BYTES,
    MAX_OSC_USER_VAR_VALUE_BYTES, MAX_OSC52_COPY_BYTES, MAX_SHELL_INTEGRATION_PAYLOAD_BYTES,
    TerminalTransparentColor,
};

const OSC99_REPORTING_SUPPORTED: bool = cfg!(any(windows, unix));

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalEvent {
    TitleChanged(String),
    TitleReset,
    IconTitleChanged(String),
    CwdChanged(String),
    RemoteHostChanged(String),
    ShellIntegrationChanged {
        version: u32,
        shell: Option<String>,
    },
    ItermVariableQuery(String),
    ItermBadgeFormatChanged(String),
    CursorLineHighlightChanged(bool),
    AttentionRequested(TerminalAttention),
    FocusRequested,
    OpenUrlRequested(String),
    UserVarChanged {
        name: String,
        value: String,
    },
    MarkSet,
    PromptStarted,
    CommandLineStarted,
    CommandStarted,
    CommandFinished(Option<i32>),
    CapturedOutputCleared,
    MouseCursorChanged(CursorIcon),
    MouseCursorControl(String),
    FontControl(String),
    FontFamilyChanged(String),
    ColorControl(String),
    ColorPresetRequested(String),
    XtermSpecialColorSet {
        index: u8,
        color: [u8; 3],
    },
    XtermSpecialColorQuery(u8),
    XtermSpecialColorReset(Option<u8>),
    XtermSpecialColorMode {
        index: u8,
        enabled: bool,
    },
    XtermAuxColorSet {
        index: u8,
        color: [u8; 3],
    },
    XtermAuxColorQuery(u8),
    XtermAuxColorReset(u8),
    ItermDefaultColorQuery(i8),
    ItermUiColorChanged {
        role: ItermUiColorRole,
        color: [u8; 3],
    },
    ItermUiColorReset(ItermUiColorRole),
    ItermUiColorQuery {
        role: ItermUiColorRole,
        osc: u16,
    },
    ColorStackPush,
    ColorStackPop,
    TabColorChanged {
        component: TabColorComponent,
        value: u8,
    },
    TabColorSet([u8; 3]),
    TabColorReset,
    SessionStatusChanged(SessionStatusUpdate),
    ProgressChanged(TerminalProgress),
    ClipboardStore(String),
    ClipboardCaptureStart,
    ClipboardCaptureEnd,
    FileDownload {
        name: Option<String>,
        data: Vec<u8>,
        permissions: Option<u32>,
        modified_ns: Option<u64>,
    },
    FileTransferCommit(Vec<FileTransferEntry>),
    FileUploadRequest {
        session_id: String,
        quiet: u8,
        paths: Vec<FileUploadPath>,
    },
    FileUploadDataRequest {
        session_id: String,
        file_id: String,
        name: String,
        compressed: bool,
    },
    FileUploadCancel(String),
    BackgroundImageRequested(Option<String>),
    Notification {
        id: Option<String>,
        title: Option<String>,
        body: String,
        occasion: NotificationOccasion,
        urgency: NotificationUrgency,
        timeout_ms: Option<u32>,
        sound: NotificationSound,
        icon_name: Option<String>,
        icon: Option<NotificationIcon>,
        buttons: Vec<String>,
        reporting: NotificationReporting,
    },
    NotificationClose(String),
    NotificationAliveQuery(String),
    PtyWrite(String),
    Bell {
        visual_bell: Option<[u8; 3]>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileTransferEntryKind {
    Regular,
    Directory,
    Symlink,
    HardLink,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileTransferEntry {
    pub id: String,
    pub parent: Option<String>,
    pub name: String,
    pub kind: FileTransferEntryKind,
    pub data: Vec<u8>,
    pub permissions: Option<u32>,
    pub modified_ns: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileUploadPath {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationIcon {
    pub width: u16,
    pub height: u16,
    pub rgba: Arc<[u8]>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NotificationReporting {
    pub activation: bool,
    pub close: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ItermUiColorRole {
    Link,
    CursorForeground,
    Underline,
    SelectionBackground,
    SelectionForeground,
    VisualBell,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum DynamicUiColor {
    #[default]
    Default,
    Dynamic,
    Explicit([u8; 3]),
}

impl DynamicUiColor {
    pub(super) fn explicit(self) -> Option<[u8; 3]> {
        match self {
            Self::Explicit(color) => Some(color),
            Self::Default | Self::Dynamic => None,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SessionStatusUpdate {
    pub indicator: Option<Option<[u8; 3]>>,
    pub status: Option<Option<String>>,
    pub status_color: Option<Option<[u8; 3]>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalProgress {
    Hidden,
    Normal(u8),
    Error(Option<u8>),
    Indeterminate,
    Warning(Option<u8>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TabColorComponent {
    Red,
    Green,
    Blue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalAttention {
    Indefinite,
    Once,
    Cancel,
    Fireworks,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NotificationOccasion {
    #[default]
    Always,
    Unfocused,
    Invisible,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NotificationUrgency {
    Low,
    #[default]
    Normal,
    Critical,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NotificationSound {
    #[default]
    System,
    Silent,
    Error,
    Warning,
    Info,
    Question,
}

#[derive(Default)]
enum ShellIntegrationState {
    #[default]
    Ground,
    Escape,
    Command(Vec<u8>),
    CommandEscape(Vec<u8>),
    Payload(Vec<u8>),
    PayloadEscape(Vec<u8>),
    Ignore,
    IgnoreEscape,
}

#[derive(Default)]
pub(super) struct ShellIntegrationParser {
    state: ShellIntegrationState,
    notifications: NotificationAssemblies,
}

#[derive(Default)]
struct NotificationAssemblies {
    pending: HashMap<String, PendingNotification>,
    pending_icons: HashMap<String, PendingNotificationIcon>,
    icons: HashMap<String, CachedNotificationIcon>,
    icon_order: VecDeque<String>,
}

#[derive(Default)]
struct PendingNotificationIcon {
    encoded_chunks: Vec<Vec<u8>>,
    encoded_len: usize,
}

#[derive(Clone, Default)]
struct CachedNotificationIcon {
    name: Option<String>,
    image: Option<NotificationIcon>,
}

#[derive(Default)]
struct PendingNotification {
    title: String,
    body: String,
    occasion: NotificationOccasion,
    urgency: NotificationUrgency,
    timeout_ms: Option<u32>,
    sound: NotificationSound,
    icon_name: Option<String>,
    icon_cache_id: Option<String>,
    button_text: String,
    reporting: NotificationReporting,
}

impl NotificationAssemblies {
    fn cache_icon(&mut self, id: String, entry: CachedNotificationIcon) {
        self.icon_order.retain(|cached| cached != &id);
        if !self.icons.contains_key(&id)
            && self.icons.len() >= 16
            && let Some(oldest) = self.icon_order.pop_front()
        {
            self.icons.remove(&oldest);
        }
        self.icons.insert(id.clone(), entry);
        self.icon_order.push_back(id);
    }

    fn cached_icon(&mut self, id: &str) -> Option<CachedNotificationIcon> {
        let icon = self.icons.get(id)?.clone();
        self.icon_order.retain(|cached| cached != id);
        self.icon_order.push_back(id.to_owned());
        Some(icon)
    }

    fn remove_icon(&mut self, id: &str) {
        self.pending_icons.remove(id);
        self.icons.remove(id);
        self.icon_order.retain(|cached| cached != id);
    }
}

#[derive(Default)]
pub(crate) struct ClipboardCapture {
    pub(super) text: String,
    pub(super) overflowed: bool,
}

impl ClipboardCapture {
    pub(crate) fn push(&mut self, text: &str) {
        if self.overflowed {
            return;
        }
        if self.text.len().saturating_add(text.len()) > MAX_OSC52_COPY_BYTES {
            self.text.clear();
            self.overflowed = true;
        } else {
            self.text.push_str(text);
        }
    }
}

#[derive(Clone)]
pub(super) struct MouseCursorShape {
    pub(super) name: &'static str,
    pub(super) icon: CursorIcon,
}

#[derive(Default)]
pub(crate) struct MouseCursorStacks {
    primary: Vec<MouseCursorShape>,
    alternate: Vec<MouseCursorShape>,
}

impl MouseCursorStacks {
    pub(super) fn active_mut(&mut self, alternate: bool) -> &mut Vec<MouseCursorShape> {
        if alternate {
            &mut self.alternate
        } else {
            &mut self.primary
        }
    }

    pub(super) fn current(&self, alternate: bool) -> Option<&MouseCursorShape> {
        if alternate {
            self.alternate.last()
        } else {
            self.primary.last()
        }
    }

    pub fn current_icon(&self, alternate: bool) -> CursorIcon {
        self.current(alternate)
            .map_or(CursorIcon::Default, |shape| shape.icon)
    }

    pub fn reset(&mut self) {
        self.primary.clear();
        self.alternate.clear();
    }
}

impl ShellIntegrationParser {
    pub(super) fn advance(
        &mut self,
        bytes: &[u8],
        allow_osc52_copy: bool,
    ) -> Vec<(usize, TerminalEvent)> {
        let mut events = Vec::new();
        for (index, &byte) in bytes.iter().enumerate() {
            let state = std::mem::take(&mut self.state);
            self.state = match state {
                ShellIntegrationState::Ground if byte == b'\x1b' => ShellIntegrationState::Escape,
                ShellIntegrationState::Ground => ShellIntegrationState::Ground,
                ShellIntegrationState::Escape if byte == b']' => {
                    ShellIntegrationState::Command(Vec::new())
                }
                ShellIntegrationState::Escape => ShellIntegrationState::Ground,
                ShellIntegrationState::Command(command)
                    if byte == b';'
                        && matches!(
                            command.as_slice(),
                            b"1" | b"4"
                                | b"5"
                                | b"6"
                                | b"7"
                                | b"9"
                                | b"13"
                                | b"14"
                                | b"15"
                                | b"16"
                                | b"17"
                                | b"18"
                                | b"19"
                                | b"21"
                                | b"22"
                                | b"50"
                                | b"99"
                                | b"105"
                                | b"106"
                                | b"133"
                                | b"777"
                                | b"1337"
                                | b"21337"
                        ) =>
                {
                    let mut payload = command;
                    payload.push(b';');
                    ShellIntegrationState::Payload(payload)
                }
                ShellIntegrationState::Command(_) if byte == b';' => ShellIntegrationState::Ignore,
                ShellIntegrationState::Command(command) if byte == b'\x07' => {
                    if let Some(event) = bare_osc_event(&command) {
                        events.push((index, event));
                    }
                    ShellIntegrationState::Ground
                }
                ShellIntegrationState::Command(command) if byte == b'\x1b' => {
                    ShellIntegrationState::CommandEscape(command)
                }
                ShellIntegrationState::Command(mut command) if command.len() < 16 => {
                    command.push(byte);
                    ShellIntegrationState::Command(command)
                }
                ShellIntegrationState::Command(_) => ShellIntegrationState::Ignore,
                ShellIntegrationState::CommandEscape(command) if byte == b'\\' => {
                    if let Some(event) = bare_osc_event(&command) {
                        events.push((index, event));
                    }
                    ShellIntegrationState::Ground
                }
                ShellIntegrationState::CommandEscape(_) => ShellIntegrationState::Ignore,
                ShellIntegrationState::Payload(payload) if byte == b'\x07' => {
                    let mut parsed = Vec::new();
                    parse_shell_integration_payload(
                        &payload,
                        &mut parsed,
                        &mut self.notifications,
                        allow_osc52_copy,
                    );
                    events.extend(parsed.into_iter().map(|event| (index, event)));
                    ShellIntegrationState::Ground
                }
                ShellIntegrationState::Payload(payload) if byte == b'\x1b' => {
                    ShellIntegrationState::PayloadEscape(payload)
                }
                ShellIntegrationState::Payload(mut payload)
                    if payload.len() < shell_integration_payload_limit(&payload) =>
                {
                    payload.push(byte);
                    ShellIntegrationState::Payload(payload)
                }
                ShellIntegrationState::Payload(_) => ShellIntegrationState::Ignore,
                ShellIntegrationState::PayloadEscape(payload) if byte == b'\\' => {
                    let mut parsed = Vec::new();
                    parse_shell_integration_payload(
                        &payload,
                        &mut parsed,
                        &mut self.notifications,
                        allow_osc52_copy,
                    );
                    events.extend(parsed.into_iter().map(|event| (index, event)));
                    ShellIntegrationState::Ground
                }
                ShellIntegrationState::PayloadEscape(mut payload) => {
                    payload.push(b'\x1b');
                    payload.push(byte);
                    ShellIntegrationState::Payload(payload)
                }
                ShellIntegrationState::Ignore if byte == b'\x07' => ShellIntegrationState::Ground,
                ShellIntegrationState::Ignore if byte == b'\x1b' => {
                    ShellIntegrationState::IgnoreEscape
                }
                ShellIntegrationState::Ignore => ShellIntegrationState::Ignore,
                ShellIntegrationState::IgnoreEscape if byte == b'\\' => {
                    ShellIntegrationState::Ground
                }
                ShellIntegrationState::IgnoreEscape => ShellIntegrationState::Ignore,
            };
        }
        events
    }
}

fn shell_integration_payload_limit(payload: &[u8]) -> usize {
    if payload.starts_with(ITERM_COPY_PREFIX) || ITERM_COPY_PREFIX.starts_with(payload) {
        ITERM_COPY_PREFIX.len() + MAX_ITERM_COPY_BASE64_BYTES
    } else {
        MAX_SHELL_INTEGRATION_PAYLOAD_BYTES
    }
}

fn parse_shell_integration_payload(
    payload: &[u8],
    events: &mut Vec<TerminalEvent>,
    notifications: &mut NotificationAssemblies,
    allow_osc52_copy: bool,
) {
    if let Some(title) = payload.strip_prefix(b"1;") {
        if title.len() <= MAX_OSC_ICON_TITLE_BYTES
            && let Ok(title) = std::str::from_utf8(title)
            && !title.chars().any(char::is_control)
        {
            events.push(TerminalEvent::IconTitleChanged(title.to_owned()));
        }
    } else if let Some(control) = payload.strip_prefix(b"4;") {
        parse_iterm_default_color_queries(control, events);
    } else if let Some(control) = payload.strip_prefix(b"5;") {
        parse_xterm_special_color_control(5, control, events);
    } else if let Some(control) = payload.strip_prefix(b"6;") {
        parse_osc6_tab_color(control, events);
        parse_xterm_special_color_control(106, control, events);
    } else if let Some(payload) = payload.strip_prefix(b"7;") {
        if let Some(path) = osc7_path(payload) {
            events.push(TerminalEvent::CwdChanged(path));
        }
    } else if let Some(control) = payload.strip_prefix(b"22;") {
        if control.len() <= 1024
            && let Ok(control) = std::str::from_utf8(control)
            && !control.chars().any(char::is_control)
        {
            events.push(TerminalEvent::MouseCursorControl(control.to_owned()));
        }
    } else if let Some(control) = payload.strip_prefix(b"50;") {
        if control.len() <= 256
            && let Ok(control) = std::str::from_utf8(control)
            && !control.chars().any(char::is_control)
        {
            events.push(TerminalEvent::FontControl(control.to_owned()));
        }
    } else if let Some(control) = payload.strip_prefix(b"21;") {
        if control.len() <= 8_192
            && let Ok(control) = std::str::from_utf8(control)
            && !control.chars().any(char::is_control)
        {
            events.push(TerminalEvent::ColorControl(control.to_owned()));
        }
    } else if let Some(colors) = payload.strip_prefix(b"13;") {
        parse_xterm_dynamic_colors(13, colors, events);
    } else if let Some(colors) = payload.strip_prefix(b"14;") {
        parse_xterm_dynamic_colors(14, colors, events);
    } else if let Some(colors) = payload.strip_prefix(b"15;") {
        parse_xterm_dynamic_colors(15, colors, events);
    } else if let Some(colors) = payload.strip_prefix(b"16;") {
        parse_xterm_dynamic_colors(16, colors, events);
    } else if let Some(color) = payload.strip_prefix(b"17;") {
        parse_xterm_ui_color(color, ItermUiColorRole::SelectionBackground, 17, events);
    } else if let Some(colors) = payload.strip_prefix(b"18;") {
        parse_xterm_dynamic_colors(18, colors, events);
    } else if let Some(color) = payload.strip_prefix(b"19;") {
        parse_xterm_ui_color(color, ItermUiColorRole::SelectionForeground, 19, events);
    } else if let Some(message) = payload.strip_prefix(b"9;") {
        if let Some(progress) = osc9_progress(message) {
            events.push(TerminalEvent::ProgressChanged(progress));
        } else if message != b"4"
            && !message.starts_with(b"4;")
            && let Some(body) = notification_text(message)
        {
            events.push(TerminalEvent::Notification {
                id: None,
                title: None,
                body,
                occasion: NotificationOccasion::Always,
                urgency: NotificationUrgency::Normal,
                timeout_ms: None,
                sound: NotificationSound::System,
                icon_name: None,
                icon: None,
                buttons: Vec::new(),
                reporting: NotificationReporting::default(),
            });
        }
    } else if let Some(payload) = payload.strip_prefix(b"777;notify;") {
        if let Some(separator) = payload.iter().position(|byte| *byte == b';')
            && let (Some(title), Some(body)) = (
                notification_text(&payload[..separator]),
                notification_text(&payload[separator + 1..]),
            )
        {
            events.push(TerminalEvent::Notification {
                id: None,
                title: Some(title),
                body,
                occasion: NotificationOccasion::Always,
                urgency: NotificationUrgency::Normal,
                timeout_ms: None,
                sound: NotificationSound::System,
                icon_name: None,
                icon: None,
                buttons: Vec::new(),
                reporting: NotificationReporting::default(),
            });
        }
    } else if let Some(payload) = payload.strip_prefix(b"99;") {
        parse_osc99_notification(payload, events, notifications);
    } else if let Some(control) = payload.strip_prefix(b"105;") {
        parse_xterm_special_color_control(105, control, events);
    } else if let Some(control) = payload.strip_prefix(b"106;") {
        parse_xterm_special_color_control(106, control, events);
    } else if let Some(payload) = payload.strip_prefix(b"21337;") {
        if let Some(update) = osc21337_session_status(payload) {
            events.push(TerminalEvent::SessionStatusChanged(update));
        }
    } else if let Some(assignment) = payload.strip_prefix(b"1337;SetColors=") {
        parse_osc1337_set_colors(assignment, events);
    } else if let Some(name) = payload.strip_prefix(b"1337;SetProfile=") {
        if let Some(name) = bounded_preset_name(name) {
            events.push(TerminalEvent::ColorPresetRequested(name));
        }
    } else if let Some(path) = payload.strip_prefix(b"1337;CurrentDir=") {
        if !path.is_empty()
            && !path.contains(&0)
            && let Ok(path) = std::str::from_utf8(path)
        {
            events.push(TerminalEvent::CwdChanged(normalize_terminal_path(
                path.to_owned(),
            )));
        }
    } else if let Some(remote_host) = payload.strip_prefix(b"1337;RemoteHost=") {
        if let Some(remote_host) = osc1337_remote_host(remote_host) {
            events.push(TerminalEvent::RemoteHostChanged(remote_host));
        }
    } else if let Some(report) = payload.strip_prefix(b"1337;ShellIntegrationVersion=") {
        if let Some((version, shell)) = osc1337_shell_integration(report) {
            events.push(TerminalEvent::ShellIntegrationChanged { version, shell });
        }
    } else if let Some(encoded_name) = payload.strip_prefix(b"1337;ReportVariable=") {
        if let Some(name) = osc1337_report_variable(encoded_name) {
            events.push(TerminalEvent::ItermVariableQuery(name));
        }
    } else if let Some(encoded_format) = payload.strip_prefix(b"1337;SetBadgeFormat=") {
        if let Some(format) = osc1337_badge_format(encoded_format) {
            events.push(TerminalEvent::ItermBadgeFormatChanged(format));
        }
    } else if let Some(enabled) = payload.strip_prefix(b"1337;HighlightCursorLine=") {
        match enabled {
            b"yes" => events.push(TerminalEvent::CursorLineHighlightChanged(true)),
            b"no" => events.push(TerminalEvent::CursorLineHighlightChanged(false)),
            _ => {}
        }
    } else if let Some(request) = payload.strip_prefix(b"1337;RequestAttention=") {
        let request = match request {
            b"yes" => Some(TerminalAttention::Indefinite),
            b"once" => Some(TerminalAttention::Once),
            b"no" => Some(TerminalAttention::Cancel),
            b"fireworks" => Some(TerminalAttention::Fireworks),
            _ => None,
        };
        if let Some(request) = request {
            events.push(TerminalEvent::AttentionRequested(request));
        }
    } else if matches!(payload, b"1337;StealFocus" | b"1337;Disinter") {
        events.push(TerminalEvent::FocusRequested);
    } else if let Some(encoded_url) = payload.strip_prefix(b"1337;OpenURL=:") {
        if let Some(bytes) = decode_standard_base64(encoded_url)
            && bytes.len() <= MAX_OSC_URL_BYTES
            && let Ok(url) = String::from_utf8(bytes)
            && !url.is_empty()
            && !url.chars().any(char::is_control)
        {
            events.push(TerminalEvent::OpenUrlRequested(url));
        }
    } else if let Some(encoded_path) = payload.strip_prefix(b"1337;SetBackgroundImageFile=") {
        if encoded_path.is_empty() {
            events.push(TerminalEvent::BackgroundImageRequested(None));
        } else if let Some(bytes) = decode_standard_base64(encoded_path)
            && bytes.len() <= super::MAX_OSC_BACKGROUND_IMAGE_PATH_BYTES
            && let Ok(path) = String::from_utf8(bytes)
            && !path.is_empty()
            && !path.chars().any(char::is_control)
        {
            events.push(TerminalEvent::BackgroundImageRequested(Some(path)));
        }
    } else if allow_osc52_copy && payload == b"1337;CopyToClipboard=" {
        events.push(TerminalEvent::ClipboardCaptureStart);
    } else if allow_osc52_copy && payload == b"1337;EndCopy" {
        events.push(TerminalEvent::ClipboardCaptureEnd);
    } else if allow_osc52_copy
        && let Some(encoded) = payload.strip_prefix(b"1337;Copy=:")
        && encoded.len() <= MAX_ITERM_COPY_BASE64_BYTES
        && let Some(bytes) = decode_standard_base64(encoded)
        && bytes.len() <= MAX_OSC52_COPY_BYTES
        && let Ok(text) = String::from_utf8(bytes)
    {
        events.push(TerminalEvent::ClipboardStore(text));
    } else if let Some(encoded) = payload.strip_prefix(ITERM_COPY_PREFIX) {
        if allow_osc52_copy
            && encoded.len() <= MAX_ITERM_COPY_BASE64_BYTES
            && let Some(bytes) = decode_standard_base64(encoded)
            && bytes.len() <= MAX_OSC52_COPY_BYTES
            && let Ok(text) = String::from_utf8(bytes)
        {
            events.push(TerminalEvent::ClipboardStore(text));
        }
    } else if let Some(user_var) = payload.strip_prefix(b"1337;SetUserVar=") {
        if let Some((name, value)) = osc1337_user_var(user_var) {
            events.push(TerminalEvent::UserVarChanged { name, value });
        }
    } else if payload == b"1337;SetMark" {
        events.push(TerminalEvent::MarkSet);
    } else if payload == b"1337;ClearCapturedOutput" {
        events.push(TerminalEvent::CapturedOutputCleared);
    } else if osc133_marker(payload, b'A') {
        events.push(TerminalEvent::PromptStarted);
    } else if osc133_marker(payload, b'B') {
        events.push(TerminalEvent::CommandLineStarted);
    } else if payload == b"133;C" {
        events.push(TerminalEvent::CommandStarted);
    } else if payload == b"133;D" {
        events.push(TerminalEvent::CommandFinished(None));
    } else if let Some(status) = payload.strip_prefix(b"133;D;") {
        let status = (!status.is_empty())
            .then_some(status)
            .and_then(|status| std::str::from_utf8(status).ok())
            .and_then(|status| status.parse().ok());
        events.push(TerminalEvent::CommandFinished(status));
    }
}

fn parse_xterm_dynamic_colors(start: u8, colors: &[u8], events: &mut Vec<TerminalEvent>) {
    if colors.len() > 1024 {
        return;
    }
    for (offset, color) in colors.split(|byte| *byte == b';').enumerate() {
        let Ok(offset) = u8::try_from(offset) else {
            break;
        };
        let Some(osc) = start.checked_add(offset).filter(|osc| *osc <= 19) else {
            break;
        };
        match osc {
            13..=16 | 18 => {
                let index = match osc {
                    13..=16 => osc - 13,
                    18 => 4,
                    _ => unreachable!(),
                };
                if color == b"?" {
                    events.push(TerminalEvent::XtermAuxColorQuery(index));
                } else if let Ok(color) = std::str::from_utf8(color)
                    && let Some(Rgb { r, g, b }) = parse_kitty_color(color)
                {
                    events.push(TerminalEvent::XtermAuxColorSet {
                        index,
                        color: [r, g, b],
                    });
                }
            }
            17 => parse_xterm_ui_color(color, ItermUiColorRole::SelectionBackground, 17, events),
            19 => parse_xterm_ui_color(color, ItermUiColorRole::SelectionForeground, 19, events),
            _ => {}
        }
    }
}

fn osc21337_session_status(payload: &[u8]) -> Option<SessionStatusUpdate> {
    let mut update = SessionStatusUpdate::default();
    let mut recognized = false;

    for field in payload.split(|byte| *byte == b';') {
        let Some(separator) = field.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let (name, value) = (&field[..separator], &field[separator + 1..]);
        match name {
            b"indicator" => {
                update.indicator = Some(parse_optional_session_color(value)?);
                recognized = true;
            }
            b"status" => {
                if value.len() > MAX_OSC_SESSION_STATUS_BYTES {
                    return None;
                }
                let value = std::str::from_utf8(value).ok()?;
                if value.chars().any(char::is_control) {
                    return None;
                }
                update.status = Some((!value.is_empty()).then(|| value.to_owned()));
                recognized = true;
            }
            b"status-color" => {
                update.status_color = Some(parse_optional_session_color(value)?);
                recognized = true;
            }
            _ => {}
        }
    }

    recognized.then_some(update)
}

fn parse_optional_session_color(value: &[u8]) -> Option<Option<[u8; 3]>> {
    if value.is_empty() {
        return Some(None);
    }
    let hex = value.strip_prefix(b"#")?;
    if hex.len() != 6 {
        return None;
    }
    Some(Some([
        parse_hex_byte(&hex[..2])?,
        parse_hex_byte(&hex[2..4])?,
        parse_hex_byte(&hex[4..])?,
    ]))
}

fn parse_hex_byte(value: &[u8]) -> Option<u8> {
    u8::from_str_radix(std::str::from_utf8(value).ok()?, 16).ok()
}

pub(super) fn parse_iterm_color(color: &[u8]) -> Option<[u8; 3]> {
    if let Some(color) = color.strip_prefix(b"p3:") {
        return Some(display_p3_to_srgb(parse_iterm_color_components(color)?));
    }
    let color = color
        .strip_prefix(b"srgb:")
        .or_else(|| color.strip_prefix(b"rgb:"))
        .unwrap_or(color);
    parse_iterm_color_components(color)
}

fn parse_iterm_color_components(color: &[u8]) -> Option<[u8; 3]> {
    match color.len() {
        3 => Some([
            hex_nibble(color[0])? * 17,
            hex_nibble(color[1])? * 17,
            hex_nibble(color[2])? * 17,
        ]),
        6 => Some([
            hex_byte(&color[0..2])?,
            hex_byte(&color[2..4])?,
            hex_byte(&color[4..6])?,
        ]),
        _ => None,
    }
}

fn display_p3_to_srgb(color: [u8; 3]) -> [u8; 3] {
    let [red, green, blue] = color.map(|channel| decode_srgb(f64::from(channel) / 255.0));
    let x = 0.486_570_948_648_216_2 * red
        + 0.265_667_693_169_093_06 * green
        + 0.198_217_285_234_362_5 * blue;
    let y = 0.228_974_564_069_748_8 * red
        + 0.691_738_521_836_506_4 * green
        + 0.079_286_914_093_745 * blue;
    let z = 0.045_113_381_858_902_64 * green + 1.043_944_368_900_976 * blue;

    [
        3.240_969_941_904_522_6 * x - 1.537_383_177_570_094 * y - 0.498_610_760_293 * z,
        -0.969_243_636_280_879_6 * x + 1.875_967_501_507_720_2 * y + 0.041_555_057_407_175 * z,
        0.055_630_079_696_993_66 * x - 0.203_976_958_888_976_52 * y + 1.056_971_514_242_878_6 * z,
    ]
    .map(|channel| (encode_srgb(channel.clamp(0.0, 1.0)) * 255.0).round() as u8)
}

fn decode_srgb(channel: f64) -> f64 {
    if channel <= 0.040_45 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

fn encode_srgb(channel: f64) -> f64 {
    if channel <= 0.003_130_8 {
        12.92 * channel
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    }
}

fn parse_osc1337_set_colors(assignment: &[u8], events: &mut Vec<TerminalEvent>) {
    let Some(separator) = assignment.iter().position(|byte| *byte == b'=') else {
        return;
    };
    let (key, value) = (
        &assignment[..separator],
        &assignment[separator.saturating_add(1)..],
    );
    if value.contains(&b';') {
        return;
    }
    if key == b"preset" {
        if let Some(name) = bounded_preset_name(value) {
            events.push(TerminalEvent::ColorPresetRequested(name));
        }
        return;
    }
    if key == b"tab" {
        if value == b"default" {
            events.push(TerminalEvent::TabColorReset);
        } else if let Some(color) = parse_iterm_color(value) {
            events.push(TerminalEvent::TabColorSet(color));
        }
        return;
    }

    let Some([red, green, blue]) = parse_iterm_color(value) else {
        return;
    };
    let color = [red, green, blue];
    let role = match key {
        b"link" => Some(ItermUiColorRole::Link),
        b"curfg" => Some(ItermUiColorRole::CursorForeground),
        b"underline" => Some(ItermUiColorRole::Underline),
        b"selbg" => Some(ItermUiColorRole::SelectionBackground),
        b"selfg" => Some(ItermUiColorRole::SelectionForeground),
        _ => None,
    };
    if let Some(role) = role {
        events.push(TerminalEvent::ItermUiColorChanged { role, color });
        return;
    }
    let Some(target) = iterm_color_control_target(key) else {
        return;
    };
    events.push(TerminalEvent::ColorControl(format!(
        "{target}=#{red:02x}{green:02x}{blue:02x}"
    )));
}

fn bounded_preset_name(value: &[u8]) -> Option<String> {
    (!value.is_empty() && value.len() <= 128)
        .then(|| std::str::from_utf8(value).ok())
        .flatten()
        .filter(|name| !name.chars().any(char::is_control))
        .map(str::to_owned)
}

fn parse_xterm_ui_color(
    value: &[u8],
    role: ItermUiColorRole,
    osc: u16,
    events: &mut Vec<TerminalEvent>,
) {
    if value == b"?" {
        events.push(TerminalEvent::ItermUiColorQuery { role, osc });
    } else if let Ok(value) = std::str::from_utf8(value)
        && let Some(Rgb { r, g, b }) = parse_kitty_color(value)
    {
        events.push(TerminalEvent::ItermUiColorChanged {
            role,
            color: [r, g, b],
        });
    }
}

fn iterm_color_control_target(key: &[u8]) -> Option<&'static str> {
    match key {
        b"fg" => Some("foreground"),
        b"bold" => Some("bright_foreground"),
        b"bg" => Some("background"),
        b"curbg" => Some("cursor"),
        b"black" => Some("0"),
        b"red" => Some("1"),
        b"green" => Some("2"),
        b"yellow" => Some("3"),
        b"blue" => Some("4"),
        b"magenta" => Some("5"),
        b"cyan" => Some("6"),
        b"white" => Some("7"),
        b"br_black" => Some("8"),
        b"br_red" => Some("9"),
        b"br_green" => Some("10"),
        b"br_yellow" => Some("11"),
        b"br_blue" => Some("12"),
        b"br_magenta" => Some("13"),
        b"br_cyan" => Some("14"),
        b"br_white" => Some("15"),
        _ => None,
    }
}

fn hex_byte(value: &[u8]) -> Option<u8> {
    Some(hex_nibble(value[0])? * 16 + hex_nibble(value[1])?)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn parse_osc6_tab_color(control: &[u8], events: &mut Vec<TerminalEvent>) {
    if control == b"1;bg;*;default" {
        events.push(TerminalEvent::TabColorReset);
        events.push(TerminalEvent::TitleReset);
        return;
    }
    let mut fields = control.split(|byte| *byte == b';');
    if fields.next() != Some(b"1".as_slice()) || fields.next() != Some(b"bg".as_slice()) {
        return;
    }
    let component = match fields.next() {
        Some(b"red") => TabColorComponent::Red,
        Some(b"green") => TabColorComponent::Green,
        Some(b"blue") => TabColorComponent::Blue,
        _ => return,
    };
    if fields.next() != Some(b"brightness".as_slice()) {
        return;
    }
    let Some(value) = fields.next() else {
        return;
    };
    if fields.next().is_some() {
        return;
    }
    let Some(value) = std::str::from_utf8(value)
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
    else {
        return;
    };
    events.push(TerminalEvent::TabColorChanged { component, value });
}

fn osc9_progress(message: &[u8]) -> Option<TerminalProgress> {
    if message == b"4" {
        return Some(TerminalProgress::Hidden);
    }
    let parameters = message.strip_prefix(b"4;")?;
    let mut parameters = parameters.split(|byte| *byte == b';');
    let state = parameters.next()?;
    let value = parameters.next().map(|value| {
        std::str::from_utf8(value)
            .ok()?
            .parse::<u8>()
            .ok()
            .filter(|value| *value <= 100)
    });
    if parameters.next().is_some() {
        return None;
    }
    let value = match value {
        Some(Some(value)) => Some(value),
        Some(None) => return None,
        None => None,
    };
    match state {
        b"0" => Some(TerminalProgress::Hidden),
        b"1" => value.map(TerminalProgress::Normal),
        b"2" => Some(TerminalProgress::Error(value)),
        b"3" => Some(TerminalProgress::Indeterminate),
        b"4" => Some(TerminalProgress::Warning(value)),
        _ => None,
    }
}

fn parse_iterm_default_color_queries(control: &[u8], events: &mut Vec<TerminalEvent>) {
    let mut fields = control.split(|byte| *byte == b';');
    while let (Some(index), Some(value)) = (fields.next(), fields.next()) {
        if value != b"?" {
            continue;
        }
        let index = match index {
            b"-1" => -1,
            b"-2" => -2,
            _ => continue,
        };
        events.push(TerminalEvent::ItermDefaultColorQuery(index));
    }
}

fn parse_xterm_special_color_control(osc: u16, control: &[u8], events: &mut Vec<TerminalEvent>) {
    if control.len() > 1024 {
        return;
    }
    let Ok(control) = std::str::from_utf8(control) else {
        return;
    };
    let fields = control.split(';').collect::<Vec<_>>();
    match osc {
        5 if fields.len() % 2 == 0 => {
            for pair in fields.as_chunks::<2>().0 {
                let Some(index) = pair[0].parse::<u8>().ok().filter(|index| *index < 5) else {
                    continue;
                };
                if pair[1] == "?" {
                    events.push(TerminalEvent::XtermSpecialColorQuery(index));
                } else if let Some(Rgb { r, g, b }) = parse_kitty_color(pair[1]) {
                    events.push(TerminalEvent::XtermSpecialColorSet {
                        index,
                        color: [r, g, b],
                    });
                }
            }
        }
        105 if control.is_empty() => {
            events.push(TerminalEvent::XtermSpecialColorReset(None));
        }
        105 => events.extend(
            fields
                .into_iter()
                .filter_map(|index| index.parse::<u8>().ok())
                .filter(|index| *index < 5)
                .map(|index| TerminalEvent::XtermSpecialColorReset(Some(index))),
        ),
        106 if fields.len() % 2 == 0 => {
            for pair in fields.as_chunks::<2>().0 {
                let (Some(index), Some(enabled)) = (
                    pair[0].parse::<u8>().ok().filter(|index| *index <= 5),
                    pair[1].parse::<i64>().ok().map(|value| value != 0),
                ) else {
                    continue;
                };
                events.push(TerminalEvent::XtermSpecialColorMode { index, enabled });
            }
        }
        _ => {}
    }
}

fn bare_osc_event(command: &[u8]) -> Option<TerminalEvent> {
    match command {
        b"113" => Some(TerminalEvent::XtermAuxColorReset(0)),
        b"114" => Some(TerminalEvent::XtermAuxColorReset(1)),
        b"115" => Some(TerminalEvent::XtermAuxColorReset(2)),
        b"116" => Some(TerminalEvent::XtermAuxColorReset(3)),
        b"117" => Some(TerminalEvent::ItermUiColorReset(
            ItermUiColorRole::SelectionBackground,
        )),
        b"119" => Some(TerminalEvent::ItermUiColorReset(
            ItermUiColorRole::SelectionForeground,
        )),
        b"118" => Some(TerminalEvent::XtermAuxColorReset(4)),
        b"105" => Some(TerminalEvent::XtermSpecialColorReset(None)),
        b"30001" => Some(TerminalEvent::ColorStackPush),
        b"30101" => Some(TerminalEvent::ColorStackPop),
        _ => None,
    }
}

pub(super) fn mouse_cursor_shape(name: &str) -> Option<MouseCursorShape> {
    let icon = match name {
        "alias" => CursorIcon::Alias,
        "cell" => CursorIcon::Cell,
        "copy" => CursorIcon::Copy,
        "crosshair" => CursorIcon::Crosshair,
        "default" => CursorIcon::Default,
        "e-resize" => CursorIcon::EResize,
        "ew-resize" => CursorIcon::EwResize,
        "grab" => CursorIcon::Grab,
        "grabbing" => CursorIcon::Grabbing,
        "help" => CursorIcon::Help,
        "move" => CursorIcon::Move,
        "n-resize" => CursorIcon::NResize,
        "ne-resize" => CursorIcon::NeResize,
        "nesw-resize" => CursorIcon::NeswResize,
        "no-drop" => CursorIcon::NoDrop,
        "not-allowed" => CursorIcon::NotAllowed,
        "ns-resize" => CursorIcon::NsResize,
        "nw-resize" => CursorIcon::NwResize,
        "nwse-resize" => CursorIcon::NwseResize,
        "pointer" => CursorIcon::Pointer,
        "progress" => CursorIcon::Progress,
        "s-resize" => CursorIcon::SResize,
        "se-resize" => CursorIcon::SeResize,
        "sw-resize" => CursorIcon::SwResize,
        "text" => CursorIcon::Text,
        "vertical-text" => CursorIcon::VerticalText,
        "w-resize" => CursorIcon::WResize,
        "wait" => CursorIcon::Wait,
        "zoom-in" => CursorIcon::ZoomIn,
        "zoom-out" => CursorIcon::ZoomOut,
        _ => return None,
    };
    Some(MouseCursorShape {
        name: match name {
            "alias" => "alias",
            "cell" => "cell",
            "copy" => "copy",
            "crosshair" => "crosshair",
            "default" => "default",
            "e-resize" => "e-resize",
            "ew-resize" => "ew-resize",
            "grab" => "grab",
            "grabbing" => "grabbing",
            "help" => "help",
            "move" => "move",
            "n-resize" => "n-resize",
            "ne-resize" => "ne-resize",
            "nesw-resize" => "nesw-resize",
            "no-drop" => "no-drop",
            "not-allowed" => "not-allowed",
            "ns-resize" => "ns-resize",
            "nw-resize" => "nw-resize",
            "nwse-resize" => "nwse-resize",
            "pointer" => "pointer",
            "progress" => "progress",
            "s-resize" => "s-resize",
            "se-resize" => "se-resize",
            "sw-resize" => "sw-resize",
            "text" => "text",
            "vertical-text" => "vertical-text",
            "w-resize" => "w-resize",
            "wait" => "wait",
            "zoom-in" => "zoom-in",
            "zoom-out" => "zoom-out",
            _ => unreachable!(),
        },
        icon,
    })
}

pub(super) fn color_control_index(key: &str) -> Option<usize> {
    match key {
        "foreground" => Some(NamedColor::Foreground as usize),
        "bright_foreground" => Some(NamedColor::BrightForeground as usize),
        "background" => Some(NamedColor::Background as usize),
        "cursor" => Some(NamedColor::Cursor as usize),
        _ => key.parse::<u8>().ok().map(usize::from),
    }
}

pub(super) fn kitty_ui_color_role(key: &str) -> Option<ItermUiColorRole> {
    match key {
        "selection_background" => Some(ItermUiColorRole::SelectionBackground),
        "selection_foreground" => Some(ItermUiColorRole::SelectionForeground),
        "cursor_text" => Some(ItermUiColorRole::CursorForeground),
        "visual_bell" => Some(ItermUiColorRole::VisualBell),
        _ => None,
    }
}

pub(super) fn transparent_background_index(key: &str) -> Option<usize> {
    let index = key
        .strip_prefix("transparent_background_color")?
        .parse::<u8>()
        .ok()?;
    (1..=7).contains(&index).then(|| usize::from(index - 1))
}

fn split_kitty_color_alpha(value: &str) -> Option<(&str, Option<f32>)> {
    let Some((color, alpha)) = value.split_once('@') else {
        return Some((value, None));
    };
    if color.contains('@') || alpha.contains('@') || color.is_empty() || alpha.is_empty() {
        return None;
    }
    let alpha = alpha.parse::<f64>().ok()?;
    if !alpha.is_finite() {
        return None;
    }
    Some((color, (alpha >= 0.0).then(|| alpha.clamp(0.0, 1.0) as f32)))
}

pub(super) fn parse_kitty_color(value: &str) -> Option<Rgb> {
    let (value, _) = split_kitty_color_alpha(value)?;
    if let Some(hex) = value.strip_prefix('#') {
        if hex.len() % 3 != 0 || !(3..=12).contains(&hex.len()) {
            return None;
        }
        let digits = hex.len() / 3;
        let component = |component: &str| {
            let raw = u16::from_str_radix(component, 16).ok()?;
            Some(match digits {
                1 => (raw << 4) as u8,
                2 => raw as u8,
                3 => (raw >> 4) as u8,
                4 => (raw >> 8) as u8,
                _ => return None,
            })
        };
        return Some(Rgb {
            r: component(&hex[..digits])?,
            g: component(&hex[digits..digits * 2])?,
            b: component(&hex[digits * 2..])?,
        });
    }
    if let Some(rgb) = value.strip_prefix("rgb:") {
        let components = rgb.split('/').collect::<Vec<_>>();
        if components.len() != 3 {
            return None;
        }
        let component = |component: &str| {
            if component.is_empty() || component.len() > 4 {
                return None;
            }
            let raw = u32::from_str_radix(component, 16).ok()?;
            let maximum = (1_u32 << (component.len() * 4)) - 1;
            Some((raw * 255 / maximum) as u8)
        };
        return Some(Rgb {
            r: component(components[0])?,
            g: component(components[1])?,
            b: component(components[2])?,
        });
    }
    if let Some(rgb) = value.strip_prefix("rgbi:") {
        let components = rgb.split('/').collect::<Vec<_>>();
        if components.len() != 3 {
            return None;
        }
        let component = |component: &str| {
            let value = component.parse::<f64>().ok()?;
            value
                .is_finite()
                .then(|| (value.clamp(0.0, 1.0) * 255.0).round() as u8)
        };
        return Some(Rgb {
            r: component(components[0])?,
            g: component(components[1])?,
            b: component(components[2])?,
        });
    }

    let [r, g, b] = match value.to_ascii_lowercase().as_str() {
        "black" => [0, 0, 0],
        "red" => [255, 0, 0],
        "green" => [0, 255, 0],
        "yellow" => [255, 255, 0],
        "blue" => [0, 0, 255],
        "magenta" => [255, 0, 255],
        "cyan" => [0, 255, 255],
        "white" => [255, 255, 255],
        "gray" | "grey" => [190, 190, 190],
        _ => return None,
    };
    Some(Rgb { r, g, b })
}

pub(super) fn parse_transparent_background(value: &str) -> Option<TerminalTransparentColor> {
    let (color, opacity) = split_kitty_color_alpha(value)?;
    let Rgb { r, g, b } = parse_kitty_color(color)?;
    Some(TerminalTransparentColor {
        color: [r, g, b],
        opacity,
    })
}

pub(super) fn format_transparent_background(value: TerminalTransparentColor) -> String {
    let [red, green, blue] = value.color;
    let mut response = format!("rgb:{red:02x}/{green:02x}/{blue:02x}");
    if let Some(opacity) = value.opacity {
        let formatted = format!("{opacity:.6}");
        response.push('@');
        response.push_str(formatted.trim_end_matches('0').trim_end_matches('.'));
    }
    response
}

fn parse_osc99_notification(
    payload: &[u8],
    events: &mut Vec<TerminalEvent>,
    notifications: &mut NotificationAssemblies,
) {
    let Some(separator) = payload.iter().position(|byte| *byte == b';') else {
        return;
    };
    let Ok(metadata) = std::str::from_utf8(&payload[..separator]) else {
        return;
    };
    let mut id = None;
    let mut payload_type = "title";
    let mut done = true;
    let mut encoded = false;
    let mut occasion = None;
    let mut urgency = None;
    let mut timeout_ms = None;
    let mut sound = None;
    let mut icon_name = None;
    let mut icon_cache_id = None;
    let mut report_activation = None;
    let mut report_close = None;
    for item in metadata.split(':').filter(|item| !item.is_empty()) {
        let Some((key, value)) = item.split_once('=') else {
            return;
        };
        if key.len() != 1 || !key.as_bytes()[0].is_ascii_alphabetic() {
            return;
        }
        match key {
            "i" => {
                if !valid_notification_id(value) {
                    return;
                }
                id = Some(value);
            }
            "p" => payload_type = value,
            "d" => match value {
                "0" => done = false,
                "1" => done = true,
                _ => return,
            },
            "e" => match value {
                "0" => encoded = false,
                "1" => encoded = true,
                _ => return,
            },
            "o" => {
                occasion = Some(match value {
                    "always" => NotificationOccasion::Always,
                    "unfocused" => NotificationOccasion::Unfocused,
                    "invisible" => NotificationOccasion::Invisible,
                    _ => return,
                });
            }
            "u" => {
                urgency = Some(match value {
                    "0" => NotificationUrgency::Low,
                    "1" => NotificationUrgency::Normal,
                    "2" => NotificationUrgency::Critical,
                    _ => return,
                });
            }
            "w" => {
                let Ok(value) = value.parse::<i64>() else {
                    return;
                };
                timeout_ms = Some(match value {
                    -1 => None,
                    value if (0..=i64::from(u32::MAX)).contains(&value) => Some(value as u32),
                    _ => return,
                });
            }
            "s" => {
                let Some(value) = decode_standard_base64(value.as_bytes()) else {
                    return;
                };
                sound = Some(match value.as_slice() {
                    b"system" => NotificationSound::System,
                    b"silent" => NotificationSound::Silent,
                    b"error" => NotificationSound::Error,
                    b"warn" | b"warning" => NotificationSound::Warning,
                    b"info" => NotificationSound::Info,
                    b"question" => NotificationSound::Question,
                    _ => return,
                });
            }
            "n" if icon_name.is_none() => {
                let Some(value) = decode_standard_base64(value.as_bytes()) else {
                    return;
                };
                let Ok(value) = String::from_utf8(value) else {
                    return;
                };
                let Some(value) = notification_icon_name(&value) else {
                    return;
                };
                icon_name = Some(value);
            }
            "g" => {
                if !valid_notification_id(value) {
                    return;
                }
                icon_cache_id = Some(value);
            }
            "a" => {
                let mut report = false;
                for action in value.split(',') {
                    match action {
                        "report" => report = true,
                        "-report" => report = false,
                        "focus" | "-focus" => {}
                        _ => return,
                    }
                }
                report_activation = Some(report && OSC99_REPORTING_SUPPORTED);
            }
            "c" => {
                report_close = Some(match value {
                    "0" => false,
                    "1" => OSC99_REPORTING_SUPPORTED,
                    _ => return,
                });
            }
            _ => {}
        }
    }

    let payload = &payload[separator + 1..];

    if payload_type == "?" {
        let id = id.unwrap_or("0");
        let payload_types = "title,body,close,icon,buttons,alive";
        let sounds = "system,silent,error,warn,warning,info,question";
        let expiry = ":w=1";
        let reports = if OSC99_REPORTING_SUPPORTED {
            ":a=report:c=1"
        } else {
            ""
        };
        events.push(TerminalEvent::PtyWrite(format!(
            "\x1b]99;i={id}:p=?;o=always,unfocused,invisible:p={payload_types}:s={sounds}:u=0,1,2{expiry}{reports}\x1b\\"
        )));
        return;
    }
    if payload_type == "close" {
        if let Some(id) = id {
            notifications.pending.remove(id);
            events.push(TerminalEvent::NotificationClose(id.to_owned()));
        }
        return;
    }
    if payload_type == "alive" {
        events.push(TerminalEvent::NotificationAliveQuery(
            id.unwrap_or("0").to_owned(),
        ));
        return;
    }
    if payload_type == "icon" {
        apply_notification_icon_chunk(
            notifications,
            icon_cache_id.or(id),
            icon_name,
            encoded,
            done,
            payload,
        );
        return;
    }
    if !matches!(payload_type, "title" | "body" | "buttons") {
        return;
    }

    if payload.len() > 4096 {
        return;
    }
    let decoded = if encoded {
        let Some(decoded) = decode_standard_base64(payload) else {
            return;
        };
        decoded
    } else {
        payload.to_vec()
    };
    if decoded.len() > 2048 {
        return;
    }
    let Ok(text) = String::from_utf8(decoded) else {
        return;
    };
    if text.chars().any(char::is_control) {
        return;
    }

    let mut notification = match id {
        Some(id) if notifications.pending.contains_key(id) => {
            notifications.pending.remove(id).unwrap_or_default()
        }
        _ => PendingNotification::default(),
    };
    if let Some(occasion) = occasion {
        notification.occasion = occasion;
    }
    if let Some(urgency) = urgency {
        notification.urgency = urgency;
    }
    if let Some(timeout_ms) = timeout_ms {
        notification.timeout_ms = timeout_ms;
    }
    if let Some(sound) = sound {
        notification.sound = sound;
    }
    if let Some(icon_name) = icon_name {
        notification.icon_name = Some(icon_name);
    }
    if let Some(icon_cache_id) = icon_cache_id {
        notification.icon_cache_id = Some(icon_cache_id.to_owned());
        if let Some(icon_name) = notification.icon_name.clone() {
            let mut cached = notifications.cached_icon(icon_cache_id).unwrap_or_default();
            cached.name = Some(icon_name);
            notifications.cache_icon(icon_cache_id.to_owned(), cached);
        }
    }
    if let Some(report_activation) = report_activation {
        notification.reporting.activation = report_activation;
    }
    if let Some(report_close) = report_close {
        notification.reporting.close = report_close;
    }
    let target = match payload_type {
        "body" => &mut notification.body,
        "buttons" => &mut notification.button_text,
        _ => &mut notification.title,
    };
    if target.len().saturating_add(text.len()) > MAX_OSC_NOTIFICATION_BYTES {
        return;
    }
    target.push_str(&text);

    if !done {
        let Some(id) = id else {
            return;
        };
        if notifications.pending.len() < 32 || notifications.pending.contains_key(id) {
            notifications.pending.insert(id.to_owned(), notification);
        }
        return;
    }
    if notification.title.is_empty() && notification.body.is_empty() {
        return;
    }
    let buttons = if notification.button_text.is_empty() {
        Vec::new()
    } else {
        let buttons = notification
            .button_text
            .split('\u{2028}')
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if buttons.len() > 3
            || buttons
                .iter()
                .any(|button| button.is_empty() || button.len() > 128)
        {
            return;
        }
        buttons
    };
    let cached_icon = notification
        .icon_cache_id
        .as_deref()
        .and_then(|id| notifications.cached_icon(id));
    if notification.icon_name.is_none() {
        notification.icon_name = cached_icon.as_ref().and_then(|icon| icon.name.clone());
    }
    events.push(TerminalEvent::Notification {
        id: id.map(str::to_owned),
        title: (!notification.title.is_empty()).then_some(notification.title),
        body: notification.body,
        occasion: notification.occasion,
        urgency: notification.urgency,
        timeout_ms: notification.timeout_ms,
        sound: notification.sound,
        icon_name: notification.icon_name,
        icon: cached_icon.and_then(|icon| icon.image),
        buttons,
        reporting: notification.reporting,
    });
}

fn apply_notification_icon_chunk(
    notifications: &mut NotificationAssemblies,
    cache_id: Option<&str>,
    icon_name: Option<String>,
    encoded: bool,
    done: bool,
    payload: &[u8],
) {
    const MAX_ENCODED_BYTES: usize = MAX_OSC_NOTIFICATION_ICON_BYTES.div_ceil(3) * 4 + 4;
    let Some(cache_id) = cache_id else {
        return;
    };
    if payload.is_empty() {
        notifications.remove_icon(cache_id);
        if let Some(icon_name) = icon_name {
            notifications.cache_icon(
                cache_id.to_owned(),
                CachedNotificationIcon {
                    name: Some(icon_name),
                    image: None,
                },
            );
        }
        return;
    }
    if !encoded || payload.len() > 4096 {
        notifications.pending_icons.remove(cache_id);
        return;
    }

    let pending = notifications
        .pending_icons
        .entry(cache_id.to_owned())
        .or_default();
    if pending.encoded_chunks.len() >= 256
        || pending.encoded_len.saturating_add(payload.len()) > MAX_ENCODED_BYTES
    {
        notifications.pending_icons.remove(cache_id);
        return;
    }
    pending.encoded_len += payload.len();
    pending.encoded_chunks.push(payload.to_vec());
    if !done {
        return;
    }

    let Some(pending) = notifications.pending_icons.remove(cache_id) else {
        return;
    };
    let Some(bytes) = decode_notification_icon_chunks(&pending.encoded_chunks) else {
        return;
    };
    let Some(image) = decode_notification_icon(&bytes) else {
        return;
    };
    notifications.cache_icon(
        cache_id.to_owned(),
        CachedNotificationIcon {
            name: icon_name,
            image: Some(image),
        },
    );
}

fn decode_notification_icon_chunks(chunks: &[Vec<u8>]) -> Option<Vec<u8>> {
    let combined = chunks.concat();
    if let Some(decoded) = decode_standard_base64(&combined)
        && decoded.len() <= MAX_OSC_NOTIFICATION_ICON_BYTES
    {
        return Some(decoded);
    }
    let mut decoded = Vec::new();
    for chunk in chunks {
        let bytes = decode_standard_base64(chunk)?;
        if decoded.len().saturating_add(bytes.len()) > MAX_OSC_NOTIFICATION_ICON_BYTES {
            return None;
        }
        decoded.extend_from_slice(&bytes);
    }
    Some(decoded)
}

fn decode_notification_icon(bytes: &[u8]) -> Option<NotificationIcon> {
    if bytes.is_empty() || bytes.len() > MAX_OSC_NOTIFICATION_ICON_BYTES {
        return None;
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    if !matches!(
        reader.format(),
        Some(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::Gif)
    ) {
        return None;
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(512);
    limits.max_image_height = Some(512);
    limits.max_alloc = Some(2 * MAX_OSC_NOTIFICATION_ICON_BYTES as u64);
    reader.limits(limits);
    let pixels = reader.decode().ok()?.into_rgba8();
    Some(NotificationIcon {
        width: pixels.width().try_into().ok()?,
        height: pixels.height().try_into().ok()?,
        rgba: Arc::from(pixels.into_raw()),
    })
}

fn notification_icon_name(value: &str) -> Option<String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+' | b'.'))
    {
        return None;
    }
    Some(
        match value {
            "error" => "dialog-error",
            "warn" | "warning" => "dialog-warning",
            "info" => "dialog-information",
            "question" => "dialog-question",
            "help" => "help-browser",
            "file-manager" => "system-file-manager",
            "system-monitor" => "utilities-system-monitor",
            "text-editor" => "accessories-text-editor",
            value => value,
        }
        .to_owned(),
    )
}

fn osc1337_remote_host(payload: &[u8]) -> Option<String> {
    if payload.is_empty() || payload.len() > MAX_OSC_REMOTE_HOST_BYTES {
        return None;
    }
    let remote_host = std::str::from_utf8(payload).ok()?;
    let (_, host) = remote_host.split_once('@')?;
    if host.is_empty() || remote_host.chars().any(char::is_control) {
        return None;
    }
    Some(remote_host.to_owned())
}

fn osc1337_shell_integration(payload: &[u8]) -> Option<(u32, Option<String>)> {
    let payload = std::str::from_utf8(payload).ok()?;
    if payload.chars().any(char::is_control) {
        return None;
    }
    let (version, shell) = payload
        .split_once(';')
        .map_or((payload, None), |(version, shell)| (version, Some(shell)));
    if version.is_empty() || !version.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let version = version.parse().ok()?;
    let shell = match shell {
        None | Some("") => None,
        Some(shell) if shell.len() <= MAX_OSC_SHELL_NAME_BYTES && !shell.contains(';') => {
            Some(shell.to_owned())
        }
        Some(_) => return None,
    };
    Some((version, shell))
}

fn osc1337_user_var(payload: &[u8]) -> Option<(String, String)> {
    let separator = payload.iter().position(|byte| *byte == b'=')?;
    let name = std::str::from_utf8(&payload[..separator]).ok()?;
    if name.is_empty()
        || name.len() > MAX_OSC_USER_VAR_NAME_BYTES
        || name.chars().any(char::is_control)
    {
        return None;
    }
    let encoded = &payload[separator + 1..];
    if encoded.len() > MAX_OSC_USER_VAR_VALUE_BYTES.saturating_mul(2) {
        return None;
    }
    let value = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    if value.len() > MAX_OSC_USER_VAR_VALUE_BYTES {
        return None;
    }
    Some((name.to_owned(), String::from_utf8(value).ok()?))
}

fn osc1337_report_variable(encoded_name: &[u8]) -> Option<String> {
    if encoded_name.is_empty()
        || encoded_name.len() > MAX_OSC_REPORT_VARIABLE_NAME_BYTES.saturating_mul(2)
    {
        return None;
    }
    let name = decode_standard_base64(encoded_name)?;
    if name.is_empty() || name.len() > MAX_OSC_REPORT_VARIABLE_NAME_BYTES {
        return None;
    }
    let name = String::from_utf8(name).ok()?;
    if name.chars().any(char::is_control) {
        return None;
    }
    Some(name)
}

fn osc1337_badge_format(encoded_format: &[u8]) -> Option<String> {
    if encoded_format.len() > MAX_OSC_BADGE_FORMAT_BYTES.saturating_mul(2) {
        return None;
    }
    let format = decode_standard_base64(encoded_format)?;
    if format.len() > MAX_OSC_BADGE_FORMAT_BYTES {
        return None;
    }
    let format = String::from_utf8(format).ok()?;
    if format.chars().any(char::is_control) {
        return None;
    }
    Some(format)
}

fn valid_notification_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-+.".contains(&byte))
}

fn decode_standard_base64(input: &[u8]) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(input)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(input))
        .ok()
}

fn notification_text(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() > MAX_OSC_NOTIFICATION_BYTES {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    (!text.chars().any(char::is_control)).then(|| text.to_owned())
}

fn osc133_marker(payload: &[u8], marker: u8) -> bool {
    payload.strip_prefix(b"133;").is_some_and(|value| {
        value.first() == Some(&marker) && (value.len() == 1 || value.get(1) == Some(&b';'))
    })
}

fn normalize_terminal_path(mut path: String) -> String {
    let bytes = path.as_bytes();
    if bytes.len() >= 3
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && (bytes[2] == b':' || bytes[2] == b'|')
        && (bytes.len() == 3 || bytes[3] == b'/' || bytes[3] == b'\\')
    {
        path.remove(0);
        if path.starts_with(|c: char| c.is_ascii_alphabetic()) && path.chars().nth(1) == Some('|') {
            path.replace_range(1..2, ":");
        }
    }
    path
}

fn osc7_path(payload: &[u8]) -> Option<String> {
    let payload = payload.strip_prefix(b"file://")?;
    let path_start = payload.iter().position(|byte| *byte == b'/')?;
    let path = &payload[path_start..];
    let mut decoded = Vec::with_capacity(path.len());
    let mut index = 0;
    while index < path.len() {
        if path[index] == b'%'
            && index + 2 < path.len()
            && let (Some(high), Some(low)) =
                (hex_digit(path[index + 1]), hex_digit(path[index + 2]))
        {
            decoded.push(high * 16 + low);
            index += 3;
        } else {
            decoded.push(path[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok().map(normalize_terminal_path)
}

const fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
