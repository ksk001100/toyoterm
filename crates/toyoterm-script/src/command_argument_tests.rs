use super::tests::script_test_context;
use super::*;

fn eval(manager: &mut ConfigManager, source: &str) -> Result<ScriptResult, ScriptError> {
    run_script_request(
        manager,
        &script_test_context(),
        &ScriptInvocation::Eval(source.into()),
    )
}

fn prompt(result: &ScriptResult) -> (u64, toyoterm_api::InputPrompt) {
    let [NativeCommand::Ui(UiCommand::OpenPrompt { id, prompt })] = result.commands.as_slice()
    else {
        panic!("expected one prompt: {:?}", result.commands);
    };
    (*id, prompt.clone())
}

fn respond(
    manager: &mut ConfigManager,
    id: u64,
    text: Option<&str>,
) -> Result<ScriptResult, ScriptError> {
    run_script_request(
        manager,
        &script_test_context(),
        &ScriptInvocation::SelectCallback {
            id,
            selection: text.map(str::to_owned),
        },
    )
}

#[test]
fn typed_commands_validate_values_and_freeze_copied_schema_metadata() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        r#"
      $choices = ['short', 'long']
      Toyoterm.command(:label, arguments: [
        {name: :style, choices: $choices},
        {name: :count, type: :integer, min: 1, max: 8, default: 2},
        {name: :scale, type: :number, default: 1.5},
        {name: :zoom, type: :boolean, default: false}
      ]) { |context, values| values }
      $choices[0].replace('changed')
    "#,
    )
    .unwrap();
    assert_eq!(
        manager
            .eval("Toyoterm.call_command(:label, arguments: {style: 'short'})")
            .unwrap(),
        "{\"style\" => \"short\", \"count\" => 2, \"scale\" => 1.5, \"zoom\" => false}"
    );
    for source in [
        "Toyoterm.call_command(:label)",
        "Toyoterm.call_command(:label, arguments: {style: 'changed'})",
        "Toyoterm.call_command(:label, arguments: {style: 'short', count: 1.0})",
        "Toyoterm.call_command(:label, arguments: {style: 'short', count: 9})",
        "Toyoterm.call_command(:label, arguments: {style: 'short', zoom: 0})",
        "Toyoterm.call_command(:label, arguments: {style: 'short', extra: true})",
        "Toyoterm.call_command(:label, arguments: {style: 'short', 'style' => 'long'})",
        "Toyoterm.command_info(:label)[:arguments][0][:choices][0].replace('bad')",
        "Toyoterm.command_info(:label)[:arguments] << {}",
        "Toyoterm.command(:label, replace: true, arguments: [{name: :n, min: 1}]) {}",
    ] {
        assert!(manager.eval(source).is_err(), "{source}");
    }
    assert_eq!(
        manager
            .eval("Toyoterm.command_info(:label)[:arguments][0][:choices]")
            .unwrap(),
        "[\"short\", \"long\"]"
    );
}

