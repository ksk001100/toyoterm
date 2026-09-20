use super::domain_handlers::PaneSearchEffect;
use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct UiEffects {
    pub(super) request_redraw: bool,
}

pub(super) struct SelectorCompletion {
    pub(super) id: u64,
    pub(super) selection: Option<String>,
}

pub(super) fn apply_pane_effect(
    effect: PaneSearchEffect,
    terminal_runtime: &mut TerminalRuntime,
    ui: &mut UiState,
) {
    ui.exit_visual_mode(terminal_runtime, Some(effect.pane));
    ui.ime_preedit = None;
    ui.search_open = true;
    ui.search_query = effect.query;
    ui.search_result = effect.result;
}

pub(super) fn close_search(
    active_pane: Option<PaneId>,
    terminal_runtime: &mut TerminalRuntime,
    ui: &mut UiState,
) {
    ui.close_search(terminal_runtime, active_pane);
}

pub(super) fn refresh_search(
    pane: Option<PaneId>,
    direction: SearchDirection,
    terminal_runtime: &mut TerminalRuntime,
    ui: &mut UiState,
) {
    let query = ui.search_query.clone();
    ui.search_result = pane
        .and_then(|pane| terminal(terminal_runtime, pane))
        .map(|terminal| terminal.search(&query, direction))
        .unwrap_or_default();
}

pub(super) fn handle_search_key(
    event: &KeyEvent,
    modifiers: ModifiersState,
    pane: Option<PaneId>,
    terminal_runtime: &mut TerminalRuntime,
    ui: &mut UiState,
) {
    match &event.logical_key {
        Key::Named(NamedKey::Escape) => ui.close_search(terminal_runtime, pane),
        Key::Named(NamedKey::Enter) => refresh_search(
            pane,
            if modifiers.shift_key() {
                SearchDirection::Previous
            } else {
                SearchDirection::Next
            },
            terminal_runtime,
            ui,
        ),
        Key::Named(NamedKey::Backspace) => {
            ui.search_query.pop();
            refresh_search(pane, SearchDirection::Next, terminal_runtime, ui);
        }
        Key::Character(text) if !modifiers.control_key() && !modifiers.super_key() => {
            ui.search_query
                .push_str(event.text.as_deref().unwrap_or(text));
            refresh_search(pane, SearchDirection::Next, terminal_runtime, ui);
        }
        _ => {}
    }
}

pub(super) fn handle_selector_key(
    event: &KeyEvent,
    modifiers: ModifiersState,
    ui: &mut UiState,
) -> Option<SelectorCompletion> {
    let mut selection = None;
    let selector = ui.selector.as_mut()?;
    match &event.logical_key {
        Key::Named(NamedKey::Escape) => selection = Some(None),
        Key::Named(NamedKey::Enter) => {
            if let Some(selected) = selector.selected_item() {
                selection = Some(Some(selected));
            }
        }
        Key::Named(NamedKey::ArrowUp) => selector.move_previous(),
        Key::Named(NamedKey::ArrowDown) => selector.move_next(),
        Key::Named(NamedKey::PageUp) => selector.page_previous(),
        Key::Named(NamedKey::PageDown) => selector.page_next(),
        Key::Named(NamedKey::Home) => selector.move_first(),
        Key::Named(NamedKey::End) => selector.move_last(),
        Key::Named(NamedKey::Backspace) => selector.pop_query(),
        Key::Character(text) if !modifiers.control_key() && !modifiers.super_key() => {
            selector.append_query(event.text.as_deref().unwrap_or(text));
        }
        _ => {}
    }
    let selection = selection?;
    let selector = ui.selector.take()?;
    ui.ime_preedit = None;
    Some(SelectorCompletion {
        id: selector.id,
        selection,
    })
}

