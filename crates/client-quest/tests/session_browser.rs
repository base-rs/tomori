// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The session browser, drawn on lavapipe from the eyes at home, as the headset draws it: 12, 40 and 150 sessions across
//! up to 15 folders, with a session pointed at, about 13 pixels a degree, as the headset's eye image is.
//! `TOMORI_SESSION_BROWSER_PREVIEW`, set to a PNG path, saves the renders, one above another, to look at. See
//! `design/arrival.md`.
use std::time::Instant;

use glam::{Mat4, Vec3};
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
    arrival::{self, HOME, Key},
    controller,
    conversation::Conversation,
    pointing::Ray,
    scene::{Senses, Step},
};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 860;
/// 60° tall over 860 pixels: about 13 pixels a degree at the middle, as the headset's eye image is.
const FOV: f32 = 60.0 * std::f32::consts::PI / 180.0;
const ROOT: &str = "/home/ada/Developer";

/// Made-up folders and titles, distributed as a developer's sessions are: a few busy folders and a long tail.
const FOLDERS: [&str; 15] = [
    "acme/billing-api",
    "acme/web-console",
    "kestrel/scheduler",
    "kestrel/io-uring",
    "notes/garden",
    "lumen/renderer",
    "acme/infra",
    "kestrel/http2",
    "tools/dotfiles",
    "lumen/shaders",
    "notes/blog",
    "tools/release-kit",
    "acme/mobile",
    "lumen/editor",
    "kestrel/bench",
];
const WORK: [&str; 8] = [
    "Fix flaky CI on macOS",
    "Speed up the timer wheel",
    "Investigate memory growth under load",
    "Add a benchmark harness",
    "Review the retry back-off",
    "Explain the login redirect loop",
    "Document the release steps",
    "Port shader hot reload",
];

/// `total` sessions, the most recently active first, across the folders in proportion to `counts`.
fn sessions(total: usize) -> Vec<SessionSummary> {
    let counts: &[usize] = match total {
        12 => &[3, 2, 2, 1, 1, 1, 1, 1],
        40 => &[9, 6, 5, 4, 3, 3, 2, 2, 1, 1, 1, 1, 1, 1],
        _ => &[38, 24, 17, 13, 11, 9, 8, 7, 6, 5, 4, 3, 2, 2, 1],
    };
    let mut sessions = Vec::new();
    let most = counts.iter().copied().max().unwrap_or(0);
    for turn in 0..most {
        for (folder, count) in counts.iter().enumerate() {
            if turn < *count {
                let n = sessions.len();
                sessions.push(SessionSummary {
                    id: format!("ses_{n}"),
                    title: WORK[(n * 5 + folder) % WORK.len()].into(),
                    directory: format!("{ROOT}/{}", FOLDERS[folder]),
                    busy: n % 11 == 0,
                    age: Some(n as u64 * n as u64 * 400),
                    space: (n % 13 == 4).then(|| "space-1".into()),
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

#[test]
fn the_session_browser_is_drawn_below_the_connected_server() {
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
        // From home, looking down at the browser's middle, as a participant reading it does.
        let look = HOME + Vec3::new(0.0, -(28f32.to_radians().tan()), -1.0);
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
        let project = |point: Vec3| {
            let clip = matrix * point.extend(1.0);
            (
                ((clip.x / clip.w * 0.5 + 0.5) * WIDTH as f32) as u32,
                ((0.5 - clip.y / clip.w * 0.5) * HEIGHT as f32) as u32,
            )
        };
        let counts = [12, 40, 150];
        let mut sheet = image::RgbaImage::new(WIDTH, HEIGHT * counts.len() as u32);
        for (index, count) in counts.into_iter().enumerate() {
            let listing = sessions(count);
            let pointed = listing[2].id.clone();
            let mut conversation = browsing(listing);
            let mut renderer = Renderer::new(&device, &queue, format, WIDTH, HEIGHT);
            let mut panels = Panels::new(&device, &renderer);
            let mut step = Step::default();
            let mut feedback = Feedback::new(Settings::default());
            let now = Instant::now();
            // The browser grows out of the create object and settles; then the ray points at a session.
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
            let row = scene
                .object(&Key::Session(Some(pointed.clone())))
                .unwrap()
                .panel
                .unwrap();
            let ray = Ray::new(HOME, row.centre() - HOME).unwrap();
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
            assert_eq!(arrival.hovered, Some(Key::Session(Some(pointed))));
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            let drawn = capture(&renderer, &panels);
            // Every row stands in view, its label drawn where it is: lit text over its translucent background.
            let rows: Vec<_> = arrival
                .scene
                .objects
                .iter()
                .filter_map(|object| object.panel.map(|area| (object, area)))
                .collect();
            // The sessions are in several folders, so they are indexed, and both columns are full.
            assert!(
                rows.iter()
                    .any(|(object, _)| matches!(object.key, Key::Folder(_)))
            );
            assert_eq!(rows.len(), 2 * arrival::BROWSER_ROWS, "{count}");
            for (object, area) in &rows {
                let (x, y) = project(area.centre());
                assert!(x < WIDTH && y < HEIGHT, "{:?} is out of view", object.key);
                let lit = (0..40)
                    .flat_map(|dx| (0..12).map(move |dy| (dx, dy)))
                    .filter(|(dx, dy)| {
                        let (x, y) = project(object.label.anchor);
                        let pixel =
                            drawn.get_pixel((x + dx).min(WIDTH - 1), (y + dy).min(HEIGHT - 1));
                        pixel[0] > 150 && pixel[1] > 150
                    })
                    .count();
                assert!(lit > 3, "{count}: {:?}'s label is not drawn", object.key);
            }
            image::imageops::replace(&mut sheet, &drawn, 0, (index as u32 * HEIGHT).into());
        }
        if let Some(path) = std::env::var_os("TOMORI_SESSION_BROWSER_PREVIEW") {
            sheet.save(path).expect("the preview is saved");
        }
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}
