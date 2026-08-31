use std::fmt;
use std::sync::Arc;

use wgpu::{
    Color, CommandEncoderDescriptor, CurrentSurfaceTexture, Device, DeviceDescriptor, Instance,
    LoadOp, Operations, Queue, RenderPassColorAttachment, RenderPassDescriptor, StoreOp, Surface,
    SurfaceConfiguration, TextureViewDescriptor,
};
use winit::dpi::PhysicalSize;
use winit::window::Window;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderOutcome {
    Presented,
    Skipped,
}

#[derive(Debug)]
pub struct RenderError {
    operation: &'static str,
    message: String,
}

impl RenderError {
    fn new(operation: &'static str, error: impl fmt::Display) -> Self {
        Self {
            operation,
            message: error.to_string(),
        }
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.operation, self.message)
    }
}

impl std::error::Error for RenderError {}

pub struct GpuRenderer {
    instance: Instance,
    window: Arc<Window>,
    surface: Surface<'static>,
    device: Device,
    queue: Queue,
    configuration: SurfaceConfiguration,
    suspended: bool,
    clear_color: Color,
}

impl GpuRenderer {
    pub async fn new(window: Arc<Window>) -> Result<Self, RenderError> {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);
        let instance = Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .map_err(|error| RenderError::new("create GPU surface", error))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|error| RenderError::new("request GPU adapter", error))?;
        let (device, queue) = adapter
            .request_device(&DeviceDescriptor {
                label: Some("toyoterm device"),
                ..Default::default()
            })
            .await
            .map_err(|error| RenderError::new("request GPU device", error))?;
        let mut configuration = surface
            .get_default_config(&adapter, width, height)
            .ok_or_else(|| RenderError::new("configure GPU surface", "unsupported surface"))?;
        configuration.desired_maximum_frame_latency = 1;
        surface.configure(&device, &configuration);

        Ok(Self {
            instance,
            window,
            surface,
            device,
            queue,
            configuration,
            suspended: size.width == 0 || size.height == 0,
            clear_color: Color {
                r: 0.035,
                g: 0.043,
                b: 0.055,
                a: 1.0,
            },
        })
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        self.suspended = size.width == 0 || size.height == 0;
        if self.suspended {
            return;
        }
        self.configuration.width = size.width;
        self.configuration.height = size.height;
        self.surface.configure(&self.device, &self.configuration);
    }

    pub fn size(&self) -> PhysicalSize<u32> {
        PhysicalSize::new(self.configuration.width, self.configuration.height)
    }

    pub fn render(&mut self) -> Result<RenderOutcome, RenderError> {
        if self.suspended {
            return Ok(RenderOutcome::Skipped);
        }

        let (frame, suboptimal) = match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) => (frame, false),
            CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => {
                return Ok(RenderOutcome::Skipped);
            }
            CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.configuration);
                return Ok(RenderOutcome::Skipped);
            }
            CurrentSurfaceTexture::Lost => {
                self.recreate_surface()?;
                return Ok(RenderOutcome::Skipped);
            }
            CurrentSurfaceTexture::Validation => {
                return Err(RenderError::new(
                    "acquire GPU frame",
                    "surface validation failed",
                ));
            }
        };

        let view = frame.texture.create_view(&TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("toyoterm frame encoder"),
            });
        {
            let color_attachments = [Some(RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(self.clear_color),
                    store: StoreOp::Store,
                },
            })];
            let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("toyoterm clear pass"),
                color_attachments: &color_attachments,
                ..Default::default()
            });
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);

        if suboptimal {
            self.surface.configure(&self.device, &self.configuration);
        }
        Ok(RenderOutcome::Presented)
    }

    fn recreate_surface(&mut self) -> Result<(), RenderError> {
        self.surface = self
            .instance
            .create_surface(self.window.clone())
            .map_err(|error| RenderError::new("recreate GPU surface", error))?;
        self.surface.configure(&self.device, &self.configuration);
        Ok(())
    }
}
