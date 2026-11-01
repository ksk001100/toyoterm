use super::*;
use toyoterm_script::ScriptResult;

/// Exercises the real script owner and native overlay/dispatch controllers,
/// without a display server or fake Ruby execution.
struct ExtensionFlow {
    worker: ScriptThread,
    completions: mpsc::Receiver<ScriptCompletion>,
    snapshot: ScriptSnapshot,
    next_request: u64,
    mux: Mux,
    terminals: TerminalRuntime,
    ui: UiState,
}

impl ExtensionFlow {
    fn new() -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "toyoterm-extension-flow-{}-{unique}.rb",
            std::process::id()
        ));
        std::fs::write(
            &path,
            include_str!("../../../../../examples/command_arguments.rb"),
        )
        .unwrap();
        let (sender, completions) = mpsc::channel();
        let started = ScriptThread::start(Some(path.clone()), move |completion| {
            let _ = sender.send(completion);
        });
        std::fs::remove_file(path).unwrap();
        let (worker, startup) = started.unwrap();
        assert!(startup.config_error.is_none());
        let mux = Mux::new();
        let terminals = test_terminal_runtime(mux.pane_ids());
        Self {
            worker,
            completions,
            snapshot: startup.snapshot,
            next_request: 0,
            mux,
            terminals,
            ui: test_ui_state(),
        }
    }

    fn request(&mut self, invocation: ScriptInvocation) -> ScriptResult {
        self.send(invocation);
        self.receive()
    }

    fn send(&mut self, invocation: ScriptInvocation) {
        self.next_request += 1;
        self.worker
            .submit(ScriptRequest {
                id: self.next_request,
                invocation,
                context: ScriptContext {
                    model: Arc::new(ruby_object_model(&self.mux, None).unwrap()),
                    handles: self.mux.native_handles().into(),
                    clipboard: None,
                },
            })
            .unwrap();
    }

    fn receive(&mut self) -> ScriptResult {
        let completion = self
            .completions
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert_eq!(completion.id, self.next_request);
        if let ScriptInvocation::CompletePrompt { id, revision, .. } = completion.invocation
            && let Some(selector) = self.ui.selector.as_mut()
        {
            selector.finish_completion(id, revision, completion.result.is_ok());
        }
        let result = completion.result.unwrap();
        if let Some(snapshot) = &result.snapshot {
            self.snapshot = snapshot.clone();
            self.ui.key_sequences.replace_registry(snapshot);
        }
        for command in &result.commands {
            self.apply(command.clone());
        }
        result
    }

    fn apply(&mut self, command: NativeCommand) {
        match command {
            NativeCommand::Ui(command) => {
                let effects = apply_ui_command(
                    command,
                    self.mux.current_pane(),
                    &mut self.terminals,
                    &mut self.ui,
                );
                assert!(effects.error.is_none());
            }
            NativeCommand::Mux(command) => {
                self.mux.dispatch(command).unwrap();
            }
            NativeCommand::Script(ScriptCommand::InvokeUserCommand { name, pane }) => {
                self.request(ScriptInvocation::UserCommand { name, pane });
            }
            NativeCommand::Action(command) => {
                let resolution = resolve_action_command(
                    command,
                    CommandOrigin::Script,
                    &self.mux,
                    &self.ui.pane_layout,
                )
                .unwrap();
                for command in resolution.activation {
                    self.mux.dispatch(command).unwrap();
                }
                if let Some(command) = resolution.command {
                    self.apply(command);
                }
            }
            other => panic!("unexpected extension command: {other:?}"),
        }
    }

    fn binding(&mut self, key: &str) {
        let pane = self.mux.current_pane().unwrap();
        let invocation =
            match resolve_keybinding_for_pane(&self.snapshot, pane, [key.to_owned()], false) {
                KeybindingDispatch::Ruby(key) => ScriptInvocation::KeyBinding { key, pane },
                KeybindingDispatch::Native(action) => {
                    let context = ActionContext {
                        workspace: self.mux.current_workspace(),
                        window: self.mux.current_window().unwrap(),
                        tab: self.mux.current_tab().unwrap(),
                        pane,
                    };
                    let resolution = resolve_action_command(
                        ActionCommand::Invoke { action, context },
                        CommandOrigin::Keybinding,
                        &self.mux,
                        &self.ui.pane_layout,
                    )
                    .unwrap();
                    assert!(resolution.activation.is_empty());
                    self.apply(resolution.command.unwrap());
                    return;
                }
                other => panic!("expected Ruby binding: {other:?}"),
            };
        self.request(invocation);
    }

    fn text(&mut self, text: &str) {
        assert!(
            handle_selector_input(
                &Key::Character(text.into()),
                Some(text),
                ModifiersState::empty(),
                &mut self.ui
            )
            .is_none()
        );
    }

    fn finish(&mut self, key: NamedKey) -> ScriptResult {
        let completion = handle_selector_input(
            &Key::Named(key),
            None,
            ModifiersState::empty(),
            &mut self.ui,
        )
        .expect("overlay completion");
        self.request(ScriptInvocation::SelectCallback {
            id: completion.id,
            selection: completion.selection,
        })
    }
}

