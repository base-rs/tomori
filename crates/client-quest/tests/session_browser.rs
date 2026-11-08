// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The session browser, drawn on lavapipe from the eyes at home as the headset draws it, at about 13 pixels a degree at
//! the middle, as the headset's eye image is: 2 folders with 10 sessions, 3 with 30, 6 with 80 and 15 with 200, all made
//! up, a session pointed at in each. `TOMORI_SESSION_BROWSER_PREVIEW`, set to a directory, saves each render there as a
//! PNG, to look at. See `design/arrival.md`.
use std::time::Instant;

use glam::{Mat4, Vec2, Vec3};
use tomori_client::{
    ConnectionState, Event,
    feedback::{Feedback, Settings},
    server_choice::{Choice, Source},
};
use tomori_protocol::{
    BackendInfo, ConnectionRole, EnvironmentInfo, ServerMessage, SessionSummary, SpaceSummary,
    WorkspaceRoot,
};
use tomori_render::{
    graphics::Renderer,
    scene_graphics::{Panels, draw},
    test_gpu,
};
use tomori_scene::{
    arrival::{HOME, Key, Kind, Scene},
    controller,
    conversation::Conversation,
    pointing::{Pointer, Ray},
    scene::{Senses, Step},
};

const WIDTH: u32 = 1536;
const HEIGHT: u32 = 960;
/// 64° tall over 960 pixels: about 13 pixels a degree at the middle, as the headset's eye image is, and 90° across.
const FOV: f32 = 64.0 * std::f32::consts::PI / 180.0;
/// Looking a little below the browser's middle.
const LOOK_PITCH: f32 = -5.0;
const ROOT: &str = "/home/ada/Developer";

/// Made-up folders and titles, as a developer's are: one busy project and a long tail.
const FOLDERS: [&str; 15] = [
    "acme/billing-api",
    "kestrel/scheduler",
    "acme/web-console",
    "lumen/renderer",
    "notes/garden",
    "kestrel/io-uring",
    "acme/infra",
    "tools/dotfiles",
    "kestrel/http2",
    "lumen/shaders",
    "notes/blog",
    "tools/release-kit",
    "acme/mobile",
    "lumen/editor",
    "kestrel/bench",
];
const WORK: [&str; 16] = [
    "Fix flaky CI on macOS",
    "Speed up the timer wheel under contention",
    "Investigate memory growth under load",
    "Add a benchmark harness",
    "Review the retry back-off",
    "Explain the login redirect loop",
    "Document the release steps",
    "Port shader hot reload to the new loader",
    "Why does the cache miss after deploy?",
    "Tidy the error messages",
    "Draft the migration plan for v3",
    "Bump dependencies",
    "Trace the slow request in staging",
    "Add retries to the webhook sender",
    "Sketch the settings page",
    "Rename the config keys",
];

/// `counts` sessions in each folder, the most recently active first, interleaved as work moves between projects, some
/// in a project's subfolder, a few busy and a few already in a space.
fn sessions(counts: &[usize]) -> Vec<SessionSummary> {
    let total: usize = counts.iter().sum();
    let mut sessions = Vec::new();
    let mut left = counts.to_vec();
    while sessions.len() < total {
        for folder in 0..counts.len() {
            // Bigger folders come up more often.
            let turns = left[folder]
                .div_ceil(counts[folder].div_ceil(4).max(1))
                .min(left[folder]);
            for _ in 0..turns.min(1 + counts[folder] / 6) {
                if left[folder] == 0 {
                    break;
                }
                left[folder] -= 1;
                let index = sessions.len();
                // About two fifths today, half within the week, the rest over months.
                let fraction = index as f32 / total as f32;
                let age = (86_400.0 * 120.0 * fraction.powf(2.6)) as u64 + index as u64 * 37;
                let subfolder = if index % 7 == 3 { "/crates/core" } else { "" };
                sessions.push(SessionSummary {
                    id: format!("ses_{index}"),
                    title: WORK[(index * 5 + folder * 3) % WORK.len()].into(),
                    directory: format!("{ROOT}/{}{subfolder}", FOLDERS[folder]),
                    busy: index % 9 == 1,
                    age: Some(age),
                    space: (index % 17 == 5).then(|| "space-1".into()),
                });
            }
        }
    }
    sessions
}

