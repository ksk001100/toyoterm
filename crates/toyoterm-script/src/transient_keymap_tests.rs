use super::tests::script_test_context;
use super::*;

fn eval(manager: &mut ConfigManager, source: &str) -> Result<ScriptResult, ScriptError> {
    run_script_request(
        manager,
        &script_test_context(),
        &ScriptInvocation::Eval(source.into()),
    )
}

fn invoke(
    manager: &mut ConfigManager,
    id: u64,
    key: Option<&str>,
) -> Result<ScriptResult, ScriptError> {
    run_script_request(
        manager,
        &script_test_context(),
        &ScriptInvocation::TransientKeymap {
            id,
            key: key.map(str::to_owned),
            pane: PaneId(4),
        },
    )
}

fn layers(manager: &mut ConfigManager) -> Vec<TransientKeymap> {
    manager.refresh_config_snapshot().unwrap();
    manager
        .snapshot()
        .keymap_bindings_for(PaneId(4))
        .map(|bindings| bindings.transients.clone())
        .unwrap_or_default()
}

#[test]
fn transient_keymaps_stack_local_before_global_and_are_inspectable_and_removable() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        r#"
      Toyoterm.configure { |c| c.keys.key('b').new_tab }
      @map = Toyoterm.keymap(:tools) { key('z').toggle_zoom }
      @local = @map.activate_transient
      @global = @map.activate_transient(pane: nil, once: false)
    "#,
    )
    .unwrap();
    let maps = layers(&mut manager);
    assert_eq!(maps.len(), 2);
    assert!(maps[0].once);
    assert!(!maps[1].once);
    assert_eq!(maps[0].keys, HashSet::from(["Z".into()]));
    assert_eq!(
        manager.eval("Toyoterm.key_bindings.keys").unwrap(),
        "[\"Z\"]"
    );
    assert_eq!(
        manager
            .eval("Toyoterm.describe_key('z')[:transient] == @local.id")
            .unwrap(),
        "true"
    );
    assert!(
        manager
            .eval("Toyoterm.transient_keymaps[0][:once] = false")
            .is_err()
    );
    eval(&mut manager, "@local.remove; @global.remove").unwrap();
    assert!(layers(&mut manager).is_empty());
    assert_eq!(
        manager.eval("Toyoterm.key_bindings.keys").unwrap(),
        "[\"B\"]"
    );
}

#[test]
fn one_shot_transients_dispatch_native_actions_through_advice_and_are_consumed_once() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        r#"
      Toyoterm.keymap(:tools) { key('s').split(:right) }.activate_transient
      Toyoterm.advise_action(:split, kind: :around) { |proceed| proceed.call(:down) }
    "#,
    )
    .unwrap();
    let id = layers(&mut manager)[0].id;
    let result = invoke(&mut manager, id, Some("S")).unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::Action(ActionCommand::Invoke {
            action: NativeAction::Split(SplitDirection::Down),
            context: ActionContext {
                workspace: WorkspaceId(1),
                window: WindowId(2),
                tab: TabId(3),
                pane: PaneId(4)
            },
        })]
    );
    assert!(layers(&mut manager).is_empty());
    assert!(
        invoke(&mut manager, id, Some("S"))
            .unwrap()
            .commands
            .is_empty()
    );
}

#[test]
fn repeat_transients_dispatch_inherited_callbacks_and_commands_then_cancel() {
    let mut manager = ConfigManager::new().unwrap();
    eval(&mut manager, r#"
      Toyoterm.command(:badge) { |c| c.pane.badge = 'command' }
      Toyoterm.keymap(:base) { sequence('CTRL+X', 'B').run { |c| c.pane.badge = 'parent' } }
      Toyoterm.keymap(:child, parent: :base) { key('b').command(:badge) }.activate_transient(once: false)
    "#).unwrap();
    let id = layers(&mut manager)[0].id;
    for (key, badge) in [("CTRL+X B", "parent"), ("B", "command")] {
        assert_eq!(
            invoke(&mut manager, id, Some(key)).unwrap().commands,
            vec![NativeCommand::Ui(UiCommand::SetPaneBadge {
                pane: PaneId(4),
                badge: Some(badge.into())
            })]
        );
        assert_eq!(layers(&mut manager).len(), 1);
    }
    assert!(invoke(&mut manager, id, None).unwrap().commands.is_empty());
    assert!(layers(&mut manager).is_empty());
}

#[test]
fn failed_transient_callbacks_roll_back_effects_but_preserve_consumption() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        r#"
      @map = Toyoterm.keymap(:bad) do
        key('b').run do |c|
          c.pane.badge = 'rollback'
          Toyoterm.command(:leaked) {}
          @map.activate_transient
          raise 'failed'
        end
      end
      @map.activate_transient
    "#,
    )
    .unwrap();
    let id = layers(&mut manager)[0].id;
    assert!(invoke(&mut manager, id, Some("B")).is_err());
    assert!(layers(&mut manager).is_empty());
    assert_eq!(
        manager.eval("Toyoterm.command_info(:leaked).nil?").unwrap(),
        "true"
    );
    eval(&mut manager, "@map.activate_transient(once: false)").unwrap();
    let id = layers(&mut manager)[0].id;
    assert!(invoke(&mut manager, id, Some("B")).is_err());
    assert_eq!(layers(&mut manager).len(), 1);
}

