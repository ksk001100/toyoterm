use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use toyoterm_script::{ScriptInvocation, ScriptTimer};

struct ScheduledTimer {
    spec: ScriptTimer,
    deadline: Option<Instant>,
}

#[derive(Default)]
pub(super) struct Timers {
    entries: BTreeMap<u64, ScheduledTimer>,
}

impl Timers {
    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(super) fn reconcile(&mut self, specs: &[ScriptTimer], now: Instant) {
        let active: BTreeSet<_> = specs.iter().map(|spec| spec.id).collect();
        self.entries.retain(|id, _| active.contains(id));
        for spec in specs {
            self.entries
                .entry(spec.id)
                .or_insert_with(|| ScheduledTimer {
                    spec: spec.clone(),
                    deadline: now.checked_add(spec.delay),
                });
        }
    }

    pub(super) fn next_deadline(&self) -> Option<Instant> {
        self.entries
            .values()
            .filter_map(|entry| entry.deadline)
            .min()
    }

    pub(super) fn take_due(&mut self, now: Instant) -> Option<u64> {
        let id = self
            .entries
            .iter()
            .filter_map(|(id, entry)| {
                entry
                    .deadline
                    .filter(|deadline| *deadline <= now)
                    .map(|deadline| (deadline, *id))
            })
            .min()?
            .1;
        if self.entries[&id].spec.interval.is_some() {
            self.entries.get_mut(&id)?.deadline = None;
        } else {
            self.entries.remove(&id);
        }
        Some(id)
    }

    pub(super) fn complete(&mut self, id: u64, now: Instant) {
        if let Some(timer) = self.entries.get_mut(&id) {
            timer.deadline = timer
                .spec
                .interval
                .and_then(|interval| now.checked_add(interval));
        }
    }
}