#[test]
fn command_arguments_follow_nested_calls_and_advice_without_changing_legacy_arity() {
    let mut manager = ConfigManager::new().unwrap();
    eval(&mut manager, r#"
      Toyoterm.command(:legacy, &lambda { |context| 7 })
      Toyoterm.command(:inner, arguments: [{name: :n, type: :integer, default: 3}]) do |c, v|
        raise 'not frozen' unless v.frozen? && Toyoterm.command_arguments.equal?(v)
        raise 'inner failure'
      end
      $seen = []
      Toyoterm.command(:outer, arguments: [{name: :n, type: :integer}]) do |c, v|
        begin; Toyoterm.call_command(:inner); rescue RuntimeError; end
        $seen << Toyoterm.command_arguments['n']
        Toyoterm.call_command(:legacy)
      end
      Toyoterm.advise_command(:outer) { $seen << Toyoterm.command_arguments['n'] }
      Toyoterm.advise_command(:outer, kind: :around) { |proceed| $seen << Toyoterm.command_arguments['n']; proceed.call }
      Toyoterm.advise_command(:outer, kind: :after) { $seen << Toyoterm.command_arguments['n'] }
    "#).unwrap();
    assert_eq!(
        manager
            .eval("Toyoterm.call_command(:outer, arguments: {n: 9})")
            .unwrap(),
        "7"
    );
    assert_eq!(manager.eval("$seen").unwrap(), "[9, 9, 9, 9]");
    assert_eq!(
        manager
            .eval("Toyoterm.command_arguments.empty? && Toyoterm.command_arguments.frozen?")
            .unwrap(),
        "true"
    );
}

#[test]
fn keyboard_commands_collect_sequential_values_retry_invalid_fields_and_cancel() {
    let mut manager = ConfigManager::new().unwrap();
    eval(&mut manager, r#"
      Toyoterm.command(:label, arguments: [
        {name: :text}, {name: :count, type: :integer, min: 1, max: 4, history: :counts},
        {name: :zoom, type: :boolean, default: false}
      ]) { |context, v| context.pane.badge = v['text'] + ':' + v['count'].to_s + ':' + v['zoom'].to_s }
    "#).unwrap();
    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::UserCommand {
            name: "label".into(),
            pane: PaneId(4),
        },
    )
    .unwrap();
    let (id, _) = prompt(&result);
    let result = respond(&mut manager, id, Some("猫")).unwrap();
    let (id, _) = prompt(&result);
    let result = respond(&mut manager, id, Some("9")).unwrap();
    let (id, retry) = prompt(&result);
    assert_eq!(retry.initial, "9");
    assert!(retry.title.starts_with("Invalid"));
    let result = respond(&mut manager, id, Some("2")).unwrap();
    let (id, boolean) = prompt(&result);
    assert_eq!(boolean.initial, "false");
    assert_eq!(boolean.completions, vec!["", "true", "false"]);
    let result = respond(&mut manager, id, Some("false")).unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::Ui(UiCommand::SetPaneBadge {
            pane: PaneId(4),
            badge: Some("猫:2:false".into())
        })]
    );
    let result = eval(
        &mut manager,
        "Toyoterm.prompt_command(:label, arguments: {text: 'skip', count: 1})",
    )
    .unwrap();
    let (id, _) = prompt(&result);
    assert!(respond(&mut manager, id, None).unwrap().commands.is_empty());
}

#[test]
fn command_collection_rejects_replacements_and_rolls_back_failed_bodies() {
    let mut manager = ConfigManager::new().unwrap();
    let result = eval(
        &mut manager,
        r#"
      Toyoterm.command(:label, arguments: [{name: :text}]) { |c, v| c.pane.badge = v['text'] }
      Toyoterm.prompt_command(:label)
    "#,
    )
    .unwrap();
    let (id, _) = prompt(&result);
    eval(
        &mut manager,
        "Toyoterm.command(:label, replace: true) { raise 'wrong command' }",
    )
    .unwrap();
    assert!(
        respond(&mut manager, id, Some("old"))
            .unwrap_err()
            .to_string()
            .contains("command changed")
    );
    let result = eval(
        &mut manager,
        r#"
      Toyoterm.command(:broken, arguments: [{name: :text, history: :failed}]) do |c, v|
        c.pane.badge = v['text']; Toyoterm.command(:leaked) {}; raise 'failed body'
      end
      Toyoterm.prompt_command(:broken)
    "#,
    )
    .unwrap();
    let (id, _) = prompt(&result);
    assert!(respond(&mut manager, id, Some("rollback")).is_err());
    assert_eq!(manager.eval("Toyoterm.command_info(:leaked).nil? && Toyoterm.prompt_history(:failed).empty? && Toyoterm.command_arguments.empty?").unwrap(), "true");
    // The consumed input is not resurrected by command rollback.
    assert!(
        respond(&mut manager, id, Some("again"))
            .unwrap()
            .commands
            .is_empty()
    );
}

#[test]
fn typed_command_prompts_are_owned_by_the_defining_mode() {
    let mut manager = ConfigManager::new().unwrap();
    let result = eval(
        &mut manager,
        r#"
      Toyoterm.keymap(:empty) {}
      Toyoterm.mode(:labels, keymap: :empty) do |enabled|
        if enabled
          Toyoterm.command(:label, arguments: [{name: :text}]) { |c, v| c.pane.badge = v['text'] }
        end
      end
      Toyoterm.mode(:labels).enable
      Toyoterm.prompt_command(:label)
    "#,
    )
    .unwrap();
    let (id, _) = prompt(&result);
    let result = eval(&mut manager, "Toyoterm.mode(:labels).disable").unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::Ui(UiCommand::ClosePrompt { id })]
    );
    assert!(
        respond(&mut manager, id, Some("late"))
            .unwrap()
            .commands
            .is_empty()
    );
}