/// At the session browser, with `listing`.
fn browsing(listing: Vec<SessionSummary>) -> Conversation {
    let mut conversation = Conversation::new("Ada");
    conversation.connect(Choice::new("Studio", "10.0.0.1:8765", Source::Discovered));
    conversation.apply(Event::State(ConnectionState::Connected {
        source: 1,
        server: "studio".into(),
        role: ConnectionRole::Owner,
        certificate: None,
        backends: vec![BackendInfo {
            name: "opencode".into(),
            attach: true,
        }],
        roots: vec![WorkspaceRoot {
            path: ROOT.into(),
            label: "Developer".into(),
        }],
        environments: vec![EnvironmentInfo {
            id: "sky".into(),
            label: "Sky".into(),
            version: 1,
        }],
        utterance_limit: None,
        name: None,
    }));
    conversation.apply(Event::Message(ServerMessage::Spaces {
        spaces: vec![SpaceSummary {
            id: "space-1".into(),
            title: "Deploy".into(),
            backend: "opencode".into(),
            directory: ROOT.into(),
            environment: "sky".into(),
            activity: tomori_protocol::Activity::Idle,
            participants: 1,
            attention: None,
            attention_age: None,
            folder: None,
            active_age: None,
            latest: None,
        }],
    }));
    conversation.choose_object(&Key::Create);
    conversation.choose_object(&Key::Backend("opencode".into()));
    conversation.choose_object(&Key::Directory(ROOT.into()));
    conversation.apply(Event::Message(ServerMessage::Sessions {
        sessions: listing,
    }));
    conversation.take_actions();
    conversation
}

/// The direction of `point` from home, as yaw and pitch in degrees.
fn angles(point: Vec3) -> Vec2 {
    let d = (point - HOME).normalize();
    Vec2::new(d.x.atan2(-d.z).to_degrees(), d.y.asin().to_degrees())
}

/// A ray from `origin` towards `target`, turned `jitter` degrees towards `towards` (an angle round the aim).
fn aimed(origin: Vec3, target: Vec3, jitter: f32, towards: f32) -> Ray {
    let forward = (target - origin).normalize();
    let right = forward.cross(Vec3::Y).normalize();
    let up = right.cross(forward);
    let (sin, cos) = towards.to_radians().sin_cos();
    let tilt = jitter.to_radians().tan();
    Ray::new(origin, forward + (right * cos + up * sin) * tilt).unwrap()
}

/// What the pointer picks along `ray` in `scene`.
fn picked(scene: &Scene, ray: Ray) -> Option<Key> {
    let mut pointer = Pointer::default();
    pointer
        .update(Some(ray), Some(0.0), &scene.targets())
        .hit
        .map(|hit| hit.key)
}

