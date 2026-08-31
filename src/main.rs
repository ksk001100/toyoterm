use std::io::Read;
use std::process::ExitCode;

use toyoterm::{Command, Mux, NativePty, Pty, PtyCommand, PtySize, SplitDirection};

fn main() -> ExitCode {
    match run(std::env::args().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("toyoterm: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    match args.next().as_deref() {
        None => {
            let mux = Mux::new();
            println!("{}", mux.summary());
            Ok(())
        }
        Some("version" | "--version" | "-V") => {
            println!("toyoterm {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("list") => {
            let mux = Mux::new();
            println!("{}", mux.summary());
            Ok(())
        }
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
        Some("help" | "--help" | "-h") => {
            print_help();
            Ok(())
        }
        Some(command) => Err(format!("unknown command `{command}`; try `toyoterm help`")),
    }
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
    let mut output = String::new();
    session
        .take_reader()
        .map_err(|error| error.to_string())?
        .read_to_string(&mut output)
        .map_err(|error| format!("read PTY output: {error}"))?;
    let status = session.wait().map_err(|error| error.to_string())?;
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

#[cfg(windows)]
fn demo_input() -> &'static str {
    "echo hello from toyoterm PTY\r\nexit\r\n"
}

fn print_help() {
    println!(
        "toyoterm - a programmable terminal emulator powered by Rust and mruby\n\n\
         Usage:\n  toyoterm [COMMAND]\n\n\
         Commands:\n  list       Show the native mux state\n  demo       Exercise tabs and pane splitting\n  pty-demo   Spawn a process in a native PTY\n  version    Print version\n  help       Print this help"
    );
}
