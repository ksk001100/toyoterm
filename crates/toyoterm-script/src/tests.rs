use super::*;

fn script_test_context() -> ScriptContext {
    ScriptContext {
        model: ScriptObjectModel {
            current_workspace: WorkspaceId(1),
            current_window: WindowId(2),
            current_tab: TabId(3),
            current_pane: PaneId(4),
            workspaces: vec![ScriptWorkspace {
                id: WorkspaceId(1),
                name: "Workspace 1".into(),
                windows: vec![WindowId(2)],
            }],
            windows: vec![ScriptWindow {
                id: WindowId(2),
                tabs: vec![TabId(3)],
            }],
            tabs: vec![ScriptTab {
                id: TabId(3),
                title: "Tab 3".into(),
                panes: vec![PaneId(4)],
                zoomed: false,
            }],
            panes: vec![ScriptPane {
                id: PaneId(4),
                title: "Pane 4".into(),
                icon_title: None,
                cwd: None,
                remote_host: None,
                shell_integration_version: None,
                shell_integration_shell: None,
                user_vars: Vec::new(),
                pid: None,
                command_running: false,
                last_exit_status: None,
                screen_text: "prompt>".into(),
                zoomed: false,
            }],
        }
        .into(),
        handles: vec![
            WorkspaceId(1).into(),
            WindowId(2).into(),
            TabId(3).into(),
            PaneId(4).into(),
        ]
        .into(),
        clipboard: None,
    }
}

/// Diagnostic baseline: real script-thread request/completion boundary, no time assertions.
fn request(manager: &mut ConfigManager, source: &str) -> Result<ScriptResult, ScriptError> {
    run_script_request(
        manager,
        &script_test_context(),
        &ScriptInvocation::Eval(source.into()),
    )
}

#[test]
fn defaults_match_native_defaults_and_scheme_is_persistent() {
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(manager.config(), &ToyotermConfig::default());
    assert_eq!(
        manager
            .eval("(define (twice x) (* x 2)) (twice 21)")
            .unwrap(),
        "42"
    );
    assert_eq!(manager.eval("(twice 5)").unwrap(), "10");
    assert_eq!(manager.eval_inspect("\"text\"").unwrap(), "\"text\"");
}

#[test]
fn reload_is_atomic_and_replaces_registry_generation() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload("(config-set! 'font.size 18) (command! 'old (lambda (ctx) void))")
        .unwrap();
    assert!(
        manager
            .reload("(config-set! 'font.size 20) (error \"failed\")")
            .is_err()
    );
    assert_eq!(manager.config().font.size, 18.0);
    assert_eq!(manager.user_command_names().collect::<Vec<_>>(), ["old"]);
    manager
        .reload("(command! 'new (lambda (ctx) void))")
        .unwrap();
    assert_eq!(manager.user_command_names().collect::<Vec<_>>(), ["new"]);
}

#[test]
fn settings_validate_types_ranges_and_nested_values() {
    let mut manager = ConfigManager::new().unwrap();
    for source in [
        "(config-set! 'font.size 0)",
        "(config-set! 'font.weight 1001)",
        "(config-set! 'font.weight 1.5)",
        "(config-set! 'font.fallback '(\"monospace\"))",
        "(config-set! 'font.fallback '(\"a\" \"a\"))",
        "(config-set! 'font.fallback '(1))",
        "(config-set! 'colors.background \"red\")",
        "(config-set! 'colors.ansi '(\"#000000\"))",
        "(config-set! 'window.decorations 1)",
        "(config-set! 'window.title \"\")",
        "(config-set! 'window.image.opacity 1.1)",
        "(config-set! 'window.image.path 1)",
        "(config-set! 'ui.padding_x -1)",
        "(config-set! 'scrollback_lines -1)",
        "(config-set! 'behavior.osc_download_directory \"relative\")",
        "(config-set! 'unknown 1)",
    ] {
        assert!(manager.reload(source).is_err(), "accepted {source}");
        assert_eq!(manager.config(), &ToyotermConfig::default());
    }
    manager.reload("(config-set! 'font.fallback '(\"A\" \"B\")) (config-set! 'window.opacity 5) (config-set! 'ui.padding-x 0)").unwrap();
    assert_eq!(manager.config().window.opacity, 1.0);
    assert_eq!(manager.config().font.fallback, ["A", "B"]);
}

