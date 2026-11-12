// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The farmyard drawn on the shared test device, on lavapipe in CI: from home, looking into the yard at the coop, as a
//! participant arriving sees it, and growing out of a point. `TOMORI_FARMYARD_PREVIEW`, set to a PNG path, saves the
//! view from home, from beside the coop's window and of the workbench from the yard before it there, and the chickens
//! beside it, with `-chickens` added to its name; `TOMORI_FARMYARD_COOP_PREVIEW` saves the coop by its ramp and in at its
//! window, with hens on the ramp and in a nest, as `-1.png`, and the path's stones, as `-2.png`;
//! `TOMORI_FARMYARD_CAMPSITE_PREVIEW` saves the campsite from the lookout's log seat, a marshmallow toasting over its
//! bonfire; and `TOMORI_FARMYARD_BEACH_PREVIEW` a little dolmen of the beach's building stones on its shingle.
use glam::{Mat4, Vec3};
use tomori_render::test_gpu;

use super::*;
use tomori_environment_farmyard::trail;

mod culling;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 200;

/// What `scenery` draws for an eye at `eye` looking at `target`, read back as RGBA rows.
pub(crate) fn capture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scenery: &dyn Scenery,
    samples: u32,
    view: (Vec3, Vec3),
) -> Vec<u8> {
    let projection = glam::camera::rh::proj::directx::perspective(
        1.2,
        WIDTH as f32 / HEIGHT as f32,
        0.05,
        200.0,
    );
    capture_sized(
        device,
        queue,
        scenery,
        samples,
        view,
        (WIDTH, HEIGHT, projection),
    )
}

/// As [`capture`], `width` by `height` through `projection`.
fn capture_sized(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scenery: &dyn Scenery,
    samples: u32,
    (eye, target): (Vec3, Vec3),
    (width, height, projection): (u32, u32, Mat4),
) -> Vec<u8> {
    let view_projection = projection * glam::camera::rh::view::look_at_mat4(eye, target, Vec3::Y);
    capture_clip(
        device,
        queue,
        scenery,
        samples,
        view_projection,
        (width, height),
    )
}

