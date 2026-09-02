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
) -> KeybindingDispatch {
    for key in keys {
        if let Some(action) = snapshot.native_actions.get(&key).cloned() {
            return KeybindingDispatch::Native(action);
        }
        if snapshot.keybindings.contains(&key) {
            return KeybindingDispatch::Ruby(key);
        }
    }
    KeybindingDispatch::Unassigned
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
        match resolve_keybinding(&self.script_snapshot, keys) {
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
            NativeAction::ReloadConfig => self.reload_config_with_notification(),
            NativeAction::CommandPalette => {
                self.open_command_palette();
                Ok(())
            }
            NativeAction::UserCommand(name) => self.execute_user_command(&name),
            NativeAction::Split(direction) => self.split_active_pane(direction),
            NativeAction::ActivatePane(direction) => self.focus_neighbor(direction),
        }
    }

    pub(super) fn open_command_palette(&mut self) {
        self.close_search();
        self.palette_open = true;
        self.palette.open();
    }

    pub(super) fn open_ruby_console(&mut self) {
        self.close_search();
        self.palette_open = true;
        self.palette.open_console();
    }

    pub(super) fn palette_items(&self) -> Vec<PaletteItem> {
        let mut items = vec![
            PaletteItem {
                label: "Reload Config".into(),
                action: PaletteAction::ReloadConfig,
            },
            PaletteItem {
                label: "New Tab".into(),
                action: PaletteAction::NewTab,
            },
            PaletteItem {
                label: "Split Right".into(),
                action: PaletteAction::Split(SplitDirection::Right),
            },
            PaletteItem {
                label: "Split Down".into(),
                action: PaletteAction::Split(SplitDirection::Down),
            },
            PaletteItem {
                label: "Close Pane".into(),
                action: PaletteAction::ClosePane,
            },
            PaletteItem {
                label: "Ruby Console".into(),
                action: PaletteAction::RubyConsole,
            },
        ];
        for workspace in self.mux.workspaces() {
            if let Some(name) = self.mux.workspace_name(workspace) {
                items.push(PaletteItem {
                    label: format!("Switch Workspace: {name}"),
                    action: PaletteAction::SwitchWorkspace(name.to_owned()),
                });
            }
        }
        let mut names = self
            .script_snapshot
            .user_command_names
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        names.sort();
        items.extend(names.into_iter().map(|name| PaletteItem {
            label: name.clone(),
            action: PaletteAction::UserCommand(name),
        }));
        items
    }

    pub(super) fn handle_palette_key(&mut self, event: &KeyEvent) -> Result<(), String> {
        match &event.logical_key {
            Key::Named(NamedKey::ArrowUp) => {
                let count = filter_items(&self.palette_items(), self.palette.query()).len();
                self.palette.move_selection(-1, count);
            }
            Key::Named(NamedKey::ArrowDown) => {
                let count = filter_items(&self.palette_items(), self.palette.query()).len();
                self.palette.move_selection(1, count);
            }
            Key::Named(NamedKey::Backspace) => self.palette.backspace(),
            Key::Named(NamedKey::Enter) if self.palette.is_console() => {
                let source = self.palette.take_input();
                if !source.trim().is_empty() {
                    let id = self.submit_script(ScriptInvocation::Eval(source.clone()))?;
                    self.eval_waiters.insert(id, EvalWaiter::Palette(source));
                }
            }
            Key::Named(NamedKey::Enter) => {
                let items = filter_items(&self.palette_items(), self.palette.query());
                if let Some(item) = items.get(self.palette.selected()).cloned() {
                    self.palette_open = false;
                    self.palette.close();
                    self.execute_palette_action(item.action)?;
                }
            }
            Key::Character(text)
                if !self.modifiers.control_key() && !self.modifiers.super_key() =>
            {
                self.palette.insert(text);
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn open_search(&mut self) {
        self.palette_open = false;
        self.palette.close();
        self.search_open = true;
        self.search_query.clear();
        self.search_result = SearchResult::default();
        if let Some(terminal) = self.active_terminal_mut() {
            terminal.clear_search();
        }
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

    pub(super) fn execute_palette_action(&mut self, action: PaletteAction) -> Result<(), String> {
        if let Some(command) = palette_native_command(&action, self.mux.current_pane())? {
            return match command {
                NativeCommand::Mux(command) => self.dispatch_gui_command(command),
                NativeCommand::ReloadConfig => self.reload_config_with_notification(),
                NativeCommand::ClipboardWrite(_) => unreachable!("palette has no clipboard action"),
            };
        }
        match action {
            PaletteAction::ReloadConfig
            | PaletteAction::NewTab
            | PaletteAction::Split(_)
            | PaletteAction::ClosePane
            | PaletteAction::SwitchWorkspace(_) => {
                unreachable!("native palette action was normalized")
            }
            PaletteAction::RubyConsole => {
                self.open_ruby_console();
                Ok(())
            }
            PaletteAction::UserCommand(name) => self.execute_user_command(&name),
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

    pub(super) fn handle_tab_shortcut(
        &mut self,
        event: &KeyEvent,
        modifiers: ModifiersState,
    ) -> Result<bool, String> {
        if let Some(shortcut) =
            gui_management_shortcut(&event.logical_key, modifiers, current_shortcut_platform())
        {
            match shortcut {
                GuiManagementShortcut::ReloadConfig => {
                    self.reload_config_with_notification()?;
                }
                GuiManagementShortcut::CommandPalette => self.open_command_palette(),
                GuiManagementShortcut::Search => self.open_search(),
                GuiManagementShortcut::NewTab => {
                    self.dispatch_gui_command(Command::NewTab)?;
                }
                GuiManagementShortcut::NewWorkspace => {
                    self.create_workspace()?;
                }
                GuiManagementShortcut::CloseTab => {
                    let tab = self
                        .mux
                        .current_tab()
                        .ok_or_else(|| "mux has no current tab".to_owned())?;
                    self.dispatch_gui_command(Command::CloseTab(tab))?;
                }
                GuiManagementShortcut::Split(direction) => self.split_active_pane(direction)?,
                GuiManagementShortcut::ClosePane => {
                    let pane = self
                        .mux
                        .current_pane()
                        .ok_or_else(|| "mux has no current pane".to_owned())?;
                    self.dispatch_gui_command(Command::ClosePane(pane))?;
                }
                GuiManagementShortcut::Focus(direction) => self.focus_neighbor(direction)?,
            }
            return Ok(true);
        }

        if modifiers.control_key() && matches!(&event.logical_key, Key::Named(NamedKey::Tab)) {
            self.cycle_tab(modifiers.shift_key())?;
            return Ok(true);
        }
        if modifiers.control_key() && modifiers.alt_key() {
            let backwards = match &event.logical_key {
                Key::Named(NamedKey::ArrowLeft) => Some(true),
                Key::Named(NamedKey::ArrowRight) => Some(false),
                _ => None,
            };
            if let Some(backwards) = backwards {
                self.cycle_workspace(backwards)?;
                return Ok(true);
            }
        }
        Ok(false)
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
        self.leader_deadline = None;
        let render_style = RenderStyle::from_hex_with_ansi(
            &config.font.family,
            config.font.fallback.clone(),
            config.font.weight,
            [
                &config.colors.background,
                &config.colors.foreground,
                &config.colors.cursor,
                &config.colors.selection,
            ],
            &config.colors.ansi,
            config.window_opacity,
        )
        .map_err(|error| error.to_string())?;
        let font_scale = f64::from(config.font.size) / 14.0;
        self.cell_metrics.width = 9.0 * font_scale;
        self.cell_metrics.height = 18.0 * font_scale;
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
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_style(render_style);
            self.cell_metrics.width =
                f64::from(renderer.terminal_cell_width(self.cell_metrics.font_size));
        }
        if let Some(window) = self.window.clone() {
            window.set_transparent(config.window_opacity < 1.0);
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

        let dispatch = resolve_keybinding(&snapshot, ["CTRL+UNASSIGNED".to_owned()]);
        if matches!(dispatch, KeybindingDispatch::Ruby(_)) {
            ruby_invocations += 1;
        }

        assert_eq!(dispatch, KeybindingDispatch::Unassigned);
        assert_eq!(ruby_invocations, 0);
    }
}
