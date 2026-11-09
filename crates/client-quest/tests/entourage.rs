// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent's entourage in the farmyard, drawn on lavapipe: its workers on their arc and its Tomori wall, at home and
//! summoned, the wall folding, travelling and unfolding before whoever summoned it, the workers gliding after it.
//! `TOMORI_ENTOURAGE_PREVIEW`, set to a directory, saves the renders there to look at: `travel.png`, the wall at home,
//! mid-fold, mid-journey and unfolded at its travel spot; and, for each arrangement compared, `arc-*.png`, three, six
//! and ten workers, down the sheet, at home and summoned, across it. See `design/projection.md` and
//! `design/environments.md`.
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::{Mat4, Vec3};
use tomori_client::{
    ConnectionState, Event,
    feedback::{Feedback, Settings},
    server_choice::{Choice, Source},
};
use tomori_environment_farmyard_client::layout;
use tomori_protocol::{
    Anchors, ConnectionRole, EnvironmentInfo, Panel, Participant, Place, Pose, Projection,
    ServerMessage, SpaceState, TOMORI_WALL, Wall, WallKind, WallShape, Worker, WorkerState,
};
use tomori_render::{
    graphics::Renderer,
    scene_graphics::{Panels, draw},
    scenery::Sceneries,
    test_gpu,
};
use tomori_scene::{
    anchors::{Anchor, HomeArc, Placement, Placing, WorkerPlacement},
    controller,
    conversation::Conversation,
    environment::Environment,
    scene::{Frame, Senses, Step},
};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
const SPACE: &str = "space-1";
const FRAME: f32 = 1.0 / 72.0;

/// What workers are called, as an agent names them, and how they are getting on.
const WORKERS: [(&str, WorkerState); 10] = [
    ("Investigate the failing render test", WorkerState::Running),
    ("Draft the release notes", WorkerState::Running),
    ("Review the wall snapping change", WorkerState::Waiting),
    (
        "Profile the frame step on the headset",
        WorkerState::Running,
    ),
    (
        "Check the protocol for older clients",
        WorkerState::Completed,
    ),
    ("Summarise the open issues", WorkerState::Running),
    ("Update the farmyard design", WorkerState::Error),
    ("Benchmark speech synthesis", WorkerState::Running),
    ("Tidy the hand menu", WorkerState::Waiting),
    ("Write the travel tests", WorkerState::Running),
];

fn farmyard() -> Environment {
    Environment::new(tomori_environment_farmyard_client::ID)
        .with_version(tomori_environment_farmyard_client::VERSION)
}

/// Where participants arrive in the farmyard, in its own coordinates, and the carriage from them into a space's.
fn carriage() -> Pose {
    Pose::carriage(
        &Pose::at(layout::ARRIVAL.to_array()).facing_towards(layout::ARRIVAL_FACING.to_array()),
    )
}

/// `at`, in the yard's own coordinates, facing the eyes where participants arrive, carried into the space, as the server
/// carries the farmyard's anchors.
fn carried(at: Vec3) -> Pose {
    let eyes = layout::ARRIVAL + Vec3::Y * 1.6;
    carriage().carry(&Pose::at(at.to_array()).facing_towards(eyes.to_array()))
}

/// The farmyard's walls, as the server sends them: the Tomori wall at `tomori` and the noticeboard.
fn walls(tomori: Pose) -> Vec<Wall> {
    vec![
        Wall::new(TOMORI_WALL, WallKind::Tomori, tomori).with_shape(WallShape::Arc { radius: 2.3 }),
        Wall::new(
            "environment-0",
            WallKind::Environment,
            carried(layout::NOTICEBOARD),
        ),
    ]
}

fn panels() -> Vec<Panel> {
    let place = |slot: u32| Place::Wall {
        wall: TOMORI_WALL.into(),
        slot,
    };
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
            "The wall travels with the agent.",
            place(2),
        ),
        Panel::new("panel-4", "Next", "Try summoning on the headset.", place(4)),
    ]
}

