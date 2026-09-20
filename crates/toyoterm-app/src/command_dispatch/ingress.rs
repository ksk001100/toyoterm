use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommandOrigin {
    Script,
    Ipc,
    Keybinding,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MuxDispatch {
    Coordinator,
    Gui,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ActionResolution {
    pub(super) activation: Vec<Command>,
    pub(super) command: Option<NativeCommand>,
}

pub(super) fn validate_command_origin(
    origin: CommandOrigin,
    command: &NativeCommand,
) -> Result<(), String> {
    if origin == CommandOrigin::Ipc
        && !matches!(command, NativeCommand::Mux(_) | NativeCommand::Config(_))
    {
        return Err("native command domain is not exposed over IPC".to_owned());
    }
    Ok(())
}

pub(super) fn resolve_mux_dispatch(origin: CommandOrigin) -> MuxDispatch {
    match origin {
        CommandOrigin::Script => MuxDispatch::Coordinator,
        CommandOrigin::Ipc | CommandOrigin::Keybinding => MuxDispatch::Gui,
    }
}

/// Resolves a user-operation-level action into a concrete domain command.
/// Domain handlers never receive `NativeAction` or `CommandOrigin`.
pub(super) fn resolve_action_command(
    command: ActionCommand,
    origin: CommandOrigin,
    mux: &Mux,
    pane_layout: &PaneLayout,
) -> Result<ActionResolution, String> {
    if origin == CommandOrigin::Ipc {
        return Err("native command domain is not exposed over IPC".to_owned());
    }
    let ActionCommand::Invoke { action, context } = command;
    if origin == CommandOrigin::Script
        && !action_is_global(&action)
        && !action_context_is_valid(mux, context)
    {
        return Err("callback action target hierarchy is no longer valid".to_owned());
    }
    let activation = if origin == CommandOrigin::Script {
        context_activation_commands(&action, context)
    } else {
        Vec::new()
    };
    let command = resolve_native_action(action, context, mux, pane_layout)?;
    Ok(ActionResolution {
        activation,
        command,
    })
}

fn resolve_native_action(
    action: NativeAction,
    context: ActionContext,
    mux: &Mux,
    pane_layout: &PaneLayout,
) -> Result<Option<NativeCommand>, String> {
    let command = match action {
        NativeAction::NewTab => NativeCommand::Mux(Command::NewTab),
        NativeAction::ClosePane => NativeCommand::Mux(Command::ClosePane(context.pane)),
        NativeAction::CloseTab => NativeCommand::Mux(Command::CloseTab(context.tab)),
        NativeAction::NewWorkspace => {
            NativeCommand::Mux(Command::SwitchWorkspace(next_workspace_name(mux)))
        }
        NativeAction::ReloadConfig => NativeCommand::Config(ConfigCommand::Reload),
        NativeAction::Search => NativeCommand::Ui(UiCommand::OpenSearch { pane: context.pane }),
        NativeAction::MaximizeWindow => NativeCommand::Window(WindowCommand::Maximize),
        NativeAction::ToggleMaximize => NativeCommand::Window(WindowCommand::ToggleMaximize),
        NativeAction::MinimizeWindow => NativeCommand::Window(WindowCommand::Minimize),
        NativeAction::ToggleFullscreen => NativeCommand::Window(WindowCommand::ToggleFullscreen),
        NativeAction::NextTab => NativeCommand::Mux(Command::ActivateTab(adjacent_tab(
            mux,
            context.window,
            context.tab,
            false,
        )?)),
        NativeAction::PreviousTab => NativeCommand::Mux(Command::ActivateTab(adjacent_tab(
            mux,
            context.window,
            context.tab,
            true,
        )?)),
        NativeAction::NextWorkspace => NativeCommand::Mux(Command::ActivateWorkspace(
            adjacent_workspace(mux, context.workspace, false)?,
        )),
        NativeAction::PreviousWorkspace => NativeCommand::Mux(Command::ActivateWorkspace(
            adjacent_workspace(mux, context.workspace, true)?,
        )),
        NativeAction::NextPrompt => navigation_command(context.pane, NavigationKind::Prompt, false),
        NativeAction::PreviousPrompt => {
            navigation_command(context.pane, NavigationKind::Prompt, true)
        }
        NativeAction::NextMark => navigation_command(context.pane, NavigationKind::Mark, false),
        NativeAction::PreviousMark => navigation_command(context.pane, NavigationKind::Mark, true),
        NativeAction::SelectNextCommandOutput => {
            navigation_command(context.pane, NavigationKind::CommandOutput, false)
        }
        NativeAction::SelectPreviousCommandOutput => {
            navigation_command(context.pane, NavigationKind::CommandOutput, true)
        }
        NativeAction::SelectLastCommandOutput => {
            NativeCommand::Ui(UiCommand::SelectLastCommandOutput { pane: context.pane })
        }
        NativeAction::CopySelection => {
            NativeCommand::Clipboard(ClipboardCommand::CopySelection { pane: context.pane })
        }
        NativeAction::PasteClipboard => {
            NativeCommand::Clipboard(ClipboardCommand::Paste { pane: context.pane })
        }
        NativeAction::StartVisualMode => {
            NativeCommand::Ui(UiCommand::StartVisualMode { pane: context.pane })
        }
        NativeAction::ToggleVisualMode => {
            NativeCommand::Ui(UiCommand::ToggleVisualMode { pane: context.pane })
        }
        NativeAction::StartVisualSelection => {
            NativeCommand::Ui(UiCommand::StartVisualSelection { pane: context.pane })
        }
        NativeAction::SelectVisualSelection => {
            NativeCommand::Ui(UiCommand::SelectVisualSelection { pane: context.pane })
        }
        NativeAction::EndVisualSelection => {
            NativeCommand::Ui(UiCommand::EndVisualSelection { pane: context.pane })
        }
        NativeAction::MoveVisualSelection(motion) => {
            NativeCommand::Ui(UiCommand::MoveVisualSelection {
                pane: context.pane,
                motion,
            })
        }
        NativeAction::YankSelection => {
            NativeCommand::Clipboard(ClipboardCommand::YankSelection { pane: context.pane })
        }
        NativeAction::UserCommand(name) => {
            NativeCommand::Script(ScriptCommand::InvokeUserCommand {
                name,
                pane: context.pane,
            })
        }
        NativeAction::Split(direction) => NativeCommand::Mux(Command::Split {
            pane: context.pane,
            direction,
        }),
        NativeAction::ActivatePane(direction) => {
            let Some(pane) = pane_layout.neighbor(context.pane, direction) else {
                return Ok(None);
            };
            NativeCommand::Mux(Command::ActivatePane(pane))
        }
        NativeAction::ToggleZoom => NativeCommand::Mux(Command::ToggleZoom),
    };
    Ok(Some(command))
}

#[derive(Clone, Copy)]
enum NavigationKind {
    Prompt,
    Mark,
    CommandOutput,
}

fn navigation_command(pane: PaneId, kind: NavigationKind, previous: bool) -> NativeCommand {
    let direction = if previous {
        PaneSearchDirection::Previous
    } else {
        PaneSearchDirection::Next
    };
    NativeCommand::Ui(match kind {
        NavigationKind::Prompt => UiCommand::NavigatePrompt { pane, direction },
        NavigationKind::Mark => UiCommand::NavigateMark { pane, direction },
        NavigationKind::CommandOutput => UiCommand::SelectCommandOutput { pane, direction },
    })
}

fn action_is_global(action: &NativeAction) -> bool {
    matches!(
        action,
        NativeAction::ReloadConfig
            | NativeAction::MaximizeWindow
            | NativeAction::ToggleMaximize
            | NativeAction::MinimizeWindow
            | NativeAction::ToggleFullscreen
            | NativeAction::NewWorkspace
    )
}

pub(super) fn context_activation_commands(
    action: &NativeAction,
    context: ActionContext,
) -> Vec<Command> {
    if action_is_global(action) {
        return Vec::new();
    }
    vec![
        Command::ActivateWorkspace(context.workspace),
        Command::ActivateWindow(context.window),
        Command::ActivateTab(context.tab),
        Command::ActivatePane(context.pane),
    ]
}

pub(super) fn action_context_is_valid(mux: &Mux, context: ActionContext) -> bool {
    mux.workspace_windows(context.workspace)
        .is_some_and(|windows| windows.contains(&context.window))
        && mux
            .tabs(context.window)
            .is_some_and(|tabs| tabs.contains(&context.tab))
        && mux.pane_tab(context.pane) == Some(context.tab)
}

fn adjacent_tab(
    mux: &Mux,
    window: toyoterm_api::WindowId,
    current: toyoterm_api::TabId,
    backwards: bool,
) -> Result<toyoterm_api::TabId, String> {
    let tabs = mux
        .tabs(window)
        .ok_or_else(|| format!("unknown window {window}"))?;
    let current_index = tabs
        .iter()
        .position(|tab| *tab == current)
        .ok_or_else(|| format!("active tab {current} is not in window {window}"))?;
    Ok(tabs[cycle_index(current_index, tabs.len(), backwards)])
}

fn adjacent_workspace(
    mux: &Mux,
    current: toyoterm_api::WorkspaceId,
    backwards: bool,
) -> Result<toyoterm_api::WorkspaceId, String> {
    let workspaces = mux.workspaces();
    let current_index = workspaces
        .iter()
        .position(|workspace| *workspace == current)
        .ok_or_else(|| format!("active workspace {current} is not registered"))?;
    Ok(workspaces[cycle_index(current_index, workspaces.len(), backwards)])
}

fn cycle_index(current: usize, len: usize, backwards: bool) -> usize {
    if backwards {
        (current + len - 1) % len
    } else {
        (current + 1) % len
    }
}

fn next_workspace_name(mux: &Mux) -> String {
    let mut suffix = mux.workspaces().len() + 1;
    loop {
        let candidate = format!("Workspace {suffix}");
        if mux
            .workspaces()
            .into_iter()
            .all(|workspace| mux.workspace_name(workspace) != Some(candidate.as_str()))
        {
            return candidate;
        }
        suffix += 1;
    }
}
