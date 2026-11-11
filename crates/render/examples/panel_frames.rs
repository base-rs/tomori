// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! How long a frame of projected panels takes on the CPU, headless: a projection at today's limits, the Tomori wall and
//! twelve walls of six slots, every slot full, and twelve panels floating, of text, code, tables, checklists, images and
//! charts; each frame the viewer's head bobs and the ray sweeps across the walls, as a participant looking about does.
//! It measures the frame step's projector, which lays the panels out, the uploads of their faces to the GPU, and the
//! renderer's encoding of both eyes' draws and their submission, without waiting for the GPU to draw them, on whatever
//! adapter the render tests use (set `VK_ICD_FILENAMES` to lavapipe's to use it). See `design/rendering.md`.
//!
//! ```text
//! cargo run --release -p tomori-render --features test-gpu --example panel_frames [frames] [panels]
//! ```
//!
//! With `panels`, only that many of them are projected, the walls still all shown, so that what each panel costs is
//! told from what the walls do.
use std::time::{Duration, Instant};

use glam::{Vec2, Vec3};
use tomori_protocol::{
    Code, Image, Meta, Panel, Place, Pose, Projection, Span, TOMORI_WALL, TokenKind, WALL_SLOTS,
    Wall, WallKind, WallShape,
};
use tomori_render::{graphics::Renderer, scene_graphics::Panels};
use tomori_scene::{
    arrival::HOME,
    pointing::Ray,
    projection::{Controls, Input, Projector, editing::Rights},
};

const TEXT: &str = "## Release checklist\n\nThe **workspace** builds on every platform, and the headset's APK is signed with the release key. Each crate's version moves together, with one `releases.md` and one tag.\n\n- Bump the version.\n- Update `releases.md`.\n- Tag the release, and push the tag.\n- Attach the APK to the GitHub Release.\n\n> Every step is reviewed before it is merged, and the headset is checked by hand.\n\nThe last paragraph wraps across several rows so that the panel has more than it shows at once, and scrolls: it goes on, and on, about what the release does and why, until it is longer than a slot.";

const TABLE: &str = "| Crate | Tests | Coverage | Notes |\n|---|---|---|---|\n| scene | 1,204 | 100% | the frame step |\n| render | 388 | 100% | lavapipe |\n| protocol | 412 | 100% | serde |\n| server | 902 | 100% | the hub, and every backend it drives |\n| text | 96 | 100% | the atlas |\n| client | 210 | 100% | connection |\n";

const CHECKLIST: &str = "- [x] Draft the design\n- [x] Review it with the maintainer\n- [ ] Lay the panels out once\n- [ ] Keep their meshes on the GPU\n- [ ] Measure the headset\n- [ ] Recommend a cap\n- [ ] Open the pull request\n";

fn code() -> (String, Code) {
    let body: String = (0..40)
        .map(|line| {
            format!(
                "    let value_{line} = compute(&input, {line}).unwrap_or_default(); // step {line}\n"
            )
        })
        .collect();
    let body = body.trim_end().to_owned();
    // `let` and the number in each line, highlighted as the server would.
    let mut spans = Vec::new();
    let mut at = 0u32;
    for line in body.split('\n') {
        spans.push(Span::new(at + 4, 3, TokenKind::Keyword));
        spans.push(Span::new(at + 8, 5, TokenKind::Variable));
        at += line.chars().count() as u32 + 1;
    }
    (body, Code::new("rust", 40).with_spans(spans))
}

fn panel(index: usize, place: Place) -> Panel {
    let id = format!("panel-{index}");
    let meta = vec![Meta::new("source", "CI")];
    let panel = match index % 6 {
        0 => Panel::new(&id, "Release", TEXT, place),
        1 => {
            let (body, code) = code();
            Panel::new(&id, "compute.rs", body, place).with_code(code)
        }
        2 => Panel::new(&id, "Tests", TABLE, place),
        3 => Panel::new(&id, "Agenda", CHECKLIST, place),
        4 => Panel::new(&id, "Diagram", "A diagram of the frame step.", place)
            .with_image(Image::new("0123456789abcdef".repeat(4), 800, 600, 120_000)),
        _ => {
            let chart = tomori_scene::chart::fixtures::pressure();
            Panel::new(&id, "Scaling pressure", chart.summary(), place).with_chart(chart)
        }
    };
    panel.with_meta(meta).with_age(300).with_version(1)
}

/// The Tomori wall and twelve walls of six slots about home, every slot full, and twelve panels floating before them.
fn projection() -> Projection {
    let mut walls = vec![
        Wall::new(
            TOMORI_WALL,
            WallKind::Tomori,
            Pose::upright_facing([0.0, 0.52, -2.3], [0.0, 0.52, 0.0]),
        )
        .with_shape(WallShape::Arc { radius: 2.3 }),
    ];
    for index in 0..12 {
        let angle = (30.0 + 25.0 * index as f32).to_radians();
        let at = [4.0 * angle.sin(), 1.5, -4.0 * angle.cos()];
        walls.push(Wall::new(
            format!("wall-{index}"),
            WallKind::Added,
            Pose::upright_facing(at, [0.0, 1.5, 0.0]),
        ));
    }
    let mut panels = Vec::new();
    for wall in &walls {
        for slot in 0..WALL_SLOTS {
            let place = Place::Wall {
                wall: wall.id.clone(),
                slot,
            };
            panels.push(panel(panels.len(), place));
        }
    }
    for index in 0..12 {
        let angle = (-40.0 + 7.0 * index as f32).to_radians();
        let at = [1.6 * angle.sin(), 1.4, -1.6 * angle.cos()];
        let place = Place::Free {
            pose: Pose::upright_facing(at, HOME.to_array()),
        };
        panels.push(panel(panels.len(), place));
    }
    Projection::new("space", "sky")
        .with_walls(walls)
        .with_panels(panels)
}

