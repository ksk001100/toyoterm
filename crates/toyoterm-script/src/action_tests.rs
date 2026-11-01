use super::tests::script_test_context;
use super::*;

fn eval(manager: &mut ConfigManager, source: &str) -> Result<ScriptResult, ScriptError> {
    run_script_request(
        manager,
        &script_test_context(),
        &ScriptInvocation::Eval(source.into()),
    )
}

fn context() -> ActionContext {
    ActionContext {
        workspace: WorkspaceId(1),
        window: WindowId(2),
        tab: TabId(3),
        pane: PaneId(4),
    }
}

fn invoke(manager: &mut ConfigManager, action: NativeAction) -> Result<ScriptResult, ScriptError> {
    run_script_request(
        manager,
        &script_test_context(),
        &ScriptInvocation::Action {
            action,
            context: context(),
        },
    )
}

fn native(action: NativeAction) -> NativeCommand {
    NativeCommand::Action(ActionCommand::Invoke {
        action,
        context: context(),
    })
}

#[test]
fn native_action_names_and_arguments_round_trip_through_the_ruby_catalog() {
    let mut manager = ConfigManager::new().unwrap();
    let names = manager.eval("Toyoterm.actions.join('|')").unwrap();
    for name in names.split('|') {
        let choices = manager
            .eval(&format!(
                "(Toyoterm::StaticBinding::ACTIONS[:{name}] || ['']).map(&:to_s).join('|')"
            ))
            .unwrap();
        for choice in choices.split('|') {
            let action = decode_native_action(name, choice).unwrap();
            assert_eq!(action.name(), name);
            assert_eq!(action.argument().unwrap_or(""), choice);
        }
    }
    let action = NativeAction::UserCommand("arbitrary command".into());
    assert_eq!(
        (action.name(), action.argument()),
        ("user_command", Some("arbitrary command"))
    );
    assert!(manager.eval("Toyoterm.actions << 'bad'").is_err());
    assert!(
        manager
            .eval("Toyoterm.action_info(:split)[:arguments] << :bad")
            .is_err()
    );
    assert_eq!(
        manager.eval("Toyoterm.action_info(:missing).nil?").unwrap(),
        "true"
    );
    manager.reload("Toyoterm.configure { |c| c.keys.ctrl('d').split(:right) }; Toyoterm.command(:new_tab) {} ").unwrap();
    assert_eq!(
        manager.eval("Toyoterm.action_keys(:split)").unwrap(),
        "[\"CTRL+D\"]"
    );
    assert_eq!(
        manager
            .eval("Toyoterm.operations.select { |i| i[:name] == 'new_tab' }.map { |i| i[:kind] }")
            .unwrap(),
        "[:action, :command]"
    );
}

#[test]
fn action_advice_orders_layers_rewrites_arguments_and_can_suppress_the_operation() {
    let mut manager = ConfigManager::new().unwrap();
    manager.reload(r#"
      @order = []
      Toyoterm.advise_action(:split) { |call| @order << 'before'; raise 'mutable call' unless call.frozen? }
      Toyoterm.advise_action(:split, kind: :around) do |proceed, call|
        @order << 'outer'
        proceed.call(:down)
        @order << 'outer end'
      end
      Toyoterm.advise_action(:split, kind: :around) do |proceed, call|
        @order << call.argument
        proceed.call
      end
      Toyoterm.advise_action(:split, kind: :after) { |call| @order << 'after' }
      Toyoterm.advise_action(:close_pane, kind: :around) { |proceed, call| }
    "#).unwrap();
    assert!(manager.snapshot().advised_actions.contains("split"));
    let result = invoke(&mut manager, NativeAction::Split(SplitDirection::Right)).unwrap();
    assert_eq!(
        result.commands,
        vec![native(NativeAction::Split(SplitDirection::Down))]
    );
    assert_eq!(
        manager.eval("@order").unwrap(),
        "[\"before\", \"outer\", \"down\", \"outer end\", \"after\"]"
    );
    assert!(
        invoke(&mut manager, NativeAction::ClosePane)
            .unwrap()
            .commands
            .is_empty()
    );
    let result = eval(&mut manager, "Toyoterm.action(:split, :right)").unwrap();
    assert_eq!(
        result.commands,
        vec![native(NativeAction::Split(SplitDirection::Down))]
    );
}

#[test]
fn advice_failure_rolls_back_host_changes_even_when_the_caller_rescues() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      @handle = Toyoterm.advise_action(:new_tab, kind: :around) do |proceed, call|
        proceed.call
        Toyoterm.configure { |c| c.window.opacity = 0.5 }
        Toyoterm.command(:discarded) {}
        Toyoterm.after(1) {}
        @task = Toyoterm.async('never-started') {}
        raise 'failed action'
      end
    "#,
        )
        .unwrap();
    let opacity = manager.config().window.opacity;
    let result = eval(
        &mut manager,
        "begin; Toyoterm.action(:new_tab); rescue RuntimeError; end; nil",
    )
    .unwrap();
    assert!(result.commands.is_empty() && result.async_requests.is_empty());
    assert_eq!(manager.config().window.opacity, opacity);
    assert!(!manager.snapshot().user_command_names.contains("discarded"));
    assert!(manager.snapshot().timers.is_empty());
    assert_eq!(manager.eval("@task.cancelled?").unwrap(), "true");
    eval(&mut manager, "@handle.remove").unwrap();
    assert!(manager.snapshot().advised_actions.is_empty());
    assert_eq!(
        invoke(&mut manager, NativeAction::NewTab).unwrap().commands,
        vec![native(NativeAction::NewTab)]
    );
}

