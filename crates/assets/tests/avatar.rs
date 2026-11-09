// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! An avatar for the agent, built as another project builds one, from the published crates and nothing else: a figure
//! built in code as a model of this crate's, its head turned towards the participant and lifted as the agent speaks by
//! a trivial controller of its own, registered with a client's sceneries beside a scenery that draws it with this
//! crate's renderer. A space whose backend names it shows it at the agent anchor in place of the swarm, without
//! changing Tomori. See Tomori's `design/avatars.md`.
#![cfg(feature = "gpu")]
use std::sync::{Arc, Mutex};
use std::time::Instant;

use glam::{Mat4, Quat, Vec3, Vec4};
use tomori_assets::{
    Instance, Limits, Material, Mesh, Model, Node, Pose, Primitive, Transform,
    gpu::{Batch, GpuModel, Lighting, Renderer},
};
use tomori_render::scenery::{Sceneries, Scenery, Target};
use tomori_scene::{
    anchors::Anchor,
    arrival_motion::Sphere,
    avatar::{Avatar, Input},
    client_side::Drawn,
    interaction::Mode,
    scene::{Senses, Step, fixtures},
};

/// The identifier its backend names it by.
const FIGURE: &str = "figure";

/// Where its head stands above its feet, at rest.
const HEAD: Vec3 = Vec3::new(0.0, 1.5, 0.0);

/// A box from `low` to `high`, in one colour.
fn block(low: Vec3, high: Vec3, colour: Vec4) -> Primitive {
    let corners: Vec<Vec3> = (0..8)
        .map(|corner| {
            Vec3::new(
                if corner & 1 == 0 { low.x } else { high.x },
                if corner & 2 == 0 { low.y } else { high.y },
                if corner & 4 == 0 { low.z } else { high.z },
            )
        })
        .collect();
    let indices = vec![
        0, 2, 1, 1, 2, 3, 4, 5, 6, 5, 7, 6, 0, 1, 4, 1, 5, 4, 2, 6, 3, 3, 6, 7, 0, 4, 2, 2, 4, 6,
        1, 3, 5, 3, 7, 5,
    ];
    Primitive::new(corners, Vec::new(), indices).with_colors(vec![colour; 8])
}

/// The figure: a body, and a head on it, which its controller moves.
fn figure() -> Arc<Model> {
    let body = block(
        Vec3::new(-0.25, 0.0, -0.15),
        Vec3::new(0.25, 1.3, 0.15),
        Vec4::new(0.9, 0.4, 0.1, 1.0),
    );
    let head = block(
        Vec3::splat(-0.15),
        Vec3::splat(0.15),
        Vec4::new(1.0, 0.85, 0.6, 1.0),
    );
    Arc::new(
        Model::built(
            vec![
                Node::new("body", Transform::IDENTITY)
                    .with_mesh(0)
                    .with_children(vec![1]),
                Node::new("head", Transform::new(HEAD, Quat::IDENTITY, Vec3::ONE)).with_mesh(1),
            ],
            vec![0],
            vec![Mesh::new("body", vec![body]), Mesh::new("head", vec![head])],
            vec![Material::new("plain", Vec4::ONE)],
            &Limits::default(),
        )
        .expect("the figure is a valid model"),
    )
}

/// The figure for one space: an instance of its model, which its controller places and poses each frame.
struct Figure {
    instance: Instance,
    head: usize,
    anchor: Anchor,
}

impl Figure {
    fn new() -> Self {
        let model = figure();
        Self {
            head: model.node("head").expect("it has a head"),
            instance: Instance::new(model),
            anchor: Anchor::AGENT,
        }
    }

    /// Where its head is in the scene.
    fn head(&self) -> Vec3 {
        self.instance
            .transform
            .transform_point3(self.instance.pose().local(self.head).translation)
    }
}

