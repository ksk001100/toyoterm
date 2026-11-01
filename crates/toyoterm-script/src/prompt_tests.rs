use super::tests::script_test_context;
use super::*;
use toyoterm_api::InputPrompt;

fn open(manager: &mut ConfigManager, source: &str) -> (u64, InputPrompt) {
    let result = run_script_request(
        manager,
        &script_test_context(),
        &ScriptInvocation::Eval(source.into()),
    )
    .unwrap();
    let [NativeCommand::Ui(UiCommand::OpenPrompt { id, prompt })] = result.commands.as_slice()
    else {
        panic!("expected one input prompt: {:?}", result.commands);
    };
    (*id, prompt.clone())
}

fn respond(
    manager: &mut ConfigManager,
    id: u64,
    value: Option<&str>,
) -> Result<ScriptResult, ScriptError> {
    run_script_request(
        manager,
        &script_test_context(),
        &ScriptInvocation::SelectCallback {
            id,
            selection: value.map(str::to_owned),
        },
    )
}

#[test]
fn prompts_decode_options_and_return_free_text_in_the_captured_context() {
    let mut manager = ConfigManager::new().unwrap();
    let (id, prompt) = open(
        &mut manager,
        r#"
      Toyoterm.read_string(title: 'Name', initial: '猫', completions: ['cat', '犬', 'cat'], history: :names) do |text, context|
        context.pane.badge = text
      end
    "#,
    );
    assert_eq!(
        prompt,
        InputPrompt {
            title: "Name".into(),
            initial: "猫".into(),
            completions: vec!["cat".into(), "犬".into()],
            history: vec![],
            require_match: false,
            dynamic: false,
        }
    );
    let result = respond(&mut manager, id, Some("#{raise 'injection'}犬")).unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::Ui(UiCommand::SetPaneBadge {
            pane: PaneId(4),
            badge: Some("#{raise 'injection'}犬".into())
        })]
    );
    assert_eq!(
        manager
            .eval("Toyoterm.prompt_history(:names).first")
            .unwrap(),
        "#{raise 'injection'}犬"
    );
    // Results are one-shot even if an old native completion is repeated.
    assert!(
        respond(&mut manager, id, Some("again"))
            .unwrap()
            .commands
            .is_empty()
    );
}

#[test]
fn prompt_cancellation_and_empty_acceptance_are_distinct_and_release_the_slot() {
    let mut manager = ConfigManager::new().unwrap();
    let (id, _) = open(
        &mut manager,
        "Toyoterm.read_string(history: :empty) { |s| $answer = s }",
    );
    respond(&mut manager, id, None).unwrap();
    assert_eq!(manager.eval("$answer.nil?").unwrap(), "true");
    assert_eq!(
        manager.eval("Toyoterm.prompt_history(:empty)").unwrap(),
        "[]"
    );
    let (id, _) = open(
        &mut manager,
        "Toyoterm.read_string(history: :empty) { |s| $answer = s }",
    );
    respond(&mut manager, id, Some("")).unwrap();
    assert_eq!(manager.eval("$answer == ''").unwrap(), "true");
    assert_eq!(
        manager.eval("Toyoterm.prompt_history(:empty)").unwrap(),
        "[\"\"]"
    );
}

#[test]
fn prompt_and_selection_share_a_pending_slot_and_validate_results() {
    let mut manager = ConfigManager::new().unwrap();
    let (id, _) = open(
        &mut manager,
        "Toyoterm.read_string(completions: ['one'], require_match: true) { |s| $answer = s }",
    );
    assert!(
        run_script_request(
            &mut manager,
            &script_test_context(),
            &ScriptInvocation::Eval("Toyoterm.select(items: ['two']) {}".into())
        )
        .is_err()
    );
    assert!(respond(&mut manager, id, Some("not one")).is_err());
    let (id, _) = open(
        &mut manager,
        "Toyoterm.read_string(completions: ['one'], require_match: true) { |s| $answer = s }",
    );
    respond(&mut manager, id, Some("one")).unwrap();
    assert_eq!(manager.eval("$answer").unwrap(), "one");
    let (id, _) = open(&mut manager, "Toyoterm.read_string {} ");
    assert!(respond(&mut manager, id, Some("bad\nvalue")).is_err());
}

