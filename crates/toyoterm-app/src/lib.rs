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
use winit::window::{Fullscreen, Window, WindowId};

use toyoterm_script::{
    RubyEvent, RubyObjectModel, RubyPane, RubyTab, RubyWindow, RubyWorkspace, ScriptCompletion,
    ScriptContext, ScriptInvocation, ScriptRequest, ScriptSnapshot, ScriptThread,
};

mod command_dispatch;
mod input;
mod lifecycle;
mod logging;
mod object_model;
mod pane_lifecycle;
mod render_coordinator;
mod runtime_events;
mod ui_geometry;

use input::*;
use object_model::*;
use ui_geometry::*;

pub use lifecycle::install_panic_hook;
pub use logging::init_logging;
pub use toyoterm_api::{
    Command, CommandResult, Event as MuxEvent, NativeAction, NativeCommand, PaneId, PaneLaunchSpec,
    PaneSearchDirection, SelectionMotion, SplitDirection,
};
pub use toyoterm_config::ToyotermConfig;
pub use toyoterm_ipc::{IpcRequest, IpcResponse, IpcServer};
pub use toyoterm_mux::Mux;
pub use toyoterm_pty::{NativePty, Pty, PtyCommand, PtyError, PtyExitStatus, PtySession, PtySize};
pub use toyoterm_render::{
    ConfigErrorLayout, ConfigErrorRenderData, GpuRenderer, PaneLayout, PaneRect, PaneRenderData,
    RenderOutcome, RenderStyle, SearchRenderData, StatusBarRenderData, TabRenderData,
    TabStripLayout, TextLayout, WorkspaceRenderData, WorkspaceStripLayout,
};
pub use toyoterm_script::ConfigManager;
pub use toyoterm_terminal::{
    AlacrittyTerminalBackend, BindingKey, CursorShape, KeyChord, KeyModifiers, KeyPress, KeypadKey,
    MouseWheelDirection, SearchDirection, SearchResult, SelectionKind, TerminalBackend,
    TerminalEvent, TerminalKey, TerminalMode, encode_key, encode_mouse_wheel, encode_paste,
};

