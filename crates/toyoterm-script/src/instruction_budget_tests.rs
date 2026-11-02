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
fn instruction_budgets_validate_limits_and_preserve_results() {
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(
        manager
            .eval(
                r#"
      raise unless Toyoterm.supports?(:instruction_budgets)
      [nil, true, '100', 1.5].each do |value|
        begin
          Toyoterm.with_instruction_budget(instructions: value) { 1 }
          raise 'accepted invalid type'
        rescue TypeError
        end
      end
      [0, -1, 1_000_000_001].each do |value|
        begin
          Toyoterm.with_instruction_budget(instructions: value) { 1 }
          raise 'accepted invalid range'
        rescue ArgumentError
        end
      end
      begin
        Toyoterm.with_instruction_budget(instructions: 100)
        raise 'accepted missing block'
      rescue ArgumentError
      end
      Toyoterm.with_instruction_budget(instructions: 1000) { [42, :ok] }
    "#
            )
            .unwrap(),
        "[42, :ok]"
    );
}

#[test]
fn instruction_budget_exhaustion_unwinds_ensures_and_rolls_back_host_effects() {
    let mut manager = ConfigManager::new().unwrap();
    let result = eval(
        &mut manager,
        r#"
      Toyoterm.current_pane.badge = 'before'
      begin
        Toyoterm.with_instruction_budget(instructions: 1000) do
          begin
            Toyoterm.current_pane.badge = 'discard'
            Toyoterm.command('discard') { nil }
            Toyoterm.after(10) { nil }
            i = 0
            while i < 1_000_000; i += 1; end
            raise 'did not interrupt'
          ensure
            $budget_ensured = true
            Toyoterm.current_pane.badge = 'also discard'
          end
        end
      rescue Toyoterm::InstructionBudgetError => error
        raise unless error.message == 'instruction budget exhausted'
      end
      [$budget_ensured, Toyoterm.commands.include?('discard'), Toyoterm.timers.empty?]
    "#,
    )
    .unwrap();
    assert_eq!(result.value.as_deref(), Some("[true, false, true]"));
    assert_eq!(
        result.commands,
        vec![NativeCommand::Ui(UiCommand::SetPaneBadge {
            pane: PaneId(4),
            badge: Some("before".into()),
        })]
    );
    assert_eq!(
        manager
            .eval("i = 0; while i < 100_000; i += 1; end; i")
            .unwrap(),
        "100000"
    );
}

#[test]
fn nested_instruction_budgets_count_outer_work_and_restore_active_frames() {
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(
        manager
            .eval(
                r#"
      $inner = false
      begin
        Toyoterm.with_instruction_budget(instructions: 100_000) do
          begin
            Toyoterm.with_instruction_budget(instructions: 10) { 10_000.times { nil } }
          rescue Toyoterm::InstructionBudgetError
            $inner = true
          end
          Toyoterm.with_instruction_budget(instructions: 1_000_000_000) { 1_000_000.times { nil } }
          raise 'outer limit lost'
        end
      rescue Toyoterm::InstructionBudgetError
      end
      [$inner, Toyoterm.with_instruction_budget(instructions: 1000) { :recovered }]
    "#
            )
            .unwrap(),
        "[true, :recovered]"
    );
}

#[test]
fn rescued_exhaustion_cannot_commit_and_ordinary_exceptions_also_rollback() {
    let mut manager = ConfigManager::new().unwrap();
    let result = eval(
        &mut manager,
        r#"
      errors = []
      begin
        Toyoterm.with_instruction_budget(instructions: 100) do
          begin
            10_000.times { nil }
          rescue Toyoterm::InstructionBudgetError
            Toyoterm.current_pane.badge = 'discard'
            :rescued
          end
        end
      rescue Toyoterm::InstructionBudgetError
        errors << :budget
      end
      begin
        Toyoterm.with_instruction_budget(instructions: 1000) do
          Toyoterm.current_pane.badge = 'discard ordinary'
          raise ArgumentError, 'ordinary'
        end
      rescue ArgumentError
        errors << :ordinary
      end
      errors
    "#,
    )
    .unwrap();
    assert_eq!(result.value.as_deref(), Some("[:budget, :ordinary]"));
    assert!(result.commands.is_empty());
}

#[test]
fn instruction_budgets_restore_frames_after_nonlocal_return_break_and_gc() {
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(
        manager
            .eval(
                r#"
      def budget_return
        Toyoterm.with_instruction_budget(instructions: 1000) { return :returned }
        :wrong
      end
      values = [budget_return]
      values << Toyoterm.with_instruction_budget(instructions: 1000) { break :broken }
      values << Toyoterm.with_instruction_budget(instructions: 10000) { GC.start; :collected }
      20.times do
        begin
          Toyoterm.with_instruction_budget(instructions: 1) { 100.times { nil } }
        rescue Toyoterm::InstructionBudgetError
        end
        GC.start
      end
      values << Toyoterm.with_instruction_budget(instructions: 1000) { :alive }
      values
    "#
            )
            .unwrap(),
        "[:returned, :broken, :collected, :alive]"
    );
}

