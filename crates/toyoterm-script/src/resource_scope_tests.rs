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
fn scopes_own_registrations_and_nested_scopes_and_remove_them_in_reverse_order() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        r#"
      @scope = Toyoterm.resource_scope(name: :library) do
        @first = Toyoterm.add_hook(:first) {}
        @child = Toyoterm.resource_scope { @second = Toyoterm.add_hook(:second) {} }
        @third = Toyoterm.add_hook(:third) {}
      end
      $order = []
      [@first, @second, @third].each_with_index do |handle, i|
        original = handle.method(:remove)
        handle.define_singleton_method(:remove) { $order << i + 1; original.call }
      end
    "#,
    )
    .unwrap();
    assert_eq!(manager.eval("@scope.frozen? && @scope.name.frozen? && @scope.resources.frozen? && Toyoterm.resource_scopes.frozen?").unwrap(), "true");
    assert_eq!(manager.eval("@scope.resources.length").unwrap(), "3");
    assert_eq!(manager.eval("@scope.close").unwrap(), "true");
    assert_eq!(manager.eval("$order").unwrap(), "[3, 2, 1]");
    assert_eq!(manager.eval("Toyoterm.resource_scopes.empty? && Toyoterm.hooks.empty? && !@child.active? && !@scope.close").unwrap(), "true");
}

#[test]
fn scope_creation_within_and_close_rollback_restore_registrations_and_handles() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        "@scope = Toyoterm.resource_scope { Toyoterm.command(:keep) {} }",
    )
    .unwrap();
    assert!(
        eval(
            &mut manager,
            "@scope.within { Toyoterm.command(:leaked) {}; raise 'failed install' }"
        )
        .is_err()
    );
    assert_eq!(manager.eval("@scope.active? && Toyoterm.command_info(:leaked).nil? && @scope.resources.length == 1").unwrap(), "true");
    assert!(eval(&mut manager, "@scope.close; raise 'failed close'").is_err());
    assert_eq!(
        manager
            .eval("@scope.active? && !Toyoterm.command_info(:keep).nil?")
            .unwrap(),
        "true"
    );
    assert!(eval(&mut manager, "Toyoterm.resource_scope { |scope| @failed = scope; Toyoterm.command(:bad) {}; raise 'failed scope' }").is_err());
    assert_eq!(manager.eval("!@failed.active? && Toyoterm.command_info(:bad).nil? && Toyoterm.resource_scopes.length == 1").unwrap(), "true");
}

#[test]
fn wrapped_callbacks_reenter_ownership_and_skip_execution_after_scope_closure() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        r#"
      @scope = Toyoterm.resource_scope
      $calls = 0
      @wrapped = @scope.wrap do |context|
        $calls += 1
        Toyoterm.command(:descendant) {}
        Toyoterm.after(1) { |t, c| c.pane.badge = 'late' }
        context.pane.badge = 'called'
      end
      @scope.within { Toyoterm.command(:scoped, &@wrapped) }
      Toyoterm.call_command(:scoped)
    "#,
    )
    .unwrap();
    assert_eq!(manager.eval("$calls").unwrap(), "1");
    eval(&mut manager, "@scope.close; @wrapped.call(nil)").unwrap();
    assert_eq!(manager.eval("$calls").unwrap(), "1");
    assert_eq!(
        manager
            .eval("Toyoterm.command_info(:descendant).nil? && Toyoterm.command_info(:scoped).nil?")
            .unwrap(),
        "true"
    );
    assert!(manager.snapshot().timers.is_empty());
    assert!(eval(&mut manager, "@scope.within {}").is_err());
}

#[test]
fn scope_close_failure_restores_panels_and_registrations_atomically() {
    let mut manager = ConfigManager::new().unwrap();
    let result = eval(
        &mut manager,
        r#"
      @scope = Toyoterm.resource_scope do
        @hook = Toyoterm.add_hook(:keep) {}
        @panel = Toyoterm.show_panel(lines: ['keep'])
      end
      $original_remove = @hook.method(:remove)
      @hook.define_singleton_method(:remove) { $original_remove.call; raise 'failed removal' }
    "#,
    )
    .unwrap();
    let [NativeCommand::Ui(UiCommand::OpenPanel { id, .. })] = result.commands.as_slice() else {
        panic!("{:?}", result.commands)
    };
    assert!(eval(&mut manager, "@scope.close").is_err());
    assert_eq!(
        manager
            .eval("@scope.active? && @panel.active? && @hook.active?")
            .unwrap(),
        "true"
    );
    let result = eval(
        &mut manager,
        "@hook.define_singleton_method(:remove) { $original_remove.call }; @scope.close",
    )
    .unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::Ui(UiCommand::ClosePrompt { id: *id })]
    );
}

