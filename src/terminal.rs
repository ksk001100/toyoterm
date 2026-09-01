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
    pub bracketed_paste: bool,
    pub mouse_reporting: bool,
    pub sgr_mouse: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionSpan {
    pub row: u16,
    pub start_column: u16,
    pub end_column: u16,
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
    pub selection: Vec<SelectionSpan>,
}

/// Adapter boundary for a VT implementation such as `alacritty_terminal`.
pub trait TerminalBackend: Send {
    fn advance(&mut self, bytes: &[u8]);
    fn resize(&mut self, columns: u16, rows: u16);
    fn snapshot(&self) -> TerminalSnapshot;
    fn cursor(&self) -> CursorState;
    fn mode(&self) -> TerminalMode;
    fn scroll_display(&mut self, lines: i32);
    fn start_selection(&mut self, column: u16, row: u16, kind: SelectionKind);
    fn update_selection(&mut self, column: u16, row: u16);
    fn clear_selection(&mut self);
    fn selected_text(&self) -> Option<String>;
}

mod alacritty;

pub use alacritty::{AlacrittyTerminalBackend, DEFAULT_SCROLLBACK_LINES};
