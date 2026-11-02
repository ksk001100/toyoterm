use super::tests::script_test_context;
use super::*;

fn eval(manager: &mut ConfigManager, source: &str) -> Result<ScriptResult, ScriptError> {
    run_script_request(
        manager,
        &script_test_context(),
        &ScriptInvocation::Eval(source.into()),
    )
}

fn tick(manager: &mut ConfigManager) -> Result<ScriptResult, ScriptError> {
    let id = manager
        .snapshot()
        .timers
        .first()
        .expect("pending slice timer")
        .id;
    run_script_request(
        manager,
        &script_test_context(),
        &ScriptInvocation::Timer { id },
    )
}

#[test]
fn monotonic_clock_is_finite_nondecreasing_and_available_before_runtime_requests() {
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(manager.eval("a = Toyoterm.monotonic_time; b = Toyoterm.monotonic_time; a.is_a?(Float) && a.finite? && a >= 0 && b >= a").unwrap(), "true");
}

#[test]
fn cooperative_work_limits_batches_and_allows_intervening_requests() {
    let mut manager = ConfigManager::new().unwrap();
    eval(&mut manager, "$seen = []; @task = Toyoterm.work_each([1,2,3], name: :example, batch_size: 1, budget_ms: 100) { |item, c, task| $seen << item; c.pane.badge = item.to_s }").unwrap();
    assert_eq!(manager.eval("@task.state").unwrap(), "pending");
    assert_eq!(manager.eval("$seen").unwrap(), "[]");
    for item in 1..=3 {
        let result = tick(&mut manager).unwrap();
        assert_eq!(
            result.commands,
            vec![NativeCommand::Ui(UiCommand::SetPaneBadge {
                pane: PaneId(4),
                badge: Some(item.to_string())
            })]
        );
        eval(&mut manager, "$seen << :between").unwrap();
    }
    tick(&mut manager).unwrap();
    assert_eq!(
        manager.eval("$seen").unwrap(),
        "[1, :between, 2, :between, 3, :between]"
    );
    assert_eq!(manager.eval("@task.state").unwrap(), "completed");
    assert_eq!(manager.eval("@task.statistics[:processed]").unwrap(), "3");
    assert_eq!(manager.eval("@task.statistics[:slices]").unwrap(), "4");
    assert_eq!(
        manager
            .eval("@task.statistics.frozen? && Toyoterm.work_tasks.empty?")
            .unwrap(),
        "true"
    );
    assert!(manager.snapshot().timers.is_empty());
}

#[test]
fn cooperative_work_uses_the_time_budget_between_items_and_reports_overruns() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        r#"
      def Toyoterm.monotonic_time; @fake_time = (@fake_time || 0.0) + 0.002; end
      @task = Toyoterm.work_each([1,2,3], batch_size: 100, budget_ms: 1) { }
    "#,
    )
    .unwrap();
    tick(&mut manager).unwrap();
    assert_eq!(manager.eval("@task.statistics[:processed]").unwrap(), "1");
    assert_eq!(manager.eval("@task.statistics[:overruns]").unwrap(), "1");
    assert_eq!(manager.eval("@task.statistics[:last_ms] >= 1 && @task.statistics[:total_ms] >= @task.statistics[:last_ms]").unwrap(), "true");
}

#[test]
fn failed_slices_rollback_host_effects_and_never_retry_consumed_iterator_input() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        r#"
      $seen = []
      @task = Toyoterm.work_each([1,2,3], budget_ms: 100) do |item, c|
        $seen << item
        c.pane.badge = item.to_s
        Toyoterm.command(:leaked) {}
        raise 'slice failed'
      end
    "#,
    )
    .unwrap();
    assert!(tick(&mut manager).is_err());
    assert_eq!(manager.eval("@task.state").unwrap(), "failed");
    assert_eq!(
        manager
            .eval("@task.error.include?('slice failed') && @task.error.frozen?")
            .unwrap(),
        "true"
    );
    assert_eq!(
        manager
            .eval("Toyoterm.command_info(:leaked).nil? && Toyoterm.work_tasks.empty?")
            .unwrap(),
        "true"
    );
    assert_eq!(manager.eval("$seen").unwrap(), "[1]");
    eval(&mut manager, "nil").unwrap();
    assert!(manager.snapshot().timers.is_empty());
}

