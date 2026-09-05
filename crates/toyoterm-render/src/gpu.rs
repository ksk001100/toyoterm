use super::*;

#[derive(Clone, Debug, Default)]
pub(super) struct DeviceLossState(Arc<AtomicBool>);

impl DeviceLossState {
    pub(super) fn mark_lost(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub(super) fn take_lost(&self) -> bool {
        self.0.swap(false, Ordering::AcqRel)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PaneTextPlacement {
    pub(super) bounds: PaneRect,
    pub(super) text_left: f32,
    pub(super) text_top: f32,
    pub(super) cursor_left: f32,
    pub(super) cursor_top: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SurfaceResize {
    Suspend,
    Configure { width: u32, height: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SurfaceRecoveryAction {
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
    pub(super) fn new(operation: &'static str, error: impl fmt::Display) -> Self {
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
    background: Option<background::GpuBackground>,
    panes: HashMap<PaneId, PaneBuffers>,
    tabs: HashMap<TabId, TabBuffer>,
    workspaces: HashMap<WorkspaceId, TabBuffer>,
    search: OverlayBuffer,
    status_bars: Vec<StatusBarBuffer>,
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

struct StatusBarBuffer {
    edge: StatusBarEdge,
    rect: PaneRect,
    sections: Vec<Buffer>,
}

struct ConfigErrorBuffers {
    title: Buffer,
    message: Buffer,
    open_log: Buffer,
    dismiss: Buffer,
    layout: Option<ConfigErrorRenderLayout>,
}

#[derive(Clone, Copy)]
struct ConfigErrorRenderLayout {
    notice: PaneRect,
    message_top: f32,
    open_log: PaneRect,
    dismiss: PaneRect,
}

struct PaneBuffers {
    text: Buffer,
    cell_runs: Vec<CellRunBuffer>,
    cursor_glyph: Buffer,
    badge: Buffer,
    has_badge: bool,
    layout: TextLayout,
    cursor: CursorState,
    cursor_x: f32,
    rect: PaneRect,
    active: bool,
    zoomed: bool,
    backgrounds: Vec<(PaneRect, [u8; 3])>,
    selection_highlights: Vec<PaneRect>,
    search_highlights: Vec<(PaneRect, bool)>,
}

struct CellRunBuffer {
    text: Buffer,
    column: u16,
    row: u16,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct UiVertex {
    pub(super) position: [f32; 2],
    pub(super) color: [f32; 4],
}

impl UiVertex {
    pub(super) const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
    };
}

pub(super) struct ConfiguredFallback {
    pub(super) configured: Vec<&'static str>,
    pub(super) common: Vec<&'static str>,
    pub(super) platform: PlatformFallback,
}

impl ConfiguredFallback {
    pub(super) fn new(families: &[String]) -> Self {
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

pub(super) fn intern_font_family(family: &str) -> &'static str {
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

pub(super) fn resolve_font_family(family: &str) -> Family<'_> {
    match family.to_ascii_lowercase().as_str() {
        "monospace" => Family::Monospace,
        "sans-serif" | "sans serif" => Family::SansSerif,
        "serif" => Family::Serif,
        "cursive" => Family::Cursive,
        "fantasy" => Family::Fantasy,
        _ => Family::Name(family),
    }
}

pub(super) fn configured_font_system(fallback: &[String]) -> FontSystem {
    let locale = sys_locale::get_locale().unwrap_or_else(|| "en-US".to_owned());
    let mut database = glyphon::fontdb::Database::new();
    database.load_system_fonts();
    #[cfg(windows)]
    database.set_monospace_family("Consolas");
    #[cfg(target_os = "macos")]
    database.set_monospace_family("Menlo");
    #[cfg(not(any(windows, target_os = "macos")))]
    database.set_monospace_family("Noto Sans Mono");
    FontSystem::new_with_locale_and_db_and_fallback(
        locale,
        database,
        ConfiguredFallback::new(fallback),
    )
}

impl PaneBuffers {
    pub(super) fn new(font_system: &mut FontSystem, metrics: Metrics, layout: TextLayout) -> Self {
        let mut buffer = || {
            let mut buffer = Buffer::new(font_system, metrics);
            buffer.set_wrap(Wrap::None);
            buffer
        };
        Self {
            text: buffer(),
            cell_runs: Vec::new(),
            cursor_glyph: buffer(),
            badge: buffer(),
            has_badge: false,
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
            zoomed: false,
            backgrounds: Vec::new(),
            selection_highlights: Vec::new(),
            search_highlights: Vec::new(),
        }
    }
}

impl GpuRenderer {
    pub async fn new(window: Arc<Window>, style: RenderStyle) -> Result<Self, RenderError> {
        tracing::debug!(target: "toyoterm::render", "initialize GPU renderer");
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);
        let instance = Instance::new(renderer_instance_descriptor());
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
        configuration.alpha_mode = preferred_alpha_mode(&supported_alpha_modes, style.opacity);
        configuration.desired_maximum_frame_latency = 1;
        surface.configure(&device, &configuration);
        tracing::info!(
            target: "toyoterm::render",
            width,
            height,
            format = ?configuration.format,
            alpha_modes = ?supported_alpha_modes,
            alpha_mode = ?configuration.alpha_mode,
            opacity = style.opacity,
            backend = ?adapter.get_info().backend,
            "GPU renderer initialized"
        );

        let mut font_system = configured_font_system(&style.font_fallback);
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
        let mut search_text = Buffer::new(&mut font_system, Metrics::new(14.0, 18.0));
        search_text.set_wrap(Wrap::None);
        let mut config_error_buffer = || {
            let mut buffer = Buffer::new(&mut font_system, Metrics::new(14.0, 18.0));
            buffer.set_wrap(Wrap::None);
            buffer
        };
        let config_error = ConfigErrorBuffers {
            title: config_error_buffer(),
            message: config_error_buffer(),
            open_log: config_error_buffer(),
            dismiss: config_error_buffer(),
            layout: None,
        };
        let background = style.background_image.as_ref().map(|image| {
            background::GpuBackground::new(&device, &queue, configuration.format, image)
        });
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
            background,
            panes: HashMap::new(),
            tabs: HashMap::new(),
            workspaces: HashMap::new(),
            search: OverlayBuffer {
                text: search_text,
                rect: None,
            },
            status_bars: Vec::new(),
            config_error,
            preedit,
            has_preedit: false,
            style,
        })
    }

    pub fn set_style(&mut self, style: RenderStyle) {
        let same_image = match (&self.style.background_image, &style.background_image) {
            (Some(old), Some(new)) => {
                old.width == new.width
                    && old.height == new.height
                    && Arc::ptr_eq(&old.rgba, &new.rgba)
            }
            (None, None) => true,
            _ => false,
        };
        if !same_image {
            self.background = style.background_image.as_ref().map(|image| {
                background::GpuBackground::new(
                    &self.device,
                    &self.queue,
                    self.configuration.format,
                    image,
                )
            });
        }
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
        tracing::debug!(
            target: "toyoterm::render",
            alpha_mode = ?self.configuration.alpha_mode,
            opacity = self.style.opacity,
            "renderer style updated"
        );
    }

    /// Returns the logical advance of one cell for the active terminal font.
    ///
    /// Cell geometry must come from the resolved font rather than a platform-
    /// independent constant: the generic `monospace` family resolves to fonts
    /// with different advances on Windows, macOS, and Linux.
    pub fn terminal_cell_width(&mut self, font_size: f32) -> f32 {
        measure_cell_width(
            &mut self.font_system,
            &self.style.font_family,
            self.style.font_weight,
            font_size,
        )
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

        let mut search_text = Buffer::new(&mut self.font_system, Metrics::new(14.0, 18.0));
        search_text.set_wrap(Wrap::None);
        self.search = OverlayBuffer {
            text: search_text,
            rect: None,
        };
        self.status_bars.clear();

        let mut buffer = || {
            let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(14.0, 18.0));
            buffer.set_wrap(Wrap::None);
            buffer
        };
        self.config_error = ConfigErrorBuffers {
            title: buffer(),
            message: buffer(),
            open_log: buffer(),
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
            let use_cell_runs = pane
                .snapshot
                .cells
                .iter()
                .flatten()
                .any(|cell| !cell.text.is_ascii());
            let default_attrs = Attrs::new()
                .family(resolve_font_family(&font_family))
                .weight(Weight(font_weight));
            let rich_text = if use_cell_runs {
                Vec::new()
            } else {
                terminal_rich_text(
                    pane.snapshot,
                    Some(pane.cursor),
                    &font_family,
                    font_weight,
                    foreground,
                    background,
                    &ansi,
                )
            };
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
            buffers.zoomed = pane.zoomed;
            buffers.backgrounds = terminal_backgrounds(
                pane.snapshot,
                pane.rect,
                layout,
                background,
                foreground,
                &ansi,
                self.style.background_image.is_some(),
            );
            buffers.selection_highlights =
                selection_highlight_rects(pane.snapshot, pane.rect, layout);
            buffers.search_highlights = search_highlight_rects(pane.snapshot, pane.rect, layout);
            if !use_cell_runs {
                let content_width = pane
                    .rect
                    .width
                    .saturating_sub((layout.horizontal_padding * 2.0) as u32)
                    as f32;
                let content_height = pane
                    .rect
                    .height
                    .saturating_sub((layout.vertical_padding * 2.0) as u32)
                    as f32;
                // A terminal is a fixed grid even when the selected font (or a
                // platform-specific fallback) reports a different advance.  In
                // particular, Windows' generic monospace resolution and DPI
                // scaling can otherwise accumulate enough fractional advance to
                // visibly break columnar output such as `ls` and move the cursor
                // away from its logical cell.
                buffers.text.set_monospace_width(Some(layout.cell_width));
                buffers.text.set_metrics_and_size(
                    metrics,
                    Some(content_width),
                    Some(content_height),
                );
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
            }
            buffers.cursor_x = pane_cursor_x(
                &buffers.text,
                pane.snapshot,
                pane.cursor,
                layout.cell_width,
                pane.cursor_uses_grid || use_cell_runs,
            );

            let cell_runs = if use_cell_runs {
                terminal_cell_runs(pane.snapshot)
            } else {
                Vec::new()
            };
            buffers.cell_runs.truncate(cell_runs.len());
            for (index, (row, cells)) in cell_runs.into_iter().enumerate() {
                if index == buffers.cell_runs.len() {
                    let mut text = Buffer::new(&mut self.font_system, metrics);
                    text.set_wrap(Wrap::None);
                    buffers.cell_runs.push(CellRunBuffer {
                        text,
                        column: 0,
                        row: 0,
                    });
                }
                let run = &mut buffers.cell_runs[index];
                run.column = cells[0].column;
                run.row = row;
                update_terminal_cell_buffer(
                    &mut run.text,
                    &mut self.font_system,
                    cells,
                    layout,
                    &self.style,
                );
            }

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
                    .family(resolve_font_family(&self.style.font_family))
                    .weight(Weight(self.style.font_weight)),
                Shaping::Advanced,
                None,
            );
            buffers
                .cursor_glyph
                .shape_until_scroll(&mut self.font_system, false);

            buffers.has_badge = pane.badge.is_some_and(|badge| !badge.is_empty());
            buffers.badge.set_metrics_and_size(
                Metrics::new(
                    (layout.font_size * 0.85).max(1.0),
                    layout.line_height.max(1.0),
                ),
                Some(
                    pane.rect
                        .width
                        .saturating_sub((layout.horizontal_padding * 2.0) as u32)
                        as f32,
                ),
                Some(layout.line_height),
            );
            buffers.badge.set_text(
                pane.badge.unwrap_or_default(),
                &Attrs::new()
                    .family(resolve_font_family(&self.style.font_family))
                    .weight(Weight::SEMIBOLD),
                Shaping::Advanced,
                None,
            );
            if let Some(line) = buffers.badge.lines.first_mut() {
                line.set_align(Some(Align::Right));
            }
            buffers
                .badge
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
                    .family(resolve_font_family(&self.style.font_family))
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
                    .family(resolve_font_family(&self.style.font_family))
                    .weight(Weight(self.style.font_weight)),
                Shaping::Advanced,
                None,
            );
            buffer.text.shape_until_scroll(&mut self.font_system, false);
        }
    }

    pub fn update_search(&mut self, search: Option<SearchRenderData<'_>>, layout: TextLayout) {
        let Some(search) = search else {
            self.search.rect = None;
            return;
        };
        self.search.rect = Some(search.rect);
        let metrics = Metrics::new(layout.font_size.max(1.0), layout.line_height.max(1.0));
        self.search.text.set_metrics_and_size(
            metrics,
            Some(search.rect.width.saturating_sub(24) as f32),
            Some(search.rect.height.saturating_sub(16) as f32),
        );
        self.search.text.set_text(
            search.text,
            &Attrs::new()
                .family(resolve_font_family(&self.style.font_family))
                .weight(Weight(self.style.font_weight)),
            Shaping::Advanced,
            None,
        );
        self.search
            .text
            .shape_until_scroll(&mut self.font_system, false);
    }

    pub fn update_status_bars(&mut self, statuses: &[StatusBarRenderData<'_>], layout: TextLayout) {
        let metrics = Metrics::new(
            (layout.font_size * 0.85).max(1.0),
            (layout.line_height * 0.85).max(1.0),
        );
        while self.status_bars.len() < statuses.len() {
            let mut sections = Vec::with_capacity(3);
            for _ in 0..3 {
                let mut text = Buffer::new(&mut self.font_system, metrics);
                text.set_wrap(Wrap::None);
                sections.push(text);
            }
            self.status_bars.push(StatusBarBuffer {
                edge: StatusBarEdge::Bottom,
                rect: PaneRect::default(),
                sections,
            });
        }
        self.status_bars.truncate(statuses.len());
        for (buffer, status) in self.status_bars.iter_mut().zip(statuses) {
            buffer.edge = status.edge;
            buffer.rect = status.rect;
            for (index, alignment) in [
                StatusBarAlignment::Left,
                StatusBarAlignment::Center,
                StatusBarAlignment::Right,
            ]
            .into_iter()
            .enumerate()
            {
                let text = status_bar_section_text(status.items, alignment);
                let section = &mut buffer.sections[index];
                section.set_metrics_and_size(
                    metrics,
                    Some(status.rect.width.saturating_sub(16) as f32),
                    Some(status.rect.height as f32),
                );
                section.set_text(
                    &text,
                    &Attrs::new()
                        .family(resolve_font_family(&self.style.font_family))
                        .weight(Weight(self.style.font_weight)),
                    Shaping::Advanced,
                    None,
                );
                if let Some(line) = section.lines.first_mut() {
                    line.set_align(Some(match alignment {
                        StatusBarAlignment::Left => Align::Left,
                        StatusBarAlignment::Center => Align::Center,
                        StatusBarAlignment::Right => Align::Right,
                    }));
                }
                section.shape_until_scroll(&mut self.font_system, false);
            }
        }
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
            dismiss: error.dismiss_rect,
        });
        let metrics = Metrics::new(layout.font_size.max(1.0), layout.line_height.max(1.0));
        let attrs = Attrs::new()
            .family(resolve_font_family(&self.style.font_family))
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
                .family(resolve_font_family(&self.style.font_family))
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
        if let Some(rect) = self.search.rect {
            text_areas.push(TextArea {
                buffer: &self.search.text,
                left: rect.x as f32 + 12.0,
                top: rect.y as f32 + 8.0,
                scale: 1.0,
                bounds: pane_bounds(rect),
                default_color: glyph_color(self.style.cursor, 255),
                custom_glyphs: &[],
            });
        }
        for status_bar in &self.status_bars {
            for section in &status_bar.sections {
                text_areas.push(TextArea {
                    buffer: section,
                    left: status_bar.rect.x as f32 + 8.0,
                    top: status_bar.rect.y as f32 + 2.0,
                    scale: 1.0,
                    bounds: pane_bounds(status_bar.rect),
                    default_color: glyph_color(self.style.foreground, 190),
                    custom_glyphs: &[],
                });
            }
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
            if pane.cell_runs.is_empty() {
                text_areas.push(TextArea {
                    buffer: &pane.text,
                    left: placement.text_left,
                    top: placement.text_top,
                    scale: 1.0,
                    bounds,
                    default_color: glyph_color(self.style.foreground, 255),
                    custom_glyphs: &[],
                });
            } else {
                for run in &pane.cell_runs {
                    text_areas.push(TextArea {
                        buffer: &run.text,
                        left: placement.text_left + f32::from(run.column) * pane.layout.cell_width,
                        top: placement.text_top + f32::from(run.row) * pane.layout.line_height,
                        scale: 1.0,
                        bounds,
                        default_color: glyph_color(self.style.foreground, 255),
                        custom_glyphs: &[],
                    });
                }
            }
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
            if pane.has_badge {
                text_areas.push(TextArea {
                    buffer: &pane.badge,
                    left: pane.rect.x as f32 + pane.layout.horizontal_padding,
                    top: pane.rect.y as f32 + pane.layout.vertical_padding,
                    scale: 1.0,
                    bounds: pane_bounds(pane.rect),
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
        if let Some(background) = &self.background {
            background.update(&self.queue, &self.style, &self.configuration);
        }
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
            if let Some(background) = &self.background {
                background.draw(&mut pass);
            }
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
                rgba(self.style.workspace_bar, 0.96),
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
                rgba(self.style.tab_bar, 0.98),
                self.configuration.width,
                self.configuration.height,
            );
        }
        for tab in self.tabs.values() {
            let fill = if tab.active {
                rgba(self.style.tab_active, 1.0)
            } else {
                rgba(self.style.tab_inactive, 0.96)
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
            for rect in &pane.selection_highlights {
                push_ui_rect(
                    &mut vertices,
                    *rect,
                    rgba(self.style.selection, 210.0 / 255.0),
                    self.configuration.width,
                    self.configuration.height,
                );
            }
            for (rect, active) in &pane.search_highlights {
                push_ui_rect(
                    &mut vertices,
                    *rect,
                    if *active {
                        rgba(self.style.search_match_active, 0.72)
                    } else {
                        rgba(self.style.search_match, 0.38)
                    },
                    self.configuration.width,
                    self.configuration.height,
                );
            }
        }

        for pane in self.panes.values().filter(|pane| pane.active) {
            let border_width = (f64::from(self.style.active_pane_border_width)
                * self.window.scale_factor())
            .round()
            .max(0.0) as u32;
            for rect in pane_border_rects(pane.rect, border_width) {
                push_ui_rect(
                    &mut vertices,
                    rect,
                    rgba(
                        if pane.zoomed {
                            self.style.zoomed_pane_border
                        } else {
                            self.style.pane_border
                        },
                        0.95,
                    ),
                    self.configuration.width,
                    self.configuration.height,
                );
            }
        }

        for status_bar in &self.status_bars {
            let rect = status_bar.rect;
            push_ui_rect(
                &mut vertices,
                rect,
                rgba(self.style.status_bar, 0.96),
                self.configuration.width,
                self.configuration.height,
            );
            let separator = match status_bar.edge {
                StatusBarEdge::Top => PaneRect::new(
                    rect.x,
                    rect.y.saturating_add(rect.height.saturating_sub(1)),
                    rect.width,
                    1,
                ),
                StatusBarEdge::Bottom => PaneRect::new(rect.x, rect.y, rect.width, 1),
            };
            push_ui_rect(
                &mut vertices,
                separator,
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

pub(super) fn renderer_instance_descriptor() -> wgpu::InstanceDescriptor {
    let descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    #[cfg(target_os = "windows")]
    {
        let mut descriptor = descriptor;
        // The window uses WS_EX_NOREDIRECTIONBITMAP. Pair it exclusively with
        // DirectComposition, not Vulkan WSI or DX12's opaque HWND swapchain.
        descriptor.backends = wgpu::Backends::DX12;
        descriptor.backend_options.dx12.presentation_system =
            wgpu::Dx12SwapchainKind::DxgiFromVisual;
        descriptor
    }
    #[cfg(not(target_os = "windows"))]
    descriptor
}

pub(super) fn measure_cell_width(
    font_system: &mut FontSystem,
    font_family: &str,
    font_weight: u16,
    font_size: f32,
) -> f32 {
    let font_size = font_size.max(1.0);
    let mut probe = Buffer::new(font_system, Metrics::new(font_size, font_size * 2.0));
    probe.set_wrap(Wrap::None);
    probe.set_text(
        "0",
        &Attrs::new()
            .family(resolve_font_family(font_family))
            .weight(Weight(font_weight)),
        Shaping::Advanced,
        None,
    );
    probe.shape_until_scroll(font_system, false);
    probe
        .layout_runs()
        .next()
        .map(|run| run.line_w)
        .filter(|width| width.is_finite() && *width >= 1.0)
        .unwrap_or(font_size * (9.0 / 14.0))
}
