use super::tests::script_test_context;
use super::*;

fn eval(manager: &mut ConfigManager, source: &str) -> Result<ScriptResult, ScriptError> {
    run_script_request(
        manager,
        &script_test_context(),
        &ScriptInvocation::Eval(source.into()),
    )
}

#[test]
fn hook_priorities_keep_default_registration_order_and_expose_immutable_metadata() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      $label = 'provider'
      Toyoterm.add_hook(:choose, priority: 5, label: $label) { :late }
      Toyoterm.add_hook(:choose) { :first }
      Toyoterm.add_hook(:choose) { :second }
      Toyoterm.add_hook(:choose, priority: -5) { :early }
      $label.replace('changed')
    "#,
        )
        .unwrap();
    assert_eq!(
        manager.eval("Toyoterm.run_hook(:choose)").unwrap(),
        "[:early, :first, :second, :late]"
    );
    assert_eq!(
        manager
            .eval("Toyoterm.hook_handlers(:choose).map { |h| h[:priority] }")
            .unwrap(),
        "[-5, 0, 0, 5]"
    );
    assert_eq!(
        manager
            .eval("Toyoterm.hook_handlers(:choose).last[:label]")
            .unwrap(),
        "provider"
    );
    assert_eq!(
        manager
            .eval("Toyoterm.hook_handlers(:choose).last[:source]")
            .unwrap(),
        "(config)"
    );
    for source in [
        "Toyoterm.hook_handlers(:choose) << {}",
        "Toyoterm.hook_handlers(:choose)[0][:priority] = 9",
        "Toyoterm.hook_handlers(:choose).last[:label].replace('bad')",
    ] {
        assert!(manager.eval(source).is_err());
    }
    assert_eq!(
        manager
            .eval("Toyoterm.hook_handlers(:missing).empty?")
            .unwrap(),
        "true"
    );
}

#[test]
fn hook_strategies_stop_on_ruby_truthiness_and_preserve_the_selected_value() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      $calls = []
      Toyoterm.add_hook(:choose) { $calls << :nil; nil }
      Toyoterm.add_hook(:choose) { $calls << :false; false }
      Toyoterm.add_hook(:choose) { $calls << :zero; 0 }
      Toyoterm.add_hook(:choose) { raise 'unreachable' }
      Toyoterm.add_hook(:permit) { true }
      Toyoterm.add_hook(:permit) { nil }
      Toyoterm.add_hook(:permit) { raise 'unreachable' }
    "#,
        )
        .unwrap();
    assert_eq!(
        manager
            .eval("Toyoterm.run_hook_until_success(:choose)")
            .unwrap(),
        "0"
    );
    assert_eq!(manager.eval("$calls").unwrap(), "[:nil, :false, :zero]");
    assert_eq!(
        manager
            .eval("Toyoterm.run_hook_until_failure(:permit)")
            .unwrap(),
        "false"
    );
    assert_eq!(
        manager
            .eval("Toyoterm.run_hook_until_success(:missing).nil?")
            .unwrap(),
        "true"
    );
    assert_eq!(
        manager
            .eval("Toyoterm.run_hook_until_failure(:missing)")
            .unwrap(),
        "true"
    );
    assert_eq!(
        manager.eval("Toyoterm.run_hook(:missing).frozen?").unwrap(),
        "true"
    );
    manager
        .eval("$result = []; Toyoterm.add_hook(:object) { $result }")
        .unwrap();
    assert_eq!(
        manager
            .eval("Toyoterm.run_hook_until_success(:object).equal?($result) && !$result.frozen?")
            .unwrap(),
        "true"
    );
}

#[test]
fn priority_dispatch_uses_a_snapshot_even_when_handlers_are_removed_or_added() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      @other = Toyoterm.add_hook(:change, priority: 10) { :removed }
      Toyoterm.add_hook(:change) do
        @other.remove
        @new ||= Toyoterm.add_hook(:change, priority: -10) { :added }
        :first
      end
    "#,
        )
        .unwrap();
    assert_eq!(
        manager.eval("Toyoterm.run_hook(:change)").unwrap(),
        "[:first, :removed]"
    );
    assert_eq!(
        manager.eval("Toyoterm.run_hook(:change)").unwrap(),
        "[:added, :first]"
    );
}

