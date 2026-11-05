// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The server-less window: the window's contents, as `desktop_window.rs` shows them. See `design/desktop.md`. The spectator's window is `tomori-client-spectator`'s.

use crate::{
    graphics::Renderer,
    pointing::Ray,
    swarm::{CENTER, Swarm},
    window::{Contents, Flow, Gpu, Input, Settings, Surface, Wake},
};
use anyhow::Result;
use glam::{Mat4, Vec2, Vec3};
use std::time::Instant;
use tomori_render::scenery::Sceneries;
use winit::{
    dpi::LogicalSize,
    event::{MouseButton, MouseScrollDelta},
    keyboard::KeyCode,
};

const TITLE: &str =
    "Tomori | LMB attract · RMB orbit · wheel zoom · E environment · R reset · Space pause";
const PAUSED_TITLE: &str = "Tomori [paused] — Space to resume";

/// The server-less window, where the mouse attracts and orbits the swarm.
pub struct Demo {
    renderer: Renderer,
    device: wgpu::Device,
    queue: wgpu::Queue,
    width: u32,
    height: u32,
    swarm: Swarm,
    camera: Camera,
    cursor: Vec2,
    attracting: bool,
    orbiting: bool,
    paused: bool,
    occluded: bool,
    last_frame: Instant,
    first_frame: bool,
}

impl Demo {
    /// The server-less window.
    pub fn settings() -> Settings {
        Settings {
            title: TITLE,
            decorations: true,
            size: LogicalSize::new(1100, 760).into(),
            min_size: None,
        }
    }

    /// Show the server-less swarm, drawn with `gpu`, among Tomori's own sceneries.
    pub fn new(gpu: Gpu) -> Self {
        Self::with_sceneries(gpu, &Sceneries::default())
    }

    /// As [`Demo::new`], among `sceneries`, which **E** cycles through.
    pub fn with_sceneries(gpu: Gpu, sceneries: &Sceneries) -> Self {
        let samples = gpu.samples(gpu.format);
        let mut renderer = Renderer::with_sceneries(
            &gpu.device,
            &gpu.queue,
            gpu.format,
            gpu.width,
            gpu.height,
            samples,
            sceneries,
        );
        // The desktop window has no server, so it is never in a space and chooses its scenery itself.
        let initial = crate::environment::Environment::initial(renderer.environments());
        renderer.set_environment(initial);
        log::info!(
            "Desktop adapter: {} — {}x{}, {samples}x multisampling",
            gpu.adapter.get_info().name,
            gpu.width,
            gpu.height
        );
        log::info!(
            "Left mouse: attract; right drag: orbit; wheel: zoom; R: reset; Space: pause; Esc: quit."
        );
        Self {
            renderer,
            device: gpu.device,
            queue: gpu.queue,
            width: gpu.width,
            height: gpu.height,
            swarm: Swarm::default(),
            camera: Camera::default(),
            cursor: Vec2::new(gpu.width as f32, gpu.height as f32) * 0.5,
            attracting: false,
            orbiting: false,
            paused: false,
            occluded: false,
            last_frame: Instant::now(),
            first_frame: true,
        }
    }

    /// Whether nothing drawn to the window would be seen.
    fn hidden(&self) -> bool {
        self.occluded || self.width == 0 || self.height == 0
    }

    fn resize(&mut self, width: u32, height: u32, surface: &mut dyn Surface) {
        self.width = width;
        self.height = height;
        self.last_frame = Instant::now();
        if width == 0 || height == 0 {
            return;
        }
        surface.resize(width, height);
        self.renderer.resize(&self.device, width, height);
        surface.request_redraw();
    }

    fn draw(&mut self, surface: &mut dyn Surface) -> Result<()> {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        let Some(target) = surface.acquire()? else {
            return Ok(());
        };
        let size = Vec2::new(self.width as f32, self.height as f32);
        let matrix = self.camera.view_projection(size.x / size.y);
        let attractor = self
            .attracting
            .then(|| self.camera.attractor(self.cursor, size));
        if !self.paused {
            self.swarm.step(dt, attractor);
        }
        self.renderer
            .update(&self.queue, &self.swarm, attractor, self.attracting);
        self.renderer.render(
            &self.device,
            &self.queue,
            &[target],
            &[matrix],
            Mat4::IDENTITY,
            &[],
        );
        surface.present();
        if self.first_frame {
            log::info!("Presented first desktop frame");
            self.first_frame = false;
        }
        Ok(())
    }
}

