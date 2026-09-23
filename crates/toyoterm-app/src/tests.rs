use super::*;

#[test]
fn pty_output_coalesces_wakeups_until_the_buffer_is_drained() {
    let pending = PtyOutputBuffer::default();
    let mut wakeups = 0;
    assert!(pending.append(b"one", || {
        wakeups += 1;
        true
    }));
    assert!(pending.append(b"two", || {
        wakeups += 1;
        true
    }));

    assert_eq!(wakeups, 1);
    assert_eq!(pending.take(), b"onetwo");
    assert!(pending.append(b"three", || {
        wakeups += 1;
        true
    }));
    assert_eq!(wakeups, 2);
}

#[test]
fn occlusion_suppresses_rendering_until_the_window_becomes_visible() {
    let mut state = WindowOcclusion::default();
    assert!(state.should_render());

    assert!(!state.update(true));
    assert!(!state.should_render());
    assert!(!state.update(true));

    assert!(state.update(false));
    assert!(state.should_render());
    assert!(!state.update(false));
}

#[test]
fn pty_output_reuses_drained_buffer_allocations() {
    let pending = PtyOutputBuffer::default();
    let payload = vec![b'x'; 64 * 1024];

    assert!(pending.append(&payload, || true));
    let first = pending.take();
    let first_allocation = first.as_ptr();
    pending.recycle(first);

    // Two allocations allow the reader and main thread to operate on
    // separate buffers. The third batch should rotate back to the first.
    assert!(pending.append(&payload, || true));
    let second = pending.take();
    pending.recycle(second);
    assert!(pending.append(&payload, || true));
    let third = pending.take();

    assert_eq!(third.as_ptr(), first_allocation);
}

#[test]
fn pty_output_applies_backpressure_at_the_memory_limit() {
    let pending = Arc::new(PtyOutputBuffer::default());
    assert!(pending.append(&vec![0; MAX_PENDING_PTY_OUTPUT_BYTES], || true));
    let (ready_tx, ready_rx) = mpsc::channel();
    let (started_tx, started_rx) = mpsc::channel();
    let worker_pending = pending.clone();
    let worker = thread::spawn(move || {
        started_tx.send(()).unwrap();
        worker_pending.append(b"x", || ready_tx.send(()).is_ok())
    });

    started_rx.recv().unwrap();
    assert!(ready_rx.recv_timeout(Duration::from_millis(20)).is_err());
    assert_eq!(pending.take().len(), MAX_PENDING_PTY_OUTPUT_BYTES);
    ready_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("draining should release the PTY reader");
    assert!(worker.join().unwrap());
    assert_eq!(pending.take(), b"x");
}

#[test]
fn tab_color_requires_all_components_and_resets_atomically() {
    let mut color = TabColorState::default();
    color.set(TabColorComponent::Red, 12);
    color.set(TabColorComponent::Green, 34);
    assert_eq!(color.complete(), None);
    color.set(TabColorComponent::Blue, 56);
    assert_eq!(color.complete(), Some([12, 34, 56]));
    color = TabColorState::default();
    assert_eq!(color.complete(), None);
}

#[test]
fn value_less_paused_progress_keeps_the_current_percentage() {
    assert_eq!(
        apply_terminal_progress(
            Some(TerminalProgress::Normal(42)),
            TerminalProgress::Warning(None),
        ),
        Some(TerminalProgress::Warning(Some(42)))
    );
    assert_eq!(
        apply_terminal_progress(
            Some(TerminalProgress::Indeterminate),
            TerminalProgress::Warning(None),
        ),
        Some(TerminalProgress::Warning(None))
    );
    assert_eq!(
        apply_terminal_progress(
            Some(TerminalProgress::Normal(42)),
            TerminalProgress::Error(None),
        ),
        Some(TerminalProgress::Error(None))
    );
    assert_eq!(
        apply_terminal_progress(
            Some(TerminalProgress::Warning(Some(42))),
            TerminalProgress::Hidden,
        ),
        None
    );
}

#[test]
fn visual_bell_deadline_requires_an_osc21_color() {
    let now = Instant::now();
    assert_eq!(visual_bell_deadline(None, now), None);
    assert_eq!(
        visual_bell_deadline(Some([1, 2, 3]), now),
        Some(now + OSC_VISUAL_BELL_DURATION)
    );
}

