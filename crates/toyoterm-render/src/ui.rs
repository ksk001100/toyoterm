use super::*;

pub(super) fn create_ui_pipeline(device: &Device, format: wgpu::TextureFormat) -> RenderPipeline {
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
        label: Some("toyoterm UI pipeline"),
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
                blend: Some(BlendState::ALPHA_BLENDING),
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
