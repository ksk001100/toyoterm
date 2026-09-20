use super::*;
use crate::pane_lifecycle::reset_scroll_for_input;

pub(super) fn apply_clipboard_command(
    command: ClipboardCommand,
    platform: &mut PlatformState,
    terminal_runtime: &mut TerminalRuntime,
    ui: &mut UiState,
) -> Result<(), String> {
    match command {
        ClipboardCommand::Write(text) => ui.pending_clipboard_writes.push(text),
        ClipboardCommand::CopySelection { pane } => {
            copy_selection(pane, platform, terminal_runtime)?
        }
        ClipboardCommand::Paste { pane } => paste(pane, platform, terminal_runtime)?,
        ClipboardCommand::YankSelection { pane } => {
            let selected = ui
                .visual_selection
                .as_ref()
                .is_some_and(|selection| selection.anchor.is_some());
            if selected {
                copy_selection(pane, platform, terminal_runtime)?;
                ui.exit_visual_mode(terminal_runtime, Some(pane));
            }
        }
    }
    Ok(())
}

fn copy_selection(
    pane: PaneId,
    platform: &mut PlatformState,
    terminal_runtime: &TerminalRuntime,
) -> Result<(), String> {
    let Some(text) = terminal_runtime
        .pane_runtimes
        .get(&pane)
        .and_then(|runtime| runtime.terminal.selected_text())
        .filter(|text| !text.is_empty())
    else {
        return Ok(());
    };
    clipboard(platform)?
        .set_text(text)
        .map_err(|error| format!("copy to clipboard: {error}"))
}

fn paste(
    pane: PaneId,
    platform: &mut PlatformState,
    terminal_runtime: &mut TerminalRuntime,
) -> Result<(), String> {
    let mode = terminal_runtime
        .pane_runtimes
        .get(&pane)
        .map(|runtime| runtime.terminal.mode())
        .unwrap_or_default();
    let text = clipboard(platform)?
        .get_text()
        .map_err(|error| format!("paste from clipboard: {error}"))?;
    let bytes = encode_paste(&text, mode);
    let runtime = terminal_runtime
        .pane_runtimes
        .get_mut(&pane)
        .ok_or_else(|| format!("pane {pane} has no runtime"))?;
    reset_scroll_for_input(&mut runtime.terminal, &bytes);
    if let Some(session) = runtime.process.pty_session.as_mut() {
        session.write(&bytes).map_err(|error| {
            tracing::error!(
                target: "toyoterm::pty",
                operation = error.operation(),
                %pane,
                bytes = bytes.len(),
                %error,
                "write pane PTY failed"
            );
            error.to_string()
        })?;
    }
    Ok(())
}

fn clipboard(platform: &mut PlatformState) -> Result<&mut Clipboard, String> {
    if platform.clipboard.is_none() {
        platform.clipboard =
            Some(Clipboard::new().map_err(|error| format!("initialize clipboard: {error}"))?);
    }
    Ok(platform
        .clipboard
        .as_mut()
        .expect("clipboard was initialized"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_clipboard_write_is_queued_without_platform_access() {
        let mut platform = PlatformState {
            window: None,
            renderer: None,
            occlusion: WindowOcclusion::default(),
            clipboard: None,
            notification_sender: None,
            #[cfg(target_os = "linux")]
            app_id: None,
        };
        let mut terminals = test_terminal_runtime([]);
        let mut ui = test_ui_state();

        apply_clipboard_command(
            ClipboardCommand::Write("copied".into()),
            &mut platform,
            &mut terminals,
            &mut ui,
        )
        .unwrap();

        assert_eq!(ui.pending_clipboard_writes, ["copied"]);
        assert!(platform.clipboard.is_none());
    }
}
