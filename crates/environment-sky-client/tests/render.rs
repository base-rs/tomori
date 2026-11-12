// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Sky drawn by `tomori-render`'s renderer, as a client registering it draws it: in stereo, beside the grid, growing out of a space, and multisampled.
use glam::Mat4;
use tomori_render::graphics::Renderer;
use tomori_render::scenery::Sceneries;
use tomori_render::test_gpu;
use tomori_scene::environment::Environment;
use tomori_scene::interaction::Mode;
use tomori_scene::swarm::Swarm;

const SKY: Environment = Environment::new(tomori_environment_sky_client::ID)
    .with_version(tomori_environment_sky_client::VERSION);
const GRID: Environment = Environment::MINIMAL;

/// Sceneries with Sky registered, as a client registers it.
fn sky() -> Sceneries {
    let mut sceneries = Sceneries::empty();
    tomori_environment_sky_client::register(&mut sceneries);
    sceneries
}

/// Save `sheet` where the environment variable `variable` says, if it is set, to look at.
fn preview(variable: &str, sheet: &image::RgbaImage) {
    if let Some(path) = std::env::var_os(variable) {
        sheet.save(path).expect("the preview is saved");
    }
}

#[test]
fn sky_environment_renders_stereo_switches_and_preserves_recenter() {
    const WIDTH: u32 = 768;
    const HEIGHT: u32 = 512;
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        pollster::block_on(async move {
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let textures: [_; 2] = std::array::from_fn(|_| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Sky test eye"),
                    size: wgpu::Extent3d {
                        width: WIDTH,
                        height: HEIGHT,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
            });
            let views = textures
                .each_ref()
                .map(|t| t.create_view(&Default::default()));
            let mut renderer = Renderer::with_sceneries(
                &device,
                &queue,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                WIDTH,
                HEIGHT,
                1,
                &sky(),
            );
            let mut swarm = Swarm::default();
            for _ in 0..240 {
                swarm.set_visual(
                    tomori_scene::interaction::Visual {
                        mode: Mode::Ready,
                        ..Default::default()
                    },
                    1.0 / 72.0,
                );
                swarm.step(1.0 / 72.0, None);
            }
            let positions: Vec<_> = swarm.agents.iter().map(|a| a.position).collect();
            let projection = glam::camera::rh::proj::directx::perspective(
                1.2,
                WIDTH as f32 / HEIGHT as f32,
                0.05,
                100.0,
            );
            let matrices = [-0.032, 0.032].map(|x| {
                let eye = glam::Vec3::new(x, 1.6, 0.0);
                projection
                    * glam::camera::rh::view::look_at_mat4(
                        eye,
                        eye + glam::Vec3::new(0.0, 0.15, -1.0),
                        glam::Vec3::Y,
                    )
            });
            let capture = |renderer: &mut Renderer, matrices: [Mat4; 2], anchor: Mat4| {
                renderer.render(&device, &queue, &views, &matrices, anchor, &[]);
                let buffers: [_; 2] = std::array::from_fn(|_| {
                    device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Sky test readback"),
                        size: u64::from(WIDTH * HEIGHT * 4),
                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    })
                });
                let mut encoder = device.create_command_encoder(&Default::default());
                for eye in 0..2 {
                    encoder.copy_texture_to_buffer(
                        textures[eye].as_image_copy(),
                        wgpu::TexelCopyBufferInfo {
                            buffer: &buffers[eye],
                            layout: wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(WIDTH * 4),
                                rows_per_image: Some(HEIGHT),
                            },
                        },
                        wgpu::Extent3d {
                            width: WIDTH,
                            height: HEIGHT,
                            depth_or_array_layers: 1,
                        },
                    );
                }
                queue.submit([encoder.finish()]);
                buffers
                    .iter()
                    .map(|buffer| {
                        let (tx, rx) = std::sync::mpsc::channel();
                        buffer
                            .slice(..)
                            .map_async(wgpu::MapMode::Read, move |result| {
                                tx.send(result).unwrap();
                            });
                        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                        rx.recv().unwrap().unwrap();
                        let data = buffer.slice(..).get_mapped_range().unwrap().to_vec();
                        buffer.unmap();
                        data
                    })
                    .collect::<Vec<_>>()
            };
            renderer.set_environment(GRID);
            renderer.update(&queue, &swarm, None, false);
            let minimal = capture(&mut renderer, matrices, Mat4::IDENTITY);
            // Everything is hazed as the environment shown is: the grid by none, and Sky by its own.
            let standing = glam::Vec3::new(0.0, 1.6, 0.0);
            assert_eq!(renderer.haze(standing), tomori_render::haze::Haze::NONE);
            renderer.set_environment(SKY);
            assert_eq!(renderer.haze(standing), tomori_environment_sky_client::HAZE);
            renderer.update(&queue, &swarm, None, false);
            let sky = capture(&mut renderer, matrices, Mat4::IDENTITY);
            assert_ne!(sky[0], minimal[0]);
            assert_ne!(sky[0], sky[1], "scenery must have stereo parallax");
            // Return to the original preset without touching the simulation.
            renderer.set_environment(GRID);
            renderer.update(&queue, &swarm, None, false);
            assert_eq!(minimal, capture(&mut renderer, matrices, Mat4::IDENTITY));
            assert_eq!(
                positions,
                swarm.agents.iter().map(|a| a.position).collect::<Vec<_>>()
            );
            renderer.set_environment(SKY);
            renderer.update(&queue, &swarm, None, false);
            let anchor = Mat4::from_rotation_translation(
                glam::Quat::from_rotation_y(0.35),
                glam::Vec3::new(1.0, 0.0, -0.5),
            );
            let compensated = capture(
                &mut renderer,
                matrices.map(|m| m * anchor.inverse()),
                anchor,
            );
            for eye in 0..2 {
                let changed = sky[eye]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(compensated[eye].as_chunks::<4>().0)
                    .filter(|(a, b)| a != b)
                    .count();
                assert!(
                    changed < 2000,
                    "recenter changed {changed} pixels in eye {eye}"
                );
            }
            // Distant sky follows orientation, not the viewer's translation.
            let translated = capture(
                &mut renderer,
                matrices.map(|m| m * Mat4::from_translation(glam::Vec3::new(-0.4, 0.0, 0.0))),
                Mat4::IDENTITY,
            );
            let top_bytes = (WIDTH * 100 * 4) as usize;
            let sky_changes = sky[0][..top_bytes]
                .as_chunks::<4>()
                .0
                .iter()
                .zip(translated[0][..top_bytes].as_chunks::<4>().0)
                .filter(|(a, b)| a != b)
                .count();
            assert!(
                sky_changes < 800,
                "translation must not drag the sky: {sky_changes}"
            );
            let overview = projection
                * glam::camera::rh::view::look_at_mat4(
                    glam::Vec3::new(6.0, 5.0, 8.0),
                    glam::Vec3::new(0.0, 0.7, -2.5),
                    glam::Vec3::Y,
                );
            let overview = capture(&mut renderer, [overview; 2], Mat4::IDENTITY);
            // Growing out of a space's object: Sky inside the sphere, the grid outside it.
            let reveal = |radius| tomori_scene::arrival_motion::Scenery {
                environment: SKY,
                reveal: Some(tomori_scene::arrival_motion::Sphere {
                    centre: glam::Vec3::new(0.0, 1.0, -2.5),
                    radius,
                }),
            };
            // Growing, nothing is hazed until the eye is inside it, as on the grid.
            let mut hazes = Vec::new();
            let mut revealed = |scenery| {
                renderer.set_scenery(scenery);
                hazes.push(renderer.haze(standing));
                renderer.update(&queue, &swarm, None, false);
                capture(&mut renderer, matrices, Mat4::IDENTITY)
            };
            let point = revealed(reveal(0.0));
            let growing = revealed(reveal(1.5));
            let world = revealed(reveal(1000.0));
            let filled = revealed(tomori_scene::arrival_motion::Scenery {
                environment: SKY,
                reveal: None,
            });
            assert_eq!(renderer.environment(), SKY);
            assert_eq!(
                hazes,
                [
                    tomori_render::haze::Haze::NONE,
                    tomori_render::haze::Haze::NONE,
                    tomori_environment_sky_client::HAZE,
                    tomori_environment_sky_client::HAZE
                ]
            );
            // Within one step, as in `workspace_graphics`: a few interpolated colours may round differently where only the pipeline differs.
            let changed = |a: &[u8], b: &[u8]| {
                a.as_chunks::<4>()
                    .0
                    .iter()
                    .zip(b.as_chunks::<4>().0)
                    .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 1))
                    .count()
            };
            let rows = |image: &[u8], rows: std::ops::Range<u32>| {
                image[(rows.start * WIDTH * 4) as usize..(rows.end * WIDTH * 4) as usize].to_vec()
            };
            for eye in 0..2 {
                assert_eq!(
                    changed(&point[eye], &minimal[eye]),
                    0,
                    "a sphere of no size shows only the grid"
                );
                assert_eq!(
                    changed(&world[eye], &sky[eye]),
                    0,
                    "a sphere around everything shows all of Sky"
                );
                assert_eq!(filled[eye], sky[eye], "filling the world draws as before");
                // The sphere, centred below the view, fills its middle and bottom, and leaves the top to the grid.
                assert_eq!(
                    changed(&rows(&growing[eye], 0..40), &rows(&minimal[eye], 0..40)),
                    0,
                    "the grid shows outside the sphere"
                );
                let middle = rows(&growing[eye], 380..420);
                assert!(
                    changed(&middle, &rows(&minimal[eye], 380..420)) > middle.len() / 4 / 2,
                    "Sky shows inside the sphere"
                );
                assert_ne!(growing[eye], sky[eye]);
            }
            {
                let mut sheet = image::RgbaImage::new(WIDTH * 2, HEIGHT * 2);
                for (i, bytes) in [&sky[0], &sky[1], &minimal[0], &overview[0]]
                    .into_iter()
                    .enumerate()
                {
                    let image = image::RgbaImage::from_raw(WIDTH, HEIGHT, bytes.clone()).unwrap();
                    image::imageops::replace(
                        &mut sheet,
                        &image,
                        (i % 2) as i64 * i64::from(WIDTH),
                        (i / 2) as i64 * i64::from(HEIGHT),
                    );
                }
                preview("TOMORI_SKY_PREVIEW", &sheet);
            }
            assert!(scope.pop().await.is_none());
        })
    });
}