#[test]
fn prompt_failures_roll_back_history_and_queued_followups_and_reload_clears_state() {
    let mut manager = ConfigManager::new().unwrap();
    let (id, _) = open(&mut manager, "Toyoterm.read_string(history: :names) {} ");
    respond(&mut manager, id, Some("keep")).unwrap();
    let (id, prompt) = open(
        &mut manager,
        r#"
      Toyoterm.read_string(history: :names) do |text, context|
        Toyoterm.read_string { }
        context.pane.badge = 'failed'
        raise 'callback failed'
      end
    "#,
    );
    assert_eq!(prompt.history, vec!["keep"]);
    assert!(respond(&mut manager, id, Some("rollback")).is_err());
    assert_eq!(
        manager.eval("Toyoterm.prompt_history(:names)").unwrap(),
        "[\"keep\"]"
    );
    let (id, _) = open(&mut manager, "Toyoterm.read_string {} ");
    respond(&mut manager, id, None).unwrap();
    assert!(
        run_script_request(
            &mut manager,
            &script_test_context(),
            &ScriptInvocation::Eval(
                "Toyoterm.read_string(history: :failed) {}; raise 'failed open'".into()
            )
        )
        .is_err()
    );
    assert_eq!(
        manager
            .eval("Toyoterm.clear_prompt_history(:failed)")
            .unwrap(),
        "false"
    );
    assert!(manager.reload("raise 'failed reload'").is_err());
    assert_eq!(
        manager.eval("Toyoterm.prompt_history(:names)").unwrap(),
        "[\"keep\"]"
    );
    manager.reload("").unwrap();
    assert_eq!(
        manager.eval("Toyoterm.prompt_history(:names)").unwrap(),
        "[]"
    );
}

#[test]
fn prompt_histories_are_bounded_deduplicated_immutable_and_can_be_cleared() {
    let mut manager = ConfigManager::new().unwrap();
    for value in 0..130 {
        let (id, _) = open(&mut manager, "Toyoterm.read_string(history: :names) {} ");
        respond(&mut manager, id, Some(&value.to_string())).unwrap();
    }
    assert_eq!(
        manager
            .eval("Toyoterm.prompt_history(:names).length")
            .unwrap(),
        "128"
    );
    assert_eq!(
        manager
            .eval("Toyoterm.prompt_history(:names).first")
            .unwrap(),
        "2"
    );
    let (id, _) = open(&mut manager, "Toyoterm.read_string(history: :names) {} ");
    respond(&mut manager, id, Some("2")).unwrap();
    assert_eq!(
        manager
            .eval("Toyoterm.prompt_history(:names).last")
            .unwrap(),
        "2"
    );
    assert_eq!(
        manager
            .eval("Toyoterm.prompt_history(:names).length")
            .unwrap(),
        "128"
    );
    assert!(
        manager
            .eval("Toyoterm.prompt_history(:names) << 'bad'")
            .is_err()
    );
    assert!(
        manager
            .eval("Toyoterm.prompt_history(:names).last << 'bad'")
            .is_err()
    );
    assert_eq!(
        manager
            .eval("Toyoterm.clear_prompt_history(:names)")
            .unwrap(),
        "true"
    );
    assert_eq!(
        manager
            .eval("Toyoterm.clear_prompt_history(:names)")
            .unwrap(),
        "false"
    );
}

