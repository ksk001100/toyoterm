use std::fmt;
use std::sync::Arc;

use glyphon::{
    Attrs, Buffer, Cache as GlyphCache, Color as GlyphColor, Family, FontSystem, Metrics,
    Resolution, Shaping, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Wrap,
};
use wgpu::{
    Color, CommandEncoderDescriptor, CurrentSurfaceTexture, Device, DeviceDescriptor, Instance,
    LoadOp, Operations, Queue, RenderPassColorAttachment, RenderPassDescriptor, StoreOp, Surface,
    SurfaceConfiguration, TextureViewDescriptor,
};
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::{CursorShape, CursorState, TerminalSnapshot};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextLayout {
    pub font_size: f32,
    pub line_height: f32,
    pub cell_width: f32,
    pub horizontal_padding: f32,
    pub vertical_padding: f32,
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
    suspended: bool,
    clear_color: Color,
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    text_atlas: TextAtlas,
    text_renderer: TextRenderer,
    text_buffer: Buffer,
    cursor_buffer: Buffer,
    text_layout: TextLayout,
    cursor: CursorState,
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
        let default_layout = TextLayout {
            font_size: 14.0,
            line_height: 18.0,
            cell_width: 9.0,
            horizontal_padding: 8.0,
            vertical_padding: 8.0,
        };
        let metrics = Metrics::new(default_layout.font_size, default_layout.line_height);
        let mut text_buffer = Buffer::new(&mut font_system, metrics);
        text_buffer.set_wrap(Wrap::None);
        let mut cursor_buffer = Buffer::new(&mut font_system, metrics);
        cursor_buffer.set_wrap(Wrap::None);

        Ok(Self {
            instance,
            window,
            surface,
            device,
            queue,
            configuration,
            suspended: size.width == 0 || size.height == 0,
            clear_color: Color {
                r: 0.035,
                g: 0.043,
                b: 0.055,
                a: 1.0,
            },
            font_system,
            swash_cache,
            viewport,
            text_atlas,
            text_renderer,
            text_buffer,
            cursor_buffer,
            text_layout: default_layout,
            cursor: CursorState {
                column: 0,
                row: 0,
                visible: false,
                shape: CursorShape::Block,
            },
        })
    }

    pub fn update_terminal(
        &mut self,
        snapshot: &TerminalSnapshot,
        cursor: CursorState,
        layout: TextLayout,
    ) {
        self.text_layout = layout;
        self.cursor = cursor;
        let metrics = Metrics::new(layout.font_size.max(1.0), layout.line_height.max(1.0));
        let content_width =
            self.configuration
                .width
                .saturating_sub((layout.horizontal_padding * 2.0) as u32) as f32;
        let content_height =
            self.configuration
                .height
                .saturating_sub((layout.vertical_padding * 2.0) as u32) as f32;
        self.text_buffer
            .set_metrics_and_size(metrics, Some(content_width), Some(content_height));
        self.text_buffer.set_text(
            &snapshot.lines.join("\n"),
            &Attrs::new().family(Family::Monospace),
            Shaping::Basic,
            None,
        );
        self.text_buffer
            .shape_until_scroll(&mut self.font_system, false);

        self.cursor_buffer.set_metrics_and_size(metrics, None, None);
        self.cursor_buffer.set_text(
            match cursor.shape {
                CursorShape::Block => "█",
                CursorShape::Beam => "▏",
                CursorShape::Underline => "▁",
            },
            &Attrs::new().family(Family::Monospace),
            Shaping::Basic,
            None,
        );
        self.cursor_buffer
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
        let bounds = TextBounds {
            left: 0,
            top: 0,
            right: self.configuration.width.min(i32::MAX as u32) as i32,
            bottom: self.configuration.height.min(i32::MAX as u32) as i32,
        };
        let cursor_left = self.text_layout.horizontal_padding
            + f32::from(self.cursor.column) * self.text_layout.cell_width;
        let cursor_top = self.text_layout.vertical_padding
            + f32::from(self.cursor.row) * self.text_layout.line_height;
        let mut text_areas = vec![TextArea {
            buffer: &self.text_buffer,
            left: self.text_layout.horizontal_padding,
            top: self.text_layout.vertical_padding,
            scale: 1.0,
            bounds,
            default_color: GlyphColor::rgb(220, 225, 232),
            custom_glyphs: &[],
        }];
        if self.cursor.visible {
            text_areas.push(TextArea {
                buffer: &self.cursor_buffer,
                left: cursor_left,
                top: cursor_top,
                scale: 1.0,
                bounds,
                default_color: GlyphColor::rgb(245, 247, 250),
                custom_glyphs: &[],
            });
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
