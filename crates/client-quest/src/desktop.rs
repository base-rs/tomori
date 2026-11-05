// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The server-less window, and the spectator's: the window's contents, as `desktop_window.rs` shows them. See `design/desktop.md` and `design/spectator.md`.

use crate::{
    graphics::Renderer,
    pipewire_video::{self, Producer, Video},
    pointing::Ray,
    spectator_feed::Feed,
    spectator_view::{RenderInputs, SpectatorView},
    swarm::{CENTER, Swarm},
    window::{Contents, Flow, Gpu, Input, Settings, Surface, Wake},
};
use anyhow::Result;
use glam::{Mat4, Vec2, Vec3};
use std::time::{Duration, Instant};
use winit::{
    dpi::{LogicalSize, PhysicalSize, Size},
    event::{MouseButton, MouseScrollDelta},
    keyboard::KeyCode,
};

const TITLE: &str =
    "Tomori | LMB attract · RMB orbit · wheel zoom · E environment · R reset · Space pause";
const PAUSED_TITLE: &str = "Tomori [paused] — Space to resume";
const SPECTATOR_TITLE: &str = "Tomori | Quest spectator (OBS)";
const SPECTATOR_WIDTH: u32 = 1920;
const SPECTATOR_HEIGHT: u32 = 1080;

fn initial_window_size(spectator: bool) -> Size {
    if spectator {
        // Request physical render pixels, not logical points, so desktop DPI
        // scaling doesn't silently change the intended OBS capture resolution.
        PhysicalSize::new(SPECTATOR_WIDTH, SPECTATOR_HEIGHT).into()
    } else {
        LogicalSize::new(1100, 760).into()
    }
}

/// The server-less window, where the mouse attracts and orbits the swarm; or, with a feed, the spectator's borderless window, following a participant in a live space; and, with a PipeWire producer, the spectator's video source with this window as its preview.
pub(crate) struct Demo {
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
    spectator: Option<SpectatorView>,
    video: Option<Video>,
    /// When the PipeWire source draws next: it is driven by a timer, even while the window is hidden.
    next_video_frame: Option<Instant>,
}

impl Demo {
    /// The window for the spectator, as `spectator` says, or the server-less window.
    pub fn settings(spectator: bool) -> Settings {
        Settings {
            title: if spectator { SPECTATOR_TITLE } else { TITLE },
            // Escape closes the spectator's borderless window.
            decorations: !spectator,
            size: initial_window_size(spectator),
            // Keep the content itself at least 1080p even when the desktop work area excludes a panel.
            min_size: spectator
                .then(|| PhysicalSize::new(SPECTATOR_WIDTH, SPECTATOR_HEIGHT).into()),
        }
    }

    /// Show the spectator's view of `feed`, or the server-less swarm without one, drawn with `gpu`; and publish it through `producer` too, when there is one.
    pub fn new(gpu: Gpu, feed: Option<Feed>, producer: Option<Producer>) -> Self {
        let render_format = if producer.is_some() {
            pipewire_video::FORMAT
        } else {
            gpu.format
        };
        let samples = gpu.samples(render_format);
        let video = producer.map(|producer| Video::new(&gpu.device, Some(gpu.format), producer));
        let (render_width, render_height) = if video.is_some() {
            (pipewire_video::WIDTH, pipewire_video::HEIGHT)
        } else {
            (gpu.width, gpu.height)
        };
        let mut renderer = Renderer::with_samples(
            &gpu.device,
            &gpu.queue,
            render_format,
            render_width,
            render_height,
            samples,
        );
        // The desktop window has no server, so it is never in a space and chooses its scenery itself.
        renderer.set_environment(crate::environment::Environment::initial());
        let spectator = feed.map(SpectatorView::new);
        log::info!(
            "Desktop adapter: {} — {}x{}, {samples}x multisampling",
            gpu.adapter.get_info().name,
            gpu.width,
            gpu.height
        );
        if spectator.is_some() {
            log::info!("Spectator: Esc closes the borderless window.");
        } else {
            log::info!(
                "Left mouse: attract; right drag: orbit; wheel: zoom; R: reset; Space: pause; Esc: quit."
            );
        }
        let next_video_frame = video.as_ref().map(|_| Instant::now());
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
            spectator,
            video,
            next_video_frame,
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
        if self.video.is_some() {
            log::info!(
                "Spectator preview size: {width}x{height} physical pixels (PipeWire output remains {}x{})",
                pipewire_video::WIDTH,
                pipewire_video::HEIGHT
            );
        } else if self.spectator.is_some() {
            log::info!("Spectator render size: {width}x{height} physical pixels");
        }
        surface.resize(width, height);
        if self.video.is_none() {
            self.renderer.resize(&self.device, width, height);
        }
        surface.request_redraw();
    }

