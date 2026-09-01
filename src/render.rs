use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use glyphon::{
    Attrs, Buffer, Cache as GlyphCache, Color as GlyphColor, Family, FontSystem, Metrics,
    Resolution, Shaping, Style, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer,
    Viewport, Weight, Wrap,
};
use wgpu::{
    Color, CommandEncoderDescriptor, CompositeAlphaMode, CurrentSurfaceTexture, Device,
    DeviceDescriptor, Instance, LoadOp, Operations, Queue, RenderPassColorAttachment,
    RenderPassDescriptor, StoreOp, Surface, SurfaceConfiguration, TextureViewDescriptor,
};
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::{
    CellAttributes, CellColor, CursorShape, CursorState, PaneId, PaneRect, TabId, TerminalSnapshot,
    WorkspaceId,
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

#[derive(Clone, Debug, PartialEq)]
pub struct RenderStyle {
    pub font_family: String,
    pub background: [u8; 3],
    pub foreground: [u8; 3],
    pub cursor: [u8; 3],
    pub selection: [u8; 3],
    pub opacity: f32,
}

impl Default for RenderStyle {
    fn default() -> Self {
        Self {
            font_family: "monospace".into(),
            background: [9, 11, 14],
            foreground: [220, 225, 232],
            cursor: [245, 247, 250],
            selection: [55, 88, 145],
            opacity: 1.0,
        }
    }
}

impl RenderStyle {
    pub fn from_hex(
        font_family: impl Into<String>,
        background: &str,
        foreground: &str,
        cursor: &str,
        selection: &str,
        opacity: f32,
    ) -> Result<Self, RenderError> {
        Ok(Self {
            font_family: font_family.into(),
            background: parse_rgb(background)?,
            foreground: parse_rgb(foreground)?,
            cursor: parse_rgb(cursor)?,
            selection: parse_rgb(selection)?,
            opacity,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderOutcome {
    Presented,
    Skipped,
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
    configuration: SurfaceConfiguration,
    supported_alpha_modes: Vec<CompositeAlphaMode>,
    suspended: bool,
    clear_color: Color,
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    text_atlas: TextAtlas,
    text_renderer: TextRenderer,
    panes: HashMap<PaneId, PaneBuffers>,
    tabs: HashMap<TabId, TabBuffer>,
    workspaces: HashMap<WorkspaceId, TabBuffer>,
    preedit: Buffer,
    has_preedit: bool,
    style: RenderStyle,
}

struct TabBuffer {
    text: Buffer,
    rect: PaneRect,
    active: bool,
}

struct PaneBuffers {
    selection: Buffer,
    text: Buffer,
    cursor_glyph: Buffer,
    focus: Buffer,
    layout: TextLayout,
    cursor: CursorState,
    rect: PaneRect,
    active: bool,
    has_selection: bool,
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
            focus: buffer(),
            layout,
            cursor: CursorState {
                column: 0,
                row: 0,
                visible: false,
                shape: CursorShape::Block,
            },
            rect: PaneRect::default(),
            active: false,
            has_selection: false,
        }
    }
}

impl GpuRenderer {
    pub async fn new(window: Arc<Window>) -> Result<Self, RenderError> {
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
        let supported_alpha_modes = surface.get_capabilities(&adapter).alpha_modes;
        let mut configuration = surface
            .get_default_config(&adapter, width, height)
            .ok_or_else(|| RenderError::new("configure GPU surface", "unsupported surface"))?;
        configuration.desired_maximum_frame_latency = 1;
        surface.configure(&device, &configuration);

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
        let mut preedit = Buffer::new(&mut font_system, Metrics::new(14.0, 18.0));
        preedit.set_wrap(Wrap::None);
        let style = RenderStyle::default();
        let initial_alpha_mode = configuration.alpha_mode;
        Ok(Self {
            instance,
            window,
            surface,
            device,
            queue,
            configuration,
            supported_alpha_modes,
            suspended: size.width == 0 || size.height == 0,
            clear_color: clear_color(&style, initial_alpha_mode),
            font_system,
            swash_cache,
            viewport,
            text_atlas,
            text_renderer,
            panes: HashMap::new(),
            tabs: HashMap::new(),
            workspaces: HashMap::new(),
            preedit,
            has_preedit: false,
            style,
        })
    }

    pub fn set_style(&mut self, style: RenderStyle) {
        let alpha_mode = preferred_alpha_mode(&self.supported_alpha_modes, style.opacity);
        let alpha_mode_changed = self.configuration.alpha_mode != alpha_mode;
        self.configuration.alpha_mode = alpha_mode;
        self.clear_color = clear_color(&style, alpha_mode);
        self.style = style;
        if alpha_mode_changed && !self.suspended {
            self.surface.configure(&self.device, &self.configuration);
        }
    }

    pub fn update_panes(&mut self, panes: &[PaneRenderData<'_>], layout: TextLayout) {
        let active_panes = panes.iter().map(|pane| pane.pane).collect::<HashSet<_>>();
        self.panes.retain(|pane, _| active_panes.contains(pane));
        let metrics = Metrics::new(layout.font_size.max(1.0), layout.line_height.max(1.0));
        let font_family = self.style.font_family.clone();
        let foreground = self.style.foreground;
        let background = self.style.background;
        for pane in panes {
            let rich_text = terminal_rich_text(pane.snapshot, &font_family, foreground, background);
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
            let default_attrs = Attrs::new().family(Family::Name(&font_family));
            if rich_text.is_empty() {
                buffers.text.set_text(
                    &pane.snapshot.lines.join("\n"),
                    &default_attrs,
                    Shaping::Basic,
                    None,
                );
            } else {
                buffers.text.set_rich_text(
                    rich_text
                        .iter()
                        .map(|(text, attrs)| (text.as_str(), attrs.clone())),
                    &default_attrs,
                    Shaping::Basic,
                    None,
                );
            }
            buffers
                .text
                .shape_until_scroll(&mut self.font_system, false);

            buffers.selection.set_metrics_and_size(
                metrics,
                Some(content_width),
                Some(content_height),
            );
            buffers.selection.set_text(
                &selection_text(pane.snapshot),
                &Attrs::new().family(Family::Name(&self.style.font_family)),
                Shaping::Basic,
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
                &Attrs::new().family(Family::Name(&self.style.font_family)),
                Shaping::Basic,
                None,
            );
            buffers
                .cursor_glyph
                .shape_until_scroll(&mut self.font_system, false);

            let focus_width = (pane.rect.width as f32 / layout.cell_width.max(1.0)) as usize;
            buffers.focus.set_metrics_and_size(metrics, None, None);
            buffers.focus.set_text(
                &"━".repeat(focus_width),
                &Attrs::new().family(Family::Name(&self.style.font_family)),
                Shaping::Basic,
                None,
            );
            buffers
                .focus
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
                &Attrs::new().family(Family::Name(&self.style.font_family)),
                Shaping::Basic,
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
                &Attrs::new().family(Family::Name(&self.style.font_family)),
                Shaping::Basic,
                None,
            );
            buffer.text.shape_until_scroll(&mut self.font_system, false);
        }
    }

    pub fn update_preedit(&mut self, text: Option<&str>, layout: TextLayout) {
        let text = text.unwrap_or_default();
        self.has_preedit = !text.is_empty();
        let metrics = Metrics::new(layout.font_size.max(1.0), layout.line_height.max(1.0));
        self.preedit.set_metrics_and_size(metrics, None, None);
        self.preedit.set_text(
            text,
            &Attrs::new().family(Family::Name(&self.style.font_family)),
            Shaping::Advanced,
            None,
        );
        self.preedit
            .shape_until_scroll(&mut self.font_system, false);
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        self.suspended = size.width == 0 || size.height == 0;
        if self.suspended {
            return;
        }
        self.configuration.width = size.width;
        self.configuration.height = size.height;
        self.surface.configure(&self.device, &self.configuration);
    }

    pub fn size(&self) -> PhysicalSize<u32> {
        PhysicalSize::new(self.configuration.width, self.configuration.height)
    }

    pub fn render(&mut self) -> Result<RenderOutcome, RenderError> {
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
            Vec::with_capacity(self.panes.len() * 4 + self.tabs.len() + self.workspaces.len());
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
        for pane in self.panes.values() {
            let bounds = pane_bounds(pane.rect);
            let left = pane.rect.x as f32 + pane.layout.horizontal_padding;
            let top = pane.rect.y as f32 + pane.layout.vertical_padding;
            if pane.has_selection {
                text_areas.push(TextArea {
                    buffer: &pane.selection,
                    left,
                    top,
                    scale: 1.0,
                    bounds,
                    default_color: glyph_color(self.style.selection, 210),
                    custom_glyphs: &[],
                });
            }
            text_areas.push(TextArea {
                buffer: &pane.text,
                left,
                top,
                scale: 1.0,
                bounds,
                default_color: glyph_color(self.style.foreground, 255),
                custom_glyphs: &[],
            });
            if pane.active {
                text_areas.push(TextArea {
                    buffer: &pane.focus,
                    left: pane.rect.x as f32,
                    top: pane.rect.y as f32,
                    scale: 1.0,
                    bounds,
                    default_color: glyph_color(self.style.cursor, 190),
                    custom_glyphs: &[],
                });
            }
            if pane.active && pane.cursor.visible {
                text_areas.push(TextArea {
                    buffer: &pane.cursor_glyph,
                    left: left + f32::from(pane.cursor.column) * pane.layout.cell_width,
                    top: top + f32::from(pane.cursor.row) * pane.layout.line_height,
                    scale: 1.0,
                    bounds,
                    default_color: glyph_color(self.style.cursor, 255),
                    custom_glyphs: &[],
                });
            }
            if pane.active && self.has_preedit {
                text_areas.push(TextArea {
                    buffer: &self.preedit,
                    left: left + f32::from(pane.cursor.column) * pane.layout.cell_width,
                    top: top + f32::from(pane.cursor.row) * pane.layout.line_height,
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
            CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => {
                return Ok(RenderOutcome::Skipped);
            }
            CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.configuration);
                return Ok(RenderOutcome::Skipped);
            }
            CurrentSurfaceTexture::Lost => {
                self.recreate_surface()?;
                return Ok(RenderOutcome::Skipped);
            }
            CurrentSurfaceTexture::Validation => {
                return Err(RenderError::new(
                    "acquire GPU frame",
                    "surface validation failed",
                ));
            }
        };

        let view = frame.texture.create_view(&TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("toyoterm frame encoder"),
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
                label: Some("toyoterm clear pass"),
                color_attachments: &color_attachments,
                ..Default::default()
            });
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

    fn recreate_surface(&mut self) -> Result<(), RenderError> {
        self.surface = self
            .instance
            .create_surface(self.window.clone())
            .map_err(|error| RenderError::new("recreate GPU surface", error))?;
        self.surface.configure(&self.device, &self.configuration);
        Ok(())
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

fn terminal_rich_text<'a>(
    snapshot: &TerminalSnapshot,
    font_family: &'a str,
    default_foreground: [u8; 3],
    default_background: [u8; 3],
) -> Vec<(String, Attrs<'a>)> {
    if snapshot.cells.is_empty() {
        return Vec::new();
    }

    let default_attrs = Attrs::new().family(Family::Name(font_family));
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
                    default_foreground,
                    default_background,
                ),
            ));
            column = cell.column.saturating_add(u16::from(cell.width.max(1)));
        }
    }
    spans
}

