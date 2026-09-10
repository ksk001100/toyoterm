// Explicit forwarding keeps Alacritty as the owner of all text semantics.
use super::Graphics;
use alacritty_terminal::vte::ansi::*;
use alacritty_terminal::{Term, event::EventListener, grid::Dimensions, term::TermMode};
use base64::Engine;
use unicode_width::UnicodeWidthChar;

const MAX_SEMANTIC_MARKERS: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SemanticMarkerKind {
    Mark,
    Prompt,
    CommandLine,
    CommandStart,
    CommandEnd,
}

#[derive(Clone, Copy)]
struct SemanticMarker {
    serial: u64,
    kind: SemanticMarkerKind,
    line: i32,
    column: usize,
    alternate: bool,
    exit_status: Option<i32>,
}

#[derive(Default)]
pub(crate) struct SemanticMarkers {
    markers: Vec<SemanticMarker>,
    serial: u64,
    current_prompt: Option<u64>,
    current_mark: Option<u64>,
    current_command: Option<u64>,
}

impl SemanticMarkers {
    pub fn mark(
        &mut self,
        kind: SemanticMarkerKind,
        line: i32,
        column: usize,
        alternate: bool,
        exit_status: Option<i32>,
    ) {
        self.serial = self.serial.wrapping_add(1);
        self.markers.push(SemanticMarker {
            serial: self.serial,
            kind,
            line,
            column,
            alternate,
            exit_status,
        });
        if kind == SemanticMarkerKind::Prompt {
            self.current_prompt = None;
        }
        if kind == SemanticMarkerKind::Mark {
            self.current_mark = None;
        }
        if kind == SemanticMarkerKind::CommandStart {
            self.current_command = None;
        }
        if self.markers.len() > MAX_SEMANTIC_MARKERS {
            let removed = self.markers.remove(0);
            if self.current_prompt == Some(removed.serial) {
                self.current_prompt = None;
            }
            if self.current_mark == Some(removed.serial) {
                self.current_mark = None;
            }
            if self.current_command == Some(removed.serial) {
                self.current_command = None;
            }
        }
    }

    pub fn navigate_prompt(
        &mut self,
        direction: crate::SearchDirection,
        alternate: bool,
    ) -> Option<(i32, usize, usize)> {
        let prompts = self
            .markers
            .iter()
            .filter(|marker| {
                marker.alternate == alternate && marker.kind == SemanticMarkerKind::Prompt
            })
            .collect::<Vec<_>>();
        if prompts.is_empty() {
            self.current_prompt = None;
            return None;
        }
        let previous = self
            .current_prompt
            .and_then(|serial| prompts.iter().position(|marker| marker.serial == serial));
        let index = match (previous, direction) {
            (None, crate::SearchDirection::Next) => 0,
            (None, crate::SearchDirection::Previous) => prompts.len() - 1,
            (Some(index), crate::SearchDirection::Next) => (index + 1) % prompts.len(),
            (Some(index), crate::SearchDirection::Previous) => {
                (index + prompts.len() - 1) % prompts.len()
            }
        };
        self.current_prompt = Some(prompts[index].serial);
        Some((prompts[index].line, index + 1, prompts.len()))
    }

    pub fn navigate_mark(
        &mut self,
        direction: crate::SearchDirection,
        alternate: bool,
    ) -> Option<(i32, usize, usize)> {
        let marks = self
            .markers
            .iter()
            .filter(|marker| {
                marker.alternate == alternate && marker.kind == SemanticMarkerKind::Mark
            })
            .collect::<Vec<_>>();
        if marks.is_empty() {
            self.current_mark = None;
            return None;
        }
        let previous = self
            .current_mark
            .and_then(|serial| marks.iter().position(|marker| marker.serial == serial));
        let index = match (previous, direction) {
            (None, crate::SearchDirection::Next) => 0,
            (None, crate::SearchDirection::Previous) => marks.len() - 1,
            (Some(index), crate::SearchDirection::Next) => (index + 1) % marks.len(),
            (Some(index), crate::SearchDirection::Previous) => {
                (index + marks.len() - 1) % marks.len()
            }
        };
        self.current_mark = Some(marks[index].serial);
        Some((marks[index].line, index + 1, marks.len()))
    }

