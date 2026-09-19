use super::*;

const MAX_PENDING_SCRIPT_EVENTS: usize = 1_024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingScriptEnqueue {
    Queued,
    Coalesced,
    Dropped,
}

fn is_coalescible_event(kind: ScriptEventKind) -> bool {
    matches!(
        kind,
        ScriptEventKind::TitleChanged
            | ScriptEventKind::CwdChanged
            | ScriptEventKind::PaneFocused
            | ScriptEventKind::WorkspaceChanged
    )
}

fn same_event_target(left: &RubyEvent, right: &RubyEvent) -> bool {
    left.kind == right.kind
        && left.workspace == right.workspace
        && left.window == right.window
        && left.tab == right.tab
        && left.pane == right.pane
}

fn enqueue_pending_script(
    queue: &mut VecDeque<(u64, ScriptInvocation)>,
    id: u64,
    invocation: ScriptInvocation,
) -> PendingScriptEnqueue {
    let ScriptInvocation::Event(event) = invocation else {
        queue.push_back((id, invocation));
        return PendingScriptEnqueue::Queued;
    };
    if is_coalescible_event(event.kind)
        && let Some((_, ScriptInvocation::Event(queued))) = queue.iter_mut().rev().find(
            |(_, invocation)| {
                matches!(invocation, ScriptInvocation::Event(queued) if same_event_target(queued, &event))
            },
        )
    {
        *queued = event;
        return PendingScriptEnqueue::Coalesced;
    }
    let pending_events = queue
        .iter()
        .filter(|(_, invocation)| matches!(invocation, ScriptInvocation::Event(_)))
        .count();
    if pending_events < MAX_PENDING_SCRIPT_EVENTS {
        queue.push_back((id, ScriptInvocation::Event(event)));
        return PendingScriptEnqueue::Queued;
    }
    PendingScriptEnqueue::Dropped
}

impl ScriptRuntimeState {
    pub(super) fn invalidate_context(&mut self) {
        self.context_dirty = true;
    }

    fn enqueue(&mut self, invocation: ScriptInvocation) -> (u64, PendingScriptEnqueue) {
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        let outcome = enqueue_pending_script(&mut self.pending, id, invocation);
        if outcome == PendingScriptEnqueue::Dropped {
            self.event_drops = self.event_drops.saturating_add(1);
        }
        (id, outcome)
    }

    fn take_next(&mut self) -> Option<(u64, ScriptInvocation)> {
        (!self.in_flight)
            .then(|| self.pending.pop_front())
            .flatten()
    }
}

fn reuse_immutable_snapshot<T: PartialEq>(cached: &mut Option<Arc<T>>, value: T) -> (Arc<T>, bool) {
    if let Some(current) = cached.as_ref()
        && current.as_ref() == &value
    {
        return (current.clone(), true);
    }
    let value = Arc::new(value);
    *cached = Some(value.clone());
    (value, false)
}

impl ToyotermApplication {
    pub(super) fn script_context(&mut self) -> Result<ScriptContext, String> {
        let started = Instant::now();
        let clipboard = self
            .clipboard()
            .and_then(|clipboard| {
                clipboard
                    .get_text()
                    .map_err(|error| format!("read clipboard for Ruby: {error}"))
            })
            .ok();
        let (model, handles, model_reused, handles_reused) = if !self.scripting.context_dirty {
            if let (Some(model), Some(handles)) = (
                self.scripting.cached_model.as_ref(),
                self.scripting.cached_handles.as_ref(),
            ) {
                (model.clone(), handles.clone(), true, true)
            } else {
                return Err("script context cache is unexpectedly empty".to_owned());
            }
        } else {
            let value = ruby_object_model(&self.mux, Some(&self.terminal_runtime.pane_runtimes))?;
            let (model, model_reused) =
                reuse_immutable_snapshot(&mut self.scripting.cached_model, value);
            let native_handles = self.mux.native_handles();
            let handles_reused = self
                .scripting
                .cached_handles
                .as_ref()
                .is_some_and(|cached| cached.as_ref() == native_handles.as_slice());
            let handles = if handles_reused {
                self.scripting
                    .cached_handles
                    .as_ref()
                    .expect("checked cached handles")
                    .clone()
            } else {
                let handles: Arc<[NativeHandle]> = native_handles.into();
                self.scripting.cached_handles = Some(handles.clone());
                handles
            };
            self.scripting.context_dirty = false;
            (model, handles, model_reused, handles_reused)
        };
        tracing::trace!(
            target: "toyoterm::script",
            snapshot_build_us = started.elapsed().as_micros(),
            model_reused,
            handles_reused,
            pane_count = model.panes.len(),
            "built immutable script context"
        );
        Ok(ScriptContext {
            model,
            handles,
            clipboard: clipboard.map(Arc::from),
        })
    }

