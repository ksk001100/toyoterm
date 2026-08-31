use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
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
    AlacrittyTerminalBackend, Command, ConfigManager, GpuRenderer, KeyModifiers, KeyPress,
    MouseWheelDirection, Mux, NativePty, PaneId, Pty, PtyCommand, PtySession, PtySize, RenderStyle,
    TerminalBackend, TerminalKey, TextLayout, encode_key, encode_mouse_wheel, encode_paste,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellMetrics {
    pub width: f64,
    pub height: f64,
    pub horizontal_padding: u32,
    pub vertical_padding: u32,
    pub font_size: f32,
}

impl Default for CellMetrics {
    fn default() -> Self {
        Self {
            width: 9.0,
            height: 18.0,
            horizontal_padding: 8,
            vertical_padding: 8,
            font_size: 14.0,
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
            font_size: self.font_size * scale,
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
    run_gui_with_config_path(None)
}

pub fn run_gui_with_config_path(config_path: Option<&Path>) -> Result<(), AppError> {
    let config_manager =
        ConfigManager::load_startup(config_path).map_err(|error| AppError(error.to_string()))?;
    let config = config_manager.config();
    let render_style = RenderStyle::from_hex(
        &config.font.family,
        &config.colors.background,
        &config.colors.foreground,
        &config.colors.cursor,
        &config.colors.selection,
        config.window_opacity,
    )
    .map_err(|error| AppError(error.to_string()))?;
    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .map_err(|error| AppError(error.to_string()))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = ToyotermApplication::new(event_loop.create_proxy(), config_manager, render_style)
        .map_err(AppError)?;
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
    Output { pane: PaneId, bytes: Vec<u8> },
    Eof { pane: PaneId },
    Error { pane: PaneId, message: String },
}

struct PaneRuntime {
    terminal: AlacrittyTerminalBackend,
    pty_session: Option<Box<dyn PtySession>>,
    process_id: Option<u32>,
    title: String,
    cwd: Option<PathBuf>,
    exited: bool,
}

impl PaneRuntime {
    fn terminate(&mut self) {
        if let Some(mut session) = self.pty_session.take() {
            let _ = session.kill();
        }
        self.exited = true;
    }
}

impl Drop for PaneRuntime {
    fn drop(&mut self) {
        self.terminate();
    }
}

struct ToyotermApplication {
    event_proxy: EventLoopProxy<AppEvent>,
    window: Option<Arc<Window>>,
    renderer: Option<GpuRenderer>,
    pane_runtimes: HashMap<PaneId, PaneRuntime>,
    modifiers: ModifiersState,
    mouse_position: PhysicalPosition<f64>,
    wheel_line_accumulator: f64,
    selecting: bool,
    clipboard: Option<Clipboard>,
    cell_metrics: CellMetrics,
    config_manager: ConfigManager,
    mux: Mux,
    render_style: RenderStyle,
    fatal_error: Option<String>,
}

impl ApplicationHandler<AppEvent> for ToyotermApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("toyoterm")
            .with_transparent(self.config_manager.config().window_opacity < 1.0)
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
        renderer.set_style(self.render_style.clone());
        let size = self
            .cell_metrics
            .terminal_size_at_scale(window.inner_size(), window.scale_factor());
        window.set_ime_allowed(true);
        self.renderer = Some(renderer);
        self.window = Some(window);
        if let Err(error) = self.sync_pane_runtimes(size) {
            self.fail(event_loop, error);
            return;
        }
        if let Err(error) = self.emit_script_event("app_started") {
            self.fail(event_loop, error);
            return;
        }
        self.sync_active_renderer(
            self.window
                .as_ref()
                .expect("window was installed")
                .scale_factor(),
        );
        self.window
            .as_ref()
            .expect("window was installed")
            .request_redraw();
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
                let terminal_size = self.resize_terminals(size, window.scale_factor());
                if let Err(error) = self.resize_ptys(terminal_size) {
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
                let terminal_size = self.resize_terminals(size, scale_factor);
                if let Err(error) = self.resize_ptys(terminal_size) {
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
                    if let Some(terminal) = self.active_terminal_mut() {
                        terminal.update_selection(column, row);
                    }
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
                match self.handle_tab_shortcut(&event) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(error) => {
                        eprintln!("toyoterm: {error}");
                        return;
                    }
                }
                match self.handle_keybinding(&event) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(error) => {
                        self.fail(event_loop, error);
                        return;
                    }
                }
                if let Some(press) = key_press(&event, self.modifiers)
                    && let Some(bytes) = encode_key(
                        &press,
                        self.active_terminal()
                            .map(TerminalBackend::mode)
                            .unwrap_or_default(),
                    )
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

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Output { pane, bytes } => {
                if let Some(runtime) = self.pane_runtimes.get_mut(&pane) {
                    runtime.terminal.advance(&bytes);
                }
                if self.mux.current_pane() == Some(pane)
                    && let Some(window) = self.window.clone()
                {
                    self.sync_active_renderer(window.scale_factor());
                    window.request_redraw();
                }
            }
            AppEvent::Eof { pane } => self.mark_pane_exited(pane, None),
            AppEvent::Error { pane, message } => {
                eprintln!("toyoterm: pane {pane}: {message}");
                self.mark_pane_exited(pane, Some(message));
            }
        }
    }
}

