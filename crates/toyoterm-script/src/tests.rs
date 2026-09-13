use super::*;

fn invoked_action(action: NativeAction, pane: PaneId) -> NativeCommand {
    NativeCommand::InvokeAction {
        action,
        context: ActionContext {
            workspace: WorkspaceId(0),
            window: WindowId(0),
            tab: TabId(0),
            pane,
        },
    }
}

#[test]
fn removed_dsl_methods_and_action_aliases_are_rejected() {
    let mut manager = ConfigManager::new().unwrap();
    for source in [
        "Toyoterm.configure { |c| c.bind('CTRL+A') {} }",
        "Toyoterm::Plugin::Definition.new('test').bind('CTRL+A') {}",
        "Toyoterm.current_workspace.create_window",
        "Toyoterm.current_workspace.focus",
        "Toyoterm.current_window.focus",
        "Toyoterm.current_tab.focus",
        "Toyoterm.current_pane.focus",
        "Toyoterm.workspace(:old)",
        "Toyoterm.switch_workspace(:old)",
        "Toyoterm::BarConfig.new.add(:left, 'old')",
        "Toyoterm::BarConfig.new.group(:left) { |_| }",
    ] {
        let error = manager.eval(source).unwrap_err();
        assert!(
            error.message().contains("NoMethodError"),
            "{source}: {error}"
        );
    }
    assert!(
        manager
            .eval("Toyoterm::Window")
            .unwrap_err()
            .message()
            .contains("NameError")
    );
    assert!(
        manager
            .eval("Toyoterm::BarGroup")
            .unwrap_err()
            .message()
            .contains("NameError")
    );
    assert!(
        manager
            .eval("Toyoterm.configure { |c| c.theme('moon') }")
            .unwrap_err()
            .message()
            .contains("ArgumentError")
    );
    for name in [
        "toggle_pane_zoom",
        "enter_visual_mode",
        "toggle_visual_selection",
        "select",
        "exit_visual_mode",
        "visual_move",
        "copy_visual_selection",
    ] {
        let error = manager
            .eval(&format!(
                "Toyoterm.configure {{ |c| c.keys.ctrl('a').{name} }}"
            ))
            .unwrap_err();
        assert!(error.message().contains("NoMethodError"), "{name}: {error}");
        for source in [
            format!("Toyoterm.action(:{name})"),
            format!("Toyoterm.configure {{ |c| c.keys.ctrl('a').action(:{name}) }}"),
        ] {
            assert!(
                manager
                    .eval(&source)
                    .unwrap_err()
                    .message()
                    .contains("unsupported action")
            );
        }
    }
    assert!(manager.drain_commands(PaneId(0)).unwrap().is_empty());
}

#[test]
fn static_and_runtime_actions_share_names_and_arguments() {
    let actions = [
        ("new_tab", "nil"),
        ("close_pane", "nil"),
        ("close_tab", "nil"),
        ("new_workspace", "nil"),
        ("reload_config", "nil"),
        ("search", "nil"),
        ("maximize_window", "nil"),
        ("toggle_maximize", "nil"),
        ("minimize_window", "nil"),
        ("toggle_fullscreen", "nil"),
        ("toggle_zoom", "nil"),
        ("next_tab", "nil"),
        ("previous_tab", "nil"),
        ("next_workspace", "nil"),
        ("previous_workspace", "nil"),
        ("next_prompt", "nil"),
        ("previous_prompt", "nil"),
        ("next_mark", "nil"),
        ("previous_mark", "nil"),
        ("next_mark", "nil"),
        ("previous_mark", "nil"),
        ("select_next_command_output", "nil"),
        ("select_previous_command_output", "nil"),
        ("select_last_command_output", "nil"),
        ("copy_selection", "nil"),
        ("paste_clipboard", "nil"),
        ("start_visual_mode", "nil"),
        ("toggle_visual_mode", "nil"),
        ("start_visual_selection", "nil"),
        ("select_visual_selection", "nil"),
        ("end_visual_selection", "nil"),
        ("yank_selection", "nil"),
        ("split", ":RIGHT"),
        ("activate_pane", ":DOWN"),
        ("move_visual_selection", ":LINE_END"),
    ];
    for (name, argument) in actions {
        let mut manager = ConfigManager::new().unwrap();
        let args = if argument == "nil" { "" } else { argument };
        manager.reload(&format!(
            "Toyoterm.configure {{ |c| c.keys.ctrl('a').{name}({args}); c.keys.ctrl('b').action('{name}', {argument}) }}"
        )).unwrap();
        let expected = manager.native_action("CTRL+A").unwrap();
        assert_eq!(
            manager.native_action("CTRL+B"),
            Some(expected.clone()),
            "{name}"
        );
        manager
            .eval(&format!(
                "Toyoterm.action('{}', {argument})",
                name.to_uppercase()
            ))
            .unwrap();
        assert_eq!(
            manager.drain_commands(PaneId(0)).unwrap(),
            vec![invoked_action(expected, PaneId(0))],
            "{name}"
        );
    }
    let mut manager = ConfigManager::new().unwrap();
    for args in [
        "''",
        ":unknown",
        ":split",
        ":split, :diagonal",
        ":toggle_zoom, :extra",
        ":visual_move, :page_down",
        ":command, :foo",
    ] {
        let runtime = manager
            .eval(&format!("Toyoterm.action({args})"))
            .unwrap_err();
        let binding = manager
            .eval(&format!(
                "Toyoterm.configure {{ |c| c.keys.ctrl('a').action({args}) }}"
            ))
            .unwrap_err();
        assert!(runtime.message().contains("ArgumentError"));
        assert!(binding.message().contains("ArgumentError"));
        assert!(manager.drain_commands(PaneId(0)).unwrap().is_empty());
    }
}

#[test]
fn fluent_dynamic_bindings_share_context_and_rollback() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
      Toyoterm.configure do |config|
        config.keys do
          ctrl("a").run do |ctx|
            ctx.pane.send_text([ctx.workspace.id, ctx.window.id, ctx.tab.id, ctx.pane.id].join(":"))
          end
          physical("KeyB", "CTRL").run do |ctx|
            ctx.pane.send_text("discard")
            Toyoterm.action(:toggle_zoom)
            raise "cancel"
          end
        end
      end
      Toyoterm.command(:context) do |ctx|
        ctx.pane.send_text([ctx.workspace.id, ctx.window.id, ctx.tab.id, ctx.pane.id].join(":"))
      end
    "#,
        )
        .unwrap();
    let context = script_test_context();
    manager
        .set_live_handles(context.handles.iter().copied())
        .unwrap();
    manager.set_object_model(&context.model).unwrap();
    assert!(manager.native_action("CTRL+A").is_none());
    manager.trigger_keybinding("CTRL+A", PaneId(4)).unwrap();
    manager.trigger_user_command("context", PaneId(4)).unwrap();
    let expected = NativeCommand::Mux(Command::SendText {
        pane: PaneId(4),
        text: "1:2:3:4".into(),
    });
    assert_eq!(
        manager.drain_commands(PaneId(4)).unwrap(),
        vec![expected.clone(), expected]
    );
    assert!(
        manager
            .trigger_keybinding("CTRL+PHYSICAL:KEYB", PaneId(4))
            .is_err()
    );
    assert!(manager.drain_commands(PaneId(4)).unwrap().is_empty());
    for source in [
        "c.keys.ctrl('a').run",
        "c.keys.ctrl('a').run {}; c.keys.ctrl('a').new_tab",
        "c.keys.ctrl('a').new_tab; c.keys.ctrl('a').run {}",
        "c.keys.key('CTRL+A').run {}; c.keys.ctrl('a').run {}",
    ] {
        assert!(
            manager
                .reload(&format!("Toyoterm.configure {{ |c| {source} }}"))
                .is_err()
        );
    }
}

#[test]
fn configuration_sections_return_objects_and_handles_use_consistent_verbs() {
    let mut manager = ConfigManager::new().unwrap();
    manager.eval(r#"
      config = Toyoterm.configure do |c|
        [
          [c.font { nil }, c.font], [c.colors { nil }, c.colors],
          [c.window { nil }, c.window], [c.ui { nil }, c.ui],
          [c.behavior { nil }, c.behavior], [c.keys { nil }, c.keys]
        ].each do |pair|
          raise "wrong section" unless pair[0].class == pair[1].class
        end
      end
      raise "wrong config" unless config.is_a?(Toyoterm::Config)
      [Toyoterm.current_workspace, Toyoterm.current_window, Toyoterm.current_tab, Toyoterm.current_pane].each do |handle|
        raise "wrong receiver" unless handle.activate == handle
      end
      Toyoterm.current_workspace.new_window(command: ["echo", "hello"])
    "#).unwrap();
    let commands = manager.drain_commands(PaneId(0)).unwrap();
    assert_eq!(commands.len(), 5);
    assert!(matches!(
        commands[4],
        NativeCommand::CreateWindowWithLaunch { .. }
    ));
    assert!(
        manager
            .eval("Toyoterm.configure")
            .unwrap_err()
            .message()
            .contains("configuration requires a block")
    );
    manager.set_live_handles([]).unwrap();
    for source in [
        "Toyoterm.current_window.activate",
        "Toyoterm.current_pane.activate",
        "Toyoterm.current_workspace.new_window",
    ] {
        assert!(
            manager
                .eval(source)
                .unwrap_err()
                .message()
                .contains("InvalidHandleError")
        );
    }
}

fn script_test_context() -> ScriptContext {
    ScriptContext {
        model: RubyObjectModel {
            current_workspace: WorkspaceId(1),
            current_window: WindowId(2),
            current_tab: TabId(3),
            current_pane: PaneId(4),
            workspaces: vec![RubyWorkspace {
                id: WorkspaceId(1),
                name: "Workspace 1".into(),
                windows: vec![WindowId(2)],
            }],
            windows: vec![RubyWindow {
                id: WindowId(2),
                tabs: vec![TabId(3)],
            }],
            tabs: vec![RubyTab {
                id: TabId(3),
                title: "Tab 3".into(),
                panes: vec![PaneId(4)],
                zoomed: false,
            }],
            panes: vec![RubyPane {
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
        },
        handles: vec![
            WorkspaceId(1).into(),
            WindowId(2).into(),
            TabId(3).into(),
            PaneId(4).into(),
        ],
        clipboard: None,
    }
}

#[test]
fn script_thread_owns_vm_and_submission_does_not_wait_for_evaluation() {
    let (completion_tx, completion_rx) = mpsc::channel();
    let (thread, _) = ScriptThread::start(None, move |completion| {
        completion_tx
            .send((thread::current().name().map(str::to_owned), completion))
            .unwrap();
    })
    .unwrap();
    let started = Instant::now();
    thread
        .submit(ScriptRequest {
            id: 7,
            context: script_test_context(),
            invocation: ScriptInvocation::Eval(
                "i = 0; while i < 2_000_000; i += 1; end; 42".into(),
            ),
        })
        .unwrap();
    assert!(started.elapsed() < Duration::from_millis(50));

    let (owner, completion) = completion_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(owner.as_deref(), Some("toyoterm-script"));
    assert_eq!(completion.id, 7);
    assert_eq!(completion.result.unwrap().value.as_deref(), Some("42"));
}

#[test]
fn classifies_callbacks_at_the_slow_threshold() {
    assert!(!is_slow_callback(Duration::from_millis(99)));
    assert!(is_slow_callback(Duration::from_millis(100)));
}

#[test]
fn window_bar_uses_typed_context_and_discards_commands() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
                Toyoterm.configure do |config|
                  config.window.bar(:bottom, interval: 0.25) do |bar|
                    bar.section(:left, separator: " ") do |section|
                      section.add do |ctx|
                        ctx.pane.send_text("must not run")
                        [ctx.workspace.name, ctx.tab.title, ctx.pane.title].join(" | ")
                      end
                      section.add("SECOND")
                    end
                    bar.section(:center) { |section| section.add("中央:表示") }
                    bar.section(:right) { |section| section.add { |ctx| ctx.pane.cwd } }
                  end
                end
                "#,
        )
        .unwrap();
    let context = script_test_context();
    manager
        .set_live_handles(context.handles.iter().copied())
        .unwrap();
    manager.set_object_model(&context.model).unwrap();

    assert_eq!(
        manager.config().status_bars,
        vec![StatusBarConfig {
            position: StatusBarPosition::Bottom,
            interval: Duration::from_millis(250),
        }]
    );
    assert_eq!(
        manager.render_bar(StatusBarPosition::Bottom).unwrap(),
        vec![
            BarItem {
                alignment: BarAlignment::Left,
                text: "Workspace 1 | Tab 3 | Pane 4 SECOND".into(),
            },
            BarItem {
                alignment: BarAlignment::Center,
                text: "中央:表示".into(),
            },
            BarItem {
                alignment: BarAlignment::Right,
                text: String::new(),
            },
        ]
    );
    assert!(manager.drain_commands(PaneId(4)).unwrap().is_empty());
}

#[test]
fn repeated_status_bar_requests_keep_the_gc_arena_bounded() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
                Toyoterm.configure do |config|
                  config.window.bar(:bottom, interval: 0.1) do |bar|
                    bar.section(:left) { |section| section.add { |context| context.workspace.name } }
                    bar.section(:right) { |section| section.add { |context| context.pane.cwd } }
                  end
                end
            "#,
        )
        .unwrap();
    let context = script_test_context();
    run_script_request(
        &mut manager,
        &context,
        &ScriptInvocation::Bar {
            position: StatusBarPosition::Bottom,
        },
    )
    .unwrap();
    manager.eval("GC.start").unwrap();
    let baseline = manager.runtime.gc_stats();

    for iteration in 0..2_000 {
        let result = run_script_request(
            &mut manager,
            &context,
            &ScriptInvocation::Bar {
                position: StatusBarPosition::Bottom,
            },
        )
        .unwrap();
        assert_eq!(result.bar.unwrap().len(), 2);
        assert_eq!(
            manager.runtime.gc_stats().arena_index,
            baseline.arena_index,
            "GC arena grew after status bar iteration {iteration}"
        );
    }

    // Force a collection so the object count distinguishes unreachable callback
    // temporaries from persistent Ruby configuration state.
    manager.eval("GC.start").unwrap();
    let after = manager.runtime.gc_stats();
    assert_eq!(after.arena_index, baseline.arena_index);
    assert!(
        after.live_objects <= baseline.live_objects + 64,
        "live mruby objects grew from {} to {}",
        baseline.live_objects,
        after.live_objects,
    );
}

