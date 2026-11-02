use super::tests::script_test_context;
use super::*;

#[test]
fn pane_events_preserve_grid_payload_and_subject_through_the_native_boundary() {
    let mut manager = ConfigManager::new().unwrap();
    manager.reload(r#"
      [:pane_ready, :pane_resized].each do |name|
        Toyoterm.on(name) do |event|
          $event = [event.name, event.pane.id, event.columns, event.rows, event.subject.id, event.width, event.window]
          event.pane.badge = event.name.to_s
        end
      end
    "#).unwrap();
    for kind in [ScriptEventKind::PaneReady, ScriptEventKind::PaneResized] {
        let mut event = RubyEvent::new(kind);
        event.pane = Some(PaneId(4));
        event.columns = Some(90);
        event.rows = Some(30);
        let result = run_script_request(
            &mut manager,
            &script_test_context(),
            &ScriptInvocation::Event(event),
        )
        .unwrap();
        assert_eq!(
            result.commands,
            vec![NativeCommand::Ui(UiCommand::SetPaneBadge {
                pane: PaneId(4),
                badge: Some(kind.as_str().into())
            })]
        );
        assert_eq!(
            manager.eval("$event").unwrap(),
            format!("[:{}, 4, 90, 30, 4, nil, nil]", kind.as_str())
        );
    }
}

#[test]
fn pane_event_example_shows_grid_dimensions_and_ignores_stale_panes() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(include_str!("../../../examples/pane_events.rb"))
        .unwrap();
    let mut event = RubyEvent::new(ScriptEventKind::PaneReady);
    event.pane = Some(PaneId(4));
    event.columns = Some(80);
    event.rows = Some(24);
    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::Event(event.clone()),
    )
    .unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::Ui(UiCommand::SetPaneBadge {
            pane: PaneId(4),
            badge: Some("80 cols x 24 rows".into())
        })]
    );
    let mut context = script_test_context();
    context.handles = vec![WorkspaceId(1).into(), WindowId(2).into(), TabId(3).into()].into();
    assert!(
        run_script_request(&mut manager, &context, &ScriptInvocation::Event(event))
            .unwrap()
            .commands
            .is_empty()
    );
}

#[test]
fn pane_event_context_derives_the_reported_pane_hierarchy_after_tab_focus_changes() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload("Toyoterm.on(:pane_ready) { |event| event.context.actions.toggle_zoom }")
        .unwrap();
    let mut context = script_test_context();
    let model = Arc::make_mut(&mut context.model);
    let mut other_pane = model.panes[0].clone();
    other_pane.id = PaneId(5);
    model.panes.push(other_pane);
    let mut other_tab = model.tabs[0].clone();
    other_tab.id = TabId(9);
    other_tab.panes = vec![PaneId(5)];
    model.tabs.push(other_tab);
    model.windows[0].tabs.push(TabId(9));
    model.current_tab = TabId(9);
    model.current_pane = PaneId(5);
    let mut handles = context.handles.to_vec();
    handles.push(PaneId(5).into());
    handles.push(TabId(9).into());
    context.handles = handles.into();
    let mut event = RubyEvent::new(ScriptEventKind::PaneReady);
    event.pane = Some(PaneId(4));
    event.columns = Some(80);
    event.rows = Some(24);
    let result =
        run_script_request(&mut manager, &context, &ScriptInvocation::Event(event)).unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::Action(ActionCommand::Invoke {
            action: NativeAction::ToggleZoom,
            context: ActionContext {
                workspace: WorkspaceId(1),
                window: WindowId(2),
                tab: TabId(3),
                pane: PaneId(4)
            }
        })]
    );
}
