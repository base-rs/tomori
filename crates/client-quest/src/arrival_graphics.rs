// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! How the arrival scene looks: placeholder forms for its objects, the pointing ray, the highlight on what it points at, and labels anchored in the world.
//!
//! [`cubes`] and [`label_mesh`] are pure, so they are tested without a GPU. Every object's form comes from [`form`] alone, so the placeholders can be replaced, for example by server constellations or environment previews, without touching the layout or the pointing.
use crate::{
    arrival::{Align, Key, Kind, Label, Object, Scene, clip, facing},
    delegation::Cube,
    server_choice::Source,
};
use bytemuck::{Pod, Zeroable};
use font8x8::{BASIC_FONTS, UnicodeFonts};
use glam::{Quat, Vec3};
use protocol::Activity;
use std::f32::consts::{FRAC_PI_4, TAU};

/// The most cubes arrival adds to the scene.
pub const MAX_CUBES: usize = 2048;
/// The most label vertices, which bounds the buffer.
const MAX_VERTICES: usize = 1 << 19;
const INITIAL_VERTICES: usize = 1 << 14;
/// How far the ray reaches when it meets nothing.
pub const RAY_LENGTH: f32 = 3.0;

/// The colour of the participant cubes around a space, distinct from the swarm's blues so that people read differently from the agent.
const PARTICIPANT: [f32; 3] = [1.0, 0.55, 0.75];
const HOVER: [f32; 3] = [0.45, 0.95, 1.0];
const TEXT: [f32; 3] = [0.92, 0.95, 1.0];
const MUTED: [f32; 3] = [0.48, 0.72, 0.85];
const BACKGROUND: [f32; 3] = [0.012, 0.024, 0.045];
const HOVER_BACKGROUND: [f32; 3] = [0.03, 0.11, 0.15];

/// What to draw this frame.
pub struct Frame<'a> {
    pub scene: &'a Scene,
    /// The object the ray meets.
    pub hovered: Option<&'a Key>,
    /// The ray, from the controller to where it ends.
    pub ray: Option<(Vec3, Vec3)>,
    /// Seconds, for slow motion such as turning.
    pub time: f32,
}

fn cube(position: Vec3, scale: Vec3, color: [f32; 3], orientation: Quat) -> Cube {
    Cube {
        position: position.to_array(),
        scale: scale.to_array(),
        color,
        orientation: orientation.normalize().to_array(),
    }
}

/// A cube from `a` to `b`, `width` thick.
fn bar(a: Vec3, b: Vec3, width: f32, color: [f32; 3]) -> Option<Cube> {
    let along = b - a;
    let length = along.length();
    let direction = along.try_normalize()?;
    Some(cube(
        (a + b) * 0.5,
        Vec3::new(width, width, length),
        color,
        Quat::from_rotation_arc(Vec3::Z, direction),
    ))
}

fn hsv(hue: f32, saturation: f32, value: f32) -> [f32; 3] {
    let h = hue.rem_euclid(1.0) * 6.0;
    let c = value * saturation;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = value - c;
    [r + m, g + m, b + m]
}