#[test]
fn callback_failure_rolls_back_commands_config_clipboard_and_registrations() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"(bind! "CTRL+E" (lambda (ctx)
      (pane-send-text! (context-ref ctx 'pane) "discard")
      (clipboard-write! "discard")
      (config-set! 'font.size 22)
      (command! 'discard (lambda (ctx) void))
      (error "intentional callback failure")))"#,
        )
        .unwrap();
    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::KeyBinding {
            key: "CTRL+E".into(),
            pane: PaneId(4),
        },
    );
    assert!(
        result
            .unwrap_err()
            .message()
            .contains("intentional callback failure")
    );
    assert_eq!(manager.config().font.size, 14.0);
    assert!(manager.user_command_names().next().is_none());
    assert!(manager.drain_commands(PaneId(4)).unwrap().is_empty());
    assert_eq!(
        request(&mut manager, "(+ 2 3)").unwrap().value.as_deref(),
        Some("5")
    );
}

#[test]
fn live_config_changes_validate_and_return_snapshots() {
    let mut manager = ConfigManager::new().unwrap();
    let result = request(
        &mut manager,
        "(config-set! 'font.size 16) (bind-action! \"CTRL+T\" 'new-tab)",
    )
    .unwrap();
    assert_eq!(result.snapshot.unwrap().config.font.size, 16.0);
    assert_eq!(manager.native_action("ctrl+t"), Some(NativeAction::NewTab));
    assert!(
        request(
            &mut manager,
            "(config-set! 'font.fallback '(\"monospace\"))"
        )
        .is_err()
    );
    assert!(manager.config().font.fallback.is_empty());
}

#[test]
fn cached_snapshots_follow_all_public_mutations_and_survive_failed_validation() {
    let mut manager = ConfigManager::new().unwrap();
    // Each category affects the native snapshot independently of font settings.
    for source in [
        "(config-set! 'font.size 18)",
        "(theme! 'cached (hash 'colors.background \"#112233\"))",
        "(config-set! 'theme 'cached)",
        "(theme! 'cached (hash 'colors.background \"#445566\") #t)",
        "(ansi-color-set! 1 \"#123456\")",
        "(leader! \"CTRL+J\" 900)",
        "(define cached-binding (bind! \"CTRL+X\" (lambda (ctx) void)))",
        "(bind-action! \"CTRL+Y\" 'new-tab)",
        "(unbind! \"CTRL+Y\")",
        "(registration-remove! cached-binding)",
        "(define cached-event (on! 'bell (lambda (event) void)))",
        "(registration-remove! cached-event)",
        "(define cached-command (command! 'cached-command (lambda (ctx) void)))",
        "(registration-remove! cached-command)",
        "(define cached-bar (bar! 'top 'left \" | \" (list (widget \"cached\"))))",
        "(registration-remove! cached-bar)",
    ] {
        assert!(
            request(&mut manager, source).unwrap().snapshot.is_some(),
            "missing change: {source}"
        );
        assert!(request(&mut manager, "(+ 1 2)").unwrap().snapshot.is_none());
    }
    let before = manager.snapshot();
    for _ in 0..2 {
        // Scheme accepts this value; native validation must reject it each time
        // even though rollback reuses the same revision for the next attempt.
        assert!(request(&mut manager, "(config-set! 'font.fallback '(\"A\" \"A\"))").is_err());
        assert_eq!(manager.snapshot(), before);
        assert!(request(&mut manager, "void").unwrap().snapshot.is_none());
    }
    assert!(
        request(&mut manager, "(config-set! 'font.size 19)")
            .unwrap()
            .snapshot
            .is_some()
    );
    assert_eq!(manager.config().font.size, 19.0);
    manager.reload("(config-set! 'font.size 21)").unwrap();
    assert!(request(&mut manager, "void").unwrap().snapshot.is_none());
    assert!(
        request(&mut manager, "(config-set! 'font.size 22)")
            .unwrap()
            .snapshot
            .is_some()
    );
}