    fn draw(&mut self, surface: &mut dyn Surface) -> Result<()> {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        let frame = if self.hidden() {
            None
        } else {
            surface.acquire()?
        };
        let (target, size) = match (&self.video, &frame) {
            (Some(video), _) => (
                video.view.clone(),
                Vec2::new(pipewire_video::WIDTH as f32, pipewire_video::HEIGHT as f32),
            ),
            (None, Some(frame)) => (
                frame.clone(),
                Vec2::new(self.width as f32, self.height as f32),
            ),
            (None, None) => return Ok(()),
        };
        if let Some(spectator) = &mut self.spectator {
            spectator.render(RenderInputs {
                device: &self.device,
                queue: &self.queue,
                renderer: &mut self.renderer,
                swarm: &mut self.swarm,
                target,
                aspect: size.x / size.y,
            });
        } else {
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
        }
        if let Some(video) = &mut self.video {
            if let Err(error) = video.capture(&self.device, &self.queue) {
                log::error!("PipeWire output stopped: {error:#}; preview remains available");
            }
            if let Some(preview) = &frame {
                video.preview(&self.device, &self.queue, preview);
            }
        }
        if frame.is_some() {
            surface.present();
        }
        if self.first_frame {
            log::info!("Presented first desktop frame");
            self.first_frame = false;
        }
        Ok(())
    }
}

