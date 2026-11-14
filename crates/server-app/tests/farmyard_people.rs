// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The others in a farmyard space, at dusk, as a participant sees them: two people at the Tomori wall, the agent at
//! work above it, Bob looking this way as he speaks and Cleo pointing at a panel, each a swarm of cubes glowing in their
//! colour against the evening, hazed as everything is, drawn by Tomori's own spectator's sceneries, as the app draws
//! them. Set `TOMORI_FARMYARD_PEOPLE_PREVIEW` to a path to save `at-the-wall.png`, `near.png` and `head.png` after it.
//! See `design/people.md`.
use std::time::Instant;

use glam::{Mat4, Quat, Vec3};
use tomori_client::{
    ConnectionState, Event,
    connection::{Report, Session},
    feedback::{Feedback, Settings},
    server_choice::{Choice, Source},
};
use tomori_protocol::{
    Activity, AgentState, AgentStatus, Anchors, ConnectionRole, EnvironmentInfo, Panel,
    Participant, Place, Pose, Projection, Ray, ServerMessage, SpaceState, TOMORI_WALL, Wall,
    WallKind, WallShape,
};
use tomori_render::{
    graphics::Renderer,
    scene_graphics::{Panels, draw},
    test_gpu,
};
use tomori_scene::{
    controller,
    conversation::Conversation,
    environment::Environment,
    scene::{Senses, Step},
};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
const SPACE: &str = "space-1";

/// Where the farmyard's own poses stand in a space, carried from its arrival to home, as the server carries them.
fn carried(pose: Pose) -> Pose {
    Pose::carriage(&tomori_server_app::farmyard_arrival()).carry(&pose)
}

/// The Tomori wall, beneath the agent's home, as the server stands it.
fn tomori_wall() -> Wall {
    Wall::new(
        TOMORI_WALL,
        WallKind::Tomori,
        tomori_protocol::stance::wall_beneath(
            &carried(tomori_server_app::farmyard_agent()),
            tomori_protocol::cluster::FLOOR,
        ),
    )
    .with_shape(WallShape::Arc { radius: 2.3 })
}

/// Joined to a farmyard space as an owner, with Bob and Cleo, the agent at work and a panel on the Tomori wall.
fn joined() -> Conversation {
    let farmyard = Environment::new(tomori_environment_farmyard::ID)
        .with_version(tomori_environment_farmyard::VERSION);
    let mut conversation = Conversation::new("Ada");
    conversation.set_drawable(vec![farmyard]);
    conversation.connect(Choice::new("Studio", "10.0.0.1:8765", Source::Discovered));
    conversation.apply(Event::State(ConnectionState::Connected {
        source: 1,
        server: "studio".into(),
        role: ConnectionRole::Owner,
        certificate: None,
        backends: vec![],
        roots: vec![],
        environments: vec![EnvironmentInfo {
            id: farmyard.id().into(),
            label: farmyard.id().into(),
            version: farmyard.version(),
        }],
        utterance_limit: None,
        name: None,
    }));
    let participant = |name: &str, source| Participant {
        name: name.into(),
        source,
        engagement: None,
    };
    conversation.apply(Event::Message(ServerMessage::Space {
        state: Box::new(SpaceState {
            id: SPACE.into(),
            title: "Farm".into(),
            backend: "opencode".into(),
            directory: "/work/farm".into(),
            environment: farmyard.id().into(),
            environment_version: Some(farmyard.version()),
            participants: vec![
                participant("Ada", 1),
                participant("Bob", 2),
                participant("Cleo", 3),
            ],
            anchors: Anchors::new()
                .with_arrival(tomori_server_app::farmyard_arrival())
                .with_agent(carried(tomori_server_app::farmyard_agent())),
            projection: true,
            walls: true,
            activity: Activity::Running,
            agent: Some(AgentStatus {
                state: AgentState::Working,
                detail: None,
                sequence: 1,
            }),
            ..Default::default()
        }),
    }));
    conversation.apply(Event::Message(ServerMessage::Projection {
        projection: Box::new(
            Projection::new(SPACE, farmyard.id())
                .with_walls(vec![tomori_wall()])
                .with_panels(vec![Panel::new(
                    "panel-1",
                    "Feeding",
                    "- [x] Grain at six\n- [ ] Fresh water\n- [ ] Collect the eggs",
                    Place::Wall {
                        wall: TOMORI_WALL.into(),
                        slot: 1,
                    },
                )]),
        ),
    }));
    conversation
}