impl Avatar for Figure {
    /// Its controller: it stands at the anchor, facing as it faces, turns its head towards the participant, and lifts
    /// it with the agent's voice while it speaks.
    fn update(&mut self, input: &Input) {
        self.anchor = input.anchor;
        self.instance.transform =
            Mat4::from_rotation_translation(input.anchor.orientation, input.anchor.position);
        let towards = input
            .head
            .map(|head| input.anchor.orientation.inverse() * (head - input.anchor.position))
            .filter(|towards| towards.x != 0.0 || towards.z != 0.0);
        let turn = towards.map_or(Quat::IDENTITY, |towards| {
            Quat::from_rotation_y(towards.x.atan2(towards.z))
        });
        let lift = if input.visual.mode == Mode::Speaking {
            input.visual.level * 0.2
        } else {
            0.0
        };
        let mut pose = Pose::rest(self.instance.model());
        pose.set_local(
            self.head,
            Transform::new(HEAD + Vec3::Y * lift, turn, Vec3::ONE),
        );
        self.instance.set_pose(pose);
    }

    /// From its head.
    fn voices(&self, voices: &mut Vec<[f32; 3]>) {
        voices.push(self.head().to_array());
    }

    /// From its chest.
    fn departures(&self, departures: &mut Vec<[f32; 3]>) {
        departures.push(self.anchor.place(Vec3::new(0.0, 1.0, 0.3)).to_array());
    }

    fn bounds(&self) -> Sphere {
        Sphere {
            centre: self.anchor.place(Vec3::Y * 0.9),
            radius: 1.0,
        }
    }

    /// The instance, placed and posed, for its scenery.
    fn drawn(&self) -> Option<Drawn> {
        Some(Arc::new(self.instance.clone()))
    }
}

/// What the figure's scenery did, for the test to read.
#[derive(Clone, Default)]
struct Seen {
    placed: Arc<Mutex<Vec<Instance>>>,
    eyes: Arc<Mutex<usize>>,
}

/// The figure's scenery: its model uploaded once, drawn where its avatar placed it.
struct FigureScenery {
    renderer: Renderer,
    model: GpuModel,
    batch: Batch,
    seen: Seen,
}

impl FigureScenery {
    fn new(target: &Target, seen: Seen) -> Self {
        let renderer = Renderer::new(target.device, target.queue, target.format, target.samples);
        renderer.set_lighting(target.queue, &Lighting::default());
        Self {
            model: renderer.upload(target.device, target.queue, &figure()),
            batch: renderer.batch(target.device),
            renderer,
            seen,
        }
    }
}

impl Scenery for FigureScenery {
    fn animate(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        drawn: &(dyn std::any::Any + Send + Sync),
    ) {
        let instance = drawn
            .downcast_ref::<Instance>()
            .expect("its avatar shows its instance");
        self.renderer.place(
            device,
            queue,
            &self.model,
            &mut self.batch,
            std::slice::from_ref(instance),
        );
        self.seen.placed.lock().unwrap().push(instance.clone());
    }

    fn prepare(&self, queue: &wgpu::Queue, eye: usize, scene_to_clip: Mat4) {
        self.renderer.prepare(queue, eye, scene_to_clip, Vec3::ZERO);
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, eye: usize) {
        self.renderer.draw(pass, eye, &self.model, &self.batch);
        *self.seen.eyes.lock().unwrap() += 1;
    }

    fn set_reveal(&mut self, reveal: Option<(Vec3, f32)>) {
        self.renderer.set_reveal(reveal);
    }
}

/// Register the figure on `sceneries`, as a downstream client crate's `register` would.
fn register(sceneries: &mut Sceneries, seen: Seen) {
    sceneries.set_avatar(
        FIGURE,
        |_space| Box::new(Figure::new()),
        move |target| Box::new(FigureScenery::new(target, seen.clone())),
    );
}

