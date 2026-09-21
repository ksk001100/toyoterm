use super::*;

const OUTER_TERMINAL_ENVIRONMENT: &[&str] = &[
    "ITERM_SESSION_ID",
    "KITTY_LISTEN_ON",
    "KITTY_WINDOW_ID",
    "KONSOLE_DBUS_SERVICE",
    "KONSOLE_DBUS_SESSION",
    "KONSOLE_VERSION",
    "LC_TERMINAL",
    "LC_TERMINAL_VERSION",
    "TMUX",
    "TMUX_PANE",
    "WEZTERM_EXECUTABLE",
    "WEZTERM_PANE",
    "WT_SESSION",
];

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
    // A terminal started from another terminal inherits its parent's private
    // capability hints. Applications such as ratatui-image trust these hints
    // and can blacklist protocols that toyoterm supports, so do not expose
    // stale outer-terminal identity inside the new PTY.
    for key in OUTER_TERMINAL_ENVIRONMENT {
        command.env_remove(key);
    }
    command
}

impl ToyotermApplication {
    pub(super) fn update_mouse_cursor(&self, window: &Window) {
        let cursor = self
            .ui
            .pane_layout
            .pane_at(self.ui.mouse_position.x, self.ui.mouse_position.y)
            .and_then(|pane| self.terminal_runtime.pane_runtimes.get(&pane))
            .map(|runtime| runtime.protocol.mouse_cursor)
            .unwrap_or_default();
        window.set_cursor(cursor);
    }

    pub(super) fn start_shell(
        &mut self,
        pane: PaneId,
        size: PtySize,
        launch: Option<&PaneLaunchSpec>,
    ) -> Result<PaneRuntime, String> {
        let command = pty_command_for_launch(
            self.scripting.snapshot.config.default_shell.as_deref(),
            launch,
        );
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
        let mut terminal = AlacrittyTerminalBackend::with_scrollback(
            size.columns,
            size.rows,
            self.scripting.snapshot.config.scrollback_lines,
        );
        terminal.set_default_colors(
            self.ui.render_style.foreground,
            self.ui.render_style.background,
            self.ui.render_style.cursor,
            self.ui.render_style.selection,
            self.ui.render_style.ansi,
        );
        terminal.set_color_presets(terminal_color_presets(&self.scripting.snapshot)?);
        terminal.set_font_menu(
            &self.scripting.snapshot.config.font.family,
            &self.scripting.snapshot.config.font.fallback,
        );
        terminal.set_active_font_family(&self.ui.render_style.font_family);
        terminal.set_osc52_copy_enabled(self.scripting.snapshot.config.behavior.allow_osc52_copy);
        terminal.set_osc_file_download_enabled(
            self.scripting
                .snapshot
                .config
                .behavior
                .allow_osc_file_downloads
                && self
                    .scripting
                    .snapshot
                    .config
                    .behavior
                    .osc_download_directory
                    .is_some(),
        );
        terminal.set_osc_file_upload_enabled(
            self.scripting
                .snapshot
                .config
                .behavior
                .allow_osc_file_uploads
                && self
                    .scripting
                    .snapshot
                    .config
                    .behavior
                    .osc_upload_directory
                    .is_some(),
        );
        terminal.set_cell_size(
            size.pixel_width / size.columns.max(1),
            size.pixel_height / size.rows.max(1),
        );
        terminal.set_cell_scale_factor(
            self.platform
                .window
                .as_ref()
                .map_or(1.0, |window| window.scale_factor()),
        );
        Ok(PaneRuntime {
            terminal,
            process: ProcessRuntime {
                pty_session: Some(session),
                process_id,
                exited: false,
            },
            metadata: PaneMetadata {
                title: format!("Pane {}", pane.0),
                icon_title: None,
                osc_badge: None,
                cwd: launch
                    .and_then(|spec| spec.cwd.as_deref().map(PathBuf::from))
                    .or_else(|| std::env::current_dir().ok()),
                remote_host: None,
            },
            protocol: PaneProtocolState::default(),
        })
    }