#[test]
fn rollback_restores_in_place_mutations_of_nested_config_values() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r##"Toyoterm.configure { |c| c.font.family = "Mono"; c.font.fallback = ["Emoji"]; c.colors.background = "#112233"; c.colors.ansi[0] = "#010203"; c.window.title = "Before" }"##,
        )
        .unwrap();
    let checkpoint = manager.begin_config_transaction().unwrap();
    let error = manager.eval("Toyoterm.__config.colors.ansi[0] = '#445566'; raise 'rollback'");
    assert!(error.is_err());
    manager.rollback_config_transaction(&checkpoint).unwrap();
    assert_eq!(
        manager.eval("Toyoterm.__config.font.family").unwrap(),
        "Mono"
    );
    assert_eq!(
        manager.eval("Toyoterm.__config.font.fallback[0]").unwrap(),
        "Emoji"
    );
    assert_eq!(
        manager.eval("Toyoterm.__config.colors.background").unwrap(),
        "#112233"
    );
    assert_eq!(
        manager.eval("Toyoterm.__config.colors.ansi[0]").unwrap(),
        "#010203"
    );
    assert_eq!(
        manager.eval("Toyoterm.__config.window.title").unwrap(),
        "Before"
    );
}

#[test]
fn async_tasks_release_runtime_registry_entries_and_can_be_cancelled() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .eval("$task = Toyoterm.async('printf', 'ok')")
        .unwrap();
    let request = manager.drain_async_requests().unwrap().remove(0);
    manager
        .invoke_async_callback(request.id, b"ok", b"", 0)
        .unwrap();
    assert_eq!(manager.eval("$task.result.stdout").unwrap(), "ok");
    assert_eq!(
        manager
            .eval("Toyoterm.instance_variable_get(:@async_tasks).length")
            .unwrap(),
        "0"
    );

    manager
        .eval("$cancelled = Toyoterm.async('sleep', '10')")
        .unwrap();
    assert_eq!(manager.eval("$cancelled.cancel").unwrap(), "true");
    assert_eq!(manager.eval("$cancelled.cancelled?").unwrap(), "true");
    assert_eq!(manager.eval("$cancelled.complete?").unwrap(), "true");
    assert!(manager.drain_async_requests().unwrap().is_empty());

    manager
        .eval("$running = Toyoterm.async('sleep', '10')")
        .unwrap();
    let running = manager.drain_async_requests().unwrap().remove(0);
    assert_eq!(manager.eval("$running.cancel").unwrap(), "true");
    assert_eq!(
        manager.drain_async_cancellations().unwrap(),
        vec![running.id]
    );
}

#[test]
fn registration_handles_support_removal_replacement_and_event_validation() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .eval("$subscription = Toyoterm.on(:bell) { }; $command = Toyoterm.command(:build) { }")
        .unwrap();
    manager.refresh_config_snapshot().unwrap();
    assert!(manager.event_names.contains("bell"));
    assert!(manager.user_command_names.contains("build"));

    manager
        .eval("$subscription.remove; $replacement = Toyoterm.command(:build, replace: true) { |ctx| ctx.pane.send_text('new') }")
        .unwrap();
    manager.refresh_config_snapshot().unwrap();
    assert!(!manager.event_names.contains("bell"));
    assert_eq!(manager.eval("$subscription.active?").unwrap(), "false");
    assert_eq!(manager.eval("$command.active?").unwrap(), "false");
    assert_eq!(manager.eval("$replacement.active?").unwrap(), "true");
    manager.trigger_user_command("build", PaneId(7)).unwrap();
    assert_eq!(
        manager.drain_commands(PaneId(7)).unwrap(),
        vec![NativeCommand::Mux(Command::SendText {
            pane: PaneId(7),
            text: "new".into(),
        })]
    );
    assert!(manager.eval("Toyoterm.on(:pane_focusd) { }").is_err());

    manager.eval("$restored = Toyoterm.on(:bell) { }").unwrap();
    let checkpoint = manager.begin_config_transaction().unwrap();
    assert!(manager.eval("$restored.remove; raise 'rollback'").is_err());
    manager.rollback_config_transaction(&checkpoint).unwrap();
    assert_eq!(manager.eval("$restored.active?").unwrap(), "true");
}

#[test]
fn exposes_api_introspection_logging_and_a_read_only_config_snapshot() {
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(
        manager.eval("Toyoterm.version").unwrap(),
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(
        manager.eval("Toyoterm.api_version").unwrap(),
        PLUGIN_API_VERSION
    );
    assert_eq!(
        manager
            .eval("Toyoterm.supports?(:targeted_actions)")
            .unwrap(),
        "true"
    );
    assert_eq!(
        manager.eval("Toyoterm.supports?(:select_overlay)").unwrap(),
        "true"
    );
    assert_eq!(
        manager.eval("Toyoterm.supports?(:bar_sections)").unwrap(),
        "true"
    );
    assert_eq!(
        manager
            .eval("Toyoterm.supports?(:mixed_bar_groups)")
            .unwrap(),
        "false"
    );
    assert!(
        manager
            .eval("Toyoterm.config[:font][:family] << 'x'")
            .is_err()
    );
    manager.eval("Toyoterm.log(:info, 'ready')").unwrap();
    assert_eq!(
        manager.drain_logs().unwrap(),
        vec![ScriptLog {
            level: "info".into(),
            message: "ready".into(),
        }]
    );
}

#[test]
fn dynamic_actions_capture_the_callback_object_context() {
    let mut manager = ConfigManager::new().unwrap();
    manager.eval("Toyoterm.action(:toggle_zoom)").unwrap();
    assert_eq!(
        manager
            .drain_commands_with_context(WorkspaceId(10), WindowId(20), TabId(30), PaneId(40))
            .unwrap(),
        vec![NativeCommand::InvokeAction {
            action: NativeAction::ToggleZoom,
            context: ActionContext {
                workspace: WorkspaceId(10),
                window: WindowId(20),
                tab: TabId(30),
                pane: PaneId(40),
            },
        }]
    );
}

#[test]
fn async_callbacks_receive_origin_context_and_launch_error_metadata() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .eval("Toyoterm.async('missing') { |result, context| $async_result = result; $async_context = context }")
        .unwrap();
    let request = manager.drain_async_requests().unwrap().remove(0);
    manager
        .invoke_async_callback_with_launch_error(request.id, b"", b"not found", -1, true)
        .unwrap();
    assert_eq!(
        manager.eval("$async_context.class").unwrap(),
        "Toyoterm::CallbackContext"
    );
    assert_eq!(manager.eval("$async_result.launch_error?").unwrap(), "true");
    assert_eq!(manager.eval("$async_result.error_kind").unwrap(), "launch");
}

#[test]
fn plugins_are_evaluated_in_generated_namespaces() {
    let directory = temporary_test_directory("plugin-namespace");
    let failed_plugin = directory.join("failed.rb");
    let plugin = directory.join("namespaced.rb");
    std::fs::write(
        &failed_plugin,
        "PLUGIN_LOCAL_CONSTANT = 41\nraise 'failed plugin'",
    )
    .unwrap();
    std::fs::write(
        &plugin,
        "PLUGIN_LOCAL_CONSTANT = 42\nToyoterm::Plugin.define('namespaced') { |p| p.version = '0.1.0' }",
    )
    .unwrap();
    let mut manager = ConfigManager::new().unwrap();
    assert!(load_plugin(&mut manager.runtime, &failed_plugin).is_err());
    let metadata = load_plugin(&mut manager.runtime, &plugin).unwrap();
    assert_eq!(metadata.name, "namespaced");
    assert_eq!(
        manager
            .eval("Object.const_defined?(:PLUGIN_LOCAL_CONSTANT)")
            .unwrap(),
        "false"
    );
    assert_eq!(
        manager
            .eval("Toyoterm::PluginNamespaces::Plugin1::PLUGIN_LOCAL_CONSTANT")
            .unwrap(),
        "41"
    );
    assert_eq!(
        manager
            .eval("Toyoterm::PluginNamespaces::Plugin2::PLUGIN_LOCAL_CONSTANT")
            .unwrap(),
        "42"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn eval_errors_restore_the_gc_arena_after_copying_the_exception() {
    let mut runtime = MrubyRuntime::new().unwrap();
    let baseline = runtime.gc_stats().arena_index;
    for _ in 0..100 {
        let error = runtime
            .eval("raise 'arena exception with backtrace'")
            .unwrap_err();
        assert!(error.message().contains("arena exception with backtrace"));
        assert_eq!(runtime.gc_stats().arena_index, baseline);
    }
}

#[test]
fn window_bar_interval_defaults_to_one_second_and_rejects_values_below_100ms() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            "Toyoterm.configure { |c| c.window.bar(:bottom) { |bar| bar.section(:left) { |section| section.add('ready') } } }",
        )
        .unwrap();
    assert_eq!(
        manager.config().status_bars,
        vec![StatusBarConfig {
            position: StatusBarPosition::Bottom,
            interval: Duration::from_secs(1),
        }]
    );

    let error = manager
        .reload("Toyoterm.configure { |c| c.window.bar(:bottom, interval: 0.099) { |bar| bar.section(:left) { |section| section.add('too fast') } } }")
        .unwrap_err();
    assert!(error.message().contains("at least 0.1 seconds"));
    assert_eq!(
        manager.render_bar(StatusBarPosition::Bottom).unwrap(),
        vec![BarItem {
            alignment: BarAlignment::Left,
            text: "ready".into(),
        }]
    );
}

#[test]
fn window_bar_supports_top_and_bottom_only() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
                Toyoterm.configure do |config|
                  config.window.bar(:top) { |bar| bar.section(:left) { |section| section.add("TOP") } }
                  config.window.bar(:bottom, interval: 2.0) { |bar| bar.section(:right) { |section| section.add("BOTTOM") } }
                end
            "#,
        )
        .unwrap();

    assert_eq!(manager.config().status_bars.len(), 2);
    assert_eq!(
        manager.render_bar(StatusBarPosition::Top).unwrap()[0].text,
        "TOP"
    );
    assert_eq!(
        manager.render_bar(StatusBarPosition::Bottom).unwrap()[0].text,
        "BOTTOM"
    );

    let error = manager
        .reload("Toyoterm.configure { |c| c.window.bar(:right) { |_| } }")
        .unwrap_err();
    assert!(error.message().contains("window bar position"));
    assert_eq!(manager.config().status_bars.len(), 2);

    let error = manager
        .reload("Toyoterm.configure { |c| c.window.bar(:top) { |_| }; c.window.bar(:top) { |_| } }")
        .unwrap_err();
    assert!(error.message().contains("already configured"));
    assert_eq!(manager.config().status_bars.len(), 2);

    let error = manager.reload("Toyoterm.status { 'removed' }").unwrap_err();
    assert!(error.message().contains("undefined method"));
    assert_eq!(manager.config().status_bars.len(), 2);

    let error = manager
        .reload("Toyoterm.configure { |c| c.window.bar(:top) { |bar| bar.section(:top) { |section| section.add('invalid') } } }")
        .unwrap_err();
    assert!(error.message().contains("section position"));
    assert_eq!(manager.config().status_bars.len(), 2);
}

#[test]
fn evaluates_ruby_in_a_persistent_vm() {
    let mut runtime = MrubyRuntime::new().unwrap();
    assert_eq!(runtime.eval("$value = 6 * 7").unwrap(), "42");
    assert_eq!(runtime.eval("$value + 1").unwrap(), "43");
}

#[test]
fn exposes_bundled_metaprogramming_gems() {
    let mut runtime = MrubyRuntime::new().unwrap();
    assert_eq!(
        runtime
            .eval(
                r##"
                module ToyotermMetaTest
                  class Base
                    def value(prefix)
                      "#{prefix}:base"
                    end
                  end

                  class Child < Base
                    define_method(:dynamic) { |value| value * 2 }
                  end
                end

                object = ToyotermMetaTest::Child.new
                object.instance_variable_set(:@answer, 21)
                object.define_singleton_method(:answer) { @answer }
                method = object.method(:dynamic)
                unbound = ToyotermMetaTest::Base.instance_method(:value)

                [
                  object.send(:answer),
                  object.instance_variables.include?(:@answer),
                  ToyotermMetaTest::Base.subclasses.include?(ToyotermMetaTest::Child),
                  ToyotermMetaTest::Child.class_exec { name },
                  method.call(21),
                  method.owner,
                  unbound.bind(object).call("meta"),
                  object.tap { |value| value.instance_variable_set(:@tapped, true) }
                        .instance_variable_get(:@tapped)
                ]
                "##,
            )
            .unwrap(),
        "[21, true, true, \"ToyotermMetaTest::Child\", 42, ToyotermMetaTest::Child, \"meta:base\", true]"
    );
}