#[test]
fn action_continuations_cannot_escape_repeat_or_exceed_the_nesting_bound() {
    let mut manager = ConfigManager::new().unwrap();
    manager.reload("@h = Toyoterm.advise_action(:new_tab, kind: :around) { |proceed, call| @saved = proceed; proceed.call }").unwrap();
    invoke(&mut manager, NativeAction::NewTab).unwrap();
    assert!(
        eval(&mut manager, "@saved.call")
            .unwrap_err()
            .message()
            .contains("no longer active")
    );
    eval(
        &mut manager,
        "@h.remove; @h = Toyoterm.advise_action(:new_tab, kind: :around) { |p, c| p.call; p.call }",
    )
    .unwrap();
    assert!(
        invoke(&mut manager, NativeAction::NewTab)
            .unwrap_err()
            .message()
            .contains("already called")
    );
    eval(
        &mut manager,
        "@h.remove; @h = Toyoterm.advise_action(:new_tab) { Toyoterm.action(:new_tab) }",
    )
    .unwrap();
    assert!(
        invoke(&mut manager, NativeAction::NewTab)
            .unwrap_err()
            .message()
            .contains("nesting exceeds")
    );
    eval(&mut manager, "@h.remove").unwrap();
    assert_eq!(
        invoke(&mut manager, NativeAction::NewTab)
            .unwrap()
            .commands
            .len(),
        1
    );
}

#[test]
fn action_advice_dispatch_uses_a_registration_snapshot_and_modes_release_it() {
    let mut manager = ConfigManager::new().unwrap();
    manager.reload(r#"
      @order = []
      @first = Toyoterm.advise_action(:new_tab) { @later.remove; @order << 'first' }
      @later = Toyoterm.advise_action(:new_tab) { @order << 'later' }
      Toyoterm.keymap(:empty) {}
      Toyoterm.mode(:policy, keymap: :empty) { |enabled| @owned = Toyoterm.advise_action(:split) {} if enabled }
    "#).unwrap();
    invoke(&mut manager, NativeAction::NewTab).unwrap();
    invoke(&mut manager, NativeAction::NewTab).unwrap();
    assert_eq!(
        manager.eval("@order").unwrap(),
        "[\"first\", \"later\", \"first\"]"
    );
    eval(&mut manager, "Toyoterm.mode(:policy).enable").unwrap();
    assert!(manager.snapshot().advised_actions.contains("split"));
    eval(&mut manager, "Toyoterm.mode(:policy).disable").unwrap();
    assert_eq!(manager.eval("@owned.active?").unwrap(), "false");
    assert!(!manager.snapshot().advised_actions.contains("split"));
    assert!(
        manager
            .reload("Toyoterm.advise_action(:split) {}; raise 'failed reload'")
            .is_err()
    );
    assert!(!manager.snapshot().advised_actions.contains("split"));
    manager.reload("").unwrap();
    assert!(manager.snapshot().advised_actions.is_empty());
}

#[test]
fn typed_native_actions_keep_the_original_target_after_the_active_pane_changes() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload("Toyoterm.advise_action(:new_tab) { |call| @target = call.context.pane.id }")
        .unwrap();
    let mut request_context = script_test_context();
    let model = Arc::make_mut(&mut request_context.model);
    model.current_pane = PaneId(5);
    model.tabs[0].panes.push(PaneId(5));
    let mut pane = model.panes[0].clone();
    pane.id = PaneId(5);
    model.panes.push(pane);
    let mut handles = request_context.handles.to_vec();
    handles.push(PaneId(5).into());
    request_context.handles = handles.into();
    let result = run_script_request(
        &mut manager,
        &request_context,
        &ScriptInvocation::Action {
            action: NativeAction::NewTab,
            context: context(),
        },
    )
    .unwrap();
    assert_eq!(result.commands, vec![native(NativeAction::NewTab)]);
    assert_eq!(
        manager.eval("[@target, Toyoterm.current_pane.id]").unwrap(),
        "[4, 5]"
    );
}

