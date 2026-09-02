use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use glyphon::cosmic_text::{Cursor as TextCursor, Fallback, PlatformFallback};
use glyphon::{
    Attrs, Buffer, Cache as GlyphCache, Color as GlyphColor, Family, FontSystem, Metrics,
    Resolution, Shaping, Style, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer,
    Viewport, Weight, Wrap,
};
use unicode_script::Script;
use wgpu::util::DeviceExt;
use wgpu::{
    BlendState, BufferUsages, Color, ColorTargetState, ColorWrites, CommandEncoderDescriptor,
    CompositeAlphaMode, CurrentSurfaceTexture, Device, DeviceDescriptor, FragmentState, Instance,
    LoadOp, Operations, PipelineCompilationOptions, PrimitiveState, PrimitiveTopology, Queue,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor,
    ShaderModuleDescriptor, ShaderSource, StoreOp, Surface, SurfaceConfiguration,
    TextureViewDescriptor, VertexState,
};
use winit::dpi::PhysicalSize;
use winit::window::Window;

use toyoterm_api::{PaneId, TabId, WorkspaceId};
use toyoterm_terminal::{CellAttributes, CellColor, CursorShape, CursorState, TerminalSnapshot};

mod layout;
pub use layout::{
    ConfigErrorLayout, PaneLayout, PanePlacement, PaneRect, SplitAxis, SplitBoundary, TabPlacement,
    TabStripLayout, WorkspacePlacement, WorkspaceStripLayout,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextLayout {
    pub font_size: f32,
    pub line_height: f32,
    pub cell_width: f32,
    pub horizontal_padding: f32,
    pub vertical_padding: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct PaneRenderData<'a> {
    pub pane: PaneId,
    pub snapshot: &'a TerminalSnapshot,
    pub cursor: CursorState,
    pub rect: PaneRect,
    pub active: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct TabRenderData<'a> {
    pub tab: TabId,
    pub title: &'a str,
    pub rect: PaneRect,
    pub active: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct WorkspaceRenderData<'a> {
    pub workspace: WorkspaceId,
    pub name: &'a str,
    pub rect: PaneRect,
    pub active: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct PaletteRenderData<'a> {
    pub rect: PaneRect,
    pub text: &'a str,
}

#[derive(Clone, Copy, Debug)]
pub struct StatusBarRenderData<'a> {
    pub rect: PaneRect,
    pub text: &'a str,
}

#[derive(Clone, Copy, Debug)]
pub struct ConfigErrorRenderData<'a> {
    pub message: &'a str,
    pub notice_rect: PaneRect,
    pub open_log_rect: PaneRect,
    pub open_ruby_console_rect: PaneRect,
    pub dismiss_rect: PaneRect,
    pub log_expanded: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderStyle {
    pub font_family: String,
    pub font_fallback: Vec<String>,
    pub font_weight: u16,
    pub background: [u8; 3],
    pub foreground: [u8; 3],
    pub cursor: [u8; 3],
    pub selection: [u8; 3],
    pub ansi: [[u8; 3]; 16],
    pub opacity: f32,
}

impl Default for RenderStyle {
    fn default() -> Self {
        Self {
            font_family: "monospace".into(),
            font_fallback: Vec::new(),
            font_weight: 400,
            background: [9, 11, 14],
            foreground: [220, 225, 232],
            cursor: [245, 247, 250],
            selection: [55, 88, 145],
            ansi: default_ansi_palette(),
            opacity: 1.0,
        }
    }
}

impl RenderStyle {
    pub fn from_hex(
        font_family: impl Into<String>,
        font_fallback: Vec<String>,
        font_weight: u16,
        colors: [&str; 4],
        opacity: f32,
    ) -> Result<Self, RenderError> {
        Self::from_hex_with_ansi(
            font_family,
            font_fallback,
            font_weight,
            colors,
            &[],
            opacity,
        )
    }

