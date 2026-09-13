use std::path::PathBuf;

use toyoterm_api::PaneLaunchSpec;
use toyoterm_app::{GuiOptions, run_gui_with_options};

pub(crate) fn is_gui_option(argument: &str) -> bool {
    matches!(
        argument,
        "--config"
            | "--title"
            | "--app-id"
            | "--working-directory"
            | "--dir"
            | "-e"
            | "--execute"
            | "--"
    ) || argument.starts_with("--config=")
        || argument.starts_with("--title=")
        || argument.starts_with("--app-id=")
        || argument.starts_with("--working-directory=")
        || argument.starts_with("--dir=")
}

pub(crate) fn run_gui_options(
    first: &str,
    remaining: impl Iterator<Item = String>,
) -> Result<(), String> {
    let options = parse_gui_options(std::iter::once(first.to_owned()).chain(remaining))?;
    run_gui_with_options(options).map_err(|error| error.to_string())
}

pub(crate) fn parse_gui_options(
    mut args: impl Iterator<Item = String>,
) -> Result<GuiOptions, String> {
    let mut options = GuiOptions::default();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--config" => options.config_path = Some(required_config_path(&mut args)?),
            "--title" => options.title = Some(required_option_value("--title", &mut args)?),
            "--app-id" => options.app_id = Some(required_option_value("--app-id", &mut args)?),
            "--working-directory" | "--dir" => {
                let cwd = required_option_value(&argument, &mut args)?;
                set_launch_cwd(&mut options, cwd);
            }
            "-e" | "--execute" | "--" => {
                let command = args.collect::<Vec<_>>();
                if command.is_empty() {
                    return Err(format!("{argument} requires a command"));
                }
                let mut command = command.into_iter();
                options.initial_pane = Some(PaneLaunchSpec {
                    program: command.next(),
                    args: command.collect(),
                    cwd: options.initial_pane.and_then(|launch| launch.cwd),
                    environment: Vec::new(),
                });
                return Ok(options);
            }
            _ if argument.starts_with("--config=") => {
                options.config_path =
                    Some(PathBuf::from(inline_option_value("--config", &argument)?));
            }
            _ if argument.starts_with("--title=") => {
                options.title = Some(inline_option_value("--title", &argument)?.to_owned());
            }
            _ if argument.starts_with("--app-id=") => {
                options.app_id = Some(inline_option_value("--app-id", &argument)?.to_owned());
            }
            _ if argument.starts_with("--working-directory=") => {
                let cwd = inline_option_value("--working-directory", &argument)?.to_owned();
                set_launch_cwd(&mut options, cwd);
            }
            _ if argument.starts_with("--dir=") => {
                let cwd = inline_option_value("--dir", &argument)?.to_owned();
                set_launch_cwd(&mut options, cwd);
            }
            _ => return Err(format!("unexpected GUI argument `{argument}`")),
        }
    }
    Ok(options)
}

fn set_launch_cwd(options: &mut GuiOptions, cwd: String) {
    options
        .initial_pane
        .get_or_insert_with(|| PaneLaunchSpec {
            program: None,
            args: Vec::new(),
            cwd: None,
            environment: Vec::new(),
        })
        .cwd = Some(cwd);
}

fn required_option_value(
    option: &str,
    args: &mut impl Iterator<Item = String>,
) -> Result<String, String> {
    args.next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{option} requires a value"))
}

fn inline_option_value<'a>(option: &str, argument: &'a str) -> Result<&'a str, String> {
    argument
        .split_once('=')
        .map(|(_, value)| value)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{option} requires a value"))
}

fn required_config_path(args: &mut impl Iterator<Item = String>) -> Result<PathBuf, String> {
    args.next()
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "--config requires a path".into())
}
