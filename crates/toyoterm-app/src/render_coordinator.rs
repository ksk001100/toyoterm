use super::*;

impl ToyotermApplication {
    pub(super) fn replace_renderer(&mut self, style: RenderStyle) -> Result<(), String> {
        let window = self
            .platform
            .window
            .clone()
            .ok_or_else(|| "recreate GPU renderer: window is unavailable".to_owned())?;
        // Rebuild on platforms requiring a new surface when transparency
        // changes. Windows keeps an alpha-capable surface at all opacities;
        // macOS reconfigures the existing Metal layer.
        self.platform.renderer = None;
        let mut renderer = pollster::block_on(GpuRenderer::new(window.clone(), style))
            .map_err(|error| format!("recreate GPU renderer: {error}"))?;
        renderer.resize(window.inner_size());
        self.ui.cell_metrics.width =
            f64::from(renderer.terminal_cell_width(self.ui.cell_metrics.font_size));
        self.platform.renderer = Some(renderer);
        Ok(())
    }

    pub(super) fn recover_renderer(&mut self) -> Result<(), String> {
        let window = self
            .platform
            .window
            .clone()
            .ok_or_else(|| "recover GPU renderer: window is unavailable".to_owned())?;
        tracing::warn!(
            target: "toyoterm::render",
            width = window.inner_size().width,
            height = window.inner_size().height,
            scale_factor = window.scale_factor(),
            "recreating renderer after GPU device loss"
        );
        let mut renderer = pollster::block_on(GpuRenderer::new(
            window.clone(),
            self.ui.render_style.clone(),
        ))
        .map_err(|error| format!("recover GPU renderer: {error}"))?;
        renderer.resize(window.inner_size());
        self.platform.renderer = Some(renderer);
        self.sync_active_renderer(window.scale_factor());
        window.request_redraw();
        tracing::info!(target: "toyoterm::render", "GPU renderer recovery completed");
        Ok(())
    }

