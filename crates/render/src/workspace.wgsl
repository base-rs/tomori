struct Camera { matrix: mat4x4<f32> }
@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var atlas: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;
struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    // The same at every vertex of a quad, so not interpolated, which could take a join's 1 to just under it.
    @location(2) @interpolate(flat) weight: f32,
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
// As `ui_panel.wgsl`'s, which this must match: glyphs from the atlas's distance fields, box drawing and blocks grown to join, distant text averaged across the pixel, and filled quads from its solid block.
fn coverage(uv: vec2<f32>, weight: f32) -> f32 {
    let size = vec2<f32>(textureDimensions(atlas));
    let at = uv / size;
    let distance = textureSample(atlas, atlas_sampler, at).r;
    let width = max(fwidth(distance), 1e-4);
    let joins = weight >= 0.75;
    let edge = 0.5 - select(weight, weight - 1.0, joins);
    let grow = select(0.5, 1.0, joins);
    let across = dpdx(at);
    let down = dpdy(at);
    // Up to a texel and a half to a pixel, one sample is enough; past that, a grid of up to four by four, each sample spanning no more.
    let texels = max(length(across * size), length(down * size));
    let n = min(i32(ceil(texels / 1.5)), 4);
    if n <= 1 {
        return clamp((distance - edge) / width + grow, 0.0, 1.0);
    }
    // Each sample's edge is as wide as its share of the pixel, in a field that changes by an eighth a texel, from its spread of four; it grows by as many of its widths as there are samples across.
    let sharpest = texels / (8.0 * f32(n));
    let grown = f32(n) * (grow - 0.5) + 0.5;
    var sum = 0.0;
    for (var j = 0; j < n; j++) {
        for (var i = 0; i < n; i++) {
            let offset = (vec2<f32>(f32(i), f32(j)) + 0.5) / f32(n) - 0.5;
            let sample = textureSampleLevel(atlas, atlas_sampler, at + across * offset.x + down * offset.y, 0.0).r;
            sum += clamp((sample - edge) / sharpest + grown, 0.0, 1.0);
        }
    }
    return sum / f32(n * n);
}
@fragment fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(input.color.rgb, input.color.a * coverage(input.uv, input.weight));
}