    pub(super) fn flush_mux_input(&mut self) -> Result<(), String> {
        let panes = self
            .terminal_runtime
            .pane_runtimes
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for pane in panes {
            let bytes = self
                .mux
                .take_pending_input(pane)
                .map_err(|error| error.to_string())?;
            if !bytes.is_empty() {
                self.write_pane_input(pane, &bytes)?;
            }
        }
        Ok(())
    }

    pub(super) fn resize_panes(
        &mut self,
        window_size: PhysicalSize<u32>,
        scale_factor: f64,
    ) -> Result<(), String> {
        self.ui.tab_layout = self.calculate_tab_layout(window_size, scale_factor);
        self.ui.workspace_layout = self.calculate_workspace_layout(window_size, scale_factor);
        self.ui.config_error_layout = self.calculate_config_error_layout(window_size, scale_factor);
        self.ui.pane_layout = self.calculate_pane_layout(window_size, scale_factor);
        let sizes = self
            .ui
            .pane_layout
            .panes()
            .iter()
            .map(|placement| {
                (
                    placement.pane,
                    self.ui.cell_metrics.terminal_size_at_scale(
                        PhysicalSize::new(placement.rect.width, placement.rect.height),
                        scale_factor,
                    ),
                )
            })
            .collect::<Vec<_>>();
        for (pane, size) in sizes {
            if let Some(runtime) = self.terminal_runtime.pane_runtimes.get_mut(&pane) {
                runtime.terminal.resize(size.columns, size.rows);
                runtime.terminal.set_cell_size(
                    size.pixel_width / size.columns.max(1),
                    size.pixel_height / size.rows.max(1),
                );
                runtime.terminal.set_cell_scale_factor(scale_factor);
                if let Some(session) = runtime.process.pty_session.as_mut() {
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

    pub(super) fn write_input(&mut self, bytes: &[u8]) -> Result<(), String> {
        let pane = self
            .mux
            .current_pane()
            .ok_or_else(|| "mux has no current pane".to_owned())?;
        self.write_pane_input(pane, bytes)
    }

    fn write_pane_input(&mut self, pane: PaneId, bytes: &[u8]) -> Result<(), String> {
        let runtime = self
            .terminal_runtime
            .pane_runtimes
            .get_mut(&pane)
            .ok_or_else(|| format!("pane {pane} has no runtime"))?;
        reset_scroll_for_input(&mut runtime.terminal, bytes);
        self.write_pane_pty(pane, bytes)
    }

    pub(super) fn write_pane_pty(&mut self, pane: PaneId, bytes: &[u8]) -> Result<(), String> {
        let runtime = self
            .terminal_runtime
            .pane_runtimes
            .get_mut(&pane)
            .ok_or_else(|| format!("pane {pane} has no runtime"))?;
        if let Some(session) = runtime.process.pty_session.as_mut() {
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
        self.ui
            .pane_badges
            .retain(|pane, _| live_panes.contains(pane));
        self.terminal_runtime
            .pending_pane_launches
            .retain(|pane, _| live_panes.contains(pane));
        self.refresh_pane_layout();
        let size = self
            .platform
            .window
            .as_ref()
            .map(|window| {
                self.ui
                    .cell_metrics
                    .terminal_size_at_scale(window.inner_size(), window.scale_factor())
            })
            .unwrap_or_default();
        self.sync_pane_runtimes(size)
    }

    pub(super) fn refresh_pane_layout(&mut self) {
        let Some(window) = self.platform.window.as_ref() else {
            return;
        };
        self.ui.tab_layout = self.calculate_tab_layout(window.inner_size(), window.scale_factor());
        self.ui.workspace_layout =
            self.calculate_workspace_layout(window.inner_size(), window.scale_factor());
        self.ui.config_error_layout =
            self.calculate_config_error_layout(window.inner_size(), window.scale_factor());
        self.ui.pane_layout =
            self.calculate_pane_layout(window.inner_size(), window.scale_factor());
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
                workspace_bar_height(&self.scripting.snapshot.config, scale_factor),
            ),
            scaled_ui_size(
                self.scripting.snapshot.config.ui.workspace_width,
                scale_factor,
            ),
        )
    }

    pub(super) fn calculate_config_error_layout(
        &self,
        window_size: PhysicalSize<u32>,
        scale_factor: f64,
    ) -> ConfigErrorLayout {
        let Some(notice) = self.ui.config_error_notice.as_ref() else {
            return ConfigErrorLayout::default();
        };
        let y = workspace_bar_height(&self.scripting.snapshot.config, scale_factor).saturating_add(
            tab_bar_height(&self.scripting.snapshot.config, scale_factor),
        );
        let height = config_error_height(scale_factor, notice.log_expanded)
            .min(window_size.height.saturating_sub(y));
        ConfigErrorLayout::calculate(
            PaneRect::new(0, y, window_size.width, height),
            tab_bar_height(&self.scripting.snapshot.config, scale_factor),
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
                workspace_bar_height(&self.scripting.snapshot.config, scale_factor),
                window_size.width,
                tab_bar_height(&self.scripting.snapshot.config, scale_factor),
            ),
            scaled_ui_size(self.scripting.snapshot.config.ui.tab_width, scale_factor),
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
            .ui
            .config_error_notice
            .as_ref()
            .map(|notice| config_error_height(scale_factor, notice.log_expanded))
            .unwrap_or(0);
        let chrome_height = workspace_bar_height(&self.scripting.snapshot.config, scale_factor)
            .saturating_add(tab_bar_height(
                &self.scripting.snapshot.config,
                scale_factor,
            ))
            .saturating_add(notification_height)
            .min(window_size.height);
        let (pane_rect, _) = edge_bar_layout(
            window_size,
            chrome_height,
            &self.scripting.snapshot.config,
            scale_factor,
        );
        PaneLayout::calculate(
            root,
            pane_rect,
            scaled_ui_size(
                self.scripting.snapshot.config.ui.pane_divider_width,
                scale_factor,
            ),
        )
    }

    pub(super) fn sync_pane_runtimes(&mut self, size: PtySize) -> Result<(), String> {
        let desired = self.mux.pane_ids().collect::<HashSet<_>>();
        let stale = self
            .terminal_runtime
            .pane_runtimes
            .keys()
            .filter(|pane| !desired.contains(pane))
            .copied()
            .collect::<Vec<_>>();
        for pane in stale {
            if let Some(mut runtime) = self.terminal_runtime.pane_runtimes.remove(&pane) {
                runtime.terminate();
            }
            if let Err(error) = downloads::queue_upload_cancel_pane(pane) {
                tracing::warn!(target: "toyoterm::upload", %error, %pane, "queue OSC upload cleanup failed");
            }
        }

        let mut missing = desired
            .into_iter()
            .filter(|pane| !self.terminal_runtime.pane_runtimes.contains_key(pane))
            .collect::<Vec<_>>();
        missing.sort_unstable();
        for pane in missing {
            let launch = self
                .terminal_runtime
                .pending_pane_launches
                .get(&pane)
                .cloned();
            let runtime = self.start_shell(pane, size, launch.as_ref())?;
            self.terminal_runtime.pending_pane_launches.remove(&pane);
            self.terminal_runtime.pane_runtimes.insert(pane, runtime);
        }
        self.flush_mux_input()?;
        if let Some(window) = self.platform.window.clone() {
            self.resize_panes(window.inner_size(), window.scale_factor())?;
        }
        Ok(())
    }

    pub(super) fn active_terminal(&self) -> Option<&AlacrittyTerminalBackend> {
        self.mux
            .current_pane()
            .and_then(|pane| self.terminal_runtime.pane_runtimes.get(&pane))
            .map(|runtime| &runtime.terminal)
    }

    pub(super) fn active_terminal_mut(&mut self) -> Option<&mut AlacrittyTerminalBackend> {
        let pane = self.mux.current_pane()?;
        self.terminal_runtime
            .pane_runtimes
            .get_mut(&pane)
            .map(|runtime| &mut runtime.terminal)
    }

    pub(super) fn mark_pane_exited(&mut self, pane: PaneId, error: Option<String>) {
        if let Some(runtime) = self.terminal_runtime.pane_runtimes.get_mut(&pane) {
            runtime.process.pty_session = None;
            runtime.process.exited = true;
            runtime.metadata.title = match error {
                Some(error) => format!("Pane {} (error: {error})", pane.0),
                None => format!("Pane {} (exited)", pane.0),
            };
        }
        if self.mux.current_pane() == Some(pane)
            && let Some(window) = self.platform.window.clone()
        {
            self.sync_active_renderer(window.scale_factor());
            window.request_redraw();
        }
    }

    pub(super) fn close_exited_pane(
        &mut self,
        event_loop: &ActiveEventLoop,
        pane: PaneId,
    ) -> Result<(), String> {
        // Closing a pane also closes its PTY reader, which can leave a stale
        // EOF event in the queue. There is nothing left to reconcile then.
        if !self.terminal_runtime.pane_runtimes.contains_key(&pane) {
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
        if let Some(window) = self.platform.window.clone() {
            self.sync_active_renderer(window.scale_factor());
            window.request_redraw();
        }
        Ok(())
    }

    pub(super) fn handle_mouse_wheel(
        &mut self,
        event_loop: &ActiveEventLoop,
        window: &Window,
        delta: MouseScrollDelta,
    ) {
        let lines = match delta {
            MouseScrollDelta::LineDelta(_, vertical) => f64::from(vertical),
            MouseScrollDelta::PixelDelta(position) => {
                position.y / (self.ui.cell_metrics.height * window.scale_factor()).max(1.0)
            }
        } * f64::from(self.scripting.snapshot.config.behavior.scroll_lines);
        self.ui.wheel_line_accumulator += lines;
        let steps = self.ui.wheel_line_accumulator.trunc() as i32;
        self.ui.wheel_line_accumulator -= f64::from(steps);
        if steps == 0 {
            return;
        }

        let mode = self
            .active_terminal()
            .map(TerminalBackend::mode)
            .unwrap_or_default();
        if mode.mouse_reporting && !self.ui.modifiers.shift_key() {
            let (column, row) = self.clamped_mouse_cell(window.scale_factor());
            let direction = if steps > 0 {
                MouseWheelDirection::Up
            } else {
                MouseWheelDirection::Down
            };
            let modifiers = key_modifiers(self.ui.modifiers);
            let sequence = encode_mouse_wheel(direction, column, row, modifiers, mode.sgr_mouse);
            let mut bytes = Vec::with_capacity(sequence.len() * steps.unsigned_abs() as usize);
            for _ in 0..steps.unsigned_abs() {
                bytes.extend_from_slice(&sequence);
            }
            if let Err(error) = self.write_pty(&bytes) {
                self.fail(event_loop, error);
            }
        } else if mode.alternate_screen && mode.alternate_scroll && !self.ui.modifiers.shift_key() {
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
            self.sync_active_renderer(window.scale_factor());
            window.request_redraw();
        }
    }

    pub(super) fn mouse_cell(&self, scale_factor: f64) -> (u16, u16) {
        let scale_factor = scale_factor.max(0.1);
        let rect = self
            .mux
            .current_pane()
            .and_then(|pane| self.ui.pane_layout.rect(pane))
            .unwrap_or_default();
        let x = (self.ui.mouse_position.x
            - f64::from(rect.x)
            - f64::from(self.ui.cell_metrics.horizontal_padding) * scale_factor)
            .max(0.0);
        let y = (self.ui.mouse_position.y
            - f64::from(rect.y)
            - f64::from(self.ui.cell_metrics.vertical_padding) * scale_factor)
            .max(0.0);
        let column = (x / (self.ui.cell_metrics.width * scale_factor).max(1.0)).floor() as u32;
        let row = (y / (self.ui.cell_metrics.height * scale_factor).max(1.0)).floor() as u32;
        (
            column.min(u16::MAX.into()) as u16,
            row.min(u16::MAX.into()) as u16,
        )
    }

    pub(super) fn clamped_mouse_cell(&self, scale_factor: f64) -> (u16, u16) {
        let (mut column, mut row) = self.mouse_cell(scale_factor);
        if let Some(terminal) = self.active_terminal() {
            let (columns, rows) = terminal.dimensions();
            if columns > 0 {
                column = column.min(columns.saturating_sub(1));
            }
            if rows > 0 {
                row = row.min(rows.saturating_sub(1));
            }
        }
        (column, row)
    }

    pub(super) fn handle_mouse_input(
        &mut self,
        event_loop: &ActiveEventLoop,
        window: &Window,
        button: MouseButton,
        state: ElementState,
    ) {
        if button == MouseButton::Left
            && state == ElementState::Pressed
            && self.ui.visual_selection.is_some()
        {
            self.exit_visual_mode();
        }
        if button == MouseButton::Left && state == ElementState::Pressed {
            if self.ui.search_open {
                self.close_search();
                self.sync_active_renderer(window.scale_factor());
                window.request_redraw();
            }
            if self.ui.config_error_notice.is_some()
                && self
                    .ui
                    .config_error_layout
                    .notice()
                    .contains(self.ui.mouse_position.x, self.ui.mouse_position.y)
            {
                let open_log = self
                    .ui
                    .config_error_layout
                    .open_log_contains(self.ui.mouse_position.x, self.ui.mouse_position.y);
                let dismiss = self
                    .ui
                    .config_error_layout
                    .dismiss_contains(self.ui.mouse_position.x, self.ui.mouse_position.y);
                if dismiss {
                    self.ui.config_error_notice = None;
                } else if let Some(notice) = self.ui.config_error_notice.as_mut()
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
                .ui
                .workspace_layout
                .workspace_at(self.ui.mouse_position.x, self.ui.mouse_position.y)
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
                .ui
                .tab_layout
                .tab_at(self.ui.mouse_position.x, self.ui.mouse_position.y)
            {
                if self.mux.current_tab() != Some(tab)
                    && let Err(error) = self.dispatch_gui_command(Command::ActivateTab(tab))
                {
                    tracing::warn!(target: "toyoterm::mux", %error, %tab, "activate tab failed");
                }
                return;
            }
            let hovered = self
                .ui
                .pane_layout
                .pane_at(self.ui.mouse_position.x, self.ui.mouse_position.y);
            let Some(hovered) = hovered else {
                return;
            };
            if self.mux.current_pane() != Some(hovered)
                && let Err(error) = self.dispatch_gui_command(Command::ActivatePane(hovered))
            {
                tracing::warn!(target: "toyoterm::mux", %error, pane = %hovered, "focus pane failed");
                return;
            }
            if has_link_modifier(self.ui.modifiers, current_shortcut_platform()) {
                let (column, row) = self.clamped_mouse_cell(window.scale_factor());
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
        } else if state == ElementState::Pressed {
            let hovered = self
                .ui
                .pane_layout
                .pane_at(self.ui.mouse_position.x, self.ui.mouse_position.y);
            let Some(hovered) = hovered else {
                return;
            };
            if self.mux.current_pane() != Some(hovered)
                && let Err(error) = self.dispatch_gui_command(Command::ActivatePane(hovered))
            {
                tracing::warn!(target: "toyoterm::mux", %error, pane = %hovered, "focus pane failed");
                return;
            }
        }

        if self.ui.selecting {
            if state == ElementState::Released {
                let (column, row) = self.clamped_mouse_cell(window.scale_factor());
                if let Some(terminal) = self.active_terminal_mut() {
                    terminal.update_selection(column, row);
                }
                self.ui.selecting = false;
                if self.scripting.snapshot.config.behavior.copy_on_select
                    && let Err(error) = self.copy_selection()
                {
                    tracing::warn!(target: "toyoterm::app", %error, "copy-on-select failed");
                }
                self.sync_active_renderer(window.scale_factor());
                window.request_redraw();
            }
            return;
        }

        let mode = self
            .active_terminal()
            .map(TerminalBackend::mode)
            .unwrap_or_default();
        let term_button = terminal_mouse_button(button);

        if mode.mouse_reporting && !self.ui.modifiers.shift_key() {
            let (column, row) = self.clamped_mouse_cell(window.scale_factor());
            let modifiers = key_modifiers(self.ui.modifiers);
            match state {
                ElementState::Pressed => {
                    if let Some(btn) = term_button {
                        self.ui.pressed_mouse_button = Some(btn);
                        self.ui.last_mouse_cell = Some((column, row));
                        let sequence = encode_mouse_event(
                            MouseEventKind::Press(btn),
                            column,
                            row,
                            modifiers,
                            mode.sgr_mouse,
                        );
                        if let Err(error) = self.write_pty(&sequence) {
                            self.fail(event_loop, error);
                        }
                    }
                }
                ElementState::Released => {
                    let btn = term_button.or(self.ui.pressed_mouse_button);
                    self.ui.pressed_mouse_button = None;
                    if let Some(btn) = btn {
                        let sequence = encode_mouse_event(
                            MouseEventKind::Release(btn),
                            column,
                            row,
                            modifiers,
                            mode.sgr_mouse,
                        );
                        if let Err(error) = self.write_pty(&sequence) {
                            self.fail(event_loop, error);
                        }
                    }
                }
            }
            return;
        }

        if state == ElementState::Released
            && let Some(btn) = self.ui.pressed_mouse_button.take()
        {
            let (column, row) = self.clamped_mouse_cell(window.scale_factor());
            let modifiers = key_modifiers(self.ui.modifiers);
            let sequence = encode_mouse_event(
                MouseEventKind::Release(btn),
                column,
                row,
                modifiers,
                mode.sgr_mouse,
            );
            if let Err(error) = self.write_pty(&sequence) {
                self.fail(event_loop, error);
            }
            return;
        }

        if button == MouseButton::Left {
            let (column, row) = self.clamped_mouse_cell(window.scale_factor());
            match state {
                ElementState::Pressed => {
                    let Some(pane) = self.mux.current_pane() else {
                        return;
                    };
                    let click_count = self
                        .ui
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
                    self.ui.selecting = true;
                }
                ElementState::Released => return,
            }
            self.sync_active_renderer(window.scale_factor());
            window.request_redraw();
        }
    }

    pub(super) fn handle_mouse_motion(&mut self, event_loop: &ActiveEventLoop, window: &Window) {
        let mode = self
            .active_terminal()
            .map(TerminalBackend::mode)
            .unwrap_or_default();
        if !mode.mouse_reporting || self.ui.modifiers.shift_key() {
            return;
        }

        let is_drag = self.ui.pressed_mouse_button.is_some();
        let should_report = if is_drag {
            mode.mouse_drag || mode.mouse_motion
        } else {
            if self
                .ui
                .pane_layout
                .pane_at(self.ui.mouse_position.x, self.ui.mouse_position.y)
                != self.mux.current_pane()
            {
                return;
            }
            mode.mouse_motion
        };

        if !should_report {
            return;
        }

        let (column, row) = self.clamped_mouse_cell(window.scale_factor());
        if self.ui.last_mouse_cell == Some((column, row)) {
            return;
        }
        self.ui.last_mouse_cell = Some((column, row));

        let modifiers = key_modifiers(self.ui.modifiers);
        let kind = match self.ui.pressed_mouse_button {
            Some(btn) => MouseEventKind::Drag(btn),
            None => MouseEventKind::Move,
        };
        let sequence = encode_mouse_event(kind, column, row, modifiers, mode.sgr_mouse);
        if let Err(error) = self.write_pty(&sequence) {
            self.fail(event_loop, error);
        }
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

    pub(super) fn clipboard(&mut self) -> Result<&mut Clipboard, String> {
        if self.platform.clipboard.is_none() {
            self.platform.clipboard =
                Some(Clipboard::new().map_err(|error| format!("initialize clipboard: {error}"))?);
        }
        Ok(self
            .platform
            .clipboard
            .as_mut()
            .expect("clipboard was initialized"))
    }
}

pub(super) fn reset_scroll_for_input(terminal: &mut dyn TerminalBackend, bytes: &[u8]) {
    if !bytes.is_empty() {
        terminal.scroll_to_bottom();
    }
}

fn spawn_pty_reader(
    pane: PaneId,
    mut reader: Box<dyn Read + Send>,
    event_proxy: EventLoopProxy<AppEvent>,
) -> Result<(), String> {
    let pending = Arc::new(PtyOutputBuffer::default());
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
                        let pending_event = pending.clone();
                        if !pending.append(&buffer[..count], || {
                            event_proxy
                                .send_event(AppEvent::Output {
                                    pane,
                                    pending: pending_event.clone(),
                                })
                                .is_ok()
                        }) {
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

pub(super) fn terminal_mouse_button(button: MouseButton) -> Option<TerminalMouseButton> {
    match button {
        MouseButton::Left => Some(TerminalMouseButton::Left),
        MouseButton::Middle => Some(TerminalMouseButton::Middle),
        MouseButton::Right => Some(TerminalMouseButton::Right),
        _ => None,
    }
}