#[test]
fn prompts_validate_input_before_reserving_the_slot_and_reject_load_time_use() {
    let mut manager = ConfigManager::new().unwrap();
    assert!(manager.reload("Toyoterm.read_string {} ").is_err());
    for source in [
        "Toyoterm.read_string",
        "Toyoterm.read_string(title: 1) {}",
        "Toyoterm.read_string(initial: 'x' * 4097) {}",
        "Toyoterm.read_string(title: 'x' * 257) {}",
        "Toyoterm.read_string(initial: \"bad\\tvalue\") {}",
        "Toyoterm.read_string(completions: 'bad') {}",
        "Toyoterm.read_string(completions: [1]) {}",
        "Toyoterm.read_string(completions: Array.new(4097, 'x')) {}",
        "Toyoterm.read_string(completions: [\"bad\\nvalue\"]) {}",
        "Toyoterm.read_string(require_match: 1) {}",
        "Toyoterm.read_string(require_match: true) {}",
        "Toyoterm.read_string(history: 1) {}",
    ] {
        assert!(
            run_script_request(
                &mut manager,
                &script_test_context(),
                &ScriptInvocation::Eval(source.into())
            )
            .is_err(),
            "{source}"
        );
    }
    let (id, _) = open(&mut manager, "Toyoterm.read_string {} ");
    respond(&mut manager, id, None).unwrap();
}

#[test]
fn named_history_count_is_bounded_and_clearing_releases_capacity() {
    let mut manager = ConfigManager::new().unwrap();
    for index in 0..64 {
        let (id, _) = open(
            &mut manager,
            &format!("Toyoterm.read_string(history: 'history{index}') {{}}"),
        );
        respond(&mut manager, id, None).unwrap();
    }
    assert!(
        run_script_request(
            &mut manager,
            &script_test_context(),
            &ScriptInvocation::Eval("Toyoterm.read_string(history: :overflow) {}".into())
        )
        .is_err()
    );
    assert_eq!(
        manager
            .eval("Toyoterm.clear_prompt_history(:history0)")
            .unwrap(),
        "true"
    );
    let (id, _) = open(&mut manager, "Toyoterm.read_string(history: :overflow) {} ");
    respond(&mut manager, id, None).unwrap();
}

#[test]
fn terminal_completion_example_navigates_directories_and_launches_a_pane() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("project")).unwrap();
    std::fs::create_dir(directory.path().join("project/nested")).unwrap();
    std::fs::write(
        directory.path().join("project/plain-file.txt"),
        "not a directory",
    )
    .unwrap();
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(include_str!("../../../examples/completion.rb"))
        .unwrap();
    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::UserCommand {
            name: "terminal_directory".into(),
            pane: PaneId(4),
        },
    )
    .unwrap();
    let NativeCommand::Ui(UiCommand::OpenPrompt { id, .. }) = result.commands[0] else {
        panic!("expected prompt")
    };
    let prefix = directory.path().to_str().unwrap().replace('\\', "/");
    let result = complete(&mut manager, id, 0, &format!("{prefix}/pro")).unwrap();
    assert!(
        matches!(&result.commands[0], NativeCommand::Ui(UiCommand::UpdatePromptCompletions { items, .. }) if items.len() == 1 && items[0].ends_with("project/"))
    );
    let result = complete(&mut manager, id, 1, &format!("{prefix}/project/")).unwrap();
    assert!(
        matches!(&result.commands[0], NativeCommand::Ui(UiCommand::UpdatePromptCompletions { items, .. }) if items.len() == 1 && items[0].ends_with("nested/"))
    );
    let path = format!("{prefix}/project/nested/");
    let result = respond(&mut manager, id, Some(&path)).unwrap();
    assert!(
        matches!(&result.commands[0], NativeCommand::Pane(PaneCommand::SplitWithLaunch { pane: PaneId(4), direction: SplitDirection::Right, launch }) if launch.cwd.as_deref() == Some(path.as_str()))
    );
}

