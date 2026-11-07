// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Scenery drawn natively in each eye's pass, as Sky is, and the sceneries a client can draw, by the identifiers that spaces name their environments by. See `design/environments.md`.
//!
//! The contract is Sky's existing hooks, named for what they do. The Minimal grid is built into the [`Renderer`](crate::graphics::Renderer), and is what a client shows for an environment it cannot draw. An environment's client half, which #75 defines, will register its scenery through the same [`Sceneries`].
use std::sync::Arc;

use glam::{Mat4, Vec3};

use crate::{environment::Environment, locomotion::Locomotion};

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

/// The sceneries a client can draw, by the identifiers spaces name them by, in the order they were registered, with the locomotion each environment allows. The Minimal grid is built in, and is not registered.
#[derive(Clone)]
pub struct Sceneries {
    entries: Vec<(Environment, Create)>,
    locomotion: Vec<(Environment, Locomotion)>,
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
            locomotion: Vec::new(),
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

    /// Let participants travel as `locomotion` says in a space bound to the environment `id`, when its scenery is drawn, in place of any declared before for it. An environment declared nothing has none, as Tomori's own do. See `design/locomotion.md`.
    pub fn set_locomotion(&mut self, id: &'static str, locomotion: Locomotion) {
        let environment = Environment::new(id);
        self.locomotion
            .retain(|(declared, _)| *declared != environment);
        self.locomotion.push((environment, locomotion));
    }

    /// The locomotion declared for each environment, for the conversation.
    pub fn locomotion(&self) -> Vec<(Environment, Locomotion)> {
        self.locomotion.clone()
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

/// A scenery for tests that check that a client draws the sceneries it is given, shared with the clients' tests through the `test-gpu` feature.
#[cfg(any(test, feature = "test-gpu"))]
pub mod test {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A scenery that draws nothing, and counts how many renderers created it and how many eyes it was drawn in.
    #[derive(Clone, Default)]
    pub struct Counted {
        created: Arc<AtomicUsize>,
        drawn: Arc<AtomicUsize>,
    }

    impl Counted {
        /// Sceneries with this one alone, for the environment `id`, so that a renderer given any others never creates it.
        pub fn sceneries(&self, id: &'static str) -> Sceneries {
            let mut sceneries = Sceneries::empty();
            let counted = self.clone();
            sceneries.register(id, move |_| counted.create());
            sceneries
        }

        /// This scenery, counted as created once more.
        pub fn create(&self) -> Box<dyn Scenery> {
            self.created.fetch_add(1, Ordering::Relaxed);
            Box::new(self.clone())
        }

        /// How many times a renderer created it.
        pub fn created(&self) -> usize {
            self.created.load(Ordering::Relaxed)
        }

        /// How many eyes it was drawn in.
        pub fn drawn(&self) -> usize {
            self.drawn.load(Ordering::Relaxed)
        }
    }

    impl Scenery for Counted {
        fn prepare(&self, _queue: &wgpu::Queue, _eye: usize, _scene_to_clip: Mat4) {}

        fn draw(&self, _pass: &mut wgpu::RenderPass<'_>, _eye: usize) {
            self.drawn.fetch_add(1, Ordering::Relaxed);
        }

        fn set_reveal(&mut self, _reveal: Option<(Vec3, f32)>) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locomotion_is_declared_by_identifier_and_tomori_s_own_declare_none() {
        use crate::locomotion::Surface;
        assert!(Sceneries::default().locomotion().is_empty());
        let mut sceneries = Sceneries::empty();
        let teleport = Locomotion::teleport(vec![Surface::disc(Vec3::ZERO, 3.0)]);
        sceneries.set_locomotion("floor", Locomotion::fly(Vec::new(), 10.0));
        sceneries.set_locomotion("deck", teleport.clone());
        // Declaring an identifier again replaces it, at the end.
        sceneries.set_locomotion("floor", teleport.clone());
        assert_eq!(
            sceneries.locomotion(),
            [
                (Environment::new("deck"), teleport.clone()),
                (Environment::new("floor"), teleport)
            ]
        );
    }

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

    #[test]
    fn a_renderer_creates_each_scenery_it_is_given_and_draws_the_current_environments() {
        use crate::graphics::Renderer;
        let counted = test::Counted::default();
        let floor = Environment::new("floor");
        crate::test_gpu::with(|crate::test_gpu::Gpu { device, queue, .. }| {
            let format = wgpu::TextureFormat::Rgba8UnormSrgb;
            let mut renderer = Renderer::with_sceneries(
                &device,
                &queue,
                format,
                8,
                8,
                1,
                &counted.sceneries("floor"),
            );
            assert_eq!(counted.created(), 1);
            assert_eq!(renderer.environments(), [Environment::MINIMAL, floor]);
            let views: Vec<_> = (0..2)
                .map(|_| {
                    device
                        .create_texture(&wgpu::TextureDescriptor {
                            label: Some("counted scenery eye"),
                            size: wgpu::Extent3d {
                                width: 8,
                                height: 8,
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
            let render = |renderer: &Renderer| {
                renderer.render(
                    &device,
                    &queue,
                    &views,
                    &[Mat4::IDENTITY; 2],
                    Mat4::IDENTITY,
                    &[],
                );
            };
            // The grid has no scenery of its own.
            render(&renderer);
            assert_eq!(counted.drawn(), 0);
            renderer.set_scenery(crate::arrival_motion::Scenery {
                environment: floor,
                reveal: Some(crate::arrival_motion::Sphere {
                    centre: Vec3::ZERO,
                    radius: 1.0,
                }),
            });
            render(&renderer);
            assert_eq!(counted.drawn(), 2);
        });
    }
}
