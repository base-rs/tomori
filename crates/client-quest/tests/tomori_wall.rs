// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The Tomori wall where the server stands it by default, curving below the swarm, drawn in each of Tomori's own
//! environments, the grid, Sky and the farmyard, with workers at their stations, as Tomori's own app registers them.
//! `TOMORI_DEFAULT_WALL_PREVIEW`, set to a PNG path, saves the renders, one above another, to look at. See
//! `design/projection.md`.
use std::time::Instant;

use glam::{Mat4, Vec3};
use tomori_client::{
    ConnectionState, Event,
    feedback::{Feedback, Settings},
    server_choice::{Choice, Source},
};
use tomori_protocol::{
    ConnectionRole, EnvironmentInfo, Panel, Participant, Place, Pose, Projection, ServerMessage,
    SpaceState, TOMORI_WALL, Wall, WallKind, WallShape, Worker, WorkerState,
};
use tomori_render::{
    graphics::Renderer,
    scene_graphics::{Panels, draw},
    scenery::Sceneries,
    test_gpu,
};
use tomori_scene::{
    arrival, controller,
    conversation::Conversation,
    environment::Environment,
    scene::{Senses, Step},
};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
const SPACE: &str = "space-1";

/// The Tomori wall where the server stands it in an environment that declares no `stage` anchor.
fn tomori_wall() -> Wall {
    Wall::new(
        TOMORI_WALL,
        WallKind::Tomori,
        Pose::upright_facing([0.0, 0.52, -2.3], [0.0, 0.52, 0.0]),
    )
    .with_shape(WallShape::Arc { radius: 2.3 })
}

/// Joined to a space in `environment`, with eight workers at their stations and, `projected`, four panels on the Tomori
/// wall.
fn joined(environment: Environment, projected: bool) -> Conversation {
    let mut conversation = Conversation::new("Ada");
    conversation.set_drawable(vec![
        Environment::new(tomori_environment_sky_client::ID)
            .with_version(tomori_environment_sky_client::VERSION),
        Environment::new(tomori_environment_farmyard_client::ID)
            .with_version(tomori_environment_farmyard_client::VERSION),
    ]);
    conversation.connect(Choice::new("Studio", "10.0.0.1:8765", Source::Discovered));
    conversation.apply(Event::State(ConnectionState::Connected {
        source: 1,
        server: "studio".into(),
        role: ConnectionRole::Owner,
        certificate: None,
        backends: vec![],
        roots: vec![],
        environments: vec![EnvironmentInfo {
            id: environment.id().into(),
            label: environment.id().into(),
            version: environment.version(),
        }],
        utterance_limit: None,
        name: None,
    }));
    conversation.apply(Event::Message(ServerMessage::Space {
        state: Box::new(SpaceState {
            id: SPACE.into(),
            title: "Tomori".into(),
            backend: "opencode".into(),
            directory: "/work/tomori".into(),
            environment: environment.id().into(),
            environment_version: Some(environment.version()),
            participants: vec![Participant {
                name: "Ada".into(),
                source: 1,
                engagement: None,
            }],
            workers: (0..8)
                .map(|index| Worker {
                    id: format!("ses_{index}"),
                    parent: None,
                    title: format!("Worker {index}"),
                    state: WorkerState::Running,
                })
                .collect(),
            projection: true,
            ..Default::default()
        }),
    }));
    let place = |slot: u32| Place::Wall {
        wall: TOMORI_WALL.into(),
        slot,
    };
    let panels = if projected {
        vec![
            Panel::new(
                "panel-1",
                "Build status",
                "**3 crates** failed:\n\n- `tomori-scene`\n- `tomori-render`",
                place(0),
            ),
            Panel::new(
                "panel-2",
                "Steps to release",
                "1. Bump the version.\n2. Run the tests.\n3. Tag and push.",
                place(1),
            ),
            Panel::new(
                "panel-3",
                "Summary",
                "Projection walls hold **text panels** the agent writes.",
                place(2),
            ),
            Panel::new(
                "panel-4",
                "Next",
                "Try the curved wall on the headset.",
                place(4),
            ),
        ]
    } else {
        Vec::new()
    };
    conversation.apply(Event::Message(ServerMessage::Projection {
        projection: Box::new(
            Projection::new(SPACE, environment.id())
                .with_walls(vec![tomori_wall()])
                .with_panels(panels),
        ),
    }));
    conversation.take_actions();
    conversation
}

