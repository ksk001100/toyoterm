use super::tests::script_test_context;
use super::*;

fn eval(manager: &mut ConfigManager, source: &str) -> Result<ScriptResult, ScriptError> {
    run_script_request(
        manager,
        &script_test_context(),
        &ScriptInvocation::Eval(source.into()),
    )
}

fn fire(manager: &mut ConfigManager, id: u64) -> Result<ScriptResult, ScriptError> {
    run_script_request(
        manager,
        &script_test_context(),
        &ScriptInvocation::Timer { id },
    )
}

#[test]
fn timer_schedules_are_typed_and_callbacks_receive_the_timer_and_captured_context() {
    let mut manager = ConfigManager::new().unwrap();
    let result = eval(&mut manager, "@once = Toyoterm.after(0) { |timer, c| c.pane.badge = timer.active?.to_s }; @repeat = Toyoterm.every(0.0101) { |timer, c| timer.cancel; c.pane.badge = 'repeat' }").unwrap();
    assert_eq!(
        result.snapshot.unwrap().timers,
        vec![
            ScriptTimer {
                id: 1,
                delay: Duration::ZERO,
                interval: None
            },
            ScriptTimer {
                id: 2,
                delay: Duration::from_millis(11),
                interval: Some(Duration::from_millis(11))
            },
        ]
    );
    let result = fire(&mut manager, 1).unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::Ui(UiCommand::SetPaneBadge {
            pane: PaneId(4),
            badge: Some("false".into())
        })]
    );
    assert_eq!(manager.eval("@once.active?").unwrap(), "false");
    assert!(fire(&mut manager, 1).unwrap().commands.is_empty());
    let result = fire(&mut manager, 2).unwrap();
    assert!(result.snapshot.unwrap().timers.is_empty());
    assert_eq!(manager.eval("@repeat.cancel").unwrap(), "false");
}

#[test]
fn failed_one_shots_are_consumed_and_failed_repeats_keep_their_schedule() {
    let mut manager = ConfigManager::new().unwrap();
    eval(&mut manager, "@once = Toyoterm.after(1) { Toyoterm.after(2) {}; raise 'failed once' }; @repeat = Toyoterm.every(1) { |timer| timer.cancel; raise 'failed repeat' }").unwrap();
    assert!(fire(&mut manager, 1).is_err());
    assert_eq!(manager.eval("@once.active?").unwrap(), "false");
    assert!(fire(&mut manager, 2).is_err());
    assert_eq!(manager.eval("@repeat.active?").unwrap(), "true");
    let result = eval(&mut manager, "nil").unwrap();
    assert_eq!(
        result
            .snapshot
            .unwrap()
            .timers
            .iter()
            .map(|timer| timer.id)
            .collect::<Vec<_>>(),
        vec![2]
    );
}

#[test]
fn timers_validate_delays_and_bounds_without_reusing_failed_ids() {
    let mut manager = ConfigManager::new().unwrap();
    assert!(manager.reload("Toyoterm.after(1) {} ").is_err());
    for source in [
        "Toyoterm.after(1)",
        "Toyoterm.after(-1) {}",
        "Toyoterm.after('1') {}",
        "Toyoterm.every(0) {}",
        "Toyoterm.every(0.009) {}",
        "Toyoterm.after(365 * 86400 + 1) {}",
        "Toyoterm.after(0.0 / 0.0) {}",
    ] {
        assert!(eval(&mut manager, source).is_err(), "{source}");
    }
    assert!(eval(&mut manager, "@lost = Toyoterm.after(1) {}; raise 'failed'").is_err());
    assert_eq!(manager.eval("@lost.active?").unwrap(), "false");
    let result = eval(&mut manager, "Toyoterm.after(1) {} ").unwrap();
    assert_eq!(result.snapshot.unwrap().timers[0].id, 2);
    eval(&mut manager, "1023.times { Toyoterm.after(1) {} }").unwrap();
    assert!(eval(&mut manager, "Toyoterm.after(1) {} ").is_err());
    eval(
        &mut manager,
        "Toyoterm.timers.first.cancel; Toyoterm.after(1) {} ",
    )
    .unwrap();
}

