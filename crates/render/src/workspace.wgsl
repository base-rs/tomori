struct Camera { matrix: mat4x4<f32> }
@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var atlas: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;
struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) weight: f32,
}
@vertex fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) weight: f32,
) -> VertexOut {
    var out: VertexOut;
    out.position = camera.matrix * vec4<f32>(position, 1.0);
    out.color = color;
    out.uv = uv;
    out.weight = weight;
    return out;
}
// As `ui_panel.wgsl`'s: glyphs from the atlas's distance fields, and filled quads from its solid block.
fn coverage(uv: vec2<f32>, weight: f32) -> f32 {
    let distance = textureSample(atlas, atlas_sampler, uv / vec2<f32>(textureDimensions(atlas))).r;
    let width = max(fwidth(distance), 1e-4);
    return clamp((distance - 0.5 + weight) / width + 0.5, 0.0, 1.0);
}
@fragment fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(input.color.rgb, input.color.a * coverage(input.uv, input.weight));
}
