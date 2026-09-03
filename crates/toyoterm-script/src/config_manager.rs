use super::*;

pub struct ConfigManager {
    pub(super) runtime: MrubyRuntime,
    pub(super) config: ToyotermConfig,
    pub(super) keybindings: HashSet<String>,
    pub(super) native_actions: HashMap<String, NativeAction>,
    pub(super) event_names: HashSet<String>,
    pub(super) user_command_names: HashSet<String>,
    pub(super) plugins: Vec<PluginMetadata>,
    source_path: Option<PathBuf>,
    plugin_dir: Option<PathBuf>,
}

pub(super) struct LoadedConfig {
    pub(super) runtime: MrubyRuntime,
    pub(super) config: ToyotermConfig,
    pub(super) keybindings: HashSet<String>,
    pub(super) native_actions: HashMap<String, NativeAction>,
    pub(super) event_names: HashSet<String>,
    pub(super) user_command_names: HashSet<String>,
    pub(super) plugins: Vec<PluginMetadata>,
}

impl ConfigManager {
    pub fn new() -> Result<Self, ScriptError> {
        let loaded = load_config("", "(default config)", &[], None)?;
        Ok(Self {
            runtime: loaded.runtime,
            config: loaded.config,
            keybindings: loaded.keybindings,
            native_actions: loaded.native_actions,
            event_names: loaded.event_names,
            user_command_names: loaded.user_command_names,
            plugins: loaded.plugins,
            source_path: None,
            plugin_dir: None,
        })
    }

    pub fn config(&self) -> &ToyotermConfig {
        &self.config
    }

    /// Starts a transaction for mutations made through the persistent Ruby VM.
    /// The returned value is the command queue checkpoint associated with it.
    pub(super) fn begin_config_transaction(&mut self) -> Result<String, ScriptError> {
        self.runtime.eval("Toyoterm.__begin_config_transaction")?;
        self.runtime.eval("Toyoterm.__command_checkpoint")
    }

    pub(super) fn rollback_config_transaction(
        &mut self,
        command_checkpoint: &str,
    ) -> Result<(), ScriptError> {
        self.runtime
            .eval("Toyoterm.__rollback_config_transaction")?;
        let checkpoint = command_checkpoint
            .parse::<usize>()
            .map_err(|_| ScriptError::new("rollback config", "command checkpoint is invalid"))?;
        self.runtime
            .eval(&format!("Toyoterm.__rollback_commands({checkpoint})"))?;
        Ok(())
    }

    pub(super) fn commit_config_transaction(&mut self) -> Result<(), ScriptError> {
        self.runtime.eval("Toyoterm.__commit_config_transaction")?;
        Ok(())
    }

    /// Reads and validates the live Ruby config after a persistent-VM request.
    /// A snapshot is returned only when a native setting actually changed.
    pub(super) fn refresh_config_snapshot(
        &mut self,
    ) -> Result<Option<ScriptSnapshot>, ScriptError> {
        let config = read_config(&mut self.runtime)?;
        if config == self.config {
            return Ok(None);
        }
        self.config = config;
        Ok(Some(self.snapshot()))
    }

    pub(super) fn snapshot(&self) -> ScriptSnapshot {
        ScriptSnapshot {
            config: self.config.clone(),
            native_actions: self.native_actions.clone(),
            keybindings: self.keybindings.clone(),
            event_names: self.event_names.clone(),
            user_command_names: self.user_command_names.clone(),
            plugins: self.plugins.clone(),
        }
    }

    pub fn load_startup(explicit_path: Option<&Path>) -> Result<Self, ScriptError> {
        let (manager, error) = Self::load_startup_recovering(explicit_path)?;
        match error {
            Some(error) => Err(error),
            None => Ok(manager),
        }
    }