#[test]
fn session_status_updates_only_present_fields_and_supports_clear() {
    let mut state = SessionStatusState::default();
    state.apply(&SessionStatusUpdate {
        indicator: Some(Some([1, 2, 3])),
        status: Some(Some("building".into())),
        status_color: Some(Some([4, 5, 6])),
    });
    state.apply(&SessionStatusUpdate {
        status: Some(Some("ready".into())),
        ..SessionStatusUpdate::default()
    });
    assert_eq!(state.indicator, Some([1, 2, 3]));
    assert_eq!(state.status.as_deref(), Some("ready"));
    assert_eq!(state.status_color, Some([4, 5, 6]));

    state.apply(&SessionStatusUpdate {
        indicator: Some(None),
        status: Some(None),
        status_color: Some(None),
    });
    assert_eq!(state.indicator, None);
    assert_eq!(state.status, None);
    assert_eq!(state.status_color, None);
}

#[test]
fn filters_notification_occasions_against_source_visibility() {
    assert!(notification_occasion_matches(
        NotificationOccasion::Always,
        true,
        true
    ));
    assert!(!notification_occasion_matches(
        NotificationOccasion::Unfocused,
        true,
        true
    ));
    assert!(notification_occasion_matches(
        NotificationOccasion::Unfocused,
        false,
        true
    ));
    assert!(!notification_occasion_matches(
        NotificationOccasion::Invisible,
        false,
        true
    ));
    assert!(notification_occasion_matches(
        NotificationOccasion::Invisible,
        false,
        false
    ));
}

#[test]
fn osc_url_opening_requires_opt_in_allowlisted_scheme_and_rate_limit() {
    let now = Instant::now();
    assert!(should_open_osc_url(true, "https://example.com", None, now));
    assert!(!should_open_osc_url(
        false,
        "https://example.com",
        None,
        now
    ));
    assert!(!should_open_osc_url(true, "file:///tmp/a", None, now));
    assert!(!should_open_osc_url(
        true,
        "https://example.com",
        Some(now - Duration::from_secs(1)),
        now
    ));
    assert!(should_open_osc_url(
        true,
        "mailto:user@example.com",
        Some(now - OSC_OPEN_URL_INTERVAL),
        now
    ));
}

#[test]
fn osc_focus_requests_require_opt_in_and_rate_limit() {
    let now = Instant::now();
    assert!(should_request_focus(true, None, now));
    assert!(!should_request_focus(false, None, now));
    assert!(!should_request_focus(
        true,
        Some(now - Duration::from_secs(1)),
        now
    ));
    assert!(should_request_focus(
        true,
        Some(now - OSC_FOCUS_REQUEST_INTERVAL),
        now
    ));
}

#[test]
fn notification_replacement_ids_are_stable_and_pane_scoped() {
    let first = notification_platform_id(PaneId(1), "build-42");
    assert_eq!(first, notification_platform_id(PaneId(1), "build-42"));
    assert_ne!(first, notification_platform_id(PaneId(2), "build-42"));
    assert_ne!(first, notification_platform_id(PaneId(1), "build-43"));
    assert_ne!(first, 0);
}

#[test]
fn reports_only_unexpired_accepted_notification_ids() {
    let now = Instant::now();
    let mut active = BTreeMap::from([
        ("persistent".into(), None),
        ("expired".into(), Some(now - Duration::from_millis(1))),
        ("pending".into(), Some(now + Duration::from_millis(1))),
    ]);

    assert_eq!(
        notification_alive_response("probe", &mut active, now),
        "\x1b]99;i=probe:p=alive;pending,persistent\x1b\\"
    );
    assert!(!active.contains_key("expired"));
    assert_eq!(notification_expiry(Some(0), now), None);
    assert_eq!(
        notification_expiry(Some(25), now),
        Some(now + Duration::from_millis(25))
    );
}

#[test]
fn maps_shell_command_lifecycle_to_ruby_events() {
    let pane = PaneId(7);
    let prompt = ruby_event_from_terminal_event(pane, TerminalEvent::PromptStarted).unwrap();
    assert_eq!(prompt.name(), "prompt_started");
    assert_eq!(prompt.pane, Some(pane));

    let command_line =
        ruby_event_from_terminal_event(pane, TerminalEvent::CommandLineStarted).unwrap();
    assert_eq!(command_line.name(), "command_line_started");
    assert_eq!(command_line.pane, Some(pane));

    let started = ruby_event_from_terminal_event(pane, TerminalEvent::CommandStarted).unwrap();
    assert_eq!(started.name(), "command_started");
    assert_eq!(started.pane, Some(pane));
    assert_eq!(started.exit_status, None);

    let finished =
        ruby_event_from_terminal_event(pane, TerminalEvent::CommandFinished(Some(23))).unwrap();
    assert_eq!(finished.name(), "command_finished");
    assert_eq!(finished.pane, Some(pane));
    assert_eq!(finished.exit_status, Some(23));
}