    pub fn from_hex_with_ansi(
        font_family: impl Into<String>,
        font_fallback: Vec<String>,
        font_weight: u16,
        colors: [&str; 4],
        ansi: &[String],
        opacity: f32,
    ) -> Result<Self, RenderError> {
        let [background, foreground, cursor, selection] = colors;
        let mut parsed_ansi = default_ansi_palette();
        if !ansi.is_empty() {
            if ansi.len() != 16 {
                return Err(RenderError::new(
                    "parse color",
                    format!("expected 16 ANSI colors, got {}", ansi.len()),
                ));
            }
            for (target, value) in parsed_ansi.iter_mut().zip(ansi) {
                *target = parse_rgb(value)?;
            }
        }
        Ok(Self {
            font_family: font_family.into(),
            font_fallback,
            font_weight,
            background: parse_rgb(background)?,
            foreground: parse_rgb(foreground)?,
            cursor: parse_rgb(cursor)?,
            selection: parse_rgb(selection)?,
            ansi: parsed_ansi,
            opacity,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderOutcome {
    Presented,
    Skipped,
    DeviceLost,
}

#[derive(Clone, Debug, Default)]
struct DeviceLossState(Arc<AtomicBool>);

impl DeviceLossState {
    fn mark_lost(&self) {
        self.0.store(true, Ordering::Release);
    }

    fn take_lost(&self) -> bool {
        self.0.swap(false, Ordering::AcqRel)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PaneTextPlacement {
    bounds: PaneRect,
    text_left: f32,
    text_top: f32,
    cursor_left: f32,
    cursor_top: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SurfaceResize {
    Suspend,
    Configure { width: u32, height: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SurfaceRecoveryAction {
    Skip,
    Reconfigure,
    Recreate,
    Fail,
}

#[derive(Debug)]
pub struct RenderError {
    operation: &'static str,
    message: String,
}

impl RenderError {
    fn new(operation: &'static str, error: impl fmt::Display) -> Self {
        Self {
            operation,
            message: error.to_string(),
        }
    }

    pub fn operation(&self) -> &'static str {
        self.operation
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.operation, self.message)
    }
}

impl std::error::Error for RenderError {}

pub struct GpuRenderer {
    instance: Instance,
    window: Arc<Window>,
    surface: Surface<'static>,
    device: Device,
    queue: Queue,
    device_loss: DeviceLossState,
    configuration: SurfaceConfiguration,
    supported_alpha_modes: Vec<CompositeAlphaMode>,
    suspended: bool,
    clear_color: Color,
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    text_atlas: TextAtlas,
    text_renderer: TextRenderer,
    ui_pipeline: RenderPipeline,
    panes: HashMap<PaneId, PaneBuffers>,
    tabs: HashMap<TabId, TabBuffer>,
    workspaces: HashMap<WorkspaceId, TabBuffer>,
    palette: OverlayBuffer,
    status_bar: OverlayBuffer,
    config_error: ConfigErrorBuffers,
    preedit: Buffer,
    has_preedit: bool,
    style: RenderStyle,
}

struct TabBuffer {
    text: Buffer,
    rect: PaneRect,
    active: bool,
}

struct OverlayBuffer {
    text: Buffer,
    rect: Option<PaneRect>,
}

struct ConfigErrorBuffers {
    title: Buffer,
    message: Buffer,
    open_log: Buffer,
    open_ruby_console: Buffer,
    dismiss: Buffer,
    layout: Option<ConfigErrorRenderLayout>,
}

#[derive(Clone, Copy)]
struct ConfigErrorRenderLayout {
    notice: PaneRect,
    message_top: f32,
    open_log: PaneRect,
    open_ruby_console: PaneRect,
    dismiss: PaneRect,
}

struct PaneBuffers {
    selection: Buffer,
    text: Buffer,
    cursor_glyph: Buffer,
    layout: TextLayout,
    cursor: CursorState,
    cursor_x: f32,
    rect: PaneRect,
    active: bool,
    has_selection: bool,
    backgrounds: Vec<(PaneRect, [u8; 3])>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct UiVertex {
    position: [f32; 2],
    color: [f32; 4],
}

impl UiVertex {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
    };
}

struct ConfiguredFallback {
    configured: Vec<&'static str>,
    common: Vec<&'static str>,
    platform: PlatformFallback,
}

impl ConfiguredFallback {
    fn new(families: &[String]) -> Self {
        let configured = families
            .iter()
            .map(|family| intern_font_family(family))
            .collect::<Vec<_>>();
        let platform = PlatformFallback;
        let mut common = configured.clone();
        common.extend_from_slice(platform.common_fallback());
        Self {
            configured,
            common,
            platform,
        }
    }
}

impl Fallback for ConfiguredFallback {
    fn common_fallback(&self) -> &[&'static str] {
        &self.common
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        self.platform.forbidden_fallback()
    }

    fn script_fallback(&self, _script: Script, _locale: &str) -> &[&'static str] {
        // Script-specific candidates are considered before the common list, so
        // returning the configured list here preserves the user's priority.
        &self.configured
    }
}

fn intern_font_family(family: &str) -> &'static str {
    static INTERNER: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let mut interner = INTERNER
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(interned) = interner.get(family) {
        return interned;
    }
    // cosmic-text's Fallback API requires family names with process lifetime.
    let interned = Box::leak(family.to_owned().into_boxed_str());
    interner.insert(family.to_owned(), interned);
    interned
}

fn configured_font_system(fallback: &[String]) -> FontSystem {
    if fallback.is_empty() {
        return FontSystem::new();
    }
    let locale = sys_locale::get_locale().unwrap_or_else(|| "en-US".to_owned());
    let mut database = glyphon::fontdb::Database::new();
    database.load_system_fonts();
    database.set_monospace_family("Noto Sans Mono");
    database.set_sans_serif_family("Open Sans");
    database.set_serif_family("DejaVu Serif");
    FontSystem::new_with_locale_and_db_and_fallback(
        locale,
        database,
        ConfiguredFallback::new(fallback),
    )
}

impl PaneBuffers {
    fn new(font_system: &mut FontSystem, metrics: Metrics, layout: TextLayout) -> Self {
        let mut buffer = || {
            let mut buffer = Buffer::new(font_system, metrics);
            buffer.set_wrap(Wrap::None);
            buffer
        };
        Self {
            selection: buffer(),
            text: buffer(),
            cursor_glyph: buffer(),
            layout,
            cursor: CursorState {
                column: 0,
                row: 0,
                visible: false,
                shape: CursorShape::Block,
            },
            cursor_x: 0.0,
            rect: PaneRect::default(),
            active: false,
            has_selection: false,
            backgrounds: Vec::new(),
        }
    }
}

impl GpuRenderer {
    pub async fn new(window: Arc<Window>) -> Result<Self, RenderError> {
        tracing::debug!(target: "toyoterm::render", "initialize GPU renderer");
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);
        let instance = Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .map_err(|error| RenderError::new("create GPU surface", error))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|error| RenderError::new("request GPU adapter", error))?;
        let (device, queue) = adapter
            .request_device(&DeviceDescriptor {
                label: Some("toyoterm device"),
                ..Default::default()
            })
            .await
            .map_err(|error| RenderError::new("request GPU device", error))?;
        let device_loss = DeviceLossState::default();
        let callback_state = device_loss.clone();
        let redraw_window = window.clone();
        device.set_device_lost_callback(move |reason, message| {
            tracing::error!(
                target: "toyoterm::render",
                ?reason,
                %message,
                "GPU device lost; renderer recovery requested"
            );
            callback_state.mark_lost();
            redraw_window.request_redraw();
        });
        let capabilities = surface.get_capabilities(&adapter);
        let supported_alpha_modes = capabilities.alpha_modes.clone();
        let mut configuration = surface
            .get_default_config(&adapter, width, height)
            .ok_or_else(|| RenderError::new("configure GPU surface", "unsupported surface"))?;
        if let Some(format) = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| format.is_srgb())
        {
            configuration.format = format;
        }
        configuration.desired_maximum_frame_latency = 1;
        surface.configure(&device, &configuration);
        tracing::info!(
            target: "toyoterm::render",
            width,
            height,
            format = ?configuration.format,
            "GPU renderer initialized"
        );

        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let glyph_cache = GlyphCache::new(&device);
        let viewport = Viewport::new(&device, &glyph_cache);
        let mut text_atlas = TextAtlas::new(&device, &queue, &glyph_cache, configuration.format);
        let text_renderer = TextRenderer::new(
            &mut text_atlas,
            &device,
            wgpu::MultisampleState::default(),
            None,
        );
        let ui_pipeline = create_ui_pipeline(&device, configuration.format);
        let mut preedit = Buffer::new(&mut font_system, Metrics::new(14.0, 18.0));
        preedit.set_wrap(Wrap::None);
        let mut palette_text = Buffer::new(&mut font_system, Metrics::new(14.0, 18.0));
        palette_text.set_wrap(Wrap::None);
        let mut status_text = Buffer::new(&mut font_system, Metrics::new(12.0, 15.0));
        status_text.set_wrap(Wrap::None);
        let mut config_error_buffer = || {
            let mut buffer = Buffer::new(&mut font_system, Metrics::new(14.0, 18.0));
            buffer.set_wrap(Wrap::None);
            buffer
        };
        let config_error = ConfigErrorBuffers {
            title: config_error_buffer(),
            message: config_error_buffer(),
            open_log: config_error_buffer(),
            open_ruby_console: config_error_buffer(),
            dismiss: config_error_buffer(),
            layout: None,
        };
        let style = RenderStyle::default();
        let initial_alpha_mode = configuration.alpha_mode;
        Ok(Self {
            instance,
            window,
            surface,
            device,
            queue,
            device_loss,
            configuration,
            supported_alpha_modes,
            suspended: size.width == 0 || size.height == 0,
            clear_color: clear_color(&style, initial_alpha_mode),
            font_system,
            swash_cache,
            viewport,
            text_atlas,
            text_renderer,
            ui_pipeline,
            panes: HashMap::new(),
            tabs: HashMap::new(),
            workspaces: HashMap::new(),
            palette: OverlayBuffer {
                text: palette_text,
                rect: None,
            },
            status_bar: OverlayBuffer {
                text: status_text,
                rect: None,
            },
            config_error,
            preedit,
            has_preedit: false,
            style,
        })
    }

    pub fn set_style(&mut self, style: RenderStyle) {
        if self.style.font_fallback != style.font_fallback {
            self.reset_font_system(&style.font_fallback);
        }
        let alpha_mode = preferred_alpha_mode(&self.supported_alpha_modes, style.opacity);
        let alpha_mode_changed = self.configuration.alpha_mode != alpha_mode;
        self.configuration.alpha_mode = alpha_mode;
        self.clear_color = clear_color(&style, alpha_mode);
        self.style = style;
        if alpha_mode_changed && !self.suspended {
            self.surface.configure(&self.device, &self.configuration);
        }
    }

    fn reset_font_system(&mut self, fallback: &[String]) {
        self.font_system = configured_font_system(fallback);
        self.panes.clear();
        self.tabs.clear();
        self.workspaces.clear();

        let mut preedit = Buffer::new(&mut self.font_system, Metrics::new(14.0, 18.0));
        preedit.set_wrap(Wrap::None);
        self.preedit = preedit;
        self.has_preedit = false;

        let mut palette_text = Buffer::new(&mut self.font_system, Metrics::new(14.0, 18.0));
        palette_text.set_wrap(Wrap::None);
        self.palette = OverlayBuffer {
            text: palette_text,
            rect: None,
        };
        let mut status_text = Buffer::new(&mut self.font_system, Metrics::new(12.0, 15.0));
        status_text.set_wrap(Wrap::None);
        self.status_bar = OverlayBuffer {
            text: status_text,
            rect: None,
        };

        let mut buffer = || {
            let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(14.0, 18.0));
            buffer.set_wrap(Wrap::None);
            buffer
        };
        self.config_error = ConfigErrorBuffers {
            title: buffer(),
            message: buffer(),
            open_log: buffer(),
            open_ruby_console: buffer(),
            dismiss: buffer(),
            layout: None,
        };
    }

    pub fn update_panes(&mut self, panes: &[PaneRenderData<'_>], layout: TextLayout) {
        let active_panes = panes.iter().map(|pane| pane.pane).collect::<HashSet<_>>();
        self.panes.retain(|pane, _| active_panes.contains(pane));
        let metrics = Metrics::new(layout.font_size.max(1.0), layout.line_height.max(1.0));
        let font_family = self.style.font_family.clone();
        let font_weight = self.style.font_weight;
        let foreground = self.style.foreground;
        let background = self.style.background;
        let ansi = self.style.ansi;
        for pane in panes {
            let rich_text = terminal_rich_text(
                pane.snapshot,
                Some(pane.cursor),
                &font_family,
                font_weight,
                foreground,
                background,
                &ansi,
            );
            if !self.panes.contains_key(&pane.pane) {
                self.panes.insert(
                    pane.pane,
                    PaneBuffers::new(&mut self.font_system, metrics, layout),
                );
            }
            let buffers = self
                .panes
                .get_mut(&pane.pane)
                .expect("pane buffers were inserted");
            buffers.layout = layout;
            buffers.cursor = pane.cursor;
            buffers.rect = pane.rect;
            buffers.active = pane.active;
            buffers.has_selection = !pane.snapshot.selection.is_empty();
            buffers.backgrounds = terminal_backgrounds(
                pane.snapshot,
                pane.rect,
                layout,
                background,
                foreground,
                &ansi,
            );
            let content_width = pane
                .rect
                .width
                .saturating_sub((layout.horizontal_padding * 2.0) as u32)
                as f32;
            let content_height =
                pane.rect
                    .height
                    .saturating_sub((layout.vertical_padding * 2.0) as u32) as f32;
            buffers
                .text
                .set_metrics_and_size(metrics, Some(content_width), Some(content_height));
            let default_attrs = Attrs::new()
                .family(Family::Name(&font_family))
                .weight(Weight(font_weight));
            if rich_text.is_empty() {
                buffers.text.set_text(
                    &pane.snapshot.lines.join("\n"),
                    &default_attrs,
                    Shaping::Advanced,
                    None,
                );
            } else {
                buffers.text.set_rich_text(
                    rich_text
                        .iter()
                        .map(|(text, attrs)| (text.as_str(), attrs.clone())),
                    &default_attrs,
                    Shaping::Advanced,
                    None,
                );
            }
            buffers
                .text
                .shape_until_scroll(&mut self.font_system, false);
            buffers.cursor_x = terminal_cursor_x(&buffers.text, pane.snapshot, pane.cursor)
                .unwrap_or_else(|| f32::from(pane.cursor.column) * layout.cell_width);

            buffers.selection.set_metrics_and_size(
                metrics,
                Some(content_width),
                Some(content_height),
            );
            buffers.selection.set_text(
                &selection_text(pane.snapshot),
                &Attrs::new()
                    .family(Family::Name(&self.style.font_family))
                    .weight(Weight(self.style.font_weight)),
                Shaping::Advanced,
                None,
            );
            buffers
                .selection
                .shape_until_scroll(&mut self.font_system, false);

            buffers
                .cursor_glyph
                .set_metrics_and_size(metrics, None, None);
            buffers.cursor_glyph.set_text(
                match pane.cursor.shape {
                    CursorShape::Block => "█",
                    CursorShape::Beam => "▏",
                    CursorShape::Underline => "▁",
                },
                &Attrs::new()
                    .family(Family::Name(&self.style.font_family))
                    .weight(Weight(self.style.font_weight)),
                Shaping::Advanced,
                None,
            );
            buffers
                .cursor_glyph
                .shape_until_scroll(&mut self.font_system, false);
        }
    }

    pub fn update_tabs(&mut self, tabs: &[TabRenderData<'_>], layout: TextLayout) {
        let active_tabs = tabs.iter().map(|tab| tab.tab).collect::<HashSet<_>>();
        self.tabs.retain(|tab, _| active_tabs.contains(tab));
        let metrics = Metrics::new(layout.font_size.max(1.0), layout.line_height.max(1.0));
        for tab in tabs {
            if !self.tabs.contains_key(&tab.tab) {
                let mut text = Buffer::new(&mut self.font_system, metrics);
                text.set_wrap(Wrap::None);
                self.tabs.insert(
                    tab.tab,
                    TabBuffer {
                        text,
                        rect: tab.rect,
                        active: tab.active,
                    },
                );
            }
            let buffer = self
                .tabs
                .get_mut(&tab.tab)
                .expect("tab buffer was inserted");
            buffer.rect = tab.rect;
            buffer.active = tab.active;
            buffer.text.set_metrics_and_size(
                metrics,
                Some(tab.rect.width.saturating_sub(16) as f32),
                Some(tab.rect.height as f32),
            );
            buffer.text.set_text(
                tab.title,
                &Attrs::new()
                    .family(Family::Name(&self.style.font_family))
                    .weight(Weight(self.style.font_weight)),
                Shaping::Advanced,
                None,
            );
            buffer.text.shape_until_scroll(&mut self.font_system, false);
        }
    }

    pub fn update_workspaces(
        &mut self,
        workspaces: &[WorkspaceRenderData<'_>],
        layout: TextLayout,
    ) {
        let active_workspaces = workspaces
            .iter()
            .map(|workspace| workspace.workspace)
            .collect::<HashSet<_>>();
        self.workspaces
            .retain(|workspace, _| active_workspaces.contains(workspace));
        let metrics = Metrics::new(
            (layout.font_size * 0.85).max(1.0),
            (layout.line_height * 0.85).max(1.0),
        );
        for workspace in workspaces {
            if !self.workspaces.contains_key(&workspace.workspace) {
                let mut text = Buffer::new(&mut self.font_system, metrics);
                text.set_wrap(Wrap::None);
                self.workspaces.insert(
                    workspace.workspace,
                    TabBuffer {
                        text,
                        rect: workspace.rect,
                        active: workspace.active,
                    },
                );
            }
            let buffer = self
                .workspaces
                .get_mut(&workspace.workspace)
                .expect("workspace buffer was inserted");
            buffer.rect = workspace.rect;
            buffer.active = workspace.active;
            buffer.text.set_metrics_and_size(
                metrics,
                Some(workspace.rect.width.saturating_sub(12) as f32),
                Some(workspace.rect.height as f32),
            );
            buffer.text.set_text(
                workspace.name,
                &Attrs::new()
                    .family(Family::Name(&self.style.font_family))
                    .weight(Weight(self.style.font_weight)),
                Shaping::Advanced,
                None,
            );
            buffer.text.shape_until_scroll(&mut self.font_system, false);
        }
    }

    pub fn update_palette(&mut self, palette: Option<PaletteRenderData<'_>>, layout: TextLayout) {
        let Some(palette) = palette else {
            self.palette.rect = None;
            return;
        };
        self.palette.rect = Some(palette.rect);
        let metrics = Metrics::new(layout.font_size.max(1.0), layout.line_height.max(1.0));
        self.palette.text.set_metrics_and_size(
            metrics,
            Some(palette.rect.width.saturating_sub(24) as f32),
            Some(palette.rect.height.saturating_sub(16) as f32),
        );
        self.palette.text.set_text(
            palette.text,
            &Attrs::new()
                .family(Family::Name(&self.style.font_family))
                .weight(Weight(self.style.font_weight)),
            Shaping::Advanced,
            None,
        );
        self.palette
            .text
            .shape_until_scroll(&mut self.font_system, false);
    }

    pub fn update_status_bar(
        &mut self,
        status: Option<StatusBarRenderData<'_>>,
        layout: TextLayout,
    ) {
        let Some(status) = status else {
            self.status_bar.rect = None;
            return;
        };
        self.status_bar.rect = Some(status.rect);
        let metrics = Metrics::new(
            (layout.font_size * 0.85).max(1.0),
            (layout.line_height * 0.85).max(1.0),
        );
        self.status_bar.text.set_metrics_and_size(
            metrics,
            Some(status.rect.width.saturating_sub(16) as f32),
            Some(status.rect.height as f32),
        );
        self.status_bar.text.set_text(
            status.text,
            &Attrs::new()
                .family(Family::Name(&self.style.font_family))
                .weight(Weight(self.style.font_weight)),
            Shaping::Advanced,
            None,
        );
        self.status_bar
            .text
            .shape_until_scroll(&mut self.font_system, false);
    }

    pub fn update_config_error(
        &mut self,
        error: Option<ConfigErrorRenderData<'_>>,
        layout: TextLayout,
    ) {
        let Some(error) = error else {
            self.config_error.layout = None;
            return;
        };
        self.config_error.layout = Some(ConfigErrorRenderLayout {
            notice: error.notice_rect,
            message_top: error.notice_rect.y as f32 + layout.line_height + 4.0,
            open_log: error.open_log_rect,
            open_ruby_console: error.open_ruby_console_rect,
            dismiss: error.dismiss_rect,
        });
        let metrics = Metrics::new(layout.font_size.max(1.0), layout.line_height.max(1.0));
        let attrs = Attrs::new()
            .family(Family::Name(&self.style.font_family))
            .weight(Weight(self.style.font_weight));

        self.config_error.title.set_metrics_and_size(
            metrics,
            Some(error.notice_rect.width.saturating_sub(16) as f32),
            Some(layout.line_height),
        );
        self.config_error.title.set_text(
            "Toyoterm Ruby Error",
            &attrs.clone().weight(Weight(700)),
            Shaping::Advanced,
            None,
        );
        self.config_error
            .title
            .shape_until_scroll(&mut self.font_system, false);

        let message_height = error
            .notice_rect
            .height
            .saturating_sub(error.open_log_rect.height)
            .saturating_sub(layout.line_height as u32);
        self.config_error.message.set_metrics_and_size(
            metrics,
            Some(error.notice_rect.width.saturating_sub(16) as f32),
            Some(message_height as f32),
        );
        self.config_error
            .message
            .set_text(error.message, &attrs, Shaping::Advanced, None);
        self.config_error
            .message
            .shape_until_scroll(&mut self.font_system, false);

        for (buffer, rect, text) in [
            (
                &mut self.config_error.open_log,
                error.open_log_rect,
                if error.log_expanded {
                    "Hide Log"
                } else {
                    "Open Log"
                },
            ),
            (
                &mut self.config_error.open_ruby_console,
                error.open_ruby_console_rect,
                "Open Ruby Console",
            ),
            (
                &mut self.config_error.dismiss,
                error.dismiss_rect,
                "Dismiss",
            ),
        ] {
            buffer.set_metrics_and_size(
                metrics,
                Some(rect.width.saturating_sub(16) as f32),
                Some(rect.height as f32),
            );
            buffer.set_text(text, &attrs, Shaping::Advanced, None);
            buffer.shape_until_scroll(&mut self.font_system, false);
        }
    }

    pub fn update_preedit(&mut self, text: Option<&str>, layout: TextLayout) {
        let text = text.unwrap_or_default();
        self.has_preedit = !text.is_empty();
        let metrics = Metrics::new(layout.font_size.max(1.0), layout.line_height.max(1.0));
        self.preedit.set_metrics_and_size(metrics, None, None);
        self.preedit.set_text(
            text,
            &Attrs::new()
                .family(Family::Name(&self.style.font_family))
                .weight(Weight(self.style.font_weight)),
            Shaping::Advanced,
            None,
        );
        self.preedit
            .shape_until_scroll(&mut self.font_system, false);
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        match surface_resize(size) {
            SurfaceResize::Suspend => {
                self.suspended = true;
            }
            SurfaceResize::Configure { width, height } => {
                self.suspended = false;
                self.configuration.width = width;
                self.configuration.height = height;
                self.surface.configure(&self.device, &self.configuration);
            }
        }
    }

    pub fn size(&self) -> PhysicalSize<u32> {
        PhysicalSize::new(self.configuration.width, self.configuration.height)
    }

    pub fn render(&mut self) -> Result<RenderOutcome, RenderError> {
        if self.device_loss.take_lost() {
            return Ok(RenderOutcome::DeviceLost);
        }
        if self.suspended {
            return Ok(RenderOutcome::Skipped);
        }

        self.viewport.update(
            &self.queue,
            Resolution {
                width: self.configuration.width,
                height: self.configuration.height,
            },
        );
        let mut text_areas =
            Vec::with_capacity(self.panes.len() * 3 + self.tabs.len() + self.workspaces.len() + 7);
        for workspace in self.workspaces.values() {
            text_areas.push(TextArea {
                buffer: &workspace.text,
                left: workspace.rect.x as f32 + 6.0,
                top: workspace.rect.y as f32 + 2.0,
                scale: 1.0,
                bounds: pane_bounds(workspace.rect),
                default_color: if workspace.active {
                    glyph_color(self.style.cursor, 255)
                } else {
                    glyph_color(self.style.foreground, 140)
                },
                custom_glyphs: &[],
            });
        }
        for tab in self.tabs.values() {
            text_areas.push(TextArea {
                buffer: &tab.text,
                left: tab.rect.x as f32 + 8.0,
                top: tab.rect.y as f32 + 4.0,
                scale: 1.0,
                bounds: pane_bounds(tab.rect),
                default_color: if tab.active {
                    glyph_color(self.style.cursor, 255)
                } else {
                    glyph_color(self.style.foreground, 170)
                },
                custom_glyphs: &[],
            });
        }
        if let Some(rect) = self.palette.rect {
            text_areas.push(TextArea {
                buffer: &self.palette.text,
                left: rect.x as f32 + 12.0,
                top: rect.y as f32 + 8.0,
                scale: 1.0,
                bounds: pane_bounds(rect),
                default_color: glyph_color(self.style.cursor, 255),
                custom_glyphs: &[],
            });
        }
        if let Some(rect) = self.status_bar.rect {
            text_areas.push(TextArea {
                buffer: &self.status_bar.text,
                left: rect.x as f32 + 8.0,
                top: rect.y as f32 + 2.0,
                scale: 1.0,
                bounds: pane_bounds(rect),
                default_color: glyph_color(self.style.foreground, 190),
                custom_glyphs: &[],
            });
        }
        if let Some(layout) = self.config_error.layout {
            text_areas.push(TextArea {
                buffer: &self.config_error.title,
                left: layout.notice.x as f32 + 8.0,
                top: layout.notice.y as f32 + 4.0,
                scale: 1.0,
                bounds: pane_bounds(layout.notice),
                default_color: glyph_color([255, 120, 110], 255),
                custom_glyphs: &[],
            });
            text_areas.push(TextArea {
                buffer: &self.config_error.message,
                left: layout.notice.x as f32 + 8.0,
                top: layout.message_top,
                scale: 1.0,
                bounds: pane_bounds(layout.notice),
                default_color: glyph_color(self.style.foreground, 255),
                custom_glyphs: &[],
            });
            for (buffer, rect) in [
                (&self.config_error.open_log, layout.open_log),
                (
                    &self.config_error.open_ruby_console,
                    layout.open_ruby_console,
                ),
                (&self.config_error.dismiss, layout.dismiss),
            ] {
                text_areas.push(TextArea {
                    buffer,
                    left: rect.x as f32 + 8.0,
                    top: rect.y as f32 + 4.0,
                    scale: 1.0,
                    bounds: pane_bounds(rect),
                    default_color: glyph_color(self.style.cursor, 255),
                    custom_glyphs: &[],
                });
            }
        }
        for pane in self.panes.values() {
            let placement = pane_text_placement(pane.rect, pane.layout, pane.cursor, pane.cursor_x);
            let bounds = pane_bounds(placement.bounds);
            if pane.has_selection {
                text_areas.push(TextArea {
                    buffer: &pane.selection,
                    left: placement.text_left,
                    top: placement.text_top,
                    scale: 1.0,
                    bounds,
                    default_color: glyph_color(self.style.selection, 210),
                    custom_glyphs: &[],
                });
            }
            text_areas.push(TextArea {
                buffer: &pane.text,
                left: placement.text_left,
                top: placement.text_top,
                scale: 1.0,
                bounds,
                default_color: glyph_color(self.style.foreground, 255),
                custom_glyphs: &[],
            });
            if pane.active && pane.cursor.visible {
                text_areas.push(TextArea {
                    buffer: &pane.cursor_glyph,
                    left: placement.cursor_left,
                    top: placement.cursor_top,
                    scale: 1.0,
                    bounds,
                    default_color: glyph_color(self.style.cursor, 255),
                    custom_glyphs: &[],
                });
            }
            if pane.active && self.has_preedit {
                text_areas.push(TextArea {
                    buffer: &self.preedit,
                    left: placement.cursor_left,
                    top: placement.cursor_top,
                    scale: 1.0,
                    bounds,
                    default_color: glyph_color(self.style.cursor, 255),
                    custom_glyphs: &[],
                });
            }
        }
        self.text_renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.text_atlas,
                &self.viewport,
                text_areas,
                &mut self.swash_cache,
            )
            .map_err(|error| RenderError::new("prepare terminal text", error))?;

        let (frame, suboptimal) = match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) => (frame, false),
            CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            status => match surface_recovery_action(&status)
                .expect("non-frame surface status has a recovery action")
            {
                SurfaceRecoveryAction::Skip => return Ok(RenderOutcome::Skipped),
                SurfaceRecoveryAction::Reconfigure => {
                    self.surface.configure(&self.device, &self.configuration);
                    return Ok(RenderOutcome::Skipped);
                }
                SurfaceRecoveryAction::Recreate => {
                    self.recreate_surface()?;
                    return Ok(RenderOutcome::Skipped);
                }
                SurfaceRecoveryAction::Fail => {
                    return Err(RenderError::new(
                        "acquire GPU frame",
                        "surface validation failed",
                    ));
                }
            },
        };

        let view = frame.texture.create_view(&TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("toyoterm frame encoder"),
            });
        let ui_vertices = self.ui_vertices();
        let ui_buffer = (!ui_vertices.is_empty()).then(|| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("toyoterm UI vertices"),
                    contents: bytemuck::cast_slice(&ui_vertices),
                    usage: BufferUsages::VERTEX,
                })
        });
        {
            let color_attachments = [Some(RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(self.clear_color),
                    store: StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("toyoterm frame pass"),
                color_attachments: &color_attachments,
                ..Default::default()
            });
            if let Some(ui_buffer) = ui_buffer.as_ref() {
                pass.set_pipeline(&self.ui_pipeline);
                pass.set_vertex_buffer(0, ui_buffer.slice(..));
                pass.draw(0..ui_vertices.len() as u32, 0..1);
            }
            self.text_renderer
                .render(&self.text_atlas, &self.viewport, &mut pass)
                .map_err(|error| RenderError::new("render terminal text", error))?;
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);

        if suboptimal {
            self.surface.configure(&self.device, &self.configuration);
        }
        self.text_atlas.trim();
        Ok(RenderOutcome::Presented)
    }

    fn ui_vertices(&self) -> Vec<UiVertex> {
        let mut vertices = Vec::new();
        let width = self.configuration.width;

        if let Some(first) = self.workspaces.values().next() {
            push_ui_rect(
                &mut vertices,
                PaneRect::new(0, first.rect.y, width, first.rect.height),
                mix_rgb(self.style.background, self.style.foreground, 0.035, 0.96),
                self.configuration.width,
                self.configuration.height,
            );
        }
        for workspace in self.workspaces.values() {
            if workspace.active {
                push_ui_rect(
                    &mut vertices,
                    PaneRect::new(
                        workspace.rect.x.saturating_add(4),
                        workspace
                            .rect
                            .y
                            .saturating_add(workspace.rect.height.saturating_sub(2)),
                        workspace.rect.width.saturating_sub(8),
                        2,
                    ),
                    rgba(self.style.selection, 0.9),
                    self.configuration.width,
                    self.configuration.height,
                );
            }
        }

        if let Some(first) = self.tabs.values().next() {
            push_ui_rect(
                &mut vertices,
                PaneRect::new(0, first.rect.y, width, first.rect.height),
                mix_rgb(self.style.background, self.style.foreground, 0.055, 0.98),
                self.configuration.width,
                self.configuration.height,
            );
        }
        for tab in self.tabs.values() {
            let fill = if tab.active {
                mix_rgb(self.style.background, self.style.selection, 0.18, 1.0)
            } else {
                mix_rgb(self.style.background, self.style.foreground, 0.075, 0.72)
            };
            push_ui_rect(
                &mut vertices,
                inset_rect(tab.rect, 1, 1, 1, 0),
                fill,
                self.configuration.width,
                self.configuration.height,
            );
            push_ui_rect(
                &mut vertices,
                PaneRect::new(
                    tab.rect.x.saturating_add(tab.rect.width.saturating_sub(1)),
                    tab.rect.y.saturating_add(5),
                    1,
                    tab.rect.height.saturating_sub(10),
                ),
                rgba(self.style.foreground, 0.2),
                self.configuration.width,
                self.configuration.height,
            );
            if tab.active {
                push_ui_rect(
                    &mut vertices,
                    PaneRect::new(
                        tab.rect.x.saturating_add(1),
                        tab.rect.y.saturating_add(tab.rect.height.saturating_sub(3)),
                        tab.rect.width.saturating_sub(2),
                        3,
                    ),
                    rgba(self.style.selection, 1.0),
                    self.configuration.width,
                    self.configuration.height,
                );
            }
        }

        for pane in self.panes.values() {
            for (rect, color) in &pane.backgrounds {
                push_ui_rect(
                    &mut vertices,
                    *rect,
                    rgba(*color, 1.0),
                    self.configuration.width,
                    self.configuration.height,
                );
            }
        }

        for pane in self.panes.values().filter(|pane| pane.active) {
            push_ui_rect(
                &mut vertices,
                PaneRect::new(pane.rect.x, pane.rect.y, pane.rect.width, 2),
                rgba(self.style.selection, 0.95),
                self.configuration.width,
                self.configuration.height,
            );
        }

        if let Some(rect) = self.status_bar.rect {
            push_ui_rect(
                &mut vertices,
                rect,
                mix_rgb(self.style.background, self.style.foreground, 0.045, 0.96),
                self.configuration.width,
                self.configuration.height,
            );
            push_ui_rect(
                &mut vertices,
                PaneRect::new(rect.x, rect.y, rect.width, 1),
                rgba(self.style.foreground, 0.14),
                self.configuration.width,
                self.configuration.height,
            );
        }

        vertices
    }

    fn recreate_surface(&mut self) -> Result<(), RenderError> {
        self.surface = self
            .instance
            .create_surface(self.window.clone())
            .map_err(|error| RenderError::new("recreate GPU surface", error))?;
        self.surface.configure(&self.device, &self.configuration);
        Ok(())
    }
}

