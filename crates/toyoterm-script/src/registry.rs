use super::*;

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct RegistrySnapshot {
    pub(super) keybindings: HashSet<String>,
    pub(super) native_actions: HashMap<String, NativeAction>,
    pub(super) event_names: HashSet<String>,
    pub(super) user_command_names: HashSet<String>,
    pub(super) plugins: Vec<PluginMetadata>,
}

impl RegistrySnapshot {
    pub(super) fn read(
        runtime: &mut MrubyRuntime,
        plugins: Vec<PluginMetadata>,
    ) -> Result<Self, ScriptError> {
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

        Ok(Self {
            keybindings,
            native_actions,
            event_names,
            user_command_names,
            plugins,
        })
    }
}
