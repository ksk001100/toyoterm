use super::*;

#[derive(Debug, PartialEq)]
enum KeybindingDispatch {
    Native(NativeAction),
    Ruby(String),
    Unassigned,
}

fn resolve_keybinding(
    snapshot: &ScriptSnapshot,
    keys: impl IntoIterator<Item = String>,
    visual_mode: bool,
) -> KeybindingDispatch {
    for key in keys {
        if let Some(action) = snapshot.native_actions.get(&key).cloned() {
            if !visual_mode
                && matches!(
                    &action,
                    NativeAction::EndVisualSelection
                        | NativeAction::SelectVisualSelection
                        | NativeAction::MoveVisualSelection(_)
                        | NativeAction::YankSelection
                )
            {
                continue;
            }
            return KeybindingDispatch::Native(action);
        }
        if snapshot.keybindings.contains(&key) {
            return KeybindingDispatch::Ruby(key);
        }
    }
    KeybindingDispatch::Unassigned
}

fn visual_line_end_column(snapshot: &toyoterm_terminal::TerminalSnapshot, row: u16) -> u16 {
    snapshot
        .cells
        .get(usize::from(row))
        .and_then(|cells| cells.last())
        .map(|cell| {
            cell.column
                .saturating_add(u16::from(cell.width.max(1)))
                .saturating_sub(1)
        })
        .unwrap_or(0)
        .min(snapshot.columns.saturating_sub(1))
}

fn ruby_event_from_mux_event(event: MuxEvent) -> Option<RubyEvent> {
    match event {
        MuxEvent::WorkspaceChanged { workspace } => {
            let mut event = RubyEvent::new("workspace_changed");
            event.workspace = Some(workspace);
            Some(event)
        }
        MuxEvent::WindowCreated { window } => {
            let mut event = RubyEvent::new("window_created");
            event.window = Some(window);
            Some(event)
        }
        MuxEvent::WindowClosed { window } => {
            let mut event = RubyEvent::new("window_closed");
            event.window = Some(window);
            Some(event)
        }
        MuxEvent::TabCreated { tab } => {
            let mut event = RubyEvent::new("tab_created");
            event.tab = Some(tab);
            Some(event)
        }
        MuxEvent::TabClosed { tab } => {
            let mut event = RubyEvent::new("tab_closed");
            event.tab = Some(tab);
            Some(event)
        }
        MuxEvent::PaneCreated { pane } => {
            let mut event = RubyEvent::new("pane_created");
            event.pane = Some(pane);
            Some(event)
        }
        MuxEvent::PaneClosed { pane } => {
            let mut event = RubyEvent::new("pane_closed");
            event.pane = Some(pane);
            Some(event)
        }
        MuxEvent::PaneFocused { pane } => {
            let mut event = RubyEvent::new("pane_focused");
            event.pane = Some(pane);
            Some(event)
        }
        MuxEvent::TextQueued { .. } => None,
    }
}

pub(super) fn dispatch_coordinator_command(
    mux: &mut Mux,
    runtime_events: &mut VecDeque<RubyEvent>,
    command: Command,
) -> Result<(), String> {
    mux.dispatch(command).map_err(|error| error.to_string())?;
    runtime_events.extend(mux.drain_events().filter_map(ruby_event_from_mux_event));
    Ok(())
}

impl ToyotermApplication {
    pub(super) fn start_visual_selection(&mut self) {
        self.start_visual_mode();
        self.select_visual_selection();
    }

    pub(super) fn start_visual_mode(&mut self) {
        let Some((cursor, snapshot)) = self
            .active_terminal()
            .map(|terminal| (terminal.cursor(), terminal.snapshot()))
        else {
            return;
        };
        let position = VisualPosition {
            column: cursor.column.min(snapshot.columns.saturating_sub(1)),
            row: cursor.row.min(snapshot.rows.saturating_sub(1)),
        };
        if let Some(terminal) = self.active_terminal_mut() {
            terminal.clear_selection();
        }
        self.visual_selection = Some(VisualSelection {
            anchor: None,
            current: position,
        });
    }

    pub(super) fn select_visual_selection(&mut self) {
        let Some(mut visual) = self.visual_selection else {
            return;
        };
        visual.anchor = Some(visual.current);
        if let Some(terminal) = self.active_terminal_mut() {
            terminal.start_selection(
                visual.current.column,
                visual.current.row,
                SelectionKind::Simple,
            );
        }
        self.visual_selection = Some(visual);
    }