#[test]
fn the_default_tomori_wall_stands_below_the_swarm_in_every_environment() {
    let environments = [
        Environment::MINIMAL,
        Environment::new(tomori_environment_sky_client::ID)
            .with_version(tomori_environment_sky_client::VERSION),
        Environment::new(tomori_environment_farmyard_client::ID)
            .with_version(tomori_environment_farmyard_client::VERSION),
    ];
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("default wall test eye"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let mut sceneries = Sceneries::empty();
        tomori_environment_sky_client::register(&mut sceneries);
        tomori_environment_farmyard_client::register(&mut sceneries);
        let capture = |renderer: &Renderer, panels: &Panels| {
            // From home, looking ahead a little down, wide enough to see the first workers' stations either side.
            let matrix = Mat4::perspective_rh(1.2, WIDTH as f32 / HEIGHT as f32, 0.05, 80.0)
                * Mat4::look_at_rh(arrival::HOME, Vec3::new(0.0, 1.15, -2.3), Vec3::Y);
            renderer.render(
                &device,
                &queue,
                std::slice::from_ref(&view),
                &[matrix],
                Mat4::IDENTITY,
                &panels.list(Mat4::IDENTITY),
            );
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("default wall readback"),
                size: (WIDTH * HEIGHT * 4) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(WIDTH * 4),
                        rows_per_image: Some(HEIGHT),
                    },
                },
                texture.size(),
            );
            queue.submit([encoder.finish()]);
            let (sender, receiver) = std::sync::mpsc::channel();
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = sender.send(result);
                });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            receiver.recv().unwrap().unwrap();
            let data = buffer.slice(..).get_mapped_range().unwrap().to_vec();
            buffer.unmap();
            image::RgbaImage::from_raw(WIDTH, HEIGHT, data).unwrap()
        };
        let head = Mat4::from_translation(arrival::HOME);
        let senses = Senses {
            focused: true,
            head: Some(head),
            head_pose: controller::Pose::from_transform(head),
            viewer: Some(arrival::HOME),
            space_head: head,
            trigger: Some(0.0),
            ..Default::default()
        };
        let mut sheet = image::RgbaImage::new(WIDTH, HEIGHT * environments.len() as u32);
        for (index, environment) in environments.into_iter().enumerate() {
            let mut drawn = Vec::new();
            for projected in [false, true] {
                let mut renderer =
                    Renderer::with_sceneries(&device, &queue, format, WIDTH, HEIGHT, 1, &sceneries);
                let mut panels = Panels::new(&device, &renderer);
                let mut conversation = joined(environment, projected);
                let mut step = Step::default();
                let mut feedback = Feedback::new(Settings::default());
                let now = Instant::now();
                // The environment grows in, and the workers fly to their stations.
                for _ in 0..600 {
                    step.update(&senses, &mut conversation, &mut feedback, now, 1.0 / 72.0);
                }
                let frame = step.update(&senses, &mut conversation, &mut feedback, now, 1.0 / 72.0);
                assert_eq!(frame.scenery.environment, environment);
                assert_eq!(frame.stations.len(), 8);
                draw(&mut renderer, &device, &queue, &mut panels, &frame);
                drawn.push(capture(&renderer, &panels));
            }
            assert_ne!(drawn[0], drawn[1], "{environment:?}: the panels are drawn");
            image::imageops::replace(&mut sheet, &drawn[1], 0, (index as u32 * HEIGHT).into());
        }
        if let Some(path) = std::env::var_os("TOMORI_DEFAULT_WALL_PREVIEW") {
            sheet.save(path).expect("the preview is saved");
        }
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}