    pub fn load_startup_recovering(
        explicit_path: Option<&Path>,
    ) -> Result<(Self, Option<ScriptError>), ScriptError> {
        let env_path = std::env::var_os("TOYOTERM_CONFIG_FILE").filter(|path| !path.is_empty());
        let home = home_directory();
        let mut manager = Self::new()?;
        manager.plugin_dir = home
            .as_deref()
            .map(|home| home.join(".config").join("toyoterm").join("plugins"));
        manager.reload_named("", "(default config)")?;
        let Some(path) = resolve_config_path(explicit_path, env_path.as_deref(), home.as_deref())
        else {
            return Ok((manager, None));
        };
        let required = explicit_path.is_some() || env_path.is_some();
        manager.source_path = Some(path.clone());
        if !required && !path.exists() {
            return Ok((manager, None));
        }
        let error = manager.reload_file().err();
        Ok((manager, error))
    }

    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
    }

    /// Reloads the selected config file, preserving the active VM on any failure.
    pub fn reload_file(&mut self) -> Result<&ToyotermConfig, ScriptError> {
        let path = self.source_path.clone().ok_or_else(|| {
            ScriptError::new("reload config", "no configuration path is available")
        })?;
        tracing::debug!(target: "toyoterm::config", path = %path.display(), "load config");
        let source = std::fs::read_to_string(&path)
            .map_err(|error| ScriptError::config_file(&path, error))?;
        self.reload_named(&source, &path.display().to_string())
            .map_err(|error| ScriptError::config_file(&path, error))
    }

    /// Evaluate config in a fresh VM and swap it in only after complete validation.
    pub fn reload(&mut self, source: &str) -> Result<&ToyotermConfig, ScriptError> {
        self.reload_named(source, "(config)")
    }

    fn reload_named(
        &mut self,
        source: &str,
        filename: &str,
    ) -> Result<&ToyotermConfig, ScriptError> {
        let plugin_paths = self
            .plugin_dir
            .as_deref()
            .map(discover_plugins)
            .unwrap_or_default();
        let source_dir = self.source_path.as_deref().and_then(Path::parent);
        let loaded = load_config(source, filename, &plugin_paths, source_dir)?;
        self.runtime = loaded.runtime;
        self.config = loaded.config;
        self.keybindings = loaded.keybindings;
        self.native_actions = loaded.native_actions;
        self.event_names = loaded.event_names;
        self.user_command_names = loaded.user_command_names;
        self.plugins = loaded.plugins;
        tracing::info!(target: "toyoterm::config", filename, "config loaded");
        Ok(&self.config)
    }

    pub fn eval(&mut self, source: &str) -> Result<String, ScriptError> {
        self.runtime.eval(source)
    }

    pub fn plugins(&self) -> &[PluginMetadata] {
        &self.plugins
    }

    /// Evaluates interactive Ruby and returns the value's `inspect` representation.
    pub fn eval_inspect(&mut self, source: &str) -> Result<String, ScriptError> {
        let checkpoint = self.runtime.eval("Toyoterm.__command_checkpoint")?;
        let result = self.runtime.eval_with_filename(
            &format!("(begin\n{source}\nend).inspect"),
            "(toyoterm ruby console)",
        );
        if result.is_err() {
            let _ = self
                .runtime
                .eval(&format!("Toyoterm.__rollback_commands({checkpoint})"));
        }
        result
    }

    pub fn native_action(&self, key: &str) -> Option<NativeAction> {
        self.native_actions.get(&key.to_uppercase()).cloned()
    }

    pub fn has_dynamic_keybinding(&self, key: &str) -> bool {
        self.keybindings.contains(&key.to_uppercase())
    }

    pub fn user_command_names(&self) -> impl Iterator<Item = &str> {
        self.user_command_names.iter().map(String::as_str)
    }

    pub fn trigger_user_command(
        &mut self,
        name: &str,
        current_pane: PaneId,
    ) -> Result<bool, ScriptError> {
        if !self.user_command_names.contains(name) {
            return Err(ScriptError::new(
                "invoke user command",
                format!("undefined user command: {name}"),
            ));
        }
        self.set_current_pane(current_pane)?;
        let source = format!(
            "Toyoterm.__invoke_command({}, Toyoterm.current_pane)",
            ruby_string_literal(name)
        );
        match self.eval_callback(CallbackKind::UserCommand, name, &source)? {
            value if value == "true" => Ok(true),
            _ => Err(ScriptError::new(
                "invoke user command",
                "callback returned an invalid state",
            )),
        }
    }

    pub fn render_status(&mut self) -> Result<String, ScriptError> {
        self.eval_callback(CallbackKind::Status, "status", "Toyoterm.__invoke_status")
    }

    /// Updates the pane exposed by `Toyoterm.current_pane` for subsequent evaluations.
    pub fn set_current_pane(&mut self, pane: PaneId) -> Result<(), ScriptError> {
        self.runtime.set_current_pane(pane)
    }

    pub fn set_live_handles(
        &mut self,
        handles: impl IntoIterator<Item = NativeHandle>,
    ) -> Result<(), ScriptError> {
        let mut workspaces = Vec::new();
        let mut windows = Vec::new();
        let mut tabs = Vec::new();
        let mut panes = Vec::new();
        for handle in handles {
            match handle.kind() {
                HandleKind::Workspace => workspaces.push(handle.id()),
                HandleKind::Window => windows.push(handle.id()),
                HandleKind::Tab => tabs.push(handle.id()),
                HandleKind::Pane => panes.push(handle.id()),
            }
        }
        workspaces.sort_unstable();
        windows.sort_unstable();
        tabs.sort_unstable();
        panes.sort_unstable();
        self.runtime
            .set_live_handles(&workspaces, &windows, &tabs, &panes)
    }

    pub fn set_object_model(&mut self, model: &RubyObjectModel) -> Result<(), ScriptError> {
        self.runtime.set_object_model(model)
    }

    /// Updates the clipboard snapshot exposed to the next Ruby callback.
    pub fn set_clipboard_text(&mut self, text: Option<&str>) -> Result<(), ScriptError> {
        self.runtime.set_clipboard_text(text)
    }

    /// Runs a configured callback only when the native key resolver found a match.
    pub fn trigger_keybinding(
        &mut self,
        key: &str,
        current_pane: PaneId,
    ) -> Result<bool, ScriptError> {
        let key = key.to_uppercase();
        if !self.keybindings.contains(&key) {
            return Ok(false);
        }
        self.set_current_pane(current_pane)?;
        let callback_name = key;
        let key = ruby_string_literal(&callback_name);
        let source = format!("Toyoterm.__config.__trigger_binding({key}, Toyoterm.current_pane)");
        match self.eval_callback(CallbackKind::KeyBinding, &callback_name, &source)? {
            value if value == "true" => Ok(true),
            value if value == "false" => Ok(false),
            _ => Err(ScriptError::new(
                "evaluate key binding",
                "callback returned an invalid match state",
            )),
        }
    }

    /// Emits an event only when Ruby registered at least one handler for it.
    pub fn emit_event(&mut self, name: &str, current_pane: PaneId) -> Result<bool, ScriptError> {
        if !self.event_names.contains(name) {
            return Ok(false);
        }
        self.set_current_pane(current_pane)?;
        let callback_name = name;
        let name = ruby_string_literal(callback_name);
        let source = format!("Toyoterm.__emit_event({name}, Toyoterm.current_pane)");
        match self.eval_callback(CallbackKind::Event, callback_name, &source)? {
            value if value == "true" => Ok(true),
            value if value == "false" => Ok(false),
            _ => Err(ScriptError::new(
                "emit mruby event",
                "event handler returned an invalid state",
            )),
        }
    }

    pub fn emit_native_event(&mut self, event: &RubyEvent) -> Result<bool, ScriptError> {
        if !self.event_names.contains(event.name) {
            return Ok(false);
        }
        let started = Instant::now();
        let result = self.runtime.emit_event(event);
        record_callback_duration(
            CallbackKind::Event,
            event.name,
            started.elapsed(),
            result.is_ok(),
        );
        result.map(|()| true)
    }

    fn eval_callback(
        &mut self,
        kind: CallbackKind,
        name: &str,
        source: &str,
    ) -> Result<String, ScriptError> {
        let started = Instant::now();
        let result = self.runtime.eval(source);
        record_callback_duration(kind, name, started.elapsed(), result.is_ok());
        result
    }

    /// Converts commands queued by Ruby into the native command API.
    ///
    /// Pane id zero is a bootstrap placeholder used while startup config is loading.
    pub fn drain_commands(
        &mut self,
        current_pane: PaneId,
    ) -> Result<Vec<NativeCommand>, ScriptError> {
        self.drain_commands_with_context(WorkspaceId(0), WindowId(0), TabId(0), current_pane)
    }

    pub fn drain_commands_with_context(
        &mut self,
        current_workspace: WorkspaceId,
        current_window: WindowId,
        current_tab: TabId,
        current_pane: PaneId,
    ) -> Result<Vec<NativeCommand>, ScriptError> {
        let mut commands = Vec::new();
        loop {
            let command_type = self.runtime.eval("Toyoterm.__next_command")?;
            if command_type.is_empty() {
                break;
            }

            let raw_id = self
                .runtime
                .eval("Toyoterm.__current_command_pane")?
                .parse::<u64>()
                .map_err(|_| ScriptError::new("decode mruby command", "handle id is invalid"))?;
            let pane = if raw_id == 0 {
                current_pane
            } else {
                PaneId(raw_id)
            };
            let payload = self.runtime.eval("Toyoterm.__current_command_payload")?;
            match command_type.as_str() {
                "send_text" => commands.push(NativeCommand::Mux(Command::SendText {
                    pane,
                    text: payload,
                })),
                "split" => commands.push(NativeCommand::Mux(Command::Split {
                    pane,
                    direction: parse_direction(&payload)?,
                })),
                "split_with_launch" => commands.push(NativeCommand::SplitWithLaunch {
                    pane,
                    direction: parse_direction(&payload)?,
                    launch: self.read_current_launch_spec()?,
                }),
                "close_pane" => commands.push(NativeCommand::Mux(Command::ClosePane(pane))),
                "activate_pane" => commands.push(NativeCommand::Mux(Command::ActivatePane(pane))),
                "close_tab" => commands.push(NativeCommand::Mux(Command::CloseTab(TabId(
                    resolve_bootstrap_id(raw_id, current_tab.0),
                )))),
                "activate_tab" => commands.push(NativeCommand::Mux(Command::ActivateTab(TabId(
                    resolve_bootstrap_id(raw_id, current_tab.0),
                )))),
                "new_tab" => commands.push(NativeCommand::Mux(Command::NewTabIn(WindowId(
                    resolve_bootstrap_id(raw_id, current_window.0),
                )))),
                "new_tab_with_launch" => commands.push(NativeCommand::NewTabWithLaunch {
                    window: WindowId(resolve_bootstrap_id(raw_id, current_window.0)),
                    launch: self.read_current_launch_spec()?,
                }),
                "close_window" => commands.push(NativeCommand::Mux(Command::CloseWindow(
                    WindowId(resolve_bootstrap_id(raw_id, current_window.0)),
                ))),
                "activate_window" => commands.push(NativeCommand::Mux(Command::ActivateWindow(
                    WindowId(resolve_bootstrap_id(raw_id, current_window.0)),
                ))),
                "activate_workspace" => {
                    commands.push(NativeCommand::Mux(Command::ActivateWorkspace(WorkspaceId(
                        resolve_bootstrap_id(raw_id, current_workspace.0),
                    ))))
                }
                "switch_workspace" => {
                    commands.push(NativeCommand::Mux(Command::SwitchWorkspace(payload)))
                }
                "create_window" => commands.push(NativeCommand::Mux(Command::CreateWindow(
                    WorkspaceId(resolve_bootstrap_id(raw_id, current_workspace.0)),
                ))),
                "create_window_with_launch" => {
                    commands.push(NativeCommand::CreateWindowWithLaunch {
                        workspace: WorkspaceId(resolve_bootstrap_id(raw_id, current_workspace.0)),
                        launch: self.read_current_launch_spec()?,
                    })
                }
                "clipboard_write" => commands.push(NativeCommand::ClipboardWrite(payload)),
                "set_pane_badge" => commands.push(NativeCommand::SetPaneBadge {
                    pane,
                    badge: Some(payload),
                }),
                "clear_pane_badge" => {
                    commands.push(NativeCommand::SetPaneBadge { pane, badge: None })
                }
                "search_pane" => {
                    let direction = match self
                        .runtime
                        .eval("Toyoterm.__current_command_search_direction")?
                        .as_str()
                    {
                        "next" => PaneSearchDirection::Next,
                        "previous" => PaneSearchDirection::Previous,
                        other => {
                            return Err(ScriptError::new(
                                "decode mruby command",
                                format!("unsupported search direction {other}"),
                            ));
                        }
                    };
                    commands.push(NativeCommand::SearchPane {
                        pane,
                        query: payload,
                        direction,
                    });
                }
                "reload_config" => commands.push(NativeCommand::ReloadConfig),
                other => {
                    return Err(ScriptError::new(
                        "decode mruby command",
                        format!("unsupported command {other}"),
                    ));
                }
            }
        }
        Ok(commands)
    }

    fn read_current_launch_spec(&mut self) -> Result<PaneLaunchSpec, ScriptError> {
        let program = self
            .ruby_bool("Toyoterm.__current_launch_has_program")?
            .then(|| self.runtime.eval("Toyoterm.__current_launch_program"))
            .transpose()?;
        let arg_count = self.ruby_usize("Toyoterm.__current_launch_arg_count")?;
        let mut args = Vec::with_capacity(arg_count);
        for index in 0..arg_count {
            args.push(
                self.runtime
                    .eval(&format!("Toyoterm.__current_launch_arg({index})"))?,
            );
        }
        let cwd = self
            .ruby_bool("Toyoterm.__current_launch_has_cwd")?
            .then(|| self.runtime.eval("Toyoterm.__current_launch_cwd"))
            .transpose()?;
        let env_count = self.ruby_usize("Toyoterm.__current_launch_env_count")?;
        let mut environment = Vec::with_capacity(env_count);
        for index in 0..env_count {
            let key = self
                .runtime
                .eval(&format!("Toyoterm.__current_launch_env_key({index})"))?;
            let value = if self.ruby_bool(&format!(
                "Toyoterm.__current_launch_env_value_is_nil({index})"
            ))? {
                None
            } else {
                Some(
                    self.runtime
                        .eval(&format!("Toyoterm.__current_launch_env_value({index})"))?,
                )
            };
            environment.push((key, value));
        }
        Ok(PaneLaunchSpec {
            program,
            args,
            cwd,
            environment,
        })
    }

    fn ruby_bool(&mut self, source: &str) -> Result<bool, ScriptError> {
        match self.runtime.eval(source)?.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(ScriptError::new(
                "decode mruby command",
                "launch boolean is invalid",
            )),
        }
    }

    fn ruby_usize(&mut self, source: &str) -> Result<usize, ScriptError> {
        self.runtime
            .eval(source)?
            .parse()
            .map_err(|_| ScriptError::new("decode mruby command", "launch count is invalid"))
    }
}