impl ToyotermApplication {
    fn new(
        event_proxy: EventLoopProxy<AppEvent>,
        config_manager: ConfigManager,
        render_style: RenderStyle,
    ) -> Result<Self, String> {
        let mut config_manager = config_manager;
        let mut mux = Mux::new();
        dispatch_script_commands(&mut config_manager, &mut mux)?;
        let config = config_manager.config();
        let font_scale = f64::from(config.font.size) / 14.0;
        Ok(Self {
            event_proxy,
            window: None,
            renderer: None,
            pane_runtimes: HashMap::new(),
            modifiers: ModifiersState::empty(),
            mouse_position: PhysicalPosition::new(0.0, 0.0),
            wheel_line_accumulator: 0.0,
            selecting: false,
            clipboard: None,
            cell_metrics: CellMetrics {
                width: 9.0 * font_scale,
                height: 18.0 * font_scale,
                font_size: config.font.size,
                ..CellMetrics::default()
            },
            config_manager,
            mux,
            render_style,
            fatal_error: None,
        })
    }

    fn start_shell(&mut self, pane: PaneId, size: PtySize) -> Result<PaneRuntime, String> {
        let mut command = match self.config_manager.config().default_shell.as_deref() {
            Some(shell) => PtyCommand::new(shell),
            None => PtyCommand::default_shell(),
        };
        command.env("TERM", "xterm-256color");
        let mut session = NativePty
            .spawn(command, size)
            .map_err(|error| error.to_string())?;
        let reader = session.take_reader().map_err(|error| error.to_string())?;
        let process_id = session.process_id();
        spawn_pty_reader(pane, reader, self.event_proxy.clone())?;
        Ok(PaneRuntime {
            terminal: AlacrittyTerminalBackend::with_scrollback(
                size.columns,
                size.rows,
                self.config_manager.config().scrollback_lines,
            ),
            pty_session: Some(session),
            process_id,
            title: format!("Pane {}", pane.0),
            cwd: std::env::current_dir().ok(),
            exited: false,
        })
    }

    fn flush_mux_input(&mut self) -> Result<(), String> {
        let panes = self.pane_runtimes.keys().copied().collect::<Vec<_>>();
        for pane in panes {
            let bytes = self
                .mux
                .take_pending_input(pane)
                .map_err(|error| error.to_string())?;
            if !bytes.is_empty() {
                self.write_pane_pty(pane, &bytes)?;
            }
        }
        Ok(())
    }

