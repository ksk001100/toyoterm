#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorShape {
    Block,
    Beam,
    Underline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CursorState {
    pub column: u16,
    pub row: u16,
    pub visible: bool,
    pub shape: CursorShape,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TerminalMode {
    pub application_cursor: bool,
    pub application_keypad: bool,
    pub bracketed_paste: bool,
    pub mouse_reporting: bool,
    pub mouse_drag: bool,
    pub mouse_motion: bool,
    pub sgr_mouse: bool,
    pub focus_reporting: bool,
    pub alternate_screen: bool,
    pub alternate_scroll: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionSpan {
    pub row: u16,
    pub start_column: u16,
    pub end_column: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SearchMatchSpan {
    pub row: u16,
    pub start_column: u16,
    pub end_column: u16,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandZoneSpan {
    pub start_row: u16,
    pub end_row: u16,
    pub exit_status: Option<i32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchDirection {
    Next,
    Previous,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SearchResult {
    pub current: usize,
    pub total: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CellColor {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CellAttributes {
    pub foreground: CellColor,
    pub background: CellColor,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub blink: bool,
    pub strikethrough: bool,
    pub dim: bool,
    pub inverse: bool,
    pub hidden: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalSpecialColors {
    pub bold: Option<[u8; 3]>,
    pub underline: Option<[u8; 3]>,
    pub blink: Option<[u8; 3]>,
    pub reverse: Option<[u8; 3]>,
    pub italic: Option<[u8; 3]>,
    pub enabled: [bool; 5],
    pub override_ansi: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerminalTransparentColor {
    pub color: [u8; 3],
    /// Explicit opacity. `None` inherits the configured window opacity.
    pub opacity: Option<f32>,
}

impl Default for TerminalSpecialColors {
    fn default() -> Self {
        Self {
            bold: None,
            underline: None,
            blink: None,
            reverse: None,
            italic: None,
            enabled: [true; 5],
            override_ansi: false,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TerminalCell {
    pub column: u16,
    pub text: String,
    pub width: u8,
    /// Kitty OSC 66 sizing for a cell-spanning text block.
    pub text_size: Option<TextSize>,
    pub attributes: CellAttributes,
    /// Explicit OSC 8 target, or a safely detected URL.
    pub hyperlink: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextSize {
    pub scale: u8,
    pub numerator: u8,
    pub denominator: u8,
    pub vertical_alignment: TextAlignment,
    pub horizontal_alignment: TextAlignment,
    pub rows: u8,
}

impl TextSize {
    pub fn rendered_scale(self) -> f32 {
        let fraction = if self.denominator == 0 {
            1.0
        } else {
            f32::from(self.numerator) / f32::from(self.denominator)
        };
        f32::from(self.scale) * fraction
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextAlignment {
    #[default]
    Start,
    End,
    Center,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SelectionKind {
    #[default]
    Simple,
    Word,
    Line,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalSnapshot {
    pub columns: u16,
    pub rows: u16,
    pub lines: Vec<String>,
    pub cells: Vec<Vec<TerminalCell>>,
    pub selection: Vec<SelectionSpan>,
    pub search_matches: Vec<SearchMatchSpan>,
    pub command_zones: Vec<CommandZoneSpan>,
    pub images: Vec<TerminalImage>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerminalColors {
    pub foreground: [u8; 3],
    pub bold: [u8; 3],
    pub background: [u8; 3],
    pub cursor: [u8; 3],
    pub ansi: [[u8; 3]; 16],
    pub link: Option<[u8; 3]>,
    pub cursor_foreground: Option<[u8; 3]>,
    pub underline: Option<[u8; 3]>,
    pub selection_background: Option<[u8; 3]>,
    pub selection_foreground: Option<[u8; 3]>,
    pub selection_background_dynamic: bool,
    pub selection_foreground_dynamic: bool,
    pub visual_bell: Option<[u8; 3]>,
    pub transparent_backgrounds: [Option<TerminalTransparentColor>; 7],
    pub special: TerminalSpecialColors,
}

/// Adapter boundary for a VT implementation such as `alacritty_terminal`.
pub trait TerminalBackend: Send {
    fn advance(&mut self, bytes: &[u8]);
    fn synchronized_update_deadline(&self) -> Option<std::time::Instant>;
    fn stop_synchronized_update(&mut self);
    fn resize(&mut self, columns: u16, rows: u16);
    fn snapshot(&self) -> TerminalSnapshot;
    fn visible_text(&self) -> String;
    fn cursor(&self) -> CursorState;
    fn mode(&self) -> TerminalMode;
    fn dimensions(&self) -> (u16, u16);
    fn scroll_display(&mut self, lines: i32);
    fn scroll_to_bottom(&mut self);
    fn start_selection(&mut self, column: u16, row: u16, kind: SelectionKind);
    fn update_selection(&mut self, column: u16, row: u16);
    fn clear_selection(&mut self);
    fn selected_text(&self) -> Option<String>;
    fn search(&mut self, query: &str, direction: SearchDirection) -> SearchResult;
    fn clear_search(&mut self);
    fn navigate_prompt(&mut self, direction: SearchDirection) -> bool;
    fn navigate_mark(&mut self, direction: SearchDirection) -> bool;
    fn select_command_output(&mut self, direction: SearchDirection) -> bool;
    fn select_last_command_output(&mut self) -> bool;
}

mod alacritty;
mod graphics;
mod input;
pub use graphics::TerminalImage;

pub use alacritty::{
    AlacrittyTerminalBackend, DEFAULT_SCROLLBACK_LINES, FileTransferEntry, FileTransferEntryKind,
    FileUploadPath, ItermUiColorRole, MAX_OSC_BACKGROUND_IMAGE_PATH_BYTES,
    MAX_OSC_NOTIFICATION_BYTES, MAX_OSC_REPORT_VARIABLE_NAME_BYTES, MAX_OSC52_COPY_BYTES,
    NotificationIcon, NotificationOccasion, NotificationReporting, NotificationSound,
    NotificationUrgency, SessionStatusUpdate, TabColorComponent, TerminalAttention,
    TerminalColorPreset, TerminalEvent, TerminalProgress,
};
pub use input::{
    BindingKey, KeyChord, KeyModifiers, KeyPress, KeypadKey, MouseEventKind, MouseWheelDirection,
    TerminalKey, TerminalMouseButton, encode_key, encode_mouse_event, encode_mouse_wheel,
    encode_paste,
};
