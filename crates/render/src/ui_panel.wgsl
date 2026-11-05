struct Camera { transform: mat4x4<f32> }
@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var atlas: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;
struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) weight: f32,
}
@vertex
fn vs_main(
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) weight: f32,
) -> Output {
    var out: Output;
    out.position = camera.transform * vec4<f32>(position, 0.0, 1.0);
    out.color = color;
    out.uv = uv;
    out.weight = weight;
    return out;
}
// How much of the pixel a glyph covers: its distance field crosses one half, less its weight, at its edge, and the field's change across the pixel antialiases the edge at any distance. Filled quads sample the atlas's solid block, which is wholly inside.
fn coverage(uv: vec2<f32>, weight: f32) -> f32 {
    let distance = textureSample(atlas, atlas_sampler, uv / vec2<f32>(textureDimensions(atlas))).r;
    let width = max(fwidth(distance), 1e-4);
    return clamp((distance - 0.5 + weight) / width + 0.5, 0.0, 1.0);
}
@fragment
fn fs_main(in: Output) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color.rgb, in.color.a * coverage(in.uv, in.weight));
}