pub(super) fn apply_ui_command(
    command: UiCommand,
    active_pane: Option<PaneId>,
    terminal_runtime: &mut TerminalRuntime,
    ui: &mut UiState,
) -> UiEffects {
    let mut effects = UiEffects::default();
    match command {
        UiCommand::OpenSelector { id, title, items } => {
            ui.close_search(terminal_runtime, active_pane);
            ui.exit_visual_mode(terminal_runtime, active_pane);
            ui.leader_deadline = None;
            ui.ime_preedit = None;
            ui.selector = Some(SelectorOverlay::new(id, title, items));
        }
        UiCommand::SetPaneBadge { pane, badge } => ui.set_pane_badge(pane, badge),
        UiCommand::OpenSearch { pane } => {
            ui.close_search(terminal_runtime, Some(pane));
            ui.search_open = true;
        }
        UiCommand::NavigatePrompt { pane, direction } => {
            effects.request_redraw = terminal(terminal_runtime, pane)
                .is_some_and(|terminal| terminal.navigate_prompt(search_direction(direction)));
        }
        UiCommand::NavigateMark { pane, direction } => {
            effects.request_redraw = terminal(terminal_runtime, pane)
                .is_some_and(|terminal| terminal.navigate_mark(search_direction(direction)));
        }
        UiCommand::SelectCommandOutput { pane, direction } => {
            effects.request_redraw = terminal(terminal_runtime, pane).is_some_and(|terminal| {
                terminal.select_command_output(search_direction(direction))
            });
        }
        UiCommand::SelectLastCommandOutput { pane } => {
            effects.request_redraw = terminal(terminal_runtime, pane)
                .is_some_and(TerminalBackend::select_last_command_output);
        }
        UiCommand::StartVisualMode { pane } => ui.start_visual_mode(terminal_runtime, pane),
        UiCommand::ToggleVisualMode { pane } => {
            if ui.visual_selection.is_some() {
                ui.exit_visual_mode(terminal_runtime, Some(pane));
            } else {
                ui.start_visual_mode(terminal_runtime, pane);
            }
        }
        UiCommand::StartVisualSelection { pane } => {
            ui.start_visual_mode(terminal_runtime, pane);
            ui.select_visual_selection(terminal_runtime, pane);
        }
        UiCommand::SelectVisualSelection { pane } => {
            ui.select_visual_selection(terminal_runtime, pane)
        }
        UiCommand::EndVisualSelection { pane } => ui.exit_visual_mode(terminal_runtime, Some(pane)),
        UiCommand::MoveVisualSelection { pane, motion } => {
            ui.move_visual_selection(terminal_runtime, pane, motion)
        }
    }
    effects
}

fn terminal(
    terminal_runtime: &mut TerminalRuntime,
    pane: PaneId,
) -> Option<&mut AlacrittyTerminalBackend> {
    terminal_runtime
        .pane_runtimes
        .get_mut(&pane)
        .map(|runtime| &mut runtime.terminal)
}

fn search_direction(direction: PaneSearchDirection) -> SearchDirection {
    match direction {
        PaneSearchDirection::Next => SearchDirection::Next,
        PaneSearchDirection::Previous => SearchDirection::Previous,
    }
}

impl UiState {
    pub(super) fn set_pane_badge(&mut self, pane: PaneId, badge: Option<String>) {
        match badge {
            Some(badge) => {
                self.pane_badges.insert(pane, badge);
            }
            None => {
                self.pane_badges.remove(&pane);
            }
        }
    }

    pub(super) fn exit_visual_mode(
        &mut self,
        terminal_runtime: &mut TerminalRuntime,
        active_pane: Option<PaneId>,
    ) {
        if self.visual_selection.take().is_some()
            && let Some(terminal) = active_pane.and_then(|pane| terminal(terminal_runtime, pane))
        {
            terminal.clear_selection();
        }
    }

    fn close_search(
        &mut self,
        terminal_runtime: &mut TerminalRuntime,
        active_pane: Option<PaneId>,
    ) {
        self.search_open = false;
        self.search_query.clear();
        self.search_result = SearchResult::default();
        if let Some(terminal) = active_pane.and_then(|pane| terminal(terminal_runtime, pane)) {
            terminal.clear_search();
        }
    }

    fn start_visual_mode(&mut self, terminal_runtime: &mut TerminalRuntime, pane: PaneId) {
        let Some(terminal) = terminal(terminal_runtime, pane) else {
            return;
        };
        let cursor = terminal.cursor();
        let snapshot = terminal.snapshot();
        let row = cursor.row.min(snapshot.rows.saturating_sub(1));
        terminal.clear_selection();
        self.visual_selection = Some(VisualSelection {
            anchor: None,
            current: VisualPosition {
                column: snap_to_cell_start(&snapshot, row, cursor.column),
                row,
            },
        });
    }

