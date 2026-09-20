use super::*;

mod clipboard_controller;
mod domain_handlers;
mod ingress;
mod ui_controller;

use clipboard_controller::apply_clipboard_command;
use domain_handlers::{apply_pane_command, apply_window_command};
pub(super) use ingress::CommandOrigin;
use ingress::{
    ActionResolution, MuxDispatch, resolve_action_command, resolve_mux_dispatch,
    validate_command_origin,
};
use ui_controller::apply_ui_command;

#[derive(Debug, PartialEq)]
enum KeybindingDispatch {
    Native(NativeAction),
    Ruby(String),
    Unassigned,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ControlEffects {
    pub(super) reload_config: bool,
}

impl ControlEffects {
    fn reload_config() -> Self {
        Self {
            reload_config: true,
        }
    }
}

enum DomainCommand {
    Mux(Command),
    Pane(PaneCommand),
    Window(WindowCommand),
    Ui(UiCommand),
    Clipboard(ClipboardCommand),
    Script(ScriptCommand),
    Config(ConfigCommand),
}

fn route_domain_command(command: NativeCommand) -> Result<DomainCommand, String> {
    match command {
        NativeCommand::Mux(command) => Ok(DomainCommand::Mux(command)),
        NativeCommand::Action(_) => Err("unresolved action reached domain dispatcher".to_owned()),
        NativeCommand::Pane(command) => Ok(DomainCommand::Pane(command)),
        NativeCommand::Window(command) => Ok(DomainCommand::Window(command)),
        NativeCommand::Ui(command) => Ok(DomainCommand::Ui(command)),
        NativeCommand::Clipboard(command) => Ok(DomainCommand::Clipboard(command)),
        NativeCommand::Script(command) => Ok(DomainCommand::Script(command)),
        NativeCommand::Config(command) => Ok(DomainCommand::Config(command)),
    }
}

fn apply_config_command(command: ConfigCommand) -> ControlEffects {
    match command {
        ConfigCommand::Reload => ControlEffects::reload_config(),
    }
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

#[cfg(test)]
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

fn visual_last_cell_column(snapshot: &toyoterm_terminal::TerminalSnapshot, row: u16) -> u16 {
    snapshot
        .cells
        .get(usize::from(row))
        .and_then(|cells| cells.last())
        .map(|cell| cell.column)
        .unwrap_or(0)
        .min(snapshot.columns.saturating_sub(1))
}

fn snap_to_cell_start(snapshot: &toyoterm_terminal::TerminalSnapshot, row: u16, col: u16) -> u16 {
    let Some(cells) = snapshot.cells.get(usize::from(row)) else {
        return 0;
    };
    if cells.is_empty() {
        return 0;
    }
    for cell in cells {
        let width = u16::from(cell.width.max(1));
        if col >= cell.column && col < cell.column.saturating_add(width) {
            return cell.column;
        }
    }
    if let Some(last) = cells.last()
        && col >= last.column
    {
        return last.column;
    }
    0
}

fn visual_next_column(
    snapshot: &toyoterm_terminal::TerminalSnapshot,
    row: u16,
    current_col: u16,
) -> u16 {
    let Some(cells) = snapshot.cells.get(usize::from(row)) else {
        return 0;
    };
    if cells.is_empty() {
        return 0;
    }
    for cell in cells {
        if cell.column > current_col {
            return cell.column;
        }
    }
    cells.last().map(|c| c.column).unwrap_or(0)
}

fn visual_prev_column(
    snapshot: &toyoterm_terminal::TerminalSnapshot,
    row: u16,
    current_col: u16,
) -> u16 {
    let Some(cells) = snapshot.cells.get(usize::from(row)) else {
        return 0;
    };
    if cells.is_empty() {
        return 0;
    }
    for cell in cells.iter().rev() {
        if cell.column < current_col {
            return cell.column;
        }
    }
    0
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CharCategory {
    Whitespace,
    Word,
    Punctuation,
}

fn cell_category(cell: &toyoterm_terminal::TerminalCell) -> CharCategory {
    let Some(first_char) = cell.text.chars().next() else {
        return CharCategory::Whitespace;
    };
    if first_char.is_whitespace() {
        CharCategory::Whitespace
    } else if first_char.is_alphanumeric() || first_char == '_' {
        CharCategory::Word
    } else {
        CharCategory::Punctuation
    }
}

fn cell_index_at_column(cells: &[toyoterm_terminal::TerminalCell], col: u16) -> Option<usize> {
    for (idx, cell) in cells.iter().enumerate() {
        let width = u16::from(cell.width.max(1));
        if col >= cell.column && col < cell.column.saturating_add(width) {
            return Some(idx);
        }
    }
    None
}

fn visual_next_word(
    snapshot: &toyoterm_terminal::TerminalSnapshot,
    row: u16,
    col: u16,
) -> (u16, u16) {
    let current_row = row;
    let current_col = col;

    if let Some(cells) = snapshot.cells.get(usize::from(current_row))
        && !cells.is_empty()
        && let Some(start_idx) = cell_index_at_column(cells, current_col)
    {
        let start_cat = cell_category(&cells[start_idx]);
        if start_cat == CharCategory::Whitespace {
            if let Some(target_idx) = cells[start_idx..]
                .iter()
                .position(|c| cell_category(c) != CharCategory::Whitespace)
            {
                return (cells[start_idx + target_idx].column, current_row);
            }
        } else {
            // Advance past the current word/punctuation run.
            if let Some(after_idx) = cells[start_idx..]
                .iter()
                .position(|c| cell_category(c) != start_cat)
            {
                let actual_idx = start_idx + after_idx;
                let after_cat = cell_category(&cells[actual_idx]);
                if after_cat != CharCategory::Whitespace {
                    return (cells[actual_idx].column, current_row);
                }
                // Advance past any trailing whitespace on the same line.
                if let Some(target_idx) = cells[actual_idx..]
                    .iter()
                    .position(|c| cell_category(c) != CharCategory::Whitespace)
                {
                    return (cells[actual_idx + target_idx].column, current_row);
                }
            }
        }
    }

    // Advance to subsequent rows looking for the next word.
    for r in (current_row + 1)..snapshot.rows {
        let Some(cells) = snapshot.cells.get(usize::from(r)) else {
            return (0, r);
        };
        if cells.is_empty() {
            // An empty line counts as a word boundary in Vim.
            return (0, r);
        }
        if let Some(cell) = cells
            .iter()
            .find(|c| cell_category(c) != CharCategory::Whitespace)
        {
            return (cell.column, r);
        }
    }

    (current_col, current_row)
}

fn visual_prev_word(
    snapshot: &toyoterm_terminal::TerminalSnapshot,
    row: u16,
    col: u16,
) -> (u16, u16) {
    let current_row = row;
    let current_col = col;

    if let Some(cells) = snapshot.cells.get(usize::from(current_row))
        && !cells.is_empty()
        && let Some(p_idx) = cells.iter().rposition(|c| c.column < current_col)
        && let Some((non_ws_idx, cell)) = cells[..=p_idx]
            .iter()
            .enumerate()
            .rev()
            .find(|(_, c)| cell_category(c) != CharCategory::Whitespace)
    {
        let target_cat = cell_category(cell);
        let mut start_idx = non_ws_idx;
        while start_idx > 0 && cell_category(&cells[start_idx - 1]) == target_cat {
            start_idx -= 1;
        }
        return (cells[start_idx].column, current_row);
    }

    // Look backwards across preceding lines.
    for r in (0..current_row).rev() {
        let Some(cells) = snapshot.cells.get(usize::from(r)) else {
            return (0, r);
        };
        if cells.is_empty() {
            return (0, r);
        }
        if let Some((non_ws_idx, cell)) = cells
            .iter()
            .enumerate()
            .rev()
            .find(|(_, c)| cell_category(c) != CharCategory::Whitespace)
        {
            let target_cat = cell_category(cell);
            let mut start_idx = non_ws_idx;
            while start_idx > 0 && cell_category(&cells[start_idx - 1]) == target_cat {
                start_idx -= 1;
            }
            return (cells[start_idx].column, r);
        }
    }

    (0, 0)
}

fn ruby_event_from_mux_event(event: MuxEvent) -> Option<RubyEvent> {
    match event {
        MuxEvent::WorkspaceChanged { workspace } => {
            let mut event = RubyEvent::new(ScriptEventKind::WorkspaceChanged);
            event.workspace = Some(workspace);
            Some(event)
        }
        MuxEvent::WindowCreated { window } => {
            let mut event = RubyEvent::new(ScriptEventKind::WindowCreated);
            event.window = Some(window);
            Some(event)
        }
        MuxEvent::WindowClosed { window } => {
            let mut event = RubyEvent::new(ScriptEventKind::WindowClosed);
            event.window = Some(window);
            Some(event)
        }
        MuxEvent::TabCreated { tab } => {
            let mut event = RubyEvent::new(ScriptEventKind::TabCreated);
            event.tab = Some(tab);
            Some(event)
        }
        MuxEvent::TabClosed { tab } => {
            let mut event = RubyEvent::new(ScriptEventKind::TabClosed);
            event.tab = Some(tab);
            Some(event)
        }
        MuxEvent::PaneCreated { pane } => {
            let mut event = RubyEvent::new(ScriptEventKind::PaneCreated);
            event.pane = Some(pane);
            Some(event)
        }
        MuxEvent::PaneClosed { pane } => {
            let mut event = RubyEvent::new(ScriptEventKind::PaneClosed);
            event.pane = Some(pane);
            Some(event)
        }
        MuxEvent::PaneFocused { pane } => {
            let mut event = RubyEvent::new(ScriptEventKind::PaneFocused);
            event.pane = Some(pane);
            Some(event)
        }
        MuxEvent::TextQueued { .. } => None,
    }
}

pub(super) enum PaneCreation {
    NewWindow(toyoterm_api::WorkspaceId),
    NewTab(toyoterm_api::WindowId),
    Split {
        pane: PaneId,
        direction: SplitDirection,
    },
}

pub(super) fn dispatch_pane_creation(
    mux: &mut Mux,
    runtime_events: &mut VecDeque<RubyEvent>,
    creation: PaneCreation,
) -> Result<PaneId, String> {
    let (command, expected) = match creation {
        PaneCreation::NewWindow(workspace) => (Command::CreateWindow(workspace), "window"),
        PaneCreation::NewTab(window) => (Command::NewTabIn(window), "tab"),
        PaneCreation::Split { pane, direction } => (Command::Split { pane, direction }, "pane"),
    };
    let result = dispatch_coordinator_command(mux, runtime_events, command)?;
    match result {
        CommandResult::Pane(pane) => Ok(pane),
        CommandResult::Window(window) => mux
            .tabs(window)
            .and_then(|tabs| tabs.first())
            .and_then(|tab| mux.tab_panes(*tab))
            .and_then(|panes| panes.into_iter().next())
            .ok_or_else(|| format!("new window {window} has no pane")),
        CommandResult::Tab(tab) => mux
            .tab_panes(tab)
            .and_then(|panes| panes.into_iter().next())
            .ok_or_else(|| format!("new tab {tab} has no pane")),
        _ => Err(format!(
            "pane creation returned an invalid {expected} result"
        )),
    }
}

pub(super) fn dispatch_coordinator_command(
    mux: &mut Mux,
    runtime_events: &mut VecDeque<RubyEvent>,
    command: Command,
) -> Result<CommandResult, String> {
    let result = mux.dispatch(command).map_err(|error| error.to_string())?;
    runtime_events.extend(mux.drain_events().filter_map(ruby_event_from_mux_event));
    Ok(result)
}

impl ToyotermApplication {
    pub(super) fn apply_control_command(
        &mut self,
        command: NativeCommand,
        origin: CommandOrigin,
    ) -> Result<ControlEffects, String> {
        self.scripting.invalidate_context();
        validate_command_origin(origin, &command)?;
        if let NativeCommand::Action(command) = command {
            let ActionResolution {
                activation,
                command,
            } = resolve_action_command(command, origin, &self.mux, &self.ui.pane_layout)?;
            for command in activation {
                self.dispatch_gui_command(command)?;
            }
            return match command {
                Some(command) => self.dispatch_domain_command(command, MuxDispatch::Gui),
                None => Ok(ControlEffects::default()),
            };
        }
        self.dispatch_domain_command(command, resolve_mux_dispatch(origin))
    }

    fn dispatch_domain_command(
        &mut self,
        command: NativeCommand,
        mux_dispatch: MuxDispatch,
    ) -> Result<ControlEffects, String> {
        let effects = match route_domain_command(command)? {
            DomainCommand::Mux(command) => {
                match mux_dispatch {
                    MuxDispatch::Coordinator => {
                        dispatch_coordinator_command(
                            &mut self.mux,
                            &mut self.scripting.runtime_events,
                            command,
                        )?;
                    }
                    MuxDispatch::Gui => self.dispatch_gui_command(command)?,
                }
                ControlEffects::default()
            }
            DomainCommand::Pane(command) => {
                let pane_effects = apply_pane_command(
                    command,
                    &mut self.mux,
                    &mut self.scripting.runtime_events,
                    &mut self.terminal_runtime,
                )?;
                if let Some(search) = pane_effects.search {
                    ui_controller::apply_pane_effect(
                        search,
                        &mut self.terminal_runtime,
                        &mut self.ui,
                    );
                }
                ControlEffects::default()
            }
            DomainCommand::Window(command) => apply_window_command(
                command,
                &mut self.mux,
                &mut self.scripting.runtime_events,
                &mut self.terminal_runtime,
                &mut self.platform,
            )
            .map(|_| ControlEffects::default())?,
            DomainCommand::Ui(command) => {
                let ui_effects = apply_ui_command(
                    command,
                    self.mux.current_pane(),
                    &mut self.terminal_runtime,
                    &mut self.ui,
                );
                if ui_effects.request_redraw
                    && let Some(window) = self.platform.window.as_ref()
                {
                    window.request_redraw();
                }
                ControlEffects::default()
            }
            DomainCommand::Clipboard(command) => apply_clipboard_command(
                command,
                &mut self.platform,
                &mut self.terminal_runtime,
                &mut self.ui,
            )
            .map(|_| ControlEffects::default())?,
            DomainCommand::Script(ScriptCommand::InvokeUserCommand { name, pane }) => {
                self.submit_script(ScriptInvocation::UserCommand { name, pane })?;
                ControlEffects::default()
            }
            DomainCommand::Config(command) => apply_config_command(command),
        };
        Ok(effects)
    }

    pub(super) fn exit_visual_mode(&mut self) {
        let pane = self.mux.current_pane();
        self.ui.exit_visual_mode(&mut self.terminal_runtime, pane);
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
        match resolve_keybinding(
            &self.scripting.snapshot,
            keys,
            self.ui.visual_selection.is_some(),
        ) {
            KeybindingDispatch::Native(action) => {
                let context = ActionContext {
                    workspace: self.mux.current_workspace(),
                    window: self
                        .mux
                        .current_window()
                        .ok_or_else(|| "mux has no current window".to_owned())?,
                    tab: self
                        .mux
                        .current_tab()
                        .ok_or_else(|| "mux has no current tab".to_owned())?,
                    pane,
                };
                let effects = self.apply_control_command(
                    NativeCommand::Action(ActionCommand::Invoke { action, context }),
                    CommandOrigin::Keybinding,
                )?;
                if effects.reload_config {
                    self.reload_config_with_notification()?;
                }
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
        if let Some(deadline) = self.ui.leader_deadline.take() {
            // A key repeat of the prefix must neither complete nor cancel the
            // leader sequence.  In particular, a user may keep a modifier
            // held while releasing and pressing the prefix key again (for
            // example Ctrl+J, Ctrl+J).  Wayland can emit a repeat after a
            // short delay before that release arrives.  Preserve the original
            // deadline so the subsequent physical press can still match, but
            // never extend the timeout or dispatch an action from the repeat.
            if event.repeat {
                if now <= deadline {
                    self.ui.leader_deadline = Some(deadline);
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

        let Some(leader) = self.scripting.snapshot.config.leader.as_ref() else {
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
        self.ui.leader_deadline = Some(
            now.checked_add(Duration::from_millis(leader.timeout_ms))
                .unwrap_or(now),
        );
        Ok(true)
    }

    pub(super) fn close_search(&mut self) {
        ui_controller::close_search(
            self.mux.current_pane(),
            &mut self.terminal_runtime,
            &mut self.ui,
        );
    }

    pub(super) fn refresh_search(&mut self, direction: SearchDirection) {
        ui_controller::refresh_search(
            self.mux.current_pane(),
            direction,
            &mut self.terminal_runtime,
            &mut self.ui,
        );
    }

    pub(super) fn handle_search_key(&mut self, event: &KeyEvent, modifiers: ModifiersState) {
        ui_controller::handle_search_key(
            event,
            modifiers,
            self.mux.current_pane(),
            &mut self.terminal_runtime,
            &mut self.ui,
        );
    }

    pub(super) fn handle_selector_key(
        &mut self,
        event: &KeyEvent,
        modifiers: ModifiersState,
    ) -> Result<(), String> {
        if let Some(completion) = ui_controller::handle_selector_key(event, modifiers, &mut self.ui)
        {
            self.submit_script(ScriptInvocation::SelectCallback {
                id: completion.id,
                selection: completion.selection,
            })?;
        }
        Ok(())
    }

    pub(super) fn dispatch_gui_command(&mut self, command: Command) -> Result<(), String> {
        self.scripting.invalidate_context();
        let previous_pane = self.mux.current_pane();
        dispatch_coordinator_command(&mut self.mux, &mut self.scripting.runtime_events, command)?;
        if self.mux.current_pane() != previous_pane {
            self.exit_visual_mode();
            self.ui.ime_preedit = None;
        }
        self.reconcile_pane_runtimes()?;
        self.deliver_runtime_events()?;
        if let Some(window) = self.platform.window.clone() {
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
        let effects = self.apply_control_command(command, CommandOrigin::Ipc)?;
        self.flush_mux_input()?;
        if effects.reload_config {
            self.reload_config_with_notification()?;
        }
        Ok("ok".to_owned())
    }

    pub(super) fn ipc_pane_list(&self) -> String {
        let active = self.mux.current_pane();
        let mut panes = self.mux.pane_ids().collect::<Vec<_>>();
        panes.sort_unstable();
        let mut output = String::from("ID\tACTIVE\tPID\tCWD\tTITLE");
        for pane in panes {
            let runtime = self.terminal_runtime.pane_runtimes.get(&pane);
            let pid = runtime
                .and_then(|runtime| runtime.process.process_id)
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "-".into());
            let cwd = runtime
                .and_then(|runtime| runtime.metadata.cwd.as_ref())
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "-".into())
                .replace(['\t', '\n'], " ");
            let title = runtime
                .map(|runtime| runtime.metadata.title.replace(['\t', '\n'], " "))
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

    pub(super) fn reload_config_with_notification(&mut self) -> Result<(), String> {
        self.submit_script(ScriptInvocation::Reload).map(|_| ())
    }

    pub(super) fn apply_script_snapshot(&mut self, snapshot: ScriptSnapshot) -> Result<(), String> {
        let config = snapshot.config.clone();
        let previous_opacity = self.scripting.snapshot.config.window.opacity;
        self.ui.leader_deadline = None;
        let mut render_style = RenderStyle::from_hex_with_ui(
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
                &config.colors.zoomed_pane_border,
                &config.colors.search_match,
                &config.colors.search_match_active,
            ],
            &config.colors.ansi,
            config.window.opacity,
            config.ui.active_pane_border_width,
        )
        .map_err(|error| error.to_string())?;
        render_style.background_image =
            config
                .window
                .background_image
                .as_ref()
                .map(|image| toyoterm_render::BackgroundImage {
                    width: image.width,
                    height: image.height,
                    rgba: image.rgba.clone(),
                });
        render_style.background_image_opacity = config.window.background_image_opacity;
        let font_scale = f64::from(config.font.size) / 14.0;
        self.ui.cell_metrics.width = 9.0 * font_scale;
        self.ui.cell_metrics.height = f64::from(config.font.size * config.ui.line_height);
        self.ui.cell_metrics.horizontal_padding = config.ui.padding_x.round() as u32;
        self.ui.cell_metrics.vertical_padding = config.ui.padding_y.round() as u32;
        self.ui.cell_metrics.font_size = config.font.size;
        for runtime in self.terminal_runtime.pane_runtimes.values_mut() {
            runtime
                .terminal
                .set_scrollback_lines(config.scrollback_lines);
            runtime.terminal.set_default_colors(
                render_style.foreground,
                render_style.background,
                render_style.cursor,
                render_style.selection,
                render_style.ansi,
            );
            runtime
                .terminal
                .set_osc52_copy_enabled(config.behavior.allow_osc52_copy);
        }
        self.ui.render_style = render_style.clone();
        self.scripting.snapshot = Arc::new(snapshot);
        self.ui.bar_items.clear();
        self.ui.bar_pending = None;
        self.ui.next_bar_at = self
            .scripting
            .snapshot
            .config
            .status_bars
            .iter()
            .map(|bar| (bar.position, Instant::now()))
            .collect();
        if let Some(window) = self.platform.window.clone() {
            #[cfg(not(target_os = "windows"))]
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
            // Metal supports changing alpha mode on the existing surface.
            // Creating another surface adds a CAMetalLayer while the view
            // retains the old one, whose opaque contents block transparency.
            // Windows keeps the same transparent surface even at opacity 1.0.
            // Replacing it at that boundary can lose compositor transparency.
            if transparency_mode_changed && !cfg!(any(target_os = "macos", target_os = "windows")) {
                self.replace_renderer(render_style.clone())?;
            } else if let Some(renderer) = self.platform.renderer.as_mut() {
                renderer.set_style(render_style);
                self.ui.cell_metrics.width =
                    f64::from(renderer.terminal_cell_width(self.ui.cell_metrics.font_size));
            }
            self.resize_panes(window.inner_size(), window.scale_factor())?;
            self.sync_active_renderer(window.scale_factor());
            window.request_redraw();
        }
        self.emit_script_event(ScriptEventKind::ConfigReloaded)?;
        Ok(())
    }

    pub(super) fn emit_script_event(&mut self, kind: ScriptEventKind) -> Result<(), String> {
        let pane = self
            .mux
            .current_pane()
            .ok_or_else(|| "mux has no current pane".to_owned())?;
        let mut event = RubyEvent::new(kind);
        event.pane = Some(pane);
        self.scripting.runtime_events.push_back(event);
        self.deliver_runtime_events()
    }

    pub(super) fn collect_mux_events(&mut self) {
        self.scripting.runtime_events.extend(
            self.mux
                .drain_events()
                .filter_map(ruby_event_from_mux_event),
        );
    }

    pub(super) fn deliver_runtime_events(&mut self) -> Result<(), String> {
        const MAX_EVENTS_PER_TURN: usize = 1_024;
        let mut delivered = 0;
        self.collect_mux_events();
        while let Some(event) = self.scripting.runtime_events.pop_front() {
            delivered += 1;
            if delivered > MAX_EVENTS_PER_TURN {
                return Err("Ruby runtime event delivery exceeded 1024 events".to_owned());
            }
            if !self.scripting.snapshot.event_names.contains(event.name()) {
                continue;
            }
            self.submit_script(ScriptInvocation::Event(event))?;
        }
        Ok(())
    }
}

#[cfg(test)]
fn test_ui_state() -> UiState {
    UiState {
        pane_layout: PaneLayout::default(),
        tab_layout: TabStripLayout::default(),
        workspace_layout: WorkspaceStripLayout::default(),
        search_open: false,
        search_query: String::new(),
        search_result: SearchResult::default(),
        selector: None,
        config_error_layout: ConfigErrorLayout::default(),
        config_error_notice: None,
        ime_preedit: Some("preedit".into()),
        modifiers: ModifiersState::empty(),
        alt_graph_active: false,
        leader_deadline: None,
        mouse_position: PhysicalPosition::new(0.0, 0.0),
        pressed_mouse_button: None,
        last_mouse_cell: None,
        wheel_line_accumulator: 0.0,
        selecting: false,
        visual_selection: None,
        click_tracker: ClickTracker::default(),
        pending_clipboard_writes: Vec::new(),
        pane_badges: HashMap::new(),
        cell_metrics: CellMetrics::default(),
        bar_items: HashMap::new(),
        bar_pending: None,
        next_bar_at: HashMap::new(),
        terminal_render_pending: false,
        render_style: RenderStyle::default(),
        window_title_override: None,
    }
}

#[cfg(test)]
fn test_terminal_runtime(panes: impl IntoIterator<Item = PaneId>) -> TerminalRuntime {
    TerminalRuntime {
        pane_runtimes: panes
            .into_iter()
            .map(|pane| {
                (
                    pane,
                    PaneRuntime {
                        terminal: AlacrittyTerminalBackend::new(80, 24),
                        process: ProcessRuntime {
                            pty_session: None,
                            process_id: None,
                            exited: false,
                        },
                        metadata: PaneMetadata {
                            title: format!("Pane {}", pane.0),
                            icon_title: None,
                            osc_badge: None,
                            cwd: None,
                            remote_host: None,
                        },
                        protocol: PaneProtocolState::default(),
                    },
                )
            })
            .collect(),
        pending_pane_launches: HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action_context() -> ActionContext {
        ActionContext {
            workspace: toyoterm_api::WorkspaceId(1),
            window: toyoterm_api::WindowId(2),
            tab: toyoterm_api::TabId(3),
            pane: PaneId(4),
        }
    }

    fn launch_spec() -> PaneLaunchSpec {
        PaneLaunchSpec {
            program: Some("shell".into()),
            args: vec!["--login".into()],
            cwd: None,
            environment: Vec::new(),
        }
    }

    #[test]
    fn command_origin_policy_preserves_script_ipc_and_keybinding_permissions() {
        let commands = [
            NativeCommand::Mux(Command::NewTab),
            NativeCommand::Action(ActionCommand::Invoke {
                action: NativeAction::ToggleZoom,
                context: action_context(),
            }),
            NativeCommand::Ui(UiCommand::SetPaneBadge {
                pane: PaneId(1),
                badge: Some("build".into()),
            }),
            NativeCommand::Window(WindowCommand::CreateWithLaunch {
                workspace: toyoterm_api::WorkspaceId(1),
                launch: launch_spec(),
            }),
            NativeCommand::Ui(UiCommand::OpenSelector {
                id: 1,
                title: "pick".into(),
                items: vec!["one".into()],
            }),
            NativeCommand::Clipboard(ClipboardCommand::Write("text".into())),
            NativeCommand::Script(ScriptCommand::InvokeUserCommand {
                name: "build".into(),
                pane: PaneId(1),
            }),
            NativeCommand::Config(ConfigCommand::Reload),
        ];

        for command in &commands {
            assert!(validate_command_origin(CommandOrigin::Script, command).is_ok());
            assert!(validate_command_origin(CommandOrigin::Keybinding, command).is_ok());
        }
        for command in &commands {
            let expected = matches!(command, NativeCommand::Mux(_) | NativeCommand::Config(_));
            assert_eq!(
                validate_command_origin(CommandOrigin::Ipc, command).is_ok(),
                expected
            );
        }
    }

    #[test]
    fn native_command_envelope_routes_every_domain_to_its_handler_boundary() {
        assert!(matches!(
            route_domain_command(NativeCommand::Mux(Command::NewTab)),
            Ok(DomainCommand::Mux(Command::NewTab))
        ));
        assert!(matches!(
            route_domain_command(NativeCommand::Pane(PaneCommand::Search {
                pane: PaneId(1),
                query: "query".into(),
                direction: PaneSearchDirection::Next,
            })),
            Ok(DomainCommand::Pane(PaneCommand::Search { .. }))
        ));
        assert!(matches!(
            route_domain_command(NativeCommand::Window(WindowCommand::Minimize)),
            Ok(DomainCommand::Window(WindowCommand::Minimize))
        ));
        assert!(matches!(
            route_domain_command(NativeCommand::Ui(UiCommand::OpenSearch { pane: PaneId(1) })),
            Ok(DomainCommand::Ui(UiCommand::OpenSearch { .. }))
        ));
        assert!(matches!(
            route_domain_command(NativeCommand::Clipboard(ClipboardCommand::Write(
                "text".into(),
            ))),
            Ok(DomainCommand::Clipboard(ClipboardCommand::Write(_)))
        ));
        assert!(matches!(
            route_domain_command(NativeCommand::Script(ScriptCommand::InvokeUserCommand {
                name: "build".into(),
                pane: PaneId(1),
            },)),
            Ok(DomainCommand::Script(
                ScriptCommand::InvokeUserCommand { .. }
            ))
        ));
        assert!(matches!(
            route_domain_command(NativeCommand::Config(ConfigCommand::Reload)),
            Ok(DomainCommand::Config(ConfigCommand::Reload))
        ));
        assert!(
            route_domain_command(NativeCommand::Action(ActionCommand::Invoke {
                action: NativeAction::ToggleZoom,
                context: action_context(),
            }))
            .is_err()
        );
    }

    #[test]
    fn action_resolution_separates_user_actions_from_context_bound_execution() {
        let mux = Mux::new();
        let context = ActionContext {
            workspace: mux.current_workspace(),
            window: mux.current_window().unwrap(),
            tab: mux.current_tab().unwrap(),
            pane: mux.current_pane().unwrap(),
        };
        let pane_layout = PaneLayout::default();
        let invoke = |action| ActionCommand::Invoke { action, context };

        assert_eq!(
            resolve_action_command(
                invoke(NativeAction::ToggleZoom),
                CommandOrigin::Keybinding,
                &mux,
                &pane_layout,
            ),
            Ok(ActionResolution {
                activation: Vec::new(),
                command: Some(NativeCommand::Mux(Command::ToggleZoom)),
            })
        );
        assert_eq!(
            resolve_action_command(
                invoke(NativeAction::ToggleZoom),
                CommandOrigin::Script,
                &mux,
                &pane_layout,
            ),
            Ok(ActionResolution {
                activation: vec![
                    Command::ActivateWorkspace(context.workspace),
                    Command::ActivateWindow(context.window),
                    Command::ActivateTab(context.tab),
                    Command::ActivatePane(context.pane),
                ],
                command: Some(NativeCommand::Mux(Command::ToggleZoom)),
            })
        );
        assert_eq!(
            resolve_action_command(
                invoke(NativeAction::ReloadConfig),
                CommandOrigin::Script,
                &mux,
                &pane_layout,
            ),
            Ok(ActionResolution {
                activation: Vec::new(),
                command: Some(NativeCommand::Config(ConfigCommand::Reload)),
            })
        );
        assert!(
            resolve_action_command(
                invoke(NativeAction::ToggleZoom),
                CommandOrigin::Ipc,
                &mux,
                &pane_layout,
            )
            .is_err()
        );
    }

    #[test]
    fn action_resolution_routes_actions_to_concrete_domains() {
        let mux = Mux::new();
        let context = ActionContext {
            workspace: mux.current_workspace(),
            window: mux.current_window().unwrap(),
            tab: mux.current_tab().unwrap(),
            pane: mux.current_pane().unwrap(),
        };
        let layout = PaneLayout::default();
        let resolve = |action| {
            resolve_action_command(
                ActionCommand::Invoke { action, context },
                CommandOrigin::Keybinding,
                &mux,
                &layout,
            )
            .unwrap()
            .command
            .unwrap()
        };

        assert!(matches!(
            resolve(NativeAction::Split(SplitDirection::Right)),
            NativeCommand::Mux(Command::Split { .. })
        ));
        assert!(matches!(
            resolve(NativeAction::Search),
            NativeCommand::Ui(UiCommand::OpenSearch { .. })
        ));
        assert!(matches!(
            resolve(NativeAction::ToggleFullscreen),
            NativeCommand::Window(WindowCommand::ToggleFullscreen)
        ));
        assert!(matches!(
            resolve(NativeAction::CopySelection),
            NativeCommand::Clipboard(ClipboardCommand::CopySelection { .. })
        ));
        assert!(matches!(
            resolve(NativeAction::UserCommand("build".into())),
            NativeCommand::Script(ScriptCommand::InvokeUserCommand { .. })
        ));
        assert!(matches!(
            resolve(NativeAction::ReloadConfig),
            NativeCommand::Config(ConfigCommand::Reload)
        ));
    }

    #[test]
    fn script_action_resolution_rejects_a_stale_context_before_dispatch() {
        let mux = Mux::new();
        let mut stale = action_context();
        stale.pane = PaneId(u64::MAX);
        assert!(
            resolve_action_command(
                ActionCommand::Invoke {
                    action: NativeAction::ToggleZoom,
                    context: stale,
                },
                CommandOrigin::Script,
                &mux,
                &PaneLayout::default(),
            )
            .is_err()
        );
    }

    #[test]
    fn config_reload_is_returned_as_an_application_effect() {
        assert_eq!(
            apply_config_command(ConfigCommand::Reload),
            ControlEffects::reload_config()
        );
    }

    #[test]
    fn pane_and_window_handlers_stage_launches_in_the_terminal_domain() {
        let mut mux = Mux::new();
        let mut events = VecDeque::new();
        let mut terminal_runtime = TerminalRuntime {
            pane_runtimes: HashMap::new(),
            pending_pane_launches: HashMap::new(),
        };
        let original_pane = mux.current_pane().unwrap();
        let original_window = mux.current_window().unwrap();
        let mut platform = PlatformState {
            window: None,
            renderer: None,
            occlusion: WindowOcclusion::default(),
            clipboard: None,
            notification_sender: None,
            #[cfg(target_os = "linux")]
            app_id: None,
        };

        domain_handlers::apply_window_command(
            WindowCommand::NewTabWithLaunch {
                window: original_window,
                launch: launch_spec(),
            },
            &mut mux,
            &mut events,
            &mut terminal_runtime,
            &mut platform,
        )
        .unwrap();
        let tab_pane = mux.current_pane().unwrap();
        assert_ne!(tab_pane, original_pane);
        assert!(
            terminal_runtime
                .pending_pane_launches
                .contains_key(&tab_pane)
        );

        let split_pane = domain_handlers::stage_pane_launch(
            PaneCreation::Split {
                pane: tab_pane,
                direction: SplitDirection::Right,
            },
            launch_spec(),
            &mut mux,
            &mut events,
            &mut terminal_runtime,
        )
        .unwrap();
        assert_ne!(split_pane, tab_pane);
        assert!(
            terminal_runtime
                .pending_pane_launches
                .contains_key(&split_pane)
        );

        let workspace = mux.current_workspace();
        let windows_before = mux.workspace_windows(workspace).unwrap().len();
        domain_handlers::apply_window_command(
            WindowCommand::CreateWithLaunch {
                workspace,
                launch: launch_spec(),
            },
            &mut mux,
            &mut events,
            &mut terminal_runtime,
            &mut platform,
        )
        .unwrap();
        let window_pane = mux.current_pane().unwrap();
        assert_eq!(
            mux.workspace_windows(workspace).unwrap().len(),
            windows_before + 1
        );
        assert!(
            terminal_runtime
                .pending_pane_launches
                .contains_key(&window_pane)
        );
    }

    #[test]
    fn callback_actions_reactivate_their_origin_context() {
        let context = ActionContext {
            workspace: toyoterm_api::WorkspaceId(1),
            window: toyoterm_api::WindowId(2),
            tab: toyoterm_api::TabId(3),
            pane: PaneId(4),
        };
        assert_eq!(
            ingress::context_activation_commands(&NativeAction::ToggleZoom, context),
            vec![
                Command::ActivateWorkspace(context.workspace),
                Command::ActivateWindow(context.window),
                Command::ActivateTab(context.tab),
                Command::ActivatePane(context.pane),
            ]
        );
        assert!(
            ingress::context_activation_commands(&NativeAction::ToggleFullscreen, context)
                .is_empty()
        );

        let mut mux = Mux::new();
        let valid = ActionContext {
            workspace: mux.current_workspace(),
            window: mux.current_window().unwrap(),
            tab: mux.current_tab().unwrap(),
            pane: mux.current_pane().unwrap(),
        };
        assert!(ingress::action_context_is_valid(&mux, valid));
        let first_pane = valid.pane;
        let CommandResult::Tab(second_tab) = mux.dispatch(Command::NewTab).unwrap() else {
            panic!("new tab did not return a tab");
        };
        assert!(!ingress::action_context_is_valid(
            &mux,
            ActionContext {
                tab: second_tab,
                pane: first_pane,
                ..valid
            }
        ));
    }

    fn event_names(events: &VecDeque<RubyEvent>) -> Vec<&'static str> {
        events.iter().map(RubyEvent::name).collect()
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
    fn pane_creation_returns_the_new_pane_for_custom_launch_staging() {
        let mut mux = Mux::new();
        let mut events = VecDeque::new();
        let workspace = mux.current_workspace();
        let window = mux.current_window().unwrap();
        let original = mux.current_pane().unwrap();

        let tab_pane =
            dispatch_pane_creation(&mut mux, &mut events, PaneCreation::NewTab(window)).unwrap();
        assert_ne!(tab_pane, original);
        assert_eq!(mux.current_pane(), Some(tab_pane));

        let split_pane = dispatch_pane_creation(
            &mut mux,
            &mut events,
            PaneCreation::Split {
                pane: tab_pane,
                direction: SplitDirection::Right,
            },
        )
        .unwrap();
        assert_ne!(split_pane, tab_pane);
        assert_eq!(mux.current_pane(), Some(split_pane));

        let window_pane =
            dispatch_pane_creation(&mut mux, &mut events, PaneCreation::NewWindow(workspace))
                .unwrap();
        assert_ne!(window_pane, split_pane);
        assert_eq!(mux.current_pane(), Some(window_pane));
        assert_eq!(mux.workspace_windows(workspace).unwrap().len(), 2);
        assert!(event_names(&events).contains(&"tab_created"));
        assert!(event_names(&events).contains(&"pane_created"));
        assert!(event_names(&events).contains(&"window_created"));
    }

    #[test]
    fn runtime_events_stay_fifo_across_reload_and_callback_commands() {
        let mut mux = Mux::new();
        let mut events = VecDeque::from([RubyEvent::new(ScriptEventKind::TitleChanged)]);

        dispatch_coordinator_command(&mut mux, &mut events, Command::NewTab).unwrap();
        events.push_back(RubyEvent::new(ScriptEventKind::ConfigReloaded));
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

    #[test]
    fn visual_navigation_navigates_wide_characters_and_clamps_boundaries() {
        let mut terminal = AlacrittyTerminalBackend::new(20, 3);
        terminal.advance(b"short\r\nwide: \xe7\x8c\xab\xe7\x8a\xac\r\n");
        let snapshot = terminal.snapshot();

        assert_eq!(snap_to_cell_start(&snapshot, 1, 6), 6);
        assert_eq!(snap_to_cell_start(&snapshot, 1, 7), 6);
        assert_eq!(snap_to_cell_start(&snapshot, 1, 8), 8);
        assert_eq!(snap_to_cell_start(&snapshot, 1, 9), 8);
        assert_eq!(snap_to_cell_start(&snapshot, 1, 15), 8);
        assert_eq!(snap_to_cell_start(&snapshot, 2, 5), 0);

        assert_eq!(visual_next_column(&snapshot, 0, 0), 1);
        assert_eq!(visual_next_column(&snapshot, 0, 3), 4);
        assert_eq!(visual_next_column(&snapshot, 0, 4), 4);
        assert_eq!(visual_next_column(&snapshot, 1, 5), 6);
        assert_eq!(visual_next_column(&snapshot, 1, 6), 8);
        assert_eq!(visual_next_column(&snapshot, 1, 8), 8);

        assert_eq!(visual_prev_column(&snapshot, 0, 4), 3);
        assert_eq!(visual_prev_column(&snapshot, 0, 1), 0);
        assert_eq!(visual_prev_column(&snapshot, 0, 0), 0);
        assert_eq!(visual_prev_column(&snapshot, 1, 8), 6);
        assert_eq!(visual_prev_column(&snapshot, 1, 6), 5);

        assert_eq!(visual_last_cell_column(&snapshot, 0), 4);
        assert_eq!(visual_last_cell_column(&snapshot, 1), 8);
        assert_eq!(visual_last_cell_column(&snapshot, 2), 0);
    }

    #[test]
    fn visual_word_navigation_navigates_words_punctuation_empty_lines_and_wide_chars() {
        let mut terminal = AlacrittyTerminalBackend::new(30, 4);
        // Line 0: "hello   world foo.bar"
        // Line 1: "" (empty)
        // Line 2: "  wide: 猫 犬" (with 猫 at col 8, 犬 at col 11)
        terminal.advance(b"hello   world foo.bar\r\n\r\n  wide: \xe7\x8c\xab \xe7\x8a\xac");
        let snapshot = terminal.snapshot();

        // Forward word navigation on line 0
        assert_eq!(visual_next_word(&snapshot, 0, 0), (8, 0)); // "hello" -> "world"
        assert_eq!(visual_next_word(&snapshot, 0, 2), (8, 0)); // inside "hello" -> "world"
        assert_eq!(visual_next_word(&snapshot, 0, 5), (8, 0)); // whitespace -> "world"
        assert_eq!(visual_next_word(&snapshot, 0, 8), (14, 0)); // "world" -> "foo"
        assert_eq!(visual_next_word(&snapshot, 0, 14), (17, 0)); // "foo" -> "."
        assert_eq!(visual_next_word(&snapshot, 0, 17), (18, 0)); // "." -> "bar"

        // Across lines: from end of line 0 -> line 1 (empty line)
        assert_eq!(visual_next_word(&snapshot, 0, 18), (0, 1));
        assert_eq!(visual_next_word(&snapshot, 0, 20), (0, 1));

        // From empty line 1 -> line 2 first word "wide" (col 2 after leading spaces)
        assert_eq!(visual_next_word(&snapshot, 1, 0), (2, 2));

        // Line 2: "wide" -> ":" -> "猫" -> "犬"
        assert_eq!(visual_next_word(&snapshot, 2, 2), (6, 2));
        assert_eq!(visual_next_word(&snapshot, 2, 6), (8, 2));
        assert_eq!(visual_next_word(&snapshot, 2, 8), (11, 2));

        // Across line 2 -> line 3 (empty line 3 in 4-row terminal)
        assert_eq!(visual_next_word(&snapshot, 2, 11), (0, 3));
        assert_eq!(visual_next_word(&snapshot, 3, 0), (0, 3));

        // Backward word navigation
        assert_eq!(visual_prev_word(&snapshot, 3, 0), (11, 2)); // from line 3 -> "犬"
        assert_eq!(visual_prev_word(&snapshot, 2, 11), (8, 2)); // from "犬" -> "猫"
        assert_eq!(visual_prev_word(&snapshot, 2, 8), (6, 2)); // from "猫" -> ":"
        assert_eq!(visual_prev_word(&snapshot, 2, 6), (2, 2)); // from ":" -> "wide"
        assert_eq!(visual_prev_word(&snapshot, 2, 2), (0, 1)); // from "wide" -> empty line 1
        assert_eq!(visual_prev_word(&snapshot, 1, 0), (18, 0)); // from line 1 -> "bar"
        assert_eq!(visual_prev_word(&snapshot, 0, 20), (18, 0)); // inside "bar" -> start of "bar"
        assert_eq!(visual_prev_word(&snapshot, 0, 18), (17, 0)); // from "bar" -> "."
        assert_eq!(visual_prev_word(&snapshot, 0, 17), (14, 0)); // from "." -> "foo"
        assert_eq!(visual_prev_word(&snapshot, 0, 14), (8, 0)); // from "foo" -> "world"
        assert_eq!(visual_prev_word(&snapshot, 0, 8), (0, 0)); // from "world" -> "hello"
        assert_eq!(visual_prev_word(&snapshot, 0, 0), (0, 0)); // start clamping
    }
}
