use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
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

use toyoterm_api::{
    ActionCommand, ClipboardCommand, ConfigCommand, NativeHandle, PaneCommand, ScriptCommand,
    UiCommand, WindowCommand,
};
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
use notifications::{
    DesktopNotification, NotificationFeedback, NotificationFeedbackKind, NotificationSender,
};
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
    MouseEventKind, MouseWheelDirection, NotificationOccasion, NotificationSound,
    NotificationUrgency, SearchDirection, SearchResult, SelectionKind, SessionStatusUpdate,
    TabColorComponent, TerminalAttention, TerminalBackend, TerminalEvent, TerminalKey,
    TerminalMode, TerminalMouseButton, TerminalProgress, encode_key, encode_mouse_event,
    encode_mouse_wheel, encode_paste,
};

const MULTI_CLICK_INTERVAL: Duration = Duration::from_millis(500);
const OSC_NOTIFICATION_INTERVAL: Duration = Duration::from_secs(2);
const OSC_OPEN_URL_INTERVAL: Duration = Duration::from_secs(2);
const OSC_VISUAL_BELL_DURATION: Duration = Duration::from_millis(150);
const OSC_CURSOR_FIREWORKS_DURATION: Duration = Duration::from_millis(350);
const MAX_OSC_USER_VARS: usize = 64;
const MAX_OSC_REPORT_VARIABLE_VALUE_BYTES: usize = 4 * 1024;
const MAX_PENDING_PTY_OUTPUT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Default)]
struct PtyOutputState {
    bytes: Vec<u8>,
    spare: Vec<u8>,
}

#[derive(Debug, Default)]
struct PtyOutputBuffer {
    state: Mutex<PtyOutputState>,
    drained: Condvar,
}

impl PtyOutputBuffer {
    fn append(&self, mut input: &[u8], mut notify_ready: impl FnMut() -> bool) -> bool {
        while !input.is_empty() {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            while state.bytes.len() == MAX_PENDING_PTY_OUTPUT_BYTES {
                state = self
                    .drained
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            let notify = state.bytes.is_empty();
            let count = input
                .len()
                .min(MAX_PENDING_PTY_OUTPUT_BYTES - state.bytes.len());
            state.bytes.extend_from_slice(&input[..count]);
            input = &input[count..];
            drop(state);
            if notify && !notify_ready() {
                return false;
            }
        }
        true
    }

    fn take(&self) -> Vec<u8> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let PtyOutputState { bytes, spare } = &mut *state;
        std::mem::swap(bytes, spare);
        let bytes = std::mem::take(spare);
        self.drained.notify_one();
        bytes
    }

    fn recycle(&self, mut bytes: Vec<u8>) {
        bytes.clear();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if bytes.capacity() > state.spare.capacity() {
            state.spare = bytes;
        }
    }
}

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
        pending: Arc<PtyOutputBuffer>,
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
    NotificationFeedback(NotificationFeedback),
}

enum EvalWaiter {
    Ipc(mpsc::Sender<Result<String, String>>),
}

struct PaneRuntime {
    terminal: AlacrittyTerminalBackend,
    process: ProcessRuntime,
    metadata: PaneMetadata,
    protocol: PaneProtocolState,
}

struct ProcessRuntime {
    pty_session: Option<Box<dyn PtySession>>,
    process_id: Option<u32>,
    exited: bool,
}

struct PaneMetadata {
    title: String,
    icon_title: Option<String>,
    osc_badge: Option<String>,
    cwd: Option<PathBuf>,
    remote_host: Option<String>,
}

struct PaneProtocolState {
    cursor_line_highlight: bool,
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
    active_notifications: BTreeMap<String, Option<Instant>>,
    last_open_url_at: Option<Instant>,
    visual_bell_deadline: Option<Instant>,
    cursor_fireworks_deadline: Option<Instant>,
}