    pub(super) fn exit_visual_mode(&mut self) {
        if self.visual_selection.take().is_some()
            && let Some(terminal) = self.active_terminal_mut()
        {
            terminal.clear_selection();
        }
    }

    pub(super) fn move_visual_selection(&mut self, motion: SelectionMotion) {
        let Some(mut selection) = self.visual_selection else {
            return;
        };
        let Some(snapshot) = self.active_terminal().map(TerminalBackend::snapshot) else {
            return;
        };
        let max_column = snapshot.columns.saturating_sub(1);
        let max_row = snapshot.rows.saturating_sub(1);
        let mut scroll = 0;
        match motion {
            SelectionMotion::Left => {
                selection.current.column = selection.current.column.saturating_sub(1)
            }
            SelectionMotion::Right => {
                selection.current.column =
                    selection.current.column.saturating_add(1).min(max_column)
            }
            SelectionMotion::Up => {
                if selection.current.row == 0 {
                    scroll = 1;
                } else {
                    selection.current.row -= 1;
                }
            }
            SelectionMotion::Down => {
                if selection.current.row == max_row {
                    scroll = -1;
                } else {
                    selection.current.row += 1;
                }
            }
            SelectionMotion::LineStart => selection.current.column = 0,
            SelectionMotion::LineEnd => {
                selection.current.column = visual_line_end_column(&snapshot, selection.current.row);
            }
        }
        if let Some(terminal) = self.active_terminal_mut()
            && selection.anchor.is_some()
        {
            if scroll != 0 {
                terminal.scroll_display(scroll);
            }
            terminal.update_selection(selection.current.column, selection.current.row);
        } else if scroll != 0
            && let Some(terminal) = self.active_terminal_mut()
        {
            terminal.scroll_display(scroll);
        }
        self.visual_selection = Some(selection);
    }

    pub(super) fn yank_selection(&mut self) -> Result<(), String> {
        if !self
            .visual_selection
            .as_ref()
            .is_some_and(|visual| visual.anchor.is_some())
        {
            return Ok(());
        }
        self.copy_selection()?;
        self.exit_visual_mode();
        Ok(())
    }

    pub(super) fn handle_keybinding(
        &mut self,
        event: &KeyEvent,
        modifiers: ModifiersState,
    ) -> Result<bool, String> {
        self.handle_keybinding_candidates(keybinding_names(event, modifiers))
    }

    pub(super) fn handle_keybinding_candidates(
        &mut self,
        keys: Vec<String>,
    ) -> Result<bool, String> {
        let pane = self
            .mux
            .current_pane()
            .ok_or_else(|| "mux has no current pane".to_owned())?;
        match resolve_keybinding(&self.script_snapshot, keys, self.visual_selection.is_some()) {
            KeybindingDispatch::Native(action) => {
                self.execute_native_action(action)?;
                Ok(true)
            }
            KeybindingDispatch::Ruby(key) => {
                self.submit_script(ScriptInvocation::KeyBinding { key, pane })?;
                Ok(true)
            }
            KeybindingDispatch::Unassigned => Ok(false),
        }
    }

    pub(super) fn handle_leader_key(
        &mut self,
        event: &KeyEvent,
        modifiers: ModifiersState,
    ) -> Result<bool, String> {
        let now = Instant::now();
        if let Some(deadline) = self.leader_deadline.take() {
            // A key repeat of the prefix must neither complete nor cancel the
            // leader sequence.  In particular, a user may keep a modifier
            // held while releasing and pressing the prefix key again (for
            // example Ctrl+J, Ctrl+J).  Wayland can emit a repeat after a
            // short delay before that release arrives.  Preserve the original
            // deadline so the subsequent physical press can still match, but
            // never extend the timeout or dispatch an action from the repeat.
            if event.repeat {
                if now <= deadline {
                    self.leader_deadline = Some(deadline);
                }
                return Ok(true);
            }
            if now <= deadline {
                let candidates = keybinding_names(event, modifiers)
                    .into_iter()
                    .map(|key| format!("LEADER+{key}"))
                    .collect();
                if self.handle_keybinding_candidates(candidates)? {
                    return Ok(true);
                }
            }
            // An unmatched or expired suffix is processed normally below by
            // the caller. Only the leader prefix itself is discarded.
            return Ok(false);
        }

        let Some(leader) = self.script_snapshot.config.leader.as_ref() else {
            return Ok(false);
        };
        let matches_leader = keybinding_names(event, modifiers)
            .iter()
            .any(|key| key == &leader.key);
        if event.repeat {
            // Holding the prefix must not leak repeated prefix bytes to the PTY.
            return Ok(matches_leader);
        }
        if !matches_leader {
            return Ok(false);
        }
        self.leader_deadline = Some(
            now.checked_add(Duration::from_millis(leader.timeout_ms))
                .unwrap_or(now),
        );
        Ok(true)
    }

