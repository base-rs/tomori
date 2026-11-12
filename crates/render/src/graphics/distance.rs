// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tomori's cubes seen from afar: a particle is drawn at least a pixel across, and the swarm keeps its light from 2 m to
//! 20 m, rather than fading into the dark as it once did. Set `TOMORI_SWARM_DISTANCE_PREVIEW` to a PNG path to see the
//! swarm at each distance. See `design/rendering.md`.
use super::*;
use crate::test_gpu;

/// The linear luminance of an sRGB pixel.
fn luminance(pixel: &image::Rgba<u8>) -> f32 {
    let linear = |channel: u8| {
        let value = f32::from(channel) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(pixel[0]) + 0.7152 * linear(pixel[1]) + 0.0722 * linear(pixel[2])
}

/// What `renderer` draws of `swarm` through `camera` into `texture`, `size`, read back.
fn capture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut Renderer,
    texture: &wgpu::Texture,
    size: (u32, u32),
    swarm: &Swarm,
    camera: Mat4,
) -> image::RgbaImage {
    let view = texture.create_view(&Default::default());
    renderer.update(queue, swarm, None, false);
    renderer.render(
        device,
        queue,
        std::slice::from_ref(&view),
        &[camera],
        Mat4::IDENTITY,
        &[],
    );
    crate::test_support::read_back(device, queue, texture, size)
}

fn target(device: &wgpu::Device, (width, height): (u32, u32)) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("distance test target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// How much of a cube's own colour the fade with distance leaves at least, as `swarm.wgsl` says.
const FADE_FLOOR: f32 = 0.9;

#[test]
fn a_particle_far_away_is_drawn_a_pixel_across_adding_the_light_it_would() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let size = (256, 256);
        let texture = target(&device, size);
        // One sample a pixel, which a particle smaller than a pixel falls between the most easily.
        let mut renderer = Renderer::new(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            size.0,
            size.1,
        );
        // One particle straight ahead of an eye standing 10 m along +Z, a metre away and then 13 m.
        let fov = 1.0f32;
        let eye = glam::Vec3::new(0.0, 1.6, 10.0);
        let camera = glam::camera::rh::proj::directx::perspective(fov, 1.0, 0.05, 200.0)
            * glam::camera::rh::view::look_at_mat4(eye, eye - glam::Vec3::Z, glam::Vec3::Y);
        let mut empty = Swarm::default();
        empty.agents.clear();
        let nothing = capture(
            &device,
            &queue,
            &mut renderer,
            &texture,
            size,
            &empty,
            camera,
        );
        let mut particle = |distance: f32| {
            let mut swarm = Swarm::default();
            swarm.agents = vec![crate::swarm::Agent {
                position: eye - glam::Vec3::Z * distance,
                velocity: glam::Vec3::ZERO,
            }];
            let image = capture(
                &device,
                &queue,
                &mut renderer,
                &texture,
                size,
                &swarm,
                camera,
            );
            let lit = image
                .pixels()
                .zip(nothing.pixels())
                .filter(|(pixel, empty)| pixel != empty)
                .count();
            let light: f32 = image
                .pixels()
                .zip(nothing.pixels())
                .map(|(pixel, empty)| luminance(pixel) - luminance(empty))
                .sum();
            (lit, light)
        };
        // How many pixels its face, 2.8 cm square, spans at a distance, by the projection alone.
        let spans =
            |distance: f32| 0.028 / (fov * 0.5).tan() / (distance - 0.014) * size.1 as f32 * 0.5;
        // Near, it is drawn as large as it is, 6.6 pixels across, not grown.
        let (near, near_light) = particle(1.0);
        let side = spans(1.0);
        assert!(
            (near as f32 - side * side).abs() <= 2.0 * side + 1.0,
            "{near} pixels for a side of {side}"
        );
        // Far, where it would span half a pixel and might fall between pixels' samples, it is drawn a pixel across
        // and dimmed as it grew, adding about the light it would: what it adds near, by the area it covers, faded by
        // at most a tenth.
        assert!((0.4..0.6).contains(&spans(13.0)));
        let (far, far_light) = particle(13.0);
        assert!((1..=4).contains(&far), "{far} pixels");
        let expected = near_light * (spans(13.0) / side).powi(2) * FADE_FLOOR / (-0.025f32).exp();
        assert!(
            (far_light / expected - 1.0).abs() < 0.35,
            "{far_light} far, {expected} expected"
        );
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

#[test]
fn the_swarm_keeps_its_light_from_2_to_20_metres() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let size = (1280, 720);
        let texture = target(&device, size);
        // As the desktop window draws it, with 4× multisampling and a 60° view.
        let mut renderer = Renderer::with_samples(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            size.0,
            size.1,
            4,
        );
        let swarm = Swarm::default();
        let mut empty = Swarm::default();
        empty.agents.clear();
        let middle = crate::swarm::CENTER;
        let mut light = Vec::new();
        let mut sheet = image::RgbaImage::new(size.0 * 2, size.1 * 2);
        for (index, distance) in [2.0f32, 5.0, 10.0, 20.0].into_iter().enumerate() {
            let camera = glam::camera::rh::proj::directx::perspective(
                60f32.to_radians(),
                size.0 as f32 / size.1 as f32,
                0.05,
                50.0,
            ) * glam::camera::rh::view::look_at_mat4(
                middle + glam::Vec3::new(0.0, 0.2, distance),
                middle,
                glam::Vec3::Y,
            );
            let image = capture(
                &device,
                &queue,
                &mut renderer,
                &texture,
                size,
                &swarm,
                camera,
            );
            let without = capture(
                &device,
                &queue,
                &mut renderer,
                &texture,
                size,
                &empty,
                camera,
            );
            // The light the swarm adds to the view, as its distance squared shrinks what it covers.
            let added: f32 = image
                .pixels()
                .zip(without.pixels())
                .map(|(with, without)| luminance(with) - luminance(without))
                .sum();
            light.push(added * distance * distance);
            image::imageops::replace(
                &mut sheet,
                &image,
                i64::from(index as u32 % 2 * size.0),
                i64::from(index as u32 / 2 * size.1),
            );
        }
        crate::test_support::preview("TOMORI_SWARM_DISTANCE_PREVIEW", &sheet);
        // Its light, in all, is about the same from 5 m to 20 m, within a fifth, where the fade once took nearly all of it
        // by 10 m; at 2 m it is more, as the cloud's nearer side is nearer still and its cubes' sides are seen, but 5 m
        // keeps well over half of it, where the fade once took half by itself.
        let [near, five, ten, twenty] = light[..] else {
            unreachable!()
        };
        assert!(five / near > 0.6, "{light:?}");
        assert!(
            [ten, twenty]
                .iter()
                .all(|light| (light / five - 1.0).abs() < 0.2),
            "{light:?}"
        );
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}
