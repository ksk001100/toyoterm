use super::tests::script_test_context;
use super::*;

#[test]
fn keymap_inheritance_and_activation_are_mirrored_with_callback_owners() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      Toyoterm.configure { |c| c.keys.ctrl('t').close_tab }
      @base = Toyoterm.keymap(:base) do
        ctrl('t').new_tab
        sequence('CTRL+X', 'F').run { |context| context.pane.send_text('parent') }
        sequence('CTRL+X', 'R').split(:right)
      end
      @child = Toyoterm.keymap(:child, parent: :base) do
        sequence('CTRL+X', 'R').split(:down)
      end
      @activation = @child.activate
    "#,
        )
        .unwrap();
    let snapshot = manager.snapshot();
    let bindings = snapshot.keymap_bindings_for(PaneId(8)).unwrap();
    assert_eq!(
        bindings.native_actions.get("CTRL+T"),
        Some(&NativeAction::NewTab)
    );
    assert_eq!(
        bindings.native_actions.get("CTRL+X R"),
        Some(&NativeAction::Split(SplitDirection::Down))
    );
    let owner = bindings.callback_owners["CTRL+X F"];
    assert_eq!(
        manager
            .eval("Toyoterm.describe_key('CTRL+X F')[:keymap]")
            .unwrap(),
        "base"
    );
    manager.runtime.eval(&format!("Toyoterm.__set_current_pane(8); Toyoterm.__invoke_keymap_binding({owner}, 'CTRL+X F', Toyoterm.current_pane)")).unwrap();
    assert_eq!(
        manager.drain_commands(PaneId(8)).unwrap(),
        vec![NativeCommand::Mux(Command::SendText {
            pane: PaneId(8),
            text: "parent".into()
        })]
    );
    manager.eval("@activation.remove").unwrap();
    manager.refresh_config_snapshot().unwrap();
    assert!(manager.snapshot().keymap_bindings.is_empty());
    assert_eq!(
        manager.native_action("CTRL+T"),
        Some(NativeAction::CloseTab)
    );
}

#[test]
fn keymap_layers_mask_prefix_conflicts_and_keep_unrelated_suffixes() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      @base = Toyoterm.keymap(:base) do
        sequence('CTRL+X', 'F').new_tab
        sequence('CTRL+X', 'R').close_tab
      end
      @single = Toyoterm.keymap(:single) { ctrl('x').toggle_zoom }
      @deep = Toyoterm.keymap(:deep) { sequence('CTRL+X', 'R', 'B').split(:right) }
      @base.activate
      @single_activation = @single.activate
    "#,
        )
        .unwrap();
    assert_eq!(
        manager
            .eval("Toyoterm.key_bindings.keys.join(',')")
            .unwrap(),
        "CTRL+X"
    );
    manager
        .eval("@single_activation.remove; @deep.activate")
        .unwrap();
    assert_eq!(
        manager
            .eval("Toyoterm.key_bindings.keys.join(',')")
            .unwrap(),
        "CTRL+X F,CTRL+X R B"
    );
}

#[test]
fn keymap_changes_and_failed_definitions_roll_back_even_if_rescued() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      @map = Toyoterm.keymap(:existing) { ctrl('t').new_tab }
      Toyoterm.command(:broken) do
        @map.keys.unbind('CTRL+T')
        @map.keys.ctrl('t').close_tab
        @map.activate
        raise 'failed'
      end
    "#,
        )
        .unwrap();
    assert!(manager.trigger_user_command("broken", PaneId(8)).is_err());
    assert_eq!(
        manager.eval("@map.bindings['CTRL+T'][:action]").unwrap(),
        "new_tab"
    );
    assert_eq!(
        manager.eval("Toyoterm.key_bindings.empty?").unwrap(),
        "true"
    );
    manager
        .eval(
            r#"
      begin
        Toyoterm.keymap(:broken) do
          Toyoterm.command(:leaked) {}
          raise 'definition failed'
        end
      rescue RuntimeError
      end
    "#,
        )
        .unwrap();
    assert_eq!(
        manager.eval("Toyoterm.keymaps.join(',')").unwrap(),
        "existing"
    );
    assert_eq!(
        manager.eval("Toyoterm.command_info(:leaked).nil?").unwrap(),
        "true"
    );
}

