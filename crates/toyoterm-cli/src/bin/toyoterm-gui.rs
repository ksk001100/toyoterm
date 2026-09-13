#![cfg_attr(windows, windows_subsystem = "windows")]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::ExitCode;

use toyoterm_app::{init_logging, install_panic_hook, run_gui};

#[path = "../gui_options.rs"]
mod gui_options;

fn main() -> ExitCode {
    if let Err(error) = init_logging() {
        eprintln!("toyoterm-gui: {error}");
        return ExitCode::FAILURE;
    }
    install_panic_hook();
    match catch_unwind(AssertUnwindSafe(|| run(std::env::args().skip(1)))) {
        Ok(Ok(())) => ExitCode::SUCCESS,
        Err(payload) => {
            if let Some(message) = payload.downcast_ref::<String>() {
                eprintln!("toyoterm-gui: fatal panic: {message}");
            } else if let Some(message) = payload.downcast_ref::<&str>() {
                eprintln!("toyoterm-gui: fatal panic: {message}");
            } else {
                eprintln!("toyoterm-gui: fatal panic");
            }
            ExitCode::FAILURE
        }
        Ok(Err(message)) => {
            eprintln!("toyoterm-gui: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    match args.next() {
        None => run_gui().map_err(|error| error.to_string()),
        Some(command) if command == "gui" => match args.next() {
            None => run_gui().map_err(|error| error.to_string()),
            Some(argument) if gui_options::is_gui_option(&argument) => {
                gui_options::run_gui_options(&argument, args)
            }
            Some(argument) => Err(format!("unexpected GUI argument `{argument}`")),
        },
        Some(argument) if gui_options::is_gui_option(&argument) => {
            gui_options::run_gui_options(&argument, args)
        }
        Some(argument) => Err(format!("unexpected GUI argument `{argument}`")),
    }
}
