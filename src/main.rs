use std::process::ExitCode;

use toyoterm::{Command, Mux, SplitDirection};

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
        Some("help" | "--help" | "-h") => {
            print_help();
            Ok(())
        }
        Some(command) => Err(format!("unknown command `{command}`; try `toyoterm help`")),
    }
}

fn print_help() {
    println!(
        "toyoterm - a programmable terminal emulator powered by Rust and mruby\n\n\
         Usage:\n  toyoterm [COMMAND]\n\n\
         Commands:\n  list       Show the native mux state\n  demo       Exercise tabs and pane splitting\n  version    Print version\n  help       Print this help"
    );
}