#[test]
fn static_and_targeted_dynamic_actions_use_the_same_decoder() {
    let mut manager = ConfigManager::new().unwrap();
    manager.reload("(bind-action! \"CTRL+T\" 'new-tab) (bind-action! \"CTRL+H\" 'split 'left) (leader! \"CTRL+SPACE\" 750)").unwrap();
    assert_eq!(manager.native_action("ctrl+t"), Some(NativeAction::NewTab));
    assert_eq!(
        manager.native_action("ctrl+h"),
        Some(NativeAction::Split(SplitDirection::Left))
    );
    assert_eq!(manager.config().leader.as_ref().unwrap().timeout_ms, 750);
    let result = request(&mut manager, "(context-action! (context) 'split 'right)").unwrap();
    assert_eq!(
        result.commands,
        [NativeCommand::Action(ActionCommand::Invoke {
            action: NativeAction::Split(SplitDirection::Right),
            context: ActionContext {
                workspace: WorkspaceId(1),
                window: WindowId(2),
                tab: TabId(3),
                pane: PaneId(4)
            }
        })]
    );
    for source in [
        "(bind-action! \"X\" 'unknown)",
        "(action! 'split)",
        "(action! 'split 'diagonal)",
        "(bind! \"CTRL+T\" (lambda (ctx) void))",
    ] {
        assert!(request(&mut manager, source).is_err(), "accepted {source}");
    }
}

#[test]
fn typed_object_model_and_stale_handles() {
    let mut manager = ConfigManager::new().unwrap();
    let result = request(&mut manager, "(list (handle-id (current-pane)) (handle-ref (current-pane) 'title) (handle-ref (current-pane) 'screen-text))").unwrap();
    assert_eq!(result.value.as_deref(), Some("(4 \"Pane 4\" \"prompt>\")"));
    assert!(request(&mut manager, "(pane-send-text! (handle 'pane 999) \"bad\")").is_err());
    let result = request(&mut manager, "(pane-send-text! (current-pane) \"こんにちは\\n\") (pane-badge-set! (current-pane) \"bell\")").unwrap();
    assert_eq!(result.commands.len(), 2);
    assert_eq!(
        result.commands[0],
        NativeCommand::Mux(Command::SendText {
            pane: PaneId(4),
            text: "こんにちは\n".into()
        })
    );
}