    pub fn has_markers(&self) -> bool {
        !self.markers.is_empty()
    }

    pub fn last_command_range(&mut self, alternate: bool) -> Option<((i32, usize), (i32, usize))> {
        let ranges = self.command_ranges(alternate);
        let (start, end) = ranges.last()?;
        self.current_command = Some(end.serial);
        Some(((start.line, start.column), (end.line, end.column)))
    }

    pub fn navigate_command_output(
        &mut self,
        direction: crate::SearchDirection,
        alternate: bool,
    ) -> Option<((i32, usize), (i32, usize))> {
        let ranges = self.command_ranges(alternate);
        if ranges.is_empty() {
            self.current_command = None;
            return None;
        }
        let previous = self
            .current_command
            .and_then(|serial| ranges.iter().position(|(_, end)| end.serial == serial));
        let index = match (previous, direction) {
            (None, crate::SearchDirection::Next) => 0,
            (None, crate::SearchDirection::Previous) => ranges.len() - 1,
            (Some(index), crate::SearchDirection::Next) => (index + 1) % ranges.len(),
            (Some(index), crate::SearchDirection::Previous) => {
                (index + ranges.len() - 1) % ranges.len()
            }
        };
        let (start, end) = ranges[index];
        self.current_command = Some(end.serial);
        Some(((start.line, start.column), (end.line, end.column)))
    }

    pub fn command_zones(&self, alternate: bool) -> Vec<(i32, i32, Option<i32>)> {
        self.command_ranges(alternate)
            .into_iter()
            .map(|(start, end)| (start.line, end.line.max(start.line), end.exit_status))
            .collect()
    }

    fn command_ranges(&self, alternate: bool) -> Vec<(SemanticMarker, SemanticMarker)> {
        let mut start = None;
        let mut ranges = Vec::new();
        for marker in self.markers.iter().copied().filter(|marker| {
            marker.alternate == alternate
                && matches!(
                    marker.kind,
                    SemanticMarkerKind::CommandStart | SemanticMarkerKind::CommandEnd
                )
        }) {
            match marker.kind {
                SemanticMarkerKind::CommandStart => start = Some(marker),
                SemanticMarkerKind::CommandEnd => {
                    if let Some(start) = start.take() {
                        ranges.push((start, marker));
                    }
                }
                _ => unreachable!(),
            }
        }
        ranges
    }

    pub fn reset(&mut self) {
        self.markers.clear();
        self.current_prompt = None;
        self.current_mark = None;
        self.current_command = None;
    }

    pub fn clear(&mut self, alternate: bool, start: i32, end: i32) {
        self.retain(|marker| {
            marker.alternate != alternate || marker.line < start || marker.line >= end
        });
    }

    pub fn scroll(&mut self, alternate: bool, top: i32, bottom: i32, amount: i32, history: usize) {
        let minimum = if top == 0 && !alternate {
            -(history as i32)
        } else {
            top
        };
        self.retain_mut(|marker| {
            if marker.alternate == alternate
                && marker.line < bottom
                && (marker.line >= top || (top == 0 && amount > 0))
            {
                marker.line -= amount;
                return marker.line >= minimum && marker.line < bottom;
            }
            true
        });
    }

    fn retain(&mut self, mut keep: impl FnMut(&SemanticMarker) -> bool) {
        let current = self.current_prompt;
        self.markers.retain(|marker| keep(marker));
        if current.is_some_and(|serial| !self.markers.iter().any(|marker| marker.serial == serial))
        {
            self.current_prompt = None;
        }
        if self
            .current_mark
            .is_some_and(|serial| !self.markers.iter().any(|marker| marker.serial == serial))
        {
            self.current_mark = None;
        }
        if self
            .current_command
            .is_some_and(|serial| !self.markers.iter().any(|marker| marker.serial == serial))
        {
            self.current_command = None;
        }
    }

