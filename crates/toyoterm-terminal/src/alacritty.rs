use super::graphics::{
    Graphics,
    handler::{GraphicsHandler, SemanticMarkerKind, SemanticMarkers},
    stream::{Stream, Token},
};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event as AlacrittyEvent, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Config, MIN_COLUMNS, MIN_SCREEN_LINES, Osc52, TermMode};
use alacritty_terminal::vte::ansi::{
    Color, CursorShape as AlacrittyCursorShape, Handler, NamedColor, Processor, Rgb,
};
use base64::Engine;
use cursor_icon::CursorIcon;

use super::{
    CellAttributes, CellColor, CommandZoneSpan, CursorShape, CursorState, SearchDirection,
    SearchMatchSpan, SearchResult, SelectionKind, SelectionSpan, TerminalBackend, TerminalCell,
    TerminalColors, TerminalMode, TerminalSnapshot,
};

pub const DEFAULT_SCROLLBACK_LINES: usize = 10_000;
pub const MAX_OSC52_COPY_BYTES: usize = 64 * 1024;
pub const MAX_OSC_NOTIFICATION_BYTES: usize = 4 * 1024;
pub const MAX_OSC_ICON_TITLE_BYTES: usize = 1024;
pub const MAX_OSC_REMOTE_HOST_BYTES: usize = 1024;
pub const MAX_OSC_SHELL_NAME_BYTES: usize = 64;
pub const MAX_OSC_REPORT_VARIABLE_NAME_BYTES: usize = 256;
pub const MAX_OSC_BADGE_FORMAT_BYTES: usize = 4 * 1024;
pub const MAX_OSC_SESSION_STATUS_BYTES: usize = 1024;
pub const MAX_OSC_URL_BYTES: usize = 2 * 1024;
pub const MAX_OSC_USER_VAR_NAME_BYTES: usize = 128;
pub const MAX_OSC_USER_VAR_VALUE_BYTES: usize = 4 * 1024;
const MAX_SHELL_INTEGRATION_PAYLOAD_BYTES: usize = 8 * 1024;
const MAX_ITERM_COPY_BASE64_BYTES: usize = MAX_OSC52_COPY_BYTES.div_ceil(3) * 4;
const ITERM_COPY_PREFIX: &[u8] = b"1337;Copy=:";

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
    MouseCursorChanged(CursorIcon),
    MouseCursorControl(String),
    ColorControl(String),
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
    Notification {
        id: Option<String>,
        title: Option<String>,
        body: String,
        occasion: NotificationOccasion,
        urgency: NotificationUrgency,
        timeout_ms: Option<u32>,
        sound: NotificationSound,
        icon_name: Option<String>,
    },
    NotificationClose(String),
    PtyWrite(String),
    Bell,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ItermUiColorRole {
    Link,
    CursorForeground,
    Underline,
    SelectionBackground,
    SelectionForeground,
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
    Error(u8),
    Indeterminate,
    Warning(u8),
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
struct ShellIntegrationParser {
    state: ShellIntegrationState,
    notifications: NotificationAssemblies,
}

#[derive(Default)]
struct NotificationAssemblies {
    pending: HashMap<String, PendingNotification>,
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
}

#[derive(Default)]
pub(crate) struct ClipboardCapture {
    text: String,
    overflowed: bool,
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
struct MouseCursorShape {
    name: &'static str,
    icon: CursorIcon,
}

#[derive(Default)]
pub(crate) struct MouseCursorStacks {
    primary: Vec<MouseCursorShape>,
    alternate: Vec<MouseCursorShape>,
}

impl MouseCursorStacks {
    fn active_mut(&mut self, alternate: bool) -> &mut Vec<MouseCursorShape> {
        if alternate {
            &mut self.alternate
        } else {
            &mut self.primary
        }
    }

    fn current(&self, alternate: bool) -> Option<&MouseCursorShape> {
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
    fn advance(&mut self, bytes: &[u8], allow_osc52_copy: bool) -> Vec<(usize, TerminalEvent)> {
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
                            b"1" | b"6"
                                | b"7"
                                | b"9"
                                | b"17"
                                | b"19"
                                | b"21"
                                | b"22"
                                | b"99"
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
    } else if let Some(control) = payload.strip_prefix(b"6;") {
        parse_osc6_tab_color(control, events);
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
    } else if let Some(control) = payload.strip_prefix(b"21;") {
        if control.len() <= 8_192
            && let Ok(control) = std::str::from_utf8(control)
            && !control.chars().any(char::is_control)
        {
            events.push(TerminalEvent::ColorControl(control.to_owned()));
        }
    } else if let Some(color) = payload.strip_prefix(b"17;") {
        parse_xterm_ui_color(color, ItermUiColorRole::SelectionBackground, 17, events);
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
            });
        }
    } else if let Some(payload) = payload.strip_prefix(b"99;") {
        parse_osc99_notification(payload, events, notifications);
    } else if let Some(payload) = payload.strip_prefix(b"21337;") {
        if let Some(update) = osc21337_session_status(payload) {
            events.push(TerminalEvent::SessionStatusChanged(update));
        }
    } else if let Some(assignment) = payload.strip_prefix(b"1337;SetColors=") {
        parse_osc1337_set_colors(assignment, events);
    } else if let Some(path) = payload.strip_prefix(b"1337;CurrentDir=") {
        if !path.is_empty()
            && !path.contains(&0)
            && let Ok(path) = std::str::from_utf8(path)
        {
            events.push(TerminalEvent::CwdChanged(path.to_owned()));
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
            _ => None,
        };
        if let Some(request) = request {
            events.push(TerminalEvent::AttentionRequested(request));
        }
    } else if let Some(encoded_url) = payload.strip_prefix(b"1337;OpenURL=:") {
        if let Some(bytes) = decode_standard_base64(encoded_url)
            && bytes.len() <= MAX_OSC_URL_BYTES
            && let Ok(url) = String::from_utf8(bytes)
            && !url.is_empty()
            && !url.chars().any(char::is_control)
        {
            events.push(TerminalEvent::OpenUrlRequested(url));
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

fn parse_iterm_color(color: &[u8]) -> Option<[u8; 3]> {
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
    let value = parameters
        .next()
        .and_then(|value| std::str::from_utf8(value).ok())
        .and_then(|value| value.parse::<u8>().ok())
        .filter(|value| *value <= 100);
    if parameters.next().is_some() {
        return None;
    }
    match state {
        b"0" => Some(TerminalProgress::Hidden),
        b"1" => value.map(TerminalProgress::Normal),
        b"2" => value.map(TerminalProgress::Error),
        b"3" => Some(TerminalProgress::Indeterminate),
        b"4" => value.map(TerminalProgress::Warning),
        _ => None,
    }
}

fn bare_osc_event(command: &[u8]) -> Option<TerminalEvent> {
    match command {
        b"117" => Some(TerminalEvent::ItermUiColorReset(
            ItermUiColorRole::SelectionBackground,
        )),
        b"119" => Some(TerminalEvent::ItermUiColorReset(
            ItermUiColorRole::SelectionForeground,
        )),
        b"30001" => Some(TerminalEvent::ColorStackPush),
        b"30101" => Some(TerminalEvent::ColorStackPop),
        _ => None,
    }
}

fn mouse_cursor_shape(name: &str) -> Option<MouseCursorShape> {
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

fn color_control_index(key: &str) -> Option<usize> {
    match key {
        "foreground" => Some(NamedColor::Foreground as usize),
        "bright_foreground" => Some(NamedColor::BrightForeground as usize),
        "background" => Some(NamedColor::Background as usize),
        "cursor" => Some(NamedColor::Cursor as usize),
        _ => key.parse::<u8>().ok().map(usize::from),
    }
}

fn kitty_ui_color_role(key: &str) -> Option<ItermUiColorRole> {
    match key {
        "selection_background" => Some(ItermUiColorRole::SelectionBackground),
        "selection_foreground" => Some(ItermUiColorRole::SelectionForeground),
        "cursor_text" => Some(ItermUiColorRole::CursorForeground),
        _ => None,
    }
}

fn parse_kitty_color(value: &str) -> Option<Rgb> {
    let value = if let Some((color, alpha)) = value.split_once('@') {
        let alpha = alpha.parse::<f64>().ok()?;
        if !alpha.is_finite() || color.contains('@') {
            return None;
        }
        color
    } else {
        value
    };
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
                    b"error" if cfg!(all(unix, not(target_os = "macos"))) => {
                        NotificationSound::Error
                    }
                    b"warn" | b"warning" if cfg!(all(unix, not(target_os = "macos"))) => {
                        NotificationSound::Warning
                    }
                    b"info" if cfg!(all(unix, not(target_os = "macos"))) => NotificationSound::Info,
                    b"question" if cfg!(all(unix, not(target_os = "macos"))) => {
                        NotificationSound::Question
                    }
                    _ => return,
                });
            }
            "n" if cfg!(all(unix, not(target_os = "macos"))) && icon_name.is_none() => {
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
            _ => {}
        }
    }

    if payload_type == "?" {
        let id = id.unwrap_or("0");
        let payload_types = if cfg!(windows) {
            "title,body"
        } else {
            "title,body,close"
        };
        let sounds = if cfg!(all(unix, not(target_os = "macos"))) {
            "system,silent,error,warn,warning,info,question"
        } else {
            "system,silent"
        };
        let expiry = if cfg!(windows) { "" } else { ":w=1" };
        events.push(TerminalEvent::PtyWrite(format!(
            "\x1b]99;i={id}:p=?;o=always,unfocused,invisible:p={payload_types}:s={sounds}:u=0,1,2{expiry}\x1b\\"
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
    if !matches!(payload_type, "title" | "body") {
        return;
    }

    let payload = &payload[separator + 1..];
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
    let target = if payload_type == "body" {
        &mut notification.body
    } else {
        &mut notification.title
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
    events.push(TerminalEvent::Notification {
        id: id.map(str::to_owned),
        title: (!notification.title.is_empty()).then_some(notification.title),
        body: notification.body,
        occasion: notification.occasion,
        urgency: notification.urgency,
        timeout_ms: notification.timeout_ms,
        sound: notification.sound,
        icon_name: notification.icon_name,
    });
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
    String::from_utf8(decoded).ok()
}

const fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

struct TerminalEventSender(Sender<TerminalEvent>);

impl EventListener for TerminalEventSender {
    fn send_event(&self, event: AlacrittyEvent) {
        let event = match event {
            AlacrittyEvent::Title(title) => Some(TerminalEvent::TitleChanged(title)),
            AlacrittyEvent::ResetTitle => Some(TerminalEvent::TitleReset),
            AlacrittyEvent::PtyWrite(text) => Some(TerminalEvent::PtyWrite(text)),
            AlacrittyEvent::Bell => Some(TerminalEvent::Bell),
            _ => None,
        };
        if let Some(event) = event {
            let _ = self.0.send(event);
        }
    }
}

pub struct AlacrittyTerminalBackend {
    graphics: Graphics,
    graphics_stream: Stream,
    processor: Processor,
    terminal: Term<TerminalEventSender>,
    events: Receiver<TerminalEvent>,
    event_sender: Sender<TerminalEvent>,
    default_colors: DefaultColors,
    allow_osc52_copy: bool,
    cell_scale_factor: f64,
    shell_integration: ShellIntegrationParser,
    semantic_markers: SemanticMarkers,
    mouse_cursor_stacks: MouseCursorStacks,
    iterm_ui_colors: ItermUiColors,
    color_stack: Vec<ColorStackEntry>,
    clipboard_capture: Option<ClipboardCapture>,
    pending_events: Vec<TerminalEvent>,
    selection_anchor: Option<Point>,
    search: SearchState,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ItermUiColors {
    link: Option<[u8; 3]>,
    cursor_foreground: Option<[u8; 3]>,
    underline: Option<[u8; 3]>,
    selection_background: Option<[u8; 3]>,
    selection_foreground: Option<[u8; 3]>,
}

struct ColorStackEntry {
    terminal: Vec<Option<alacritty_terminal::vte::ansi::Rgb>>,
    iterm_ui: ItermUiColors,
}

#[derive(Clone, Copy)]
pub(crate) struct DefaultColors {
    foreground: [u8; 3],
    background: [u8; 3],
    cursor: [u8; 3],
    selection: [u8; 3],
    ansi: [[u8; 3]; 16],
}

impl Default for DefaultColors {
    fn default() -> Self {
        Self {
            foreground: [220, 225, 232],
            background: [9, 11, 14],
            cursor: [245, 247, 250],
            selection: [55, 88, 145],
            ansi: [
                [0, 0, 0],
                [205, 0, 0],
                [0, 205, 0],
                [205, 205, 0],
                [0, 0, 238],
                [205, 0, 205],
                [0, 205, 205],
                [229, 229, 229],
                [127, 127, 127],
                [255, 0, 0],
                [0, 255, 0],
                [255, 255, 0],
                [92, 92, 255],
                [255, 0, 255],
                [0, 255, 255],
                [255, 255, 255],
            ],
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct GridMatch {
    line: Line,
    start_column: u16,
    end_column: u16,
}

#[derive(Default)]
struct SearchState {
    query: String,
    matches: Vec<GridMatch>,
    current: Option<usize>,
}

impl AlacrittyTerminalBackend {
    pub fn new(columns: u16, rows: u16) -> Self {
        Self::with_scrollback(columns, rows, DEFAULT_SCROLLBACK_LINES)
    }

    pub fn with_scrollback(columns: u16, rows: u16, scrollback_lines: usize) -> Self {
        let size = TermSize::new(columns, rows);
        let config = terminal_config(scrollback_lines);
        let (event_sender, events) = mpsc::channel();
        Self {
            graphics: Graphics::new(),
            graphics_stream: Stream::default(),
            processor: Processor::new(),
            terminal: Term::new(config, &size, TerminalEventSender(event_sender.clone())),
            event_sender,
            default_colors: DefaultColors::default(),
            allow_osc52_copy: false,
            cell_scale_factor: 1.0,
            events,
            shell_integration: ShellIntegrationParser::default(),
            semantic_markers: SemanticMarkers::default(),
            mouse_cursor_stacks: MouseCursorStacks::default(),
            iterm_ui_colors: ItermUiColors::default(),
            color_stack: Vec::new(),
            clipboard_capture: None,
            pending_events: Vec::new(),
            selection_anchor: None,
            search: SearchState::default(),
        }
    }

    pub fn drain_events(&mut self) -> Vec<TerminalEvent> {
        let mut events = std::mem::take(&mut self.pending_events);
        events.extend(self.events.try_iter());
        events
    }

    fn advance_vt(&mut self, bytes: &[u8]) {
        for token in self.graphics_stream.advance(bytes) {
            match token {
                Token::Cancel => self.graphics.cancel_transfer(),
                Token::Text(bytes) => self.processor.advance(
                    &mut GraphicsHandler {
                        terminal: &mut self.terminal,
                        graphics: &mut self.graphics,
                        semantic_markers: &mut self.semantic_markers,
                        mouse_cursor_stacks: &mut self.mouse_cursor_stacks,
                        clipboard_capture: &mut self.clipboard_capture,
                        output: &self.event_sender,
                        default_colors: &self.default_colors,
                        allow_osc52_copy: self.allow_osc52_copy,
                    },
                    &bytes,
                ),
                Token::Graphic(kind, payload) => {
                    use alacritty_terminal::vte::ansi::Handler;
                    // Flush buffered synchronized text before capturing the image cursor.
                    self.processor.stop_sync(&mut GraphicsHandler {
                        terminal: &mut self.terminal,
                        graphics: &mut self.graphics,
                        semantic_markers: &mut self.semantic_markers,
                        mouse_cursor_stacks: &mut self.mouse_cursor_stacks,
                        clipboard_capture: &mut self.clipboard_capture,
                        output: &self.event_sender,
                        default_colors: &self.default_colors,
                        allow_osc52_copy: self.allow_osc52_copy,
                    });
                    let at = self.terminal.grid().cursor.point;
                    let result = self.graphics.receive(
                        kind,
                        &payload,
                        (at.column.0 as u16, at.line.0),
                        self.dimensions(),
                        self.terminal.mode().contains(TermMode::ALT_SCREEN),
                    );
                    if let Some(reply) = result.reply {
                        let _ = self.event_sender.send(TerminalEvent::PtyWrite(reply));
                    }
                    if let Some((columns, rows)) = result.advance {
                        let mut handler = GraphicsHandler {
                            terminal: &mut self.terminal,
                            graphics: &mut self.graphics,
                            semantic_markers: &mut self.semantic_markers,
                            mouse_cursor_stacks: &mut self.mouse_cursor_stacks,
                            clipboard_capture: &mut self.clipboard_capture,
                            output: &self.event_sender,
                            default_colors: &self.default_colors,
                            allow_osc52_copy: self.allow_osc52_copy,
                        };
                        for _ in 1..rows.min(handler.terminal.screen_lines() as u16) {
                            handler.linefeed();
                        }
                        handler.goto_col(at.column.0.saturating_add(usize::from(columns)));
                    }
                }
                Token::Osc1337(payload) => {
                    if let Some(shape) = payload.strip_prefix(b"1337;CursorShape=")
                        && matches!(shape, b"0" | b"1" | b"2")
                    {
                        let sequence =
                            [b"\x1b]50;CursorShape=".as_slice(), shape, b"\x1b\\"].concat();
                        self.processor.advance(
                            &mut GraphicsHandler {
                                terminal: &mut self.terminal,
                                graphics: &mut self.graphics,
                                semantic_markers: &mut self.semantic_markers,
                                mouse_cursor_stacks: &mut self.mouse_cursor_stacks,
                                clipboard_capture: &mut self.clipboard_capture,
                                output: &self.event_sender,
                                default_colors: &self.default_colors,
                                allow_osc52_copy: self.allow_osc52_copy,
                            },
                            &sequence,
                        );
                    } else if payload == b"1337;ReportCellSize" {
                        let (width, height) = self.graphics.size();
                        let scale = self.cell_scale_factor;
                        let _ = self.event_sender.send(TerminalEvent::PtyWrite(format!(
                            "\x1b]1337;ReportCellSize={:.2};{:.2};{scale:.2}\x1b\\",
                            f64::from(height) / scale,
                            f64::from(width) / scale,
                        )));
                    } else if payload == b"1337;ClearScrollback" {
                        self.processor.advance(
                            &mut GraphicsHandler {
                                terminal: &mut self.terminal,
                                graphics: &mut self.graphics,
                                semantic_markers: &mut self.semantic_markers,
                                mouse_cursor_stacks: &mut self.mouse_cursor_stacks,
                                clipboard_capture: &mut self.clipboard_capture,
                                output: &self.event_sender,
                                default_colors: &self.default_colors,
                                allow_osc52_copy: self.allow_osc52_copy,
                            },
                            b"\x1b[3J",
                        );
                        self.selection_anchor = None;
                        self.search = SearchState::default();
                    }
                }
            }
        }
    }

    fn record_shell_event(&mut self, event: TerminalEvent) {
        if event == TerminalEvent::ClipboardCaptureStart {
            self.clipboard_capture = Some(ClipboardCapture::default());
            return;
        }
        if event == TerminalEvent::ClipboardCaptureEnd {
            if let Some(capture) = self.clipboard_capture.take()
                && !capture.overflowed
            {
                self.pending_events
                    .push(TerminalEvent::ClipboardStore(capture.text));
            }
            return;
        }
        if let TerminalEvent::MouseCursorControl(control) = &event {
            self.apply_mouse_cursor_control(control);
            return;
        }
        if let TerminalEvent::ColorControl(control) = &event {
            self.apply_color_control(control);
            return;
        }
        if let TerminalEvent::ItermUiColorChanged { role, color } = &event {
            self.set_iterm_ui_color(*role, *color);
            return;
        }
        if let TerminalEvent::ItermUiColorReset(role) = &event {
            self.reset_iterm_ui_color(*role);
            return;
        }
        if let TerminalEvent::ItermUiColorQuery { role, osc } = &event {
            let [red, green, blue] = self.xterm_ui_color(*role);
            self.pending_events.push(TerminalEvent::PtyWrite(format!(
                "\x1b]{osc};rgb:{red:02x}{red:02x}/{green:02x}{green:02x}/{blue:02x}{blue:02x}\x1b\\"
            )));
            return;
        }
        if event == TerminalEvent::ColorStackPush {
            self.push_color_stack();
            return;
        }
        if event == TerminalEvent::ColorStackPop {
            self.pop_color_stack();
            return;
        }
        let kind = match &event {
            TerminalEvent::MarkSet => Some(SemanticMarkerKind::Mark),
            TerminalEvent::PromptStarted => Some(SemanticMarkerKind::Prompt),
            TerminalEvent::CommandLineStarted => Some(SemanticMarkerKind::CommandLine),
            TerminalEvent::CommandStarted => Some(SemanticMarkerKind::CommandStart),
            TerminalEvent::CommandFinished(_) => Some(SemanticMarkerKind::CommandEnd),
            _ => None,
        };
        if let Some(kind) = kind {
            self.semantic_markers.mark(
                kind,
                self.terminal.grid().cursor.point.line.0,
                self.terminal.grid().cursor.point.column.0,
                self.terminal.mode().contains(TermMode::ALT_SCREEN),
                match &event {
                    TerminalEvent::CommandFinished(status) => *status,
                    _ => None,
                },
            );
        }
        self.pending_events.push(event);
    }

    fn apply_color_control(&mut self, control: &str) {
        let mut responses = Vec::new();
        for assignment in control
            .split(';')
            .filter(|assignment| !assignment.is_empty())
        {
            let (key, value, reset) = assignment
                .split_once('=')
                .map_or((assignment, "", true), |(key, value)| (key, value, false));
            if let Some(role) = kitty_ui_color_role(key) {
                if value == "?" {
                    let response = self
                        .resolved_iterm_ui_color(role)
                        .map_or_else(String::new, |[red, green, blue]| {
                            format!("rgb:{red:02x}/{green:02x}/{blue:02x}")
                        });
                    responses.push(format!("{key}={response}"));
                } else if reset {
                    self.reset_iterm_ui_color(role);
                } else if !value.is_empty()
                    && let Some(Rgb { r, g, b }) = parse_kitty_color(value)
                {
                    self.set_iterm_ui_color(role, [r, g, b]);
                }
                continue;
            }
            let Some(index) = color_control_index(key) else {
                if value == "?" {
                    responses.push(format!("{key}=?"));
                }
                continue;
            };
            if value == "?" {
                let response = resolved_color(&self.terminal, &self.default_colors, index)
                    .map_or_else(
                        || "?".to_owned(),
                        |color| format!("rgb:{:02x}/{:02x}/{:02x}", color.r, color.g, color.b),
                    );
                responses.push(format!("{key}={response}"));
            } else if value.is_empty() {
                Handler::reset_color(&mut self.terminal, index);
            } else if let Some(color) = parse_kitty_color(value) {
                Handler::set_color(&mut self.terminal, index, color);
            }
        }
        if !responses.is_empty() {
            self.pending_events.push(TerminalEvent::PtyWrite(format!(
                "\x1b]21;{}\x1b\\",
                responses.join(";")
            )));
        }
    }

    fn set_iterm_ui_color(&mut self, role: ItermUiColorRole, color: [u8; 3]) {
        *self.iterm_ui_color_mut(role) = Some(color);
    }

    fn reset_iterm_ui_color(&mut self, role: ItermUiColorRole) {
        *self.iterm_ui_color_mut(role) = None;
    }

    fn iterm_ui_color_mut(&mut self, role: ItermUiColorRole) -> &mut Option<[u8; 3]> {
        match role {
            ItermUiColorRole::Link => &mut self.iterm_ui_colors.link,
            ItermUiColorRole::CursorForeground => &mut self.iterm_ui_colors.cursor_foreground,
            ItermUiColorRole::Underline => &mut self.iterm_ui_colors.underline,
            ItermUiColorRole::SelectionBackground => &mut self.iterm_ui_colors.selection_background,
            ItermUiColorRole::SelectionForeground => &mut self.iterm_ui_colors.selection_foreground,
        }
    }

    fn resolved_iterm_ui_color(&self, role: ItermUiColorRole) -> Option<[u8; 3]> {
        match role {
            ItermUiColorRole::Link => Some(
                self.iterm_ui_colors
                    .link
                    .unwrap_or(self.default_colors.foreground),
            ),
            ItermUiColorRole::CursorForeground => self.iterm_ui_colors.cursor_foreground,
            ItermUiColorRole::Underline => Some(
                self.iterm_ui_colors
                    .underline
                    .unwrap_or(self.default_colors.foreground),
            ),
            ItermUiColorRole::SelectionBackground => Some(
                self.iterm_ui_colors
                    .selection_background
                    .unwrap_or(self.default_colors.selection),
            ),
            ItermUiColorRole::SelectionForeground => self.iterm_ui_colors.selection_foreground,
        }
    }

    fn xterm_ui_color(&self, role: ItermUiColorRole) -> [u8; 3] {
        self.resolved_iterm_ui_color(role).unwrap_or(match role {
            ItermUiColorRole::CursorForeground => self.default_colors.background,
            ItermUiColorRole::SelectionForeground
            | ItermUiColorRole::Link
            | ItermUiColorRole::Underline => self.default_colors.foreground,
            ItermUiColorRole::SelectionBackground => self.default_colors.selection,
        })
    }

    fn push_color_stack(&mut self) {
        if self.color_stack.len() == 10 {
            self.color_stack.remove(0);
        }
        self.color_stack.push(ColorStackEntry {
            terminal: (0..=NamedColor::DimForeground as usize)
                .map(|index| self.terminal.colors()[index])
                .collect(),
            iterm_ui: self.iterm_ui_colors,
        });
    }

    fn pop_color_stack(&mut self) {
        let Some(colors) = self.color_stack.pop() else {
            return;
        };
        for (index, color) in colors.terminal.into_iter().enumerate() {
            match color {
                Some(color) => Handler::set_color(&mut self.terminal, index, color),
                None => Handler::reset_color(&mut self.terminal, index),
            }
        }
        self.iterm_ui_colors = colors.iterm_ui;
    }

    fn apply_mouse_cursor_control(&mut self, control: &str) {
        let alternate = self.terminal.mode().contains(TermMode::ALT_SCREEN);
        if let Some(query) = control.strip_prefix('?') {
            let response = query
                .split(',')
                .map(|name| match name {
                    "__current__" => self
                        .mouse_cursor_stacks
                        .current(alternate)
                        .map_or("0", |shape| shape.name),
                    "__default__" => "default",
                    "__grabbed__" => "grabbing",
                    name if mouse_cursor_shape(name).is_some() => "1",
                    _ => "0",
                })
                .collect::<Vec<_>>()
                .join(",");
            self.pending_events
                .push(TerminalEvent::PtyWrite(format!("\x1b]22;{response}\x1b\\")));
            return;
        }

        let stack = self.mouse_cursor_stacks.active_mut(alternate);
        if control.starts_with('<') {
            stack.pop();
        } else if let Some(names) = control.strip_prefix('>') {
            for shape in names.split(',').filter_map(mouse_cursor_shape) {
                if stack.len() == 32 {
                    stack.remove(0);
                }
                stack.push(shape);
            }
        } else {
            let name = control.strip_prefix('=').unwrap_or(control);
            if name.is_empty() {
                stack.clear();
            } else if let Some(shape) = mouse_cursor_shape(name) {
                stack.clear();
                stack.push(shape);
            } else {
                return;
            }
        }
        self.pending_events.push(TerminalEvent::MouseCursorChanged(
            stack.last().map_or(CursorIcon::Default, |shape| shape.icon),
        ));
    }

    /// Set the fallback colors used for rendering and terminal palette queries.
    pub fn set_default_colors(
        &mut self,
        foreground: [u8; 3],
        background: [u8; 3],
        cursor: [u8; 3],
        selection: [u8; 3],
        ansi: [[u8; 3]; 16],
    ) {
        self.default_colors = DefaultColors {
            foreground,
            background,
            cursor,
            selection,
            ansi,
        };
    }

    pub fn render_colors(&self) -> TerminalColors {
        let resolve = |index| {
            let color = resolved_color(&self.terminal, &self.default_colors, index)
                .expect("renderable terminal color has a configured fallback");
            [color.r, color.g, color.b]
        };
        TerminalColors {
            foreground: resolve(NamedColor::Foreground as usize),
            bold: resolve(NamedColor::BrightForeground as usize),
            background: resolve(NamedColor::Background as usize),
            cursor: resolve(NamedColor::Cursor as usize),
            ansi: std::array::from_fn(resolve),
            link: self.iterm_ui_colors.link,
            cursor_foreground: self.iterm_ui_colors.cursor_foreground,
            underline: self.iterm_ui_colors.underline,
            selection_background: self.iterm_ui_colors.selection_background,
            selection_foreground: self.iterm_ui_colors.selection_foreground,
        }
    }

    /// Permit OSC 52 writes to the host clipboard. Clipboard reads stay disabled.
    pub fn set_osc52_copy_enabled(&mut self, enabled: bool) {
        self.allow_osc52_copy = enabled;
        if !enabled {
            self.clipboard_capture = None;
        }
    }

    /// Physical cell size used by pixel-based image protocols.
    pub fn set_cell_size(&mut self, width: u16, height: u16) {
        let size = (width.max(1), height.max(1));
        if self.graphics.cell_size != size {
            self.graphics.clear(false, i32::MIN, i32::MAX);
            self.graphics.clear(true, i32::MIN, i32::MAX);
            self.graphics.cell_size = size;
        }
    }

    /// Physical-pixel to logical-point scale used by OSC 1337 cell-size reports.
    pub fn set_cell_scale_factor(&mut self, scale_factor: f64) {
        if scale_factor.is_finite() && scale_factor > 0.0 {
            self.cell_scale_factor = scale_factor;
        }
    }

    pub fn set_scrollback_lines(&mut self, scrollback_lines: usize) {
        self.terminal.grid_mut().update_history(scrollback_lines);
    }

    /// Current terminal grid size without constructing a full cell snapshot.
    pub fn dimensions(&self) -> (u16, u16) {
        let grid = self.terminal.grid();
        (grid.columns() as u16, grid.screen_lines() as u16)
    }

    fn viewport_point(&self, column: u16, row: u16) -> Point {
        let grid = self.terminal.grid();
        let column = usize::from(column).min(grid.columns().saturating_sub(1));
        let row = i32::from(row).min(grid.screen_lines().saturating_sub(1) as i32);
        Point::new(Line(row - grid.display_offset() as i32), Column(column))
    }
}

pub(crate) fn resolved_color<E: EventListener>(
    terminal: &Term<E>,
    defaults: &DefaultColors,
    index: usize,
) -> Option<alacritty_terminal::vte::ansi::Rgb> {
    let configured = (index < 269).then(|| terminal.colors()[index]).flatten();
    let [r, g, b] = configured
        .map(|color| [color.r, color.g, color.b])
        .or_else(|| match index {
            0..=15 => Some(defaults.ansi[index]),
            16..=231 => {
                let index = index - 16;
                let component = |value: usize| if value == 0 { 0 } else { value * 40 + 55 };
                Some([
                    component(index / 36) as u8,
                    component(index / 6 % 6) as u8,
                    component(index % 6) as u8,
                ])
            }
            232..=255 => {
                let value = 8 + (index - 232) as u8 * 10;
                Some([value; 3])
            }
            index if index == NamedColor::Foreground as usize => Some(defaults.foreground),
            index if index == NamedColor::BrightForeground as usize => Some(defaults.foreground),
            index if index == NamedColor::Background as usize => Some(defaults.background),
            index if index == NamedColor::Cursor as usize => Some(defaults.cursor),
            _ => None,
        })?;
    Some(alacritty_terminal::vte::ansi::Rgb { r, g, b })
}

fn terminal_config(scrollback_lines: usize) -> Config {
    Config {
        scrolling_history: scrollback_lines,
        // Keep alacritty's bidirectional OSC 52 path disabled. Toyoterm's
        // custom handler permits only bounded, explicitly opted-in writes and
        // never exposes clipboard contents to terminal output.
        osc52: Osc52::Disabled,
        ..Config::default()
    }
}

impl Default for AlacrittyTerminalBackend {
    fn default() -> Self {
        Self::new(80, 24)
    }
}

impl TerminalBackend for AlacrittyTerminalBackend {
    fn advance(&mut self, bytes: &[u8]) {
        let shell_events = self.shell_integration.advance(bytes, self.allow_osc52_copy);
        let mut start = 0;
        for (end, event) in shell_events {
            self.advance_vt(&bytes[start..=end]);
            self.record_shell_event(event);
            start = end + 1;
        }
        self.advance_vt(&bytes[start..]);
        if !self.search.query.is_empty() {
            self.search.matches = terminal_matches(&self.terminal, &self.search.query);
            if self
                .search
                .current
                .is_some_and(|current| current >= self.search.matches.len())
            {
                self.search.current = self.search.matches.len().checked_sub(1);
            }
        }
    }

    fn resize(&mut self, columns: u16, rows: u16) {
        if self.dimensions() != (columns, rows) {
            self.graphics.clear(false, i32::MIN, i32::MAX);
            self.graphics.clear(true, i32::MIN, i32::MAX);
            self.graphics.region = None;
            // Resize can reflow grid rows without exposing an old-to-new mapping.
            self.semantic_markers.reset();
        }
        self.terminal.resize(TermSize::new(columns, rows));
    }

    fn snapshot(&self) -> TerminalSnapshot {
        let grid = self.terminal.grid();
        let (columns, rows) = self.dimensions();
        let display_offset = grid.display_offset() as i32;
        let mut lines = Vec::with_capacity(rows as usize);
        let mut rows_of_cells = Vec::with_capacity(rows as usize);
        let selection_range = self
            .terminal
            .selection
            .as_ref()
            .and_then(|selection| selection.to_range(&self.terminal));
        let mut selection = Vec::new();

        for viewport_row in 0..rows {
            let line = Line(viewport_row as i32 - display_offset);
            // The grid is normally dominated by untouched cells. Find the
            // useful suffix once so we do not allocate a String and a
            // TerminalCell for every trailing blank only to pop them below.
            let rendered_columns = (0..columns)
                .rev()
                .find(|column| {
                    let cell = &grid[line][Column(usize::from(*column))];
                    !cell
                        .flags
                        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                        && !is_default_blank_grid_cell(cell)
                })
                .map_or(0, |column| column + 1);
            let mut text = String::with_capacity(rendered_columns as usize);
            let mut cells = Vec::with_capacity(rendered_columns as usize);
            for column in 0..rendered_columns {
                let cell = &grid[line][Column(column as usize)];
                if cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }
                text.push(cell.c);
                let mut cell_text = cell.c.to_string();
                if let Some(zerowidth) = cell.zerowidth() {
                    text.extend(zerowidth);
                    cell_text.extend(zerowidth);
                }
                cells.push(TerminalCell {
                    column,
                    text: cell_text,
                    width: if cell.flags.contains(Flags::WIDE_CHAR) {
                        2
                    } else {
                        1
                    },
                    attributes: cell_attributes(cell.fg, cell.bg, cell.flags),
                    hyperlink: cell.hyperlink().map(|link| link.uri().to_owned()),
                });
            }
            lines.push(text.trim_end().to_owned());
            rows_of_cells.push(cells);
            if let Some(range) = selection_range {
                let mut selected_columns = (0..columns).filter(|column| {
                    range.contains(Point::new(line, Column(usize::from(*column))))
                });
                if let Some(start_column) = selected_columns.next() {
                    let end_column = selected_columns.next_back().unwrap_or(start_column);
                    selection.push(SelectionSpan {
                        row: viewport_row,
                        start_column,
                        end_column,
                    });
                }
            }
        }

        detect_plain_urls(&lines, &mut rows_of_cells);

        let mut search_matches = Vec::new();
        for (index, found) in self.search.matches.iter().enumerate() {
            let viewport_row = found.line.0 + display_offset;
            if (0..i32::from(rows)).contains(&viewport_row) {
                search_matches.push(SearchMatchSpan {
                    row: viewport_row as u16,
                    start_column: found.start_column,
                    end_column: found.end_column,
                    active: self.search.current == Some(index),
                });
            }
        }

        let command_zones = self
            .semantic_markers
            .command_zones(self.mode().alternate_screen)
            .into_iter()
            .filter_map(|(start, end, exit_status)| {
                let start = start + display_offset;
                let end = end + display_offset;
                if end < 0 || start >= i32::from(rows) {
                    return None;
                }
                Some(CommandZoneSpan {
                    start_row: start.max(0) as u16,
                    end_row: end.min(i32::from(rows) - 1).max(0) as u16,
                    exit_status,
                })
            })
            .collect();

        TerminalSnapshot {
            images: self
                .graphics
                .snapshot(self.mode().alternate_screen, display_offset, rows),
            columns,
            rows,
            lines,
            cells: rows_of_cells,
            selection,
            search_matches,
            command_zones,
        }
    }

    fn visible_text(&self) -> String {
        let grid = self.terminal.grid();
        let (columns, rows) = self.dimensions();
        let display_offset = grid.display_offset() as i32;
        let mut text = String::new();
        for viewport_row in 0..rows {
            if viewport_row != 0 {
                text.push('\n');
            }
            let line = Line(viewport_row as i32 - display_offset);
            let line_start = text.len();
            for column in 0..columns {
                let cell = &grid[line][Column(column as usize)];
                if cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }
                text.push(cell.c);
                if let Some(zerowidth) = cell.zerowidth() {
                    text.extend(zerowidth);
                }
            }
            while text.len() > line_start && text.ends_with(char::is_whitespace) {
                text.pop();
            }
        }
        text
    }

    fn cursor(&self) -> CursorState {
        let content = self.terminal.renderable_content();
        let (_, rows) = self.dimensions();
        let viewport_row = content.cursor.point.line.0 + content.display_offset as i32;
        let shape = match content.cursor.shape {
            AlacrittyCursorShape::Beam => CursorShape::Beam,
            AlacrittyCursorShape::Underline => CursorShape::Underline,
            AlacrittyCursorShape::Block
            | AlacrittyCursorShape::HollowBlock
            | AlacrittyCursorShape::Hidden => CursorShape::Block,
        };
        CursorState {
            column: content.cursor.point.column.0 as u16,
            row: viewport_row.clamp(0, u16::MAX as i32) as u16,
            visible: content.cursor.shape != AlacrittyCursorShape::Hidden
                && viewport_row >= 0
                && viewport_row < rows as i32,
            shape,
        }
    }

    fn mode(&self) -> TerminalMode {
        let mode = *self.terminal.mode();
        TerminalMode {
            application_cursor: mode.contains(TermMode::APP_CURSOR),
            application_keypad: mode.contains(TermMode::APP_KEYPAD),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            mouse_reporting: mode.intersects(TermMode::MOUSE_MODE),
            sgr_mouse: mode.contains(TermMode::SGR_MOUSE),
            focus_reporting: mode.contains(TermMode::FOCUS_IN_OUT),
            alternate_screen: mode.contains(TermMode::ALT_SCREEN),
            alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
        }
    }

    fn scroll_display(&mut self, lines: i32) {
        self.terminal.scroll_display(Scroll::Delta(lines));
    }

    fn scroll_to_bottom(&mut self) {
        self.terminal.scroll_display(Scroll::Bottom);
    }

    fn start_selection(&mut self, column: u16, row: u16, kind: SelectionKind) {
        let point = self.viewport_point(column, row);
        self.selection_anchor = Some(point);
        let selection_type = match kind {
            SelectionKind::Simple => SelectionType::Simple,
            SelectionKind::Word => SelectionType::Semantic,
            SelectionKind::Line => SelectionType::Lines,
        };
        let mut selection = Selection::new(selection_type, point, Side::Left);
        if selection_type == SelectionType::Simple {
            selection.update(point, Side::Right);
        }
        self.terminal.selection = Some(selection);
    }

    fn update_selection(&mut self, column: u16, row: u16) {
        let point = self.viewport_point(column, row);
        let Some(selection) = self.terminal.selection.as_ref() else {
            return;
        };
        if selection.ty == SelectionType::Simple
            && let Some(anchor) = self.selection_anchor
        {
            let (anchor_side, point_side) = if point < anchor {
                (Side::Right, Side::Left)
            } else {
                (Side::Left, Side::Right)
            };
            let mut selection = Selection::new(SelectionType::Simple, anchor, anchor_side);
            selection.update(point, point_side);
            self.terminal.selection = Some(selection);
        } else if let Some(selection) = self.terminal.selection.as_mut() {
            selection.update(point, Side::Right);
        }
    }

    fn clear_selection(&mut self) {
        self.terminal.selection = None;
        self.selection_anchor = None;
    }

    fn selected_text(&self) -> Option<String> {
        self.terminal.selection_to_string()
    }

    fn search(&mut self, query: &str, direction: SearchDirection) -> SearchResult {
        if query.is_empty() {
            self.clear_search();
            return SearchResult::default();
        }

        if self.search.query != query {
            self.search.query = query.to_owned();
            self.search.matches = terminal_matches(&self.terminal, query);
            self.search.current = None;
        }
        let total = self.search.matches.len();
        if total == 0 {
            self.search.current = None;
            return SearchResult::default();
        }
        let current = match (self.search.current, direction) {
            (None, SearchDirection::Next) => 0,
            (None, SearchDirection::Previous) => total - 1,
            (Some(index), SearchDirection::Next) => (index + 1) % total,
            (Some(index), SearchDirection::Previous) => (index + total - 1) % total,
        };
        self.search.current = Some(current);
        reveal_line(&mut self.terminal, self.search.matches[current].line);
        SearchResult {
            current: current + 1,
            total,
        }
    }

    fn clear_search(&mut self) {
        self.search = SearchState::default();
    }

    fn navigate_prompt(&mut self, direction: SearchDirection) -> bool {
        let alternate = self.terminal.mode().contains(TermMode::ALT_SCREEN);
        let Some((line, _, _)) = self.semantic_markers.navigate_prompt(direction, alternate) else {
            return false;
        };
        reveal_line(&mut self.terminal, Line(line));
        true
    }

    fn navigate_mark(&mut self, direction: SearchDirection) -> bool {
        let alternate = self.terminal.mode().contains(TermMode::ALT_SCREEN);
        let Some((line, _, _)) = self.semantic_markers.navigate_mark(direction, alternate) else {
            return false;
        };
        reveal_line(&mut self.terminal, Line(line));
        true
    }

    fn select_last_command_output(&mut self) -> bool {
        let alternate = self.terminal.mode().contains(TermMode::ALT_SCREEN);
        let range = self.semantic_markers.last_command_range(alternate);
        self.select_command_range(range)
    }

    fn select_command_output(&mut self, direction: SearchDirection) -> bool {
        let alternate = self.terminal.mode().contains(TermMode::ALT_SCREEN);
        let range = self
            .semantic_markers
            .navigate_command_output(direction, alternate);
        self.select_command_range(range)
    }
}

impl AlacrittyTerminalBackend {
    fn select_command_range(&mut self, range: Option<((i32, usize), (i32, usize))>) -> bool {
        let Some(((start_line, start_column), (end_line, end_column))) = range else {
            return false;
        };
        let columns = self.terminal.columns();
        let start = Point::new(Line(start_line), Column(start_column.min(columns - 1)));
        let end = Point::new(Line(end_line), Column(end_column.min(columns - 1)));
        if start >= end {
            return false;
        }
        let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
        selection.update(end, Side::Left);
        self.selection_anchor = Some(start);
        self.terminal.selection = Some(selection);
        reveal_line(&mut self.terminal, start.line);
        true
    }
}

fn is_default_blank_grid_cell(cell: &Cell) -> bool {
    cell.c == ' '
        && cell.zerowidth().is_none_or(<[char]>::is_empty)
        && cell_attributes(cell.fg, cell.bg, cell.flags) == CellAttributes::default()
        && cell.hyperlink().is_none()
}

fn terminal_matches<T: EventListener>(terminal: &Term<T>, query: &str) -> Vec<GridMatch> {
    let grid = terminal.grid();
    let start = -(grid.history_size() as i32);
    let end = grid.screen_lines() as i32;
    let mut matches = Vec::new();
    for line_number in start..end {
        let line = Line(line_number);
        let mut text = String::new();
        let mut byte_cells = Vec::new();
        for column in 0..grid.columns() {
            let cell = &grid[line][Column(column)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let start = text.len();
            text.push(cell.c);
            if let Some(zerowidth) = cell.zerowidth() {
                text.extend(zerowidth);
            }
            byte_cells.push((
                start,
                text.len(),
                column as u16,
                cell.flags.contains(Flags::WIDE_CHAR),
            ));
        }
        for (start_byte, _) in text.match_indices(query) {
            let end_byte = start_byte + query.len();
            let Some(start_cell) = byte_cells.iter().find(|(_, end, _, _)| *end > start_byte)
            else {
                continue;
            };
            let Some(end_cell) = byte_cells
                .iter()
                .rev()
                .find(|(start, _, _, _)| *start < end_byte)
            else {
                continue;
            };
            matches.push(GridMatch {
                line,
                start_column: start_cell.2,
                end_column: end_cell.2 + u16::from(end_cell.3),
            });
        }
    }
    matches
}

fn reveal_line<T: EventListener>(terminal: &mut Term<T>, line: Line) {
    let grid = terminal.grid();
    let offset = grid.display_offset() as i32;
    let rows = grid.screen_lines() as i32;
    let desired = if line.0 < -offset {
        -line.0
    } else if line.0 >= rows - offset {
        (rows - 1 - line.0).max(0)
    } else {
        offset
    };
    terminal.scroll_display(Scroll::Delta(desired - offset));
}

fn detect_plain_urls(lines: &[String], cells: &mut [Vec<TerminalCell>]) {
    const PREFIXES: [&str; 3] = ["https://", "http://", "mailto:"];
    for (line, cells) in lines.iter().zip(cells) {
        let mut from = 0;
        while from < line.len() {
            let Some((start, _)) = PREFIXES
                .iter()
                .filter_map(|prefix| line[from..].find(prefix).map(|at| (from + at, *prefix)))
                .min_by_key(|(at, _)| *at)
            else {
                break;
            };
            let raw_end = line[start..]
                .find(char::is_whitespace)
                .map_or(line.len(), |length| start + length);
            let url = line[start..raw_end]
                .trim_end_matches(['.', ',', ';', '!', '?', ')', ']', '}', '\'', '"']);
            let end = start + url.len();
            if end > start {
                let mut byte = 0;
                for cell in cells.iter_mut() {
                    let cell_start = byte;
                    let cell_end = byte + cell.text.len();
                    if cell_start < end && cell_end > start && cell.hyperlink.is_none() {
                        cell.hyperlink = Some(url.to_owned());
                    }
                    byte = cell_end;
                }
            }
            from = raw_end.max(start + 1);
        }
    }
}

fn cell_attributes(foreground: Color, background: Color, flags: Flags) -> CellAttributes {
    CellAttributes {
        foreground: cell_color(foreground, false),
        background: cell_color(background, true),
        bold: flags.contains(Flags::BOLD),
        italic: flags.contains(Flags::ITALIC),
        underline: flags.intersects(Flags::ALL_UNDERLINES),
        strikethrough: flags.contains(Flags::STRIKEOUT),
        dim: flags.contains(Flags::DIM),
        inverse: flags.contains(Flags::INVERSE),
        hidden: flags.contains(Flags::HIDDEN),
    }
}

fn cell_color(color: Color, background: bool) -> CellColor {
    match color {
        Color::Spec(rgb) => CellColor::Rgb(rgb.r, rgb.g, rgb.b),
        Color::Indexed(index) => CellColor::Indexed(index),
        Color::Named(NamedColor::Black) | Color::Named(NamedColor::DimBlack) => {
            CellColor::Indexed(0)
        }
        Color::Named(NamedColor::Red) | Color::Named(NamedColor::DimRed) => CellColor::Indexed(1),
        Color::Named(NamedColor::Green) | Color::Named(NamedColor::DimGreen) => {
            CellColor::Indexed(2)
        }
        Color::Named(NamedColor::Yellow) | Color::Named(NamedColor::DimYellow) => {
            CellColor::Indexed(3)
        }
        Color::Named(NamedColor::Blue) | Color::Named(NamedColor::DimBlue) => CellColor::Indexed(4),
        Color::Named(NamedColor::Magenta) | Color::Named(NamedColor::DimMagenta) => {
            CellColor::Indexed(5)
        }
        Color::Named(NamedColor::Cyan) | Color::Named(NamedColor::DimCyan) => CellColor::Indexed(6),
        Color::Named(NamedColor::White) | Color::Named(NamedColor::DimWhite) => {
            CellColor::Indexed(7)
        }
        Color::Named(named)
            if (NamedColor::BrightBlack..=NamedColor::BrightWhite).contains(&named) =>
        {
            CellColor::Indexed(named as u8)
        }
        Color::Named(NamedColor::Foreground)
        | Color::Named(NamedColor::Background)
        | Color::Named(NamedColor::Cursor)
        | Color::Named(NamedColor::BrightForeground)
        | Color::Named(NamedColor::DimForeground) => CellColor::Default,
        Color::Named(_) if background => CellColor::Default,
        Color::Named(_) => CellColor::Default,
    }
}

#[derive(Clone, Copy, Debug)]
struct TermSize {
    columns: usize,
    rows: usize,
}

impl TermSize {
    fn new(columns: u16, rows: u16) -> Self {
        Self {
            columns: usize::from(columns).max(MIN_COLUMNS),
            rows: usize::from(rows).max(MIN_SCREEN_LINES),
        }
    }
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_and_ansi_cursor_movement() {
        let mut backend = AlacrittyTerminalBackend::new(10, 3);
        backend.advance(b"hello\rX");
        assert_eq!(backend.snapshot().lines[0], "Xello");
        assert_eq!(
            backend.cursor(),
            CursorState {
                column: 1,
                row: 0,
                visible: true,
                shape: CursorShape::Block,
            }
        );
    }

    #[test]
    fn preserves_utf8_across_input_chunks() {
        let mut backend = AlacrittyTerminalBackend::new(10, 2);
        let bytes = "日本語".as_bytes();
        backend.advance(&bytes[..2]);
        backend.advance(&bytes[2..5]);
        backend.advance(&bytes[5..]);
        assert_eq!(backend.snapshot().lines[0], "日本語");
    }

    #[test]
    fn tracks_wide_combining_cjk_and_emoji_cell_widths() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance("A界e\u{301}😀".as_bytes());

        let snapshot = backend.snapshot();
        assert_eq!(snapshot.lines[0], "A界e\u{301}😀");
        assert_eq!(
            snapshot.cells[0]
                .iter()
                .map(|cell| (cell.column, cell.text.as_str(), cell.width))
                .collect::<Vec<_>>(),
            [(0, "A", 1), (1, "界", 2), (3, "e\u{301}", 1), (4, "😀", 2),]
        );
        assert_eq!(backend.cursor().column, 6);
    }

    #[test]
    fn exposes_sgr_colors_and_text_attributes_in_snapshot_cells() {
        let mut backend = AlacrittyTerminalBackend::new(10, 2);
        backend.advance(b"\x1b[1;2;3;4;7;9;38;5;196;48;2;1;2;3mX");

        let cell = &backend.snapshot().cells[0][0];
        assert_eq!(cell.text, "X");
        assert_eq!(cell.attributes.foreground, CellColor::Indexed(196));
        assert_eq!(cell.attributes.background, CellColor::Rgb(1, 2, 3));
        assert!(cell.attributes.bold);
        assert!(cell.attributes.dim);
        assert!(cell.attributes.italic);
        assert!(cell.attributes.underline);
        assert!(cell.attributes.inverse);
        assert!(cell.attributes.strikethrough);
    }

    #[test]
    fn exposes_osc8_links_and_detects_plain_urls() {
        let mut backend = AlacrittyTerminalBackend::new(60, 2);
        backend.advance(
            b"\x1b]8;;https://example.com/docs\x1b\\manual\x1b]8;;\x1b\\ http://localhost:3000/test).",
        );

        let snapshot = backend.snapshot();
        assert_eq!(
            snapshot.cells[0][0].hyperlink.as_deref(),
            Some("https://example.com/docs")
        );
        let plain = snapshot.cells[0]
            .iter()
            .find(|cell| cell.text == "l" && cell.column > 10)
            .expect("plain URL cell");
        assert_eq!(
            plain.hyperlink.as_deref(),
            Some("http://localhost:3000/test")
        );
    }

    #[test]
    fn answers_osc_palette_and_dynamic_color_queries() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        let mut ansi = [[0, 0, 0]; 16];
        ansi[3] = [12, 34, 56];
        backend.set_default_colors(
            [0x11, 0x22, 0x33],
            [0x44, 0x55, 0x66],
            [0x77, 0x88, 0x99],
            [0xaa, 0xbb, 0xcc],
            ansi,
        );

        backend.advance(b"\x1b]4;3;?\x07\x1b]10;?\x1b\\\x1b]11;?\x07\x1b]12;?\x1b\\");

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::PtyWrite("\x1b]4;3;rgb:0c0c/2222/3838\x07".into()),
                TerminalEvent::PtyWrite("\x1b]10;rgb:1111/2222/3333\x1b\\".into()),
                TerminalEvent::PtyWrite("\x1b]11;rgb:4444/5555/6666\x07".into()),
                TerminalEvent::PtyWrite("\x1b]12;rgb:7777/8888/9999\x1b\\".into()),
            ]
        );
    }

    #[test]
    fn supports_xterm_selection_color_controls() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        let ansi = [[0, 0, 0]; 16];
        backend.set_default_colors(
            [0x11, 0x22, 0x33],
            [0x44, 0x55, 0x66],
            [0x77, 0x88, 0x99],
            [0xaa, 0xbb, 0xcc],
            ansi,
        );

        backend.advance(b"\x1b]17;#123\x07\x1b]19;rgb:40/50/60\x1b\\");
        assert_eq!(
            backend.render_colors().selection_background,
            Some([0x10, 0x20, 0x30])
        );
        assert_eq!(
            backend.render_colors().selection_foreground,
            Some([0x40, 0x50, 0x60])
        );
        backend.advance(b"\x1b]17;?\x07\x1b]19;?\x1b\\\x1b]117\x07\x1b]119\x1b\\");
        backend.advance(b"\x1b]17;?\x1b\\\x1b]19;?\x07");

        assert_eq!(backend.render_colors().selection_background, None);
        assert_eq!(backend.render_colors().selection_foreground, None);
        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::PtyWrite("\x1b]17;rgb:1010/2020/3030\x1b\\".into()),
                TerminalEvent::PtyWrite("\x1b]19;rgb:4040/5050/6060\x1b\\".into()),
                TerminalEvent::PtyWrite("\x1b]17;rgb:aaaa/bbbb/cccc\x1b\\".into()),
                TerminalEvent::PtyWrite("\x1b]19;rgb:1111/2222/3333\x1b\\".into()),
            ]
        );
    }

    #[test]
    fn supports_osc21_color_sets_queries_and_resets() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        let mut ansi = [[0, 0, 0]; 16];
        ansi[1] = [1, 2, 3];
        backend.set_default_colors([4, 5, 6], [7, 8, 9], [10, 11, 12], [13, 14, 15], ansi);

        backend.advance(
            b"\x1b]21;1=#abc;foreground=rgb:f/0/8;cursor=rgbi:0.5/1/-1;unknown=?;1=?;foreground=?;cursor=?\x1b\\",
        );
        backend.advance(b"\x1b]21;1;foreground;1=?;foreground=?\x07");

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::PtyWrite(
                    "\x1b]21;unknown=?;1=rgb:a0/b0/c0;foreground=rgb:ff/00/88;cursor=rgb:80/ff/00\x1b\\"
                        .into(),
                ),
                TerminalEvent::PtyWrite(
                    "\x1b]21;1=rgb:01/02/03;foreground=rgb:04/05/06\x1b\\".into(),
                ),
            ]
        );
    }

    #[test]
    fn supports_explicit_kitty_selection_colors_and_resets() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        let ansi = [[0, 0, 0]; 16];
        backend.set_default_colors([1, 2, 3], [4, 5, 6], [7, 8, 9], [10, 11, 12], ansi);

        backend.advance(
            b"\x1b]21;selection_background=#abc;selection_foreground=rgb:1/2/3;cursor_text=#456;selection_background=?;selection_foreground=?;cursor_text=?\x1b\\",
        );
        backend.advance(b"\x1b]21;selection_background=;selection_background=?\x07");
        backend.advance(b"\x1b]21;selection_background;selection_foreground;cursor_text;selection_background=?;selection_foreground=?;cursor_text=?\x1b\\");

        assert_eq!(backend.render_colors().selection_background, None);
        assert_eq!(backend.render_colors().selection_foreground, None);
        assert_eq!(backend.render_colors().cursor_foreground, None);
        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::PtyWrite(
                    "\x1b]21;selection_background=rgb:a0/b0/c0;selection_foreground=rgb:11/22/33;cursor_text=rgb:40/50/60\x1b\\"
                        .into()
                ),
                TerminalEvent::PtyWrite(
                    "\x1b]21;selection_background=rgb:a0/b0/c0\x1b\\".into()
                ),
                TerminalEvent::PtyWrite(
                    "\x1b]21;selection_background=rgb:0a/0b/0c;selection_foreground=;cursor_text=\x1b\\"
                        .into()
                ),
            ]
        );
    }

    #[test]
    fn supports_bounded_kitty_color_stack_with_bel_and_st() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]21;1=red\x1b\\\x1b]30001\x07\x1b]21;1=blue\x1b\\");
        backend.advance(b"\x1b]30001\x1b\\\x1b]21;1=green\x1b\\\x1b]30101\x07\x1b]21;1=?\x1b\\");
        backend.advance(b"\x1b]30101\x1b\\\x1b]21;1=?\x1b\\");

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::PtyWrite("\x1b]21;1=rgb:00/00/ff\x1b\\".into()),
                TerminalEvent::PtyWrite("\x1b]21;1=rgb:ff/00/00\x1b\\".into()),
            ]
        );

        for _ in 0..12 {
            backend.advance(b"\x1b]30001\x1b\\");
        }
        assert_eq!(backend.color_stack.len(), 10);
    }

    #[test]
    fn kitty_color_stack_preserves_iterm_session_colors() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(
            b"\x1b]1337;SetColors=bold=123456\x07\x1b]1337;SetColors=link=234567\x07\x1b]1337;SetColors=curfg=314159\x07\x1b]1337;SetColors=underline=271828\x07\x1b]1337;SetColors=selbg=345678\x07\x1b]1337;SetColors=selfg=456789\x07\x1b]30001\x07",
        );
        backend.advance(
            b"\x1b]1337;SetColors=bold=abcdef\x07\x1b]1337;SetColors=link=abcdef\x07\x1b]1337;SetColors=curfg=abcdef\x07\x1b]1337;SetColors=underline=abcdef\x07\x1b]1337;SetColors=selbg=abcdef\x07\x1b]1337;SetColors=selfg=abcdef\x07\x1b]30101\x07",
        );

        let colors = backend.render_colors();
        assert_eq!(colors.bold, [0x12, 0x34, 0x56]);
        assert_eq!(colors.link, Some([0x23, 0x45, 0x67]));
        assert_eq!(colors.cursor_foreground, Some([0x31, 0x41, 0x59]));
        assert_eq!(colors.underline, Some([0x27, 0x18, 0x28]));
        assert_eq!(colors.selection_background, Some([0x34, 0x56, 0x78]));
        assert_eq!(colors.selection_foreground, Some([0x45, 0x67, 0x89]));
    }

    #[test]
    fn ignores_malformed_and_oversized_osc21_controls() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]21;1=not-a-color;foreground=rgb:ff/00\x07");
        let oversized = "x".repeat(8_193);
        backend.advance(format!("\x1b]21;{oversized}\x07").as_bytes());

        assert!(backend.drain_events().is_empty());
    }

    #[test]
    fn emits_osc22_mouse_cursor_changes() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]22;pointer\x07\x1b]22;text\x1b\\\x1b]22;invalid-shape\x07");

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::MouseCursorChanged(CursorIcon::Pointer),
                TerminalEvent::MouseCursorChanged(CursorIcon::Text),
            ]
        );
    }

    #[test]
    fn supports_osc22_cursor_stacks_and_queries() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(
            b"\x1b]22;pointer\x1b\\\x1b]22;>wait,text\x1b\\\x1b]22;?__current__\x1b\\\x1b]22;<\x1b\\\x1b]22;?pointer,zoom-in,nope\x1b\\",
        );

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::MouseCursorChanged(CursorIcon::Pointer),
                TerminalEvent::MouseCursorChanged(CursorIcon::Text),
                TerminalEvent::PtyWrite("\x1b]22;text\x1b\\".into()),
                TerminalEvent::MouseCursorChanged(CursorIcon::Wait),
                TerminalEvent::PtyWrite("\x1b]22;1,1,0\x1b\\".into()),
            ]
        );
    }

    #[test]
    fn keeps_osc22_cursor_stacks_separate_per_screen() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]22;pointer\x1b\\\x1b[?1049h\x1b]22;wait\x1b\\\x1b[?1049l");

        let events = backend.drain_events();
        assert_eq!(
            events.last(),
            Some(&TerminalEvent::MouseCursorChanged(CursorIcon::Pointer))
        );
    }

    #[test]
    fn supports_iterm_text_cursor_shape_extension() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]1337;CursorShape=1\x07");
        assert_eq!(backend.cursor().shape, CursorShape::Beam);

        backend.advance(b"\x1b]1337;CursorShape=2\x1b\\");
        assert_eq!(backend.cursor().shape, CursorShape::Underline);

        backend.advance(b"\x1b]1337;CursorShape=0\x07");
        assert_eq!(backend.cursor().shape, CursorShape::Block);
    }

    #[test]
    fn osc_palette_queries_report_overrides_and_resets() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        let mut ansi = [[0, 0, 0]; 16];
        ansi[1] = [1, 2, 3];
        backend.set_default_colors([4, 5, 6], [7, 8, 9], [10, 11, 12], [13, 14, 15], ansi);

        backend.advance(b"\x1b]4;1;#abcdef\x07\x1b]4;1;?\x07\x1b]104;1\x07\x1b]4;1;?\x07");

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::PtyWrite("\x1b]4;1;rgb:abab/cdcd/efef\x07".into()),
                TerminalEvent::PtyWrite("\x1b]4;1;rgb:0101/0202/0303\x07".into()),
            ]
        );
    }

    #[test]
    fn literal_search_navigates_scrollback_and_marks_visible_matches() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"needle one\r\nother\r\nneedle two");

        assert_eq!(
            backend.search("needle", SearchDirection::Next),
            SearchResult {
                current: 1,
                total: 2
            }
        );
        let snapshot = backend.snapshot();
        assert_eq!(snapshot.lines[0], "needle one");
        assert!(
            snapshot
                .search_matches
                .iter()
                .any(|found| found.active && found.row == 0)
        );

        assert_eq!(
            backend.search("needle", SearchDirection::Next),
            SearchResult {
                current: 2,
                total: 2
            }
        );
        assert_eq!(backend.snapshot().lines[1], "needle two");
        assert_eq!(
            backend.search("needle", SearchDirection::Previous),
            SearchResult {
                current: 1,
                total: 2
            }
        );
        backend.clear_search();
        assert!(backend.snapshot().search_matches.is_empty());
    }

    #[test]
    fn visible_text_tracks_the_current_viewport_without_cell_metadata() {
        let mut backend = AlacrittyTerminalBackend::new(12, 2);
        backend.advance("one\r\n日本語\r\nthree".as_bytes());

        assert_eq!(backend.visible_text(), "日本語\nthree");
        backend.scroll_display(1);
        assert_eq!(backend.visible_text(), "one\n日本語");
        assert_eq!(backend.visible_text(), backend.snapshot().lines.join("\n"));
    }

    #[test]
    fn exposes_true_color_and_reset_attributes() {
        let mut backend = AlacrittyTerminalBackend::new(10, 2);
        backend.advance(b"\x1b[38;2;12;34;56mA\x1b[0mB");

        let snapshot = backend.snapshot();
        assert_eq!(
            snapshot.cells[0][0].attributes.foreground,
            CellColor::Rgb(12, 34, 56)
        );
        assert_eq!(snapshot.cells[0][1].attributes, CellAttributes::default());
    }

    #[test]
    fn tracks_terminal_modes_and_cursor_visibility() {
        let mut backend = AlacrittyTerminalBackend::new(10, 2);
        backend.advance(b"\x1b[?1h\x1b[?2004h\x1b[?1000h\x1b[?25l");
        assert_eq!(
            backend.mode(),
            TerminalMode {
                application_cursor: true,
                bracketed_paste: true,
                mouse_reporting: true,
                alternate_scroll: true,
                ..TerminalMode::default()
            }
        );
        assert!(!backend.cursor().visible);
    }

    #[test]
    fn tracks_keypad_focus_and_alternate_screen_modes() {
        let mut backend = AlacrittyTerminalBackend::new(10, 2);
        backend.advance(b"\x1b=\x1b[?1004h\x1b[?1049h");
        let mode = backend.mode();
        assert!(mode.application_keypad);
        assert!(mode.focus_reporting);
        assert!(mode.alternate_screen);
        assert!(mode.alternate_scroll);

        backend.advance(b"\x1b>\x1b[?1004l\x1b[?1049l");
        let mode = backend.mode();
        assert!(!mode.application_keypad);
        assert!(!mode.focus_reporting);
        assert!(!mode.alternate_screen);
    }

    #[test]
    fn handles_dec_autowrap_origin_and_bracketed_paste_modes() {
        let mut backend = AlacrittyTerminalBackend::new(5, 4);
        backend.advance(b"\x1b[?7labcdeX");
        assert_eq!(backend.snapshot().lines[0], "abcdX");

        backend.advance(b"\x1b[2J\x1b[2;3r\x1b[?6h\x1b[H");
        assert_eq!(backend.cursor().row, 1);

        backend.advance(b"\x1b[?2004h");
        assert!(backend.mode().bracketed_paste);
        backend.advance(b"\x1b[?2004l");
        assert!(!backend.mode().bracketed_paste);
    }

    #[test]
    fn resize_updates_snapshot_dimensions() {
        let mut backend = AlacrittyTerminalBackend::new(80, 24);
        backend.resize(120, 40);
        let snapshot = backend.snapshot();
        assert_eq!((snapshot.columns, snapshot.rows), (120, 40));
        assert_eq!(snapshot.lines.len(), 40);
    }

    #[test]
    fn disables_osc52_clipboard_access_without_disrupting_terminal_output() {
        assert_eq!(
            terminal_config(DEFAULT_SCROLLBACK_LINES).osc52,
            Osc52::Disabled
        );

        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"before\x1b]52;c;dG95b3Rlcm0=\x07after");

        assert_eq!(backend.snapshot().lines[0], "beforeafter");
        assert!(backend.drain_events().is_empty());
    }

    #[test]
    fn permits_bounded_osc52_copies_without_clipboard_reads() {
        use base64::Engine as _;

        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.set_osc52_copy_enabled(true);
        backend.advance(b"\x1b]52;c;dG95b3Rlcm0=\x07\x1b]52;c;?\x1b\\");

        assert_eq!(
            backend.drain_events(),
            vec![TerminalEvent::ClipboardStore("toyoterm".into())]
        );

        let oversized =
            base64::engine::general_purpose::STANDARD.encode(vec![b'x'; MAX_OSC52_COPY_BYTES + 1]);
        backend.advance(format!("\x1b]52;c;{oversized}\x07").as_bytes());
        assert!(backend.drain_events().is_empty());
    }

    #[test]
    fn parses_bounded_legacy_osc_notifications() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend
            .advance(b"\x1b]9;build finished\x07\x1b]777;notify;Deploy;Production is ready\x1b\\");
        backend.advance(b"\x1b]9;4;1;50\x07\x1b]9;bad\nmessage\x07");

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::Notification {
                    id: None,
                    title: None,
                    body: "build finished".into(),
                    occasion: NotificationOccasion::Always,
                    urgency: NotificationUrgency::Normal,
                    timeout_ms: None,
                    sound: NotificationSound::System,
                    icon_name: None,
                },
                TerminalEvent::Notification {
                    id: None,
                    title: Some("Deploy".into()),
                    body: "Production is ready".into(),
                    occasion: NotificationOccasion::Always,
                    urgency: NotificationUrgency::Normal,
                    timeout_ms: None,
                    sound: NotificationSound::System,
                    icon_name: None,
                },
                TerminalEvent::ProgressChanged(TerminalProgress::Normal(50)),
            ]
        );

        let oversized = "x".repeat(MAX_OSC_NOTIFICATION_BYTES + 1);
        backend.advance(format!("\x1b]9;{oversized}\x07").as_bytes());
        assert!(backend.drain_events().is_empty());
    }

    #[test]
    fn parses_osc9_progress_states_and_rejects_invalid_values() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(
            b"\x1b]9;4;1;42\x07\x1b]9;4;2;100\x1b\\\x1b]9;4;3\x07\x1b]9;4;4;7\x1b\\\x1b]9;4\x07",
        );
        backend.advance(b"\x1b]9;4;1;101\x07\x1b]9;4;1;50;extra\x07\x1b]9;4;bogus\x07");

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::ProgressChanged(TerminalProgress::Normal(42)),
                TerminalEvent::ProgressChanged(TerminalProgress::Error(100)),
                TerminalEvent::ProgressChanged(TerminalProgress::Indeterminate),
                TerminalEvent::ProgressChanged(TerminalProgress::Warning(7)),
                TerminalEvent::ProgressChanged(TerminalProgress::Hidden),
            ]
        );
    }

    #[test]
    fn parses_iterm_osc6_tab_colors_and_reset() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(
            b"\x1b]6;1;bg;red;brightness;255\x07\x1b]6;1;bg;green;brightness;32\x1b\\\x1b]6;1;bg;blue;brightness;128\x07",
        );
        backend.advance(b"\x1b]6;1;bg;red;brightness;256\x07\x1b]6;other\x07");
        backend.advance(b"\x1b]6;1;bg;*;default\x1b\\");

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::TabColorChanged {
                    component: TabColorComponent::Red,
                    value: 255,
                },
                TerminalEvent::TabColorChanged {
                    component: TabColorComponent::Green,
                    value: 32,
                },
                TerminalEvent::TabColorChanged {
                    component: TabColorComponent::Blue,
                    value: 128,
                },
                TerminalEvent::TabColorReset,
                TerminalEvent::TitleReset,
            ]
        );
    }

    #[test]
    fn parses_iterm_osc21337_session_status_updates_and_clears() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]21337;indicator=#12aBcF;status=deploying");
        backend.advance(b";status-color=#ff8800\x1b\\");
        backend
            .advance(b"\x1b]21337;status=ready\x07\x1b]21337;indicator=;status=;status-color=\x07");

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::SessionStatusChanged(SessionStatusUpdate {
                    indicator: Some(Some([0x12, 0xab, 0xcf])),
                    status: Some(Some("deploying".into())),
                    status_color: Some(Some([0xff, 0x88, 0x00])),
                }),
                TerminalEvent::SessionStatusChanged(SessionStatusUpdate {
                    status: Some(Some("ready".into())),
                    ..SessionStatusUpdate::default()
                }),
                TerminalEvent::SessionStatusChanged(SessionStatusUpdate {
                    indicator: Some(None),
                    status: Some(None),
                    status_color: Some(None),
                }),
            ]
        );
    }

    #[test]
    fn rejects_invalid_iterm_osc21337_session_status_updates() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]21337;indicator=red;status=ignored\x07");
        backend.advance(b"\x1b]21337;status=line\nbreak\x07");
        backend.advance(b"\x1b]21337;status-color=#12345g\x07");
        backend.advance(b"\x1b]21337;unknown=value\x07");
        let oversized = "x".repeat(MAX_OSC_SESSION_STATUS_BYTES + 1);
        backend.advance(format!("\x1b]21337;status={oversized}\x07").as_bytes());

        assert!(backend.drain_events().is_empty());
    }

    #[test]
    fn parses_iterm_osc1337_tab_colors_and_reset() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend
            .advance(b"\x1b]1337;SetColors=tab=f0a\x07\x1b]1337;SetColors=tab=srgb:102030\x1b\\");
        backend.advance(b"\x1b]1337;SetColors=tab=p3:ffffff\x07\x1b]1337;SetColors=tab=xyz\x07");
        backend.advance(b"\x1b]1337;SetColors=tab=default\x1b\\");

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::TabColorSet([255, 0, 170]),
                TerminalEvent::TabColorSet([16, 32, 48]),
                TerminalEvent::TabColorSet([255, 255, 255]),
                TerminalEvent::TabColorReset,
            ]
        );
    }

    #[test]
    fn converts_iterm_display_p3_colors_to_srgb() {
        assert_eq!(parse_iterm_color(b"p3:808080"), Some([128, 128, 128]));
        assert_eq!(parse_iterm_color(b"p3:ff8000"), Some([255, 119, 0]));
        assert_eq!(parse_iterm_color(b"p3:fg0"), None);
    }

    #[test]
    fn applies_iterm_osc1337_terminal_and_ansi_colors() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(
            b"\x1b]1337;SetColors=fg=f0a\x07\x1b]1337;SetColors=bg=srgb:102030\x1b\\\x1b]1337;SetColors=curbg=rgb:abcdef\x07",
        );
        backend
            .advance(b"\x1b]1337;SetColors=red=123\x07\x1b]1337;SetColors=br_white=ffffff\x1b\\");
        backend.advance(b"\x1b]1337;SetColors=bold=ffffff\x07\x1b]1337;SetColors=fg=invalid\x07");
        backend.advance(
            b"\x1b]1337;SetColors=link=112233\x07\x1b]1337;SetColors=curfg=223344\x07\x1b]1337;SetColors=underline=334455\x07\x1b]1337;SetColors=selbg=445566\x07\x1b]1337;SetColors=selfg=778899\x07",
        );
        let colors = backend.render_colors();
        assert_eq!(colors.foreground, [0xff, 0x00, 0xaa]);
        assert_eq!(colors.bold, [0xff, 0xff, 0xff]);
        assert_eq!(colors.background, [0x10, 0x20, 0x30]);
        assert_eq!(colors.cursor, [0xab, 0xcd, 0xef]);
        assert_eq!(colors.ansi[1], [0x11, 0x22, 0x33]);
        assert_eq!(colors.ansi[15], [0xff, 0xff, 0xff]);
        assert_eq!(colors.link, Some([0x11, 0x22, 0x33]));
        assert_eq!(colors.cursor_foreground, Some([0x22, 0x33, 0x44]));
        assert_eq!(colors.underline, Some([0x33, 0x44, 0x55]));
        assert_eq!(colors.selection_background, Some([0x44, 0x55, 0x66]));
        assert_eq!(colors.selection_foreground, Some([0x77, 0x88, 0x99]));
        backend.advance(b"\x1b]21;foreground=?;background=?;cursor=?;1=?;15=?\x1b\\");

        assert_eq!(
            backend.drain_events(),
            vec![TerminalEvent::PtyWrite(
                "\x1b]21;foreground=rgb:ff/00/aa;background=rgb:10/20/30;cursor=rgb:ab/cd/ef;1=rgb:11/22/33;15=rgb:ff/ff/ff\x1b\\"
                    .into()
            )]
        );
    }

    #[test]
    fn parses_osc99_simple_chunked_and_encoded_notifications() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]99;;Simple title\x1b\\");
        backend.advance(b"\x1b]99;i=job:d=0;Build \x1b\\");
        backend.advance(b"\x1b]99;i=job:p=title:d=0;succeeded\x1b\\");
        backend.advance(b"\x1b]99;i=job:p=body:e=1;QWxsIHRlc3RzIHBhc3NlZA\x1b\\");

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::Notification {
                    id: None,
                    title: Some("Simple title".into()),
                    body: String::new(),
                    occasion: NotificationOccasion::Always,
                    urgency: NotificationUrgency::Normal,
                    timeout_ms: None,
                    sound: NotificationSound::System,
                    icon_name: None,
                },
                TerminalEvent::Notification {
                    id: Some("job".into()),
                    title: Some("Build succeeded".into()),
                    body: "All tests passed".into(),
                    occasion: NotificationOccasion::Always,
                    urgency: NotificationUrgency::Normal,
                    timeout_ms: None,
                    sound: NotificationSound::System,
                    icon_name: None,
                },
            ]
        );
    }

    #[test]
    fn preserves_osc99_occasion_urgency_expiry_and_silence_across_chunks() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]99;i=job-1:d=0:o=invisible:u=2:w=5000;Deploy\x1b\\");
        backend.advance(b"\x1b]99;i=job-1:p=body:s=c2lsZW50;Production is ready\x1b\\");
        backend.advance(b"\x1b]99;longkey=value;ignored\x1b\\");
        backend.advance(b"\x1b]99;s=not-base64!;ignored\x1b\\");

        assert_eq!(
            backend.drain_events(),
            vec![TerminalEvent::Notification {
                id: Some("job-1".into()),
                title: Some("Deploy".into()),
                body: "Production is ready".into(),
                occasion: NotificationOccasion::Invisible,
                urgency: NotificationUrgency::Critical,
                timeout_ms: Some(5_000),
                sound: NotificationSound::Silent,
                icon_name: None,
            }]
        );
    }

    #[test]
    fn accepts_only_platform_supported_osc99_named_sounds() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]99;s=ZXJyb3I=;Build failed\x1b\\");

        let events = backend.drain_events();
        if cfg!(all(unix, not(target_os = "macos"))) {
            assert_eq!(
                events,
                vec![TerminalEvent::Notification {
                    id: None,
                    title: Some("Build failed".into()),
                    body: String::new(),
                    occasion: NotificationOccasion::Always,
                    urgency: NotificationUrgency::Normal,
                    timeout_ms: None,
                    sound: NotificationSound::Error,
                    icon_name: None,
                }]
            );
        } else {
            assert!(events.is_empty());
        }
    }

    #[test]
    fn maps_safe_osc99_named_icons_only_on_linux() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]99;n=ZXJyb3I=;Failure\x1b\\");

        assert_eq!(
            backend.drain_events(),
            vec![TerminalEvent::Notification {
                id: None,
                title: Some("Failure".into()),
                body: String::new(),
                occasion: NotificationOccasion::Always,
                urgency: NotificationUrgency::Normal,
                timeout_ms: None,
                sound: NotificationSound::System,
                icon_name: cfg!(all(unix, not(target_os = "macos"))).then(|| "dialog-error".into()),
            }]
        );
    }

    #[test]
    fn answers_osc99_capability_queries_and_rejects_unsafe_payloads() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]99;i=probe:p=?;\x1b\\");
        backend.advance(b"\x1b]99;i=bad id;ignored\x1b\\");
        backend.advance(b"\x1b]99;i=bad/id;ignored\x1b\\");
        backend.advance(b"\x1b]99;e=1;%%%\x1b\\");

        let payload_types = if cfg!(windows) {
            "title,body"
        } else {
            "title,body,close"
        };
        let sounds = if cfg!(all(unix, not(target_os = "macos"))) {
            "system,silent,error,warn,warning,info,question"
        } else {
            "system,silent"
        };
        let expiry = if cfg!(windows) { "" } else { ":w=1" };
        assert_eq!(
            backend.drain_events(),
            vec![TerminalEvent::PtyWrite(format!(
                "\x1b]99;i=probe:p=?;o=always,unfocused,invisible:p={payload_types}:s={sounds}:u=0,1,2{expiry}\x1b\\"
            ))]
        );
    }

    #[test]
    fn parses_osc99_explicit_close_requests_with_valid_ids() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]99;i=job:p=title:d=0;Build\x1b\\");
        backend.advance(b"\x1b]99;i=job:p=close;\x1b\\");
        backend.advance(b"\x1b]99;p=close;\x1b\\");

        assert_eq!(
            backend.drain_events(),
            vec![TerminalEvent::NotificationClose("job".into())]
        );
    }

    #[test]
    fn enforces_alacritty_minimum_dimensions() {
        let backend = AlacrittyTerminalBackend::new(0, 0);
        let snapshot = backend.snapshot();
        assert_eq!(
            (snapshot.columns, snapshot.rows),
            (MIN_COLUMNS as u16, MIN_SCREEN_LINES as u16)
        );
    }

    #[test]
    fn scrolls_through_saved_history() {
        let mut backend = AlacrittyTerminalBackend::new(10, 2);
        backend.advance(b"one\r\ntwo\r\nthree");
        assert_eq!(backend.snapshot().lines, ["two", "three"]);

        backend.scroll_display(1);
        assert_eq!(backend.snapshot().lines, ["one", "two"]);
        backend.scroll_display(-1);
        assert_eq!(backend.snapshot().lines, ["two", "three"]);
    }

    #[test]
    fn iterm_clear_scrollback_preserves_viewport_and_discards_history() {
        let mut backend = AlacrittyTerminalBackend::new(10, 2);
        backend.advance(b"\x1b]1337;SetMark\x07");
        backend.advance(b"one\r\ntwo\r\nthree");
        assert!(backend.navigate_mark(SearchDirection::Previous));
        assert_eq!(backend.snapshot().lines, ["one", "two"]);

        backend.advance(b"\x1b]1337;ClearScrollback\x1b\\");
        backend.scroll_display(i32::MAX);

        assert_eq!(backend.snapshot().lines, ["two", "three"]);
        assert!(!backend.navigate_mark(SearchDirection::Previous));
    }

    #[test]
    fn scrolls_directly_to_the_bottom_of_saved_history() {
        let mut backend = AlacrittyTerminalBackend::new(10, 2);
        backend.advance(b"one\r\ntwo\r\nthree");
        backend.scroll_display(1);

        backend.scroll_to_bottom();

        assert_eq!(backend.snapshot().lines, ["two", "three"]);
    }

    #[test]
    fn selects_words_from_the_scrollback_viewport() {
        let mut backend = AlacrittyTerminalBackend::new(12, 2);
        backend.advance(b"first line\r\nsecond line\r\nthird line");
        backend.scroll_display(1);

        backend.start_selection(2, 0, SelectionKind::Word);

        assert_eq!(backend.selected_text().as_deref(), Some("first"));
        assert_eq!(backend.snapshot().selection[0].row, 0);
    }

    #[test]
    fn selects_text_on_the_alternate_screen_and_restores_primary_screen() {
        let mut backend = AlacrittyTerminalBackend::new(12, 2);
        backend.advance(b"primary");
        backend.advance(b"\x1b[?1049h\x1b[Halternate");

        backend.start_selection(2, 0, SelectionKind::Word);
        assert_eq!(backend.selected_text().as_deref(), Some("alternate"));

        backend.advance(b"\x1b[?1049l");
        assert_eq!(backend.snapshot().lines[0], "primary");
    }

    #[test]
    fn supports_a_configurable_scrollback_limit() {
        let mut backend = AlacrittyTerminalBackend::with_scrollback(10, 2, 1);
        backend.advance(b"one\r\ntwo\r\nthree\r\nfour");
        backend.scroll_display(i32::MAX);
        assert_eq!(backend.snapshot().lines, ["two", "three"]);
    }

    #[test]
    fn sustained_output_does_not_grow_history_past_the_scrollback_limit() {
        const SCROLLBACK: usize = 32;
        let mut backend = AlacrittyTerminalBackend::with_scrollback(20, 4, SCROLLBACK);
        for line in 0..20_000 {
            backend.advance(format!("line {line}\r\n").as_bytes());
        }

        assert_eq!(backend.terminal.grid().history_size(), SCROLLBACK);
    }

    #[test]
    fn updates_the_scrollback_limit_without_replacing_the_terminal() {
        let mut backend = AlacrittyTerminalBackend::with_scrollback(10, 2, 10);
        backend.advance(b"one\r\ntwo\r\nthree\r\nfour");

        backend.set_scrollback_lines(1);
        backend.scroll_display(i32::MAX);

        assert_eq!(backend.snapshot().lines, ["two", "three"]);
    }

    #[test]
    fn selects_visible_cells_and_extracts_text() {
        let mut backend = AlacrittyTerminalBackend::new(10, 2);
        backend.advance(b"hello world");
        backend.start_selection(1, 0, SelectionKind::Simple);
        backend.update_selection(3, 0);

        assert_eq!(backend.selected_text().as_deref(), Some("ell"));
        assert_eq!(
            backend.snapshot().selection,
            [SelectionSpan {
                row: 0,
                start_column: 1,
                end_column: 3,
            }]
        );
        backend.clear_selection();
        assert_eq!(backend.selected_text(), None);
    }

    #[test]
    fn selection_drag_includes_both_endpoints_in_either_direction() {
        let mut backend = AlacrittyTerminalBackend::new(10, 2);
        backend.advance(b"abcdefghij");

        backend.start_selection(2, 0, SelectionKind::Simple);
        backend.update_selection(5, 0);
        assert_eq!(backend.selected_text().as_deref(), Some("cdef"));

        backend.start_selection(5, 0, SelectionKind::Simple);
        backend.update_selection(2, 0);
        assert_eq!(backend.selected_text().as_deref(), Some("cdef"));

        backend.start_selection(4, 0, SelectionKind::Simple);
        backend.update_selection(4, 0);
        assert_eq!(backend.selected_text().as_deref(), Some("e"));
    }

    #[test]
    fn simple_selection_marks_the_anchor_and_spans_multiple_rows_exactly() {
        let mut backend = AlacrittyTerminalBackend::new(8, 3);
        backend.advance(b"abcdef\r\nghijkl\r\nmnopqr");

        backend.start_selection(2, 0, SelectionKind::Simple);
        assert_eq!(
            backend.snapshot().selection,
            [SelectionSpan {
                row: 0,
                start_column: 2,
                end_column: 2,
            }]
        );

        backend.update_selection(3, 2);
        assert_eq!(
            backend.snapshot().selection,
            [
                SelectionSpan {
                    row: 0,
                    start_column: 2,
                    end_column: 7,
                },
                SelectionSpan {
                    row: 1,
                    start_column: 0,
                    end_column: 7,
                },
                SelectionSpan {
                    row: 2,
                    start_column: 0,
                    end_column: 3,
                },
            ]
        );

        backend.start_selection(3, 2, SelectionKind::Simple);
        backend.update_selection(2, 0);
        assert_eq!(
            backend.snapshot().selection,
            [
                SelectionSpan {
                    row: 0,
                    start_column: 2,
                    end_column: 7,
                },
                SelectionSpan {
                    row: 1,
                    start_column: 0,
                    end_column: 7,
                },
                SelectionSpan {
                    row: 2,
                    start_column: 0,
                    end_column: 3,
                },
            ]
        );
    }

    #[test]
    fn selects_words_and_lines() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"alpha beta\r\nsecond line");

        backend.start_selection(7, 0, SelectionKind::Word);
        assert_eq!(backend.selected_text().as_deref(), Some("beta"));

        backend.start_selection(3, 1, SelectionKind::Line);
        assert_eq!(backend.selected_text().as_deref(), Some("second line\n"));
    }

    #[test]
    fn exposes_title_cwd_command_and_bell_terminal_events() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]0;build server\x07");
        backend.advance(b"\x1b]7;file://localhost/srv/my%20app");
        backend.advance(
            b"\x1b\\\x1b]133;A\x1b\\\x1b]133;B;aid=7\x07\x1b]133;C\x1b\\\x1b]133;D;17\x07\x07",
        );

        let events = backend.drain_events();
        assert!(events.contains(&TerminalEvent::TitleChanged("build server".into())));
        assert!(events.contains(&TerminalEvent::CwdChanged("/srv/my app".into())));
        assert!(events.contains(&TerminalEvent::PromptStarted));
        assert!(events.contains(&TerminalEvent::CommandLineStarted));
        assert!(events.contains(&TerminalEvent::CommandStarted));
        assert!(events.contains(&TerminalEvent::CommandFinished(Some(17))));
        assert!(events.contains(&TerminalEvent::Bell));
        assert!(backend.drain_events().is_empty());
    }

    #[test]
    fn exposes_bounded_icon_title_metadata() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]1;build icon\x1b\\");
        backend.advance(b"\x1b]1;bad\ntitle\x07");
        let oversized = format!("\x1b]1;{}\x07", "x".repeat(MAX_OSC_ICON_TITLE_BYTES + 1));
        backend.advance(oversized.as_bytes());

        assert_eq!(
            backend.drain_events(),
            vec![TerminalEvent::IconTitleChanged("build icon".into())]
        );
    }

    #[test]
    fn reports_cursor_position_to_the_pty() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"hello\x1b[6n");

        assert_eq!(
            backend.drain_events(),
            vec![TerminalEvent::PtyWrite("\x1b[1;6R".into())]
        );
    }

    #[test]
    fn accepts_command_end_without_status_and_ignores_invalid_status() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]133;D\x07\x1b]133;D;invalid\x1b\\\x1b]133;DX\x07");

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::CommandFinished(None),
                TerminalEvent::CommandFinished(None),
            ]
        );
    }

    #[test]
    fn rejects_malformed_osc133_prompt_markers() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]133;AX\x07\x1b]133;BX\x1b\\");

        assert!(backend.drain_events().is_empty());
    }

    #[test]
    fn accepts_bounded_iterm_current_directory_reports() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]1337;CurrentDir=/srv/my project\x1b\\");
        backend.advance(b"\x1b]1337;CurrentDir=\x07");

        assert_eq!(
            backend.drain_events(),
            vec![TerminalEvent::CwdChanged("/srv/my project".into())]
        );
    }

    #[test]
    fn accepts_bounded_iterm_remote_host_reports() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]1337;RemoteHost=alice@build.example.com\x1b\\");
        backend.advance(b"\x1b]1337;RemoteHost=@host-only.example.com\x07");

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::RemoteHostChanged("alice@build.example.com".into()),
                TerminalEvent::RemoteHostChanged("@host-only.example.com".into()),
            ]
        );
    }

    #[test]
    fn parses_bounded_iterm_shell_integration_versions() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]1337;ShellIntegrationVersion=12;fish\x1b\\");
        backend.advance(b"\x1b]1337;ShellIntegrationVersion=7\x07");
        backend.advance(b"\x1b]1337;ShellIntegrationVersion=bad;zsh\x07");
        backend.advance(b"\x1b]1337;ShellIntegrationVersion=3;bad;shell\x07");
        let oversized = format!(
            "\x1b]1337;ShellIntegrationVersion=3;{}\x07",
            "s".repeat(MAX_OSC_SHELL_NAME_BYTES + 1)
        );
        backend.advance(oversized.as_bytes());

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::ShellIntegrationChanged {
                    version: 12,
                    shell: Some("fish".into()),
                },
                TerminalEvent::ShellIntegrationChanged {
                    version: 7,
                    shell: None,
                },
            ]
        );
    }

    #[test]
    fn parses_bounded_iterm_variable_queries() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]1337;ReportVariable=c2Vzc2lvbi5uYW1l\x1b\\");
        backend.advance(b"\x1b]1337;ReportVariable=c2Vzc2lvbi51c2VyLmdpdEJyYW5jaA==\x07");

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::ItermVariableQuery("session.name".into()),
                TerminalEvent::ItermVariableQuery("session.user.gitBranch".into()),
            ]
        );
    }

    #[test]
    fn parses_fragmented_iterm_variable_queries() {
        let sequence = b"\x1b]1337;ReportVariable=c2Vzc2lvbi5wYXRo\x1b\\";
        for split in 0..=sequence.len() {
            let mut backend = AlacrittyTerminalBackend::new(20, 2);
            backend.advance(&sequence[..split]);
            backend.advance(&sequence[split..]);
            assert_eq!(
                backend.drain_events(),
                vec![TerminalEvent::ItermVariableQuery("session.path".into())],
                "split at {split}"
            );
        }
    }

    #[test]
    fn rejects_malformed_iterm_variable_queries() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]1337;ReportVariable=\x07");
        backend.advance(b"\x1b]1337;ReportVariable=not-base64!\x07");
        backend.advance(b"\x1b]1337;ReportVariable=bGluZQpicmVhaw==\x07");
        let oversized = base64::engine::general_purpose::STANDARD.encode(vec![
            b'x';
            MAX_OSC_REPORT_VARIABLE_NAME_BYTES
                + 1
        ]);
        backend.advance(format!("\x1b]1337;ReportVariable={oversized}\x07").as_bytes());

        assert!(backend.drain_events().is_empty());
    }

    #[test]
    fn parses_bounded_iterm_badge_formats() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        let format = base64::engine::general_purpose::STANDARD
            .encode(r"build \(session.name) \(user.gitBranch)");
        backend.advance(format!("\x1b]1337;SetBadgeFormat={format}\x1b\\").as_bytes());
        backend.advance(b"\x1b]1337;SetBadgeFormat=\x07");

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::ItermBadgeFormatChanged(
                    r"build \(session.name) \(user.gitBranch)".into()
                ),
                TerminalEvent::ItermBadgeFormatChanged(String::new()),
            ]
        );
    }

    #[test]
    fn rejects_malformed_iterm_badge_formats() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]1337;SetBadgeFormat=not-base64!\x07");
        backend.advance(b"\x1b]1337;SetBadgeFormat=bGluZQpicmVhaw==\x07");
        let oversized =
            base64::engine::general_purpose::STANDARD
                .encode(vec![b'x'; MAX_OSC_BADGE_FORMAT_BYTES + 1]);
        backend.advance(format!("\x1b]1337;SetBadgeFormat={oversized}\x07").as_bytes());

        assert!(backend.drain_events().is_empty());
    }

    #[test]
    fn parses_iterm_cursor_line_highlight_toggles() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]1337;HighlightCursorLine=yes\x1b\\");
        backend.advance(b"\x1b]1337;HighlightCursorLine=no\x07");
        backend.advance(b"\x1b]1337;HighlightCursorLine=true\x07");

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::CursorLineHighlightChanged(true),
                TerminalEvent::CursorLineHighlightChanged(false),
            ]
        );
    }

    #[test]
    fn parses_iterm_attention_requests_without_accepting_fireworks() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]1337;RequestAttention=yes\x1b\\");
        backend.advance(b"\x1b]1337;RequestAttention=once\x07");
        backend.advance(b"\x1b]1337;RequestAttention=no\x07");
        backend.advance(b"\x1b]1337;RequestAttention=fireworks\x07");

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::AttentionRequested(TerminalAttention::Indefinite),
                TerminalEvent::AttentionRequested(TerminalAttention::Once),
                TerminalEvent::AttentionRequested(TerminalAttention::Cancel),
            ]
        );
    }

    #[test]
    fn parses_bounded_iterm_open_url_requests() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        let url = base64::engine::general_purpose::STANDARD.encode("https://example.com/docs");
        backend.advance(format!("\x1b]1337;OpenURL=:{url}\x1b\\").as_bytes());
        backend.advance(b"\x1b]1337;OpenURL=:not-base64!\x07");
        let control = base64::engine::general_purpose::STANDARD.encode("https://example.com\ncmd");
        backend.advance(format!("\x1b]1337;OpenURL=:{control}\x07").as_bytes());
        let oversized =
            base64::engine::general_purpose::STANDARD.encode(vec![b'x'; MAX_OSC_URL_BYTES + 1]);
        backend.advance(format!("\x1b]1337;OpenURL=:{oversized}\x07").as_bytes());

        assert_eq!(
            backend.drain_events(),
            vec![TerminalEvent::OpenUrlRequested(
                "https://example.com/docs".into()
            )]
        );
    }

    #[test]
    fn permits_bounded_iterm_clipboard_copies_only_when_enabled() {
        let first = base64::engine::general_purpose::STANDARD.encode("first");
        let second = base64::engine::general_purpose::STANDARD_NO_PAD.encode("second");
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(format!("\x1b]1337;Copy=:{first}\x1b\\").as_bytes());
        assert!(backend.drain_events().is_empty());

        backend.set_osc52_copy_enabled(true);
        backend.advance(format!("\x1b]1337;Copy=:{first}\x1b\\").as_bytes());
        backend.advance(format!("\x1b]1337;Copy=:{second}\x07").as_bytes());
        backend.advance(b"\x1b]1337;Copy=:not-base64!\x07");

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::ClipboardStore("first".into()),
                TerminalEvent::ClipboardStore("second".into()),
            ]
        );

        let oversized =
            base64::engine::general_purpose::STANDARD.encode(vec![b'x'; MAX_OSC52_COPY_BYTES + 1]);
        backend.advance(format!("\x1b]1337;Copy=:{oversized}\x07").as_bytes());
        assert!(backend.drain_events().is_empty());
    }

    #[test]
    fn captures_bounded_iterm_clipboard_text_across_fragments() {
        let sequence = b"\x1b]1337;CopyToClipboard=\x07plain \x1b[31mred\x1b[0m\r\n\twide \xf0\x9f\x99\x82\x1b]1337;EndCopy\x1b\\";
        for split in 0..=sequence.len() {
            let mut backend = AlacrittyTerminalBackend::new(20, 2);
            backend.set_osc52_copy_enabled(true);
            backend.advance(&sequence[..split]);
            backend.advance(&sequence[split..]);
            assert_eq!(
                backend.drain_events(),
                vec![TerminalEvent::ClipboardStore(
                    "plain red\r\n\twide 🙂".into()
                )],
                "split at {split}"
            );
        }
    }

    #[test]
    fn discards_oversized_or_cancelled_iterm_clipboard_captures() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.set_osc52_copy_enabled(true);
        backend.advance(b"\x1b]1337;CopyToClipboard=find\x07ignored\x1b]1337;EndCopy\x07");
        assert!(backend.drain_events().is_empty());

        backend.advance(b"\x1b]1337;CopyToClipboard=\x07");
        backend.advance(&vec![b'x'; MAX_OSC52_COPY_BYTES + 1]);
        backend.advance(b"\x1b]1337;EndCopy\x07");
        assert!(backend.drain_events().is_empty());

        backend.advance(b"\x1b]1337;CopyToClipboard=\x07partial");
        backend.set_osc52_copy_enabled(false);
        backend.advance(b"\x1b]1337;EndCopy\x07");
        assert!(backend.drain_events().is_empty());
    }

    #[test]
    fn rejects_malformed_iterm_remote_host_reports() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]1337;RemoteHost=no-separator\x07");
        backend.advance(b"\x1b]1337;RemoteHost=alice@\x07");
        backend.advance(b"\x1b]1337;RemoteHost=alice@host\nname\x07");
        let oversized = format!("\x1b]1337;RemoteHost=user@{}\x07", "h".repeat(1024));
        backend.advance(oversized.as_bytes());

        assert!(backend.drain_events().is_empty());
    }

    #[test]
    fn parses_bounded_iterm_user_variables() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]1337;SetUserVar=gitBranch=bWFpbg==\x1b\\");
        backend.advance(b"\x1b]1337;SetUserVar=empty=\x07");

        assert_eq!(
            backend.drain_events(),
            vec![
                TerminalEvent::UserVarChanged {
                    name: "gitBranch".into(),
                    value: "main".into(),
                },
                TerminalEvent::UserVarChanged {
                    name: "empty".into(),
                    value: String::new(),
                },
            ]
        );
    }

    #[test]
    fn rejects_malformed_iterm_user_variables() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"\x1b]1337;SetUserVar==bWFpbg==\x07");
        backend.advance(b"\x1b]1337;SetUserVar=bad\nname=bWFpbg==\x07");
        backend.advance(b"\x1b]1337;SetUserVar=name=not-base64!\x07");
        let oversized = base64::engine::general_purpose::STANDARD.encode(vec![b'x'; 4097]);
        backend.advance(format!("\x1b]1337;SetUserVar=name={oversized}\x07").as_bytes());

        assert!(backend.drain_events().is_empty());
    }

    #[test]
    fn navigates_osc133_prompts_in_scrollback_order() {
        let mut backend = AlacrittyTerminalBackend::with_scrollback(20, 3, 20);
        backend.advance(
            b"\x1b]133;A\x1b\\prompt-one\r\nout-1\r\nout-2\r\n\x1b]133;A\x07prompt-two\r\ntail-1\r\ntail-2\r\ntail-3",
        );

        assert!(backend.navigate_prompt(SearchDirection::Previous));
        assert!(backend.visible_text().contains("prompt-two"));
        assert!(backend.navigate_prompt(SearchDirection::Previous));
        assert!(backend.visible_text().contains("prompt-one"));
        assert!(backend.navigate_prompt(SearchDirection::Next));
        assert!(backend.visible_text().contains("prompt-two"));

        backend.resize(21, 3);
        assert!(!backend.navigate_prompt(SearchDirection::Previous));
    }

    #[test]
    fn navigates_iterm_set_marks_in_scrollback_order() {
        let mut backend = AlacrittyTerminalBackend::with_scrollback(20, 3, 20);
        backend.advance(
            b"\x1b]1337;SetMark\x1b\\mark-one\r\nout-1\r\nout-2\r\n\x1b]1337;SetMark\x07mark-two\r\ntail-1\r\ntail-2\r\ntail-3",
        );
        backend.advance(b"\x1b]1337;SetMark=invalid\x07");

        assert!(backend.navigate_mark(SearchDirection::Previous));
        assert!(backend.visible_text().contains("mark-two"));
        assert!(backend.navigate_mark(SearchDirection::Previous));
        assert!(backend.visible_text().contains("mark-one"));
        assert!(backend.navigate_mark(SearchDirection::Next));
        assert!(backend.visible_text().contains("mark-two"));

        backend.resize(21, 3);
        assert!(!backend.navigate_mark(SearchDirection::Previous));
    }

    #[test]
    fn selects_the_last_completed_osc133_command_output() {
        let mut backend = AlacrittyTerminalBackend::with_scrollback(20, 3, 20);
        backend.advance(
            b"older\r\n\x1b]133;C\x1b\\output-a\r\noutput-b\r\n\x1b]133;D;0\x07\x1b]133;A\x07prompt",
        );

        assert!(backend.select_last_command_output());
        let selected = backend.selected_text().unwrap();
        assert!(selected.contains("output-a"), "{selected:?}");
        assert!(selected.contains("output-b"), "{selected:?}");
        assert!(!selected.contains("older"), "{selected:?}");
        assert!(!selected.contains("prompt"), "{selected:?}");
    }

    #[test]
    fn exposes_completed_osc133_command_zones_with_exit_status() {
        let mut backend = AlacrittyTerminalBackend::new(20, 5);
        backend.advance(b"\x1b]133;C\x1b\\output-a\r\noutput-b\x1b]133;D;7\x07");

        assert_eq!(
            backend.snapshot().command_zones,
            [CommandZoneSpan {
                start_row: 0,
                end_row: 1,
                exit_status: Some(7),
            }]
        );
    }

    #[test]
    fn cycles_selection_across_completed_osc133_command_outputs() {
        let mut backend = AlacrittyTerminalBackend::new(20, 5);
        backend.advance(
            b"\x1b]133;C\x1b\\first\x1b]133;D;0\x1b\\\r\n\x1b]133;C\x1b\\second\x1b]133;D;1\x1b\\",
        );

        assert!(backend.select_command_output(SearchDirection::Next));
        assert_eq!(backend.selected_text().as_deref(), Some("first"));
        assert!(backend.select_command_output(SearchDirection::Next));
        assert_eq!(backend.selected_text().as_deref(), Some("second"));
        assert!(backend.select_command_output(SearchDirection::Previous));
        assert_eq!(backend.selected_text().as_deref(), Some("first"));
    }
}
