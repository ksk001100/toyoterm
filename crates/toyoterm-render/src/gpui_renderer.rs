use super::*;
use gpui::{
    App, Bounds, ContentMask, FontWeight, Pixels, Point, ShapedLine, TextRun, Window, fill, font,
    point, px, size,
};
use std::{
    cell::RefCell,
    collections::HashMap,
    hash::{Hash, Hasher},
    rc::Rc,
    sync::Arc,
};

#[derive(Clone)]
struct Quad {
    rect: PaneRect,
    color: [u8; 3],
    alpha: f32,
}
#[derive(Clone)]
struct Label {
    text: String,
    x: f32,
    y: f32,
    clip: PaneRect,
    layout: TextLayout,
    color: [u8; 3],
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    fixed: bool,
    cells: u16,
}
/// Retained terminal scene. GPUI owns the native window, GPU and glyph caches.
#[derive(Clone)]
pub struct GpuiRenderer {
    style: RenderStyle,
    panes: Arc<Vec<Quad>>,
    text: Arc<Vec<Label>>,
    tabs: Vec<Quad>,
    tab_text: Vec<Label>,
    workspaces: Vec<Quad>,
    workspace_text: Vec<Label>,
    search: Vec<Quad>,
    search_text: Vec<Label>,
    bars: Vec<Quad>,
    bar_text: Vec<Label>,
    errors: Vec<Quad>,
    error_text: Vec<Label>,
    preedit: Option<Label>,
    cursor: Option<(f32, f32, PaneRect)>,
    /// Shaping is substantially more expensive than painting. Keep shaped
    /// lines across GPUI render passes; terminal output invalidates only the
    /// scene labels that are rebuilt by `update_panes`.
    shape_cache: Rc<RefCell<HashMap<u64, ShapedLine>>>,
}
impl GpuiRenderer {
    pub fn new(style: RenderStyle) -> Self {
        Self {
            style,
            panes: Arc::new(vec![]),
            text: Arc::new(vec![]),
            tabs: vec![],
            tab_text: vec![],
            workspaces: vec![],
            workspace_text: vec![],
            search: vec![],
            search_text: vec![],
            bars: vec![],
            bar_text: vec![],
            errors: vec![],
            error_text: vec![],
            preedit: None,
            cursor: None,
            shape_cache: Rc::new(RefCell::new(HashMap::new())),
        }
    }
    pub fn set_style(&mut self, style: RenderStyle) {
        self.style = style;
        self.shape_cache.borrow_mut().clear();
    }
    pub fn terminal_cell_width(&self, font_size: f32, window: &Window) -> f32 {
        let run = self.run("M", self.style.foreground, false, false, false, false);
        f32::from(
            window
                .text_system()
                .shape_line("M".into(), px(font_size), &[run], None)
                .width,
        )
        .max(1.0)
    }
    fn run(
        &self,
        text: &str,
        color: [u8; 3],
        bold: bool,
        italic: bool,
        underline: bool,
        strike: bool,
    ) -> TextRun {
        let mut face = font(resolved_family(&self.style.font_family));
        face.weight = FontWeight(if bold {
            700.0
        } else {
            f32::from(self.style.font_weight)
        });
        if italic {
            face = face.italic();
        }
        face.fallbacks = Some(gpui::FontFallbacks::from_fonts(
            self.style.font_fallback.clone(),
        ));
        TextRun {
            len: text.len(),
            font: face,
            color: color_value(color, 1.0),
            background_color: None,
            underline: underline.then(|| gpui::UnderlineStyle {
                thickness: px(1.0),
                color: None,
                wavy: false,
            }),
            strikethrough: strike.then(|| gpui::StrikethroughStyle {
                thickness: px(1.0),
                color: None,
            }),
        }
    }
    pub fn update_panes(&mut self, panes: &[PaneRenderData<'_>], layout: TextLayout, scale: f32) {
        let mut pane_quads = reusable_scene(&mut self.panes);
        let mut text = reusable_scene(&mut self.text);
        self.cursor = None;
        for pane in panes {
            let rect = pane.rect;
            for (rect, color) in terminal_backgrounds(
                pane.snapshot,
                rect,
                layout,
                self.style.background,
                self.style.foreground,
                &self.style.ansi,
            ) {
                pane_quads.push(Quad {
                    rect,
                    color,
                    alpha: 1.0,
                });
            }
            for (rect, active) in search_highlight_rects(pane.snapshot, rect, layout) {
                pane_quads.push(Quad {
                    rect,
                    color: if active {
                        self.style.search_match_active
                    } else {
                        self.style.search_match
                    },
                    alpha: 1.0,
                });
            }
            for rect in selection_highlight_rects(pane.snapshot, rect, layout) {
                pane_quads.push(Quad {
                    rect,
                    color: self.style.selection,
                    alpha: 1.0,
                });
            }
            let x = rect.x as f32 + layout.horizontal_padding;
            let y = rect.y as f32 + layout.vertical_padding;
            let cx = x + f32::from(pane.cursor.column) * layout.cell_width;
            let cy = y + f32::from(pane.cursor.row) * layout.line_height;
            if pane.active {
                self.cursor = Some((cx, cy, rect));
                if pane.cursor.visible {
                    let (left, top, width, height) = match pane.cursor.shape {
                        CursorShape::Block => (cx, cy, layout.cell_width, layout.line_height),
                        CursorShape::Beam => (cx, cy, 2.0, layout.line_height),
                        CursorShape::Underline => {
                            (cx, cy + layout.line_height - 2.0, layout.cell_width, 2.0)
                        }
                    };
                    pane_quads.push(Quad {
                        rect: PaneRect::new(
                            left.max(0.) as u32,
                            top.max(0.) as u32,
                            width.max(1.) as u32,
                            height.max(1.) as u32,
                        ),
                        color: self.style.cursor,
                        alpha: 1.0,
                    });
                }
            }
            for (row, cells) in pane
                .snapshot
                .cells
                .iter()
                .take(usize::from(pane.snapshot.rows))
                .enumerate()
            {
                for cell in cells {
                    let attrs = cell.attributes;
                    if attrs.hidden || cell.text.is_empty() {
                        continue;
                    }
                    let mut color = if attrs.inverse {
                        resolve_cell_color(
                            attrs.background,
                            self.style.background,
                            &self.style.ansi,
                        )
                    } else {
                        resolve_cell_color(
                            attrs.foreground,
                            self.style.foreground,
                            &self.style.ansi,
                        )
                    };
                    if attrs.dim {
                        color = color.map(|c| (f32::from(c) * 0.66) as u8);
                    }
                    if pane.active
                        && pane.cursor.visible
                        && pane.cursor.shape == CursorShape::Block
                        && pane.cursor.row as usize == row
                        && pane.cursor.column >= cell.column
                        && pane.cursor.column
                            < cell.column.saturating_add(u16::from(cell.width.max(1)))
                    {
                        color = self.style.background;
                    }
                    let item = Label {
                        text: cell.text.replace(['\r', '\n'], ""),
                        x: x + f32::from(cell.column) * layout.cell_width,
                        y: y + row as f32 * layout.line_height,
                        clip: rect,
                        layout,
                        color,
                        bold: attrs.bold,
                        italic: attrs.italic,
                        underline: attrs.underline || cell.hyperlink.is_some(),
                        strike: attrs.strikethrough,
                        fixed: cell.width == 1 && cell.text.len() == 1 && cell.text.is_ascii(),
                        cells: u16::from(cell.width.max(1)),
                    };
                    if item.fixed
                        && let Some(previous) = text.last_mut()
                        && previous.text.is_ascii()
                        && previous.y == item.y
                        && previous.clip == item.clip
                        && previous.color == item.color
                        && previous.bold == item.bold
                        && previous.italic == item.italic
                        && previous.underline == item.underline
                        && previous.strike == item.strike
                        && (previous.x + previous.layout.cell_width * f32::from(previous.cells)
                            - item.x)
                            .abs()
                            < 0.01
                        && previous.cells < 256
                    {
                        // Keep the run anchored at its first grid cell. The
                        // natural advance is used for the run; forcing a
                        // width on a multi-glyph line can distort glyphs.
                        previous.text.push_str(&item.text);
                        previous.fixed = false;
                        previous.cells = previous.cells.saturating_add(item.cells);
                    } else {
                        text.push(item);
                    }
                }
            }
            if pane.active {
                for rect in pane_border_rects(
                    rect,
                    (self.style.active_pane_border_width * scale)
                        .round()
                        .max(0.) as u32,
                ) {
                    pane_quads.push(Quad {
                        rect,
                        color: if pane.zoomed {
                            self.style.zoomed_pane_border
                        } else {
                            self.style.pane_border
                        },
                        alpha: 1.0,
                    });
                }
            }
            if let Some(badge) = pane.badge {
                let mut item = label(badge, rect, layout, self.style.foreground);
                item.x = -2.0;
                item.y = rect.y as f32 + layout.vertical_padding;
                text.push(item);
            }
        }
        self.panes = Arc::new(pane_quads);
        self.text = Arc::new(text);
    }
    pub fn update_tabs(&mut self, tabs: &[TabRenderData<'_>], layout: TextLayout) {
        self.tabs.clear();
        self.tab_text.clear();
        for tab in tabs {
            self.tabs.push(Quad {
                rect: tab.rect,
                color: if tab.active {
                    self.style.tab_active
                } else {
                    self.style.tab_inactive
                },
                alpha: 1.0,
            });
            self.tab_text
                .push(label(tab.title, tab.rect, layout, self.style.foreground));
        }
    }
    pub fn update_workspaces(
        &mut self,
        workspaces: &[WorkspaceRenderData<'_>],
        layout: TextLayout,
    ) {
        self.workspaces.clear();
        self.workspace_text.clear();
        for workspace in workspaces {
            self.workspaces.push(Quad {
                rect: workspace.rect,
                color: if workspace.active {
                    self.style.tab_active
                } else {
                    self.style.workspace_bar
                },
                alpha: 1.0,
            });
            self.workspace_text.push(label(
                workspace.name,
                workspace.rect,
                layout,
                self.style.foreground,
            ));
        }
    }
    pub fn update_search(&mut self, search: Option<SearchRenderData<'_>>, layout: TextLayout) {
        self.search.clear();
        self.search_text.clear();
        if let Some(search) = search {
            self.search.push(Quad {
                rect: search.rect,
                color: self.style.status_bar,
                alpha: 1.0,
            });
            self.search_text.push(label(
                search.text,
                search.rect,
                layout,
                self.style.foreground,
            ));
        }
    }
    pub fn update_status_bars(&mut self, bars: &[StatusBarRenderData<'_>], layout: TextLayout) {
        self.bars.clear();
        self.bar_text.clear();
        for bar in bars {
            self.bars.push(Quad {
                rect: bar.rect,
                color: self.style.status_bar,
                alpha: 1.0,
            });
            for alignment in [
                StatusBarAlignment::Left,
                StatusBarAlignment::Center,
                StatusBarAlignment::Right,
            ] {
                let text = status_bar_section_text(bar.items, alignment);
                let mut item = label(&text, bar.rect, layout, self.style.foreground);
                // Final alignment is based on shaped glyph width in paint.
                item.fixed = false;
                item.x = match alignment {
                    StatusBarAlignment::Left => item.x,
                    StatusBarAlignment::Center => -1.0,
                    StatusBarAlignment::Right => -2.0,
                };
                self.bar_text.push(item);
            }
        }
    }
    pub fn update_config_error(
        &mut self,
        error: Option<ConfigErrorRenderData<'_>>,
        layout: TextLayout,
    ) {
        self.errors.clear();
        self.error_text.clear();
        if let Some(error) = error {
            self.errors.push(Quad {
                rect: error.notice_rect,
                color: [70, 24, 24],
                alpha: 1.0,
            });
            for (row, line) in error.message.lines().enumerate() {
                let mut item = label(line, error.notice_rect, layout, self.style.foreground);
                item.y = error.notice_rect.y as f32
                    + layout.vertical_padding
                    + row as f32 * layout.line_height;
                self.error_text.push(item);
            }
            self.error_text.push(label(
                if error.log_expanded {
                    "Hide log"
                } else {
                    "Open log"
                },
                error.open_log_rect,
                layout,
                self.style.foreground,
            ));
            self.error_text.push(label(
                "Dismiss",
                error.dismiss_rect,
                layout,
                self.style.foreground,
            ));
        }
    }
    pub fn update_preedit(&mut self, text: Option<&str>, layout: TextLayout) {
        self.preedit = text.zip(self.cursor).map(|(text, (x, y, clip))| {
            let mut item = label(text, clip, layout, self.style.foreground);
            item.x = x;
            item.y = y;
            item.underline = true;
            item
        });
    }
    pub fn paint(
        &self,
        origin: Point<Pixels>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.paint_quad(fill(
            bounds,
            color_value(self.style.background, self.style.opacity),
        ));
        let scale = window.scale_factor().max(0.1);
        for (quads, color) in [
            (&self.tabs, self.style.tab_bar),
            (&self.workspaces, self.style.workspace_bar),
        ] {
            if let Some(first) = quads.first() {
                window.paint_quad(fill(
                    Bounds::new(
                        origin + point(px(0.), px(first.rect.y as f32 / scale)),
                        size(bounds.size.width, px(first.rect.height as f32 / scale)),
                    ),
                    color_value(color, 1.0),
                ));
            }
        }
        self.paint_group(&self.panes, &self.text, origin, scale, window, cx);
        self.paint_group(&self.tabs, &self.tab_text, origin, scale, window, cx);
        self.paint_group(
            &self.workspaces,
            &self.workspace_text,
            origin,
            scale,
            window,
            cx,
        );
        self.paint_group(&self.search, &self.search_text, origin, scale, window, cx);
        self.paint_group(&self.bars, &self.bar_text, origin, scale, window, cx);
        self.paint_group(&self.errors, &self.error_text, origin, scale, window, cx);
        if let Some(text) = &self.preedit {
            self.paint_label(text, origin, scale, window, cx);
        }
    }
    fn paint_group(
        &self,
        quads: &[Quad],
        texts: &[Label],
        origin: Point<Pixels>,
        scale: f32,
        window: &mut Window,
        cx: &mut App,
    ) {
        for quad in quads {
            window.paint_quad(fill(
                gpui_bounds(quad.rect, origin, scale),
                color_value(quad.color, quad.alpha),
            ));
        }
        for text in texts {
            self.paint_label(text, origin, scale, window, cx);
        }
    }
    fn paint_label(
        &self,
        label: &Label,
        origin: Point<Pixels>,
        scale: f32,
        window: &mut Window,
        cx: &mut App,
    ) {
        if label.text.is_empty() {
            return;
        }
        let run = self.run(
            &label.text,
            label.color,
            label.bold,
            label.italic,
            label.underline,
            label.strike,
        );
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        label.text.hash(&mut hasher);
        label.layout.font_size.to_bits().hash(&mut hasher);
        label.layout.cell_width.to_bits().hash(&mut hasher);
        scale.to_bits().hash(&mut hasher);
        label.color.hash(&mut hasher);
        label.bold.hash(&mut hasher);
        label.italic.hash(&mut hasher);
        label.underline.hash(&mut hasher);
        label.strike.hash(&mut hasher);
        label.fixed.hash(&mut hasher);
        label.cells.hash(&mut hasher);
        let cache_key = hasher.finish();
        let line = if let Some(line) = self.shape_cache.borrow().get(&cache_key) {
            line.clone()
        } else {
            let line = window.text_system().shape_line(
                label.text.clone().into(),
                px(label.layout.font_size / scale),
                &[run],
                label
                    .fixed
                    .then(|| px(label.layout.cell_width * f32::from(label.cells) / scale)),
            );
            if self.shape_cache.borrow().len() >= 8192 {
                self.shape_cache.borrow_mut().clear();
            }
            self.shape_cache
                .borrow_mut()
                .insert(cache_key, line.clone());
            line
        };
        let width = f32::from(line.width) * scale;
        let x = if label.x == -1.0 {
            label.clip.x as f32 + (label.clip.width as f32 - width) / 2.0
        } else if label.x == -2.0 {
            label.clip.x as f32 + label.clip.width as f32 - width - label.layout.horizontal_padding
        } else {
            label.x
        };
        let position = origin + point(px(x / scale), px(label.y / scale));
        let line_height = px(label.layout.line_height / scale);
        let mut paint = |window: &mut Window| {
            if let Err(error) = line.paint(position, line_height, window, cx) {
                tracing::warn!(%error, "GPUI text paint failed");
            }
        };
        // Fixed-width ASCII cells cannot draw outside their own grid cell.
        // Avoid installing a content mask for them; masks are relatively
        // expensive when thousands of cells are painted per frame.
        if label.text.is_ascii()
            && x >= label.clip.x as f32
            && x + width <= label.clip.x as f32 + label.clip.width as f32
        {
            paint(window);
        } else {
            window.with_content_mask(
                Some(ContentMask {
                    bounds: gpui_bounds(label.clip, origin, scale),
                }),
                paint,
            );
        }
    }
}
fn label(text: &str, rect: PaneRect, layout: TextLayout, color: [u8; 3]) -> Label {
    Label {
        text: text.replace(['\r', '\n'], " "),
        x: rect.x as f32 + layout.horizontal_padding,
        y: rect.y as f32 + (rect.height as f32 - layout.line_height).max(0.) / 2.0,
        clip: rect,
        layout,
        color,
        bold: false,
        italic: false,
        underline: false,
        strike: false,
        fixed: false,
        cells: 1,
    }
}
fn color_value(color: [u8; 3], alpha: f32) -> gpui::Hsla {
    gpui::Rgba {
        r: f32::from(color[0]) / 255.,
        g: f32::from(color[1]) / 255.,
        b: f32::from(color[2]) / 255.,
        a: alpha,
    }
    .into()
}
fn gpui_bounds(rect: PaneRect, origin: Point<Pixels>, scale: f32) -> Bounds<Pixels> {
    Bounds::new(
        origin + point(px(rect.x as f32 / scale), px(rect.y as f32 / scale)),
        size(
            px(rect.width as f32 / scale),
            px(rect.height as f32 / scale),
        ),
    )
}

fn reusable_scene<T>(scene: &mut Arc<Vec<T>>) -> Vec<T> {
    let capacity = scene.len();
    if let Some(scene) = Arc::get_mut(scene) {
        let mut reused = std::mem::take(scene);
        reused.clear();
        reused
    } else {
        Vec::with_capacity(capacity)
    }
}

fn resolved_family(family: &str) -> String {
    if family.eq_ignore_ascii_case("monospace") {
        if cfg!(target_os = "windows") {
            return "Consolas".into();
        }
        if cfg!(target_os = "macos") {
            return "Menlo".into();
        }
    }
    family.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use toyoterm_terminal::{AlacrittyTerminalBackend, TerminalBackend};
    fn layout() -> TextLayout {
        TextLayout {
            font_size: 14.,
            cell_width: 9.,
            line_height: 18.,
            horizontal_padding: 8.,
            vertical_padding: 4.,
        }
    }
    fn update(renderer: &mut GpuiRenderer, terminal: &AlacrittyTerminalBackend) {
        renderer.update_panes(
            &[PaneRenderData {
                pane: PaneId(1),
                snapshot: &terminal.snapshot(),
                cursor: terminal.cursor(),
                cursor_uses_grid: true,
                rect: PaneRect::new(10, 20, 200, 100),
                active: true,
                zoomed: false,
                badge: None,
            }],
            layout(),
            1.0,
        );
    }
    #[test]
    fn unicode_and_combining_cells_keep_their_grid_origins() {
        let mut terminal = AlacrittyTerminalBackend::new(20, 3);
        terminal.advance("A日e\u{301}Z".as_bytes());
        let mut renderer = GpuiRenderer::new(RenderStyle::default());
        update(&mut renderer, &terminal);
        let labels: Vec<_> = renderer
            .text
            .iter()
            .filter(|l| !l.text.trim().is_empty())
            .map(|l| (l.text.as_str(), l.x, l.fixed))
            .collect();
        assert_eq!(
            labels,
            vec![
                ("A", 18., true),
                ("日", 27., false),
                ("e\u{301}", 45., false),
                ("Z", 54., true)
            ]
        );
        assert_eq!(
            renderer.cursor,
            Some((63., 24., PaneRect::new(10, 20, 200, 100)))
        );
    }
    #[test]
    fn ansi_attributes_and_selection_survive_scene_construction() {
        let mut terminal = AlacrittyTerminalBackend::new(10, 2);
        terminal.advance(b"\x1b[1;3;4;9;31;44mA\x1b[0mB");
        terminal.start_selection(0, 0, toyoterm_terminal::SelectionKind::Simple);
        terminal.update_selection(1, 0);
        let style = RenderStyle::default();
        let mut renderer = GpuiRenderer::new(style.clone());
        update(&mut renderer, &terminal);
        let label = renderer.text.iter().find(|l| l.text == "A").unwrap();
        assert!(label.bold && label.italic && label.underline && label.strike);
        assert_eq!(label.color, style.ansi[1]);
        assert!(renderer.panes.iter().any(|q| q.color == style.ansi[4]));
        assert!(renderer.panes.iter().any(|q| q.color == style.selection));
    }
    #[test]
    fn removing_panes_and_overlays_clears_retained_content() {
        let terminal = AlacrittyTerminalBackend::new(10, 2);
        let mut renderer = GpuiRenderer::new(RenderStyle::default());
        update(&mut renderer, &terminal);
        renderer.update_preedit(Some("日本語"), layout());
        assert!(renderer.preedit.is_some());
        renderer.update_panes(&[], layout(), 1.0);
        renderer.update_preedit(None, layout());
        assert!(
            renderer.panes.is_empty() && renderer.text.is_empty() && renderer.preedit.is_none()
        );
        assert!(renderer.cursor.is_none());
        renderer.update_search(
            Some(SearchRenderData {
                rect: PaneRect::new(0, 0, 100, 20),
                text: "Find",
            }),
            layout(),
        );
        renderer.update_search(None, layout());
        assert!(renderer.search.is_empty() && renderer.search_text.is_empty());
    }
    #[test]
    fn notices_start_at_the_top_and_preedit_tracks_the_cursor() {
        let mut renderer = GpuiRenderer::new(RenderStyle::default());
        let rect = PaneRect::new(0, 0, 300, 100);
        renderer.update_config_error(
            Some(ConfigErrorRenderData {
                message: "first\nsecond",
                notice_rect: rect,
                open_log_rect: rect,
                dismiss_rect: rect,
                log_expanded: true,
            }),
            layout(),
        );
        assert_eq!(renderer.error_text[0].y, 4.);
        assert_eq!(renderer.error_text[1].y, 22.);
        let terminal = AlacrittyTerminalBackend::new(10, 2);
        update(&mut renderer, &terminal);
        renderer.update_preedit(Some("候補"), layout());
        let preedit = renderer.preedit.as_ref().unwrap();
        assert_eq!((preedit.x, preedit.y), (18., 24.));
        assert!(preedit.underline);
    }
    #[test]
    fn chrome_alignment_and_background_opacity_are_independent() {
        let style = RenderStyle {
            opacity: 0.3,
            ..RenderStyle::default()
        };
        let mut renderer = GpuiRenderer::new(style);
        let items = [
            StatusBarRenderItem {
                alignment: StatusBarAlignment::Center,
                text: "center",
            },
            StatusBarRenderItem {
                alignment: StatusBarAlignment::Right,
                text: "right",
            },
        ];
        renderer.update_status_bars(
            &[StatusBarRenderData {
                rect: PaneRect::new(0, 0, 300, 30),
                items: &items,
                edge: StatusBarEdge::Top,
            }],
            layout(),
        );
        assert_eq!(renderer.bar_text[1].x, -1.);
        assert_eq!(renderer.bar_text[2].x, -2.);
        assert_eq!(renderer.bars[0].alpha, 1.);
        assert_eq!(renderer.style.opacity, 0.3);
        let logical = gpui_bounds(PaneRect::new(20, 40, 200, 100), point(px(0.), px(0.)), 2.0);
        assert_eq!(
            logical,
            Bounds::new(point(px(10.), px(20.)), size(px(100.), px(50.)))
        );
    }
    #[test]
    fn rebuilding_a_retained_scene_does_not_clone_the_previous_frame() {
        #[derive(Debug)]
        struct CloneCounter(Arc<AtomicUsize>);
        impl Clone for CloneCounter {
            fn clone(&self) -> Self {
                self.0.fetch_add(1, Ordering::Relaxed);
                Self(self.0.clone())
            }
        }

        let clones = Arc::new(AtomicUsize::new(0));
        let mut scene = Arc::new(vec![
            CloneCounter(clones.clone()),
            CloneCounter(clones.clone()),
        ]);
        let retained_frame = scene.clone();

        let next_frame = reusable_scene(&mut scene);

        assert_eq!(clones.load(Ordering::Relaxed), 0);
        assert!(next_frame.is_empty());
        assert!(next_frame.capacity() >= retained_frame.len());
        assert_eq!(retained_frame.len(), 2);
    }
}