#[test]
fn exposes_bundled_portable_standard_library_gemboxes() {
    let mut runtime = MrubyRuntime::new().unwrap();
    assert_eq!(
        runtime
            .eval(
                r#"
                raise "set" unless Set[1, 1, 2] == Set[1, 2]
                raise "lazy enumerator" unless [1, 2, 3].lazy.map { |n| n * 2 }.force == [2, 4, 6]
                raise "enumerator chain" unless [1].chain([2, 3]).to_a == [1, 2, 3]

                fiber = Fiber.new { Fiber.yield(7); 8 }
                raise "fiber" unless fiber.resume == 7 && fiber.resume == 8

                point = Struct.new(:x, :y).new(3, 4)
                record = Data.define(:name).new("toyoterm")
                raise "struct" unless point.to_a == [3, 4]
                raise "data" unless record.name == "toyoterm" && record.frozen?
                raise "pack" unless [65, 66].pack("C*").unpack("C*") == [65, 66]
                raise "sprintf" unless sprintf("%s-%02d", "ruby", 7) == "ruby-07"
                raise "time" unless Time.at(0).to_i == 0

                random = Random.new(1234).rand(100)
                raise "random" unless random >= 0 && random < 100
                raise "math" unless Math.sqrt(81) == 9.0
                raise "rational" unless Rational(1, 3) + Rational(2, 3) == 1
                raise "complex" unless Complex(2, 3).real == 2
                raise "bigint" unless 2**100 > 2**99

                context = binding
                raise "binding" unless eval("6 * 7", context) == 42
                raise "proc binding" unless proc { 1 }.binding.is_a?(Binding)
                raise "catch" unless catch(:done) { throw(:done, 42) } == 42
                raise "objectspace" unless ObjectSpace.count_objects[:TOTAL] > 0

                "portable stdlib ok"
                "#,
            )
            .unwrap(),
        "portable stdlib ok"
    );
}

#[test]
fn loads_the_configuration_dsl() {
    let mut manager = ConfigManager::new().unwrap();
    let config = manager
        .reload(
            r##"
                Toyoterm.configure do |config|
                  config.font do |font|
                    font.family = "JetBrains Mono"
                    font.fallback = ["Noto Sans Mono CJK JP", "Noto Color Emoji"]
                    font.size = 16
                    font.weight = 500
                  end
                  config.colors.background = "#111111"
                  config.colors.tab_active = "#224466"
                  config.window.opacity = 0.92
                  config.window.title = "my terminal"
                  config.window.width = 1200
                  config.window.always_on_top = true
                  config.ui.padding_x = 12
                  config.ui.line_height = 1.4
                  config.ui.tab_bar = false
                  config.ui.pane_divider_width = 0
                  config.behavior.scroll_lines = 5
                  config.behavior.copy_on_select = true
                  config.behavior.allow_osc52_copy = true
                  config.behavior.allow_osc_notifications = true
                  config.behavior.allow_osc_attention_requests = true
                  config.behavior.allow_osc_open_url = true
                  config.default_shell = "/bin/zsh"
                  config.scrollback_lines = 50_000
                end
                "##,
        )
        .unwrap();

    assert_eq!(config.font.family, "JetBrains Mono");
    assert_eq!(
        config.font.fallback,
        ["Noto Sans Mono CJK JP", "Noto Color Emoji"]
    );
    assert_eq!(config.font.size, 16.0);
    assert_eq!(config.font.weight, 500);
    assert_eq!(config.colors.background, "#111111");
    assert_eq!(config.colors.tab_active, "#224466");
    assert_eq!(config.window.opacity, 0.92);
    assert_eq!(config.window.title, "my terminal");
    assert_eq!(config.window.width, 1200.0);
    assert!(config.window.always_on_top);
    assert_eq!(config.ui.padding_x, 12.0);
    assert_eq!(config.ui.line_height, 1.4);
    assert!(!config.ui.tab_bar);
    assert_eq!(config.ui.pane_divider_width, 0.0);
    assert_eq!(config.behavior.scroll_lines, 5.0);
    assert!(config.behavior.copy_on_select);
    assert!(config.behavior.allow_osc52_copy);
    assert!(config.behavior.allow_osc_notifications);
    assert!(config.behavior.allow_osc_attention_requests);
    assert!(config.behavior.allow_osc_open_url);
    assert_eq!(config.default_shell.as_deref(), Some("/bin/zsh"));
    assert_eq!(config.scrollback_lines, 50_000);
}

#[test]
fn loads_local_plugins_with_metadata_and_registrations() {
    let directory = temporary_test_directory("plugins");
    let plugin = directory.join("git.rb");
    std::fs::write(
        &plugin,
        r#"
            Toyoterm::Plugin.define "git-tools" do |plugin|
              plugin.version = "0.1.0"
              plugin.api_requirement = ">= 0.1.0, < 0.2.0"
              plugin.command(:git_root) { |ctx| ctx.pane.send_text("git root\n") }
              plugin.on(:bell) { |event| event.pane.badge = "bell" }
              plugin.keys.key("CTRL+G").run { |ctx| ctx.pane.send_text("git status\n") }
              plugin.keys.ctrl("h").run { |ctx| ctx.pane.send_text("git log\n") }
              plugin.keys { ctrl_shift("G").command(:git_root) }
            end
            "#,
    )
    .unwrap();

    let loaded = load_config("", "(config)", std::slice::from_ref(&plugin), None).unwrap();
    assert_eq!(
        loaded.plugins,
        [PluginMetadata {
            name: "git-tools".into(),
            version: "0.1.0".into(),
            api_requirement: ">= 0.1.0, < 0.2.0".into(),
            path: plugin.clone(),
        }]
    );
    assert!(loaded.user_command_names.contains("git_root"));
    assert!(loaded.event_names.contains("bell"));
    assert!(loaded.keybindings.contains("CTRL+G"));
    assert!(loaded.keybindings.contains("CTRL+H"));
    assert_eq!(
        loaded.native_actions.get("CTRL+SHIFT+G"),
        Some(&NativeAction::UserCommand("git_root".into()))
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn loads_and_selects_a_theme_defined_by_a_plugin() {
    let directory = temporary_test_directory("theme-plugin");
    let plugin = directory.join("night.rb");
    std::fs::write(
        &plugin,
        r##"
            Toyoterm::Plugin.define "night-themes" do |plugin|
              plugin.version = "0.1.0"
              plugin.theme "moon" do |theme|
                theme.background = "#10131a"
                theme.foreground = "#d8dee9"
                theme.cursor = "#88c0d0"
                theme.zoomed_pane_border = "#abcdef"
                theme.ansi = [
                  "#000000", "#bf616a", "#a3be8c", "#ebcb8b",
                  "#81a1c1", "#b48ead", "#88c0d0", "#e5e9f0",
                  "#4c566a", "#bf616a", "#a3be8c", "#ebcb8b",
                  "#81a1c1", "#b48ead", "#8fbcbb", "#eceff4"
                ]
              end
            end
        "##,
    )
    .unwrap();

    let mut loaded = load_config(
        r##"
            Toyoterm.plugin "night.rb"
            Toyoterm.configure do |config|
              config.theme = "moon"
              config.colors.cursor = "#ffffff"
              config.colors.ansi[1] = "#ff0000"
            end
        "##,
        &directory.join("config.rb").display().to_string(),
        &[],
        Some(&directory),
    )
    .unwrap();

    assert_eq!(loaded.config.colors.background, "#10131a");
    assert_eq!(loaded.config.colors.foreground, "#d8dee9");
    assert_eq!(loaded.config.colors.zoomed_pane_border, "#abcdef");
    assert_eq!(loaded.config.colors.cursor, "#ffffff");
    assert_eq!(loaded.config.colors.ansi[1], "#ff0000");
    assert_eq!(loaded.config.colors.ansi[2], "#a3be8c");
    assert_eq!(
        loaded.runtime.eval("Toyoterm.themes.join(',')").unwrap(),
        "moon"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn rejects_an_unknown_theme_without_replacing_the_active_config() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(r##"Toyoterm.configure { |config| config.colors.background = "#123456" }"##)
        .unwrap();

    let error = manager
        .reload("Toyoterm.configure { |config| config.theme = 'missing' }")
        .unwrap_err();

    assert!(error.message().contains("unknown theme: missing"));
    assert_eq!(manager.config().colors.background, "#123456");
}

#[test]
fn plugin_failures_are_isolated_and_rolled_back() {
    let directory = temporary_test_directory("plugin-isolation");
    let broken = directory.join("10-broken.rb");
    let incompatible = directory.join("20-incompatible.rb");
    let healthy = directory.join("30-healthy.rb");
    std::fs::write(
        &broken,
        r#"
            Toyoterm::Plugin.define "broken" do |plugin|
              plugin.version = "0.1.0"
              plugin.command(:leaked) { }
              raise "boom"
            end
            "#,
    )
    .unwrap();
    std::fs::write(
        &incompatible,
        r#"
            Toyoterm::Plugin.define "future" do |plugin|
              plugin.version = "1.0.0"
              plugin.api_requirement = ">= 1.0.0"
              plugin.command(:also_leaked) { }
            end
            "#,
    )
    .unwrap();
    std::fs::write(
        &healthy,
        r#"
            Toyoterm::Plugin.define "healthy" do |plugin|
              plugin.version = "0.2.0"
              plugin.command(:works) { }
            end
            "#,
    )
    .unwrap();

    let loaded = load_config(
        "",
        "(config)",
        &[broken, incompatible, healthy.clone()],
        None,
    )
    .unwrap();
    assert_eq!(
        loaded
            .plugins
            .iter()
            .map(|plugin| plugin.name.as_str())
            .collect::<Vec<_>>(),
        ["healthy"]
    );
    assert_eq!(loaded.user_command_names, HashSet::from(["works".into()]));
    assert_eq!(loaded.plugins[0].path, healthy);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn explicit_plugins_resolve_relative_to_the_config_and_keep_declaration_order() {
    let directory = temporary_test_directory("explicit-plugins");
    let plugins = directory.join("plugins");
    std::fs::create_dir(&plugins).unwrap();
    for (file, name) in [("second.rb", "second"), ("first.rb", "first")] {
        std::fs::write(
            plugins.join(file),
            format!(
                "Toyoterm::Plugin.define {name:?} do |plugin|\n  plugin.version = \"0.1.0\"\nend\n"
            ),
        )
        .unwrap();
    }
    let loaded = load_config(
        "Toyoterm.plugin 'plugins/second.rb'; Toyoterm.plugin 'plugins/first.rb'",
        &directory.join("config.rb").display().to_string(),
        &[],
        Some(&directory),
    )
    .unwrap();
    assert_eq!(
        loaded
            .plugins
            .iter()
            .map(|plugin| plugin.name.as_str())
            .collect::<Vec<_>>(),
        ["second", "first"]
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn automatic_plugins_are_sorted_and_duplicate_names_do_not_stop_later_plugins() {
    let directory = temporary_test_directory("plugin-order");
    for (file, name) in [
        ("20-duplicate.rb", "shared"),
        ("10-first.rb", "shared"),
        ("30-last.rb", "last"),
        ("ignored.txt", "ignored"),
    ] {
        std::fs::write(
            directory.join(file),
            format!(
                "Toyoterm::Plugin.define {name:?} do |plugin|\n  plugin.version = \"0.1.0\"\nend\n"
            ),
        )
        .unwrap();
    }
    let paths = discover_plugins(&directory);
    assert_eq!(
        paths
            .iter()
            .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
            .collect::<Vec<_>>(),
        ["10-first.rb", "20-duplicate.rb", "30-last.rb"]
    );
    let loaded = load_config("", "(config)", &paths, None).unwrap();
    assert_eq!(
        loaded
            .plugins
            .iter()
            .map(|plugin| plugin.name.as_str())
            .collect::<Vec<_>>(),
        ["shared", "last"]
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn plugin_versions_use_strict_semver_triplets() {
    assert_eq!(parse_semver("0.1.0"), Ok((0, 1, 0)));
    assert!(parse_semver("0.1").is_err());
    assert!(parse_semver("0.01.0").is_err());
    assert!(parse_semver("0.1.beta").is_err());
}

fn temporary_test_directory(label: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("toyoterm-{label}-{}-{unique}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    path
}

#[test]
fn rejects_font_weight_outside_css_range() {
    let mut manager = ConfigManager::new().unwrap();
    let error = manager
        .reload("Toyoterm.configure { |config| config.font.weight = 1001 }")
        .unwrap_err();
    assert!(
        error
            .message()
            .contains("font weight must be between 1 and 1000")
    );
}

#[test]
fn rejects_invalid_font_fallbacks() {
    let mut manager = ConfigManager::new().unwrap();
    let error = manager
        .reload("Toyoterm.configure { |config| config.font.fallback = 'emoji' }")
        .unwrap_err();
    assert!(error.message().contains("font fallback must be an array"));

    let error = manager
        .reload("Toyoterm.configure { |config| config.font.fallback = [''] }")
        .unwrap_err();
    assert!(error.message().contains("cannot be empty"));

    let error = manager
        .reload("Toyoterm.configure { |config| config.font.fallback = ['Noto', 'Noto'] }")
        .unwrap_err();
    assert!(error.message().contains("duplicate font family"));
}

#[test]
fn bundled_minimal_configuration_is_executable() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(include_str!("../../../examples/minimal_config.rb"))
        .unwrap();
    assert_eq!(manager.config().font.size, 14.0);
    assert_eq!(manager.config().scrollback_lines, 10_000);
    assert!(
        manager
            .trigger_keybinding("CTRL+SHIFT+H", PaneId(7))
            .unwrap()
    );
}

#[test]
fn failed_reload_preserves_the_previous_runtime_and_config() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload("Toyoterm.configure { |config| config.font.size = 18 }")
        .unwrap();

    let error = manager.reload("Toyoterm.configure {").unwrap_err();
    assert_eq!(error.operation(), "evaluate mruby");
    assert_eq!(manager.config().font.size, 18.0);
    assert_eq!(manager.eval("Toyoterm.__config.font.size").unwrap(), "18");
}

#[test]
fn rejects_invalid_colors_without_replacing_the_config() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(r##"Toyoterm.configure { |config| config.colors.cursor = "#123456" }"##)
        .unwrap();

    let error = manager
        .reload(r#"Toyoterm.configure { |config| config.colors.cursor = "red" }"#)
        .unwrap_err();

    assert_eq!(error.operation(), "evaluate mruby");
    assert!(error.message().contains("#RRGGBB"));
    assert_eq!(manager.config().colors.cursor, "#123456");
}

#[test]
fn bundled_default_key_configuration_is_executable() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(include_str!("../../../examples/default_config.rb"))
        .unwrap();

    assert!(manager.native_action("F11").is_some());
    let search_key = if cfg!(target_os = "macos") {
        "SHIFT+SUPER+F"
    } else {
        "CTRL+SHIFT+F"
    };
    assert!(manager.native_action(search_key).is_some());
    assert!(manager.native_action("CTRL+TAB").is_some());
    assert!(manager.native_action("CTRL+SHIFT+P").is_none());
    assert!(manager.native_action("SHIFT+SUPER+P").is_none());
}

#[test]
fn registers_toggle_zoom_keybinding() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
            Toyoterm.configure do |config|
              config.keys.ctrl("z").toggle_zoom
              config.keys.ctrl_shift("z").toggle_zoom
            end
            "#,
        )
        .unwrap();

    assert_eq!(
        manager.native_action("CTRL+Z"),
        Some(NativeAction::ToggleZoom)
    );
    assert_eq!(
        manager.native_action("CTRL+SHIFT+Z"),
        Some(NativeAction::ToggleZoom)
    );
}

#[test]
fn exposes_the_host_platform_to_ruby() {
    let mut manager = ConfigManager::new().unwrap();
    manager.reload("").unwrap();

    let expected = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "other"
    };
    assert_eq!(manager.eval("Toyoterm.platform"), Ok(expected.to_owned()));
    assert_eq!(
        manager.eval("Toyoterm.platform.class"),
        Ok("Symbol".to_owned())
    );
}

#[test]
fn loads_and_validates_the_ansi_palette() {
    let mut manager = ConfigManager::new().unwrap();
    let config = manager
        .reload(r##"Toyoterm.configure { |config| config.colors.ansi[1] = "#123456" }"##)
        .unwrap();
    assert_eq!(config.colors.ansi.len(), 16);
    assert_eq!(config.colors.ansi[1], "#123456");

    let error = manager
        .reload("Toyoterm.configure { |config| config.colors.ansi = [] }")
        .unwrap_err();
    assert!(error.message().contains("exactly 16 colors"));
    assert_eq!(manager.config().colors.ansi[1], "#123456");
}

#[test]
fn reloads_the_selected_file_atomically() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "toyoterm-config-{}-{unique}.rb",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "Toyoterm.configure { |config| config.font.size = 17 }",
    )
    .unwrap();
    let mut manager = ConfigManager::load_startup(Some(&path)).unwrap();
    assert_eq!(manager.source_path(), Some(path.as_path()));
    assert_eq!(manager.config().font.size, 17.0);

    std::fs::write(
        &path,
        "Toyoterm.configure { |config| config.font.size = 19 }",
    )
    .unwrap();
    manager.reload_file().unwrap();
    assert_eq!(manager.config().font.size, 19.0);

    std::fs::write(&path, "Toyoterm.configure {").unwrap();
    assert!(manager.reload_file().is_err());
    assert_eq!(manager.config().font.size, 19.0);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn reports_config_filename_line_and_ruby_backtrace() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "toyoterm-broken-config-{}-{unique}.rb",
        std::process::id()
    ));
    std::fs::write(
        &path,
        r#"def fail_config
  raise "broken config"
end
fail_config
"#,
    )
    .unwrap();

    let error = match ConfigManager::load_startup(Some(&path)) {
        Ok(_) => panic!("broken config unexpectedly loaded"),
        Err(error) => error,
    };
    std::fs::remove_file(&path).unwrap();
    let message = error.to_string();

    assert!(message.contains(&path.display().to_string()), "{message}");
    assert!(message.contains(":2"), "{message}");
    assert!(message.contains(":4"), "{message}");
    assert!(message.contains("broken config"), "{message}");
}

#[test]
fn gui_startup_recovers_with_defaults_and_keeps_the_broken_path() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "toyoterm-gui-config-{}-{unique}.rb",
        std::process::id()
    ));
    std::fs::write(&path, "raise 'broken GUI config'").unwrap();

    let (manager, error) = ConfigManager::load_startup_recovering(Some(&path)).unwrap();
    std::fs::remove_file(&path).unwrap();

    assert_eq!(manager.source_path(), Some(path.as_path()));
    assert_eq!(manager.config(), &ToyotermConfig::default());
    assert!(
        error
            .expect("broken config should be reported")
            .message()
            .contains("broken GUI config")
    );
}

