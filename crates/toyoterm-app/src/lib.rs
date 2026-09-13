use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use arboard::Clipboard;
use base64::Engine;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{CursorIcon, Fullscreen, UserAttentionType, Window, WindowId};

#[cfg(target_os = "linux")]
use winit::platform::wayland::WindowAttributesExtWayland;
#[cfg(target_os = "windows")]
use winit::platform::windows::WindowAttributesExtWindows;

use toyoterm_script::{
    AsyncProcessOutput, BarItem, RubyEvent, RubyObjectModel, RubyPane, RubyTab, RubyWindow,
    RubyWorkspace, ScriptCompletion, ScriptContext, ScriptInvocation, ScriptRequest,
    ScriptSnapshot, ScriptThread,
};

mod command_dispatch;
mod input;
mod lifecycle;
mod logging;
mod notifications;
mod object_model;
mod pane_lifecycle;
mod render_coordinator;
mod runtime_events;
mod selector;
mod ui_geometry;

use input::*;
use notifications::{DesktopNotification, NotificationSender};
use object_model::*;
use selector::*;
use ui_geometry::*;

pub use lifecycle::install_panic_hook;
pub use logging::init_logging;
pub use toyoterm_api::{
    ActionContext, Command, CommandResult, Event as MuxEvent, NativeAction, NativeCommand, PaneId,
    PaneLaunchSpec, PaneSearchDirection, ScriptEventKind, SelectionMotion, SplitDirection,
};
pub use toyoterm_config::{StatusBarPosition, ToyotermConfig};
pub use toyoterm_ipc::{IpcRequest, IpcResponse, IpcServer};
pub use toyoterm_mux::Mux;
pub use toyoterm_pty::{NativePty, Pty, PtyCommand, PtyError, PtyExitStatus, PtySession, PtySize};
pub use toyoterm_render::{
    ConfigErrorLayout, ConfigErrorRenderData, GpuRenderer, PaneLayout, PaneRect, PaneRenderData,
    RenderOutcome, RenderStyle, SearchRenderData, SelectorRenderData, StatusBarAlignment,
    StatusBarEdge, StatusBarRenderData, StatusBarRenderItem, TabRenderData, TabStripLayout,
    TextLayout, WorkspaceRenderData, WorkspaceStripLayout,
};
pub use toyoterm_script::ConfigManager;
pub use toyoterm_terminal::{
    AlacrittyTerminalBackend, BindingKey, CursorShape, KeyChord, KeyModifiers, KeyPress, KeypadKey,
    MouseWheelDirection, NotificationOccasion, NotificationSound, NotificationUrgency,
    SearchDirection, SearchResult, SelectionKind, SessionStatusUpdate, TabColorComponent,
    TerminalAttention, TerminalBackend, TerminalEvent, TerminalKey, TerminalMode, TerminalProgress,
    encode_key, encode_mouse_wheel, encode_paste,
};

const MULTI_CLICK_INTERVAL: Duration = Duration::from_millis(500);
const OSC_NOTIFICATION_INTERVAL: Duration = Duration::from_secs(2);
const OSC_OPEN_URL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_OSC_USER_VARS: usize = 64;
const MAX_OSC_REPORT_VARIABLE_VALUE_BYTES: usize = 4 * 1024;

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
        let mut size = PtySize::new(
            columns.clamp(2, u16::MAX.into()) as u16,
            rows.clamp(1, u16::MAX.into()) as u16,
        );
        size.pixel_width = (f64::from(size.columns) * (self.width * scale_factor).max(1.0))
            .round()
            .clamp(1.0, f64::from(u16::MAX)) as u16;
        size.pixel_height = (f64::from(size.rows) * (self.height * scale_factor).max(1.0))
            .round()
            .clamp(1.0, f64::from(u16::MAX)) as u16;
        size
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GuiOptions {
    pub config_path: Option<PathBuf>,
    pub initial_pane: Option<PaneLaunchSpec>,
    pub title: Option<String>,
    pub app_id: Option<String>,
}

pub fn run_gui() -> Result<(), AppError> {
    run_gui_with_options(GuiOptions::default())
}

pub fn run_gui_with_config_path(config_path: Option<&Path>) -> Result<(), AppError> {
    run_gui_with_options(GuiOptions {
        config_path: config_path.map(Path::to_owned),
        ..GuiOptions::default()
    })
}

pub fn run_gui_with_options(options: GuiOptions) -> Result<(), AppError> {
    run_gui_inner(options, false)
}

/// Starts the complete GUI stack and exits after successful initialization.
pub fn run_gui_smoke_test() -> Result<(), AppError> {
    run_gui_inner(GuiOptions::default(), true)
}

