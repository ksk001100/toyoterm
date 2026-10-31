use std::collections::HashMap;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::rc::Rc;

use steel::rvals::{IntoSteelVal, SteelVal};
use steel::steel_vm::{engine::Engine, register_fn::RegisterFn};

use crate::ScriptError;
use crate::{PaneId, ScriptEvent, ScriptObjectModel};

fn value<T: IntoSteelVal>(value: T) -> SteelVal {
    value
        .into_steelval()
        .expect("host data has a Steel representation")
}

fn error(message: impl ToString) -> steel::rerrs::SteelErr {
    steel::rerrs::SteelErr::new(steel::rerrs::ErrorKind::Generic, message.to_string())
}

fn validate_process(
    program: &str,
    args: &[String],
    cwd: Option<&str>,
) -> Result<bool, steel::rerrs::SteelErr> {
    if program.is_empty()
        || program.contains('\0')
        || args.iter().any(|arg| arg.contains('\0'))
        || cwd.is_some_and(|cwd| cwd.is_empty() || cwd.contains('\0'))
    {
        return Err(error(
            "program/cwd must be nonempty and process values cannot contain NUL",
        ));
    }
    Ok(true)
}

#[cfg(windows)]
fn normalize_spawn_cwd(cwd: &str) -> std::borrow::Cow<'_, str> {
    let bytes = cwd.as_bytes();
    if bytes.len() >= 3
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && (bytes[2] == b':' || bytes[2] == b'|')
        && (bytes.len() == 3 || matches!(bytes[3], b'/' | b'\\'))
    {
        let stripped = &cwd[1..];
        if bytes[2] == b'|' {
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

/// The VM is constructed, used and dropped exclusively on the script thread.
/// Keep this ownership constraint even if a future Steel feature enables Send.
pub struct SteelRuntime {
    engine: Engine,
    _owner: PhantomData<Rc<()>>,
}

impl SteelRuntime {
    pub fn new() -> Result<Self, ScriptError> {
        let mut engine = Engine::new();
        engine.register_value("__host-root", value(false));
        engine.register_fn("__toyoterm-byte-length", |text: String| text.len());
        engine.register_fn("__normalize-name", |text: String| text.replace('-', "_"));
        engine.register_fn("__uppercase", |text: String| text.to_uppercase());
        engine.register_fn("__decode-utf8", |bytes: Vec<u8>| {
            String::from_utf8(bytes).map_err(error)
        });
        engine.register_fn("__nul-free?", |text: String| !text.contains('\0'));
        engine.register_fn("string-contains?", |text: String, pattern: String| {
            text.contains(&pattern)
        });
        engine.register_fn(
            "__validate-process",
            |program: String, args: Vec<String>, cwd: Option<String>| {
                validate_process(&program, &args, cwd.as_deref())
            },
        );
        engine.register_fn(
            "__validate-environment",
            |env: HashMap<String, Option<String>>| {
                if env.iter().any(|(key, value)| {
                    key.is_empty()
                        || key.contains(['=', '\0'])
                        || value.as_ref().is_some_and(|v| v.contains('\0'))
                }) {
                    Err(error("invalid launch environment key or value"))
                } else {
                    Ok(true)
                }
            },
        );
        engine.register_fn("string-starts-with?", |text: String, prefix: String| {
            text.starts_with(&prefix)
        });
        engine.register_fn("__valid-color?", |text: String| {
            crate::validate_color("color", &text).is_ok()
        });
        engine.register_fn(
            "__validate-selector",
            |title: String, items: Vec<String>| {
                let invalid = |text: &str, limit: usize| {
                    text.len() > limit || text.contains(['\0', '\n', '\r'])
                };
                if invalid(&title, 256)
                    || items.is_empty()
                    || items.len() > 4096
                    || items
                        .iter()
                        .any(|text| text.is_empty() || invalid(text, 4096))
                    || items.iter().map(String::len).sum::<usize>() > 4 * 1024 * 1024
                {
                    Err(error("invalid selector title or items"))
                } else {
                    Ok(true)
                }
            },
        );
        engine.register_fn(
            "__validate-action",
            |name: String, argument: Option<String>| {
                let takes_argument = matches!(
                    name.as_str(),
                    "split" | "activate_pane" | "move_visual_selection" | "user_command"
                );
                if takes_argument != argument.is_some() {
                    return Err(error("invalid action argument count"));
                }
                crate::decode_native_action(&name, &argument.unwrap_or_default())
                    .map(|_| true)
                    .map_err(error)
            },
        );
        engine.register_fn("__monotonic-seconds", {
            let start = std::time::Instant::now();
            move || start.elapsed().as_secs_f64()
        });
        engine.register_fn(
            "__host-spawn",
            |program: String, args: Vec<String>, cwd: Option<String>| {
                validate_process(&program, &args, cwd.as_deref())?;
                let mut command = std::process::Command::new(program);
                command.args(args);
                if let Some(cwd) = cwd {
                    #[cfg(windows)]
                    let cwd = normalize_spawn_cwd(&cwd);
                    command.current_dir(&*cwd);
                }
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    command.creation_flags(0x0800_0000);
                }
                Ok::<_, steel::rerrs::SteelErr>(match command.output() {
                    Ok(output) => value(vec![
                        value(output.stdout),
                        value(output.stderr),
                        value(output.status.code().unwrap_or(-1)),
                        value(false),
                    ]),
                    Err(cause) => value(vec![
                        value(Vec::<u8>::new()),
                        value(cause.to_string().into_bytes()),
                        value(-1_i32),
                        value(true),
                    ]),
                })
            },
        );
        engine.register_value("__platform", value(crate::platform_name().to_owned()));
        engine.register_value(
            "__primary-modifier",
            value(crate::platform_primary_modifier().to_owned()),
        );
        engine.register_value("__version", value(env!("CARGO_PKG_VERSION").to_owned()));
        engine.register_value(
            "__native-events",
            value(
                crate::ScriptEventKind::ALL
                    .iter()
                    .map(|event| event.as_str().to_owned())
                    .collect::<Vec<_>>(),
            ),
        );
        let mut runtime = Self {
            engine,
            _owner: PhantomData,
        };
        runtime.eval_with_filename(include_str!("steel_defaults.scm"), "(toyoterm defaults)")?;
        runtime.eval_with_filename(include_str!("steel_prelude.scm"), "(toyoterm prelude)")?;
        Ok(runtime)
    }

    pub fn eval(&mut self, source: &str) -> Result<String, ScriptError> {
        self.eval_with_filename(source, "(eval)")
    }

    /// Host queries are typed calls, so reading a snapshot never compiles or
    /// retains a source file for each setting on every request.
    pub(super) fn eval_host(&mut self, source: &str) -> Result<String, ScriptError> {
        if let Some(key) = source
            .strip_prefix("(config-ref '")
            .and_then(|s| s.strip_suffix(')'))
        {
            return self.invoke("config-ref", vec![value(key.to_owned())]);
        }
        if let Some(inner) = source
            .strip_prefix("(or ")
            .and_then(|s| s.strip_suffix(" \"\")"))
        {
            let result = self.eval_host(inner)?;
            return Ok(if result == "false" {
                String::new()
            } else {
                result
            });
        }
        let body = source
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .ok_or_else(|| ScriptError::new("host query", "invalid host query"))?;
        let mut words = body.split_whitespace();
        let function = words
            .next()
            .ok_or_else(|| ScriptError::new("host query", "missing function"))?;
        let mut args = Vec::new();
        for word in words {
            args.push(if let Some(symbol) = word.strip_prefix('\'') {
                SteelVal::SymbolV(symbol.into())
            } else if let Some(boolean) = word.strip_prefix('#') {
                value(boolean == "true" || boolean == "t")
            } else {
                value(
                    word.parse::<u64>()
                        .map_err(|_| ScriptError::new("host query", "invalid numeric argument"))?,
                )
            });
        }
        self.invoke(function, args)
    }

    pub(super) fn invoke(
        &mut self,
        function: &str,
        args: Vec<SteelVal>,
    ) -> Result<String, ScriptError> {
        let result = self.call(function, args)?;
        match result {
            SteelVal::StringV(_)
            | SteelVal::BoolV(_)
            | SteelVal::Void
            | SteelVal::IntV(_)
            | SteelVal::NumV(_)
            | SteelVal::BigNum(_)
            | SteelVal::Rational(_)
            | SteelVal::BigRational(_)
            | SteelVal::SymbolV(_) => Ok(Self::display_value(&result)),
            _ => Err(ScriptError::new(
                "host query",
                "host query returned a non-scalar value",
            )),
        }
    }

    pub(super) fn invoke_named(
        &mut self,
        function: &str,
        name: &str,
    ) -> Result<String, ScriptError> {
        self.invoke(function, vec![value(name.to_owned()), value(false)])
    }

    pub(super) fn invoke_selection(
        &mut self,
        id: u64,
        selection: Option<&str>,
    ) -> Result<String, ScriptError> {
        let result = self.invoke(
            "__invoke_select_callback",
            vec![value(id), value(selection.map(str::to_owned))],
        );
        if result.is_err() {
            let _ = self.eval_host(&format!("(__discard-completion 'select {id})"));
        }
        result
    }

    fn display_value(value: &SteelVal) -> String {
        match value {
            SteelVal::StringV(text) => text.to_string(),
            SteelVal::BoolV(flag) => flag.to_string(),
            SteelVal::Void => String::new(),
            value => Self::inspect_value(value, 0),
        }
    }

    // Avoid traversing closure captures through Steel 0.8.3's cycle printer,
    // which can panic for valid cyclic widget closures. Mutable values stay
    // opaque; immutable containers are inspected with a bounded nesting depth.
    fn inspect_value(value: &SteelVal, depth: usize) -> String {
        if depth >= 128 {
            return "#<depth-limit>".to_owned();
        }
        match value {
            SteelVal::ListV(values) => format!(
                "({})",
                values
                    .iter()
                    .map(|v| Self::inspect_value(v, depth + 1))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            SteelVal::VectorV(values) => format!(
                "#({})",
                values
                    .iter()
                    .map(|v| Self::inspect_value(v, depth + 1))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            SteelVal::HashMapV(values) => format!(
                "#hash({})",
                values
                    .iter()
                    .map(|(k, v)| format!(
                        "({} . {})",
                        Self::inspect_value(k, depth + 1),
                        Self::inspect_value(v, depth + 1)
                    ))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            SteelVal::StringV(_)
            | SteelVal::BoolV(_)
            | SteelVal::Void
            | SteelVal::CharV(_)
            | SteelVal::IntV(_)
            | SteelVal::NumV(_)
            | SteelVal::BigNum(_)
            | SteelVal::Complex(_)
            | SteelVal::Rational(_)
            | SteelVal::BigRational(_)
            | SteelVal::SymbolV(_)
            | SteelVal::ByteVector(_) => value.to_string(),
            SteelVal::Closure(_)
            | SteelVal::FuncV(_)
            | SteelVal::BuiltIn(_)
            | SteelVal::BoxedFunction(_)
            | SteelVal::MutFunc(_) => "#<procedure>".to_owned(),
            SteelVal::Boxed(_) | SteelVal::HeapAllocated(_) => "#<box>".to_owned(),
            _ => "#<opaque>".to_owned(),
        }
    }

    pub fn eval_with_filename(
        &mut self,
        source: &str,
        filename: &str,
    ) -> Result<String, ScriptError> {
        let path = PathBuf::from(filename);
        let result = if path.is_file() {
            self.engine
                .compile_and_run_raw_program_with_path(source.to_owned(), path)
        } else {
            self.engine.compile_and_run_raw_program(source.to_owned())
        };
        result
            .map(|values| values.last().map(Self::display_value).unwrap_or_default())
            .map_err(|error| {
                ScriptError::new(
                    "evaluate Steel",
                    format!("{filename}: {}", self.diagnostic(error)),
                )
            })
    }

    pub(crate) fn call(
        &mut self,
        name: &str,
        args: Vec<SteelVal>,
    ) -> Result<SteelVal, ScriptError> {
        let checkpoint = self.engine.extract_value("__tt-state").ok();
        let clipboard = self.engine.extract_value("__clipboard").ok();
        // Keep rollback data visible to Steel's collector, including when user
        // code explicitly collects while replacing application state.
        self.engine.update_value(
            "__host-root",
            value(
                checkpoint
                    .iter()
                    .chain(clipboard.iter())
                    .cloned()
                    .collect::<Vec<_>>(),
            ),
        );
        let result = self.engine.call_function_by_name_with_args(name, args);
        if result.is_err() {
            if let Some(state) = checkpoint {
                self.engine.update_value("__tt-state", state);
            }
            if let Some(clipboard) = clipboard {
                self.engine.update_value("__clipboard", clipboard);
            }
            let _ = self
                .engine
                .call_function_by_name_with_args("__restore_pending_tasks", Vec::new());
        }
        self.engine.update_value("__host-root", value(false));
        result.map_err(|error| ScriptError::new("call Steel", self.diagnostic(error)))
    }

    pub(super) fn eval_console(&mut self, source: &str) -> Result<String, ScriptError> {
        self.engine
            .compile_and_run_raw_program(source.to_owned())
            .map(|values| {
                values
                    .last()
                    .map(|value| Self::inspect_value(value, 0))
                    .unwrap_or_default()
            })
            .map_err(|cause| ScriptError::new("evaluate Steel", self.diagnostic(cause)))
    }

    fn diagnostic(&self, cause: steel::rerrs::SteelErr) -> String {
        self.engine
            .raise_error_to_string(cause.clone())
            .unwrap_or_else(|| cause.to_string())
    }

    pub(super) fn set_environment(&mut self) -> Result<(), ScriptError> {
        let entries = std::env::vars_os()
            .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
            .collect::<HashMap<_, _>>();
        self.engine.update_value("__environment", value(entries));
        Ok(())
    }

    pub(super) fn add_search_directory(&mut self, path: PathBuf) {
        self.engine.add_search_directory(path);
    }

    pub(super) fn set_current_pane(&mut self, pane: PaneId) -> Result<(), ScriptError> {
        self.call("__set_current_pane", vec![value(pane.0)])
            .map(|_| ())
    }

    pub(super) fn set_live_handles(
        &mut self,
        workspaces: &[u64],
        windows: &[u64],
        tabs: &[u64],
        panes: &[u64],
    ) -> Result<(), ScriptError> {
        self.call(
            "__replace_live_handles",
            vec![
                value(workspaces.to_vec()),
                value(windows.to_vec()),
                value(tabs.to_vec()),
                value(panes.to_vec()),
            ],
        )
        .map(|_| ())
    }

    pub(super) fn set_clipboard_text(&mut self, text: Option<&str>) -> Result<(), ScriptError> {
        self.engine.update_value(
            "__clipboard",
            text.map_or(value(false), |text| value(text.to_owned())),
        );
        Ok(())
    }

    pub(super) fn set_object_model(
        &mut self,
        model: &ScriptObjectModel,
    ) -> Result<(), ScriptError> {
        self.call(
            "__reset_object_model",
            vec![
                value(model.current_workspace.0),
                value(model.current_window.0),
                value(model.current_tab.0),
                value(model.current_pane.0),
            ],
        )?;
        for workspace in &model.workspaces {
            self.call(
                "__add_object",
                vec![
                    value("workspace".to_owned()),
                    value(workspace.id.0),
                    value(HashMap::from([
                        ("name".to_owned(), value(workspace.name.clone())),
                        (
                            "windows".to_owned(),
                            value(workspace.windows.iter().map(|id| id.0).collect::<Vec<_>>()),
                        ),
                    ])),
                ],
            )?;
        }
        for window in &model.windows {
            self.call(
                "__add_object",
                vec![
                    value("window".to_owned()),
                    value(window.id.0),
                    value(HashMap::from([(
                        "tabs".to_owned(),
                        value(window.tabs.iter().map(|id| id.0).collect::<Vec<_>>()),
                    )])),
                ],
            )?;
        }
        for tab in &model.tabs {
            self.call(
                "__add_object",
                vec![
                    value("tab".to_owned()),
                    value(tab.id.0),
                    value(HashMap::from([
                        ("title".to_owned(), value(tab.title.clone())),
                        (
                            "panes".to_owned(),
                            value(tab.panes.iter().map(|id| id.0).collect::<Vec<_>>()),
                        ),
                        ("zoomed?".to_owned(), value(tab.zoomed)),
                    ])),
                ],
            )?;
        }
        for pane in &model.panes {
            self.call(
                "__add_object",
                vec![
                    value("pane".to_owned()),
                    value(pane.id.0),
                    value(HashMap::from([
                        ("title".to_owned(), value(pane.title.clone())),
                        ("icon-title".to_owned(), value(pane.icon_title.clone())),
                        ("cwd".to_owned(), value(pane.cwd.clone())),
                        ("remote-host".to_owned(), value(pane.remote_host.clone())),
                        (
                            "shell-integration-version".to_owned(),
                            value(pane.shell_integration_version),
                        ),
                        (
                            "shell-integration-shell".to_owned(),
                            value(pane.shell_integration_shell.clone()),
                        ),
                        (
                            "user-vars".to_owned(),
                            value(pane.user_vars.iter().cloned().collect::<HashMap<_, _>>()),
                        ),
                        ("pid".to_owned(), value(pane.pid)),
                        ("command-running?".to_owned(), value(pane.command_running)),
                        ("last-exit-status".to_owned(), value(pane.last_exit_status)),
                        ("screen-text".to_owned(), value(pane.screen_text.clone())),
                        ("zoomed?".to_owned(), value(pane.zoomed)),
                    ])),
                ],
            )?;
        }
        Ok(())
    }

    pub(super) fn emit_event(&mut self, event: &ScriptEvent) -> Result<(), ScriptError> {
        self.call(
            "__emit_native_event",
            vec![
                value(event.name().to_owned()),
                value(HashMap::from([
                    (
                        "workspace".to_owned(),
                        value(event.workspace.map(|id| id.0)),
                    ),
                    ("window".to_owned(), value(event.window.map(|id| id.0))),
                    ("tab".to_owned(), value(event.tab.map(|id| id.0))),
                    ("pane".to_owned(), value(event.pane.map(|id| id.0))),
                    ("title".to_owned(), value(event.title.clone())),
                    ("cwd".to_owned(), value(event.cwd.clone())),
                    ("exit-status".to_owned(), value(event.exit_status)),
                    ("width".to_owned(), value(event.width)),
                    ("height".to_owned(), value(event.height)),
                    ("columns".to_owned(), value(event.columns)),
                    ("rows".to_owned(), value(event.rows)),
                ])),
            ],
        )
        .map(|_| ())
    }

    pub(super) fn invoke_async_callback(
        &mut self,
        id: u64,
        stdout: &[u8],
        stderr: &[u8],
        exit_status: i32,
        launch_error: bool,
    ) -> Result<(), ScriptError> {
        let result = self
            .call(
                "__invoke_async_callback",
                vec![
                    value(id),
                    value(stdout.to_vec()),
                    value(stderr.to_vec()),
                    value(exit_status),
                    value(launch_error),
                ],
            )
            .map(|_| ());
        if result.is_err() {
            let _ = self.eval_host(&format!("(__discard-completion 'async {id})"));
        }
        result
    }
}