#[test]
fn reports_bounded_iterm_session_variables() {
    let mut user_vars = BTreeMap::new();
    user_vars.insert("gitBranch".into(), "main".into());
    let mut runtime = PaneRuntime {
        terminal: AlacrittyTerminalBackend::new(80, 24),
        process: ProcessRuntime {
            pty_session: None,
            process_id: None,
            exited: false,
        },
        metadata: PaneMetadata {
            title: "build server".into(),
            icon_title: Some("build".into()),
            osc_badge: None,
            cwd: Some(PathBuf::from("/srv/project")),
            remote_host: Some("alice@example.com".into()),
        },
        protocol: PaneProtocolState {
            shell_integration_version: Some(1),
            shell_integration_shell: Some("bash".into()),
            user_vars,
            ..PaneProtocolState::default()
        },
        input_pacing: PaneInputPacing::default(),
    };

    assert_eq!(
        iterm_variable_response(&runtime, "session.name"),
        "\x1b]1337;ReportVariable=YnVpbGQgc2VydmVy\x1b\\"
    );
    assert_eq!(
        iterm_variable_value(&runtime, "session.terminalIconName").as_deref(),
        Some("build")
    );
    assert_eq!(
        iterm_variable_value(&runtime, "session.path").as_deref(),
        Some("/srv/project")
    );
    assert_eq!(
        iterm_variable_value(&runtime, "session.columns").as_deref(),
        Some("80")
    );
    assert_eq!(
        iterm_variable_value(&runtime, "session.rows").as_deref(),
        Some("24")
    );
    assert_eq!(
        iterm_variable_value(&runtime, "session.shell").as_deref(),
        Some("bash")
    );
    assert_eq!(
        iterm_variable_value(&runtime, "session.hostname").as_deref(),
        Some("example.com")
    );
    assert_eq!(
        iterm_variable_value(&runtime, "session.username").as_deref(),
        Some("alice")
    );
    assert_eq!(
        iterm_variable_value(&runtime, "session.user.gitBranch").as_deref(),
        Some("main")
    );
    assert_eq!(
        iterm_variable_response(&runtime, "session.unknown"),
        "\x1b]1337;ReportVariable=\x1b\\"
    );
    assert_eq!(
        interpolate_iterm_badge(
            &runtime,
            r"\(session.name) · \(user.gitBranch) · \(session.hostname)"
        )
        .as_deref(),
        Some("build server · main · example.com")
    );
    assert_eq!(
        interpolate_iterm_badge(&runtime, r"unknown=\(session.missing)").as_deref(),
        Some("unknown=")
    );
    apply_iterm_badge_format(&mut runtime, r"\(session.name):\(user.gitBranch)");
    assert_eq!(
        runtime.metadata.osc_badge.as_deref(),
        Some("build server:main")
    );
    apply_iterm_badge_format(&mut runtime, "");
    assert_eq!(runtime.metadata.osc_badge, None);
}

#[cfg(unix)]
#[test]
fn custom_pane_launch_applies_argv_cwd_and_environment() {
    let cwd = std::env::temp_dir();
    let expected_cwd = cwd
        .canonicalize()
        .expect("canonicalize temporary directory");
    let launch = PaneLaunchSpec {
        program: Some("/bin/sh".into()),
        args: vec![
            "-c".into(),
            "printf '%s|%s' \"$PWD\" \"$TOYOTERM_LAUNCH_TEST\"".into(),
        ],
        cwd: Some(cwd.display().to_string()),
        environment: vec![("TOYOTERM_LAUNCH_TEST".into(), Some("works".into()))],
    };
    let command = pane_lifecycle::pty_command_for_launch(None, Some(&launch));
    let mut session = NativePty
        .spawn(command, PtySize::new(80, 24))
        .expect("spawn custom pane command");
    let mut reader = session.take_reader().expect("take PTY reader");
    let mut output = String::new();
    reader.read_to_string(&mut output).expect("read PTY output");
    let status = session.wait().expect("wait for custom pane command");

    assert_eq!(status.code, 0);
    assert!(
        output.contains(&format!("{}|works", expected_cwd.display())),
        "unexpected output: {output:?}"
    );
}

