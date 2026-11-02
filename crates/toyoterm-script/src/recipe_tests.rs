use super::tests::script_test_context;
use super::*;

fn load(recipe: &str) -> ConfigManager {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(recipe)
        .join("config.rb");
    ConfigManager::load_startup(Some(&path)).unwrap()
}

fn invoke(manager: &mut ConfigManager, invocation: ScriptInvocation) -> ScriptResult {
    run_script_request(manager, &script_test_context(), &invocation).unwrap()
}

fn command(manager: &mut ConfigManager, name: &str) -> ScriptResult {
    invoke(
        manager,
        ScriptInvocation::UserCommand {
            name: name.into(),
            pane: PaneId(4),
        },
    )
}

#[test]
fn recipes_load_from_disk_render_and_reload_without_duplicate_registrations() {
    for recipe in [
        "status_bar",
        "dynamic_theme",
        "pane_workbench",
        "command_journal",
    ] {
        let mut manager = load(recipe);
        for _ in 0..2 {
            let result = invoke(
                &mut manager,
                ScriptInvocation::Bar {
                    position: StatusBarPosition::Bottom,
                },
            );
            assert!(!result.bar.unwrap().is_empty(), "{recipe}");
            assert!(result.commands.is_empty(), "{recipe}");
            manager.reload_file().unwrap();
        }
    }
}

#[test]
fn recipe_status_bar_uses_background_git_and_clears_failure_badges_on_next_command() {
    let mut manager = load("status_bar");
    let result = invoke(
        &mut manager,
        ScriptInvocation::Bar {
            position: StatusBarPosition::Bottom,
        },
    );
    assert_eq!(result.async_requests.len(), 1);
    let request = &result.async_requests[0];
    assert_eq!(request.program, "git");
    assert_eq!(request.args, ["branch", "--show-current"]);
    manager
        .invoke_async_callback(request.id, b"", b"git unavailable", -1)
        .unwrap();
    assert!(
        manager
            .render_bar(StatusBarPosition::Bottom)
            .unwrap()
            .iter()
            .any(|item| item.text.contains("git: unavailable"))
    );
    for (kind, badge) in [
        (ScriptEventKind::CommandFinished, Some("exit 1".into())),
        (ScriptEventKind::CommandStarted, None),
    ] {
        let mut event = RubyEvent::new(kind);
        event.pane = Some(PaneId(4));
        event.exit_status = Some(1);
        let result = invoke(&mut manager, ScriptInvocation::Event(event));
        assert_eq!(
            result.commands,
            vec![NativeCommand::Ui(UiCommand::SetPaneBadge {
                pane: PaneId(4),
                badge,
            })]
        );
    }
}

#[test]
fn recipe_theme_manual_policy_survives_timer_and_cancel_until_automatic_or_reload() {
    let mut manager = load("dynamic_theme");
    for kind in [ScriptEventKind::AppStarted, ScriptEventKind::ConfigReloaded] {
        invoke(&mut manager, ScriptInvocation::Event(RubyEvent::new(kind)));
    }
    assert_eq!(manager.eval("Toyoterm.timers.length").unwrap(), "1");
    let timer_id = manager
        .eval("Toyoterm.timers.first.id")
        .unwrap()
        .parse()
        .unwrap();
    let original = manager.eval("Toyoterm.config[:theme]").unwrap();
    let manual = if original == "recipe.day" {
        "recipe.night"
    } else {
        "recipe.day"
    };
    for choice in [Some(manual), None, Some("Automatic")] {
        let result = command(&mut manager, "recipe_theme");
        let [NativeCommand::Ui(UiCommand::OpenSelector { id, .. })] = result.commands.as_slice()
        else {
            panic!("expected theme selector");
        };
        invoke(
            &mut manager,
            ScriptInvocation::SelectCallback {
                id: *id,
                selection: choice.map(str::to_owned),
            },
        );
        invoke(&mut manager, ScriptInvocation::Timer { id: timer_id });
        assert_eq!(
            manager.eval("Toyoterm.config[:theme]").unwrap(),
            if choice == Some("Automatic") {
                &original
            } else {
                manual
            }
        );
    }
    manager.reload_file().unwrap();
    assert_eq!(manager.eval("Toyoterm.config[:theme]").unwrap(), original);
}

#[test]
fn recipe_workbench_preserves_cwd_and_stale_picker_targets_are_ignored() {
    let mut manager = load("pane_workbench");
    let mut context = script_test_context();
    Arc::make_mut(&mut context.model).panes[0].cwd = Some("/project with spaces".into());
    let result = run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::UserCommand {
            name: "recipe_workbench".into(),
            pane: PaneId(4),
        },
    )
    .unwrap();
    assert_eq!(result.commands.len(), 2);
    for (command, role) in result.commands.iter().zip(["tools", "scratch"]) {
        let NativeCommand::Pane(PaneCommand::SplitWithLaunch { pane, launch, .. }) = command else {
            panic!("expected a shell split");
        };
        assert_eq!(*pane, PaneId(4));
        assert_eq!(launch.cwd.as_deref(), Some("/project with spaces"));
        assert_eq!(
            launch.environment,
            vec![("TOYOTERM_ROLE".into(), Some(role.into()))]
        );
    }
    let result = command(&mut manager, "recipe_find_pane");
    let [NativeCommand::Ui(UiCommand::OpenSelector { id, items, .. })] = result.commands.as_slice()
    else {
        panic!("expected Pane selector");
    };
    let selection = ScriptInvocation::SelectCallback {
        id: *id,
        selection: Some(items[0].clone()),
    };
    context.handles = vec![WorkspaceId(1).into(), WindowId(2).into(), TabId(3).into()].into();
    assert!(
        run_script_request(&mut manager, &context, &selection)
            .unwrap()
            .commands
            .is_empty()
    );
}

#[test]
fn recipe_journal_bounds_results_handles_missing_markers_and_escapes_metadata() {
    let mut manager = load("command_journal");
    let mut context = script_test_context();
    Arc::make_mut(&mut context.model).panes[0].cwd = Some("/project\nwith\tcontrols".into());
    for index in 0..55 {
        if index != 54 {
            let mut event = RubyEvent::new(ScriptEventKind::CommandStarted);
            event.pane = Some(PaneId(4));
            run_script_request(&mut manager, &context, &ScriptInvocation::Event(event)).unwrap();
        }
        let mut event = RubyEvent::new(ScriptEventKind::CommandFinished);
        event.pane = Some(PaneId(4));
        event.exit_status = Some(index);
        run_script_request(&mut manager, &context, &ScriptInvocation::Event(event)).unwrap();
    }
    let result = command(&mut manager, "recipe_journal");
    let [NativeCommand::Ui(UiCommand::OpenPanel { id, lines, .. })] = result.commands.as_slice()
    else {
        panic!("expected journal panel");
    };
    assert_eq!(lines.len(), 50);
    assert!(lines[0].contains("exit 54 | duration unknown"));
    assert!(lines[0].contains("/project\\nwith\\tcontrols"));
    assert!(lines[49].contains("exit 5 |"));
    invoke(
        &mut manager,
        ScriptInvocation::SelectCallback {
            id: *id,
            selection: None,
        },
    );
    command(&mut manager, "recipe_clear_journal");
    let result = command(&mut manager, "recipe_journal");
    let [NativeCommand::Ui(UiCommand::OpenPanel { lines, .. })] = result.commands.as_slice() else {
        panic!("expected empty journal panel");
    };
    assert!(lines[0].starts_with("No completed commands."));
}
