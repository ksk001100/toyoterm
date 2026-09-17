use super::*;

pub(super) fn create_ui_pipeline(device: &Device, format: wgpu::TextureFormat) -> RenderPipeline {
    create_ui_pipeline_with_blend(
        device,
        format,
        BlendState::ALPHA_BLENDING,
        "toyoterm UI pipeline",
    )
}

pub(super) fn create_ui_replace_pipeline(
    device: &Device,
    format: wgpu::TextureFormat,
) -> RenderPipeline {
    create_ui_pipeline_with_blend(
        device,
        format,
        BlendState::REPLACE,
        "toyoterm transparent-cell pipeline",
    )
}

fn create_ui_pipeline_with_blend(
    device: &Device,
    format: wgpu::TextureFormat,
    blend: BlendState,
    label: &'static str,
) -> RenderPipeline {
    let shader = device.create_shader_module(ShaderModuleDescriptor {
        label: Some("toyoterm UI shader"),
        source: ShaderSource::Wgsl(
            r#"
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(input.position, 0.0, 1.0);
    output.color = input.color;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}
"#
            .into(),
        ),
    });
    device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some(label),
        layout: None,
        vertex: VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: PipelineCompilationOptions::default(),
            buffers: &[Some(UiVertex::LAYOUT)],
        },
        primitive: PrimitiveState {
            topology: PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: PipelineCompilationOptions::default(),
            targets: &[Some(ColorTargetState {
                format,
                blend: Some(blend),
                write_mask: ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

pub(super) fn push_ui_rect(
    vertices: &mut Vec<UiVertex>,
    rect: PaneRect,
    color: [f32; 4],
    surface_width: u32,
    surface_height: u32,
) {
    if rect.width == 0 || rect.height == 0 || surface_width == 0 || surface_height == 0 {
        return;
    }
    let left = rect.x as f32 / surface_width as f32 * 2.0 - 1.0;
    let right = rect.x.saturating_add(rect.width) as f32 / surface_width as f32 * 2.0 - 1.0;
    let top = 1.0 - rect.y as f32 / surface_height as f32 * 2.0;
    let bottom = 1.0 - rect.y.saturating_add(rect.height) as f32 / surface_height as f32 * 2.0;
    for position in [
        [left, top],
        [left, bottom],
        [right, bottom],
        [left, top],
        [right, bottom],
        [right, top],
    ] {
        vertices.push(UiVertex { position, color });
    }
}

pub(super) fn inset_rect(rect: PaneRect, left: u32, top: u32, right: u32, bottom: u32) -> PaneRect {
    PaneRect::new(
        rect.x.saturating_add(left),
        rect.y.saturating_add(top),
        rect.width.saturating_sub(left.saturating_add(right)),
        rect.height.saturating_sub(top.saturating_add(bottom)),
    )
}

pub(super) fn rgba(rgb: [u8; 3], alpha: f32) -> [f32; 4] {
    [
        srgb_channel_to_linear(rgb[0]),
        srgb_channel_to_linear(rgb[1]),
        srgb_channel_to_linear(rgb[2]),
        alpha,
    ]
}

pub(super) fn srgb_channel_to_linear(channel: u8) -> f32 {
    let value = f32::from(channel) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

// Keep all four edges inside the pane, without blending the corners twice.
pub(super) fn pane_border_rects(rect: PaneRect, width: u32) -> [PaneRect; 4] {
    let top = width.min(rect.height);
    let bottom = width.min(rect.height - top);
    let left = width.min(rect.width);
    let right = width.min(rect.width - left);
    let middle_height = rect.height - top - bottom;
    [
        PaneRect::new(rect.x, rect.y, rect.width, top),
        PaneRect::new(
            rect.x,
            rect.y.saturating_add(rect.height - bottom),
            rect.width,
            bottom,
        ),
        PaneRect::new(rect.x, rect.y.saturating_add(top), left, middle_height),
        PaneRect::new(
            rect.x.saturating_add(rect.width - right),
            rect.y.saturating_add(top),
            right,
            middle_height,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borders_cover_each_edge_once_and_leave_the_interior_clear() {
        // Includes hidden borders, empty/tiny panes, and widths larger than a pane.
        for (w, h) in [(10, 8), (1, 1), (0, 5), (5, 0), (3, 7)] {
            for width in [0, 1, 2, 20, u32::MAX] {
                let pane = PaneRect::new(3, 4, w, h);
                let borders = pane_border_rects(pane, width);
                for edge in borders.iter().filter(|r| r.width > 0 && r.height > 0) {
                    assert!(edge.x >= pane.x && edge.y >= pane.y);
                    assert!(edge.x + edge.width <= pane.x + w);
                    assert!(edge.y + edge.height <= pane.y + h);
                }
                for y in 0..h {
                    for x in 0..w {
                        let count = borders
                            .iter()
                            .filter(|r| {
                                let px = pane.x + x;
                                let py = pane.y + y;
                                px >= r.x && px < r.x + r.width && py >= r.y && py < r.y + r.height
                            })
                            .count();
                        let on_edge =
                            x < width || y < width || w - 1 - x < width || h - 1 - y < width;
                        assert_eq!(
                            count,
                            usize::from(on_edge),
                            "{w}x{h}, width {width}, ({x}, {y})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "requires a working GPU or software adapter"]
    fn replace_pipeline_writes_transparent_cell_alpha() {
        pollster::block_on(async {
            let instance = Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance.request_adapter(&Default::default()).await.unwrap();
            let (device, queue) = adapter
                .request_device(&DeviceDescriptor::default())
                .await
                .unwrap();
            let format = wgpu::TextureFormat::Rgba8Unorm;
            let pipeline = create_ui_replace_pipeline(&device, format);
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("transparent cell test target"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = target.create_view(&Default::default());
            let mut vertices = Vec::new();
            push_ui_rect(
                &mut vertices,
                PaneRect::new(0, 0, 1, 1),
                [0.5, 0.0, 0.0, 0.5],
                1,
                1,
            );
            let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("transparent cell test vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: BufferUsages::VERTEX,
            });
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("transparent cell test readback"),
                size: 256,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let attachments = [Some(RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color::BLUE),
                        store: StoreOp::Store,
                    },
                })];
                let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    color_attachments: &attachments,
                    ..Default::default()
                });
                pass.set_pipeline(&pipeline);
                pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                pass.draw(0..vertices.len() as u32, 0..1);
            }
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(1),
                    },
                },
                target.size(),
            );
            queue.submit([encoder.finish()]);
            let (sender, receiver) = std::sync::mpsc::channel();
            readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    sender.send(result).unwrap()
                });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            receiver.recv().unwrap().unwrap();
            let bytes = readback.slice(..).get_mapped_range().unwrap();
            for (actual, expected) in bytes[..4].iter().zip([128, 0, 0, 128]) {
                assert!(i32::from(*actual).abs_diff(expected) <= 1);
            }
        });
    }
}
