// Explicit forwarding keeps Alacritty as the owner of all text semantics.
use super::Graphics;
use alacritty_terminal::vte::ansi::*;
use alacritty_terminal::{Term, event::EventListener, grid::Dimensions, term::TermMode};
use unicode_width::UnicodeWidthChar;
pub(crate) struct GraphicsHandler<'a, E> {
    pub terminal: &'a mut Term<E>,
    pub output: &'a std::sync::mpsc::Sender<crate::TerminalEvent>,
    pub graphics: &'a mut Graphics,
}
impl<E: EventListener> Handler for GraphicsHandler<'_, E> {
    fn input(&mut self, c: char) {
        if !self.graphics.has_placements() {
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
        self.track_linefeed();
        self.terminal.linefeed();
    }
    fn newline(&mut self) {
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
        self.terminal.clear_screen(mode);
    }
    fn clear_line(&mut self, mode: LineClearMode) {
        let row = self.terminal.grid().cursor.point.line.0;
        self.graphics.clear(self.alternate(), row, row + 1);
        self.terminal.clear_line(mode);
    }
    fn reset_state(&mut self) {
        self.graphics.reset();
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
        self.terminal.set_private_mode(mode);
        if before != self.alternate() {
            self.graphics.clear(true, i32::MIN, i32::MAX);
        }
    }
    fn unset_private_mode(&mut self, mode: PrivateMode) {
        let before = self.alternate();
        self.terminal.unset_private_mode(mode);
        if before != self.alternate() {
            self.graphics.clear(true, i32::MIN, i32::MAX);
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
        self.terminal.put_tab(arg0);
    }
    fn backspace(&mut self) {
        self.terminal.backspace();
    }
    fn carriage_return(&mut self) {
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
        self.terminal.dynamic_color_sequence(arg0, arg1, arg2);
    }
    fn reset_color(&mut self, arg0: usize) {
        self.terminal.reset_color(arg0);
    }
    fn clipboard_store(&mut self, arg0: u8, arg1: &[u8]) {
        self.terminal.clipboard_store(arg0, arg1);
    }
    fn clipboard_load(&mut self, arg0: u8, arg1: &str) {
        self.terminal.clipboard_load(arg0, arg1);
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
        self.graphics.scroll(
            self.alternate(),
            top,
            bottom,
            if down { -amount } else { amount },
            self.terminal.history_size().saturating_add(count),
        );
    }
}
