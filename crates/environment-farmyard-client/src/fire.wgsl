// The campsite's bonfire: low-poly flame tongues, each a card turned towards the eye about the upright, swaying and
// flickering by the time alone; embers rising from them and fading; and a little flame on a marshmallow that burns. All
// of it glows, added to what is behind it, so that nothing of it needs sorting, and it is drawn only inside the reveal
// sphere while the farmyard grows.

struct Camera {
    inverse_view_projection: mat4x4<f32>,
    // The centre and radius of the sphere the farmyard is drawn inside while it grows; a negative radius fills the world.
    reveal: vec4<f32>,
    view_projection: mat4x4<f32>,
    // Where the eye is.
    eye: vec4<f32>,
    // The haze's colour, and its density in its fourth component.
    haze: vec4<f32>,
    // The time in seconds, in its first component.
    time: vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera: Camera;

// A corner of a flame or an ember: the foot of what it belongs to; the corner, across it from -1 to 1 and up it from 0
// to 1; its colour, with how brightly it glows; and how it moves: its phase, its height, its width, and what it is, a
// flame 0 or an ember 1.
struct Corner {
    @location(0) foot: vec3<f32>,
    @location(1) corner: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) motion: vec4<f32>,
}

struct Shaded {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) color: vec3<f32>,
}

@vertex
fn fire_vertex(corner: Corner) -> Shaded {
    let t = camera.time.x;
    let phase = corner.motion.x;
    // Turned towards the eye about the upright.
    var towards = camera.eye.xyz - corner.foot;
    towards.y = 0.0;
    let along = select(vec3<f32>(1.0, 0.0, 0.0), normalize(vec3<f32>(towards.z, 0.0, -towards.x)), length(towards) > 1e-4);
    var world: vec3<f32>;
    var glow = corner.color.a;
    if corner.motion.w < 0.5 {
        // A flame: taller and shorter by turns, its tip swaying, more the higher it is.
        let flicker = 0.8 + 0.12 * sin(t * 9.1 + phase * 7.0) + 0.08 * sin(t * 23.7 + phase * 13.0);
        let up = corner.corner.y;
        let sway = (sin(t * 3.3 + phase * 5.0) * 0.05 + sin(t * 7.9 + phase * 3.0) * 0.025) * up * up;
        world = corner.foot
            + along * (corner.corner.x * corner.motion.z * (1.0 - 0.35 * up) + sway)
            + vec3<f32>(sway * 0.4, up * corner.motion.y * flicker, 0.0);
        glow = glow * (0.85 + 0.15 * flicker);
    } else {
        // An ember: rising from the flames over its life, drifting, shrinking and fading as it goes.
        let life = fract(t * (0.35 + fract(phase * 3.7) * 0.25) + phase);
        let drift = vec3<f32>(sin(t * 1.7 + phase * 11.0), 0.0, cos(t * 1.3 + phase * 7.0)) * 0.12 * life;
        let size = corner.motion.z * (1.0 - life);
        let rise = vec3<f32>(0.0, 0.25 + life * corner.motion.y, 0.0);
        world = corner.foot + rise + drift + along * corner.corner.x * size
            + vec3<f32>(0.0, (corner.corner.y - 0.5) * 2.0 * size, 0.0);
        glow = glow * (1.0 - life) * (1.0 - life);
    }
    var out: Shaded;
    out.position = camera.view_projection * vec4<f32>(world, 1.0);
    out.world = world;
    out.color = corner.color.rgb * glow;
    return out;
}

@fragment
fn fire_fragment(in: Shaded) -> @location(0) vec4<f32> {
    if camera.reveal.w >= 0.0 && distance(in.world, camera.reveal.xyz) > camera.reveal.w {
        discard;
    }
    return vec4<f32>(in.color, 1.0);
}