#[test]
fn modes_and_scopes_follow_lexical_ownership_and_modes_close_their_child_scopes() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        r#"
      @outer = Toyoterm.resource_scope
      Toyoterm.keymap(:empty) {}
      Toyoterm.mode(:scoped, keymap: :empty) do |enabled|
        if enabled
          @child = Toyoterm.resource_scope { Toyoterm.command(:child) {} }
          Toyoterm.command(:mode) {}
        end
      end
      @outer.within { Toyoterm.mode(:scoped).enable }
    "#,
    )
    .unwrap();
    assert_eq!(manager.eval("@outer.resources.length").unwrap(), "1");
    assert_eq!(manager.eval("@child.resources.length").unwrap(), "1");
    eval(&mut manager, "@outer.close").unwrap();
    assert_eq!(manager.eval("!@child.active? && !@outer.active? && !Toyoterm.mode(:scoped).enabled? && Toyoterm.command_info(:child).nil? && Toyoterm.command_info(:mode).nil?").unwrap(), "true");
}

#[test]
fn scoped_timer_callbacks_own_descendants_and_late_async_delivery_is_cancelled() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        r#"
      @scope = Toyoterm.resource_scope(pane: Toyoterm.current_pane) do
        Toyoterm.after(0) do
          Toyoterm.command(:timer_child) {}
          @async = Toyoterm.async('unused-tool') { Toyoterm.command(:leaked) {} }
        end
      end
    "#,
    )
    .unwrap();
    let id = manager.snapshot().timers[0].id;
    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::Timer { id },
    )
    .unwrap();
    let async_id = result.async_requests[0].id;
    eval(&mut manager, "@scope.close").unwrap();
    assert_eq!(
        manager
            .eval("Toyoterm.command_info(:timer_child).nil? && @async.cancelled?")
            .unwrap(),
        "true"
    );
    run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::AsyncCallback {
            id: async_id,
            output: AsyncProcessOutput {
                stdout: vec![],
                stderr: vec![],
                exit_status: 0,
                launch_error: false,
            },
        },
    )
    .unwrap();
    assert_eq!(
        manager.eval("Toyoterm.command_info(:leaked).nil?").unwrap(),
        "true"
    );
}

#[test]
fn closed_scopes_reject_new_resources_from_a_still_running_callback() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        r#"
      @scope = Toyoterm.resource_scope
      @scope.within do
        @scope.close
        begin
          Toyoterm.command(:leaked) {}
        rescue RuntimeError
          $rejected = true
        end
      end
    "#,
    )
    .unwrap();
    assert_eq!(
        manager
            .eval("$rejected && !@scope.active? && Toyoterm.command_info(:leaked).nil?")
            .unwrap(),
        "true"
    );
}

#[test]
fn pane_close_prunes_scope_descendants_and_failed_reload_preserves_scopes() {
    let mut manager = ConfigManager::new().unwrap();
    eval(&mut manager, "@scope = Toyoterm.resource_scope(pane: Toyoterm.current_pane) { @child = Toyoterm.resource_scope { Toyoterm.command(:owned) {} } }").unwrap();
    assert!(manager.reload("raise 'failed reload'").is_err());
    assert_eq!(manager.eval("@scope.active?").unwrap(), "true");
    let mut context = script_test_context();
    context.handles = vec![WorkspaceId(1).into(), WindowId(2).into(), TabId(3).into()].into();
    run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::Eval("nil".into()),
    )
    .unwrap();
    assert_eq!(
        manager
            .eval("!@scope.active? && !@child.active? && Toyoterm.command_info(:owned).nil?")
            .unwrap(),
        "true"
    );
}

#[test]
fn scopes_own_cooperative_work_and_its_descendants_and_restore_cancelled_continuations() {
    let mut manager = ConfigManager::new().unwrap();
    eval(&mut manager, "@scope = Toyoterm.resource_scope(pane: Toyoterm.current_pane) { @work = Toyoterm.work_each([1,2], batch_size: 1) { |item| Toyoterm.command(('owned' + item.to_s)) {} } }").unwrap();
    let id = manager.snapshot().timers[0].id;
    run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::Timer { id },
    )
    .unwrap();
    assert!(eval(&mut manager, "@scope.close; raise 'rollback'").is_err());
    assert_eq!(
        manager.eval("@work.active? && @scope.active?").unwrap(),
        "true"
    );
    let id = manager.snapshot().timers[0].id;
    run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::Timer { id },
    )
    .unwrap();
    assert_eq!(
        manager
            .eval("!Toyoterm.command_info(:owned2).nil?")
            .unwrap(),
        "true"
    );
    eval(&mut manager, "@scope.close").unwrap();
    assert_eq!(manager.eval("@work.state").unwrap(), "cancelled");
    assert_eq!(
        manager
            .eval("Toyoterm.command_info(:owned1).nil? && Toyoterm.command_info(:owned2).nil?")
            .unwrap(),
        "true"
    );
    assert!(manager.snapshot().timers.is_empty());
}

