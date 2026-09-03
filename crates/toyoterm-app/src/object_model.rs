use super::*;

impl Drop for ToyotermApplication {
    fn drop(&mut self) {
        // This is also reached while unwinding from a panic. PaneRuntime and
        // native PTY sessions provide a second idempotent kill-on-drop guard.
        self.shutdown();
    }
}

#[cfg(test)]
#[derive(Debug, Default, Eq, PartialEq)]
pub(super) struct NativeCommandEffects {
    pub(super) clipboard_writes: Vec<String>,
    pub(super) reload_requested: bool,
}

pub(super) fn ruby_object_model(
    mux: &Mux,
    pane_runtimes: Option<&HashMap<PaneId, PaneRuntime>>,
) -> Result<RubyObjectModel, String> {
    let current_workspace = mux.current_workspace();
    let current_window = mux
        .current_window()
        .ok_or_else(|| "mux has no current window".to_owned())?;
    let current_tab = mux
        .current_tab()
        .ok_or_else(|| "mux has no current tab".to_owned())?;
    let current_pane = mux
        .current_pane()
        .ok_or_else(|| "mux has no current pane".to_owned())?;
    let mut workspaces = Vec::new();
    let mut windows = Vec::new();
    let mut tabs = Vec::new();
    let mut panes = Vec::new();

    for workspace_id in mux.workspaces() {
        let window_ids = mux
            .workspace_windows(workspace_id)
            .ok_or_else(|| format!("workspace {workspace_id} is missing"))?
            .to_vec();
        workspaces.push(RubyWorkspace {
            id: workspace_id,
            name: mux
                .workspace_name(workspace_id)
                .unwrap_or_default()
                .to_owned(),
            windows: window_ids.clone(),
        });
        for window_id in window_ids {
            let tab_ids = mux
                .tabs(window_id)
                .ok_or_else(|| format!("window {window_id} is missing"))?
                .to_vec();
            windows.push(RubyWindow {
                id: window_id,
                tabs: tab_ids.clone(),
            });
            for tab_id in tab_ids {
                let pane_ids = mux
                    .tab_panes(tab_id)
                    .ok_or_else(|| format!("tab {tab_id} is missing"))?;
                let tab_number = mux
                    .tab_number(tab_id)
                    .ok_or_else(|| format!("tab {tab_id} is missing from its window"))?;
                tabs.push(RubyTab {
                    id: tab_id,
                    title: format!("Tab {tab_number}"),
                    panes: pane_ids.clone(),
                });
                for pane_id in pane_ids {
                    let runtime = pane_runtimes.and_then(|runtimes| runtimes.get(&pane_id));
                    panes.push(RubyPane {
                        id: pane_id,
                        title: runtime
                            .map(|runtime| runtime.title.clone())
                            .unwrap_or_else(|| format!("Pane {}", pane_id.0)),
                        cwd: runtime
                            .and_then(|runtime| runtime.cwd.as_ref())
                            .map(|cwd| cwd.display().to_string()),
                        pid: runtime.and_then(|runtime| runtime.process_id),
                        command_running: runtime.is_some_and(|runtime| runtime.command_running),
                        last_exit_status: runtime.and_then(|runtime| runtime.last_exit_status),
                    });
                }
            }
        }
    }

    workspaces.sort_by_key(|workspace| workspace.id);
    windows.sort_by_key(|window| window.id);
    tabs.sort_by_key(|tab| tab.id);
    panes.sort_by_key(|pane| pane.id);
    Ok(RubyObjectModel {
        current_workspace,
        current_window,
        current_tab,
        current_pane,
        workspaces,
        windows,
        tabs,
        panes,
    })
}

#[cfg(test)]
pub(super) fn dispatch_script_commands(
    config_manager: &mut ConfigManager,
    mux: &mut Mux,
) -> Result<NativeCommandEffects, String> {
    let current_workspace = mux.current_workspace();
    let current_window = mux
        .current_window()
        .ok_or_else(|| "mux has no current window".to_owned())?;
    let current_tab = mux
        .current_tab()
        .ok_or_else(|| "mux has no current tab".to_owned())?;
    let current_pane = mux
        .current_pane()
        .ok_or_else(|| "mux has no current pane".to_owned())?;
    let model = ruby_object_model(mux, None)?;
    config_manager
        .set_live_handles(mux.native_handles())
        .map_err(|error| error.to_string())?;
    config_manager
        .set_object_model(&model)
        .map_err(|error| error.to_string())?;
    let mut effects = NativeCommandEffects::default();
    for command in config_manager
        .drain_commands_with_context(current_workspace, current_window, current_tab, current_pane)
        .map_err(|error| error.to_string())?
    {
        match command {
            NativeCommand::Mux(command) => {
                mux.dispatch(command).map_err(|error| error.to_string())?;
            }
            NativeCommand::ClipboardWrite(text) => effects.clipboard_writes.push(text),
            NativeCommand::NewTabWithLaunch { .. } | NativeCommand::SplitWithLaunch { .. } => {}
            NativeCommand::SetPaneBadge { .. } => {}
            NativeCommand::SearchPane { .. } => {}
            NativeCommand::ReloadConfig => effects.reload_requested = true,
        }
    }
    Ok(effects)
}
