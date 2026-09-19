use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use glyphon::cosmic_text::{Align, Fallback, PlatformFallback};
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
use toyoterm_terminal::{
    CellAttributes, CellColor, CursorShape, CursorState, TerminalColors, TerminalSnapshot,
    TerminalTransparentColor,
};

mod background;
mod graphics;
pub use background::BackgroundImage;
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
    pub cursor_line_highlight: bool,
    pub visual_bell: bool,
    pub cursor_fireworks: bool,
    pub rect: PaneRect,
    pub active: bool,
    pub zoomed: bool,
    pub badge: Option<&'a str>,
    pub colors: TerminalColors,
}

#[derive(Clone, Copy, Debug)]
pub struct TabRenderData<'a> {
    pub tab: TabId,
    pub title: &'a str,
    pub status: Option<&'a str>,
    pub status_color: Option<[u8; 3]>,
    pub indicator: Option<[u8; 3]>,
    pub rect: PaneRect,
    pub active: bool,
    pub background: Option<[u8; 3]>,
}

fn tab_fill_color(style: &RenderStyle, background: Option<[u8; 3]>, active: bool) -> [f32; 4] {
    background.map_or_else(
        || {
            if active {
                rgba(style.tab_active, 1.0)
            } else {
                rgba(style.tab_inactive, 0.96)
            }
        },
        |color| rgba(color, if active { 1.0 } else { 0.96 }),
    )
}

fn tab_indicator_rect(rect: PaneRect) -> PaneRect {
    PaneRect::new(
        rect.x.saturating_add(7),
        rect.y
            .saturating_add(rect.height.saturating_sub(6).saturating_div(2)),
        6.min(rect.width.saturating_sub(7)),
        6.min(rect.height),
    )
}

fn pane_background_override(style: &RenderStyle, colors: TerminalColors) -> Option<[u8; 3]> {
    (colors.background != style.background).then_some(colors.background)
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
pub struct SelectorRenderData<'a> {
    pub rect: PaneRect,
    pub text: &'a str,
    pub selected_rect: Option<PaneRect>,
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
    pub background_image: Option<BackgroundImage>,
    pub background_image_opacity: f32,
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
            background_image: None,
            background_image_opacity: 1.0,
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
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
mod offscreen_tests;
