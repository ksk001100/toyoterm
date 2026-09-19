use super::*;

pub(super) struct CommandCollector<'a> {
    runtime: &'a mut MrubyRuntime,
}

impl<'a> CommandCollector<'a> {
    pub(super) fn new(runtime: &'a mut MrubyRuntime) -> Self {
        Self { runtime }
    }

    fn bool(&mut self, expression: &str) -> Result<bool, ScriptError> {
        match self.runtime.eval(expression)?.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(ScriptError::new(
                "decode mruby command",
                "expected a Ruby boolean",
            )),
        }
    }

    fn usize(&mut self, expression: &str) -> Result<usize, ScriptError> {
        self.runtime
            .eval(expression)?
            .parse::<usize>()
            .map_err(|_| ScriptError::new("decode mruby command", "count is invalid"))
    }

    pub(super) fn read_launch_spec(&mut self) -> Result<PaneLaunchSpec, ScriptError> {
        let program = self
            .bool("Toyoterm.__current_launch_has_program")?
            .then(|| self.runtime.eval("Toyoterm.__current_launch_program"))
            .transpose()?;
        let arg_count = self.usize("Toyoterm.__current_launch_arg_count")?;
        let mut args = Vec::with_capacity(arg_count);
        for index in 0..arg_count {
            args.push(
                self.runtime
                    .eval(&format!("Toyoterm.__current_launch_arg({index})"))?,
            );
        }
        let cwd = self
            .bool("Toyoterm.__current_launch_has_cwd")?
            .then(|| self.runtime.eval("Toyoterm.__current_launch_cwd"))
            .transpose()?;
        let env_count = self.usize("Toyoterm.__current_launch_env_count")?;
        let mut environment = Vec::with_capacity(env_count);
        for index in 0..env_count {
            let key = self
                .runtime
                .eval(&format!("Toyoterm.__current_launch_env_key({index})"))?;
            let value = if self.bool(&format!(
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
}
