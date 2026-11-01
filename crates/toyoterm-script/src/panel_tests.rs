use super::tests::script_test_context;
use super::*;

fn eval(manager: &mut ConfigManager, source: &str) -> Result<ScriptResult, ScriptError> {
    run_script_request(
        manager,
        &script_test_context(),
        &ScriptInvocation::Eval(source.into()),
    )
}

fn close(manager: &mut ConfigManager, id: u64) -> Result<ScriptResult, ScriptError> {
    run_script_request(
        manager,
        &script_test_context(),
        &ScriptInvocation::SelectCallback {
            id,
            selection: None,
        },
    )
}

fn opened(result: &ScriptResult) -> u64 {
    let [NativeCommand::Ui(UiCommand::OpenPanel { id, .. })] = result.commands.as_slice() else {
        panic!("{:?}", result.commands)
    };
    *id
}

#[test]
fn panel_creation_copies_lines_and_updates_use_typed_commands() {
    let mut manager = ConfigManager::new().unwrap();
    let result = eval(&mut manager, "$line = '猫'; @panel = Toyoterm.show_panel(title: 'Tasks', lines: [$line, '', $line]); $line.replace('changed')").unwrap();
    let id = opened(&result);
    assert_eq!(
        result.commands,
        vec![NativeCommand::Ui(UiCommand::OpenPanel {
            id,
            title: "Tasks".into(),
            lines: vec!["猫".into(), "".into(), "猫".into()]
        })]
    );
    assert_eq!(
        manager.eval("@panel.frozen? && @panel.active?").unwrap(),
        "true"
    );
    let result = eval(&mut manager, "@panel.update(lines: ['Done'])").unwrap();
    assert_eq!(result.value.as_deref(), Some("true"));
    assert_eq!(
        result.commands,
        vec![NativeCommand::Ui(UiCommand::UpdatePanel {
            id,
            title: None,
            lines: vec!["Done".into()]
        })]
    );
    let result = eval(&mut manager, "@panel.update(title: 'Finished', lines: [])").unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::Ui(UiCommand::UpdatePanel {
            id,
            title: Some("Finished".into()),
            lines: vec![]
        })]
    );
    assert_eq!(
        eval(&mut manager, "@panel.close").unwrap().commands,
        vec![NativeCommand::Ui(UiCommand::ClosePrompt { id })]
    );
    assert_eq!(
        manager
            .eval("@panel.active? || @panel.close || @panel.update(lines: ['late'])")
            .unwrap(),
        "false"
    );
}

#[test]
fn user_dismissal_consumes_the_panel_slot_without_returning_line_data() {
    let mut manager = ConfigManager::new().unwrap();
    let id = opened(&eval(&mut manager, "@panel = Toyoterm.show_panel(lines: ['one'])").unwrap());
    assert!(eval(&mut manager, "Toyoterm.read_string {}").is_err());
    assert!(eval(&mut manager, "Toyoterm.select(items: ['one']) {}").is_err());
    assert!(eval(&mut manager, "Toyoterm.show_panel(lines: [])").is_err());
    assert!(close(&mut manager, id).unwrap().commands.is_empty());
    assert_eq!(manager.eval("@panel.active?").unwrap(), "false");
    assert!(close(&mut manager, id).unwrap().commands.is_empty());
    let result = eval(&mut manager, "Toyoterm.read_string {}").unwrap();
    assert!(matches!(
        result.commands.as_slice(),
        [NativeCommand::Ui(UiCommand::OpenPrompt { .. })]
    ));
}