#[test]
fn cancelling_pending_work_is_transactional_and_running_work_stops_between_items() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        "@task = Toyoterm.work_each([1,2,3], budget_ms: 100) { |item, c, task| task.cancel }",
    )
    .unwrap();
    assert!(eval(&mut manager, "@task.cancel; raise 'rollback cancellation'").is_err());
    assert_eq!(manager.eval("@task.active?").unwrap(), "true");
    tick(&mut manager).unwrap();
    assert_eq!(manager.eval("@task.state").unwrap(), "cancelled");
    assert_eq!(manager.eval("@task.statistics[:processed]").unwrap(), "1");
    assert!(manager.snapshot().timers.is_empty());
}

#[test]
fn modes_own_work_and_descendants_and_failed_disable_restores_pending_work() {
    let mut manager = ConfigManager::new().unwrap();
    eval(&mut manager, r#"
      Toyoterm.keymap(:empty) {}
      Toyoterm.mode(:work, keymap: :empty) do |enabled|
        if enabled
          @task = Toyoterm.work_each([1,2,3], batch_size: 1) { |item| Toyoterm.command(('owned' + item.to_s)) {} }
        else
          @task.cancel
          raise 'failed teardown' if @fail
        end
      end
      Toyoterm.mode(:work).enable
    "#).unwrap();
    tick(&mut manager).unwrap();
    assert!(eval(&mut manager, "@fail = true; Toyoterm.mode(:work).disable").is_err());
    assert_eq!(manager.eval("@task.active?").unwrap(), "true");
    eval(&mut manager, "@fail = false; Toyoterm.mode(:work).disable").unwrap();
    assert_eq!(manager.eval("@task.state").unwrap(), "cancelled");
    assert_eq!(
        manager.eval("Toyoterm.command_info(:owned1).nil?").unwrap(),
        "true"
    );
    assert!(manager.snapshot().timers.is_empty());
}

#[test]
fn configuration_validation_failure_is_terminal_even_after_the_iterator_finishes() {
    let mut manager = ConfigManager::new().unwrap();
    eval(&mut manager, "@task = Toyoterm.work_each([1]) { |item, c| c.pane.badge = 'rollback'; Toyoterm.configure { |config| config.theme = 'missing' } }").unwrap();
    assert!(tick(&mut manager).is_err());
    assert_eq!(manager.eval("@task.state").unwrap(), "failed");
    assert_eq!(
        manager
            .eval("@task.error.include?('theme') && Toyoterm.work_tasks.empty?")
            .unwrap(),
        "true"
    );
    eval(&mut manager, "nil").unwrap();
    assert!(manager.snapshot().timers.is_empty());
}

#[test]
fn continuation_timers_keep_the_original_pane_after_focus_changes() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        "@task = Toyoterm.work_each([1,2], batch_size: 1) { |item, c| c.pane.badge = item.to_s }",
    )
    .unwrap();
    let mut context = script_test_context();
    let model = Arc::make_mut(&mut context.model);
    let mut other = model.panes[0].clone();
    other.id = PaneId(5);
    model.panes.push(other);
    model.tabs[0].panes.push(PaneId(5));
    model.current_pane = PaneId(5);
    let mut handles = context.handles.to_vec();
    handles.push(PaneId(5).into());
    context.handles = handles.into();
    for item in 1..=2 {
        let id = manager.snapshot().timers[0].id;
        let result =
            run_script_request(&mut manager, &context, &ScriptInvocation::Timer { id }).unwrap();
        assert_eq!(
            result.commands,
            vec![NativeCommand::Ui(UiCommand::SetPaneBadge {
                pane: PaneId(4),
                badge: Some(item.to_string())
            })]
        );
    }
    assert_eq!(
        manager
            .eval("Toyoterm.timers.first.context.pane.id")
            .unwrap(),
        "4"
    );
}

