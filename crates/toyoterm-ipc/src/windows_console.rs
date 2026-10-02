use std::io::{self, IsTerminal};
use std::os::windows::io::AsRawHandle;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Console::{
    ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT, ENABLE_VIRTUAL_TERMINAL_INPUT,
    GetConsoleMode, SetConsoleMode,
};

pub(super) struct ConsoleInputMode {
    handle: HANDLE,
    original: u32,
}

impl ConsoleInputMode {
    pub(super) fn begin(stdin: &io::Stdin) -> Result<Option<Self>, String> {
        // Redirected input is a byte stream and must retain ordinary read_line behavior.
        if !stdin.is_terminal() {
            tracing::debug!(target: "toyoterm::ipc", "console stdin is redirected");
            return Ok(None);
        }
        let handle = stdin.as_raw_handle();
        let mut original = 0;
        // SAFETY: stdin owns a live console input handle, and original is writable.
        if unsafe { GetConsoleMode(handle, &mut original) } == 0 {
            return Err(format!(
                "read console input mode: {}",
                io::Error::last_os_error()
            ));
        }
        // Shells can leave raw/VT input enabled. ReadConsole then returns CR without
        // LF, so Rust's read_line waits forever, even after an empty Enter press.
        let mode = (original | ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT)
            & !ENABLE_VIRTUAL_TERMINAL_INPUT;
        // SAFETY: handle remains owned by stdin throughout the REPL.
        if unsafe { SetConsoleMode(handle, mode) } == 0 {
            return Err(format!(
                "set console input mode: {}",
                io::Error::last_os_error()
            ));
        }
        tracing::debug!(target: "toyoterm::ipc", original, mode, "configured console input mode");
        Ok(Some(Self { handle, original }))
    }
}

impl Drop for ConsoleInputMode {
    fn drop(&mut self) {
        // SAFETY: run_console keeps stdin alive until after this guard is dropped.
        if unsafe { SetConsoleMode(self.handle, self.original) } == 0 {
            tracing::warn!(target: "toyoterm::ipc", error = %io::Error::last_os_error(),
                "restore console input mode failed");
        }
    }
}
