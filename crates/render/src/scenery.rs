// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Scenery drawn natively in each eye's pass, as Sky is, and the sceneries a client can draw, by the identifiers that spaces name their environments by. See `design/environments.md`.
//!
//! The contract is Sky's existing hooks, named for what they do. The Minimal grid is built into the [`Renderer`](crate::graphics::Renderer), and is what a client shows for an environment it cannot draw. An environment's client half, which #75 defines, will register its scenery through the same [`Sceneries`].

use std::sync::Arc;

use glam::{Mat4, Vec3};

use crate::environment::Environment;

/// What a scenery is given when the renderer creates it.
pub struct Target<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub format: wgpu::TextureFormat,
    pub width: u32,
    pub height: u32,
    pub samples: u32,
}

/// Scenery drawn natively in each eye's pass.
pub trait Scenery {
    /// Before the eye's pass: per-eye uniforms, for the eye's view and projection.
    fn prepare(&self, queue: &wgpu::Queue, eye: usize, scene_to_clip: Mat4);
    /// Before the eye's pass: work in its own passes, such as Sky's clouds.
    fn encode(&self, _encoder: &mut wgpu::CommandEncoder, _eye: usize) {}
    /// In the eye's pass, before the scene.
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, eye: usize);
    /// In the eye's pass, after the scene and before the panels.
    fn draw_translucent(&self, _pass: &mut wgpu::RenderPass<'_>, _eye: usize) {}
    /// While a space's scenery grows out of it, only inside this sphere (centre, radius).
    fn set_reveal(&mut self, reveal: Option<(Vec3, f32)>);
    /// The eyes' targets changed size.
    fn resize(&mut self, _device: &wgpu::Device, _width: u32, _height: u32) {}
}

impl Scenery for tomori_environment_sky::Sky {
    fn prepare(&self, queue: &wgpu::Queue, eye: usize, scene_to_clip: Mat4) {
        self.prepare_eye(queue, eye, scene_to_clip);
    }

    fn encode(&self, encoder: &mut wgpu::CommandEncoder, eye: usize) {
        self.encode_clouds(encoder, eye);
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, eye: usize) {
        tomori_environment_sky::Sky::draw(self, pass, eye);
    }

    fn draw_translucent(&self, pass: &mut wgpu::RenderPass<'_>, eye: usize) {
        self.draw_clouds(pass, eye);
    }

    fn set_reveal(&mut self, reveal: Option<(Vec3, f32)>) {
        tomori_environment_sky::Sky::set_reveal(self, reveal);
    }

    fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        tomori_environment_sky::Sky::resize(self, device, width, height);
    }
}

/// Makes a scenery for a renderer's target.
type Create = Arc<dyn Fn(&Target) -> Box<dyn Scenery> + Send + Sync>;

/// The sceneries a client can draw, by the identifiers spaces name them by, in the order they were registered. The Minimal grid is built in, and is not registered.
#[derive(Clone)]
pub struct Sceneries {
    entries: Vec<(Environment, Create)>,
}

/// Tomori's moonlit sky, for `target`.
pub fn sky(target: &Target) -> Box<dyn Scenery> {
    Box::new(tomori_environment_sky::Sky::with_samples(
        target.device,
        target.queue,
        target.format,
        target.width,
        target.height,
        target.samples,
    ))
}

impl Default for Sceneries {
    /// Tomori's own: [`sky`].
    fn default() -> Self {
        let mut sceneries = Self::empty();
        sceneries.register(Environment::SKY.id(), sky);
        sceneries
    }
}

impl Sceneries {
    /// None but the built-in grid.
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Draw `create`'s scenery for the environment `id`, in place of any registered before under the same identifier.
    pub fn register(
        &mut self,
        id: &'static str,
        create: impl Fn(&Target) -> Box<dyn Scenery> + Send + Sync + 'static,
    ) {
        let environment = Environment::new(id);
        self.entries
            .retain(|(registered, _)| *registered != environment);
        self.entries.push((environment, Arc::new(create)));
    }

    /// Whether the environment `id` has a scenery here; the grid's, `minimal`, always does.
    pub fn contains(&self, id: &str) -> bool {
        self.environments()
            .iter()
            .any(|environment| environment.id() == id)
    }

    /// Every environment a client with these sceneries can draw: the grid, then the registered ones in order.
    pub fn environments(&self) -> Vec<Environment> {
        std::iter::once(Environment::MINIMAL)
            .chain(
                self.entries
                    .iter()
                    .map(|(environment, _)| *environment)
                    .filter(|environment| *environment != Environment::MINIMAL),
            )
            .collect()
    }

    /// Make each registered scenery for `target`.
    pub fn create(&self, target: &Target) -> Vec<(Environment, Box<dyn Scenery>)> {
        self.entries
            .iter()
            .map(|(environment, create)| (*environment, create(target)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_is_built_in_and_sceneries_are_kept_in_order_by_identifier() {
        let mut sceneries = Sceneries::empty();
        assert_eq!(sceneries.environments(), [Environment::MINIMAL]);
        assert!(sceneries.contains("minimal"));
        assert!(!sceneries.contains("sky"));
        sceneries.register("floor", sky);
        sceneries.register("sky", sky);
        // Registering an identifier again replaces it, at the end.
        sceneries.register("floor", sky);
        assert_eq!(
            sceneries.environments(),
            [
                Environment::MINIMAL,
                Environment::SKY,
                Environment::new("floor")
            ]
        );
        // A scenery for the grid is drawn with it, and the grid is listed once.
        sceneries.register("minimal", sky);
        assert_eq!(sceneries.environments().len(), 3);
        assert_eq!(Sceneries::default().environments(), Environment::BUILT_IN);
    }
}
