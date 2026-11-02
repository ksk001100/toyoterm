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
        "command_forge",
        "project_modes",
        "output_lenses",
        "commands",
        "pane_tools",
        "monitoring",
        "screen_analysis",
        "extension_libraries",
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

#[test]
fn recipe_forge_generated_methods_preserve_each_declaration_and_have_real_help() {
    let mut manager = load("command_forge");
    for (name, direction, role, key) in [
        ("forge_tools", SplitDirection::Right, "tools", "ALT+1"),
        ("forge_notes", SplitDirection::Down, "notes", "ALT+2"),
        ("forge_review", SplitDirection::Left, "review", "ALT+3"),
    ] {
        let result = command(&mut manager, name);
        let [
            NativeCommand::Pane(PaneCommand::SplitWithLaunch {
                pane,
                direction: actual,
                launch,
            }),
            NativeCommand::Ui(UiCommand::SetPaneBadge { badge, .. }),
        ] = result.commands.as_slice()
        else {
            panic!(
                "generated command lost its launch or observer: {:?}",
                result.commands
            );
        };
        assert_eq!(*pane, PaneId(4));
        assert_eq!(*actual, direction);
        assert_eq!(
            launch.environment,
            vec![("TOYOTERM_ROLE".into(), Some(role.into()))]
        );
        assert_eq!(badge.as_deref(), Some(format!("queued: {role}").as_str()));
        assert_eq!(
            manager.native_action(key),
            Some(NativeAction::UserCommand(name.into()))
        );
    }
    let result = command(&mut manager, "forge_inspect");
    let [NativeCommand::Ui(UiCommand::OpenPanel { lines, .. })] = result.commands.as_slice() else {
        panic!("expected generated method inspection");
    };
    assert_eq!(lines.len(), 3);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("forge_tools | ALT+1"))
    );
    // The code generator only extends its private receiver, never Object.
    assert_eq!(
        manager
            .eval("Object.new.respond_to?(:forge_tools)")
            .unwrap(),
        "false"
    );
}

#[test]
fn recipe_project_modes_reconcile_per_pane_keys_and_release_old_event_subscriptions() {
    let project = tempfile::tempdir().unwrap();
    let cargo = project.path().join("Cargo.toml");
    let package = project.path().join("package.json");
    std::fs::write(&cargo, "").unwrap();
    std::fs::write(&package, "{}").unwrap();
    let mut manager = load("project_modes");
    let mut context = script_test_context();
    Arc::make_mut(&mut context.model).panes[0].cwd = Some(project.path().to_str().unwrap().into());
    let mut event = RubyEvent::new(ScriptEventKind::CwdChanged);
    event.pane = Some(PaneId(4));
    let change = ScriptInvocation::Event(event);
    for (kind, program) in [("rust", "cargo"), ("node", "npm")] {
        run_script_request(&mut manager, &context, &change).unwrap();
        // A second detection should not re-enable the mode or add handlers.
        assert!(
            run_script_request(&mut manager, &context, &change)
                .unwrap()
                .commands
                .is_empty()
        );
        let map = manager
            .eval(&format!("Toyoterm.keymap('recipe.project.{kind}').id"))
            .unwrap()
            .parse()
            .unwrap();
        let result = run_script_request(
            &mut manager,
            &context,
            &ScriptInvocation::KeymapBinding {
                map,
                key: "ALT+ENTER".into(),
                pane: PaneId(4),
            },
        )
        .unwrap();
        let [NativeCommand::Pane(PaneCommand::SplitWithLaunch { launch, .. })] =
            result.commands.as_slice()
        else {
            panic!("expected context-sensitive task");
        };
        assert_eq!(launch.program.as_deref(), Some(program));
        assert_eq!(launch.args, ["test"]);
        let mut finish = RubyEvent::new(ScriptEventKind::CommandFinished);
        finish.pane = Some(PaneId(4));
        finish.exit_status = Some(7);
        let result =
            run_script_request(&mut manager, &context, &ScriptInvocation::Event(finish)).unwrap();
        assert_eq!(result.commands.len(), 1, "old mode subscription leaked");
        if kind == "rust" {
            std::fs::remove_file(&cargo).unwrap();
        }
    }
    std::fs::remove_file(&package).unwrap();
    run_script_request(&mut manager, &context, &change).unwrap();
    assert_eq!(manager.eval("Toyoterm.current_pane.modes").unwrap(), "[]");
    let mut finish = RubyEvent::new(ScriptEventKind::CommandFinished);
    finish.pane = Some(PaneId(4));
    assert!(
        run_script_request(&mut manager, &context, &ScriptInvocation::Event(finish))
            .unwrap()
            .commands
            .is_empty()
    );
}

fn choose_recipe_item(
    manager: &mut ConfigManager,
    result: ScriptResult,
    wanted: &str,
) -> ScriptResult {
    let [NativeCommand::Ui(UiCommand::OpenSelector { id, items, .. })] = result.commands.as_slice()
    else {
        panic!("expected selection, got {:?}", result.commands);
    };
    let selection = items
        .iter()
        .find(|item| item.contains(wanted))
        .unwrap()
        .clone();
    invoke(
        manager,
        ScriptInvocation::SelectCallback {
            id: *id,
            selection: Some(selection),
        },
    )
}

#[test]
fn recipe_output_lenses_extend_via_hooks_and_keep_the_original_screen_snapshot() {
    let mut manager = load("output_lenses");
    let mut context = script_test_context();
    Arc::make_mut(&mut context.model).panes[0].screen_text =
        "warning: captured\nTODO: review\nhttps://one.test https://two.test/path.\n".into();
    let opened = run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::UserCommand {
            name: "recipe_lenses".into(),
            pane: PaneId(4),
        },
    )
    .unwrap();
    // Later callbacks use the default screen ("prompt>"). Results must still
    // refer to the original capture and include the second URL on one line.
    let matches = choose_recipe_item(&mut manager, opened, "URLs");
    let actions = choose_recipe_item(&mut manager, matches, "https://two.test/path");
    let result = choose_recipe_item(&mut manager, actions, "Copy to clipboard");
    assert_eq!(
        result.commands,
        vec![NativeCommand::Clipboard(ClipboardCommand::Write(
            "https://two.test/path".into()
        ))]
    );
    invoke(&mut manager, ScriptInvocation::Eval(
        "ExampleOutputLenses.register('Prompts') { |line| line.start_with?('prompt>') ? [line] : [] }".into()
    ));
    let opened = command(&mut manager, "recipe_lenses");
    let matches = choose_recipe_item(&mut manager, opened, "Prompts");
    let actions = choose_recipe_item(&mut manager, matches, "prompt>");
    let result = choose_recipe_item(&mut manager, actions, "Find in scrollback");
    assert!(
        matches!(result.commands.as_slice(), [NativeCommand::Pane(PaneCommand::Search { pane: PaneId(4), query, .. })] if query == "prompt>")
    );
}
