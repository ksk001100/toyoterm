use std::fmt;
use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::{AlacrittyTerminalBackend, GpuRenderer, PtySize, TerminalBackend, TextLayout};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellMetrics {
    pub width: f64,
    pub height: f64,
    pub horizontal_padding: u32,
    pub vertical_padding: u32,
}

impl Default for CellMetrics {
    fn default() -> Self {
        Self {
            width: 9.0,
            height: 18.0,
            horizontal_padding: 8,
            vertical_padding: 8,
        }
    }
}

impl CellMetrics {
    pub fn terminal_size(self, window_size: PhysicalSize<u32>) -> PtySize {
        self.terminal_size_at_scale(window_size, 1.0)
    }

    pub fn terminal_size_at_scale(
        self,
        window_size: PhysicalSize<u32>,
        scale_factor: f64,
    ) -> PtySize {
        let scale_factor = scale_factor.max(0.1);
        let horizontal_padding = (f64::from(self.horizontal_padding) * scale_factor).round() as u32;
        let vertical_padding = (f64::from(self.vertical_padding) * scale_factor).round() as u32;
        let content_width = window_size
            .width
            .saturating_sub(horizontal_padding.saturating_mul(2));
        let content_height = window_size
            .height
            .saturating_sub(vertical_padding.saturating_mul(2));
        let columns =
            (f64::from(content_width) / (self.width * scale_factor).max(1.0)).floor() as u32;
        let rows =
            (f64::from(content_height) / (self.height * scale_factor).max(1.0)).floor() as u32;
        PtySize::new(
            columns.clamp(2, u16::MAX.into()) as u16,
            rows.clamp(1, u16::MAX.into()) as u16,
        )
    }

    pub fn text_layout(self, scale_factor: f64) -> TextLayout {
        let scale = scale_factor.max(0.1) as f32;
        TextLayout {
            font_size: 14.0 * scale,
            line_height: self.height as f32 * scale,
            cell_width: self.width as f32 * scale,
            horizontal_padding: self.horizontal_padding as f32 * scale,
            vertical_padding: self.vertical_padding as f32 * scale,
        }
    }
}

#[derive(Debug)]
pub struct AppError(String);

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for AppError {}

pub fn run_gui() -> Result<(), AppError> {
    let event_loop = EventLoop::new().map_err(|error| AppError(error.to_string()))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = ToyotermApplication::default();
    event_loop
        .run_app(&mut app)
        .map_err(|error| AppError(error.to_string()))?;
    match app.fatal_error {
        Some(error) => Err(AppError(error)),
        None => Ok(()),
    }
}

#[derive(Default)]
struct ToyotermApplication {
    window: Option<Arc<Window>>,
    renderer: Option<GpuRenderer>,
    terminal: AlacrittyTerminalBackend,
    cell_metrics: CellMetrics,
    fatal_error: Option<String>,
}

impl ApplicationHandler for ToyotermApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("toyoterm")
            .with_inner_size(LogicalSize::new(960.0, 600.0))
            .with_min_inner_size(LogicalSize::new(320.0, 180.0));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.fail(event_loop, format!("create window: {error}"));
                return;
            }
        };
        let mut renderer = match pollster::block_on(GpuRenderer::new(window.clone())) {
            Ok(renderer) => renderer,
            Err(error) => {
                self.fail(event_loop, error.to_string());
                return;
            }
        };
        self.resize_terminal(window.inner_size(), window.scale_factor());
        self.terminal
            .advance(b"toyoterm\r\nGPU text renderer ready\r\n");
        self.sync_renderer(&mut renderer, window.scale_factor());
        window.request_redraw();
        self.renderer = Some(renderer);
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.clone() else {
            return;
        };
        if window.id() != window_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size);
                }
                self.resize_terminal(size, window.scale_factor());
                self.sync_active_renderer(window.scale_factor());
                window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let size = window.inner_size();
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size);
                }
                self.resize_terminal(size, scale_factor);
                self.sync_active_renderer(scale_factor);
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Some(renderer) = self.renderer.as_mut()
                    && let Err(error) = renderer.render()
                {
                    self.fail(event_loop, error.to_string());
                }
            }
            _ => {}
        }
    }
}

impl ToyotermApplication {
    fn resize_terminal(&mut self, window_size: PhysicalSize<u32>, scale_factor: f64) {
        let size = self
            .cell_metrics
            .terminal_size_at_scale(window_size, scale_factor);
        self.terminal.resize(size.columns, size.rows);
    }

    fn sync_active_renderer(&mut self, scale_factor: f64) {
        let snapshot = self.terminal.snapshot();
        let cursor = self.terminal.cursor();
        let layout = self.cell_metrics.text_layout(scale_factor);
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.update_terminal(&snapshot, cursor, layout);
        }
    }

    fn sync_renderer(&self, renderer: &mut GpuRenderer, scale_factor: f64) {
        renderer.update_terminal(
            &self.terminal.snapshot(),
            self.terminal.cursor(),
            self.cell_metrics.text_layout(scale_factor),
        );
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.fatal_error = Some(error);
        event_loop.exit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_grid_from_physical_window_size() {
        let metrics = CellMetrics::default();
        assert_eq!(
            metrics.terminal_size(PhysicalSize::new(916, 556)),
            PtySize::new(100, 30)
        );
    }

    #[test]
    fn grid_dimensions_never_reach_zero() {
        let metrics = CellMetrics::default();
        assert_eq!(
            metrics.terminal_size(PhysicalSize::new(0, 0)),
            PtySize::new(2, 1)
        );
    }

    #[test]
    fn hidpi_scaling_preserves_logical_grid_size() {
        let metrics = CellMetrics::default();
        let logical = metrics.terminal_size_at_scale(PhysicalSize::new(916, 556), 1.0);
        let hidpi = metrics.terminal_size_at_scale(PhysicalSize::new(1832, 1112), 2.0);
        assert_eq!(logical, hidpi);
    }
}