#[test]
fn task_cancellation_rolls_back_with_failed_mode_teardown() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      Toyoterm.keymap(:empty) {}
      Toyoterm.mode(:jobs, keymap: :empty) do |enabled, context|
        if enabled
          @timer = Toyoterm.every(1) {}
          @task = Toyoterm.async('example-program') { |result| raise 'should be cancelled' }
        else
          @timer.cancel
          @task.cancel
          raise 'failed teardown' if @fail
        end
      end
    "#,
        )
        .unwrap();
    let result = eval(&mut manager, "Toyoterm.mode(:jobs).enable; @fail = true").unwrap();
    assert_eq!(result.async_requests.len(), 1);
    assert!(eval(&mut manager, "Toyoterm.mode(:jobs).disable").is_err());
    assert_eq!(
        manager
            .eval("[@timer.active?, @task.pending?, @task.cancelled?]")
            .unwrap(),
        "[true, true, false]"
    );
    assert!(
        eval(&mut manager, "nil")
            .unwrap()
            .async_cancellations
            .is_empty()
    );
    let result = eval(&mut manager, "@fail = false; Toyoterm.mode(:jobs).disable").unwrap();
    assert!(result.snapshot.unwrap().timers.is_empty());
    assert_eq!(result.async_cancellations, vec![1]);
    assert_eq!(
        manager.eval("[@timer.active?, @task.cancelled?]").unwrap(),
        "[false, true]"
    );
    assert!(fire(&mut manager, 1).unwrap().commands.is_empty());
}

#[test]
fn mode_timer_callbacks_own_descendant_tasks_and_can_disable_their_mode() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      Toyoterm.keymap(:empty) {}
      Toyoterm.mode(:jobs, keymap: :empty) do |enabled, context|
        Toyoterm.every(1) do
          @child = Toyoterm.after(2) {}
          @task = Toyoterm.async('example-program') {}
          Toyoterm.command(:from_timer) {}
          Toyoterm.mode(:jobs).disable
        end if enabled
      end
    "#,
        )
        .unwrap();
    eval(&mut manager, "Toyoterm.mode(:jobs).enable").unwrap();
    let result = fire(&mut manager, 1).unwrap();
    assert!(result.async_requests.is_empty());
    assert!(result.snapshot.unwrap().timers.is_empty());
    assert!(!manager.snapshot().user_command_names.contains("from_timer"));
    assert_eq!(
        manager.eval("[@child.active?, @task.cancelled?]").unwrap(),
        "[false, true]"
    );
}

#[test]
fn completed_async_callbacks_inherit_mode_ownership_without_accumulating_resources() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      Toyoterm.keymap(:empty) {}
      Toyoterm.mode(:jobs, keymap: :empty) do |enabled, context|
        Toyoterm.every(1) do
          Toyoterm.async('example-program') { Toyoterm.command(:from_async, replace: true) {} }
        end if enabled
      end
    "#,
        )
        .unwrap();
    eval(&mut manager, "Toyoterm.mode(:jobs).enable").unwrap();
    for _ in 0..8 {
        let result = fire(&mut manager, 1).unwrap();
        let id = result.async_requests[0].id;
        run_script_request(
            &mut manager,
            &script_test_context(),
            &ScriptInvocation::AsyncCallback {
                id,
                output: AsyncProcessOutput {
                    stdout: vec![],
                    stderr: vec![],
                    exit_status: 0,
                    launch_error: false,
                },
            },
        )
        .unwrap();
    }
    assert!(manager.snapshot().user_command_names.contains("from_async"));
    assert_eq!(
        manager
            .eval("Toyoterm.instance_variable_get(:@mode_instances).values.first[1].length <= 3")
            .unwrap(),
        "true"
    );
    eval(&mut manager, "Toyoterm.mode(:jobs).disable").unwrap();
    assert!(!manager.snapshot().user_command_names.contains("from_async"));
}