#[test]
fn instruction_budgets_do_not_leave_stack_frames_in_suspended_fibers() {
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(
        manager
            .eval(
                r#"
      f = Fiber.new do
        Toyoterm.with_instruction_budget(instructions: 1000) { Fiber.yield(:unsafe) }
      end
      begin
        f.resume
        raise 'yield crossed budget C frame'
      rescue FiberError
      end
      g = Fiber.new { Fiber.yield(:first); :second }
      first = Toyoterm.with_instruction_budget(instructions: 1000) { g.resume }
      [first, g.resume, Toyoterm.with_instruction_budget(instructions: 1000) { :alive }]
    "#
            )
            .unwrap(),
        "[:first, :second, :alive]"
    );
}

#[test]
fn exhausted_budgets_override_nonlocal_exits_and_cleanup_errors() {
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(
        manager
            .eval(
                r#"
      def exhausted_return
        Toyoterm.with_instruction_budget(instructions: 10) do
          begin
            10000.times { nil }
          rescue Toyoterm::InstructionBudgetError
            return :escaped
          end
        end
      end
      caught = 0
      begin
        exhausted_return
      rescue Toyoterm::InstructionBudgetError
        caught += 1
      end
      begin
        Toyoterm.with_instruction_budget(instructions: 10) do
          begin
            10000.times { nil }
          rescue Toyoterm::InstructionBudgetError
            break :escaped
          end
        end
      rescue Toyoterm::InstructionBudgetError
        caught += 1
      end
      begin
        Toyoterm.with_instruction_budget(instructions: 10) do
          begin
            10000.times { nil }
          ensure
            raise ArgumentError, 'cleanup'
          end
        end
      rescue Toyoterm::InstructionBudgetError
        caught += 1
      end
      caught
    "#
            )
            .unwrap(),
        "3"
    );
}

#[test]
fn instruction_budget_nesting_is_bounded_and_vm_state_is_independent() {
    let mut first = ConfigManager::new().unwrap();
    let mut second = ConfigManager::new().unwrap();
    let error = first
        .eval(
            r#"
      def recursive_budget(n)
        Toyoterm.with_instruction_budget(instructions: 1_000_000_000) do
          n == 0 ? :done : recursive_budget(n - 1)
        end
      end
      recursive_budget(64)
    "#,
        )
        .unwrap_err();
    assert!(
        error
            .message()
            .contains("instruction budget nesting exceeds 64")
    );
    assert_eq!(first.eval("recursive_budget(63)").unwrap(), "done");
    assert_eq!(
        second
            .eval("Toyoterm.with_instruction_budget(instructions: 1000) { :separate }")
            .unwrap(),
        "separate"
    );
}

#[test]
fn budgeted_cooperative_item_failure_is_retained_after_request_rollback() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        r#"
      @work = Toyoterm.work_each([1, 2]) do |item, context, task|
        Toyoterm.with_instruction_budget(instructions: 1000) do
          context.pane.badge = 'discard'
          1_000_000.times { nil }
        end
      end
    "#,
    )
    .unwrap();
    let id = manager.snapshot().timers[0].id;
    let error = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::Timer { id },
    )
    .unwrap_err();
    assert!(error.message().contains("instruction budget exhausted"));
    assert_eq!(
        manager
            .eval("[@work.state, @work.statistics[:processed], Toyoterm.timers.empty?]")
            .unwrap(),
        "[:failed, 0, true]"
    );
    assert_eq!(
        eval(&mut manager, "Toyoterm.current_pane.badge")
            .unwrap()
            .value
            .as_deref(),
        Some("nil")
    );
}

#[test]
fn budget_guard_works_during_load_and_example_summarizes_terminal_text() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      begin
        Toyoterm.with_instruction_budget(instructions: 1000) do
          Toyoterm.configure { |c| c.scrollback_lines = 987 }
          100_000.times { nil }
        end
      rescue Toyoterm::InstructionBudgetError
      end
    "#,
        )
        .unwrap();
    assert_ne!(manager.snapshot().config.scrollback_lines, 987);
    manager
        .reload(include_str!("../../../examples/instruction_budgets.rb"))
        .unwrap();
    let mut context = script_test_context();
    Arc::make_mut(&mut context.model).panes[0].screen_text = "one two\none three".into();
    let result = run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::Eval("Toyoterm.call_command(:bounded_screen_summary)".into()),
    )
    .unwrap();
    assert!(result.commands.iter().any(|command| matches!(command,
        NativeCommand::Ui(UiCommand::OpenPanel { title, lines, .. })
          if title == "Bounded screen summary" && lines == &["\"one\": 2", "\"three\": 1", "\"two\": 1"]
    )));
}

/// Same workload before/after enabling the VM hook; no timing assertions.
#[test]
#[ignore = "opt-in instruction-budget performance baseline"]
fn performance_instruction_budget() {
    let mut manager = ConfigManager::new().unwrap();
    manager.eval("def budget_workload; i = 0; total = 0; while i < 500_000; total += i; i += 1; end; total; end").unwrap();
    let available = manager
        .eval("Toyoterm.respond_to?(:with_instruction_budget)")
        .unwrap()
        == "true";
    for guarded in [false, true] {
        if guarded && !available {
            continue;
        }
        let source = if guarded {
            "Toyoterm.with_instruction_budget(instructions: 100_000_000) { budget_workload }"
        } else {
            "budget_workload"
        };
        let mut samples = Vec::new();
        for iteration in 0..23 {
            let start = Instant::now();
            assert_eq!(manager.eval(source).unwrap(), "124999750000");
            if iteration >= 3 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "instruction_budget guarded={guarded} median_ms={:.3} p95_ms={:.3}",
            samples[samples.len() / 2],
            samples[samples.len() * 95 / 100]
        );
    }
}
