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

pub(super) fn edge_bar_layout(
    window_size: PhysicalSize<u32>,
    chrome_height: u32,
    config: &ToyotermConfig,
    scale_factor: f64,
) -> (PaneRect, Vec<(StatusBarPosition, PaneRect)>) {
    let mut content = PaneRect::new(
        0,
        chrome_height.min(window_size.height),
        window_size.width,
        window_size.height.saturating_sub(chrome_height),
    );
    let height = scaled_ui_size(config.ui.status_bar_height, scale_factor);
    let mut bars = Vec::with_capacity(config.status_bars.len());
    for position in [StatusBarPosition::Top, StatusBarPosition::Bottom] {
        if !config
            .status_bars
            .iter()
            .any(|bar| bar.position == position)
        {
            continue;
        }
        let rect = match position {
            StatusBarPosition::Top => {
                let size = height.min(content.height);
                let rect = PaneRect::new(content.x, content.y, content.width, size);
                content.y = content.y.saturating_add(size);
                content.height = content.height.saturating_sub(size);
                rect
            }
            StatusBarPosition::Bottom => {
                let size = height.min(content.height);
                content.height = content.height.saturating_sub(size);
                PaneRect::new(
                    content.x,
                    content.y.saturating_add(content.height),
                    content.width,
                    size,
                )
            }
        };
        bars.push((position, rect));
    }
    (content, bars)
}

pub(super) fn config_error_height(scale_factor: f64, log_expanded: bool) -> u32 {
    let logical_height = if log_expanded { 240.0 } else { 120.0 };
    (logical_height * scale_factor.max(0.1)).round() as u32
}
