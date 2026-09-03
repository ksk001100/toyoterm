use super::*;

/// A single-threaded owner for one embedded mruby VM.
pub struct MrubyRuntime {
    pub(super) state: NonNull<c_void>,
    not_send_or_sync: PhantomData<Rc<()>>,
}

impl MrubyRuntime {
    pub fn new() -> Result<Self, ScriptError> {
        // SAFETY: The returned state is exclusively owned by this wrapper and closed in Drop.
        let state = NonNull::new(unsafe { toyoterm_mruby_open() })
            .ok_or_else(|| ScriptError::new("initialize mruby", "mrb_open failed"))?;
        Ok(Self {
            state,
            not_send_or_sync: PhantomData,
        })
    }

    pub(super) fn set_environment(&mut self) -> Result<(), ScriptError> {
        // Ruby strings and this API are byte-safe, but paths and environment names cross
        // platform boundaries. Expose only entries representable as UTF-8 on every target.
        let entries = std::env::vars_os()
            .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
            .collect::<Vec<_>>();
        let keys = entries
            .iter()
            .map(|(key, _)| key.as_ptr().cast::<c_char>())
            .collect::<Vec<_>>();
        let values = entries
            .iter()
            .map(|(_, value)| value.as_ptr().cast::<c_char>())
            .collect::<Vec<_>>();
        let lengths = entries
            .iter()
            .flat_map(|(key, value)| [key.len(), value.len()])
            .collect::<Vec<_>>();
        let mut error = std::ptr::null_mut();
        // SAFETY: All strings and pointer arrays remain live for the duration of the call.
        let status = unsafe {
            toyoterm_mruby_set_environment(
                self.state.as_ptr(),
                keys.as_ptr(),
                values.as_ptr(),
                lengths.as_ptr(),
                entries.len(),
                &mut error,
            )
        };
        typed_call_result("set environment snapshot", status, error)
    }

    pub fn eval(&mut self, source: &str) -> Result<String, ScriptError> {
        self.eval_with_filename(source, "(eval)")
    }

    pub(super) fn set_current_pane(&mut self, pane: PaneId) -> Result<(), ScriptError> {
        let mut error = std::ptr::null_mut();
        // SAFETY: The VM is exclusively borrowed and the shim initializes `error`.
        let status =
            unsafe { toyoterm_mruby_set_current_pane(self.state.as_ptr(), pane.0, &mut error) };
        typed_call_result("set current pane", status, error)
    }

    pub(super) fn set_live_handles(
        &mut self,
        workspaces: &[u64],
        windows: &[u64],
        tabs: &[u64],
        panes: &[u64],
    ) -> Result<(), ScriptError> {
        let mut error = std::ptr::null_mut();
        // SAFETY: All slices remain live for the call and the VM is exclusively borrowed.
        let status = unsafe {
            toyoterm_mruby_set_live_handles(
                self.state.as_ptr(),
                workspaces.as_ptr(),
                workspaces.len(),
                windows.as_ptr(),
                windows.len(),
                tabs.as_ptr(),
                tabs.len(),
                panes.as_ptr(),
                panes.len(),
                &mut error,
            )
        };
        typed_call_result("set live handles", status, error)
    }

    pub(super) fn set_object_model(&mut self, model: &RubyObjectModel) -> Result<(), ScriptError> {
        let mut error = std::ptr::null_mut();
        // SAFETY: The VM is exclusively borrowed and the shim initializes `error`.
        let status = unsafe {
            toyoterm_mruby_reset_object_model(
                self.state.as_ptr(),
                model.current_workspace.0,
                model.current_window.0,
                model.current_tab.0,
                model.current_pane.0,
                &mut error,
            )
        };
        typed_call_result("reset object model", status, error)?;

        for workspace in &model.workspaces {
            let windows = workspace
                .windows
                .iter()
                .map(|window| window.0)
                .collect::<Vec<_>>();
            let mut error = std::ptr::null_mut();
            // SAFETY: String and slice storage remains live for the duration of the call.
            let status = unsafe {
                toyoterm_mruby_add_workspace(
                    self.state.as_ptr(),
                    workspace.id.0,
                    workspace.name.as_ptr().cast(),
                    workspace.name.len(),
                    windows.as_ptr(),
                    windows.len(),
                    &mut error,
                )
            };
            typed_call_result("add workspace object", status, error)?;
        }
        for window in &model.windows {
            let tabs = window.tabs.iter().map(|tab| tab.0).collect::<Vec<_>>();
            let mut error = std::ptr::null_mut();
            // SAFETY: Slice storage remains live for the duration of the call.
            let status = unsafe {
                toyoterm_mruby_add_window(
                    self.state.as_ptr(),
                    window.id.0,
                    tabs.as_ptr(),
                    tabs.len(),
                    &mut error,
                )
            };
            typed_call_result("add window object", status, error)?;
        }
        for tab in &model.tabs {
            let panes = tab.panes.iter().map(|pane| pane.0).collect::<Vec<_>>();
            let mut error = std::ptr::null_mut();
            // SAFETY: String and slice storage remains live for the duration of the call.
            let status = unsafe {
                toyoterm_mruby_add_tab(
                    self.state.as_ptr(),
                    tab.id.0,
                    tab.title.as_ptr().cast(),
                    tab.title.len(),
                    panes.as_ptr(),
                    panes.len(),
                    &mut error,
                )
            };
            typed_call_result("add tab object", status, error)?;
        }
        for pane in &model.panes {
            let (cwd, cwd_len, cwd_available) =
                pane.cwd.as_deref().map_or((std::ptr::null(), 0, 0), |cwd| {
                    (cwd.as_ptr().cast::<c_char>(), cwd.len(), 1)
                });
            let mut error = std::ptr::null_mut();
            // SAFETY: Optional string storage remains live for the duration of the call.
            let status = unsafe {
                toyoterm_mruby_add_pane(
                    self.state.as_ptr(),
                    pane.id.0,
                    pane.title.as_ptr().cast(),
                    pane.title.len(),
                    cwd,
                    cwd_len,
                    cwd_available,
                    pane.pid.unwrap_or_default().into(),
                    i32::from(pane.pid.is_some()),
                    i32::from(pane.command_running),
                    pane.last_exit_status.unwrap_or_default(),
                    i32::from(pane.last_exit_status.is_some()),
                    pane.screen_text.as_ptr().cast(),
                    pane.screen_text.len(),
                    &mut error,
                )
            };
            typed_call_result("add pane object", status, error)?;
        }
        Ok(())
    }

