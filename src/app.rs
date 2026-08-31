use std::fmt;
use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::{AlacrittyTerminalBackend, GpuRenderer, PtySize, TerminalBackend};

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
        let content_width = window_size
            .width
            .saturating_sub(self.horizontal_padding.saturating_mul(2));
        let content_height = window_size
            .height
            .saturating_sub(self.vertical_padding.saturating_mul(2));
        let columns = (f64::from(content_width) / self.width.max(1.0)).floor() as u32;
        let rows = (f64::from(content_height) / self.height.max(1.0)).floor() as u32;
        PtySize::new(
            columns.clamp(2, u16::MAX.into()) as u16,
            rows.clamp(1, u16::MAX.into()) as u16,
        )
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
        let renderer = match pollster::block_on(GpuRenderer::new(window.clone())) {
            Ok(renderer) => renderer,
            Err(error) => {
                self.fail(event_loop, error.to_string());
                return;
            }
        };
        self.resize_terminal(window.inner_size());
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
                self.resize_terminal(size);
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
    fn resize_terminal(&mut self, window_size: PhysicalSize<u32>) {
        let size = self.cell_metrics.terminal_size(window_size);
        self.terminal.resize(size.columns, size.rows);
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
}
