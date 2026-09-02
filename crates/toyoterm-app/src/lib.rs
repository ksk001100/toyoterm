use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use arboard::Clipboard;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

use toyoterm_script::{
    RubyEvent, RubyObjectModel, RubyPane, RubyTab, RubyWindow, RubyWorkspace, ScriptCompletion,
    ScriptContext, ScriptInvocation, ScriptRequest, ScriptSnapshot, ScriptThread,
};

mod lifecycle;
mod logging;
mod palette;

pub use lifecycle::install_panic_hook;
pub use logging::init_logging;
pub use palette::{CommandPalette, PaletteAction, PaletteItem, filter_items};
pub use toyoterm_api::{
    Command, Event as MuxEvent, NativeAction, NativeCommand, PaneId, SplitDirection,
};
pub use toyoterm_config::ToyotermConfig;
pub use toyoterm_ipc::{IpcRequest, IpcResponse, IpcServer};
pub use toyoterm_mux::Mux;
pub use toyoterm_pty::{NativePty, Pty, PtyCommand, PtyError, PtyExitStatus, PtySession, PtySize};
pub use toyoterm_render::{
    ConfigErrorLayout, ConfigErrorRenderData, GpuRenderer, PaletteRenderData, PaneLayout, PaneRect,
    PaneRenderData, RenderOutcome, RenderStyle, StatusBarRenderData, TabRenderData, TabStripLayout,
    TextLayout, WorkspaceRenderData, WorkspaceStripLayout,
};
pub use toyoterm_script::ConfigManager;
pub use toyoterm_terminal::{
    AlacrittyTerminalBackend, BindingKey, KeyChord, KeyModifiers, KeyPress, KeypadKey,
    MouseWheelDirection, SelectionKind, TerminalBackend, TerminalEvent, TerminalKey, TerminalMode,
    encode_key, encode_mouse_wheel, encode_paste,
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
    run_gui_inner(None, false)
}

pub fn run_gui_with_config_path(config_path: Option<&Path>) -> Result<(), AppError> {
    run_gui_inner(config_path, false)
}

/// Starts the complete GUI stack and exits after successful initialization.
pub fn run_gui_smoke_test() -> Result<(), AppError> {
    run_gui_inner(None, true)
}

fn run_gui_inner(config_path: Option<&Path>, exit_after_startup: bool) -> Result<(), AppError> {
    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .map_err(|error| AppError(error.to_string()))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let event_proxy = event_loop.create_proxy();
    let completion_proxy = event_proxy.clone();
    let (script_thread, startup) =
        ScriptThread::start(config_path.map(Path::to_owned), move |completion| {
            let _ = completion_proxy.send_event(AppEvent::ScriptCompleted(Box::new(completion)));
        })
        .map_err(|error| {
            tracing::error!(
                target: "toyoterm::config",
                operation = error.operation(),
                config_path = ?config_path,
                %error,
                "load startup config failed"
            );
            AppError(error.to_string())
        })?;
    let startup_config_error = startup.config_error.map(|error| {
        tracing::warn!(
            target: "toyoterm::config",
            operation = error.operation(),
            config_path = ?config_path,
            %error,
            "startup config rejected; using defaults"
        );
        error.to_string()
    });
    let config = &startup.snapshot.config;
    let render_style = RenderStyle::from_hex(
        &config.font.family,
        config.font.fallback.clone(),
        config.font.weight,
        [
            &config.colors.background,
            &config.colors.foreground,
            &config.colors.cursor,
            &config.colors.selection,
        ],
        config.window_opacity,
    )
    .map_err(|error| {
        tracing::error!(
            target: "toyoterm::render",
            operation = error.operation(),
            %error,
            "build render style failed"
        );
        AppError(error.to_string())
    })?;
    let mut app = ToyotermApplication::new(
        event_proxy,
        script_thread,
        startup.snapshot,
        render_style,
        startup_config_error,
        exit_after_startup,
    )
    .map_err(AppError)?;
    app.submit_script(ScriptInvocation::DrainStartup)
        .map_err(AppError)?;
    event_loop
        .run_app(&mut app)
        .map_err(|error| AppError(error.to_string()))?;
    match app.fatal_error.take() {
        Some(error) => Err(AppError(error)),
        None => Ok(()),
    }
}

#[derive(Debug)]
enum AppEvent {
    Output {
        pane: PaneId,
        bytes: Vec<u8>,
    },
    Eof {
        pane: PaneId,
    },
    Error {
        pane: PaneId,
        message: String,
    },
    Ipc {
        request: IpcRequest,
        response: IpcResponse,
    },
    ScriptCompleted(Box<ScriptCompletion>),
}

enum EvalWaiter {
    Ipc(mpsc::Sender<Result<String, String>>),
    Palette(String),
}

struct PaneRuntime {
    terminal: AlacrittyTerminalBackend,
    pty_session: Option<Box<dyn PtySession>>,
    process_id: Option<u32>,
    title: String,
    cwd: Option<PathBuf>,
    command_running: bool,
    last_exit_status: Option<i32>,
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
    palette_open: bool,
    palette: CommandPalette,
    _ipc_server: Option<IpcServer>,
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
    pending_clipboard_writes: Vec<String>,
    runtime_events: VecDeque<RubyEvent>,
    next_script_request: u64,
    script_in_flight: bool,
    pending_script: VecDeque<(u64, ScriptInvocation)>,
    eval_waiters: HashMap<u64, EvalWaiter>,
    cell_metrics: CellMetrics,
    script_thread: ScriptThread,
    script_snapshot: ScriptSnapshot,
    status_text: String,
    status_pending: bool,
    next_status_at: Option<Instant>,
    terminal_render_pending: bool,
    mux: Mux,
    render_style: RenderStyle,
    fatal_error: Option<String>,
    exit_after_startup: bool,
}