#[test]
fn a_downstream_avatar_built_on_these_models_stands_in_for_the_swarm() {
    let seen = Seen::default();
    let mut sceneries = Sceneries::empty();
    register(&mut sceneries, seen.clone());
    let agent = tomori_protocol::Pose {
        position: [1.0, 0.0, -3.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    };
    let mut conversation = fixtures::lobby(tomori_protocol::ConnectionRole::Owner);
    conversation.set_drawable(sceneries.environments());
    conversation.set_avatars(sceneries.avatars());
    conversation.apply(fixtures::space(|state| {
        state.avatar = Some(FIGURE.into());
        state.anchors = tomori_protocol::Anchors::new().with_agent(agent);
    }));
    let head = Mat4::from_translation(Vec3::new(1.0, 1.6, 0.0));
    let senses = Senses {
        focused: true,
        head: Some(head),
        head_pose: tomori_scene::controller::Pose::from_transform(head),
        viewer: Some(Vec3::new(1.0, 1.6, 0.0)),
        space_head: head,
        trigger: Some(0.0),
        ..Default::default()
    };
    let mut step = Step::default();
    tomori_render::test_gpu::with(|tomori_render::test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut renderer = tomori_render::graphics::Renderer::with_sceneries(
            &device, &queue, format, 32, 32, 1, &sceneries,
        );
        let mut panels = tomori_render::scene_graphics::Panels::new(&device, &renderer);
        let frame = step.update(
            &senses,
            &mut conversation,
            &mut fixtures::felt(),
            Instant::now(),
            1.0 / 72.0,
        );
        // The figure shows, and the swarm does not.
        assert_eq!(
            frame.avatar.as_ref().map(|shown| shown.avatar),
            Some(FIGURE)
        );
        assert!(frame.swarm.agents.is_empty());
        // The agent is heard from its head, standing at the anchor, turned to face the participant.
        assert_eq!(frame.agent, [1.0, 1.5, -3.0]);
        tomori_render::scene_graphics::draw(&mut renderer, &device, &queue, &mut panels, &frame);
        let view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("figure eye"),
                size: wgpu::Extent3d {
                    width: 32,
                    height: 32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let camera = Mat4::perspective_rh(1.0, 1.0, 0.1, 20.0)
            * Mat4::look_at_rh(Vec3::new(1.0, 1.6, 0.0), Vec3::new(1.0, 1.0, -3.0), Vec3::Y);
        renderer.render(
            &device,
            &queue,
            std::slice::from_ref(&view),
            &[camera],
            Mat4::IDENTITY,
            &[],
        );
        assert!(
            pollster::block_on(scope.pop()).is_none(),
            "drawn without validation errors"
        );
    });
    // Its scenery placed it at the anchor and drew it in the eye.
    let placed = seen.placed.lock().unwrap().last().cloned().expect("placed");
    assert_eq!(placed.draws().len(), 2);
    assert_eq!(
        placed.transform.transform_point3(Vec3::ZERO),
        Vec3::new(1.0, 0.0, -3.0)
    );
    assert_eq!(*seen.eyes.lock().unwrap(), 1);
    // Its head faces the participant, who stands straight ahead of it along its +Z, as the anchor faces.
    let turn = placed
        .pose()
        .local(placed.model().node("head").unwrap())
        .rotation;
    assert!(turn.angle_between(Quat::IDENTITY) < 1e-4, "{turn}");
}

#[test]
fn the_figure_lifts_its_head_as_the_agent_speaks_and_turns_it_to_whoever_it_faces() {
    let mut figure = Figure::new();
    let speaking = tomori_scene::interaction::Visual {
        mode: Mode::Speaking,
        level: 0.5,
        ..Default::default()
    };
    figure.update(
        &Input::new(speaking, Anchor::AGENT, 0.01).head(Some(Anchor::AGENT.place(Vec3::X * 2.0))),
    );
    let mut voices = Vec::new();
    figure.voices(&mut voices);
    assert_eq!(
        voices,
        [Anchor::AGENT.place(HEAD + Vec3::Y * 0.1).to_array()]
    );
    let turn = *figure.instance.pose().local(figure.head);
    assert!(
        (turn.rotation * Vec3::Z).distance(Vec3::X) < 1e-4,
        "{}",
        turn.rotation
    );
    let mut departures = Vec::new();
    figure.departures(&mut departures);
    assert_eq!(departures.len(), 1);
    assert_eq!(figure.bounds().radius, 1.0);
    // Quiet, and with nobody to face, it stands at rest.
    figure.update(&Input::new(Default::default(), Anchor::AGENT, 0.01));
    assert_eq!(
        *figure.instance.pose().local(figure.head),
        Transform::new(HEAD, Quat::IDENTITY, Vec3::ONE)
    );
}