    fn select_visual_selection(&mut self, terminal_runtime: &mut TerminalRuntime, pane: PaneId) {
        let Some(mut visual) = self.visual_selection else {
            return;
        };
        visual.anchor = Some(visual.current);
        if let Some(terminal) = terminal(terminal_runtime, pane) {
            terminal.start_selection(
                visual.current.column,
                visual.current.row,
                SelectionKind::Simple,
            );
        }
        self.visual_selection = Some(visual);
    }

    fn move_visual_selection(
        &mut self,
        terminal_runtime: &mut TerminalRuntime,
        pane: PaneId,
        motion: SelectionMotion,
    ) {
        let Some(mut selection) = self.visual_selection else {
            return;
        };
        let Some(terminal) = terminal(terminal_runtime, pane) else {
            return;
        };
        let snapshot = terminal.snapshot();
        let max_row = snapshot.rows.saturating_sub(1);
        let mut scroll = 0;
        match motion {
            SelectionMotion::Left => {
                selection.current.column =
                    visual_prev_column(&snapshot, selection.current.row, selection.current.column);
            }
            SelectionMotion::Right => {
                selection.current.column =
                    visual_next_column(&snapshot, selection.current.row, selection.current.column);
            }
            SelectionMotion::Up => {
                if selection.current.row == 0 {
                    scroll = 1;
                } else {
                    selection.current.row -= 1;
                    selection.current.column = snap_to_cell_start(
                        &snapshot,
                        selection.current.row,
                        selection.current.column,
                    );
                }
            }
            SelectionMotion::Down => {
                if selection.current.row == max_row {
                    scroll = -1;
                } else {
                    selection.current.row += 1;
                    selection.current.column = snap_to_cell_start(
                        &snapshot,
                        selection.current.row,
                        selection.current.column,
                    );
                }
            }
            SelectionMotion::LineStart => selection.current.column = 0,
            SelectionMotion::LineEnd => {
                selection.current.column =
                    visual_last_cell_column(&snapshot, selection.current.row);
            }
            SelectionMotion::WordForward => {
                let (column, row) =
                    visual_next_word(&snapshot, selection.current.row, selection.current.column);
                selection.current.column = column;
                selection.current.row = row;
            }
            SelectionMotion::WordBackward => {
                let (column, row) =
                    visual_prev_word(&snapshot, selection.current.row, selection.current.column);
                selection.current.column = column;
                selection.current.row = row;
            }
        }
        if selection.anchor.is_some() {
            if scroll != 0 {
                terminal.scroll_display(scroll);
            }
            terminal.update_selection(selection.current.column, selection.current.row);
        } else if scroll != 0 {
            terminal.scroll_display(scroll);
        }
        self.visual_selection = Some(selection);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_handler_owns_badge_search_selector_and_visual_state() {
        let pane = PaneId(1);
        let mut terminals = test_terminal_runtime([pane]);
        let mut ui = test_ui_state();

        apply_ui_command(
            UiCommand::SetPaneBadge {
                pane,
                badge: Some("build".into()),
            },
            Some(pane),
            &mut terminals,
            &mut ui,
        );
        assert_eq!(ui.pane_badges.get(&pane).map(String::as_str), Some("build"));

        apply_ui_command(
            UiCommand::OpenSearch { pane },
            Some(pane),
            &mut terminals,
            &mut ui,
        );
        assert!(ui.search_open);
        assert!(ui.search_query.is_empty());

        apply_pane_effect(
            PaneSearchEffect {
                pane,
                query: "needle".into(),
                result: SearchResult::default(),
            },
            &mut terminals,
            &mut ui,
        );
        assert_eq!(ui.search_query, "needle");

        apply_ui_command(
            UiCommand::StartVisualSelection { pane },
            Some(pane),
            &mut terminals,
            &mut ui,
        );
        assert!(
            ui.visual_selection
                .as_ref()
                .is_some_and(|selection| selection.anchor.is_some())
        );

        apply_ui_command(
            UiCommand::OpenSelector {
                id: 42,
                title: "Pick".into(),
                items: vec!["one".into(), "two".into()],
            },
            Some(pane),
            &mut terminals,
            &mut ui,
        );
        assert!(!ui.search_open);
        assert!(ui.visual_selection.is_none());
        assert_eq!(ui.selector.as_ref().map(|selector| selector.id), Some(42));
    }
}