#[test]
fn cooperative_work_validates_options_and_creation_rollback_leaves_no_schedule() {
    let mut manager = ConfigManager::new().unwrap();
    assert!(manager.reload("Toyoterm.work_each([]) {}").is_err());
    for source in [
        "Toyoterm.work_each(1) {}",
        "Toyoterm.work_each([])",
        "Toyoterm.work_each([], batch_size: 1.0) {}",
        "Toyoterm.work_each([], batch_size: 0) {}",
        "Toyoterm.work_each([], batch_size: 4097) {}",
        "Toyoterm.work_each([], budget_ms: 0) {}",
        "Toyoterm.work_each([], budget_ms: 101) {}",
        "Toyoterm.work_each([], interval: 0) {}",
        "Toyoterm.work_each([], interval: 61) {}",
        "Toyoterm.work_each([], name: '') {}",
        "257.times { Toyoterm.work_each([]) {} }",
        "@task = Toyoterm.work_each([]) {}; raise 'failed creation'",
    ] {
        assert!(eval(&mut manager, source).is_err(), "{source}");
        assert_eq!(manager.eval("Toyoterm.work_tasks.empty?").unwrap(), "true");
    }
    assert_eq!(manager.eval("@task.state").unwrap(), "cancelled");
    assert!(manager.snapshot().timers.is_empty());
}

#[test]
fn closed_targets_are_pruned_and_failed_reload_preserves_pending_work() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        "@task = Toyoterm.work_each([1]) { raise 'stale callback' }",
    )
    .unwrap();
    assert!(manager.reload("raise 'failed reload'").is_err());
    assert_eq!(manager.eval("@task.state").unwrap(), "pending");
    let mut context = script_test_context();
    context.handles = vec![WorkspaceId(1).into(), WindowId(2).into(), TabId(3).into()].into();
    run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::Eval("nil".into()),
    )
    .unwrap();
    assert_eq!(manager.eval("@task.state").unwrap(), "cancelled");
    assert!(manager.snapshot().timers.is_empty());
}

#[test]
fn cooperative_example_counts_snapshot_words_and_releases_tasks_when_its_panel_closes() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(include_str!("../../../examples/cooperative_work.rb"))
        .unwrap();
    let mut context = script_test_context();
    Arc::make_mut(&mut context.model).panes[0].screen_text = "one two\none three".into();
    let result = run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::UserCommand {
            name: "screen_word_counts".into(),
            pane: PaneId(4),
        },
    )
    .unwrap();
    let [NativeCommand::Ui(UiCommand::OpenPanel { id: panel, .. })] = result.commands.as_slice()
    else {
        panic!("{:?}", result.commands)
    };
    let work = manager
        .snapshot()
        .timers
        .iter()
        .find(|timer| timer.interval.is_none())
        .unwrap()
        .id;
    run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::Timer { id: work },
    )
    .unwrap();
    let monitor = manager
        .snapshot()
        .timers
        .iter()
        .find(|timer| timer.interval.is_some())
        .unwrap()
        .id;
    let result = run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::Timer { id: monitor },
    )
    .unwrap();
    assert!(
        matches!(result.commands.as_slice(), [NativeCommand::Ui(UiCommand::UpdatePanel { lines, .. })] if lines.iter().any(|line| line == "\"one\": 2") && lines.iter().any(|line| line == "State: completed"))
    );
    run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::SelectCallback {
            id: *panel,
            selection: None,
        },
    )
    .unwrap();
    run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::Timer { id: monitor },
    )
    .unwrap();
    assert!(manager.snapshot().timers.is_empty());
    assert_eq!(manager.eval("Toyoterm.work_tasks.empty?").unwrap(), "true");
}