#[test]
fn custom_launch_and_search_cross_the_native_boundary() {
    let mut manager = ConfigManager::new().unwrap();
    let result = request(&mut manager, r#"(split! (current-pane) 'right (hash 'program "test" 'args '("a" "b") 'env (hash "A" "1" "B" #f)))
      (pane-search! (current-pane) "needle" 'previous)"#).unwrap();
    let NativeCommand::Pane(PaneCommand::SplitWithLaunch { launch, .. }) = &result.commands[0]
    else {
        panic!("{:?}", result.commands)
    };
    assert_eq!(launch.program.as_deref(), Some("test"));
    assert_eq!(launch.args, ["a", "b"]);
    assert!(launch.environment.contains(&("B".into(), None)));
    assert!(matches!(
        result.commands[1],
        NativeCommand::Pane(PaneCommand::Search {
            direction: PaneSearchDirection::Previous,
            ..
        })
    ));
}

#[test]
fn registration_removal_does_not_remove_a_replacement() {
    let mut manager = ConfigManager::new().unwrap();
    request(&mut manager, "(define old (command! 'test (lambda (ctx) void))) (command! 'test (lambda (ctx) void) #t) (registration-remove! old)").unwrap();
    assert_eq!(manager.user_command_names().collect::<Vec<_>>(), ["test"]);
    assert_eq!(
        request(&mut manager, "(registration-active? old)")
            .unwrap()
            .value
            .as_deref(),
        Some("#false")
    );
}

#[test]
fn events_are_typed_ordered_and_failures_are_isolated() {
    let mut manager = ConfigManager::new().unwrap();
    manager.reload(r#"(on! 'bell (lambda (event) (pane-send-text! (context-ref event 'pane) "discard") (error "broken handler")))
      (on! 'bell (lambda (event) (pane-send-text! (context-ref event 'pane) (context-ref event 'title))))"#).unwrap();
    let mut event = ScriptEvent::new(ScriptEventKind::Bell);
    event.pane = Some(PaneId(4));
    event.title = Some("after".into());
    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::Event(event),
    )
    .unwrap();
    assert_eq!(
        result.commands,
        [NativeCommand::Mux(Command::SendText {
            pane: PaneId(4),
            text: "after".into()
        })]
    );
    assert_eq!(result.logs.len(), 1);
    assert!(result.logs[0].message.contains("broken handler"));
}

#[test]
fn themes_apply_and_explicit_colors_win() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r##"(theme! 'moon (hash 'colors.background "#10131a" 'colors.cursor "#88c0d0"))
      (config-set! 'theme 'moon) (config-set! 'colors.cursor "#ffffff")"##,
        )
        .unwrap();
    assert_eq!(manager.config().colors.background, "#10131a");
    assert_eq!(manager.config().colors.cursor, "#ffffff");
    assert_eq!(manager.color_presets["moon"].cursor, "#88c0d0");
    assert!(manager.reload("(config-set! 'theme 'unknown)").is_err());
}

#[test]
fn selection_resumes_with_origin_context() {
    let mut manager = ConfigManager::new().unwrap();
    let result = request(&mut manager, r#"(select! "Choose" '("a" "b") (lambda (selected ctx) (when selected (pane-send-text! (context-ref ctx 'pane) selected))))"#).unwrap();
    let NativeCommand::Ui(UiCommand::OpenSelector { id, .. }) = result.commands[0] else {
        panic!("missing selector")
    };
    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::SelectCallback {
            id,
            selection: Some("b".into()),
        },
    )
    .unwrap();
    assert_eq!(
        result.commands,
        [NativeCommand::Mux(Command::SendText {
            pane: PaneId(4),
            text: "b".into()
        })]
    );
}

#[test]
fn async_completion_and_cancellation_preserve_origin() {
    let mut manager = ConfigManager::new().unwrap();
    let result = request(&mut manager, r#"(define task (async! "program" '("arg") (lambda (result ctx) (pane-send-text! (context-ref ctx 'pane) "done"))))"#).unwrap();
    assert_eq!(result.async_requests.len(), 1);
    let id = result.async_requests[0].id;
    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::AsyncCallback {
            id,
            output: AsyncProcessOutput {
                stdout: b"ok".to_vec(),
                stderr: Vec::new(),
                exit_status: 0,
                launch_error: false,
            },
        },
    )
    .unwrap();
    assert_eq!(result.commands.len(), 1);
    assert_eq!(manager.eval("(task-success? task)").unwrap(), "true");
    let result = request(
        &mut manager,
        "(define cancelled (async! \"program\" '() #f)) (task-cancel! cancelled)",
    )
    .unwrap();
    assert!(result.async_requests.is_empty());
}

#[test]
fn bars_render_unicode_and_discard_commands() {
    let mut manager = ConfigManager::new().unwrap();
    manager.reload(r#"(bar! 'top 'left " | " (list (widget "日本語" 1.0) (widget (lambda (ctx) (pane-send-text! (context-ref ctx 'pane) "discard") "status") 1.0)))"#).unwrap();
    manager
        .set_live_handles(script_test_context().handles.iter().copied())
        .unwrap();
    manager
        .set_object_model(&script_test_context().model)
        .unwrap();
    assert_eq!(
        manager.render_bar(StatusBarPosition::Top).unwrap(),
        [BarItem {
            alignment: BarAlignment::Left,
            text: "日本語 | status".into()
        }]
    );
    assert!(manager.drain_commands(PaneId(4)).unwrap().is_empty());
}

#[test]
fn script_thread_executes_scheme_requests() {
    let (tx, rx) = mpsc::channel();
    let (thread, _) = ScriptThread::start(None, move |completion| {
        tx.send(completion).unwrap();
    })
    .unwrap();
    thread
        .submit(ScriptRequest {
            id: 7,
            context: script_test_context(),
            invocation: ScriptInvocation::Eval("(+ 19 23)".into()),
        })
        .unwrap();
    let result = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(result.id, 7);
    assert_eq!(result.result.unwrap().value.as_deref(), Some("42"));
}

#[test]
fn slow_threshold_and_snapshot_decoder_validation() {
    assert!(!is_slow_callback(Duration::from_millis(99)));
    assert!(is_slow_callback(Duration::from_millis(100)));
    assert!(decode_color_presets("1;4:name16;", 1).is_err());
    assert_eq!(
        resolve_config_path(Some(Path::new("explicit")), None, Some("default".into())),
        Some("explicit".into())
    );
}
#[test]
fn native_scheme_modules_load_relative_to_config() {
    let dir = std::env::temp_dir().join(format!("toyoterm-steel-module-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("helper.scm"),
        "(provide setup) (define (setup) (config-set! 'font.size 19))",
    )
    .unwrap();
    let path = dir.join("config.scm");
    std::fs::write(&path, "(require \"helper.scm\") (setup)").unwrap();
    let manager = ConfigManager::load_startup(Some(&path)).unwrap();
    assert_eq!(manager.config().font.size, 19.0);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_file(dir.join("helper.scm")).unwrap();
    std::fs::remove_dir(dir).unwrap();
}

#[test]
fn sync_process_output_remains_byte_exact() {
    let mut manager = ConfigManager::new().unwrap();
    #[cfg(windows)]
    let source = "(process-exit-status (spawn \"cmd.exe\" '(\"/c\" \"exit 7\")))";
    #[cfg(unix)]
    let source = "(process-exit-status (spawn \"/bin/sh\" '(\"-c\" \"exit 7\")))";
    assert_eq!(manager.eval(source).unwrap(), "7");
}
#[test]
fn bundled_scheme_examples_are_executable() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    for name in [
        "minimal_config.scm",
        "default_config.scm",
        "theme_selector.scm",
        "async_bar.scm",
        "performance.scm",
    ] {
        ConfigManager::load_startup(Some(&dir.join(name)))
            .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}

#[test]
fn failing_completions_release_their_pending_registry_slots() {
    let mut manager = ConfigManager::new().unwrap();
    let selected = request(
        &mut manager,
        "(select! \"Pick\" '(\"a\") (lambda (value ctx) (error \"selection failed\")))",
    )
    .unwrap();
    let NativeCommand::Ui(UiCommand::OpenSelector { id, .. }) = selected.commands[0] else {
        panic!("missing selector")
    };
    assert!(
        run_script_request(
            &mut manager,
            &script_test_context(),
            &ScriptInvocation::SelectCallback {
                id,
                selection: Some("a".into())
            }
        )
        .is_err()
    );
    request(
        &mut manager,
        "(select! \"Next\" '(\"b\") (lambda (value ctx) void))",
    )
    .unwrap();
    let spawned = request(&mut manager, "(define failed-task (async! \"program\" '() (lambda (result ctx) (error \"async failed\"))))").unwrap();
    assert!(
        run_script_request(
            &mut manager,
            &script_test_context(),
            &ScriptInvocation::AsyncCallback {
                id: spawned.async_requests[0].id,
                output: AsyncProcessOutput {
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                    exit_status: 0,
                    launch_error: false
                }
            }
        )
        .is_err()
    );
    assert_eq!(manager.eval("(length (__ref 'tasks '()))").unwrap(), "0");
    assert_eq!(
        manager.eval("(task-complete? failed-task)").unwrap(),
        "true"
    );
}

#[test]
fn switching_themes_recomputes_unmodified_colors() {
    let mut manager = ConfigManager::new().unwrap();
    manager.reload(r##"(theme! 'a (hash 'colors.background "#111111")) (theme! 'b (hash 'colors.background "#222222")) (config-set! 'theme 'a)"##).unwrap();
    assert_eq!(manager.config().colors.background, "#111111");
    request(&mut manager, "(config-set! 'theme 'b)").unwrap();
    assert_eq!(manager.config().colors.background, "#222222");
    request(&mut manager, "(config-set! 'theme #f)").unwrap();
    assert_eq!(
        manager.config().colors.background,
        ToyotermConfig::default().colors.background
    );
}

#[test]
fn typed_transfers_preserve_nuls_unicode_and_do_not_compile_data() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .set_clipboard_text(Some("日本語\0\" (error \"injected\")"))
        .unwrap();
    assert_eq!(
        manager.eval("(clipboard-read)").unwrap(),
        "日本語\0\" (error \"injected\")"
    );
    let mut ctx = script_test_context();
    Arc::make_mut(&mut ctx.model).panes[0].title = "title\0with\"quotes".into();
    manager.set_object_model(&ctx.model).unwrap();
    manager
        .set_live_handles(ctx.handles.iter().copied())
        .unwrap();
    assert_eq!(
        manager.eval("(handle-ref (current-pane) 'title)").unwrap(),
        "title\0with\"quotes"
    );
}

#[test]
fn window_images_preserve_atomic_reload_and_live_cache() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut manager = ConfigManager::new().unwrap();
    let file = root
        .join("tests/fixtures/background.png")
        .to_string_lossy()
        .replace('\\', "/");
    manager
        .reload(&format!(
            "(config-set! 'window.image.path \"{file}\") (config-set! 'window.image.opacity 0.3)"
        ))
        .unwrap();
    let image = manager.config().window.background_image.clone().unwrap();
    request(&mut manager, "(config-set! 'window.image.opacity 0.6)").unwrap();
    assert!(Arc::ptr_eq(
        &image,
        manager.config().window.background_image.as_ref().unwrap()
    ));
    assert!(
        request(
            &mut manager,
            "(config-set! 'window.image.path \"missing-wallpaper.png\")"
        )
        .is_err()
    );
    assert!(Arc::ptr_eq(
        &image,
        manager.config().window.background_image.as_ref().unwrap()
    ));
    request(&mut manager, "(config-set! 'window.image.path #f)").unwrap();
    assert!(manager.config().window.background_image.is_none());
}

#[test]
#[ignore = "opt-in script performance baseline"]
fn performance_script_roundtrip() {
    let (tx, rx) = mpsc::channel();
    let config_path =
        std::env::temp_dir().join(format!("toyoterm-performance-{}.scm", std::process::id()));
    std::fs::write(&config_path, "; isolated performance configuration\n").unwrap();
    let startup = ScriptThread::start(Some(config_path.clone()), move |completion| {
        let _ = tx.send(completion);
    });
    std::fs::remove_file(config_path).unwrap();
    let (worker, _) = startup.unwrap();
    let context = script_test_context();
    let mut request_id = 0;
    let mut request = |invocation| {
        request_id += 1;
        worker
            .submit(ScriptRequest {
                id: request_id,
                context: context.clone(),
                invocation,
            })
            .unwrap();
        let completion = rx.recv_timeout(Duration::from_secs(30)).unwrap();
        assert_eq!(completion.id, request_id);
        completion.result.unwrap()
    };
    request(ScriptInvocation::Eval(
        r#"(bind! "CTRL+SHIFT+F12" (lambda (ctx) void)) (on! 'bell (lambda (event) void))"#.into(),
    ));
    for scenario in ["steel_key", "steel_event", "steel_async_completion"] {
        let mut times = Vec::new();
        for iteration in 0..1100 {
            let invocation = match scenario {
                "steel_key" => ScriptInvocation::KeyBinding {
                    key: "CTRL+SHIFT+F12".into(),
                    pane: PaneId(4),
                },
                "steel_event" => ScriptInvocation::Event(ScriptEvent {
                    kind: ScriptEventKind::Bell,
                    workspace: Some(WorkspaceId(1)),
                    window: Some(WindowId(2)),
                    tab: Some(TabId(3)),
                    pane: Some(PaneId(4)),
                    title: None,
                    cwd: None,
                    exit_status: None,
                    width: None,
                    height: None,
                    columns: None,
                    rows: None,
                }),
                _ => {
                    // Register a genuine async! callback; process launch is outside
                    // this benchmark. Feed the same typed completion used by the app.
                    let prepared = request(ScriptInvocation::Eval(
                        "(async! \"baseline-child\" '() (lambda (result ctx) void)) void".into(),
                    ));
                    assert_eq!(prepared.async_requests.len(), 1);
                    ScriptInvocation::AsyncCallback {
                        id: prepared.async_requests[0].id,
                        output: AsyncProcessOutput {
                            stdout: Vec::new(),
                            stderr: Vec::new(),
                            exit_status: 0,
                            launch_error: false,
                        },
                    }
                }
            };
            let started = Instant::now();
            std::hint::black_box(request(invocation));
            if iteration >= 100 {
                times.push(started.elapsed().as_secs_f64() * 1000.0);
            }
        }
        times.sort_by(f64::total_cmp);
        println!(
            "{{\"schema_version\":1,\"scenario\":\"{scenario}\",\"samples\":1000,\"warmup\":100,\"p50_ms\":{},\"p95_ms\":{},\"total_ms\":{}}}",
            times[500],
            times[950],
            times.iter().sum::<f64>()
        );
    }
}

#[test]
#[ignore = "opt-in script reload/async soak: scripts/soak-test.py"]
fn soak_config_reload_and_async_registry() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/soak-config.scm");
    let mut manager = ConfigManager::load_startup(Some(&path)).unwrap();
    let iterations = std::env::var("TOYOTERM_SOAK_ITERATIONS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1000);
    for iteration in 1..=iterations {
        manager.reload_file().unwrap();
        assert_eq!(manager.user_command_names().count(), 1);
        manager
            .trigger_user_command("soak_command", PaneId(0))
            .unwrap();
        if iteration % 100 == 0 {
            println!("soak reload iteration={iteration}");
        }
    }
    for iteration in 1..=iterations * 2 {
        manager
            .emit_event(ScriptEventKind::Bell, PaneId(0))
            .unwrap();
        manager
            .eval("(async! \"fixture\" '() (lambda (result ctx) void))")
            .unwrap();
        let spawned = manager.drain_async_requests().unwrap();
        assert_eq!(spawned.len(), 1);
        manager
            .invoke_async_callback(spawned[0].id, b"ready", b"", 0)
            .unwrap();
        manager.eval("(define cancelled (async! \"fixture\" '() (lambda (r ctx) (error \"cancelled callback ran\"))))").unwrap();
        let cancelled = manager.drain_async_requests().unwrap();
        manager.eval("(task-cancel! cancelled)").unwrap();
        assert_eq!(manager.drain_async_cancellations().unwrap().len(), 1);
        manager
            .invoke_async_callback(cancelled[0].id, b"", b"", 0)
            .unwrap();
        assert_eq!(manager.eval("(length (__ref 'tasks '()))").unwrap(), "0");
        if iteration % 100 == 0 {
            println!("soak async iteration={iteration} registry=0");
        }
    }
}

#[test]
fn async_widgets_update_release_and_cancel_with_their_sections() {
    let mut manager = ConfigManager::new().unwrap();
    manager.reload(r#"(define section (bar! 'bottom 'left " " (list (async-widget "fixture" '() 1.0 "initial" (lambda (result) (process-stdout-text result))))))"#).unwrap();
    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::Bar {
            position: StatusBarPosition::Bottom,
        },
    )
    .unwrap();
    assert_eq!(result.bar.unwrap()[0].text, "initial");
    assert_eq!(result.async_requests.len(), 1);
    let id = result.async_requests[0].id;
    run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::AsyncCallback {
            id,
            output: AsyncProcessOutput {
                stdout: b"ready".to_vec(),
                stderr: Vec::new(),
                exit_status: 0,
                launch_error: false,
            },
        },
    )
    .unwrap();
    assert_eq!(
        manager.render_bar(StatusBarPosition::Bottom).unwrap()[0].text,
        "ready"
    );
    assert_eq!(manager.eval("(length (__ref 'tasks '()))").unwrap(), "0");
    request(&mut manager, "(registration-remove! section)").unwrap();
    assert_eq!(
        manager
            .eval("(length (hash-keys->list (__ref 'async-widget-state (hash))))")
            .unwrap(),
        "0"
    );
}

#[test]
fn per_entry_palette_overrides_survive_theme_switches() {
    let mut manager = ConfigManager::new().unwrap();
    manager.reload(r##"(theme! 'a (hash 'colors.ansi (map (lambda (i) "#111111") (range 0 16)))) (theme! 'b (hash 'colors.ansi (map (lambda (i) "#222222") (range 0 16)))) (config-set! 'theme 'a) (ansi-color-set! 1 "#ffffff")"##).unwrap();
    assert_eq!(manager.config().colors.ansi[0], "#111111");
    assert_eq!(manager.config().colors.ansi[1], "#ffffff");
    request(&mut manager, "(config-set! 'theme 'b)").unwrap();
    assert_eq!(manager.config().colors.ansi[0], "#222222");
    assert_eq!(manager.config().colors.ansi[1], "#ffffff");
}

#[test]
fn failed_evaluation_restores_cancellation_of_an_existing_task() {
    let mut manager = ConfigManager::new().unwrap();
    request(
        &mut manager,
        "(define retained-task (async! \"fixture\" '() #f))",
    )
    .unwrap();
    assert!(
        request(
            &mut manager,
            "(task-cancel! retained-task) (error \"rollback\")"
        )
        .is_err()
    );
    assert_eq!(
        manager.eval("(task-pending? retained-task)").unwrap(),
        "true"
    );
    assert!(manager.drain_async_cancellations().unwrap().is_empty());
}

#[test]
fn scheme_ports_preserve_binary_file_contents() {
    let path = std::env::temp_dir().join(format!("toyoterm-steel-binary-{}", std::process::id()));
    std::fs::write(&path, [0_u8, 255, 65]).unwrap();
    let mut manager = ConfigManager::new().unwrap();
    let filename = path.to_string_lossy().replace('\\', "/");
    assert_eq!(manager.eval(&format!("(equal? (call-with-input-file \"{filename}\" read-port-to-bytes) (bytevector 0 255 65))")).unwrap(), "true");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn introspection_environment_and_action_validation_match_the_public_contract() {
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(
        manager.eval("(toyoterm-supports? 'async-process)").unwrap(),
        "true"
    );
    assert_eq!(
        manager
            .eval("(> (length (hash-keys->list (toyoterm-env))) 0)")
            .unwrap(),
        "true"
    );
    for source in [
        "(bind-action! \"X\" 'new-tab \"\")",
        "(bind-action! \"X\" 'split 'left 'right)",
        "(on! 'bell 1)",
        "(action! 'user-command 'name)",
    ] {
        assert!(request(&mut manager, source).is_err(), "accepted {source}");
    }
    request(
        &mut manager,
        "(command! 'with-hyphen (lambda (ctx) void)) (bind-command! \"CTRL+G\" 'with-hyphen)",
    )
    .unwrap();
    assert_eq!(
        manager.native_action("CTRL+G"),
        Some(NativeAction::UserCommand("with-hyphen".into()))
    );
    manager
        .runtime
        .call("__collect_garbage", Vec::new())
        .unwrap();
}

#[test]
fn event_failure_restores_cancellation_and_continues_later_handlers() {
    let mut manager = ConfigManager::new().unwrap();
    request(&mut manager, "(define event-task (async! \"fixture\" '() #f)) (on! 'bell (lambda (event) (task-cancel! event-task) (error \"rollback event\")))").unwrap();
    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::Event(ScriptEvent::new(ScriptEventKind::Bell)),
    )
    .unwrap();
    assert_eq!(result.logs.len(), 1);
    assert!(result.async_cancellations.is_empty());
    assert_eq!(manager.eval("(task-pending? event-task)").unwrap(), "true");
}

#[test]
fn collection_inside_a_failing_callback_keeps_rollback_roots_live() {
    let mut manager = ConfigManager::new().unwrap();
    manager.reload(r#"(bind! "CTRL+E" (lambda (ctx) (config-set! 'font.size 25) (#%gc-collect) (error "after collection")))"#).unwrap();
    for _ in 0..2 {
        assert!(
            manager
                .trigger_keybinding("CTRL+E", PaneId(4))
                .unwrap_err()
                .message()
                .contains("after collection")
        );
        assert_eq!(manager.eval("(config-ref 'font.size)").unwrap(), "14.0");
    }
    assert!(
        manager
            .eval_inspect(
                "(list (async-widget \"fixture\" '() 1.0 \"initial\" (lambda (r) \"done\")))"
            )
            .unwrap()
            .contains("#<procedure>")
    );
}

#[test]
fn allocation_driven_collection_preserves_callbacks_and_rollback_roots() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      (define (churn)
        (let loop ([remaining 60000])
          (unless (= remaining 0)
            (let ([cell (box #f)])
              (set-box! cell (lambda () cell)))
            (loop (- remaining 1)))))
      (let ([retained (box 23)])
        (bind! "CTRL+S" (lambda (ctx) (config-set! 'font.size (unbox retained))))
        (bind! "CTRL+E" (lambda (ctx)
          (config-set! 'font.size 25)
          (churn)
          (error "after automatic collection"))))
    "#,
        )
        .unwrap();
    for _ in 0..3 {
        request(&mut manager, "(churn)").unwrap();
        run_script_request(
            &mut manager,
            &script_test_context(),
            &ScriptInvocation::KeyBinding {
                key: "CTRL+S".into(),
                pane: PaneId(4),
            },
        )
        .unwrap();
        assert_eq!(manager.config().font.size, 23.0);
        let error = run_script_request(
            &mut manager,
            &script_test_context(),
            &ScriptInvocation::KeyBinding {
                key: "CTRL+E".into(),
                pane: PaneId(4),
            },
        )
        .unwrap_err();
        assert!(error.message().contains("after automatic collection"));
        assert_eq!(manager.config().font.size, 23.0);
        assert_eq!(manager.eval("(config-ref 'font.size)").unwrap(), "23");
    }
}

#[test]
fn bar_sections_omit_empty_widgets_and_async_cwd_uses_the_current_context() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
        (bar! 'top 'left " | " (list (widget "") (widget "visible") (widget "")))
        (bar! 'bottom 'left " | "
          (list (async-widget "fixture" '() 1.0 "" (lambda (result) "done")
            (lambda (ctx) (handle-ref (context-ref ctx 'pane) 'cwd)))))
    "#,
        )
        .unwrap();
    let mut ctx = script_test_context();
    Arc::make_mut(&mut ctx.model).panes[0].cwd = Some("widget-working-directory".into());
    manager
        .set_live_handles(ctx.handles.iter().copied())
        .unwrap();
    manager.set_object_model(&ctx.model).unwrap();
    assert_eq!(
        manager.render_bar(StatusBarPosition::Top).unwrap()[0].text,
        "visible"
    );
    assert_eq!(
        manager.render_bar(StatusBarPosition::Bottom).unwrap()[0].text,
        ""
    );
    let requests = manager.drain_async_requests().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].cwd.as_deref(), Some("widget-working-directory"));
}