/// A head at `at` facing `target`.
fn looking(at: Vec3, target: Vec3) -> Pose {
    Pose::upright_facing(at.to_array(), (at * 2.0 - target).to_array())
}

/// `at` from `head`, in its own coordinates, turned as it is.
fn held(head: Pose, at: Vec3) -> Option<Pose> {
    let turn = Quat::from_array(head.orientation);
    Some(Pose {
        position: (Vec3::from_array(head.position) + turn * at).to_array(),
        orientation: head.orientation,
    })
}

/// The mean linear luminance of `image`'s pixels that differ from `without`'s, and how many there are.
fn differing(image: &image::RgbaImage, without: &image::RgbaImage) -> (f32, usize) {
    let linear = |channel: u8| ((f32::from(channel) / 255.0 + 0.055) / 1.055).powf(2.4);
    let luminance = |pixel: [u8; 4]| {
        0.2126 * linear(pixel[0]) + 0.7152 * linear(pixel[1]) + 0.0722 * linear(pixel[2])
    };
    let (mut sum, mut count) = (0.0, 0);
    for (a, b) in image.pixels().zip(without.pixels()) {
        if a.0.iter().zip(b.0).any(|(a, b)| a.abs_diff(b) > 8) {
            sum += luminance(a.0);
            count += 1;
        }
    }
    (sum / count.max(1) as f32, count)
}

/// How many of `image`'s pixels are a bright, warm gold, as the eyes glow.
fn gold(image: &image::RgbaImage) -> usize {
    image
        .pixels()
        .filter(|pixel| {
            let [r, g, b, _] = pixel.0.map(i32::from);
            r > 230 && g > 200 && g - b > 60
        })
        .count()
}