impl Contents for Demo {
    fn input(&mut self, input: Input, surface: &mut dyn Surface) -> Result<Flow> {
        match input {
            Input::Close => return Ok(Flow::Exit),
            Input::Resized { width, height } => self.resize(width, height, surface),
            Input::Occluded(occluded) => {
                self.occluded = occluded;
                self.last_frame = Instant::now();
                if !occluded {
                    surface.request_redraw();
                }
            }
            Input::Focused(false) | Input::CursorLeft => {
                self.attracting = false;
                self.orbiting = false;
            }
            Input::CursorMoved(cursor) => {
                if self.orbiting {
                    self.camera.orbit(cursor - self.cursor);
                }
                self.cursor = cursor;
            }
            Input::Mouse { button, pressed } => match button {
                MouseButton::Left => self.attracting = pressed,
                MouseButton::Right => self.orbiting = pressed,
                _ => {}
            },
            Input::Wheel(delta) => {
                self.camera.zoom(match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 0.12,
                    MouseScrollDelta::PixelDelta(position) => position.y as f32 * 0.002,
                });
            }
            Input::Key {
                code,
                pressed: true,
                repeat: false,
            } => match code {
                KeyCode::Escape => return Ok(Flow::Exit),
                KeyCode::KeyR => self.swarm = Swarm::default(),
                KeyCode::KeyE => {
                    self.renderer.set_environment(
                        self.renderer
                            .environment()
                            .next(self.renderer.environments()),
                    );
                    log::info!("Environment: {:?}", self.renderer.environment());
                }
                KeyCode::Space => {
                    self.paused = !self.paused;
                    surface.set_title(if self.paused { PAUSED_TITLE } else { TITLE });
                }
                _ => {}
            },
            _ => {}
        }
        Ok(Flow::Continue)
    }

    fn redraw(&mut self, surface: &mut dyn Surface) -> Result<()> {
        if self.hidden() {
            return Ok(());
        }
        self.draw(surface)?;
        surface.request_redraw();
        Ok(())
    }

    fn about_to_wait(&mut self, _surface: &mut dyn Surface) -> Result<Wake> {
        Ok(Wake::Wait)
    }
}

struct Camera {
    yaw: f32,
    pitch: f32,
    distance: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.18,
            distance: 3.5,
        }
    }
}

impl Camera {
    fn position(&self) -> Vec3 {
        CENTER
            + self.distance
                * Vec3::new(
                    self.yaw.sin() * self.pitch.cos(),
                    self.pitch.sin(),
                    self.yaw.cos() * self.pitch.cos(),
                )
    }

    fn view_projection(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_rh(60_f32.to_radians(), aspect, 0.05, 50.0)
            * Mat4::look_at_rh(self.position(), CENTER, Vec3::Y)
    }

    fn orbit(&mut self, delta: Vec2) {
        self.yaw -= delta.x * 0.006;
        self.pitch = (self.pitch + delta.y * 0.006).clamp(-1.2, 1.2);
    }

    fn zoom(&mut self, amount: f32) {
        self.distance = (self.distance * (-amount).exp()).clamp(1.0, 10.0);
    }

    /// The ray from the camera through the cursor, for pointing with the mouse as a controller points on the headset.
    fn ray(&self, cursor: Vec2, size: Vec2) -> Option<Ray> {
        let ndc = Vec2::new(cursor.x / size.x * 2.0 - 1.0, 1.0 - cursor.y / size.y * 2.0);
        let inverse = self.view_projection(size.x / size.y).inverse();
        let near = inverse.project_point3(ndc.extend(0.0));
        let far = inverse.project_point3(ndc.extend(1.0));
        Ray::new(near, far - near)
    }