/// The space's state, with `workers` of them, and the agent summoned to `summoned`, by someone whose eyes are at
/// `summoner`.
fn state(workers: usize, summoned: Option<(Pose, [f32; 3])>) -> SpaceState {
    let environment = farmyard();
    SpaceState {
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
        workers: WORKERS[..workers]
            .iter()
            .enumerate()
            .map(|(index, (title, state))| Worker {
                id: format!("ses_{index}"),
                parent: None,
                title: (*title).into(),
                state: *state,
            })
            .collect(),
        projection: true,
        anchors: Anchors::new()
            .with_agent(carried(layout::AGENT_HOME))
            .with_arrival(
                Pose::at(layout::ARRIVAL.to_array())
                    .facing_towards(layout::ARRIVAL_FACING.to_array()),
            ),
        summoned: summoned.map(|(at, _)| at),
        summoner: summoned.map(|(_, eyes)| eyes),
        summons: true,
        ..Default::default()
    }
}

/// Joined to a space in the farmyard with `workers` of them, arranged by `placement`, its panels on its Tomori wall.
fn joined(workers: usize, placement: Option<Placement>) -> Conversation {
    let mut conversation = Conversation::new("Ada");
    conversation.set_drawable(vec![farmyard()]);
    if let Some(placement) = placement {
        conversation.set_worker_placements(vec![(farmyard(), placement)]);
    }
    conversation.connect(Choice::new("Studio", "10.0.0.1:8765", Source::Discovered));
    conversation.apply(Event::State(ConnectionState::Connected {
        source: 1,
        server: "studio".into(),
        role: ConnectionRole::Owner,
        certificate: None,
        backends: vec![],
        roots: vec![],
        environments: vec![EnvironmentInfo {
            id: farmyard().id().into(),
            label: "Farmyard".into(),
            version: farmyard().version(),
        }],
        utterance_limit: None,
        name: None,
    }));
    conversation.apply(Event::Message(ServerMessage::Space {
        state: Box::new(state(workers, None)),
    }));
    conversation.apply(Event::Message(ServerMessage::Projection {
        projection: Box::new(
            Projection::new(SPACE, farmyard().id())
                .with_walls(walls(carried(layout::TOMORI_WALL)))
                .with_panels(panels()),
        ),
    }));
    conversation.take_actions();
    conversation
}

/// Where the eyes are of Ada, who summons the agent: at home, turned 70° to the right, towards the open yard.
const SUMMONER: [f32; 3] = [0.0, 1.6, 0.0];

/// The way Ada looks to summon the agent, along the ground.
fn summoning() -> Vec3 {
    let yaw = -70f32.to_radians();
    Vec3::new(-yaw.sin(), 0.0, -yaw.cos())
}

/// Where the agent stands, summoned 1.6 m before her, a little below her eyes, facing her; and where the server stands
/// its Tomori wall meanwhile, in the open yard: 2.3 m before her eyes, towards the agent, 1.08 m below them, facing her,
/// as `tomori-server`'s `stage::travel` stands it, which its own tests check.
fn summons() -> (Pose, Pose) {
    let eyes = Vec3::from_array(SUMMONER);
    let agent = eyes + summoning() * 1.6 - Vec3::Y * 0.15;
    let wall = eyes + summoning() * 2.3 - Vec3::Y * 1.08;
    (
        Pose::at(agent.to_array()).facing_towards(SUMMONER),
        Pose::at(wall.to_array()).facing_towards([eyes.x, wall.y, eyes.z]),
    )
}

/// Summon the agent, for everyone, as the server tells of it.
fn summon(conversation: &mut Conversation, workers: usize) {
    let (agent, wall) = summons();
    conversation.apply(Event::Message(ServerMessage::Space {
        state: Box::new(state(workers, Some((agent, SUMMONER)))),
    }));
    conversation.apply(Event::Message(ServerMessage::Projection {
        projection: Box::new(
            Projection::new(SPACE, farmyard().id())
                .with_walls(walls(wall))
                .with_panels(panels()),
        ),
    }));
    conversation.take_actions();
}

/// Step the scene for `seconds` from `now`, as `senses` say, and return the last frame.
fn run<'a>(
    step: &'a mut Step,
    conversation: &mut Conversation,
    feedback: &mut Feedback,
    senses: &Senses,
    seconds: f32,
    now: &mut Instant,
) -> Frame<'a> {
    let frames = ((seconds / FRAME).round() as usize).max(1);
    for _ in 1..frames {
        *now += Duration::from_secs_f32(FRAME);
        step.update(senses, conversation, feedback, *now, FRAME);
    }
    *now += Duration::from_secs_f32(FRAME);
    step.update(senses, conversation, feedback, *now, FRAME)
}

