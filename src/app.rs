use std::fmt;
use std::io::Read;
use std::sync::Arc;
use std::thread;

use arboard::Clipboard;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use crate::{
    AlacrittyTerminalBackend, GpuRenderer, KeyModifiers, KeyPress, MouseWheelDirection, NativePty,
    Pty, PtyCommand, PtySession, PtySize, TerminalBackend, TerminalKey, TextLayout, encode_key,
    encode_mouse_wheel, encode_paste,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellMetrics {
    pub width: f64,
    pub height: f64,
    pub horizontal_padding: u32,
    pub vertical_padding: u32,
}

impl Default for CellMetrics {
    fn default() -> Self {
        Self {
            width: 9.0,
            height: 18.0,
            horizontal_padding: 8,
            vertical_padding: 8,
        }
    }
}

impl CellMetrics {
    pub fn terminal_size(self, window_size: PhysicalSize<u32>) -> PtySize {
        self.terminal_size_at_scale(window_size, 1.0)
    }

    pub fn terminal_size_at_scale(
        self,
        window_size: PhysicalSize<u32>,
        scale_factor: f64,
    ) -> PtySize {
        let scale_factor = scale_factor.max(0.1);
        let horizontal_padding = (f64::from(self.horizontal_padding) * scale_factor).round() as u32;
        let vertical_padding = (f64::from(self.vertical_padding) * scale_factor).round() as u32;
        let content_width = window_size
            .width
            .saturating_sub(horizontal_padding.saturating_mul(2));
        let content_height = window_size
            .height
            .saturating_sub(vertical_padding.saturating_mul(2));
        let columns =
            (f64::from(content_width) / (self.width * scale_factor).max(1.0)).floor() as u32;
        let rows =
            (f64::from(content_height) / (self.height * scale_factor).max(1.0)).floor() as u32;
        PtySize::new(
            columns.clamp(2, u16::MAX.into()) as u16,
            rows.clamp(1, u16::MAX.into()) as u16,
        )
    }

    pub fn text_layout(self, scale_factor: f64) -> TextLayout {
        let scale = scale_factor.max(0.1) as f32;
        TextLayout {
            font_size: 14.0 * scale,
            line_height: self.height as f32 * scale,
            cell_width: self.width as f32 * scale,
            horizontal_padding: self.horizontal_padding as f32 * scale,
            vertical_padding: self.vertical_padding as f32 * scale,
        }
    }
}

#[derive(Debug)]
pub struct AppError(String);

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for AppError {}

pub fn run_gui() -> Result<(), AppError> {
    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .map_err(|error| AppError(error.to_string()))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = ToyotermApplication::new(event_loop.create_proxy());
    event_loop
        .run_app(&mut app)
        .map_err(|error| AppError(error.to_string()))?;
    match app.fatal_error {
        Some(error) => Err(AppError(error)),
        None => Ok(()),
    }
}

#[derive(Debug)]
enum AppEvent {
    Output(Vec<u8>),
    Eof,
    Error(String),
}

struct ToyotermApplication {
    event_proxy: EventLoopProxy<AppEvent>,
    window: Option<Arc<Window>>,
    renderer: Option<GpuRenderer>,
    terminal: AlacrittyTerminalBackend,
    pty_session: Option<Box<dyn PtySession>>,
    modifiers: ModifiersState,
    mouse_position: PhysicalPosition<f64>,
    wheel_line_accumulator: f64,
    selecting: bool,
    clipboard: Option<Clipboard>,
    cell_metrics: CellMetrics,
    fatal_error: Option<String>,
}

impl ApplicationHandler<AppEvent> for ToyotermApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("toyoterm")
            .with_inner_size(LogicalSize::new(960.0, 600.0))
            .with_min_inner_size(LogicalSize::new(320.0, 180.0));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.fail(event_loop, format!("create window: {error}"));
                return;
            }
        };
        let mut renderer = match pollster::block_on(GpuRenderer::new(window.clone())) {
            Ok(renderer) => renderer,
            Err(error) => {
                self.fail(event_loop, error.to_string());
                return;
            }
        };
        let size = self.resize_terminal(window.inner_size(), window.scale_factor());
        self.sync_renderer(&mut renderer, window.scale_factor());
        window.set_ime_allowed(true);
        window.request_redraw();
        self.renderer = Some(renderer);
        self.window = Some(window);
        if let Err(error) = self.start_shell(size) {
            self.fail(event_loop, error);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.clone() else {
            return;
        };
        if window.id() != window_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size);
                }
                let terminal_size = self.resize_terminal(size, window.scale_factor());
                if let Err(error) = self.resize_pty(terminal_size) {
                    self.fail(event_loop, error);
                    return;
                }
                self.sync_active_renderer(window.scale_factor());
                window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let size = window.inner_size();
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size);
                }
                let terminal_size = self.resize_terminal(size, scale_factor);
                if let Err(error) = self.resize_pty(terminal_size) {
                    self.fail(event_loop, error);
                    return;
                }
                self.sync_active_renderer(scale_factor);
                window.request_redraw();
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_position = position;
                if self.selecting {
                    let (column, row) = self.mouse_cell(window.scale_factor());
                    self.terminal.update_selection(column, row);
                    self.sync_active_renderer(window.scale_factor());
                    window.request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                self.handle_left_mouse(&window, state);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.handle_mouse_wheel(event_loop, &window, delta);
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if is_clipboard_shortcut(&event, self.modifiers, 'c') {
                    if let Err(error) = self.copy_selection() {
                        eprintln!("toyoterm: {error}");
                    }
                    return;
                }
                if is_clipboard_shortcut(&event, self.modifiers, 'v') {
                    if let Err(error) = self.paste_clipboard() {
                        eprintln!("toyoterm: {error}");
                    }
                    return;
                }
                if let Some(press) = key_press(&event, self.modifiers)
                    && let Some(bytes) = encode_key(&press, self.terminal.mode())
                    && let Err(error) = self.write_pty(&bytes)
                {
                    self.fail(event_loop, error);
                }
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                if let Err(error) = self.write_pty(text.as_bytes()) {
                    self.fail(event_loop, error);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(renderer) = self.renderer.as_mut()
                    && let Err(error) = renderer.render()
                {
                    self.fail(event_loop, error.to_string());
                }
            }
            _ => {}
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Output(bytes) => {
                self.terminal.advance(&bytes);
                if let Some(window) = self.window.clone() {
                    self.sync_active_renderer(window.scale_factor());
                    window.request_redraw();
                }
            }
            AppEvent::Eof => event_loop.exit(),
            AppEvent::Error(error) => self.fail(event_loop, error),
        }
    }
}