    fn handle_keybinding(&mut self, event: &KeyEvent) -> Result<bool, String> {
        let Some(key) = keybinding_name(&event.logical_key, self.modifiers) else {
            return Ok(false);
        };
        let pane = self
            .mux
            .current_pane()
            .ok_or_else(|| "mux has no current pane".to_owned())?;
        let matched = match self.config_manager.trigger_keybinding(&key, pane) {
            Ok(matched) => matched,
            Err(error) => {
                eprintln!("toyoterm: Ruby key binding error: {error}");
                return Ok(true);
            }
        };
        if !matched {
            return Ok(false);
        }

        if self
            .config_manager
            .take_reload_request()
            .map_err(|error| error.to_string())?
        {
            if let Err(error) = self.reload_config() {
                eprintln!("toyoterm: config reload failed: {error}");
            }
            return Ok(true);
        }

        dispatch_script_commands(&mut self.config_manager, &mut self.mux)?;
        self.reconcile_pane_runtimes()?;
        self.flush_mux_input()?;
        Ok(true)
    }

    fn handle_tab_shortcut(&mut self, event: &KeyEvent) -> Result<bool, String> {
        if self.modifiers.control_key() && self.modifiers.shift_key() {
            if matches!(&event.logical_key, Key::Character(key) if key.eq_ignore_ascii_case("t")) {
                self.dispatch_gui_command(Command::NewTab)?;
                return Ok(true);
            }
            if matches!(&event.logical_key, Key::Character(key) if key.eq_ignore_ascii_case("w")) {
                let tab = self
                    .mux
                    .current_tab()
                    .ok_or_else(|| "mux has no current tab".to_owned())?;
                self.dispatch_gui_command(Command::CloseTab(tab))?;
                return Ok(true);
            }
        }

        if self.modifiers.control_key() && matches!(&event.logical_key, Key::Named(NamedKey::Tab)) {
            self.cycle_tab(self.modifiers.shift_key())?;
            return Ok(true);
        }
        Ok(false)
    }

    fn cycle_tab(&mut self, backwards: bool) -> Result<(), String> {
        let window = self
            .mux
            .current_window()
            .ok_or_else(|| "mux has no current window".to_owned())?;
        let current = self
            .mux
            .current_tab()
            .ok_or_else(|| "mux has no current tab".to_owned())?;
        let tabs = self
            .mux
            .tabs(window)
            .ok_or_else(|| format!("unknown window {window}"))?;
        let current_index = tabs
            .iter()
            .position(|tab| *tab == current)
            .ok_or_else(|| format!("active tab {current} is not in window {window}"))?;
        let next_index = if backwards {
            (current_index + tabs.len() - 1) % tabs.len()
        } else {
            (current_index + 1) % tabs.len()
        };
        let next = tabs[next_index];
        self.dispatch_gui_command(Command::ActivateTab(next))
    }

    fn dispatch_gui_command(&mut self, command: Command) -> Result<(), String> {
        self.mux
            .dispatch(command)
            .map_err(|error| error.to_string())?;
        self.reconcile_pane_runtimes()?;
        if let Some(window) = self.window.clone() {
            self.sync_active_renderer(window.scale_factor());
            window.request_redraw();
        }
        Ok(())
    }