impl Default for PaneProtocolState {
    fn default() -> Self {
        Self {
            cursor_line_highlight: false,
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
            active_notifications: BTreeMap::new(),
            last_open_url_at: None,
            visual_bell_deadline: None,
            cursor_fireworks_deadline: None,
        }
    }
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

fn apply_terminal_progress(
    current: Option<TerminalProgress>,
    update: TerminalProgress,
) -> Option<TerminalProgress> {
    match update {
        TerminalProgress::Hidden => None,
        TerminalProgress::Warning(None) => {
            let value = current.and_then(|progress| match progress {
                TerminalProgress::Normal(value)
                | TerminalProgress::Error(Some(value))
                | TerminalProgress::Warning(Some(value)) => Some(value),
                TerminalProgress::Hidden
                | TerminalProgress::Error(None)
                | TerminalProgress::Indeterminate
                | TerminalProgress::Warning(None) => None,
            });
            Some(TerminalProgress::Warning(value))
        }
        progress => Some(progress),
    }
}

fn visual_bell_deadline(color: Option<[u8; 3]>, now: Instant) -> Option<Instant> {
    color.map(|_| now + OSC_VISUAL_BELL_DURATION)
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
        "session.name" => Some(runtime.metadata.title.clone()),
        "session.terminalIconName" => runtime.metadata.icon_title.clone(),
        "session.columns" => Some(columns.to_string()),
        "session.rows" => Some(rows.to_string()),
        "session.path" => runtime
            .metadata
            .cwd
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        "session.shell" => runtime.protocol.shell_integration_shell.clone(),
        "session.hostname" => runtime
            .metadata
            .remote_host
            .as_deref()
            .and_then(|remote_host| remote_host.split_once('@'))
            .map(|(_, hostname)| hostname.to_owned()),
        "session.username" => runtime
            .metadata
            .remote_host
            .as_deref()
            .and_then(|remote_host| remote_host.split_once('@'))
            .map(|(username, _)| username.to_owned()),
        name => name
            .strip_prefix("session.user.")
            .and_then(|name| runtime.protocol.user_vars.get(name))
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
    runtime.metadata.osc_badge = if format.is_empty() {
        None
    } else {
        interpolate_iterm_badge(runtime, format)
    };
}

impl PaneRuntime {
    fn terminate(&mut self) {
        self.process.terminate();
    }
}

impl ProcessRuntime {
    fn terminate(&mut self) {
        if let Some(mut session) = self.pty_session.take() {
            let _ = session.kill();
        }
        self.exited = true;
    }
}

impl Drop for ProcessRuntime {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct WindowOcclusion {
    occluded: bool,
}

impl WindowOcclusion {
    fn update(&mut self, occluded: bool) -> bool {
        let became_visible = self.occluded && !occluded;
        self.occluded = occluded;
        became_visible
    }

