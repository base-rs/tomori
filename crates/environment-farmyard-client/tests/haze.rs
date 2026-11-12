// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tomori's own geometry beside the workbench, drawn by `tomori-render`'s renderer in the farmyard, fading with distance
//! as the bench does: by the scene's shared haze alone. See `design/rendering.md`.
use std::sync::Arc;

use glam::{Mat4, Quat, Vec2, Vec3};
use tomori_environment_farmyard_client::coop::Coop;
use tomori_environment_farmyard_client::layout;
use tomori_environment_farmyard_client::scenery::{Clock, Farmyard};
use tomori_render::graphics::Renderer;
use tomori_render::scenery::Sceneries;
use tomori_render::test_gpu;
use tomori_render::ui_panel::Canvas;
use tomori_scene::environment::Environment;
use tomori_scene::swarm::Swarm;
use tomori_scene::ui_text::{Row, Style};

const FARMYARD: Environment = Environment::new(tomori_environment_farmyard_client::ID)
    .with_version(tomori_environment_farmyard_client::VERSION);
const SIZE: (u32, u32) = (480, 360);
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// The distances, in metres, each is seen from.
const DISTANCES: [f32; 4] = [2.0, 5.0, 10.0, 20.0];

/// The linear luminance of an sRGB pixel, by the sRGB curve's power, whose linear toe, below a 25th, no pixel measured
/// here reaches but the night's.
fn luminance(pixel: &[u8]) -> f32 {
    let linear = |channel: u8| ((f32::from(channel) / 255.0 + 0.055) / 1.055).powf(2.4);
    0.2126 * linear(pixel[0]) + 0.7152 * linear(pixel[1]) + 0.0722 * linear(pixel[2])
}

