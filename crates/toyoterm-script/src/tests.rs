use super::*;
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
            }],
            panes: vec![RubyPane {
                id: PaneId(4),
                title: "Pane 4".into(),
                cwd: None,
                pid: None,
                command_running: false,
                last_exit_status: None,
                screen_text: "prompt>".into(),
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
fn status_dsl_uses_typed_context_and_discards_commands() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
                Toyoterm.status(interval: 0.25) do |ctx|
                  ctx.pane.send_text("must not run")
                  [ctx.workspace.name, ctx.tab.title, ctx.pane.title].join(" | ")
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
        manager.config().status_interval,
        Some(Duration::from_millis(250))
    );
    assert_eq!(
        manager.render_status().unwrap(),
        "Workspace 1 | Tab 3 | Pane 4"
    );
    assert!(manager.drain_commands(PaneId(4)).unwrap().is_empty());
}

#[test]
fn status_interval_defaults_to_one_second_and_rejects_values_below_100ms() {
    let mut manager = ConfigManager::new().unwrap();
    manager.reload("Toyoterm.status { 'ready' }").unwrap();
    assert_eq!(
        manager.config().status_interval,
        Some(Duration::from_secs(1))
    );

    let error = manager
        .reload("Toyoterm.status(interval: 0.099) { 'too fast' }")
        .unwrap_err();
    assert!(error.message().contains("at least 0.1 seconds"));
    assert_eq!(manager.render_status().unwrap(), "ready");
}

#[test]
fn evaluates_ruby_in_a_persistent_vm() {
    let mut runtime = MrubyRuntime::new().unwrap();
    assert_eq!(runtime.eval("$value = 6 * 7").unwrap(), "42");
    assert_eq!(runtime.eval("$value + 1").unwrap(), "43");
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
              plugin.requires = ">= 0.1.0, < 0.2.0"
              plugin.command(:git_root) { |ctx| ctx.pane.send_text("git root\n") }
              plugin.on(:bell) { |event| event.pane.badge = "bell" }
              plugin.bind("CTRL+G") { |ctx| ctx.pane.send_text("git status\n") }
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
            requires: ">= 0.1.0, < 0.2.0".into(),
            path: plugin.clone(),
        }]
    );
    assert!(loaded.user_command_names.contains("git_root"));
    assert!(loaded.event_names.contains("bell"));
    assert!(loaded.keybindings.contains("CTRL+G"));
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
              plugin.requires = ">= 1.0.0"
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

    assert_eq!(error.operation(), "validate config");
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
            }],
            panes: vec![RubyPane {
                id: PaneId(40),
                title: "shell".into(),
                cwd: Some("/srv/app".into()),
                pid: Some(1234),
                command_running: true,
                last_exit_status: Some(17),
                screen_text: "build started\ncompiling toyoterm".into(),
            }],
        })
        .unwrap();

    assert_eq!(
        manager.eval("Toyoterm.current_workspace.name").unwrap(),
        "backend"
    );
    assert_eq!(
        manager.eval("Toyoterm.workspace('backend').id").unwrap(),
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
        manager.eval("Toyoterm.current_tab.panes[0].title").unwrap(),
        "shell"
    );
    assert_eq!(
        manager.eval("Toyoterm.current_pane.cwd").unwrap(),
        "/srv/app"
    );
    assert_eq!(manager.eval("Toyoterm.current_pane.pid").unwrap(), "1234");
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
    assert_eq!(manager.eval("Toyoterm.workspace('missing')").unwrap(), "");

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
                      Toyoterm.current_workspace.create_window].map(&:inspect)",
            )
            .unwrap(),
        "[\"#<Toyoterm::Pane:0>\", \"#<Toyoterm::Window:0>\", \"#<Toyoterm::Workspace:0>\"]"
    );
    manager
        .eval(
            "Toyoterm.current_pane.focus; Toyoterm.current_tab.close; \
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
fn switches_to_a_named_workspace_through_a_native_command() {
    let mut manager = ConfigManager::new().unwrap();
    assert_eq!(
        manager.eval("Toyoterm.switch_workspace(:backend)").unwrap(),
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
            "Toyoterm.switch_workspace('')",
            "workspace name cannot be empty",
        ),
        (
            "Toyoterm.switch_workspace(\"bad\\0name\")",
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
            NativeCommand::InvokeAction(NativeAction::ToggleFullscreen),
            NativeCommand::InvokeAction(NativeAction::Search),
            NativeCommand::InvokeAction(NativeAction::YankSelection),
            NativeCommand::InvokeAction(NativeAction::Split(SplitDirection::Down)),
            NativeCommand::InvokeAction(NativeAction::ActivatePane(SplitDirection::Left)),
            NativeCommand::InvokeAction(NativeAction::MoveVisualSelection(
                toyoterm_api::SelectionMotion::LineEnd,
            )),
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
              config.bind "CTRL+A" do
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
            Toyoterm.current_workspace.create_window(
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

    let error = manager.set_current_pane(PaneId(23)).unwrap_err();
    assert_eq!(error.operation(), "set current pane");
    assert!(error.message().contains("bad pane 23"));
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
                  config.bind "CTRL+C" do
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
                  config.bind "CTRL+B" do |context|
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
                  config.bind "CTRL+SHIFT+H" do |ctx|
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
                    key("v").toggle_visual_selection
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
                  config.bind("CTRL+X") { }
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
                  config.bind "CTRL+E" do |ctx|
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
                  config.bind("CTRL+SHIFT+R") { Toyoterm.reload_config }
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

    assert!(!manager.emit_event("config_reloaded", PaneId(12)).unwrap());
    assert_eq!(manager.eval("$event_count").unwrap(), "0");
    assert!(manager.emit_event("app_started", PaneId(12)).unwrap());
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
        name: "title_changed",
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
            .emit_native_event(&RubyEvent::new("pane_created"))
            .unwrap()
    );
}

#[test]
fn ruby_event_errors_roll_back_commands() {
    let mut manager = ConfigManager::new().unwrap();
    manager
        .reload(
            r#"
                Toyoterm.on :config_reloaded do |event|
                  event.pane.send_text("must not run\n")
                  raise "broken event"
                end
                "#,
        )
        .unwrap();

    let error = manager
        .emit_event("config_reloaded", PaneId(3))
        .unwrap_err();
    assert!(error.message().contains("broken event"));
    assert!(manager.drain_commands(PaneId(3)).unwrap().is_empty());
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
