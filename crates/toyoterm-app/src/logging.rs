use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Metadata, Subscriber};

const LOG_ENV: &str = "TOYOTERM_LOG";

/// Installs toyoterm's stderr tracing subscriber.
///
/// `TOYOTERM_LOG` accepts a global level such as `debug`, or comma-separated
/// target directives such as `toyoterm::pty=trace,render=debug`.
pub fn init_logging() -> Result<(), String> {
    let filter = LogFilter::parse(std::env::var(LOG_ENV).as_deref().unwrap_or("warn"))?;
    tracing::subscriber::set_global_default(StderrSubscriber::new(filter))
        .map_err(|error| format!("initialize logging: {error}"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "error" => Some(Self::Error),
            "warn" | "warning" => Some(Self::Warn),
            "info" => Some(Self::Info),
            "debug" => Some(Self::Debug),
            "trace" => Some(Self::Trace),
            _ => None,
        }
    }

    const fn rank(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Error => 1,
            Self::Warn => 2,
            Self::Info => 3,
            Self::Debug => 4,
            Self::Trace => 5,
        }
    }

    fn enables(self, level: &Level) -> bool {
        let event_rank = match *level {
            Level::ERROR => Self::Error.rank(),
            Level::WARN => Self::Warn.rank(),
            Level::INFO => Self::Info.rank(),
            Level::DEBUG => Self::Debug.rank(),
            Level::TRACE => Self::Trace.rank(),
        };
        event_rank <= self.rank()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LogFilter {
    default: LogLevel,
    directives: Vec<(String, LogLevel)>,
}

impl LogFilter {
    fn parse(value: &str) -> Result<Self, String> {
        let mut filter = Self {
            default: LogLevel::Warn,
            directives: Vec::new(),
        };
        for directive in value
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            if let Some((target, level)) = directive.split_once('=') {
                let target = target.trim();
                if target.is_empty() {
                    return Err(format!("{LOG_ENV} contains an empty target"));
                }
                let target = normalize_target(target);
                let level = LogLevel::parse(level).ok_or_else(|| {
                    format!("{LOG_ENV} contains an invalid level `{}`", level.trim())
                })?;
                filter.directives.push((target, level));
            } else {
                filter.default = LogLevel::parse(directive).ok_or_else(|| {
                    format!("{LOG_ENV} contains an invalid directive `{directive}`")
                })?;
            }
        }
        Ok(filter)
    }

    fn enabled(&self, target: &str, level: &Level) -> bool {
        let configured = self
            .directives
            .iter()
            .filter(|(prefix, _)| target_matches(target, prefix))
            .max_by_key(|(prefix, _)| prefix.len())
            .map(|(_, level)| *level)
            .unwrap_or(self.default);
        configured.enables(level)
    }
}

fn normalize_target(target: &str) -> String {
    if target.starts_with("toyoterm") {
        target.to_owned()
    } else {
        format!("toyoterm::{target}")
    }
}

fn target_matches(target: &str, prefix: &str) -> bool {
    target == prefix
        || target
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with("::"))
}

struct StderrSubscriber {
    filter: LogFilter,
    next_span: AtomicU64,
}

impl StderrSubscriber {
    fn new(filter: LogFilter) -> Self {
        Self {
            filter,
            next_span: AtomicU64::new(1),
        }
    }
}

impl Subscriber for StderrSubscriber {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.filter.enabled(metadata.target(), metadata.level())
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(self.next_span.fetch_add(1, Ordering::Relaxed).max(1))
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let metadata = event.metadata();
        if !self.enabled(metadata) {
            return;
        }
        let mut fields = EventFields::default();
        event.record(&mut fields);
        eprintln!("{} {}: {}", metadata.level(), metadata.target(), fields);
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

#[derive(Default)]
struct EventFields {
    message: Option<String>,
    values: Vec<(String, String)>,
}

impl Visit for EventFields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}").trim_matches('"').to_owned());
        } else {
            self.values
                .push((field.name().to_owned(), format!("{value:?}")));
        }
    }
}

impl fmt::Display for EventFields {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut wrote_field = false;
        if let Some(message) = &self.message {
            formatter.write_str(message)?;
            wrote_field = true;
        }
        for (name, value) in &self.values {
            if wrote_field {
                formatter.write_str(" ")?;
            }
            write!(formatter, "{name}={value}")?;
            wrote_field = true;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_global_and_target_filters() {
        let filter = LogFilter::parse("info,pty=trace,toyoterm::render=debug").unwrap();

        assert!(filter.enabled("toyoterm::app", &Level::INFO));
        assert!(!filter.enabled("toyoterm::app", &Level::DEBUG));
        assert!(filter.enabled("toyoterm::pty::reader", &Level::TRACE));
        assert!(filter.enabled("toyoterm::render", &Level::DEBUG));
        assert!(!filter.enabled("toyoterm::render", &Level::TRACE));
    }

    #[test]
    fn rejects_invalid_filters() {
        assert!(LogFilter::parse("verbose").is_err());
        assert!(LogFilter::parse("pty=loud").is_err());
        assert!(LogFilter::parse("=debug").is_err());
    }

    #[test]
    fn off_disables_a_target() {
        let filter = LogFilter::parse("trace,script=off").unwrap();
        assert!(!filter.enabled("toyoterm::script", &Level::ERROR));
        assert!(filter.enabled("toyoterm::pty", &Level::TRACE));
    }
}