#[test]
fn scoped_typed_command_collection_closes_with_its_defining_scope() {
    let mut manager = ConfigManager::new().unwrap();
    let result = eval(&mut manager, r#"
      @scope = Toyoterm.resource_scope do
        Toyoterm.command(:label, arguments: [{name: :text}]) { |c, values| c.pane.badge = values['text'] }
      end
      Toyoterm.prompt_command(:label)
    "#).unwrap();
    let [NativeCommand::Ui(UiCommand::OpenPrompt { id, .. })] = result.commands.as_slice() else {
        panic!("{:?}", result.commands)
    };
    assert_eq!(
        eval(&mut manager, "@scope.close").unwrap().commands,
        vec![NativeCommand::Ui(UiCommand::ClosePrompt { id: *id })]
    );
    assert!(
        run_script_request(
            &mut manager,
            &script_test_context(),
            &ScriptInvocation::SelectCallback {
                id: *id,
                selection: Some("late".into())
            }
        )
        .unwrap()
        .commands
        .is_empty()
    );
}

#[test]
fn scope_validation_and_capacity_errors_leave_no_partial_installation() {
    let mut manager = ConfigManager::new().unwrap();
    for source in [
        "Toyoterm.resource_scope(name: '')",
        "Toyoterm.resource_scope(name: 1)",
        "Toyoterm.resource_scope(pane: 1)",
        "257.times { @last = Toyoterm.resource_scope }",
    ] {
        assert!(eval(&mut manager, source).is_err(), "{source}");
        assert_eq!(
            manager.eval("Toyoterm.resource_scopes.empty?").unwrap(),
            "true"
        );
    }
    assert_eq!(manager.eval("@last.active?").unwrap(), "false");
    eval(&mut manager, "@scope = Toyoterm.resource_scope").unwrap();
    assert!(eval(&mut manager, "@scope.within").is_err());
    assert!(eval(&mut manager, "@scope.wrap").is_err());
}

#[test]
fn scoped_library_example_installs_closes_and_reinstalls_without_definition_collisions() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(include_str!("../../../examples/resource_scopes.rb"))
        .unwrap();
    let result = eval(&mut manager, "Toyoterm.call_command(:scoped_status)").unwrap();
    let [NativeCommand::Ui(UiCommand::OpenPanel { id, .. })] = result.commands.as_slice() else {
        panic!("{:?}", result.commands)
    };
    assert!(!manager.snapshot().timers.is_empty());
    let result = eval(&mut manager, "ScopedStatusExample.disable").unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::Ui(UiCommand::ClosePrompt { id: *id })]
    );
    assert_eq!(
        manager
            .eval(
                "Toyoterm.command_info(:scoped_status).nil? && Toyoterm.describe_key('ALT+S').nil?"
            )
            .unwrap(),
        "true"
    );
    assert!(manager.snapshot().timers.is_empty());
    eval(&mut manager, "ScopedStatusExample.enable").unwrap();
    assert_eq!(manager.eval("!Toyoterm.command_info(:scoped_status).nil? && !Toyoterm.describe_key('ALT+S').nil?").unwrap(), "true");
}

#[test]
fn scope_mode_handles_do_not_disable_a_later_unowned_activation_of_the_same_mode() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        r#"
      Toyoterm.keymap(:empty) {}
      Toyoterm.mode(:tools, keymap: :empty)
      @scope = Toyoterm.resource_scope { Toyoterm.mode(:tools).enable }
      @handle = @scope.resources.first
      Toyoterm.mode(:tools).disable
      Toyoterm.mode(:tools).enable
      @scope.close
    "#,
    )
    .unwrap();
    assert_eq!(manager.eval("@handle.kind").unwrap(), "mode_activation");
    assert_eq!(
        manager
            .eval("!@handle.active? && Toyoterm.mode(:tools).enabled?")
            .unwrap(),
        "true"
    );
}

#[test]
fn scope_wrappers_preserve_keyword_arguments_and_inner_failures_can_be_rescued_safely() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        r#"
      @scope = Toyoterm.resource_scope
      @wrapped = @scope.wrap { |prefix, text:| prefix + text }
      begin
        @scope.within { Toyoterm.command(:leaked) {}; raise 'failed inner call' }
      rescue RuntimeError
      end
      Toyoterm.command(:outside) {}
    "#,
    )
    .unwrap();
    assert_eq!(
        manager
            .eval("@wrapped.call('hello ', text: 'world')")
            .unwrap(),
        "hello world"
    );
    assert_eq!(
        manager
            .eval("Toyoterm.command_info(:leaked).nil? && @scope.resources.empty?")
            .unwrap(),
        "true"
    );
    eval(&mut manager, "@scope.close").unwrap();
    assert_eq!(
        manager
            .eval("!Toyoterm.command_info(:outside).nil?")
            .unwrap(),
        "true"
    );
}