    pub(super) fn submit_script(&mut self, invocation: ScriptInvocation) -> Result<u64, String> {
        let event_name = match &invocation {
            ScriptInvocation::Event(event) => Some(event.name()),
            _ => None,
        };
        let (id, outcome) = self.scripting.enqueue(invocation);
        match outcome {
            PendingScriptEnqueue::Queued => {}
            PendingScriptEnqueue::Coalesced => {
                tracing::trace!(
                    target: "toyoterm::script",
                    event_name,
                    pending_script = self.scripting.pending.len(),
                    runtime_events = self.scripting.runtime_events.len(),
                    "coalesced stale queued Ruby event"
                );
            }
            PendingScriptEnqueue::Dropped => {
                if self.scripting.event_drops.is_power_of_two() {
                    tracing::warn!(
                        target: "toyoterm::script",
                        event_name,
                        dropped_events = self.scripting.event_drops,
                        pending_script = self.scripting.pending.len(),
                        runtime_events = self.scripting.runtime_events.len(),
                        max_pending_events = MAX_PENDING_SCRIPT_EVENTS,
                        "dropping Ruby event because the script queue is saturated"
                    );
                }
            }
        }
        tracing::trace!(
            target: "toyoterm::script",
            pending_script = self.scripting.pending.len(),
            runtime_events = self.scripting.runtime_events.len(),
            script_in_flight = self.scripting.in_flight,
            "script queue state"
        );
        self.start_next_script()?;
        Ok(id)
    }

    pub(super) fn start_next_script(&mut self) -> Result<(), String> {
        let Some((id, invocation)) = self.scripting.take_next() else {
            return Ok(());
        };
        let request = ScriptRequest {
            id,
            context: self.script_context()?,
            invocation,
        };
        self.scripting
            .thread
            .submit(request)
            .map_err(|error| error.to_string())?;
        self.scripting.in_flight = true;
        Ok(())
    }

