use super::*;

impl ToyotermApplication {
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
                self.pane_runtimes.get_mut(&placement.pane).map(|runtime| {
                    let snapshot = runtime
                        .snapshot_cache
                        .get_or_insert_with(|| Rc::new(runtime.terminal.snapshot()))
                        .clone();
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
                        snapshot,
                        cursor,
                        cursor_uses_grid,
                        placement.rect,
                        is_active,
                        self.pane_badges.get(&placement.pane).cloned(),
                    )
                })
            })
            .collect::<Vec<_>>();
        let panes = snapshots
            .iter()
            .map(
                |(pane, snapshot, cursor, cursor_uses_grid, rect, active, badge)| PaneRenderData {
                    pane: *pane,
                    snapshot: snapshot.as_ref(),
                    cursor: *cursor,
                    cursor_uses_grid: *cursor_uses_grid,
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
                (
                    placement.tab,
                    format!(
                        "Tab {}",
                        self.mux
                            .tab_number(placement.tab)
                            .expect("layout tab exists in mux")
                    ),
                    placement.rect,
                    active_tab == Some(placement.tab),
                )
            })
            .collect::<Vec<_>>();
        let tabs = tab_titles
            .iter()
            .map(|(tab, title, rect, active)| TabRenderData {
                tab: *tab,
                title,
                rect: *rect,
                active: *active,
            })
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
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.update_panes(&panes, layout, scale_factor as f32);
            renderer.update_tabs(&tabs, layout);
            renderer.update_workspaces(&workspaces, layout);
            renderer.update_search(
                self.search_open.then(|| SearchRenderData {
                    rect: search_rect.unwrap_or_default(),
                    text: &search_text,
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