/// What is drawn into `texture`, read back.
fn read_back(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> image::RgbaImage {
    let (width, height) = SIZE;
    let row = (width * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("haze test read back"),
        size: u64::from(row * height),
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
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = buffer.slice(..).get_mapped_range().unwrap().to_vec();
    let mut image = image::RgbaImage::new(width, height);
    for y in 0..height {
        let start = (y * row) as usize;
        image.as_mut()[(y * width * 4) as usize..((y + 1) * width * 4) as usize]
            .copy_from_slice(&data[start..start + (width * 4) as usize]);
    }
    image
}

/// Where each thing measured stands: the landing ring on the ground in front of the bench, the swarm, a controller and
/// a panel beside it, all about as far from the eye as the bench is.
struct Placed {
    ring: Vec3,
    swarm: Vec3,
    controller: Vec3,
    panel: Vec3,
    /// A point of the bench's front, which is sampled about.
    bench: Vec3,
    /// What the eye looks at, and which way it looks from.
    target: Vec3,
    towards: Vec3,
    across: Vec3,
}

fn placed() -> Placed {
    let on = |radius: f32, turn: f32, height: f32| {
        let point: Vec2 = layout::on_workbench(radius, turn);
        Vec3::new(point.x, height, point.y)
    };
    let ground = tomori_environment_farmyard::backdrop::ground(layout::on_workbench(1.1, 0.0));
    let ring = on(1.1, 0.0, ground);
    let middle = on(0.0, 0.0, ground);
    // From the bench out towards the arrival, a little down, so that the bench stands behind the ring.
    let out = (middle - ring).with_y(0.0).normalize();
    let towards = (-out + Vec3::Y * -0.36).normalize();
    let across = out.cross(Vec3::Y).normalize();
    Placed {
        ring,
        swarm: ring + across * 0.75 + Vec3::Y * 0.55,
        controller: ring - across * 0.55 + Vec3::Y * 0.35,
        panel: ring - across * 0.9 + Vec3::Y * 0.9 - out * 0.2,
        bench: on(
            layout::WORKBENCH_TOP[0] + 0.02,
            0.18,
            ground + layout::WORKBENCH_HEIGHT - 0.06,
        ),
        target: ring + Vec3::Y * 0.45 - out * 0.3,
        towards,
        across,
    }
}

/// The eye `distance` from what it looks at, seeing as much about it, 1.2 m up and down, from every distance, so that each
/// thing measured covers the same pixels from each.
fn camera(placed: &Placed, distance: f32) -> Mat4 {
    let eye = placed.target - placed.towards * distance;
    let fov = 2.0 * (1.2 / distance).atan();
    glam::camera::rh::proj::directx::perspective(fov, SIZE.0 as f32 / SIZE.1 as f32, 0.05, 50.0)
        * glam::camera::rh::view::look_at_mat4(eye, placed.target, Vec3::Y)
}

/// Where `point` is drawn by `camera`, in pixels.
fn pixel(camera: Mat4, point: Vec3) -> (u32, u32) {
    let clip = camera * point.extend(1.0);
    let ndc = clip.truncate() / clip.w;
    (
        ((ndc.x * 0.5 + 0.5) * SIZE.0 as f32) as u32,
        ((0.5 - ndc.y * 0.5) * SIZE.1 as f32) as u32,
    )
}

/// The mean luminance of `with`'s pixels that differ from `without`'s, and how many there are.
fn differing(with: &image::RgbaImage, without: &image::RgbaImage) -> (f32, usize) {
    let mut sum = 0.0;
    let mut count = 0;
    for (a, b) in with.pixels().zip(without.pixels()) {
        if a.0.iter().zip(b.0).any(|(a, b)| a.abs_diff(b) > 2) {
            sum += luminance(&a.0);
            count += 1;
        }
    }
    (sum / count.max(1) as f32, count)
}

/// The mean luminance of the 7 by 7 pixels about `at`.
fn about(image: &image::RgbaImage, (x, y): (u32, u32)) -> f32 {
    let mut sum = 0.0;
    for dy in 0..7 {
        for dx in 0..7 {
            sum += luminance(&image.get_pixel(x + dx - 3, y + dy - 3).0);
        }
    }
    sum / 49.0
}

/// The share of what the haze leaves of something's own light at each distance, against its light at 2 m: its
/// luminance less the haze's, over the same at 2 m.
fn kept(luminances: &[f32], haze: f32) -> Vec<f32> {
    luminances
        .iter()
        .map(|light| (light - haze) / (luminances[0] - haze))
        .collect()
}

#[test]
fn tomori_s_geometry_fades_with_distance_as_the_workbench_beside_it_does() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut sceneries = Sceneries::empty();
        tomori_environment_farmyard_client::register(&mut sceneries);
        // Its clock held still, so that its fire, embers and water are the same in each frame compared: running, they
        // moved between a frame and the one without the thing measured, and the few pixels they changed, counted as the
        // controller's of its hundred or so, moved its share kept at 5 m from 0.88 to 0.93 from one run to the next.
        sceneries.register_versioned(
            tomori_environment_farmyard_client::ID,
            tomori_environment_farmyard_client::VERSION,
            |target| {
                Box::new(Farmyard::with_clock(
                    target,
                    &Coop::Built,
                    tomori_assets::Library::shared(),
                    Clock::Held(0.0),
                ))
            },
        );
        let mut renderer =
            Renderer::with_sceneries(&device, &queue, FORMAT, SIZE.0, SIZE.1, 1, &sceneries);
        renderer.set_environment(FARMYARD);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("haze test eye"),
            size: wgpu::Extent3d {
                width: SIZE.0,
                height: SIZE.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let placed = placed();
        // The swarm, gathered beside the ring.
        let mut swarm = Swarm::default();
        let offset = placed.swarm - tomori_scene::swarm::CENTER;
        for agent in &mut swarm.agents {
            agent.position = tomori_scene::swarm::CENTER
                + (agent.position - tomori_scene::swarm::CENTER) * 0.6
                + offset;
        }
        let mut nobody = Swarm::default();
        nobody.agents.clear();
        // A panel facing the eye, a pale card with a line of text on it.
        let mut canvas = Canvas::new(12, 0.004);
        canvas.background = [0.55, 0.6, 0.66, 1.0];
        canvas.padding = (1, 1);
        canvas.rows.push(Row::text(
            "Workbench",
            Style::bold([0.05, 0.05, 0.08, 1.0]),
            12,
        ));
        let canvas = Arc::new(canvas);
        let mut panel = renderer.panel(&device, 2000);
        let facing = Quat::from_mat3(&glam::Mat3::from_cols(
            -placed.across,
            Vec3::Y,
            -placed.towards.with_y(0.0).normalize(),
        ));
        let model = Mat4::from_rotation_translation(facing, placed.panel);
        let controller = Some(tomori_render::controller::Pose {
            position: placed.controller.to_array(),
            orientation: Quat::from_rotation_y(0.6).to_array(),
        });
        let draw = |renderer: &mut Renderer,
                    panel: &tomori_render::ui_panel::Panel,
                    camera: Mat4,
                    shown: &Shown| {
            renderer.set_landing(shown.ring.then_some(placed.ring));
            renderer.update(
                &queue,
                if shown.swarm { &swarm } else { &nobody },
                None,
                false,
            );
            renderer.update_controllers(
                &queue,
                [controller.filter(|_| shown.controller), None],
                [None, None],
            );
            let panels: Vec<(&tomori_render::ui_panel::Panel, Mat4)> = if shown.panel {
                vec![(panel, model)]
            } else {
                Vec::new()
            };
            renderer.render(
                &device,
                &queue,
                std::slice::from_ref(&view),
                &[camera],
                Mat4::IDENTITY,
                &panels,
            );
            read_back(&device, &queue, &texture)
        };
        // Its glyphs reach the atlas over a frame or two.
        for _ in 0..3 {
            panel.show(&queue, &canvas);
            draw(&mut renderer, &panel, camera(&placed, 2.0), &Shown::ALL);
        }
        let mut sheet = image::RgbaImage::new(SIZE.0 * 4, SIZE.1);
        let mut light: [Vec<f32>; 5] = Default::default();
        let mut counts = Vec::new();
        for (index, &distance) in DISTANCES.iter().enumerate() {
            let camera = camera(&placed, distance);
            let all = draw(&mut renderer, &panel, camera, &Shown::ALL);
            let none = draw(&mut renderer, &panel, camera, &Shown::NONE);
            let measured = [
                Shown {
                    ring: true,
                    ..Shown::NONE
                },
                Shown {
                    swarm: true,
                    ..Shown::NONE
                },
                Shown {
                    controller: true,
                    ..Shown::NONE
                },
                Shown {
                    panel: true,
                    ..Shown::NONE
                },
            ]
            .map(|shown| differing(&draw(&mut renderer, &panel, camera, &shown), &none));
            for (slot, (mean, count)) in light.iter_mut().zip(measured) {
                counts.push((distance, count));
                slot.push(mean);
            }
            light[4].push(about(&none, pixel(camera, placed.bench)));
            image::imageops::replace(&mut sheet, &all, i64::from(index as u32 * SIZE.0), 0);
        }
        if let Some(path) = std::env::var_os("TOMORI_SHARED_HAZE_PREVIEW") {
            sheet.save(path).expect("the preview is saved");
        }
        // Each covers enough pixels from every distance to be measured.
        assert!(counts.iter().all(|(_, count)| *count > 50), "{counts:?}");
        let lighting = tomori_environment_farmyard_client::scenery::lighting();
        let haze = 0.2126 * lighting.haze.x + 0.7152 * lighting.haze.y + 0.0722 * lighting.haze.z;
        let names = ["ring", "swarm", "controller", "panel", "bench"];
        let kept: Vec<Vec<f32>> = light.iter().map(|light| kept(light, haze)).collect();
        for ((name, light), kept) in names.iter().zip(&light).zip(&kept) {
            println!("{name}: luminance {light:?}, kept {kept:?}");
        }
        // The bench keeps about three quarters of its own light at 20 m, as the haze, `1 - exp(-0.0008 d²)`, says.
        let bench = &kept[4];
        assert!((0.6..0.9).contains(&bench[3]), "{bench:?}");
        // Each thing beside it keeps as much, within 0.12, from every distance, where the ring and the swarm, by their
        // own fade, once kept a third at 10 m and none at 20 m. The most any differs, drawn the same each run, is the
        // controller's 0.088 at 20 m, its hundred or so pixels shaded a little differently as the eye draws back,
        // alike on lavapipe and a discrete GPU, within 0.003 of each other.
        for (name, kept) in names.iter().zip(&kept).take(4) {
            for (distance, (kept, bench)) in DISTANCES.iter().zip(kept.iter().zip(bench)) {
                assert!(
                    (kept - bench).abs() < 0.12,
                    "{name} at {distance} m keeps {kept}, the bench {bench}"
                );
            }
        }
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

/// Which of the things measured are drawn.
struct Shown {
    ring: bool,
    swarm: bool,
    controller: bool,
    panel: bool,
}

impl Shown {
    const ALL: Self = Self {
        ring: true,
        swarm: true,
        controller: true,
        panel: true,
    };
    const NONE: Self = Self {
        ring: false,
        swarm: false,
        controller: false,
        panel: false,
    };
}