/// The senses of someone standing at `eyes`, looking along `towards`.
fn senses(eyes: Vec3, towards: Vec3) -> Senses {
    let head = Mat4::look_to_rh(eyes, towards, Vec3::Y).inverse();
    Senses {
        focused: true,
        head: Some(head),
        head_pose: controller::Pose::from_transform(head),
        viewer: Some(eyes),
        space_head: head,
        trigger: Some(0.0),
        ..Default::default()
    }
}

/// The restored arc as it stood before environments placed the agent: its rows 0.55 m apart, about whoever the agent
/// faces, for comparison with the default, whose rows stand 0.8 m apart.
struct Restored;

impl WorkerPlacement for Restored {
    fn place(&self, origin: Anchor, worker: &Placing<'_>) -> Vec3 {
        const BEFORE: [f32; 2] = [1.35, 1.90];
        let row = (worker.slot % 8) / 4;
        let at = HomeArc::at(origin, worker.audience, worker.slot);
        let now = tomori_scene::delegation::slot_position(worker.slot).y;
        if worker.slot < 32 {
            at + Vec3::Y * (BEFORE[row] - now)
        } else {
            at
        }
    }
}

/// One row: every worker at the same height, a little below the eyes, alternately left and right of the agent, a
/// column apart.
struct Row;

impl WorkerPlacement for Row {
    fn place(&self, origin: Anchor, worker: &Placing<'_>) -> Vec3 {
        // The arc's own columns, but every slot in the first row: slot 2 takes column 1, as slot 4 would.
        let column_slot =
            (worker.slot / 2) * 4 % 32 / 8 * 8 + (worker.slot / 2) % 2 * 2 + worker.slot % 2;
        let mut at = HomeArc::at(origin, worker.audience, column_slot);
        at.y = worker.audience.y - 0.3;
        at
    }
}

