struct Parameters {
    background: vec4<f32>, // Linear RGB and window opacity.
    image: vec4<f32>, // Cover UV scale, image opacity, premultiplied surface flag.
};
@group(0) @binding(0) var wallpaper: texture_2d<f32>;
@group(0) @binding(1) var wallpaper_sampler: sampler;
@group(0) @binding(2) var<uniform> params: Parameters;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    let positions = array<vec2<f32>, 3>(vec2(-1.0, 1.0), vec2(-1.0, -3.0), vec2(3.0, 1.0));
    let p = positions[index];
    var output: VertexOutput;
    output.position = vec4(p, 0.0, 1.0);
    output.uv = vec2(p.x + 1.0, 1.0 - p.y) * 0.5;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let uv = (input.uv - vec2(0.5)) * params.image.xy + vec2(0.5);
    let pixel = textureSample(wallpaper, wallpaper_sampler, uv);
    let rgb = mix(params.background.rgb, pixel.rgb, pixel.a * params.image.z);
    let multiplier = mix(1.0, params.background.a, params.image.w);
    return vec4(rgb * multiplier, params.background.a);
}
