// An image panel's image, drawn over its panel's face in the panel's plane, from a texture with every mipmap level,
// sampled trilinearly, so that it is sharp near and does not shimmer far. Opaque: the server lays transparency over white.
struct Camera { transform: mat4x4<f32> }
@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var image: texture_2d<f32>;
@group(1) @binding(1) var image_sampler: sampler;
struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex
fn vs_main(@location(0) position: vec2<f32>, @location(1) uv: vec2<f32>) -> Output {
    var out: Output;
    out.position = camera.transform * vec4<f32>(position, 0.0, 1.0);
    out.uv = uv;
    return out;
}
@fragment
fn fs_main(in: Output) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSample(image, image_sampler, in.uv).rgb, 1.0);
}