impl Contents for Demo {
    fn input(&mut self, input: Input, surface: &mut dyn Surface) -> Result<Flow> {
        // The spectator's window only follows its feed; Escape closes it.
        let interactive = self.spectator.is_none();
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
            Input::CursorMoved(cursor) if interactive => {
                if self.orbiting {
                    self.camera.orbit(cursor - self.cursor);
                }
                self.cursor = cursor;
            }
            Input::Mouse { button, pressed } if interactive => match button {
                MouseButton::Left => self.attracting = pressed,
                MouseButton::Right => self.orbiting = pressed,
                _ => {}
            },
            Input::Wheel(delta) if interactive => {
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
                KeyCode::KeyR if interactive => self.swarm = Swarm::default(),
                KeyCode::KeyE if interactive => {
                    self.renderer
                        .set_environment(self.renderer.environment().next());
                    log::info!("Environment: {:?}", self.renderer.environment());
                }
                KeyCode::Space if interactive => {
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
        // The PipeWire source is driven by its timer, even while the window is hidden.
        if self.video.is_some() || self.hidden() {
            return Ok(());
        }
        self.draw(surface)?;
        surface.request_redraw();
        Ok(())
    }

    fn about_to_wait(&mut self, surface: &mut dyn Surface) -> Result<Wake> {
        let Some(mut next) = self.next_video_frame else {
            return Ok(Wake::Wait);
        };
        if Instant::now() >= next {
            self.draw(surface)?;
            // Do not catch up with bursts after stalls.
            next = Instant::now() + Duration::from_secs_f64(1.0 / f64::from(pipewire_video::FPS));
            self.next_video_frame = Some(next);
        }
        Ok(Wake::At(next))
    }
}

pub(crate) struct Camera {
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

    pub(crate) fn view_projection(&self, aspect: f32) -> Mat4 {
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
    use crate::{
        spectator_feed::SpectatorOptions,
        test_support::{Nowhere, Silent},
        window::test::{TestSurface, with_gpu},
    };
    use std::process::{Command, Stdio};
    use winit::dpi::PhysicalPosition;

    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

    fn feed() -> Feed {
        Feed::with_audio(
            SpectatorOptions::default(),
            Box::new(Silent),
            Box::new(Nowhere),
        )
    }

    /// A producer that reads every frame and publishes it nowhere, or that has already stopped if `stopped`.
    fn producer(stopped: bool) -> Producer {
        let mut command = Command::new(if stopped { "true" } else { "cat" });
        command.stdin(Stdio::piped()).stdout(Stdio::null());
        Producer::spawn(command).unwrap()
    }

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
    fn the_server_less_window_has_decorations_and_the_spectator_none() {
        let window = Demo::settings(false);
        assert_eq!(window.title, TITLE);
        assert!(window.decorations);
        assert_eq!(window.min_size, None);
        let spectator = Demo::settings(true);
        assert_eq!(spectator.title, SPECTATOR_TITLE);
        assert!(!spectator.decorations);
        assert_eq!(
            spectator.min_size,
            Some(Size::Physical(PhysicalSize::new(1920, 1080)))
        );
    }

    #[test]
    fn the_server_less_window_draws_whenever_it_is_seen() {
        with_gpu(FORMAT, |gpu| {
            let mut surface = TestSurface::new(&gpu.device, FORMAT);
            let mut demo = Demo::new(gpu, None, None);
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
            let mut demo = Demo::new(gpu, None, None);
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
            assert_eq!(demo.renderer.environment(), environment.next());
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
    fn the_spectator_only_follows_its_feed() {
        with_gpu(FORMAT, |gpu| {
            let mut surface = TestSurface::new(&gpu.device, FORMAT);
            let mut demo = Demo::new(gpu, Some(feed()), None);
            let cursor = demo.cursor;
            for ignored in [
                Input::CursorMoved(Vec2::ZERO),
                Input::Mouse {
                    button: MouseButton::Left,
                    pressed: true,
                },
                Input::Wheel(MouseScrollDelta::LineDelta(0.0, 1.0)),
                key(KeyCode::KeyR),
                key(KeyCode::KeyE),
                key(KeyCode::Space),
            ] {
                input(&mut demo, &mut surface, ignored);
            }
            assert_eq!(demo.cursor, cursor);
            assert!(!demo.attracting && !demo.paused);
            input(
                &mut demo,
                &mut surface,
                Input::Resized {
                    width: 64,
                    height: 48,
                },
            );
            demo.redraw(&mut surface).unwrap();
            assert_eq!(surface.presented, 1);
            assert_eq!(
                input(&mut demo, &mut surface, key(KeyCode::Escape)),
                Flow::Exit
            );
        });
    }

    #[test]
    fn the_pipewire_source_draws_on_its_timer_with_the_window_as_its_preview() {
        with_gpu(FORMAT, |gpu| {
            let mut surface = TestSurface::new(&gpu.device, FORMAT);
            let mut demo = Demo::new(gpu, Some(feed()), Some(producer(false)));
            // Redraws are left to the timer.
            demo.redraw(&mut surface).unwrap();
            assert_eq!(surface.presented, 0);
            let next = demo.about_to_wait(&mut surface).unwrap();
            assert_ne!(next, Wake::Wait);
            assert_eq!(surface.presented, 1);
            // Not yet due: nothing more is drawn.
            assert_eq!(demo.about_to_wait(&mut surface).unwrap(), next);
            assert_eq!(surface.presented, 1);
            input(
                &mut demo,
                &mut surface,
                Input::Resized {
                    width: 64,
                    height: 48,
                },
            );
            assert_eq!(surface.sizes, [(64, 48)]);
            // Hidden, it still publishes, without a preview.
            input(&mut demo, &mut surface, Input::Occluded(true));
            for _ in 0..3 {
                std::thread::sleep(Duration::from_millis(40));
                demo.about_to_wait(&mut surface).unwrap();
            }
            assert_eq!(surface.presented, 1);
            // A frame that cannot be drawn stops the window.
            input(&mut demo, &mut surface, Input::Occluded(false));
            surface.fail = true;
            demo.next_video_frame = Some(Instant::now());
            assert!(demo.about_to_wait(&mut surface).is_err());
        });
    }

    #[test]
    fn a_stopped_pipewire_source_leaves_the_preview() {
        with_gpu(FORMAT, |gpu| {
            let mut surface = TestSurface::new(&gpu.device, FORMAT);
            let mut demo = Demo::new(gpu, None, Some(producer(true)));
            std::thread::sleep(Duration::from_millis(100));
            for _ in 0..2 {
                demo.about_to_wait(&mut surface).unwrap();
                demo.next_video_frame = Some(Instant::now());
            }
            assert_eq!(surface.presented, 2);
        });
    }

    #[test]
    fn spectator_requests_1080p_physical_pixels() {
        assert_eq!(
            initial_window_size(true),
            Size::Physical(PhysicalSize::new(1920, 1080))
        );
        assert_eq!(
            initial_window_size(false),
            Size::Logical(LogicalSize::new(1100.0, 760.0))
        );
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