#[test]
fn sky_multisamples_with_the_scene_pass() {
    // Rows of a multiple of 256 bytes, as copying the image out needs.
    const WIDTH: u32 = 256;
    const HEIGHT: u32 = 160;
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        pollster::block_on(async move {
            let format = wgpu::TextureFormat::Rgba8UnormSrgb;
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Sky multisample test eye"),
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
            let eye = glam::Vec3::new(0.0, 1.6, 0.0);
            let matrix = glam::camera::rh::proj::directx::perspective(
                1.0,
                WIDTH as f32 / HEIGHT as f32,
                0.05,
                50.0,
            ) * glam::camera::rh::view::look_at_mat4(
                eye,
                glam::Vec3::new(0.0, 0.8, -3.0),
                glam::Vec3::Y,
            );
            let swarm = Swarm::default();
            // Sky's pipelines must match the pass's sample count, or drawing fails validation.
            let render = |samples: u32| {
                let mut renderer = Renderer::with_sceneries(
                    &device,
                    &queue,
                    format,
                    WIDTH,
                    HEIGHT,
                    samples,
                    &sky(),
                );
                renderer.set_environment(SKY);
                renderer.update(&queue, &swarm, None, false);
                renderer.render(
                    &device,
                    &queue,
                    std::slice::from_ref(&view),
                    &[matrix],
                    Mat4::IDENTITY,
                    &[],
                );
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Sky multisample test readback"),
                    size: u64::from(WIDTH * HEIGHT * 4),
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
                let (tx, rx) = std::sync::mpsc::channel();
                buffer
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                rx.recv().unwrap().unwrap();
                let data = buffer.slice(..).get_mapped_range().unwrap().to_vec();
                buffer.unmap();
                data.as_chunks::<4>()
                    .0
                    .iter()
                    .copied()
                    .collect::<std::collections::HashSet<_>>()
                    .len()
            };
            let single = render(1);
            let multi = render(4);
            assert!(
                multi > single,
                "multisampling blends more colours: {multi} > {single}"
            );
            assert!(scope.pop().await.is_none());
        })
    });
}
