use super::*;

impl ToyotermApplication {
    pub(super) fn replace_renderer(&mut self, style: RenderStyle) -> Result<(), String> {
        let window = self
            .window
            .clone()
            .ok_or_else(|| "recreate GPU renderer: window is unavailable".to_owned())?;
        // Rebuild on platforms requiring a new surface when transparency
        // changes. Windows keeps an alpha-capable surface at all opacities;
        // macOS reconfigures the existing Metal layer.
        self.renderer = None;
        let mut renderer = pollster::block_on(GpuRenderer::new(window.clone(), style))
            .map_err(|error| format!("recreate GPU renderer: {error}"))?;
        renderer.resize(window.inner_size());
        self.cell_metrics.width =
            f64::from(renderer.terminal_cell_width(self.cell_metrics.font_size));
        self.renderer = Some(renderer);
        Ok(())
    }

    pub(super) fn recover_renderer(&mut self) -> Result<(), String> {
        let window = self
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
        let mut renderer =
            pollster::block_on(GpuRenderer::new(window.clone(), self.render_style.clone()))
                .map_err(|error| format!("recover GPU renderer: {error}"))?;
        renderer.resize(window.inner_size());
        self.renderer = Some(renderer);
        self.sync_active_renderer(window.scale_factor());
        window.request_redraw();
        tracing::info!(target: "toyoterm::render", "GPU renderer recovery completed");
        Ok(())
    }