impl ToyotermApplication {
    fn new(event_proxy: EventLoopProxy<AppEvent>) -> Self {
        Self {
            event_proxy,
            window: None,
            renderer: None,
            terminal: AlacrittyTerminalBackend::default(),
            pty_session: None,
            modifiers: ModifiersState::empty(),
            mouse_position: PhysicalPosition::new(0.0, 0.0),
            wheel_line_accumulator: 0.0,
            selecting: false,
            clipboard: None,
            cell_metrics: CellMetrics::default(),
            fatal_error: None,
        }
    }

    fn start_shell(&mut self, size: PtySize) -> Result<(), String> {
        let mut command = PtyCommand::default_shell();
        command.env("TERM", "xterm-256color");
        let mut session = NativePty
            .spawn(command, size)
            .map_err(|error| error.to_string())?;
        let reader = session.take_reader().map_err(|error| error.to_string())?;
        spawn_pty_reader(reader, self.event_proxy.clone())?;
        self.pty_session = Some(session);
        Ok(())
    }

    fn resize_terminal(&mut self, window_size: PhysicalSize<u32>, scale_factor: f64) -> PtySize {
        let size = self
            .cell_metrics
            .terminal_size_at_scale(window_size, scale_factor);
        self.terminal.resize(size.columns, size.rows);
        size
    }

