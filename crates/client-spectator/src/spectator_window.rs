// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The spectator's borderless window, following a participant in a live space; and, with a PipeWire producer, the spectator's video source with the window as its preview: the window's contents, as `tomori-client-desktop`'s window shows them. See `design/spectator.md`.

use crate::{
    graphics::Renderer,
    pipewire_video::{self, Producer, Video},
    spectator_feed::Feed,
    spectator_view::{RenderInputs, SpectatorView},
    swarm::Swarm,
};
use anyhow::Result;
use glam::Vec2;
use std::time::{Duration, Instant};
use tomori_client_desktop::window::{Contents, Flow, Gpu, Input, Settings, Surface, Wake};
use winit::{dpi::PhysicalSize, keyboard::KeyCode};

const TITLE: &str = "Tomori | Quest spectator (OBS)";
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

/// The spectator's window, following its feed; and its PipeWire video source, when it has a producer.
pub struct SpectatorWindow {
    renderer: Renderer,
    device: wgpu::Device,
    queue: wgpu::Queue,
    width: u32,
    height: u32,
    swarm: Swarm,
    occluded: bool,
    first_frame: bool,
    spectator: SpectatorView,
    video: Option<Video>,
    /// When the PipeWire source draws next: it is driven by a timer, even while the window is hidden.
    next_video_frame: Option<Instant>,
}

impl SpectatorWindow {
    /// The spectator's window: borderless, which Escape closes, at 1080p.
    pub fn settings() -> Settings {
        // Request physical render pixels, not logical points, so desktop DPI scaling doesn't silently change the intended OBS capture resolution.
        let size = PhysicalSize::new(WIDTH, HEIGHT);
        Settings {
            title: TITLE,
            decorations: false,
            size: size.into(),
            // Keep the content itself at least 1080p even when the desktop work area excludes a panel.
            min_size: Some(size.into()),
        }
    }

    /// Show the spectator's view of `feed`, drawn with `gpu`; and publish it through `producer` too, when there is one, with this window as its preview.
    pub fn new(gpu: Gpu, feed: Feed, producer: Option<Producer>) -> Self {
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
        let initial = crate::environment::Environment::initial(renderer.environments());
        renderer.set_environment(initial);
        log::info!(
            "Desktop adapter: {} — {}x{}, {samples}x multisampling",
            gpu.adapter.get_info().name,
            gpu.width,
            gpu.height
        );
        log::info!("Spectator: Esc closes the borderless window.");
        let next_video_frame = video.as_ref().map(|_| Instant::now());
        Self {
            renderer,
            device: gpu.device,
            queue: gpu.queue,
            width: gpu.width,
            height: gpu.height,
            swarm: Swarm::default(),
            occluded: false,
            first_frame: true,
            spectator: SpectatorView::new(feed),
            video,
            next_video_frame,
        }
    }

    /// [`new`](Self::new), publishing to PipeWire through `video`, the GStreamer command that reads the frames, when there is one; for [`desktop_window::show`](tomori_client_desktop::desktop_window::show), which starts it once the window and its device exist.
    pub fn open(gpu: Gpu, (feed, video): (Feed, Option<std::process::Command>)) -> Result<Self> {
        let producer = video.map(Producer::spawn).transpose()?;
        Ok(Self::new(gpu, feed, producer))
    }

    /// Whether nothing drawn to the window would be seen.
    fn hidden(&self) -> bool {
        self.occluded || self.width == 0 || self.height == 0
    }

    fn resize(&mut self, width: u32, height: u32, surface: &mut dyn Surface) {
        self.width = width;
        self.height = height;
        if width == 0 || height == 0 {
            return;
        }
        if self.video.is_some() {
            log::info!(
                "Spectator preview size: {width}x{height} physical pixels (PipeWire output remains {}x{})",
                pipewire_video::WIDTH,
                pipewire_video::HEIGHT
            );
        } else {
            log::info!("Spectator render size: {width}x{height} physical pixels");
        }
        surface.resize(width, height);
        if self.video.is_none() {
            self.renderer.resize(&self.device, width, height);
        }
        surface.request_redraw();
    }