#[test]
fn the_tomori_wall_and_the_workers_follow_the_summoned_agent_in_the_farmyard() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("entourage test eye"),
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
        tomori_environment_farmyard_client::register(&mut sceneries);
        let capture = |frame: &Frame, eyes: Vec3, towards: Vec3| {
            let mut renderer =
                Renderer::with_sceneries(&device, &queue, format, WIDTH, HEIGHT, 1, &sceneries);
            let mut panels = Panels::new(&device, &renderer);
            draw(&mut renderer, &device, &queue, &mut panels, frame);
            // Wide, to see the arc either side of the agent.
            let matrix = Mat4::perspective_rh(1.3, WIDTH as f32 / HEIGHT as f32, 0.05, 80.0)
                * Mat4::look_to_rh(eyes, towards - Vec3::Y * 0.12, Vec3::Y);
            renderer.render(
                &device,
                &queue,
                std::slice::from_ref(&view),
                &[matrix],
                Mat4::IDENTITY,
                &panels.list(Mat4::IDENTITY),
            );
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("entourage readback"),
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
        let preview = std::env::var_os("TOMORI_ENTOURAGE_PREVIEW").map(std::path::PathBuf::from);
        let eyes = Vec3::from_array(SUMMONER);
        let home = senses(eyes, Vec3::NEG_Z);
        let turned = senses(eyes, summoning());
        // Looking at the agent at home, and where Ada turns to summon it.
        let agent_home = Vec3::from_array(carried(layout::AGENT_HOME).position);
        let towards_home = Vec3::new(agent_home.x, 0.0, agent_home.z).normalize();

        // The wall folds where it stood, travels and unfolds before her, its panels with it.
        let mut conversation = joined(6, None);
        let mut step = Step::default();
        let mut feedback = Feedback::new(Settings::default());
        let start = Instant::now();
        let mut now = start;
        // Seen between its home and where it goes, so that its whole journey is in view.
        let between = (summoning()
            + Vec3::from_array(carried(layout::TOMORI_WALL).position)
                .with_y(0.0)
                .normalize())
        .normalize();
        let frame = run(
            &mut step,
            &mut conversation,
            &mut feedback,
            &turned,
            8.0,
            &mut now,
        );
        let before = capture(&frame, eyes, between);
        summon(&mut conversation, 6);
        let frame = run(
            &mut step,
            &mut conversation,
            &mut feedback,
            &turned,
            0.15,
            &mut now,
        );
        let folding = capture(&frame, eyes, between);
        let frame = run(
            &mut step,
            &mut conversation,
            &mut feedback,
            &turned,
            0.6,
            &mut now,
        );
        let travelling = capture(&frame, eyes, between);
        let frame = run(
            &mut step,
            &mut conversation,
            &mut feedback,
            &turned,
            6.0,
            &mut now,
        );
        let arrived = capture(&frame, eyes, summoning());
        // Each step of its journey looks different; arrived, its panels stand before her.
        assert_ne!(before, folding);
        assert_ne!(folding, travelling);
        assert_ne!(travelling, arrived);
        assert!(frame.projection.len() > 6, "its slots and panels are drawn");
        // The workers stand about her and the agent: each on the arc about her eyes, within 60° of the agent's bearing
        // for the first eight, none above 2.6 m nor below 1 m.
        let agent = Vec3::from_array(summons().0.position);
        let bearing = |at: Vec3| (at.x - eyes.x).atan2(-(at.z - eyes.z));
        for station in frame.stations {
            let at = Vec3::from_array(station.position);
            assert!(
                (Vec3::new(at.x - eyes.x, 0.0, at.z - eyes.z).length() - 3.2).abs() < 0.05,
                "{at}"
            );
            let off = (bearing(at) - bearing(agent)).abs();
            assert!(off < 61f32.to_radians(), "{}", off.to_degrees());
            assert!((1.0..=2.6).contains(&at.y), "{at}");
            assert_eq!(station.facing, SUMMONER);
        }
        if let Some(directory) = &preview {
            let mut sheet = image::RgbaImage::new(WIDTH * 2, HEIGHT * 2);
            for (index, drawn) in [&before, &folding, &travelling, &arrived]
                .into_iter()
                .enumerate()
            {
                let (x, y) = (index as u32 % 2 * WIDTH, index as u32 / 2 * HEIGHT);
                image::imageops::replace(&mut sheet, drawn, x.into(), y.into());
            }
            std::fs::create_dir_all(directory).unwrap();
            sheet
                .save(directory.join("travel.png"))
                .expect("the preview is saved");
        }
        // Sent home, they go home with it.
        conversation.apply(Event::Message(ServerMessage::Space {
            state: Box::new(state(6, None)),
        }));
        conversation.apply(Event::Message(ServerMessage::Projection {
            projection: Box::new(
                Projection::new(SPACE, farmyard().id())
                    .with_walls(walls(carried(layout::TOMORI_WALL)))
                    .with_panels(panels()),
            ),
        }));
        let frame = run(
            &mut step,
            &mut conversation,
            &mut feedback,
            &home,
            8.0,
            &mut now,
        );
        for station in frame.stations {
            assert_eq!(station.facing, [0.0, 1.6, 0.0]);
        }

        // The arrangements compared: the default, the arc as it stood before, and one row.
        let arrangements: [(&str, Option<Placement>); 3] = [
            ("arc-default", None),
            ("arc-restored", Some(Arc::new(Restored))),
            ("arc-row", Some(Arc::new(Row))),
        ];
        for (name, placement) in arrangements {
            let mut sheet = image::RgbaImage::new(WIDTH * 2, HEIGHT * 3);
            for (row, workers) in [3, 6, 10].into_iter().enumerate() {
                let mut conversation = joined(workers, placement.clone());
                let mut now = start;
                let mut step = Step::default();
                let frame = run(
                    &mut step,
                    &mut conversation,
                    &mut feedback,
                    &home,
                    8.0,
                    &mut now,
                );
                assert_eq!(frame.stations.len(), workers);
                let at_home = capture(&frame, eyes, towards_home);
                summon(&mut conversation, workers);
                let frame = run(
                    &mut step,
                    &mut conversation,
                    &mut feedback,
                    &turned,
                    8.0,
                    &mut now,
                );
                let summoned = capture(&frame, eyes, summoning());
                image::imageops::replace(&mut sheet, &at_home, 0, (row as u32 * HEIGHT).into());
                image::imageops::replace(
                    &mut sheet,
                    &summoned,
                    WIDTH.into(),
                    (row as u32 * HEIGHT).into(),
                );
            }
            if let Some(directory) = &preview {
                sheet
                    .save(directory.join(format!("{name}.png")))
                    .expect("the preview is saved");
            }
        }
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}