/// An object's placeholder form, in the world. Replace this to change how arrival looks.
fn form(object: &Object, time: f32, out: &mut Vec<Cube>) {
    let p = object.position;
    let s = object.size;
    let face = Quat::from_rotation_arc(Vec3::Z, facing(p));
    match object.kind {
        Kind::Server { source, available } => {
            let color = match (available, source) {
                (false, _) => [0.32, 0.33, 0.36],
                (true, Source::Remembered | Source::Override) => [1.0, 0.72, 0.38],
                (true, Source::Discovered) => [0.45, 0.8, 1.0],
                (true, Source::Usb) => [0.62, 0.74, 0.9],
            };
            let turn = Quat::from_rotation_y(time * 0.4) * Quat::from_rotation_x(0.5);
            out.push(cube(p, Vec3::splat(s * 1.3), color, turn));
            if source == Source::Usb {
                out.extend(bar(
                    Vec3::new(p.x, 0.0, p.z),
                    p - Vec3::Y * s,
                    0.004,
                    [0.14, 0.2, 0.26],
                ));
            }
        }
        Kind::Connected => {
            let turn = Quat::from_rotation_y(time * 0.4) * Quat::from_rotation_x(0.5);
            out.push(cube(p, Vec3::splat(s * 1.2), [1.0, 0.72, 0.38], turn));
            for n in 0..8 {
                let angle = n as f32 * TAU / 8.0 + time * 0.3;
                let offset = face * Vec3::new(angle.cos(), angle.sin(), 0.0) * s * 1.7;
                out.push(cube(
                    p + offset,
                    Vec3::splat(s * 0.18),
                    [1.0, 0.85, 0.6],
                    Quat::IDENTITY,
                ));
            }
        }
        Kind::Space {
            activity,
            participants,
            hue,
        } => {
            let (saturation, value) = if activity == Activity::Closed {
                (0.2, 0.45)
            } else {
                (0.6, 0.95)
            };
            let speed = match activity {
                Activity::Running => 1.6,
                Activity::Closed => 0.0,
                _ => 0.25,
            };
            // A cube on its corner reads as a diamond from any side.
            let diamond = Quat::from_rotation_y(time * speed + hue * TAU)
                * Quat::from_rotation_z(FRAC_PI_4)
                * Quat::from_rotation_x(0.6155);
            out.push(cube(
                p,
                Vec3::splat(s * 1.15),
                hsv(hue, saturation, value),
                diamond,
            ));
            if activity == Activity::Waiting {
                let pulse = 0.7 + 0.3 * (time * 3.0).sin();
                out.push(cube(
                    p + Vec3::Y * s * 1.6,
                    Vec3::splat(s * 0.3),
                    [pulse, pulse * 0.7, 0.15],
                    Quat::IDENTITY,
                ));
            }
            let shown = participants.min(12);
            for n in 0..shown {
                let angle = n as f32 * TAU / shown as f32 + time * 0.5;
                out.push(cube(
                    p + Vec3::new(angle.cos(), -0.6, angle.sin()) * s * 1.7,
                    Vec3::splat(s * 0.25),
                    PARTICIPANT,
                    Quat::IDENTITY,
                ));
            }
        }
        Kind::Create { open } => {
            // A plus, which turns into a cross while its rings are open, to step back.
            let twist = face * Quat::from_rotation_z(if open { FRAC_PI_4 } else { 0.0 });
            let color = [0.55, 0.95, 0.72];
            out.push(cube(
                p,
                Vec3::new(s * 2.0, s * 0.45, s * 0.45),
                color,
                twist,
            ));
            out.push(cube(
                p,
                Vec3::new(s * 0.45, s * 2.0, s * 0.45),
                color,
                twist,
            ));
        }
        Kind::Choice => {
            let turn = face * Quat::from_rotation_z(FRAC_PI_4);
            out.push(cube(p, Vec3::splat(s * 1.2), [0.7, 0.85, 1.0], turn));
        }
    }
}

fn brighten(color: [f32; 3], amount: f32) -> [f32; 3] {
    color.map(|v| v + (1.0 - v) * amount)
}