/// What `scenery` draws through `view_projection`, `width` by `height`, read back as RGBA rows.
fn capture_clip(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scenery: &dyn Scenery,
    samples: u32,
    view_projection: Mat4,
    (width, height): (u32, u32),
) -> Vec<u8> {
    let texture = |format, samples, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Farmyard test target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let resolved = texture(
        wgpu::TextureFormat::Rgba8UnormSrgb,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let color = texture(
        wgpu::TextureFormat::Rgba8UnormSrgb,
        samples,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let depth = texture(
        wgpu::TextureFormat::Depth32Float,
        samples,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let (resolved_view, color_view, depth_view) = (
        resolved.create_view(&Default::default()),
        color.create_view(&Default::default()),
        depth.create_view(&Default::default()),
    );
    scenery.prepare(queue, 0, view_projection);
    let mut encoder = device.create_command_encoder(&Default::default());
    scenery.encode(&mut encoder, 0);
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Farmyard test pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: if samples > 1 {
                    &color_view
                } else {
                    &resolved_view
                },
                depth_slice: None,
                resolve_target: (samples > 1).then_some(&resolved_view),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        scenery.draw(&mut pass, 0);
        scenery.draw_translucent(&mut pass, 0);
    }
    let stride = (width * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Farmyard test readback"),
        size: u64::from(stride * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        resolved.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
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
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap();
        });
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    receiver.recv().unwrap().unwrap();
    let data = buffer.slice(..).get_mapped_range().unwrap();
    let pixels = data
        .chunks(stride as usize)
        .flat_map(|row| &row[..(width * 4) as usize])
        .copied()
        .collect();
    drop(data);
    buffer.unmap();
    pixels
}

/// The average colour of the pixels in the box from `low` to `high`, fractions of the view across and down.
fn average(pixels: &[u8], low: (f32, f32), high: (f32, f32)) -> [f32; 3] {
    let mut sum = [0.0; 3];
    let mut count = 0.0;
    for y in (low.1 * HEIGHT as f32) as u32..(high.1 * HEIGHT as f32) as u32 {
        for x in (low.0 * WIDTH as f32) as u32..(high.0 * WIDTH as f32) as u32 {
            let at = ((y * WIDTH + x) * 4) as usize;
            for channel in 0..3 {
                sum[channel] += f32::from(pixels[at + channel]);
            }
            count += 1.0;
        }
    }
    sum.map(|value| value / count)
}

fn target(device: &wgpu::Device, queue: &wgpu::Queue, samples: u32) -> Farmyard {
    Farmyard::with_clock(
        &Target::new(
            device,
            queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            WIDTH,
            HEIGHT,
            samples,
            tomori_render::glyphs::SharedGlyphs::new(device),
        ),
        &Coop::Built,
        Library::shared(),
        Clock::Held(0.0),
    )
}

const HOME: (Vec3, Vec3) = (Vec3::new(0.0, 1.6, 0.0), Vec3::new(-1.2, 0.8, -8.0));
const WINDOW: (Vec3, Vec3) = (Vec3::new(-0.9, 1.25, -8.6), Vec3::new(-2.8, 0.85, -8.6));

/// From the yard 3 m before the workbench's middle, looking into its corner at the middle of its top.
fn workbench() -> (Vec3, Vec3) {
    let at = |point: glam::Vec2, y: f32| Vec3::new(point.x, y, point.y);
    let middle = layout::on_workbench(
        (layout::WORKBENCH_TOP[0] + layout::WORKBENCH_TOP[1]) * 0.5,
        0.0,
    );
    (
        at(layout::on_workbench(-1.0, 0.0), 1.6),
        at(middle, layout::WORKBENCH_HEIGHT + 0.2),
    )
}

#[test]
fn every_shader_is_valid_wgsl() {
    let files = test_gpu::wgsl_files(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
    let names: Vec<_> = files.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["fern.wgsl", "fire.wgsl", "sky.wgsl", "water.wgsl"]);
    for (name, source) in files {
        test_gpu::validate_wgsl(&name, &source);
    }
}

#[test]
fn the_eye_is_where_every_ray_of_its_view_begins() {
    let view = glam::camera::rh::proj::directx::perspective(1.0, 1.0, 0.1, 10.0)
        * glam::camera::rh::view::look_at_mat4(Vec3::new(1.0, 2.0, 3.0), Vec3::ZERO, Vec3::Y);
    assert!(eye(view).distance(Vec3::new(1.0, 2.0, 3.0)) < 1e-3);
    // A view without a point of view, such as an orthographic one, is taken as from the origin.
    assert_eq!(eye(Mat4::IDENTITY), Vec3::ZERO);
}

#[test]
fn the_farmyard_is_an_evening_yard_with_a_warmly_lit_red_coop() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut farmyard = target(&device, &queue, 4);
        let home = capture(&device, &queue, &farmyard, 4, HOME);
        // The sky is darker overhead than at the horizon, the ground below is the yard's, and the coop, ahead and left of
        // the gate, is red, lit warmly by its lantern.
        let overhead = average(&home, (0.0, 0.0), (1.0, 0.08));
        let ground = average(&home, (0.3, 0.9), (0.7, 1.0));
        assert!(
            overhead.iter().sum::<f32>() < ground.iter().sum::<f32>(),
            "{overhead:?} {ground:?}"
        );
        let coop = average(&home, (0.35, 0.3), (0.5, 0.55));
        assert!(coop[0] > coop[2] * 1.2, "the coop is red: {coop:?}");
        // Seen from beside its window, its inside is darker than its sunlit, lantern-lit walls.
        let window = capture(&device, &queue, &farmyard, 4, WINDOW);
        // The workbench in the corner, seen from the yard before it, is wood lit warmly by its lantern, against the
        // evening beyond the fence.
        let workbench = capture(&device, &queue, &farmyard, 4, workbench());
        let bench = average(&workbench, (0.4, 0.55), (0.6, 0.7));
        assert!(bench[0] > bench[2] * 1.2, "the bench is wood: {bench:?}");
        let rack = average(&workbench, (0.45, 0.3), (0.55, 0.45));
        assert!(rack[0] > rack[2], "the rack is wood: {rack:?}");
        if let Ok(path) = std::env::var("TOMORI_FARMYARD_PREVIEW") {
            let mut sheet = image::RgbaImage::new(WIDTH, HEIGHT * 3);
            for (row, pixels) in [&home, &window, &workbench].into_iter().enumerate() {
                let view = image::RgbaImage::from_raw(WIDTH, HEIGHT, pixels.clone()).unwrap();
                image::imageops::replace(&mut sheet, &view, 0, i64::from(row as u32 * HEIGHT));
            }
            sheet.save(path).unwrap();
        }
        // Growing out of a point in front of home, only what is near it is drawn, and the rest is left as it was.
        farmyard.set_reveal(Some((Vec3::new(0.0, 1.0, -2.0), 0.5)));
        let growing = capture(&device, &queue, &farmyard, 4, HOME);
        let black = growing
            .chunks(4)
            .filter(|pixel| pixel[..3] == [0, 0, 0])
            .count();
        assert!(black > (WIDTH * HEIGHT) as usize / 2, "{black}");
        farmyard.set_reveal(Some((Vec3::NAN, 1.0)));
        assert!(farmyard.reveal.is_none());
        assert_eq!(capture(&device, &queue, &farmyard, 4, HOME), home);
        // A view that cannot be inverted draws the sky as if from the origin, rather than failing.
        farmyard.prepare(&queue, 1, Mat4::ZERO);
        // Registered, the build's sceneries make it for their targets.
        let mut sceneries = tomori_render::scenery::Sceneries::empty();
        crate::registered(&mut sceneries, Coop::Built, Clock::Held(0.0));
        let made = sceneries.create(&Target::new(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            WIDTH,
            HEIGHT,
            4,
            tomori_render::glyphs::SharedGlyphs::new(&device),
        ));
        assert_eq!(made.len(), 1);
        assert_eq!(capture(&device, &queue, made[0].1.as_ref(), 4, HOME), home);
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

#[test]
fn the_chickens_are_drawn_where_their_client_side_shows_them() {
    use tomori_scene::client_side::{ClientSide, Input};
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut farmyard = target(&device, &queue, 4);
        // The flock of a space, as its server side shares it, a few seconds on.
        let chicken = tomori_environment_farmyard::chicken::Chicken::shared().unwrap();
        let flock = tomori_environment_farmyard::flock::Flock::new(
            tomori_environment_farmyard::flock::seed("space-1"),
            chicken.clone(),
            0,
        );
        let state = tomori_protocol::Payload::new(serde_json::to_value(flock.state(0)).unwrap());
        let epoch = std::time::Instant::now();
        let mut client = crate::client::FarmyardClient::new("space-1", Some(chicken), epoch);
        let output = client.update(Input::new(
            vec![tomori_scene::channel::Update::State(state)],
            epoch + std::time::Duration::from_millis(400),
        ));
        let drawn = output.drawn.unwrap();
        let at = glam::Vec3::from(flock.hens()[0].behaviour.at);
        let close = (
            at + Vec3::new(0.0, 0.45, 1.1),
            at + Vec3::new(0.0, 0.18, 0.0),
        );
        // Animated once, as every frame is, so that what else it shows, such as the stations' plaques, is shown already.
        farmyard.animate(&device, &queue, &());
        let without = capture(&device, &queue, &farmyard, 4, close);
        // What is not the farmyard's, or shows no chickens, changes nothing.
        farmyard.animate(&device, &queue, &42u32);
        farmyard.animate(
            &device,
            &queue,
            &crate::client::Drawn {
                stones: Vec::new(),
                chickens: Vec::new(),
                sticks: Vec::new(),
            },
        );
        assert_eq!(capture(&device, &queue, &farmyard, 4, close), without);
        // Its chickens are drawn, a chicken filling the middle of a close view of it; again, from the same upload.
        farmyard.animate(&device, &queue, drawn.as_ref());
        farmyard.animate(&device, &queue, drawn.as_ref());
        let with = capture(&device, &queue, &farmyard, 4, close);
        // Pixels the chickens change, by a clear difference.
        let changed = |a: &[u8], b: &[u8]| {
            a.chunks(4)
                .zip(b.chunks(4))
                .filter(|(a, b)| {
                    a[..3]
                        .iter()
                        .zip(&b[..3])
                        .map(|(a, b)| a.abs_diff(*b) as u32)
                        .sum::<u32>()
                        > 30
                })
                .count()
        };
        let home = capture(&device, &queue, &farmyard, 4, HOME);
        if let Ok(path) = std::env::var("TOMORI_FARMYARD_PREVIEW") {
            let mut sheet = image::RgbaImage::new(WIDTH, HEIGHT * 3);
            for (row, pixels) in [&home, &with, &without].into_iter().enumerate() {
                let view = image::RgbaImage::from_raw(WIDTH, HEIGHT, pixels.clone()).unwrap();
                image::imageops::replace(&mut sheet, &view, 0, i64::from(row as u32 * HEIGHT));
            }
            sheet.save(path.replace(".png", "-chickens.png")).unwrap();
        }
        assert!(
            changed(&with, &without) > 500,
            "{}",
            changed(&with, &without)
        );
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

/// The backdrop's pictures: wide, through the headset's 50 m far plane.
const PICTURE: (u32, u32) = (1280, 800);

fn picture() -> (u32, u32, Mat4) {
    let (width, height) = PICTURE;
    (
        width,
        height,
        glam::camera::rh::proj::directx::perspective(1.3, width as f32 / height as f32, 0.05, 50.0),
    )
}

/// Where `point` is drawn in a picture from `view`, as fractions across and down.
fn seen_at((eye, target): (Vec3, Vec3), point: Vec3) -> (f32, f32) {
    let clip = picture().2
        * glam::camera::rh::view::look_at_mat4(eye, target, Vec3::Y)
        * point.extend(1.0);
    (clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5)
}

/// The average colour of a picture's pixels about `at`, fractions across and down.
fn about(pixels: &[u8], at: (f32, f32)) -> f32 {
    let (width, height) = PICTURE;
    let (x, y) = ((at.0 * width as f32) as i32, (at.1 * height as f32) as i32);
    let mut sum = 0.0;
    for dy in -2..=2 {
        for dx in -2..=2 {
            let index = (((y + dy) as u32 * width + (x + dx) as u32) * 4) as usize;
            sum += pixels[index..index + 3]
                .iter()
                .map(|value| f32::from(*value))
                .sum::<f32>();
        }
    }
    sum / 25.0
}

/// From where participants arrive, looking at the coop, the waterfall over the back fence to its right; and from the
/// workbench's curve, looking out over the yard to the river beyond its right side.
fn backdrop_views() -> [(Vec3, Vec3); 2] {
    let eyes = layout::ARRIVAL + Vec3::Y * 1.6;
    let bench = Vec3::new(layout::WORKBENCH.x, 1.6, layout::WORKBENCH.y);
    let out = Vec3::new(
        layout::ARRIVAL.x - bench.x,
        0.0,
        layout::ARRIVAL.z - bench.z,
    )
    .normalize();
    [
        (
            eyes,
            Vec3::new(layout::ARRIVAL_FACING.x, 2.2, layout::ARRIVAL_FACING.z),
        ),
        (bench, bench + out * 10.0 + Vec3::Y * 0.4),
    ]
}

#[test]
fn the_backdrop_is_seen_from_the_arrival_and_the_workbench_its_water_moving() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let target = Target::new(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            PICTURE.0,
            PICTURE.1,
            4,
            tomori_render::glyphs::SharedGlyphs::new(&device),
        );
        let made = |clock| Farmyard::with_clock(&target, &Coop::Built, Library::shared(), clock);
        let farmyard = made(Clock::Held(0.0));
        let [arrival, workbench] = backdrop_views();
        let from_arrival = capture_sized(&device, &queue, &farmyard, 4, arrival, picture());
        let from_workbench = capture_sized(&device, &queue, &farmyard, 4, workbench, picture());
        if let Ok(path) = std::env::var("TOMORI_FARMYARD_BACKDROP_PREVIEW") {
            for (index, pixels) in [&from_arrival, &from_workbench].into_iter().enumerate() {
                let picture =
                    image::RgbaImage::from_raw(PICTURE.0, PICTURE.1, pixels.clone()).unwrap();
                picture
                    .save(path.replace(".png", &format!("-{}.png", index + 1)))
                    .unwrap();
            }
        }
        // From the arrival, the waterfall falls brighter than the cliff and the bush either side of it.
        let fall = seen_at(arrival, layout::WATERFALL);
        assert!(
            (0.5..0.95).contains(&fall.0) && (0.2..0.6).contains(&fall.1),
            "{fall:?}"
        );
        let across = Vec3::new(4.0, 0.0, 0.0);
        let beside = [
            seen_at(arrival, layout::WATERFALL - across),
            seen_at(arrival, layout::WATERFALL + across),
        ];
        let falling = about(&from_arrival, fall);
        for side in beside {
            assert!(
                falling > about(&from_arrival, side) * 1.5,
                "{falling} {side:?}"
            );
        }
        // From the workbench, the river beyond the fence is drawn, darker water between paler shingle and grass.
        let river = seen_at(workbench, layout::RIVER[7]);
        assert!(
            (0.0..1.0).contains(&river.0) && (0.5..1.0).contains(&river.1),
            "{river:?}"
        );
        // The water moves: a moment later, the waterfall's streaks have fallen on.
        let later = made(Clock::Held(0.6));
        let moved = capture_sized(&device, &queue, &later, 4, arrival, picture());
        assert_ne!(about(&moved, fall), falling);
        // Moving with the time, as the farmyard is made for a client, it is drawn too.
        let running = Farmyard::new(&target, &Coop::Built, Library::shared());
        assert_eq!(running.clock, Clock::Running);
        capture_sized(&device, &queue, &running, 4, arrival, picture());
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

/// Beside the coop's ramp, looking up it at the door, its lantern and the roof's overhang; in at the coop's window, at
/// the nest boxes and the perch; and from inside the gate along the path's stones.
fn coop_views() -> [(Vec3, Vec3); 3] {
    [
        (Vec3::new(-1.55, 1.15, -5.5), Vec3::new(-2.8, 0.75, -7.7)),
        (Vec3::new(-1.4, 1.2, -8.45), Vec3::new(-3.0, 0.75, -8.85)),
        (Vec3::new(0.4, 1.3, 1.1), Vec3::new(-1.4, 0.0, -3.0)),
    ]
}

#[test]
fn the_coop_s_ramp_nests_and_perch_and_the_path_s_stones_are_drawn() {
    use tomori_environment_farmyard::behaviour::{Behaviour, Doing};
    use tomori_environment_farmyard::controller::ChickenController;
    use tomori_environment_farmyard::flock::{Hen, ROOSTS};
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let target = Target::new(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            PICTURE.0,
            PICTURE.1,
            4,
            tomori_render::glyphs::SharedGlyphs::new(&device),
        );
        let mut farmyard =
            Farmyard::with_clock(&target, &Coop::Built, Library::shared(), Clock::Held(0.0));
        // A hen halfway up the ramp, one on the second nest's bottom, as one goes in, and one on a stone of the path.
        let chicken = tomori_environment_farmyard::chicken::Chicken::shared().unwrap();
        let door = layout::DOOR - Vec3::Z * 0.15;
        let stone = layout::stones()[5].middle();
        let hens = [
            (
                layout::RAMP_FOOT,
                std::f32::consts::PI,
                Doing::Walk {
                    to: door.to_array(),
                },
                false,
            ),
            (
                Vec3::from(ROOSTS[1]),
                std::f32::consts::PI,
                Doing::Roost,
                true,
            ),
            (
                Vec3::new(stone.x, layout::underfoot(stone), stone.y),
                2.0,
                Doing::Stand,
                false,
            ),
        ];
        let walk = Behaviour {
            since: 0,
            at: layout::RAMP_FOOT.to_array(),
            heading: std::f32::consts::PI,
            doing: hens[0].2,
        };
        let halfway = walk.walk(&chicken, 1.0).unwrap();
        let now = (halfway.walks + halfway.stops) / 2;
        let chickens = hens
            .iter()
            .map(|(at, heading, doing, cone)| {
                let hen = Hen {
                    name: "Pip".into(),
                    cone: *cone,
                    pace: 1.0,
                    behaviour: Behaviour {
                        since: 0,
                        at: at.to_array(),
                        heading: *heading,
                        doing: *doing,
                    },
                };
                let mut controller = ChickenController::new(chicken.clone(), &hen);
                controller.update(now);
                controller.instance().clone()
            })
            .collect();
        farmyard.animate(
            &device,
            &queue,
            &crate::client::Drawn {
                stones: Vec::new(),
                chickens,
                sticks: Vec::new(),
            },
        );
        let [ramp, window, path] =
            coop_views().map(|view| capture_sized(&device, &queue, &farmyard, 4, view, picture()));
        if let Ok(path_to) = std::env::var("TOMORI_FARMYARD_COOP_PREVIEW") {
            let (width, height) = PICTURE;
            let mut sheet = image::RgbaImage::new(width, height * 2);
            for (row, pixels) in [&ramp, &window].into_iter().enumerate() {
                let view = image::RgbaImage::from_raw(width, height, pixels.clone()).unwrap();
                image::imageops::replace(&mut sheet, &view, 0, i64::from(row as u32 * height));
            }
            sheet.save(path_to.replace(".png", "-1.png")).unwrap();
            image::RgbaImage::from_raw(width, height, path.clone())
                .unwrap()
                .save(path_to.replace(".png", "-2.png"))
                .unwrap();
        }
        // The path's stones stand out paler than the earth between them.
        let view = coop_views()[2];
        let stones = layout::stones();
        for pair in stones[2..6].windows(2) {
            let (a, b) = (pair[0].middle(), pair[1].middle());
            let on = seen_at(view, Vec3::new(a.x, pair[0].height, a.y));
            let between = seen_at(view, Vec3::new((a.x + b.x) * 0.5, 0.0, (a.y + b.y) * 0.5));
            assert!(
                about(&path, on) > about(&path, between) * 1.1,
                "{} {}",
                about(&path, on),
                about(&path, between)
            );
        }
        // The door, up the ramp, is lit warmly by the lantern beside it.
        let lit = seen_at(coop_views()[0], layout::DOOR + Vec3::new(0.25, 0.2, 0.05));
        assert!(about(&ramp, lit) > 120.0, "{}", about(&ramp, lit));
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

#[test]
fn a_running_clock_says_the_time_since_the_scenery_was_made_and_a_held_one_its_moment() {
    let made = std::time::Instant::now();
    assert!(Clock::Running.seconds(made) < 60.0);
    assert_eq!(Clock::Held(2.5).seconds(made), 2.5);
}

/// Where the trail is seen from, as `(eye, target)`: along it on the yard's right side, by the river, towards the second
/// station; outside the gate, looking back at it where the trail leaves the yard, the first station to the right; from
/// behind the coop towards the lookout over the pool, its seat and the fourth station; and the second station's
/// plaque, from its clearing.
pub(crate) fn trail_views() -> [(Vec3, Vec3); 4] {
    let at = |point: glam::Vec2, y: f32| Vec3::new(point.x, y, point.y);
    let (riverbank, _, _) = trail::frame(11);
    let (ahead, _, outward) = trail::frame(17);
    let (behind, _, _) = trail::frame(31);
    let (lookout, _, out) = trail::frame(37);
    let station = trail::STATIONS[1];
    [
        (at(riverbank, 1.6), at(ahead + outward * 2.5, 0.6)),
        (Vec3::new(2.4, 1.6, 7.6), Vec3::new(0.0, 0.5, 1.6)),
        (at(behind, 1.6), at(lookout + out * 3.0, 0.5)),
        (
            at(station.clearing() + station.faces() * 0.4, 1.55),
            at(station.post(), trail::PLAQUE_HEIGHT),
        ),
    ]
}

#[test]
fn the_trail_is_drawn_round_the_fence_by_the_river_its_plaques_read() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        // An atlas this test syncs, as a renderer syncs its own, so that the plaques' text is drawn.
        let glyphs = Arc::new(tomori_render::glyphs::Glyphs::new(&device));
        let target = Target::new(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            PICTURE.0,
            PICTURE.1,
            4,
            glyphs.clone().into(),
        );
        let mut farmyard =
            Farmyard::with_clock(&target, &Coop::Built, Library::shared(), Clock::Held(0.0));
        for _ in 0..30 {
            farmyard.animate(&device, &queue, &());
            glyphs.sync(&device, &queue);
        }
        let pictures: Vec<Vec<u8>> = trail_views()
            .into_iter()
            .map(|view| capture_sized(&device, &queue, &farmyard, 4, view, picture()))
            .collect();
        if let Ok(path) = std::env::var("TOMORI_FARMYARD_TRAIL_PREVIEW") {
            for (index, pixels) in pictures.iter().enumerate() {
                let picture =
                    image::RgbaImage::from_raw(PICTURE.0, PICTURE.1, pixels.clone()).unwrap();
                picture
                    .save(path.replace(".png", &format!("-{}.png", index + 1)))
                    .unwrap();
            }
        }
        let views = trail_views();
        // Along the trail by the river, its track is drawn ahead, earth, redder than the meadow beside it, which is
        // greener.
        let colour = |pixels: &[u8], at: (f32, f32)| {
            assert!(
                (0.0..1.0).contains(&at.0) && (0.0..1.0).contains(&at.1),
                "{at:?}"
            );
            let (x, y) = (
                (at.0 * PICTURE.0 as f32) as u32,
                (at.1 * PICTURE.1 as f32) as u32,
            );
            let index = ((y * PICTURE.0 + x) * 4) as usize;
            [pixels[index], pixels[index + 1], pixels[index + 2]]
        };
        // The first point of the trail ahead that is well within the picture.
        let (point, _, outward) = (12..20)
            .map(trail::frame)
            .find(|(point, _, _)| {
                let at = seen_at(views[0], Vec3::new(point.x, trail::LIFT, point.y));
                (0.2..0.8).contains(&at.0) && (0.5..0.95).contains(&at.1)
            })
            .unwrap();
        let track = colour(
            &pictures[0],
            seen_at(views[0], Vec3::new(point.x, trail::LIFT, point.y)),
        );
        assert!(track[0] > track[1], "{track:?}");
        let beside = point + outward * 1.5;
        let meadow = colour(
            &pictures[0],
            seen_at(views[0], Vec3::new(beside.x, 0.0, beside.y)),
        );
        assert!(meadow[1] > meadow[0], "{meadow:?}");
        // From its clearing, a station's plaque reads: pale letters on its dark board.
        let station = trail::STATIONS[1];
        let post = station.post();
        let middle = seen_at(views[3], Vec3::new(post.x, trail::PLAQUE_HEIGHT, post.y));
        let letters = |pixels: &[u8]| {
            let (mut pale, mut dark) = (0, 0);
            for dy in -40..40 {
                for dx in -120..120 {
                    let x = (middle.0 * PICTURE.0 as f32) as i32 + dx;
                    let y = (middle.1 * PICTURE.1 as f32) as i32 + dy;
                    let index = ((y as u32 * PICTURE.0 + x as u32) * 4) as usize;
                    let red = pixels[index];
                    if red > 170 {
                        pale += 1;
                    } else if red < 50 {
                        dark += 1;
                    }
                }
            }
            (pale, dark)
        };
        let (pale, dark) = letters(&pictures[3]);
        assert!(pale > 300 && dark > pale * 2, "{pale} {dark}");
        // Growing out of a point far from it, the plaque's text is not drawn before its board is.
        farmyard.set_reveal(Some((Vec3::new(0.0, 1.0, -2.0), 0.5)));
        let growing = capture_sized(&device, &queue, &farmyard, 4, views[3], picture());
        assert_eq!(letters(&growing).0, 0);
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

/// Where the river and the waterfall rock are seen from, as `(eye, target)`: on the trail at the riverbank, looking
/// down into the river's channel beyond its bank; and at the lookout, looking over the pool at the rock before the
/// waterfall.
fn river_rock_views() -> [(Vec3, Vec3); 2] {
    let (riverbank, _, _) = trail::frame(trail::VIEWPOINTS[0].2);
    let (lookout, _, _) = trail::frame(trail::VIEWPOINTS[1].2);
    [
        (
            Vec3::new(riverbank.x, 1.6, riverbank.y),
            layout::RIVER[6].lerp(layout::RIVER[7], 0.3),
        ),
        (
            Vec3::new(lookout.x, 1.6, lookout.y),
            layout::WATERFALL_ROCK + Vec3::Y * 0.6,
        ),
    ]
}

#[test]
fn the_river_lies_in_its_channel_below_the_trail_and_the_waterfall_rock_stands_in_the_pool_before_the_fall()
 {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let target = Target::new(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            PICTURE.0,
            PICTURE.1,
            4,
            tomori_render::glyphs::SharedGlyphs::new(&device),
        );
        let farmyard =
            Farmyard::with_clock(&target, &Coop::Built, Library::shared(), Clock::Held(0.0));
        let views = river_rock_views();
        let pictures: Vec<Vec<u8>> = views
            .into_iter()
            .map(|view| capture_sized(&device, &queue, &farmyard, 4, view, picture()))
            .collect();
        if let Ok(path) = std::env::var("TOMORI_FARMYARD_RIVER_ROCK_PREVIEW") {
            for (index, pixels) in pictures.iter().enumerate() {
                let picture =
                    image::RgbaImage::from_raw(PICTURE.0, PICTURE.1, pixels.clone()).unwrap();
                picture
                    .save(path.replace(".png", &format!("-{}.png", index + 1)))
                    .unwrap();
            }
        }
        let within = |at: (f32, f32)| {
            assert!(
                (0.05..0.95).contains(&at.0) && (0.05..0.95).contains(&at.1),
                "{at:?}"
            );
            at
        };
        // The average colour about `at` in `pixels`, red, green and blue.
        let tint = |pixels: &[u8], at: (f32, f32)| {
            let (x, y) = (
                (at.0 * PICTURE.0 as f32) as i32,
                (at.1 * PICTURE.1 as f32) as i32,
            );
            let mut sum = Vec3::ZERO;
            for dy in -2..=2 {
                for dx in -2..=2 {
                    let index = (((y + dy) as u32 * PICTURE.0 + (x + dx) as u32) * 4) as usize;
                    sum += Vec3::new(
                        f32::from(pixels[index]),
                        f32::from(pixels[index + 1]),
                        f32::from(pixels[index + 2]),
                    );
                }
            }
            sum / 25.0
        };
        // From the trail, the river's water is drawn in its channel, blue, between the grey shingle of its banks,
        // below the green meadow the trail is on.
        let water = layout::RIVER[6].lerp(layout::RIVER[7], 0.3);
        let (from, _) = views[0];
        let away = ((water - from) * Vec3::new(1.0, 0.0, 1.0)).normalize();
        let flat = |at: Vec3| glam::Vec2::new(at.x, at.z);
        let on = |at: Vec3| {
            let ground =
                tomori_environment_farmyard::backdrop::drawn_ground(flat(at)).unwrap_or(0.0);
            Vec3::new(at.x, ground, at.z)
        };
        let river = tint(&pictures[0], within(seen_at(views[0], water)));
        let banks = [water - away * 2.6, water + away * 2.6]
            .map(|bank| tint(&pictures[0], within(seen_at(views[0], on(bank)))));
        // The meadow beside the trail is green, but where the beach's shingle lies towards the river, which is grey.
        let along = glam::Vec3::new(-away.z, 0.0, away.x);
        let meadow = tint(
            &pictures[0],
            within(seen_at(views[0], on(water - away * 4.6 + along * 1.9))),
        );
        let shingle = tint(
            &pictures[0],
            within(seen_at(views[0], on(water - away * 4.6))),
        );
        assert!(
            shingle.y < shingle.x + 8.0 && shingle.z < shingle.x + 18.0,
            "{shingle}"
        );
        assert!(river.z > river.x + 18.0, "{river}");
        for bank in banks {
            assert!(bank.z < bank.x + 18.0 && bank.y < bank.x + 8.0, "{bank}");
        }
        assert!(meadow.y > meadow.x + 10.0, "{meadow}");
        // From the lookout, the rock is drawn in the pool, its flat top paler than the pool's water beside it.
        let rock = layout::WATERFALL_ROCK;
        let top = tint(&pictures[1], within(seen_at(views[1], rock)));
        let beside = rock + Vec3::new(-1.6, -0.5, 0.4);
        let pool = tint(&pictures[1], within(seen_at(views[1], beside)));
        assert!(pool.z > pool.x + 18.0 && pool.y > pool.x + 12.0, "{pool}");
        assert!(top.x > pool.x * 1.5 && top.y < top.x + 10.0, "{top} {pool}");
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

/// From the lookout's log seat, sitting at its end nearest the fire, looking at the fire, the far seat and its sticks.
pub(crate) fn campsite_view() -> (Vec3, Vec3) {
    let (middle, along, length, _) = trail::seat();
    let end = middle + Vec3::new(along.x, 0.0, along.y) * length * 0.35;
    let fire = tomori_environment_farmyard::campsite::FIRE;
    (
        end + Vec3::new(0.0, 1.15, 0.0),
        Vec3::new(fire.x - 0.5, 0.4, fire.z - 0.05),
    )
}

/// The campsite's sticks as its client side shows them: on their log, but the second, held over the fire by someone
/// else, its marshmallow toasted golden.
fn campsite_sticks() -> Vec<crate::campsite::Shown> {
    use tomori_environment_farmyard::campsite;
    use tomori_physics::Precision;
    use tomori_physics::message::{Held, Shared, State};
    let mut sticks = crate::campsite::Sticks::new();
    let bodies = (0..campsite::STICKS)
        .map(|index| Shared {
            body: campsite::body(index),
            epoch: (index == 1) as u32,
            at: campsite::home(index).encoded(Precision::Rest),
            moving: false,
            held: (index == 1).then_some(Held {
                by: 5,
                hand: tomori_physics::Side::Right,
                offset: campsite::grip().encoded(Precision::Rest),
            }),
        })
        .collect();
    let mut toast = vec![0.0; campsite::STICKS];
    toast[1] = campsite::GOLDEN + 0.05;
    sticks.share(&campsite::Shared {
        sticks: State { bodies },
        toast,
    });
    let along = Vec3::new(0.5, -0.2, -0.85).normalize();
    let hand = campsite::heart() + Vec3::Y * 0.12
        - along * (campsite::STICK_LENGTH - campsite::MARSHMALLOW_INSET - campsite::HOLD);
    let hand = tomori_scene::client_side::Hand::new(
        hand,
        glam::Quat::from_rotation_arc(Vec3::NEG_Z, along),
    );
    sticks.shown(&[(5, hand)], &[], std::time::Instant::now(), 0)
}

#[test]
fn the_campsite_s_bonfire_glows_lighting_its_seats_and_its_sticks_are_drawn_where_its_client_side_shows_them()
 {
    use tomori_environment_farmyard::campsite;
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let target = Target::new(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            PICTURE.0,
            PICTURE.1,
            4,
            tomori_render::glyphs::SharedGlyphs::new(&device),
        );
        let mut farmyard =
            Farmyard::with_clock(&target, &Coop::Built, Library::shared(), Clock::Held(1.3));
        let view = campsite_view();
        let none = crate::client::Drawn {
            stones: Vec::new(),
            chickens: Vec::new(),
            sticks: Vec::new(),
        };
        farmyard.animate(&device, &queue, &none);
        let without = capture_sized(&device, &queue, &farmyard, 4, view, picture());
        let shown = campsite_sticks();
        let sticks = crate::client::Drawn {
            stones: Vec::new(),
            chickens: Vec::new(),
            sticks: shown.clone(),
        };
        farmyard.animate(&device, &queue, &sticks);
        let with = capture_sized(&device, &queue, &farmyard, 4, view, picture());
        if let Ok(path) = std::env::var("TOMORI_FARMYARD_CAMPSITE_PREVIEW") {
            image::RgbaImage::from_raw(PICTURE.0, PICTURE.1, with.clone())
                .unwrap()
                .save(path)
                .unwrap();
        }
        let colour = |pixels: &[u8], point: Vec3| {
            let at = seen_at(view, point);
            assert!(
                (0.05..0.95).contains(&at.0) && (0.05..0.95).contains(&at.1),
                "{at:?}"
            );
            let (x, y) = (
                (at.0 * PICTURE.0 as f32) as u32,
                (at.1 * PICTURE.1 as f32) as u32,
            );
            let mut sum = [0.0f32; 3];
            for dy in 0..3 {
                for dx in 0..3 {
                    let index = (((y + dy - 1) * PICTURE.0 + x + dx - 1) * 4) as usize;
                    for channel in 0..3 {
                        sum[channel] += f32::from(pixels[index + channel]) / 9.0;
                    }
                }
            }
            sum
        };
        // The flames glow, bright and warm, red over green over blue, among the logs above the embers.
        let brightest = |pixels: &[u8]| {
            let low = seen_at(view, campsite::FIRE + Vec3::new(-0.2, 0.5, 0.0));
            let high = seen_at(view, campsite::FIRE + Vec3::new(0.2, 0.05, 0.0));
            let mut best = [0u8; 3];
            for y in (low.1.min(high.1) * PICTURE.1 as f32) as u32
                ..(low.1.max(high.1) * PICTURE.1 as f32) as u32
            {
                for x in (low.0.min(high.0) * PICTURE.0 as f32) as u32
                    ..(low.0.max(high.0) * PICTURE.0 as f32) as u32
                {
                    let index = ((y * PICTURE.0 + x) * 4) as usize;
                    if pixels[index] as u32 + pixels[index + 1] as u32
                        > best[0] as u32 + best[1] as u32
                    {
                        best = [pixels[index], pixels[index + 1], pixels[index + 2]];
                    }
                }
            }
            best
        };
        let flames = brightest(&with);
        assert!(
            flames[0] > 220 && flames[0] >= flames[1] && flames[1] > flames[2],
            "{flames:?}"
        );
        // The far seat, by the fire, is lit warmly, redder than the meadow far from it.
        let (seat, ..) = campsite::seat();
        let log = colour(&with, seat + Vec3::Y * 0.3);
        assert!(log[0] > log[2] * 1.4, "{log:?}");
        // The sticks are drawn where the client side shows them, a marshmallow over the fire golden and warm.
        let changed = with
            .chunks(4)
            .zip(without.chunks(4))
            .filter(|(a, b)| {
                a[..3]
                    .iter()
                    .zip(&b[..3])
                    .map(|(a, b)| a.abs_diff(*b) as u32)
                    .sum::<u32>()
                    > 30
            })
            .count();
        assert!(changed > 400, "{changed}");
        // A marshmallow on its log is pale, lit by the fire.
        let white = colour(&with, shown[0].marshmallow);
        let bare = colour(&without, shown[0].marshmallow);
        assert!(
            white[2] > bare[2] + 20.0 && white[0] > 120.0,
            "{white:?} {bare:?}"
        );
        // Burning marshmallows add their own little flames, and none are added for those that do not burn.
        let burning: Vec<crate::campsite::Shown> = shown
            .iter()
            .map(|stick| crate::campsite::Shown {
                burning: true,
                ..*stick
            })
            .collect();
        farmyard.animate(
            &device,
            &queue,
            &crate::client::Drawn {
                stones: Vec::new(),
                chickens: Vec::new(),
                sticks: burning,
            },
        );
        assert_eq!(
            farmyard.fire.count,
            farmyard.fire.lasting + 9 * campsite::STICKS as u32
        );
        farmyard.animate(&device, &queue, &sticks);
        assert_eq!(farmyard.fire.count, farmyard.fire.lasting);
        // Growing out of a point far from it, its flames are not drawn before the rest of it.
        farmyard.set_reveal(Some((Vec3::new(0.0, 1.0, -2.0), 0.5)));
        let growing = capture_sized(&device, &queue, &farmyard, 4, view, picture());
        let unlit = brightest(&growing);
        assert!(unlit[0] < 40, "{unlit:?}");
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

#[test]
fn the_bonfire_s_light_flickers_and_the_campsite_draws_within_its_budget() {
    use tomori_environment_farmyard::campsite;
    // About its middle, never dark, each moment its own.
    let flickers: Vec<f32> = (0..200).map(|step| flicker(step as f32 * 0.05)).collect();
    assert!(flickers.iter().all(|value| (0.7..1.3).contains(value)));
    assert!(
        flickers
            .windows(2)
            .any(|pair| (pair[0] - pair[1]).abs() > 0.02)
    );
    // It is the farmyard's fourth lamp, the last a scenery's models are lit by, at the fire, its light changing.
    let lit = lighting_at(1.0);
    assert_eq!(lit.lamps.len(), tomori_assets::gpu::LAMPS);
    assert!(lit.lamps[3].position.distance(campsite::heart()) < 0.3);
    assert_ne!(lighting_at(1.0).lamps[3], lighting_at(1.4).lamps[3]);
    assert_eq!(lighting(), lighting_at(0.0));
    // The campsite, its four sticks, each with one marshmallow shown, and its flames, embers and the little flames of
    // every marshmallow burning, together within its budget.
    let triangles = |model: &tomori_assets::Model, shown: &dyn Fn(&str) -> bool| -> usize {
        model
            .nodes
            .iter()
            .filter(|node| shown(node.name.as_deref().unwrap_or("")))
            .filter_map(|node| node.mesh)
            .flat_map(|mesh| &model.meshes[mesh].primitives)
            .map(|primitive| primitive.indices.len() / 3)
            .sum()
    };
    let site = triangles(&campsite::model(), &|_| true);
    let stick = triangles(&campsite::stick(), &|name| {
        name == "stick" || name == campsite::shade_node(0)
    });
    let fire = bonfire().len() / 3 + campsite::STICKS * 3;
    let together = site + campsite::STICKS * stick + fire;
    println!("campsite {site}, a stick {stick}, the fire {fire}: {together} triangles");
    assert!(together <= campsite::BUDGET, "{together}");
    // Each stick shows one marshmallow, of its shade.
    let shown = campsite_sticks();
    let instances = stick_instances(&shown);
    for (instance, stick) in instances.iter().zip(&shown) {
        let meshes: Vec<usize> = instance.draws().iter().map(|draw| draw.mesh).collect();
        assert_eq!(meshes.len(), 2);
        assert_eq!(meshes[1], 1 + stick.shade);
    }
}

/// The beach's stones as its server side settles them, but three of its building stones, two blocks and its broadest
/// flagstone, built into a little dolmen on the flat shingle where no other stone lies; and the dolmen's middle, on the
/// shingle.
fn beach_with_a_dolmen() -> (Vec<crate::stones::Shown>, Vec3) {
    use tomori_environment_farmyard::beach;
    use tomori_physics::Placement;
    use tomori_physics::world::World;
    let by_width = |kind: beach::Kind| {
        let mut stones: Vec<usize> = (beach::RIVER..beach::STONES)
            .filter(|index| beach::kind(*index) == kind)
            .collect();
        stones.sort_by(|a, b| beach::size(*b).0.total_cmp(&beach::size(*a).0));
        stones
    };
    let (blocks, lintel) = (
        by_width(beach::Kind::Block),
        by_width(beach::Kind::Flagstone)[0],
    );
    let (left, right) = (blocks[0], blocks[2]);
    let mut bodies = beach::bodies();
    // Where on the flat shingle the dolmen stands farthest from every other stone.
    let others: Vec<glam::Vec2> = bodies
        .iter()
        .filter(|body| ![left, right, lintel].contains(&(body.id as usize)))
        .map(|body| glam::Vec2::new(body.home.position.x, body.home.position.z))
        .collect();
    let (out, along) = (0..=10)
        .flat_map(|out| {
            (0..=16).map(move |along| {
                (
                    beach::NEAR + 0.6 + out as f32 * 0.05,
                    -0.8 + along as f32 * 0.1,
                )
            })
        })
        .max_by(|a, b| {
            let clear = |(out, along): (f32, f32)| {
                let middle = beach::at(out, along);
                others
                    .iter()
                    .map(|other| other.distance(middle))
                    .fold(f32::INFINITY, f32::min)
            };
            clear(*a).total_cmp(&clear(*b))
        })
        .unwrap();
    let (_, way, _) = beach::frame();
    let along_beach = glam::Quat::from_rotation_y((-way.y).atan2(way.x));
    let span = |stone: usize, at: &Placement| {
        beach::stone(stone)
            .into_iter()
            .map(|point| at.transform_point(point).y)
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), y| {
                (low.min(y), high.max(y))
            })
    };
    let pillar = |stone: usize, offset: f32| {
        let point = beach::at(out, along + offset);
        let mut at = Placement::new(
            Vec3::new(point.x, 0.0, point.y),
            along_beach * glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        );
        at.position.y = beach::height(point).unwrap() + 0.01 - span(stone, &at).0;
        at
    };
    let (first, second) = (pillar(left, -0.24), pillar(right, 0.24));
    let middle = beach::at(out, along);
    let mut top = Placement::new(Vec3::new(middle.x, 0.0, middle.y), along_beach);
    top.position.y =
        span(left, &first).1.max(span(right, &second).1) + 0.003 - span(lintel, &top).0;
    for (stone, at) in [(left, first), (right, second), (lintel, top)] {
        bodies[stone].home = at;
    }
    let mut world = World::new(
        beach::settings(),
        beach::colliders(),
        beach::waters(),
        bodies,
    )
    .unwrap();
    assert!(world.settle(15.0));
    let shown = world
        .bodies()
        .map(|body| crate::stones::Shown {
            stone: body as usize,
            transform: world.placement(body).unwrap().matrix(),
        })
        .collect();
    (
        shown,
        Vec3::new(middle.x, beach::height(middle).unwrap(), middle.y),
    )
}

#[test]
fn a_little_dolmen_of_the_beach_s_building_stones_is_drawn_standing_on_its_shingle() {
    use tomori_environment_farmyard::beach;
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let target = Target::new(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            PICTURE.0,
            PICTURE.1,
            4,
            tomori_render::glyphs::SharedGlyphs::new(&device),
        );
        let mut farmyard =
            Farmyard::with_clock(&target, &Coop::Built, Library::shared(), Clock::Held(0.0));
        let (stones, middle) = beach_with_a_dolmen();
        // From the trail beside the beach, a step along from the dolmen, looking down at it.
        let (_, way, outward) = beach::frame();
        let eye = middle - Vec3::new(outward.x, 0.0, outward.y) * 1.3
            + Vec3::new(way.x, 0.0, way.y) * 1.1
            + Vec3::Y * 1.5;
        let view = (eye, middle + Vec3::Y * 0.3);
        let mut drawn = |stones: Vec<crate::stones::Shown>| {
            farmyard.animate(
                &device,
                &queue,
                &crate::client::Drawn {
                    stones,
                    chickens: Vec::new(),
                    sticks: Vec::new(),
                },
            );
            capture_sized(&device, &queue, &farmyard, 4, view, picture())
        };
        let without = drawn(Vec::new());
        let pixels = drawn(stones);
        if let Ok(path) = std::env::var("TOMORI_FARMYARD_BEACH_PREVIEW") {
            image::RgbaImage::from_raw(PICTURE.0, PICTURE.1, pixels.clone())
                .unwrap()
                .save(path)
                .unwrap();
        }
        // The lintel is drawn high on its pillars, over the shingle seen there without it, and its shade beneath it
        // darkens nothing of the shingle well beyond it.
        let lintel = seen_at(view, middle + Vec3::Y * 0.49);
        assert!(
            (about(&pixels, lintel) - about(&without, lintel)).abs() > 20.0,
            "{} {}",
            about(&pixels, lintel),
            about(&without, lintel)
        );
        let beyond = seen_at(view, middle - Vec3::new(outward.x, 0.0, outward.y) * 0.9);
        assert!((about(&pixels, beyond) - about(&without, beyond)).abs() < 2.0);
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

#[test]
fn each_stone_is_an_instance_of_the_stones_model_showing_only_its_own() {
    let shown = [
        crate::stones::Shown {
            stone: 3,
            transform: Mat4::from_translation(Vec3::new(1.0, 0.0, 2.0)),
        },
        crate::stones::Shown {
            stone: 7,
            transform: Mat4::IDENTITY,
        },
    ];
    let instances = stone_instances(&shown);
    assert_eq!(instances.len(), 2);
    assert_eq!(instances[0].transform, shown[0].transform);
    for (instance, stone) in instances.iter().zip(&shown) {
        let meshes: Vec<usize> = instance.draws().iter().map(|draw| draw.mesh).collect();
        assert_eq!(meshes, [stone.stone]);
    }
}

/// Where the ferns are seen from, as `(eye, target)`: on the trail by the river, looking along it past the ferns on its
/// banks; and from the trail beside the campsite, looking past its fire to the tree ferns beyond it.
fn fern_views() -> [(Vec3, Vec3); 2] {
    let at = |point: glam::Vec2, y: f32| Vec3::new(point.x, y, point.y);
    let (from, _, _) = trail::frame(10);
    let (ahead, _, outward) = trail::frame(19);
    let fire = tomori_environment_farmyard::campsite::FIRE;
    let (beside, _, _) = trail::frame(38);
    [
        (at(from, 1.6), at(ahead + outward * 1.8, 1.0)),
        (at(beside, 1.6), Vec3::new(fire.x - 1.5, 0.9, fire.z - 1.2)),
    ]
}

/// A ponga near the trail, away from the bonfire's light, and the heart of its crown.
fn ponga() -> Vec3 {
    use tomori_environment_farmyard::{campsite, ferns};
    let fire = campsite::FIRE;
    ferns::ferns()
        .into_iter()
        .filter(|fern| fern.kind == ferns::Kind::Ponga)
        .map(|fern| fern.transform().transform_point3(fern.kind.heart()))
        .find(|heart| glam::Vec2::new(heart.x - fire.x, heart.z - fire.z).length() > 8.0)
        .expect("a ponga stands away from the fire")
}

/// How many pixels of `a` and `b` differ, within `reach` pixels of `at`, fractions across and down.
fn changed(a: &[u8], b: &[u8], at: (f32, f32), reach: i32) -> usize {
    let (x, y) = (
        (at.0 * PICTURE.0 as f32) as i32,
        (at.1 * PICTURE.1 as f32) as i32,
    );
    let mut count = 0;
    for dy in -reach..reach {
        for dx in -reach..reach {
            let (px, py) = (x + dx, y + dy);
            if px < 0 || py < 0 || px >= PICTURE.0 as i32 || py >= PICTURE.1 as i32 {
                continue;
            }
            let index = ((py as u32 * PICTURE.0 + px as u32) * 4) as usize;
            let difference: u32 = (0..3)
                .map(|channel| u32::from(a[index + channel].abs_diff(b[index + channel])))
                .sum();
            count += usize::from(difference > 6);
        }
    }
    count
}

#[test]
fn the_ferns_stand_about_the_trail_swaying_in_the_breeze_ponga_silver_beneath() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let target = Target::new(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            PICTURE.0,
            PICTURE.1,
            4,
            tomori_render::glyphs::SharedGlyphs::new(&device),
        );
        let made = |seconds| {
            Farmyard::with_clock(
                &target,
                &Coop::Built,
                Library::shared(),
                Clock::Held(seconds),
            )
        };
        let mut farmyard = made(1.3);
        let pictures: Vec<Vec<u8>> = fern_views()
            .into_iter()
            .map(|view| capture_sized(&device, &queue, &farmyard, 4, view, picture()))
            .collect();
        if let Ok(path) = std::env::var("TOMORI_FARMYARD_FERNS_PREVIEW") {
            for (index, pixels) in pictures.iter().enumerate() {
                image::RgbaImage::from_raw(PICTURE.0, PICTURE.1, pixels.clone())
                    .unwrap()
                    .save(path.replace(".png", &format!("-{}.png", index + 1)))
                    .unwrap();
            }
        }
        // A ponga's crown, seen from the side, sways in the breeze: a moment later, its fronds have moved.
        let heart = ponga();
        let side = (heart + Vec3::new(6.0, 0.0, 0.0)).with_y(1.6);
        let view = (side, heart);
        let now = capture_sized(&device, &queue, &farmyard, 4, view, picture());
        let later = capture_sized(&device, &queue, &made(2.4), 4, view, picture());
        let crown = seen_at(view, heart);
        assert!(changed(&now, &later, crown, 120) > 300);
        // Looked up at from beneath, its fronds are silver, grey against the sky; looked down on from above, green.
        let hues = |pixels: &[u8]| {
            let (mut silver, mut green) = (0, 0);
            for pixel in pixels.chunks(4) {
                let [red, green_, blue] = [0, 1, 2].map(|channel| i32::from(pixel[channel]));
                if red > 35 && (red - green_).abs() < 12 && (green_ - blue).abs() < 14 {
                    silver += 1;
                } else if green_ * 4 > red * 5 && green_ > blue {
                    green += 1;
                }
            }
            (silver, green)
        };
        let tip = heart + Vec3::new(1.4, 0.0, 0.3);
        let beneath = capture_sized(
            &device,
            &queue,
            &farmyard,
            4,
            (tip - Vec3::Y * 1.5, tip + Vec3::new(0.1, 0.0, 0.0)),
            picture(),
        );
        let above = capture_sized(
            &device,
            &queue,
            &farmyard,
            4,
            (tip + Vec3::Y * 2.5, tip - Vec3::new(0.1, 0.0, 0.0)),
            picture(),
        );
        let (silver, green) = hues(&beneath);
        assert!(silver > 100_000 && silver > green * 3, "{silver} {green}");
        let (silver, green) = hues(&above);
        assert!(green > 100_000 && green > silver * 3, "{silver} {green}");
        // Growing out of a point far from it, the ponga is not drawn before the reveal reaches it.
        farmyard.set_reveal(Some((Vec3::new(0.0, 1.0, -2.0), 0.5)));
        let growing = capture_sized(&device, &queue, &farmyard, 4, view, picture());
        let again = capture_sized(&device, &queue, &farmyard, 4, view, picture());
        assert_eq!(growing, again);
        assert!(changed(&now, &growing, crown, 120) > 3000);
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

#[test]
fn every_fern_is_drawn_once_in_the_detail_its_distance_from_the_eyes_asks() {
    use tomori_environment_farmyard::ferns::{self, Detail};
    let (vertices, meshes) = fern_vertices();
    assert_eq!(meshes.len(), ferns::drawn().len());
    assert_eq!(meshes.last().unwrap().1.end as usize, vertices.len());
    let all = ferns::all();
    for eyes in [layout::ARRIVAL + Vec3::Y * 1.6, ponga()] {
        let details: Vec<Detail> = all.iter().map(|fern| fern.detail(eyes)).collect();
        let placed: Vec<Option<Detail>> = details.iter().copied().map(Some).collect();
        let (instances, draws) = fern_instances(&all, &placed, &meshes);
        // Every one, but the koru beyond their distance, which are too small to see.
        let unseen = all
            .iter()
            .zip(&details)
            .filter(|(fern, detail)| fern.kind == ferns::Kind::Koru && **detail == Detail::Far)
            .count();
        assert_eq!(instances.len(), all.len() - unseen);
        // One draw for each mesh any fern is drawn with, its instances one after another.
        assert!(draws.len() <= meshes.len());
        assert!(
            draws
                .iter()
                .all(|(drawn, _)| meshes.iter().any(|(_, vertices)| vertices == drawn))
        );
        assert!(
            draws
                .windows(2)
                .all(|pair| pair[0].1.end == pair[1].1.start)
        );
        let near = details
            .iter()
            .filter(|detail| **detail == Detail::Near)
            .count();
        assert!(near > 0 && near < all.len());
        // Each is drawn where it stands, turned, sized and swaying its own way.
        for (fern, detail) in all.iter().zip(&details) {
            let drawn = instances.contains(&FernInstance::from(fern));
            assert_eq!(
                drawn,
                fern.kind != ferns::Kind::Koru || *detail == Detail::Near
            );
        }
    }
}
