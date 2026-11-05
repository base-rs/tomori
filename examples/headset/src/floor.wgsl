// A flat, coloured floor around the space, for the placeholder environment.

struct Eye {
    scene_to_clip: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> eye: Eye;

const EXTENT: f32 = 20.0;
const CORNERS = array<vec2<f32>, 6>(
    vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
    vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0),
);

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = CORNERS[index] * EXTENT;
    return eye.scene_to_clip * vec4(corner.x, 0.0, corner.y, 1.0);
}

@fragment
fn fragment() -> @location(0) vec4<f32> {
    return vec4(0.55, 0.25, 0.45, 1.0);
}
