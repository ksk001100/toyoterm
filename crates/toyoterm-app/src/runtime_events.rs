use super::*;

impl ToyotermApplication {
    pub(super) fn script_context(&mut self) -> Result<ScriptContext, String> {
        let clipboard = self
            .clipboard()
            .and_then(|clipboard| {
                clipboard
                    .get_text()
                    .map_err(|error| format!("read clipboard for Ruby: {error}"))
            })
            .ok();
        Ok(ScriptContext {
            model: ruby_object_model(&self.mux, Some(&self.pane_runtimes))?,
            handles: self.mux.native_handles(),
            clipboard,
        })
    }

    pub(super) fn submit_script(&mut self, invocation: ScriptInvocation) -> Result<u64, String> {
        let id = self.next_script_request;
        self.next_script_request = self.next_script_request.wrapping_add(1).max(1);
        self.pending_script.push_back((id, invocation));
        self.start_next_script()?;
        Ok(id)
    }

    pub(super) fn start_next_script(&mut self) -> Result<(), String> {
        if self.script_in_flight {
            return Ok(());
        }
        let Some((id, invocation)) = self.pending_script.pop_front() else {
            return Ok(());
        };
        let request = ScriptRequest {
            id,
            context: self.script_context()?,
            invocation,
        };
        self.script_thread
            .submit(request)
            .map_err(|error| error.to_string())?;
        self.script_in_flight = true;
        Ok(())
    }

    pub(super) fn handle_script_completion(
        &mut self,
        completion: ScriptCompletion,
    ) -> Result<(), String> {
        let waiter = self.eval_waiters.remove(&completion.id);
        let is_reload = matches!(completion.invocation, ScriptInvocation::Reload);
        let status_position = match &completion.invocation {
            ScriptInvocation::Status { position } => Some(*position),
            _ => None,
        };
        let mut result = match completion.result {
            Ok(result) => result,
            Err(error) => {
                let message = error.to_string();
                tracing::warn!(
                    target: "toyoterm::script",
                    operation = error.operation(),
                    request_id = completion.id,
                    %error,
                    "script request failed"
                );
                if is_reload {
                    self.config_error_notice = Some(ConfigErrorNotice {
                        message: message.clone(),
                        log_expanded: false,
                    });
                }
                if let Some(position) = status_position {
                    self.status_pending = None;
                    if let Some(interval) = self
                        .script_snapshot
                        .config
                        .status_bars
                        .iter()
                        .find(|bar| bar.position == position)
                        .map(|bar| bar.interval)
                    {
                        self.next_status_at
                            .insert(position, Instant::now() + interval);
                    }
                }
                self.finish_eval(waiter, Err(message));
                return Ok(());
            }
        };

        if let Some(position) = status_position {
            self.status_pending = None;
            self.status_text
                .insert(position, result.value.take().unwrap_or_default());
            if let Some(interval) = self
                .script_snapshot
                .config
                .status_bars
                .iter()
                .find(|bar| bar.position == position)
                .map(|bar| bar.interval)
            {
                self.next_status_at
                    .insert(position, Instant::now() + interval);
            }
        }
        let value = result.value.unwrap_or_default();
        let apply_result: Result<(), String> = (|| {
            if is_reload {
                // Reload replaces the mruby VM, so its callback-owned badge state is gone too.
                self.pane_badges.clear();
            }
            if let Some(snapshot) = result.snapshot {
                self.config_error_notice = None;
                self.apply_script_snapshot(snapshot)?;
            }
            let mut reload_requested = false;
            for command in result.commands {
                match command {
                    NativeCommand::Mux(command) => {
                        command_dispatch::dispatch_coordinator_command(
                            &mut self.mux,
                            &mut self.runtime_events,
                            command,
                        )?;
                    }
                    NativeCommand::InvokeAction(NativeAction::ReloadConfig) => {
                        reload_requested = true;
                    }
                    NativeCommand::InvokeAction(action) => self.execute_native_action(action)?,
                    NativeCommand::CreateWindowWithLaunch { workspace, launch } => {
                        let pane = command_dispatch::dispatch_pane_creation(
                            &mut self.mux,
                            &mut self.runtime_events,
                            command_dispatch::PaneCreation::NewWindow(workspace),
                        )?;
                        self.pending_pane_launches.insert(pane, launch);
                    }
                    NativeCommand::NewTabWithLaunch { window, launch } => {
                        let pane = command_dispatch::dispatch_pane_creation(
                            &mut self.mux,
                            &mut self.runtime_events,
                            command_dispatch::PaneCreation::NewTab(window),
                        )?;
                        self.pending_pane_launches.insert(pane, launch);
                    }
                    NativeCommand::SplitWithLaunch {
                        pane,
                        direction,
                        launch,
                    } => {
                        let created = command_dispatch::dispatch_pane_creation(
                            &mut self.mux,
                            &mut self.runtime_events,
                            command_dispatch::PaneCreation::Split { pane, direction },
                        )?;
                        self.pending_pane_launches.insert(created, launch);
                    }
                    NativeCommand::ClipboardWrite(text) => self.pending_clipboard_writes.push(text),
                    NativeCommand::SetPaneBadge { pane, badge } => match badge {
                        Some(badge) => {
                            self.pane_badges.insert(pane, badge);
                        }
                        None => {
                            self.pane_badges.remove(&pane);
                        }
                    },
                    NativeCommand::SearchPane {
                        pane,
                        query,
                        direction,
                    } => self.search_pane(pane, query, direction)?,
                    NativeCommand::ReloadConfig => reload_requested = true,
                }
            }
            self.flush_script_clipboard_writes()?;
            self.reconcile_pane_runtimes()?;
            self.flush_mux_input()?;
            self.deliver_runtime_events()?;
            if reload_requested && !is_reload {
                self.reload_config_with_notification()?;
            }
            Ok(())
        })();
        if let Err(error) = apply_result {
            self.finish_eval(waiter, Err(error.clone()));
            return Err(error);
        }
        self.finish_eval(waiter, Ok(value));
        Ok(())
    }

    pub(super) fn finish_eval(
        &mut self,
        waiter: Option<EvalWaiter>,
        result: Result<String, String>,
    ) {
        match waiter {
            Some(EvalWaiter::Ipc(response)) => {
                let _ = response.send(result);
            }
            None => {}
        }
    }

    pub(super) fn flush_script_clipboard_writes(&mut self) -> Result<(), String> {
        if self.pending_clipboard_writes.is_empty() {
            return Ok(());
        }
        let writes = std::mem::take(&mut self.pending_clipboard_writes);
        for text in writes {
            self.clipboard()?
                .set_text(text)
                .map_err(|error| format!("write clipboard from Ruby: {error}"))?;
        }
        Ok(())
    }
}
