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
mod terminal;
mod ui;

pub use layout::{
    ConfigErrorLayout, PaneLayout, PanePlacement, PaneRect, SplitAxis, SplitBoundary, TabPlacement,
    TabStripLayout, WorkspacePlacement, WorkspaceStripLayout,
};
use terminal::*;
use ui::*;

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

mod gpu;
use gpu::*;
pub use gpu::{GpuRenderer, RenderError};
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
            search_matches: Vec::new(),
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
    fn styled_shell_prompt_cursor_stays_on_the_terminal_cell_grid() {
        let mut terminal = AlacrittyTerminalBackend::new(80, 4);
        terminal.advance(
            "\n\x1b[1;36mtoyoterm\x1b[0m \x1b[3;36mmaster\x1b[0m \
             \x1b[36m? \x1b[1m>\x1b[0m "
                .as_bytes(),
        );
        let snapshot = terminal.snapshot();
        let cursor = terminal.cursor();
        let metrics = Metrics::new(14.0, 18.0);
        let mut font_system = configured_font_system(&[]);
        let cell_width = measure_cell_width(&mut font_system, "monospace", 400, 14.0);
        let mut buffer = Buffer::new(&mut font_system, metrics);
        buffer.set_wrap(Wrap::None);
        buffer.set_monospace_width(Some(cell_width));
        let rich_text = terminal_rich_text(
            &snapshot,
            Some(cursor),
            "monospace",
            400,
            [220, 225, 232],
            [9, 11, 14],
            &default_ansi_palette(),
        );
        let default_attrs = Attrs::new().family(Family::Monospace);
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
        assert!(
            (rendered_cursor_x - f32::from(cursor.column) * cell_width).abs() < 0.01,
            "cursor x {rendered_cursor_x} did not match column {} at {cell_width}px",
            cursor.column,
        );
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
            search_matches: Vec::new(),
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
                    hyperlink: None,
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
                    hyperlink: None,
                },
            ]],
            selection: Vec::new(),
            search_matches: Vec::new(),
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
        assert_eq!(
            preferred_alpha_mode(&[CompositeAlphaMode::Opaque], 0.8),
            CompositeAlphaMode::Auto
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
