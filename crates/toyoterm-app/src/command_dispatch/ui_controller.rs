use super::domain_handlers::PaneSearchEffect;
use super::*;

#[cfg(test)]
mod extension_flow_tests;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct UiEffects {
    pub(super) request_redraw: bool,
    pub(super) error: Option<String>,
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
    handle_selector_input(&event.logical_key, event.text.as_deref(), modifiers, ui)
}

fn handle_selector_input(
    key: &Key,
    text: Option<&str>,
    modifiers: ModifiersState,
    ui: &mut UiState,
) -> Option<SelectorCompletion> {
    if ui.ime_preedit.is_some() && matches!(key, Key::Named(NamedKey::Enter)) {
        return None;
    }
    let mut selection = None;
    let selector = ui.selector.as_mut()?;
    match key {
        Key::Named(NamedKey::Escape) => selection = Some(None),
        Key::Named(NamedKey::Enter) if selector.is_panel() => selection = Some(None),
        Key::Named(NamedKey::Enter) => {
            if let Some(selected) = selector.result() {
                selection = Some(Some(selected));
            }
        }
        Key::Named(NamedKey::ArrowUp) if selector.is_prompt() && modifiers.alt_key() => {
            selector.recall_history(true)
        }
        Key::Named(NamedKey::ArrowDown) if selector.is_prompt() && modifiers.alt_key() => {
            selector.recall_history(false)
        }
        Key::Named(NamedKey::ArrowUp) => selector.move_previous(),
        Key::Named(NamedKey::ArrowDown) => selector.move_next(),
        Key::Named(NamedKey::PageUp) => selector.page_previous(),
        Key::Named(NamedKey::PageDown) => selector.page_next(),
        Key::Named(NamedKey::ArrowLeft) if selector.is_prompt() => selector.move_cursor(false),
        Key::Named(NamedKey::ArrowRight) if selector.is_prompt() => selector.move_cursor(true),
        Key::Named(NamedKey::Home) if selector.is_prompt() => selector.cursor_edge(false),
        Key::Named(NamedKey::End) if selector.is_prompt() => selector.cursor_edge(true),
        Key::Named(NamedKey::Delete) if selector.is_prompt() => selector.delete_query(),
        Key::Named(NamedKey::Tab) if selector.is_prompt() => selector.complete(),
        Key::Named(NamedKey::Home) => selector.move_first(),
        Key::Named(NamedKey::End) => selector.move_last(),
        Key::Named(NamedKey::Backspace) => selector.pop_query(),
        Key::Character(text)
            if (selector.is_prompt() || selector.is_panel())
                && modifiers.control_key()
                && !modifiers.alt_key() =>
        {
            match text.to_lowercase().as_str() {
                "a" => selector.cursor_edge(false),
                "e" => selector.cursor_edge(true),
                "b" => selector.move_cursor(false),
                "f" => selector.move_cursor(true),
                "p" => selector.move_previous(),
                "n" => selector.move_next(),
                "u" => selector.clear_query(true),
                "k" => selector.clear_query(false),
                "g" => selection = Some(None),
                _ => {}
            }
        }
        Key::Character(character) if !modifiers.control_key() && !modifiers.super_key() => {
            selector.append_query(text.unwrap_or(character));
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
        UiCommand::OpenPanel { id, title, lines } => {
            ui.close_search(terminal_runtime, active_pane);
            ui.exit_visual_mode(terminal_runtime, active_pane);
            ui.leader_deadline = None;
            ui.key_sequences.clear();
            ui.ime_preedit = None;
            ui.selector = Some(SelectorOverlay::panel(id, title, lines));
            effects.request_redraw = true;
        }
        UiCommand::UpdatePanel { id, title, lines } => {
            if let Some(selector) = ui.selector.as_mut() {
                selector.update_panel(id, title, lines);
                effects.request_redraw = true;
            }
        }
        UiCommand::ClosePrompt { id } => {
            if ui
                .selector
                .as_ref()
                .is_some_and(|selector| selector.id == id)
            {
                ui.selector = None;
                ui.ime_preedit = None;
                effects.request_redraw = true;
            }
        }
        UiCommand::UpdatePromptCompletions {
            id,
            revision,
            items,
        } => {
            if let Some(selector) = ui.selector.as_mut() {
                selector.update_completions(id, revision, items);
            }
            effects.request_redraw = true;
        }
        UiCommand::OpenPrompt { id, prompt } => {
            ui.close_search(terminal_runtime, active_pane);
            ui.exit_visual_mode(terminal_runtime, active_pane);
            ui.leader_deadline = None;
            ui.key_sequences.clear();
            ui.ime_preedit = None;
            ui.selector = Some(SelectorOverlay::prompt(id, prompt));
            effects.request_redraw = true;
        }
        UiCommand::OpenSelector { id, title, items } => {
            ui.close_search(terminal_runtime, active_pane);
            ui.exit_visual_mode(terminal_runtime, active_pane);
            ui.leader_deadline = None;
            ui.key_sequences.clear();
            ui.ime_preedit = None;
            ui.selector = Some(SelectorOverlay::new(id, title, items));
        }
        UiCommand::SetPaneBadge { pane, badge } => ui.set_pane_badge(pane, badge),
        UiCommand::OpenSearch { pane } => {
            ui.key_sequences.clear();
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
    fn obsolete_prompt_close_or_completion_cannot_affect_a_new_overlay() {
        let pane = PaneId(1);
        let mut terminals = test_terminal_runtime([pane]);
        let mut ui = test_ui_state();
        ui.selector = Some(SelectorOverlay::prompt(
            2,
            toyoterm_api::InputPrompt {
                dynamic: true,
                ..toyoterm_api::InputPrompt::default()
            },
        ));
        apply_ui_command(
            UiCommand::UpdatePromptCompletions {
                id: 1,
                revision: 0,
                items: vec!["old".into()],
            },
            Some(pane),
            &mut terminals,
            &mut ui,
        );
        apply_ui_command(
            UiCommand::ClosePrompt { id: 1 },
            Some(pane),
            &mut terminals,
            &mut ui,
        );
        assert_eq!(ui.selector.as_ref().unwrap().id, 2);
        assert!(ui.selector.as_ref().unwrap().items.is_empty());
        apply_ui_command(
            UiCommand::ClosePrompt { id: 2 },
            Some(pane),
            &mut terminals,
            &mut ui,
        );
        assert!(ui.selector.is_none());
        apply_ui_command(
            UiCommand::UpdatePromptCompletions {
                id: 2,
                revision: 0,
                items: vec!["late".into()],
            },
            Some(pane),
            &mut terminals,
            &mut ui,
        );
        assert!(ui.selector.is_none());
    }

    #[test]
    fn panels_filter_read_only_lines_update_in_place_and_only_deliver_dismissal() {
        let pane = PaneId(1);
        let mut terminals = test_terminal_runtime([pane]);
        let mut ui = test_ui_state();
        apply_ui_command(
            UiCommand::OpenPanel {
                id: 7,
                title: "Tasks".into(),
                lines: vec!["one".into(), "two".into()],
            },
            Some(pane),
            &mut terminals,
            &mut ui,
        );
        handle_selector_input(
            &Key::Character("one".into()),
            Some("one"),
            ModifiersState::empty(),
            &mut ui,
        );
        assert_eq!(ui.selector.as_ref().unwrap().items, vec!["one", "two"]);
        assert!(ui.selector.as_ref().unwrap().result().is_none());
        apply_ui_command(
            UiCommand::UpdatePanel {
                id: 8,
                title: Some("stale".into()),
                lines: vec!["stale".into()],
            },
            Some(pane),
            &mut terminals,
            &mut ui,
        );
        assert_eq!(ui.selector.as_ref().unwrap().title, "Tasks");
        apply_ui_command(
            UiCommand::UpdatePanel {
                id: 7,
                title: None,
                lines: vec!["one updated".into()],
            },
            Some(pane),
            &mut terminals,
            &mut ui,
        );
        assert_eq!(ui.selector.as_ref().unwrap().query, "one");
        assert_eq!(ui.selector.as_ref().unwrap().items, vec!["one updated"]);
        let completion = handle_selector_input(
            &Key::Named(NamedKey::Enter),
            None,
            ModifiersState::empty(),
            &mut ui,
        )
        .unwrap();
        assert_eq!(completion.id, 7);
        assert!(completion.selection.is_none());
        apply_ui_command(
            UiCommand::OpenSelector {
                id: 8,
                title: "Other".into(),
                items: vec!["keep".into()],
            },
            Some(pane),
            &mut terminals,
            &mut ui,
        );
        apply_ui_command(
            UiCommand::UpdatePanel {
                id: 7,
                title: None,
                lines: vec!["late".into()],
            },
            Some(pane),
            &mut terminals,
            &mut ui,
        );
        assert_eq!(ui.selector.as_ref().unwrap().items, vec!["keep"]);
        // A non-panel with a matching numeric ID must also reject panel updates.
        apply_ui_command(
            UiCommand::UpdatePanel {
                id: 8,
                title: Some("wrong kind".into()),
                lines: vec!["wrong kind".into()],
            },
            Some(pane),
            &mut terminals,
            &mut ui,
        );
        assert_eq!(ui.selector.as_ref().unwrap().items, vec!["keep"]);
    }

    #[test]
    fn empty_panels_close_with_control_g_and_ignore_updates_after_close() {
        let pane = PaneId(1);
        let mut terminals = test_terminal_runtime([pane]);
        let mut ui = test_ui_state();
        apply_ui_command(
            UiCommand::OpenPanel {
                id: 9,
                title: "Empty".into(),
                lines: vec![],
            },
            Some(pane),
            &mut terminals,
            &mut ui,
        );
        let completion = handle_selector_input(
            &Key::Character("g".into()),
            None,
            ModifiersState::CONTROL,
            &mut ui,
        )
        .unwrap();
        assert_eq!(completion.id, 9);
        assert!(completion.selection.is_none());
        apply_ui_command(
            UiCommand::UpdatePanel {
                id: 9,
                title: None,
                lines: vec!["late".into()],
            },
            Some(pane),
            &mut terminals,
            &mut ui,
        );
        assert!(ui.selector.is_none());
    }

    #[test]
    fn prompt_keys_complete_recall_edit_and_deliver_only_a_final_result() {
        let pane = PaneId(1);
        let mut terminals = test_terminal_runtime([pane]);
        let mut ui = test_ui_state();
        apply_ui_command(
            UiCommand::OpenPrompt {
                id: 51,
                prompt: toyoterm_api::InputPrompt {
                    initial: "ca".into(),
                    completions: vec!["cat".into(), "car".into()],
                    history: vec!["old".into()],
                    ..toyoterm_api::InputPrompt::default()
                },
            },
            Some(pane),
            &mut terminals,
            &mut ui,
        );
        let press = |key, modifiers, ui: &mut UiState| {
            handle_selector_input(&Key::Named(key), None, modifiers, ui)
        };
        assert!(press(NamedKey::ArrowDown, ModifiersState::empty(), &mut ui).is_none());
        assert!(press(NamedKey::Tab, ModifiersState::empty(), &mut ui).is_none());
        assert_eq!(ui.selector.as_ref().unwrap().query, "car");
        press(NamedKey::ArrowUp, ModifiersState::ALT, &mut ui);
        assert_eq!(ui.selector.as_ref().unwrap().query, "old");
        press(NamedKey::ArrowDown, ModifiersState::ALT, &mut ui);
        assert_eq!(ui.selector.as_ref().unwrap().query, "car");
        press(NamedKey::Home, ModifiersState::empty(), &mut ui);
        handle_selector_input(
            &Key::Character("猫".into()),
            Some("猫"),
            ModifiersState::empty(),
            &mut ui,
        );
        assert_eq!(ui.selector.as_ref().unwrap().query, "猫car");
        ui.ime_preedit = Some("変換".into());
        assert!(press(NamedKey::Enter, ModifiersState::empty(), &mut ui).is_none());
        assert!(ui.selector.is_some());
        ui.ime_preedit = None;
        let completion = press(NamedKey::Enter, ModifiersState::empty(), &mut ui).unwrap();
        assert_eq!(completion.id, 51);
        assert_eq!(completion.selection.as_deref(), Some("猫car"));
        assert!(ui.selector.is_none());
        ui.selector = Some(SelectorOverlay::prompt(
            52,
            toyoterm_api::InputPrompt::default(),
        ));
        let completion = press(NamedKey::Escape, ModifiersState::empty(), &mut ui).unwrap();
        assert_eq!(completion.selection, None);
    }

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