pub(super) fn run_script_request(
    manager: &mut ConfigManager,
    context: &ScriptContext,
    invocation: &ScriptInvocation,
) -> Result<ScriptResult, ScriptError> {
    manager.set_live_handles(context.handles.iter().copied())?;
    manager.set_object_model(&context.model)?;
    manager.set_clipboard_text(context.clipboard.as_deref())?;

    let command_checkpoint = if matches!(invocation, ScriptInvocation::Reload) {
        None
    } else {
        Some(manager.begin_config_transaction()?)
    };
    let request_result: Result<(Option<String>, Option<ScriptSnapshot>), ScriptError> = (|| {
        Ok(match invocation {
            ScriptInvocation::DrainStartup => (None, None),
            ScriptInvocation::KeyBinding { key, pane } => {
                manager.trigger_keybinding(key, *pane)?;
                (None, None)
            }
            ScriptInvocation::UserCommand { name, pane } => {
                manager.trigger_user_command(name, *pane)?;
                (None, None)
            }
            ScriptInvocation::Event(event) => {
                manager.emit_native_event(event)?;
                (None, None)
            }
            ScriptInvocation::Eval(source) => (Some(manager.eval_inspect(source)?), None),
            ScriptInvocation::Reload => {
                manager.reload_file()?;
                (None, Some(manager.snapshot()))
            }
            ScriptInvocation::Status => (Some(manager.render_status()?), None),
        })
    })();
    let (value, mut snapshot) = match request_result {
        Ok(result) => result,
        Err(error) => {
            if let Some(checkpoint) = command_checkpoint.as_deref() {
                manager.rollback_config_transaction(checkpoint)?;
            }
            return Err(error);
        }
    };
    if let Some(checkpoint) = command_checkpoint.as_deref() {
        match manager.refresh_config_snapshot() {
            Ok(changed) => {
                snapshot = changed;
                manager.commit_config_transaction()?;
            }
            Err(error) => {
                manager.rollback_config_transaction(checkpoint)?;
                return Err(error);
            }
        }
    }
    let model = &context.model;
    let commands = manager.drain_commands_with_context(
        model.current_workspace,
        model.current_window,
        model.current_tab,
        model.current_pane,
    )?;
    Ok(ScriptResult {
        value,
        commands,
        snapshot,
    })
}

