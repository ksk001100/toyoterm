use alacritty_terminal::Term;
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, MIN_COLUMNS, MIN_SCREEN_LINES, TermMode};
use alacritty_terminal::vte::ansi::{
    Color, CursorShape as AlacrittyCursorShape, NamedColor, Processor,
};

use super::{
    CellAttributes, CellColor, CursorShape, CursorState, SelectionKind, SelectionSpan,
    TerminalBackend, TerminalCell, TerminalMode, TerminalSnapshot,
};

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

    pub fn set_scrollback_lines(&mut self, scrollback_lines: usize) {
        self.terminal.grid_mut().update_history(scrollback_lines);
    }

    fn dimensions(&self) -> (u16, u16) {
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
        let mut rows_of_cells = Vec::with_capacity(rows as usize);
        let selection_range = self
            .terminal
            .selection
            .as_ref()
            .and_then(|selection| selection.to_range(&self.terminal));
        let mut selection = Vec::new();

        for viewport_row in 0..rows {
            let line = Line(viewport_row as i32 - display_offset);
            let mut text = String::with_capacity(columns as usize);
            let mut cells = Vec::with_capacity(columns as usize);
            for column in 0..columns {
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
                });
            }
            lines.push(text.trim_end().to_owned());
            while cells.last().is_some_and(is_default_blank_cell) {
                cells.pop();
            }
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

        TerminalSnapshot {
            columns,
            rows,
            lines,
            cells: rows_of_cells,
            selection,
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

    fn start_selection(&mut self, column: u16, row: u16, kind: SelectionKind) {
        let point = self.viewport_point(column, row);
        let selection_type = match kind {
            SelectionKind::Simple => SelectionType::Simple,
            SelectionKind::Word => SelectionType::Semantic,
            SelectionKind::Line => SelectionType::Lines,
        };
        self.terminal.selection = Some(Selection::new(selection_type, point, Side::Left));
    }

    fn update_selection(&mut self, column: u16, row: u16) {
        let point = self.viewport_point(column, row);
        if let Some(selection) = self.terminal.selection.as_mut() {
            selection.update(point, Side::Right);
        }
    }

    fn clear_selection(&mut self) {
        self.terminal.selection = None;
    }

    fn selected_text(&self) -> Option<String> {
        self.terminal.selection_to_string()
    }
}

fn is_default_blank_cell(cell: &TerminalCell) -> bool {
    cell.text == " " && cell.attributes == CellAttributes::default()
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
    fn selects_words_and_lines() {
        let mut backend = AlacrittyTerminalBackend::new(20, 2);
        backend.advance(b"alpha beta\r\nsecond line");

        backend.start_selection(7, 0, SelectionKind::Word);
        assert_eq!(backend.selected_text().as_deref(), Some("beta"));

        backend.start_selection(3, 1, SelectionKind::Line);
        assert_eq!(backend.selected_text().as_deref(), Some("second line\n"));
    }
}
