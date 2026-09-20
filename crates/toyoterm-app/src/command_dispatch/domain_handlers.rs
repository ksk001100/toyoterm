use super::*;

#[derive(Debug, Default)]
pub(super) struct PaneEffects {
    pub(super) search: Option<PaneSearchEffect>,
}

#[derive(Debug)]
pub(super) struct PaneSearchEffect {
    pub(super) pane: PaneId,
    pub(super) query: String,
    pub(super) result: SearchResult,
}

pub(super) fn apply_window_command(
    command: WindowCommand,
    mux: &mut Mux,
    runtime_events: &mut VecDeque<RubyEvent>,
    terminal_runtime: &mut TerminalRuntime,
    platform: &mut PlatformState,
) -> Result<(), String> {
    match command {
        WindowCommand::CreateWithLaunch { workspace, launch } => {
            stage_pane_launch(
                PaneCreation::NewWindow(workspace),
                launch,
                mux,
                runtime_events,
                terminal_runtime,
            )?;
        }
        WindowCommand::NewTabWithLaunch { window, launch } => {
            stage_pane_launch(
                PaneCreation::NewTab(window),
                launch,
                mux,
                runtime_events,
                terminal_runtime,
            )?;
        }
        WindowCommand::Maximize => native_window(platform)?.set_maximized(true),
        WindowCommand::ToggleMaximize => {
            let window = native_window(platform)?;
            window.set_maximized(!window.is_maximized());
        }
        WindowCommand::Minimize => native_window(platform)?.set_minimized(true),
        WindowCommand::ToggleFullscreen => {
            let window = native_window(platform)?;
            let fullscreen = if window.fullscreen().is_some() {
                None
            } else {
                Some(Fullscreen::Borderless(window.current_monitor()))
            };
            window.set_fullscreen(fullscreen);
        }
    }
    Ok(())
}

fn native_window(platform: &PlatformState) -> Result<&Window, String> {
    platform
        .window
        .as_deref()
        .ok_or_else(|| "native window is not available".to_owned())
}

pub(super) fn apply_pane_command(
    command: PaneCommand,
    mux: &mut Mux,
    runtime_events: &mut VecDeque<RubyEvent>,
    terminal_runtime: &mut TerminalRuntime,
) -> Result<PaneEffects, String> {
    let mut effects = PaneEffects::default();
    match command {
        PaneCommand::SplitWithLaunch {
            pane,
            direction,
            launch,
        } => {
            stage_pane_launch(
                PaneCreation::Split { pane, direction },
                launch,
                mux,
                runtime_events,
                terminal_runtime,
            )?;
        }
        PaneCommand::Search {
            pane,
            query,
            direction,
        } => {
            effects.search = Some(apply_pane_search(
                pane,
                query,
                direction,
                mux,
                runtime_events,
                terminal_runtime,
            )?);
        }
    }
    Ok(effects)
}

pub(super) fn stage_pane_launch(
    creation: PaneCreation,
    launch: PaneLaunchSpec,
    mux: &mut Mux,
    runtime_events: &mut VecDeque<RubyEvent>,
    terminal_runtime: &mut TerminalRuntime,
) -> Result<PaneId, String> {
    let pane = dispatch_pane_creation(mux, runtime_events, creation)?;
    terminal_runtime.pending_pane_launches.insert(pane, launch);
    Ok(pane)
}

fn apply_pane_search(
    pane: PaneId,
    query: String,
    direction: PaneSearchDirection,
    mux: &mut Mux,
    runtime_events: &mut VecDeque<RubyEvent>,
    terminal_runtime: &mut TerminalRuntime,
) -> Result<PaneSearchEffect, String> {
    if query.is_empty() {
        return Err("pane search query cannot be empty".to_owned());
    }
    dispatch_coordinator_command(mux, runtime_events, Command::ActivatePane(pane))?;
    let direction = match direction {
        PaneSearchDirection::Next => SearchDirection::Next,
        PaneSearchDirection::Previous => SearchDirection::Previous,
    };
    let result = terminal_runtime
        .pane_runtimes
        .get_mut(&pane)
        .ok_or_else(|| format!("pane {pane} has no terminal runtime"))?
        .terminal
        .search(&query, direction);
    Ok(PaneSearchEffect {
        pane,
        query,
        result,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_search_activates_the_target_and_returns_a_ui_effect() {
        let mut mux = Mux::new();
        let searched_pane = mux.current_pane().unwrap();
        mux.dispatch(Command::NewTab).unwrap();
        let other_pane = mux.current_pane().unwrap();
        let mut events = VecDeque::new();
        let mut terminals = test_terminal_runtime([searched_pane, other_pane]);
        let effects = apply_pane_command(
            PaneCommand::Search {
                pane: searched_pane,
                query: "needle".into(),
                direction: PaneSearchDirection::Next,
            },
            &mut mux,
            &mut events,
            &mut terminals,
        )
        .unwrap();

        assert_eq!(mux.current_pane(), Some(searched_pane));
        let search = effects.search.unwrap();
        assert_eq!(search.pane, searched_pane);
        assert_eq!(search.query, "needle");
    }
}