const fn resolve_bootstrap_id(id: u64, current: u64) -> u64 {
    if id == 0 { current } else { id }
}

fn record_callback_duration(kind: CallbackKind, name: &str, elapsed: Duration, succeeded: bool) {
    let duration_ms = elapsed.as_secs_f64() * 1_000.0;
    if is_slow_callback(elapsed) {
        tracing::warn!(
            target: "toyoterm::script",
            callback_kind = kind.as_str(),
            callback_name = name,
            duration_ms,
            threshold_ms = SLOW_CALLBACK_THRESHOLD.as_millis() as u64,
            succeeded,
            "slow Ruby callback"
        );
    } else {
        tracing::debug!(
            target: "toyoterm::script",
            callback_kind = kind.as_str(),
            callback_name = name,
            duration_ms,
            succeeded,
            "Ruby callback completed"
        );
    }
}

pub(super) fn is_slow_callback(elapsed: Duration) -> bool {
    elapsed >= SLOW_CALLBACK_THRESHOLD
}

pub(super) fn resolve_config_path(
    explicit_path: Option<&Path>,
    env_path: Option<&std::ffi::OsStr>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    explicit_path
        .map(Path::to_owned)
        .or_else(|| env_path.map(PathBuf::from))
        .or_else(|| home.map(|home| home.join(".config").join("toyoterm").join("config.rb")))
}