    pub(super) fn set_clipboard_text(&mut self, text: Option<&str>) -> Result<(), ScriptError> {
        let (pointer, length, available) = match text {
            Some(text) => (text.as_ptr().cast::<c_char>(), text.len(), 1),
            None => (std::ptr::null(), 0, 0),
        };
        let mut error = std::ptr::null_mut();
        // SAFETY: The optional string remains live for the call and length bounds the pointer.
        let status = unsafe {
            toyoterm_mruby_set_clipboard_text(
                self.state.as_ptr(),
                pointer,
                length,
                available,
                &mut error,
            )
        };
        typed_call_result("set clipboard text", status, error)
    }

    pub(super) fn emit_event(&mut self, event: &RubyEvent) -> Result<(), ScriptError> {
        let (title, title_length, title_available) = optional_string_parts(event.title.as_deref());
        let (cwd, cwd_length, cwd_available) = optional_string_parts(event.cwd.as_deref());
        let mut error = std::ptr::null_mut();
        // SAFETY: All optional string storage remains live for the duration of the call.
        let status = unsafe {
            toyoterm_mruby_emit_event(
                self.state.as_ptr(),
                event.name.as_ptr().cast(),
                event.name.len(),
                event.workspace.map_or(u64::MAX, |id| id.0),
                event.window.map_or(u64::MAX, |id| id.0),
                event.tab.map_or(u64::MAX, |id| id.0),
                event.pane.map_or(u64::MAX, |id| id.0),
                title,
                title_length,
                title_available,
                cwd,
                cwd_length,
                cwd_available,
                event.exit_status.unwrap_or_default(),
                i32::from(event.exit_status.is_some()),
                &mut error,
            )
        };
        typed_call_result("emit native event", status, error)
    }

    pub(super) fn eval_with_filename(
        &mut self,
        source: &str,
        filename: &str,
    ) -> Result<String, ScriptError> {
        let source = CString::new(source)
            .map_err(|_| ScriptError::new("evaluate mruby", "source contains a NUL byte"))?;
        let filename = CString::new(filename)
            .map_err(|_| ScriptError::new("evaluate mruby", "filename contains a NUL byte"))?;
        let mut output = std::ptr::null_mut();
        // SAFETY: `state` is live, strings are NUL terminated, and the shim initializes `output`.
        let status = unsafe {
            toyoterm_mruby_eval(
                self.state.as_ptr(),
                source.as_ptr(),
                filename.as_ptr(),
                &mut output,
            )
        };
        let output = NonNull::new(output)
            .ok_or_else(|| ScriptError::new("evaluate mruby", "failed to allocate result"))?;
        // SAFETY: The shim returns a NUL-terminated allocation which remains live until freed below.
        let text = unsafe { CStr::from_ptr(output.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: `output` was allocated by the shim and has not been freed yet.
        unsafe { toyoterm_mruby_string_free(output.as_ptr()) };

        match status {
            0 => Ok(text),
            1 => Err(ScriptError::new("evaluate mruby", text)),
            _ => Err(ScriptError::new(
                "evaluate mruby",
                "mruby evaluation failed",
            )),
        }
    }
}

fn typed_call_result(
    operation: &'static str,
    status: i32,
    error: *mut c_char,
) -> Result<(), ScriptError> {
    let message = NonNull::new(error).map(|error| {
        // SAFETY: Error strings are NUL-terminated allocations owned by the shim.
        let message = unsafe { CStr::from_ptr(error.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: The allocation came from the shim and is freed exactly once.
        unsafe { toyoterm_mruby_string_free(error.as_ptr()) };
        message
    });
    match status {
        0 => Ok(()),
        1 => Err(ScriptError::new(
            operation,
            message.unwrap_or_else(|| "mruby call failed without an exception".to_owned()),
        )),
        _ => Err(ScriptError::new(
            operation,
            message.unwrap_or_else(|| "mruby typed call failed".to_owned()),
        )),
    }
}

fn optional_string_parts(value: Option<&str>) -> (*const c_char, usize, i32) {
    value.map_or((std::ptr::null(), 0, 0), |value| {
        (value.as_ptr().cast(), value.len(), 1)
    })
}

impl Drop for MrubyRuntime {
    fn drop(&mut self) {
        // SAFETY: This is the only owner, and Drop runs exactly once.
        unsafe { toyoterm_mruby_close(self.state.as_ptr()) };
    }
}