const MULTI_CLICK_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ClickTarget {
    pane: PaneId,
    column: u16,
    row: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct VisualPosition {
    column: u16,
    row: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct VisualSelection {
    anchor: Option<VisualPosition>,
    current: VisualPosition,
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
}

impl ConfigErrorNotice {
    fn display_message(&self) -> String {
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
    let render_style = RenderStyle::from_hex_with_ui(
        &config.font.family,
        config.font.fallback.clone(),
        config.font.weight,
        [
            &config.colors.background,
            &config.colors.foreground,
            &config.colors.cursor,
            &config.colors.selection,
            &config.colors.tab_bar,
            &config.colors.tab_active,
            &config.colors.tab_inactive,
            &config.colors.workspace_bar,
            &config.colors.status_bar,
            &config.colors.pane_border,
            &config.colors.search_match,
            &config.colors.search_match_active,
        ],
        &config.colors.ansi,
        config.window.opacity,
        config.ui.active_pane_border_width,
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
    pending_pane_launches: HashMap<PaneId, PaneLaunchSpec>,
    pane_layout: PaneLayout,
    tab_layout: TabStripLayout,
    workspace_layout: WorkspaceStripLayout,
    search_open: bool,
    search_query: String,
    search_result: SearchResult,
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
    visual_selection: Option<VisualSelection>,
    click_tracker: ClickTracker,
    clipboard: Option<Clipboard>,
    pending_clipboard_writes: Vec<String>,
    pane_badges: HashMap<PaneId, String>,
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

fn ruby_event_from_terminal_event(pane: PaneId, event: TerminalEvent) -> Option<RubyEvent> {
    let mut event = match event {
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
        TerminalEvent::CommandStarted => RubyEvent::new("command_started"),
        TerminalEvent::CommandFinished(exit_status) => {
            let mut event = RubyEvent::new("command_finished");
            event.exit_status = exit_status;
            event
        }
        TerminalEvent::PtyWrite(_) => return None,
        TerminalEvent::Bell => RubyEvent::new("bell"),
    };
    event.pane = Some(pane);
    Some(event)
}

impl ApplicationHandler<AppEvent> for ToyotermApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title(&self.script_snapshot.config.window.title)
            .with_transparent(self.script_snapshot.config.window.opacity < 1.0)
            .with_inner_size(LogicalSize::new(
                self.script_snapshot.config.window.width,
                self.script_snapshot.config.window.height,
            ))
            .with_min_inner_size(LogicalSize::new(
                self.script_snapshot.config.window.min_width,
                self.script_snapshot.config.window.min_height,
            ))
            .with_decorations(self.script_snapshot.config.window.decorations)
            .with_resizable(self.script_snapshot.config.window.resizable)
            .with_window_level(if self.script_snapshot.config.window.always_on_top {
                winit::window::WindowLevel::AlwaysOnTop
            } else {
                winit::window::WindowLevel::Normal
            });
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.fail(event_loop, format!("create window: {error}"));
                return;
            }
        };
        let mut renderer =
            match pollster::block_on(GpuRenderer::new(window.clone(), self.render_style.clone())) {
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
        self.cell_metrics.width =
            f64::from(renderer.terminal_cell_width(self.cell_metrics.font_size));
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
                    self.exit_visual_mode();
                    if self.search_open {
                        self.close_search();
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
                if self.search_open {
                    self.handle_search_key(&event, modifiers);
                    self.sync_active_renderer(window.scale_factor());
                    window.request_redraw();
                    return;
                }
                if self.visual_selection.is_some() {
                    match self.handle_keybinding(&event, modifiers) {
                        Ok(true) => {}
                        Ok(false) if matches!(event.logical_key, Key::Named(NamedKey::Escape)) => {
                            self.exit_visual_mode();
                        }
                        Ok(false) => {}
                        Err(error) => {
                            tracing::warn!(target: "toyoterm::script", %error, "visual selection key binding failed");
                        }
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
                // Physical bindings are checked before logical bindings by the resolver.
                match self.handle_keybinding(&event, modifiers) {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(target: "toyoterm::script", %error, "key binding failed");
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
                if self.visual_selection.is_some() {
                    return;
                }
                if self.search_open {
                    self.search_query.push_str(&text);
                    self.refresh_search(SearchDirection::Next);
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
                    if let Some(runtime_event) = ruby_event_from_terminal_event(pane, event) {
                        self.runtime_events.push_back(runtime_event);
                    }
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
            pending_pane_launches: HashMap::new(),
            pane_layout: PaneLayout::default(),
            tab_layout: TabStripLayout::default(),
            workspace_layout: WorkspaceStripLayout::default(),
            search_open: false,
            search_query: String::new(),
            search_result: SearchResult::default(),
            _ipc_server: ipc_server,
            config_error_layout: ConfigErrorLayout::default(),
            config_error_notice: startup_config_error.map(|message| ConfigErrorNotice {
                message,
                log_expanded: false,
            }),
            ime_preedit: None,
            modifiers: ModifiersState::empty(),
            alt_graph_active: false,
            leader_deadline: None,
            mouse_position: PhysicalPosition::new(0.0, 0.0),
            wheel_line_accumulator: 0.0,
            selecting: false,
            visual_selection: None,
            click_tracker: ClickTracker::default(),
            clipboard: None,
            pending_clipboard_writes: Vec::new(),
            pane_badges: HashMap::new(),
            runtime_events: VecDeque::new(),
            next_script_request: 1,
            script_in_flight: false,
            pending_script: VecDeque::new(),
            eval_waiters: HashMap::new(),
            cell_metrics: CellMetrics {
                width: 9.0 * font_scale,
                height: f64::from(config.font.size * config.ui.line_height),
                horizontal_padding: config.ui.padding_x.round() as u32,
                vertical_padding: config.ui.padding_y.round() as u32,
                font_size: config.font.size,
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

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.fatal_error = Some(error);
        event_loop.exit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_shell_command_lifecycle_to_ruby_events() {
        let pane = PaneId(7);
        let started = ruby_event_from_terminal_event(pane, TerminalEvent::CommandStarted).unwrap();
        assert_eq!(started.name, "command_started");
        assert_eq!(started.pane, Some(pane));
        assert_eq!(started.exit_status, None);

        let finished =
            ruby_event_from_terminal_event(pane, TerminalEvent::CommandFinished(Some(23))).unwrap();
        assert_eq!(finished.name, "command_finished");
        assert_eq!(finished.pane, Some(pane));
        assert_eq!(finished.exit_status, Some(23));
    }

    #[cfg(unix)]
    #[test]
    fn custom_pane_launch_applies_argv_cwd_and_environment() {
        let cwd = std::env::temp_dir();
        let expected_cwd = cwd
            .canonicalize()
            .expect("canonicalize temporary directory");
        let launch = PaneLaunchSpec {
            program: Some("/bin/sh".into()),
            args: vec![
                "-c".into(),
                "printf '%s|%s' \"$PWD\" \"$TOYOTERM_LAUNCH_TEST\"".into(),
            ],
            cwd: Some(cwd.display().to_string()),
            environment: vec![("TOYOTERM_LAUNCH_TEST".into(), Some("works".into()))],
        };
        let command = pane_lifecycle::pty_command_for_launch(None, Some(&launch));
        let mut session = NativePty
            .spawn(command, PtySize::new(80, 24))
            .expect("spawn custom pane command");
        let mut reader = session.take_reader().expect("take PTY reader");
        let mut output = String::new();
        reader.read_to_string(&mut output).expect("read PTY output");
        let status = session.wait().expect("wait for custom pane command");

        assert_eq!(status.code, 0);
        assert!(
            output.contains(&format!("{}|works", expected_cwd.display())),
            "unexpected output: {output:?}"
        );
    }

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
    fn config_error_notice_switches_between_summary_and_log() {
        let mut notice = ConfigErrorNotice {
            message: "error\nconfig.rb:2\nconfig.rb:5\nextra".into(),
            log_expanded: false,
        };
        assert_eq!(notice.display_message(), "error\nconfig.rb:2\nconfig.rb:5");

        notice.log_expanded = true;
        assert_eq!(notice.display_message(), notice.message);
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
    fn hyperlink_hit_testing_and_scheme_allowlist_are_safe() {
        let mut terminal = AlacrittyTerminalBackend::new(30, 2);
        terminal.advance(b"visit https://example.com now");
        let snapshot = terminal.snapshot();

        assert_eq!(
            hyperlink_at(&snapshot, 8, 0).as_deref(),
            Some("https://example.com")
        );
        assert_eq!(hyperlink_at(&snapshot, 0, 0), None);
        assert!(validate_allowed_url("https://example.com/path").is_ok());
        assert!(validate_allowed_url("mailto:user@example.com").is_ok());
        assert!(validate_allowed_url("file:///etc/passwd").is_err());
        assert!(validate_allowed_url("javascript:alert(1)").is_err());
        assert!(validate_allowed_url("https://example.com\ncommand").is_err());
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
            status_bar_rect(PhysicalSize::new(960, 600), 24),
            PaneRect::new(0, 576, 960, 24)
        );
        assert_eq!(
            status_bar_rect(PhysicalSize::new(1200, 750), 36),
            PaneRect::new(0, 714, 1200, 36)
        );
    }

    #[test]
    fn ui_sizes_scale_from_logical_pixels() {
        assert_eq!(scaled_ui_size(160.0, 1.0), 160);
        assert_eq!(scaled_ui_size(160.0, 1.5), 240);
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