    fn should_render(self) -> bool {
        !self.occluded
    }
}

struct ToyotermApplication {
    event_proxy: EventLoopProxy<AppEvent>,
    platform: PlatformState,
    terminal_runtime: TerminalRuntime,
    scripting: ScriptRuntimeState,
    ui: UiState,
    _ipc_server: Option<IpcServer>,
    mux: Mux,
    fatal_error: Option<String>,
    exit_after_startup: bool,
}

struct PlatformState {
    window: Option<Arc<Window>>,
    renderer: Option<GpuRenderer>,
    occlusion: WindowOcclusion,
    clipboard: Option<Clipboard>,
    notification_sender: Option<NotificationSender>,
    #[cfg(target_os = "linux")]
    app_id: Option<String>,
}

struct TerminalRuntime {
    pane_runtimes: HashMap<PaneId, PaneRuntime>,
    pending_pane_launches: HashMap<PaneId, PaneLaunchSpec>,
}

struct ScriptRuntimeState {
    thread: ScriptThread,
    snapshot: Arc<ScriptSnapshot>,
    next_request_id: u64,
    in_flight: bool,
    pending: VecDeque<(u64, ScriptInvocation)>,
    runtime_events: VecDeque<RubyEvent>,
    event_drops: u64,
    eval_waiters: HashMap<u64, EvalWaiter>,
    cancelled_async_tasks: HashSet<u64>,
    cached_model: Option<Arc<RubyObjectModel>>,
    cached_handles: Option<Arc<[NativeHandle]>>,
    context_dirty: bool,
}

struct UiState {
    pane_layout: PaneLayout,
    tab_layout: TabStripLayout,
    workspace_layout: WorkspaceStripLayout,
    search_open: bool,
    search_query: String,
    search_result: SearchResult,
    selector: Option<SelectorOverlay>,
    config_error_layout: ConfigErrorLayout,
    config_error_notice: Option<ConfigErrorNotice>,
    ime_preedit: Option<String>,
    modifiers: ModifiersState,
    alt_graph_active: bool,
    leader_deadline: Option<Instant>,
    mouse_position: PhysicalPosition<f64>,
    pressed_mouse_button: Option<TerminalMouseButton>,
    last_mouse_cell: Option<(u16, u16)>,
    wheel_line_accumulator: f64,
    selecting: bool,
    visual_selection: Option<VisualSelection>,
    click_tracker: ClickTracker,
    pending_clipboard_writes: Vec<String>,
    pane_badges: HashMap<PaneId, String>,
    cell_metrics: CellMetrics,
    bar_items: HashMap<StatusBarPosition, Vec<BarItem>>,
    bar_pending: Option<StatusBarPosition>,
    next_bar_at: HashMap<StatusBarPosition, Instant>,
    terminal_render_pending: bool,
    render_style: RenderStyle,
    window_title_override: Option<String>,
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
        TerminalEvent::CapturedOutputCleared => return None,
        TerminalEvent::MouseCursorChanged(_) => return None,
        TerminalEvent::MouseCursorControl(_) => return None,
        TerminalEvent::ColorControl(_)
        | TerminalEvent::ItermUiColorChanged { .. }
        | TerminalEvent::ItermUiColorReset(_)
        | TerminalEvent::ItermUiColorQuery { .. }
        | TerminalEvent::ItermDefaultColorQuery(_)
        | TerminalEvent::XtermSpecialColorSet { .. }
        | TerminalEvent::XtermSpecialColorQuery(_)
        | TerminalEvent::XtermSpecialColorReset(_)
        | TerminalEvent::XtermSpecialColorMode { .. }
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
        TerminalEvent::Notification { .. }
        | TerminalEvent::NotificationClose(_)
        | TerminalEvent::NotificationAliveQuery(_) => return None,
        TerminalEvent::PtyWrite(_) => return None,
        TerminalEvent::Bell { .. } => RubyEvent::new(ScriptEventKind::Bell),
    };
    event.pane = Some(pane);
    Some(event)
}

impl ApplicationHandler<AppEvent> for ToyotermApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.platform.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title(self.base_window_title())
            // Windows transparency must be enabled at creation, including when
            // starting opaque. Later opacity changes only update the GPU alpha.
            .with_transparent(
                cfg!(target_os = "windows") || self.scripting.snapshot.config.window.opacity < 1.0,
            )
            .with_inner_size(LogicalSize::new(
                self.scripting.snapshot.config.window.width,
                self.scripting.snapshot.config.window.height,
            ))
            .with_min_inner_size(LogicalSize::new(
                self.scripting.snapshot.config.window.min_width,
                self.scripting.snapshot.config.window.min_height,
            ))
            .with_decorations(self.scripting.snapshot.config.window.decorations)
            .with_resizable(self.scripting.snapshot.config.window.resizable)
            .with_window_level(if self.scripting.snapshot.config.window.always_on_top {
                winit::window::WindowLevel::AlwaysOnTop
            } else {
                winit::window::WindowLevel::Normal
            });
        #[cfg(target_os = "linux")]
        let attributes = match self.platform.app_id.as_deref() {
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
        let mut renderer = match pollster::block_on(GpuRenderer::new(
            window.clone(),
            self.ui.render_style.clone(),
        )) {
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
        self.ui.cell_metrics.width =
            f64::from(renderer.terminal_cell_width(self.ui.cell_metrics.font_size));
        let size = self
            .ui
            .cell_metrics
            .terminal_size_at_scale(window.inner_size(), window.scale_factor());
        window.set_ime_allowed(true);
        self.platform.renderer = Some(renderer);
        self.platform.window = Some(window);
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
            self.platform
                .window
                .as_ref()
                .expect("window was installed")
                .scale_factor(),
        );
        self.platform
            .window
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
        let Some(window) = self.platform.window.clone() else {
            return;
        };
        if window.id() != window_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Occluded(occluded) => {
                if self.platform.occlusion.update(occluded) {
                    window.request_redraw();
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.platform.renderer.as_mut() {
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
                if let Some(renderer) = self.platform.renderer.as_mut() {
                    renderer.resize(size);
                }
                if let Err(error) = self.resize_panes(size, scale_factor) {
                    self.fail(event_loop, error);
                    return;
                }
                self.sync_active_renderer(scale_factor);
                window.request_redraw();
            }
            WindowEvent::ModifiersChanged(modifiers) => self.ui.modifiers = modifiers.state(),
            WindowEvent::Focused(focused) => {
                // A platform is not required to send key-release events after
                // the window loses focus. Do not leave modifiers (especially
                // AltGraph) stuck when focus returns.
                if !focused {
                    clear_modifier_state(&mut self.ui.modifiers, &mut self.ui.alt_graph_active);
                    self.ui.leader_deadline = None;
                    self.exit_visual_mode();
                    self.ui.pressed_mouse_button = None;
                    self.ui.last_mouse_cell = None;
                    if self.ui.search_open {
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
                self.ui.mouse_position = position;
                self.update_mouse_cursor(&window);
                if self.ui.selecting {
                    let (column, row) = self.mouse_cell(window.scale_factor());
                    if let Some(terminal) = self.active_terminal_mut() {
                        terminal.update_selection(column, row);
                    }
                    self.sync_active_renderer(window.scale_factor());
                    window.request_redraw();
                } else if self.ui.selector.is_none() {
                    self.handle_mouse_motion(event_loop, &window);
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.ui.last_mouse_cell = None;
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if self.ui.selector.is_none() {
                    self.handle_mouse_input(event_loop, &window, button, state);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.ui.selector.is_none() {
                    self.handle_mouse_wheel(event_loop, &window, delta);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if matches!(event.logical_key, Key::Named(NamedKey::AltGraph)) {
                    self.ui.alt_graph_active = event.state == ElementState::Pressed;
                    return;
                }
                if !should_handle_key_event(event.state, event.repeat) {
                    return;
                }
                let modifiers = effective_modifiers(self.ui.modifiers, self.ui.alt_graph_active);
                if self.ui.config_error_notice.is_some()
                    && matches!(event.logical_key, Key::Named(NamedKey::Escape))
                {
                    self.ui.config_error_notice = None;
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
                if self.ui.selector.is_some() {
                    if let Err(error) = self.handle_selector_key(&event, modifiers) {
                        tracing::warn!(target: "toyoterm::script", %error, "selector callback submission failed");
                    }
                    self.sync_active_renderer(window.scale_factor());
                    window.request_redraw();
                    return;
                }
                if self.ui.search_open {
                    self.handle_search_key(&event, modifiers);
                    self.sync_active_renderer(window.scale_factor());
                    window.request_redraw();
                    return;
                }
                if self.ui.visual_selection.is_some() {
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
                self.ui.leader_deadline = None;
                self.ui.ime_preedit = (!text.is_empty()).then_some(text);
                self.sync_active_renderer(window.scale_factor());
                window.request_redraw();
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                self.ui.leader_deadline = None;
                self.ui.ime_preedit = None;
                if let Some(selector) = self.ui.selector.as_mut() {
                    selector.append_query(&text);
                    self.sync_active_renderer(window.scale_factor());
                    window.request_redraw();
                    return;
                }
                if self.ui.visual_selection.is_some() {
                    return;
                }
                if self.ui.search_open {
                    self.ui.search_query.push_str(&text);
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
                self.ui.leader_deadline = None;
                self.ui.ime_preedit = None;
                self.sync_active_renderer(window.scale_factor());
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if !self.platform.occlusion.should_render() {
                    return;
                }
                if self.ui.terminal_render_pending {
                    self.ui.terminal_render_pending = false;
                    self.sync_active_renderer(window.scale_factor());
                }
                let render_result = self.platform.renderer.as_mut().map(GpuRenderer::render);
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
        let now = Instant::now();
        let expired_sync_panes = self
            .terminal_runtime
            .pane_runtimes
            .iter()
            .filter_map(|(pane, runtime)| {
                runtime
                    .terminal
                    .synchronized_update_deadline()
                    .filter(|deadline| *deadline <= now)
                    .map(|_| *pane)
            })
            .collect::<Vec<_>>();
        for pane in expired_sync_panes {
            if let Some(runtime) = self.terminal_runtime.pane_runtimes.get_mut(&pane) {
                runtime.terminal.stop_synchronized_update();
                // Re-enter the regular output path so terminal events emitted
                // while releasing the synchronized bytes retain their normal
                // ordering and side effects (including PTY replies).
                let _ = self.event_proxy.send_event(AppEvent::Output {
                    pane,
                    pending: Arc::new(PtyOutputBuffer::default()),
                });
            }
        }
        let next_sync_at = self
            .terminal_runtime
            .pane_runtimes
            .values()
            .filter_map(|runtime| runtime.terminal.synchronized_update_deadline())
            .min();
        let mut transient_effect_expired = false;
        for runtime in self.terminal_runtime.pane_runtimes.values_mut() {
            if runtime
                .protocol
                .visual_bell_deadline
                .is_some_and(|deadline| deadline <= now)
            {
                runtime.protocol.visual_bell_deadline = None;
                transient_effect_expired = true;
            }
            if runtime
                .protocol
                .cursor_fireworks_deadline
                .is_some_and(|deadline| deadline <= now)
            {
                runtime.protocol.cursor_fireworks_deadline = None;
                transient_effect_expired = true;
            }
        }
        if transient_effect_expired && let Some(window) = self.platform.window.as_ref() {
            window.request_redraw();
        }
        let next_effect_at = self
            .terminal_runtime
            .pane_runtimes
            .values()
            .flat_map(|runtime| {
                [
                    runtime.protocol.visual_bell_deadline,
                    runtime.protocol.cursor_fireworks_deadline,
                ]
            })
            .flatten()
            .min();
        let next_terminal_at = match (next_sync_at, next_effect_at) {
            (Some(sync), Some(effect)) => Some(sync.min(effect)),
            (Some(deadline), None) | (None, Some(deadline)) => Some(deadline),
            (None, None) => None,
        };

        if self.scripting.snapshot.config.status_bars.is_empty() {
            self.ui.next_bar_at.clear();
            set_wait_control_flow(event_loop, next_terminal_at);
            return;
        }
        if self.ui.bar_pending.is_some() || self.platform.window.is_none() {
            set_wait_control_flow(event_loop, next_terminal_at);
            return;
        }
        for bar in &self.scripting.snapshot.config.status_bars {
            if !self.ui.bar_items.contains_key(&bar.position) {
                self.ui.next_bar_at.entry(bar.position).or_insert(now);
            }
        }
        let Some((position, deadline)) = self
            .scripting
            .snapshot
            .config
            .status_bars
            .iter()
            .filter_map(|bar| {
                self.ui
                    .next_bar_at
                    .get(&bar.position)
                    .map(|deadline| (bar.position, *deadline))
            })
            .min_by_key(|(_, deadline)| *deadline)
        else {
            set_wait_control_flow(event_loop, next_terminal_at);
            return;
        };
        if now < deadline {
            set_wait_control_flow(
                event_loop,
                Some(next_terminal_at.map_or(deadline, |terminal| terminal.min(deadline))),
            );
            return;
        }
        match self.submit_script(ScriptInvocation::Bar { position }) {
            Ok(_) => {
                self.ui.bar_pending = Some(position);
                self.ui.next_bar_at.remove(&position);
                set_wait_control_flow(event_loop, next_terminal_at);
            }
            Err(error) => {
                tracing::warn!(target: "toyoterm::script", %error, "submit bar callback failed");
                let deadline = now + Duration::from_secs(1);
                self.ui.next_bar_at.insert(position, deadline);
                set_wait_control_flow(
                    event_loop,
                    Some(next_terminal_at.map_or(deadline, |terminal| terminal.min(deadline))),
                );
            }
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.shutdown();
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Output { pane, pending } => {
                self.scripting.invalidate_context();
                let bytes = pending.take();
                let mut terminal_events = Vec::new();
                let mut mouse_cursor_changed = false;
                let mut osc52_copies = Vec::new();
                let mut notification = None;
                let mut attention_request = None;
                let mut open_urls = Vec::new();
                let allow_notifications = self
                    .scripting
                    .snapshot
                    .config
                    .behavior
                    .allow_osc_notifications;
                let allow_attention = self
                    .scripting
                    .snapshot
                    .config
                    .behavior
                    .allow_osc_attention_requests;
                let allow_open_url = self.scripting.snapshot.config.behavior.allow_osc_open_url;
                let window_focused = self
                    .platform
                    .window
                    .as_ref()
                    .is_some_and(|window| window.has_focus());
                let source_focused = window_focused && self.mux.current_pane() == Some(pane);
                let source_visible = window_focused && self.ui.pane_layout.rect(pane).is_some();
                if let Some(runtime) = self.terminal_runtime.pane_runtimes.get_mut(&pane) {
                    runtime.terminal.advance(&bytes);
                    terminal_events = runtime.terminal.drain_events();
                    for event in &terminal_events {
                        match event {
                            TerminalEvent::TitleChanged(title) => {
                                runtime.metadata.title = title.clone()
                            }
                            TerminalEvent::TitleReset => {
                                runtime.metadata.title = format!("Pane {}", pane.0)
                            }
                            TerminalEvent::IconTitleChanged(title) => {
                                runtime.metadata.icon_title = Some(title.clone());
                            }
                            TerminalEvent::CwdChanged(cwd) => {
                                runtime.metadata.cwd = Some(PathBuf::from(cwd))
                            }
                            TerminalEvent::RemoteHostChanged(remote_host) => {
                                runtime.metadata.remote_host = Some(remote_host.clone());
                            }
                            TerminalEvent::ShellIntegrationChanged { version, shell } => {
                                runtime.protocol.shell_integration_version = Some(*version);
                                runtime.protocol.shell_integration_shell = shell.clone();
                            }
                            TerminalEvent::ItermVariableQuery(name) => {
                                let response = iterm_variable_response(runtime, name);
                                if let Some(session) = runtime.process.pty_session.as_mut()
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
                                runtime.protocol.cursor_line_highlight = *enabled;
                            }
                            TerminalEvent::AttentionRequested(TerminalAttention::Cancel) => {
                                attention_request = Some(TerminalAttention::Cancel);
                            }
                            TerminalEvent::AttentionRequested(TerminalAttention::Fireworks)
                                if allow_attention =>
                            {
                                runtime.protocol.cursor_fireworks_deadline =
                                    Some(Instant::now() + OSC_CURSOR_FIREWORKS_DURATION);
                            }
                            TerminalEvent::AttentionRequested(request) if allow_attention => {
                                attention_request = Some(*request);
                            }
                            TerminalEvent::AttentionRequested(_) => {}
                            TerminalEvent::OpenUrlRequested(url)
                                if should_open_osc_url(
                                    allow_open_url,
                                    url,
                                    runtime.protocol.last_open_url_at,
                                    Instant::now(),
                                ) =>
                            {
                                runtime.protocol.last_open_url_at = Some(Instant::now());
                                open_urls.push(url.clone());
                            }
                            TerminalEvent::OpenUrlRequested(_) => {}
                            TerminalEvent::UserVarChanged { name, value }
                                if runtime.protocol.user_vars.contains_key(name)
                                    || runtime.protocol.user_vars.len() < MAX_OSC_USER_VARS =>
                            {
                                runtime
                                    .protocol
                                    .user_vars
                                    .insert(name.clone(), value.clone());
                            }
                            TerminalEvent::UserVarChanged { .. } => {}
                            TerminalEvent::MarkSet => {}
                            TerminalEvent::PromptStarted | TerminalEvent::CommandLineStarted => {}
                            TerminalEvent::CommandStarted => {
                                runtime.protocol.command_running = true
                            }
                            TerminalEvent::CommandFinished(status) => {
                                runtime.protocol.command_running = false;
                                runtime.protocol.last_exit_status = *status;
                            }
                            TerminalEvent::CapturedOutputCleared => {}
                            TerminalEvent::MouseCursorChanged(cursor) => {
                                runtime.protocol.mouse_cursor = *cursor;
                                mouse_cursor_changed = true;
                            }
                            TerminalEvent::MouseCursorControl(_) => {}
                            TerminalEvent::ColorControl(_)
                            | TerminalEvent::ItermDefaultColorQuery(_)
                            | TerminalEvent::XtermSpecialColorSet { .. }
                            | TerminalEvent::XtermSpecialColorQuery(_)
                            | TerminalEvent::XtermSpecialColorReset(_)
                            | TerminalEvent::XtermSpecialColorMode { .. }
                            | TerminalEvent::ItermUiColorChanged { .. }
                            | TerminalEvent::ItermUiColorReset(_)
                            | TerminalEvent::ItermUiColorQuery { .. }
                            | TerminalEvent::ColorStackPush
                            | TerminalEvent::ColorStackPop => {}
                            TerminalEvent::TabColorChanged { component, value } => {
                                runtime.protocol.tab_color.set(*component, *value);
                            }
                            TerminalEvent::TabColorSet(color) => {
                                runtime.protocol.tab_color =
                                    TabColorState([Some(color[0]), Some(color[1]), Some(color[2])]);
                            }
                            TerminalEvent::TabColorReset => {
                                runtime.protocol.tab_color = TabColorState::default();
                            }
                            TerminalEvent::SessionStatusChanged(update) => {
                                runtime.protocol.session_status.apply(update);
                            }
                            TerminalEvent::ProgressChanged(progress) => {
                                runtime.protocol.progress =
                                    apply_terminal_progress(runtime.protocol.progress, *progress);
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
                                icon,
                                buttons,
                                reporting,
                            } if allow_notifications
                                && notification_occasion_matches(
                                    *occasion,
                                    source_focused,
                                    source_visible,
                                )
                                && runtime.protocol.last_notification_at.is_none_or(|last| {
                                    last.elapsed() >= OSC_NOTIFICATION_INTERVAL
                                }) =>
                            {
                                runtime.protocol.last_notification_at = Some(Instant::now());
                                if let Some(id) = id
                                    && (runtime.protocol.active_notifications.contains_key(id)
                                        || runtime.protocol.active_notifications.len() < 32)
                                {
                                    runtime.protocol.active_notifications.insert(
                                        id.clone(),
                                        notification_expiry(*timeout_ms, Instant::now()),
                                    );
                                }
                                notification = Some(DesktopNotification {
                                    pane,
                                    protocol_id: id.clone().unwrap_or_else(|| "0".into()),
                                    id: id.as_deref().map(|id| notification_platform_id(pane, id)),
                                    title: title.clone(),
                                    body: body.clone(),
                                    urgency: *urgency,
                                    timeout_ms: *timeout_ms,
                                    sound: *sound,
                                    icon_name: icon_name.clone(),
                                    icon: icon.clone(),
                                    buttons: buttons.clone(),
                                    reporting: *reporting,
                                });
                            }
                            TerminalEvent::Notification { .. } => {}
                            TerminalEvent::NotificationClose(id) => {
                                let was_active =
                                    runtime.protocol.active_notifications.remove(id).is_some();
                                if was_active
                                    && let Some(sender) = self.platform.notification_sender.as_ref()
                                    && let Err(error) =
                                        sender.close(notification_platform_id(pane, id))
                                {
                                    tracing::warn!(target: "toyoterm::notification", %error, "queue OSC notification close failed");
                                }
                            }
                            TerminalEvent::NotificationAliveQuery(request_id) => {
                                let response = notification_alive_response(
                                    request_id,
                                    &mut runtime.protocol.active_notifications,
                                    Instant::now(),
                                );
                                if let Some(session) = runtime.process.pty_session.as_mut()
                                    && let Err(error) = session.write(response.as_bytes())
                                {
                                    tracing::error!(
                                        target: "toyoterm::pty",
                                        operation = error.operation(),
                                        %pane,
                                        bytes = response.len(),
                                        %error,
                                        "write OSC 99 alive response to pane PTY failed"
                                    );
                                }
                            }
                            TerminalEvent::PtyWrite(response) => {
                                if let Some(session) = runtime.process.pty_session.as_mut()
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
                            TerminalEvent::Bell { visual_bell } => {
                                runtime.protocol.visual_bell_deadline =
                                    visual_bell_deadline(*visual_bell, Instant::now());
                            }
                        }
                    }
                }
                // The terminal parser has consumed the batch. Retain its
                // allocation for a later PTY read instead of repeatedly
                // allocating and freeing buffers during sustained output.
                pending.recycle(bytes);
                if mouse_cursor_changed && let Some(window) = self.platform.window.as_ref() {
                    self.update_mouse_cursor(window);
                }
                if let Some(request) = attention_request
                    && let Some(window) = self.platform.window.as_ref()
                {
                    window.request_user_attention(match request {
                        TerminalAttention::Indefinite => Some(UserAttentionType::Critical),
                        TerminalAttention::Once => Some(UserAttentionType::Informational),
                        TerminalAttention::Cancel => None,
                        TerminalAttention::Fireworks => None,
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
                    && let Some(sender) = self.platform.notification_sender.as_ref()
                    && let Err(error) = sender.send(notification)
                {
                    tracing::warn!(target: "toyoterm::notification", %error, "queue OSC notification failed");
                }
                for event in terminal_events {
                    if let Some(runtime_event) = ruby_event_from_terminal_event(pane, event) {
                        self.scripting.runtime_events.push_back(runtime_event);
                    }
                }
                if let Err(error) = self.deliver_runtime_events() {
                    self.fail(event_loop, error);
                    return;
                }
                if self.ui.pane_layout.rect(pane).is_some() {
                    // Winit coalesces redraw requests. Keep parsing PTY bytes
                    // immediately to preserve ordering, but defer the costly
                    // full-grid snapshot and text shaping until the matching
                    // redraw. This prevents bursty alternate-screen output
                    // (notably Neovim exit) from building a render backlog.
                    self.ui.terminal_render_pending = true;
                    if let Some(window) = self.platform.window.as_ref() {
                        window.request_redraw();
                    }
                }
            }
            AppEvent::Eof { pane } => {
                self.scripting.invalidate_context();
                if let Err(error) = self.close_exited_pane(event_loop, pane) {
                    self.fail(event_loop, error);
                }
            }
            AppEvent::Error { pane, message } => {
                self.scripting.invalidate_context();
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
                            self.scripting
                                .eval_waiters
                                .insert(id, EvalWaiter::Ipc(response));
                        }
                        Err(error) => {
                            let _ = response.send(Err(error));
                        }
                    }
                } else if matches!(request, IpcRequest::Reload) {
                    match self.submit_script(ScriptInvocation::Reload) {
                        Ok(id) => {
                            self.scripting
                                .eval_waiters
                                .insert(id, EvalWaiter::Ipc(response));
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
                self.scripting.in_flight = false;
                if let Err(error) = self.start_next_script() {
                    tracing::warn!(target: "toyoterm::script", %error, "submit queued script request failed");
                }
                if let Some(window) = self.platform.window.clone() {
                    self.sync_active_renderer(window.scale_factor());
                    window.request_redraw();
                }
            }
            AppEvent::AsyncCompleted { id, output } => {
                if self.scripting.cancelled_async_tasks.remove(&id) {
                    return;
                }
                let invocation = ScriptInvocation::AsyncCallback { id, output };
                if let Err(error) = self.submit_script(invocation) {
                    tracing::warn!(target: "toyoterm::script", %error, "submit async callback failed");
                }
            }
            AppEvent::NotificationFeedback(feedback) => {
                let response = notification_feedback_response(&feedback);
                if let Some(runtime) = self.terminal_runtime.pane_runtimes.get_mut(&feedback.pane) {
                    runtime.protocol.active_notifications.remove(&feedback.id);
                    if let Some(session) = runtime.process.pty_session.as_mut()
                        && let Err(error) = session.write(response.as_bytes())
                    {
                        tracing::error!(
                            target: "toyoterm::pty",
                            operation = error.operation(),
                            pane = %feedback.pane,
                            bytes = response.len(),
                            %error,
                            "write OSC 99 notification feedback to pane PTY failed"
                        );
                    }
                }
            }
        }
    }
}

fn set_wait_control_flow(event_loop: &ActiveEventLoop, deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => event_loop.set_control_flow(ControlFlow::WaitUntil(deadline)),
        None => event_loop.set_control_flow(ControlFlow::Wait),
    }
}

fn notification_feedback_response(feedback: &NotificationFeedback) -> String {
    match feedback.kind {
        NotificationFeedbackKind::Activated => format!("\x1b]99;i={};\x1b\\", feedback.id),
        NotificationFeedbackKind::Button(button) => {
            format!("\x1b]99;i={};{button}\x1b\\", feedback.id)
        }
        NotificationFeedbackKind::Closed => {
            format!("\x1b]99;i={}:p=close;\x1b\\", feedback.id)
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

fn notification_expiry(timeout_ms: Option<u32>, now: Instant) -> Option<Instant> {
    timeout_ms
        .filter(|timeout_ms| *timeout_ms > 0)
        .and_then(|timeout_ms| now.checked_add(Duration::from_millis(u64::from(timeout_ms))))
}

fn notification_alive_response(
    request_id: &str,
    active: &mut BTreeMap<String, Option<Instant>>,
    now: Instant,
) -> String {
    active.retain(|_, deadline| deadline.is_none_or(|deadline| deadline > now));
    let ids = active.keys().cloned().collect::<Vec<_>>().join(",");
    format!("\x1b]99;i={request_id}:p=alive;{ids}\x1b\\")
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
        if let Some(window) = self.platform.window.as_ref() {
            window.set_ime_allowed(false);
        }
        for (_, mut runtime) in self.terminal_runtime.pane_runtimes.drain() {
            runtime.terminate();
        }
        self.platform.renderer = None;
        self.platform.window = None;
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
        let notification_proxy = event_proxy.clone();
        let notification_sender = NotificationSender::start(move |feedback| {
            let _ = notification_proxy.send_event(AppEvent::NotificationFeedback(feedback));
        })
        .map_err(|error| {
            tracing::warn!(target: "toyoterm::notification", %error);
            error
        })
        .ok();
        let cell_metrics = CellMetrics {
            width: 9.0 * font_scale,
            height: f64::from(config.font.size * config.ui.line_height),
            horizontal_padding: config.ui.padding_x.round() as u32,
            vertical_padding: config.ui.padding_y.round() as u32,
            font_size: config.font.size,
        };
        Ok(Self {
            event_proxy,
            platform: PlatformState {
                window: None,
                renderer: None,
                occlusion: WindowOcclusion::default(),
                clipboard: None,
                notification_sender,
                #[cfg(target_os = "linux")]
                app_id,
            },
            terminal_runtime: TerminalRuntime {
                pane_runtimes: HashMap::new(),
                pending_pane_launches,
            },
            scripting: ScriptRuntimeState {
                thread: script_thread,
                snapshot: Arc::new(script_snapshot),
                next_request_id: 1,
                in_flight: false,
                pending: VecDeque::new(),
                runtime_events: VecDeque::new(),
                event_drops: 0,
                eval_waiters: HashMap::new(),
                cancelled_async_tasks: HashSet::new(),
                cached_model: None,
                cached_handles: None,
                context_dirty: true,
            },
            ui: UiState {
                pane_layout: PaneLayout::default(),
                tab_layout: TabStripLayout::default(),
                workspace_layout: WorkspaceStripLayout::default(),
                search_open: false,
                search_query: String::new(),
                search_result: SearchResult::default(),
                selector: None,
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
                pressed_mouse_button: None,
                last_mouse_cell: None,
                wheel_line_accumulator: 0.0,
                selecting: false,
                visual_selection: None,
                click_tracker: ClickTracker::default(),
                pending_clipboard_writes: Vec::new(),
                pane_badges: HashMap::new(),
                cell_metrics,
                bar_items: HashMap::new(),
                bar_pending: None,
                next_bar_at: HashMap::new(),
                terminal_render_pending: false,
                render_style,
                window_title_override,
            },
            _ipc_server: ipc_server,
            mux,
            fatal_error: None,
            exit_after_startup,
        })
    }

    fn base_window_title(&self) -> &str {
        self.ui
            .window_title_override
            .as_deref()
            .unwrap_or(&self.scripting.snapshot.config.window.title)
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.fatal_error = Some(error);
        event_loop.exit();
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