#[test]
fn automatic_scope_pruning_skips_owned_mode_teardown_even_for_another_live_pane() {
    let mut manager = ConfigManager::new().unwrap();
    let mut context = script_test_context();
    let model = Arc::make_mut(&mut context.model);
    let mut other = model.panes[0].clone();
    other.id = PaneId(5);
    model.panes.push(other);
    model.tabs[0].panes.push(PaneId(5));
    let mut handles = context.handles.to_vec();
    handles.push(PaneId(5).into());
    context.handles = handles.into();
    run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::Eval(
            r#"
      Toyoterm.keymap(:empty) {}
      Toyoterm.mode(:tools, keymap: :empty) { |enabled| raise 'bad teardown' unless enabled }
      @scope = Toyoterm.resource_scope(pane: Toyoterm.current_pane) do
        Toyoterm.mode(:tools).enable(pane: Toyoterm::Pane.new(5))
      end
    "#
            .into(),
        ),
    )
    .unwrap();
    assert!(
        run_script_request(
            &mut manager,
            &context,
            &ScriptInvocation::Eval("@scope.close".into())
        )
        .is_err()
    );
    let model = Arc::make_mut(&mut context.model);
    model.panes.retain(|pane| pane.id != PaneId(4));
    model.tabs[0].panes.retain(|pane| *pane != PaneId(4));
    model.current_pane = PaneId(5);
    context.handles = context
        .handles
        .iter()
        .copied()
        .filter(|handle| *handle != NativeHandle::from(PaneId(4)))
        .collect::<Vec<_>>()
        .into();
    run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::Eval("nil".into()),
    )
    .unwrap();
    assert_eq!(
        manager
            .eval("!@scope.active? && !Toyoterm.mode(:tools).enabled?(pane: Toyoterm::Pane.new(5))")
            .unwrap(),
        "true"
    );
}

#[test]
fn repeated_scope_installation_and_cleanup_do_not_retain_callback_payloads_after_gc() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      Toyoterm.keymap(:churn_keys) { |keys| keys.key('h').split(:right) }
      Toyoterm.mode(:churn_mode, keymap: :churn_keys) do |enabled|
        Toyoterm.every(1) {} if enabled
      end
      def churn_extension_scope
        payload = Array.new(32) { 'retained callback payload' * 32 }
        scope = Toyoterm.resource_scope(name: :churn, pane: Toyoterm.current_pane) do
          Toyoterm.command(:churn_command) { payload.first }
          Toyoterm.add_hook(:churn_hook) { payload.last }
          Toyoterm.advise_action(:split) { payload.length }
          Toyoterm.every(1) { payload.first }
          Toyoterm.async('never-started-by-this-test') { payload.last }
          Toyoterm.work_each([1,2,3]) { payload.length }
          Toyoterm.show_panel(title: 'Churn', lines: ['Read-only'])
          Toyoterm.keymap(:churn_keys).activate
          Toyoterm.keymap(:churn_keys).activate_transient
          Toyoterm.mode(:churn_mode).enable
        end
        scope.close
        nil
      end
    "#,
        )
        .unwrap();
    for _ in 0..10 {
        eval(&mut manager, "churn_extension_scope").unwrap();
    }
    manager.runtime.collect_garbage();
    let baseline = manager.runtime.gc_stats();
    for batch in 0..5 {
        for _ in 0..50 {
            let result = eval(&mut manager, "churn_extension_scope").unwrap();
            assert!(result.async_requests.is_empty());
        }
        assert_eq!(manager.eval("Toyoterm.resource_scopes.empty? && Toyoterm.timers.empty? && Toyoterm.work_tasks.empty? && Toyoterm.hook_handlers(:churn_hook).empty? && Toyoterm.command_info(:churn_command).nil? && Toyoterm.action_advice(:split).empty? && !Toyoterm.mode(:churn_mode).enabled?").unwrap(), "true");
        manager.runtime.collect_garbage();
        let stats = manager.runtime.gc_stats();
        assert_eq!(stats.arena_index, baseline.arena_index);
        assert!(
            stats.live_objects <= baseline.live_objects + 64,
            "batch {batch}: retained VM objects grew from {} to {}",
            baseline.live_objects,
            stats.live_objects
        );
        assert!(manager.snapshot().timers.is_empty());
        assert!(
            manager
                .snapshot()
                .keymap_bindings
                .values()
                .all(|bindings| bindings.transients.is_empty())
        );
    }
}