    fn reload_config(&mut self) -> Result<(), String> {
        let config = self
            .config_manager
            .reload_file()
            .map_err(|error| error.to_string())?
            .clone();
        let render_style = RenderStyle::from_hex(
            &config.font.family,
            &config.colors.background,
            &config.colors.foreground,
            &config.colors.cursor,
            &config.colors.selection,
            config.window_opacity,
        )
        .map_err(|error| error.to_string())?;
        // A reload request in the newly loaded file is not carried into later callbacks.
        self.config_manager
            .take_reload_request()
            .map_err(|error| error.to_string())?;
        dispatch_script_commands(&mut self.config_manager, &mut self.mux)?;
        self.reconcile_pane_runtimes()?;

        let font_scale = f64::from(config.font.size) / 14.0;
        self.cell_metrics.width = 9.0 * font_scale;
        self.cell_metrics.height = 18.0 * font_scale;
        self.cell_metrics.font_size = config.font.size;
        for runtime in self.pane_runtimes.values_mut() {
            runtime
                .terminal
                .set_scrollback_lines(config.scrollback_lines);
        }
        self.render_style = render_style.clone();
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_style(render_style);
        }
        if let Some(window) = self.window.clone() {
            window.set_transparent(config.window_opacity < 1.0);
            let size = self.resize_terminals(window.inner_size(), window.scale_factor());
            self.resize_ptys(size)?;
            self.sync_active_renderer(window.scale_factor());
            window.request_redraw();
        }
        self.emit_script_event("config_reloaded")?;
        self.flush_mux_input()?;
        Ok(())
    }

    fn emit_script_event(&mut self, name: &str) -> Result<(), String> {
        let pane = self
            .mux
            .current_pane()
            .ok_or_else(|| "mux has no current pane".to_owned())?;
        match self.config_manager.emit_event(name, pane) {
            Ok(true) => {
                dispatch_script_commands(&mut self.config_manager, &mut self.mux)?;
                self.reconcile_pane_runtimes()?;
                self.flush_mux_input()
            }
            Ok(false) => Ok(()),
            Err(error) => {
                eprintln!("toyoterm: Ruby `{name}` event error: {error}");
                Ok(())
            }
        }
    }

    fn resize_terminals(&mut self, window_size: PhysicalSize<u32>, scale_factor: f64) -> PtySize {
        let size = self
            .cell_metrics
            .terminal_size_at_scale(window_size, scale_factor);
        for runtime in self.pane_runtimes.values_mut() {
            runtime.terminal.resize(size.columns, size.rows);
        }
        size
    }

    fn resize_ptys(&mut self, size: PtySize) -> Result<(), String> {
        for runtime in self.pane_runtimes.values_mut() {
            if let Some(session) = runtime.pty_session.as_mut() {
                session.resize(size).map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }

    fn write_pty(&mut self, bytes: &[u8]) -> Result<(), String> {
        let pane = self
            .mux
            .current_pane()
            .ok_or_else(|| "mux has no current pane".to_owned())?;
        self.write_pane_pty(pane, bytes)
    }

    fn write_pane_pty(&mut self, pane: PaneId, bytes: &[u8]) -> Result<(), String> {
        let runtime = self
            .pane_runtimes
            .get_mut(&pane)
            .ok_or_else(|| format!("pane {pane} has no runtime"))?;
        if let Some(session) = runtime.pty_session.as_mut() {
            session.write(bytes).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn reconcile_pane_runtimes(&mut self) -> Result<(), String> {
        let size = self
            .window
            .as_ref()
            .map(|window| {
                self.cell_metrics
                    .terminal_size_at_scale(window.inner_size(), window.scale_factor())
            })
            .unwrap_or_default();
        self.sync_pane_runtimes(size)
    }

    fn sync_pane_runtimes(&mut self, size: PtySize) -> Result<(), String> {
        let desired = self.mux.pane_ids().collect::<HashSet<_>>();
        let stale = self
            .pane_runtimes
            .keys()
            .filter(|pane| !desired.contains(pane))
            .copied()
            .collect::<Vec<_>>();
        for pane in stale {
            if let Some(mut runtime) = self.pane_runtimes.remove(&pane) {
                runtime.terminate();
            }
        }

        let mut missing = desired
            .into_iter()
            .filter(|pane| !self.pane_runtimes.contains_key(pane))
            .collect::<Vec<_>>();
        missing.sort_unstable();
        for pane in missing {
            let runtime = self.start_shell(pane, size)?;
            self.pane_runtimes.insert(pane, runtime);
        }
        self.flush_mux_input()
    }

    fn active_terminal(&self) -> Option<&AlacrittyTerminalBackend> {
        self.mux
            .current_pane()
            .and_then(|pane| self.pane_runtimes.get(&pane))
            .map(|runtime| &runtime.terminal)
    }

    fn active_terminal_mut(&mut self) -> Option<&mut AlacrittyTerminalBackend> {
        let pane = self.mux.current_pane()?;
        self.pane_runtimes
            .get_mut(&pane)
            .map(|runtime| &mut runtime.terminal)
    }

    fn mark_pane_exited(&mut self, pane: PaneId, error: Option<String>) {
        if let Some(runtime) = self.pane_runtimes.get_mut(&pane) {
            runtime.pty_session = None;
            runtime.exited = true;
            runtime.title = match error {
                Some(error) => format!("Pane {} (error: {error})", pane.0),
                None => format!("Pane {} (exited)", pane.0),
            };
        }
        if self.mux.current_pane() == Some(pane)
            && let Some(window) = self.window.clone()
        {
            self.sync_active_renderer(window.scale_factor());
            window.request_redraw();
        }
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

        let mode = self
            .active_terminal()
            .map(TerminalBackend::mode)
            .unwrap_or_default();
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
            if let Some(terminal) = self.active_terminal_mut() {
                terminal.scroll_display(steps);
            }
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
        if self
            .active_terminal()
            .is_some_and(|terminal| terminal.mode().mouse_reporting)
            && !self.modifiers.shift_key()
        {
            return;
        }

        let (column, row) = self.mouse_cell(window.scale_factor());
        match state {
            ElementState::Pressed => {
                if let Some(terminal) = self.active_terminal_mut() {
                    terminal.clear_selection();
                    terminal.start_selection(column, row);
                }
                self.selecting = true;
            }
            ElementState::Released if self.selecting => {
                if let Some(terminal) = self.active_terminal_mut() {
                    terminal.update_selection(column, row);
                }
                self.selecting = false;
            }
            ElementState::Released => return,
        }
        self.sync_active_renderer(window.scale_factor());
        window.request_redraw();
    }

    fn copy_selection(&mut self) -> Result<(), String> {
        let Some(text) = self
            .active_terminal()
            .and_then(TerminalBackend::selected_text)
            .filter(|text| !text.is_empty())
        else {
            return Ok(());
        };
        self.clipboard()?
            .set_text(text)
            .map_err(|error| format!("copy to clipboard: {error}"))
    }

    fn paste_clipboard(&mut self) -> Result<(), String> {
        let mode = self
            .active_terminal()
            .map(TerminalBackend::mode)
            .unwrap_or_default();
        let text = self
            .clipboard()?
            .get_text()
            .map_err(|error| format!("paste from clipboard: {error}"))?;
        let bytes = encode_paste(&text, mode);
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
        let Some(terminal) = self.active_terminal() else {
            return;
        };
        let snapshot = terminal.snapshot();
        let cursor = terminal.cursor();
        let layout = self.cell_metrics.text_layout(scale_factor);
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.update_terminal(&snapshot, cursor, layout);
        }
        self.update_window_title();
    }

    fn update_window_title(&self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(pane) = self.mux.current_pane() else {
            window.set_title("toyoterm");
            return;
        };
        let Some(runtime) = self.pane_runtimes.get(&pane) else {
            window.set_title("toyoterm");
            return;
        };
        let pid = runtime
            .process_id
            .map(|pid| format!(" · pid {pid}"))
            .unwrap_or_default();
        let cwd = runtime
            .cwd
            .as_ref()
            .map(|cwd| format!(" · {}", cwd.display()))
            .unwrap_or_default();
        window.set_title(&format!("toyoterm — {}{pid}{cwd}", runtime.title));
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.fatal_error = Some(error);
        event_loop.exit();
    }
}

fn dispatch_script_commands(
    config_manager: &mut ConfigManager,
    mux: &mut Mux,
) -> Result<(), String> {
    let current_pane = mux
        .current_pane()
        .ok_or_else(|| "mux has no current pane".to_owned())?;
    config_manager
        .set_current_pane(current_pane)
        .map_err(|error| error.to_string())?;
    for command in config_manager
        .drain_commands(current_pane)
        .map_err(|error| error.to_string())?
    {
        mux.dispatch(command).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn spawn_pty_reader(
    pane: PaneId,
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
                        let _ = event_proxy.send_event(AppEvent::Eof { pane });
                        break;
                    }
                    Ok(count) => {
                        if event_proxy
                            .send_event(AppEvent::Output {
                                pane,
                                bytes: buffer[..count].to_vec(),
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(error) if error.raw_os_error() == Some(5) => {
                        let _ = event_proxy.send_event(AppEvent::Eof { pane });
                        break;
                    }
                    Err(error) => {
                        let _ = event_proxy.send_event(AppEvent::Error {
                            pane,
                            message: format!("read PTY output: {error}"),
                        });
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

fn keybinding_name(logical_key: &Key, modifiers: ModifiersState) -> Option<String> {
    let key = match logical_key {
        Key::Character(text) if !text.is_empty() => text.to_uppercase(),
        Key::Named(key) => keybinding_named_key(key)?.to_owned(),
        _ => return None,
    };
    let mut parts = Vec::with_capacity(5);
    if modifiers.control_key() {
        parts.push("CTRL".to_owned());
    }
    if modifiers.shift_key() {
        parts.push("SHIFT".to_owned());
    }
    if modifiers.alt_key() {
        parts.push("ALT".to_owned());
    }
    if modifiers.super_key() {
        parts.push("SUPER".to_owned());
    }
    parts.push(key);
    Some(parts.join("+"))
}

fn keybinding_named_key(key: &NamedKey) -> Option<&'static str> {
    Some(match key {
        NamedKey::Enter => "ENTER",
        NamedKey::Backspace => "BACKSPACE",
        NamedKey::Tab => "TAB",
        NamedKey::Escape => "ESCAPE",
        NamedKey::Space => "SPACE",
        NamedKey::ArrowUp => "UP",
        NamedKey::ArrowDown => "DOWN",
        NamedKey::ArrowLeft => "LEFT",
        NamedKey::ArrowRight => "RIGHT",
        NamedKey::Home => "HOME",
        NamedKey::End => "END",
        NamedKey::PageUp => "PAGEUP",
        NamedKey::PageDown => "PAGEDOWN",
        NamedKey::Insert => "INSERT",
        NamedKey::Delete => "DELETE",
        NamedKey::F1 => "F1",
        NamedKey::F2 => "F2",
        NamedKey::F3 => "F3",
        NamedKey::F4 => "F4",
        NamedKey::F5 => "F5",
        NamedKey::F6 => "F6",
        NamedKey::F7 => "F7",
        NamedKey::F8 => "F8",
        NamedKey::F9 => "F9",
        NamedKey::F10 => "F10",
        NamedKey::F11 => "F11",
        NamedKey::F12 => "F12",
        _ => return None,
    })
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
        NamedKey::Space => TerminalKey::Text(" ".into()),
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

    #[test]
    fn dispatches_startup_ruby_commands_through_the_mux() {
        let mut config_manager = ConfigManager::new().unwrap();
        config_manager
            .reload(r#"Toyoterm.current_pane.send_text("echo hello\n")"#)
            .unwrap();
        let mut mux = Mux::new();
        let pane = mux.current_pane().unwrap();

        dispatch_script_commands(&mut config_manager, &mut mux).unwrap();

        assert_eq!(mux.take_pending_input(pane).unwrap(), b"echo hello\n");
    }

    #[test]
    fn normalizes_native_keys_for_the_ruby_binding_resolver() {
        assert_eq!(
            keybinding_name(
                &Key::Character("h".into()),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
            )
            .as_deref(),
            Some("CTRL+SHIFT+H")
        );
        assert_eq!(
            keybinding_name(&Key::Named(NamedKey::F5), ModifiersState::ALT).as_deref(),
            Some("ALT+F5")
        );
    }

    #[test]
    fn encodes_named_space_and_tab_for_the_pty() {
        let mode = crate::TerminalMode::default();
        let space = KeyPress::new(
            named_key(&NamedKey::Space).unwrap(),
            KeyModifiers::default(),
        );
        let tab = KeyPress::new(named_key(&NamedKey::Tab).unwrap(), KeyModifiers::default());

        assert_eq!(encode_key(&space, mode), Some(b" ".to_vec()));
        assert_eq!(encode_key(&tab, mode), Some(b"\t".to_vec()));
    }
}
