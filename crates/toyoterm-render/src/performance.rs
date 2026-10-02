//! Opt-in CPU wall timings; no clock reads when the trace target is disabled.
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub(crate) struct Stage {
    name: &'static str,
    started: Option<Instant>,
}

impl Stage {
    pub(crate) fn new(name: &'static str) -> Self {
        Self {
            name,
            started: tracing::enabled!(target: "toyoterm::perf", tracing::Level::TRACE)
                .then(Instant::now),
        }
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        if let Some(started) = self.started {
            let duration_us = started.elapsed().as_secs_f64() * 1_000_000.0;
            let ts_ns = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            tracing::trace!(target: "toyoterm::perf", stage = self.name, duration_us, ts_ns, "stage");
        }
    }
}