#[test]
fn converts_pane_send_text_to_a_native_command() {
    let mut manager = ConfigManager::new().unwrap();
    manager.set_current_pane(PaneId(42)).unwrap();
    manager
        .eval(r#"Toyoterm.current_pane.send_text("echo hello\n")"#)
        .unwrap();

    assert_eq!(
        manager.drain_commands(PaneId(42)).unwrap(),
        vec![NativeCommand::Mux(Command::SendText {
            pane: PaneId(42),
            text: "echo hello\n".into(),
        })]
    );
    assert!(manager.drain_commands(PaneId(42)).unwrap().is_empty());
}

#[test]
fn exposes_the_synced_ruby_object_model() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .set_live_handles([
            NativeHandle::from(WorkspaceId(10)),
            NativeHandle::from(WindowId(20)),
            NativeHandle::from(TabId(30)),
            NativeHandle::from(PaneId(40)),
        ])
        .unwrap();
    manager
        .set_object_model(&RubyObjectModel {
            current_workspace: WorkspaceId(10),
            current_window: WindowId(20),
            current_tab: TabId(30),
            current_pane: PaneId(40),
            workspaces: vec![RubyWorkspace {
                id: WorkspaceId(10),
                name: "backend".into(),
                windows: vec![WindowId(20)],
            }],
            windows: vec![RubyWindow {
                id: WindowId(20),
                tabs: vec![TabId(30)],
            }],
            tabs: vec![RubyTab {
                id: TabId(30),
                title: "server".into(),
                panes: vec![PaneId(40)],
                zoomed: true,
            }],
            panes: vec![RubyPane {
                id: PaneId(40),
                title: "shell".into(),
                icon_title: Some("server icon".into()),
                cwd: Some("/srv/app".into()),
                remote_host: Some("alice@build.example.com".into()),
                shell_integration_version: Some(12),
                shell_integration_shell: Some("fish".into()),
                user_vars: vec![("gitBranch".into(), "main".into())],
                pid: Some(1234),
                command_running: true,
                last_exit_status: Some(17),
                screen_text: "build started\ncompiling toyoterm".into(),
                zoomed: true,
            }],
        })
        .unwrap();

    assert_eq!(
        manager.eval("Toyoterm.current_workspace.name").unwrap(),
        "backend"
    );
    assert_eq!(
        manager
            .eval("Toyoterm.find_workspace('backend').id")
            .unwrap(),
        "10"
    );
    assert_eq!(
        manager.eval("Toyoterm.workspaces.map(&:id)").unwrap(),
        "[10]"
    );
    assert_eq!(manager.eval("Toyoterm.windows.map(&:id)").unwrap(), "[20]");
    assert_eq!(
        manager.eval("Toyoterm.current_window.tabs[0].id").unwrap(),
        "30"
    );
    assert_eq!(
        manager.eval("Toyoterm.current_tab.title").unwrap(),
        "server"
    );
    assert_eq!(
        manager.eval("Toyoterm.current_tab.zoomed?").unwrap(),
        "true"
    );
    assert_eq!(
        manager.eval("Toyoterm.current_tab.panes[0].title").unwrap(),
        "shell"
    );
    assert_eq!(
        manager.eval("Toyoterm.current_pane.icon_title").unwrap(),
        "server icon"
    );
    assert_eq!(
        manager.eval("Toyoterm.current_pane.cwd").unwrap(),
        "/srv/app"
    );
    assert_eq!(manager.eval("Toyoterm.current_pane.pid").unwrap(), "1234");
    assert_eq!(
        manager.eval("Toyoterm.current_pane.remote_host").unwrap(),
        "alice@build.example.com"
    );
    assert_eq!(
        manager
            .eval("Toyoterm.current_pane.shell_integration_version")
            .unwrap(),
        "12"
    );
    assert_eq!(
        manager
            .eval("Toyoterm.current_pane.shell_integration_shell")
            .unwrap(),
        "fish"
    );
    assert_eq!(
        manager.eval("Toyoterm.current_pane.user_vars").unwrap(),
        r#"{"gitBranch" => "main"}"#
    );
    manager
        .eval("Toyoterm.current_pane.user_vars['gitBranch'] = 'changed'")
        .unwrap();
    assert_eq!(
        manager
            .eval("Toyoterm.current_pane.user_vars['gitBranch']")
            .unwrap(),
        "main"
    );
    assert_eq!(
        manager.eval("Toyoterm.current_pane.zoomed?").unwrap(),
        "true"
    );
    assert_eq!(
        manager
            .eval("Toyoterm.current_pane.command_running?")
            .unwrap(),
        "true"
    );
    assert_eq!(
        manager
            .eval("Toyoterm.current_pane.last_exit_status")
            .unwrap(),
        "17"
    );
    assert_eq!(
        manager.eval("Toyoterm.current_pane.screen_text").unwrap(),
        "build started\ncompiling toyoterm"
    );
    manager
        .eval("Toyoterm.current_pane.screen_text << ' changed'")
        .unwrap();
    assert_eq!(
        manager.eval("Toyoterm.current_pane.screen_text").unwrap(),
        "build started\ncompiling toyoterm"
    );
    assert_eq!(
        manager.eval("Toyoterm.find_workspace('missing')").unwrap(),
        ""
    );

    manager.eval("Toyoterm.current_pane.badge = 'dev'").unwrap();
    assert_eq!(manager.eval("Toyoterm.current_pane.badge").unwrap(), "dev");
    assert_eq!(
        manager.drain_commands(PaneId(40)).unwrap(),
        vec![NativeCommand::SetPaneBadge {
            pane: PaneId(40),
            badge: Some("dev".into()),
        }]
    );
    manager.eval("Toyoterm.current_pane.badge = nil").unwrap();
    assert_eq!(
        manager.drain_commands(PaneId(40)).unwrap(),
        vec![NativeCommand::SetPaneBadge {
            pane: PaneId(40),
            badge: None,
        }]
    );
}

#[test]
fn converts_object_model_operations_to_native_commands() {
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(
        manager
            .eval(
                "[Toyoterm.current_pane.split(:left), \
                      Toyoterm.current_window.new_tab, \
                      Toyoterm.current_workspace.new_window].map(&:inspect)",
            )
            .unwrap(),
        "[\"nil\", \"nil\", \"nil\"]"
    );
    manager
        .eval(
            "Toyoterm.current_pane.activate; Toyoterm.current_tab.close; \
                 Toyoterm.current_window.close; Toyoterm.current_workspace.activate",
        )
        .unwrap();

    assert_eq!(
        manager
            .drain_commands_with_context(WorkspaceId(10), WindowId(20), TabId(30), PaneId(40),)
            .unwrap(),
        vec![
            NativeCommand::Mux(Command::Split {
                pane: PaneId(40),
                direction: SplitDirection::Left,
            }),
            NativeCommand::Mux(Command::NewTabIn(WindowId(20))),
            NativeCommand::Mux(Command::CreateWindow(WorkspaceId(10))),
            NativeCommand::Mux(Command::ActivatePane(PaneId(40))),
            NativeCommand::Mux(Command::CloseTab(TabId(30))),
            NativeCommand::Mux(Command::CloseWindow(WindowId(20))),
            NativeCommand::Mux(Command::ActivateWorkspace(WorkspaceId(10))),
        ]
    );
}

#[test]
fn opens_a_named_workspace_through_a_native_command() {
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(
        manager.eval("Toyoterm.open_workspace(:backend)").unwrap(),
        ""
    );
    assert_eq!(
        manager.drain_commands(PaneId(40)).unwrap(),
        vec![NativeCommand::Mux(Command::SwitchWorkspace(
            "backend".into()
        ))]
    );

    for (source, message) in [
        (
            "Toyoterm.open_workspace('')",
            "workspace name cannot be empty",
        ),
        (
            "Toyoterm.open_workspace(\"bad\\0name\")",
            "workspace name contains a NUL byte",
        ),
    ] {
        let error = manager.eval(source).unwrap_err();
        assert!(error.message().contains(message), "{error}");
    }
    assert!(manager.drain_commands(PaneId(40)).unwrap().is_empty());
}

#[test]
fn queues_builtin_actions_from_ruby_callbacks() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .eval(
            r#"
            Toyoterm.action(:toggle_fullscreen)
            Toyoterm.action(:search)
            Toyoterm.action(:yank_selection)
            Toyoterm.action(:split, :down)
            Toyoterm.action(:activate_pane, :left)
            Toyoterm.action(:move_visual_selection, :line_end)
            "#,
        )
        .unwrap();

    assert_eq!(
        manager.drain_commands(PaneId(40)).unwrap(),
        vec![
            invoked_action(NativeAction::ToggleFullscreen, PaneId(40)),
            invoked_action(NativeAction::Search, PaneId(40)),
            invoked_action(NativeAction::YankSelection, PaneId(40)),
            invoked_action(NativeAction::Split(SplitDirection::Down), PaneId(40)),
            invoked_action(NativeAction::ActivatePane(SplitDirection::Left), PaneId(40)),
            invoked_action(
                NativeAction::MoveVisualSelection(toyoterm_api::SelectionMotion::LineEnd,),
                PaneId(40)
            ),
        ]
    );
}