fn create_ui_pipeline(device: &Device, format: wgpu::TextureFormat) -> RenderPipeline {
    let shader = device.create_shader_module(ShaderModuleDescriptor {
        label: Some("toyoterm UI shader"),
        source: ShaderSource::Wgsl(
            r#"
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(input.position, 0.0, 1.0);
    output.color = input.color;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}
"#
            .into(),
        ),
    });
    device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some("toyoterm UI pipeline"),
        layout: None,
        vertex: VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: PipelineCompilationOptions::default(),
            buffers: &[Some(UiVertex::LAYOUT)],
        },
        primitive: PrimitiveState {
            topology: PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: PipelineCompilationOptions::default(),
            targets: &[Some(ColorTargetState {
                format,
                blend: Some(BlendState::ALPHA_BLENDING),
                write_mask: ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn push_ui_rect(
    vertices: &mut Vec<UiVertex>,
    rect: PaneRect,
    color: [f32; 4],
    surface_width: u32,
    surface_height: u32,
) {
    if rect.width == 0 || rect.height == 0 || surface_width == 0 || surface_height == 0 {
        return;
    }
    let left = rect.x as f32 / surface_width as f32 * 2.0 - 1.0;
    let right = rect.x.saturating_add(rect.width) as f32 / surface_width as f32 * 2.0 - 1.0;
    let top = 1.0 - rect.y as f32 / surface_height as f32 * 2.0;
    let bottom = 1.0 - rect.y.saturating_add(rect.height) as f32 / surface_height as f32 * 2.0;
    for position in [
        [left, top],
        [left, bottom],
        [right, bottom],
        [left, top],
        [right, bottom],
        [right, top],
    ] {
        vertices.push(UiVertex { position, color });
    }
}

fn inset_rect(rect: PaneRect, left: u32, top: u32, right: u32, bottom: u32) -> PaneRect {
    PaneRect::new(
        rect.x.saturating_add(left),
        rect.y.saturating_add(top),
        rect.width.saturating_sub(left.saturating_add(right)),
        rect.height.saturating_sub(top.saturating_add(bottom)),
    )
}

fn rgba(rgb: [u8; 3], alpha: f32) -> [f32; 4] {
    [
        srgb_channel_to_linear(rgb[0]),
        srgb_channel_to_linear(rgb[1]),
        srgb_channel_to_linear(rgb[2]),
        alpha,
    ]
}

fn mix_rgb(base: [u8; 3], tint: [u8; 3], amount: f32, alpha: f32) -> [f32; 4] {
    let amount = amount.clamp(0.0, 1.0);
    let mix = |base: u8, tint: u8| {
        (f32::from(base) * (1.0 - amount) + f32::from(tint) * amount).round() as u8
    };
    rgba(
        [
            mix(base[0], tint[0]),
            mix(base[1], tint[1]),
            mix(base[2], tint[2]),
        ],
        alpha,
    )
}

fn srgb_channel_to_linear(channel: u8) -> f32 {
    let value = f32::from(channel) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn pane_text_placement(
    rect: PaneRect,
    layout: TextLayout,
    cursor: CursorState,
    cursor_x: f32,
) -> PaneTextPlacement {
    let text_left = rect.x as f32 + layout.horizontal_padding;
    let text_top = rect.y as f32 + layout.vertical_padding;
    PaneTextPlacement {
        bounds: rect,
        text_left,
        text_top,
        cursor_left: text_left + cursor_x,
        cursor_top: text_top + f32::from(cursor.row) * layout.line_height,
    }
}

fn surface_resize(size: PhysicalSize<u32>) -> SurfaceResize {
    if size.width == 0 || size.height == 0 {
        SurfaceResize::Suspend
    } else {
        SurfaceResize::Configure {
            width: size.width,
            height: size.height,
        }
    }
}

fn surface_recovery_action(status: &CurrentSurfaceTexture) -> Option<SurfaceRecoveryAction> {
    match status {
        CurrentSurfaceTexture::Success(_) | CurrentSurfaceTexture::Suboptimal(_) => None,
        CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => {
            Some(SurfaceRecoveryAction::Skip)
        }
        CurrentSurfaceTexture::Outdated => Some(SurfaceRecoveryAction::Reconfigure),
        CurrentSurfaceTexture::Lost => Some(SurfaceRecoveryAction::Recreate),
        CurrentSurfaceTexture::Validation => Some(SurfaceRecoveryAction::Fail),
    }
}

fn pane_bounds(rect: PaneRect) -> TextBounds {
    TextBounds {
        left: rect.x.min(i32::MAX as u32) as i32,
        top: rect.y.min(i32::MAX as u32) as i32,
        right: rect.x.saturating_add(rect.width).min(i32::MAX as u32) as i32,
        bottom: rect.y.saturating_add(rect.height).min(i32::MAX as u32) as i32,
    }
}

fn selection_text(snapshot: &TerminalSnapshot) -> String {
    if snapshot.selection.is_empty() {
        return String::new();
    }

    let mut text = String::new();
    for row in 0..snapshot.rows {
        if row > 0 {
            text.push('\n');
        }
        let Some(span) = snapshot.selection.iter().find(|span| span.row == row) else {
            continue;
        };
        text.extend(std::iter::repeat_n(' ', usize::from(span.start_column)));
        let width = span.end_column.saturating_sub(span.start_column) + 1;
        text.extend(std::iter::repeat_n('█', usize::from(width)));
    }
    text
}

fn terminal_cursor_x(
    buffer: &Buffer,
    snapshot: &TerminalSnapshot,
    cursor: CursorState,
) -> Option<f32> {
    let text_cursor = TextCursor::new(
        usize::from(cursor.row),
        terminal_cursor_byte_index(snapshot, cursor),
    );
    buffer
        .layout_runs()
        .find(|run| run.line_i == usize::from(cursor.row))
        .and_then(|run| run.cursor_position(&text_cursor))
}

fn terminal_cursor_byte_index(snapshot: &TerminalSnapshot, cursor: CursorState) -> usize {
    let Some(cells) = snapshot.cells.get(usize::from(cursor.row)) else {
        return usize::from(cursor.column);
    };
    let mut byte_index = 0;
    let mut column = 0;
    for cell in cells {
        if cursor.column <= cell.column {
            return byte_index + usize::from(cursor.column.saturating_sub(column));
        }
        byte_index += usize::from(cell.column.saturating_sub(column));
        let cell_end = cell.column.saturating_add(u16::from(cell.width.max(1)));
        if cursor.column < cell_end {
            return byte_index;
        }
        byte_index += cell.text.len();
        column = cell_end;
    }
    byte_index + usize::from(cursor.column.saturating_sub(column))
}

fn terminal_backgrounds(
    snapshot: &TerminalSnapshot,
    pane: PaneRect,
    layout: TextLayout,
    default_background: [u8; 3],
    default_foreground: [u8; 3],
    ansi: &[[u8; 3]; 16],
) -> Vec<(PaneRect, [u8; 3])> {
    let pane_right = pane.x.saturating_add(pane.width);
    let pane_bottom = pane.y.saturating_add(pane.height);
    let origin_x = pane.x as f32 + layout.horizontal_padding;
    let origin_y = pane.y as f32 + layout.vertical_padding;
    let mut backgrounds = Vec::new();

    for (row, cells) in snapshot.cells.iter().enumerate() {
        let top = (origin_y + row as f32 * layout.line_height)
            .floor()
            .max(0.0) as u32;
        let bottom = (origin_y + (row + 1) as f32 * layout.line_height)
            .ceil()
            .max(0.0) as u32;
        if top >= pane_bottom {
            break;
        }
        for cell in cells {
            let color = if cell.attributes.inverse {
                resolve_cell_color(cell.attributes.foreground, default_foreground, ansi)
            } else {
                resolve_cell_color(cell.attributes.background, default_background, ansi)
            };
            if color == default_background {
                continue;
            }
            let left = (origin_x + f32::from(cell.column) * layout.cell_width)
                .floor()
                .max(0.0) as u32;
            let right = (origin_x
                + f32::from(cell.column.saturating_add(u16::from(cell.width.max(1))))
                    * layout.cell_width)
                .ceil()
                .max(0.0) as u32;
            let left = left.max(pane.x);
            let top = top.max(pane.y);
            let right = right.min(pane_right);
            let bottom = bottom.min(pane_bottom);
            if right > left && bottom > top {
                backgrounds.push((PaneRect::new(left, top, right - left, bottom - top), color));
            }
        }
    }
    backgrounds
}

fn terminal_rich_text<'a>(
    snapshot: &TerminalSnapshot,
    cursor: Option<CursorState>,
    font_family: &'a str,
    font_weight: u16,
    default_foreground: [u8; 3],
    default_background: [u8; 3],
    ansi: &[[u8; 3]; 16],
) -> Vec<(String, Attrs<'a>)> {
    if snapshot.cells.is_empty() {
        return Vec::new();
    }

    let default_attrs = Attrs::new()
        .family(Family::Name(font_family))
        .weight(Weight(font_weight));
    let mut spans = Vec::new();
    for row in 0..snapshot.rows {
        if row > 0 {
            spans.push(("\n".to_owned(), default_attrs.clone()));
        }
        let Some(cells) = snapshot.cells.get(usize::from(row)) else {
            continue;
        };
        let mut column = 0;
        for cell in cells {
            if cell.column > column {
                spans.push((
                    " ".repeat(usize::from(cell.column - column)),
                    default_attrs.clone(),
                ));
            }
            spans.push((
                cell.text.clone(),
                glyph_attrs(
                    cell.attributes,
                    font_family,
                    font_weight,
                    default_foreground,
                    default_background,
                    ansi,
                ),
            ));
            column = cell.column.saturating_add(u16::from(cell.width.max(1)));
        }
        if let Some(cursor) = cursor.filter(|cursor| cursor.row == row)
            && cursor.column > column
        {
            spans.push((
                " ".repeat(usize::from(cursor.column - column)),
                default_attrs.clone(),
            ));
        }
    }
    spans
}

fn glyph_attrs<'a>(
    attributes: CellAttributes,
    font_family: &'a str,
    font_weight: u16,
    default_foreground: [u8; 3],
    default_background: [u8; 3],
    ansi: &[[u8; 3]; 16],
) -> Attrs<'a> {
    let foreground = if attributes.inverse {
        resolve_cell_color(attributes.background, default_background, ansi)
    } else {
        resolve_cell_color(attributes.foreground, default_foreground, ansi)
    };
    let alpha = if attributes.hidden {
        0
    } else if attributes.dim {
        150
    } else {
        255
    };
    let mut attrs = Attrs::new()
        .family(Family::Name(font_family))
        .weight(Weight(font_weight))
        .color(glyph_color(foreground, alpha));
    if attributes.bold {
        attrs = attrs.weight(Weight(font_weight.saturating_add(300).min(1000)));
    }
    if attributes.italic {
        attrs = attrs.style(Style::Italic);
    }
    if attributes.underline {
        attrs = attrs.underline(glyphon::cosmic_text::UnderlineStyle::Single);
    }
    if attributes.strikethrough {
        attrs = attrs.strikethrough();
    }
    attrs
}

fn resolve_cell_color(color: CellColor, default: [u8; 3], ansi: &[[u8; 3]; 16]) -> [u8; 3] {
    match color {
        CellColor::Default => default,
        CellColor::Rgb(red, green, blue) => [red, green, blue],
        CellColor::Indexed(index) => xterm_color(index, ansi),
    }
}

const fn default_ansi_palette() -> [[u8; 3]; 16] {
    [
        [0, 0, 0],
        [205, 0, 0],
        [0, 205, 0],
        [205, 205, 0],
        [0, 0, 238],
        [205, 0, 205],
        [0, 205, 205],
        [229, 229, 229],
        [127, 127, 127],
        [255, 0, 0],
        [0, 255, 0],
        [255, 255, 0],
        [92, 92, 255],
        [255, 0, 255],
        [0, 255, 255],
        [255, 255, 255],
    ]
}

fn xterm_color(index: u8, ansi: &[[u8; 3]; 16]) -> [u8; 3] {
    match index {
        0..=15 => ansi[usize::from(index)],
        16..=231 => {
            let index = index - 16;
            let component = |value: u8| if value == 0 { 0 } else { value * 40 + 55 };
            [
                component(index / 36),
                component(index / 6 % 6),
                component(index % 6),
            ]
        }
        232..=255 => {
            let level = (index - 232) * 10 + 8;
            [level, level, level]
        }
    }
}

fn parse_rgb(value: &str) -> Result<[u8; 3], RenderError> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 {
        return Err(RenderError::new(
            "parse color",
            format!("expected #RRGGBB, got `{value}`"),
        ));
    }
    let parse = |range| {
        u8::from_str_radix(&hex[range], 16)
            .map_err(|_| RenderError::new("parse color", format!("invalid color `{value}`")))
    };
    Ok([parse(0..2)?, parse(2..4)?, parse(4..6)?])
}

fn clear_color(style: &RenderStyle, alpha_mode: CompositeAlphaMode) -> Color {
    let alpha = f64::from(style.opacity.clamp(0.0, 1.0));
    let multiplier = if alpha_mode == CompositeAlphaMode::PreMultiplied {
        alpha
    } else {
        1.0
    };
    Color {
        r: f64::from(srgb_channel_to_linear(style.background[0])) * multiplier,
        g: f64::from(srgb_channel_to_linear(style.background[1])) * multiplier,
        b: f64::from(srgb_channel_to_linear(style.background[2])) * multiplier,
        a: alpha,
    }
}

fn preferred_alpha_mode(supported: &[CompositeAlphaMode], opacity: f32) -> CompositeAlphaMode {
    let preferences: &[CompositeAlphaMode] = if opacity < 1.0 {
        &[
            CompositeAlphaMode::PostMultiplied,
            CompositeAlphaMode::PreMultiplied,
            CompositeAlphaMode::Inherit,
        ]
    } else {
        &[CompositeAlphaMode::Opaque, CompositeAlphaMode::Inherit]
    };
    preferences
        .iter()
        .copied()
        .find(|mode| supported.contains(mode))
        .unwrap_or(CompositeAlphaMode::Auto)
}

fn glyph_color(color: [u8; 3], alpha: u8) -> GlyphColor {
    GlyphColor::rgba(color[0], color[1], color[2], alpha)
}

#[cfg(test)]
mod tests {
    use super::*;
    use toyoterm_terminal::{AlacrittyTerminalBackend, SelectionSpan, TerminalBackend};

    #[test]
    fn render_plan_matches_snapshot() {
        let placement = pane_text_placement(
            PaneRect::new(10, 20, 320, 180),
            TextLayout {
                font_size: 14.0,
                line_height: 18.0,
                cell_width: 9.0,
                horizontal_padding: 4.0,
                vertical_padding: 6.0,
            },
            CursorState {
                column: 3,
                row: 2,
                visible: true,
                shape: CursorShape::Beam,
            },
            27.0,
        );
        let style = RenderStyle {
            background: [32, 64, 128],
            opacity: 0.5,
            ..RenderStyle::default()
        };
        let background = clear_color(&style, CompositeAlphaMode::PreMultiplied);
        let terminal = TerminalSnapshot {
            columns: 8,
            rows: 3,
            lines: vec!["alpha".into(), "beta".into(), "gamma".into()],
            cells: Vec::new(),
            selection: vec![
                SelectionSpan {
                    row: 0,
                    start_column: 2,
                    end_column: 4,
                },
                SelectionSpan {
                    row: 1,
                    start_column: 0,
                    end_column: 1,
                },
            ],
        };
        let selection_mask = selection_text(&terminal).replace(' ', "·");
        let snapshot = format!(
            concat!(
                "background_rgba=({:.6},{:.6},{:.6},{:.6})\n",
                "glyph_origin=({:.1},{:.1})\n",
                "cursor_origin=({:.1},{:.1})\n",
                "clip=({},{},{},{})\n",
                "selection_mask:\n{}",
                "resize_minimized={:?}\n",
                "resize_restored={:?}\n"
            ),
            background.r,
            background.g,
            background.b,
            background.a,
            placement.text_left,
            placement.text_top,
            placement.cursor_left,
            placement.cursor_top,
            placement.bounds.x,
            placement.bounds.y,
            placement.bounds.width,
            placement.bounds.height,
            selection_mask,
            surface_resize(PhysicalSize::new(0, 180)),
            surface_resize(PhysicalSize::new(640, 360)),
        );
        assert_eq!(snapshot, include_str!("snapshots/render_plan.snap"));
    }

    #[test]
    fn styled_shell_prompt_cursor_uses_the_rendered_text_position() {
        let mut terminal = AlacrittyTerminalBackend::new(80, 4);
        terminal.advance(
            "\n\x1b[1;36mtoyoterm\x1b[0m \x1b[3;36mmaster\x1b[0m \
             \x1b[36m? \x1b[1m❯\x1b[0m "
                .as_bytes(),
        );
        let snapshot = terminal.snapshot();
        let cursor = terminal.cursor();
        let metrics = Metrics::new(14.0, 18.0);
        let mut font_system = FontSystem::new();
        let mut buffer = Buffer::new(&mut font_system, metrics);
        buffer.set_wrap(Wrap::None);
        let rich_text = terminal_rich_text(
            &snapshot,
            Some(cursor),
            "JetBrainsMono Nerd Font",
            400,
            [220, 225, 232],
            [9, 11, 14],
            &default_ansi_palette(),
        );
        let default_attrs = Attrs::new().family(Family::Name("JetBrainsMono Nerd Font"));
        buffer.set_rich_text(
            rich_text
                .iter()
                .map(|(text, attrs)| (text.as_str(), attrs.clone())),
            &default_attrs,
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut font_system, false);

        let rendered_cursor_x = terminal_cursor_x(&buffer, &snapshot, cursor)
            .expect("cursor row and byte index are laid out");
        let line_width = buffer
            .layout_runs()
            .find(|run| run.line_i == usize::from(cursor.row))
            .map(|run| run.line_w)
            .expect("cursor row is laid out");
        assert!((rendered_cursor_x - line_width).abs() < 0.01);
        assert!((rendered_cursor_x - f32::from(cursor.column) * 9.0).abs() > 1.0);
    }

    #[test]
    fn builds_a_cell_aligned_selection_mask() {
        let snapshot = TerminalSnapshot {
            columns: 8,
            rows: 3,
            lines: vec!["alpha".into(), "beta".into(), "gamma".into()],
            cells: Vec::new(),
            selection: vec![
                SelectionSpan {
                    row: 0,
                    start_column: 2,
                    end_column: 4,
                },
                SelectionSpan {
                    row: 1,
                    start_column: 0,
                    end_column: 1,
                },
            ],
        };
        assert_eq!(selection_text(&snapshot), "  ███\n██\n");
    }

    #[test]
    fn maps_xterm_palette_and_rich_cell_attributes() {
        let ansi = default_ansi_palette();
        assert_eq!(xterm_color(1, &ansi), [205, 0, 0]);
        assert_eq!(xterm_color(21, &ansi), [0, 0, 255]);
        assert_eq!(xterm_color(232, &ansi), [8, 8, 8]);

        let attrs = glyph_attrs(
            CellAttributes {
                foreground: CellColor::Rgb(1, 2, 3),
                bold: true,
                italic: true,
                dim: true,
                ..CellAttributes::default()
            },
            "monospace",
            400,
            [200, 200, 200],
            [0, 0, 0],
            &ansi,
        );
        assert_eq!(attrs.color_opt, Some(GlyphColor::rgba(1, 2, 3, 150)));
        assert_eq!(attrs.weight, Weight::BOLD);
        assert_eq!(attrs.style, Style::Italic);

        let inverse = glyph_attrs(
            CellAttributes {
                foreground: CellColor::Indexed(1),
                background: CellColor::Rgb(9, 8, 7),
                inverse: true,
                ..CellAttributes::default()
            },
            "monospace",
            400,
            [200, 200, 200],
            [0, 0, 0],
            &ansi,
        );
        assert_eq!(inverse.color_opt, Some(GlyphColor::rgba(9, 8, 7, 255)));
    }

    #[test]
    fn builds_background_rectangles_for_indexed_and_inverse_cells() {
        let ansi = default_ansi_palette();
        let snapshot = TerminalSnapshot {
            columns: 4,
            rows: 1,
            lines: vec!["ab".into()],
            cells: vec![vec![
                toyoterm_terminal::TerminalCell {
                    column: 0,
                    text: "a".into(),
                    width: 1,
                    attributes: CellAttributes {
                        background: CellColor::Indexed(196),
                        ..CellAttributes::default()
                    },
                },
                toyoterm_terminal::TerminalCell {
                    column: 1,
                    text: "b".into(),
                    width: 1,
                    attributes: CellAttributes {
                        foreground: CellColor::Indexed(21),
                        inverse: true,
                        ..CellAttributes::default()
                    },
                },
            ]],
            selection: Vec::new(),
        };
        let backgrounds = terminal_backgrounds(
            &snapshot,
            PaneRect::new(10, 20, 100, 40),
            TextLayout {
                font_size: 14.0,
                line_height: 18.0,
                cell_width: 9.0,
                horizontal_padding: 4.0,
                vertical_padding: 3.0,
            },
            [9, 11, 14],
            [220, 225, 232],
            &ansi,
        );
        assert_eq!(
            backgrounds,
            [
                (PaneRect::new(14, 23, 9, 18), [255, 0, 0]),
                (PaneRect::new(23, 23, 9, 18), [0, 0, 255]),
            ]
        );
    }

    #[test]
    fn parses_configured_render_colors() {
        let style = RenderStyle::from_hex(
            "JetBrains Mono",
            vec!["Noto Sans Mono CJK JP".into(), "Noto Color Emoji".into()],
            500,
            ["#112233", "aabbcc", "#ffffff", "#010203"],
            0.9,
        )
        .unwrap();
        assert_eq!(style.background, [0x11, 0x22, 0x33]);
        assert_eq!(style.font_weight, 500);
        assert_eq!(style.font_fallback.len(), 2);
        assert_eq!(style.foreground, [0xaa, 0xbb, 0xcc]);
        assert_eq!(style.opacity, 0.9);
        let mut custom_ansi = vec!["#000000".to_owned(); 16];
        custom_ansi[1] = "#123456".to_owned();
        let style = RenderStyle::from_hex_with_ansi(
            "mono",
            Vec::new(),
            400,
            ["#000000", "#ffffff", "#ffffff", "#333333"],
            &custom_ansi,
            1.0,
        )
        .unwrap();
        assert_eq!(style.ansi[1], [0x12, 0x34, 0x56]);
        let error = RenderStyle::from_hex(
            "mono",
            Vec::new(),
            400,
            ["bad", "#fff", "#fff", "#fff"],
            1.0,
        )
        .unwrap_err();
        assert_eq!(error.operation(), "parse color");
        assert!(error.message().contains("expected #RRGGBB"));
    }

    #[test]
    fn configured_font_fallback_preserves_user_priority() {
        let fallback =
            ConfiguredFallback::new(&["CJK First".to_owned(), "Emoji Second".to_owned()]);
        assert_eq!(
            &fallback.common_fallback()[..2],
            &["CJK First", "Emoji Second"]
        );
        assert_eq!(
            fallback.script_fallback(Script::Han, "ja-JP"),
            &["CJK First", "Emoji Second"]
        );
    }

    #[test]
    fn device_loss_is_reported_once_for_renderer_recovery() {
        let state = DeviceLossState::default();
        let callback_state = state.clone();
        assert!(!state.take_lost());

        callback_state.mark_lost();
        assert!(state.take_lost());
        assert!(!state.take_lost());
    }

    #[test]
    fn classifies_recoverable_surface_failures() {
        assert_eq!(
            surface_recovery_action(&CurrentSurfaceTexture::Timeout),
            Some(SurfaceRecoveryAction::Skip)
        );
        assert_eq!(
            surface_recovery_action(&CurrentSurfaceTexture::Occluded),
            Some(SurfaceRecoveryAction::Skip)
        );
        assert_eq!(
            surface_recovery_action(&CurrentSurfaceTexture::Outdated),
            Some(SurfaceRecoveryAction::Reconfigure)
        );
        assert_eq!(
            surface_recovery_action(&CurrentSurfaceTexture::Lost),
            Some(SurfaceRecoveryAction::Recreate)
        );
        assert_eq!(
            surface_recovery_action(&CurrentSurfaceTexture::Validation),
            Some(SurfaceRecoveryAction::Fail)
        );
    }

    #[test]
    fn selects_a_supported_transparency_mode() {
        let supported = [
            CompositeAlphaMode::Opaque,
            CompositeAlphaMode::PreMultiplied,
        ];
        assert_eq!(
            preferred_alpha_mode(&supported, 0.8),
            CompositeAlphaMode::PreMultiplied
        );
        assert_eq!(
            preferred_alpha_mode(&supported, 1.0),
            CompositeAlphaMode::Opaque
        );
    }

    #[test]
    fn converts_configured_srgb_colors_to_linear_gpu_values() {
        let converted = rgba([0x1a, 0x1b, 0x26], 1.0);
        assert!((converted[0] - 0.0103298).abs() < 0.000001);
        assert!((converted[1] - 0.0109601).abs() < 0.000001);
        assert!((converted[2] - 0.0193824).abs() < 0.000001);
        assert_eq!(converted[3], 1.0);

        assert_eq!(rgba([0, 0, 0], 0.5), [0.0, 0.0, 0.0, 0.5]);
        assert_eq!(rgba([255, 255, 255], 1.0), [1.0, 1.0, 1.0, 1.0]);
    }
}
