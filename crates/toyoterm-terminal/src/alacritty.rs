use std::sync::mpsc::{self, Receiver, Sender};

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event as AlacrittyEvent, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, MIN_COLUMNS, MIN_SCREEN_LINES, Osc52, TermMode};
use alacritty_terminal::vte::ansi::{
    Color, CursorShape as AlacrittyCursorShape, NamedColor, Processor,
};

use super::{
    CellAttributes, CellColor, CursorShape, CursorState, SearchDirection, SearchMatchSpan,
    SearchResult, SelectionKind, SelectionSpan, TerminalBackend, TerminalCell, TerminalMode,
    TerminalSnapshot,
};

pub const DEFAULT_SCROLLBACK_LINES: usize = 10_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalEvent {
    TitleChanged(String),
    TitleReset,
    CwdChanged(String),
    CommandStarted,
    CommandFinished(Option<i32>),
    PtyWrite(String),
    Bell,
}

#[derive(Default)]
enum ShellIntegrationState {
    #[default]
    Ground,
    Escape,
    Command(Vec<u8>),
    Payload(Vec<u8>),
    PayloadEscape(Vec<u8>),
    Ignore,
    IgnoreEscape,
}

#[derive(Default)]
struct ShellIntegrationParser {
    state: ShellIntegrationState,
}

impl ShellIntegrationParser {
    fn advance(&mut self, bytes: &[u8]) -> Vec<TerminalEvent> {
        let mut events = Vec::new();
        for &byte in bytes {
            let state = std::mem::take(&mut self.state);
            self.state = match state {
                ShellIntegrationState::Ground if byte == b'\x1b' => ShellIntegrationState::Escape,
                ShellIntegrationState::Ground => ShellIntegrationState::Ground,
                ShellIntegrationState::Escape if byte == b']' => {
                    ShellIntegrationState::Command(Vec::new())
                }
                ShellIntegrationState::Escape => ShellIntegrationState::Ground,
                ShellIntegrationState::Command(command)
                    if byte == b';' && (command == b"7" || command == b"133") =>
                {
                    let mut payload = command;
                    payload.push(b';');
                    ShellIntegrationState::Payload(payload)
                }
                ShellIntegrationState::Command(_) if byte == b';' => ShellIntegrationState::Ignore,
                ShellIntegrationState::Command(_) if byte == b'\x07' => {
                    ShellIntegrationState::Ground
                }
                ShellIntegrationState::Command(mut command) if command.len() < 16 => {
                    command.push(byte);
                    ShellIntegrationState::Command(command)
                }
                ShellIntegrationState::Command(_) => ShellIntegrationState::Ignore,
                ShellIntegrationState::Payload(payload) if byte == b'\x07' => {
                    parse_shell_integration_payload(&payload, &mut events);
                    ShellIntegrationState::Ground
                }
                ShellIntegrationState::Payload(payload) if byte == b'\x1b' => {
                    ShellIntegrationState::PayloadEscape(payload)
                }
                ShellIntegrationState::Payload(mut payload) if payload.len() < 8_192 => {
                    payload.push(byte);
                    ShellIntegrationState::Payload(payload)
                }
                ShellIntegrationState::Payload(_) => ShellIntegrationState::Ignore,
                ShellIntegrationState::PayloadEscape(payload) if byte == b'\\' => {
                    parse_shell_integration_payload(&payload, &mut events);
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

fn parse_shell_integration_payload(payload: &[u8], events: &mut Vec<TerminalEvent>) {
    if let Some(payload) = payload.strip_prefix(b"7;") {
        if let Some(path) = osc7_path(payload) {
            events.push(TerminalEvent::CwdChanged(path));
        }
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
    processor: Processor,
    terminal: Term<TerminalEventSender>,
    events: Receiver<TerminalEvent>,
    shell_integration: ShellIntegrationParser,
    pending_events: Vec<TerminalEvent>,
    selection_anchor: Option<Point>,
    search: SearchState,
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
            processor: Processor::new(),
            terminal: Term::new(config, &size, TerminalEventSender(event_sender)),
            events,
            shell_integration: ShellIntegrationParser::default(),
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

fn terminal_config(scrollback_lines: usize) -> Config {
    Config {
        scrolling_history: scrollback_lines,
        // OSC 52 allows terminal output, including output from a remote host,
        // to access the host clipboard without an explicit user gesture. Keep
        // it disabled until toyoterm has an opt-in permission and confirmation UI.
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
        let shell_events = self.shell_integration.advance(bytes);
        self.processor.advance(&mut self.terminal, bytes);
        self.pending_events.extend(shell_events);
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
                    hyperlink: cell.hyperlink().map(|link| link.uri().to_owned()),
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

        TerminalSnapshot {
            columns,
            rows,
            lines,
            cells: rows_of_cells,
            selection,
            search_matches,
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
        self.selection_anchor = Some(point);
        let selection_type = match kind {
            SelectionKind::Simple => SelectionType::Simple,
            SelectionKind::Word => SelectionType::Semantic,
            SelectionKind::Line => SelectionType::Lines,
        };
        self.terminal.selection = Some(Selection::new(selection_type, point, Side::Left));
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
}

fn is_default_blank_cell(cell: &TerminalCell) -> bool {
    cell.text == " " && cell.attributes == CellAttributes::default() && cell.hyperlink.is_none()
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
        backend.advance(b"\x1b\\\x1b]133;C\x1b\\\x1b]133;D;17\x07\x07");

        let events = backend.drain_events();
        assert!(events.contains(&TerminalEvent::TitleChanged("build server".into())));
        assert!(events.contains(&TerminalEvent::CwdChanged("/srv/my app".into())));
        assert!(events.contains(&TerminalEvent::CommandStarted));
        assert!(events.contains(&TerminalEvent::CommandFinished(Some(17))));
        assert!(events.contains(&TerminalEvent::Bell));
        assert!(backend.drain_events().is_empty());
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
}