    fn resize_pty(&mut self, size: PtySize) -> Result<(), String> {
        if let Some(session) = self.pty_session.as_mut() {
            session.resize(size).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn write_pty(&mut self, bytes: &[u8]) -> Result<(), String> {
        if let Some(session) = self.pty_session.as_mut() {
            session.write(bytes).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn handle_mouse_wheel(
        &mut self,
        event_loop: &ActiveEventLoop,
        window: &Window,
        delta: MouseScrollDelta,
    ) {
        let lines = match delta {
            MouseScrollDelta::LineDelta(_, vertical) => f64::from(vertical),
            MouseScrollDelta::PixelDelta(position) => {
                position.y / (self.cell_metrics.height * window.scale_factor()).max(1.0)
            }
        };
        self.wheel_line_accumulator += lines;
        let steps = self.wheel_line_accumulator.trunc() as i32;
        self.wheel_line_accumulator -= f64::from(steps);
        if steps == 0 {
            return;
        }

        let mode = self.terminal.mode();
        if mode.mouse_reporting && !self.modifiers.shift_key() {
            let (column, row) = self.mouse_cell(window.scale_factor());
            let direction = if steps > 0 {
                MouseWheelDirection::Up
            } else {
                MouseWheelDirection::Down
            };
            let modifiers = key_modifiers(self.modifiers);
            let sequence = encode_mouse_wheel(direction, column, row, modifiers, mode.sgr_mouse);
            let mut bytes = Vec::with_capacity(sequence.len() * steps.unsigned_abs() as usize);
            for _ in 0..steps.unsigned_abs() {
                bytes.extend_from_slice(&sequence);
            }
            if let Err(error) = self.write_pty(&bytes) {
                self.fail(event_loop, error);
            }
        } else {
            self.terminal.scroll_display(steps);
            self.sync_active_renderer(window.scale_factor());
            window.request_redraw();
        }
    }

    fn mouse_cell(&self, scale_factor: f64) -> (u16, u16) {
        let scale_factor = scale_factor.max(0.1);
        let x = (self.mouse_position.x
            - f64::from(self.cell_metrics.horizontal_padding) * scale_factor)
            .max(0.0);
        let y = (self.mouse_position.y
            - f64::from(self.cell_metrics.vertical_padding) * scale_factor)
            .max(0.0);
        let column = (x / (self.cell_metrics.width * scale_factor).max(1.0)).floor() as u32;
        let row = (y / (self.cell_metrics.height * scale_factor).max(1.0)).floor() as u32;
        (
            column.min(u16::MAX.into()) as u16,
            row.min(u16::MAX.into()) as u16,
        )
    }

    fn handle_left_mouse(&mut self, window: &Window, state: ElementState) {
        if self.terminal.mode().mouse_reporting && !self.modifiers.shift_key() {
            return;
        }

        let (column, row) = self.mouse_cell(window.scale_factor());
        match state {
            ElementState::Pressed => {
                self.terminal.clear_selection();
                self.terminal.start_selection(column, row);
                self.selecting = true;
            }
            ElementState::Released if self.selecting => {
                self.terminal.update_selection(column, row);
                self.selecting = false;
            }
            ElementState::Released => return,
        }
        self.sync_active_renderer(window.scale_factor());
        window.request_redraw();
    }

    fn copy_selection(&mut self) -> Result<(), String> {
        let Some(text) = self
            .terminal
            .selected_text()
            .filter(|text| !text.is_empty())
        else {
            return Ok(());
        };
        self.clipboard()?
            .set_text(text)
            .map_err(|error| format!("copy to clipboard: {error}"))
    }

    fn paste_clipboard(&mut self) -> Result<(), String> {
        let text = self
            .clipboard()?
            .get_text()
            .map_err(|error| format!("paste from clipboard: {error}"))?;
        let bytes = encode_paste(&text, self.terminal.mode());
        self.write_pty(&bytes)
    }

    fn clipboard(&mut self) -> Result<&mut Clipboard, String> {
        if self.clipboard.is_none() {
            self.clipboard =
                Some(Clipboard::new().map_err(|error| format!("initialize clipboard: {error}"))?);
        }
        Ok(self.clipboard.as_mut().expect("clipboard was initialized"))
    }

    fn sync_active_renderer(&mut self, scale_factor: f64) {
        let snapshot = self.terminal.snapshot();
        let cursor = self.terminal.cursor();
        let layout = self.cell_metrics.text_layout(scale_factor);
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.update_terminal(&snapshot, cursor, layout);
        }
    }

    fn sync_renderer(&self, renderer: &mut GpuRenderer, scale_factor: f64) {
        renderer.update_terminal(
            &self.terminal.snapshot(),
            self.terminal.cursor(),
            self.cell_metrics.text_layout(scale_factor),
        );
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.fatal_error = Some(error);
        event_loop.exit();
    }
}

fn spawn_pty_reader(
    mut reader: Box<dyn Read + Send>,
    event_proxy: EventLoopProxy<AppEvent>,
) -> Result<(), String> {
    thread::Builder::new()
        .name("toyoterm-pty-reader".into())
        .spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => {
                        let _ = event_proxy.send_event(AppEvent::Eof);
                        break;
                    }
                    Ok(count) => {
                        if event_proxy
                            .send_event(AppEvent::Output(buffer[..count].to_vec()))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(error) if error.raw_os_error() == Some(5) => {
                        let _ = event_proxy.send_event(AppEvent::Eof);
                        break;
                    }
                    Err(error) => {
                        let _ = event_proxy
                            .send_event(AppEvent::Error(format!("read PTY output: {error}")));
                        break;
                    }
                }
            }
        })
        .map(|_| ())
        .map_err(|error| format!("start PTY reader: {error}"))
}

