use super::*;

pub(super) fn scaled_ui_size(value: f32, scale_factor: f64) -> u32 {
    (f64::from(value) * scale_factor.max(0.1)).round() as u32
}

pub(super) fn tab_bar_height(config: &ToyotermConfig, scale_factor: f64) -> u32 {
    if config.ui.tab_bar {
        scaled_ui_size(config.ui.tab_bar_height, scale_factor)
    } else {
        0
    }
}

pub(super) fn workspace_bar_height(config: &ToyotermConfig, scale_factor: f64) -> u32 {
    if config.ui.workspace_bar {
        scaled_ui_size(config.ui.workspace_bar_height, scale_factor)
    } else {
        0
    }
}

pub(super) fn status_bar_rect(window_size: PhysicalSize<u32>, height: u32) -> PaneRect {
    let height = height.min(window_size.height);
    PaneRect::new(
        0,
        window_size.height.saturating_sub(height),
        window_size.width,
        height,
    )
}

pub(super) fn config_error_height(scale_factor: f64, log_expanded: bool) -> u32 {
    let logical_height = if log_expanded { 240.0 } else { 120.0 };
    (logical_height * scale_factor.max(0.1)).round() as u32
}

pub(super) fn spawn_pty_reader(
    pane: PaneId,
    mut reader: Box<dyn Read + Send>,
    event_proxy: EventLoopProxy<AppEvent>,
) -> Result<(), String> {
    thread::Builder::new()
        .name("toyoterm-pty-reader".into())
        .spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => {
                        let _ = event_proxy.send_event(AppEvent::Eof { pane });
                        break;
                    }
                    Ok(count) => {
                        if event_proxy
                            .send_event(AppEvent::Output {
                                pane,
                                bytes: buffer[..count].to_vec(),
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(error) if error.raw_os_error() == Some(5) => {
                        let _ = event_proxy.send_event(AppEvent::Eof { pane });
                        break;
                    }
                    Err(error) => {
                        let _ = event_proxy.send_event(AppEvent::Error {
                            pane,
                            message: format!("read PTY output: {error}"),
                        });
                        break;
                    }
                }
            }
        })
        .map(|_| ())
        .map_err(|error| format!("start PTY reader: {error}"))
}