#[test]
fn typed_commands_are_collected_through_both_palettes() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        "Toyoterm.command(:label, arguments: [{name: :text}]) { |c, v| c.pane.badge = v['text'] }",
    )
    .unwrap();
    for (source, selection) in [
        ("Toyoterm.command_palette", "label"),
        ("Toyoterm.operation_palette", "command: label"),
    ] {
        let result = eval(&mut manager, source).unwrap();
        let [NativeCommand::Ui(UiCommand::OpenSelector { id, .. })] = result.commands.as_slice()
        else {
            panic!("{:?}", result.commands)
        };
        let result = respond(&mut manager, *id, Some(selection)).unwrap();
        let (id, _) = prompt(&result);
        assert_eq!(
            respond(&mut manager, id, Some("chosen")).unwrap().commands,
            vec![NativeCommand::Ui(UiCommand::SetPaneBadge {
                pane: PaneId(4),
                badge: Some("chosen".into())
            })]
        );
    }
    assert_eq!(
        manager
            .eval("Toyoterm.operations.find { |i| i[:kind] == :command }[:argument_required]")
            .unwrap(),
        "true"
    );
    assert_eq!(
        eval(
            &mut manager,
            "Toyoterm.invoke_operation(:command, :label, arguments: {text: 'direct'})"
        )
        .unwrap()
        .commands
        .len(),
        1
    );
}

#[test]
fn argument_completion_preserves_context_and_hides_the_callable() {
    let mut manager = ConfigManager::new().unwrap();
    let result = eval(&mut manager, r#"
      Toyoterm.command(:label, arguments: [{name: :text, completer: proc { |q, c| [q + c.pane.id.to_s] }}]) { |c, v| c.pane.badge = v['text'] }
      Toyoterm.prompt_command(:label)
    "#).unwrap();
    let (id, options) = prompt(&result);
    assert!(options.dynamic);
    assert_eq!(manager.eval("f = Toyoterm.command_info(:label)[:arguments][0]; f[:dynamic] && !f.key?(:completer)").unwrap(), "true");
    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::CompletePrompt {
            id,
            revision: 1,
            query: "pane".into(),
        },
    )
    .unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::Ui(UiCommand::UpdatePromptCompletions {
            id,
            revision: 1,
            items: vec!["pane4".into()]
        })]
    );
    let result = respond(&mut manager, id, Some("pane4")).unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::Ui(UiCommand::SetPaneBadge {
            pane: PaneId(4),
            badge: Some("pane4".into())
        })]
    );
}

#[test]
fn command_schema_errors_do_not_replace_existing_registrations() {
    let mut manager = ConfigManager::new().unwrap();
    manager.reload("Toyoterm.command(:keep) { 42 }").unwrap();
    for schema in [
        "nil",
        "[{}]",
        "[{name: :n, type: :file}]",
        "[{name: :n}, {name: 'n'}]",
        "[{name: :n, extra: true}]",
        "[{name: :n, required: 1}]",
        "[{name: :n, choices: []}]",
        "[{name: :n, type: :integer, min: 4, max: 1}]",
        "[{name: :n, type: :integer, min: 1, default: 0}]",
        "[{name: :n, type: :boolean, completer: proc { [] }}]",
        "[{name: :n, completer: 1}]",
        "[{name: :n, require_match: true}]",
        "[{name: :n, default: \"bad\\nvalue\"}]",
        "Array.new(17) { |i| {name: i.to_s} }",
        "[{name: :n, required: false, choices: Array.new(4096) { |i| i.to_s }}]",
    ] {
        assert!(
            manager
                .eval(&format!(
                    "Toyoterm.command(:keep, replace: true, arguments: {schema}) {{ 0 }}"
                ))
                .is_err(),
            "{schema}"
        );
        assert_eq!(manager.eval("Toyoterm.call_command(:keep)").unwrap(), "42");
    }
}

#[test]
fn bundled_typed_command_example_loads_and_invokes_without_input() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(include_str!("../../../examples/command_arguments.rb"))
        .unwrap();
    let result = eval(
        &mut manager,
        "Toyoterm.call_command(:label_pane, arguments: {label: 'Build'})",
    )
    .unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::Ui(UiCommand::SetPaneBadge {
            pane: PaneId(4),
            badge: Some("Build".into())
        })]
    );
}