impl crate::ToyotermApplication {
    pub(super) fn dispatch_due_timer(&mut self, now: Instant) -> Option<Instant> {
        // Wait for the completion wakeup while Ruby is occupied. Overdue timers
        // must not cause a busy-wait or build an unbounded callback queue.
        if self.scripting.in_flight
            || !self.scripting.pending.is_empty()
            || self.platform.window.is_none()
        {
            return None;
        }
        if let Some(id) = self.scripting.timers.take_due(now) {
            if let Err(error) = self.submit_script(ScriptInvocation::Timer { id }) {
                self.scripting.timers.complete(id, now);
                tracing::warn!(target: "toyoterm::script", %error, "submit timer failed");
            }
            return None;
        }
        self.scripting.timers.next_deadline()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn spec(id: u64, repeat: bool) -> ScriptTimer {
        ScriptTimer {
            id,
            delay: Duration::from_millis(10),
            interval: repeat.then_some(Duration::from_millis(10)),
        }
    }

    #[test]
    fn deadlines_are_monotonic_ordered_and_one_shots_are_consumed_once() {
        let now = Instant::now();
        let mut timers = Timers::default();
        timers.reconcile(&[spec(2, false), spec(1, false)], now);
        assert_eq!(
            timers.next_deadline(),
            Some(now + Duration::from_millis(10))
        );
        assert_eq!(timers.take_due(now), None);
        let due = now + Duration::from_millis(10);
        assert_eq!(timers.take_due(due), Some(1));
        assert_eq!(timers.take_due(due), Some(2));
        assert_eq!(timers.take_due(due), None);
    }

    #[test]
    fn repeat_callbacks_coalesce_until_completion_then_wait_a_full_interval() {
        let now = Instant::now();
        let mut timers = Timers::default();
        timers.reconcile(&[spec(1, true)], now);
        let late = now + Duration::from_secs(30);
        assert_eq!(timers.take_due(late), Some(1));
        assert_eq!(timers.next_deadline(), None);
        timers.reconcile(&[spec(1, true)], late);
        assert_eq!(timers.take_due(late), None);
        timers.complete(1, late);
        assert_eq!(
            timers.next_deadline(),
            Some(late + Duration::from_millis(10))
        );
        timers.reconcile(&[], late);
        timers.complete(1, late);
        assert_eq!(timers.next_deadline(), None);
    }

    #[test]
    fn unchanged_registry_updates_preserve_deadlines_and_reload_clears_them() {
        let now = Instant::now();
        let mut timers = Timers::default();
        timers.reconcile(&[spec(1, true)], now);
        timers.reconcile(&[spec(1, true)], now + Duration::from_secs(1));
        assert_eq!(
            timers.next_deadline(),
            Some(now + Duration::from_millis(10))
        );
        timers.clear();
        timers.reconcile(&[spec(1, true)], now + Duration::from_secs(2));
        assert_eq!(
            timers.next_deadline(),
            Some(now + Duration::from_millis(2010))
        );
    }

    #[test]
    fn late_repeat_failure_restores_cancellation_and_rearms_without_replaying_missed_ticks() {
        use std::sync::{Arc, mpsc};
        use toyoterm_script::{ScriptContext, ScriptRequest, ScriptThread};
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "toyoterm-timer-failure-{}-{unique}.rb",
            std::process::id()
        ));
        std::fs::write(&path, "# isolated timer failure integration\n").unwrap();
        let (sender, receiver) = mpsc::channel();
        let started = ScriptThread::start(Some(path.clone()), move |completion| {
            let _ = sender.send(completion);
        });
        std::fs::remove_file(path).unwrap();
        let (worker, _) = started.unwrap();
        let mux = toyoterm_mux::Mux::new();
        let pane = mux.current_pane().unwrap();
        let context = ScriptContext {
            model: Arc::new(crate::object_model::ruby_object_model(&mux, None).unwrap()),
            handles: mux.native_handles().into(),
            clipboard: None,
        };
        let mut sequence = 0;
        let mut invoke = |invocation| {
            sequence += 1;
            worker
                .submit(ScriptRequest {
                    id: sequence,
                    context: context.clone(),
                    invocation,
                })
                .unwrap();
            let completion = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
            assert_eq!(completion.id, sequence);
            completion.result
        };
        let registered = invoke(ScriptInvocation::Eval(
            r#"
          @ticks = 0
          @repeater = Toyoterm.every(0.01) do |timer, context|
            @ticks += 1
            timer.cancel
            context.pane.badge = @ticks.to_s
            if @ticks == 1
              Toyoterm.after(0) { raise 'failed tick leaked a task' }
              raise 'failed first tick'
            end
          end
        "#
            .into(),
        ))
        .unwrap();
        let now = Instant::now();
        let mut timers = Timers::default();
        timers.reconcile(&registered.snapshot.unwrap().timers, now);
        // Advance the scheduler's clock rather than sleeping or changing OS time.
        let late = now + Duration::from_secs(60 * 60);
        let id = timers.take_due(late).unwrap();
        let failure = invoke(ScriptInvocation::Timer { id }).unwrap_err();
        assert!(failure.message().contains("failed first tick"));
        assert_eq!(timers.take_due(late), None);
        assert_eq!(timers.next_deadline(), None);
        // Main-thread completion rearms a failed repeat, just as a success.
        let completed = late + Duration::from_secs(30);
        timers.complete(id, completed);
        let inspected = invoke(ScriptInvocation::Eval(
            "[@ticks, @repeater.active?, Toyoterm.timers.length, Toyoterm.current_pane.badge]"
                .into(),
        ))
        .unwrap();
        assert_eq!(inspected.value.as_deref(), Some("[1, true, 1, nil]"));
        assert!(inspected.commands.is_empty());
        let due = completed + Duration::from_millis(10);
        assert_eq!(timers.next_deadline(), Some(due));
        assert_eq!(timers.take_due(completed), None);
        assert_eq!(timers.take_due(due), Some(id));
        let succeeded = invoke(ScriptInvocation::Timer { id }).unwrap();
        assert_eq!(
            succeeded.commands,
            vec![toyoterm_api::NativeCommand::Ui(
                toyoterm_api::UiCommand::SetPaneBadge {
                    pane,
                    badge: Some("2".into())
                }
            )]
        );
        timers.complete(id, due);
        timers.reconcile(&succeeded.snapshot.unwrap().timers, due);
        assert_eq!(timers.take_due(due + Duration::from_secs(60 * 60)), None);
        assert_eq!(timers.next_deadline(), None);
    }

    #[test]
    fn native_deadlines_drive_the_real_script_thread_and_apply_callback_cancellation() {
        use std::sync::{Arc, mpsc};
        use toyoterm_script::{ScriptContext, ScriptRequest, ScriptThread};
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "toyoterm-timers-{}-{unique}.rb",
            std::process::id()
        ));
        std::fs::write(&path, "# timer integration\n").unwrap();
        let (sender, receiver) = mpsc::channel();
        let (worker, _) = ScriptThread::start(Some(path.clone()), move |completion| {
            let _ = sender.send(completion);
        })
        .unwrap();
        let mux = toyoterm_mux::Mux::new();
        let pane = mux.current_pane().unwrap();
        let context = ScriptContext {
            model: Arc::new(crate::object_model::ruby_object_model(&mux, None).unwrap()),
            handles: mux.native_handles().into(),
            clipboard: None,
        };
        let mut sequence = 0;
        let mut invoke = |invocation| {
            sequence += 1;
            worker
                .submit(ScriptRequest {
                    id: sequence,
                    context: context.clone(),
                    invocation,
                })
                .unwrap();
            receiver
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .result
                .unwrap()
        };
        let result = invoke(ScriptInvocation::Eval("@count = 0; Toyoterm.every(0.01) { |timer, c| @count += 1; c.pane.badge = @count.to_s; timer.cancel if @count == 2 }".into()));
        let now = Instant::now();
        let mut timers = Timers::default();
        timers.reconcile(&result.snapshot.unwrap().timers, now);
        let due = now + Duration::from_millis(10);
        let id = timers.take_due(due).unwrap();
        let result = invoke(ScriptInvocation::Timer { id });
        assert_eq!(
            result.commands,
            vec![toyoterm_api::NativeCommand::Ui(
                toyoterm_api::UiCommand::SetPaneBadge {
                    pane,
                    badge: Some("1".into())
                }
            )]
        );
        assert!(result.snapshot.is_none());
        timers.complete(id, due);
        let due = due + Duration::from_millis(10);
        let result = invoke(ScriptInvocation::Timer {
            id: timers.take_due(due).unwrap(),
        });
        timers.complete(id, due);
        timers.reconcile(&result.snapshot.unwrap().timers, due);
        assert_eq!(timers.next_deadline(), None);
        assert_eq!(
            result.commands,
            vec![toyoterm_api::NativeCommand::Ui(
                toyoterm_api::UiCommand::SetPaneBadge {
                    pane,
                    badge: Some("2".into())
                }
            )]
        );
        std::fs::remove_file(path).unwrap();
    }
}