    fn retain_mut(&mut self, mut keep: impl FnMut(&mut SemanticMarker) -> bool) {
        let current = self.current_prompt;
        self.markers.retain_mut(|marker| keep(marker));
        if current.is_some_and(|serial| !self.markers.iter().any(|marker| marker.serial == serial))
        {
            self.current_prompt = None;
        }
        if self
            .current_mark
            .is_some_and(|serial| !self.markers.iter().any(|marker| marker.serial == serial))
        {
            self.current_mark = None;
        }
        if self
            .current_command
            .is_some_and(|serial| !self.markers.iter().any(|marker| marker.serial == serial))
        {
            self.current_command = None;
        }
    }
}

pub(crate) struct GraphicsHandler<'a, E> {
    pub terminal: &'a mut Term<E>,
    pub output: &'a std::sync::mpsc::Sender<crate::TerminalEvent>,
    pub default_colors: &'a super::super::alacritty::DefaultColors,
    pub allow_osc52_copy: bool,
    pub graphics: &'a mut Graphics,
    pub semantic_markers: &'a mut SemanticMarkers,
    pub mouse_cursor_stacks: &'a mut super::super::alacritty::MouseCursorStacks,
    pub clipboard_capture: &'a mut Option<super::super::alacritty::ClipboardCapture>,
}
impl<E: EventListener> Handler for GraphicsHandler<'_, E> {
    fn input(&mut self, c: char) {
        if let Some(capture) = self.clipboard_capture.as_mut() {
            let mut encoded = [0; 4];
            capture.push(c.encode_utf8(&mut encoded));
        }
        if !self.graphics.has_placements() && !self.semantic_markers.has_markers() {
            self.terminal.input(c);
            return;
        }
        let cursor = &self.terminal.grid().cursor;
        let width = c.width().unwrap_or(0);
        let wraps = self.terminal.mode().contains(TermMode::LINE_WRAP)
            && width > 0
            && (cursor.input_needs_wrap
                || (width == 2 && cursor.point.column.0 + 1 >= self.terminal.columns()));
        if wraps {
            self.track_linefeed();
        }
        self.terminal.input(c);
    }
    fn linefeed(&mut self) {
        if let Some(capture) = self.clipboard_capture.as_mut() {
            capture.push("\n");
        }
        self.track_linefeed();
        self.terminal.linefeed();
    }
    fn newline(&mut self) {
        if let Some(capture) = self.clipboard_capture.as_mut() {
            capture.push("\n");
        }
        self.track_linefeed();
        self.terminal.newline();
    }
    fn scroll_up(&mut self, count: usize) {
        self.track_scroll(count, false, false);
        self.terminal.scroll_up(count);
    }
    fn scroll_down(&mut self, count: usize) {
        self.track_scroll(count, true, false);
        self.terminal.scroll_down(count);
    }
    fn insert_blank_lines(&mut self, count: usize) {
        self.track_scroll(count, true, true);
        self.terminal.insert_blank_lines(count);
    }
    fn delete_lines(&mut self, count: usize) {
        self.track_scroll(count, false, true);
        self.terminal.delete_lines(count);
    }
    fn reverse_index(&mut self) {
        if self.terminal.grid().cursor.point.line.0 == self.region().0 {
            self.track_scroll(1, true, false);
        }
        self.terminal.reverse_index();
    }
    fn clear_screen(&mut self, mode: ClearMode) {
        let row = self.terminal.grid().cursor.point.line.0;
        let rows = self.terminal.screen_lines() as i32;
        let (start, end) = match mode {
            ClearMode::All => (0, rows),
            ClearMode::Above => (0, row + 1),
            ClearMode::Below => (row, rows),
            ClearMode::Saved => (i32::MIN, 0),
        };
        self.graphics.clear(self.alternate(), start, end);
        self.semantic_markers.clear(self.alternate(), start, end);
        self.terminal.clear_screen(mode);
    }
    fn clear_line(&mut self, mode: LineClearMode) {
        let row = self.terminal.grid().cursor.point.line.0;
        self.graphics.clear(self.alternate(), row, row + 1);
        self.semantic_markers.clear(self.alternate(), row, row + 1);
        self.terminal.clear_line(mode);
    }
    fn reset_state(&mut self) {
        *self.clipboard_capture = None;
        self.graphics.reset();
        self.semantic_markers.reset();
        let cursor_changed = self.mouse_cursor_stacks.current_icon(self.alternate())
            != cursor_icon::CursorIcon::Default;
        self.mouse_cursor_stacks.reset();
        if cursor_changed {
            let _ = self.output.send(crate::TerminalEvent::MouseCursorChanged(
                cursor_icon::CursorIcon::Default,
            ));
        }
        self.terminal.reset_state();
    }
    fn set_scrolling_region(&mut self, top: usize, bottom: Option<usize>) {
        let end = bottom.unwrap_or(self.terminal.screen_lines());
        if top < end {
            self.graphics.region = Some((
                (top.saturating_sub(1)).min(self.terminal.screen_lines()) as i32,
                end.min(self.terminal.screen_lines()) as i32,
            ));
        }
        self.terminal.set_scrolling_region(top, bottom);
    }
    fn set_private_mode(&mut self, mode: PrivateMode) {
        let before = self.alternate();
        let before_icon = self.mouse_cursor_stacks.current_icon(before);
        self.terminal.set_private_mode(mode);
        let after = self.alternate();
        if before != after {
            self.graphics.clear(true, i32::MIN, i32::MAX);
            self.semantic_markers.clear(true, i32::MIN, i32::MAX);
            let after_icon = self.mouse_cursor_stacks.current_icon(after);
            if before_icon != after_icon {
                let _ = self
                    .output
                    .send(crate::TerminalEvent::MouseCursorChanged(after_icon));
            }
        }
    }
    fn unset_private_mode(&mut self, mode: PrivateMode) {
        let before = self.alternate();
        let before_icon = self.mouse_cursor_stacks.current_icon(before);
        self.terminal.unset_private_mode(mode);
        let after = self.alternate();
        if before != after {
            self.graphics.clear(true, i32::MIN, i32::MAX);
            self.semantic_markers.clear(true, i32::MIN, i32::MAX);
            let after_icon = self.mouse_cursor_stacks.current_icon(after);
            if before_icon != after_icon {
                let _ = self
                    .output
                    .send(crate::TerminalEvent::MouseCursorChanged(after_icon));
            }
        }
    }
    fn set_title(&mut self, arg0: Option<String>) {
        self.terminal.set_title(arg0);
    }
    fn set_cursor_style(&mut self, arg0: Option<CursorStyle>) {
        self.terminal.set_cursor_style(arg0);
    }
    fn set_cursor_shape(&mut self, arg0: CursorShape) {
        self.terminal.set_cursor_shape(arg0);
    }
    fn set_mouse_cursor_icon(&mut self, _icon: cursor_icon::CursorIcon) {
        // OSC 22 is handled by the ordered parser so stack and query forms can
        // share one screen-aware state machine.
    }
    fn goto(&mut self, arg0: i32, arg1: usize) {
        self.terminal.goto(arg0, arg1);
    }
    fn goto_line(&mut self, arg0: i32) {
        self.terminal.goto_line(arg0);
    }
    fn goto_col(&mut self, arg0: usize) {
        self.terminal.goto_col(arg0);
    }
    fn insert_blank(&mut self, arg0: usize) {
        self.terminal.insert_blank(arg0);
    }
    fn move_up(&mut self, arg0: usize) {
        self.terminal.move_up(arg0);
    }
    fn move_down(&mut self, arg0: usize) {
        self.terminal.move_down(arg0);
    }
    fn identify_terminal(&mut self, arg0: Option<char>) {
        if arg0.is_none() {
            self.reply("\x1b[?62;4c".into());
        } else {
            self.terminal.identify_terminal(arg0);
        }
    }
    fn device_status(&mut self, arg0: usize) {
        self.terminal.device_status(arg0);
    }
    fn move_forward(&mut self, arg0: usize) {
        self.terminal.move_forward(arg0);
    }
    fn move_backward(&mut self, arg0: usize) {
        self.terminal.move_backward(arg0);
    }
    fn move_down_and_cr(&mut self, arg0: usize) {
        self.terminal.move_down_and_cr(arg0);
    }
    fn move_up_and_cr(&mut self, arg0: usize) {
        self.terminal.move_up_and_cr(arg0);
    }
    fn put_tab(&mut self, arg0: u16) {
        if let Some(capture) = self.clipboard_capture.as_mut() {
            for _ in 0..arg0 {
                capture.push("\t");
            }
        }
        self.terminal.put_tab(arg0);
    }
    fn backspace(&mut self) {
        self.terminal.backspace();
    }
    fn carriage_return(&mut self) {
        if let Some(capture) = self.clipboard_capture.as_mut() {
            capture.push("\r");
        }
        self.terminal.carriage_return();
    }
    fn bell(&mut self) {
        self.terminal.bell();
    }
    fn substitute(&mut self) {
        self.terminal.substitute();
    }
    fn set_horizontal_tabstop(&mut self) {
        self.terminal.set_horizontal_tabstop();
    }
    fn erase_chars(&mut self, arg0: usize) {
        self.terminal.erase_chars(arg0);
    }
    fn delete_chars(&mut self, arg0: usize) {
        self.terminal.delete_chars(arg0);
    }
    fn move_backward_tabs(&mut self, arg0: u16) {
        self.terminal.move_backward_tabs(arg0);
    }
    fn move_forward_tabs(&mut self, arg0: u16) {
        self.terminal.move_forward_tabs(arg0);
    }
    fn save_cursor_position(&mut self) {
        self.terminal.save_cursor_position();
    }
    fn restore_cursor_position(&mut self) {
        self.terminal.restore_cursor_position();
    }
    fn clear_tabs(&mut self, arg0: TabulationClearMode) {
        self.terminal.clear_tabs(arg0);
    }
    fn set_tabs(&mut self, arg0: u16) {
        self.terminal.set_tabs(arg0);
    }
    fn terminal_attribute(&mut self, arg0: Attr) {
        self.terminal.terminal_attribute(arg0);
    }
    fn set_mode(&mut self, arg0: Mode) {
        self.terminal.set_mode(arg0);
    }
    fn unset_mode(&mut self, arg0: Mode) {
        self.terminal.unset_mode(arg0);
    }
    fn report_mode(&mut self, arg0: Mode) {
        self.terminal.report_mode(arg0);
    }
    fn report_private_mode(&mut self, arg0: PrivateMode) {
        self.terminal.report_private_mode(arg0);
    }
    fn set_keypad_application_mode(&mut self) {
        self.terminal.set_keypad_application_mode();
    }
    fn unset_keypad_application_mode(&mut self) {
        self.terminal.unset_keypad_application_mode();
    }
    fn set_active_charset(&mut self, arg0: CharsetIndex) {
        self.terminal.set_active_charset(arg0);
    }
    fn configure_charset(&mut self, arg0: CharsetIndex, arg1: StandardCharset) {
        self.terminal.configure_charset(arg0, arg1);
    }
    fn set_color(&mut self, arg0: usize, arg1: Rgb) {
        self.terminal.set_color(arg0, arg1);
    }
    fn dynamic_color_sequence(&mut self, arg0: String, arg1: usize, arg2: &str) {
        if let Some(color) =
            super::super::alacritty::resolved_color(self.terminal, self.default_colors, arg1)
        {
            self.reply(format!(
                "\x1b]{arg0};rgb:{0:02x}{0:02x}/{1:02x}{1:02x}/{2:02x}{2:02x}{arg2}",
                color.r, color.g, color.b
            ));
        }
    }
    fn reset_color(&mut self, arg0: usize) {
        self.terminal.reset_color(arg0);
    }
    fn clipboard_store(&mut self, arg0: u8, arg1: &[u8]) {
        const MAX_BASE64_BYTES: usize = crate::MAX_OSC52_COPY_BYTES.div_ceil(3) * 4;
        if !self.allow_osc52_copy
            || !matches!(arg0, b'c' | b'p' | b's')
            || arg1.len() > MAX_BASE64_BYTES
        {
            return;
        }
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(arg1) else {
            return;
        };
        if bytes.len() > crate::MAX_OSC52_COPY_BYTES {
            return;
        }
        if let Ok(text) = String::from_utf8(bytes) {
            let _ = self.output.send(crate::TerminalEvent::ClipboardStore(text));
        }
    }
    fn clipboard_load(&mut self, _clipboard: u8, _terminator: &str) {
        // Never expose clipboard contents to terminal output, even when writes are enabled.
    }
    fn decaln(&mut self) {
        self.terminal.decaln();
    }
    fn push_title(&mut self) {
        self.terminal.push_title();
    }
    fn pop_title(&mut self) {
        self.terminal.pop_title();
    }
    fn text_area_size_pixels(&mut self) {
        self.reply(format!(
            "\x1b[4;{};{}t",
            self.terminal.screen_lines() * usize::from(self.graphics.cell_size.1),
            self.terminal.columns() * usize::from(self.graphics.cell_size.0)
        ));
    }
    fn text_area_size_chars(&mut self) {
        self.reply(format!(
            "\x1b[8;{};{}t",
            self.terminal.screen_lines(),
            self.terminal.columns()
        ));
    }
    fn set_hyperlink(&mut self, arg0: Option<Hyperlink>) {
        self.terminal.set_hyperlink(arg0);
    }
    fn report_keyboard_mode(&mut self) {
        self.terminal.report_keyboard_mode();
    }
    fn push_keyboard_mode(&mut self, arg0: KeyboardModes) {
        self.terminal.push_keyboard_mode(arg0);
    }
    fn pop_keyboard_modes(&mut self, arg0: u16) {
        self.terminal.pop_keyboard_modes(arg0);
    }
    fn set_keyboard_mode(&mut self, arg0: KeyboardModes, arg1: KeyboardModesApplyBehavior) {
        self.terminal.set_keyboard_mode(arg0, arg1);
    }
    fn set_modify_other_keys(&mut self, arg0: ModifyOtherKeys) {
        self.terminal.set_modify_other_keys(arg0);
    }
    fn report_modify_other_keys(&mut self) {
        self.terminal.report_modify_other_keys();
    }
    fn set_scp(&mut self, arg0: ScpCharPath, arg1: ScpUpdateMode) {
        self.terminal.set_scp(arg0, arg1);
    }
}
impl<E: EventListener> GraphicsHandler<'_, E> {
    fn reply(&self, text: String) {
        let _ = self.output.send(crate::TerminalEvent::PtyWrite(text));
    }
    fn alternate(&self) -> bool {
        self.terminal.mode().contains(TermMode::ALT_SCREEN)
    }
    fn region(&self) -> (i32, i32) {
        self.graphics
            .region
            .unwrap_or((0, self.terminal.screen_lines() as i32))
    }
    fn track_linefeed(&mut self) {
        if self.terminal.grid().cursor.point.line.0 + 1 == self.region().1 {
            self.track_scroll(1, false, false);
        }
    }
    fn track_scroll(&mut self, count: usize, down: bool, from_cursor: bool) {
        let (mut top, bottom) = self.region();
        if from_cursor {
            let row = self.terminal.grid().cursor.point.line.0;
            if row < top || row >= bottom {
                return;
            }
            top = row;
        }
        let amount = count.min((bottom - top).max(0) as usize) as i32;
        let alternate = self.alternate();
        let amount = if down { -amount } else { amount };
        let history = self.terminal.history_size().saturating_add(count);
        self.graphics
            .scroll(alternate, top, bottom, amount, history);
        self.semantic_markers
            .scroll(alternate, top, bottom, amount, history);
    }
}
