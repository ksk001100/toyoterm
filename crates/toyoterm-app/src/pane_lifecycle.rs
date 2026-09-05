use super::*;

pub(super) fn pty_command_for_launch(
    default_shell: Option<&str>,
    launch: Option<&PaneLaunchSpec>,
) -> PtyCommand {
    let mut command = match launch
        .and_then(|launch| launch.program.as_deref())
        .or(default_shell)
    {
        Some(shell) => PtyCommand::new(shell),
        None => PtyCommand::default_shell(),
    };
    if let Some(launch) = launch {
        command.args(&launch.args);
        if let Some(cwd) = launch.cwd.as_deref() {
            command.cwd(cwd);
        }
        for (key, value) in &launch.environment {
            match value {
                Some(value) => {
                    command.env(key, value);
                }
                None => {
                    command.env_remove(key);
                }
            }
        }
    }
    command.env("TERM", "xterm-256color");
    command.env("TERM_PROGRAM", "toyoterm");
    command.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
    command
}

impl ToyotermApplication {
    pub(super) fn start_shell(
        &mut self,
        pane: PaneId,
        size: PtySize,
        launch: Option<&PaneLaunchSpec>,
    ) -> Result<PaneRuntime, String> {
        let command =
            pty_command_for_launch(self.script_snapshot.config.default_shell.as_deref(), launch);
        let mut session = NativePty.spawn(command, size).map_err(|error| {
            tracing::error!(
                target: "toyoterm::pty",
                operation = error.operation(),
                %pane,
                columns = size.columns,
                rows = size.rows,
                %error,
                "start pane shell failed"
            );
            error.to_string()
        })?;
        let reader = session.take_reader().map_err(|error| {
            tracing::error!(
                target: "toyoterm::pty",
                operation = error.operation(),
                %pane,
                %error,
                "open pane PTY reader failed"
            );
            error.to_string()
        })?;
        let process_id = session.process_id();
        spawn_pty_reader(pane, reader, self.event_proxy.clone())?;
        Ok(PaneRuntime {
            terminal: AlacrittyTerminalBackend::with_scrollback(
                size.columns,
                size.rows,
                self.script_snapshot.config.scrollback_lines,
            ),
            snapshot_cache: None,
            pty_session: Some(session),
            process_id,
            title: format!("Pane {}", pane.0),
            cwd: std::env::current_dir().ok(),
            command_running: false,
            last_exit_status: None,
            exited: false,
        })
    }

    pub(super) fn flush_mux_input(&mut self) -> Result<(), String> {
        let panes = self.pane_runtimes.keys().copied().collect::<Vec<_>>();
        for pane in panes {
            let bytes = self
                .mux
                .take_pending_input(pane)
                .map_err(|error| error.to_string())?;
            if !bytes.is_empty() {
                self.write_pane_pty(pane, &bytes)?;
            }
        }
        Ok(())
    }

