use super::graphics::{
    Graphics, PlaceholderCell,
    handler::{GraphicsHandler, SemanticMarkerKind, SemanticMarkers},
    stream::{Stream, Token},
};
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
use cursor_icon::CursorIcon;

use super::{
    CellAttributes, CellColor, CommandZoneSpan, CursorShape, CursorState, SearchDirection,
    SearchMatchSpan, SearchResult, SelectionKind, SelectionSpan, TerminalBackend, TerminalCell,
    TerminalColors, TerminalMode, TerminalSnapshot, TerminalSpecialColors,
    TerminalTransparentColor,
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
pub const MAX_OSC_NOTIFICATION_ICON_BYTES: usize = 1024 * 1024;
// alacritty_terminal deliberately ignores SGR 5/6/25, but its cell flag
// storage still has one unused bit. Keeping blink there makes the attribute
// follow normal cell copies, scrollback, alternate screens, and reflow.
pub(crate) const BLINK_FLAG_BITS: u16 = 1 << 15;
const MAX_SHELL_INTEGRATION_PAYLOAD_BYTES: usize = 8 * 1024;
const MAX_ITERM_COPY_BASE64_BYTES: usize = MAX_OSC52_COPY_BYTES.div_ceil(3) * 4;
const ITERM_COPY_PREFIX: &[u8] = b"1337;Copy=:";

mod protocol;

#[cfg(test)]
use protocol::parse_iterm_color;
pub(crate) use protocol::{ClipboardCapture, MouseCursorStacks};
use protocol::{
    DynamicUiColor, ShellIntegrationParser, color_control_index, format_transparent_background,
    kitty_ui_color_role, mouse_cursor_shape, parse_kitty_color, parse_transparent_background,
    transparent_background_index,
};
pub use protocol::{
    ItermUiColorRole, NotificationIcon, NotificationOccasion, NotificationReporting,
    NotificationSound, NotificationUrgency, SessionStatusUpdate, TabColorComponent,
    TerminalAttention, TerminalEvent, TerminalProgress,
};

struct TerminalEventSender(Sender<TerminalEvent>);

impl EventListener for TerminalEventSender {
    fn send_event(&self, event: AlacrittyEvent) {
        let event = match event {
            AlacrittyEvent::Title(title) => Some(TerminalEvent::TitleChanged(title)),
            AlacrittyEvent::ResetTitle => Some(TerminalEvent::TitleReset),
            AlacrittyEvent::PtyWrite(text) => Some(TerminalEvent::PtyWrite(text)),
            AlacrittyEvent::Bell => Some(TerminalEvent::Bell { visual_bell: None }),
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
    special_colors: TerminalSpecialColors,
    cursor_dynamic: bool,
    color_stack: Vec<ColorStackEntry>,
    clipboard_capture: Option<ClipboardCapture>,
    pending_events: Vec<TerminalEvent>,
    selection_anchor: Option<Point>,
    search: SearchState,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ItermUiColors {
    link: Option<[u8; 3]>,
    cursor_foreground: DynamicUiColor,
    underline: Option<[u8; 3]>,
    selection_background: DynamicUiColor,
    selection_foreground: DynamicUiColor,
    visual_bell: DynamicUiColor,
    transparent_backgrounds: [Option<TerminalTransparentColor>; 7],
}

struct ColorStackEntry {
    terminal: Vec<Option<alacritty_terminal::vte::ansi::Rgb>>,
    iterm_ui: ItermUiColors,
    special: TerminalSpecialColors,
    cursor_dynamic: bool,
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
            special_colors: TerminalSpecialColors::default(),
            cursor_dynamic: false,
            color_stack: Vec::new(),
            clipboard_capture: None,
            pending_events: Vec::new(),
            selection_anchor: None,
            search: SearchState::default(),
        }
    }

    pub fn drain_events(&mut self) -> Vec<TerminalEvent> {
        self.collect_vt_events();
        std::mem::take(&mut self.pending_events)
    }

    fn collect_vt_events(&mut self) {
        let visual_bell = self.render_colors().visual_bell;
        self.pending_events
            .extend(self.events.try_iter().map(|event| {
                if matches!(event, TerminalEvent::Bell { .. }) {
                    TerminalEvent::Bell { visual_bell }
                } else {
                    event
                }
            }));
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
                Token::CellSizeQuery => {
                    let (width, height) = self.graphics.size();
                    let _ = self
                        .event_sender
                        .send(TerminalEvent::PtyWrite(format!("\x1b[6;{height};{width}t")));
                }
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
                    let background = match self.terminal.grid().cursor.template.bg {
                        Color::Spec(rgb) => rgb,
                        Color::Indexed(index) => {
                            resolved_color(&self.terminal, &self.default_colors, usize::from(index))
                                .expect("indexed terminal colors always resolve")
                        }
                        Color::Named(color) => {
                            resolved_color(&self.terminal, &self.default_colors, color as usize)
                                .unwrap_or_else(|| {
                                    resolved_color(
                                        &self.terminal,
                                        &self.default_colors,
                                        NamedColor::Background as usize,
                                    )
                                    .expect("terminal background always resolves")
                                })
                        }
                    };
                    let result = self.graphics.receive(
                        kind,
                        &payload,
                        (at.column.0 as u16, at.line.0),
                        self.dimensions(),
                        self.terminal.mode().contains(TermMode::ALT_SCREEN),
                        [background.r, background.g, background.b, 255],
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
        match event {
            TerminalEvent::XtermSpecialColorSet { index, color } => {
                *self.special_color_mut(usize::from(index)) = Some(color);
                return;
            }
            TerminalEvent::XtermSpecialColorQuery(index) => {
                let [red, green, blue] = self.resolved_special_color(usize::from(index));
                self.pending_events.push(TerminalEvent::PtyWrite(format!(
                    "\x1b]5;{index};rgb:{red:02x}{red:02x}/{green:02x}{green:02x}/{blue:02x}{blue:02x}\x1b\\"
                )));
                return;
            }
            TerminalEvent::XtermSpecialColorReset(index) => {
                if let Some(index) = index {
                    *self.special_color_mut(usize::from(index)) = None;
                } else {
                    self.special_colors.bold = None;
                    self.special_colors.underline = None;
                    self.special_colors.blink = None;
                    self.special_colors.reverse = None;
                    self.special_colors.italic = None;
                }
                return;
            }
            TerminalEvent::XtermSpecialColorMode { index: 5, enabled } => {
                self.special_colors.override_ansi = enabled;
                return;
            }
            TerminalEvent::XtermSpecialColorMode { index, enabled } => {
                self.special_colors.enabled[usize::from(index)] = enabled;
                return;
            }
            _ => {}
        }
        if let TerminalEvent::ItermDefaultColorQuery(index) = event {
            let color_index = match index {
                -1 => NamedColor::Foreground as usize,
                -2 => NamedColor::Background as usize,
                _ => return,
            };
            let Some(Rgb {
                r: red,
                g: green,
                b: blue,
            }) = resolved_color(&self.terminal, &self.default_colors, color_index)
            else {
                return;
            };
            self.pending_events.push(TerminalEvent::PtyWrite(format!(
                "\x1b]4;{index};rgb:{red:02x}{red:02x}/{green:02x}{green:02x}/{blue:02x}{blue:02x}\x1b\\"
            )));
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
        if event == TerminalEvent::CapturedOutputCleared {
            self.semantic_markers.clear_commands();
            self.pending_events.push(event);
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
            if let Some(index) = transparent_background_index(key) {
                if value == "?" {
                    let response = self.iterm_ui_colors.transparent_backgrounds[index]
                        .map_or_else(String::new, format_transparent_background);
                    responses.push(format!("{key}={response}"));
                } else if reset || value.is_empty() {
                    self.iterm_ui_colors.transparent_backgrounds[index] = None;
                } else if let Some(color) = parse_transparent_background(value) {
                    self.iterm_ui_colors.transparent_backgrounds[index] = Some(color);
                }
                continue;
            }
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
                } else if value.is_empty() {
                    self.set_iterm_ui_color_dynamic(role);
                } else if let Some(Rgb { r, g, b }) = parse_kitty_color(value) {
                    self.set_iterm_ui_color(role, [r, g, b]);
                }
                continue;
            }
            if key == "cursor" {
                let index = NamedColor::Cursor as usize;
                if value == "?" {
                    let response = if self.cursor_dynamic {
                        String::new()
                    } else {
                        resolved_color(&self.terminal, &self.default_colors, index).map_or_else(
                            || "?".to_owned(),
                            |color| format!("rgb:{:02x}/{:02x}/{:02x}", color.r, color.g, color.b),
                        )
                    };
                    responses.push(format!("{key}={response}"));
                } else if reset {
                    self.cursor_dynamic = false;
                    Handler::reset_color(&mut self.terminal, index);
                } else if value.is_empty() {
                    self.cursor_dynamic = true;
                    Handler::reset_color(&mut self.terminal, index);
                } else if let Some(color) = parse_kitty_color(value) {
                    self.cursor_dynamic = false;
                    Handler::set_color(&mut self.terminal, index, color);
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
            } else if reset {
                Handler::reset_color(&mut self.terminal, index);
            } else if !value.is_empty()
                && let Some(color) = parse_kitty_color(value)
            {
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

    fn special_color_mut(&mut self, index: usize) -> &mut Option<[u8; 3]> {
        match index {
            0 => &mut self.special_colors.bold,
            1 => &mut self.special_colors.underline,
            2 => &mut self.special_colors.blink,
            3 => &mut self.special_colors.reverse,
            4 => &mut self.special_colors.italic,
            _ => unreachable!("special color index is validated"),
        }
    }

    fn resolved_special_color(&self, index: usize) -> [u8; 3] {
        let configured = match index {
            0 => self.special_colors.bold,
            1 => self.special_colors.underline,
            2 => self.special_colors.blink,
            3 => self.special_colors.reverse,
            4 => self.special_colors.italic,
            _ => None,
        };
        configured.unwrap_or_else(|| match index {
            0 => {
                let color = resolved_color(
                    &self.terminal,
                    &self.default_colors,
                    NamedColor::BrightForeground as usize,
                )
                .expect("bold foreground has a configured fallback");
                [color.r, color.g, color.b]
            }
            3 => {
                let color = resolved_color(
                    &self.terminal,
                    &self.default_colors,
                    NamedColor::Background as usize,
                )
                .expect("background has a configured fallback");
                [color.r, color.g, color.b]
            }
            _ => {
                let color = resolved_color(
                    &self.terminal,
                    &self.default_colors,
                    NamedColor::Foreground as usize,
                )
                .expect("foreground has a configured fallback");
                [color.r, color.g, color.b]
            }
        })
    }

    fn set_iterm_ui_color(&mut self, role: ItermUiColorRole, color: [u8; 3]) {
        if role == ItermUiColorRole::VisualBell {
            self.iterm_ui_colors.visual_bell = DynamicUiColor::Explicit(color);
        } else {
            match role {
                ItermUiColorRole::Link => self.iterm_ui_colors.link = Some(color),
                ItermUiColorRole::CursorForeground => {
                    self.iterm_ui_colors.cursor_foreground = DynamicUiColor::Explicit(color);
                }
                ItermUiColorRole::Underline => self.iterm_ui_colors.underline = Some(color),
                ItermUiColorRole::SelectionBackground => {
                    self.iterm_ui_colors.selection_background = DynamicUiColor::Explicit(color);
                }
                ItermUiColorRole::SelectionForeground => {
                    self.iterm_ui_colors.selection_foreground = DynamicUiColor::Explicit(color);
                }
                ItermUiColorRole::VisualBell => unreachable!(),
            }
        }
    }

    fn reset_iterm_ui_color(&mut self, role: ItermUiColorRole) {
        match role {
            ItermUiColorRole::Link => self.iterm_ui_colors.link = None,
            ItermUiColorRole::CursorForeground => {
                self.iterm_ui_colors.cursor_foreground = DynamicUiColor::Default;
            }
            ItermUiColorRole::Underline => self.iterm_ui_colors.underline = None,
            ItermUiColorRole::SelectionBackground => {
                self.iterm_ui_colors.selection_background = DynamicUiColor::Default;
            }
            ItermUiColorRole::SelectionForeground => {
                self.iterm_ui_colors.selection_foreground = DynamicUiColor::Default;
            }
            ItermUiColorRole::VisualBell => {
                self.iterm_ui_colors.visual_bell = DynamicUiColor::Default;
            }
        }
    }

    fn set_iterm_ui_color_dynamic(&mut self, role: ItermUiColorRole) {
        match role {
            ItermUiColorRole::Link => self.iterm_ui_colors.link = None,
            ItermUiColorRole::CursorForeground => {
                self.iterm_ui_colors.cursor_foreground = DynamicUiColor::Dynamic;
            }
            ItermUiColorRole::Underline => self.iterm_ui_colors.underline = None,
            ItermUiColorRole::SelectionBackground => {
                self.iterm_ui_colors.selection_background = DynamicUiColor::Dynamic;
            }
            ItermUiColorRole::SelectionForeground => {
                self.iterm_ui_colors.selection_foreground = DynamicUiColor::Dynamic;
            }
            ItermUiColorRole::VisualBell => {
                self.iterm_ui_colors.visual_bell = DynamicUiColor::Dynamic;
            }
        }
    }

    fn resolved_iterm_ui_color(&self, role: ItermUiColorRole) -> Option<[u8; 3]> {
        match role {
            ItermUiColorRole::Link => Some(
                self.iterm_ui_colors
                    .link
                    .unwrap_or(self.default_colors.foreground),
            ),
            ItermUiColorRole::CursorForeground => self.iterm_ui_colors.cursor_foreground.explicit(),
            ItermUiColorRole::Underline => Some(
                self.iterm_ui_colors
                    .underline
                    .unwrap_or(self.default_colors.foreground),
            ),
            ItermUiColorRole::SelectionBackground => {
                match self.iterm_ui_colors.selection_background {
                    DynamicUiColor::Default => Some(self.default_colors.selection),
                    DynamicUiColor::Dynamic => None,
                    DynamicUiColor::Explicit(color) => Some(color),
                }
            }
            ItermUiColorRole::SelectionForeground => {
                self.iterm_ui_colors.selection_foreground.explicit()
            }
            ItermUiColorRole::VisualBell => match self.iterm_ui_colors.visual_bell {
                DynamicUiColor::Explicit(color) => Some(color),
                DynamicUiColor::Default | DynamicUiColor::Dynamic => None,
            },
        }
    }

    fn xterm_ui_color(&self, role: ItermUiColorRole) -> [u8; 3] {
        self.resolved_iterm_ui_color(role).unwrap_or(match role {
            ItermUiColorRole::CursorForeground => self.default_colors.background,
            ItermUiColorRole::SelectionForeground
            | ItermUiColorRole::Link
            | ItermUiColorRole::Underline => self.default_colors.foreground,
            ItermUiColorRole::SelectionBackground => self.default_colors.selection,
            ItermUiColorRole::VisualBell => self.default_colors.foreground,
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
            special: self.special_colors,
            cursor_dynamic: self.cursor_dynamic,
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
        self.special_colors = colors.special;
        self.cursor_dynamic = colors.cursor_dynamic;
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
        let foreground = resolve(NamedColor::Foreground as usize);
        let background = resolve(NamedColor::Background as usize);
        TerminalColors {
            foreground,
            bold: resolve(NamedColor::BrightForeground as usize),
            background,
            cursor: if self.cursor_dynamic {
                foreground
            } else {
                resolve(NamedColor::Cursor as usize)
            },
            ansi: std::array::from_fn(resolve),
            link: self.iterm_ui_colors.link,
            cursor_foreground: match self.iterm_ui_colors.cursor_foreground {
                DynamicUiColor::Default => None,
                DynamicUiColor::Dynamic => Some(background),
                DynamicUiColor::Explicit(color) => Some(color),
            },
            underline: self.iterm_ui_colors.underline,
            selection_background: self.iterm_ui_colors.selection_background.explicit(),
            selection_foreground: self.iterm_ui_colors.selection_foreground.explicit(),
            selection_background_dynamic: matches!(
                self.iterm_ui_colors.selection_background,
                DynamicUiColor::Dynamic
            ),
            selection_foreground_dynamic: matches!(
                self.iterm_ui_colors.selection_foreground,
                DynamicUiColor::Dynamic
            ),
            visual_bell: match self.iterm_ui_colors.visual_bell {
                DynamicUiColor::Default => None,
                DynamicUiColor::Dynamic => Some(contrasting_color(background)),
                DynamicUiColor::Explicit(color) => Some(color),
            },
            transparent_backgrounds: self.iterm_ui_colors.transparent_backgrounds,
            special: self.special_colors,
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

    fn expand_point_side(&self, mut point: Point, side: Side) -> Point {
        let grid = self.terminal.grid();
        let display_offset = grid.display_offset() as i32;
        let screen_lines = grid.screen_lines() as i32;
        let min_line = Line(-display_offset);
        let max_line = Line(screen_lines - 1 - display_offset);
        if point.line < min_line || point.line > max_line {
            return point;
        }
        let cell = &grid[point.line][point.column];
        if side == Side::Right && cell.flags.contains(Flags::WIDE_CHAR) {
            let max_col = grid.columns().saturating_sub(1);
            point.column = Column((point.column.0 + 1).min(max_col));
        } else if side == Side::Left && cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            point.column = Column(point.column.0.saturating_sub(1));
        }
        point
    }
}

fn contrasting_color([red, green, blue]: [u8; 3]) -> [u8; 3] {
    let luminance = 299 * u32::from(red) + 587 * u32::from(green) + 114 * u32::from(blue);
    if luminance >= 128_000 {
        [0, 0, 0]
    } else {
        [255, 255, 255]
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
            self.collect_vt_events();
            self.record_shell_event(event);
            start = end + 1;
        }
        self.advance_vt(&bytes[start..]);
        self.collect_vt_events();
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

    fn synchronized_update_deadline(&self) -> Option<std::time::Instant> {
        self.processor.sync_timeout().sync_timeout()
    }

    fn stop_synchronized_update(&mut self) {
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
        let mut placeholders = Vec::new();

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
                let placeholder = cell.c == '\u{10eeee}';
                let mut cell_text = if placeholder {
                    " ".to_owned()
                } else {
                    cell.c.to_string()
                };
                text.push(if placeholder { ' ' } else { cell.c });
                if let Some(zerowidth) = cell.zerowidth() {
                    if placeholder {
                        let diacritics = kitty_placeholder_diacritics(zerowidth);
                        if let (Some(image_color), Some((diacritics, diacritic_count))) =
                            (kitty_placeholder_color(cell.fg), diacritics)
                        {
                            placeholders.push(PlaceholderCell {
                                column,
                                row: i32::from(viewport_row),
                                image_color,
                                placement_color: cell
                                    .underline_color()
                                    .and_then(kitty_placeholder_color)
                                    .unwrap_or(0),
                                diacritics,
                                diacritic_count,
                            });
                        }
                    } else {
                        text.extend(zerowidth);
                        cell_text.extend(zerowidth);
                    }
                } else if placeholder && let Some(image_color) = kitty_placeholder_color(cell.fg) {
                    placeholders.push(PlaceholderCell {
                        column,
                        row: i32::from(viewport_row),
                        image_color,
                        placement_color: cell
                            .underline_color()
                            .and_then(kitty_placeholder_color)
                            .unwrap_or(0),
                        diacritics: [0; 3],
                        diacritic_count: 0,
                    });
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
                if let Some(mut start_column) = selected_columns.next() {
                    let mut end_column = selected_columns.next_back().unwrap_or(start_column);
                    if grid[line][Column(usize::from(start_column))]
                        .flags
                        .contains(Flags::WIDE_CHAR_SPACER)
                    {
                        start_column = start_column.saturating_sub(1);
                    }
                    if grid[line][Column(usize::from(end_column))]
                        .flags
                        .contains(Flags::WIDE_CHAR)
                    {
                        end_column = (end_column + 1).min(columns.saturating_sub(1));
                    }
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
            images: self.graphics.snapshot(
                self.mode().alternate_screen,
                display_offset,
                rows,
                &placeholders,
            ),
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
            mouse_drag: mode.contains(TermMode::MOUSE_DRAG),
            mouse_motion: mode.contains(TermMode::MOUSE_MOTION),
            sgr_mouse: mode.contains(TermMode::SGR_MOUSE),
            focus_reporting: mode.contains(TermMode::FOCUS_IN_OUT),
            alternate_screen: mode.contains(TermMode::ALT_SCREEN),
            alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
        }
    }

    fn dimensions(&self) -> (u16, u16) {
        self.dimensions()
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
        let start_point = self.expand_point_side(point, Side::Left);
        let mut selection = Selection::new(selection_type, start_point, Side::Left);
        if selection_type == SelectionType::Simple {
            let end_point = self.expand_point_side(point, Side::Right);
            selection.update(end_point, Side::Right);
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
            let anchor_point = self.expand_point_side(anchor, anchor_side);
            let target_point = self.expand_point_side(point, point_side);
            let mut selection = Selection::new(SelectionType::Simple, anchor_point, anchor_side);
            selection.update(target_point, point_side);
            self.terminal.selection = Some(selection);
        } else {
            let target_point = self.expand_point_side(point, Side::Right);
            if let Some(selection) = self.terminal.selection.as_mut() {
                selection.update(target_point, Side::Right);
            }
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
        blink: flags.intersects(Flags::from_bits_retain(BLINK_FLAG_BITS)),
        strikethrough: flags.contains(Flags::STRIKEOUT),
        dim: flags.contains(Flags::DIM),
        inverse: flags.contains(Flags::INVERSE),
        hidden: flags.contains(Flags::HIDDEN),
    }
}

fn kitty_placeholder_color(color: Color) -> Option<u32> {
    match cell_color(color, false) {
        CellColor::Rgb(r, g, b) => Some((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)),
        CellColor::Indexed(index) => Some(u32::from(index)),
        CellColor::Default => None,
    }
}

fn kitty_placeholder_diacritics(characters: &[char]) -> Option<([u16; 3], u8)> {
    if characters.len() > 3 {
        return None;
    }
    let mut diacritics = [0; 3];
    for (index, character) in characters.iter().enumerate() {
        diacritics[index] = kitty_diacritic_number(*character)?;
    }
    Some((diacritics, characters.len() as u8))
}

fn kitty_diacritic_number(character: char) -> Option<u16> {
    let codepoint = u32::from(character);
    let (offset, start) = match codepoint {
        0x0305 => (0, 0x0305),
        0x030d..=0x030e => (1, 0x030d),
        0x0310 => (3, 0x0310),
        0x0312 => (4, 0x0312),
        0x033d..=0x033f => (5, 0x033d),
        0x0346 => (8, 0x0346),
        0x034a..=0x034c => (9, 0x034a),
        0x0350..=0x0352 => (12, 0x0350),
        0x0357 => (15, 0x0357),
        0x035b => (16, 0x035b),
        0x0363..=0x036f => (17, 0x0363),
        0x0483..=0x0487 => (30, 0x0483),
        0x0592..=0x0595 => (35, 0x0592),
        0x0597..=0x0599 => (39, 0x0597),
        0x059c..=0x05a1 => (42, 0x059c),
        0x05a8..=0x05a9 => (48, 0x05a8),
        0x05ab..=0x05ac => (50, 0x05ab),
        0x05af => (52, 0x05af),
        0x05c4 => (53, 0x05c4),
        0x0610..=0x0617 => (54, 0x0610),
        0x0657..=0x065b => (62, 0x0657),
        0x065d..=0x065e => (67, 0x065d),
        0x06d6..=0x06dc => (69, 0x06d6),
        0x06df..=0x06e2 => (76, 0x06df),
        0x06e4 => (80, 0x06e4),
        0x06e7..=0x06e8 => (81, 0x06e7),
        0x06eb..=0x06ec => (83, 0x06eb),
        0x0730 => (85, 0x0730),
        0x0732..=0x0733 => (86, 0x0732),
        0x0735..=0x0736 => (88, 0x0735),
        0x073a => (90, 0x073a),
        0x073d => (91, 0x073d),
        0x073f..=0x0741 => (92, 0x073f),
        0x0743 => (95, 0x0743),
        0x0745 => (96, 0x0745),
        0x0747 => (97, 0x0747),
        0x0749..=0x074a => (98, 0x0749),
        0x07eb..=0x07f1 => (100, 0x07eb),
        0x07f3 => (107, 0x07f3),
        0x0816..=0x0819 => (108, 0x0816),
        0x081b..=0x0823 => (112, 0x081b),
        0x0825..=0x0827 => (121, 0x0825),
        0x0829..=0x082d => (124, 0x0829),
        0x0951 => (129, 0x0951),
        0x0953..=0x0954 => (130, 0x0953),
        0x0f82..=0x0f83 => (132, 0x0f82),
        0x0f86..=0x0f87 => (134, 0x0f86),
        0x135d..=0x135f => (136, 0x135d),
        0x17dd => (139, 0x17dd),
        0x193a => (140, 0x193a),
        0x1a17 => (141, 0x1a17),
        0x1a75..=0x1a7c => (142, 0x1a75),
        0x1b6b => (150, 0x1b6b),
        0x1b6d..=0x1b73 => (151, 0x1b6d),
        0x1cd0..=0x1cd2 => (158, 0x1cd0),
        0x1cda..=0x1cdb => (161, 0x1cda),
        0x1ce0 => (163, 0x1ce0),
        0x1dc0..=0x1dc1 => (164, 0x1dc0),
        0x1dc3..=0x1dc9 => (166, 0x1dc3),
        0x1dcb..=0x1dcc => (173, 0x1dcb),
        0x1dd1..=0x1de6 => (175, 0x1dd1),
        0x1dfe => (197, 0x1dfe),
        0x20d0..=0x20d1 => (198, 0x20d0),
        0x20d4..=0x20d7 => (200, 0x20d4),
        0x20db..=0x20dc => (204, 0x20db),
        0x20e1 => (206, 0x20e1),
        0x20e7 => (207, 0x20e7),
        0x20e9 => (208, 0x20e9),
        0x20f0 => (209, 0x20f0),
        0x2cef..=0x2cf1 => (210, 0x2cef),
        0x2de0..=0x2dff => (213, 0x2de0),
        0xa66f => (245, 0xa66f),
        0xa67c..=0xa67d => (246, 0xa67c),
        0xa6f0..=0xa6f1 => (248, 0xa6f0),
        0xa8e0..=0xa8f1 => (250, 0xa8e0),
        0xaab0 => (268, 0xaab0),
        0xaab2..=0xaab3 => (269, 0xaab2),
        0xaab7..=0xaab8 => (271, 0xaab7),
        0xaabe..=0xaabf => (273, 0xaabe),
        0xaac1 => (275, 0xaac1),
        0xfe20..=0xfe26 => (276, 0xfe20),
        0x10a0f => (283, 0x10a0f),
        0x10a38 => (284, 0x10a38),
        0x1d185..=0x1d189 => (285, 0x1d185),
        0x1d1aa..=0x1d1ad => (290, 0x1d1aa),
        0x1d242..=0x1d244 => (294, 0x1d242),
        _ => return None,
    };
    Some((offset + codepoint - start) as u16)
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
#[path = "alacritty/tests.rs"]
mod tests;