/// Every cube arrival adds: object forms, the highlight on the object pointed at and on the stick's selection, and the ray. An object's size is already scaled by its presence; its colour fades with it too.
pub fn cubes(frame: &Frame) -> Vec<Cube> {
    let mut out = Vec::new();
    for object in &frame.scene.objects {
        if object.presence <= 0.0 {
            continue;
        }
        let hovered = object.choosable && frame.hovered == Some(&object.key);
        let fade = object.presence * (1.0 - 0.7 * object.dim);
        let start = out.len();
        form(object, frame.time, &mut out);
        for cube in &mut out[start..] {
            if hovered {
                let position = Vec3::from_array(cube.position);
                cube.position = (object.position + (position - object.position) * 1.25).to_array();
                cube.scale = cube.scale.map(|v| v * 1.25);
                cube.color = brighten(cube.color, 0.35);
            }
            cube.color = cube.color.map(|v| v * fade);
        }
        let face = Quat::from_rotation_arc(Vec3::Z, facing(object.position));
        if hovered {
            for n in 0..16 {
                let angle = n as f32 * TAU / 16.0 + frame.time;
                let offset = face * Vec3::new(angle.cos(), angle.sin(), 0.0) * object.size * 2.1;
                out.push(cube(
                    object.position + offset,
                    Vec3::splat(object.size * 0.14),
                    HOVER,
                    face,
                ));
            }
        }
        if object.selected && object.choosable {
            out.push(cube(
                object.position - Vec3::Y * object.size * 1.45,
                Vec3::new(object.size * 2.0, 0.006, 0.006),
                [0.9, 0.92, 1.0],
                face,
            ));
        }
    }
    if let Some((origin, end)) = frame.ray {
        let color = if frame.hovered.is_some() {
            HOVER
        } else {
            [0.55, 0.6, 0.75]
        };
        out.extend(bar(origin, end, 0.003, color));
        out.push(cube(
            end,
            Vec3::splat(0.012),
            brighten(color, 0.4),
            Quat::IDENTITY,
        ));
    }
    out.truncate(MAX_CUBES);
    out
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    position: [f32; 3],
    color: [f32; 4],
}

const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];

/// A label and how to draw it.
struct Styled<'a> {
    label: &'a Label,
    hovered: bool,
    /// How opaque it is, from its own alpha and its object's dimming.
    alpha: f32,
}

/// One label's text, on a translucent background, facing [`crate::arrival::HOME`] and anchored where its object is.
fn label_into(styled: &Styled, mesh: &mut Vec<Vertex>) {
    let label = styled.label;
    let p = label.pixel;
    let columns = label
        .lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    if columns == 0 {
        return;
    }
    let normal = facing(label.anchor);
    let right = Vec3::Y.cross(normal);
    let width = columns as f32 * 9.0 * p;
    let height = label.lines.len() as f32 * 11.0 * p;
    let origin = match label.align {
        Align::Left => label.anchor,
        Align::Centre => label.anchor - right * width * 0.5,
        Align::Right => label.anchor - right * width,
    };
    let alpha = styled.alpha;
    if alpha <= 0.0 {
        return;
    }
    let rgba = |rgb: [f32; 3], a: f32| [rgb[0], rgb[1], rgb[2], a * alpha];
    let mut quad = |x: f32, y: f32, w: f32, h: f32, z: f32, color: [f32; 4]| {
        if mesh.len() + 6 > MAX_VERTICES {
            return;
        }
        let corner = origin + right * x - Vec3::Y * y + normal * z;
        for [u, v] in [
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 1.0],
            [0.0, 0.0],
            [1.0, 1.0],
            [0.0, 1.0],
        ] {
            mesh.push(Vertex {
                position: (corner + right * (u * w) - Vec3::Y * (v * h)).to_array(),
                color,
            });
        }
    };
    let (background, opacity) = if styled.hovered {
        (HOVER_BACKGROUND, 0.9)
    } else {
        (BACKGROUND, 0.72)
    };
    quad(
        -3.0 * p,
        -3.0 * p,
        width + 5.0 * p,
        height + 4.0 * p,
        0.0,
        rgba(background, opacity),
    );
    for (row, line) in label.lines.iter().enumerate() {
        let color = rgba(
            match (row, styled.hovered) {
                (0, true) => HOVER,
                (0, false) => TEXT,
                _ => MUTED,
            },
            1.0,
        );
        for (column, ch) in line.chars().enumerate() {
            let glyph = BASIC_FONTS
                .get(ch)
                .or_else(|| BASIC_FONTS.get('?'))
                .unwrap_or([0; 8]);
            for (gy, bits) in glyph.iter().enumerate() {
                let mut gx = 0;
                while gx < 8 {
                    if bits & (1 << gx) == 0 {
                        gx += 1;
                        continue;
                    }
                    let start = gx;
                    while gx < 8 && bits & (1 << gx) != 0 {
                        gx += 1;
                    }
                    quad(
                        (column * 9 + start) as f32 * p,
                        (row * 11 + gy) as f32 * p,
                        (gx - start) as f32 * p,
                        p,
                        0.002,
                        color,
                    );
                }
            }
        }
    }
}