impl ApplicationHandler<AppEvent> for ToyotermApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("toyoterm")
            .with_transparent(self.script_snapshot.config.window_opacity < 1.0)
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
                tracing::error!(
                    target: "toyoterm::render",
                    operation = error.operation(),
                    width = window.inner_size().width,
                    height = window.inner_size().height,
                    scale_factor = window.scale_factor(),
                    %error,
                    "initialize renderer failed"
                );
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
        if let Err(error) = self.flush_script_clipboard_writes() {
            self.fail(event_loop, error);
            return;
        }
        self.refresh_pane_layout();
        if let Err(error) = self.sync_pane_runtimes(size) {
            self.fail(event_loop, error);
            return;
        }
        if let Err(error) = self.deliver_runtime_events() {
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
        if self.exit_after_startup {
            event_loop.exit();
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
                    if self.palette_open {
                        self.palette_open = false;
                        self.palette.close();
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
                if self.palette_open && matches!(event.logical_key, Key::Named(NamedKey::Escape)) {
                    self.palette_open = false;
                    self.palette.close();
                    self.sync_active_renderer(window.scale_factor());
                    window.request_redraw();
                    return;
                }
                if self.palette_open {
                    if let Err(error) = self.handle_palette_key(&event) {
                        tracing::warn!(target: "toyoterm::app", %error, "command palette action failed");
                    }
                    self.sync_active_renderer(window.scale_factor());
                    window.request_redraw();
                    return;
                }
                match self.handle_leader_key(&event, modifiers) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(target: "toyoterm::app", %error, "leader key handling failed");
                        return;
                    }
                }
                if is_clipboard_shortcut(&event, modifiers, 'c') {
                    if let Err(error) = self.copy_selection() {
                        tracing::warn!(target: "toyoterm::app", %error, "copy failed");
                    }
                    return;
                }
                if is_clipboard_shortcut(&event, modifiers, 'v') {
                    if let Err(error) = self.paste_clipboard() {
                        tracing::warn!(target: "toyoterm::app", %error, "paste failed");
                    }
                    return;
                }
                // Configured bindings override built-in GUI shortcuts. Physical
                // bindings are checked before logical bindings by the resolver.
                match self.handle_keybinding(&event, modifiers) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(target: "toyoterm::script", %error, "key binding failed");
                        return;
                    }
                }
                match self.handle_tab_shortcut(&event, modifiers) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(target: "toyoterm::app", %error, "tab shortcut failed");
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
                if self.palette_open {
                    self.palette.insert(&text);
                    self.sync_active_renderer(window.scale_factor());
                    window.request_redraw();
                    return;
                }
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
                if self.terminal_render_pending {
                    self.terminal_render_pending = false;
                    self.sync_active_renderer(window.scale_factor());
                }
                let render_result = self.renderer.as_mut().map(GpuRenderer::render);
                match render_result {
                    Some(Ok(RenderOutcome::DeviceLost)) => {
                        if let Err(error) = self.recover_renderer() {
                            self.fail(event_loop, error);
                        }
                    }
                    Some(Err(error)) => {
                        tracing::error!(
                            target: "toyoterm::render",
                            operation = error.operation(),
                            %error,
                            "render failed"
                        );
                        self.fail(event_loop, error.to_string());
                    }
                    Some(Ok(RenderOutcome::Presented | RenderOutcome::Skipped)) | None => {}
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(interval) = self.script_snapshot.config.status_interval else {
            self.next_status_at = None;
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        };
        if self.status_pending || self.window.is_none() {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        let now = Instant::now();
        let deadline = *self.next_status_at.get_or_insert(now);
        if now < deadline {
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            return;
        }
        match self.submit_script(ScriptInvocation::Status) {
            Ok(_) => {
                self.status_pending = true;
                self.next_status_at = None;
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Err(error) => {
                tracing::warn!(target: "toyoterm::script", %error, "submit status callback failed");
                self.next_status_at = Some(now + interval);
                event_loop.set_control_flow(ControlFlow::WaitUntil(now + interval));
            }
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.shutdown();
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Output { pane, bytes } => {
                let mut terminal_events = Vec::new();
                if let Some(runtime) = self.pane_runtimes.get_mut(&pane) {
                    runtime.terminal.advance(&bytes);
                    terminal_events = runtime.terminal.drain_events();
                    for event in &terminal_events {
                        match event {
                            TerminalEvent::TitleChanged(title) => runtime.title = title.clone(),
                            TerminalEvent::TitleReset => runtime.title = format!("Pane {}", pane.0),
                            TerminalEvent::CwdChanged(cwd) => {
                                runtime.cwd = Some(PathBuf::from(cwd))
                            }
                            TerminalEvent::CommandStarted => runtime.command_running = true,
                            TerminalEvent::CommandFinished(status) => {
                                runtime.command_running = false;
                                runtime.last_exit_status = *status;
                            }
                            TerminalEvent::PtyWrite(response) => {
                                if let Some(session) = runtime.pty_session.as_mut()
                                    && let Err(error) = session.write(response.as_bytes())
                                {
                                    tracing::error!(
                                        target: "toyoterm::pty",
                                        operation = error.operation(),
                                        %pane,
                                        bytes = response.len(),
                                        %error,
                                        "write terminal response to pane PTY failed"
                                    );
                                }
                            }
                            TerminalEvent::Bell => {}
                        }
                    }
                }
                for event in terminal_events {
                    let mut runtime_event = match event {
                        TerminalEvent::TitleChanged(title) => {
                            let mut event = RubyEvent::new("title_changed");
                            event.title = Some(title);
                            event
                        }
                        TerminalEvent::TitleReset => {
                            let mut event = RubyEvent::new("title_changed");
                            event.title = Some(format!("Pane {}", pane.0));
                            event
                        }
                        TerminalEvent::CwdChanged(cwd) => {
                            let mut event = RubyEvent::new("cwd_changed");
                            event.cwd = Some(cwd);
                            event
                        }
                        TerminalEvent::CommandStarted | TerminalEvent::CommandFinished(_) => {
                            continue;
                        }
                        TerminalEvent::PtyWrite(_) => continue,
                        TerminalEvent::Bell => RubyEvent::new("bell"),
                    };
                    runtime_event.pane = Some(pane);
                    self.runtime_events.push_back(runtime_event);
                }
                if let Err(error) = self.deliver_runtime_events() {
                    self.fail(event_loop, error);
                    return;
                }
                if self.pane_layout.rect(pane).is_some() {
                    // Winit coalesces redraw requests. Keep parsing PTY bytes
                    // immediately to preserve ordering, but defer the costly
                    // full-grid snapshot and text shaping until the matching
                    // redraw. This prevents bursty alternate-screen output
                    // (notably Neovim exit) from building a render backlog.
                    self.terminal_render_pending = true;
                    if let Some(window) = self.window.as_ref() {
                        window.request_redraw();
                    }
                }
            }
            AppEvent::Eof { pane } => {
                if let Err(error) = self.close_exited_pane(event_loop, pane) {
                    self.fail(event_loop, error);
                }
            }
            AppEvent::Error { pane, message } => {
                tracing::error!(
                    target: "toyoterm::pty",
                    operation = "read PTY output",
                    %pane,
                    %message,
                    "PTY reader failed"
                );
                self.mark_pane_exited(pane, Some(message));
            }
            AppEvent::Ipc { request, response } => {
                if let IpcRequest::Eval(source) = request {
                    match self.submit_script(ScriptInvocation::Eval(source)) {
                        Ok(id) => {
                            self.eval_waiters.insert(id, EvalWaiter::Ipc(response));
                        }
                        Err(error) => {
                            let _ = response.send(Err(error));
                        }
                    }
                } else if matches!(request, IpcRequest::Reload) {
                    match self.submit_script(ScriptInvocation::Reload) {
                        Ok(id) => {
                            self.eval_waiters.insert(id, EvalWaiter::Ipc(response));
                        }
                        Err(error) => {
                            let _ = response.send(Err(error));
                        }
                    }
                } else {
                    let result = self.handle_ipc_request(&request);
                    let _ = response.send(result);
                }
            }
            AppEvent::ScriptCompleted(completion) => {
                if let Err(error) = self.handle_script_completion(*completion) {
                    tracing::warn!(target: "toyoterm::script", %error, "apply script result failed");
                }
                self.script_in_flight = false;
                if let Err(error) = self.start_next_script() {
                    tracing::warn!(target: "toyoterm::script", %error, "submit queued script request failed");
                }
                if let Some(window) = self.window.clone() {
                    self.sync_active_renderer(window.scale_factor());
                    window.request_redraw();
                }
            }
        }
    }
}

impl ToyotermApplication {
    fn shutdown(&mut self) {
        if let Some(window) = self.window.as_ref() {
            window.set_ime_allowed(false);
        }
        for (_, mut runtime) in self.pane_runtimes.drain() {
            runtime.terminate();
        }
        self.renderer = None;
        self.window = None;
    }

    fn new(
        event_proxy: EventLoopProxy<AppEvent>,
        script_thread: ScriptThread,
        script_snapshot: ScriptSnapshot,
        render_style: RenderStyle,
        startup_config_error: Option<String>,
        exit_after_startup: bool,
    ) -> Result<Self, String> {
        let mux = Mux::new();
        let config = &script_snapshot.config;
        let font_scale = f64::from(config.font.size) / 14.0;
        let ipc_proxy = event_proxy.clone();
        let ipc_server = IpcServer::start(move |request, response| {
            ipc_proxy
                .send_event(AppEvent::Ipc { request, response })
                .map_err(|_| "toyoterm GUI event loop is closed".to_owned())
        })
        .map_err(|error| {
            tracing::warn!(target: "toyoterm::script", %error, "live Ruby console unavailable");
            error
        })
        .ok();
        Ok(Self {
            event_proxy,
            window: None,
            renderer: None,
            pane_runtimes: HashMap::new(),
            pane_layout: PaneLayout::default(),
            tab_layout: TabStripLayout::default(),
            workspace_layout: WorkspaceStripLayout::default(),
            palette_open: false,
            palette: CommandPalette::default(),
            _ipc_server: ipc_server,
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
            pending_clipboard_writes: Vec::new(),
            runtime_events: VecDeque::new(),
            next_script_request: 1,
            script_in_flight: false,
            pending_script: VecDeque::new(),
            eval_waiters: HashMap::new(),
            cell_metrics: CellMetrics {
                width: 9.0 * font_scale,
                height: 18.0 * font_scale,
                font_size: config.font.size,
                ..CellMetrics::default()
            },
            script_thread,
            script_snapshot,
            status_text: String::new(),
            status_pending: false,
            next_status_at: None,
            terminal_render_pending: false,
            mux,
            render_style,
            fatal_error: None,
            exit_after_startup,
        })
    }

    fn start_shell(&mut self, pane: PaneId, size: PtySize) -> Result<PaneRuntime, String> {
        let mut command = match self.script_snapshot.config.default_shell.as_deref() {
            Some(shell) => PtyCommand::new(shell),
            None => PtyCommand::default_shell(),
        };
        command.env("TERM", "xterm-256color");
        command.env("TERM_PROGRAM", "toyoterm");
        command.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
        let mut session = NativePty.spawn(command, size).map_err(|error| {
            tracing::error!(
                target: "toyoterm::pty",
                operation = error.operation(),
                %pane,
                columns = size.columns,
                rows = size.rows,
                %error,
                "start pane shell failed"
            );
            error.to_string()
        })?;
        let reader = session.take_reader().map_err(|error| {
            tracing::error!(
                target: "toyoterm::pty",
                operation = error.operation(),
                %pane,
                %error,
                "open pane PTY reader failed"
            );
            error.to_string()
        })?;
        let process_id = session.process_id();
        spawn_pty_reader(pane, reader, self.event_proxy.clone())?;
        Ok(PaneRuntime {
            terminal: AlacrittyTerminalBackend::with_scrollback(
                size.columns,
                size.rows,
                self.script_snapshot.config.scrollback_lines,
            ),
            pty_session: Some(session),
            process_id,
            title: format!("Pane {}", pane.0),
            cwd: std::env::current_dir().ok(),
            command_running: false,
            last_exit_status: None,
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
            if let Some(action) = self.script_snapshot.native_actions.get(&key).cloned() {
                self.execute_native_action(action)?;
                return Ok(true);
            }
            if !self.script_snapshot.keybindings.contains(&key) {
                continue;
            }
            self.submit_script(ScriptInvocation::KeyBinding { key, pane })?;
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

        let Some(leader) = self.script_snapshot.config.leader.as_ref() else {
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
            NativeAction::CommandPalette => {
                self.open_command_palette();
                Ok(())
            }
            NativeAction::UserCommand(name) => self.execute_user_command(&name),
            NativeAction::Split(direction) => self.split_active_pane(direction),
            NativeAction::ActivatePane(direction) => self.focus_neighbor(direction),
        }
    }

    fn open_command_palette(&mut self) {
        self.palette_open = true;
        self.palette.open();
    }

    fn open_ruby_console(&mut self) {
        self.palette_open = true;
        self.palette.open_console();
    }

    fn palette_items(&self) -> Vec<PaletteItem> {
        let mut items = vec![
            PaletteItem {
                label: "Reload Config".into(),
                action: PaletteAction::ReloadConfig,
            },
            PaletteItem {
                label: "New Tab".into(),
                action: PaletteAction::NewTab,
            },
            PaletteItem {
                label: "Split Right".into(),
                action: PaletteAction::Split(SplitDirection::Right),
            },
            PaletteItem {
                label: "Split Down".into(),
                action: PaletteAction::Split(SplitDirection::Down),
            },
            PaletteItem {
                label: "Close Pane".into(),
                action: PaletteAction::ClosePane,
            },
            PaletteItem {
                label: "Ruby Console".into(),
                action: PaletteAction::RubyConsole,
            },
        ];
        for workspace in self.mux.workspaces() {
            if let Some(name) = self.mux.workspace_name(workspace) {
                items.push(PaletteItem {
                    label: format!("Switch Workspace: {name}"),
                    action: PaletteAction::SwitchWorkspace(name.to_owned()),
                });
            }
        }
        let mut names = self
            .script_snapshot
            .user_command_names
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        names.sort();
        items.extend(names.into_iter().map(|name| PaletteItem {
            label: name.clone(),
            action: PaletteAction::UserCommand(name),
        }));
        items
    }

    fn handle_palette_key(&mut self, event: &KeyEvent) -> Result<(), String> {
        match &event.logical_key {
            Key::Named(NamedKey::ArrowUp) => {
                let count = filter_items(&self.palette_items(), self.palette.query()).len();
                self.palette.move_selection(-1, count);
            }
            Key::Named(NamedKey::ArrowDown) => {
                let count = filter_items(&self.palette_items(), self.palette.query()).len();
                self.palette.move_selection(1, count);
            }
            Key::Named(NamedKey::Backspace) => self.palette.backspace(),
            Key::Named(NamedKey::Enter) if self.palette.is_console() => {
                let source = self.palette.take_input();
                if !source.trim().is_empty() {
                    let id = self.submit_script(ScriptInvocation::Eval(source.clone()))?;
                    self.eval_waiters.insert(id, EvalWaiter::Palette(source));
                }
            }
            Key::Named(NamedKey::Enter) => {
                let items = filter_items(&self.palette_items(), self.palette.query());
                if let Some(item) = items.get(self.palette.selected()).cloned() {
                    self.palette_open = false;
                    self.palette.close();
                    self.execute_palette_action(item.action)?;
                }
            }
            Key::Character(text)
                if !self.modifiers.control_key() && !self.modifiers.super_key() =>
            {
                self.palette.insert(text);
            }
            _ => {}
        }
        Ok(())
    }

    fn execute_palette_action(&mut self, action: PaletteAction) -> Result<(), String> {
        if let Some(command) = palette_native_command(&action, self.mux.current_pane())? {
            return match command {
                NativeCommand::Mux(command) => self.dispatch_gui_command(command),
                NativeCommand::ReloadConfig => self.reload_config_with_notification(),
                NativeCommand::ClipboardWrite(_) => unreachable!("palette has no clipboard action"),
            };
        }
        match action {
            PaletteAction::ReloadConfig
            | PaletteAction::NewTab
            | PaletteAction::Split(_)
            | PaletteAction::ClosePane
            | PaletteAction::SwitchWorkspace(_) => {
                unreachable!("native palette action was normalized")
            }
            PaletteAction::RubyConsole => {
                self.open_ruby_console();
                Ok(())
            }
            PaletteAction::UserCommand(name) => self.execute_user_command(&name),
        }
    }

    fn execute_user_command(&mut self, name: &str) -> Result<(), String> {
        let pane = self
            .mux
            .current_pane()
            .ok_or_else(|| "mux has no current pane".to_owned())?;
        self.submit_script(ScriptInvocation::UserCommand {
            name: name.to_owned(),
            pane,
        })?;
        Ok(())
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
                GuiManagementShortcut::CommandPalette => self.open_command_palette(),
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
            let candidate = format!("Workspace {suffix}");
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
        self.deliver_runtime_events()?;
        if let Some(window) = self.window.clone() {
            self.sync_active_renderer(window.scale_factor());
            window.request_redraw();
        }
        Ok(())
    }

    fn handle_ipc_request(&mut self, request: &IpcRequest) -> Result<String, String> {
        match request {
            IpcRequest::List => return Ok(self.mux.summary()),
            IpcRequest::ListPanes => return Ok(self.ipc_pane_list()),
            IpcRequest::Eval(_) | IpcRequest::Reload => {
                return Err("script IPC request reached native command handler".to_owned());
            }
            IpcRequest::SendText { .. }
            | IpcRequest::Split { .. }
            | IpcRequest::ActivateWorkspace(_) => {}
        }
        let command = request
            .native_command(self.mux.current_pane())?
            .ok_or_else(|| "IPC request has no native command".to_owned())?;
        match command {
            NativeCommand::Mux(command) => {
                self.dispatch_gui_command(command)?;
                self.flush_mux_input()?;
            }
            NativeCommand::ReloadConfig => self.reload_config_with_notification()?,
            NativeCommand::ClipboardWrite(_) => {
                return Err("clipboard commands are not exposed over IPC".to_owned());
            }
        }
        Ok("ok".to_owned())
    }

    fn ipc_pane_list(&self) -> String {
        let active = self.mux.current_pane();
        let mut panes = self.mux.pane_ids().collect::<Vec<_>>();
        panes.sort_unstable();
        let mut output = String::from("ID\tACTIVE\tPID\tCWD\tTITLE");
        for pane in panes {
            let runtime = self.pane_runtimes.get(&pane);
            let pid = runtime
                .and_then(|runtime| runtime.process_id)
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "-".into());
            let cwd = runtime
                .and_then(|runtime| runtime.cwd.as_ref())
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "-".into())
                .replace(['\t', '\n'], " ");
            let title = runtime
                .map(|runtime| runtime.title.replace(['\t', '\n'], " "))
                .unwrap_or_else(|| format!("Pane {}", pane.0));
            output.push_str(&format!(
                "\n{}\t{}\t{}\t{}\t{}",
                pane.0,
                if active == Some(pane) { "*" } else { "" },
                pid,
                cwd,
                title
            ));
        }
        output
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
        self.submit_script(ScriptInvocation::Reload).map(|_| ())
    }

    fn apply_script_snapshot(&mut self, snapshot: ScriptSnapshot) -> Result<(), String> {
        let config = snapshot.config.clone();
        self.leader_deadline = None;
        let render_style = RenderStyle::from_hex(
            &config.font.family,
            config.font.fallback.clone(),
            config.font.weight,
            [
                &config.colors.background,
                &config.colors.foreground,
                &config.colors.cursor,
                &config.colors.selection,
            ],
            config.window_opacity,
        )
        .map_err(|error| error.to_string())?;
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
        self.script_snapshot = snapshot;
        self.status_text.clear();
        self.next_status_at = self
            .script_snapshot
            .config
            .status_interval
            .map(|_| Instant::now());
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
        Ok(())
    }

    fn emit_script_event(&mut self, name: &str) -> Result<(), String> {
        let pane = self
            .mux
            .current_pane()
            .ok_or_else(|| "mux has no current pane".to_owned())?;
        let name = match name {
            "app_started" => "app_started",
            "config_reloaded" => "config_reloaded",
            _ => return Err(format!("unsupported application event {name}")),
        };
        let mut event = RubyEvent::new(name);
        event.pane = Some(pane);
        self.runtime_events.push_back(event);
        self.deliver_runtime_events()
    }

    fn collect_mux_events(&mut self) {
        let events = self.mux.drain_events().collect::<Vec<_>>();
        for event in events {
            let event = match event {
                MuxEvent::WorkspaceChanged { workspace } => {
                    let mut event = RubyEvent::new("workspace_changed");
                    event.workspace = Some(workspace);
                    Some(event)
                }
                MuxEvent::WindowCreated { window } => {
                    let mut event = RubyEvent::new("window_created");
                    event.window = Some(window);
                    Some(event)
                }
                MuxEvent::WindowClosed { window } => {
                    let mut event = RubyEvent::new("window_closed");
                    event.window = Some(window);
                    Some(event)
                }
                MuxEvent::TabCreated { tab } => {
                    let mut event = RubyEvent::new("tab_created");
                    event.tab = Some(tab);
                    Some(event)
                }
                MuxEvent::TabClosed { tab } => {
                    let mut event = RubyEvent::new("tab_closed");
                    event.tab = Some(tab);
                    Some(event)
                }
                MuxEvent::PaneCreated { pane } => {
                    let mut event = RubyEvent::new("pane_created");
                    event.pane = Some(pane);
                    Some(event)
                }
                MuxEvent::PaneClosed { pane } => {
                    let mut event = RubyEvent::new("pane_closed");
                    event.pane = Some(pane);
                    Some(event)
                }
                MuxEvent::PaneFocused { pane } => {
                    let mut event = RubyEvent::new("pane_focused");
                    event.pane = Some(pane);
                    Some(event)
                }
                MuxEvent::TextQueued { .. } => None,
            };
            self.runtime_events.extend(event);
        }
    }

    fn deliver_runtime_events(&mut self) -> Result<(), String> {
        const MAX_EVENTS_PER_TURN: usize = 1_024;
        let mut delivered = 0;
        self.collect_mux_events();
        while let Some(event) = self.runtime_events.pop_front() {
            delivered += 1;
            if delivered > MAX_EVENTS_PER_TURN {
                return Err("Ruby runtime event delivery exceeded 1024 events".to_owned());
            }
            if !self.script_snapshot.event_names.contains(event.name) {
                continue;
            }
            self.submit_script(ScriptInvocation::Event(event))?;
        }
        Ok(())
    }

    fn resize_panes(
        &mut self,
        window_size: PhysicalSize<u32>,
        scale_factor: f64,
    ) -> Result<(), String> {
        self.tab_layout = self.calculate_tab_layout(window_size, scale_factor);
        self.workspace_layout = self.calculate_workspace_layout(window_size, scale_factor);
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
                    session.resize(size).map_err(|error| {
                        tracing::error!(
                            target: "toyoterm::pty",
                            operation = error.operation(),
                            %pane,
                            columns = size.columns,
                            rows = size.rows,
                            %error,
                            "resize pane PTY failed"
                        );
                        error.to_string()
                    })?;
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
            session.write(bytes).map_err(|error| {
                tracing::error!(
                    target: "toyoterm::pty",
                    operation = error.operation(),
                    %pane,
                    bytes = bytes.len(),
                    %error,
                    "write pane PTY failed"
                );
                error.to_string()
            })?;
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
            PaneRect::new(0, 0, window_size.width, workspace_bar_height(scale_factor)),
            chrome_item_width(scale_factor),
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
                window_size.width,
                tab_bar_height(scale_factor),
            ),
            chrome_item_width(scale_factor),
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
        let status_height = self
            .script_snapshot
            .config
            .status_interval
            .map(|_| status_bar_height(scale_factor))
            .unwrap_or(0)
            .min(window_size.height.saturating_sub(chrome_height));
        PaneLayout::calculate(
            root,
            PaneRect::new(
                0,
                chrome_height,
                window_size.width,
                window_size
                    .height
                    .saturating_sub(chrome_height)
                    .saturating_sub(status_height),
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

    fn close_exited_pane(
        &mut self,
        event_loop: &ActiveEventLoop,
        pane: PaneId,
    ) -> Result<(), String> {
        // Closing a pane also closes its PTY reader, which can leave a stale
        // EOF event in the queue. There is nothing left to reconcile then.
        if !self.pane_runtimes.contains_key(&pane) {
            return Ok(());
        }
        if self
            .mux
            .close_exited_pane(pane)
            .map_err(|error| error.to_string())?
        {
            event_loop.exit();
            return Ok(());
        }
        self.reconcile_pane_runtimes()?;
        self.deliver_runtime_events()?;
        if let Some(window) = self.window.clone() {
            self.sync_active_renderer(window.scale_factor());
            window.request_redraw();
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
            if self.palette_open {
                self.palette_open = false;
                self.palette.close();
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
                } else if let Some(notice) = self.config_error_notice.as_mut()
                    && open_log
                {
                    notice.log_expanded = !notice.log_expanded;
                    notice.console_hint = false;
                }
                if open_ruby_console {
                    self.open_ruby_console();
                }
                if open_log || open_ruby_console || dismiss {
                    if let Err(error) =
                        self.resize_panes(window.inner_size(), window.scale_factor())
                    {
                        tracing::warn!(target: "toyoterm::render", %error, "resize after config notification failed");
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
                    tracing::warn!(target: "toyoterm::mux", %error, %workspace, "activate workspace failed");
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
                    tracing::warn!(target: "toyoterm::mux", %error, %tab, "activate tab failed");
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
                tracing::warn!(target: "toyoterm::mux", %error, pane = %hovered, "focus pane failed");
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

    fn script_context(&mut self) -> Result<ScriptContext, String> {
        let clipboard = self
            .clipboard()
            .and_then(|clipboard| {
                clipboard
                    .get_text()
                    .map_err(|error| format!("read clipboard for Ruby: {error}"))
            })
            .ok();
        Ok(ScriptContext {
            model: ruby_object_model(&self.mux, Some(&self.pane_runtimes))?,
            handles: self.mux.native_handles(),
            clipboard,
        })
    }

    fn submit_script(&mut self, invocation: ScriptInvocation) -> Result<u64, String> {
        let id = self.next_script_request;
        self.next_script_request = self.next_script_request.wrapping_add(1).max(1);
        self.pending_script.push_back((id, invocation));
        self.start_next_script()?;
        Ok(id)
    }

    fn start_next_script(&mut self) -> Result<(), String> {
        if self.script_in_flight {
            return Ok(());
        }
        let Some((id, invocation)) = self.pending_script.pop_front() else {
            return Ok(());
        };
        let request = ScriptRequest {
            id,
            context: self.script_context()?,
            invocation,
        };
        self.script_thread
            .submit(request)
            .map_err(|error| error.to_string())?;
        self.script_in_flight = true;
        Ok(())
    }

    fn handle_script_completion(&mut self, completion: ScriptCompletion) -> Result<(), String> {
        let waiter = self.eval_waiters.remove(&completion.id);
        let is_reload = matches!(completion.invocation, ScriptInvocation::Reload);
        let is_status = matches!(completion.invocation, ScriptInvocation::Status);
        let mut result = match completion.result {
            Ok(result) => result,
            Err(error) => {
                let message = error.to_string();
                tracing::warn!(
                    target: "toyoterm::script",
                    operation = error.operation(),
                    request_id = completion.id,
                    %error,
                    "script request failed"
                );
                if is_reload {
                    self.config_error_notice = Some(ConfigErrorNotice {
                        message: message.clone(),
                        log_expanded: false,
                        console_hint: false,
                    });
                }
                if is_status {
                    self.status_pending = false;
                    self.next_status_at = self
                        .script_snapshot
                        .config
                        .status_interval
                        .map(|interval| Instant::now() + interval);
                }
                self.finish_eval(waiter, Err(message));
                return Ok(());
            }
        };

        if is_status {
            self.status_pending = false;
            self.status_text = result.value.take().unwrap_or_default();
            self.next_status_at = self
                .script_snapshot
                .config
                .status_interval
                .map(|interval| Instant::now() + interval);
        }
        let value = result.value.unwrap_or_default();
        let apply_result: Result<(), String> = (|| {
            if let Some(snapshot) = result.snapshot {
                self.config_error_notice = None;
                self.apply_script_snapshot(snapshot)?;
            }
            let mut reload_requested = false;
            for command in result.commands {
                match command {
                    NativeCommand::Mux(command) => {
                        self.mux
                            .dispatch(command)
                            .map_err(|error| error.to_string())?;
                    }
                    NativeCommand::ClipboardWrite(text) => self.pending_clipboard_writes.push(text),
                    NativeCommand::ReloadConfig => reload_requested = true,
                }
            }
            self.flush_script_clipboard_writes()?;
            self.reconcile_pane_runtimes()?;
            self.flush_mux_input()?;
            self.deliver_runtime_events()?;
            if reload_requested && !is_reload {
                self.reload_config_with_notification()?;
            }
            Ok(())
        })();
        if let Err(error) = apply_result {
            self.finish_eval(waiter, Err(error.clone()));
            return Err(error);
        }
        self.finish_eval(waiter, Ok(value));
        Ok(())
    }

    fn finish_eval(&mut self, waiter: Option<EvalWaiter>, result: Result<String, String>) {
        match waiter {
            Some(EvalWaiter::Ipc(response)) => {
                let _ = response.send(result);
            }
            Some(EvalWaiter::Palette(source)) => match result {
                Ok(value) => self.palette.push_console_result(&source, Ok(&value)),
                Err(error) if toyoterm_ipc::is_incomplete_ruby_error(&error) => {
                    self.palette.insert(&source);
                    self.palette.insert("\n");
                }
                Err(error) => self.palette.push_console_result(&source, Err(&error)),
            },
            None => {}
        }
    }

    fn flush_script_clipboard_writes(&mut self) -> Result<(), String> {
        if self.pending_clipboard_writes.is_empty() {
            return Ok(());
        }
        let writes = std::mem::take(&mut self.pending_clipboard_writes);
        for text in writes {
            self.clipboard()?
                .set_text(text)
                .map_err(|error| format!("write clipboard from Ruby: {error}"))?;
        }
        Ok(())
    }

    fn recover_renderer(&mut self) -> Result<(), String> {
        let window = self
            .window
            .clone()
            .ok_or_else(|| "recover GPU renderer: window is unavailable".to_owned())?;
        tracing::warn!(
            target: "toyoterm::render",
            width = window.inner_size().width,
            height = window.inner_size().height,
            scale_factor = window.scale_factor(),
            "recreating renderer after GPU device loss"
        );
        let mut renderer = pollster::block_on(GpuRenderer::new(window.clone()))
            .map_err(|error| format!("recover GPU renderer: {error}"))?;
        renderer.set_style(self.render_style.clone());
        renderer.resize(window.inner_size());
        self.renderer = Some(renderer);
        self.sync_active_renderer(window.scale_factor());
        window.request_redraw();
        tracing::info!(target: "toyoterm::render", "GPU renderer recovery completed");
        Ok(())
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
                    format!(
                        "Tab {}",
                        self.mux
                            .tab_number(placement.tab)
                            .expect("layout tab exists in mux")
                    ),
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
        let palette_text = self.palette_render_text();
        let palette_rect = self.window.as_ref().map(|window| {
            let size = window.inner_size();
            let width = size
                .width
                .min((560.0 * scale_factor.max(0.1)).round() as u32);
            let height = size
                .height
                .min((360.0 * scale_factor.max(0.1)).round() as u32);
            PaneRect::new(
                (size.width - width) / 2,
                (size.height - height) / 4,
                width,
                height,
            )
        });
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.update_panes(&panes, layout);
            renderer.update_tabs(&tabs, layout);
            renderer.update_workspaces(&workspaces, layout);
            renderer.update_palette(
                self.palette_open.then(|| PaletteRenderData {
                    rect: palette_rect.unwrap_or_default(),
                    text: &palette_text,
                }),
                layout,
            );
            renderer.update_status_bar(
                self.script_snapshot
                    .config
                    .status_interval
                    .map(|_| StatusBarRenderData {
                        rect: status_bar_rect(
                            self.window
                                .as_ref()
                                .map(|window| window.inner_size())
                                .unwrap_or_default(),
                            scale_factor,
                        ),
                        text: &self.status_text,
                    }),
                layout,
            );
            renderer.update_config_error(config_error, layout);
            renderer.update_preedit(self.ime_preedit.as_deref(), layout);
        }
        self.update_ime_cursor_area(scale_factor);
        self.update_window_title();
    }

    fn palette_render_text(&self) -> String {
        if self.palette.is_console() {
            let mut lines = vec!["Ruby Console  (Esc to close)".to_owned()];
            lines.extend(self.palette.console_output().iter().cloned());
            lines.push(format!("toyoterm> {}▏", self.palette.query()));
            return lines.join("\n");
        }
        let items = filter_items(&self.palette_items(), self.palette.query());
        let mut lines = vec![format!("> {}▏", self.palette.query())];
        if items.is_empty() {
            lines.push("  No matching commands".into());
        } else {
            let start = self.palette.selected().saturating_sub(11);
            lines.extend(
                items
                    .iter()
                    .enumerate()
                    .skip(start)
                    .take(12)
                    .map(|(index, item)| {
                        format!(
                            "{} {}",
                            if index == self.palette.selected() {
                                "›"
                            } else {
                                " "
                            },
                            item.label
                        )
                    }),
            );
        }
        lines.join("\n")
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
            .and_then(|tab| self.mux.tab_number(tab))
            .map(|number| format!("Tab {number} · "))
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

fn palette_native_command(
    action: &PaletteAction,
    current_pane: Option<PaneId>,
) -> Result<Option<NativeCommand>, String> {
    Ok(match action {
        PaletteAction::ReloadConfig => Some(NativeCommand::ReloadConfig),
        PaletteAction::NewTab => Some(NativeCommand::Mux(Command::NewTab)),
        PaletteAction::Split(direction) => Some(NativeCommand::Mux(Command::Split {
            pane: current_pane.ok_or_else(|| "mux has no current pane".to_owned())?,
            direction: *direction,
        })),
        PaletteAction::ClosePane => Some(NativeCommand::Mux(Command::ClosePane(
            current_pane.ok_or_else(|| "mux has no current pane".to_owned())?,
        ))),
        PaletteAction::SwitchWorkspace(name) => {
            Some(NativeCommand::Mux(Command::SwitchWorkspace(name.clone())))
        }
        PaletteAction::RubyConsole | PaletteAction::UserCommand(_) => None,
    })
}

impl Drop for ToyotermApplication {
    fn drop(&mut self) {
        // This is also reached while unwinding from a panic. PaneRuntime and
        // native PTY sessions provide a second idempotent kill-on-drop guard.
        self.shutdown();
    }
}

#[cfg(test)]
#[derive(Debug, Default, Eq, PartialEq)]
struct NativeCommandEffects {
    clipboard_writes: Vec<String>,
    reload_requested: bool,
}

fn ruby_object_model(
    mux: &Mux,
    pane_runtimes: Option<&HashMap<PaneId, PaneRuntime>>,
) -> Result<RubyObjectModel, String> {
    let current_workspace = mux.current_workspace();
    let current_window = mux
        .current_window()
        .ok_or_else(|| "mux has no current window".to_owned())?;
    let current_tab = mux
        .current_tab()
        .ok_or_else(|| "mux has no current tab".to_owned())?;
    let current_pane = mux
        .current_pane()
        .ok_or_else(|| "mux has no current pane".to_owned())?;
    let mut workspaces = Vec::new();
    let mut windows = Vec::new();
    let mut tabs = Vec::new();
    let mut panes = Vec::new();

    for workspace_id in mux.workspaces() {
        let window_ids = mux
            .workspace_windows(workspace_id)
            .ok_or_else(|| format!("workspace {workspace_id} is missing"))?
            .to_vec();
        workspaces.push(RubyWorkspace {
            id: workspace_id,
            name: mux
                .workspace_name(workspace_id)
                .unwrap_or_default()
                .to_owned(),
            windows: window_ids.clone(),
        });
        for window_id in window_ids {
            let tab_ids = mux
                .tabs(window_id)
                .ok_or_else(|| format!("window {window_id} is missing"))?
                .to_vec();
            windows.push(RubyWindow {
                id: window_id,
                tabs: tab_ids.clone(),
            });
            for tab_id in tab_ids {
                let pane_ids = mux
                    .tab_panes(tab_id)
                    .ok_or_else(|| format!("tab {tab_id} is missing"))?;
                let tab_number = mux
                    .tab_number(tab_id)
                    .ok_or_else(|| format!("tab {tab_id} is missing from its window"))?;
                tabs.push(RubyTab {
                    id: tab_id,
                    title: format!("Tab {tab_number}"),
                    panes: pane_ids.clone(),
                });
                for pane_id in pane_ids {
                    let runtime = pane_runtimes.and_then(|runtimes| runtimes.get(&pane_id));
                    panes.push(RubyPane {
                        id: pane_id,
                        title: runtime
                            .map(|runtime| runtime.title.clone())
                            .unwrap_or_else(|| format!("Pane {}", pane_id.0)),
                        cwd: runtime
                            .and_then(|runtime| runtime.cwd.as_ref())
                            .map(|cwd| cwd.display().to_string()),
                        pid: runtime.and_then(|runtime| runtime.process_id),
                        command_running: runtime.is_some_and(|runtime| runtime.command_running),
                        last_exit_status: runtime.and_then(|runtime| runtime.last_exit_status),
                    });
                }
            }
        }
    }

    workspaces.sort_by_key(|workspace| workspace.id);
    windows.sort_by_key(|window| window.id);
    tabs.sort_by_key(|tab| tab.id);
    panes.sort_by_key(|pane| pane.id);
    Ok(RubyObjectModel {
        current_workspace,
        current_window,
        current_tab,
        current_pane,
        workspaces,
        windows,
        tabs,
        panes,
    })
}

#[cfg(test)]
fn dispatch_script_commands(
    config_manager: &mut ConfigManager,
    mux: &mut Mux,
) -> Result<NativeCommandEffects, String> {
    let current_workspace = mux.current_workspace();
    let current_window = mux
        .current_window()
        .ok_or_else(|| "mux has no current window".to_owned())?;
    let current_tab = mux
        .current_tab()
        .ok_or_else(|| "mux has no current tab".to_owned())?;
    let current_pane = mux
        .current_pane()
        .ok_or_else(|| "mux has no current pane".to_owned())?;
    let model = ruby_object_model(mux, None)?;
    config_manager
        .set_live_handles(mux.native_handles())
        .map_err(|error| error.to_string())?;
    config_manager
        .set_object_model(&model)
        .map_err(|error| error.to_string())?;
    let mut effects = NativeCommandEffects::default();
    for command in config_manager
        .drain_commands_with_context(current_workspace, current_window, current_tab, current_pane)
        .map_err(|error| error.to_string())?
    {
        match command {
            NativeCommand::Mux(command) => {
                mux.dispatch(command).map_err(|error| error.to_string())?;
            }
            NativeCommand::ClipboardWrite(text) => effects.clipboard_writes.push(text),
            NativeCommand::ReloadConfig => effects.reload_requested = true,
        }
    }
    Ok(effects)
}

fn tab_bar_height(scale_factor: f64) -> u32 {
    (30.0 * scale_factor.max(0.1)).round() as u32
}

fn chrome_item_width(scale_factor: f64) -> u32 {
    (160.0 * scale_factor.max(0.1)).round() as u32
}

fn workspace_bar_height(scale_factor: f64) -> u32 {
    (24.0 * scale_factor.max(0.1)).round() as u32
}

fn status_bar_height(scale_factor: f64) -> u32 {
    (24.0 * scale_factor.max(0.1)).round() as u32
}

fn status_bar_rect(window_size: PhysicalSize<u32>, scale_factor: f64) -> PaneRect {
    let height = status_bar_height(scale_factor).min(window_size.height);
    PaneRect::new(
        0,
        window_size.height.saturating_sub(height),
        window_size.width,
        height,
    )
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
    CommandPalette,
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
                Some(key) if key.eq_ignore_ascii_case("p") => {
                    Some(GuiManagementShortcut::CommandPalette)
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
            Some(key) if key.eq_ignore_ascii_case("p") && modifiers.shift_key() => {
                Some(GuiManagementShortcut::CommandPalette)
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

    struct KillTrackingSession(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    impl PtySession for KillTrackingSession {
        fn process_id(&self) -> Option<u32> {
            Some(42)
        }

        fn take_reader(&mut self) -> Result<Box<dyn Read + Send>, crate::PtyError> {
            Ok(Box::new(std::io::Cursor::new(Vec::<u8>::new())))
        }

        fn write(&mut self, _data: &[u8]) -> Result<(), crate::PtyError> {
            Ok(())
        }

        fn resize(&mut self, _size: PtySize) -> Result<(), crate::PtyError> {
            Ok(())
        }

        fn try_wait(&mut self) -> Result<Option<crate::PtyExitStatus>, crate::PtyError> {
            Ok(None)
        }

        fn wait(&mut self) -> Result<crate::PtyExitStatus, crate::PtyError> {
            Ok(crate::PtyExitStatus {
                code: 0,
                signal: None,
            })
        }

        fn kill(&mut self) -> Result<(), crate::PtyError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn pane_runtime_kills_its_child_when_dropped_during_shutdown() {
        let kills = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        {
            let _runtime = PaneRuntime {
                terminal: AlacrittyTerminalBackend::new(80, 24),
                pty_session: Some(Box::new(KillTrackingSession(kills.clone()))),
                process_id: Some(42),
                title: "test".into(),
                cwd: None,
                command_running: false,
                last_exit_status: None,
                exited: false,
            };
        }

        assert_eq!(kills.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn cli_ruby_and_palette_normalize_to_the_same_command() {
        let pane = PaneId(42);
        let expected = NativeCommand::Mux(Command::Split {
            pane,
            direction: SplitDirection::Right,
        });

        let ipc = IpcRequest::Split {
            direction: SplitDirection::Right,
        }
        .native_command(Some(pane))
        .unwrap();
        let palette =
            palette_native_command(&PaletteAction::Split(SplitDirection::Right), Some(pane))
                .unwrap();
        let mut ruby = ConfigManager::new().unwrap();
        ruby.reload("Toyoterm.current_pane.split(:right)").unwrap();

        assert_eq!(ipc, Some(expected.clone()));
        assert_eq!(palette, Some(expected.clone()));
        assert_eq!(ruby.drain_commands(pane).unwrap(), vec![expected]);
    }

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

        let effects = dispatch_script_commands(&mut config_manager, &mut mux).unwrap();

        assert_eq!(mux.take_pending_input(pane).unwrap(), b"echo hello\n");
        assert_eq!(effects, NativeCommandEffects::default());
    }

    #[test]
    fn separates_ruby_clipboard_writes_from_mux_commands() {
        let mut config_manager = ConfigManager::new().unwrap();
        config_manager
            .eval(r#"Toyoterm.clipboard.write("from Ruby")"#)
            .unwrap();
        let mut mux = Mux::new();

        assert_eq!(
            dispatch_script_commands(&mut config_manager, &mut mux).unwrap(),
            NativeCommandEffects {
                clipboard_writes: vec!["from Ruby".to_owned()],
                reload_requested: false,
            }
        );
    }

    #[test]
    fn normalizes_ruby_reload_requests_as_native_commands() {
        let mut config_manager = ConfigManager::new().unwrap();
        config_manager.eval("Toyoterm.reload_config").unwrap();
        let mut mux = Mux::new();

        assert_eq!(
            dispatch_script_commands(&mut config_manager, &mut mux).unwrap(),
            NativeCommandEffects {
                clipboard_writes: Vec::new(),
                reload_requested: true,
            }
        );
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
                &Key::Character("p".into()),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                ShortcutPlatform::LinuxOrWindows,
            ),
            Some(GuiManagementShortcut::CommandPalette)
        );
        assert_eq!(
            gui_management_shortcut(
                &Key::Character("p".into()),
                ModifiersState::SUPER | ModifiersState::SHIFT,
                ShortcutPlatform::MacOs,
            ),
            Some(GuiManagementShortcut::CommandPalette)
        );
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
    fn status_bar_occupies_the_scaled_bottom_edge() {
        assert_eq!(
            status_bar_rect(PhysicalSize::new(960, 600), 1.0),
            PaneRect::new(0, 576, 960, 24)
        );
        assert_eq!(
            status_bar_rect(PhysicalSize::new(1200, 750), 1.5),
            PaneRect::new(0, 714, 1200, 36)
        );
    }

    #[test]
    fn chrome_item_width_scales_from_a_shared_logical_width() {
        assert_eq!(chrome_item_width(1.0), 160);
        assert_eq!(chrome_item_width(1.5), 240);
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