#[test]
fn extension_runtime_excludes_editor_apis_and_preserves_terminal_extension_capabilities() {
    let mut manager = ConfigManager::new().unwrap();
    let result = run_script_request(&mut manager, &script_test_context(), &ScriptInvocation::Eval(r#"
      raise 'editor capability' if [:text_buffers, :buffer_files, :buffer_search, :buffer_modes].any? { |name| Toyoterm.supports?(name) }
      raise 'editor API' if Toyoterm.respond_to?(:create_buffer) || Toyoterm.respond_to?(:open_buffer)
      raise 'editor context' if Toyoterm::CallbackContext.new(Toyoterm.current_pane).respond_to?(:buffer)
      [:keymaps, :modes, :named_hooks, :timers, :input_prompt, :dynamic_completion].all? { |name| Toyoterm.supports?(name) }
    "#.into())).unwrap();
    assert_eq!(result.value.as_deref(), Some("true"));
}

fn complete(
    manager: &mut ConfigManager,
    id: u64,
    revision: u64,
    query: &str,
) -> Result<ScriptResult, ScriptError> {
    run_script_request(
        manager,
        &script_test_context(),
        &ScriptInvocation::CompletePrompt {
            id,
            revision,
            query: query.into(),
        },
    )
}

#[test]
fn dynamic_completers_receive_frozen_queries_and_emit_typed_candidate_updates() {
    let mut manager = ConfigManager::new().unwrap();
    let (id, prompt) = open(
        &mut manager,
        r#"
      Toyoterm.read_string(require_match: true, completer: proc { |query, c|
        raise 'query is mutable' unless query.frozen?
        [query + '猫', c.pane.id.to_s]
      }) { |text, c| c.pane.badge = text }
    "#,
    );
    assert!(prompt.dynamic);
    let result = complete(&mut manager, id, 7, "#{literal}").unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::Ui(UiCommand::UpdatePromptCompletions {
            id,
            revision: 7,
            items: vec!["#{literal}猫".into(), "4".into()]
        })]
    );
    let result = respond(&mut manager, id, Some("#{literal}猫")).unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::Ui(UiCommand::SetPaneBadge {
            pane: PaneId(4),
            badge: Some("#{literal}猫".into())
        })]
    );
    assert!(
        complete(&mut manager, id, 8, "late")
            .unwrap()
            .commands
            .is_empty()
    );
}

#[test]
fn provider_host_mutations_are_discarded_on_success_and_failure() {
    let mut manager = ConfigManager::new().unwrap();
    let original = manager.config().window.opacity;
    let (id, _) = open(
        &mut manager,
        r#"
      @fail_provider = false
      Toyoterm.read_string(completer: proc { |query, c|
        Toyoterm.configure { |config| config.window.opacity = 0.4 }
        c.pane.badge = 'discard'
        Toyoterm.command(:discarded_command) {}
        @discarded_task = Toyoterm.async('never-started') {}
        Toyoterm.after(1) {}
        raise 'provider failed' if @fail_provider
        ['one']
      }) {}
    "#,
    );
    let result = complete(&mut manager, id, 0, "").unwrap();
    assert_eq!(result.commands.len(), 1);
    assert!(result.async_requests.is_empty());
    assert!(result.snapshot.is_none());
    assert_eq!(manager.config().window.opacity, original);
    assert!(
        !manager
            .snapshot()
            .user_command_names
            .contains("discarded_command")
    );
    assert!(manager.snapshot().timers.is_empty());
    assert_eq!(manager.eval("@discarded_task.cancelled?").unwrap(), "true");
    manager.eval("@fail_provider = true").unwrap();
    assert!(complete(&mut manager, id, 1, "bad").is_err());
    manager.eval("@fail_provider = false").unwrap();
    assert_eq!(
        complete(&mut manager, id, 2, "retry")
            .unwrap()
            .commands
            .len(),
        1
    );
    respond(&mut manager, id, None).unwrap();
}

