use super::tests::script_test_context;
use super::*;

fn eval(manager: &mut ConfigManager, source: &str) -> Result<ScriptResult, ScriptError> {
    run_script_request(
        manager,
        &script_test_context(),
        &ScriptInvocation::Eval(source.into()),
    )
}

fn overlay(result: &ScriptResult) -> (u64, Vec<String>) {
    let [NativeCommand::Ui(UiCommand::OpenSelector { id, title, items })] =
        result.commands.as_slice()
    else {
        panic!("{:?}", result.commands)
    };
    assert_eq!(title, "Command help");
    (*id, items.clone())
}

fn choose(
    manager: &mut ConfigManager,
    id: u64,
    row: Option<&str>,
) -> Result<ScriptResult, ScriptError> {
    run_script_request(
        manager,
        &script_test_context(),
        &ScriptInvocation::SelectCallback {
            id,
            selection: row.map(str::to_owned),
        },
    )
}

#[test]
fn command_help_inspects_schema_effective_keys_and_advice_without_running_the_command() {
    let mut manager = ConfigManager::new().unwrap();
    eval(
        &mut manager,
        r#"
      Toyoterm.command(:label, description: "Label\nthis Pane", arguments: [
        {name: :text, choices: ['Build', 'Test'], history: :labels},
        {name: :count, type: :integer, min: 1, max: 9, default: 2}
      ]) { raise 'help must not run me' }
      Toyoterm.configure { |c| c.keys.ctrl('b').command(:label) }
      Toyoterm.keymap(:local) { key('l').command(:label) }.activate(pane: Toyoterm.current_pane)
      Toyoterm.advise_command(:label) { raise 'help must not run advice' }
    "#,
    )
    .unwrap();
    let result = eval(&mut manager, "Toyoterm.command_help(:label)").unwrap();
    let (id, rows) = overlay(&result);
    for expected in [
        "Command: \"label\"",
        "Description: \"Label\\nthis Pane\"",
        "Keys: 2",
        "  Key: \"CTRL+B\"",
        "  Key: \"L\"",
        "Arguments: 2",
        "    Default: 2",
        "    Minimum: 1",
        "    Maximum: 9",
        "      Choice: \"Build\"",
        "Advice: 1",
    ] {
        assert!(
            rows.iter().any(|row| row == expected),
            "missing {expected}: {rows:?}"
        );
    }
    assert!(
        choose(&mut manager, id, Some(&rows[0]))
            .unwrap()
            .commands
            .is_empty()
    );
}

#[test]
fn help_browse_and_definition_selection_return_immutable_full_provenance() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.rb");
    let library = directory.path().join("library.rb");
    std::fs::write(
        &path,
        "require_relative 'library'; Toyoterm.command(:root) {}\n",
    )
    .unwrap();
    std::fs::write(&library, "Toyoterm.command(:library) {}\n").unwrap();
    let mut manager = ConfigManager::load_startup(Some(&path)).unwrap();
    assert_eq!(
        manager
            .eval("Toyoterm.command_definition(:root)[:source]")
            .unwrap(),
        path.to_string_lossy()
    );
    assert!(
        manager
            .eval("Toyoterm.command_definition(:library)[:source]")
            .unwrap()
            .ends_with("library.rb")
    );
    assert_eq!(
        manager
            .eval("Toyoterm.instance_variable_get(:@current_source_label).nil?")
            .unwrap(),
        "true"
    );
    let result = eval(
        &mut manager,
        r#"
      Toyoterm.command_help do |definition, context|
        raise 'mutable definition' unless definition.frozen? && definition[:source].frozen?
        Toyoterm.clipboard.write(definition[:source])
        context.pane.badge = definition[:name]
      end
    "#,
    )
    .unwrap();
    let (id, choices) = overlay(&result);
    let result = choose(
        &mut manager,
        id,
        choices
            .iter()
            .find(|row| row.contains("root"))
            .map(String::as_str),
    )
    .unwrap();
    let (id, rows) = overlay(&result);
    let result = choose(
        &mut manager,
        id,
        rows.iter()
            .find(|row| row.starts_with("Definition:"))
            .map(String::as_str),
    )
    .unwrap();
    assert_eq!(
        result.commands,
        vec![
            NativeCommand::Clipboard(ClipboardCommand::Write(path.to_string_lossy().into_owned())),
            NativeCommand::Ui(UiCommand::SetPaneBadge {
                pane: PaneId(4),
                badge: Some("root".into())
            }),
        ]
    );
    assert_eq!(
        manager
            .eval("Toyoterm.command_definition(:missing).nil?")
            .unwrap(),
        "true"
    );
}

