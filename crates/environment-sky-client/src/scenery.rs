// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Sky as a scenery that `tomori-render` draws in each eye's pass, and its registration with a client's sceneries at Sky's identifier and version. See `design/environments.md`.
use glam::{Mat4, Vec3};
use tomori_render::scenery::{Sceneries, Scenery, Target};

use crate::Sky;

impl Scenery for Sky {
    fn prepare(&self, queue: &wgpu::Queue, eye: usize, scene_to_clip: Mat4) {
        self.prepare_eye(queue, eye, scene_to_clip);
    }

    fn encode(&self, encoder: &mut wgpu::CommandEncoder, eye: usize) {
        self.encode_clouds(encoder, eye);
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, eye: usize) {
        Sky::draw(self, pass, eye);
    }

    fn draw_translucent(&self, pass: &mut wgpu::RenderPass<'_>, eye: usize) {
        self.draw_clouds(pass, eye);
    }

    fn set_reveal(&mut self, reveal: Option<(Vec3, f32)>) {
        Sky::set_reveal(self, reveal);
    }

    fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        Sky::resize(self, device, width, height);
    }
}

/// Tomori's moonlit sky, for `target`.
pub fn scenery(target: &Target) -> Box<dyn Scenery> {
    Box::new(Sky::with_samples(
        target.device,
        target.queue,
        target.format,
        target.width,
        target.height,
        target.samples,
    ))
}

/// Draw Sky for spaces bound to it, at [`crate::ID`] and [`crate::VERSION`], which a server registering Sky by the same identity offers it at, in place of any scenery registered for it before. It declares no locomotion.
///
/// ```
/// let mut sceneries = tomori_render::scenery::Sceneries::empty();
/// tomori_environment_sky_client::register(&mut sceneries);
/// assert!(sceneries.contains(tomori_environment_sky_client::ID));
/// ```
pub fn register(sceneries: &mut Sceneries) {
    sceneries.register_versioned(crate::ID, crate::VERSION, scenery);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sky_is_registered_at_its_identity_and_declares_no_locomotion() {
        let mut sceneries = Sceneries::empty();
        register(&mut sceneries);
        let environments: Vec<(&str, u32)> = sceneries
            .environments()
            .into_iter()
            .map(|environment| (environment.id(), environment.version()))
            .collect();
        assert_eq!(
            environments,
            [
                (tomori_protocol::GRID, tomori_protocol::GRID_VERSION),
                ("sky", 1)
            ]
        );
        assert!(sceneries.locomotion().is_empty());
    }

    #[test]
    fn a_build_may_declare_teleport_pads_beside_sky() {
        use tomori_scene::locomotion::{Locomotion, Pad, Surface};
        // A floor, racks 4.5 m below it, and a pair of pads between them, declared in Sky's own coordinates, as a build
        // of another project declares them for its own environment.
        let floors = Locomotion::fly(
            vec![
                Surface::rectangle(Vec3::ZERO, glam::Vec2::new(12.0, 12.0)),
                Surface::rectangle(Vec3::new(0.0, -4.5, 0.0), glam::Vec2::new(12.0, 12.0)),
            ],
            6.0,
        )
        .with_pads([[
            Pad::at(Vec3::new(2.0, 0.0, -3.0)).named("Main floor"),
            Pad::at(Vec3::new(2.0, -4.5, -3.0))
                .named("Racks")
                .facing(Vec3::NEG_Z),
        ]]);
        let mut sceneries = Sceneries::empty();
        register(&mut sceneries);
        sceneries.set_locomotion(crate::ID, floors.clone());
        let declared = sceneries.locomotion();
        assert_eq!(declared.len(), 1);
        assert_eq!(declared[0].0.id(), crate::ID);
        let [floor, racks] = declared[0].1.pads().next().expect("the pair is used");
        assert_eq!(
            (floor.name(), racks.name()),
            (Some("Main floor"), Some("Racks"))
        );
        assert_eq!(declared[0].1, floors);
    }

    #[test]
    fn sky_is_drawn_by_a_renderer_given_it() {
        tomori_render::test_gpu::with(|gpu| {
            let mut sceneries = Sceneries::empty();
            register(&mut sceneries);
            let created = sceneries.create(&Target::new(
                &gpu.device,
                &gpu.queue,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                32,
                32,
                1,
                tomori_render::glyphs::SharedGlyphs::new(&gpu.device),
            ));
            assert_eq!(created.len(), 1);
            let (environment, mut sky) = created.into_iter().next().unwrap();
            assert_eq!(environment.id(), crate::ID);
            sky.set_reveal(Some((Vec3::ZERO, 1.0)));
            sky.resize(&gpu.device, 16, 16);
            sky.prepare(&gpu.queue, 0, Mat4::IDENTITY);
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            sky.encode(&mut encoder, 0);
            gpu.queue.submit([encoder.finish()]);
        });
    }
}