#[test]
fn dynamic_completion_validates_providers_and_results_without_losing_the_prompt() {
    let mut manager = ConfigManager::new().unwrap();
    for source in [
        "Toyoterm.read_string(completer: 1) {}",
        "Toyoterm.read_string(completer: proc { [] }, completions: ['one']) {}",
    ] {
        assert!(
            run_script_request(
                &mut manager,
                &script_test_context(),
                &ScriptInvocation::Eval(source.into())
            )
            .is_err()
        );
    }
    for value in [
        "'not an Array'",
        "[1]",
        "[\"bad\\nvalue\"]",
        "['x' * 4097]",
        "Array.new(4097, 'x')",
    ] {
        let (id, _) = open(
            &mut manager,
            &format!("Toyoterm.read_string(completer: proc {{ {value} }}) {{}}"),
        );
        assert!(complete(&mut manager, id, 0, "").is_err(), "{value}");
        respond(&mut manager, id, None).unwrap();
    }
    let (id, _) = open(
        &mut manager,
        "Toyoterm.read_string(require_match: true, completer: proc { [] }) {} ",
    );
    assert_eq!(
        complete(&mut manager, id, 0, "").unwrap().commands,
        vec![NativeCommand::Ui(UiCommand::UpdatePromptCompletions {
            id,
            revision: 0,
            items: vec![]
        })]
    );
    respond(&mut manager, id, None).unwrap();
}

#[test]
fn mode_owned_prompts_close_transactionally_and_consumed_results_stay_consumed() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      Toyoterm.keymap(:empty) {}
      Toyoterm.mode(:input, keymap: :empty) do |enabled|
        if enabled
          Toyoterm.read_string(completer: proc { ['one'] }) do
            Toyoterm.after(1) {}
            Toyoterm.mode(:input).disable
          end
        else
          raise 'failed teardown' if @fail_teardown
        end
      end
    "#,
        )
        .unwrap();
    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::Eval("Toyoterm.mode(:input).enable; @fail_teardown = true".into()),
    )
    .unwrap();
    let NativeCommand::Ui(UiCommand::OpenPrompt { id, .. }) = result.commands[0] else {
        panic!("expected prompt")
    };
    assert!(
        run_script_request(
            &mut manager,
            &script_test_context(),
            &ScriptInvocation::Eval("Toyoterm.mode(:input).disable".into())
        )
        .is_err()
    );
    assert_eq!(complete(&mut manager, id, 0, "").unwrap().commands.len(), 1);
    manager.eval("@fail_teardown = false").unwrap();
    respond(&mut manager, id, Some("one")).unwrap();
    assert!(manager.snapshot().timers.is_empty());
    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::Eval(
            "Toyoterm.mode(:input).enable; Toyoterm.mode(:input).disable".into(),
        ),
    )
    .unwrap();
    assert!(matches!(
        result.commands.last(),
        Some(NativeCommand::Ui(UiCommand::ClosePrompt { .. }))
    ));
    assert!(
        complete(&mut manager, id + 1, 0, "late")
            .unwrap()
            .commands
            .is_empty()
    );
}

#[test]
fn failed_major_mode_replacement_restores_an_owned_selection() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      Toyoterm.keymap(:empty) {}
      Toyoterm.mode(:first, keymap: :empty, kind: :major) do |enabled|
        Toyoterm.select(items: ['one']) { |s, c| c.pane.badge = s } if enabled
      end
      Toyoterm.mode(:broken, keymap: :empty, kind: :major) { raise 'failed setup' }
    "#,
        )
        .unwrap();
    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::Eval("Toyoterm.mode(:first).enable".into()),
    )
    .unwrap();
    let NativeCommand::Ui(UiCommand::OpenSelector { id, .. }) = result.commands[0] else {
        panic!("expected selector")
    };
    assert!(
        run_script_request(
            &mut manager,
            &script_test_context(),
            &ScriptInvocation::Eval("Toyoterm.mode(:broken).enable".into())
        )
        .is_err()
    );
    let result = respond(&mut manager, id, Some("one")).unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::Ui(UiCommand::SetPaneBadge {
            pane: PaneId(4),
            badge: Some("one".into())
        })]
    );
    assert!(
        respond(&mut manager, id, Some("one"))
            .unwrap()
            .commands
            .is_empty()
    );
}
