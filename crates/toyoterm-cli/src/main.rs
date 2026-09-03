#![cfg_attr(windows, windows_subsystem = "windows")]

use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::process::ExitCode;

use toyoterm_api::{Command, PaneId, PaneLaunchSpec, SplitDirection};
use toyoterm_app::{
    GuiOptions, init_logging, install_panic_hook, run_gui, run_gui_smoke_test, run_gui_with_options,
};
use toyoterm_ipc::{IpcRequest, request_remote, run_console};
use toyoterm_mux::Mux;
use toyoterm_pty::{NativePty, Pty, PtyCommand, PtySize};
use toyoterm_terminal::{AlacrittyTerminalBackend, TerminalBackend};

mod shell_integration;

fn main() -> ExitCode {
    if let Err(error) = init_logging() {
        eprintln!("toyoterm: {error}");
        return ExitCode::FAILURE;
    }
    install_panic_hook();
    match catch_unwind(AssertUnwindSafe(|| run(std::env::args().skip(1)))) {
        Ok(Ok(())) => ExitCode::SUCCESS,
        Err(payload) => {
            if let Some(message) = payload.downcast_ref::<String>() {
                eprintln!("toyoterm: fatal panic: {message}");
            } else if let Some(message) = payload.downcast_ref::<&str>() {
                eprintln!("toyoterm: fatal panic: {message}");
            } else {
                eprintln!("toyoterm: fatal panic");
            }
            ExitCode::FAILURE
        }
        Ok(Err(message)) => {
            eprintln!("toyoterm: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    match args.next().as_deref() {
        None => run_gui().map_err(|error| error.to_string()),
        Some(argument) if is_gui_option(argument) => run_gui_options(argument, args),
        Some("version" | "--version" | "-V") => {
            println!("toyoterm {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("list") => {
            ensure_no_arguments(&mut args)?;
            println!("{}", request_remote(IpcRequest::List)?);
            Ok(())
        }
        Some("reload") => {
            ensure_no_arguments(&mut args)?;
            request_remote(IpcRequest::Reload)?;
            println!("config reloaded");
            Ok(())
        }
        Some("cli") => run_cli(&mut args),
        Some("demo") => {
            let mut mux = Mux::new();
            let pane = mux.current_pane().ok_or("no active pane")?;
            mux.dispatch(Command::Split {
                pane,
                direction: SplitDirection::Right,
            })
            .map_err(|error| error.to_string())?;
            mux.dispatch(Command::NewTab)
                .map_err(|error| error.to_string())?;
            println!("{}", mux.summary());
            Ok(())
        }
        Some("pty-demo") => run_pty_demo(),
        Some("screen-demo") => run_screen_demo(),
        Some("shell-integration") => {
            let shell = args
                .next()
                .ok_or("shell-integration requires bash, zsh, fish, or powershell")?;
            ensure_no_arguments(&mut args)?;
            print!(
                "{}",
                shell_integration::script(&shell).ok_or_else(|| format!(
                    "unsupported shell `{shell}`; expected bash, zsh, fish, or powershell"
                ))?
            );
            Ok(())
        }
        Some("gui-smoke-test") => {
            ensure_no_arguments(&mut args)?;
            run_gui_smoke_test().map_err(|error| error.to_string())
        }
        Some("ruby") => match args.next().as_deref() {
            None | Some("console") => {
                ensure_no_arguments(&mut args)?;
                run_console()
            }
            Some(argument) => Err(format!("unexpected ruby argument `{argument}`")),
        },
        Some("gui") => match args.next() {
            None => run_gui().map_err(|error| error.to_string()),
            Some(argument) if is_gui_option(&argument) => run_gui_options(&argument, args),
            Some(argument) => Err(format!("unexpected GUI argument `{argument}`")),
        },
        Some("help" | "--help" | "-h") => {
            print_help();
            Ok(())
        }
        Some(command) => Err(format!("unknown command `{command}`; try `toyoterm help`")),
    }
}

fn is_gui_option(argument: &str) -> bool {
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

fn run_gui_options(first: &str, remaining: impl Iterator<Item = String>) -> Result<(), String> {
    let options = parse_gui_options(std::iter::once(first.to_owned()).chain(remaining))?;
    run_gui_with_options(options).map_err(|error| error.to_string())
}

fn parse_gui_options(mut args: impl Iterator<Item = String>) -> Result<GuiOptions, String> {
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

fn ensure_no_arguments(args: &mut impl Iterator<Item = String>) -> Result<(), String> {
    match args.next() {
        Some(argument) => Err(format!("unexpected argument `{argument}`")),
        None => Ok(()),
    }
}

fn run_cli(args: &mut impl Iterator<Item = String>) -> Result<(), String> {
    let request = match args.next().as_deref() {
        Some("list-panes") => {
            ensure_no_arguments(args)?;
            IpcRequest::ListPanes
        }
        Some("send-text") => {
            if args.next().as_deref() != Some("--pane") {
                return Err("send-text requires --pane ID TEXT".to_owned());
            }
            let pane = args
                .next()
                .ok_or("--pane requires an ID")?
                .parse::<u64>()
                .map_err(|_| "pane ID must be an unsigned integer".to_owned())?;
            let text = args.collect::<Vec<_>>().join(" ");
            if text.is_empty() {
                return Err("send-text requires TEXT".to_owned());
            }
            IpcRequest::SendText {
                pane: PaneId(pane),
                text,
            }
        }
        Some("split") => {
            let first = args.next();
            let direction = match first.as_deref() {
                None => SplitDirection::Right,
                Some("--direction") => parse_direction(
                    &args
                        .next()
                        .ok_or("--direction requires left, right, up, or down")?,
                )?,
                Some(value) => parse_direction(value)?,
            };
            ensure_no_arguments(args)?;
            IpcRequest::Split { direction }
        }
        Some("activate-workspace") => {
            let name = args
                .next()
                .filter(|name| !name.is_empty())
                .ok_or("activate-workspace requires NAME")?;
            ensure_no_arguments(args)?;
            IpcRequest::ActivateWorkspace(name)
        }
        Some(command) => {
            return Err(format!(
                "unknown cli command `{command}`; try `toyoterm help`"
            ));
        }
        None => return Err("cli requires a command; try `toyoterm help`".to_owned()),
    };
    let output = request_remote(request)?;
    if !output.is_empty() {
        println!("{output}");
    }
    Ok(())
}

fn parse_direction(value: &str) -> Result<SplitDirection, String> {
    match value {
        "left" => Ok(SplitDirection::Left),
        "right" => Ok(SplitDirection::Right),
        "up" => Ok(SplitDirection::Up),
        "down" => Ok(SplitDirection::Down),
        _ => Err(format!(
            "invalid split direction `{value}`; expected left, right, up, or down"
        )),
    }
}

fn run_screen_demo() -> Result<(), String> {
    let mut session = NativePty
        .spawn(screen_demo_command(), PtySize::new(40, 6))
        .map_err(|error| error.to_string())?;
    let mut reader = session.take_reader().map_err(|error| error.to_string())?;
    let reader_thread = std::thread::spawn(move || {
        let mut output = Vec::new();
        reader.read_to_end(&mut output).map(|_| output)
    });
    let status = session.wait().map_err(|error| error.to_string())?;
    let output = reader_thread
        .join()
        .map_err(|_| "PTY reader thread panicked".to_owned())?
        .map_err(|error| format!("read PTY output: {error}"))?;
    if status.code != 0 {
        return Err(format!("PTY process exited with code {}", status.code));
    }

    let mut terminal = AlacrittyTerminalBackend::new(40, 6);
    terminal.advance(&output);
    let snapshot = terminal.snapshot();
    for line in snapshot.lines.iter().filter(|line| !line.is_empty()) {
        println!("{line}");
    }
    Ok(())
}

fn run_pty_demo() -> Result<(), String> {
    let mut mux = Mux::new();
    let pane = mux.current_pane().ok_or("no active pane")?;
    mux.dispatch(Command::SendText {
        pane,
        text: demo_input().to_owned(),
    })
    .map_err(|error| error.to_string())?;

    let mut command = demo_shell();
    command.env("TERM", "xterm-256color");
    let mut session = NativePty
        .spawn(command, PtySize::default())
        .map_err(|error| error.to_string())?;
    session
        .write(
            &mux.take_pending_input(pane)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
    let mut reader = session.take_reader().map_err(|error| error.to_string())?;
    let reader_thread = std::thread::spawn(move || {
        let mut output = String::new();
        reader.read_to_string(&mut output).map(|_| output)
    });
    let status = session.wait().map_err(|error| error.to_string())?;
    let output = reader_thread
        .join()
        .map_err(|_| "PTY reader thread panicked".to_owned())?
        .map_err(|error| format!("read PTY output: {error}"))?;
    print!("{output}");
    if status.code == 0 {
        Ok(())
    } else {
        Err(format!("PTY process exited with code {}", status.code))
    }
}

#[cfg(unix)]
fn demo_shell() -> PtyCommand {
    PtyCommand::new("/bin/sh")
}

#[cfg(windows)]
fn demo_shell() -> PtyCommand {
    PtyCommand::new("cmd.exe")
}

#[cfg(unix)]
fn demo_input() -> &'static str {
    "printf 'hello from toyoterm PTY\\n'\nexit\n"
}

#[cfg(unix)]
fn screen_demo_command() -> PtyCommand {
    let mut command = PtyCommand::new("/bin/sh");
    command.args([
        "-c",
        "printf '\\033[2J\\033[Htoyoterm VT backend\\nready\\n'",
    ]);
    command
}

#[cfg(windows)]
fn screen_demo_command() -> PtyCommand {
    let mut command = PtyCommand::new("cmd.exe");
    command.args(["/C", "echo toyoterm VT backend&&echo ready"]);
    command
}

#[cfg(windows)]
fn demo_input() -> &'static str {
    "echo hello from toyoterm PTY\r\nexit\r\n"
}

fn print_help() {
    println!(
        "toyoterm - a programmable terminal emulator powered by Rust and mruby\n\n\
         Usage:\n  toyoterm [GUI-OPTIONS] [-e COMMAND [ARG...]]\n  toyoterm [COMMAND]\n\n\
         GUI options:\n  --config PATH                    Use a specific Ruby configuration\n  --title TITLE                    Set the initial window title\n  --app-id APP-ID                  Set the Wayland app ID / X11 class on Linux\n  --working-directory DIR          Set the initial pane working directory\n  -e, --execute COMMAND [ARG...]   Run a command instead of the default shell\n\n\
         Commands:\n  gui                              Open the native GPU window (default)\n  ruby console                     Connect to the running GUI Ruby VM\n  list                             Show the running GUI mux state\n  reload                           Reload the running GUI configuration\n  cli list-panes                   List panes in the running GUI\n  cli send-text --pane ID TEXT     Send text to a pane\n  cli split [DIRECTION]            Split the active pane (default: right)\n  cli activate-workspace NAME      Activate or create a workspace\n  demo                             Exercise tabs and pane splitting\n  pty-demo                         Spawn a process in a native PTY\n  screen-demo                      Parse PTY output into a terminal snapshot\n  shell-integration SHELL         Print the integration script for a shell\n  version                          Print version\n  help                             Print this help\n\nEnvironment:\n  TOYOTERM_INSTANCE                Select a named running GUI instance"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_xdg_terminal_launch_options() {
        let options = parse_gui_options(
            [
                "--app-id=org.omarchy.terminal",
                "--title=Omarchy",
                "--working-directory=/tmp",
                "-e",
                "bash",
                "-c",
                "omarchy update",
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .unwrap();

        assert_eq!(options.title.as_deref(), Some("Omarchy"));
        assert_eq!(options.app_id.as_deref(), Some("org.omarchy.terminal"));
        assert_eq!(
            options.initial_pane,
            Some(PaneLaunchSpec {
                program: Some("bash".into()),
                args: vec!["-c".into(), "omarchy update".into()],
                cwd: Some("/tmp".into()),
                environment: Vec::new(),
            })
        );
    }

    #[test]
    fn command_arguments_are_not_parsed_as_gui_options() {
        let options = parse_gui_options(
            ["-e", "printf", "--title=child argument"]
                .into_iter()
                .map(str::to_owned),
        )
        .unwrap();

        let launch = options.initial_pane.unwrap();
        assert_eq!(launch.program.as_deref(), Some("printf"));
        assert_eq!(launch.args, ["--title=child argument"]);
    }

    #[test]
    fn rejects_execute_without_a_command() {
        assert_eq!(
            parse_gui_options(["-e"].into_iter().map(str::to_owned)).unwrap_err(),
            "-e requires a command"
        );
    }
}