fn glyph_attrs<'a>(
    attributes: CellAttributes,
    font_family: &'a str,
    default_foreground: [u8; 3],
    default_background: [u8; 3],
) -> Attrs<'a> {
    let foreground = if attributes.inverse {
        resolve_cell_color(attributes.background, default_background)
    } else {
        resolve_cell_color(attributes.foreground, default_foreground)
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
        .color(glyph_color(foreground, alpha));
    if attributes.bold {
        attrs = attrs.weight(Weight::BOLD);
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

fn resolve_cell_color(color: CellColor, default: [u8; 3]) -> [u8; 3] {
    match color {
        CellColor::Default => default,
        CellColor::Rgb(red, green, blue) => [red, green, blue],
        CellColor::Indexed(index) => xterm_color(index),
    }
}

fn xterm_color(index: u8) -> [u8; 3] {
    const ANSI: [[u8; 3]; 16] = [
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
    ];
    match index {
        0..=15 => ANSI[usize::from(index)],
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
        r: f64::from(style.background[0]) / 255.0 * multiplier,
        g: f64::from(style.background[1]) / 255.0 * multiplier,
        b: f64::from(style.background[2]) / 255.0 * multiplier,
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
    use crate::SelectionSpan;

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
        assert_eq!(xterm_color(1), [205, 0, 0]);
        assert_eq!(xterm_color(21), [0, 0, 255]);
        assert_eq!(xterm_color(232), [8, 8, 8]);

        let attrs = glyph_attrs(
            CellAttributes {
                foreground: CellColor::Rgb(1, 2, 3),
                bold: true,
                italic: true,
                dim: true,
                ..CellAttributes::default()
            },
            "monospace",
            [200, 200, 200],
            [0, 0, 0],
        );
        assert_eq!(attrs.color_opt, Some(GlyphColor::rgba(1, 2, 3, 150)));
        assert_eq!(attrs.weight, Weight::BOLD);
        assert_eq!(attrs.style, Style::Italic);
    }

    #[test]
    fn parses_configured_render_colors() {
        let style = RenderStyle::from_hex(
            "JetBrains Mono",
            "#112233",
            "aabbcc",
            "#ffffff",
            "#010203",
            0.9,
        )
        .unwrap();
        assert_eq!(style.background, [0x11, 0x22, 0x33]);
        assert_eq!(style.foreground, [0xaa, 0xbb, 0xcc]);
        assert_eq!(style.opacity, 0.9);
        assert!(RenderStyle::from_hex("mono", "bad", "#fff", "#fff", "#fff", 1.0).is_err());
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
}