/// The heading, with the full text of the object pointed at beneath it.
fn heading(scene: &Scene, hovered: Option<&Key>) -> Option<Label> {
    let mut heading = scene.heading.clone()?;
    if let Some(object) = hovered.and_then(|key| scene.object(key)) {
        heading.lines.push(clip(&object.description, 48));
    }
    Some(heading)
}

/// Every label's vertices in scene coordinates, the farthest from [`crate::arrival::HOME`] first, so that translucent backgrounds blend over what is behind them.
pub fn label_mesh(scene: &Scene, hovered: Option<&Key>) -> Vec<Vertex> {
    let heading = heading(scene, hovered);
    let mut labels: Vec<Styled> = scene
        .objects
        .iter()
        .map(|object| Styled {
            label: &object.label,
            hovered: object.choosable && hovered == Some(&object.key),
            alpha: object.label.alpha * (1.0 - 0.65 * object.dim),
        })
        .chain(heading.iter().map(|label| Styled {
            label,
            hovered: false,
            alpha: label.alpha,
        }))
        .collect();
    let distance = |styled: &Styled| styled.label.anchor.distance(crate::arrival::HOME);
    labels.sort_by(|a, b| distance(b).total_cmp(&distance(a)));
    let mut mesh = Vec::new();
    for styled in &labels {
        label_into(styled, &mut mesh);
    }
    mesh
}

/// The arrival labels on the GPU, rebuilt only when the scene or what is pointed at changes.
pub struct ArrivalLabels {
    pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    capacity: usize,
    count: u32,
    /// The vertices in the buffer, drawn unless hidden.
    length: u32,
    cached: Option<(Scene, Option<Key>)>,
}

