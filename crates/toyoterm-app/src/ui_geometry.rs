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