    pub(super) fn sync_active_renderer(&mut self, scale_factor: f64) {
        let now = Instant::now();
        let active = self.mux.current_pane();
        let zoomed = self
            .mux
            .current_tab()
            .and_then(|tab| self.mux.zoomed_pane(tab));
        let snapshot_timer = performance::Stage::new("snapshot");
        let snapshots = self
            .ui
            .pane_layout
            .panes()
            .iter()
            .filter_map(|placement| {
                self.terminal_runtime
                    .pane_runtimes
                    .get(&placement.pane)
                    .map(|runtime| {
                        let is_active = active == Some(placement.pane);
                        let visual_cursor = is_active && self.ui.visual_selection.is_some();
                        let mut cursor = runtime.terminal.cursor();
                        if visual_cursor && let Some(visual) = self.ui.visual_selection {
                            cursor.column = visual.current.column;
                            cursor.row = visual.current.row;
                            cursor.visible = true;
                            cursor.shape = CursorShape::Block;
                        }
                        (
                            placement.pane,
                            runtime.terminal.snapshot(),
                            runtime.terminal.render_colors(),
                            cursor,
                            runtime.protocol.cursor_line_highlight,
                            runtime
                                .protocol
                                .visual_bell_deadline
                                .is_some_and(|deadline| deadline > now),
                            runtime
                                .protocol
                                .cursor_fireworks_deadline
                                .is_some_and(|deadline| deadline > now),
                            placement.rect,
                            is_active,
                            self.ui
                                .pane_badges
                                .get(&placement.pane)
                                .or(runtime.metadata.osc_badge.as_ref())
                                .cloned(),
                        )
                    })
            })
            .collect::<Vec<_>>();
        drop(snapshot_timer);
        let _update_timer = performance::Stage::new("renderer_update");
        let panes = snapshots
            .iter()
            .map(
                |(
                    pane,
                    snapshot,
                    colors,
                    cursor,
                    cursor_line_highlight,
                    visual_bell,
                    cursor_fireworks,
                    rect,
                    active,
                    badge,
                )| {
                    PaneRenderData {
                        pane: *pane,
                        snapshot,
                        colors: *colors,
                        cursor: *cursor,
                        cursor_line_highlight: *cursor_line_highlight,
                        visual_bell: *visual_bell,
                        cursor_fireworks: *cursor_fireworks,
                        rect: *rect,
                        active: *active,
                        badge: badge.as_deref(),
                        zoomed: zoomed == Some(*pane),
                    }
                },
            )
            .collect::<Vec<_>>();
        let active_tab = self.mux.current_tab();
        let tab_titles = self
            .ui
            .tab_layout
            .tabs()
            .iter()
            .map(|placement| {
                let mut title = format!(
                    "Tab {}",
                    self.mux
                        .tab_number(placement.tab)
                        .expect("layout tab exists in mux")
                );
                if let Some(progress) = self
                    .mux
                    .active_pane(placement.tab)
                    .and_then(|pane| self.terminal_runtime.pane_runtimes.get(&pane))
                    .and_then(|runtime| runtime.protocol.progress)
                {
                    title.push_str(&progress_title_suffix(progress));
                }
                let background = self
                    .mux
                    .active_pane(placement.tab)
                    .and_then(|pane| self.terminal_runtime.pane_runtimes.get(&pane))
                    .and_then(|runtime| runtime.protocol.tab_color.complete());
                let session = self
                    .mux
                    .active_pane(placement.tab)
                    .and_then(|pane| self.terminal_runtime.pane_runtimes.get(&pane));
                (
                    placement.tab,
                    title,
                    placement.rect,
                    active_tab == Some(placement.tab),
                    background,
                    session.and_then(|runtime| runtime.protocol.session_status.status.as_deref()),
                    session.and_then(|runtime| runtime.protocol.session_status.status_color),
                    session.and_then(|runtime| runtime.protocol.session_status.indicator),
                )
            })
            .collect::<Vec<_>>();
        let tabs = tab_titles
            .iter()
            .map(
                |(tab, title, rect, active, background, status, status_color, indicator)| {
                    TabRenderData {
                        tab: *tab,
                        title,
                        status: *status,
                        status_color: *status_color,
                        indicator: *indicator,
                        rect: *rect,
                        active: *active,
                        background: *background,
                    }
                },
            )
            .collect::<Vec<_>>();
        let active_workspace = self.mux.current_workspace();
        let workspace_titles = self
            .ui
            .workspace_layout
            .workspaces()
            .iter()
            .filter_map(|placement| {
                self.mux.workspace_name(placement.workspace).map(|name| {
                    (
                        placement.workspace,
                        name.to_owned(),
                        placement.rect,
                        active_workspace == placement.workspace,
                    )
                })
            })
            .collect::<Vec<_>>();
        let workspaces = workspace_titles
            .iter()
            .map(|(workspace, name, rect, active)| WorkspaceRenderData {
                workspace: *workspace,
                name,
                rect: *rect,
                active: *active,
            })
            .collect::<Vec<_>>();
        let config_error_message = self
            .ui
            .config_error_notice
            .as_ref()
            .map(ConfigErrorNotice::display_message);
        let config_error = self.ui.config_error_notice.as_ref().and_then(|notice| {
            config_error_message
                .as_deref()
                .map(|message| ConfigErrorRenderData {
                    message,
                    notice_rect: self.ui.config_error_layout.notice(),
                    open_log_rect: self.ui.config_error_layout.open_log(),
                    dismiss_rect: self.ui.config_error_layout.dismiss(),
                    log_expanded: notice.log_expanded,
                })
        });
        let layout = self.ui.cell_metrics.text_layout(scale_factor);
        let search_text = self.search_render_text();
        let search_rect = self.platform.window.as_ref().map(|window| {
            let size = window.inner_size();
            let width = size
                .width
                .min((560.0 * scale_factor.max(0.1)).round() as u32);
            let height = size
                .height
                .min((52.0 * scale_factor.max(0.1)).round() as u32);
            PaneRect::new(
                (size.width - width) / 2,
                (size.height - height) / 4,
                width,
                height,
            )
        });
        let selector_view = self.ui.selector.as_ref().and_then(|selector| {
            self.platform.window.as_ref().map(|window| {
                selector_render_view(
                    selector,
                    self.ui.ime_preedit.as_deref(),
                    window.inner_size(),
                    layout,
                    scale_factor,
                )
            })
        });
        if let Some(renderer) = self.platform.renderer.as_mut() {
            renderer.update_panes(&panes, layout);
            renderer.update_tabs(&tabs, layout);
            renderer.update_workspaces(&workspaces, layout);
            renderer.update_search(
                self.ui.search_open.then(|| SearchRenderData {
                    rect: search_rect.unwrap_or_default(),
                    text: &search_text,
                }),
                layout,
            );
            renderer.update_selector(
                selector_view.as_ref().map(|view| SelectorRenderData {
                    rect: view.rect,
                    text: &view.text,
                    selected_rect: view.selected_rect,
                }),
                layout,
            );
            let window_size = self
                .platform
                .window
                .as_ref()
                .map(|window| window.inner_size())
                .unwrap_or_default();
            let notification_height = self
                .ui
                .config_error_notice
                .as_ref()
                .map(|notice| config_error_height(scale_factor, notice.log_expanded))
                .unwrap_or(0);
            let chrome_height = workspace_bar_height(&self.scripting.snapshot.config, scale_factor)
                .saturating_add(tab_bar_height(
                    &self.scripting.snapshot.config,
                    scale_factor,
                ))
                .saturating_add(notification_height)
                .min(window_size.height);
            let (_, bar_rects) = edge_bar_layout(
                window_size,
                chrome_height,
                &self.scripting.snapshot.config,
                scale_factor,
            );
            let rendered_items = bar_rects
                .iter()
                .map(|(position, _)| {
                    self.ui
                        .bar_items
                        .get(position)
                        .into_iter()
                        .flatten()
                        .map(|item| StatusBarRenderItem {
                            alignment: match item.alignment {
                                toyoterm_script::BarAlignment::Left => StatusBarAlignment::Left,
                                toyoterm_script::BarAlignment::Center => StatusBarAlignment::Center,
                                toyoterm_script::BarAlignment::Right => StatusBarAlignment::Right,
                            },
                            text: item.text.as_str(),
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let statuses = bar_rects
                .iter()
                .zip(&rendered_items)
                .filter(|((_, rect), _)| rect.width > 0 && rect.height > 0)
                .map(|((position, rect), items)| StatusBarRenderData {
                    rect: *rect,
                    items,
                    edge: match position {
                        StatusBarPosition::Top => StatusBarEdge::Top,
                        StatusBarPosition::Bottom => StatusBarEdge::Bottom,
                    },
                })
                .collect::<Vec<_>>();
            renderer.update_status_bars(&statuses, layout);
            renderer.update_config_error(config_error, layout);
            renderer.update_preedit(self.ui.ime_preedit.as_deref(), layout);
        }
        self.update_ime_cursor_area(
            scale_factor,
            selector_view.as_ref().map(|view| view.ime_rect),
        );
        self.update_window_title();
    }

    pub(super) fn search_render_text(&self) -> String {
        let status = if self.ui.search_query.is_empty() {
            "Type to search".to_owned()
        } else if self.ui.search_result.total == 0 {
            "No matches".to_owned()
        } else {
            format!(
                "{} / {}",
                self.ui.search_result.current, self.ui.search_result.total
            )
        };
        format!(
            "Find: {}▏  {}  (Enter next, Shift+Enter previous, Esc close)",
            self.ui.search_query, status
        )
    }

    fn update_ime_cursor_area(&self, scale_factor: f64, selector_area: Option<PaneRect>) {
        let Some(window) = self.platform.window.as_ref() else {
            return;
        };
        if let Some(rect) = selector_area {
            window.set_ime_cursor_area(
                PhysicalPosition::new(rect.x, rect.y),
                PhysicalSize::new(rect.width, rect.height),
            );
            return;
        }
        let Some(pane) = self.mux.current_pane() else {
            return;
        };
        let Some(rect) = self.ui.pane_layout.rect(pane) else {
            return;
        };
        let Some(terminal) = self.active_terminal() else {
            return;
        };
        let cursor = terminal.cursor();
        let layout = self.ui.cell_metrics.text_layout(scale_factor);
        window.set_ime_cursor_area(
            PhysicalPosition::new(
                f64::from(rect.x)
                    + f64::from(layout.horizontal_padding)
                    + f64::from(cursor.column) * f64::from(layout.cell_width),
                f64::from(rect.y)
                    + f64::from(layout.vertical_padding)
                    + f64::from(cursor.row) * f64::from(layout.line_height),
            ),
            PhysicalSize::new(
                layout.cell_width.max(1.0) as u32,
                layout.line_height.max(1.0) as u32,
            ),
        );
    }

    pub(super) fn update_window_title(&self) {
        let Some(window) = self.platform.window.as_ref() else {
            return;
        };
        let Some(pane) = self.mux.current_pane() else {
            window.set_title(self.base_window_title());
            return;
        };
        let Some(runtime) = self.terminal_runtime.pane_runtimes.get(&pane) else {
            window.set_title(self.base_window_title());
            return;
        };
        let tab = self
            .mux
            .current_tab()
            .and_then(|tab| self.mux.tab_number(tab))
            .map(|number| format!("Tab {number} · "))
            .unwrap_or_default();
        let workspace = self
            .mux
            .workspace_name(self.mux.current_workspace())
            .map(|workspace| format!("{workspace} · "))
            .unwrap_or_default();
        let pid = runtime
            .process
            .process_id
            .map(|pid| format!(" · pid {pid}"))
            .unwrap_or_default();
        let cwd = runtime
            .metadata
            .cwd
            .as_ref()
            .map(|cwd| format!(" · {}", cwd.display()))
            .unwrap_or_default();
        window.set_title(&format!(
            "{} — {workspace}{tab}{}{pid}{cwd}",
            self.base_window_title(),
            runtime.metadata.title
        ));
    }
}

struct SelectorRenderView {
    text: String,
    rect: PaneRect,
    selected_rect: Option<PaneRect>,
    ime_rect: PaneRect,
}

fn selector_render_view(
    selector: &SelectorOverlay,
    preedit: Option<&str>,
    size: PhysicalSize<u32>,
    layout: TextLayout,
    scale_factor: f64,
) -> SelectorRenderView {
    let margin = (24.0 * scale_factor.max(0.1)).round() as u32;
    let maximum_width = (720.0 * scale_factor.max(0.1)).round() as u32;
    let width = maximum_width.min(size.width.saturating_sub(margin.saturating_mul(2)));
    let line_height = layout.line_height.ceil().max(1.0) as u32;
    let maximum_height = size.height.saturating_sub(margin.saturating_mul(2));
    let max_items = maximum_height
        .saturating_sub(24)
        .checked_div(line_height)
        .unwrap_or(0)
        .saturating_sub(5)
        .clamp(1, 12) as usize;
    let lines = selector.render_lines(max_items);
    let visible_count = lines.items.len().max(1);
    let height = line_height
        .saturating_mul(visible_count.saturating_add(5) as u32)
        .saturating_add(24)
        .min(maximum_height);
    let rect = PaneRect::new(
        size.width.saturating_sub(width) / 2,
        size.height.saturating_sub(height) / 2,
        width,
        height,
    );

    let title = if selector.title.is_empty() {
        if selector.is_panel() {
            "Panel"
        } else if selector.is_prompt() {
            "Input"
        } else {
            "Select"
        }
    } else {
        selector.title.as_str()
    };
    let input_columns =
        ((width.saturating_sub(32) as f32 / layout.cell_width.max(1.0)) as usize).saturating_sub(2);
    let input = selector.visible_input_line(preedit, input_columns);
    let mut text = format!("{title}\n> {}\n\n", input.text);
    let ime_x = rect
        .x
        .saturating_add(16)
        .saturating_add(
            ((2 + input.caret_column.unwrap_or(0)) as f32 * layout.cell_width).round() as u32,
        )
        .min(rect.x.saturating_add(rect.width.saturating_sub(1)));
    let ime_y = rect
        .y
        .saturating_add(12)
        .saturating_add(line_height)
        .min(rect.y.saturating_add(rect.height.saturating_sub(1)));
    let ime_rect = PaneRect::new(
        ime_x,
        ime_y,
        (layout.cell_width.ceil().max(1.0) as u32)
            .min(rect.x.saturating_add(rect.width).saturating_sub(ime_x)),
        line_height.min(rect.y.saturating_add(rect.height).saturating_sub(ime_y)),
    );
    if lines.items.is_empty() {
        text.push_str("  ");
        text.push_str(selector.completion_status().unwrap_or("No matches"));
        text.push('\n');
    } else {
        for item in &lines.items {
            text.push_str("  ");
            text.push_str(item);
            text.push('\n');
        }
    }
    text.push('\n');
    if selector.is_panel() {
        text.push_str(&format!(
            "{} lines   ↑↓/PgUp/PgDn browse   Enter/Esc close",
            lines.total
        ));
    } else if selector.is_prompt() {
        text.push_str("Tab complete · ↑↓ candidates · Alt+↑↓ history · Enter accept · Esc cancel");
    } else {
        text.push_str(&format!(
            "{} matches   ↑↓ move   Enter select   Esc cancel",
            lines.total
        ));
    }

    let selected_rect = lines.selected.and_then(|selected| {
        let selected = PaneRect::new(
            rect.x.saturating_add(8),
            rect.y
                .saturating_add(12)
                .saturating_add(line_height.saturating_mul(selected.saturating_add(3) as u32)),
            rect.width.saturating_sub(16),
            line_height,
        );
        (selected.width > 0
            && selected.y.saturating_add(selected.height)
                <= rect.y.saturating_add(rect.height.saturating_sub(12)))
        .then_some(selected)
    });
    SelectorRenderView {
        text,
        rect,
        selected_rect,
        ime_rect,
    }
}

fn progress_title_suffix(progress: TerminalProgress) -> String {
    match progress {
        TerminalProgress::Hidden => String::new(),
        TerminalProgress::Normal(value) => format!(" · {value}%"),
        TerminalProgress::Error(Some(value)) => format!(" · error {value}%"),
        TerminalProgress::Error(None) => " · error".to_owned(),
        TerminalProgress::Indeterminate => " · working".to_owned(),
        TerminalProgress::Warning(Some(value)) => format!(" · warning {value}%"),
        TerminalProgress::Warning(None) => " · warning".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_terminal_progress_for_tab_titles() {
        assert_eq!(
            progress_title_suffix(TerminalProgress::Normal(42)),
            " · 42%"
        );
        assert_eq!(
            progress_title_suffix(TerminalProgress::Error(Some(100))),
            " · error 100%"
        );
        assert_eq!(
            progress_title_suffix(TerminalProgress::Indeterminate),
            " · working"
        );
        assert_eq!(
            progress_title_suffix(TerminalProgress::Warning(Some(7))),
            " · warning 7%"
        );
        assert_eq!(
            progress_title_suffix(TerminalProgress::Error(None)),
            " · error"
        );
        assert_eq!(
            progress_title_suffix(TerminalProgress::Warning(None)),
            " · warning"
        );
        assert!(progress_title_suffix(TerminalProgress::Hidden).is_empty());
    }

    #[test]
    fn panel_view_pages_content_and_identifies_dismissal_instead_of_selection() {
        let mut panel = SelectorOverlay::panel(
            3,
            String::new(),
            (0..40).map(|i| format!("Row {i}")).collect(),
        );
        panel.move_last();
        let layout = TextLayout {
            font_size: 14.0,
            line_height: 20.0,
            cell_width: 9.0,
            horizontal_padding: 0.0,
            vertical_padding: 0.0,
        };
        let SelectorRenderView { text, rect, .. } =
            selector_render_view(&panel, None, PhysicalSize::new(1000, 600), layout, 1.0);
        assert!(text.starts_with("Panel\n"));
        assert!(text.contains("Row 39\n"));
        assert!(!text.contains("Row 0\n"));
        assert!(text.contains("Enter/Esc close"));
        assert!(!text.contains("Enter select"));
        assert!(rect.height <= 600);
    }

    #[test]
    fn selector_view_is_centered_and_bounds_visible_results() {
        let selector = SelectorOverlay::new(
            1,
            "Theme".into(),
            (0..30).map(|index| format!("Theme {index}")).collect(),
        );
        let layout = TextLayout {
            font_size: 14.0,
            line_height: 20.0,
            cell_width: 9.0,
            horizontal_padding: 0.0,
            vertical_padding: 0.0,
        };
        let SelectorRenderView {
            text,
            rect,
            selected_rect: selected,
            ..
        } = selector_render_view(&selector, None, PhysicalSize::new(1000, 600), layout, 1.0);
        assert_eq!(rect.x, 140);
        assert_eq!(rect.y, 118);
        assert_eq!(rect.width, 720);
        assert_eq!(rect.height, 364);
        assert!(text.contains("Theme 0"));
        assert!(!text.contains("Theme 12\n"));
        assert!(selected.is_some_and(|selected| selected.y > rect.y));
    }

    #[test]
    fn selector_ime_anchor_tracks_input_cursor_and_preedit_instead_of_literal_caret_text() {
        let mut selector = SelectorOverlay::prompt(
            5,
            toyoterm_api::InputPrompt {
                initial: "▏猫".into(),
                ..Default::default()
            },
        );
        let layout = TextLayout {
            font_size: 14.0,
            line_height: 20.0,
            cell_width: 9.0,
            horizontal_padding: 0.0,
            vertical_padding: 0.0,
        };
        let view = |selector: &SelectorOverlay, preedit| {
            selector_render_view(selector, preedit, PhysicalSize::new(1000, 600), layout, 1.0)
        };
        let end = view(&selector, None);
        assert_eq!(
            end.ime_rect,
            PaneRect::new(end.rect.x + 61, end.rect.y + 32, 9, 20)
        );
        selector.move_cursor(false);
        let middle = view(&selector, None);
        assert_eq!(middle.ime_rect.x, middle.rect.x + 43);
        let composing = view(&selector, Some("変換"));
        assert_eq!(composing.ime_rect.x, middle.ime_rect.x + 36);
        selector.cursor_edge(false);
        let start = view(&selector, None);
        assert_eq!(start.ime_rect.x, start.rect.x + 34);
    }

    #[test]
    fn wide_prompt_input_and_selection_remain_inside_small_scaled_overlays() {
        use unicode_width::UnicodeWidthStr;
        let selector = SelectorOverlay::prompt(
            4,
            toyoterm_api::InputPrompt {
                initial: "日本語👩‍💻".repeat(40),
                completions: vec!["日本語👩‍💻".repeat(40)],
                ..Default::default()
            },
        );
        for scale in [1.0, 1.5, 2.0] {
            let layout = TextLayout {
                font_size: 14.0 * scale as f32,
                line_height: 20.0 * scale as f32,
                cell_width: 9.0 * scale as f32,
                horizontal_padding: 0.0,
                vertical_padding: 0.0,
            };
            for (width, height) in [(0, 0), (64, 64), (240, 160), (500, 300), (1000, 600)] {
                let SelectorRenderView {
                    text,
                    rect,
                    selected_rect: selected,
                    ime_rect,
                } = selector_render_view(
                    &selector,
                    Some("変換"),
                    PhysicalSize::new(width, height),
                    layout,
                    scale,
                );
                assert!(rect.x + rect.width <= width);
                assert!(rect.y + rect.height <= height);
                assert!(ime_rect.x >= rect.x && ime_rect.y >= rect.y);
                assert!(ime_rect.x + ime_rect.width <= rect.x + rect.width);
                assert!(ime_rect.y + ime_rect.height <= rect.y + rect.height);
                if let Some(selected) = selected {
                    assert!(selected.x >= rect.x && selected.y >= rect.y);
                    assert!(selected.x + selected.width <= rect.x + rect.width);
                    assert!(selected.y + selected.height <= rect.y + rect.height);
                }
                let input = text.lines().nth(1).unwrap();
                if rect.width as f32 >= 32.0 + 3.0 * layout.cell_width {
                    assert!(input.contains('▏'));
                    assert!(
                        input.width() as f32 * layout.cell_width
                            <= rect.width.saturating_sub(32) as f32
                    );
                }
            }
        }
    }
}