#[test]
fn mode_cleanup_owns_transients_and_their_callback_resources() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        r#"
      @tools = Toyoterm.keymap(:tools) { key('b').run { Toyoterm.command(:owned) {} } }
      Toyoterm.keymap(:empty) {}
      Toyoterm.mode(:tools, keymap: :empty) { |on| @tools.activate_transient(once: false) if on }
      Toyoterm.mode(:tools).enable
    "#,
    )
    .unwrap();
    let id = layers(&mut manager)[0].id;
    invoke(&mut manager, id, Some("B")).unwrap();
    assert_eq!(
        manager.eval("Toyoterm.command_info(:owned).nil?").unwrap(),
        "false"
    );
    eval(&mut manager, "Toyoterm.mode(:tools).disable").unwrap();
    assert!(layers(&mut manager).is_empty());
    assert_eq!(
        manager.eval("Toyoterm.command_info(:owned).nil?").unwrap(),
        "true"
    );
}

#[test]
fn transient_validation_loading_reload_and_transaction_rules_are_enforced() {
    let mut manager = ConfigManager::new().unwrap();
    assert!(
        manager
            .reload("Toyoterm.keymap(:tools) { key('b').new_tab }.activate_transient(pane: nil)")
            .is_err()
    );
    eval(
        &mut manager,
        r#"
      @map = Toyoterm.keymap(:tools) { key('b').new_tab }
      Toyoterm.keymap(:empty) {}
      Toyoterm.keymap(:cancel) { key('ESCAPE').new_tab }
    "#,
    )
    .unwrap();
    for source in [
        "@map.activate_transient(once: 1)",
        "@map.activate_transient(pane: 1)",
        "Toyoterm.keymap(:empty).activate_transient",
        "Toyoterm.keymap(:cancel).activate_transient",
        "@map.activate_transient; raise 'rollback'",
    ] {
        assert!(eval(&mut manager, source).is_err(), "{source}");
        assert!(layers(&mut manager).is_empty());
    }
    eval(&mut manager, "@map.activate_transient").unwrap();
    let id = layers(&mut manager)[0].id;
    assert!(manager.reload("raise 'failed reload'").is_err());
    assert_eq!(layers(&mut manager)[0].id, id);
    manager.reload("").unwrap();
    assert!(layers(&mut manager).is_empty());
}

#[test]
fn edited_active_maps_are_validated_and_failed_changes_roll_back() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        "@map = Toyoterm.keymap(:tools) { key('b').new_tab }; @map.activate_transient",
    )
    .unwrap();
    for source in ["@map.keys.unbind('B')", "@map.keys.key('ESCAPE').new_tab"] {
        assert!(eval(&mut manager, source).is_err());
        assert_eq!(layers(&mut manager)[0].keys, HashSet::from(["B".into()]));
    }
    eval(&mut manager, "@map.keys.key('z').toggle_zoom").unwrap();
    assert_eq!(
        layers(&mut manager)[0].keys,
        HashSet::from(["B".into(), "Z".into()])
    );
    assert!(eval(&mut manager, "64.times { @map.activate_transient }").is_err());
    assert_eq!(layers(&mut manager).len(), 1);
}

#[test]
fn closing_the_origin_consumes_global_one_shots_without_running_their_callbacks() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        "Toyoterm.keymap(:tools) { key('b').run { $ran = true } }.activate_transient(pane: nil)",
    )
    .unwrap();
    let id = layers(&mut manager)[0].id;
    let mut context = script_test_context();
    context.handles = vec![WorkspaceId(1).into(), WindowId(2).into(), TabId(3).into()].into();
    let result = run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::TransientKeymap {
            id,
            key: Some("B".into()),
            pane: PaneId(4),
        },
    )
    .unwrap();
    assert!(result.commands.is_empty());
    assert_eq!(
        manager
            .eval("$ran.nil? && Toyoterm.transient_keymaps(pane: nil).empty?")
            .unwrap(),
        "true"
    );
}

#[test]
fn typed_command_prompts_inherit_the_transient_mode_owner() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        r#"
      Toyoterm.command(:label, arguments: [{name: :text}]) { |c, v| c.pane.badge = v['text'] }
      @tools = Toyoterm.keymap(:tools) { key('b').command(:label) }
      Toyoterm.keymap(:empty) {}
      Toyoterm.mode(:tools, keymap: :empty) { |on| @tools.activate_transient if on }
      Toyoterm.mode(:tools).enable
    "#,
    )
    .unwrap();
    let id = layers(&mut manager)[0].id;
    let result = invoke(&mut manager, id, Some("B")).unwrap();
    let [NativeCommand::Ui(UiCommand::OpenPrompt { id, .. })] = result.commands.as_slice() else {
        panic!("{:?}", result.commands)
    };
    let result = eval(&mut manager, "Toyoterm.mode(:tools).disable").unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::Ui(UiCommand::ClosePrompt { id: *id })]
    );
}

#[test]
fn bundled_transient_example_loads_and_dispatches_the_selected_split() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(include_str!("../../../examples/transient_keymaps.rb"))
        .unwrap();
    eval(&mut manager, "Toyoterm.call_command(:choose_split)").unwrap();
    let id = layers(&mut manager)[0].id;
    assert!(matches!(
        invoke(&mut manager, id, Some("J"))
            .unwrap()
            .commands
            .as_slice(),
        [NativeCommand::Action(ActionCommand::Invoke {
            action: NativeAction::Split(SplitDirection::Down),
            ..
        })]
    ));
    eval(&mut manager, "Toyoterm.call_command(:pane_tools)").unwrap();
    let id = layers(&mut manager)[0].id;
    assert!(!layers(&mut manager)[0].once);
    assert!(matches!(
        invoke(&mut manager, id, Some("H"))
            .unwrap()
            .commands
            .as_slice(),
        [NativeCommand::Action(ActionCommand::Invoke {
            action: NativeAction::ActivatePane(_),
            ..
        })]
    ));
}
