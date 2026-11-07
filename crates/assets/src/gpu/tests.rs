// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Drawing models on the shared test device, on lavapipe in CI: where the GPU skins a model is where the CPU does, every
//! instance of a primitive is drawn in one draw, a hidden node is not drawn, and the reveal sphere cuts models off.
use std::sync::Arc;

use glam::{Mat4, Quat, Vec3, Vec4};
use tomori_render::test_gpu;

use super::*;
use crate::{Pose, Transform};

const SIZE: u32 = 96;

/// What `draw` draws into a cleared target of [`SIZE`] square, read back as RGBA.
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    samples: u32,
    draw: impl FnOnce(&mut wgpu::RenderPass<'_>),
) -> Vec<u8> {
    let target = |format, samples, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Model test target"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
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
    let resolved = target(
        wgpu::TextureFormat::Rgba8Unorm,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let color = (samples > 1).then(|| {
        target(
            wgpu::TextureFormat::Rgba8Unorm,
            samples,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        )
    });
    let depth = target(
        wgpu::TextureFormat::Depth32Float,
        samples,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let resolved_view = resolved.create_view(&Default::default());
    let color_view = color
        .as_ref()
        .map(|color| color.create_view(&Default::default()));
    let depth_view = depth.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Model test pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view.as_ref().unwrap_or(&resolved_view),
                depth_slice: None,
                resolve_target: color_view.as_ref().map(|_| &resolved_view),
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
        draw(&mut pass);
    }
    let stride = (SIZE * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Model test readback"),
        size: u64::from(stride * SIZE),
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
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
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
        .flat_map(|row| &row[..(SIZE * 4) as usize])
        .copied()
        .collect();
    drop(data);
    buffer.unmap();
    pixels
}

/// The pixels drawn, not black, as `(x, y)`.
fn drawn(pixels: &[u8]) -> Vec<(u32, u32)> {
    pixels
        .chunks(4)
        .enumerate()
        .filter(|(_, pixel)| pixel[..3].iter().any(|channel| *channel > 8))
        .map(|(index, _)| (index as u32 % SIZE, index as u32 / SIZE))
        .collect()
}

/// Where `point` is drawn on the target, through `view_projection`.
fn projected(view_projection: Mat4, point: Vec3) -> (f32, f32) {
    let clip = view_projection.project_point3(point);
    (
        (clip.x * 0.5 + 0.5) * SIZE as f32,
        (0.5 - clip.y * 0.5) * SIZE as f32,
    )
}

/// The arm, thickened into a strip so that it covers pixels, with square ends.
fn arm() -> Arc<Model> {
    let mut model = crate::pose::tests::arm();
    let primitive = &mut model.meshes[0].primitives[0];
    for (position, joint) in [
        (Vec3::new(1.0, -0.3, 0.0), 0),
        (Vec3::new(2.0, -0.3, 0.0), 1),
        (Vec3::new(0.0, -0.3, 0.0), 0),
    ] {
        primitive.positions.push(position);
        primitive.joints.push([joint; INFLUENCES]);
        primitive
            .weights
            .push([1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    }
    primitive.indices = vec![0, 5, 3, 0, 3, 1, 1, 3, 4, 1, 4, 2];
    Arc::new(model)
}

fn camera() -> Mat4 {
    Mat4::perspective_rh(1.0, 1.0, 0.1, 20.0)
        * Mat4::look_at_rh(Vec3::new(1.0, 0.3, 4.0), Vec3::new(1.0, 0.3, 0.0), Vec3::Y)
}

fn even() -> Lighting {
    Lighting {
        sky: Vec3::ONE,
        ground: Vec3::ONE,
        sun: Vec3::ZERO,
        ..Lighting::default()
    }
}

#[test]
fn every_shader_is_valid_wgsl() {
    let files = test_gpu::wgsl_files(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
    let names: Vec<_> = files.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["model.wgsl"]);
    for (name, source) in files {
        test_gpu::validate_wgsl(&name, &source);
    }
}

#[test]
fn a_skinned_model_is_drawn_where_its_joints_put_it() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let renderer = Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, 1);
        renderer.set_lighting(&queue, &even());
        let model = arm();
        let gpu = renderer.upload(&device, &queue, &model);
        let mut batch = renderer.batch(&device);
        let view_projection = camera();
        renderer.prepare(&queue, 0, view_projection, Vec3::new(1.0, 0.3, 4.0));
        let bounds = |pixels: &[u8]| {
            let drawn = drawn(pixels);
            let (xs, ys): (Vec<_>, Vec<_>) = drawn.into_iter().unzip();
            (
                *xs.iter().min().unwrap(),
                *xs.iter().max().unwrap(),
                *ys.iter().min().unwrap(),
                *ys.iter().max().unwrap(),
            )
        };
        // At rest, the arm lies along X; bent, its hand points up, where the CPU skins it.
        for time in [0.0, 1.0] {
            let mut instance = Instance::new(model.clone());
            instance.set_pose(Pose::sampled(&model, 0, time));
            renderer.place(
                &device,
                &queue,
                &gpu,
                &mut batch,
                std::slice::from_ref(&instance),
            );
            let pixels = render(&device, &queue, 1, |pass| {
                renderer.draw(pass, 0, &gpu, &batch)
            });
            let draw = &instance.draws()[0];
            let points = crate::deformed(
                &model.meshes[0].primitives[0],
                draw.joints.as_ref().unwrap(),
                draw.world,
            );
            let projected: Vec<_> = points
                .iter()
                .map(|point| projected(view_projection, *point))
                .collect();
            let (low_x, high_x, low_y, high_y) = bounds(&pixels);
            let near = |drawn: u32, expected: f32| (drawn as f32 - expected).abs() <= 2.0;
            assert!(
                near(
                    low_x,
                    projected.iter().map(|p| p.0).fold(f32::MAX, f32::min)
                ),
                "{time}"
            );
            assert!(
                near(
                    high_x,
                    projected.iter().map(|p| p.0).fold(f32::MIN, f32::max)
                ),
                "{time}"
            );
            assert!(
                near(
                    low_y,
                    projected.iter().map(|p| p.1).fold(f32::MAX, f32::min)
                ),
                "{time}"
            );
            assert!(
                near(
                    high_y,
                    projected.iter().map(|p| p.1).fold(f32::MIN, f32::max)
                ),
                "{time}"
            );
        }
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

#[test]
fn instances_are_drawn_together_and_hidden_nodes_and_what_lies_outside_the_reveal_are_not() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut renderer = Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, 4);
        // A lamp, haze and an emissive, textured material, as a scenery lights its models.
        let mut lighting = even();
        lighting.lamps =
            vec![Lamp::new(Vec3::new(1.0, 1.0, 1.0), 3.0, Vec3::new(1.0, 0.6, 0.2)); 6];
        lighting.haze = Vec3::new(0.1, 0.1, 0.2);
        lighting.haze_density = 0.001;
        renderer.set_lighting(&queue, &lighting);
        // A square that is not skinned, with no normals, so that each triangle is lit by its own, and two materials.
        let square = Model::built(
            vec![
                crate::Node::new("square", Transform::IDENTITY).with_mesh(0),
                crate::Node::new(
                    "lid",
                    Transform::new(Vec3::Y * 0.5, Quat::IDENTITY, Vec3::ONE),
                )
                .with_mesh(1),
            ],
            vec![0, 1],
            vec![
                crate::Mesh::new(
                    "square",
                    vec![
                        crate::Primitive::new(
                            vec![
                                Vec3::ZERO,
                                Vec3::X * 0.5,
                                Vec3::new(0.5, 0.5, 0.0),
                                Vec3::Y * 0.5,
                            ],
                            Vec::new(),
                            vec![0, 1, 2, 0, 2, 3],
                        )
                        .with_colors(vec![Vec4::ONE; 4])
                        .with_material(0),
                    ],
                ),
                crate::Mesh::new(
                    "lid",
                    vec![crate::Primitive::new(
                        vec![Vec3::ZERO, Vec3::X * 0.5, Vec3::Y * 0.1],
                        vec![Vec3::Z; 3],
                        vec![0, 1, 2],
                    )],
                ),
            ],
            vec![crate::Material::new("glow", Vec4::ONE).with_emissive(Vec3::splat(0.2))],
            &crate::Limits::default(),
        )
        .unwrap();
        let model = Arc::new(square);
        let gpu = renderer.upload(&device, &queue, &model);
        let mut batch = renderer.batch(&device);
        let view_projection = Mat4::orthographic_rh(0.0, 2.0, 0.0, 2.0, -1.0, 1.0);
        renderer.prepare(&queue, 0, view_projection, Vec3::new(1.0, 1.0, 5.0));
        // Nothing to draw draws nothing.
        renderer.place(&device, &queue, &gpu, &mut batch, &[]);
        assert!(
            drawn(&render(&device, &queue, 4, |pass| renderer
                .draw(pass, 0, &gpu, &batch)))
            .is_empty()
        );
        // Three instances, growing the batch, in one draw each primitive; one without its lid.
        let instances: Vec<Instance> = (0..3)
            .map(|index| {
                let mut instance = Instance::new(model.clone());
                instance.transform =
                    Mat4::from_translation(Vec3::new(index as f32 * 0.6, 0.2, 0.0));
                instance.set_shown("lid", index != 1);
                instance
            })
            .collect();
        renderer.place(&device, &queue, &gpu, &mut batch, &instances);
        assert_eq!(batch.calls.len(), 2);
        assert_eq!(batch.calls[0].instances, 0..3);
        assert_eq!(batch.calls[1].instances, 3..5);
        let pixels = render(&device, &queue, 4, |pass| {
            renderer.draw(pass, 0, &gpu, &batch)
        });
        let lit = |x: f32, y: f32| {
            let (px, py) = projected(view_projection, Vec3::new(x, y, 0.0));
            let at = ((py as u32).min(SIZE - 1) * SIZE + (px as u32).min(SIZE - 1)) as usize * 4;
            pixels[at..at + 3].iter().any(|channel| *channel > 8)
        };
        assert!(lit(0.25, 0.45) && lit(0.85, 0.45) && lit(1.45, 0.45));
        // The second's lid is hidden.
        assert!(lit(0.05, 0.72) && !lit(0.65, 0.72) && lit(1.25, 0.72));
        // Revealed within a sphere about the first, only it is drawn.
        renderer.set_reveal(Some((Vec3::new(0.25, 0.45, 0.0), 0.4)));
        renderer.prepare(&queue, 0, view_projection, Vec3::new(1.0, 1.0, 5.0));
        let revealed = render(&device, &queue, 4, |pass| {
            renderer.draw(pass, 0, &gpu, &batch)
        });
        assert!(drawn(&revealed).len() < drawn(&pixels).len() / 2);
        renderer.set_reveal(Some((Vec3::NAN, 1.0)));
        assert!(renderer.reveal.is_none());
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

#[test]
fn a_texture_s_mipmaps_average_it_down_to_a_texel() {
    let texture = Texture {
        width: 3,
        height: 2,
        pixels: [
            [255, 255, 255, 255],
            [0, 0, 0, 255],
            [255, 255, 255, 0],
            [0, 0, 0, 0],
            [255, 255, 255, 255],
            [0, 0, 0, 255],
        ]
        .concat(),
    };
    let levels = mipmaps(&texture);
    assert_eq!(
        levels.iter().map(|(w, h, _)| (*w, *h)).collect::<Vec<_>>(),
        [(3, 2), (1, 1)]
    );
    // Half white and half black in linear light is brighter than half in sRGB.
    assert!(levels[1].2[0] > 128);
    assert_eq!(levels[1].2[3], 191);
}

#[test]
fn a_textured_model_read_from_a_file_is_drawn() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let renderer = Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, 1);
        let model = Arc::new(crate::gltf::tests::Glb::arm().read_model());
        let gpu = renderer.upload(&device, &queue, &model);
        let mut batch = renderer.batch(&device);
        renderer.place(
            &device,
            &queue,
            &gpu,
            &mut batch,
            &[Instance::new(model.clone())],
        );
        renderer.prepare(
            &queue,
            0,
            Mat4::orthographic_rh(-1.0, 3.0, -1.0, 3.0, -5.0, 5.0),
            Vec3::Z * 5.0,
        );
        // The arm is a line, with no area; drawing it must still be valid.
        render(&device, &queue, 1, |pass| {
            renderer.draw(pass, 0, &gpu, &batch)
        });
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}
