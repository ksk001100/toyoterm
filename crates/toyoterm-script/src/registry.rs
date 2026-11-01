use super::*;

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct RegistrySnapshot {
    pub(super) keybindings: HashSet<String>,
    pub(super) native_actions: HashMap<String, NativeAction>,
    pub(super) event_names: HashSet<String>,
    pub(super) user_command_names: HashSet<String>,
    pub(super) keymap_bindings: HashMap<KeymapScope, KeymapBindings>,
    pub(super) timers: Vec<ScriptTimer>,
    pub(super) advised_actions: HashSet<String>,
}

impl RegistrySnapshot {
    pub(super) fn read(runtime: &mut MrubyRuntime) -> Result<Self, ScriptError> {
        runtime.eval("Toyoterm.__prune_keymap_scopes")?;
        let count = |runtime: &mut MrubyRuntime, expression: &str, operation: &'static str| {
            runtime
                .eval(expression)?
                .parse::<usize>()
                .map_err(|_| ScriptError::new(operation, "registry count is invalid"))
        };

        let dynamic_count = count(
            runtime,
            "Toyoterm.__config.__binding_count",
            "load key bindings",
        )?;
        let mut keybindings = HashSet::with_capacity(dynamic_count);
        for index in 0..dynamic_count {
            keybindings.insert(runtime.eval(&format!("Toyoterm.__config.__binding_key({index})"))?);
        }

        let static_count = count(
            runtime,
            "Toyoterm.__config.__static_binding_count",
            "load key bindings",
        )?;
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

        let event_count = count(runtime, "Toyoterm.__event_count", "load events")?;
        let mut event_names = HashSet::with_capacity(event_count);
        for index in 0..event_count {
            event_names.insert(runtime.eval(&format!("Toyoterm.__event_name({index})"))?);
        }

        let command_count = count(runtime, "Toyoterm.__command_count", "load user commands")?;
        let mut user_command_names = HashSet::with_capacity(command_count);
        for index in 0..command_count {
            user_command_names.insert(runtime.eval(&format!("Toyoterm.__command_name({index})"))?);
        }

        let scope_count = count(runtime, "Toyoterm.__key_scope_count", "load keymap scopes")?;
        let mut keymap_bindings = HashMap::with_capacity(scope_count);
        for index in 0..scope_count {
            let scope = runtime.eval(&format!("Toyoterm.__select_key_scope({index})"))?;
            let scope = if scope == "global" {
                KeymapScope::Global
            } else {
                KeymapScope::Pane(PaneId(
                    scope
                        .parse::<u64>()
                        .map_err(|_| ScriptError::new("load keymaps", "invalid Pane ID"))?,
                ))
            };
            let entry_count = count(
                runtime,
                "Toyoterm.__key_scope_entry_count",
                "load keymap bindings",
            )?;
            let mut bindings = KeymapBindings::default();
            for entry in 0..entry_count {
                let key = runtime.eval(&format!("Toyoterm.__key_scope_entry({entry}, :key)"))?;
                let kind = runtime.eval(&format!("Toyoterm.__key_scope_entry({entry}, :kind)"))?;
                let priority = runtime
                    .eval(&format!("Toyoterm.__key_scope_entry({entry}, :layer) || 0"))?
                    .parse::<u64>()
                    .map_err(|_| ScriptError::new("load keymaps", "invalid layer priority"))?;
                bindings.priorities.insert(key.clone(), priority);
                if kind == "ruby" {
                    let owner = runtime
                        .eval(&format!(
                            "Toyoterm.__key_scope_entry({entry}, :keymap_id) || 0"
                        ))?
                        .parse::<u64>()
                        .map_err(|_| ScriptError::new("load keymaps", "invalid keymap ID"))?;
                    bindings.callback_owners.insert(key.clone(), owner);
                    bindings.keybindings.insert(key);
                } else {
                    let action =
                        runtime.eval(&format!("Toyoterm.__key_scope_entry({entry}, :action)"))?;
                    let argument =
                        runtime.eval(&format!("Toyoterm.__key_scope_entry({entry}, :argument)"))?;
                    bindings
                        .native_actions
                        .insert(key, decode_native_action(&action, &argument)?);
                }
            }
            let transient_count = count(
                runtime,
                "Toyoterm.__transient_count",
                "load transient keymaps",
            )?;
            for index in 0..transient_count {
                let id = runtime
                    .eval(&format!("Toyoterm.__transient_field({index}, :id)"))?
                    .parse::<u64>()
                    .map_err(|_| {
                        ScriptError::new("load transient keymaps", "invalid activation ID")
                    })?;
                let once =
                    runtime.eval(&format!("Toyoterm.__transient_field({index}, :once)"))? == "true";
                let key_count = count(
                    runtime,
                    &format!("Toyoterm.__transient_field({index}, :key_count)"),
                    "load transient bindings",
                )?;
                let mut keys = HashSet::with_capacity(key_count);
                for key in 0..key_count {
                    keys.insert(runtime.eval(&format!("Toyoterm.__transient_key({key})"))?);
                }
                bindings.transients.push(TransientKeymap { id, once, keys });
            }
            keymap_bindings.insert(scope, bindings);
        }
        let timer_count = count(runtime, "Toyoterm.__timer_count", "load timers")?;
        let mut timers = Vec::with_capacity(timer_count);
        for index in 0..timer_count {
            let id = runtime
                .eval(&format!("Toyoterm.__timer_field({index}, 0)"))?
                .parse::<u64>()
                .map_err(|_| ScriptError::new("load timers", "invalid timer ID"))?;
            let delay = runtime
                .eval(&format!("Toyoterm.__timer_field({index}, 1)"))?
                .parse::<u64>()
                .map(Duration::from_millis)
                .map_err(|_| ScriptError::new("load timers", "invalid timer delay"))?;
            let repeat = runtime.eval(&format!("Toyoterm.__timer_field({index}, 2)"))? == "true";
            timers.push(ScriptTimer {
                id,
                delay,
                interval: repeat.then_some(delay),
            });
        }
        let advice_count = count(
            runtime,
            "Toyoterm.__action_advice_count",
            "load action advice",
        )?;
        let mut advised_actions = HashSet::with_capacity(advice_count);
        for index in 0..advice_count {
            advised_actions
                .insert(runtime.eval(&format!("Toyoterm.__action_advice_name({index})"))?);
        }
        Ok(Self {
            keybindings,
            native_actions,
            event_names,
            user_command_names,
            keymap_bindings,
            timers,
            advised_actions,
        })
    }
}
