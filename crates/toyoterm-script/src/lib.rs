use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{CStr, CString, c_char, c_void};
use std::fmt;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::ptr::NonNull;
use std::rc::Rc;
use std::slice;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use toyoterm_api::{
    Command, HandleKind, NativeAction, NativeCommand, NativeHandle, PaneId, PaneLaunchSpec,
    PaneSearchDirection, SplitDirection, TabId, WindowId, WorkspaceId,
};
use toyoterm_config::home_directory;
pub use toyoterm_config::{
    BehaviorConfig, ColorConfig, FontConfig, LeaderConfig, StatusBarConfig, StatusBarPosition,
    ToyotermConfig, UiConfig, WindowConfig, default_config_path, default_plugin_dir,
};

const SLOW_CALLBACK_THRESHOLD: Duration = Duration::from_millis(100);
pub const PLUGIN_API_VERSION: &str = "0.1.0";

fn return_host_bytes(bytes: Vec<u8>, output: *mut *mut u8, length: *mut usize) {
    let mut bytes = bytes.into_boxed_slice();
    // SAFETY: The caller supplies valid out-pointers and releases non-empty buffers through
    // `toyoterm_host_bytes_free` after copying them into mruby-owned strings.
    unsafe {
        *length = bytes.len();
        *output = if bytes.is_empty() {
            std::ptr::null_mut()
        } else {
            let pointer = bytes.as_mut_ptr();
            std::mem::forget(bytes);
            pointer
        };
    }
}

fn return_host_error(message: String, error: *mut *mut c_char) -> i32 {
    let message = message.replace('\0', "\\0");
    // SAFETY: The caller releases this CString with `toyoterm_host_string_free`.
    unsafe {
        *error = CString::new(message)
            .expect("NUL bytes were replaced")
            .into_raw();
    }
    1
}

/// Reads a file for the mruby host API. Paths are UTF-8 on every supported platform while file
/// contents remain arbitrary bytes.
///
/// # Safety
///
/// `path` must address `path_length` readable bytes. All three out-pointers must be valid for
/// writes; the caller owns any returned buffer and must release it with the matching free function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toyoterm_host_read_file(
    path: *const u8,
    path_length: usize,
    output: *mut *mut u8,
    output_length: *mut usize,
    error: *mut *mut c_char,
) -> i32 {
    // SAFETY: The C shim supplies a live Ruby string buffer bounded by `path_length`.
    let path = unsafe { slice::from_raw_parts(path, path_length) };
    let path = match std::str::from_utf8(path) {
        Ok(path) => path,
        Err(_) => return return_host_error("path must be valid UTF-8".to_owned(), error),
    };
    match std::fs::read(path) {
        Ok(bytes) => {
            return_host_bytes(bytes, output, output_length);
            0
        }
        Err(cause) => return_host_error(
            format!("read {}: {cause}", Path::new(path).display()),
            error,
        ),
    }
}

/// Executes a child process synchronously and captures its byte-exact standard output and error.
///
/// # Safety
///
/// `arguments` and `lengths` must each contain `count` readable entries, and every argument pointer
/// must address the corresponding number of bytes. All out-pointers must be valid for writes; the
/// caller owns returned buffers and must release them with the matching free functions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toyoterm_host_spawn(
    arguments: *const *const u8,
    lengths: *const usize,
    count: usize,
    stdout_output: *mut *mut u8,
    stdout_length: *mut usize,
    stderr_output: *mut *mut u8,
    stderr_length: *mut usize,
    exit_status: *mut i32,
    error: *mut *mut c_char,
) -> i32 {
    // SAFETY: The C shim supplies two arrays of `count` entries backed by live Ruby strings.
    let pointers = unsafe { slice::from_raw_parts(arguments, count) };
    let lengths = unsafe { slice::from_raw_parts(lengths, count) };
    let decoded = pointers
        .iter()
        .zip(lengths)
        .map(|(&pointer, &length)| {
            // SAFETY: Each pointer refers to its corresponding Ruby string for this call.
            let bytes = unsafe { slice::from_raw_parts(pointer, length) };
            std::str::from_utf8(bytes).map(str::to_owned)
        })
        .collect::<Result<Vec<_>, _>>();
    let decoded = match decoded {
        Ok(decoded) => decoded,
        Err(_) => {
            return return_host_error(
                "program and arguments must be valid UTF-8".to_owned(),
                error,
            );
        }
    };
    let Some((program, arguments)) = decoded.split_first() else {
        return return_host_error("program cannot be empty".to_owned(), error);
    };
    match ProcessCommand::new(program).args(arguments).output() {
        Ok(output) => {
            return_host_bytes(output.stdout, stdout_output, stdout_length);
            return_host_bytes(output.stderr, stderr_output, stderr_length);
            // A signal has no portable numeric exit code; use -1 as the documented sentinel.
            unsafe { *exit_status = output.status.code().unwrap_or(-1) };
            0
        }
        Err(cause) => return_host_error(format!("spawn {program}: {cause}"), error),
    }
}

