// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent's entourage in the farmyard, drawn on lavapipe: its workers on their arc and its Tomori wall beneath it, at
//! home and summoned, the wall folding, travelling and unfolding beneath the agent before whoever summoned it, the
//! workers gliding after it. `TOMORI_ENTOURAGE_PREVIEW`, set to a directory, saves the renders there to look at:
//! `travel.png`, the wall at home, mid-fold, mid-journey and unfolded at its travel spot; for each arrangement compared,
//! `arc-*.png`, three, six and ten workers, down the sheet, at home and summoned, across it; `signs.png`, the first four
//! workers' signs close up, summoned; `signs-viewpoints.png`, the default arc of six, summoned, seen from in front, 45°
//! to its left and 70° to its right, each sign turned to face whoever looks; `home.png`, the agent at home with its wall
//! beneath it, from the arrival; `summoned.png`, the agent summoned by someone seated, their eyes 1 m up, and by
//! someone standing, 1.6 m and 1.8 m, down the sheet; and `workbench.png`, the agent at its workbench in the yard's
//! corner, its workers in the ring about it, from the arrival turned towards it, from the yard before it, from within
//! the ring and from straight above, down the sheet. See `design/projection.md`, `design/environments.md`, `design/farmyard.md` and
//! `design/interaction.md`.
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
    entourage,
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

/// The Tomori wall's home in the farmyard, as the server stands it: beneath the agent's home.
fn home_wall() -> Pose {
    entourage::wall_beneath(&carried(layout::AGENT_HOME), entourage::FLOOR)
}

