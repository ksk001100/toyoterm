use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use glyphon::cosmic_text::{Align, Cursor as TextCursor, Fallback, PlatformFallback};
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
    pub cursor_uses_grid: bool,
    pub rect: PaneRect,
    pub active: bool,
    pub zoomed: bool,
    pub badge: Option<&'a str>,
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
pub struct SearchRenderData<'a> {
    pub rect: PaneRect,
    pub text: &'a str,
}

#[derive(Clone, Copy, Debug)]
pub struct StatusBarRenderData<'a> {
    pub rect: PaneRect,
    pub items: &'a [StatusBarRenderItem<'a>],
    pub edge: StatusBarEdge,
}

#[derive(Clone, Copy, Debug)]
pub struct StatusBarRenderItem<'a> {
    pub alignment: StatusBarAlignment,
    pub text: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusBarAlignment {
    Left,
    Center,
    Right,
}

fn status_bar_section_text(
    items: &[StatusBarRenderItem<'_>],
    alignment: StatusBarAlignment,
) -> String {
    items
        .iter()
        .filter(|item| item.alignment == alignment)
        .map(|item| item.text)
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusBarEdge {
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug)]
pub struct ConfigErrorRenderData<'a> {
    pub message: &'a str,
    pub notice_rect: PaneRect,
    pub open_log_rect: PaneRect,
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
    pub tab_bar: [u8; 3],
    pub tab_active: [u8; 3],
    pub tab_inactive: [u8; 3],
    pub workspace_bar: [u8; 3],
    pub status_bar: [u8; 3],
    pub pane_border: [u8; 3],
    pub zoomed_pane_border: [u8; 3],
    pub search_match: [u8; 3],
    pub search_match_active: [u8; 3],
    pub ansi: [[u8; 3]; 16],
    pub opacity: f32,
    pub active_pane_border_width: f32,
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
            tab_bar: [17, 21, 27],
            tab_active: [24, 36, 58],
            tab_inactive: [21, 25, 31],
            workspace_bar: [13, 16, 20],
            status_bar: [16, 20, 25],
            pane_border: [55, 88, 145],
            zoomed_pane_border: [255, 190, 58],
            search_match: [196, 151, 47],
            search_match_active: [255, 190, 58],
            ansi: default_ansi_palette(),
            opacity: 1.0,
            active_pane_border_width: 2.0,
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
            ..Self::default()
        })
    }

    pub fn from_hex_with_ui(
        font_family: impl Into<String>,
        font_fallback: Vec<String>,
        font_weight: u16,
        colors: [&str; 13],
        ansi: &[String],
        opacity: f32,
        active_pane_border_width: f32,
    ) -> Result<Self, RenderError> {
        let [
            background,
            foreground,
            cursor,
            selection,
            tab_bar,
            tab_active,
            tab_inactive,
            workspace_bar,
            status_bar,
            pane_border,
            zoomed_pane_border,
            search_match,
            search_match_active,
        ] = colors;
        let mut style = Self::from_hex_with_ansi(
            font_family,
            font_fallback,
            font_weight,
            [background, foreground, cursor, selection],
            ansi,
            opacity,
        )?;
        style.tab_bar = parse_rgb(tab_bar)?;
        style.tab_active = parse_rgb(tab_active)?;
        style.tab_inactive = parse_rgb(tab_inactive)?;
        style.workspace_bar = parse_rgb(workspace_bar)?;
        style.status_bar = parse_rgb(status_bar)?;
        style.pane_border = parse_rgb(pane_border)?;
        style.zoomed_pane_border = parse_rgb(zoomed_pane_border)?;
        style.search_match = parse_rgb(search_match)?;
        style.search_match_active = parse_rgb(search_match_active)?;
        style.active_pane_border_width = active_pane_border_width;
        Ok(style)
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
    #[cfg(target_os = "windows")]
    fn windows_presentation_uses_direct_composition_and_premultiplied_alpha() {
        let descriptor = renderer_instance_descriptor();
        assert_eq!(descriptor.backends, wgpu::Backends::DX12);
        assert_eq!(
            descriptor.backend_options.dx12.presentation_system,
            wgpu::Dx12SwapchainKind::DxgiFromVisual
        );
        let supported = [
            CompositeAlphaMode::Opaque,
            CompositeAlphaMode::PostMultiplied,
            CompositeAlphaMode::PreMultiplied,
        ];
        for opacity in [1.0, 0.9, 0.5, 0.0, 0.8, 1.0] {
            let mode = preferred_alpha_mode(&supported, opacity);
            assert_eq!(mode, CompositeAlphaMode::PreMultiplied);
            let style = RenderStyle {
                background: [255, 128, 64],
                opacity,
                ..RenderStyle::default()
            };
            let clear = clear_color(&style, mode);
            assert_eq!(clear.a, f64::from(opacity));
            assert_eq!(clear.r, clear.a);
            assert!(clear.g <= clear.a && clear.b <= clear.a);
        }
    }

    #[test]
    fn window_bar_groups_widgets_by_alignment_in_registration_order() {
        let items = [
            StatusBarRenderItem {
                alignment: StatusBarAlignment::Right,
                text: "clock",
            },
            StatusBarRenderItem {
                alignment: StatusBarAlignment::Left,
                text: "cwd",
            },
            StatusBarRenderItem {
                alignment: StatusBarAlignment::Left,
                text: "branch",
            },
        ];

        assert_eq!(
            status_bar_section_text(&items, StatusBarAlignment::Left),
            "cwd branch"
        );
        assert_eq!(
            status_bar_section_text(&items, StatusBarAlignment::Center),
            ""
        );
        assert_eq!(
            status_bar_section_text(&items, StatusBarAlignment::Right),
            "clock"
        );
    }

    #[test]
    fn render_plan_matches_snapshot() {
        let pane = PaneRect::new(10, 20, 320, 180);
        let layout = TextLayout {
            font_size: 14.0,
            line_height: 18.0,
            cell_width: 9.0,
            horizontal_padding: 4.0,
            vertical_padding: 6.0,
        };
        let placement = pane_text_placement(
            pane,
            layout,
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
        let selection_rects = selection_highlight_rects(&terminal, pane, layout);
        let snapshot = format!(
            concat!(
                "background_rgba=({:.6},{:.6},{:.6},{:.6})\n",
                "glyph_origin=({:.1},{:.1})\n",
                "cursor_origin=({:.1},{:.1})\n",
                "clip=({},{},{},{})\n",
                "selection_rects={:?}\n",
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
            selection_rects,
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
    fn split_separator_stays_in_the_same_column_with_diagnostic_symbols() {
        let mut font_system = configured_font_system(&[]);
        let cell_width = measure_cell_width(&mut font_system, "monospace", 400, 14.0);
        for text in [
            "plain",
            "󰅚 error",
            "⚠ warning",
            "日本語",
            "→ hint",
            "e\u{301}",
        ] {
            let mut terminal = AlacrittyTerminalBackend::new(80, 2);
            terminal.advance(text.as_bytes());
            terminal.advance("\x1b[1;41H│ right pane".as_bytes());
            let snapshot = terminal.snapshot();
            let layout = TextLayout {
                cell_width,
                font_size: 14.0,
                line_height: 18.0,
                horizontal_padding: 0.0,
                vertical_padding: 0.0,
            };
            let style = RenderStyle::default();
            let mut separator_x = None;
            for (_, cells) in terminal_cell_runs(&snapshot) {
                let mut buffer = Buffer::new(&mut font_system, Metrics::new(14.0, 18.0));
                update_terminal_cell_buffer(&mut buffer, &mut font_system, cells, layout, &style);
                if cells.iter().any(|cell| cell.column == 40) {
                    let glyph = buffer.layout_runs().next().unwrap().glyphs.first().unwrap();
                    separator_x = Some(f32::from(cells[0].column) * cell_width + glyph.x);
                }
            }
            let x = separator_x.expect("separator is rendered");
            assert!(
                (x - 40.0 * cell_width).abs() < 0.01,
                "{text:?}: separator x {x}, expected {}",
                40.0 * cell_width
            );
        }
    }

    #[test]
    fn scrolled_cell_runs_keep_wide_and_combining_cells_at_grid_positions() {
        let mut terminal = AlacrittyTerminalBackend::new(80, 4);
        terminal.advance("\x1b[?1049h\x1b[1;3r".as_bytes());
        for (row, text) in [(1, "󰅚 error"), (2, "日本語"), (3, "e\u{301} hint")] {
            terminal.advance(format!("\x1b[{row};1H{text}\x1b[{row};41H│ right").as_bytes());
        }
        let mut font_system = configured_font_system(&[]);
        let layout = TextLayout {
            cell_width: 8.25,
            font_size: 14.0,
            line_height: 18.0,
            horizontal_padding: 0.0,
            vertical_padding: 0.0,
        };
        let style = RenderStyle::default();
        let mut buffer = Buffer::new(&mut font_system, Metrics::new(14.0, 18.0));
        for scrolls in 0..3 {
            let snapshot = terminal.snapshot();
            let mut separators = 0;
            for (row, cells) in terminal_cell_runs(&snapshot) {
                update_terminal_cell_buffer(&mut buffer, &mut font_system, cells, layout, &style);
                let run = buffer.layout_runs().next().unwrap();
                assert_eq!(run.line_i, 0, "each run uses its explicit terminal row");
                if cells[0].text == "│" {
                    assert_eq!(cells[0].column, 40);
                    assert!(row < 3 - scrolls);
                    assert_eq!(run.glyphs[0].x, 0.0);
                    separators += 1;
                }
                if cells[0].text == "日" {
                    assert_eq!(cells.len(), 1);
                    assert_eq!(cells[0].width, 2);
                }
                if cells[0].text.starts_with('e') && cells[0].text.contains('\u{301}') {
                    assert_eq!(cells.len(), 1, "combining marks stay with the base cell");
                }
            }
            assert_eq!(separators, 3 - scrolls);
            terminal.advance(b"\x1b[1;1H\x1b[M");
        }
    }

    #[test]
    fn terminal_rich_text_coalesces_adjacent_cells_with_the_same_attributes() {
        let mut terminal = AlacrittyTerminalBackend::new(10, 2);
        terminal.advance(b"abcdefghij");
        let snapshot = terminal.snapshot();
        let spans = terminal_rich_text(
            &snapshot,
            None,
            "monospace",
            400,
            [220, 225, 232],
            [9, 11, 14],
            &default_ansi_palette(),
        );

        assert_eq!(
            spans
                .iter()
                .map(|(text, _)| text.as_str())
                .collect::<String>(),
            "abcdefghij\n"
        );
        assert_eq!(spans.len(), 2, "one text run plus the row separator");
    }

    #[test]
    fn visual_cursor_uses_the_same_fixed_cell_grid_as_selection() {
        let snapshot = TerminalSnapshot {
            columns: 12,
            rows: 1,
            lines: vec!["wide: 日本語".into()],
            cells: Vec::new(),
            selection: Vec::new(),
            search_matches: Vec::new(),
        };
        let cursor = CursorState {
            column: 7,
            row: 0,
            visible: true,
            shape: CursorShape::Block,
        };
        let metrics = Metrics::new(14.0, 18.0);
        let mut font_system = configured_font_system(&[]);
        let cell_width = measure_cell_width(&mut font_system, "monospace", 400, 14.0);
        let mut buffer = Buffer::new(&mut font_system, metrics);
        buffer.set_text(
            &snapshot.lines.join("\n"),
            &Attrs::new().family(Family::Monospace),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut font_system, false);

        assert_eq!(
            pane_cursor_x(&buffer, &snapshot, cursor, cell_width, true),
            f32::from(cursor.column) * cell_width
        );
    }

    #[test]
    fn builds_cell_aligned_selection_rectangles() {
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
        let layout = TextLayout {
            font_size: 14.0,
            line_height: 18.0,
            cell_width: 9.0,
            horizontal_padding: 8.0,
            vertical_padding: 4.0,
        };
        assert_eq!(
            selection_highlight_rects(&snapshot, PaneRect::new(10, 20, 100, 80), layout),
            [PaneRect::new(36, 24, 27, 18), PaneRect::new(18, 42, 18, 18),]
        );
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
            if cfg!(target_os = "windows") {
                CompositeAlphaMode::PreMultiplied
            } else {
                CompositeAlphaMode::Opaque
            }
        );
        assert_eq!(
            preferred_alpha_mode(&[CompositeAlphaMode::Opaque], 0.8),
            CompositeAlphaMode::Auto
        );
    }

    #[test]
    fn opacity_transitions_update_alpha_mode_and_clear_color() {
        let supported = [
            CompositeAlphaMode::Opaque,
            CompositeAlphaMode::PostMultiplied,
        ];
        let mut style = RenderStyle::default();
        for opacity in [1.0, 0.8, 0.0, 0.5, 1.0] {
            style.opacity = opacity;
            let mode = preferred_alpha_mode(&supported, opacity);
            assert_eq!(
                mode,
                if cfg!(target_os = "windows") || opacity < 1.0 {
                    CompositeAlphaMode::PostMultiplied
                } else {
                    CompositeAlphaMode::Opaque
                }
            );
            let clear = clear_color(&style, mode);
            assert_eq!(clear.a, f64::from(opacity));
            assert_eq!(
                clear.r,
                f64::from(srgb_channel_to_linear(style.background[0]))
            );
        }
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_opacity_rounding_keeps_the_same_compositor_mode() {
        // Ruby arithmetic uses f64, then native configuration narrows to f32.
        let rounded_one = (0.9_f64 + 0.1) as f32;
        let just_below_one = f32::from_bits(1.0_f32.to_bits() - 1);
        for supported in [
            vec![
                CompositeAlphaMode::Opaque,
                CompositeAlphaMode::PreMultiplied,
            ],
            vec![
                CompositeAlphaMode::Opaque,
                CompositeAlphaMode::PostMultiplied,
            ],
            vec![CompositeAlphaMode::Opaque, CompositeAlphaMode::Inherit],
            vec![CompositeAlphaMode::Opaque],
        ] {
            let initial_mode = preferred_alpha_mode(&supported, 0.9);
            for opacity in [0.9, rounded_one, 0.9, just_below_one, 1.0, 0.8, 0.0, 1.0] {
                let mode = preferred_alpha_mode(&supported, opacity);
                assert_eq!(mode, initial_mode);
                let style = RenderStyle {
                    opacity,
                    ..RenderStyle::default()
                };
                let clear = clear_color(&style, mode);
                assert_eq!(clear.a, f64::from(opacity));
                let background = f64::from(srgb_channel_to_linear(style.background[0]));
                assert_eq!(
                    clear.r,
                    if mode == CompositeAlphaMode::PreMultiplied {
                        background * f64::from(opacity)
                    } else {
                        background
                    }
                );
            }
        }
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

#[cfg(test)]
mod offscreen_tests;