#[cfg(windows)]
#[test]
fn pane_launch_removes_stale_outer_terminal_identity() {
    let launch = PaneLaunchSpec {
        program: Some("cmd.exe".into()),
        args: vec![
            "/D".into(),
            "/S".into(),
            "/C".into(),
            concat!(
                "if defined WEZTERM_EXECUTABLE (exit /b 7) else ",
                "if defined TMUX (exit /b 8) else ",
                "if \"%TERM_PROGRAM%\"==\"toyoterm\" (exit /b 0) else exit /b 9"
            )
            .into(),
        ],
        cwd: None,
        environment: vec![
            ("WEZTERM_EXECUTABLE".into(), Some("stale".into())),
            ("TMUX".into(), Some("stale".into())),
        ],
    };
    let command = pane_lifecycle::pty_command_for_launch(None, Some(&launch));
    let mut session = NativePty
        .spawn(command, PtySize::new(80, 24))
        .expect("spawn custom pane command");
    let mut reader = session.take_reader().expect("take PTY reader");
    let mut output = Vec::new();
    reader.read_to_end(&mut output).expect("read PTY output");
    let status = session.wait().expect("wait for custom pane command");

    assert_eq!(status.code, 0, "unexpected output: {output:?}");
}

struct KillTrackingSession(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl PtySession for KillTrackingSession {
    fn process_id(&self) -> Option<u32> {
        Some(42)
    }

    fn take_reader(&mut self) -> Result<Box<dyn Read + Send>, crate::PtyError> {
        Ok(Box::new(std::io::Cursor::new(Vec::<u8>::new())))
    }

    fn write(&mut self, _data: &[u8]) -> Result<(), crate::PtyError> {
        Ok(())
    }

    fn resize(&mut self, _size: PtySize) -> Result<(), crate::PtyError> {
        Ok(())
    }

    fn try_wait(&mut self) -> Result<Option<crate::PtyExitStatus>, crate::PtyError> {
        Ok(None)
    }

    fn wait(&mut self) -> Result<crate::PtyExitStatus, crate::PtyError> {
        Ok(crate::PtyExitStatus {
            code: 0,
            signal: None,
        })
    }

    fn kill(&mut self) -> Result<(), crate::PtyError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

struct RecordingPtySession(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl PtySession for RecordingPtySession {
    fn process_id(&self) -> Option<u32> {
        Some(42)
    }

    fn take_reader(&mut self) -> Result<Box<dyn Read + Send>, crate::PtyError> {
        Ok(Box::new(std::io::Cursor::new(Vec::<u8>::new())))
    }

    fn write(&mut self, data: &[u8]) -> Result<(), crate::PtyError> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend_from_slice(data);
        Ok(())
    }

    fn resize(&mut self, _size: PtySize) -> Result<(), crate::PtyError> {
        Ok(())
    }

    fn try_wait(&mut self) -> Result<Option<crate::PtyExitStatus>, crate::PtyError> {
        Ok(None)
    }

    fn wait(&mut self) -> Result<crate::PtyExitStatus, crate::PtyError> {
        Ok(crate::PtyExitStatus {
            code: 0,
            signal: None,
        })
    }

    fn kill(&mut self) -> Result<(), crate::PtyError> {
        Ok(())
    }
}

fn recording_pane_runtime(recorded: std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> PaneRuntime {
    PaneRuntime {
        terminal: AlacrittyTerminalBackend::new(80, 24),
        process: ProcessRuntime {
            pty_session: Some(Box::new(RecordingPtySession(recorded))),
            process_id: Some(42),
            exited: false,
        },
        metadata: PaneMetadata {
            title: "test".into(),
            icon_title: None,
            osc_badge: None,
            cwd: None,
            remote_host: None,
        },
        protocol: PaneProtocolState::default(),
        input_pacing: PaneInputPacing::default(),
    }
}

#[test]
fn pane_runtime_kills_its_child_when_dropped_during_shutdown() {
    let kills = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    {
        let _runtime = PaneRuntime {
            terminal: AlacrittyTerminalBackend::new(80, 24),
            process: ProcessRuntime {
                pty_session: Some(Box::new(KillTrackingSession(kills.clone()))),
                process_id: Some(42),
                exited: false,
            },
            metadata: PaneMetadata {
                title: "test".into(),
                icon_title: None,
                osc_badge: None,
                cwd: None,
                remote_host: None,
            },
            protocol: PaneProtocolState::default(),
            input_pacing: PaneInputPacing::default(),
        };
    }

    assert_eq!(kills.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn write_input_paces_after_carriage_return_on_primary_screen() {
    let recorded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut runtime = recording_pane_runtime(recorded.clone());
    let pane = PaneId(1);

    // Initial input without return writes immediately.
    runtime.write_input(pane, b"echo").unwrap();
    assert_eq!(recorded.lock().unwrap().as_slice(), b"echo");
    assert!(!runtime.input_pacing.awaiting_transition);
    assert!(runtime.input_pacing.pending.is_empty());

    // Input containing '\r' writes up to '\r' and paces the trailing bytes.
    runtime.write_input(pane, b" nvim\r:q\r").unwrap();
    assert_eq!(recorded.lock().unwrap().as_slice(), b"echo nvim\r");
    assert!(runtime.input_pacing.awaiting_transition);
    assert_eq!(runtime.input_pacing.pending, b":q\r");
    assert!(runtime.input_pacing.deadline.is_some());

    // Additional input while awaiting transition is queued.
    runtime.write_input(pane, b"more").unwrap();
    assert_eq!(recorded.lock().unwrap().as_slice(), b"echo nvim\r");
    assert_eq!(runtime.input_pacing.pending, b":q\rmore");

    // Flushing sends pending bytes and clears the transition state.
    runtime.flush_input_pacing(pane);
    assert_eq!(recorded.lock().unwrap().as_slice(), b"echo nvim\r:q\rmore");
    assert!(!runtime.input_pacing.awaiting_transition);
    assert!(runtime.input_pacing.pending.is_empty());
    assert!(runtime.input_pacing.deadline.is_none());
}

#[test]
fn write_input_bypasses_pacing_on_alternate_screen() {
    let recorded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut runtime = recording_pane_runtime(recorded.clone());
    let pane = PaneId(1);

    // Switch terminal to alternate screen (e.g., Neovim already running).
    runtime.terminal.advance(b"\x1b[?1049h");
    assert!(runtime.terminal.mode().alternate_screen);

    // Input with '\r' on alternate screen is written immediately without pacing.
    runtime.write_input(pane, b":q\r").unwrap();
    assert_eq!(recorded.lock().unwrap().as_slice(), b":q\r");
    assert!(!runtime.input_pacing.awaiting_transition);
    assert!(runtime.input_pacing.pending.is_empty());
}

#[test]
fn write_input_holds_pending_when_alternate_screen_is_entered_during_transition() {
    let recorded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut runtime = recording_pane_runtime(recorded.clone());
    let pane = PaneId(1);

    // Launch a command on primary screen.
    runtime.write_input(pane, b"nvim\r").unwrap();
    assert_eq!(recorded.lock().unwrap().as_slice(), b"nvim\r");
    assert!(runtime.input_pacing.awaiting_transition);

    // Neovim enters alternate screen early during its startup.
    runtime.terminal.advance(b"\x1b[?1049h");
    assert!(runtime.terminal.mode().alternate_screen);

    // Fast user input (:q\r) while transition is still active MUST be held,
    // not bypassed or prematurely flushed while Neovim plugins/UI are loading.
    runtime.write_input(pane, b":q\r").unwrap();
    assert_eq!(recorded.lock().unwrap().as_slice(), b"nvim\r");
    assert!(runtime.input_pacing.awaiting_transition);
    assert_eq!(runtime.input_pacing.pending, b":q\r");

    // Once pacing window flushes (e.g., timer expires), keys are sent.
    runtime.flush_input_pacing(pane);
    assert_eq!(recorded.lock().unwrap().as_slice(), b"nvim\r:q\r");
    assert!(!runtime.input_pacing.awaiting_transition);
    assert!(runtime.input_pacing.pending.is_empty());
}

#[test]
fn write_input_ctrl_c_cancels_pending_pacing_immediately() {
    let recorded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut runtime = recording_pane_runtime(recorded.clone());
    let pane = PaneId(1);

    // Queue input behind a carriage return.
    runtime
        .write_input(pane, b"slow_command\rqueued_keys")
        .unwrap();
    assert_eq!(recorded.lock().unwrap().as_slice(), b"slow_command\r");
    assert!(runtime.input_pacing.awaiting_transition);
    assert_eq!(runtime.input_pacing.pending, b"queued_keys");

    // Sending Ctrl+C (\x03) drops pending input and writes interrupt immediately.
    runtime.write_input(pane, b"\x03").unwrap();
    assert_eq!(recorded.lock().unwrap().as_slice(), b"slow_command\r\x03");
    assert!(!runtime.input_pacing.awaiting_transition);
    assert!(runtime.input_pacing.pending.is_empty());
    assert!(runtime.input_pacing.deadline.is_none());
}

#[test]
fn config_error_notice_switches_between_summary_and_log() {
    let mut notice = ConfigErrorNotice {
        message: "error\nconfig.rb:2\nconfig.rb:5\nextra".into(),
        log_expanded: false,
    };
    assert_eq!(notice.display_message(), "error\nconfig.rb:2\nconfig.rb:5");

    notice.log_expanded = true;
    assert_eq!(notice.display_message(), notice.message);
}

#[test]
fn calculates_grid_from_physical_window_size() {
    let metrics = CellMetrics::default();
    assert_eq!(
        metrics.terminal_size(PhysicalSize::new(916, 556)),
        PtySize {
            columns: 100,
            rows: 30,
            pixel_width: 900,
            pixel_height: 540
        }
    );
}

#[test]
fn grid_dimensions_never_reach_zero() {
    let metrics = CellMetrics::default();
    assert_eq!(
        metrics.terminal_size(PhysicalSize::new(0, 0)),
        PtySize {
            columns: 2,
            rows: 1,
            pixel_width: 18,
            pixel_height: 18
        }
    );
}

#[test]
fn hidpi_scaling_preserves_logical_grid_size() {
    let metrics = CellMetrics::default();
    let logical = metrics.terminal_size_at_scale(PhysicalSize::new(916, 556), 1.0);
    let hidpi = metrics.terminal_size_at_scale(PhysicalSize::new(1832, 1112), 2.0);
    assert_eq!((logical.columns, logical.rows), (hidpi.columns, hidpi.rows));
    assert_eq!(
        (hidpi.pixel_width, hidpi.pixel_height),
        (logical.pixel_width * 2, logical.pixel_height * 2)
    );
}

#[test]
fn dispatches_startup_ruby_commands_through_the_mux() {
    let mut config_manager = ConfigManager::new().unwrap();
    config_manager
        .reload(r#"Toyoterm.current_pane.send_text("echo hello\n")"#)
        .unwrap();
    let mut mux = Mux::new();
    let pane = mux.current_pane().unwrap();

    let effects = dispatch_script_commands(&mut config_manager, &mut mux).unwrap();

    assert_eq!(mux.take_pending_input(pane).unwrap(), b"echo hello\n");
    assert_eq!(effects, NativeCommandEffects::default());
}

#[test]
fn separates_ruby_clipboard_writes_from_mux_commands() {
    let mut config_manager = ConfigManager::new().unwrap();
    config_manager
        .eval(r#"Toyoterm.clipboard.write("from Ruby")"#)
        .unwrap();
    let mut mux = Mux::new();

    assert_eq!(
        dispatch_script_commands(&mut config_manager, &mut mux).unwrap(),
        NativeCommandEffects {
            clipboard_writes: vec!["from Ruby".to_owned()],
            reload_requested: false,
        }
    );
}

#[test]
fn normalizes_ruby_reload_requests_as_native_commands() {
    let mut config_manager = ConfigManager::new().unwrap();
    config_manager.eval("Toyoterm.reload_config").unwrap();
    let mut mux = Mux::new();

    assert_eq!(
        dispatch_script_commands(&mut config_manager, &mut mux).unwrap(),
        NativeCommandEffects {
            clipboard_writes: Vec::new(),
            reload_requested: true,
        }
    );
}

#[test]
fn normalizes_native_keys_for_the_ruby_binding_resolver() {
    assert_eq!(
        keybinding_name(
            &Key::Character("h".into()),
            ModifiersState::CONTROL | ModifiersState::SHIFT,
        )
        .as_deref(),
        Some("CTRL+SHIFT+H")
    );
    assert_eq!(
        keybinding_name(&Key::Named(NamedKey::F5), ModifiersState::ALT).as_deref(),
        Some("ALT+F5")
    );
}

#[test]
fn physical_binding_candidates_take_priority_over_logical_keys() {
    assert_eq!(
        binding_candidates(
            Some("KeyY".into()),
            Some("Z".into()),
            KeyModifiers {
                control: true,
                ..KeyModifiers::default()
            },
            false,
        ),
        vec!["CTRL+PHYSICAL:KEYY", "CTRL+Z"]
    );
}

#[test]
fn shifted_symbols_also_match_their_logical_key_without_shift() {
    assert_eq!(
        binding_candidates(
            Some("Digit4".into()),
            Some("$".into()),
            KeyModifiers {
                shift: true,
                ..KeyModifiers::default()
            },
            true,
        ),
        vec!["SHIFT+PHYSICAL:DIGIT4", "SHIFT+$", "$"]
    );
}

#[test]
fn shifted_letters_do_not_fall_back_to_unmodified_bindings() {
    assert_eq!(
        binding_candidates(
            Some("KeyH".into()),
            Some("H".into()),
            KeyModifiers {
                shift: true,
                ..KeyModifiers::default()
            },
            false,
        ),
        vec!["SHIFT+PHYSICAL:KEYH", "SHIFT+H"]
    );
}

#[test]
fn alt_graph_is_not_treated_as_control_alt() {
    let modifiers = effective_modifiers(ModifiersState::CONTROL | ModifiersState::ALT, true);
    assert!(!modifiers.control_key());
    assert!(!modifiers.alt_key());

    let press = KeyPress::new(TerminalKey::Text("@".into()), key_modifiers(modifiers));
    assert_eq!(
        encode_key(&press, crate::TerminalMode::default()),
        Some(b"@".to_vec())
    );
}

#[test]
fn real_control_alt_remains_available_for_bindings() {
    let modifiers = effective_modifiers(ModifiersState::CONTROL | ModifiersState::ALT, false);
    assert_eq!(
        keybinding_name(&Key::Character("x".into()), modifiers).as_deref(),
        Some("CTRL+ALT+X")
    );
}

#[test]
fn hyperlink_hit_testing_and_scheme_allowlist_are_safe() {
    let mut terminal = AlacrittyTerminalBackend::new(30, 2);
    terminal.advance(b"visit https://example.com now");
    let snapshot = terminal.snapshot();

    assert_eq!(
        hyperlink_at(&snapshot, 8, 0).as_deref(),
        Some("https://example.com")
    );
    assert_eq!(hyperlink_at(&snapshot, 0, 0), None);
    assert!(validate_allowed_url("https://example.com/path").is_ok());
    assert!(validate_allowed_url("mailto:user@example.com").is_ok());
    assert!(validate_allowed_url("file:///etc/passwd").is_err());
    assert!(validate_allowed_url("javascript:alert(1)").is_err());
    assert!(validate_allowed_url("https://example.com\ncommand").is_err());
}

#[test]
fn key_repeat_is_delivered_and_releases_are_ignored() {
    assert!(should_handle_key_event(ElementState::Pressed, false));
    assert!(should_handle_key_event(ElementState::Pressed, true));
    assert!(!should_handle_key_event(ElementState::Released, false));
    assert!(!should_handle_key_event(ElementState::Released, true));
}

#[test]
fn terminal_input_returns_a_scrolled_viewport_to_the_bottom() {
    let mut terminal = AlacrittyTerminalBackend::new(10, 2);
    terminal.advance(b"one\r\ntwo\r\nthree");
    terminal.scroll_display(1);
    assert_eq!(terminal.snapshot().lines, ["one", "two"]);

    pane_lifecycle::reset_scroll_for_input(&mut terminal, b"x");

    assert_eq!(terminal.snapshot().lines, ["two", "three"]);
}

#[test]
fn empty_terminal_input_preserves_a_scrolled_viewport() {
    let mut terminal = AlacrittyTerminalBackend::new(10, 2);
    terminal.advance(b"one\r\ntwo\r\nthree");
    terminal.scroll_display(1);

    pane_lifecycle::reset_scroll_for_input(&mut terminal, b"");

    assert_eq!(terminal.snapshot().lines, ["one", "two"]);
}

#[test]
fn focus_loss_clears_all_modifier_state() {
    let mut modifiers = ModifiersState::SHIFT
        | ModifiersState::CONTROL
        | ModifiersState::ALT
        | ModifiersState::SUPER;
    let mut alt_graph_active = true;

    clear_modifier_state(&mut modifiers, &mut alt_graph_active);

    assert!(modifiers.is_empty());
    assert!(!alt_graph_active);
}

#[test]
fn maps_physical_numpad_keys_independently_of_layout() {
    assert_eq!(
        keypad_key(PhysicalKey::Code(KeyCode::Numpad7)),
        Some(KeypadKey::Digit(7))
    );
    assert_eq!(
        keypad_key(PhysicalKey::Code(KeyCode::NumpadEnter)),
        Some(KeypadKey::Enter)
    );
    assert_eq!(keypad_key(PhysicalKey::Code(KeyCode::Digit7)), None);
}

#[test]
fn click_tracker_recognizes_double_and_triple_clicks() {
    let mut tracker = ClickTracker::default();
    let start = Instant::now();
    let target = ClickTarget {
        pane: PaneId(1),
        column: 4,
        row: 2,
    };

    assert_eq!(tracker.register(start, target), 1);
    assert_eq!(
        tracker.register(start + Duration::from_millis(100), target),
        2
    );
    assert_eq!(
        tracker.register(start + Duration::from_millis(200), target),
        3
    );
    assert_eq!(
        tracker.register(start + Duration::from_millis(300), target),
        1
    );
}

#[test]
fn click_tracker_resets_for_a_new_cell_or_after_timeout() {
    let mut tracker = ClickTracker::default();
    let start = Instant::now();
    let target = ClickTarget {
        pane: PaneId(1),
        column: 4,
        row: 2,
    };
    tracker.register(start, target);

    assert_eq!(
        tracker.register(
            start + Duration::from_millis(100),
            ClickTarget {
                column: 5,
                ..target
            },
        ),
        1
    );
    assert_eq!(
        tracker.register(
            start + MULTI_CLICK_INTERVAL + Duration::from_millis(200),
            target
        ),
        1
    );
}

#[test]
fn window_bars_reserve_the_top_and_bottom_edges() {
    let config = ToyotermConfig {
        status_bars: [StatusBarPosition::Top, StatusBarPosition::Bottom]
            .map(|position| toyoterm_config::StatusBarConfig { position })
            .into(),
        ..ToyotermConfig::default()
    };

    let (pane, bars) = edge_bar_layout(PhysicalSize::new(960, 600), 54, &config, 1.0);
    assert_eq!(pane, PaneRect::new(0, 78, 960, 498));
    assert_eq!(
        bars,
        vec![
            (StatusBarPosition::Top, PaneRect::new(0, 54, 960, 24)),
            (StatusBarPosition::Bottom, PaneRect::new(0, 576, 960, 24)),
        ]
    );
}

#[test]
fn ui_sizes_scale_from_logical_pixels() {
    assert_eq!(scaled_ui_size(160.0, 1.0), 160);
    assert_eq!(scaled_ui_size(160.0, 1.5), 240);
}

#[test]
fn encodes_named_space_and_tab_for_the_pty() {
    let mode = crate::TerminalMode::default();
    let space = KeyPress::new(
        named_key(&NamedKey::Space).unwrap(),
        KeyModifiers::default(),
    );
    let tab = KeyPress::new(named_key(&NamedKey::Tab).unwrap(), KeyModifiers::default());

    assert_eq!(encode_key(&space, mode), Some(b" ".to_vec()));
    assert_eq!(encode_key(&tab, mode), Some(b"\t".to_vec()));
}

#[test]
fn encodes_osc99_notification_feedback() {
    let feedback = |kind| NotificationFeedback {
        pane: PaneId(7),
        id: "job-1".into(),
        kind,
    };

    assert_eq!(
        notification_feedback_response(&feedback(NotificationFeedbackKind::Activated)),
        "\x1b]99;i=job-1;\x1b\\"
    );
    assert_eq!(
        notification_feedback_response(&feedback(NotificationFeedbackKind::Button(2))),
        "\x1b]99;i=job-1;2\x1b\\"
    );
    assert_eq!(
        notification_feedback_response(&feedback(NotificationFeedbackKind::Closed)),
        "\x1b]99;i=job-1:p=close;\x1b\\"
    );
}

#[test]
fn maps_winit_mouse_buttons() {
    assert_eq!(
        pane_lifecycle::terminal_mouse_button(MouseButton::Left),
        Some(TerminalMouseButton::Left)
    );
    assert_eq!(
        pane_lifecycle::terminal_mouse_button(MouseButton::Middle),
        Some(TerminalMouseButton::Middle)
    );
    assert_eq!(
        pane_lifecycle::terminal_mouse_button(MouseButton::Right),
        Some(TerminalMouseButton::Right)
    );
    assert_eq!(
        pane_lifecycle::terminal_mouse_button(MouseButton::Back),
        None
    );
    assert_eq!(
        pane_lifecycle::terminal_mouse_button(MouseButton::Forward),
        None
    );
}