    fn draw(&mut self, surface: &mut dyn Surface) -> Result<()> {
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
        self.spectator.render(RenderInputs {
            device: &self.device,
            queue: &self.queue,
            renderer: &mut self.renderer,
            swarm: &mut self.swarm,
            target,
            aspect: size.x / size.y,
        });
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

impl Contents for SpectatorWindow {
    /// The spectator's window only follows its feed; Escape closes it.
    fn input(&mut self, input: Input, surface: &mut dyn Surface) -> Result<Flow> {
        match input {
            Input::Close => return Ok(Flow::Exit),
            Input::Resized { width, height } => self.resize(width, height, surface),
            Input::Occluded(occluded) => {
                self.occluded = occluded;
                if !occluded {
                    surface.request_redraw();
                }
            }
            Input::Key {
                code: KeyCode::Escape,
                pressed: true,
                repeat: false,
            } => return Ok(Flow::Exit),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        spectator_feed::SpectatorOptions,
        test_support::{Nowhere, Silent},
    };
    use std::process::{Command, Stdio};
    use tomori_client_desktop::window::test::{TestSurface, with_gpu};
    use winit::{
        dpi::Size,
        event::{MouseButton, MouseScrollDelta},
    };

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

    fn input(demo: &mut SpectatorWindow, surface: &mut TestSurface, input: Input) -> Flow {
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
    fn the_spectator_only_follows_its_feed() {
        with_gpu(FORMAT, |gpu| {
            let mut surface = TestSurface::new(&gpu.device, FORMAT);
            let mut demo = SpectatorWindow::new(gpu, feed(), None);
            let environment = demo.renderer.environment();
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
            assert_eq!(demo.renderer.environment(), environment);
            assert_eq!(surface.titles, Vec::<String>::new());
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
            assert_eq!(demo.about_to_wait(&mut surface).unwrap(), Wake::Wait);
            // Minimised, it draws nothing.
            input(
                &mut demo,
                &mut surface,
                Input::Resized {
                    width: 0,
                    height: 48,
                },
            );
            demo.redraw(&mut surface).unwrap();
            assert_eq!(surface.sizes, [(64, 48)]);
            input(
                &mut demo,
                &mut surface,
                Input::Resized {
                    width: 64,
                    height: 48,
                },
            );
            // Nothing to draw into: nothing is presented.
            surface.available = false;
            demo.redraw(&mut surface).unwrap();
            assert_eq!(surface.presented, 1);
            // A surface that fails stops the window.
            surface.fail = true;
            assert!(demo.redraw(&mut surface).is_err());
            assert_eq!(
                input(&mut demo, &mut surface, key(KeyCode::Escape)),
                Flow::Exit
            );
            assert_eq!(input(&mut demo, &mut surface, Input::Close), Flow::Exit);
        });
    }

    #[test]
    fn the_pipewire_source_draws_on_its_timer_with_the_window_as_its_preview() {
        with_gpu(FORMAT, |gpu| {
            let mut surface = TestSurface::new(&gpu.device, FORMAT);
            let mut demo = SpectatorWindow::new(gpu, feed(), Some(producer(false)));
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
            let mut demo = SpectatorWindow::new(gpu, feed(), Some(producer(true)));
            std::thread::sleep(Duration::from_millis(100));
            for _ in 0..2 {
                demo.about_to_wait(&mut surface).unwrap();
                demo.next_video_frame = Some(Instant::now());
            }
            assert_eq!(surface.presented, 2);
        });
    }

    #[test]
    fn the_spectator_window_is_borderless_at_1080p_physical_pixels() {
        let window = SpectatorWindow::settings();
        assert_eq!(window.title, TITLE);
        assert!(!window.decorations);
        assert_eq!(window.size, Size::Physical(PhysicalSize::new(1920, 1080)));
        assert_eq!(
            window.min_size,
            Some(Size::Physical(PhysicalSize::new(1920, 1080)))
        );
    }

    #[test]
    fn opening_starts_the_pipewire_source_once_there_is_a_device() {
        with_gpu(FORMAT, |gpu| {
            let window = SpectatorWindow::open(gpu, (feed(), None)).unwrap();
            assert!(window.video.is_none());
        });
        with_gpu(FORMAT, |gpu| {
            let mut command = Command::new("cat");
            command.stdin(Stdio::piped()).stdout(Stdio::null());
            let window = SpectatorWindow::open(gpu, (feed(), Some(command))).unwrap();
            assert!(window.video.is_some());
        });
        with_gpu(FORMAT, |gpu| {
            let missing = Command::new("/nonexistent/gst-launch-1.0");
            let error = SpectatorWindow::open(gpu, (feed(), Some(missing)))
                .err()
                .unwrap();
            assert!(error.to_string().contains("starting gst-launch-1.0"));
        });
    }
}
