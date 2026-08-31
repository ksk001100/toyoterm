use alacritty_terminal::Term;
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, MIN_COLUMNS, MIN_SCREEN_LINES, TermMode};
use alacritty_terminal::vte::ansi::{CursorShape as AlacrittyCursorShape, Processor};

use super::{CursorShape, CursorState, TerminalBackend, TerminalMode, TerminalSnapshot};

pub const DEFAULT_SCROLLBACK_LINES: usize = 10_000;

pub struct AlacrittyTerminalBackend {
    processor: Processor,
    terminal: Term<VoidListener>,
}

impl AlacrittyTerminalBackend {
    pub fn new(columns: u16, rows: u16) -> Self {
        Self::with_scrollback(columns, rows, DEFAULT_SCROLLBACK_LINES)
    }

    pub fn with_scrollback(columns: u16, rows: u16, scrollback_lines: usize) -> Self {
        let size = TermSize::new(columns, rows);
        let config = Config {
            scrolling_history: scrollback_lines,
            ..Config::default()
        };
        Self {
            processor: Processor::new(),
            terminal: Term::new(config, &size, VoidListener),
        }
    }

    fn dimensions(&self) -> (u16, u16) {
        let grid = self.terminal.grid();
        (grid.columns() as u16, grid.screen_lines() as u16)
    }
}

impl Default for AlacrittyTerminalBackend {
    fn default() -> Self {
        Self::new(80, 24)
    }
}

impl TerminalBackend for AlacrittyTerminalBackend {
    fn advance(&mut self, bytes: &[u8]) {
        self.processor.advance(&mut self.terminal, bytes);
    }

    fn resize(&mut self, columns: u16, rows: u16) {
        self.terminal.resize(TermSize::new(columns, rows));
    }

    fn snapshot(&self) -> TerminalSnapshot {
        let grid = self.terminal.grid();
        let (columns, rows) = self.dimensions();
        let display_offset = grid.display_offset() as i32;
        let mut lines = Vec::with_capacity(rows as usize);

        for viewport_row in 0..rows {
            let line = Line(viewport_row as i32 - display_offset);
            let mut text = String::with_capacity(columns as usize);
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
            lines.push(text.trim_end().to_owned());
        }

        TerminalSnapshot {
            columns,
            rows,
            lines,
        }
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
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            mouse_reporting: mode.intersects(TermMode::MOUSE_MODE),
            sgr_mouse: mode.contains(TermMode::SGR_MOUSE),
        }
    }

    fn scroll_display(&mut self, lines: i32) {
        self.terminal.scroll_display(Scroll::Delta(lines));
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
    fn tracks_terminal_modes_and_cursor_visibility() {
        let mut backend = AlacrittyTerminalBackend::new(10, 2);
        backend.advance(b"\x1b[?1h\x1b[?2004h\x1b[?1000h\x1b[?25l");
        assert_eq!(
            backend.mode(),
            TerminalMode {
                application_cursor: true,
                bracketed_paste: true,
                mouse_reporting: true,
                sgr_mouse: false,
            }
        );
        assert!(!backend.cursor().visible);
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
    fn supports_a_configurable_scrollback_limit() {
        let mut backend = AlacrittyTerminalBackend::with_scrollback(10, 2, 1);
        backend.advance(b"one\r\ntwo\r\nthree\r\nfour");
        backend.scroll_display(i32::MAX);
        assert_eq!(backend.snapshot().lines, ["two", "three"]);
    }
}