#[test]
fn hook_strategy_failures_roll_back_all_host_effects_and_restore_nesting() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        r#"
      Toyoterm.add_hook(:fail) do |c|
        c.pane.badge = 'rollback'
        Toyoterm.add_hook(:leaked) {}
        nil
      end
      Toyoterm.add_hook(:fail) { raise 'handler failed' }
      Toyoterm.add_hook(:policy_fail) do |c|
        c.pane.badge = 'rollback'
        Toyoterm.add_hook(:leaked) {}
        true
      end
      Toyoterm.add_hook(:policy_fail) { raise 'handler failed' }
      Toyoterm.add_hook(:recurse) { Toyoterm.run_hook_until_success(:recurse) }
    "#,
    )
    .unwrap();
    assert!(eval(&mut manager, "Toyoterm.run_hook_until_success(:fail, Toyoterm::CallbackContext.new(Toyoterm.current_pane))").is_err());
    assert!(eval(&mut manager, "Toyoterm.run_hook_until_failure(:policy_fail, Toyoterm::CallbackContext.new(Toyoterm.current_pane))").is_err());
    assert_eq!(
        manager
            .eval("Toyoterm.hook_handlers(:leaked).empty?")
            .unwrap(),
        "true"
    );
    assert!(
        manager
            .eval("Toyoterm.run_hook_until_success(:recurse)")
            .unwrap_err()
            .to_string()
            .contains("64")
    );
    assert_eq!(
        manager
            .eval("Toyoterm.run_hook_until_failure(:unknown)")
            .unwrap(),
        "true"
    );
}

#[test]
fn hook_options_validate_before_registration_and_metadata_tracks_mode_cleanup() {
    let mut manager = ConfigManager::new().unwrap();
    for source in [
        "Toyoterm.add_hook(:bad, priority: 1.0) {}",
        "Toyoterm.add_hook(:bad, priority: 32768) {}",
        "Toyoterm.add_hook(:bad, priority: -32769) {}",
        "Toyoterm.add_hook(:bad, label: '') {}",
        "Toyoterm.add_hook(:bad, label: 1) {}",
    ] {
        assert!(manager.eval(source).is_err(), "{source}");
    }
    assert_eq!(manager.eval("Toyoterm.hooks.empty?").unwrap(), "true");
    eval(&mut manager, r#"
      Toyoterm.keymap(:empty) {}
      Toyoterm.mode(:owned, keymap: :empty) { |on| Toyoterm.add_hook(:owned, label: :library) {} if on }
      Toyoterm.mode(:owned).enable
    "#).unwrap();
    assert_eq!(
        manager
            .eval("Toyoterm.hook_handlers(:owned).length")
            .unwrap(),
        "1"
    );
    eval(&mut manager, "Toyoterm.mode(:owned).disable").unwrap();
    assert_eq!(
        manager
            .eval("Toyoterm.hook_handlers(:owned).empty?")
            .unwrap(),
        "true"
    );
}

#[test]
fn independent_hook_libraries_compose_validate_observe_and_remove_without_load_order_priority() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/hook_composition.rb");
    let mut manager = ConfigManager::load_startup(Some(&path)).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let directory_text = directory.path().to_string_lossy().into_owned();
    let mut context = script_test_context();
    Arc::make_mut(&mut context.model).panes[0].cwd = Some(directory_text.clone());
    let result = run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::UserCommand {
            name: "project_terminal".into(),
            pane: PaneId(4),
        },
    )
    .unwrap();
    assert!(
        matches!(result.commands.as_slice(), [NativeCommand::Pane(PaneCommand::SplitWithLaunch { pane: PaneId(4), launch, .. }), NativeCommand::Ui(UiCommand::SetPaneBadge { pane: PaneId(4), .. })] if launch.cwd.as_deref() == Some(directory_text.as_str()))
    );
    assert!(
        manager
            .eval("Toyoterm.hook_handlers(:terminal_directory).first[:source]")
            .unwrap()
            .ends_with("directory_provider.rb")
    );
    eval(&mut manager, "ExampleLaunchObserver::REGISTRATION.remove; Toyoterm.add_hook(:allow_terminal_directory, priority: -100, label: :deny) { false }").unwrap();
    let result = run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::UserCommand {
            name: "project_terminal".into(),
            pane: PaneId(4),
        },
    )
    .unwrap();
    assert!(result.commands.is_empty());
    assert_eq!(
        manager
            .eval("Toyoterm.hook_handlers(:terminal_launch_requested).empty?")
            .unwrap(),
        "true"
    );
}