#[test]
fn operation_palette_collects_unicode_arguments_and_applies_to_origin_after_focus_changes() {
    let mut flow = ExtensionFlow::new();
    let origin = flow.mux.current_pane().unwrap();
    let origin_tab = flow.mux.current_tab().unwrap();
    flow.binding("ALT+O");
    assert!(!flow.ui.selector.as_ref().unwrap().is_prompt());
    flow.text("label_pane");
    flow.finish(NamedKey::Enter);
    assert_eq!(flow.ui.selector.as_ref().unwrap().title, "Pane label");

    flow.mux.dispatch(Command::NewTab).unwrap();
    let other = flow.mux.current_pane().unwrap();
    let other_tab = flow.mux.current_tab().unwrap();
    assert_ne!(origin, other);
    flow.text("開発👩‍💻");
    flow.ui.ime_preedit = Some("変換".into());
    assert!(
        handle_selector_input(
            &Key::Named(NamedKey::Enter),
            None,
            ModifiersState::empty(),
            &mut flow.ui
        )
        .is_none()
    );
    assert_eq!(flow.ui.selector.as_ref().unwrap().query, "開発👩‍💻");
    flow.ui.ime_preedit = None;
    flow.finish(NamedKey::Enter);
    assert_eq!(
        flow.ui.selector.as_ref().unwrap().title,
        "Toggle Pane zoom?"
    );
    assert!(flow.ui.pane_badges.is_empty());
    assert!(!flow.mux.is_zoomed(origin_tab));
    // Replace the default false value through native prompt input.
    handle_selector_input(
        &Key::Character("u".into()),
        None,
        ModifiersState::CONTROL,
        &mut flow.ui,
    );
    flow.text("true");
    flow.finish(NamedKey::Enter);
    assert!(flow.ui.selector.is_none());
    assert_eq!(
        flow.ui.pane_badges.get(&origin).map(String::as_str),
        Some("開発👩‍💻")
    );
    assert!(!flow.ui.pane_badges.contains_key(&other));
    assert_eq!(flow.mux.zoomed_pane(origin_tab), Some(origin));
    assert!(!flow.mux.is_zoomed(other_tab));
    assert_eq!(flow.mux.current_pane(), Some(origin));
}

#[test]
fn cancelling_argument_collection_from_native_input_runs_no_partial_command_and_allows_retry() {
    let mut flow = ExtensionFlow::new();
    let origin_tab = flow.mux.current_tab().unwrap();
    flow.binding("ALT+L");
    flow.text("discard");
    flow.finish(NamedKey::Enter);
    let cancelled = flow.finish(NamedKey::Escape);
    assert!(cancelled.commands.is_empty());
    assert!(flow.ui.pane_badges.is_empty());
    assert!(!flow.mux.is_zoomed(origin_tab));
    assert!(flow.ui.selector.is_none());
    flow.binding("ALT+L");
    flow.text("retry");
    flow.finish(NamedKey::Enter);
    flow.finish(NamedKey::Enter);
    let pane = flow.mux.current_pane().unwrap();
    assert_eq!(
        flow.ui.pane_badges.get(&pane).map(String::as_str),
        Some("retry")
    );
    assert!(!flow.mux.is_zoomed(origin_tab));
}

#[test]
fn delayed_dynamic_completion_rejects_old_query_and_keeps_provider_effects_out_of_native_state() {
    let mut flow = ExtensionFlow::new();
    let origin = flow.mux.current_pane().unwrap();
    flow.request(ScriptInvocation::Eval(
        r#"
      Toyoterm.read_string(require_match: true, history: :flow, completer: proc { |query, context|
        context.pane.badge = 'discard provider effect'
        [query + 'build']
      }) { |text, context| context.pane.badge = text }
    "#
        .into(),
    ));
    flow.text("old");
    let selector = flow.ui.selector.as_mut().unwrap();
    let id = selector.id;
    let (old_revision, query) = selector
        .take_completion(Instant::now() + Duration::from_secs(1))
        .unwrap();
    flow.send(ScriptInvocation::CompletePrompt {
        id,
        revision: old_revision,
        query,
    });
    // Input can change before the main thread receives the script completion.
    handle_selector_input(
        &Key::Character("u".into()),
        None,
        ModifiersState::CONTROL,
        &mut flow.ui,
    );
    flow.text("猫");
    assert!(
        handle_selector_input(
            &Key::Named(NamedKey::Enter),
            None,
            ModifiersState::empty(),
            &mut flow.ui
        )
        .is_none()
    );
    let old = flow.receive();
    assert!(
        matches!(old.commands.as_slice(), [NativeCommand::Ui(UiCommand::UpdatePromptCompletions { revision, .. })] if *revision == old_revision)
    );
    let selector = flow.ui.selector.as_mut().unwrap();
    assert_eq!(selector.query, "猫");
    assert!(selector.items.is_empty());
    let (revision, query) = selector
        .take_completion(Instant::now() + Duration::from_secs(1))
        .unwrap();
    assert_ne!(revision, old_revision);
    flow.request(ScriptInvocation::CompletePrompt {
        id,
        revision,
        query,
    });
    assert_eq!(flow.ui.selector.as_ref().unwrap().items, vec!["猫build"]);
    assert!(flow.ui.pane_badges.is_empty());
    flow.finish(NamedKey::Enter);
    assert_eq!(
        flow.ui.pane_badges.get(&origin).map(String::as_str),
        Some("猫build")
    );
    let history = flow.request(ScriptInvocation::Eval(
        "Toyoterm.prompt_history(:flow) == ['猫build']".into(),
    ));
    assert_eq!(history.value.as_deref(), Some("true"));
}