    pub(super) fn handle_script_completion(
        &mut self,
        completion: ScriptCompletion,
    ) -> Result<(), String> {
        let waiter = self.scripting.eval_waiters.remove(&completion.id);
        let is_reload = matches!(completion.invocation, ScriptInvocation::Reload);
        let bar_position = match &completion.invocation {
            ScriptInvocation::Bar { position } => Some(*position),
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
                    self.ui.config_error_notice = Some(ConfigErrorNotice {
                        message: message.clone(),
                        log_expanded: false,
                    });
                }
                if let Some(position) = bar_position {
                    self.ui.bar_pending = None;
                    self.ui
                        .next_bar_at
                        .insert(position, Instant::now() + Duration::from_secs(1));
                }
                self.finish_eval(waiter, Err(message));
                return Ok(());
            }
        };

        if let Some(position) = bar_position {
            self.ui.bar_pending = None;
            self.ui
                .bar_items
                .insert(position, result.bar.take().unwrap_or_default());
            if let Some(interval) = result.bar_next_refresh {
                self.ui
                    .next_bar_at
                    .insert(position, Instant::now() + interval);
            } else {
                self.ui.next_bar_at.remove(&position);
            }
        }
        for log in std::mem::take(&mut result.logs) {
            match log.level.as_str() {
                "debug" => {
                    tracing::debug!(target: "toyoterm::script::ruby", message = %log.message)
                }
                "info" => tracing::info!(target: "toyoterm::script::ruby", message = %log.message),
                "warn" => tracing::warn!(target: "toyoterm::script::ruby", message = %log.message),
                _ => tracing::error!(target: "toyoterm::script::ruby", message = %log.message),
            }
        }
        let value = result.value.unwrap_or_default();
        let apply_result: Result<(), String> = (|| {
            if is_reload {
                // Reload replaces the mruby VM, so its callback-owned badge state is gone too.
                self.ui.pane_badges.clear();
                self.ui.selector = None;
            }
            if let Some(snapshot) = result.snapshot {
                self.ui.config_error_notice = None;
                self.apply_script_snapshot(snapshot)?;
            }
            let mut reload_requested = false;
            for command in result.commands {
                reload_requested |= self
                    .apply_control_command(command, command_dispatch::CommandOrigin::Script)?
                    .reload_config;
            }
            self.flush_script_clipboard_writes()?;
            self.reconcile_pane_runtimes()?;
            self.flush_mux_input()?;
            self.deliver_runtime_events()?;
            for request in result.async_requests {
                let proxy = self.event_proxy.clone();
                let worker_proxy = proxy.clone();
                let id = request.id;
                let program = request.program;
                let args = request.args;
                let cwd = request.cwd;
                if let Err(error) = std::thread::Builder::new()
                    .name(format!("toyoterm-async-{id}"))
                    .spawn(move || {
                        let output = execute_async_spawn(&program, &args, cwd.as_deref());
                        let _ = worker_proxy.send_event(AppEvent::AsyncCompleted { id, output });
                    })
                {
                    let output = AsyncProcessOutput {
                        stdout: Vec::new(),
                        stderr: format!("spawn async worker: {error}").into_bytes(),
                        exit_status: -1,
                        launch_error: true,
                    };
                    proxy
                        .send_event(AppEvent::AsyncCompleted { id, output })
                        .map_err(|error| format!("report async worker launch failure: {error}"))?;
                }
            }
            self.scripting
                .cancelled_async_tasks
                .extend(result.async_cancellations);
            if matches!(
                completion.invocation,
                ScriptInvocation::AsyncCallback { .. }
            ) {
                let now = Instant::now();
                for bar in &self.scripting.snapshot.config.status_bars {
                    self.ui.next_bar_at.insert(bar.position, now);
                }
            }
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
        if self.ui.pending_clipboard_writes.is_empty() {
            return Ok(());
        }
        let writes = std::mem::take(&mut self.ui.pending_clipboard_writes);
        for text in writes {
            self.clipboard()?
                .set_text(text)
                .map_err(|error| format!("write clipboard from Ruby: {error}"))?;
        }
        Ok(())
    }
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
fn normalize_spawn_cwd(cwd: &str) -> std::borrow::Cow<'_, str> {
    let bytes = cwd.as_bytes();
    if bytes.len() >= 3
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && (bytes[2] == b':' || bytes[2] == b'|')
        && (bytes.len() == 3 || bytes[3] == b'/' || bytes[3] == b'\\')
    {
        let stripped = &cwd[1..];
        if stripped.as_bytes().get(1) == Some(&b'|') {
            let mut owned = stripped.to_owned();
            owned.replace_range(1..2, ":");
            std::borrow::Cow::Owned(owned)
        } else {
            std::borrow::Cow::Borrowed(stripped)
        }
    } else {
        std::borrow::Cow::Borrowed(cwd)
    }
}