#[test]
fn panel_validation_is_bounded_and_failures_do_not_leave_a_pending_slot() {
    let mut manager = ConfigManager::new().unwrap();
    assert!(manager.reload("Toyoterm.show_panel(lines: [])").is_err());
    for source in [
        "Toyoterm.show_panel(lines: nil)",
        "Toyoterm.show_panel(title: 'x' * 257, lines: [])",
        "Toyoterm.show_panel(lines: [1])",
        "Toyoterm.show_panel(lines: [\"line\\nline\"])",
        "Toyoterm.show_panel(lines: ['x' * 4097])",
        "Toyoterm.show_panel(lines: Array.new(4097, 'x'))",
        "Toyoterm.show_panel(lines: Array.new(1025, 'x' * 4096))",
        "Toyoterm.show_panel(lines: []); raise 'failed create'",
    ] {
        assert!(eval(&mut manager, source).is_err(), "{source}");
    }
    let id = opened(&eval(&mut manager, "@panel = Toyoterm.show_panel(lines: [])").unwrap());
    assert!(eval(&mut manager, "@panel.update(lines: [\"bad\\nline\"])").is_err());
    assert_eq!(manager.eval("@panel.active?").unwrap(), "true");
    close(&mut manager, id).unwrap();
}

#[test]
fn panel_updates_and_close_roll_back_and_failed_reload_preserves_handles() {
    let mut manager = ConfigManager::new().unwrap();
    let id = opened(
        &eval(
            &mut manager,
            "@panel = Toyoterm.show_panel(lines: ['keep'])",
        )
        .unwrap(),
    );
    assert!(eval(&mut manager, "@panel.update(lines: ['rollback']); @panel.close; Toyoterm.show_panel(lines: ['new']); raise 'failed'").is_err());
    assert_eq!(manager.eval("@panel.active?").unwrap(), "true");
    assert!(manager.reload("raise 'failed reload'").is_err());
    assert_eq!(manager.eval("@panel.active?").unwrap(), "true");
    assert_eq!(
        eval(&mut manager, "@panel.update(lines: ['keep'])")
            .unwrap()
            .commands,
        vec![NativeCommand::Ui(UiCommand::UpdatePanel {
            id,
            title: None,
            lines: vec!["keep".into()]
        })]
    );
    manager.reload("").unwrap();
    assert!(close(&mut manager, id).unwrap().commands.is_empty());
}

#[test]
fn mode_owned_panels_close_on_disable_and_failed_disable_restores_them() {
    let mut manager = ConfigManager::new().unwrap();
    let result = eval(
        &mut manager,
        r#"
      Toyoterm.keymap(:empty) {}
      Toyoterm.mode(:panel, keymap: :empty) do |enabled|
        if enabled
          @panel = Toyoterm.show_panel(lines: ['Status'])
        else
          @panel.close
          raise 'failed teardown' if @fail
        end
      end
      Toyoterm.mode(:panel).enable
    "#,
    )
    .unwrap();
    let id = opened(&result);
    assert!(eval(&mut manager, "@fail = true; Toyoterm.mode(:panel).disable").is_err());
    assert_eq!(manager.eval("@panel.active?").unwrap(), "true");
    assert_eq!(
        eval(&mut manager, "@fail = false; Toyoterm.mode(:panel).disable")
            .unwrap()
            .commands,
        vec![NativeCommand::Ui(UiCommand::ClosePrompt { id })]
    );
    assert_eq!(manager.eval("@panel.active?").unwrap(), "false");
}

#[test]
fn panel_example_updates_from_a_timer_and_releases_its_mode_after_user_close() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(include_str!("../../../examples/panels.rb"))
        .unwrap();
    let id = opened(&eval(&mut manager, "Toyoterm.call_command(:terminal_monitor)").unwrap());
    let timer = manager.snapshot().timers[0].id;
    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::Timer { id: timer },
    )
    .unwrap();
    assert!(
        matches!(result.commands.as_slice(), [NativeCommand::Ui(UiCommand::UpdatePanel { id: update, lines, .. })] if *update == id && lines.iter().any(|line| line == "Pane: 4"))
    );
    close(&mut manager, id).unwrap();
    run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::Timer { id: timer },
    )
    .unwrap();
    assert!(manager.snapshot().timers.is_empty());
    assert_eq!(
        manager
            .eval("Toyoterm.mode(:terminal_monitor).enabled?")
            .unwrap(),
        "false"
    );
}