#[test]
fn independent_library_require_permutations_preserve_priority_and_load_once() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/hook_libraries");
    let fixture = tempfile::tempdir().unwrap();
    let project = fixture.path().join("project");
    let shell = fixture.path().join("shell");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir(&shell).unwrap();
    let expected = project.to_string_lossy().into_owned();
    let extra = fixture.path().join("project_provider.rb");
    std::fs::write(&extra, format!(
        "module ExampleProjectProvider\n  REGISTRATION = Toyoterm.add_hook(:terminal_directory, priority: -20, label: :project_directory) {{ |_context| {} }}\nend\n",
        ruby_string_literal(&expected)
    )).unwrap();
    let files = [
        root.join("directory_provider.rb"),
        root.join("launch_policy.rb"),
        root.join("launch_observer.rb"),
        extra,
    ];
    let command_source = include_str!("../../../examples/hook_composition.rb")
        .lines()
        .filter(|line| !line.starts_with("require_relative "))
        .collect::<Vec<_>>()
        .join("\n");
    let mut context = script_test_context();
    Arc::make_mut(&mut context.model).panes[0].cwd = Some(shell.to_string_lossy().into_owned());
    let mut permutations = 0;
    for a in 0..4 {
        for b in 0..4 {
            for c in 0..4 {
                for d in 0..4 {
                    let order = [a, b, c, d];
                    if order.iter().copied().collect::<HashSet<_>>().len() != 4 {
                        continue;
                    }
                    permutations += 1;
                    let imports = order
                        .iter()
                        .map(|&index| {
                            format!(
                                "require {}\n",
                                ruby_string_literal(&files[index].to_string_lossy())
                            )
                        })
                        .collect::<String>();
                    let mut manager = ConfigManager::new().unwrap();
                    manager
                        .reload(&format!("{imports}{command_source}"))
                        .unwrap();
                    for path in &files {
                        assert_eq!(
                            manager
                                .eval(&format!(
                                    "require {}",
                                    ruby_string_literal(&path.to_string_lossy())
                                ))
                                .unwrap(),
                            "false",
                            "require repeated for {order:?}"
                        );
                    }
                    assert_eq!(
                        manager
                            .eval(
                                "Toyoterm.hook_handlers(:terminal_directory).map { |h| h[:label] }"
                            )
                            .unwrap(),
                        "[\"project_directory\", \"shell_directory\", \"process_directory\"]",
                        "order {order:?}"
                    );
                    assert_eq!(manager.eval("[Toyoterm.hook_handlers(:allow_terminal_directory).length, Toyoterm.hook_handlers(:terminal_launch_requested).length]").unwrap(), "[1, 1]");
                    let result = run_script_request(
                        &mut manager,
                        &context,
                        &ScriptInvocation::UserCommand {
                            name: "project_terminal".into(),
                            pane: PaneId(4),
                        },
                    )
                    .unwrap();
                    assert!(
                        matches!(result.commands.as_slice(), [NativeCommand::Pane(PaneCommand::SplitWithLaunch { launch, .. }), NativeCommand::Ui(UiCommand::SetPaneBadge { badge: Some(badge), .. })]
            if launch.cwd.as_deref() == Some(expected.as_str()) && badge == "Requested: project"),
                        "order {order:?}: {:?}",
                        result.commands
                    );
                    eval(&mut manager, "ExampleProjectProvider::REGISTRATION.remove; ExampleLaunchObserver::REGISTRATION.remove").unwrap();
                    let result = run_script_request(
                        &mut manager,
                        &context,
                        &ScriptInvocation::UserCommand {
                            name: "project_terminal".into(),
                            pane: PaneId(4),
                        },
                    )
                    .unwrap();
                    assert!(
                        matches!(result.commands.as_slice(), [NativeCommand::Pane(PaneCommand::SplitWithLaunch { launch, .. })] if launch.cwd.as_deref() == context.model.panes[0].cwd.as_deref()),
                        "fallback after removal for {order:?}"
                    );
                }
            }
        }
    }
    assert_eq!(permutations, 24);
}