/// Every session shown is drawn in view, within the reach the layout allows, its label lit where it stands, at the
/// size the text rules ask for, and is pointed at by its middle from the eye and the right hand, without validation
/// errors.
#[test]
fn the_session_browser_is_drawn_around_the_create_object() {
    let scales: [(&str, Vec<usize>); 4] = [
        ("2-folders-10", vec![7, 3]),
        ("3-folders-30", vec![16, 9, 5]),
        ("6-folders-80", vec![38, 17, 11, 7, 4, 3]),
        (
            "15-folders-200",
            vec![70, 32, 22, 16, 13, 10, 8, 7, 6, 5, 4, 3, 2, 1, 1],
        ),
    ];
    let preview = std::env::var_os("TOMORI_SESSION_BROWSER_PREVIEW").map(std::path::PathBuf::from);
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("session browser test eye"),
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
        let look = HOME + Vec3::new(0.0, LOOK_PITCH.to_radians().tan(), -1.0);
        let matrix = Mat4::perspective_rh(FOV, WIDTH as f32 / HEIGHT as f32, 0.05, 80.0)
            * Mat4::look_at_rh(HOME, look, Vec3::Y);
        let capture = |renderer: &Renderer, panels: &Panels| {
            renderer.render(
                &device,
                &queue,
                std::slice::from_ref(&view),
                &[matrix],
                Mat4::IDENTITY,
                &panels.list(Mat4::IDENTITY),
            );
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("session browser readback"),
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
        let head = Mat4::from_translation(HOME);
        let senses = |ray: Option<Ray>| Senses {
            focused: true,
            head: Some(head),
            head_pose: controller::Pose::from_transform(head),
            viewer: Some(HOME),
            space_head: head,
            ray,
            pointing: ray.is_some(),
            trigger: Some(0.0),
            ..Default::default()
        };
        {
            for (scale, counts) in &scales {
                let listing = sessions(counts);
                let mut conversation = browsing(listing);
                let mut renderer = Renderer::new(&device, &queue, format, WIDTH, HEIGHT);
                let mut panels = Panels::new(&device, &renderer);
                let mut step = Step::default();
                let mut feedback = Feedback::new(Settings::default());
                let now = Instant::now();
                // The browser grows out of its centre and settles; then the ray points at a session.
                for _ in 0..300 {
                    step.update(
                        &senses(None),
                        &mut conversation,
                        &mut feedback,
                        now,
                        1.0 / 72.0,
                    );
                }
                let frame = step.update(
                    &senses(None),
                    &mut conversation,
                    &mut feedback,
                    now,
                    1.0 / 72.0,
                );
                let scene = frame.arrival.as_ref().unwrap().scene.clone();
                let pointed = scene
                    .objects
                    .iter()
                    .filter(|object| matches!(object.kind, Kind::Session { .. }))
                    .nth(3)
                    .map(|object| object.key.clone())
                    .unwrap();
                let area = scene.object(&pointed).unwrap().panel.unwrap();
                let ray = Ray::new(HOME, area.centre() - HOME).unwrap();
                for _ in 0..3 {
                    step.update(
                        &senses(Some(ray)),
                        &mut conversation,
                        &mut feedback,
                        now,
                        1.0 / 72.0,
                    );
                }
                let frame = step.update(
                    &senses(Some(ray)),
                    &mut conversation,
                    &mut feedback,
                    now,
                    1.0 / 72.0,
                );
                let arrival = frame.arrival.as_ref().unwrap();
                assert_eq!(arrival.hovered.as_ref(), Some(&pointed));
                draw(&mut renderer, &device, &queue, &mut panels, &frame);
                let drawn = capture(&renderer, &panels);
                if let Some(directory) = &preview {
                    drawn
                        .save(directory.join(format!("{scale}.png")))
                        .expect("the preview is saved");
                }
                let scene = &arrival.scene;
                let chips: Vec<_> = scene
                    .objects
                    .iter()
                    .filter(|object| object.panel.is_some())
                    .collect();
                assert!(
                    chips
                        .iter()
                        .filter(|object| matches!(object.kind, Kind::Session { .. }))
                        .count()
                        >= counts.iter().sum::<usize>().min(10),
                    "{scale}"
                );
                let project = |point: Vec3| {
                    let clip = matrix * point.extend(1.0);
                    Vec2::new(
                        (clip.x / clip.w * 0.5 + 0.5) * WIDTH as f32,
                        (0.5 - clip.y / clip.w * 0.5) * HEIGHT as f32,
                    )
                };
                let hand = HOME + Vec3::new(0.18, -0.35, -0.25);
                for object in &chips {
                    let area = object.panel.unwrap();
                    for corner in [area.rect.min, area.rect.max] {
                        let at = angles(area.plane.at(corner));
                        assert!(at.x.abs() <= 41.0, "{scale}: {:?} reaches {at}", object.key);
                        assert!(
                            (-36.0..=20.0).contains(&at.y),
                            "{scale}: {:?} reaches {at}",
                            object.key
                        );
                    }
                    // A font pixel of at least 0.08° as seen, an x-height of about half a degree.
                    let pixel = object.label.pixel / object.label.anchor.distance(HOME);
                    assert!(pixel.to_degrees() >= 0.08, "{scale}");
                    // Its label is drawn where it stands: lit text over its background.
                    let at = project(object.label.anchor);
                    assert!(
                        at.x >= 0.0 && at.y >= 0.0 && at.x < WIDTH as f32 && at.y < HEIGHT as f32
                    );
                    let lit = (0..40)
                        .flat_map(|dx| (0..12).map(move |dy| (dx, dy)))
                        .filter(|(dx, dy)| {
                            let pixel = drawn.get_pixel(
                                (at.x as u32 + dx).min(WIDTH - 1),
                                (at.y as u32 + dy).min(HEIGHT - 1),
                            );
                            pixel[0] > 150 && pixel[1] > 150
                        })
                        .count();
                    assert!(lit > 3, "{scale}: {:?}'s label is not drawn", object.key);
                    for origin in [HOME, hand] {
                        let ray = aimed(origin, area.centre(), 0.0, 0.0);
                        assert_eq!(picked(scene, ray).as_ref(), Some(&object.key), "{scale}");
                    }
                }
            }
        }
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}