#[test]
fn the_others_glow_in_the_farmyard_at_dusk_their_eyes_warm_as_they_look_this_way() {
    let wall = tomori_wall();
    let middle = Vec3::from_array(wall.pose.position);
    let out = (Quat::from_array(wall.pose.orientation) * Vec3::Z)
        .with_y(0.0)
        .normalize();
    let across = Vec3::Y.cross(out).normalize();
    // Ada stands 3.6 m out from the wall's middle, her eyes 1.6 m above the floor, as they are at home.
    let floor = tomori_scene::arrival::HOME.y - 1.6;
    let eye = (middle + out * 3.6).with_y(floor + 1.6);
    let bob_head = (middle + out * 1.5 - across * 0.75).with_y(floor + 1.68);
    let cleo_head = (middle + out * 1.35 + across * 0.7).with_y(floor + 1.56);
    // Cleo points at the panel in the wall's second slot.
    let slot = wall.slot_pose(1).expect("a second slot");
    let on_wall =
        Vec3::from_array(slot.position) + Quat::from_array(slot.orientation) * Vec3::Z * 0.03;
    let cleo_looking = looking(cleo_head, on_wall);
    let aim = cleo_head + Quat::from_array(cleo_looking.orientation) * Vec3::new(0.22, -0.3, -0.42);
    let along = on_wall - aim;
    let pointing = Quat::from_rotation_arc(Vec3::NEG_Z, along.normalize());
    let presence = |source, head: Pose, controllers: [Option<Pose>; 2], ray: Option<Ray>| {
        Event::Message(ServerMessage::Presence {
            source,
            head,
            left: controllers[0],
            right: controllers[1],
            hands: None,
            pointing: ray.map(|ray| Box::new([None, Some(ray)])),
        })
    };
    let bob = |looking_at: Vec3| {
        let head = looking(bob_head, looking_at);
        presence(
            2,
            head,
            [
                held(head, Vec3::new(-0.24, -0.5, -0.3)),
                held(head, Vec3::new(0.26, -0.4, -0.34)),
            ],
            None,
        )
    };
    let cleo = presence(
        3,
        cleo_looking,
        [
            None,
            Some(Pose {
                position: (aim - along.normalize() * 0.06).to_array(),
                orientation: pointing.to_array(),
            }),
        ],
        Some(Ray {
            aim: Pose {
                position: aim.to_array(),
                orientation: pointing.to_array(),
            },
            length: along.length(),
            meets: true,
        }),
    );
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("farmyard people test eye"),
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
        let sceneries = tomori_client_spectator::app::sceneries();
        let mut renderer =
            Renderer::with_sceneries(&device, &queue, format, WIDTH, HEIGHT, 4, &sceneries);
        let mut panels = Panels::new(&device, &renderer);
        let now = Instant::now();
        // Ada at `seen_from`, with the others as `others` say, Bob's voice playing, seen from `looked_from`, looking at
        // `at` through a lens `fov` high.
        let mut capture =
            |others: &[&Event], seen_from: Vec3, looked_from: Vec3, at: Vec3, fov: f32| {
                let mut conversation = joined();
                for event in others {
                    conversation.apply((*event).clone());
                }
                conversation.report(Report::Voices([2].into_iter().collect()));
                let head = glam::camera::rh::view::look_at_mat4(seen_from, at, Vec3::Y).inverse();
                let senses = Senses {
                    focused: true,
                    head: Some(head),
                    head_pose: controller::Pose::from_transform(head),
                    viewer: Some(seen_from),
                    space_head: head,
                    trigger: Some(0.0),
                    ..Default::default()
                };
                let mut step = Step::default();
                let mut feedback = Feedback::new(Settings::default());
                // The farmyard grows in, and the others are drawn.
                let mut frame = step.update(&senses, &mut conversation, &mut feedback, now, 0.01);
                for _ in 0..600 {
                    frame = step.update(&senses, &mut conversation, &mut feedback, now, 1.0 / 72.0);
                }
                assert_eq!(frame.people.len(), others.len());
                draw(&mut renderer, &device, &queue, &mut panels, &frame);
                let matrix = glam::camera::rh::proj::directx::perspective(
                    fov,
                    WIDTH as f32 / HEIGHT as f32,
                    0.05,
                    80.0,
                ) * glam::camera::rh::view::look_at_mat4(looked_from, at, Vec3::Y);
                renderer.render(
                    &device,
                    &queue,
                    std::slice::from_ref(&view),
                    &[matrix],
                    Mat4::IDENTITY,
                    &panels.list(Mat4::IDENTITY),
                );
                read_back(&device, &queue, &texture)
            };
        let bob_looking = bob(eye);
        let bob_away = bob(on_wall);
        let ahead = (middle + out * 1.4).with_y(floor + 1.45);
        let wide = capture(&[&bob_looking, &cleo], eye, eye, ahead, 0.95);
        let nobody = capture(&[], eye, eye, ahead, 0.95);
        let away = capture(&[&bob_away, &cleo], eye, eye, ahead, 0.95);
        // Nearer, from their left, the two at the wall; and Bob up close, looking at Ada a little under a metre away.
        let nearer = (middle + out * 3.1 - across * 1.5).with_y(floor + 1.6);
        let near = capture(
            &[&bob(nearer), &cleo],
            nearer,
            nearer,
            (middle + out * 1.4).with_y(floor + 1.3),
            0.75,
        );
        let close = bob_head + out * 0.85 + across * 0.2 - Vec3::Y * 0.02;
        let mut up_close =
            |bob: &Event| capture(&[bob, &cleo], close, close, bob_head - Vec3::Y * 0.05, 0.75);
        let head = up_close(&bob(close));
        let head_away = up_close(&bob_away);
        if let Some(prefix) = std::env::var_os("TOMORI_FARMYARD_PEOPLE_PREVIEW") {
            let prefix = prefix.to_string_lossy();
            for (name, image) in [("at-the-wall", &wide), ("near", &near), ("head", &head)] {
                image
                    .save(format!("{prefix}{name}.png"))
                    .expect("the preview is saved");
            }
        }
        // The two glow against the evening: what they add to it is much brighter than the dusk about them.
        let (lit, count) = differing(&wide, &nobody);
        let (dusk, _) = differing(&nobody, &image::RgbaImage::new(WIDTH, HEIGHT));
        assert!(count > 4_000, "{count}");
        assert!(lit > dusk * 3.0, "{lit} against {dusk}");
        // Bob's eyes glow gold as he looks this way, and not as he looks at the wall.
        assert!(gold(&wide) > gold(&away), "{} {}", gold(&wide), gold(&away));
        assert!(
            gold(&head) > gold(&head_away) + 300,
            "{} {}",
            gold(&head),
            gold(&head_away)
        );
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

/// What is drawn into `texture`, read back.
fn read_back(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> image::RgbaImage {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("farmyard people readback"),
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