    pub(super) fn execute_native_action(&mut self, action: NativeAction) -> Result<(), String> {
        match action {
            NativeAction::NewTab => self.dispatch_gui_command(Command::NewTab),
            NativeAction::ClosePane => {
                let pane = self
                    .mux
                    .current_pane()
                    .ok_or_else(|| "mux has no current pane".to_owned())?;
                self.dispatch_gui_command(Command::ClosePane(pane))
            }
            NativeAction::CloseTab => {
                let tab = self
                    .mux
                    .current_tab()
                    .ok_or_else(|| "mux has no current tab".to_owned())?;
                self.dispatch_gui_command(Command::CloseTab(tab))
            }
            NativeAction::NewWorkspace => self.create_workspace(),
            NativeAction::ReloadConfig => self.reload_config_with_notification(),
            NativeAction::Search => self.open_search(),
            NativeAction::MaximizeWindow => self.maximize_window(),
            NativeAction::ToggleMaximize => self.toggle_maximize_window(),
            NativeAction::MinimizeWindow => self.minimize_window(),
            NativeAction::ToggleFullscreen => self.toggle_fullscreen(),
            NativeAction::NextTab => self.cycle_tab(false),
            NativeAction::PreviousTab => self.cycle_tab(true),
            NativeAction::NextWorkspace => self.cycle_workspace(false),
            NativeAction::PreviousWorkspace => self.cycle_workspace(true),
            NativeAction::CopySelection => self.copy_selection(),
            NativeAction::PasteClipboard => self.paste_clipboard(),
            NativeAction::StartVisualSelection => {
                self.start_visual_selection();
                Ok(())
            }
            NativeAction::StartVisualMode => {
                self.start_visual_mode();
                Ok(())
            }
            NativeAction::ToggleVisualMode => {
                if self.visual_selection.is_some() {
                    self.exit_visual_mode();
                } else {
                    self.start_visual_mode();
                }
                Ok(())
            }
            NativeAction::SelectVisualSelection => {
                self.select_visual_selection();
                Ok(())
            }
            NativeAction::EndVisualSelection => {
                self.exit_visual_mode();
                Ok(())
            }
            NativeAction::MoveVisualSelection(motion) => {
                self.move_visual_selection(motion);
                Ok(())
            }
            NativeAction::YankSelection => self.yank_selection(),
            NativeAction::UserCommand(name) => self.execute_user_command(&name),
            NativeAction::Split(direction) => self.split_active_pane(direction),
            NativeAction::ActivatePane(direction) => self.focus_neighbor(direction),
        }
    }

    pub(super) fn maximize_window(&mut self) -> Result<(), String> {
        let window = self
            .window
            .as_ref()
            .ok_or_else(|| "native window is not available".to_owned())?;
        window.set_maximized(true);
        Ok(())
    }

    pub(super) fn toggle_maximize_window(&mut self) -> Result<(), String> {
        let window = self
            .window
            .as_ref()
            .ok_or_else(|| "native window is not available".to_owned())?;
        window.set_maximized(!window.is_maximized());
        Ok(())
    }

    pub(super) fn minimize_window(&mut self) -> Result<(), String> {
        let window = self
            .window
            .as_ref()
            .ok_or_else(|| "native window is not available".to_owned())?;
        window.set_minimized(true);
        Ok(())
    }

    pub(super) fn toggle_fullscreen(&mut self) -> Result<(), String> {
        let window = self
            .window
            .as_ref()
            .ok_or_else(|| "native window is not available".to_owned())?;
        let fullscreen = if window.fullscreen().is_some() {
            None
        } else {
            Some(Fullscreen::Borderless(window.current_monitor()))
        };
        window.set_fullscreen(fullscreen);
        Ok(())
    }