#[unsafe(no_mangle)]
/// Releases a non-empty buffer returned by a toyoterm host callback.
///
/// # Safety
///
/// `bytes` and `length` must be the exact pair returned by `return_host_bytes`, and must be freed
/// exactly once.
pub unsafe extern "C" fn toyoterm_host_bytes_free(bytes: *mut u8, length: usize) {
    if !bytes.is_null() {
        // SAFETY: This is the exact pointer and length leaked by `return_host_bytes`.
        unsafe {
            drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                bytes, length,
            )))
        };
    }
}

#[unsafe(no_mangle)]
/// Releases an error string returned by a toyoterm host callback.
///
/// # Safety
///
/// `string` must be null or a pointer returned by `return_host_error`, and must be freed exactly
/// once.
pub unsafe extern "C" fn toyoterm_host_string_free(string: *mut c_char) {
    if !string.is_null() {
        // SAFETY: Error strings originate from `CString::into_raw` in `return_host_error`.
        unsafe { drop(CString::from_raw(string)) };
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CallbackKind {
    KeyBinding,
    Event,
    UserCommand,
    Status,
}

impl CallbackKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::KeyBinding => "key_binding",
            Self::Event => "event",
            Self::UserCommand => "user_command",
            Self::Status => "status",
        }
    }
}

const CONFIG_DSL: &str = include_str!("config_dsl.rb");

unsafe extern "C" {
    fn toyoterm_mruby_open() -> *mut c_void;
    fn toyoterm_mruby_close(state: *mut c_void);
    fn toyoterm_mruby_eval(
        state: *mut c_void,
        source: *const c_char,
        filename: *const c_char,
        output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_set_current_pane(
        state: *mut c_void,
        pane_id: u64,
        error_output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_set_live_handles(
        state: *mut c_void,
        workspaces: *const u64,
        workspace_count: usize,
        windows: *const u64,
        window_count: usize,
        tabs: *const u64,
        tab_count: usize,
        panes: *const u64,
        pane_count: usize,
        error_output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_reset_object_model(
        state: *mut c_void,
        workspace_id: u64,
        window_id: u64,
        tab_id: u64,
        pane_id: u64,
        error_output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_add_workspace(
        state: *mut c_void,
        workspace_id: u64,
        name: *const c_char,
        name_length: usize,
        windows: *const u64,
        window_count: usize,
        error_output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_add_window(
        state: *mut c_void,
        window_id: u64,
        tabs: *const u64,
        tab_count: usize,
        error_output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_add_tab(
        state: *mut c_void,
        tab_id: u64,
        title: *const c_char,
        title_length: usize,
        panes: *const u64,
        pane_count: usize,
        error_output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_add_pane(
        state: *mut c_void,
        pane_id: u64,
        title: *const c_char,
        title_length: usize,
        cwd: *const c_char,
        cwd_length: usize,
        cwd_available: i32,
        pid: u64,
        pid_available: i32,
        command_running: i32,
        last_exit_status: i32,
        last_exit_status_available: i32,
        screen_text: *const c_char,
        screen_text_length: usize,
        error_output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_emit_event(
        state: *mut c_void,
        name: *const c_char,
        name_length: usize,
        workspace_id: u64,
        window_id: u64,
        tab_id: u64,
        pane_id: u64,
        title: *const c_char,
        title_length: usize,
        title_available: i32,
        cwd: *const c_char,
        cwd_length: usize,
        cwd_available: i32,
        exit_status: i32,
        exit_status_available: i32,
        error_output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_set_clipboard_text(
        state: *mut c_void,
        text: *const c_char,
        length: usize,
        available: i32,
        error_output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_set_environment(
        state: *mut c_void,
        keys: *const *const c_char,
        values: *const *const c_char,
        lengths: *const usize,
        count: usize,
        error_output: *mut *mut c_char,
    ) -> i32;
    fn toyoterm_mruby_install_host_api(state: *mut c_void);
    fn toyoterm_mruby_string_free(string: *mut c_char);
}

mod script_thread;
pub use script_thread::*;
mod runtime;
pub use runtime::MrubyRuntime;
mod config_manager;
pub use config_manager::ConfigManager;
use config_manager::run_script_request;
#[cfg(test)]
use config_manager::{is_slow_callback, load_config, resolve_config_path};
mod plugin;
use plugin::*;
mod parsing;
use parsing::*;
#[cfg(test)]
mod tests;
