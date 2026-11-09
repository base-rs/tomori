// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The farmyard's notice board as participants see it from where they arrive: its wall, two slots wide and one high,
//! locked, stands on the wooden board against the fence, where Tomori's own server registers it, carried into the
//! space's coordinates as the server carries it, and drawn by Tomori's own spectator's sceneries, as the app draws
//! them. `TOMORI_NOTICE_BOARD_PREVIEW`, set to a PNG path, saves the render. See `design/farmyard.md`.
use std::time::Instant;

use glam::{Mat4, Vec3};
use tomori_client::{
    ConnectionState, Event,
    feedback::{Feedback, Settings},
    server_choice::{Choice, Source},
};
use tomori_protocol::{
    Anchors, ConnectionRole, EnvironmentInfo, Panel, Participant, Place, Pose, Projection,
    ServerMessage, SpaceState, TOMORI_WALL, Wall, WallKind, WallShape,
};
use tomori_render::{
    graphics::Renderer,
    scene_graphics::{Panels, draw},
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

/// Where the farmyard's own poses stand in a space, carried from its arrival to home, as the server carries them.
fn carried(pose: Pose) -> Pose {
    Pose::carriage(&tomori_server_app::farmyard_arrival()).carry(&pose)
}

/// The farmyard's notice board, as the server sends it: carried, two slots wide and one high, and locked.
fn notice_board() -> Wall {
    let board = tomori_server_app::farmyard_noticeboard();
    Wall::new("environment-0", WallKind::Environment, carried(board.pose))
        .with_grid(board.grid())
        .with_locked(true)
}

/// Joined to a farmyard space as an owner, as Tomori's own server sends it: its arrival, the Tomori wall at its home
/// and the notice board, with a panel on each.
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
    conversation.apply(Event::Message(ServerMessage::Space {
        state: Box::new(SpaceState {
            id: SPACE.into(),
            title: "Farm".into(),
            backend: "opencode".into(),
            directory: "/work/farm".into(),
            environment: farmyard.id().into(),
            environment_version: Some(farmyard.version()),
            participants: vec![Participant {
                name: "Ada".into(),
                source: 1,
                engagement: None,
            }],
            anchors: Anchors::new()
                .with_arrival(tomori_server_app::farmyard_arrival())
                .with_agent(carried(tomori_server_app::farmyard_agent())),
            projection: true,
            walls: true,
            wall_locks: true,
            ..Default::default()
        }),
    }));
    let tomori = Wall::new(
        TOMORI_WALL,
        WallKind::Tomori,
        // Beneath the agent's home, as the server stands it.
        tomori_protocol::entourage::wall_beneath(
            &carried(tomori_server_app::farmyard_agent()),
            tomori_protocol::entourage::FLOOR,
        ),
    )
    .with_shape(WallShape::Arc { radius: 2.3 });
    let panels = vec![
        Panel::new(
            "panel-1",
            "Feeding",
            "- [x] Grain at six\n- [ ] Fresh water\n- [ ] Collect the eggs",
            Place::Wall {
                wall: "environment-0".into(),
                slot: 0,
            },
        ),
        Panel::new(
            "panel-2",
            "Summary",
            "The hens are **settled** for the night.",
            Place::Wall {
                wall: TOMORI_WALL.into(),
                slot: 1,
            },
        ),
    ];
    conversation.apply(Event::Message(ServerMessage::Projection {
        projection: Box::new(
            Projection::new(SPACE, farmyard.id())
                .with_walls(vec![tomori, notice_board()])
                .with_panels(panels),
        ),
    }));
    conversation
}

/// Where `point`, in the space, is drawn by `matrix`, in pixels from the top left, if it is in front.
fn on_screen(matrix: Mat4, point: Vec3) -> Option<(u32, u32)> {
    let clip = matrix * point.extend(1.0);
    (clip.w > 0.0).then(|| {
        let ndc = clip.truncate() / clip.w;
        (
            ((ndc.x + 1.0) / 2.0 * WIDTH as f32) as u32,
            ((1.0 - ndc.y) / 2.0 * HEIGHT as f32) as u32,
        )
    })
}

#[test]
fn the_farmyard_s_notice_board_stands_on_its_wooden_board_seen_from_where_participants_arrive() {
    let wall = notice_board();
    let middle = Vec3::from_array(wall.pose.position);
    // Its wall stands about 4.5 m from the eyes where participants arrive, as a board on the fence to their right.
    let distance = middle.distance(arrival::HOME);
    assert!((3.5..5.5).contains(&distance), "{distance}");
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("notice board test eye"),
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
            Renderer::with_sceneries(&device, &queue, format, WIDTH, HEIGHT, 1, &sceneries);
        let mut panels = Panels::new(&device, &renderer);
        let mut conversation = joined();
        let mut step = Step::default();
        let mut feedback = Feedback::new(Settings::default());
        let now = Instant::now();
        // Standing where participants arrive, turned towards the notice board.
        let towards = Vec3::new(middle.x, arrival::HOME.y, middle.z);
        let head = Mat4::look_at_rh(arrival::HOME, towards, Vec3::Y).inverse();
        let senses = Senses {
            focused: true,
            head: Some(head),
            head_pose: controller::Pose::from_transform(head),
            viewer: Some(arrival::HOME),
            space_head: head,
            trigger: Some(0.0),
            ..Default::default()
        };
        // The farmyard grows in.
        for _ in 0..600 {
            step.update(&senses, &mut conversation, &mut feedback, now, 1.0 / 72.0);
        }
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, 1.0 / 72.0);
        draw(&mut renderer, &device, &queue, &mut panels, &frame);
        let matrix = Mat4::perspective_rh(1.2, WIDTH as f32 / HEIGHT as f32, 0.05, 80.0)
            * Mat4::look_at_rh(arrival::HOME, middle - Vec3::Y * 0.2, Vec3::Y);
        renderer.render(
            &device,
            &queue,
            std::slice::from_ref(&view),
            &[matrix],
            Mat4::IDENTITY,
            &panels.list(Mat4::IDENTITY),
        );
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("notice board readback"),
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
        let image = image::RgbaImage::from_raw(WIDTH, HEIGHT, data).unwrap();
        if let Some(path) = std::env::var_os("TOMORI_NOTICE_BOARD_PREVIEW") {
            image.save(path).expect("the preview is saved");
        }
        // The wooden board frames the wall: just beyond each side of the wall, on its back, the wood is drawn, warmer
        // than the evening sky behind it.
        let frame_of =
            Mat4::from_rotation_translation(glam::Quat::from_array(wall.pose.orientation), middle);
        let (width, _) = wall.board().unwrap();
        for side in [-1.0f32, 1.0] {
            let wood =
                frame_of.transform_point3(Vec3::new(side * (width / 2.0 + 0.05), 0.0, -0.03));
            let (x, y) = on_screen(matrix, wood).expect("in view");
            let [red, _, blue, _] = image.get_pixel(x, y).0;
            assert!(red > blue, "wood at {side}: {:?}", image.get_pixel(x, y).0);
        }
        // Above the roof, the sky.
        let sky = frame_of.transform_point3(Vec3::new(0.0, 1.2, 0.0));
        let (x, y) = on_screen(matrix, sky).expect("in view");
        let [red, _, blue, _] = image.get_pixel(x, y).0;
        assert!(blue > red, "sky: {:?}", image.get_pixel(x, y).0);
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}
