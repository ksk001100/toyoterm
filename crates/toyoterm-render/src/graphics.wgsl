struct Params { rect: vec4<f32>, reserved: vec4<f32> }
@group(0) @binding(0) var pixels: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;
@group(0) @binding(2) var<uniform> params: Params;
struct Vertex { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vs_main(@builtin(vertex_index) index: u32) -> Vertex {
    let corners = array<vec2<f32>,6>(vec2(0.,0.),vec2(1.,0.),vec2(0.,1.),vec2(0.,1.),vec2(1.,0.),vec2(1.,1.));
    let uv = corners[index];
    return Vertex(vec4(params.rect.xy + uv*params.rect.zw,0.,1.),uv);
}
@fragment fn fs_main(vertex: Vertex) -> @location(0) vec4<f32> {
    return textureSample(pixels,image_sampler,vertex.uv);
}