    pub(super) fn resize_panes(
        &mut self,
        window_size: PhysicalSize<u32>,
        scale_factor: f64,
    ) -> Result<(), String> {
        self.tab_layout = self.calculate_tab_layout(window_size, scale_factor);
        self.workspace_layout = self.calculate_workspace_layout(window_size, scale_factor);
        self.config_error_layout = self.calculate_config_error_layout(window_size, scale_factor);
        self.pane_layout = self.calculate_pane_layout(window_size, scale_factor);
        let sizes = self
            .pane_layout
            .panes()
            .iter()
            .map(|placement| {
                (
                    placement.pane,
                    self.cell_metrics.terminal_size_at_scale(
                        PhysicalSize::new(placement.rect.width, placement.rect.height),
                        scale_factor,
                    ),
                )
            })
            .collect::<Vec<_>>();
        for (pane, size) in sizes {
            if let Some(runtime) = self.pane_runtimes.get_mut(&pane) {
                runtime.terminal.resize(size.columns, size.rows);
                runtime.invalidate_snapshot();
                if let Some(session) = runtime.pty_session.as_mut() {
                    session.resize(size).map_err(|error| {
                        tracing::error!(
                            target: "toyoterm::pty",
                            operation = error.operation(),
                            %pane,
                            columns = size.columns,
                            rows = size.rows,
                            %error,
                            "resize pane PTY failed"
                        );
                        error.to_string()
                    })?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn write_pty(&mut self, bytes: &[u8]) -> Result<(), String> {
        let pane = self
            .mux
            .current_pane()
            .ok_or_else(|| "mux has no current pane".to_owned())?;
        self.write_pane_pty(pane, bytes)
    }

    pub(super) fn write_pane_pty(&mut self, pane: PaneId, bytes: &[u8]) -> Result<(), String> {
        let runtime = self
            .pane_runtimes
            .get_mut(&pane)
            .ok_or_else(|| format!("pane {pane} has no runtime"))?;
        if let Some(session) = runtime.pty_session.as_mut() {
            session.write(bytes).map_err(|error| {
                tracing::error!(
                    target: "toyoterm::pty",
                    operation = error.operation(),
                    %pane,
                    bytes = bytes.len(),
                    %error,
                    "write pane PTY failed"
                );
                error.to_string()
            })?;
        }
        Ok(())
    }

    pub(super) fn reconcile_pane_runtimes(&mut self) -> Result<(), String> {
        let live_panes = self.mux.pane_ids().collect::<HashSet<_>>();
        self.pane_badges.retain(|pane, _| live_panes.contains(pane));
        self.pending_pane_launches
            .retain(|pane, _| live_panes.contains(pane));
        self.refresh_pane_layout();
        let size = self
            .window
            .as_ref()
            .map(|window| {
                self.cell_metrics
                    .terminal_size_at_scale(window.inner_size(), window.scale_factor())
            })
            .unwrap_or_default();
        self.sync_pane_runtimes(size)
    }

    pub(super) fn refresh_pane_layout(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        self.tab_layout = self.calculate_tab_layout(window.inner_size(), window.scale_factor());
        self.workspace_layout =
            self.calculate_workspace_layout(window.inner_size(), window.scale_factor());
        self.config_error_layout =
            self.calculate_config_error_layout(window.inner_size(), window.scale_factor());
        self.pane_layout = self.calculate_pane_layout(window.inner_size(), window.scale_factor());
    }

    pub(super) fn calculate_workspace_layout(
        &self,
        window_size: PhysicalSize<u32>,
        scale_factor: f64,
    ) -> WorkspaceStripLayout {
        WorkspaceStripLayout::calculate(
            &self.mux.workspaces(),
            PaneRect::new(
                0,
                0,
                window_size.width,
                workspace_bar_height(&self.script_snapshot.config, scale_factor),
            ),
            scaled_ui_size(self.script_snapshot.config.ui.workspace_width, scale_factor),
        )
    }

    pub(super) fn calculate_config_error_layout(
        &self,
        window_size: PhysicalSize<u32>,
        scale_factor: f64,
    ) -> ConfigErrorLayout {
        let Some(notice) = self.config_error_notice.as_ref() else {
            return ConfigErrorLayout::default();
        };
        let y = workspace_bar_height(&self.script_snapshot.config, scale_factor)
            .saturating_add(tab_bar_height(&self.script_snapshot.config, scale_factor));
        let height = config_error_height(scale_factor, notice.log_expanded)
            .min(window_size.height.saturating_sub(y));
        ConfigErrorLayout::calculate(
            PaneRect::new(0, y, window_size.width, height),
            tab_bar_height(&self.script_snapshot.config, scale_factor),
        )
    }

    pub(super) fn calculate_tab_layout(
        &self,
        window_size: PhysicalSize<u32>,
        scale_factor: f64,
    ) -> TabStripLayout {
        let Some(window) = self.mux.current_window() else {
            return TabStripLayout::default();
        };
        let Some(tabs) = self.mux.tabs(window) else {
            return TabStripLayout::default();
        };
        TabStripLayout::calculate(
            tabs,
            PaneRect::new(
                0,
                workspace_bar_height(&self.script_snapshot.config, scale_factor),
                window_size.width,
                tab_bar_height(&self.script_snapshot.config, scale_factor),
            ),
            scaled_ui_size(self.script_snapshot.config.ui.tab_width, scale_factor),
        )
    }

    pub(super) fn calculate_pane_layout(
        &self,
        window_size: PhysicalSize<u32>,
        scale_factor: f64,
    ) -> PaneLayout {
        let Some(tab) = self.mux.current_tab() else {
            return PaneLayout::default();
        };
        let Some(root) = self.mux.pane_tree(tab) else {
            return PaneLayout::default();
        };
        let notification_height = self
            .config_error_notice
            .as_ref()
            .map(|notice| config_error_height(scale_factor, notice.log_expanded))
            .unwrap_or(0);
        let chrome_height = workspace_bar_height(&self.script_snapshot.config, scale_factor)
            .saturating_add(tab_bar_height(&self.script_snapshot.config, scale_factor))
            .saturating_add(notification_height)
            .min(window_size.height);
        let (pane_rect, _) = edge_bar_layout(
            window_size,
            chrome_height,
            &self.script_snapshot.config,
            scale_factor,
        );
        PaneLayout::calculate(
            root,
            pane_rect,
            scaled_ui_size(
                self.script_snapshot.config.ui.pane_divider_width,
                scale_factor,
            ),
        )
    }

    pub(super) fn sync_pane_runtimes(&mut self, size: PtySize) -> Result<(), String> {
        let desired = self.mux.pane_ids().collect::<HashSet<_>>();
        let stale = self
            .pane_runtimes
            .keys()
            .filter(|pane| !desired.contains(pane))
            .copied()
            .collect::<Vec<_>>();
        for pane in stale {
            if let Some(mut runtime) = self.pane_runtimes.remove(&pane) {
                runtime.terminate();
            }
        }

        let mut missing = desired
            .into_iter()
            .filter(|pane| !self.pane_runtimes.contains_key(pane))
            .collect::<Vec<_>>();
        missing.sort_unstable();
        for pane in missing {
            let launch = self.pending_pane_launches.get(&pane).cloned();
            let runtime = self.start_shell(pane, size, launch.as_ref())?;
            self.pending_pane_launches.remove(&pane);
            self.pane_runtimes.insert(pane, runtime);
        }
        self.flush_mux_input()?;
        if let Some(window) = self.window.clone() {
            self.resize_panes(window.inner_size(), window.scale_factor())?;
        }
        Ok(())
    }

    pub(super) fn active_terminal(&self) -> Option<&AlacrittyTerminalBackend> {
        self.mux
            .current_pane()
            .and_then(|pane| self.pane_runtimes.get(&pane))
            .map(|runtime| &runtime.terminal)
    }

    pub(super) fn active_terminal_mut(&mut self) -> Option<&mut AlacrittyTerminalBackend> {
        let pane = self.mux.current_pane()?;
        self.pane_runtimes
            .get_mut(&pane)
            .map(|runtime| &mut runtime.terminal)
    }

    pub(super) fn invalidate_active_snapshot(&mut self) {
        if let Some(pane) = self.mux.current_pane()
            && let Some(runtime) = self.pane_runtimes.get_mut(&pane)
        {
            runtime.invalidate_snapshot();
        }
    }

    pub(super) fn mark_pane_exited(&mut self, pane: PaneId, error: Option<String>) {
        if let Some(runtime) = self.pane_runtimes.get_mut(&pane) {
            runtime.pty_session = None;
            runtime.exited = true;
            runtime.title = match error {
                Some(error) => format!("Pane {} (error: {error})", pane.0),
                None => format!("Pane {} (exited)", pane.0),
            };
        }
        if self.mux.current_pane() == Some(pane)
            && let Some(window) = self.window.clone()
        {
            self.sync_active_renderer(window.scale_factor());
            window.request_redraw();
        }
    }

    pub(super) fn close_exited_pane(
        &mut self,
        event_loop: &AppControl,
        pane: PaneId,
    ) -> Result<(), String> {
        // Closing a pane also closes its PTY reader, which can leave a stale
        // EOF event in the queue. There is nothing left to reconcile then.
        if !self.pane_runtimes.contains_key(&pane) {
            return Ok(());
        }
        if self
            .mux
            .close_exited_pane(pane)
            .map_err(|error| error.to_string())?
        {
            event_loop.exit();
            return Ok(());
        }
        self.reconcile_pane_runtimes()?;
        self.deliver_runtime_events()?;
        if let Some(window) = self.window.clone() {
            self.sync_active_renderer(window.scale_factor());
            window.request_redraw();
        }
        Ok(())
    }

    pub(super) fn handle_mouse_wheel(
        &mut self,
        event_loop: &AppControl,
        window: &Window,
        delta: MouseScrollDelta,
    ) {
        let lines = match delta {
            MouseScrollDelta::LineDelta(vertical) => f64::from(vertical),
            MouseScrollDelta::PixelDelta(position) => {
                position.y / (self.cell_metrics.height * window.scale_factor()).max(1.0)
            }
        } * f64::from(self.script_snapshot.config.behavior.scroll_lines);
        self.wheel_line_accumulator += lines;
        let steps = self.wheel_line_accumulator.trunc() as i32;
        self.wheel_line_accumulator -= f64::from(steps);
        if steps == 0 {
            return;
        }

        let mode = self
            .active_terminal()
            .map(TerminalBackend::mode)
            .unwrap_or_default();
        if mode.mouse_reporting && !self.modifiers.shift_key() {
            let (column, row) = self.mouse_cell(window.scale_factor());
            let direction = if steps > 0 {
                MouseWheelDirection::Up
            } else {
                MouseWheelDirection::Down
            };
            let modifiers = key_modifiers(self.modifiers);
            let sequence = encode_mouse_wheel(direction, column, row, modifiers, mode.sgr_mouse);
            let mut bytes = Vec::with_capacity(sequence.len() * steps.unsigned_abs() as usize);
            for _ in 0..steps.unsigned_abs() {
                bytes.extend_from_slice(&sequence);
            }
            if let Err(error) = self.write_pty(&bytes) {
                self.fail(event_loop, error);
            }
        } else if mode.alternate_screen && mode.alternate_scroll && !self.modifiers.shift_key() {
            let key = if steps > 0 {
                TerminalKey::ArrowUp
            } else {
                TerminalKey::ArrowDown
            };
            let sequence = encode_key(&KeyPress::new(key, KeyModifiers::default()), mode)
                .expect("arrow keys always encode");
            let mut bytes = Vec::with_capacity(sequence.len() * steps.unsigned_abs() as usize);
            for _ in 0..steps.unsigned_abs() {
                bytes.extend_from_slice(&sequence);
            }
            if let Err(error) = self.write_pty(&bytes) {
                self.fail(event_loop, error);
            }
        } else {
            if let Some(terminal) = self.active_terminal_mut() {
                terminal.scroll_display(steps);
            }
            if let Some(pane) = self.mux.current_pane()
                && let Some(runtime) = self.pane_runtimes.get_mut(&pane)
            {
                runtime.invalidate_snapshot();
            }
            self.sync_active_renderer(window.scale_factor());
            window.request_redraw();
        }
    }

    pub(super) fn mouse_cell(&self, scale_factor: f64) -> (u16, u16) {
        let scale_factor = scale_factor.max(0.1);
        let rect = self
            .mux
            .current_pane()
            .and_then(|pane| self.pane_layout.rect(pane))
            .unwrap_or_default();
        let x = (self.mouse_position.x
            - f64::from(rect.x)
            - f64::from(self.cell_metrics.horizontal_padding) * scale_factor)
            .max(0.0);
        let y = (self.mouse_position.y
            - f64::from(rect.y)
            - f64::from(self.cell_metrics.vertical_padding) * scale_factor)
            .max(0.0);
        let column = (x / (self.cell_metrics.width * scale_factor).max(1.0)).floor() as u32;
        let row = (y / (self.cell_metrics.height * scale_factor).max(1.0)).floor() as u32;
        (
            column.min(u16::MAX.into()) as u16,
            row.min(u16::MAX.into()) as u16,
        )
    }

    pub(super) fn handle_left_mouse(&mut self, window: &Window, state: ElementState) {
        if state == ElementState::Pressed && self.visual_selection.is_some() {
            self.exit_visual_mode();
        }
        if state == ElementState::Pressed {
            if self.search_open {
                self.close_search();
                self.sync_active_renderer(window.scale_factor());
                window.request_redraw();
            }
            if self.config_error_notice.is_some()
                && self
                    .config_error_layout
                    .notice()
                    .contains(self.mouse_position.x, self.mouse_position.y)
            {
                let open_log = self
                    .config_error_layout
                    .open_log_contains(self.mouse_position.x, self.mouse_position.y);
                let dismiss = self
                    .config_error_layout
                    .dismiss_contains(self.mouse_position.x, self.mouse_position.y);
                if dismiss {
                    self.config_error_notice = None;
                } else if let Some(notice) = self.config_error_notice.as_mut()
                    && open_log
                {
                    notice.log_expanded = !notice.log_expanded;
                }
                if open_log || dismiss {
                    if let Err(error) =
                        self.resize_panes(window.inner_size(), window.scale_factor())
                    {
                        tracing::warn!(target: "toyoterm::render", %error, "resize after config notification failed");
                    }
                    self.sync_active_renderer(window.scale_factor());
                    window.request_redraw();
                }
                return;
            }
            if let Some(workspace) = self
                .workspace_layout
                .workspace_at(self.mouse_position.x, self.mouse_position.y)
            {
                if self.mux.current_workspace() != workspace
                    && let Err(error) =
                        self.dispatch_gui_command(Command::ActivateWorkspace(workspace))
                {
                    tracing::warn!(target: "toyoterm::mux", %error, %workspace, "activate workspace failed");
                }
                return;
            }
            if let Some(tab) = self
                .tab_layout
                .tab_at(self.mouse_position.x, self.mouse_position.y)
            {
                if self.mux.current_tab() != Some(tab)
                    && let Err(error) = self.dispatch_gui_command(Command::ActivateTab(tab))
                {
                    tracing::warn!(target: "toyoterm::mux", %error, %tab, "activate tab failed");
                }
                return;
            }
            let hovered = self
                .pane_layout
                .pane_at(self.mouse_position.x, self.mouse_position.y);
            let Some(hovered) = hovered else {
                return;
            };
            if self.mux.current_pane() != Some(hovered)
                && let Err(error) = self.dispatch_gui_command(Command::ActivatePane(hovered))
            {
                tracing::warn!(target: "toyoterm::mux", %error, pane = %hovered, "focus pane failed");
                return;
            }
            if has_link_modifier(self.modifiers, current_shortcut_platform()) {
                let (column, row) = self.mouse_cell(window.scale_factor());
                if let Some(url) = self
                    .active_terminal()
                    .map(TerminalBackend::snapshot)
                    .and_then(|snapshot| hyperlink_at(&snapshot, column, row))
                {
                    if let Err(error) = open_allowed_url(&url) {
                        tracing::warn!(target: "toyoterm::app", %error, %url, "open hyperlink failed");
                    }
                    return;
                }
            }
        }
        if self
            .active_terminal()
            .is_some_and(|terminal| terminal.mode().mouse_reporting)
            && !self.modifiers.shift_key()
        {
            return;
        }

        let (column, row) = self.mouse_cell(window.scale_factor());
        match state {
            ElementState::Pressed => {
                let Some(pane) = self.mux.current_pane() else {
                    return;
                };
                let click_count = self
                    .click_tracker
                    .register(Instant::now(), ClickTarget { pane, column, row });
                let kind = match click_count {
                    2 => SelectionKind::Word,
                    3 => SelectionKind::Line,
                    _ => SelectionKind::Simple,
                };
                if let Some(terminal) = self.active_terminal_mut() {
                    terminal.clear_selection();
                    terminal.start_selection(column, row, kind);
                }
                self.selecting = true;
            }
            ElementState::Released if self.selecting => {
                if let Some(terminal) = self.active_terminal_mut() {
                    terminal.update_selection(column, row);
                }
                if let Some(pane) = self.mux.current_pane()
                    && let Some(runtime) = self.pane_runtimes.get_mut(&pane)
                {
                    runtime.invalidate_snapshot();
                }
                self.selecting = false;
                if self.script_snapshot.config.behavior.copy_on_select
                    && let Err(error) = self.copy_selection()
                {
                    tracing::warn!(target: "toyoterm::app", %error, "copy-on-select failed");
                }
            }
            ElementState::Released => return,
        }
        self.sync_active_renderer(window.scale_factor());
        window.request_redraw();
    }

    pub(super) fn copy_selection(&mut self) -> Result<(), String> {
        let Some(text) = self
            .active_terminal()
            .and_then(TerminalBackend::selected_text)
            .filter(|text| !text.is_empty())
        else {
            return Ok(());
        };
        self.clipboard()?
            .set_text(text)
            .map_err(|error| format!("copy to clipboard: {error}"))
    }

    pub(super) fn paste_clipboard(&mut self) -> Result<(), String> {
        let mode = self
            .active_terminal()
            .map(TerminalBackend::mode)
            .unwrap_or_default();
        let text = self
            .clipboard()?
            .get_text()
            .map_err(|error| format!("paste from clipboard: {error}"))?;
        let bytes = encode_paste(&text, mode);
        self.write_pty(&bytes)
    }

    pub(super) fn clipboard(&mut self) -> Result<&mut Clipboard, String> {
        if self.clipboard.is_none() {
            self.clipboard =
                Some(Clipboard::new().map_err(|error| format!("initialize clipboard: {error}"))?);
        }
        Ok(self.clipboard.as_mut().expect("clipboard was initialized"))
    }
}

fn spawn_pty_reader(
    pane: PaneId,
    mut reader: Box<dyn Read + Send>,
    event_proxy: EventSender,
) -> Result<(), String> {
    thread::Builder::new()
        .name("toyoterm-pty-reader".into())
        .spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => {
                        let _ = event_proxy.send_event(AppEvent::Eof { pane });
                        break;
                    }
                    Ok(count) => {
                        if event_proxy
                            .send_event(AppEvent::Output {
                                pane,
                                bytes: buffer[..count].to_vec(),
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(error) if error.raw_os_error() == Some(5) => {
                        let _ = event_proxy.send_event(AppEvent::Eof { pane });
                        break;
                    }
                    Err(error) => {
                        let _ = event_proxy.send_event(AppEvent::Error {
                            pane,
                            message: format!("read PTY output: {error}"),
                        });
                        break;
                    }
                }
            }
        })
        .map(|_| ())
        .map_err(|error| format!("start PTY reader: {error}"))
}
