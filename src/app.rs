use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use arboard::Clipboard;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::{
    AlacrittyTerminalBackend, BindingKey, Command, CommandMenuLayout, CommandMenuRenderData,
    ConfigErrorLayout, ConfigErrorRenderData, ConfigManager, GpuRenderer, KeyChord, KeyModifiers,
    KeyPress, KeypadKey, MouseWheelDirection, Mux, NativeAction, NativePty, PaneId, PaneLayout,
    PaneRect, PaneRenderData, Pty, PtyCommand, PtySession, PtySize, RenderStyle, SelectionKind,
    SplitDirection, TabRenderData, TabStripLayout, TerminalBackend, TerminalKey, TextLayout,
    WorkspaceRenderData, WorkspaceStripLayout, encode_key, encode_mouse_wheel, encode_paste,
};

const MULTI_CLICK_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ClickTarget {
    pane: PaneId,
    column: u16,
    row: u16,
}

#[derive(Default)]
struct ClickTracker {
    previous: Option<(Instant, ClickTarget, u8)>,
}

impl ClickTracker {
    fn register(&mut self, now: Instant, target: ClickTarget) -> u8 {
        let count = match self.previous {
            Some((previous, previous_target, count))
                if target == previous_target
                    && now.saturating_duration_since(previous) <= MULTI_CLICK_INTERVAL =>
            {
                count % 3 + 1
            }
            _ => 1,
        };
        self.previous = Some((now, target, count));
        count
    }
}

struct ConfigErrorNotice {
    message: String,
    log_expanded: bool,
    console_hint: bool,
}