#[test]
fn validates_and_rolls_back_builtin_actions() {
    let mut manager = ConfigManager::new().unwrap();
    for (source, message) in [
        ("Toyoterm.action('')", "action name cannot be empty"),
        (
            "Toyoterm.action(:toggle_fullscreen, :extra)",
            "does not accept an argument",
        ),
        (
            "Toyoterm.action(:split)",
            "action split requires left, right, up, or down",
        ),
        (
            "Toyoterm.action(:move_visual_selection, :page_down)",
            "move_visual_selection requires",
        ),
        (
            "Toyoterm.action(:user_command, :recursive)",
            "unsupported action: user_command",
        ),
    ] {
        let error = manager.eval(source).unwrap_err();
        assert!(error.message().contains(message), "{error}");
    }
    assert!(manager.drain_commands(PaneId(40)).unwrap().is_empty());

    manager
        .reload(
            r#"
            Toyoterm.configure do |config|
              config.keys.key("CTRL+A").run do
                Toyoterm.action(:toggle_fullscreen)
                raise "cancel action"
              end
            end
            "#,
        )
        .unwrap();
    let error = manager
        .trigger_keybinding("CTRL+A", PaneId(40))
        .unwrap_err();
    assert!(error.message().contains("cancel action"));
    assert!(manager.drain_commands(PaneId(40)).unwrap().is_empty());
}

#[test]
fn converts_custom_pane_launches_to_native_commands() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .eval(
            r#"
            Toyoterm.current_pane.split(
              :right,
              command: ["ssh", "devbox"],
              cwd: "/srv/app",
              env: { "MODE" => "dev", "OLD_TOKEN" => nil }
            )
            Toyoterm.current_window.new_tab(command: "btop")
            Toyoterm.current_window.new_tab(cwd: "/tmp")
            Toyoterm.current_workspace.new_window(
              command: ["tail", "-f", "app.log"],
              env: { "LC_ALL" => "C" }
            )
            "#,
        )
        .unwrap();

    assert_eq!(
        manager
            .drain_commands_with_context(WorkspaceId(10), WindowId(20), TabId(30), PaneId(40))
            .unwrap(),
        vec![
            NativeCommand::SplitWithLaunch {
                pane: PaneId(40),
                direction: SplitDirection::Right,
                launch: PaneLaunchSpec {
                    program: Some("ssh".into()),
                    args: vec!["devbox".into()],
                    cwd: Some("/srv/app".into()),
                    environment: vec![
                        ("MODE".into(), Some("dev".into())),
                        ("OLD_TOKEN".into(), None),
                    ],
                },
            },
            NativeCommand::NewTabWithLaunch {
                window: WindowId(20),
                launch: PaneLaunchSpec {
                    program: Some("btop".into()),
                    args: Vec::new(),
                    cwd: None,
                    environment: Vec::new(),
                },
            },
            NativeCommand::NewTabWithLaunch {
                window: WindowId(20),
                launch: PaneLaunchSpec {
                    program: None,
                    args: Vec::new(),
                    cwd: Some("/tmp".into()),
                    environment: Vec::new(),
                },
            },
            NativeCommand::CreateWindowWithLaunch {
                workspace: WorkspaceId(10),
                launch: PaneLaunchSpec {
                    program: Some("tail".into()),
                    args: vec!["-f".into(), "app.log".into()],
                    cwd: None,
                    environment: vec![("LC_ALL".into(), Some("C".into()))],
                },
            },
        ]
    );
}

#[test]
fn validates_custom_pane_launch_options() {
    let mut manager = ConfigManager::new().unwrap();
    for (source, message) in [
        (
            "Toyoterm.current_pane.split(:right, command: [])",
            "command array cannot be empty",
        ),
        (
            "Toyoterm.current_pane.split(1)",
            "split direction must be a String or Symbol",
        ),
        (
            "Toyoterm.current_pane.split(:right, command: ['sh', 1])",
            "command array entries must be strings",
        ),
        (
            "Toyoterm.current_window.new_tab(cwd: '')",
            "cwd cannot be empty",
        ),
        (
            "Toyoterm.current_window.new_tab(env: {'OK' => 1})",
            "environment values must be strings or nil",
        ),
        (
            "Toyoterm.current_window.new_tab(env: {'BAD=NAME' => 'value'})",
            "environment name cannot contain =",
        ),
        (
            "Toyoterm.current_window.new_tab(command: \"bad\\0program\")",
            "launch value contains a NUL byte",
        ),
    ] {
        let error = manager.eval(source).unwrap_err();
        assert!(error.message().contains(message), "{error}");
    }
    assert!(manager.drain_commands(PaneId(1)).unwrap().is_empty());
}

#[test]
fn converts_pane_searches_to_native_commands() {
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(
        manager
            .eval(
                "[Toyoterm.current_pane.search('error'), \
                  Toyoterm.current_pane.search('warning', direction: :previous)].map(&:inspect)",
            )
            .unwrap(),
        "[\"#<Toyoterm::Pane:0>\", \"#<Toyoterm::Pane:0>\"]"
    );

    assert_eq!(
        manager.drain_commands(PaneId(42)).unwrap(),
        vec![
            NativeCommand::SearchPane {
                pane: PaneId(42),
                query: "error".into(),
                direction: PaneSearchDirection::Next,
            },
            NativeCommand::SearchPane {
                pane: PaneId(42),
                query: "warning".into(),
                direction: PaneSearchDirection::Previous,
            },
        ]
    );
}

#[test]
fn validates_pane_search_options_before_queueing() {
    let mut manager = ConfigManager::new().unwrap();
    for (source, message) in [
        (
            "Toyoterm.current_pane.search('')",
            "search query cannot be empty",
        ),
        (
            "Toyoterm.current_pane.search(\"bad\\0query\")",
            "search query contains a NUL byte",
        ),
        (
            "Toyoterm.current_pane.search('error', direction: :sideways)",
            "search direction must be next or previous",
        ),
    ] {
        let error = manager.eval(source).unwrap_err();
        assert!(error.message().contains(message), "{error}");
    }
    assert!(manager.drain_commands(PaneId(1)).unwrap().is_empty());
}

#[test]
fn ruby_native_handles_are_typed_id_values() {
    let mut manager = ConfigManager::new().unwrap();
    manager.set_current_pane(PaneId(42)).unwrap();

    assert_eq!(
        manager
            .eval("Toyoterm.current_pane.class.superclass")
            .unwrap(),
        "Toyoterm::NativeHandle"
    );
    assert_eq!(manager.eval("Toyoterm.current_pane.id").unwrap(), "42");
    assert_eq!(
        manager.eval("Toyoterm.current_pane.inspect").unwrap(),
        "#<Toyoterm::Pane:42>"
    );
    assert_eq!(
        manager
            .eval("Toyoterm::Pane.new(7) == Toyoterm::Pane.new(7)")
            .unwrap(),
        "true"
    );
    assert_eq!(
        manager
            .eval("Toyoterm::Pane.new(7) == Toyoterm::Tab.new(7)")
            .unwrap(),
        "false"
    );

    let error = manager.eval("Toyoterm::Pane.new(-1)").unwrap_err();
    assert!(error.message().contains("non-negative integer"));
}

#[test]
fn deleted_ruby_handles_raise_a_typed_exception() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .set_live_handles([
            NativeHandle::new(HandleKind::Workspace, 1),
            NativeHandle::new(HandleKind::Pane, 7),
        ])
        .unwrap();
    manager.set_current_pane(PaneId(7)).unwrap();
    manager.eval("$saved_pane = Toyoterm.current_pane").unwrap();
    assert_eq!(manager.eval("$saved_pane.valid?").unwrap(), "true");

    manager
        .set_live_handles([NativeHandle::new(HandleKind::Workspace, 1)])
        .unwrap();
    assert_eq!(manager.eval("$saved_pane.valid?").unwrap(), "false");
    let error = manager.eval("$saved_pane.send_text('stale')").unwrap_err();
    assert!(error.message().contains("Toyoterm::InvalidHandleError"));
    assert!(error.message().contains("invalid pane handle 7"));
    assert!(manager.drain_commands(PaneId(9)).unwrap().is_empty());
}

#[test]
fn exposes_clipboard_read_and_write_to_ruby() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .set_clipboard_text(Some("こんにちは\nclipboard"))
        .unwrap();

    assert_eq!(
        manager.eval("Toyoterm.clipboard.read").unwrap(),
        "こんにちは\nclipboard"
    );
    manager
        .eval(r#"Toyoterm.clipboard.write("copied from Ruby")"#)
        .unwrap();
    assert_eq!(
        manager.drain_commands(PaneId(42)).unwrap(),
        vec![NativeCommand::ClipboardWrite("copied from Ruby".into())]
    );
}

#[test]
fn exposes_environment_as_an_isolated_string_hash() {
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(
        manager
            .eval("Toyoterm.env.is_a?(Hash) && Toyoterm.env.keys.all? { |key| key.is_a?(String) }")
            .unwrap(),
        "true"
    );
    assert_eq!(
            manager
                .eval("copy = Toyoterm.env; copy['TOYOTERM_TEST_ONLY'] = 'changed'; Toyoterm.env['TOYOTERM_TEST_ONLY'].nil?")
                .unwrap(),
            "true"
        );
}

#[test]
fn read_file_preserves_arbitrary_bytes_and_maps_io_errors() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "toyoterm-ruby-read-{}-{unique}.bin",
        std::process::id()
    ));
    std::fs::write(&path, b"left\0\xffright").unwrap();
    let literal = ruby_string_literal(path.to_str().unwrap());
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(
        manager
            .eval(&format!("Toyoterm.read_file({literal}).bytes.join(',')"))
            .unwrap(),
        "108,101,102,116,0,255,114,105,103,104,116"
    );
    std::fs::remove_file(&path).unwrap();
    let error = manager
        .eval(&format!("Toyoterm.read_file({literal})"))
        .unwrap_err();
    assert!(error.message().contains("read"));
}

#[cfg(unix)]
#[test]
fn spawn_captures_stdout_stderr_and_nonzero_status() {
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(
            manager
                .eval(
                    r#"result = Toyoterm.spawn("/bin/sh", "-c", "printf out; printf err >&2; exit 7"); [result.stdout, result.stderr, result.exit_status, result.success?].join('|')"#,
                )
                .unwrap(),
            "out|err|7|false"
        );
    let error = manager
        .eval(r#"Toyoterm.spawn("/definitely/missing/toyoterm-program")"#)
        .unwrap_err();
    assert!(error.message().contains("spawn"));
}

#[cfg(unix)]
#[test]
fn spawn_uses_requested_working_directory() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "toyoterm-ruby-spawn-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("marker.txt"), b"requested cwd").unwrap();
    let literal = ruby_string_literal(directory.to_str().unwrap());
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(
        manager
            .eval(&format!(
                r#"Toyoterm.spawn("/bin/sh", "-c", "cat marker.txt", cwd: {literal}).stdout"#
            ))
            .unwrap(),
        "requested cwd"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(windows)]
#[test]
fn spawn_captures_stdout_stderr_and_nonzero_status() {
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(
            manager
                .eval(
                    r#"result = Toyoterm.spawn("cmd", "/C", "<nul set /p=out&1>&2 <nul set /p=err&exit /b 7"); [result.stdout, result.stderr, result.exit_status, result.success?].join('|')"#,
                )
                .unwrap(),
            "out|err|7|false"
        );
}

#[cfg(windows)]
#[test]
fn spawn_uses_requested_working_directory() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "toyoterm-ruby-spawn-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("marker.txt"), b"requested cwd").unwrap();
    let literal = ruby_string_literal(directory.to_str().unwrap());
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(
        manager
            .eval(&format!(
                r#"Toyoterm.spawn("cmd", "/C", "type marker.txt", cwd: {literal}).stdout"#
            ))
            .unwrap(),
        "requested cwd"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(windows)]
#[test]
fn spawn_normalizes_windows_working_directory_with_leading_slash() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "toyoterm-ruby-spawn-slash-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("marker.txt"), b"normalized leading slash").unwrap();
    let slash_path = format!("/{}", directory.to_str().unwrap().replace('\\', "/"));
    let literal = ruby_string_literal(&slash_path);
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(
        manager
            .eval(&format!(
                r#"Toyoterm.spawn("cmd", "/C", "type marker.txt", cwd: {literal}).stdout"#
            ))
            .unwrap(),
        "normalized leading slash"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn spawn_rejects_invalid_working_directories_before_launch() {
    let mut manager = ConfigManager::new().unwrap();
    let empty = manager
        .eval(r#"Toyoterm.spawn("unused", cwd: "")"#)
        .unwrap_err();
    assert!(empty.message().contains("cwd cannot be empty"));
    let nul = manager
        .eval(r#"Toyoterm.spawn("unused", cwd: "bad\0cwd")"#)
        .unwrap_err();
    assert!(nul.message().contains("cwd contains a NUL byte"));
}

#[test]
fn clipboard_text_cannot_interpolate_ruby_source() {
    let mut manager = ConfigManager::new().unwrap();
    let text = r#"#{raise "clipboard interpolation ran"}"#;

    manager.set_clipboard_text(Some(text)).unwrap();

    assert_eq!(manager.eval("Toyoterm.clipboard.read").unwrap(), text);
}

#[test]
fn typed_clipboard_transfer_preserves_embedded_nul_bytes() {
    let mut manager = ConfigManager::new().unwrap();
    manager.set_clipboard_text(Some("left\0right")).unwrap();

    assert_eq!(
        manager
            .eval("Toyoterm.clipboard.read.bytes.join(',')")
            .unwrap(),
        "108,101,102,116,0,114,105,103,104,116"
    );
}

#[test]
fn typed_mruby_calls_preserve_ruby_exceptions() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .eval("def Toyoterm.__set_current_pane(id); raise ArgumentError, \"bad pane #{id}\"; end")
        .unwrap();
    let baseline_arena = manager.runtime.gc_stats().arena_index;

    let error = manager.set_current_pane(PaneId(23)).unwrap_err();
    assert_eq!(error.operation(), "set current pane");
    assert!(error.message().contains("bad pane 23"));
    assert_eq!(manager.runtime.gc_stats().arena_index, baseline_arena);
}

#[test]
fn reports_an_unavailable_clipboard_to_ruby() {
    let mut manager = ConfigManager::new().unwrap();
    let error = manager.eval("Toyoterm.clipboard.read").unwrap_err();
    assert!(error.message().contains("clipboard is unavailable"));
}

#[test]
fn ruby_callback_errors_roll_back_clipboard_writes() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
                Toyoterm.configure do |config|
                  config.keys.key("CTRL+C").run do
                    Toyoterm.clipboard.write("must not be copied")
                    raise "broken clipboard callback"
                  end
                end
                "#,
        )
        .unwrap();

    let error = manager.trigger_keybinding("CTRL+C", PaneId(4)).unwrap_err();
    assert!(error.message().contains("broken clipboard callback"));
    assert!(manager.drain_commands(PaneId(4)).unwrap().is_empty());
}

#[test]
fn ruby_callback_errors_roll_back_pane_badges() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
                Toyoterm.configure do |config|
                  config.keys.key("CTRL+B").run do |context|
                    context.pane.badge = "must not persist"
                    raise "broken badge callback"
                  end
                end
                "#,
        )
        .unwrap();

    let error = manager.trigger_keybinding("CTRL+B", PaneId(4)).unwrap_err();
    assert!(error.message().contains("broken badge callback"));
    assert!(manager.drain_commands(PaneId(4)).unwrap().is_empty());
    assert_eq!(manager.eval("Toyoterm.current_pane.badge").unwrap(), "");
}

