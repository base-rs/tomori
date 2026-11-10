// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Panels the agent floats beside someone across the farmyard from every wall, drawn on lavapipe as they see them and
//! from behind them: three, fanned to their right, their left and further right, where the server places them, clear
//! of the farmyard's scenery and walls, as the server carries them into the space. `TOMORI_BESIDE_PREVIEW`, set to a
//! directory, saves the renders there: `beside-first-person.png`, from their eyes, and `beside-overview.png`, from
//! behind and above them. See `design/projection.md`.
use std::time::Instant;

use glam::{Mat4, Vec3};
use tomori_client::{
    ConnectionState, Event,
    feedback::{Feedback, Settings},
    server_choice::{Choice, Source},
};
use tomori_protocol::{
    Anchors, ConnectionRole, EnvironmentInfo, Panel, Participant, Place, Pose, Projection,
    ServerMessage, SpaceState, Wall, WallKind,
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

/// The farmyard's scenery, carried into the space as the server carries it: each box about its corners carried.
fn solids() -> Vec<([f32; 3], [f32; 3])> {
    tomori_environment_farmyard::layout::solids()
        .into_iter()
        .map(|(low, high)| {
            let corners: Vec<[f32; 3]> = (0..8)
                .map(|corner: usize| {
                    let point = std::array::from_fn(|axis| {
                        if corner >> axis & 1 == 0 {
                            low[axis]
                        } else {
                            high[axis]
                        }
                    });
                    carried(Pose::at(point)).position
                })
                .collect();
            let bound = |pick: fn(f32, f32) -> f32| -> [f32; 3] {
                std::array::from_fn(|axis| {
                    corners
                        .iter()
                        .map(|corner| corner[axis])
                        .reduce(pick)
                        .unwrap()
                })
            };
            (bound(f32::min), bound(f32::max))
        })
        .collect()
}

/// The walls of a farmyard space, as the server sends them: the notice board, its status wall.
fn walls() -> Vec<Wall> {
    let board = tomori_server_app::farmyard_noticeboard();
    vec![
        Wall::new("environment-0", WallKind::Environment, carried(board.pose))
            .with_grid(board.grid())
            .with_locked(true)
            .with_status(true),
    ]
}

/// Samuel's head, in the farmyard's left corner, by the fence, looking towards the gate, along +Z of the farmyard's own
/// coordinates, far from the notice board: a head looks along its −Z.
fn head() -> Pose {
    let eyes = tomori_environment::sightlines::EYES;
    carried(Pose::at([-3.5, eyes, -1.0]).facing_towards([-3.5, eyes, -5.0]))
}

/// Joined to a farmyard space, with three panels floating beside Samuel where the server places them.
fn joined(beside: &[Pose]) -> Conversation {
    let farmyard = Environment::new(tomori_environment_farmyard::ID)
        .with_version(tomori_environment_farmyard::VERSION);
    let mut conversation = Conversation::new("Samuel");
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
                name: "Samuel".into(),
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
    let contents = [
        (
            "Evening rounds",
            "- [x] Grain at six\n- [ ] Fresh water\n- [ ] Collect the eggs\n- [ ] Close the coop",
        ),
        (
            "Steps",
            "1. Fill the trough\n2. Check the hay\n3. Lock the gate",
        ),
        (
            "Figures",
            "Hens: **4**\nEggs today: **7**\nWater: 60 litres",
        ),
    ];
    let panels = contents
        .iter()
        .zip(beside)
        .enumerate()
        .map(|(index, ((title, body), pose))| {
            Panel::new(
                format!("panel-{}", index + 1),
                *title,
                *body,
                Place::Free { pose: *pose },
            )
        })
        .collect();
    conversation.apply(Event::Message(ServerMessage::Projection {
        projection: Box::new(
            Projection::new(SPACE, farmyard.id())
                .with_walls(walls())
                .with_panels(panels),
        ),
    }));
    conversation
}

/// Where `point`, in the space, is drawn by `matrix`, in pixels from the left, if it is in front.
fn across(matrix: Mat4, point: Vec3) -> Option<f32> {
    let clip = matrix * point.extend(1.0);
    (clip.w > 0.0).then(|| (clip.x / clip.w + 1.0) / 2.0 * WIDTH as f32)
}

#[test]
fn three_panels_float_fanned_beside_someone_far_from_every_wall() {
    let head = head();
    let beside = tomori_server::presence::fanned_beside(&head, &walls(), &solids(), 3);
    let eyes = Vec3::from_array(head.position);
    let ahead = glam::Quat::from_array(head.orientation) * Vec3::NEG_Z;
    let ahead = Vec3::new(ahead.x, 0.0, ahead.z).normalize();
    // Every wall is further than it can be read from, so the server floats them beside Samuel, as it does by default.
    for wall in walls() {
        let middle = Vec3::from_array(wall.pose.position);
        assert!(middle.distance(eyes) > 4.5, "{}", wall.id);
    }
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("beside test eye"),
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
        let mut conversation = joined(&beside);
        let mut step = Step::default();
        let mut feedback = Feedback::new(Settings::default());
        let now = Instant::now();
        let transform = Mat4::look_at_rh(eyes, eyes + ahead, Vec3::Y).inverse();
        let senses = Senses {
            focused: true,
            head: Some(transform),
            head_pose: controller::Pose::from_transform(transform),
            viewer: Some(eyes),
            space_head: transform,
            trigger: Some(0.0),
            ..Default::default()
        };
        // The farmyard grows in.
        for _ in 0..600 {
            step.update(&senses, &mut conversation, &mut feedback, now, 1.0 / 72.0);
        }
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, 1.0 / 72.0);
        draw(&mut renderer, &device, &queue, &mut panels, &frame);
        let aspect = WIDTH as f32 / HEIGHT as f32;
        // From their eyes, about as wide as a headset sees; and from behind and above them, to see all three.
        let first = Mat4::perspective_rh(1.35, aspect, 0.05, 80.0)
            * Mat4::look_at_rh(eyes, eyes + ahead - Vec3::Y * 0.1, Vec3::Y);
        let behind = eyes - ahead * 0.8 + Vec3::Y * 2.0;
        let overview = Mat4::perspective_rh(1.5, aspect, 0.05, 80.0)
            * Mat4::look_at_rh(behind, eyes + ahead * 0.6 - Vec3::Y * 0.6, Vec3::Y);
        for (name, matrix) in [
            ("beside-first-person.png", first),
            ("beside-overview.png", overview),
        ] {
            let image = rendered(
                &device,
                &queue,
                &mut renderer,
                &panels,
                &view,
                &texture,
                matrix,
            );
            if let Some(directory) = std::env::var_os("TOMORI_BESIDE_PREVIEW") {
                image
                    .save(std::path::Path::new(&directory).join(name))
                    .expect("the preview is saved");
            }
            // To the right of the middle, to its left, and further right, as seen.
            let seen: Vec<f32> = beside
                .iter()
                .map(|pose| across(matrix, Vec3::from_array(pose.position)).expect("in view"))
                .collect();
            let middle = WIDTH as f32 / 2.0;
            assert!(
                seen[0] > middle && seen[1] < middle && seen[2] > seen[0],
                "{name}: {seen:?}"
            );
            // Each panel's face is drawn there, darker than the evening behind it; from their eyes, the third, 70° to
            // their right, is at the edge of the view, as it is at the edge of a headset's.
            for (pose, x) in beside.iter().zip(&seen).filter(|(_, x)| **x < WIDTH as f32) {
                let clip = matrix * Vec3::from_array(pose.position).extend(1.0);
                let y = ((1.0 - clip.y / clip.w) / 2.0 * HEIGHT as f32) as u32;
                let [red, green, blue, _] = image.get_pixel(*x as u32, y).0;
                let lightness = u32::from(red) + u32::from(green) + u32::from(blue);
                assert!(
                    lightness < 300,
                    "{name}: {:?}",
                    image.get_pixel(*x as u32, y).0
                );
            }
        }
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

/// What `renderer` draws through `matrix` into `texture`, read back.
fn rendered(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut Renderer,
    panels: &Panels,
    view: &wgpu::TextureView,
    texture: &wgpu::Texture,
    matrix: Mat4,
) -> image::RgbaImage {
    renderer.render(
        device,
        queue,
        std::slice::from_ref(view),
        &[matrix],
        Mat4::IDENTITY,
        &panels.list(Mat4::IDENTITY),
    );
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("beside readback"),
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