    pub(super) fn sync_active_renderer(&mut self, scale_factor: f64) {
        let active = self.mux.current_pane();
        let zoomed = self
            .mux
            .current_tab()
            .and_then(|tab| self.mux.zoomed_pane(tab));
        let snapshots = self
            .pane_layout
            .panes()
            .iter()
            .filter_map(|placement| {
                self.pane_runtimes.get(&placement.pane).map(|runtime| {
                    let is_active = active == Some(placement.pane);
                    let cursor_uses_grid = is_active && self.visual_selection.is_some();
                    let mut cursor = runtime.terminal.cursor();
                    if cursor_uses_grid && let Some(visual) = self.visual_selection {
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
                        cursor_uses_grid,
                        runtime.cursor_line_highlight,
                        placement.rect,
                        is_active,
                        self.pane_badges
                            .get(&placement.pane)
                            .or(runtime.osc_badge.as_ref())
                            .cloned(),
                    )
                })
            })
            .collect::<Vec<_>>();
        let panes = snapshots
            .iter()
            .map(
                |(
                    pane,
                    snapshot,
                    colors,
                    cursor,
                    cursor_uses_grid,
                    cursor_line_highlight,
                    rect,
                    active,
                    badge,
                )| PaneRenderData {
                    pane: *pane,
                    snapshot,
                    colors: *colors,
                    cursor: *cursor,
                    cursor_uses_grid: *cursor_uses_grid,
                    cursor_line_highlight: *cursor_line_highlight,
                    rect: *rect,
                    active: *active,
                    badge: badge.as_deref(),
                    zoomed: zoomed == Some(*pane),
                },
            )
            .collect::<Vec<_>>();
        let active_tab = self.mux.current_tab();
        let tab_titles = self
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
                    .and_then(|pane| self.pane_runtimes.get(&pane))
                    .and_then(|runtime| runtime.progress)
                {
                    title.push_str(&progress_title_suffix(progress));
                }
                let background = self
                    .mux
                    .active_pane(placement.tab)
                    .and_then(|pane| self.pane_runtimes.get(&pane))
                    .and_then(|runtime| runtime.tab_color.complete());
                let session = self
                    .mux
                    .active_pane(placement.tab)
                    .and_then(|pane| self.pane_runtimes.get(&pane));
                (
                    placement.tab,
                    title,
                    placement.rect,
                    active_tab == Some(placement.tab),
                    background,
                    session.and_then(|runtime| runtime.session_status.status.as_deref()),
                    session.and_then(|runtime| runtime.session_status.status_color),
                    session.and_then(|runtime| runtime.session_status.indicator),
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
            .config_error_notice
            .as_ref()
            .map(ConfigErrorNotice::display_message);
        let config_error = self.config_error_notice.as_ref().and_then(|notice| {
            config_error_message
                .as_deref()
                .map(|message| ConfigErrorRenderData {
                    message,
                    notice_rect: self.config_error_layout.notice(),
                    open_log_rect: self.config_error_layout.open_log(),
                    dismiss_rect: self.config_error_layout.dismiss(),
                    log_expanded: notice.log_expanded,
                })
        });
        let layout = self.cell_metrics.text_layout(scale_factor);
        let search_text = self.search_render_text();
        let search_rect = self.window.as_ref().map(|window| {
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
        let selector_view = self.selector.as_ref().and_then(|selector| {
            self.window.as_ref().map(|window| {
                selector_render_view(
                    selector,
                    self.ime_preedit.as_deref(),
                    window.inner_size(),
                    layout,
                    scale_factor,
                )
            })
        });
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.update_panes(&panes, layout);
            renderer.update_tabs(&tabs, layout);
            renderer.update_workspaces(&workspaces, layout);
            renderer.update_search(
                self.search_open.then(|| SearchRenderData {
                    rect: search_rect.unwrap_or_default(),
                    text: &search_text,
                }),
                layout,
            );
            renderer.update_selector(
                selector_view
                    .as_ref()
                    .map(|(text, rect, selected_rect)| SelectorRenderData {
                        rect: *rect,
                        text,
                        selected_rect: *selected_rect,
                    }),
                layout,
            );
            let window_size = self
                .window
                .as_ref()
                .map(|window| window.inner_size())
                .unwrap_or_default();
            let notification_height = self
                .config_error_notice
                .as_ref()
                .map(|notice| config_error_height(scale_factor, notice.log_expanded))
                .unwrap_or(0);
            let chrome_height = workspace_bar_height(&self.script_snapshot.config, scale_factor)
                .saturating_add(tab_bar_height(&self.script_snapshot.config, scale_factor))
                .saturating_add(notification_height)
                .min(window_size.height);
            let (_, bar_rects) = edge_bar_layout(
                window_size,
                chrome_height,
                &self.script_snapshot.config,
                scale_factor,
            );
            let rendered_items = bar_rects
                .iter()
                .map(|(position, _)| {
                    self.bar_items
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
            renderer.update_preedit(self.ime_preedit.as_deref(), layout);
        }
        self.update_ime_cursor_area(scale_factor);
        self.update_window_title();
    }

    pub(super) fn search_render_text(&self) -> String {
        let status = if self.search_query.is_empty() {
            "Type to search".to_owned()
        } else if self.search_result.total == 0 {
            "No matches".to_owned()
        } else {
            format!(
                "{} / {}",
                self.search_result.current, self.search_result.total
            )
        };
        format!(
            "Find: {}▏  {}  (Enter next, Shift+Enter previous, Esc close)",
            self.search_query, status
        )
    }

    pub(super) fn update_ime_cursor_area(&self, scale_factor: f64) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(pane) = self.mux.current_pane() else {
            return;
        };
        let Some(rect) = self.pane_layout.rect(pane) else {
            return;
        };
        let Some(terminal) = self.active_terminal() else {
            return;
        };
        let cursor = terminal.cursor();
        let layout = self.cell_metrics.text_layout(scale_factor);
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
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(pane) = self.mux.current_pane() else {
            window.set_title(self.base_window_title());
            return;
        };
        let Some(runtime) = self.pane_runtimes.get(&pane) else {
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
            .process_id
            .map(|pid| format!(" · pid {pid}"))
            .unwrap_or_default();
        let cwd = runtime
            .cwd
            .as_ref()
            .map(|cwd| format!(" · {}", cwd.display()))
            .unwrap_or_default();
        window.set_title(&format!(
            "{} — {workspace}{tab}{}{pid}{cwd}",
            self.base_window_title(),
            runtime.title
        ));
    }
}

fn selector_render_view(
    selector: &SelectorOverlay,
    preedit: Option<&str>,
    size: PhysicalSize<u32>,
    layout: TextLayout,
    scale_factor: f64,
) -> (String, PaneRect, Option<PaneRect>) {
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
        "Select"
    } else {
        selector.title.as_str()
    };
    let mut text = format!(
        "{title}\n> {}{}▏\n\n",
        selector.query,
        preedit.unwrap_or("")
    );
    if lines.items.is_empty() {
        text.push_str("  No matches\n");
    } else {
        for item in &lines.items {
            text.push_str("  ");
            text.push_str(item);
            text.push('\n');
        }
    }
    text.push('\n');
    text.push_str(&format!(
        "{} matches   ↑↓ move   Enter select   Esc cancel",
        lines.total
    ));

    let selected_rect = lines.selected.map(|selected| {
        PaneRect::new(
            rect.x.saturating_add(8),
            rect.y
                .saturating_add(12)
                .saturating_add(line_height.saturating_mul(selected.saturating_add(3) as u32)),
            rect.width.saturating_sub(16),
            line_height,
        )
    });
    (text, rect, selected_rect)
}

fn progress_title_suffix(progress: TerminalProgress) -> String {
    match progress {
        TerminalProgress::Hidden => String::new(),
        TerminalProgress::Normal(value) => format!(" · {value}%"),
        TerminalProgress::Error(value) => format!(" · error {value}%"),
        TerminalProgress::Indeterminate => " · working".to_owned(),
        TerminalProgress::Warning(value) => format!(" · warning {value}%"),
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
            progress_title_suffix(TerminalProgress::Error(100)),
            " · error 100%"
        );
        assert_eq!(
            progress_title_suffix(TerminalProgress::Indeterminate),
            " · working"
        );
        assert_eq!(
            progress_title_suffix(TerminalProgress::Warning(7)),
            " · warning 7%"
        );
        assert!(progress_title_suffix(TerminalProgress::Hidden).is_empty());
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
        let (text, rect, selected) =
            selector_render_view(&selector, None, PhysicalSize::new(1000, 600), layout, 1.0);
        assert_eq!(rect.x, 140);
        assert_eq!(rect.y, 118);
        assert_eq!(rect.width, 720);
        assert_eq!(rect.height, 364);
        assert!(text.contains("Theme 0"));
        assert!(!text.contains("Theme 12\n"));
        assert!(selected.is_some_and(|selected| selected.y > rect.y));
    }
}