#[test]
fn resolves_startup_commands_to_the_current_native_pane() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(r#"Toyoterm.current_pane.send_text("pwd\n")"#)
        .unwrap();

    assert_eq!(
        manager.drain_commands(PaneId(7)).unwrap(),
        vec![NativeCommand::Mux(Command::SendText {
            pane: PaneId(7),
            text: "pwd\n".into(),
        })]
    );
}

#[test]
fn invokes_only_matching_dynamic_keybindings() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
                $callback_count = 0
                Toyoterm.configure do |config|
                  config.keys.key("CTRL+SHIFT+H").run do |ctx|
                    $callback_count += 1
                    ctx.pane.send_text("echo from ruby\n")
                  end
                end
                "#,
        )
        .unwrap();

    assert!(!manager.trigger_keybinding("A", PaneId(9)).unwrap());
    assert_eq!(manager.eval("$callback_count").unwrap(), "0");
    assert!(
        manager
            .trigger_keybinding("ctrl+shift+h", PaneId(9))
            .unwrap()
    );
    assert_eq!(manager.eval("$callback_count").unwrap(), "1");
    assert_eq!(
        manager.drain_commands(PaneId(9)).unwrap(),
        vec![NativeCommand::Mux(Command::SendText {
            pane: PaneId(9),
            text: "echo from ruby\n".into(),
        })]
    );
}

#[test]
fn compiles_static_key_dsl_to_native_actions() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
                Toyoterm.configure do |config|
                  config.keys do
                    ctrl_shift("v").split(:right)
                    ctrl_shift("j").activate_pane(:down)
                    ctrl("t").new_tab
                    alt("q").close_pane
                    alt("F10").toggle_maximize
                    alt("F9").minimize_window
                    ctrl("F11").toggle_fullscreen
                    primary("p").close_pane
                    primary_shift("o").reload_config
                    ctrl_shift("r").reload_config
                    physical("KeyH", "CTRL").activate_pane(:left)
                    key("v").toggle_visual_mode
                    key("ESCAPE").end_visual_selection
                    key("h").move_visual_selection(:left)
                    key("0").move_visual_selection(:line_start)
                    key("$").move_visual_selection(:line_end)
                    key("y").yank_selection
                  end
                end
                "#,
        )
        .unwrap();

    assert_eq!(
        manager.native_action("CTRL+SHIFT+V"),
        Some(NativeAction::Split(SplitDirection::Right))
    );
    assert_eq!(
        manager.native_action("CTRL+SHIFT+J"),
        Some(NativeAction::ActivatePane(SplitDirection::Down))
    );
    assert_eq!(
        manager.native_action("CTRL+PHYSICAL:KEYH"),
        Some(NativeAction::ActivatePane(SplitDirection::Left))
    );
    assert_eq!(
        manager.native_action("V"),
        Some(NativeAction::ToggleVisualMode)
    );
    assert_eq!(
        manager.native_action("ESCAPE"),
        Some(NativeAction::EndVisualSelection)
    );
    assert_eq!(
        manager.native_action("H"),
        Some(NativeAction::MoveVisualSelection(
            toyoterm_api::SelectionMotion::Left
        ))
    );
    assert_eq!(
        manager.native_action("0"),
        Some(NativeAction::MoveVisualSelection(
            toyoterm_api::SelectionMotion::LineStart
        ))
    );
    assert_eq!(
        manager.native_action("$"),
        Some(NativeAction::MoveVisualSelection(
            toyoterm_api::SelectionMotion::LineEnd
        ))
    );
    assert_eq!(
        manager.native_action("Y"),
        Some(NativeAction::YankSelection)
    );
    assert_eq!(manager.native_action("CTRL+T"), Some(NativeAction::NewTab));
    assert_eq!(
        manager.native_action("ALT+F10"),
        Some(NativeAction::ToggleMaximize)
    );
    assert_eq!(
        manager.native_action("ALT+F9"),
        Some(NativeAction::MinimizeWindow)
    );
    assert_eq!(
        manager.native_action("CTRL+F11"),
        Some(NativeAction::ToggleFullscreen)
    );
    assert_eq!(
        manager.native_action("ALT+Q"),
        Some(NativeAction::ClosePane)
    );
    assert_eq!(
        manager.native_action("CTRL+SHIFT+R"),
        Some(NativeAction::ReloadConfig)
    );
    assert_eq!(
        manager.native_action(&format!("{}+P", platform_primary_modifier())),
        Some(NativeAction::ClosePane)
    );
    assert_eq!(
        manager.native_action(if cfg!(target_os = "macos") {
            "SHIFT+SUPER+O"
        } else {
            "CTRL+SHIFT+O"
        }),
        Some(NativeAction::ReloadConfig)
    );
}

#[test]
fn loads_leader_configuration_and_compiles_leader_actions() {
    let mut manager = ConfigManager::new().unwrap();
    let config = manager
        .reload(
            r#"
                Toyoterm.configure do |config|
                  config.leader key: "b", mods: "CTRL", timeout: 750
                  config.keys do
                    leader("v").split(:right)
                    leader("t").new_tab
                  end
                end
                "#,
        )
        .unwrap();

    assert_eq!(
        config.leader,
        Some(LeaderConfig {
            key: "CTRL+B".into(),
            timeout_ms: 750,
        })
    );
    assert_eq!(
        manager.native_action("LEADER+V"),
        Some(NativeAction::Split(SplitDirection::Right))
    );
    assert_eq!(
        manager.native_action("LEADER+T"),
        Some(NativeAction::NewTab)
    );
}

#[test]
fn rejects_invalid_leader_timeout() {
    let mut manager = ConfigManager::new().unwrap();
    let error = manager
        .reload("Toyoterm.configure { |config| config.leader key: 'b', timeout: 0 }")
        .unwrap_err();
    assert!(error.message().contains("leader timeout must be positive"));
}

#[test]
fn rejects_duplicate_static_and_dynamic_bindings() {
    let mut manager = ConfigManager::new().unwrap();
    let error = manager
        .reload(
            r#"
                Toyoterm.configure do |config|
                  config.keys { ctrl("x").new_tab }
                  config.keys.key("CTRL+X").run { }
                end
                "#,
        )
        .unwrap_err();
    assert!(error.message().contains("duplicate key binding"));
}

#[test]
fn ruby_keybinding_errors_leave_the_runtime_usable() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
                Toyoterm.configure do |config|
                  config.keys.key("CTRL+E").run do |ctx|
                    ctx.pane.send_text("must not run\n")
                    raise "broken callback"
                  end
                end
                "#,
        )
        .unwrap();

    let error = manager.trigger_keybinding("CTRL+E", PaneId(4)).unwrap_err();
    assert_eq!(error.operation(), "evaluate mruby");
    assert!(error.message().contains("broken callback"));
    assert_eq!(manager.eval("6 * 7").unwrap(), "42");
    assert!(manager.drain_commands(PaneId(4)).unwrap().is_empty());
}

#[test]
fn exposes_reload_requests_from_ruby_keybindings() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
                Toyoterm.configure do |config|
                  config.keys.key("CTRL+SHIFT+R").run { Toyoterm.reload_config }
                end
                "#,
        )
        .unwrap();

    assert!(
        manager
            .trigger_keybinding("CTRL+SHIFT+R", PaneId(4))
            .unwrap()
    );
    assert_eq!(
        manager.drain_commands(PaneId(4)).unwrap(),
        vec![NativeCommand::ReloadConfig]
    );
    assert!(manager.drain_commands(PaneId(4)).unwrap().is_empty());
}

#[test]
fn emits_registered_events_with_the_current_pane() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
                $event_count = 0
                Toyoterm.on :app_started do |event|
                  $event_count += 1
                  $event_name = event.name
                  $event_pane = event.pane.id
                  event.pane.send_text("echo app started\n")
                end
                "#,
        )
        .unwrap();

    assert!(
        !manager
            .emit_event(ScriptEventKind::ConfigReloaded, PaneId(12))
            .unwrap()
    );
    assert_eq!(manager.eval("$event_count").unwrap(), "0");
    assert!(
        manager
            .emit_event(ScriptEventKind::AppStarted, PaneId(12))
            .unwrap()
    );
    assert_eq!(manager.eval("$event_count").unwrap(), "1");
    assert_eq!(manager.eval("$event_name").unwrap(), "app_started");
    assert_eq!(manager.eval("$event_pane").unwrap(), "12");
    assert_eq!(
        manager.drain_commands(PaneId(12)).unwrap(),
        vec![NativeCommand::Mux(Command::SendText {
            pane: PaneId(12),
            text: "echo app started\n".into(),
        })]
    );
}

#[test]
fn emits_typed_native_event_payloads() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
                Toyoterm.on :title_changed do |event|
                  $native_event = [
                    event.name, event.workspace.id, event.window.id,
                    event.tab.id, event.pane.id, event.title, event.cwd,
                    event.exit_status
                  ]
                end
                "#,
        )
        .unwrap();
    let event = RubyEvent {
        kind: ScriptEventKind::TitleChanged,
        workspace: Some(WorkspaceId(1)),
        window: Some(WindowId(2)),
        tab: Some(TabId(3)),
        pane: Some(PaneId(4)),
        title: Some("server \"one\"".into()),
        cwd: Some("/srv/日本語".into()),
        exit_status: Some(17),
    };

    assert!(manager.emit_native_event(&event).unwrap());
    assert_eq!(
        manager.eval("$native_event[0, 5].inspect").unwrap(),
        "[:title_changed, 1, 2, 3, 4]"
    );
    assert_eq!(manager.eval("$native_event[5]").unwrap(), "server \"one\"");
    assert_eq!(manager.eval("$native_event[6]").unwrap(), "/srv/日本語");
    assert_eq!(manager.eval("$native_event[7]").unwrap(), "17");
}

#[test]
fn unregistered_native_events_do_not_call_the_ruby_vm() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .eval("def Toyoterm.__emit_native_event(*args); raise 'VM must not be called'; end")
        .unwrap();

    assert!(
        !manager
            .emit_native_event(&RubyEvent::new(ScriptEventKind::PaneCreated))
            .unwrap()
    );
}

#[test]
fn ruby_event_errors_are_isolated_per_handler() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
                Toyoterm.on :config_reloaded do |event|
                  event.pane.send_text("must not run\n")
                  Toyoterm.log(:info, "must not survive")
                  raise "broken event"
                end
                Toyoterm.on :config_reloaded do |event|
                  event.pane.send_text("later handler ran\n")
                end
                "#,
        )
        .unwrap();

    assert!(
        manager
            .emit_event(ScriptEventKind::ConfigReloaded, PaneId(3))
            .unwrap()
    );
    assert_eq!(
        manager.drain_commands(PaneId(3)).unwrap(),
        vec![NativeCommand::Mux(Command::SendText {
            pane: PaneId(3),
            text: "later handler ran\n".into(),
        })]
    );
    assert_eq!(
        manager.drain_logs().unwrap(),
        vec![ScriptLog {
            level: "error".into(),
            message: "event handler config_reloaded failed: broken event".into(),
        }]
    );
    assert_eq!(manager.eval("21 * 2").unwrap(), "42");
}

#[test]
fn user_commands_are_listed_and_dispatch_native_commands() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
            Toyoterm.command :git_status do |ctx|
              ctx.pane.send_text("git status\n")
            end
            Toyoterm.configure do |config|
              config.keys { leader("g").command(:git_status) }
            end
        "#,
        )
        .unwrap();

    assert_eq!(
        manager.user_command_names().collect::<Vec<_>>(),
        vec!["git_status"]
    );
    assert_eq!(
        manager.native_action("LEADER+G"),
        Some(NativeAction::UserCommand("git_status".into()))
    );
    assert!(
        manager
            .trigger_user_command("git_status", PaneId(8))
            .unwrap()
    );
    assert_eq!(
        manager.drain_commands(PaneId(8)).unwrap(),
        vec![NativeCommand::Mux(Command::SendText {
            pane: PaneId(8),
            text: "git status\n".into()
        })]
    );
}