fn execute_async_spawn(program: &str, args: &[String], cwd: Option<&str>) -> AsyncProcessOutput {
    let mut command = std::process::Command::new(program);
    command.args(args);
    if let Some(cwd) = cwd {
        #[cfg(windows)]
        let cwd = normalize_spawn_cwd(cwd);
        #[cfg(windows)]
        command.current_dir(std::path::Path::new(&*cwd));
        #[cfg(not(windows))]
        command.current_dir(std::path::Path::new(cwd));
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    match command.output() {
        Ok(output) => AsyncProcessOutput {
            stdout: output.stdout,
            stderr: output.stderr,
            exit_status: output.status.code().unwrap_or(-1),
            launch_error: false,
        },
        Err(err) => AsyncProcessOutput {
            stdout: Vec::new(),
            stderr: format!("spawn {program}: {err}").into_bytes(),
            exit_status: -1,
            launch_error: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immutable_snapshot_cache_reuses_equal_values() {
        let mut cache = None;
        let (first, reused) = reuse_immutable_snapshot(&mut cache, vec![1, 2, 3]);
        assert!(!reused);
        let (second, reused) = reuse_immutable_snapshot(&mut cache, vec![1, 2, 3]);
        assert!(reused);
        assert!(Arc::ptr_eq(&first, &second));

        let (third, reused) = reuse_immutable_snapshot(&mut cache, vec![1, 2, 4]);
        assert!(!reused);
        assert!(!Arc::ptr_eq(&second, &third));
    }

    fn event(kind: ScriptEventKind, pane: u64, title: &str) -> ScriptInvocation {
        let mut event = RubyEvent::new(kind);
        event.pane = Some(PaneId(pane));
        event.title = Some(title.to_owned());
        ScriptInvocation::Event(event)
    }

    #[test]
    fn pending_event_queue_is_bounded_and_state_events_are_coalesced() {
        let mut queue = VecDeque::new();
        for id in 0..MAX_PENDING_SCRIPT_EVENTS as u64 {
            assert_eq!(
                enqueue_pending_script(&mut queue, id, event(ScriptEventKind::Bell, id, "old")),
                PendingScriptEnqueue::Queued
            );
        }
        assert_eq!(
            enqueue_pending_script(
                &mut queue,
                2_000,
                event(ScriptEventKind::Bell, 2_000, "new"),
            ),
            PendingScriptEnqueue::Dropped
        );
        assert_eq!(queue.len(), MAX_PENDING_SCRIPT_EVENTS);

        queue.pop_back();
        assert_eq!(
            enqueue_pending_script(
                &mut queue,
                3_000,
                event(ScriptEventKind::TitleChanged, 7, "old"),
            ),
            PendingScriptEnqueue::Queued
        );
        assert_eq!(
            enqueue_pending_script(
                &mut queue,
                3_001,
                event(ScriptEventKind::TitleChanged, 7, "new"),
            ),
            PendingScriptEnqueue::Coalesced
        );
        assert_eq!(queue.len(), MAX_PENDING_SCRIPT_EVENTS);
        let (_, ScriptInvocation::Event(latest)) = queue.back().unwrap() else {
            panic!("expected queued event");
        };
        assert_eq!(latest.title.as_deref(), Some("new"));
    }

    #[test]
    fn pending_event_limit_never_drops_non_event_requests() {
        let mut queue = (0..MAX_PENDING_SCRIPT_EVENTS as u64)
            .map(|id| (id, event(ScriptEventKind::Bell, id, "event")))
            .collect::<VecDeque<_>>();
        assert_eq!(
            enqueue_pending_script(&mut queue, 9_000, ScriptInvocation::Eval("42".into())),
            PendingScriptEnqueue::Queued
        );
        assert_eq!(queue.len(), MAX_PENDING_SCRIPT_EVENTS + 1);
        assert!(matches!(
            queue.back(),
            Some((9_000, ScriptInvocation::Eval(source))) if source == "42"
        ));
    }

    #[test]
    fn execute_async_spawn_captures_output_or_failure() {
        // Test non-existent command returns error and -1 status
        let output = execute_async_spawn("this-command-does-not-exist-toyoterm", &[], None);
        assert_eq!(output.exit_status, -1);
        assert!(output.launch_error);
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}