impl ConfigErrorNotice {
    fn display_message(&self) -> String {
        if self.console_hint {
            return "Ruby Console is not available yet. Use Open Log to inspect the complete error."
                .to_owned();
        }
        if self.log_expanded {
            return self.message.clone();
        }
        self.message.lines().take(3).collect::<Vec<_>>().join("\n")
    }
}

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
    let (config_manager, startup_config_error) =
        ConfigManager::load_startup_recovering(config_path)
            .map_err(|error| AppError(error.to_string()))?;
    let config = config_manager.config();
    let render_style = RenderStyle::from_hex(
        &config.font.family,
        config.font.weight,
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
    let mut app = ToyotermApplication::new(
        event_loop.create_proxy(),
        config_manager,
        render_style,
        startup_config_error.map(|error| error.to_string()),
    )
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
    pane_layout: PaneLayout,
    tab_layout: TabStripLayout,
    workspace_layout: WorkspaceStripLayout,
    command_menu_layout: CommandMenuLayout,
    command_menu_open: bool,
    config_error_layout: ConfigErrorLayout,
    config_error_notice: Option<ConfigErrorNotice>,
    ime_preedit: Option<String>,
    modifiers: ModifiersState,
    alt_graph_active: bool,
    leader_deadline: Option<Instant>,
    mouse_position: PhysicalPosition<f64>,
    wheel_line_accumulator: f64,
    selecting: bool,
    click_tracker: ClickTracker,
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
        self.refresh_pane_layout();
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
                if let Err(error) = self.resize_panes(size, window.scale_factor()) {
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
                if let Err(error) = self.resize_panes(size, scale_factor) {
                    self.fail(event_loop, error);
                    return;
                }
                self.sync_active_renderer(scale_factor);
                window.request_redraw();
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::Focused(focused) => {
                // A platform is not required to send key-release events after
                // the window loses focus. Do not leave modifiers (especially
                // AltGraph) stuck when focus returns.
                if !focused {
                    clear_modifier_state(&mut self.modifiers, &mut self.alt_graph_active);
                    self.leader_deadline = None;
                    if self.command_menu_open {
                        self.command_menu_open = false;
                        self.sync_active_renderer(window.scale_factor());
                        window.request_redraw();
                    }
                }
                if self
                    .active_terminal()
                    .is_some_and(|terminal| terminal.mode().focus_reporting)
                    && let Err(error) = self.write_pty(if focused { b"\x1b[I" } else { b"\x1b[O" })
                {
                    self.fail(event_loop, error);
                }
            }
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
            WindowEvent::KeyboardInput { event, .. } => {
                if matches!(event.logical_key, Key::Named(NamedKey::AltGraph)) {
                    self.alt_graph_active = event.state == ElementState::Pressed;
                    return;
                }
                if !should_handle_key_event(event.state, event.repeat) {
                    return;
                }
                let modifiers = effective_modifiers(self.modifiers, self.alt_graph_active);
                if self.config_error_notice.is_some()
                    && matches!(event.logical_key, Key::Named(NamedKey::Escape))
                {
                    self.config_error_notice = None;
                    if let Err(error) =
                        self.resize_panes(window.inner_size(), window.scale_factor())
                    {
                        self.fail(event_loop, error);
                        return;
                    }
                    self.sync_active_renderer(window.scale_factor());
                    window.request_redraw();
                    return;
                }
                if self.command_menu_open
                    && matches!(event.logical_key, Key::Named(NamedKey::Escape))
                {
                    self.command_menu_open = false;
                    self.sync_active_renderer(window.scale_factor());
                    window.request_redraw();
                    return;
                }
                match self.handle_leader_key(&event, modifiers) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(error) => {
                        eprintln!("toyoterm: {error}");
                        return;
                    }
                }
                if is_clipboard_shortcut(&event, modifiers, 'c') {
                    if let Err(error) = self.copy_selection() {
                        eprintln!("toyoterm: {error}");
                    }
                    return;
                }
                if is_clipboard_shortcut(&event, modifiers, 'v') {
                    if let Err(error) = self.paste_clipboard() {
                        eprintln!("toyoterm: {error}");
                    }
                    return;
                }
                // Configured bindings override built-in GUI shortcuts. Physical
                // bindings are checked before logical bindings by the resolver.
                match self.handle_keybinding(&event, modifiers) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(error) => {
                        eprintln!("toyoterm: {error}");
                        return;
                    }
                }
                match self.handle_tab_shortcut(&event, modifiers) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(error) => {
                        eprintln!("toyoterm: {error}");
                        return;
                    }
                }
                let mode = self
                    .active_terminal()
                    .map(TerminalBackend::mode)
                    .unwrap_or_default();
                if let Some(press) = key_press(&event, modifiers, mode)
                    && let Some(bytes) = encode_key(&press, mode)
                    && let Err(error) = self.write_pty(&bytes)
                {
                    self.fail(event_loop, error);
                }
            }
            WindowEvent::Ime(Ime::Preedit(text, _cursor)) => {
                self.leader_deadline = None;
                self.ime_preedit = (!text.is_empty()).then_some(text);
                self.sync_active_renderer(window.scale_factor());
                window.request_redraw();
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                self.leader_deadline = None;
                self.ime_preedit = None;
                if let Err(error) = self.write_pty(text.as_bytes()) {
                    self.fail(event_loop, error);
                }
                self.sync_active_renderer(window.scale_factor());
                window.request_redraw();
            }
            WindowEvent::Ime(Ime::Disabled) => {
                self.leader_deadline = None;
                self.ime_preedit = None;
                self.sync_active_renderer(window.scale_factor());
                window.request_redraw();
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
                if self.pane_layout.rect(pane).is_some()
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
        startup_config_error: Option<String>,
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
            pane_layout: PaneLayout::default(),
            tab_layout: TabStripLayout::default(),
            workspace_layout: WorkspaceStripLayout::default(),
            command_menu_layout: CommandMenuLayout::default(),
            command_menu_open: false,
            config_error_layout: ConfigErrorLayout::default(),
            config_error_notice: startup_config_error.map(|message| ConfigErrorNotice {
                message,
                log_expanded: false,
                console_hint: false,
            }),
            ime_preedit: None,
            modifiers: ModifiersState::empty(),
            alt_graph_active: false,
            leader_deadline: None,
            mouse_position: PhysicalPosition::new(0.0, 0.0),
            wheel_line_accumulator: 0.0,
            selecting: false,
            click_tracker: ClickTracker::default(),
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

    fn handle_keybinding(
        &mut self,
        event: &KeyEvent,
        modifiers: ModifiersState,
    ) -> Result<bool, String> {
        self.handle_keybinding_candidates(keybinding_names(event, modifiers))
    }

    fn handle_keybinding_candidates(&mut self, keys: Vec<String>) -> Result<bool, String> {
        let pane = self
            .mux
            .current_pane()
            .ok_or_else(|| "mux has no current pane".to_owned())?;
        for key in keys {
            if let Some(action) = self.config_manager.native_action(&key) {
                self.execute_native_action(action)?;
                return Ok(true);
            }
            if !self.config_manager.has_dynamic_keybinding(&key) {
                continue;
            }
            match self.config_manager.trigger_keybinding(&key, pane) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(error) => {
                    eprintln!("toyoterm: Ruby key binding error: {error}");
                    return Ok(true);
                }
            }

            if self
                .config_manager
                .take_reload_request()
                .map_err(|error| error.to_string())?
            {
                self.reload_config_with_notification()?;
                return Ok(true);
            }

            dispatch_script_commands(&mut self.config_manager, &mut self.mux)?;
            self.reconcile_pane_runtimes()?;
            self.flush_mux_input()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn handle_leader_key(
        &mut self,
        event: &KeyEvent,
        modifiers: ModifiersState,
    ) -> Result<bool, String> {
        let now = Instant::now();
        if let Some(deadline) = self.leader_deadline.take() {
            // Repeated events must never extend or accidentally complete a
            // leader sequence.
            if event.repeat {
                return Ok(true);
            }
            if now <= deadline {
                let candidates = keybinding_names(event, modifiers)
                    .into_iter()
                    .map(|key| format!("LEADER+{key}"))
                    .collect();
                if self.handle_keybinding_candidates(candidates)? {
                    return Ok(true);
                }
            }
            // An unmatched or expired suffix is processed normally below by
            // the caller. Only the leader prefix itself is discarded.
            return Ok(false);
        }

        let Some(leader) = self.config_manager.config().leader.as_ref() else {
            return Ok(false);
        };
        let matches_leader = keybinding_names(event, modifiers)
            .iter()
            .any(|key| key == &leader.key);
        if event.repeat {
            // Holding the prefix must not leak repeated prefix bytes to the PTY.
            return Ok(matches_leader);
        }
        if !matches_leader {
            return Ok(false);
        }
        self.leader_deadline = Some(
            now.checked_add(Duration::from_millis(leader.timeout_ms))
                .unwrap_or(now),
        );
        Ok(true)
    }

    fn execute_native_action(&mut self, action: NativeAction) -> Result<(), String> {
        match action {
            NativeAction::NewTab => self.dispatch_gui_command(Command::NewTab),
            NativeAction::ClosePane => {
                let pane = self
                    .mux
                    .current_pane()
                    .ok_or_else(|| "mux has no current pane".to_owned())?;
                self.dispatch_gui_command(Command::ClosePane(pane))
            }
            NativeAction::ReloadConfig => self.reload_config_with_notification(),
            NativeAction::Split(direction) => self.split_active_pane(direction),
            NativeAction::ActivatePane(direction) => self.focus_neighbor(direction),
        }
    }

    fn handle_tab_shortcut(
        &mut self,
        event: &KeyEvent,
        modifiers: ModifiersState,
    ) -> Result<bool, String> {
        if let Some(shortcut) =
            gui_management_shortcut(&event.logical_key, modifiers, current_shortcut_platform())
        {
            match shortcut {
                GuiManagementShortcut::ReloadConfig => {
                    self.reload_config_with_notification()?;
                }
                GuiManagementShortcut::NewTab => {
                    self.dispatch_gui_command(Command::NewTab)?;
                }
                GuiManagementShortcut::NewWorkspace => {
                    self.create_workspace()?;
                }
                GuiManagementShortcut::CloseTab => {
                    let tab = self
                        .mux
                        .current_tab()
                        .ok_or_else(|| "mux has no current tab".to_owned())?;
                    self.dispatch_gui_command(Command::CloseTab(tab))?;
                }
                GuiManagementShortcut::Split(direction) => self.split_active_pane(direction)?,
                GuiManagementShortcut::ClosePane => {
                    let pane = self
                        .mux
                        .current_pane()
                        .ok_or_else(|| "mux has no current pane".to_owned())?;
                    self.dispatch_gui_command(Command::ClosePane(pane))?;
                }
                GuiManagementShortcut::Focus(direction) => self.focus_neighbor(direction)?,
            }
            return Ok(true);
        }

        if modifiers.control_key() && matches!(&event.logical_key, Key::Named(NamedKey::Tab)) {
            self.cycle_tab(modifiers.shift_key())?;
            return Ok(true);
        }
        if modifiers.control_key() && modifiers.alt_key() {
            let backwards = match &event.logical_key {
                Key::Named(NamedKey::ArrowLeft) => Some(true),
                Key::Named(NamedKey::ArrowRight) => Some(false),
                _ => None,
            };
            if let Some(backwards) = backwards {
                self.cycle_workspace(backwards)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn create_workspace(&mut self) -> Result<(), String> {
        let mut suffix = self.mux.workspaces().len() + 1;
        let name = loop {
            let candidate = format!("workspace-{suffix}");
            if self
                .mux
                .workspaces()
                .into_iter()
                .all(|workspace| self.mux.workspace_name(workspace) != Some(candidate.as_str()))
            {
                break candidate;
            }
            suffix += 1;
        };
        self.dispatch_gui_command(Command::SwitchWorkspace(name))
    }

    fn cycle_workspace(&mut self, backwards: bool) -> Result<(), String> {
        let workspaces = self.mux.workspaces();
        let current = self.mux.current_workspace();
        let current_index = workspaces
            .iter()
            .position(|workspace| *workspace == current)
            .ok_or_else(|| format!("active workspace {current} is not registered"))?;
        let next_index = if backwards {
            (current_index + workspaces.len() - 1) % workspaces.len()
        } else {
            (current_index + 1) % workspaces.len()
        };
        self.dispatch_gui_command(Command::ActivateWorkspace(workspaces[next_index]))
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
        let previous_pane = self.mux.current_pane();
        self.mux
            .dispatch(command)
            .map_err(|error| error.to_string())?;
        if self.mux.current_pane() != previous_pane {
            self.ime_preedit = None;
        }
        self.reconcile_pane_runtimes()?;
        if let Some(window) = self.window.clone() {
            self.sync_active_renderer(window.scale_factor());
            window.request_redraw();
        }
        Ok(())
    }

    fn split_active_pane(&mut self, direction: SplitDirection) -> Result<(), String> {
        let pane = self
            .mux
            .current_pane()
            .ok_or_else(|| "mux has no current pane".to_owned())?;
        self.dispatch_gui_command(Command::Split { pane, direction })
    }

    fn focus_neighbor(&mut self, direction: SplitDirection) -> Result<(), String> {
        let pane = self
            .mux
            .current_pane()
            .ok_or_else(|| "mux has no current pane".to_owned())?;
        if let Some(neighbor) = self.pane_layout.neighbor(pane, direction) {
            self.dispatch_gui_command(Command::ActivatePane(neighbor))?;
        }
        Ok(())
    }

    fn reload_config_with_notification(&mut self) -> Result<(), String> {
        match self.reload_config() {
            Ok(()) => self.config_error_notice = None,
            Err(error) => {
                eprintln!("toyoterm: config reload failed: {error}");
                self.config_error_notice = Some(ConfigErrorNotice {
                    message: error,
                    log_expanded: false,
                    console_hint: false,
                });
            }
        }
        if let Some(window) = self.window.clone() {
            self.resize_panes(window.inner_size(), window.scale_factor())?;
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
        self.leader_deadline = None;
        let render_style = RenderStyle::from_hex(
            &config.font.family,
            config.font.weight,
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
            self.resize_panes(window.inner_size(), window.scale_factor())?;
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

    fn resize_panes(
        &mut self,
        window_size: PhysicalSize<u32>,
        scale_factor: f64,
    ) -> Result<(), String> {
        self.tab_layout = self.calculate_tab_layout(window_size, scale_factor);
        self.workspace_layout = self.calculate_workspace_layout(window_size, scale_factor);
        self.command_menu_layout = self.calculate_command_menu_layout(window_size, scale_factor);
        self.config_error_layout = self.calculate_config_error_layout(window_size, scale_factor);
        self.pane_layout = self.calculate_pane_layout(window_size, scale_factor);
        let sizes = self
            .pane_layout
            .panes()
            .iter()
            .map(|placement| {
                (
                    placement.pane,
                    self.cell_metrics.terminal_size_at_scale(
                        PhysicalSize::new(placement.rect.width, placement.rect.height),
                        scale_factor,
                    ),
                )
            })
            .collect::<Vec<_>>();
        for (pane, size) in sizes {
            if let Some(runtime) = self.pane_runtimes.get_mut(&pane) {
                runtime.terminal.resize(size.columns, size.rows);
                if let Some(session) = runtime.pty_session.as_mut() {
                    session.resize(size).map_err(|error| error.to_string())?;
                }
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
        self.refresh_pane_layout();
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

    fn refresh_pane_layout(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        self.tab_layout = self.calculate_tab_layout(window.inner_size(), window.scale_factor());
        self.workspace_layout =
            self.calculate_workspace_layout(window.inner_size(), window.scale_factor());
        self.command_menu_layout =
            self.calculate_command_menu_layout(window.inner_size(), window.scale_factor());
        self.config_error_layout =
            self.calculate_config_error_layout(window.inner_size(), window.scale_factor());
        self.pane_layout = self.calculate_pane_layout(window.inner_size(), window.scale_factor());
    }

    fn calculate_workspace_layout(
        &self,
        window_size: PhysicalSize<u32>,
        scale_factor: f64,
    ) -> WorkspaceStripLayout {
        WorkspaceStripLayout::calculate(
            &self.mux.workspaces(),
            PaneRect::new(
                0,
                0,
                window_size
                    .width
                    .saturating_sub(command_menu_width(scale_factor)),
                workspace_bar_height(scale_factor),
            ),
            (140.0 * scale_factor.max(0.1)).round() as u32,
        )
    }

    fn calculate_command_menu_layout(
        &self,
        window_size: PhysicalSize<u32>,
        scale_factor: f64,
    ) -> CommandMenuLayout {
        CommandMenuLayout::calculate(
            window_size.width,
            workspace_bar_height(scale_factor),
            tab_bar_height(scale_factor),
            command_menu_width(scale_factor),
        )
    }

    fn calculate_config_error_layout(
        &self,
        window_size: PhysicalSize<u32>,
        scale_factor: f64,
    ) -> ConfigErrorLayout {
        let Some(notice) = self.config_error_notice.as_ref() else {
            return ConfigErrorLayout::default();
        };
        let y = workspace_bar_height(scale_factor).saturating_add(tab_bar_height(scale_factor));
        let height = config_error_height(scale_factor, notice.log_expanded)
            .min(window_size.height.saturating_sub(y));
        ConfigErrorLayout::calculate(
            PaneRect::new(0, y, window_size.width, height),
            tab_bar_height(scale_factor),
        )
    }

    fn calculate_tab_layout(
        &self,
        window_size: PhysicalSize<u32>,
        scale_factor: f64,
    ) -> TabStripLayout {
        let Some(window) = self.mux.current_window() else {
            return TabStripLayout::default();
        };
        let Some(tabs) = self.mux.tabs(window) else {
            return TabStripLayout::default();
        };
        TabStripLayout::calculate(
            tabs,
            PaneRect::new(
                0,
                workspace_bar_height(scale_factor),
                window_size
                    .width
                    .saturating_sub(command_menu_width(scale_factor)),
                tab_bar_height(scale_factor),
            ),
            (160.0 * scale_factor.max(0.1)).round() as u32,
        )
    }

    fn calculate_pane_layout(
        &self,
        window_size: PhysicalSize<u32>,
        scale_factor: f64,
    ) -> PaneLayout {
        let Some(tab) = self.mux.current_tab() else {
            return PaneLayout::default();
        };
        let Some(root) = self.mux.pane_tree(tab) else {
            return PaneLayout::default();
        };
        let notification_height = self
            .config_error_notice
            .as_ref()
            .map(|notice| config_error_height(scale_factor, notice.log_expanded))
            .unwrap_or(0);
        let chrome_height = workspace_bar_height(scale_factor)
            .saturating_add(tab_bar_height(scale_factor))
            .saturating_add(notification_height)
            .min(window_size.height);
        PaneLayout::calculate(
            root,
            PaneRect::new(
                0,
                chrome_height,
                window_size.width,
                window_size.height.saturating_sub(chrome_height),
            ),
            (2.0 * scale_factor.max(0.1)).round() as u32,
        )
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
        self.flush_mux_input()?;
        if let Some(window) = self.window.clone() {
            self.resize_panes(window.inner_size(), window.scale_factor())?;
        }
        Ok(())
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
        } else if mode.alternate_screen && mode.alternate_scroll && !self.modifiers.shift_key() {
            let key = if steps > 0 {
                TerminalKey::ArrowUp
            } else {
                TerminalKey::ArrowDown
            };
            let sequence = encode_key(&KeyPress::new(key, KeyModifiers::default()), mode)
                .expect("arrow keys always encode");
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
        let rect = self
            .mux
            .current_pane()
            .and_then(|pane| self.pane_layout.rect(pane))
            .unwrap_or_default();
        let x = (self.mouse_position.x
            - f64::from(rect.x)
            - f64::from(self.cell_metrics.horizontal_padding) * scale_factor)
            .max(0.0);
        let y = (self.mouse_position.y
            - f64::from(rect.y)
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
        if state == ElementState::Pressed {
            if self
                .command_menu_layout
                .button_contains(self.mouse_position.x, self.mouse_position.y)
            {
                self.command_menu_open = !self.command_menu_open;
                self.sync_active_renderer(window.scale_factor());
                window.request_redraw();
                return;
            }
            if self.command_menu_open {
                let reload_config = self
                    .command_menu_layout
                    .reload_config_contains(self.mouse_position.x, self.mouse_position.y);
                self.command_menu_open = false;
                if reload_config {
                    if let Err(error) = self.reload_config_with_notification() {
                        eprintln!("toyoterm: update config error notification: {error}");
                    }
                    self.sync_active_renderer(window.scale_factor());
                    window.request_redraw();
                    return;
                }
                self.sync_active_renderer(window.scale_factor());
                window.request_redraw();
            }
            if self.config_error_notice.is_some()
                && self
                    .config_error_layout
                    .notice()
                    .contains(self.mouse_position.x, self.mouse_position.y)
            {
                let open_log = self
                    .config_error_layout
                    .open_log_contains(self.mouse_position.x, self.mouse_position.y);
                let open_ruby_console = self
                    .config_error_layout
                    .open_ruby_console_contains(self.mouse_position.x, self.mouse_position.y);
                let dismiss = self
                    .config_error_layout
                    .dismiss_contains(self.mouse_position.x, self.mouse_position.y);
                if dismiss {
                    self.config_error_notice = None;
                } else if let Some(notice) = self.config_error_notice.as_mut() {
                    if open_log {
                        notice.log_expanded = !notice.log_expanded;
                        notice.console_hint = false;
                    } else if open_ruby_console {
                        notice.console_hint = true;
                    }
                }
                if open_log || open_ruby_console || dismiss {
                    if let Err(error) =
                        self.resize_panes(window.inner_size(), window.scale_factor())
                    {
                        eprintln!("toyoterm: resize after config notification action: {error}");
                    }
                    self.sync_active_renderer(window.scale_factor());
                    window.request_redraw();
                }
                return;
            }
            if let Some(workspace) = self
                .workspace_layout
                .workspace_at(self.mouse_position.x, self.mouse_position.y)
            {
                if self.mux.current_workspace() != workspace
                    && let Err(error) =
                        self.dispatch_gui_command(Command::ActivateWorkspace(workspace))
                {
                    eprintln!("toyoterm: activate workspace {workspace}: {error}");
                }
                return;
            }
            if let Some(tab) = self
                .tab_layout
                .tab_at(self.mouse_position.x, self.mouse_position.y)
            {
                if self.mux.current_tab() != Some(tab)
                    && let Err(error) = self.dispatch_gui_command(Command::ActivateTab(tab))
                {
                    eprintln!("toyoterm: activate tab {tab}: {error}");
                }
                return;
            }
            let hovered = self
                .pane_layout
                .pane_at(self.mouse_position.x, self.mouse_position.y);
            let Some(hovered) = hovered else {
                return;
            };
            if self.mux.current_pane() != Some(hovered)
                && let Err(error) = self.dispatch_gui_command(Command::ActivatePane(hovered))
            {
                eprintln!("toyoterm: focus pane {hovered}: {error}");
                return;
            }
        }
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
                let Some(pane) = self.mux.current_pane() else {
                    return;
                };
                let click_count = self
                    .click_tracker
                    .register(Instant::now(), ClickTarget { pane, column, row });
                let kind = match click_count {
                    2 => SelectionKind::Word,
                    3 => SelectionKind::Line,
                    _ => SelectionKind::Simple,
                };
                if let Some(terminal) = self.active_terminal_mut() {
                    terminal.clear_selection();
                    terminal.start_selection(column, row, kind);
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
        let active = self.mux.current_pane();
        let snapshots = self
            .pane_layout
            .panes()
            .iter()
            .filter_map(|placement| {
                self.pane_runtimes.get(&placement.pane).map(|runtime| {
                    (
                        placement.pane,
                        runtime.terminal.snapshot(),
                        runtime.terminal.cursor(),
                        placement.rect,
                        active == Some(placement.pane),
                    )
                })
            })
            .collect::<Vec<_>>();
        let panes = snapshots
            .iter()
            .map(|(pane, snapshot, cursor, rect, active)| PaneRenderData {
                pane: *pane,
                snapshot,
                cursor: *cursor,
                rect: *rect,
                active: *active,
            })
            .collect::<Vec<_>>();
        let active_tab = self.mux.current_tab();
        let tab_titles = self
            .tab_layout
            .tabs()
            .iter()
            .map(|placement| {
                (
                    placement.tab,
                    format!("Tab {}", placement.tab.0),
                    placement.rect,
                    active_tab == Some(placement.tab),
                )
            })
            .collect::<Vec<_>>();
        let tabs = tab_titles
            .iter()
            .map(|(tab, title, rect, active)| TabRenderData {
                tab: *tab,
                title,
                rect: *rect,
                active: *active,
            })
            .collect::<Vec<_>>();
        let active_workspace = self.mux.current_workspace();
        let workspace_titles = self
            .workspace_layout
            .workspaces()
            .iter()
            .filter_map(|placement| {
                self.mux.workspace_name(placement.workspace).map(|name| {
                    (
                        placement.workspace,
                        name.to_owned(),
                        placement.rect,
                        active_workspace == placement.workspace,
                    )
                })
            })
            .collect::<Vec<_>>();
        let workspaces = workspace_titles
            .iter()
            .map(|(workspace, name, rect, active)| WorkspaceRenderData {
                workspace: *workspace,
                name,
                rect: *rect,
                active: *active,
            })
            .collect::<Vec<_>>();
        let config_error_message = self
            .config_error_notice
            .as_ref()
            .map(ConfigErrorNotice::display_message);
        let config_error = self.config_error_notice.as_ref().and_then(|notice| {
            config_error_message
                .as_deref()
                .map(|message| ConfigErrorRenderData {
                    message,
                    notice_rect: self.config_error_layout.notice(),
                    open_log_rect: self.config_error_layout.open_log(),
                    open_ruby_console_rect: self.config_error_layout.open_ruby_console(),
                    dismiss_rect: self.config_error_layout.dismiss(),
                    log_expanded: notice.log_expanded,
                })
        });
        let layout = self.cell_metrics.text_layout(scale_factor);
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.update_panes(&panes, layout);
            renderer.update_tabs(&tabs, layout);
            renderer.update_workspaces(&workspaces, layout);
            renderer.update_command_menu(
                CommandMenuRenderData {
                    button_rect: self.command_menu_layout.button(),
                    reload_config_rect: self
                        .command_menu_open
                        .then_some(self.command_menu_layout.reload_config()),
                },
                layout,
            );
            renderer.update_config_error(config_error, layout);
            renderer.update_preedit(self.ime_preedit.as_deref(), layout);
        }
        self.update_ime_cursor_area(scale_factor);
        self.update_window_title();
    }

    fn update_ime_cursor_area(&self, scale_factor: f64) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(pane) = self.mux.current_pane() else {
            return;
        };
        let Some(rect) = self.pane_layout.rect(pane) else {
            return;
        };
        let Some(terminal) = self.active_terminal() else {
            return;
        };
        let cursor = terminal.cursor();
        let layout = self.cell_metrics.text_layout(scale_factor);
        window.set_ime_cursor_area(
            PhysicalPosition::new(
                f64::from(rect.x)
                    + f64::from(layout.horizontal_padding)
                    + f64::from(cursor.column) * f64::from(layout.cell_width),
                f64::from(rect.y)
                    + f64::from(layout.vertical_padding)
                    + f64::from(cursor.row) * f64::from(layout.line_height),
            ),
            PhysicalSize::new(
                layout.cell_width.max(1.0) as u32,
                layout.line_height.max(1.0) as u32,
            ),
        );
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
        let tab = self
            .mux
            .current_tab()
            .map(|tab| format!("Tab {} · ", tab.0))
            .unwrap_or_default();
        let workspace = self
            .mux
            .workspace_name(self.mux.current_workspace())
            .map(|workspace| format!("{workspace} · "))
            .unwrap_or_default();
        let pid = runtime
            .process_id
            .map(|pid| format!(" · pid {pid}"))
            .unwrap_or_default();
        let cwd = runtime
            .cwd
            .as_ref()
            .map(|cwd| format!(" · {}", cwd.display()))
            .unwrap_or_default();
        window.set_title(&format!(
            "toyoterm — {workspace}{tab}{}{pid}{cwd}",
            runtime.title
        ));
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

fn tab_bar_height(scale_factor: f64) -> u32 {
    (30.0 * scale_factor.max(0.1)).round() as u32
}

fn workspace_bar_height(scale_factor: f64) -> u32 {
    (24.0 * scale_factor.max(0.1)).round() as u32
}

fn command_menu_width(scale_factor: f64) -> u32 {
    (160.0 * scale_factor.max(0.1)).round() as u32
}

fn config_error_height(scale_factor: f64, log_expanded: bool) -> u32 {
    let logical_height = if log_expanded { 240.0 } else { 120.0 };
    (logical_height * scale_factor.max(0.1)).round() as u32
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

fn key_press(
    event: &KeyEvent,
    modifiers: ModifiersState,
    mode: crate::TerminalMode,
) -> Option<KeyPress> {
    let key = if mode.application_keypad
        && let Some(key) = keypad_key(event.physical_key)
    {
        TerminalKey::Keypad(key)
    } else {
        match &event.logical_key {
            Key::Named(named) => named_key(named)?,
            Key::Character(text) => {
                TerminalKey::Text(event.text.as_deref().unwrap_or(text.as_str()).to_owned())
            }
            _ => return None,
        }
    };
    Some(KeyPress::new(key, key_modifiers(modifiers)))
}

fn keypad_key(physical_key: PhysicalKey) -> Option<KeypadKey> {
    Some(match physical_key {
        PhysicalKey::Code(KeyCode::Numpad0) => KeypadKey::Digit(0),
        PhysicalKey::Code(KeyCode::Numpad1) => KeypadKey::Digit(1),
        PhysicalKey::Code(KeyCode::Numpad2) => KeypadKey::Digit(2),
        PhysicalKey::Code(KeyCode::Numpad3) => KeypadKey::Digit(3),
        PhysicalKey::Code(KeyCode::Numpad4) => KeypadKey::Digit(4),
        PhysicalKey::Code(KeyCode::Numpad5) => KeypadKey::Digit(5),
        PhysicalKey::Code(KeyCode::Numpad6) => KeypadKey::Digit(6),
        PhysicalKey::Code(KeyCode::Numpad7) => KeypadKey::Digit(7),
        PhysicalKey::Code(KeyCode::Numpad8) => KeypadKey::Digit(8),
        PhysicalKey::Code(KeyCode::Numpad9) => KeypadKey::Digit(9),
        PhysicalKey::Code(KeyCode::NumpadAdd) => KeypadKey::Add,
        PhysicalKey::Code(KeyCode::NumpadSubtract) => KeypadKey::Subtract,
        PhysicalKey::Code(KeyCode::NumpadMultiply) => KeypadKey::Multiply,
        PhysicalKey::Code(KeyCode::NumpadDivide) => KeypadKey::Divide,
        PhysicalKey::Code(KeyCode::NumpadDecimal) => KeypadKey::Decimal,
        PhysicalKey::Code(KeyCode::NumpadComma) => KeypadKey::Comma,
        PhysicalKey::Code(KeyCode::NumpadEqual) => KeypadKey::Equal,
        PhysicalKey::Code(KeyCode::NumpadEnter) => KeypadKey::Enter,
        _ => return None,
    })
}

fn should_handle_key_event(state: ElementState, _repeat: bool) -> bool {
    state == ElementState::Pressed
}

fn clear_modifier_state(modifiers: &mut ModifiersState, alt_graph_active: &mut bool) {
    *modifiers = ModifiersState::empty();
    *alt_graph_active = false;
}

fn effective_modifiers(mut modifiers: ModifiersState, alt_graph_active: bool) -> ModifiersState {
    if alt_graph_active {
        // AltGr is commonly exposed as the right Alt key together with a
        // synthetic Control modifier. Treat it as a text-layout modifier,
        // rather than emitting a control byte with an escape prefix.
        modifiers.remove(ModifiersState::CONTROL | ModifiersState::ALT);
    }
    modifiers
}

fn keybinding_names(event: &KeyEvent, modifiers: ModifiersState) -> Vec<String> {
    let modifiers = key_modifiers(modifiers);
    let physical = match event.physical_key {
        PhysicalKey::Code(code) => Some(format!("{code:?}")),
        PhysicalKey::Unidentified(_) => None,
    };
    binding_candidates(physical, logical_binding_key(&event.logical_key), modifiers)
}

fn binding_candidates(
    physical: Option<String>,
    logical: Option<String>,
    modifiers: KeyModifiers,
) -> Vec<String> {
    physical
        .map(|key| KeyChord::new(BindingKey::Physical(key), modifiers).canonical_name())
        .into_iter()
        .chain(
            logical.map(|key| KeyChord::new(BindingKey::Logical(key), modifiers).canonical_name()),
        )
        .collect()
}

#[cfg(test)]
fn keybinding_name(logical_key: &Key, modifiers: ModifiersState) -> Option<String> {
    let key = logical_binding_key(logical_key)?;
    Some(KeyChord::new(BindingKey::Logical(key), key_modifiers(modifiers)).canonical_name())
}

fn logical_binding_key(logical_key: &Key) -> Option<String> {
    Some(match logical_key {
        Key::Character(text) if !text.is_empty() => text.to_uppercase(),
        Key::Named(key) => keybinding_named_key(key)?.to_owned(),
        _ => return None,
    })
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
    let primary_modifier = has_gui_primary_modifier(modifiers, current_shortcut_platform());
    primary_modifier && key.eq_ignore_ascii_case(&character.to_string())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShortcutPlatform {
    MacOs,
    LinuxOrWindows,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GuiManagementShortcut {
    ReloadConfig,
    NewTab,
    NewWorkspace,
    CloseTab,
    Split(SplitDirection),
    ClosePane,
    Focus(SplitDirection),
}

fn current_shortcut_platform() -> ShortcutPlatform {
    if cfg!(target_os = "macos") {
        ShortcutPlatform::MacOs
    } else {
        ShortcutPlatform::LinuxOrWindows
    }
}

fn has_gui_primary_modifier(modifiers: ModifiersState, platform: ShortcutPlatform) -> bool {
    match platform {
        ShortcutPlatform::MacOs => modifiers.super_key(),
        ShortcutPlatform::LinuxOrWindows => modifiers.control_key() && modifiers.shift_key(),
    }
}

fn gui_management_shortcut(
    key: &Key,
    modifiers: ModifiersState,
    platform: ShortcutPlatform,
) -> Option<GuiManagementShortcut> {
    let character = match key {
        Key::Character(character) => Some(character.as_str()),
        _ => None,
    };
    let arrow = match key {
        Key::Named(NamedKey::ArrowLeft) => Some(SplitDirection::Left),
        Key::Named(NamedKey::ArrowRight) => Some(SplitDirection::Right),
        Key::Named(NamedKey::ArrowUp) => Some(SplitDirection::Up),
        Key::Named(NamedKey::ArrowDown) => Some(SplitDirection::Down),
        _ => None,
    };

    match platform {
        ShortcutPlatform::LinuxOrWindows if modifiers.control_key() && modifiers.shift_key() => {
            match character {
                Some(key) if key.eq_ignore_ascii_case("r") => {
                    Some(GuiManagementShortcut::ReloadConfig)
                }
                Some(key) if key.eq_ignore_ascii_case("t") => Some(GuiManagementShortcut::NewTab),
                Some(key) if key.eq_ignore_ascii_case("n") => {
                    Some(GuiManagementShortcut::NewWorkspace)
                }
                Some(key) if key.eq_ignore_ascii_case("w") => Some(GuiManagementShortcut::CloseTab),
                Some("\\" | "|") => Some(GuiManagementShortcut::Split(SplitDirection::Right)),
                Some("-" | "_") => Some(GuiManagementShortcut::Split(SplitDirection::Down)),
                Some(key) if key.eq_ignore_ascii_case("q") => {
                    Some(GuiManagementShortcut::ClosePane)
                }
                _ => arrow.map(GuiManagementShortcut::Focus),
            }
        }
        ShortcutPlatform::MacOs if modifiers.super_key() => match character {
            Some(key) if key.eq_ignore_ascii_case("r") && modifiers.shift_key() => {
                Some(GuiManagementShortcut::ReloadConfig)
            }
            Some(key) if key.eq_ignore_ascii_case("t") && !modifiers.shift_key() => {
                Some(GuiManagementShortcut::NewTab)
            }
            Some(key) if key.eq_ignore_ascii_case("n") && !modifiers.shift_key() => {
                Some(GuiManagementShortcut::NewWorkspace)
            }
            Some(key) if key.eq_ignore_ascii_case("w") && modifiers.shift_key() => {
                Some(GuiManagementShortcut::ClosePane)
            }
            Some(key) if key.eq_ignore_ascii_case("w") => Some(GuiManagementShortcut::CloseTab),
            Some(key) if key.eq_ignore_ascii_case("d") && modifiers.shift_key() => {
                Some(GuiManagementShortcut::Split(SplitDirection::Down))
            }
            Some(key) if key.eq_ignore_ascii_case("d") => {
                Some(GuiManagementShortcut::Split(SplitDirection::Right))
            }
            _ if modifiers.alt_key() => arrow.map(GuiManagementShortcut::Focus),
            _ => None,
        },
        _ => None,
    }
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
    fn config_error_notice_switches_between_summary_log_and_console_hint() {
        let mut notice = ConfigErrorNotice {
            message: "error\nconfig.rb:2\nconfig.rb:5\nextra".into(),
            log_expanded: false,
            console_hint: false,
        };
        assert_eq!(notice.display_message(), "error\nconfig.rb:2\nconfig.rb:5");

        notice.log_expanded = true;
        assert_eq!(notice.display_message(), notice.message);

        notice.console_hint = true;
        assert!(notice.display_message().contains("not available yet"));
    }

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
    fn physical_binding_candidates_take_priority_over_logical_keys() {
        assert_eq!(
            binding_candidates(
                Some("KeyY".into()),
                Some("Z".into()),
                KeyModifiers {
                    control: true,
                    ..KeyModifiers::default()
                },
            ),
            vec!["CTRL+PHYSICAL:KEYY", "CTRL+Z"]
        );
    }

    #[test]
    fn alt_graph_is_not_treated_as_control_alt() {
        let modifiers = effective_modifiers(ModifiersState::CONTROL | ModifiersState::ALT, true);
        assert!(!modifiers.control_key());
        assert!(!modifiers.alt_key());

        let press = KeyPress::new(TerminalKey::Text("@".into()), key_modifiers(modifiers));
        assert_eq!(
            encode_key(&press, crate::TerminalMode::default()),
            Some(b"@".to_vec())
        );
    }

    #[test]
    fn real_control_alt_remains_available_for_bindings() {
        let modifiers = effective_modifiers(ModifiersState::CONTROL | ModifiersState::ALT, false);
        assert_eq!(
            keybinding_name(&Key::Character("x".into()), modifiers).as_deref(),
            Some("CTRL+ALT+X")
        );
    }

    #[test]
    fn gui_primary_modifier_matches_each_platform_default() {
        assert!(has_gui_primary_modifier(
            ModifiersState::SUPER,
            ShortcutPlatform::MacOs,
        ));
        assert!(!has_gui_primary_modifier(
            ModifiersState::CONTROL | ModifiersState::SHIFT,
            ShortcutPlatform::MacOs,
        ));
        assert!(has_gui_primary_modifier(
            ModifiersState::CONTROL | ModifiersState::SHIFT,
            ShortcutPlatform::LinuxOrWindows,
        ));
        assert!(!has_gui_primary_modifier(
            ModifiersState::SUPER,
            ShortcutPlatform::LinuxOrWindows,
        ));
    }

    #[test]
    fn gui_management_shortcuts_follow_platform_conventions() {
        assert_eq!(
            gui_management_shortcut(
                &Key::Character("t".into()),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                ShortcutPlatform::LinuxOrWindows,
            ),
            Some(GuiManagementShortcut::NewTab)
        );
        assert_eq!(
            gui_management_shortcut(
                &Key::Character("t".into()),
                ModifiersState::SUPER,
                ShortcutPlatform::MacOs,
            ),
            Some(GuiManagementShortcut::NewTab)
        );
        assert_eq!(
            gui_management_shortcut(
                &Key::Character("d".into()),
                ModifiersState::SUPER | ModifiersState::SHIFT,
                ShortcutPlatform::MacOs,
            ),
            Some(GuiManagementShortcut::Split(SplitDirection::Down))
        );
        assert_eq!(
            gui_management_shortcut(
                &Key::Character("q".into()),
                ModifiersState::SUPER,
                ShortcutPlatform::MacOs,
            ),
            None
        );
    }

    #[test]
    fn key_repeat_is_delivered_and_releases_are_ignored() {
        assert!(should_handle_key_event(ElementState::Pressed, false));
        assert!(should_handle_key_event(ElementState::Pressed, true));
        assert!(!should_handle_key_event(ElementState::Released, false));
        assert!(!should_handle_key_event(ElementState::Released, true));
    }

    #[test]
    fn focus_loss_clears_all_modifier_state() {
        let mut modifiers = ModifiersState::SHIFT
            | ModifiersState::CONTROL
            | ModifiersState::ALT
            | ModifiersState::SUPER;
        let mut alt_graph_active = true;

        clear_modifier_state(&mut modifiers, &mut alt_graph_active);

        assert!(modifiers.is_empty());
        assert!(!alt_graph_active);
    }

    #[test]
    fn maps_physical_numpad_keys_independently_of_layout() {
        assert_eq!(
            keypad_key(PhysicalKey::Code(KeyCode::Numpad7)),
            Some(KeypadKey::Digit(7))
        );
        assert_eq!(
            keypad_key(PhysicalKey::Code(KeyCode::NumpadEnter)),
            Some(KeypadKey::Enter)
        );
        assert_eq!(keypad_key(PhysicalKey::Code(KeyCode::Digit7)), None);
    }

    #[test]
    fn click_tracker_recognizes_double_and_triple_clicks() {
        let mut tracker = ClickTracker::default();
        let start = Instant::now();
        let target = ClickTarget {
            pane: PaneId(1),
            column: 4,
            row: 2,
        };

        assert_eq!(tracker.register(start, target), 1);
        assert_eq!(
            tracker.register(start + Duration::from_millis(100), target),
            2
        );
        assert_eq!(
            tracker.register(start + Duration::from_millis(200), target),
            3
        );
        assert_eq!(
            tracker.register(start + Duration::from_millis(300), target),
            1
        );
    }

    #[test]
    fn click_tracker_resets_for_a_new_cell_or_after_timeout() {
        let mut tracker = ClickTracker::default();
        let start = Instant::now();
        let target = ClickTarget {
            pane: PaneId(1),
            column: 4,
            row: 2,
        };
        tracker.register(start, target);

        assert_eq!(
            tracker.register(
                start + Duration::from_millis(100),
                ClickTarget {
                    column: 5,
                    ..target
                },
            ),
            1
        );
        assert_eq!(
            tracker.register(
                start + MULTI_CLICK_INTERVAL + Duration::from_millis(200),
                target
            ),
            1
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