/// The mean and the 95th percentile of `times`, in milliseconds.
fn summary(times: &mut [Duration]) -> (f64, f64) {
    times.sort();
    let mean = times.iter().map(Duration::as_secs_f64).sum::<f64>() / times.len() as f64;
    let p95 = times[times.len() * 95 / 100].as_secs_f64();
    (mean * 1e3, p95 * 1e3)
}

fn main() {
    let frames: usize = std::env::args()
        .nth(1)
        .and_then(|frames| frames.parse().ok())
        .unwrap_or(600);
    let shown: usize = std::env::args()
        .nth(2)
        .and_then(|panels| panels.parse().ok())
        .unwrap_or(usize::MAX);
    let Some(gpu) = tomori_render::test_gpu::open() else {
        eprintln!("No GPU adapter: install lavapipe, or another Vulkan driver.");
        std::process::exit(1);
    };
    let (device, queue) = (&gpu.device, &gpu.queue);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let (width, height) = (256, 256);
    let renderer = Renderer::new(device, queue, format, width, height);
    let eyes: Vec<wgpu::TextureView> = (0..2)
        .map(|_| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("eye"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        })
        .collect();
    let mut panels = Panels::new(device, &renderer);
    let mut projection = projection();
    projection.panels.truncate(shown);
    let mut projector = Projector::default();
    let start = Instant::now();
    let (mut scene, mut upload, mut render) = (Vec::new(), Vec::new(), Vec::new());
    let mut sheets = 0;
    for frame in 0..frames + 60 {
        let time = frame as f32 / 72.0;
        // The head bobs a centimetre or two, and the ray sweeps slowly across the walls and the panels floating.
        let viewer = HOME + Vec3::new(0.02 * (time * 1.3).sin(), 0.01 * (time * 2.1).sin(), 0.0);
        let sweep = 1.2 * (time * 0.4).sin();
        let ray = Ray::new(
            viewer - Vec3::Y * 0.3,
            Vec3::new(sweep.sin(), -0.05, -sweep.cos()),
        );
        let input = Input {
            projection: Some(&projection),
            ray,
            grip: Some(0.0),
            viewer: Some(viewer),
            active: true,
            pointing: true,
            nearer: f32::INFINITY,
            stick: Vec2::ZERO,
            straighten: false,
            free: false,
            controls: Controls::Controllers,
            scroll: None,
            rights: Rights::default(),
            left: tomori_scene::projection::Hold::default(),
            now: start + Duration::from_secs_f32(time),
            dt: 1.0 / 72.0,
        };
        let began = Instant::now();
        let output = projector.update(input);
        let laid = Instant::now();
        panels.set_cards(device, queue, &renderer, &output.sheets);
        let uploaded = Instant::now();
        // Both eyes, a few centimetres apart, looking ahead.
        let matrices = [-0.032f32, 0.032].map(|side| {
            let eye = viewer + Vec3::X * side;
            glam::camera::rh::proj::directx::perspective(1.6, 1.0, 0.05, 50.0)
                * glam::camera::rh::view::look_at_mat4(eye, eye - Vec3::Z, Vec3::Y)
        });
        renderer.render(
            device,
            queue,
            &eyes,
            &matrices,
            glam::Mat4::IDENTITY,
            &panels.list(glam::Mat4::IDENTITY),
        );
        let rendered = Instant::now();
        // The GPU draws while the next frame is stepped, as the headset's does; here it is waited for, untimed, so that
        // frames do not queue up.
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        // The first frames build every face's mesh and wait on the atlas's glyphs: not what a frame costs once shown.
        if frame >= 60 {
            scene.push(laid - began);
            upload.push(uploaded - laid);
            render.push(rendered - uploaded);
        }
        sheets = output.sheets.len();
    }
    let mut total: Vec<Duration> = scene.iter().zip(&upload).map(|(a, b)| *a + *b).collect();
    let (scene, upload, render) = (
        summary(&mut scene),
        summary(&mut upload),
        summary(&mut render),
    );
    let total = summary(&mut total);
    let count = projection.panels.len().max(1) as f64;
    println!(
        "{} panels, {sheets} sheets, {frames} frames: projector {:.3} ms (p95 {:.3}), uploads {:.3} ms (p95 {:.3}), together {:.3} ms (p95 {:.3}), {:.4} ms a panel; drawing both eyes {:.3} ms (p95 {:.3}), {:.4} ms a sheet",
        projection.panels.len(),
        scene.0,
        scene.1,
        upload.0,
        upload.1,
        total.0,
        total.1,
        total.0 / count,
        render.0,
        render.1,
        render.0 / sheets as f64,
    );
}