#[test]
fn operation_palette_collects_native_arguments_and_rejects_replaced_commands() {
    let mut manager = ConfigManager::new().unwrap();
    manager.reload("Toyoterm.command(:custom) {}; Toyoterm.advise_action(:split, kind: :around) { |p, c| p.call(:down) }").unwrap();
    eval(&mut manager, "Toyoterm.operation_palette").unwrap();
    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::SelectCallback {
            id: 1,
            selection: Some("action: split".into()),
        },
    )
    .unwrap();
    assert!(
        matches!(&result.commands[0], NativeCommand::Ui(UiCommand::OpenPrompt { id: 2, prompt }) if prompt.require_match && prompt.completions.contains(&"left".to_owned()))
    );
    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::SelectCallback {
            id: 2,
            selection: Some("left".into()),
        },
    )
    .unwrap();
    assert_eq!(
        result.commands,
        vec![native(NativeAction::Split(SplitDirection::Down))]
    );
    eval(
        &mut manager,
        "Toyoterm.operation_palette; Toyoterm.command(:custom, replace: true) {}",
    )
    .unwrap();
    assert!(
        run_script_request(
            &mut manager,
            &script_test_context(),
            &ScriptInvocation::SelectCallback {
                id: 3,
                selection: Some("command: custom".into())
            }
        )
        .unwrap_err()
        .message()
        .contains("changed while palette")
    );
}

#[test]
fn action_advice_validates_registrations_and_replacement_arguments_before_queuing() {
    let mut manager = ConfigManager::new().unwrap();
    for source in [
        "Toyoterm.advise_action(:new_tab)",
        "Toyoterm.advise_action(:missing) {}",
        "Toyoterm.advise_action(:new_tab, kind: :invalid) {}",
        "Toyoterm.advise_action(1) {}",
        "Toyoterm.invoke_operation(:unknown, :new_tab)",
        "Toyoterm.invoke_operation(:action, :new_tab, context: 1)",
    ] {
        assert!(eval(&mut manager, source).is_err(), "{source}");
        assert!(manager.snapshot().advised_actions.is_empty());
    }
    eval(
        &mut manager,
        "@h = Toyoterm.advise_action(:split, kind: :around) { |p, c| p.call(:invalid) }",
    )
    .unwrap();
    assert!(invoke(&mut manager, NativeAction::Split(SplitDirection::Right)).is_err());
    eval(
        &mut manager,
        "@h.remove; Toyoterm.command(:custom) { :done }",
    )
    .unwrap();
    assert_eq!(
        eval(&mut manager, "Toyoterm.invoke_operation(:command, :custom)")
            .unwrap()
            .value
            .as_deref(),
        Some(":done")
    );
    assert!(
        eval(
            &mut manager,
            "Toyoterm.invoke_operation(:command, :custom, argument: 'bad')"
        )
        .is_err()
    );
}

#[test]
fn bundled_action_policy_example_intercepts_and_releases_native_actions() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(include_str!("../../../examples/actions.rb"))
        .unwrap();
    run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::UserCommand {
            name: "toggle_split_policy".into(),
            pane: PaneId(4),
        },
    )
    .unwrap();
    assert_eq!(manager.snapshot().advised_actions.len(), 2);
    assert_eq!(
        invoke(&mut manager, NativeAction::Split(SplitDirection::Right))
            .unwrap()
            .commands,
        vec![native(NativeAction::Split(SplitDirection::Down))]
    );
    let mut running = script_test_context();
    Arc::make_mut(&mut running.model).panes[0].command_running = true;
    let result = run_script_request(
        &mut manager,
        &running,
        &ScriptInvocation::Action {
            action: NativeAction::ClosePane,
            context: context(),
        },
    )
    .unwrap();
    assert!(result.commands.is_empty());
    assert_eq!(result.logs.len(), 1);
    let snapshot = manager.snapshot();
    let owner = snapshot
        .keymap_bindings_for(PaneId(4))
        .unwrap()
        .callback_owners["ESCAPE"];
    run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::KeymapBinding {
            map: owner,
            key: "ESCAPE".into(),
            pane: PaneId(4),
        },
    )
    .unwrap();
    assert!(manager.snapshot().advised_actions.is_empty());
    assert_eq!(
        invoke(&mut manager, NativeAction::Split(SplitDirection::Right))
            .unwrap()
            .commands,
        vec![native(NativeAction::Split(SplitDirection::Right))]
    );
    let result = eval(&mut manager, "Toyoterm.operation_palette").unwrap();
    assert!(
        matches!(&result.commands[0], NativeCommand::Ui(UiCommand::OpenSelector { items, .. }) if items.contains(&"action: split".to_owned()) && items.contains(&"command: toggle_split_policy".to_owned()))
    );
}