#[test]
fn pane_keymaps_override_global_layers_only_in_their_scope() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      @global = Toyoterm.keymap(:global) { ctrl('t').new_tab }
      @local = Toyoterm.keymap(:local) { ctrl('t').close_tab }
      @global.activate
    "#,
        )
        .unwrap();
    manager.set_current_pane(PaneId(8)).unwrap();
    manager
        .eval("@local.activate(pane: Toyoterm.current_pane)")
        .unwrap();
    manager.refresh_config_snapshot().unwrap();
    let snapshot = manager.snapshot();
    assert_eq!(
        snapshot
            .keymap_bindings_for(PaneId(8))
            .unwrap()
            .native_actions["CTRL+T"],
        NativeAction::CloseTab
    );
    assert_eq!(
        snapshot
            .keymap_bindings_for(PaneId(9))
            .unwrap()
            .native_actions["CTRL+T"],
        NativeAction::NewTab
    );
}

#[test]
fn modes_own_registrations_and_major_mode_replacement_is_atomic() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      @map = Toyoterm.keymap(:navigation) { key('h').activate_pane(:left) }
      @first = Toyoterm.mode(:first, keymap: @map, kind: :major) do |enabled, context|
        if enabled
          @hook = Toyoterm.add_hook(:owned) {}
          Toyoterm.command(:owned) {}
        end
      end
      @failed = Toyoterm.mode(:failed, keymap: @map, kind: :major) do |enabled, context|
        raise 'enable failed' if enabled
      end
      @second = Toyoterm.mode(:second, keymap: @map, kind: :major)
      @minor = Toyoterm.mode(:minor, keymap: @map)
    "#,
        )
        .unwrap();
    manager.set_current_pane(PaneId(8)).unwrap();
    assert_eq!(manager.eval("@first.enable").unwrap(), "true");
    assert_eq!(manager.eval("@first.enable").unwrap(), "false");
    assert!(manager.eval("@failed.enable").is_err());
    assert_eq!(manager.eval("@first.enabled?").unwrap(), "true");
    assert_eq!(manager.eval("@hook.active?").unwrap(), "true");
    assert_eq!(manager.eval("@failed.enabled?").unwrap(), "false");
    manager.eval("@minor.enable; @second.enable").unwrap();
    assert_eq!(
        manager.eval("Toyoterm.active_modes.join(',')").unwrap(),
        "minor,second"
    );
    assert_eq!(manager.eval("@hook.active?").unwrap(), "false");
    assert_eq!(
        manager.eval("Toyoterm.command_info(:owned).nil?").unwrap(),
        "true"
    );
    assert_eq!(
        manager.eval("@minor.disable; @minor.disable").unwrap(),
        "false"
    );
}

#[test]
fn failed_mode_disable_preserves_the_mode_and_its_resources() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      @map = Toyoterm.keymap(:navigation) { key('h').activate_pane(:left) }
      @mode = Toyoterm.mode(:navigation, keymap: @map) do |enabled, context|
        if enabled
          @hook = Toyoterm.add_hook(:owned) {}
        else
          @hook.remove
          raise 'disable failed'
        end
      end
    "#,
        )
        .unwrap();
    manager.set_current_pane(PaneId(8)).unwrap();
    manager.eval("@mode.enable").unwrap();
    assert!(manager.eval("@mode.disable").is_err());
    assert_eq!(manager.eval("@mode.enabled?").unwrap(), "true");
    assert_eq!(manager.eval("@hook.active?").unwrap(), "true");
    manager.refresh_config_snapshot().unwrap();
    assert!(
        manager
            .snapshot()
            .keymap_bindings_for(PaneId(8))
            .unwrap()
            .native_actions
            .contains_key("H")
    );
}

#[test]
fn scoped_keymap_callbacks_cross_the_typed_request_boundary() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      @map = Toyoterm.keymap(:navigation) do
        sequence('CTRL+X', 'F').run { |context| context.pane.send_text('scoped callback') }
      end
      Toyoterm.mode(:navigation, keymap: @map)
    "#,
        )
        .unwrap();
    let context = script_test_context();
    let result = run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::Eval("Toyoterm.current_pane.enable_mode(:navigation)".into()),
    )
    .unwrap();
    let snapshot = result.snapshot.unwrap();
    let bindings = snapshot.keymap_bindings_for(PaneId(4)).unwrap();
    assert!(bindings.keybindings.contains("CTRL+X F"));
    let owner = bindings.callback_owners["CTRL+X F"];
    let result = run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::KeymapBinding {
            map: owner,
            key: "CTRL+X F".into(),
            pane: PaneId(4),
        },
    )
    .unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::Mux(Command::SendText {
            pane: PaneId(4),
            text: "scoped callback".into()
        })]
    );
    let result = run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::Eval("Toyoterm.current_pane.disable_mode(:navigation)".into()),
    )
    .unwrap();
    assert!(result.snapshot.unwrap().keymap_bindings.is_empty());
}