fn run_gui_inner(options: GuiOptions, exit_after_startup: bool) -> Result<(), AppError> {
    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .map_err(|error| AppError(error.to_string()))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let event_proxy = event_loop.create_proxy();
    let completion_proxy = event_proxy.clone();
    let (script_thread, startup) =
        ScriptThread::start(options.config_path.clone(), move |completion| {
            let _ = completion_proxy.send_event(AppEvent::ScriptCompleted(Box::new(completion)));
        })
        .map_err(|error| {
            tracing::error!(
                target: "toyoterm::config",
                operation = error.operation(),
                config_path = ?options.config_path,
                %error,
                "load startup config failed"
            );
            AppError(error.to_string())
        })?;
    let startup_config_error = startup.config_error.map(|error| {
        tracing::warn!(
            target: "toyoterm::config",
            operation = error.operation(),
            config_path = ?options.config_path,
            %error,
            "startup config rejected; using defaults"
        );
        error.to_string()
    });
    let config = &startup.snapshot.config;
    let mut render_style = RenderStyle::from_hex_with_ui(
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
            &config.colors.zoomed_pane_border,
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
    render_style.background_image =
        config
            .window
            .background_image
            .as_ref()
            .map(|image| toyoterm_render::BackgroundImage {
                width: image.width,
                height: image.height,
                rgba: image.rgba.clone(),
            });
    render_style.background_image_opacity = config.window.background_image_opacity;
    let mut app = ToyotermApplication::new(
        event_proxy,
        script_thread,
        startup.snapshot,
        render_style,
        startup_config_error,
        exit_after_startup,
        options,
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
    AsyncCompleted {
        id: u64,
        output: AsyncProcessOutput,
    },
}

enum EvalWaiter {
    Ipc(mpsc::Sender<Result<String, String>>),
}

struct PaneRuntime {
    terminal: AlacrittyTerminalBackend,
    pty_session: Option<Box<dyn PtySession>>,
    process_id: Option<u32>,
    title: String,
    icon_title: Option<String>,
    osc_badge: Option<String>,
    cursor_line_highlight: bool,
    cwd: Option<PathBuf>,
    remote_host: Option<String>,
    shell_integration_version: Option<u32>,
    shell_integration_shell: Option<String>,
    user_vars: BTreeMap<String, String>,
    command_running: bool,
    last_exit_status: Option<i32>,
    progress: Option<TerminalProgress>,
    tab_color: TabColorState,
    session_status: SessionStatusState,
    mouse_cursor: CursorIcon,
    last_notification_at: Option<Instant>,
    last_open_url_at: Option<Instant>,
    exited: bool,
}

#[derive(Default)]
struct TabColorState([Option<u8>; 3]);

impl TabColorState {
    fn set(&mut self, component: TabColorComponent, value: u8) {
        self.0[match component {
            TabColorComponent::Red => 0,
            TabColorComponent::Green => 1,
            TabColorComponent::Blue => 2,
        }] = Some(value);
    }

    fn complete(&self) -> Option<[u8; 3]> {
        Some([self.0[0]?, self.0[1]?, self.0[2]?])
    }
}

#[derive(Default)]
struct SessionStatusState {
    indicator: Option<[u8; 3]>,
    status: Option<String>,
    status_color: Option<[u8; 3]>,
}

impl SessionStatusState {
    fn apply(&mut self, update: &SessionStatusUpdate) {
        if let Some(indicator) = update.indicator {
            self.indicator = indicator;
        }
        if let Some(status) = &update.status {
            self.status = status.clone();
        }
        if let Some(status_color) = update.status_color {
            self.status_color = status_color;
        }
    }
}

fn iterm_variable_value(runtime: &PaneRuntime, name: &str) -> Option<String> {
    let (columns, rows) = runtime.terminal.dimensions();
    let value = match name {
        "session.name" => Some(runtime.title.clone()),
        "session.terminalIconName" => runtime.icon_title.clone(),
        "session.columns" => Some(columns.to_string()),
        "session.rows" => Some(rows.to_string()),
        "session.path" => runtime
            .cwd
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        "session.shell" => runtime.shell_integration_shell.clone(),
        "session.hostname" => runtime
            .remote_host
            .as_deref()
            .and_then(|remote_host| remote_host.split_once('@'))
            .map(|(_, hostname)| hostname.to_owned()),
        "session.username" => runtime
            .remote_host
            .as_deref()
            .and_then(|remote_host| remote_host.split_once('@'))
            .map(|(username, _)| username.to_owned()),
        name => name
            .strip_prefix("session.user.")
            .and_then(|name| runtime.user_vars.get(name))
            .cloned(),
    }?;
    (value.len() <= MAX_OSC_REPORT_VARIABLE_VALUE_BYTES).then_some(value)
}

fn iterm_variable_response(runtime: &PaneRuntime, name: &str) -> String {
    let value = iterm_variable_value(runtime, name).unwrap_or_default();
    let encoded = base64::engine::general_purpose::STANDARD.encode(value);
    format!("\x1b]1337;ReportVariable={encoded}\x1b\\")
}

fn interpolate_iterm_badge(runtime: &PaneRuntime, format: &str) -> Option<String> {
    let mut rendered = String::with_capacity(format.len());
    let mut remaining = format;
    while let Some(start) = remaining.find("\\(") {
        rendered.push_str(&remaining[..start]);
        let expression = &remaining[start + 2..];
        let Some(end) = expression.find(')') else {
            rendered.push_str(&remaining[start..]);
            remaining = "";
            break;
        };
        let name = &expression[..end];
        let qualified = name
            .strip_prefix("user.")
            .map(|name| format!("session.user.{name}"));
        let name = qualified.as_deref().unwrap_or(name);
        if name.len() <= toyoterm_terminal::MAX_OSC_REPORT_VARIABLE_NAME_BYTES
            && !name.chars().any(char::is_control)
            && let Some(value) = iterm_variable_value(runtime, name)
        {
            rendered.push_str(&value);
        }
        if rendered.len() > MAX_OSC_REPORT_VARIABLE_VALUE_BYTES {
            return None;
        }
        remaining = &expression[end + 1..];
    }
    rendered.push_str(remaining);
    (rendered.len() <= MAX_OSC_REPORT_VARIABLE_VALUE_BYTES).then_some(rendered)
}

fn apply_iterm_badge_format(runtime: &mut PaneRuntime, format: &str) {
    runtime.osc_badge = if format.is_empty() {
        None
    } else {
        interpolate_iterm_badge(runtime, format)
    };
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
    selector: Option<SelectorOverlay>,
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
    notification_sender: Option<NotificationSender>,
    pane_badges: HashMap<PaneId, String>,
    runtime_events: VecDeque<RubyEvent>,
    next_script_request: u64,
    script_in_flight: bool,
    pending_script: VecDeque<(u64, ScriptInvocation)>,
    script_event_drops: u64,
    eval_waiters: HashMap<u64, EvalWaiter>,
    cell_metrics: CellMetrics,
    script_thread: ScriptThread,
    script_snapshot: ScriptSnapshot,
    bar_items: HashMap<StatusBarPosition, Vec<BarItem>>,
    bar_pending: Option<StatusBarPosition>,
    next_bar_at: HashMap<StatusBarPosition, Instant>,
    cancelled_async_tasks: HashSet<u64>,
    terminal_render_pending: bool,
    mux: Mux,
    render_style: RenderStyle,
    fatal_error: Option<String>,
    exit_after_startup: bool,
    window_title_override: Option<String>,
    #[cfg(target_os = "linux")]
    app_id: Option<String>,
}

fn ruby_event_from_terminal_event(pane: PaneId, event: TerminalEvent) -> Option<RubyEvent> {
    let mut event = match event {
        TerminalEvent::TitleChanged(title) => {
            let mut event = RubyEvent::new(ScriptEventKind::TitleChanged);
            event.title = Some(title);
            event
        }
        TerminalEvent::TitleReset => {
            let mut event = RubyEvent::new(ScriptEventKind::TitleChanged);
            event.title = Some(format!("Pane {}", pane.0));
            event
        }
        TerminalEvent::IconTitleChanged(_) => return None,
        TerminalEvent::CwdChanged(cwd) => {
            let mut event = RubyEvent::new(ScriptEventKind::CwdChanged);
            event.cwd = Some(cwd);
            event
        }
        TerminalEvent::RemoteHostChanged(_) => return None,
        TerminalEvent::ShellIntegrationChanged { .. } => return None,
        TerminalEvent::ItermVariableQuery(_) => return None,
        TerminalEvent::ItermBadgeFormatChanged(_) => return None,
        TerminalEvent::CursorLineHighlightChanged(_) => return None,
        TerminalEvent::AttentionRequested(_) => return None,
        TerminalEvent::OpenUrlRequested(_) => return None,
        TerminalEvent::UserVarChanged { .. } => return None,
        TerminalEvent::MarkSet => return None,
        TerminalEvent::PromptStarted => RubyEvent::new(ScriptEventKind::PromptStarted),
        TerminalEvent::CommandLineStarted => RubyEvent::new(ScriptEventKind::CommandLineStarted),
        TerminalEvent::CommandStarted => RubyEvent::new(ScriptEventKind::CommandStarted),
        TerminalEvent::CommandFinished(exit_status) => {
            let mut event = RubyEvent::new(ScriptEventKind::CommandFinished);
            event.exit_status = exit_status;
            event
        }
        TerminalEvent::MouseCursorChanged(_) => return None,
        TerminalEvent::MouseCursorControl(_) => return None,
        TerminalEvent::ColorControl(_)
        | TerminalEvent::ItermUiColorChanged { .. }
        | TerminalEvent::ItermUiColorReset(_)
        | TerminalEvent::ItermUiColorQuery { .. }
        | TerminalEvent::ColorStackPush
        | TerminalEvent::ColorStackPop
        | TerminalEvent::TabColorChanged { .. }
        | TerminalEvent::TabColorSet(_)
        | TerminalEvent::TabColorReset
        | TerminalEvent::SessionStatusChanged(_) => return None,
        TerminalEvent::ProgressChanged(_) => return None,
        TerminalEvent::ClipboardStore(_)
        | TerminalEvent::ClipboardCaptureStart
        | TerminalEvent::ClipboardCaptureEnd => return None,
        TerminalEvent::Notification { .. } | TerminalEvent::NotificationClose(_) => return None,
        TerminalEvent::PtyWrite(_) => return None,
        TerminalEvent::Bell => RubyEvent::new(ScriptEventKind::Bell),
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
            .with_title(self.base_window_title())
            // Windows transparency must be enabled at creation, including when
            // starting opaque. Later opacity changes only update the GPU alpha.
            .with_transparent(
                cfg!(target_os = "windows") || self.script_snapshot.config.window.opacity < 1.0,
            )
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
        #[cfg(target_os = "linux")]
        let attributes = match self.app_id.as_deref() {
            Some(app_id) => attributes.with_name(app_id, app_id),
            None => attributes,
        };
        // Match the renderer's DX12 DirectComposition visual. An HWND
        // redirection bitmap would otherwise cover its per-pixel transparency.
        #[cfg(target_os = "windows")]
        let attributes = attributes.with_no_redirection_bitmap(true);
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
        if let Err(error) = self.emit_script_event(ScriptEventKind::AppStarted) {
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
                self.update_mouse_cursor(&window);
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
                if self.selector.is_none() {
                    self.handle_left_mouse(&window, state);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.selector.is_none() {
                    self.handle_mouse_wheel(event_loop, &window, delta);
                }
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
                if self.selector.is_some() {
                    if let Err(error) = self.handle_selector_key(&event, modifiers) {
                        tracing::warn!(target: "toyoterm::script", %error, "selector callback submission failed");
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
                    && let Err(error) = self.write_input(&bytes)
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
                if let Some(selector) = self.selector.as_mut() {
                    selector.append_query(&text);
                    self.sync_active_renderer(window.scale_factor());
                    window.request_redraw();
                    return;
                }
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
                if let Err(error) = self.write_input(text.as_bytes()) {
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
        if self.script_snapshot.config.status_bars.is_empty() {
            self.next_bar_at.clear();
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        if self.bar_pending.is_some() || self.window.is_none() {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        let now = Instant::now();
        for bar in &self.script_snapshot.config.status_bars {
            self.next_bar_at.entry(bar.position).or_insert(now);
        }
        let Some((position, deadline, interval)) = self
            .script_snapshot
            .config
            .status_bars
            .iter()
            .filter_map(|bar| {
                self.next_bar_at
                    .get(&bar.position)
                    .map(|deadline| (bar.position, *deadline, bar.interval))
            })
            .min_by_key(|(_, deadline, _)| *deadline)
        else {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        };
        if now < deadline {
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            return;
        }
        match self.submit_script(ScriptInvocation::Bar { position }) {
            Ok(_) => {
                self.bar_pending = Some(position);
                self.next_bar_at.remove(&position);
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Err(error) => {
                tracing::warn!(target: "toyoterm::script", %error, "submit bar callback failed");
                self.next_bar_at.insert(position, now + interval);
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
                let mut mouse_cursor_changed = false;
                let mut osc52_copies = Vec::new();
                let mut notification = None;
                let mut attention_request = None;
                let mut open_urls = Vec::new();
                let allow_notifications =
                    self.script_snapshot.config.behavior.allow_osc_notifications;
                let allow_attention = self
                    .script_snapshot
                    .config
                    .behavior
                    .allow_osc_attention_requests;
                let allow_open_url = self.script_snapshot.config.behavior.allow_osc_open_url;
                let window_focused = self
                    .window
                    .as_ref()
                    .is_some_and(|window| window.has_focus());
                let source_focused = window_focused && self.mux.current_pane() == Some(pane);
                let source_visible = window_focused && self.pane_layout.rect(pane).is_some();
                if let Some(runtime) = self.pane_runtimes.get_mut(&pane) {
                    runtime.terminal.advance(&bytes);
                    terminal_events = runtime.terminal.drain_events();
                    for event in &terminal_events {
                        match event {
                            TerminalEvent::TitleChanged(title) => runtime.title = title.clone(),
                            TerminalEvent::TitleReset => runtime.title = format!("Pane {}", pane.0),
                            TerminalEvent::IconTitleChanged(title) => {
                                runtime.icon_title = Some(title.clone());
                            }
                            TerminalEvent::CwdChanged(cwd) => {
                                runtime.cwd = Some(PathBuf::from(cwd))
                            }
                            TerminalEvent::RemoteHostChanged(remote_host) => {
                                runtime.remote_host = Some(remote_host.clone());
                            }
                            TerminalEvent::ShellIntegrationChanged { version, shell } => {
                                runtime.shell_integration_version = Some(*version);
                                runtime.shell_integration_shell = shell.clone();
                            }
                            TerminalEvent::ItermVariableQuery(name) => {
                                let response = iterm_variable_response(runtime, name);
                                if let Some(session) = runtime.pty_session.as_mut()
                                    && let Err(error) = session.write(response.as_bytes())
                                {
                                    tracing::error!(
                                        target: "toyoterm::pty",
                                        operation = error.operation(),
                                        %pane,
                                        bytes = response.len(),
                                        %error,
                                        "write iTerm2 variable response to pane PTY failed"
                                    );
                                }
                            }
                            TerminalEvent::ItermBadgeFormatChanged(format) => {
                                apply_iterm_badge_format(runtime, format);
                            }
                            TerminalEvent::CursorLineHighlightChanged(enabled) => {
                                runtime.cursor_line_highlight = *enabled;
                            }
                            TerminalEvent::AttentionRequested(TerminalAttention::Cancel) => {
                                attention_request = Some(TerminalAttention::Cancel);
                            }
                            TerminalEvent::AttentionRequested(request) if allow_attention => {
                                attention_request = Some(*request);
                            }
                            TerminalEvent::AttentionRequested(_) => {}
                            TerminalEvent::OpenUrlRequested(url)
                                if should_open_osc_url(
                                    allow_open_url,
                                    url,
                                    runtime.last_open_url_at,
                                    Instant::now(),
                                ) =>
                            {
                                runtime.last_open_url_at = Some(Instant::now());
                                open_urls.push(url.clone());
                            }
                            TerminalEvent::OpenUrlRequested(_) => {}
                            TerminalEvent::UserVarChanged { name, value }
                                if runtime.user_vars.contains_key(name)
                                    || runtime.user_vars.len() < MAX_OSC_USER_VARS =>
                            {
                                runtime.user_vars.insert(name.clone(), value.clone());
                            }
                            TerminalEvent::UserVarChanged { .. } => {}
                            TerminalEvent::MarkSet => {}
                            TerminalEvent::PromptStarted | TerminalEvent::CommandLineStarted => {}
                            TerminalEvent::CommandStarted => runtime.command_running = true,
                            TerminalEvent::CommandFinished(status) => {
                                runtime.command_running = false;
                                runtime.last_exit_status = *status;
                            }
                            TerminalEvent::MouseCursorChanged(cursor) => {
                                runtime.mouse_cursor = *cursor;
                                mouse_cursor_changed = true;
                            }
                            TerminalEvent::MouseCursorControl(_) => {}
                            TerminalEvent::ColorControl(_)
                            | TerminalEvent::ItermUiColorChanged { .. }
                            | TerminalEvent::ItermUiColorReset(_)
                            | TerminalEvent::ItermUiColorQuery { .. }
                            | TerminalEvent::ColorStackPush
                            | TerminalEvent::ColorStackPop => {}
                            TerminalEvent::TabColorChanged { component, value } => {
                                runtime.tab_color.set(*component, *value);
                            }
                            TerminalEvent::TabColorSet(color) => {
                                runtime.tab_color =
                                    TabColorState([Some(color[0]), Some(color[1]), Some(color[2])]);
                            }
                            TerminalEvent::TabColorReset => {
                                runtime.tab_color = TabColorState::default();
                            }
                            TerminalEvent::SessionStatusChanged(update) => {
                                runtime.session_status.apply(update);
                            }
                            TerminalEvent::ProgressChanged(progress) => {
                                runtime.progress =
                                    (*progress != TerminalProgress::Hidden).then_some(*progress);
                            }
                            TerminalEvent::ClipboardStore(text) => {
                                osc52_copies.push(text.clone());
                            }
                            TerminalEvent::ClipboardCaptureStart
                            | TerminalEvent::ClipboardCaptureEnd => {}
                            TerminalEvent::Notification {
                                id,
                                title,
                                body,
                                occasion,
                                urgency,
                                timeout_ms,
                                sound,
                                icon_name,
                            } if allow_notifications
                                && notification_occasion_matches(
                                    *occasion,
                                    source_focused,
                                    source_visible,
                                )
                                && runtime.last_notification_at.is_none_or(|last| {
                                    last.elapsed() >= OSC_NOTIFICATION_INTERVAL
                                }) =>
                            {
                                runtime.last_notification_at = Some(Instant::now());
                                notification = Some(DesktopNotification {
                                    id: id.as_deref().map(|id| notification_platform_id(pane, id)),
                                    title: title.clone(),
                                    body: body.clone(),
                                    urgency: *urgency,
                                    timeout_ms: *timeout_ms,
                                    sound: *sound,
                                    icon_name: icon_name.clone(),
                                });
                            }
                            TerminalEvent::Notification { .. } => {}
                            TerminalEvent::NotificationClose(id) if allow_notifications => {
                                if let Some(sender) = self.notification_sender.as_ref()
                                    && let Err(error) =
                                        sender.close(notification_platform_id(pane, id))
                                {
                                    tracing::warn!(target: "toyoterm::notification", %error, "queue OSC notification close failed");
                                }
                            }
                            TerminalEvent::NotificationClose(_) => {}
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
                if mouse_cursor_changed && let Some(window) = self.window.as_ref() {
                    self.update_mouse_cursor(window);
                }
                if let Some(request) = attention_request
                    && let Some(window) = self.window.as_ref()
                {
                    window.request_user_attention(match request {
                        TerminalAttention::Indefinite => Some(UserAttentionType::Critical),
                        TerminalAttention::Once => Some(UserAttentionType::Informational),
                        TerminalAttention::Cancel => None,
                    });
                }
                for url in open_urls {
                    if let Err(error) = open_allowed_url(&url) {
                        tracing::warn!(target: "toyoterm::app", %error, %url, "open OSC URL failed");
                    }
                }
                for text in osc52_copies {
                    if let Err(error) = self.clipboard().and_then(|clipboard| {
                        clipboard.set_text(text).map_err(|error| error.to_string())
                    }) {
                        tracing::warn!(target: "toyoterm::clipboard", %error, "OSC 52 clipboard copy failed");
                    }
                }
                if let Some(notification) = notification
                    && let Some(sender) = self.notification_sender.as_ref()
                    && let Err(error) = sender.send(notification)
                {
                    tracing::warn!(target: "toyoterm::notification", %error, "queue OSC notification failed");
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
            AppEvent::AsyncCompleted { id, output } => {
                if self.cancelled_async_tasks.remove(&id) {
                    return;
                }
                let invocation = ScriptInvocation::AsyncCallback { id, output };
                if let Err(error) = self.submit_script(invocation) {
                    tracing::warn!(target: "toyoterm::script", %error, "submit async callback failed");
                }
            }
        }
    }
}

fn notification_occasion_matches(
    occasion: NotificationOccasion,
    source_focused: bool,
    source_visible: bool,
) -> bool {
    match occasion {
        NotificationOccasion::Always => true,
        NotificationOccasion::Unfocused => !source_focused,
        NotificationOccasion::Invisible => !source_visible,
    }
}

fn notification_platform_id(pane: PaneId, id: &str) -> u32 {
    let mut hash = 2_166_136_261_u32;
    for byte in pane.0.to_le_bytes().into_iter().chain(id.bytes()) {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    hash.max(1)
}

fn should_open_osc_url(
    enabled: bool,
    url: &str,
    last_opened_at: Option<Instant>,
    now: Instant,
) -> bool {
    enabled
        && validate_allowed_url(url).is_ok()
        && last_opened_at
            .is_none_or(|last| now.saturating_duration_since(last) >= OSC_OPEN_URL_INTERVAL)
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
        options: GuiOptions,
    ) -> Result<Self, String> {
        let GuiOptions {
            initial_pane,
            title: window_title_override,
            ..
        } = options;
        #[cfg(target_os = "linux")]
        let app_id = options.app_id;
        let mux = Mux::new();
        let mut pending_pane_launches = HashMap::new();
        if let Some(launch) = initial_pane {
            let pane = mux
                .current_pane()
                .ok_or_else(|| "new mux has no current pane".to_owned())?;
            pending_pane_launches.insert(pane, launch);
        }
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
            pending_pane_launches,
            pane_layout: PaneLayout::default(),
            tab_layout: TabStripLayout::default(),
            workspace_layout: WorkspaceStripLayout::default(),
            search_open: false,
            search_query: String::new(),
            search_result: SearchResult::default(),
            selector: None,
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
            notification_sender: NotificationSender::start()
                .map_err(|error| {
                    tracing::warn!(target: "toyoterm::notification", %error);
                    error
                })
                .ok(),
            pane_badges: HashMap::new(),
            runtime_events: VecDeque::new(),
            next_script_request: 1,
            script_in_flight: false,
            pending_script: VecDeque::new(),
            script_event_drops: 0,
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
            bar_items: HashMap::new(),
            bar_pending: None,
            next_bar_at: HashMap::new(),
            cancelled_async_tasks: HashSet::new(),
            terminal_render_pending: false,
            mux,
            render_style,
            fatal_error: None,
            exit_after_startup,
            window_title_override,
            #[cfg(target_os = "linux")]
            app_id,
        })
    }

    fn base_window_title(&self) -> &str {
        self.window_title_override
            .as_deref()
            .unwrap_or(&self.script_snapshot.config.window.title)
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
    fn tab_color_requires_all_components_and_resets_atomically() {
        let mut color = TabColorState::default();
        color.set(TabColorComponent::Red, 12);
        color.set(TabColorComponent::Green, 34);
        assert_eq!(color.complete(), None);
        color.set(TabColorComponent::Blue, 56);
        assert_eq!(color.complete(), Some([12, 34, 56]));
        color = TabColorState::default();
        assert_eq!(color.complete(), None);
    }

    #[test]
    fn session_status_updates_only_present_fields_and_supports_clear() {
        let mut state = SessionStatusState::default();
        state.apply(&SessionStatusUpdate {
            indicator: Some(Some([1, 2, 3])),
            status: Some(Some("building".into())),
            status_color: Some(Some([4, 5, 6])),
        });
        state.apply(&SessionStatusUpdate {
            status: Some(Some("ready".into())),
            ..SessionStatusUpdate::default()
        });
        assert_eq!(state.indicator, Some([1, 2, 3]));
        assert_eq!(state.status.as_deref(), Some("ready"));
        assert_eq!(state.status_color, Some([4, 5, 6]));

        state.apply(&SessionStatusUpdate {
            indicator: Some(None),
            status: Some(None),
            status_color: Some(None),
        });
        assert_eq!(state.indicator, None);
        assert_eq!(state.status, None);
        assert_eq!(state.status_color, None);
    }

    #[test]
    fn filters_notification_occasions_against_source_visibility() {
        assert!(notification_occasion_matches(
            NotificationOccasion::Always,
            true,
            true
        ));
        assert!(!notification_occasion_matches(
            NotificationOccasion::Unfocused,
            true,
            true
        ));
        assert!(notification_occasion_matches(
            NotificationOccasion::Unfocused,
            false,
            true
        ));
        assert!(!notification_occasion_matches(
            NotificationOccasion::Invisible,
            false,
            true
        ));
        assert!(notification_occasion_matches(
            NotificationOccasion::Invisible,
            false,
            false
        ));
    }

    #[test]
    fn osc_url_opening_requires_opt_in_allowlisted_scheme_and_rate_limit() {
        let now = Instant::now();
        assert!(should_open_osc_url(true, "https://example.com", None, now));
        assert!(!should_open_osc_url(
            false,
            "https://example.com",
            None,
            now
        ));
        assert!(!should_open_osc_url(true, "file:///tmp/a", None, now));
        assert!(!should_open_osc_url(
            true,
            "https://example.com",
            Some(now - Duration::from_secs(1)),
            now
        ));
        assert!(should_open_osc_url(
            true,
            "mailto:user@example.com",
            Some(now - OSC_OPEN_URL_INTERVAL),
            now
        ));
    }

    #[test]
    fn notification_replacement_ids_are_stable_and_pane_scoped() {
        let first = notification_platform_id(PaneId(1), "build-42");
        assert_eq!(first, notification_platform_id(PaneId(1), "build-42"));
        assert_ne!(first, notification_platform_id(PaneId(2), "build-42"));
        assert_ne!(first, notification_platform_id(PaneId(1), "build-43"));
        assert_ne!(first, 0);
    }

    #[test]
    fn maps_shell_command_lifecycle_to_ruby_events() {
        let pane = PaneId(7);
        let prompt = ruby_event_from_terminal_event(pane, TerminalEvent::PromptStarted).unwrap();
        assert_eq!(prompt.name(), "prompt_started");
        assert_eq!(prompt.pane, Some(pane));

        let command_line =
            ruby_event_from_terminal_event(pane, TerminalEvent::CommandLineStarted).unwrap();
        assert_eq!(command_line.name(), "command_line_started");
        assert_eq!(command_line.pane, Some(pane));

        let started = ruby_event_from_terminal_event(pane, TerminalEvent::CommandStarted).unwrap();
        assert_eq!(started.name(), "command_started");
        assert_eq!(started.pane, Some(pane));
        assert_eq!(started.exit_status, None);

        let finished =
            ruby_event_from_terminal_event(pane, TerminalEvent::CommandFinished(Some(23))).unwrap();
        assert_eq!(finished.name(), "command_finished");
        assert_eq!(finished.pane, Some(pane));
        assert_eq!(finished.exit_status, Some(23));
    }

    #[test]
    fn reports_bounded_iterm_session_variables() {
        let mut user_vars = BTreeMap::new();
        user_vars.insert("gitBranch".into(), "main".into());
        let mut runtime = PaneRuntime {
            terminal: AlacrittyTerminalBackend::new(80, 24),
            pty_session: None,
            process_id: None,
            title: "build server".into(),
            icon_title: Some("build".into()),
            osc_badge: None,
            cursor_line_highlight: false,
            cwd: Some(PathBuf::from("/srv/project")),
            remote_host: Some("alice@example.com".into()),
            shell_integration_version: Some(1),
            shell_integration_shell: Some("bash".into()),
            user_vars,
            command_running: false,
            last_exit_status: None,
            progress: None,
            tab_color: TabColorState::default(),
            session_status: SessionStatusState::default(),
            mouse_cursor: CursorIcon::Default,
            last_notification_at: None,
            last_open_url_at: None,
            exited: false,
        };

        assert_eq!(
            iterm_variable_response(&runtime, "session.name"),
            "\x1b]1337;ReportVariable=YnVpbGQgc2VydmVy\x1b\\"
        );
        assert_eq!(
            iterm_variable_value(&runtime, "session.terminalIconName").as_deref(),
            Some("build")
        );
        assert_eq!(
            iterm_variable_value(&runtime, "session.path").as_deref(),
            Some("/srv/project")
        );
        assert_eq!(
            iterm_variable_value(&runtime, "session.columns").as_deref(),
            Some("80")
        );
        assert_eq!(
            iterm_variable_value(&runtime, "session.rows").as_deref(),
            Some("24")
        );
        assert_eq!(
            iterm_variable_value(&runtime, "session.shell").as_deref(),
            Some("bash")
        );
        assert_eq!(
            iterm_variable_value(&runtime, "session.hostname").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            iterm_variable_value(&runtime, "session.username").as_deref(),
            Some("alice")
        );
        assert_eq!(
            iterm_variable_value(&runtime, "session.user.gitBranch").as_deref(),
            Some("main")
        );
        assert_eq!(
            iterm_variable_response(&runtime, "session.unknown"),
            "\x1b]1337;ReportVariable=\x1b\\"
        );
        assert_eq!(
            interpolate_iterm_badge(
                &runtime,
                r"\(session.name) · \(user.gitBranch) · \(session.hostname)"
            )
            .as_deref(),
            Some("build server · main · example.com")
        );
        assert_eq!(
            interpolate_iterm_badge(&runtime, r"unknown=\(session.missing)").as_deref(),
            Some("unknown=")
        );
        apply_iterm_badge_format(&mut runtime, r"\(session.name):\(user.gitBranch)");
        assert_eq!(runtime.osc_badge.as_deref(), Some("build server:main"));
        apply_iterm_badge_format(&mut runtime, "");
        assert_eq!(runtime.osc_badge, None);
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

    #[cfg(windows)]
    #[test]
    fn pane_launch_removes_stale_outer_terminal_identity() {
        let launch = PaneLaunchSpec {
            program: Some("cmd.exe".into()),
            args: vec![
                "/D".into(),
                "/S".into(),
                "/C".into(),
                concat!(
                    "if defined WEZTERM_EXECUTABLE (exit /b 7) else ",
                    "if defined TMUX (exit /b 8) else ",
                    "if \"%TERM_PROGRAM%\"==\"toyoterm\" (exit /b 0) else exit /b 9"
                )
                .into(),
            ],
            cwd: None,
            environment: vec![
                ("WEZTERM_EXECUTABLE".into(), Some("stale".into())),
                ("TMUX".into(), Some("stale".into())),
            ],
        };
        let command = pane_lifecycle::pty_command_for_launch(None, Some(&launch));
        let mut session = NativePty
            .spawn(command, PtySize::new(80, 24))
            .expect("spawn custom pane command");
        let mut reader = session.take_reader().expect("take PTY reader");
        let mut output = Vec::new();
        reader.read_to_end(&mut output).expect("read PTY output");
        let status = session.wait().expect("wait for custom pane command");

        assert_eq!(status.code, 0, "unexpected output: {output:?}");
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
                icon_title: None,
                osc_badge: None,
                cursor_line_highlight: false,
                cwd: None,
                remote_host: None,
                shell_integration_version: None,
                shell_integration_shell: None,
                user_vars: BTreeMap::new(),
                command_running: false,
                last_exit_status: None,
                progress: None,
                tab_color: TabColorState::default(),
                session_status: SessionStatusState::default(),
                mouse_cursor: CursorIcon::Default,
                last_notification_at: None,
                last_open_url_at: None,
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
            PtySize {
                columns: 100,
                rows: 30,
                pixel_width: 900,
                pixel_height: 540
            }
        );
    }

    #[test]
    fn grid_dimensions_never_reach_zero() {
        let metrics = CellMetrics::default();
        assert_eq!(
            metrics.terminal_size(PhysicalSize::new(0, 0)),
            PtySize {
                columns: 2,
                rows: 1,
                pixel_width: 18,
                pixel_height: 18
            }
        );
    }

    #[test]
    fn hidpi_scaling_preserves_logical_grid_size() {
        let metrics = CellMetrics::default();
        let logical = metrics.terminal_size_at_scale(PhysicalSize::new(916, 556), 1.0);
        let hidpi = metrics.terminal_size_at_scale(PhysicalSize::new(1832, 1112), 2.0);
        assert_eq!((logical.columns, logical.rows), (hidpi.columns, hidpi.rows));
        assert_eq!(
            (hidpi.pixel_width, hidpi.pixel_height),
            (logical.pixel_width * 2, logical.pixel_height * 2)
        );
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
    fn terminal_input_returns_a_scrolled_viewport_to_the_bottom() {
        let mut terminal = AlacrittyTerminalBackend::new(10, 2);
        terminal.advance(b"one\r\ntwo\r\nthree");
        terminal.scroll_display(1);
        assert_eq!(terminal.snapshot().lines, ["one", "two"]);

        pane_lifecycle::reset_scroll_for_input(&mut terminal, b"x");

        assert_eq!(terminal.snapshot().lines, ["two", "three"]);
    }

    #[test]
    fn empty_terminal_input_preserves_a_scrolled_viewport() {
        let mut terminal = AlacrittyTerminalBackend::new(10, 2);
        terminal.advance(b"one\r\ntwo\r\nthree");
        terminal.scroll_display(1);

        pane_lifecycle::reset_scroll_for_input(&mut terminal, b"");

        assert_eq!(terminal.snapshot().lines, ["one", "two"]);
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
    fn window_bars_reserve_the_top_and_bottom_edges() {
        let config = ToyotermConfig {
            status_bars: [StatusBarPosition::Top, StatusBarPosition::Bottom]
                .map(|position| toyoterm_config::StatusBarConfig {
                    position,
                    interval: Duration::from_secs(1),
                })
                .into(),
            ..ToyotermConfig::default()
        };

        let (pane, bars) = edge_bar_layout(PhysicalSize::new(960, 600), 54, &config, 1.0);
        assert_eq!(pane, PaneRect::new(0, 78, 960, 498));
        assert_eq!(
            bars,
            vec![
                (StatusBarPosition::Top, PaneRect::new(0, 54, 960, 24)),
                (StatusBarPosition::Bottom, PaneRect::new(0, 576, 960, 24)),
            ]
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