pub(super) fn load_config(
    source: &str,
    filename: &str,
    plugin_paths: &[PathBuf],
    source_dir: Option<&Path>,
) -> Result<LoadedConfig, ScriptError> {
    let mut runtime = MrubyRuntime::new()?;
    let config_dsl = CONFIG_DSL
        .replace("__TOYOTERM_PRIMARY_MODIFIER__", platform_primary_modifier())
        .replace("__TOYOTERM_PLATFORM__", platform_name());
    runtime.eval_with_filename(&config_dsl, "(toyoterm DSL)")?;
    // SAFETY: The DSL has created the Toyoterm module in this exclusively owned VM.
    unsafe { toyoterm_mruby_install_host_api(runtime.state.as_ptr()) };
    runtime.set_environment()?;
    runtime.eval_with_filename(source, filename)?;
    let plugins = load_plugins(&mut runtime, plugin_paths, source_dir);

    let config = read_config(&mut runtime)?;
    let binding_count = runtime
        .eval("Toyoterm.__config.__binding_count")?
        .parse::<usize>()
        .map_err(|_| ScriptError::new("load key bindings", "binding count is invalid"))?;
    let mut keybindings = HashSet::with_capacity(binding_count);
    for index in 0..binding_count {
        keybindings.insert(runtime.eval(&format!("Toyoterm.__config.__binding_key({index})"))?);
    }

    let static_count = runtime
        .eval("Toyoterm.__config.__static_binding_count")?
        .parse::<usize>()
        .map_err(|_| ScriptError::new("load key bindings", "static binding count is invalid"))?;
    let mut native_actions = HashMap::with_capacity(static_count);
    for index in 0..static_count {
        let key = runtime.eval(&format!("Toyoterm.__config.__static_binding_key({index})"))?;
        let action = runtime.eval(&format!(
            "Toyoterm.__config.__static_binding_action({index})"
        ))?;
        let argument = runtime.eval(&format!(
            "Toyoterm.__config.__static_binding_argument({index})"
        ))?;
        native_actions.insert(key, decode_native_action(&action, &argument)?);
    }

    let event_count = runtime
        .eval("Toyoterm.__event_count")?
        .parse::<usize>()
        .map_err(|_| ScriptError::new("load events", "event count is invalid"))?;
    let mut event_names = HashSet::with_capacity(event_count);
    for index in 0..event_count {
        event_names.insert(runtime.eval(&format!("Toyoterm.__event_name({index})"))?);
    }

    let user_command_count = runtime
        .eval("Toyoterm.__command_count")?
        .parse::<usize>()
        .map_err(|_| ScriptError::new("load user commands", "command count is invalid"))?;
    let mut user_command_names = HashSet::with_capacity(user_command_count);
    for index in 0..user_command_count {
        user_command_names.insert(runtime.eval(&format!("Toyoterm.__command_name({index})"))?);
    }

    Ok(LoadedConfig {
        runtime,
        config,
        keybindings,
        native_actions,
        event_names,
        user_command_names,
        plugins,
    })
}

