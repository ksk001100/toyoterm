mod performance;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use toyoterm_api::{
    ActionCommand, ActionContext, ClipboardCommand, Command, ConfigCommand, HandleKind,
    NativeAction, NativeCommand, NativeHandle, PaneCommand, PaneId, PaneLaunchSpec,
    PaneSearchDirection, ScriptEventKind, SplitDirection, TabId, UiCommand, WindowCommand,
    WindowId, WorkspaceId,
};
use toyoterm_config::home_directory;
pub use toyoterm_config::{
    BehaviorConfig, ColorConfig, FontConfig, LeaderConfig, StatusBarConfig, StatusBarPosition,
    ToyotermConfig, UiConfig, WindowConfig, default_config_path,
};

const SLOW_CALLBACK_THRESHOLD: Duration = Duration::from_millis(100);
pub const STEEL_API_VERSION: &str = "0.1.0";

fn platform_primary_modifier() -> &'static str {
    if cfg!(target_os = "macos") {
        "SUPER"
    } else {
        "CTRL"
    }
}

fn platform_name() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "other"
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CallbackKind {
    KeyBinding,
    Event,
    UserCommand,
    Bar,
    AsyncCallback,
    SelectCallback,
}

impl CallbackKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::KeyBinding => "key_binding",
            Self::Event => "event",
            Self::UserCommand => "user_command",
            Self::Bar => "bar",
            Self::AsyncCallback => "async_callback",
            Self::SelectCallback => "select_callback",
        }
    }
}

mod script_thread;
pub use script_thread::*;
mod steel_runtime;
pub use steel_runtime::SteelRuntime;
mod config_manager;
pub use config_manager::ConfigManager;
use config_manager::run_script_request;
mod command_collector;
use command_collector::CommandCollector;
#[cfg(test)]
use config_manager::{decode_color_presets, is_slow_callback, resolve_config_path};
mod parsing;
use parsing::*;
mod registry;
use registry::RegistrySnapshot;
#[cfg(test)]
mod tests;