#[test]
fn help_cancellation_and_replacement_checks_cover_both_stages() {
    let mut manager = ConfigManager::new().unwrap();
    manager.reload("Toyoterm.command(:work) {}").unwrap();
    let result = eval(&mut manager, "Toyoterm.command_help").unwrap();
    let (id, rows) = overlay(&result);
    eval(&mut manager, "Toyoterm.command(:work, replace: true) {}").unwrap();
    assert!(
        choose(&mut manager, id, Some(&rows[0]))
            .unwrap_err()
            .to_string()
            .contains("command changed")
    );
    let result = eval(
        &mut manager,
        "Toyoterm.command_help(:work) { raise 'must not run' }",
    )
    .unwrap();
    let (id, _) = overlay(&result);
    assert!(choose(&mut manager, id, None).unwrap().commands.is_empty());
    assert!(eval(&mut manager, "Toyoterm.command_help(:missing)").is_err());
    assert!(eval(&mut manager, "Toyoterm.command_help(1)").is_err());
    assert!(
        manager
            .reload("Toyoterm.command(:work) {}; Toyoterm.command_help(:work)")
            .is_err()
    );
}

#[test]
fn definition_callback_failure_rolls_back_effects_and_does_not_resurrect_help() {
    let mut manager = ConfigManager::new().unwrap();
    manager.reload("Toyoterm.command(:work) {}").unwrap();
    let result = eval(&mut manager, "Toyoterm.command_help(:work) { |d, c| c.pane.badge = 'rollback'; Toyoterm.command(:leaked) {}; raise 'failed handler' }").unwrap();
    let (id, rows) = overlay(&result);
    let row = rows
        .iter()
        .find(|row| row.starts_with("Definition:"))
        .unwrap();
    assert!(choose(&mut manager, id, Some(row)).is_err());
    assert_eq!(
        manager.eval("Toyoterm.command_info(:leaked).nil?").unwrap(),
        "true"
    );
    assert!(
        choose(&mut manager, id, Some(row))
            .unwrap()
            .commands
            .is_empty()
    );
    let result = eval(
        &mut manager,
        "Toyoterm.command_help(:work) { raise 'stale' }",
    )
    .unwrap();
    let (id, rows) = overlay(&result);
    eval(&mut manager, "Toyoterm.command(:work, replace: true) {}").unwrap();
    assert!(
        choose(
            &mut manager,
            id,
            rows.iter()
                .find(|row| row.starts_with("Definition:"))
                .map(String::as_str)
        )
        .is_err()
    );
}

#[test]
fn help_bounds_large_metadata_without_truncating_programmatic_values() {
    let mut manager = ConfigManager::new().unwrap();
    eval(&mut manager, r#"
      Toyoterm.command(:large, description: '猫' * 500, arguments: [{name: :text, choices: Array.new(4096) { |i| i.to_s }}]) {}
      33.times { Toyoterm.advise_command(:large) {} }
      Toyoterm.command_help(:large)
    "#).map(|result| {
        let (_, rows) = overlay(&result);
        assert!(rows.iter().all(|row| row.len() <= 4096 && !row.contains(['\n', '\r', '\0'])));
        assert!(rows.iter().any(|row| row.contains("More choices")));
        assert!(rows.iter().any(|row| row.contains("More advice")));
        assert!(rows[1].ends_with('…'));
    }).unwrap();
    assert_eq!(
        manager
            .eval("Toyoterm.command_info(:large)[:arguments][0][:choices].length")
            .unwrap(),
        "4096"
    );
}

#[test]
fn mode_owned_help_closes_on_disable_and_the_example_loads() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(include_str!("../../../examples/command_help.rb"))
        .unwrap();
    let result = eval(
        &mut manager,
        r#"
      Toyoterm.keymap(:empty) {}
      Toyoterm.mode(:help, keymap: :empty) { |enabled| Toyoterm.command_help if enabled }
      Toyoterm.mode(:help).enable
    "#,
    )
    .unwrap();
    let (id, _) = overlay(&result);
    let result = eval(&mut manager, "Toyoterm.mode(:help).disable").unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::Ui(UiCommand::ClosePrompt { id })]
    );
    assert!(choose(&mut manager, id, None).unwrap().commands.is_empty());
}