fn read_config(runtime: &mut MrubyRuntime) -> Result<ToyotermConfig, ScriptError> {
    runtime.eval("Toyoterm.__validate_theme!")?;
    let defaults = ToyotermConfig::default();
    let family = runtime.eval("Toyoterm.__config.font.family")?;
    if family.trim().is_empty() {
        return Err(ScriptError::new(
            "validate config",
            "font family cannot be empty",
        ));
    }
    let fallback_count = runtime
        .eval("Toyoterm.__config.font.__fallback_count")?
        .parse::<usize>()
        .map_err(|_| {
            ScriptError::new("validate config", "font fallback count must be an integer")
        })?;
    if fallback_count > 32 {
        return Err(ScriptError::new(
            "validate config",
            "font fallback supports at most 32 families",
        ));
    }
    let mut fallback = Vec::with_capacity(fallback_count);
    for index in 0..fallback_count {
        let fallback_family =
            runtime.eval(&format!("Toyoterm.__config.font.__fallback_at({index})"))?;
        if fallback_family.trim().is_empty() {
            return Err(ScriptError::new(
                "validate config",
                "font fallback entries cannot be empty",
            ));
        }
        if fallback_family == family || fallback.contains(&fallback_family) {
            return Err(ScriptError::new(
                "validate config",
                format!("duplicate font family in fallback: {fallback_family}"),
            ));
        }
        fallback.push(fallback_family);
    }
    let font_size = parse_positive_f32("font size", &runtime.eval("Toyoterm.__config.font.size")?)?;
    let font_weight = runtime
        .eval("Toyoterm.__config.font.weight")?
        .parse::<u16>()
        .map_err(|_| ScriptError::new("validate config", "font weight must be an integer"))?;
    if !(1..=1000).contains(&font_weight) {
        return Err(ScriptError::new(
            "validate config",
            "font weight must be between 1 and 1000",
        ));
    }
    let opacity = parse_f32(
        "window opacity",
        &runtime.eval("Toyoterm.__config.window.opacity")?,
    )?;
    if !(0.0..=1.0).contains(&opacity) {
        return Err(ScriptError::new(
            "validate config",
            "window opacity must be between 0 and 1",
        ));
    }
    let number = |runtime: &mut MrubyRuntime, field: &str, ruby: &str| {
        parse_positive_f32(field, &runtime.eval(ruby)?)
    };
    let nonnegative = |runtime: &mut MrubyRuntime, field: &str, ruby: &str| {
        parse_nonnegative_f32(field, &runtime.eval(ruby)?)
    };
    let boolean = |runtime: &mut MrubyRuntime, field: &str, ruby: &str| match runtime
        .eval(&format!(
            "({ruby}) == true ? 'true' : (({ruby}) == false ? 'false' : 'invalid')"
        ))?
        .as_str()
    {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ScriptError::new(
            "validate config",
            format!("{field} must be true or false"),
        )),
    };
    let scrollback_lines = runtime
        .eval("Toyoterm.__config.scrollback_lines")?
        .parse::<usize>()
        .map_err(|_| ScriptError::new("validate config", "scrollback_lines must be an integer"))?;
    let default_shell = runtime.eval("Toyoterm.__config.default_shell")?;
    let leader_key = runtime.eval("Toyoterm.__config.__leader_key")?;
    let leader = if leader_key.is_empty() {
        None
    } else {
        let timeout_ms = runtime
            .eval("Toyoterm.__config.__leader_timeout")?
            .parse::<u64>()
            .map_err(|_| {
                ScriptError::new("validate config", "leader timeout must be an integer")
            })?;
        if timeout_ms == 0 {
            return Err(ScriptError::new(
                "validate config",
                "leader timeout must be positive",
            ));
        }
        Some(LeaderConfig {
            key: leader_key,
            timeout_ms,
        })
    };
    let status_interval = match runtime.eval("Toyoterm.__status_interval")?.as_str() {
        "" => None,
        value => {
            let seconds = value.parse::<f64>().map_err(|_| {
                ScriptError::new("validate status", "status interval must be numeric")
            })?;
            if !seconds.is_finite() || seconds < 0.1 {
                return Err(ScriptError::new(
                    "validate status",
                    "status interval must be at least 0.1 seconds",
                ));
            }
            Some(Duration::from_secs_f64(seconds))
        }
    };

    let ansi_count = runtime
        .eval("Toyoterm.__config.colors.__ansi_count")?
        .parse::<usize>()
        .map_err(|_| ScriptError::new("validate config", "colors.ansi length is invalid"))?;
    if ansi_count != 16 {
        return Err(ScriptError::new(
            "validate config",
            format!("colors.ansi must contain exactly 16 colors, got {ansi_count}"),
        ));
    }
    let mut ansi = Vec::with_capacity(ansi_count);
    for index in 0..ansi_count {
        ansi.push(runtime.eval(&format!("Toyoterm.__config.colors.__ansi_at({index})"))?);
    }

    let config = ToyotermConfig {
        font: FontConfig {
            family,
            fallback,
            size: font_size,
            weight: font_weight,
        },
        colors: ColorConfig {
            background: runtime.eval("Toyoterm.__config.colors.background")?,
            foreground: runtime.eval("Toyoterm.__config.colors.foreground")?,
            cursor: runtime.eval("Toyoterm.__config.colors.cursor")?,
            selection: runtime.eval("Toyoterm.__config.colors.selection")?,
            ansi,
            tab_bar: runtime.eval("Toyoterm.__config.colors.tab_bar")?,
            tab_active: runtime.eval("Toyoterm.__config.colors.tab_active")?,
            tab_inactive: runtime.eval("Toyoterm.__config.colors.tab_inactive")?,
            workspace_bar: runtime.eval("Toyoterm.__config.colors.workspace_bar")?,
            status_bar: runtime.eval("Toyoterm.__config.colors.status_bar")?,
            pane_border: runtime.eval("Toyoterm.__config.colors.pane_border")?,
            search_match: runtime.eval("Toyoterm.__config.colors.search_match")?,
            search_match_active: runtime.eval("Toyoterm.__config.colors.search_match_active")?,
        },
        ui: UiConfig {
            padding_x: nonnegative(runtime, "ui.padding_x", "Toyoterm.__config.ui.padding_x")?,
            padding_y: nonnegative(runtime, "ui.padding_y", "Toyoterm.__config.ui.padding_y")?,
            line_height: number(
                runtime,
                "ui.line_height",
                "Toyoterm.__config.ui.line_height",
            )?,
            tab_bar: boolean(runtime, "ui.tab_bar", "Toyoterm.__config.ui.tab_bar")?,
            tab_bar_height: number(
                runtime,
                "ui.tab_bar_height",
                "Toyoterm.__config.ui.tab_bar_height",
            )?,
            tab_width: number(runtime, "ui.tab_width", "Toyoterm.__config.ui.tab_width")?,
            workspace_bar: boolean(
                runtime,
                "ui.workspace_bar",
                "Toyoterm.__config.ui.workspace_bar",
            )?,
            workspace_bar_height: number(
                runtime,
                "ui.workspace_bar_height",
                "Toyoterm.__config.ui.workspace_bar_height",
            )?,
            workspace_width: number(
                runtime,
                "ui.workspace_width",
                "Toyoterm.__config.ui.workspace_width",
            )?,
            status_bar_height: number(
                runtime,
                "ui.status_bar_height",
                "Toyoterm.__config.ui.status_bar_height",
            )?,
            pane_divider_width: nonnegative(
                runtime,
                "ui.pane_divider_width",
                "Toyoterm.__config.ui.pane_divider_width",
            )?,
            active_pane_border_width: nonnegative(
                runtime,
                "ui.active_pane_border_width",
                "Toyoterm.__config.ui.active_pane_border_width",
            )?,
        },
        window: WindowConfig {
            opacity,
            width: number(runtime, "window.width", "Toyoterm.__config.window.width")?,
            height: number(runtime, "window.height", "Toyoterm.__config.window.height")?,
            min_width: number(
                runtime,
                "window.min_width",
                "Toyoterm.__config.window.min_width",
            )?,
            min_height: number(
                runtime,
                "window.min_height",
                "Toyoterm.__config.window.min_height",
            )?,
            decorations: boolean(
                runtime,
                "window.decorations",
                "Toyoterm.__config.window.decorations",
            )?,
            resizable: boolean(
                runtime,
                "window.resizable",
                "Toyoterm.__config.window.resizable",
            )?,
            always_on_top: boolean(
                runtime,
                "window.always_on_top",
                "Toyoterm.__config.window.always_on_top",
            )?,
            title: runtime.eval("Toyoterm.__config.window.title")?,
        },
        behavior: BehaviorConfig {
            scroll_lines: number(
                runtime,
                "behavior.scroll_lines",
                "Toyoterm.__config.behavior.scroll_lines",
            )?,
            copy_on_select: boolean(
                runtime,
                "behavior.copy_on_select",
                "Toyoterm.__config.behavior.copy_on_select",
            )?,
        },
        default_shell: if default_shell.is_empty() {
            defaults.default_shell
        } else {
            Some(default_shell)
        },
        scrollback_lines,
        leader,
        status_interval,
    };
    validate_color("background", &config.colors.background)?;
    validate_color("foreground", &config.colors.foreground)?;
    validate_color("cursor", &config.colors.cursor)?;
    validate_color("selection", &config.colors.selection)?;
    for (name, color) in [
        ("tab_bar", &config.colors.tab_bar),
        ("tab_active", &config.colors.tab_active),
        ("tab_inactive", &config.colors.tab_inactive),
        ("workspace_bar", &config.colors.workspace_bar),
        ("status_bar", &config.colors.status_bar),
        ("pane_border", &config.colors.pane_border),
        ("search_match", &config.colors.search_match),
        ("search_match_active", &config.colors.search_match_active),
    ] {
        validate_color(name, color)?;
    }
    if config.window.title.trim().is_empty() {
        return Err(ScriptError::new(
            "validate config",
            "window.title cannot be empty",
        ));
    }
    for (index, color) in config.colors.ansi.iter().enumerate() {
        validate_color(&format!("ansi[{index}]"), color)?;
    }
    Ok(config)
}