/// The Tomori wall at `pose`, as the server sends it: six slots curved along an arc of 2.3 m.
fn tomori_wall(pose: Pose) -> Wall {
    Wall::new(TOMORI_WALL, WallKind::Tomori, pose)
        .with_shape(WallShape::Arc { radius: 2.3 })
        .with_grid(tomori_protocol::wall::ARC_GRID)
        .with_scale(tomori_protocol::wall::ARC_SCALE)
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
                .with_walls(walls(home_wall()))
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

/// Where the agent stands, summoned by someone whose eyes are at `eyes`, drawn 1.6 m before them, low by their knees,
/// as `entourage::summoned` stands it, 2.3 m before them, a little above their eyes, facing them; and where the server
/// stands its Tomori wall meanwhile, in the open yard: beneath it, facing them, as `tomori-server`'s `stage::travel`
/// stands it, which its own tests check.
fn summons_by(eyes: [f32; 3]) -> (Pose, Pose) {
    let at = Vec3::from_array(eyes) + summoning() * 1.6 - Vec3::Y * 0.9;
    let agent = entourage::summoned(at.to_array(), eyes, entourage::FLOOR);
    let beneath = entourage::wall_beneath(&agent, entourage::FLOOR).position;
    (
        agent,
        Pose::upright_facing(beneath, [eyes[0], beneath[1], eyes[2]]),
    )
}

/// The agent and its wall, summoned by Ada, standing at home.
fn summons() -> (Pose, Pose) {
    summons_by(SUMMONER)
}

/// Summon the agent, for everyone, as the server tells of it.
fn summon(conversation: &mut Conversation, workers: usize) {
    summon_by(conversation, workers, SUMMONER);
}

/// Summon the agent by someone whose eyes are at `eyes`, for everyone, as the server tells of it.
fn summon_by(conversation: &mut Conversation, workers: usize, eyes: [f32; 3]) {
    let (agent, wall) = summons_by(eyes);
    conversation.apply(Event::Message(ServerMessage::Space {
        state: Box::new(state(workers, Some((agent, eyes)))),
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
        let capture_with = |frame: &Frame, eyes: Vec3, towards: Vec3, fov: f32| {
            let mut renderer =
                Renderer::with_sceneries(&device, &queue, format, WIDTH, HEIGHT, 1, &sceneries);
            let mut panels = Panels::new(&device, &renderer);
            draw(&mut renderer, &device, &queue, &mut panels, frame);
            // Wide, to see the arc either side of the agent.
            let matrix = Mat4::perspective_rh(fov, WIDTH as f32 / HEIGHT as f32, 0.05, 80.0)
                * Mat4::look_to_rh(eyes, towards, Vec3::Y);
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
        // Wide, to see the arc either side of the agent, looking a little down.
        let capture = |frame: &Frame, eyes: Vec3, towards: Vec3| {
            capture_with(frame, eyes, towards - Vec3::Y * 0.12, 1.3)
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
            + Vec3::from_array(home_wall().position)
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
        // The workers stand about her, on the arc through the agent, as far from her eyes as it is, to either side of
        // it, about its height, none above 2.6 m nor below 1 m.
        let agent = Vec3::from_array(summons().0.position);
        check_arc(&frame, agent, eyes, "summoned");
        for station in frame.stations {
            let at = Vec3::from_array(station.position);
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
                    .with_walls(walls(home_wall()))
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
                // Close up, as Ada turns her head to each of the first four workers' signs.
                if name == "arc-default" && workers == 6 {
                    let mut close = image::RgbaImage::new(WIDTH * 2, HEIGHT * 2);
                    let mut stations: Vec<_> = frame.stations.iter().collect();
                    stations.sort_by(|a, b| a.id.cmp(&b.id));
                    for (index, station) in stations.iter().take(4).enumerate() {
                        let at = Vec3::from_array(station.position) + Vec3::Y * 0.4;
                        let drawn = capture_with(&frame, eyes, (at - eyes).normalize(), 0.45);
                        let (x, y) = (index as u32 % 2 * WIDTH, index as u32 / 2 * HEIGHT);
                        image::imageops::replace(&mut close, &drawn, x.into(), y.into());
                    }
                    if let Some(directory) = &preview {
                        close
                            .save(directory.join("signs.png"))
                            .expect("the preview is saved");
                    }
                }
            }
            if let Some(directory) = &preview {
                sheet
                    .save(directory.join(format!("{name}.png")))
                    .expect("the preview is saved");
            }
        }
        // Walking about the default arc of six workers, summoned into the open yard: from in front of it, where Ada
        // stands, 45° to its left and 70° to its right about its middle. Each sign turns to
        // face her wherever she stands, upright, its rows centred above its worker. She stands 4.5 m from its middle,
        // to see the whole arc.
        let mut conversation = joined(6, None);
        let mut step = Step::default();
        let mut now = start;
        run(
            &mut step,
            &mut conversation,
            &mut feedback,
            &home,
            8.0,
            &mut now,
        );
        summon(&mut conversation, 6);
        let frame = run(
            &mut step,
            &mut conversation,
            &mut feedback,
            &turned,
            8.0,
            &mut now,
        );
        let middle = frame
            .stations
            .iter()
            .map(|station| Vec3::from_array(station.position))
            .sum::<Vec3>()
            / frame.stations.len() as f32;
        // Far enough back to see the whole arc from each side.
        let from = (eyes - middle).with_y(0.0).normalize() * 4.5;
        let mut sheet = image::RgbaImage::new(WIDTH * 3, HEIGHT);
        for (index, degrees) in [0.0_f32, 45.0, -70.0].into_iter().enumerate() {
            let at =
                middle.with_y(eyes.y) + glam::Quat::from_rotation_y(degrees.to_radians()) * from;
            let looking = (middle + Vec3::Y * 0.35 - at).normalize();
            let walked = senses(at, looking);
            let frame = run(
                &mut step,
                &mut conversation,
                &mut feedback,
                &walked,
                2.0,
                &mut now,
            );
            let facing = frame.sign_facing.expect("signs face whoever looks at them");
            assert!(facing.distance(at) < 0.05, "{degrees}: {facing} {at}");
            let drawn = capture_with(&frame, at, looking, 1.2);
            image::imageops::replace(&mut sheet, &drawn, i64::from(WIDTH) * index as i64, 0);
        }
        if let Some(directory) = &preview {
            sheet
                .save(directory.join("signs-viewpoints.png"))
                .expect("the preview is saved");
        }
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

/// Draw `frame` on lavapipe, from `eyes`, looking along `towards`, `fov` radians high.
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    sceneries: &Sceneries,
    frame: &Frame,
    eyes: Vec3,
    towards: Vec3,
    fov: f32,
) -> image::RgbaImage {
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("entourage stack eye"),
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
    let mut renderer = Renderer::with_sceneries(device, queue, format, WIDTH, HEIGHT, 1, sceneries);
    let mut panels = Panels::new(device, &renderer);
    draw(&mut renderer, device, queue, &mut panels, frame);
    let matrix = Mat4::perspective_rh(fov, WIDTH as f32 / HEIGHT as f32, 0.05, 80.0)
        * Mat4::look_to_rh(eyes, towards, Vec3::Y);
    renderer.render(
        device,
        queue,
        std::slice::from_ref(&view),
        &[matrix],
        Mat4::IDENTITY,
        &panels.list(Mat4::IDENTITY),
    );
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("entourage stack readback"),
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
}

/// The four corners of each slot of `wall`, where it stands, as its clients draw them: bottom left, bottom right, top
/// left and top right.
fn slot_quads(wall: &Wall) -> Vec<[Vec3; 4]> {
    let frame = wall.frame();
    wall.slot_frames()
        .iter()
        .map(|slot| {
            let at = slot.pose(&frame);
            let rotation = glam::Quat::from_array(at.orientation);
            let middle = Vec3::from_array(at.position);
            let [width, height] = slot.size;
            let corner = |across: f32, rise: f32| {
                middle + rotation * Vec3::new(across * width / 2.0, rise * height / 2.0, 0.0)
            };
            [
                corner(-1.0, -1.0),
                corner(1.0, -1.0),
                corner(-1.0, 1.0),
                corner(1.0, 1.0),
            ]
        })
        .collect()
}

/// Where the segment from `from` to `to` passes through the quad `quad`, as a fraction of the way, if it does.
fn through(from: Vec3, to: Vec3, quad: &[Vec3; 4]) -> Option<f32> {
    let [a, b, c, _] = *quad;
    let (right, up) = (b - a, c - a);
    let normal = right.cross(up);
    let along = to - from;
    // Along the quad's plane, it is never through it: the fraction is not finite.
    let t = normal.dot(a - from) / normal.dot(along);
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    let hit = from + along * t - a;
    let (u, v) = (
        hit.dot(right) / right.length_squared(),
        hit.dot(up) / up.length_squared(),
    );
    ((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)).then_some(t)
}

/// Check that the workers `frame` draws stand on one band through the agent at `agent`, about `eyes`: each as far from
/// the eyes, horizontally, as the agent is, so that the arc passes through it; in pairs either side of it, mirrored
/// about its bearing; none behind it, nor within 40° of it, as seen from the eyes; and about its height, the first rows
/// a little below and above it.
fn check_arc(frame: &Frame, agent: Vec3, eyes: Vec3, label: &str) {
    let level = |at: Vec3| Vec3::new(at.x - eyes.x, 0.0, at.z - eyes.z);
    let bearing = |at: Vec3| {
        let along = level(at);
        along.x.atan2(-along.z)
    };
    let off = |at: Vec3| {
        let turn = bearing(at) - bearing(agent);
        turn.sin().atan2(turn.cos())
    };
    let radius = level(agent).length();
    let mut sides = Vec::new();
    for station in frame.stations {
        let at = Vec3::from_array(station.position);
        assert!(
            (level(at).length() - radius).abs() < 0.05,
            "{label}: {} at {at} is not on the arc of {radius} m through the agent",
            station.id
        );
        let turn = off(at);
        assert!(
            turn.abs() > 39.9f32.to_radians(),
            "{label}: {} stands {}° from the agent",
            station.id,
            turn.to_degrees()
        );
        sides.push((station.id.clone(), turn, at.y));
    }
    // Mirrored about the agent: the first two workers, either side of it, at the same height and angle.
    sides.sort_by(|a, b| a.0.cmp(&b.0));
    if sides.len() >= 2 {
        let (left, right) = (&sides[0], &sides[1]);
        assert!((left.1 + right.1).abs() < 1e-3, "{label}: {sides:?}");
        assert!((left.2 - right.2).abs() < 1e-4, "{label}: {sides:?}");
        // About the agent's height: the first row a little below its middle.
        assert!(
            (left.2 - agent.y).abs() <= 0.3,
            "{label}: the first row at {} m, the agent at {} m",
            left.2,
            agent.y
        );
    }
}

/// Check that the Tomori wall `wall` stands beneath the swarm and clear of the workers as `frame` draws them, seen from
/// `eyes`, looking along `towards`: every slot clear of the floor; nothing of the swarm, where its agents are drawn, seen
/// on the screen over or beside any slot, the swarm's lowest point drawn above the top of every slot; and no worker,
/// nor the sphere it is pointed at by, hidden behind a slot, nor near one.
fn check_stack(frame: &Frame, wall: &Wall, eyes: Vec3, towards: Vec3, label: &str) {
    let quads = slot_quads(wall);
    for quad in &quads {
        for corner in quad {
            assert!(
                corner.y >= entourage::CLEARANCE,
                "{label}: a slot reaches {corner} into the floor"
            );
        }
    }
    // On the screen, from the eyes, the camera level, looking towards the agent.
    let view = Mat4::look_to_rh(eyes, towards.with_y(0.0).normalize(), Vec3::Y);
    let screen = |point: Vec3| {
        let at = view.transform_point3(point);
        assert!(at.z < -0.05, "{label}: {point} is behind the eyes");
        glam::Vec2::new(at.x / -at.z, at.y / -at.z)
    };
    let swarm_lowest = frame
        .swarm
        .agents
        .iter()
        .map(|agent| screen(agent.position).y)
        .fold(f32::MAX, f32::min);
    let wall_highest = quads
        .iter()
        .flatten()
        .map(|corner| screen(*corner).y)
        .fold(f32::MIN, f32::max);
    assert!(
        swarm_lowest > wall_highest,
        "{label}: the swarm, drawn down to {swarm_lowest}, overlaps the wall, drawn up to {wall_highest}"
    );
    // Its lowest agent is above the wall's top, with the gap between them.
    let swarm_bottom = frame
        .swarm
        .agents
        .iter()
        .map(|agent| agent.position.y)
        .fold(f32::MAX, f32::min);
    let wall_top = quads
        .iter()
        .flatten()
        .map(|corner| corner.y)
        .fold(f32::MIN, f32::max);
    assert!(
        swarm_bottom - wall_top >= entourage::GAP - 0.05,
        "{label}: {swarm_bottom} above {wall_top}"
    );
    // No worker is hidden behind a slot, nor stands near one, at the edge of where it is pointed at either.
    let reach = 0.3;
    for station in frame.stations {
        let at = Vec3::from_array(station.position);
        for offset in [
            Vec3::ZERO,
            Vec3::X * reach,
            -Vec3::X * reach,
            Vec3::Y * reach,
            -Vec3::Y * reach,
        ] {
            for quad in &quads {
                assert_eq!(
                    through(eyes, at + offset, quad),
                    None,
                    "{label}: worker {} at {at} is behind a slot",
                    station.id
                );
            }
        }
        for quad in &quads {
            let middle = (quad[0] + quad[3]) / 2.0;
            assert!(
                middle.distance(at) > 0.6,
                "{label}: worker {} at {at} is by a slot",
                station.id
            );
        }
    }
}

#[test]
fn the_tomori_wall_stands_beneath_the_agent_at_home_and_summoned_by_someone_seated_or_standing() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut sceneries = Sceneries::empty();
        tomori_environment_farmyard_client::register(&mut sceneries);
        let preview = std::env::var_os("TOMORI_ENTOURAGE_PREVIEW").map(std::path::PathBuf::from);
        let mut feedback = Feedback::new(Settings::default());

        // At home: from where participants arrive, the agent at its bench, its wall beneath it, its workers about it.
        let eyes = Vec3::from_array(SUMMONER);
        let agent_home = Vec3::from_array(carried(layout::AGENT_HOME).position);
        let towards = (agent_home - eyes).with_y(0.0).normalize();
        let mut conversation = joined(6, None);
        let mut step = Step::default();
        let mut now = Instant::now();
        let frame = run(
            &mut step,
            &mut conversation,
            &mut feedback,
            &senses(eyes, towards),
            8.0,
            &mut now,
        );
        let home = tomori_wall(home_wall());
        check_stack(&frame, &home, eyes, towards, "at home");
        check_arc(&frame, agent_home, eyes, "at home");
        assert!(entourage::seen_beneath(
            SUMMONER,
            &carried(layout::AGENT_HOME),
            &home
        ));
        let at_home = render(
            &device,
            &queue,
            &sceneries,
            &frame,
            eyes,
            towards - Vec3::Y * 0.12,
            1.3,
        );
        if let Some(directory) = &preview {
            std::fs::create_dir_all(directory).unwrap();
            at_home
                .save(directory.join("home.png"))
                .expect("the preview is saved");
        }

        // Summoned by someone seated, their eyes 1 m up, and by someone standing, 1.6 m and 1.8 m: the agent a little
        // above their eyes, or high enough for its wall, and the wall beneath it, read below their eyes.
        let mut sheet = image::RgbaImage::new(WIDTH, HEIGHT * 3);
        for (row, height) in [1.0f32, 1.6, 1.8].into_iter().enumerate() {
            let eyes = Vec3::new(0.0, height, 0.0);
            let mut conversation = joined(6, None);
            let mut step = Step::default();
            let mut now = Instant::now();
            let turned = senses(eyes, summoning());
            run(
                &mut step,
                &mut conversation,
                &mut feedback,
                &turned,
                8.0,
                &mut now,
            );
            summon_by(&mut conversation, 6, eyes.to_array());
            let frame = run(
                &mut step,
                &mut conversation,
                &mut feedback,
                &turned,
                10.0,
                &mut now,
            );
            let (agent, wall) = summons_by(eyes.to_array());
            let label = format!("summoned, eyes {height} m up");
            assert!(
                (agent.position[1] - entourage::summoned_height(height, entourage::FLOOR)).abs()
                    < 1e-5
            );
            assert!(agent.position[1] >= entourage::LOWEST_AGENT - 1e-5);
            assert!(wall.position[1] < height, "{label}");
            // The swarm has glided there.
            assert!(Vec3::from_array(frame.agent).distance(Vec3::from_array(agent.position)) < 0.6);
            let wall = tomori_wall(wall);
            assert!(
                entourage::seen_beneath(eyes.to_array(), &agent, &wall),
                "{label}"
            );
            check_stack(&frame, &wall, eyes, summoning(), &label);
            check_arc(&frame, Vec3::from_array(agent.position), eyes, &label);
            let drawn = render(
                &device,
                &queue,
                &sceneries,
                &frame,
                eyes,
                summoning() - Vec3::Y * 0.2,
                1.6,
            );
            image::imageops::replace(&mut sheet, &drawn, 0, (row as u32 * HEIGHT).into());
        }
        if let Some(directory) = &preview {
            std::fs::create_dir_all(directory).unwrap();
            sheet
                .save(directory.join("summoned.png"))
                .expect("the preview is saved");
        }
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

/// The farmyard's `home` cluster anchor, as Tomori's own server sends it, carried: its workers in a ring about the agent
/// at its workbench, all but straight behind it.
fn ring_home() -> tomori_protocol::cluster::Anchor {
    tomori_protocol::cluster::Anchor::new(
        tomori_protocol::cluster::HOME,
        carried(layout::AGENT_HOME),
    )
    .with_formation(tomori_protocol::cluster::Formation::Ring)
    .with_sweep(layout::HOME_SWEEP)
}

#[test]
fn at_home_the_workers_ring_the_agent_at_its_workbench_those_behind_it_over_the_bench() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut sceneries = Sceneries::empty();
        tomori_environment_farmyard_client::register(&mut sceneries);
        let mut feedback = Feedback::new(Settings::default());
        // Ten workers, the farmyard's home anchor a ring, as the server sends them.
        let mut conversation = joined(10, None);
        let mut ringed = state(10, None);
        ringed.anchors = ringed
            .anchors
            .with_workers(carried(layout::AGENT_HOME))
            .with_cluster(ring_home());
        conversation.apply(Event::Message(ServerMessage::Space {
            state: Box::new(ringed),
        }));
        // Its status wall the notice board, across the yard, and no wall beneath the agent.
        conversation.apply(Event::Message(ServerMessage::Projection {
            projection: Box::new(Projection::new(SPACE, farmyard().id()).with_walls(vec![
                Wall::new(
                    "environment-0",
                    WallKind::Environment,
                    carried(layout::NOTICEBOARD),
                )
                .with_status(true),
            ])),
        }));
        conversation.take_actions();
        let eyes = Vec3::from_array(SUMMONER);
        let agent = Vec3::from_array(carried(layout::AGENT_HOME).position);
        let towards = (agent - eyes).with_y(0.0).normalize();
        let mut step = Step::default();
        let mut now = Instant::now();
        let frame = run(
            &mut step,
            &mut conversation,
            &mut feedback,
            &senses(eyes, towards),
            8.0,
            &mut now,
        );
        // The swarm at home, at the heart of the bench's curve; every worker on the ring about it, those behind it, as
        // seen from the arrival, above what lies on the bench, along its curve, and the rest in the yard about it.
        assert!(Vec3::from_array(frame.agent).distance(agent) < 0.05);
        assert_eq!(frame.stations.len(), 10);
        let carriage = carriage();
        let into_space = |at: glam::Vec2| {
            let at = carriage.carry(&Pose::at([at.x, 0.0, at.y])).position;
            glam::Vec2::new(at[0], at[2])
        };
        let middle = into_space(layout::WORKBENCH);
        let behind = into_space(layout::on_workbench(1.0, 0.0)) - middle;
        let mut over = 0;
        for station in frame.stations {
            let at = Vec3::from_array(station.position);
            let flat = glam::Vec2::new(at.x, at.z);
            let radius = flat.distance(middle);
            assert!(
                (radius - tomori_protocol::cluster::RING).abs() < 0.05,
                "{} at {at}, {radius} m from the agent",
                station.id
            );
            if (flat - middle).dot(behind) > 0.0 {
                assert!(
                    at.y > layout::WORKBENCH_CLUTTER + 0.2,
                    "{} at {at}",
                    station.id
                );
                over += 1;
            }
        }
        assert_eq!(over, 4, "two of each row's six");
        // Drawn from the arrival turned towards it, from the yard before it, looking into the corner, and from within
        // the ring, beside the agent, looking at the rack's tools.
        let out = (eyes - agent).with_y(0.0).normalize();
        let yard = agent + out * 2.6 + Vec3::Y * 0.1;
        let within = agent + out * 0.9 + out.cross(Vec3::Y) * 0.5 - Vec3::Y * 0.1;
        // And from straight above the agent, the ring on the floor about the middle of the bench's curve.
        let above = agent.with_y(7.0);
        let views = [
            (eyes, towards - Vec3::Y * 0.12, 1.2),
            (yard, -out - Vec3::Y * 0.25, 1.3),
            (within, -out - Vec3::Y * 0.3, 1.2),
            (above, -Vec3::Y + out * 0.001, 0.75),
        ];
        let mut sheet = image::RgbaImage::new(WIDTH, HEIGHT * views.len() as u32);
        for (row, (from, looking, fov)) in views.into_iter().enumerate() {
            let drawn = render(&device, &queue, &sceneries, &frame, from, looking, fov);
            // The bench's wood, lit warmly by its lantern, below the swarm: redder than it is blue.
            let matrix = Mat4::perspective_rh(fov, WIDTH as f32 / HEIGHT as f32, 0.05, 80.0)
                * Mat4::look_to_rh(from, looking, Vec3::Y);
            let top = layout::on_workbench(layout::WORKBENCH_TOP[1] - 0.15, 0.25);
            let top = carriage
                .carry(&Pose::at([top.x, layout::WORKBENCH_HEIGHT + 0.01, top.y]))
                .position;
            let clip = matrix * Vec3::from_array(top).extend(1.0);
            let (x, y) = (
                ((clip.x / clip.w + 1.0) / 2.0 * WIDTH as f32) as u32,
                ((1.0 - clip.y / clip.w) / 2.0 * HEIGHT as f32) as u32,
            );
            assert!(x < WIDTH && y < HEIGHT, "the bench is in view: {x} {y}");
            let [red, _, blue, _] = drawn.get_pixel(x, y).0;
            assert!(
                red > blue,
                "the bench is wood: {:?}",
                drawn.get_pixel(x, y).0
            );
            image::imageops::replace(&mut sheet, &drawn, 0, (row as u32 * HEIGHT).into());
        }
        if let Some(directory) = std::env::var_os("TOMORI_ENTOURAGE_PREVIEW") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            sheet
                .save(directory.join("workbench.png"))
                .expect("the preview is saved");
        }
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}