#[test]
fn closing_a_pane_prunes_its_modes_and_owned_command_and_event_registries() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      @map = Toyoterm.keymap(:navigation) { key('h').activate_pane(:left) }
      Toyoterm.mode(:navigation, keymap: @map) do |enabled, context|
        if enabled
          @event = Toyoterm.on(:bell) {}
          Toyoterm.command(:owned) {}
        end
      end
    "#,
        )
        .unwrap();
    let mut context = script_test_context();
    run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::Eval("Toyoterm.mode(:navigation).enable".into()),
    )
    .unwrap();
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
    let result = run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::Eval("nil".into()),
    )
    .unwrap();
    let snapshot = result.snapshot.unwrap();
    assert!(snapshot.keymap_bindings.is_empty());
    assert!(!snapshot.event_names.contains("bell"));
    assert!(!snapshot.user_command_names.contains("owned"));
    assert_eq!(manager.eval("@event.active?").unwrap(), "false");
}

#[test]
fn keymaps_validate_targets_and_failed_reload_preserves_active_modes() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      @map = Toyoterm.keymap(:navigation) { key('h').activate_pane(:left) }
      @mode = Toyoterm.mode(:navigation, keymap: @map, kind: :major)
    "#,
        )
        .unwrap();
    assert!(manager.eval("@map.activate(pane: 42)").is_err());
    assert!(
        manager
            .eval("@map.activate(pane: Toyoterm::Pane.new(999))")
            .is_err()
    );
    assert!(manager.eval("@mode.enable(pane: nil)").is_err());
    assert!(manager.eval("@mode.enable").is_err());
    manager.set_current_pane(PaneId(8)).unwrap();
    manager.eval("@mode.enable").unwrap();
    assert!(manager.reload("raise 'failed reload'").is_err());
    assert_eq!(manager.eval("@mode.enabled?").unwrap(), "true");
    manager.reload("").unwrap();
    assert_eq!(
        manager
            .eval("Toyoterm.keymaps.empty? && Toyoterm.modes.empty?")
            .unwrap(),
        "true"
    );
}

#[test]
fn mode_and_explicit_map_precedence_is_independent_of_enable_order() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      @global = Toyoterm.keymap(:global) { key('t').new_tab }
      @major = Toyoterm.keymap(:major) { key('t').close_tab }
      @minor = Toyoterm.keymap(:minor) { key('t').toggle_zoom }
      @explicit = Toyoterm.keymap(:explicit) { key('t').split(:right) }
      Toyoterm.mode(:major, keymap: @major, kind: :major)
      Toyoterm.mode(:minor, keymap: @minor)
      @global.activate
    "#,
        )
        .unwrap();
    manager.set_current_pane(PaneId(8)).unwrap();
    manager
        .eval("Toyoterm.mode(:minor).enable; Toyoterm.mode(:major).enable")
        .unwrap();
    assert_eq!(
        manager.eval("Toyoterm.describe_key('T')[:keymap]").unwrap(),
        "minor"
    );
    manager.eval("@activation = @explicit.activate(pane: Toyoterm.current_pane); Toyoterm.mode(:minor).disable; Toyoterm.mode(:minor).enable").unwrap();
    assert_eq!(
        manager.eval("Toyoterm.describe_key('T')[:keymap]").unwrap(),
        "explicit"
    );
    manager
        .eval("@activation.remove; Toyoterm.mode(:minor).disable")
        .unwrap();
    assert_eq!(
        manager.eval("Toyoterm.describe_key('T')[:keymap]").unwrap(),
        "major"
    );
    manager.eval("Toyoterm.mode(:major).disable").unwrap();
    assert_eq!(
        manager.eval("Toyoterm.describe_key('T')[:keymap]").unwrap(),
        "global"
    );
}

#[test]
fn bundled_modes_example_toggles_scope_and_exits_through_its_own_keymap() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(include_str!("../../../examples/modes.rb"))
        .unwrap();
    manager.trigger_keybinding("ALT+N", PaneId(8)).unwrap();
    manager.refresh_config_snapshot().unwrap();
    let snapshot = manager.snapshot();
    let bindings = snapshot.keymap_bindings_for(PaneId(8)).unwrap();
    assert!(bindings.native_actions.contains_key("H"));
    let owner = bindings.callback_owners["ESCAPE"];
    assert_eq!(
        manager
            .eval("Toyoterm.active_modes(pane: nil).join(',')")
            .unwrap(),
        "navigation"
    );
    manager
        .runtime
        .eval(&format!(
            "Toyoterm.__invoke_keymap_binding({owner}, 'ESCAPE', Toyoterm.current_pane)"
        ))
        .unwrap();
    manager.refresh_config_snapshot().unwrap();
    assert!(manager.snapshot().keymap_bindings.is_empty());
    assert_eq!(
        manager
            .eval("Toyoterm.active_modes(pane: nil).empty?")
            .unwrap(),
        "true"
    );
}