#[test]
fn user_command_validation_and_callback_failures_are_isolated() {
    let mut manager = ConfigManager::new().unwrap();
    let duplicate = manager
        .reload(
            r#"
            Toyoterm.command(:same) {}
            Toyoterm.command(:same) {}
        "#,
        )
        .unwrap_err();
    assert!(duplicate.message().contains("duplicate user command"));

    manager
        .reload(
            r#"
            Toyoterm.command :broken do |ctx|
              ctx.pane.send_text("must not run\n")
              raise "broken command"
            end
        "#,
        )
        .unwrap();
    let undefined = manager
        .trigger_user_command("missing", PaneId(2))
        .unwrap_err();
    assert!(undefined.message().contains("undefined user command"));
    let broken = manager
        .trigger_user_command("broken", PaneId(2))
        .unwrap_err();
    assert!(broken.message().contains("broken command"));
    assert!(manager.drain_commands(PaneId(2)).unwrap().is_empty());
    assert_eq!(manager.eval("6 * 7").unwrap(), "42");
}

#[test]
fn interactive_evaluation_returns_inspect_output() {
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(manager.eval_inspect("[1, 'two']").unwrap(), "[1, \"two\"]");
    assert!(
        manager
            .eval_inspect("Toyoterm.current_pane.send_text('leak'); raise 'nope'")
            .is_err()
    );
    assert!(manager.drain_commands(PaneId(1)).unwrap().is_empty());
}

#[test]
fn opacity_bindings_saturate_and_reverse_at_both_limits() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
            Toyoterm.configure do |config|
              config.window.opacity = 0.9
              config.keys.key("CTRL+[").run do
                config.window.opacity -= 0.1
              end
              config.keys.key("CTRL+]").run do
                config.window.opacity += 0.1
              end
            end
            "#,
        )
        .unwrap();
    let context = script_test_context();
    // Repeated overshoots must not accumulate in the captured Ruby config.
    for (key, count, expected) in [
        ("CTRL+]", 1, 1.0),
        ("CTRL+[", 1, 0.9),
        ("CTRL+]", 20, 1.0),
        ("CTRL+[", 1, 0.9),
        ("CTRL+[", 20, 0.0),
        ("CTRL+]", 1, 0.1),
        ("CTRL+]", 20, 1.0),
        ("CTRL+[", 1, 0.9),
    ] {
        for _ in 0..count {
            run_script_request(
                &mut manager,
                &context,
                &ScriptInvocation::KeyBinding {
                    key: key.into(),
                    pane: context.model.current_pane,
                },
            )
            .unwrap();
        }
        assert!((manager.config().window.opacity - expected).abs() < 0.00001);
        let ruby_opacity: f32 = manager
            .eval("Toyoterm.__config.window.opacity")
            .unwrap()
            .parse()
            .unwrap();
        assert!((ruby_opacity - expected).abs() < 0.00001);
    }
}

#[test]
fn window_image_section_returns_the_same_object_and_rejects_old_api() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .eval(
            r#"
      Toyoterm.configure do |config|
        image = config.window.image
        raise "wrong defaults" unless image.path.nil? && image.opacity == 1.0
        section = config.window.image do |value|
          raise "wrong image" unless value.equal?(image)
          value.opacity = 0.25
          nil
        end
        raise "wrong return" unless section.equal?(image)
        raise "wrong opacity" unless config.window.image.opacity == 0.25
        raise "window opacity changed" unless config.window.opacity == 1.0
      end
    "#,
        )
        .unwrap();
    for expression in [
        "background_image",
        "background_image = nil",
        "background_image_opacity",
        "background_image_opacity = 0.5",
        "image = nil",
    ] {
        assert!(
            manager
                .eval(&format!(
                    "Toyoterm.configure {{ |c| c.window.{expression} }}"
                ))
                .unwrap_err()
                .message()
                .contains("NoMethodError")
        );
    }
}

#[test]
fn background_images_reload_cache_clear_and_rollback() {
    let directory = temporary_test_directory("background-image");
    let image_path = directory.join("wallpaper.png");
    std::fs::write(
        &image_path,
        include_bytes!("../tests/fixtures/background.png"),
    )
    .unwrap();
    let config_path = directory.join("config.rb");
    std::fs::write(&config_path, "Toyoterm.configure { |c| c.window.image { |image| image.path = 'wallpaper.png'; image.opacity = 0.4 } }").unwrap();
    let mut manager = ConfigManager::load_startup(Some(&config_path)).unwrap();
    let original = manager.config().window.background_image.clone().unwrap();
    assert_eq!(original.path, image_path);
    assert_eq!((original.width, original.height), (2, 1));
    assert_eq!(&*original.rgba, &[255, 0, 0, 255, 0, 0, 255, 128]);
    assert_eq!(manager.config().window.background_image_opacity, 0.4);

    // Unrelated live settings reuse pixels, even if the file has been removed.
    std::fs::remove_file(&image_path).unwrap();
    let eval = |manager: &mut ConfigManager, source: &str| {
        run_script_request(
            manager,
            &script_test_context(),
            &ScriptInvocation::Eval(source.into()),
        )
    };
    eval(
        &mut manager,
        "Toyoterm.configure { |c| c.window.opacity = 0.7 }",
    )
    .unwrap();
    assert!(std::sync::Arc::ptr_eq(
        &original,
        manager.config().window.background_image.as_ref().unwrap()
    ));
    // File reload always rereads the image and preserves the active config on failure.
    assert!(manager.reload_file().is_err());
    assert_eq!(manager.config().window.opacity, 0.7);
    for source in [
        "Toyoterm.configure { |c| c.window.image.path = 'missing.png' }",
        "Toyoterm.configure { |c| c.window.image.path = '' }",
        "Toyoterm.configure { |c| c.window.image.path = 123 }",
        "Toyoterm.configure { |c| c.window.image.path = \"a\\0b\" }",
        "Toyoterm.configure { |c| c.window.image.opacity = -0.1 }",
        "Toyoterm.configure { |c| c.window.image.opacity = 1.1 }",
        "Toyoterm.configure { |c| c.window.image.opacity = 0.0/0.0 }",
        "Toyoterm.configure { |c| c.window.image.opacity = '0.5' }",
        "Toyoterm.configure { |c| c.window.image.path = nil; c.window.image.opacity = 0; c.font.size = 0 }",
        "Toyoterm.configure { |c| c.window.image { |image| image.path = nil; image.opacity = 0; raise 'cancel image' } }",
    ] {
        assert!(eval(&mut manager, source).is_err(), "{source}");
        assert_eq!(
            manager.config().window.background_image.as_ref().unwrap(),
            &original
        );
        assert_eq!(manager.config().window.background_image_opacity, 0.4);
        assert_eq!(
            manager.eval("Toyoterm.__config.window.image.path").unwrap(),
            "wallpaper.png"
        );
    }
    eval(
        &mut manager,
        "Toyoterm.configure { |c| c.window.image.path = nil }",
    )
    .unwrap();
    assert!(manager.config().window.background_image.is_none());
    std::fs::write(
        &image_path,
        include_bytes!("../tests/fixtures/background.png"),
    )
    .unwrap();
    manager.reload_file().unwrap();
    assert!(manager.config().window.background_image.is_some());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn opacity_validation_preserves_transactions() {
    let mut manager = ConfigManager::new().unwrap();
    for (value, expected) in [("2", 1.0), ("-1", 0.0), ("0.8", 0.8)] {
        manager
            .reload(&format!(
                "Toyoterm.configure {{ |config| config.window.opacity = {value} }}"
            ))
            .unwrap();
        assert_eq!(manager.config().window.opacity, expected);
    }
    for value in [
        "nil",
        "'0.5'",
        "true",
        "0.0 / 0.0",
        "1.0 / 0.0",
        "-1.0 / 0.0",
    ] {
        assert!(
            run_script_request(
                &mut manager,
                &script_test_context(),
                &ScriptInvocation::Eval(format!(
                    "Toyoterm.configure {{ |c| c.window.opacity = 0.5; c.window.opacity = {value} }}"
                )),
            )
            .is_err()
        );
        assert_eq!(manager.config().window.opacity, 0.8);
        assert_eq!(
            manager.eval("Toyoterm.__config.window.opacity").unwrap(),
            "0.8"
        );
    }
    assert!(
        run_script_request(
            &mut manager,
            &script_test_context(),
            &ScriptInvocation::Eval(
                "Toyoterm.configure { |c| c.window.opacity = 2; c.font.size = 0 }".into()
            ),
        )
        .is_err()
    );
    assert_eq!(manager.config().window.opacity, 0.8);
    assert_eq!(
        manager.eval("Toyoterm.__config.window.opacity").unwrap(),
        "0.8"
    );
}

#[test]
fn interactive_config_mutations_return_a_new_native_snapshot() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
                Toyoterm.configure do |config|
                  config.font.family = "Old Font"
                  config.font.size = 14
                  config.window.opacity = 1.0
                end
            "#,
        )
        .unwrap();

    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::Eval(
            r#"Toyoterm.configure { |config| config.font.family = "New Font"; config.font.size = 18; config.window.opacity = 0.8 }"#
                .into(),
        ),
    )
    .unwrap();

    let config = result.snapshot.unwrap().config;
    assert_eq!(config.font.family, "New Font");
    assert_eq!(config.font.size, 18.0);
    assert_eq!(config.window.opacity, 0.8);
    assert_eq!(manager.config().font.family, "New Font");
}

#[test]
fn select_queues_a_native_overlay_and_resumes_its_callback() {
    let mut manager = ConfigManager::new().unwrap();
    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::Eval(
            r#"Toyoterm.select(title: "Theme", items: ["Ayu", "Solarized Dark"]) { |choice, context| context.pane.badge = choice }"#
                .into(),
        ),
    )
    .unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::OpenSelector {
            id: 1,
            title: "Theme".into(),
            items: vec!["Ayu".into(), "Solarized Dark".into()],
        }]
    );

    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::SelectCallback {
            id: 1,
            selection: Some("Solarized Dark".into()),
        },
    )
    .unwrap();
    assert_eq!(
        result.commands,
        vec![NativeCommand::SetPaneBadge {
            pane: PaneId(4),
            badge: Some("Solarized Dark".into()),
        }]
    );
}

#[test]
fn select_callback_can_apply_a_plugin_theme() {
    let directory = temporary_test_directory("theme-selector");
    let plugin = directory.join("themes.rb");
    std::fs::write(
        &plugin,
        r##"
            Toyoterm::Plugin.define "selectable-themes" do |plugin|
              plugin.version = "0.1.0"
              plugin.theme("night") { |colors| colors.background = "#101010" }
              plugin.theme("day") { |colors| colors.background = "#fafafa" }
              plugin.command :choose_theme do
                Toyoterm.select(title: "Theme", items: Toyoterm.themes) do |theme|
                  Toyoterm.configure { |config| config.theme = theme } unless theme.nil?
                end
              end
            end
        "##,
    )
    .unwrap();
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(&format!(
            "Toyoterm.plugin({})",
            ruby_string_literal(&plugin.display().to_string())
        ))
        .unwrap();
    let opened = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::UserCommand {
            name: "choose_theme".into(),
            pane: PaneId(4),
        },
    )
    .unwrap();
    let id = match &opened.commands[0] {
        NativeCommand::OpenSelector { id, items, .. } => {
            assert_eq!(items, &["night", "day"]);
            *id
        }
        command => panic!("expected selector command, got {command:?}"),
    };
    let selected = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::SelectCallback {
            id,
            selection: Some("day".into()),
        },
    )
    .unwrap();
    assert_eq!(
        selected.snapshot.unwrap().config.colors.background,
        "#fafafa"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn select_cancel_passes_nil_and_releases_the_pending_slot() {
    let mut manager = ConfigManager::new().unwrap();
    let open = |manager: &mut ConfigManager| {
        run_script_request(
            manager,
            &script_test_context(),
            &ScriptInvocation::Eval(
                "Toyoterm.select(items: ['one']) { |choice| $selection = choice }".into(),
            ),
        )
    };
    let first = open(&mut manager).unwrap();
    let id = match &first.commands[0] {
        NativeCommand::OpenSelector { id, .. } => *id,
        command => panic!("expected selector command, got {command:?}"),
    };
    run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::SelectCallback {
            id,
            selection: None,
        },
    )
    .unwrap();
    assert_eq!(manager.eval("$selection.nil?").unwrap(), "true");
    assert!(open(&mut manager).is_ok());
}

#[test]
fn select_validates_input_and_rolls_back_pending_callbacks() {
    let mut manager = ConfigManager::new().unwrap();
    for source in [
        "Toyoterm.select(items: ['one'])",
        "Toyoterm.select(items: []) {}",
        "Toyoterm.select(items: 'one') {}",
        "Toyoterm.select(items: ['']) {}",
        "Toyoterm.select(items: [1]) {}",
        "Toyoterm.select(title: \"bad\\nname\", items: ['one']) {}",
        "Toyoterm.select(title: 'x' * 257, items: ['one']) {}",
        "Toyoterm.select(items: ['x' * 4097]) {}",
        "Toyoterm.select(items: Array.new(4097, 'x')) {}",
        "Toyoterm.select(items: Array.new(1025, 'x' * 4096)) {}",
    ] {
        assert!(
            run_script_request(
                &mut manager,
                &script_test_context(),
                &ScriptInvocation::Eval(source.into()),
            )
            .is_err(),
            "{source}"
        );
    }
    assert!(
        run_script_request(
            &mut manager,
            &script_test_context(),
            &ScriptInvocation::Eval(
                "Toyoterm.select(items: ['one']) {}; raise 'rollback selector'".into(),
            ),
        )
        .is_err()
    );
    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::Eval("Toyoterm.select(items: ['two']) {}".into()),
    )
    .unwrap();
    let id = match result.commands[0] {
        NativeCommand::OpenSelector { id, .. } => id,
        ref command => panic!("expected selector command, got {command:?}"),
    };
    let error = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::SelectCallback {
            id,
            selection: Some("not requested".into()),
        },
    )
    .unwrap_err();
    assert!(error.message().contains("not one of the requested items"));
    assert!(
        run_script_request(
            &mut manager,
            &script_test_context(),
            &ScriptInvocation::Eval("Toyoterm.select(items: ['three']) {}".into()),
        )
        .is_ok()
    );
}

