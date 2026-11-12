// Tomori's own cubes: the swarm, workers, satellites, rings, pads, rays and volumes' solids, each an instance of a unit
// cube, self-lit, and hazed by the scene's shared haze, as everything is. See `design/rendering.md`.
struct Camera {
    view_projection: mat4x4<f32>,
    // Where the eye is in the scene, in `xyz`.
    eye: vec4<f32>,
    // The scene's haze, the environment's: its colour, and its density in `w`, for `hazed`.
    haze: vec4<f32>,
    // Half the view's height in pixels: the pixels across one unit of normalised device coordinates.
    pixels: f32,
}
@group(0) @binding(0) var<uniform> camera: Camera;

// The fewest pixels a cube's largest side spans however far away it is, so that a particle never falls between the
// samples of a pixel and goes missing, thinning the cloud it is part of; grown, it is dimmed by as much as its area grew,
// so that it adds the light it would, rather than more.
const SMALLEST: f32 = 1.0;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) world: vec3<f32>,
}

// How many times larger a cube of `scale` at `position` is drawn so that its largest side spans at least `SMALLEST`
// pixels: 1 for any cube that already does.
fn growth(position: vec3<f32>, scale: vec3<f32>) -> f32 {
    let matrix = camera.view_projection;
    let depth = (matrix * vec4<f32>(position, 1.0)).w;
    // Clip coordinates down the view for each of the scene's units across it, which over `depth` are normalised device
    // coordinates, however the scene is scaled into the view.
    let down = length(vec3<f32>(matrix[0].y, matrix[1].y, matrix[2].y));
    let side = max(max(scale.x, scale.y), scale.z);
    let spans = side * down * camera.pixels / max(depth, 1e-6);
    return max(SMALLEST / max(spans, 1e-6), 1.0);
}

@vertex
fn vs_main(
    @location(0) vertex: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) position: vec3<f32>,
    @location(3) scale: vec3<f32>,
    @location(4) color: vec3<f32>,
) -> VertexOutput {
    let grown = growth(position, scale);
    let drawn = scale * grown;
    var out: VertexOutput;
    let world = position + vertex * drawn;
    out.clip_position = camera.view_projection * vec4<f32>(world, 1.0);
    out.normal = normal;
    out.color = color / (grown * grown);
    out.world = world;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let light = 0.5 + 0.5 * max(dot(normalize(in.normal), normalize(vec3<f32>(0.4, 0.8, 0.5))), 0.0);
    return vec4<f32>(hazed(in.color * light, camera.haze, distance(in.world, camera.eye.xyz)), 1.0);
}