impl ArrivalLabels {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("workspace.wgsl"));
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("arrival labels layout"),
            bind_group_layouts: &[Some(camera_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("arrival labels"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &ATTRIBUTES,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                // Translucent backgrounds must not hide the glyphs or other labels.
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let vertices = Self::buffer(device, INITIAL_VERTICES);
        Self {
            pipeline,
            vertices,
            capacity: INITIAL_VERTICES,
            count: 0,
            length: 0,
            cached: None,
        }
    }

    fn buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("arrival label glyph runs"),
            size: (capacity * size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn hide(&mut self) {
        self.count = 0;
    }

    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
        hovered: Option<&Key>,
    ) {
        let unchanged = self
            .cached
            .as_ref()
            .is_some_and(|(cached, key)| cached == scene && key.as_ref() == hovered);
        if !unchanged {
            let mesh = label_mesh(scene, hovered);
            if mesh.len() > self.capacity {
                self.capacity = mesh.len().next_power_of_two().min(MAX_VERTICES);
                self.vertices = Self::buffer(device, self.capacity);
            }
            if !mesh.is_empty() {
                queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&mesh));
            }
            self.length = mesh.len().min(self.capacity) as u32;
            self.cached = Some((scene.clone(), hovered.cloned()));
        }
        // `hide` clears only the count; the buffer still holds the mesh.
        self.count = self.length;
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, camera: &wgpu::BindGroup) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.draw(0..self.count, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrival::{HOME, Item, View};

    fn view() -> View {
        let space = |id: &str, participants| Item {
            key: Key::Space(id.into()),
            kind: Kind::Space {
                activity: Activity::Running,
                participants,
                hue: crate::arrival::hue(id),
            },
            label: id.into(),
            detail: Some("running".into()),
            selected: false,
        };
        View {
            heading: "Spaces".into(),
            items: vec![
                space("one", 2),
                space("two", 0),
                Item {
                    key: Key::Create,
                    kind: Kind::Create { open: false },
                    label: "New space".into(),
                    detail: None,
                    selected: true,
                },
            ],
            ..Default::default()
        }
    }

    fn frame(scene: &Scene, hovered: Option<&Key>) -> Vec<Cube> {
        cubes(&Frame {
            scene,
            hovered,
            ray: None,
            time: 0.0,
        })
    }

    #[test]
    fn every_object_has_a_form_near_it() {
        let scene = Scene::layout(&view());
        let cubes = frame(&scene, None);
        for object in &scene.objects {
            assert!(cubes.iter().any(|cube| {
                Vec3::from_array(cube.position).distance(object.position) < object.size * 3.0
            }));
        }
        assert!(cubes.iter().all(Cube::valid));
    }

    #[test]
    fn participants_show_as_cubes_in_their_own_colour() {
        let scene = Scene::layout(&view());
        let cubes = frame(&scene, None);
        assert_eq!(
            cubes
                .iter()
                .filter(|cube| cube.color == PARTICIPANT)
                .count(),
            2
        );
    }

    #[test]
    fn pointing_at_an_object_highlights_it_and_shows_its_full_text() {
        let scene = Scene::layout(&view());
        let key = Key::Space("one".into());
        let plain = frame(&scene, None);
        let hovered = frame(&scene, Some(&key));
        assert_eq!(
            hovered.iter().filter(|cube| cube.color == HOVER).count(),
            16
        );
        assert!(hovered.len() > plain.len());
        let lines = heading(&scene, Some(&key)).unwrap().lines;
        assert_eq!(lines.last().unwrap(), "one: running");
        assert_ne!(label_mesh(&scene, Some(&key)), label_mesh(&scene, None));
    }

    #[test]
    fn the_ray_reaches_from_the_controller_to_its_end() {
        let scene = Scene::default();
        let origin = Vec3::new(0.2, 1.2, -0.3);
        let end = Vec3::new(0.0, 1.3, -1.6);
        let cubes = cubes(&Frame {
            scene: &scene,
            hovered: None,
            ray: Some((origin, end)),
            time: 0.0,
        });
        assert_eq!(cubes.len(), 2);
        let beam = &cubes[0];
        assert!(Vec3::from_array(beam.position).distance((origin + end) * 0.5) < 1e-5);
        assert!((beam.scale[2] - origin.distance(end)).abs() < 1e-5);
        assert!(Vec3::from_array(cubes[1].position).distance(end) < 1e-5);
    }

    #[test]
    fn labels_face_home_and_stay_at_their_anchors() {
        let scene = Scene::layout(&view());
        let mesh = label_mesh(&scene, None);
        assert!(!mesh.is_empty());
        // Every vertex lies within a label's extent of some anchor, whichever way the head turns.
        for vertex in &mesh {
            let position = Vec3::from_array(vertex.position);
            assert!(
                scene
                    .objects
                    .iter()
                    .map(|object| object.label.anchor)
                    .chain(scene.heading.iter().map(|label| label.anchor))
                    .any(|anchor| position.distance(anchor) < 0.35),
                "{position}"
            );
        }
        // A label straight ahead is upright in the plane facing home.
        let object = scene.object(&Key::Create).unwrap();
        let normal = facing(object.label.anchor);
        let mut one = Scene::default();
        one.objects.push(object.clone());
        let mesh = label_mesh(&one, None);
        let depth =
            |vertex: &Vertex| (Vec3::from_array(vertex.position) - object.label.anchor).dot(normal);
        assert!(mesh.iter().all(|vertex| depth(vertex).abs() < 0.003));
        assert!(object.label.anchor.distance(HOME) > 0.5);
    }

    #[test]
    fn a_space_looks_the_same_wherever_it_is_drawn() {
        let mut a = Scene::layout(&view());
        let b = a.clone();
        a.objects.swap(0, 1);
        let colour = |scene: &Scene, id: &str| {
            let object = scene.object(&Key::Space(id.into())).unwrap().clone();
            let mut cubes = Vec::new();
            form(&object, 0.0, &mut cubes);
            cubes[0].color
        };
        assert_eq!(colour(&a, "one"), colour(&b, "one"));
        assert_ne!(colour(&b, "one"), colour(&b, "two"));
    }
}