    fn attractor(&self, cursor: Vec2, size: Vec2) -> Vec3 {
        let Some(ray) = self.ray(cursor, size) else {
            return CENTER;
        };
        let normal = (self.position() - CENTER).normalize();
        // Intersect a camera-facing plane through the swarm's center.
        ray.at((CENTER - ray.origin).dot(normal) / ray.direction.dot(normal))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::test::{TestSurface, with_gpu};
    use winit::dpi::{PhysicalPosition, Size};

    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

    fn input(demo: &mut Demo, surface: &mut TestSurface, input: Input) -> Flow {
        demo.input(input, surface).unwrap()
    }

    fn key(code: KeyCode) -> Input {
        Input::Key {
            code,
            pressed: true,
            repeat: false,
        }
    }

    #[test]
    fn the_server_less_window_has_decorations_at_its_logical_size() {
        let window = Demo::settings();
        assert_eq!(window.title, TITLE);
        assert!(window.decorations);
        assert_eq!(window.size, Size::Logical(LogicalSize::new(1100.0, 760.0)));
        assert_eq!(window.min_size, None);
    }

    #[test]
    fn the_server_less_window_draws_whenever_it_is_seen() {
        with_gpu(FORMAT, |gpu| {
            let mut surface = TestSurface::new(&gpu.device, FORMAT);
            let mut demo = Demo::new(gpu);
            demo.redraw(&mut surface).unwrap();
            assert_eq!((surface.presented, surface.redraws), (1, 1));
            assert_eq!(demo.about_to_wait(&mut surface).unwrap(), Wake::Wait);
            // Hidden, it draws nothing, and asks to draw again once it is seen.
            input(&mut demo, &mut surface, Input::Occluded(true));
            demo.redraw(&mut surface).unwrap();
            assert_eq!((surface.presented, surface.redraws), (1, 1));
            input(&mut demo, &mut surface, Input::Occluded(false));
            assert_eq!(surface.redraws, 2);
            // Minimised, too.
            input(
                &mut demo,
                &mut surface,
                Input::Resized {
                    width: 0,
                    height: 48,
                },
            );
            demo.redraw(&mut surface).unwrap();
            assert_eq!(surface.presented, 1);
            assert!(surface.sizes.is_empty());
            input(
                &mut demo,
                &mut surface,
                Input::Resized {
                    width: 64,
                    height: 48,
                },
            );
            assert_eq!(surface.sizes, [(64, 48)]);
            // Nothing to draw into this time: nothing is presented, and it asks again.
            surface.available = false;
            demo.redraw(&mut surface).unwrap();
            assert_eq!((surface.presented, surface.redraws), (1, 4));
            surface.fail = true;
            assert!(demo.redraw(&mut surface).is_err());
        });
    }

    #[test]
    fn the_mouse_orbits_zooms_and_attracts_and_keys_reset_change_and_pause() {
        with_gpu(FORMAT, |gpu| {
            let mut surface = TestSurface::new(&gpu.device, FORMAT);
            let mut demo = Demo::new(gpu);
            let start = Vec2::new(10.0, 10.0);
            input(&mut demo, &mut surface, Input::CursorMoved(start));
            assert_eq!(demo.cursor, start);
            input(
                &mut demo,
                &mut surface,
                Input::Mouse {
                    button: MouseButton::Right,
                    pressed: true,
                },
            );
            let yaw = demo.camera.yaw;
            input(
                &mut demo,
                &mut surface,
                Input::CursorMoved(Vec2::new(60.0, 10.0)),
            );
            assert_ne!(demo.camera.yaw, yaw);
            input(
                &mut demo,
                &mut surface,
                Input::Mouse {
                    button: MouseButton::Left,
                    pressed: true,
                },
            );
            input(
                &mut demo,
                &mut surface,
                Input::Mouse {
                    button: MouseButton::Middle,
                    pressed: true,
                },
            );
            assert!(demo.attracting && demo.orbiting);
            // Attracting while drawn moves the swarm toward the cursor.
            demo.redraw(&mut surface).unwrap();
            input(&mut demo, &mut surface, Input::Focused(true));
            assert!(demo.attracting && demo.orbiting);
            input(&mut demo, &mut surface, Input::Focused(false));
            assert!(!demo.attracting && !demo.orbiting);
            demo.attracting = true;
            input(&mut demo, &mut surface, Input::CursorLeft);
            assert!(!demo.attracting);
            let distance = demo.camera.distance;
            input(
                &mut demo,
                &mut surface,
                Input::Wheel(MouseScrollDelta::LineDelta(0.0, 1.0)),
            );
            assert!(demo.camera.distance < distance);
            let distance = demo.camera.distance;
            input(
                &mut demo,
                &mut surface,
                Input::Wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
                    0.0, -100.0,
                ))),
            );
            assert!(demo.camera.distance > distance);
            // R resets the swarm.
            demo.swarm.step(0.5, Some(Vec3::ONE));
            let moved = demo.swarm.agents[0].position;
            input(&mut demo, &mut surface, key(KeyCode::KeyR));
            assert_ne!(demo.swarm.agents[0].position, moved);
            // E changes the environment.
            let environment = demo.renderer.environment();
            input(&mut demo, &mut surface, key(KeyCode::KeyE));
            assert_eq!(
                demo.renderer.environment(),
                environment.next(demo.renderer.environments())
            );
            // Space pauses, and says so in the title; a paused swarm stays where it is.
            input(&mut demo, &mut surface, key(KeyCode::Space));
            assert!(demo.paused);
            let still = demo.swarm.agents[0].position;
            demo.redraw(&mut surface).unwrap();
            assert_eq!(demo.swarm.agents[0].position, still);
            input(&mut demo, &mut surface, key(KeyCode::Space));
            assert_eq!(surface.titles, [PAUSED_TITLE, TITLE]);
            // Held and released keys, and other keys, do nothing.
            for ignored in [
                Input::Key {
                    code: KeyCode::Space,
                    pressed: true,
                    repeat: true,
                },
                Input::Key {
                    code: KeyCode::Space,
                    pressed: false,
                    repeat: false,
                },
                key(KeyCode::KeyQ),
            ] {
                assert_eq!(input(&mut demo, &mut surface, ignored), Flow::Continue);
            }
            assert!(!demo.paused);
            assert_eq!(
                input(&mut demo, &mut surface, key(KeyCode::Escape)),
                Flow::Exit
            );
            assert_eq!(input(&mut demo, &mut surface, Input::Close), Flow::Exit);
        });
    }

    #[test]
    fn center_cursor_targets_swarm_even_after_orbit_and_zoom() {
        let mut camera = Camera::default();
        let size = Vec2::new(1100.0, 760.0);
        for delta in [
            Vec2::ZERO,
            Vec2::new(230.0, -100.0),
            Vec2::new(-400.0, 300.0),
        ] {
            camera.orbit(delta);
            camera.zoom(0.4);
            assert!(camera.attractor(size * 0.5, size).distance(CENTER) < 0.001);
        }
    }

    #[test]
    fn cursor_motion_maps_right_and_up_on_the_attraction_plane() {
        let camera = Camera::default();
        let size = Vec2::new(1100.0, 760.0);
        let center = size * 0.5;
        let right = camera.attractor(center + Vec2::new(100.0, 0.0), size);
        let up = camera.attractor(center - Vec2::new(0.0, 100.0), size);
        let normal = (camera.position() - CENTER).normalize();
        assert!(right.x > CENTER.x);
        assert!(up.y > CENTER.y);
        assert!((right - CENTER).dot(normal).abs() < 0.001);
        assert!((up - CENTER).dot(normal).abs() < 0.001);
    }

    #[test]
    fn the_mouse_points_at_what_is_under_the_cursor() {
        use crate::pointing::{Pointer, Shape, Target};
        let mut camera = Camera::default();
        camera.orbit(Vec2::new(120.0, -40.0));
        let size = Vec2::new(1100.0, 760.0);
        let target = Target {
            key: "swarm",
            shape: Shape::Sphere {
                center: CENTER,
                radius: 0.1,
            },
        };
        let mut pointer = Pointer::default();
        let centre = camera.ray(size * 0.5, size);
        let pointing = pointer.update(centre, Some(1.0), [&target]);
        assert_eq!(pointing.chosen, Some("swarm"));
        let corner = camera.ray(Vec2::ZERO, size);
        assert_eq!(pointer.update(corner, Some(0.0), [&target]).hit, None);
    }

    #[test]
    fn a_cursor_that_points_nowhere_attracts_to_the_centre() {
        let camera = Camera::default();
        assert_eq!(camera.attractor(Vec2::NAN, Vec2::splat(100.0)), CENTER);
    }

    #[test]
    fn camera_stays_finite_at_input_limits() {
        let mut camera = Camera::default();
        for amount in [-100.0, 100.0] {
            camera.zoom(amount);
            camera.orbit(Vec2::splat(amount * 100.0));
            assert!((1.0..=10.0).contains(&camera.distance));
            assert!(camera.view_projection(1.0).is_finite());
            for cursor in [Vec2::ZERO, Vec2::splat(1000.0)] {
                assert!(camera.attractor(cursor, Vec2::splat(1000.0)).is_finite());
            }
        }
    }
}
