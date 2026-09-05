use std::fmt;
use toyoterm_api::{PaneId, TabId, WorkspaceId};
use toyoterm_terminal::{CellColor, CursorShape, CursorState, TerminalSnapshot};
mod gpui_renderer;
mod layout;
mod terminal;
mod ui;
pub use gpui_renderer::GpuiRenderer;
pub use layout::{
    ConfigErrorLayout, PaneLayout, PanePlacement, PaneRect, SplitAxis, SplitBoundary, TabPlacement,
    TabStripLayout, WorkspacePlacement, WorkspaceStripLayout,
};
use terminal::*;
use ui::*;
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.operation, self.message)
    }
}
impl std::error::Error for RenderError {}
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
    pub content_changed: bool,
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

#[cfg(test)]
mod tests {
    use super::*;
    use toyoterm_terminal::{CellAttributes, SelectionSpan};
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
}