#[test]
fn closing_the_origin_prunes_timers_and_owned_async_delivery_and_reload_resets_them() {
    let mut manager = ConfigManager::new().unwrap();
    manager.reload("Toyoterm.keymap(:empty) {}; Toyoterm.mode(:jobs, keymap: :empty) { |enabled| if enabled; @task = Toyoterm.async('example-program') {}; end }").unwrap();
    eval(
        &mut manager,
        "@timer = Toyoterm.every(1) {}; Toyoterm.mode(:jobs).enable",
    )
    .unwrap();
    let mut context = script_test_context();
    let model = Arc::make_mut(&mut context.model);
    model.current_pane = PaneId(5);
    model.tabs[0].panes = vec![PaneId(5)];
    model.panes[0].id = PaneId(5);
    context.handles = vec![
        WorkspaceId(1).into(),
        WindowId(2).into(),
        TabId(3).into(),
        PaneId(5).into(),
    ]
    .into();
    let result =
        run_script_request(&mut manager, &context, &ScriptInvocation::Timer { id: 1 }).unwrap();
    assert!(result.snapshot.unwrap().timers.is_empty());
    assert_eq!(result.async_cancellations, vec![1]);
    assert_eq!(
        manager.eval("[@timer.active?, @task.cancelled?]").unwrap(),
        "[false, true]"
    );
    eval(&mut manager, "Toyoterm.every(1) {} ").unwrap();
    assert!(manager.reload("raise 'failed reload'").is_err());
    assert_eq!(manager.snapshot().timers.len(), 1);
    manager.reload("").unwrap();
    assert!(manager.snapshot().timers.is_empty());
}

#[test]
fn bundled_timer_example_ticks_and_cancels_through_its_mode_keymap() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(include_str!("../../../examples/timers.rb"))
        .unwrap();
    let context = script_test_context();
    let result = run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::UserCommand {
            name: "toggle_clock".into(),
            pane: PaneId(4),
        },
    )
    .unwrap();
    let snapshot = result.snapshot.unwrap();
    assert_eq!(snapshot.timers[0].interval, Some(Duration::from_secs(1)));
    let owner = snapshot
        .keymap_bindings_for(PaneId(4))
        .unwrap()
        .callback_owners["ESCAPE"];
    let tick = fire(&mut manager, snapshot.timers[0].id).unwrap();
    assert!(
        matches!(&tick.commands[0], NativeCommand::Ui(UiCommand::SetPaneBadge { pane: PaneId(4), badge: Some(clock) }) if clock.split(':').count() == 3)
    );
    let result = run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::KeymapBinding {
            map: owner,
            key: "ESCAPE".into(),
            pane: PaneId(4),
        },
    )
    .unwrap();
    assert!(result.snapshot.unwrap().timers.is_empty());
    assert_eq!(
        result.commands,
        vec![NativeCommand::Ui(UiCommand::SetPaneBadge {
            pane: PaneId(4),
            badge: None
        })]
    );
    run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::UserCommand {
            name: "reminder".into(),
            pane: PaneId(4),
        },
    )
    .unwrap();
    assert_eq!(manager.eval("Toyoterm.timers.first.delay").unwrap(), "5.0");
    assert!(manager.eval("Toyoterm.timers.clear").is_err());
    assert!(
        manager
            .eval("Toyoterm.timers.first.instance_variable_set(:@id, 999)")
            .is_err()
    );
}

#[test]
fn failed_async_delivery_is_consumed_while_nested_task_changes_roll_back() {
    let mut manager = ConfigManager::new().unwrap();
    eval(&mut manager, "@other = Toyoterm.async('other') {}; @done = Toyoterm.async('done') { @other.cancel; @lost = Toyoterm.async('new') {}; Toyoterm.after(1) {}; raise 'failed completion' }").unwrap();
    assert!(
        run_script_request(
            &mut manager,
            &script_test_context(),
            &ScriptInvocation::AsyncCallback {
                id: 2,
                output: AsyncProcessOutput {
                    stdout: b"done".to_vec(),
                    stderr: vec![],
                    exit_status: 0,
                    launch_error: false
                }
            }
        )
        .is_err()
    );
    assert_eq!(
        manager
            .eval("[@done.complete?, @other.pending?, @lost.cancelled?, Toyoterm.timers.empty?]")
            .unwrap(),
        "[true, true, true, true]"
    );
    assert_eq!(
        manager
            .eval("Toyoterm.instance_variable_get(:@async_tasks).keys")
            .unwrap(),
        "[1]"
    );
    assert!(
        eval(&mut manager, "nil")
            .unwrap()
            .async_cancellations
            .is_empty()
    );
}
