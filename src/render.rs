use std::fmt;
use std::sync::Arc;

use glyphon::{
    Attrs, Buffer, Cache as GlyphCache, Color as GlyphColor, Family, FontSystem, Metrics,
    Resolution, Shaping, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Wrap,
};
use wgpu::{
    Color, CommandEncoderDescriptor, CompositeAlphaMode, CurrentSurfaceTexture, Device,
    DeviceDescriptor, Instance, LoadOp, Operations, Queue, RenderPassColorAttachment,
    RenderPassDescriptor, StoreOp, Surface, SurfaceConfiguration, TextureViewDescriptor,
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
    selection_buffer: Buffer,
    text_buffer: Buffer,
    cursor_buffer: Buffer,
    text_layout: TextLayout,
    cursor: CursorState,
    has_selection: bool,
    style: RenderStyle,
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
        let mut selection_buffer = Buffer::new(&mut font_system, metrics);
        selection_buffer.set_wrap(Wrap::None);
        let mut cursor_buffer = Buffer::new(&mut font_system, metrics);
        cursor_buffer.set_wrap(Wrap::None);

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
            selection_buffer,
            text_buffer,
            cursor_buffer,
            text_layout: default_layout,
            cursor: CursorState {
                column: 0,
                row: 0,
                visible: false,
                shape: CursorShape::Block,
            },
            has_selection: false,
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

    pub fn update_terminal(
        &mut self,
        snapshot: &TerminalSnapshot,
        cursor: CursorState,
        layout: TextLayout,
    ) {
        self.text_layout = layout;
        self.cursor = cursor;
        self.has_selection = !snapshot.selection.is_empty();
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
            &Attrs::new().family(Family::Name(&self.style.font_family)),
            Shaping::Basic,
            None,
        );
        self.text_buffer
            .shape_until_scroll(&mut self.font_system, false);

        self.selection_buffer.set_metrics_and_size(
            metrics,
            Some(content_width),
            Some(content_height),
        );
        self.selection_buffer.set_text(
            &selection_text(snapshot),
            &Attrs::new().family(Family::Name(&self.style.font_family)),
            Shaping::Basic,
            None,
        );
        self.selection_buffer
            .shape_until_scroll(&mut self.font_system, false);

        self.cursor_buffer.set_metrics_and_size(metrics, None, None);
        self.cursor_buffer.set_text(
            match cursor.shape {
                CursorShape::Block => "█",
                CursorShape::Beam => "▏",
                CursorShape::Underline => "▁",
            },
            &Attrs::new().family(Family::Name(&self.style.font_family)),
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
        let mut text_areas = Vec::with_capacity(3);
        if self.has_selection {
            text_areas.push(TextArea {
                buffer: &self.selection_buffer,
                left: self.text_layout.horizontal_padding,
                top: self.text_layout.vertical_padding,
                scale: 1.0,
                bounds,
                default_color: glyph_color(self.style.selection, 210),
                custom_glyphs: &[],
            });
        }
        text_areas.push(TextArea {
            buffer: &self.text_buffer,
            left: self.text_layout.horizontal_padding,
            top: self.text_layout.vertical_padding,
            scale: 1.0,
            bounds,
            default_color: glyph_color(self.style.foreground, 255),
            custom_glyphs: &[],
        });
        if self.cursor.visible {
            text_areas.push(TextArea {
                buffer: &self.cursor_buffer,
                left: cursor_left,
                top: cursor_top,
                scale: 1.0,
                bounds,
                default_color: glyph_color(self.style.cursor, 255),
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