fn key_press(event: &KeyEvent, modifiers: ModifiersState) -> Option<KeyPress> {
    let key = match &event.logical_key {
        Key::Named(named) => named_key(named)?,
        Key::Character(text) => {
            TerminalKey::Text(event.text.as_deref().unwrap_or(text.as_str()).to_owned())
        }
        _ => return None,
    };
    Some(KeyPress::new(key, key_modifiers(modifiers)))
}

fn key_modifiers(modifiers: ModifiersState) -> KeyModifiers {
    KeyModifiers {
        shift: modifiers.shift_key(),
        control: modifiers.control_key(),
        alt: modifiers.alt_key(),
        super_key: modifiers.super_key(),
    }
}

fn is_clipboard_shortcut(event: &KeyEvent, modifiers: ModifiersState, character: char) -> bool {
    let Key::Character(key) = &event.logical_key else {
        return false;
    };
    let primary_modifier =
        modifiers.super_key() || (modifiers.control_key() && modifiers.shift_key());
    primary_modifier && key.eq_ignore_ascii_case(&character.to_string())
}

fn named_key(key: &NamedKey) -> Option<TerminalKey> {
    Some(match key {
        NamedKey::Enter => TerminalKey::Enter,
        NamedKey::Backspace => TerminalKey::Backspace,
        NamedKey::Tab => TerminalKey::Tab,
        NamedKey::Escape => TerminalKey::Escape,
        NamedKey::ArrowUp => TerminalKey::ArrowUp,
        NamedKey::ArrowDown => TerminalKey::ArrowDown,
        NamedKey::ArrowLeft => TerminalKey::ArrowLeft,
        NamedKey::ArrowRight => TerminalKey::ArrowRight,
        NamedKey::Home => TerminalKey::Home,
        NamedKey::End => TerminalKey::End,
        NamedKey::PageUp => TerminalKey::PageUp,
        NamedKey::PageDown => TerminalKey::PageDown,
        NamedKey::Insert => TerminalKey::Insert,
        NamedKey::Delete => TerminalKey::Delete,
        NamedKey::F1 => TerminalKey::Function(1),
        NamedKey::F2 => TerminalKey::Function(2),
        NamedKey::F3 => TerminalKey::Function(3),
        NamedKey::F4 => TerminalKey::Function(4),
        NamedKey::F5 => TerminalKey::Function(5),
        NamedKey::F6 => TerminalKey::Function(6),
        NamedKey::F7 => TerminalKey::Function(7),
        NamedKey::F8 => TerminalKey::Function(8),
        NamedKey::F9 => TerminalKey::Function(9),
        NamedKey::F10 => TerminalKey::Function(10),
        NamedKey::F11 => TerminalKey::Function(11),
        NamedKey::F12 => TerminalKey::Function(12),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_grid_from_physical_window_size() {
        let metrics = CellMetrics::default();
        assert_eq!(
            metrics.terminal_size(PhysicalSize::new(916, 556)),
            PtySize::new(100, 30)
        );
    }

    #[test]
    fn grid_dimensions_never_reach_zero() {
        let metrics = CellMetrics::default();
        assert_eq!(
            metrics.terminal_size(PhysicalSize::new(0, 0)),
            PtySize::new(2, 1)
        );
    }

    #[test]
    fn hidpi_scaling_preserves_logical_grid_size() {
        let metrics = CellMetrics::default();
        let logical = metrics.terminal_size_at_scale(PhysicalSize::new(916, 556), 1.0);
        let hidpi = metrics.terminal_size_at_scale(PhysicalSize::new(1832, 1112), 2.0);
        assert_eq!(logical, hidpi);
    }
}
