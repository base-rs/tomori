// Models drawn with tomori-assets: each instance's meshes, skinned on the GPU from its joint matrices or placed by its
// node's matrix, lit by the scenery's light: a sky and ground ambient, the sun or moon, and a few lamps, with haze.

struct Camera {
    view_projection: mat4x4<f32>,
    // Where the eye is, for haze.
    eye: vec4<f32>,
    // The centre and radius of the sphere the scenery is drawn inside while it grows; a negative radius fills the world.
    reveal: vec4<f32>,
}

struct Lamp {
    // Where it is, and how far it reaches.
    position: vec4<f32>,
    // Its colour, as bright as it is at its centre.
    color: vec4<f32>,
}

struct Lighting {
    // The unit direction towards the sun or moon.
    sun_direction: vec4<f32>,
    sun: vec4<f32>,
    sky: vec4<f32>,
    ground: vec4<f32>,
    // The haze's colour, and its density in its fourth component.
    haze: vec4<f32>,
    lamps: array<Lamp, 4>,
    // How many lamps there are.
    count: vec4<u32>,
}

struct Material {
    base_color: vec4<f32>,
    // Its light, and whether it is cut out below half alpha in its fourth component.
    emissive: vec4<f32>,
}

// One instance of one primitive: where its node is, and where its joint matrices begin, if it is skinned.
struct Placed {
    world: mat4x4<f32>,
    // The first joint matrix, and whether the primitive is skinned.
    joints: vec4<u32>,
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<uniform> lighting: Lighting;
@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_texture: texture_2d<f32>;
@group(1) @binding(2) var base_sampler: sampler;
@group(2) @binding(0) var<storage, read> placed: array<Placed>;
@group(2) @binding(1) var<storage, read> joints: array<mat4x4<f32>>;

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) joints_0: vec4<u32>,
    @location(5) joints_1: vec4<u32>,
    @location(6) weights_0: vec4<f32>,
    @location(7) weights_1: vec4<f32>,
}

struct Shaded {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
}

@vertex
fn model_vertex(vertex: Vertex, @builtin(instance_index) instance: u32) -> Shaded {
    let one = placed[instance];
    var transform = one.world;
    if one.joints.y != 0u {
        let base = one.joints.x;
        transform = joints[base + vertex.joints_0.x] * vertex.weights_0.x
            + joints[base + vertex.joints_0.y] * vertex.weights_0.y
            + joints[base + vertex.joints_0.z] * vertex.weights_0.z
            + joints[base + vertex.joints_0.w] * vertex.weights_0.w
            + joints[base + vertex.joints_1.x] * vertex.weights_1.x
            + joints[base + vertex.joints_1.y] * vertex.weights_1.y
            + joints[base + vertex.joints_1.z] * vertex.weights_1.z
            + joints[base + vertex.joints_1.w] * vertex.weights_1.w;
    }
    let world = transform * vec4<f32>(vertex.position, 1.0);
    var out: Shaded;
    out.position = camera.view_projection * world;
    out.world = world.xyz;
    out.normal = (transform * vec4<f32>(vertex.normal, 0.0)).xyz;
    out.uv = vertex.uv;
    out.color = vertex.color;
    return out;
}

fn shade(in: Shaded, facing: bool) -> vec4<f32> {
    // Derivatives first, in uniform control flow: a mesh without normals is lit by each triangle's own, from how its
    // position changes across the screen.
    let flat = cross(dpdx(in.world), dpdy(in.world));
    let surface = material.base_color * in.color * textureSample(base_texture, base_sampler, in.uv);
    if material.emissive.w > 0.0 && surface.a < 0.5 {
        discard;
    }
    // A normal faces the eye on whichever side of a two-sided surface is seen.
    var normal = in.normal;
    if dot(normal, normal) < 0.25 {
        normal = select(-flat, flat, dot(flat, camera.eye.xyz - in.world) >= 0.0);
    } else if !facing {
        normal = -normal;
    }
    normal = normalize(normal);
    var light = mix(lighting.ground.rgb, lighting.sky.rgb, normal.y * 0.5 + 0.5)
        + lighting.sun.rgb * max(dot(normal, lighting.sun_direction.xyz), 0.0);
    for (var index = 0u; index < min(lighting.count.x, 4u); index += 1u) {
        let lamp = lighting.lamps[index];
        let offset = lamp.position.xyz - in.world;
        let distance = length(offset);
        let reach = max(lamp.position.w, 0.001);
        let falloff = clamp(1.0 - distance / reach, 0.0, 1.0);
        let towards = offset / max(distance, 0.0001);
        light += lamp.color.rgb * falloff * falloff * (0.35 + 0.65 * max(dot(normal, towards), 0.0));
    }
    let color = surface.rgb * light + material.emissive.rgb;
    let far = distance(in.world, camera.eye.xyz);
    return vec4<f32>(hazed(color, lighting.haze, far), 1.0);
}

@fragment
fn model_fragment(in: Shaded, @builtin(front_facing) facing: bool) -> @location(0) vec4<f32> {
    return shade(in, facing);
}

// Only inside the reveal sphere, while the scenery grows. Shaded first, as shading takes derivatives.
@fragment
fn revealed_model_fragment(in: Shaded, @builtin(front_facing) facing: bool) -> @location(0) vec4<f32> {
    let color = shade(in, facing);
    if camera.reveal.w >= 0.0 && distance(in.world, camera.reveal.xyz) > camera.reveal.w {
        discard;
    }
    return color;
}