#[test]
fn invalid_interactive_config_mutations_are_rolled_back() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload("Toyoterm.configure { |config| config.font.size = 14 }")
        .unwrap();

    let error = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::Eval(
            "Toyoterm.configure { |config| config.font.size = 0; config.window.opacity = 2 }"
                .into(),
        ),
    )
    .unwrap_err();

    assert!(error.message().contains("font size must be positive"));
    assert_eq!(manager.config().font.size, 14.0);
    assert_eq!(manager.eval("Toyoterm.__config.font.size").unwrap(), "14");

    let error = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::Eval(
            "Toyoterm.async('echo'); Toyoterm.log(:info, 'discard'); Toyoterm.configure { |config| config.theme = :missing }"
                .into(),
        ),
    )
    .unwrap_err();
    assert!(error.message().contains("unknown theme"));
    assert!(manager.drain_async_requests().unwrap().is_empty());
    assert!(manager.drain_logs().unwrap().is_empty());
}

#[test]
fn resolves_config_paths_in_priority_order() {
    let explicit = Path::new("custom.rb");
    let environment = std::ffi::OsStr::new("environment.rb");
    let default_path = PathBuf::from("/users/toyo/.config/toyoterm/config.rb");
    assert_eq!(
        resolve_config_path(
            Some(explicit),
            Some(environment),
            Some(default_path.clone())
        ),
        Some(explicit.to_owned())
    );
    assert_eq!(
        resolve_config_path(None, Some(environment), Some(default_path.clone())),
        Some(PathBuf::from("environment.rb"))
    );
    assert_eq!(
        resolve_config_path(None, None, Some(default_path.clone())),
        Some(default_path)
    );
    assert_eq!(resolve_config_path(None, None, None), None);
}

#[test]
fn zoomed_pane_border_supports_reload_and_atomic_runtime_updates() {
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(manager.config().colors.zoomed_pane_border, "#ffbe3a");
    manager
        .reload(r##"Toyoterm.configure { |c| c.colors.zoomed_pane_border = "#123456" }"##)
        .unwrap();
    assert_eq!(manager.config().colors.zoomed_pane_border, "#123456");
    let result = run_script_request(
        &mut manager,
        &script_test_context(),
        &ScriptInvocation::Eval(
            r##"Toyoterm.configure { |c| c.colors.zoomed_pane_border = "#abcdef" }"##.into(),
        ),
    )
    .unwrap();
    assert_eq!(
        result.snapshot.unwrap().config.colors.zoomed_pane_border,
        "#abcdef"
    );
    for source in [
        r#"Toyoterm.configure { |c| c.colors.zoomed_pane_border = "red" }"#,
        r##"Toyoterm.configure { |c| c.colors.zoomed_pane_border = "#112233"; c.font.size = 0 }"##,
    ] {
        assert!(
            run_script_request(
                &mut manager,
                &script_test_context(),
                &ScriptInvocation::Eval(source.into())
            )
            .is_err()
        );
        assert_eq!(manager.config().colors.zoomed_pane_border, "#abcdef");
        assert_eq!(
            manager
                .eval("Toyoterm.__config.colors.zoomed_pane_border")
                .unwrap(),
            "#abcdef"
        );
    }
    assert!(
        manager
            .reload(r#"Toyoterm.configure { |c| c.colors.zoomed_pane_border = "invalid" }"#)
            .is_err()
    );
    assert_eq!(manager.config().colors.zoomed_pane_border, "#abcdef");
}

#[test]
fn async_api_validates_arguments() {
    let mut manager = ConfigManager::new().unwrap();

    // A callback is optional; the returned handle exposes the result.
    manager.eval("$task = Toyoterm.async('ping')").unwrap();
    assert_eq!(manager.eval("$task.class").unwrap(), "Toyoterm::AsyncTask");
    assert_eq!(manager.eval("$task.pending?").unwrap(), "true");
    assert_eq!(manager.eval("$task.success?").unwrap(), "false");

    // Empty program rejected
    assert!(manager.eval("Toyoterm.async('') { |r| }").is_err());

    // NUL bytes rejected
    assert!(
        manager
            .eval("Toyoterm.async(\"p\\x00ing\") { |r| }")
            .is_err()
    );
    assert!(
        manager
            .eval("Toyoterm.async('ping', \"1.1.1.\\x00\") { |r| }")
            .is_err()
    );
    assert!(
        manager
            .eval("Toyoterm.async('ping', cwd: \"/tmp/\\x00dir\") { |r| }")
            .is_err()
    );

    // Empty cwd rejected
    assert!(
        manager
            .eval("Toyoterm.async('ping', cwd: '') { |r| }")
            .is_err()
    );

    // Text values are not implicitly stringified.
    assert!(manager.eval("Toyoterm.async(1)").is_err());
    assert!(
        manager
            .eval("Toyoterm::BarSection.new(' ').add_async('echo', initial: 1)")
            .is_err()
    );
}

#[test]
fn async_task_handle_exposes_result_without_global_state() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .eval(
            r#"
            task = Toyoterm.async("printf", "hello")
            $task_handle = task
            $before = [task.pending?, task.complete?]
            "#,
        )
        .unwrap();

    let requests = manager.drain_async_requests().unwrap();
    assert_eq!(requests.len(), 1);
    manager
        .invoke_async_callback(requests[0].id, b"hello", b"", 0)
        .unwrap();

    assert_eq!(manager.eval("$before[0]").unwrap(), "true");
    assert_eq!(manager.eval("$task_handle.complete?").unwrap(), "true");
    assert_eq!(manager.eval("$task_handle.result.stdout").unwrap(), "hello");
    assert_eq!(manager.eval("$task_handle.success?").unwrap(), "true");
}

#[test]
fn async_api_queues_and_drains_requests() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .eval(
            r#"
            $task1 = Toyoterm.async("curl", "-s", "https://wttr.in/?format=1") { |res| $res1 = res }
            $task2 = Toyoterm.async_spawn("ping", "-c", "1", "1.1.1.1", cwd: "/tmp") { |res| $res2 = res }
            "#,
        )
        .unwrap();

    let requests = manager.drain_async_requests().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].id, 1);
    assert_eq!(requests[0].program, "curl");
    assert_eq!(requests[0].args, vec!["-s", "https://wttr.in/?format=1"]);
    assert_eq!(requests[0].cwd, None);

    assert_eq!(requests[1].id, 2);
    assert_eq!(requests[1].program, "ping");
    assert_eq!(requests[1].args, vec!["-c", "1", "1.1.1.1"]);
    assert_eq!(requests[1].cwd.as_deref(), Some("/tmp"));

    // Draining again returns empty
    assert!(manager.drain_async_requests().unwrap().is_empty());
}

#[test]
fn async_api_invokes_callback_and_applies_results() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .eval(
            r#"
            Toyoterm.async("curl", "https://example.com") do |res|
              $received_stdout = res.stdout
              $received_stderr = res.stderr
              $received_status = res.exit_status
              $received_success = res.success?
              Toyoterm.current_pane.send_text("fetched: #{res.stdout}\n")
            end
            "#,
        )
        .unwrap();

    let requests = manager.drain_async_requests().unwrap();
    assert_eq!(requests.len(), 1);
    let task_id = requests[0].id;

    manager
        .invoke_async_callback(task_id, b"hello world", b"", 0)
        .unwrap();

    assert_eq!(manager.eval("$received_stdout").unwrap(), "hello world");
    assert_eq!(manager.eval("$received_stderr").unwrap(), "");
    assert_eq!(manager.eval("$received_status").unwrap(), "0");
    assert_eq!(manager.eval("$received_success").unwrap(), "true");

    let commands = manager.drain_commands(PaneId(42)).unwrap();
    assert_eq!(
        commands,
        vec![NativeCommand::Mux(Command::SendText {
            pane: PaneId(42),
            text: "fetched: hello world\n".into(),
        })]
    );
}

#[test]
fn async_api_callback_exception_rolls_back_mutations() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .eval(
            r#"
            Toyoterm.async("broken") do |res|
              Toyoterm.current_pane.send_text("must not be sent\n")
              raise "callback exploded"
            end
            "#,
        )
        .unwrap();

    let requests = manager.drain_async_requests().unwrap();
    assert_eq!(requests.len(), 1);

    let error = manager
        .invoke_async_callback(requests[0].id, b"", b"failed", 1)
        .unwrap_err();
    assert!(error.to_string().contains("callback exploded"));

    // The queued send_text must have been rolled back
    assert!(manager.drain_commands(PaneId(1)).unwrap().is_empty());
}

#[test]
fn async_api_works_inside_window_bar_widgets() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
            $status_text = "initial"
            Toyoterm.configure do |config|
              config.window.bar :bottom, interval: 1.0 do |bar|
                bar.section(:left) do |section|
                  section.add do
                    Toyoterm.async("ping", "1.1.1.1") do |res|
                      $status_text = "updated: #{res.stdout}"
                    end
                    $status_text
                  end
                end
              end
            end
            "#,
        )
        .unwrap();

    let items = manager.render_bar(StatusBarPosition::Bottom).unwrap();
    assert_eq!(items[0].text, "initial");

    // The async request queued during bar rendering should be retained and drained
    let requests = manager.drain_async_requests().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].program, "ping");

    // Now simulate completion of that async request
    manager
        .invoke_async_callback(requests[0].id, b"pong", b"", 0)
        .unwrap();

    // Next bar render displays the updated value
    let items = manager.render_bar(StatusBarPosition::Bottom).unwrap();
    assert_eq!(items[0].text, "updated: pong");
}

#[test]
fn async_task_can_be_retained_in_bar_closure_without_global_state() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
            Toyoterm.configure do |config|
              config.window.bar :bottom, interval: 1.0 do |bar|
                task = nil
                bar.section(:left) do |section|
                  section.add do
                    task = Toyoterm.async("printf", "ready") if task.nil?
                    task.pending? ? "waiting" : task.result.stdout
                  end
                end
              end
            end
            "#,
        )
        .unwrap();

    assert_eq!(
        manager.render_bar(StatusBarPosition::Bottom).unwrap()[0].text,
        "waiting"
    );
    let requests = manager.drain_async_requests().unwrap();
    manager
        .invoke_async_callback(requests[0].id, b"ready", b"", 0)
        .unwrap();
    assert_eq!(
        manager.render_bar(StatusBarPosition::Bottom).unwrap()[0].text,
        "ready"
    );
    assert!(manager.drain_async_requests().unwrap().is_empty());
}

#[test]
fn async_bar_section_renders_multiple_tasks_with_a_separator() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
            Toyoterm.configure do |config|
              config.window.bar :bottom, interval: 1.0 do |bar|
                bar.section(:right, separator: " | ") do |section|
                  section.add_async("printf", "clock", initial: "clock...") do |result|
                    result.success? ? result.stdout : ""
                  end
                  section.add_async("printf", "branch", initial: "branch...") do |result|
                    result.success? ? result.stdout : ""
                  end
                end
              end
            end
            "#,
        )
        .unwrap();

    let items = manager.render_bar(StatusBarPosition::Bottom).unwrap();
    assert_eq!(items[0].text, "clock... | branch...");
    let requests = manager.drain_async_requests().unwrap();
    assert_eq!(requests.len(), 2);

    manager
        .invoke_async_callback(requests[0].id, b"12:00", b"", 0)
        .unwrap();
    assert_eq!(
        manager.render_bar(StatusBarPosition::Bottom).unwrap()[0].text,
        "12:00 | branch..."
    );

    manager
        .invoke_async_callback(requests[1].id, b"main", b"", 0)
        .unwrap();
    assert_eq!(
        manager.render_bar(StatusBarPosition::Bottom).unwrap()[0].text,
        "12:00 | main"
    );
    assert!(manager.drain_async_requests().unwrap().is_empty());
}

#[test]
fn bar_section_mixes_synchronous_and_asynchronous_widgets() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
            Toyoterm.configure do |config|
              config.window.bar :top, interval: 1.0 do |bar|
                bar.section(:right, separator: " | ") do |section|
                  updates = 0
                  section.add_async("printf", "clock", initial: "clock...") do |result|
                    result.success? ? result.stdout : ""
                  end
                  section.add("host")
                  section.add do |_context|
                    updates += 1
                    "battery#{updates}"
                  end
                  section.add("")
                  section.add(nil)
                end
              end
            end
            "#,
        )
        .unwrap();

    let items = manager.render_bar(StatusBarPosition::Top).unwrap();
    assert_eq!(items[0].text, "clock... | host | battery1");
    let requests = manager.drain_async_requests().unwrap();
    assert_eq!(requests.len(), 1);

    manager
        .invoke_async_callback(requests[0].id, b"12:00", b"", 0)
        .unwrap();
    assert_eq!(
        manager.render_bar(StatusBarPosition::Top).unwrap()[0].text,
        "12:00 | host | battery2"
    );
}

#[test]
fn bar_section_add_rejects_a_value_and_block_together() {
    let mut manager = ConfigManager::new().unwrap();
    let error = manager
        .eval("Toyoterm::BarSection.new(' ').add('value') { 'block' }")
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("bar section widget accepts either a value or a block, not both")
    );
}

#[test]
fn bar_section_validates_its_block_and_separator() {
    let mut manager = ConfigManager::new().unwrap();

    let missing_block = manager
        .eval("Toyoterm::BarConfig.new.section(:left)")
        .unwrap_err();
    assert!(
        missing_block
            .to_string()
            .contains("bar section requires a block")
    );

    let invalid_separator = manager
        .eval("Toyoterm::BarConfig.new.section(:left, separator: 1) { |_| }")
        .unwrap_err();
    assert!(
        invalid_separator
            .to_string()
            .contains("bar section separator must be a String")
    );
}