    pub(super) fn open_search(&mut self) -> Result<(), String> {
        self.close_search();
        self.search_open = true;
        self.search_query.clear();
        self.search_result = SearchResult::default();
        if let Some(terminal) = self.active_terminal_mut() {
            terminal.clear_search();
        }
        Ok(())
    }

    pub(super) fn close_search(&mut self) {
        self.search_open = false;
        self.search_query.clear();
        self.search_result = SearchResult::default();
        if let Some(terminal) = self.active_terminal_mut() {
            terminal.clear_search();
        }
    }

    pub(super) fn refresh_search(&mut self, direction: SearchDirection) {
        let query = self.search_query.clone();
        self.search_result = self
            .active_terminal_mut()
            .map(|terminal| terminal.search(&query, direction))
            .unwrap_or_default();
    }

    pub(super) fn handle_search_key(&mut self, event: &KeyEvent, modifiers: ModifiersState) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.close_search(),
            Key::Named(NamedKey::Enter) => self.refresh_search(if modifiers.shift_key() {
                SearchDirection::Previous
            } else {
                SearchDirection::Next
            }),
            Key::Named(NamedKey::Backspace) => {
                self.search_query.pop();
                self.refresh_search(SearchDirection::Next);
            }
            Key::Character(text) if !modifiers.control_key() && !modifiers.super_key() => {
                self.search_query
                    .push_str(event.text.as_deref().unwrap_or(text));
                self.refresh_search(SearchDirection::Next);
            }
            _ => {}
        }
    }

    pub(super) fn execute_user_command(&mut self, name: &str) -> Result<(), String> {
        let pane = self
            .mux
            .current_pane()
            .ok_or_else(|| "mux has no current pane".to_owned())?;
        self.submit_script(ScriptInvocation::UserCommand {
            name: name.to_owned(),
            pane,
        })?;
        Ok(())
    }

    pub(super) fn create_workspace(&mut self) -> Result<(), String> {
        let mut suffix = self.mux.workspaces().len() + 1;
        let name = loop {
            let candidate = format!("Workspace {suffix}");
            if self
                .mux
                .workspaces()
                .into_iter()
                .all(|workspace| self.mux.workspace_name(workspace) != Some(candidate.as_str()))
            {
                break candidate;
            }
            suffix += 1;
        };
        self.dispatch_gui_command(Command::SwitchWorkspace(name))
    }

    pub(super) fn cycle_workspace(&mut self, backwards: bool) -> Result<(), String> {
        let workspaces = self.mux.workspaces();
        let current = self.mux.current_workspace();
        let current_index = workspaces
            .iter()
            .position(|workspace| *workspace == current)
            .ok_or_else(|| format!("active workspace {current} is not registered"))?;
        let next_index = if backwards {
            (current_index + workspaces.len() - 1) % workspaces.len()
        } else {
            (current_index + 1) % workspaces.len()
        };
        self.dispatch_gui_command(Command::ActivateWorkspace(workspaces[next_index]))
    }

    pub(super) fn cycle_tab(&mut self, backwards: bool) -> Result<(), String> {
        let window = self
            .mux
            .current_window()
            .ok_or_else(|| "mux has no current window".to_owned())?;
        let current = self
            .mux
            .current_tab()
            .ok_or_else(|| "mux has no current tab".to_owned())?;
        let tabs = self
            .mux
            .tabs(window)
            .ok_or_else(|| format!("unknown window {window}"))?;
        let current_index = tabs
            .iter()
            .position(|tab| *tab == current)
            .ok_or_else(|| format!("active tab {current} is not in window {window}"))?;
        let next_index = if backwards {
            (current_index + tabs.len() - 1) % tabs.len()
        } else {
            (current_index + 1) % tabs.len()
        };
        let next = tabs[next_index];
        self.dispatch_gui_command(Command::ActivateTab(next))
    }

    pub(super) fn dispatch_gui_command(&mut self, command: Command) -> Result<(), String> {
        let previous_pane = self.mux.current_pane();
        dispatch_coordinator_command(&mut self.mux, &mut self.runtime_events, command)?;
        if self.mux.current_pane() != previous_pane {
            self.exit_visual_mode();
            self.ime_preedit = None;
        }
        self.reconcile_pane_runtimes()?;
        self.deliver_runtime_events()?;
        if let Some(window) = self.window.clone() {
            self.sync_active_renderer(window.scale_factor());
            window.request_redraw();
        }
        Ok(())
    }

    pub(super) fn handle_ipc_request(&mut self, request: &IpcRequest) -> Result<String, String> {
        match request {
            IpcRequest::List => return Ok(self.mux.summary()),
            IpcRequest::ListPanes => return Ok(self.ipc_pane_list()),
            IpcRequest::Eval(_) | IpcRequest::Reload => {
                return Err("script IPC request reached native command handler".to_owned());
            }
            IpcRequest::SendText { .. }
            | IpcRequest::Split { .. }
            | IpcRequest::ActivateWorkspace(_) => {}
        }
        let command = request
            .native_command(self.mux.current_pane())?
            .ok_or_else(|| "IPC request has no native command".to_owned())?;
        match command {
            NativeCommand::Mux(command) => {
                self.dispatch_gui_command(command)?;
                self.flush_mux_input()?;
            }
            NativeCommand::ReloadConfig => self.reload_config_with_notification()?,
            NativeCommand::SetPaneBadge { .. } => {
                return Err("pane badge commands are not exposed over IPC".to_owned());
            }
            NativeCommand::ClipboardWrite(_) => {
                return Err("clipboard commands are not exposed over IPC".to_owned());
            }
        }
        Ok("ok".to_owned())
    }

    pub(super) fn ipc_pane_list(&self) -> String {
        let active = self.mux.current_pane();
        let mut panes = self.mux.pane_ids().collect::<Vec<_>>();
        panes.sort_unstable();
        let mut output = String::from("ID\tACTIVE\tPID\tCWD\tTITLE");
        for pane in panes {
            let runtime = self.pane_runtimes.get(&pane);
            let pid = runtime
                .and_then(|runtime| runtime.process_id)
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "-".into());
            let cwd = runtime
                .and_then(|runtime| runtime.cwd.as_ref())
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "-".into())
                .replace(['\t', '\n'], " ");
            let title = runtime
                .map(|runtime| runtime.title.replace(['\t', '\n'], " "))
                .unwrap_or_else(|| format!("Pane {}", pane.0));
            output.push_str(&format!(
                "\n{}\t{}\t{}\t{}\t{}",
                pane.0,
                if active == Some(pane) { "*" } else { "" },
                pid,
                cwd,
                title
            ));
        }
        output
    }

    pub(super) fn split_active_pane(&mut self, direction: SplitDirection) -> Result<(), String> {
        let pane = self
            .mux
            .current_pane()
            .ok_or_else(|| "mux has no current pane".to_owned())?;
        self.dispatch_gui_command(Command::Split { pane, direction })
    }

    pub(super) fn focus_neighbor(&mut self, direction: SplitDirection) -> Result<(), String> {
        let pane = self
            .mux
            .current_pane()
            .ok_or_else(|| "mux has no current pane".to_owned())?;
        if let Some(neighbor) = self.pane_layout.neighbor(pane, direction) {
            self.dispatch_gui_command(Command::ActivatePane(neighbor))?;
        }
        Ok(())
    }

    pub(super) fn reload_config_with_notification(&mut self) -> Result<(), String> {
        self.submit_script(ScriptInvocation::Reload).map(|_| ())
    }

    pub(super) fn apply_script_snapshot(&mut self, snapshot: ScriptSnapshot) -> Result<(), String> {
        let config = snapshot.config.clone();
        let previous_opacity = self.script_snapshot.config.window.opacity;
        self.leader_deadline = None;
        let render_style = RenderStyle::from_hex_with_ui(
            &config.font.family,
            config.font.fallback.clone(),
            config.font.weight,
            [
                &config.colors.background,
                &config.colors.foreground,
                &config.colors.cursor,
                &config.colors.selection,
                &config.colors.tab_bar,
                &config.colors.tab_active,
                &config.colors.tab_inactive,
                &config.colors.workspace_bar,
                &config.colors.status_bar,
                &config.colors.pane_border,
                &config.colors.search_match,
                &config.colors.search_match_active,
            ],
            &config.colors.ansi,
            config.window.opacity,
            config.ui.active_pane_border_width,
        )
        .map_err(|error| error.to_string())?;
        let font_scale = f64::from(config.font.size) / 14.0;
        self.cell_metrics.width = 9.0 * font_scale;
        self.cell_metrics.height = f64::from(config.font.size * config.ui.line_height);
        self.cell_metrics.horizontal_padding = config.ui.padding_x.round() as u32;
        self.cell_metrics.vertical_padding = config.ui.padding_y.round() as u32;
        self.cell_metrics.font_size = config.font.size;
        for runtime in self.pane_runtimes.values_mut() {
            runtime
                .terminal
                .set_scrollback_lines(config.scrollback_lines);
        }
        self.render_style = render_style.clone();
        self.script_snapshot = snapshot;
        self.status_text.clear();
        self.next_status_at = self
            .script_snapshot
            .config
            .status_interval
            .map(|_| Instant::now());
        if let Some(window) = self.window.clone() {
            window.set_transparent(config.window.opacity < 1.0);
            window.set_decorations(config.window.decorations);
            window.set_resizable(config.window.resizable);
            window.set_window_level(if config.window.always_on_top {
                winit::window::WindowLevel::AlwaysOnTop
            } else {
                winit::window::WindowLevel::Normal
            });
            let transparency_mode_changed =
                (previous_opacity < 1.0) != (config.window.opacity < 1.0);
            if transparency_mode_changed {
                self.replace_renderer(render_style.clone())?;
            } else if let Some(renderer) = self.renderer.as_mut() {
                renderer.set_style(render_style);
                self.cell_metrics.width =
                    f64::from(renderer.terminal_cell_width(self.cell_metrics.font_size));
            }
            self.resize_panes(window.inner_size(), window.scale_factor())?;
            self.sync_active_renderer(window.scale_factor());
            window.request_redraw();
        }
        self.emit_script_event("config_reloaded")?;
        Ok(())
    }

    pub(super) fn emit_script_event(&mut self, name: &str) -> Result<(), String> {
        let pane = self
            .mux
            .current_pane()
            .ok_or_else(|| "mux has no current pane".to_owned())?;
        let name = match name {
            "app_started" => "app_started",
            "config_reloaded" => "config_reloaded",
            _ => return Err(format!("unsupported application event {name}")),
        };
        let mut event = RubyEvent::new(name);
        event.pane = Some(pane);
        self.runtime_events.push_back(event);
        self.deliver_runtime_events()
    }

    pub(super) fn collect_mux_events(&mut self) {
        self.runtime_events.extend(
            self.mux
                .drain_events()
                .filter_map(ruby_event_from_mux_event),
        );
    }

    pub(super) fn deliver_runtime_events(&mut self) -> Result<(), String> {
        const MAX_EVENTS_PER_TURN: usize = 1_024;
        let mut delivered = 0;
        self.collect_mux_events();
        while let Some(event) = self.runtime_events.pop_front() {
            delivered += 1;
            if delivered > MAX_EVENTS_PER_TURN {
                return Err("Ruby runtime event delivery exceeded 1024 events".to_owned());
            }
            if !self.script_snapshot.event_names.contains(event.name) {
                continue;
            }
            self.submit_script(ScriptInvocation::Event(event))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event_names(events: &VecDeque<RubyEvent>) -> Vec<&'static str> {
        events.iter().map(|event| event.name).collect()
    }

    #[test]
    fn coordinator_covers_tab_pane_and_workspace_lifecycles() {
        let mut mux = Mux::new();
        let mut events = VecDeque::new();
        let original_workspace = mux.current_workspace();
        let window = mux.current_window().unwrap();
        let original_tab = mux.current_tab().unwrap();
        let original_pane = mux.current_pane().unwrap();

        dispatch_coordinator_command(&mut mux, &mut events, Command::NewTab).unwrap();
        let new_tab = mux.current_tab().unwrap();
        let new_tab_pane = mux.current_pane().unwrap();
        assert_ne!(new_tab, original_tab);
        assert_eq!(mux.tabs(window).unwrap(), &[original_tab, new_tab]);
        assert_eq!(
            event_names(&events),
            ["tab_created", "pane_created", "pane_focused"]
        );
        events.clear();

        dispatch_coordinator_command(&mut mux, &mut events, Command::ActivateTab(original_tab))
            .unwrap();
        assert_eq!(mux.current_pane(), Some(original_pane));
        dispatch_coordinator_command(&mut mux, &mut events, Command::ActivateTab(new_tab)).unwrap();
        dispatch_coordinator_command(&mut mux, &mut events, Command::CloseTab(new_tab)).unwrap();
        assert_eq!(mux.current_tab(), Some(original_tab));
        assert_eq!(mux.tabs(window).unwrap(), &[original_tab]);
        assert!(!mux.pane_ids().any(|pane| pane == new_tab_pane));
        assert!(event_names(&events).contains(&"tab_closed"));
        events.clear();

        dispatch_coordinator_command(
            &mut mux,
            &mut events,
            Command::Split {
                pane: original_pane,
                direction: SplitDirection::Right,
            },
        )
        .unwrap();
        let split_pane = mux.current_pane().unwrap();
        assert_ne!(split_pane, original_pane);
        dispatch_coordinator_command(&mut mux, &mut events, Command::ActivatePane(original_pane))
            .unwrap();
        assert_eq!(mux.current_pane(), Some(original_pane));
        dispatch_coordinator_command(&mut mux, &mut events, Command::ClosePane(split_pane))
            .unwrap();
        assert_eq!(mux.tab_panes(original_tab).unwrap(), vec![original_pane]);
        assert!(event_names(&events).contains(&"pane_closed"));
        events.clear();

        dispatch_coordinator_command(
            &mut mux,
            &mut events,
            Command::SwitchWorkspace("tests".into()),
        )
        .unwrap();
        let new_workspace = mux.current_workspace();
        assert_ne!(new_workspace, original_workspace);
        dispatch_coordinator_command(
            &mut mux,
            &mut events,
            Command::ActivateWorkspace(original_workspace),
        )
        .unwrap();
        assert_eq!(mux.current_workspace(), original_workspace);
        assert_eq!(
            event_names(&events),
            [
                "workspace_changed",
                "pane_focused",
                "workspace_changed",
                "pane_focused"
            ]
        );
    }

    #[test]
    fn runtime_events_stay_fifo_across_reload_and_callback_commands() {
        let mut mux = Mux::new();
        let mut events = VecDeque::from([RubyEvent::new("title_changed")]);

        dispatch_coordinator_command(&mut mux, &mut events, Command::NewTab).unwrap();
        events.push_back(RubyEvent::new("config_reloaded"));
        let pane = mux.current_pane().unwrap();
        dispatch_coordinator_command(
            &mut mux,
            &mut events,
            Command::Split {
                pane,
                direction: SplitDirection::Down,
            },
        )
        .unwrap();

        assert_eq!(
            event_names(&events),
            [
                "title_changed",
                "tab_created",
                "pane_created",
                "pane_focused",
                "config_reloaded",
                "pane_created",
                "pane_focused"
            ]
        );
    }

    #[test]
    fn unassigned_keys_do_not_schedule_ruby_invocations() {
        let snapshot = ScriptSnapshot {
            config: ToyotermConfig::default(),
            native_actions: HashMap::new(),
            keybindings: HashSet::new(),
            event_names: HashSet::new(),
            user_command_names: HashSet::new(),
            plugins: Vec::new(),
        };
        let mut ruby_invocations = 0;

        let dispatch = resolve_keybinding(&snapshot, ["CTRL+UNASSIGNED".to_owned()], false);
        if matches!(dispatch, KeybindingDispatch::Ruby(_)) {
            ruby_invocations += 1;
        }

        assert_eq!(dispatch, KeybindingDispatch::Unassigned);
        assert_eq!(ruby_invocations, 0);
    }

    #[test]
    fn visual_only_actions_are_skipped_outside_visual_mode() {
        let mut snapshot = ScriptSnapshot {
            config: ToyotermConfig::default(),
            native_actions: HashMap::new(),
            keybindings: HashSet::new(),
            event_names: HashSet::new(),
            user_command_names: HashSet::new(),
            plugins: Vec::new(),
        };
        snapshot.native_actions.insert(
            "H".into(),
            NativeAction::MoveVisualSelection(SelectionMotion::Left),
        );

        assert_eq!(
            resolve_keybinding(&snapshot, ["H".into()], false),
            KeybindingDispatch::Unassigned
        );
        assert_eq!(
            resolve_keybinding(&snapshot, ["H".into()], true),
            KeybindingDispatch::Native(NativeAction::MoveVisualSelection(SelectionMotion::Left))
        );
    }

    #[test]
    fn visual_line_end_stops_at_the_last_content_cell() {
        let mut terminal = AlacrittyTerminalBackend::new(20, 2);
        terminal.advance(b"short\r\nwide: \xe7\x8c\xab");
        let snapshot = terminal.snapshot();

        assert_eq!(visual_line_end_column(&snapshot, 0), 4);
        assert_eq!(visual_line_end_column(&snapshot, 1), 7);
    }
}
