// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The spectator's window, decorated or borderless, following a participant in a live space; and, with a PipeWire producer, the spectator's video source with the window as its preview: the window's contents, as `tomori-client-desktop`'s window shows them. See `design/spectator.md`.
use crate::{
    graphics::Renderer,
    headless_spectator::next_frame_time,
    pipewire_video::{self, Producer, Video},
    recording::{Recording, capture::Capture, clock::FrameClock},
    spectator_feed::{Feed, SpectatorOptions},
    spectator_view::{RenderInputs, SpectatorView},
};
use anyhow::Result;
use glam::Vec2;
use std::time::{Duration, Instant};
use tomori_client_desktop::window::{Contents, Flow, Gpu, Input, Settings, Surface, Wake};
use winit::{dpi::PhysicalSize, keyboard::KeyCode};

const TITLE: &str = "Tomori | Quest spectator (OBS)";
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

/// What the spectator's window shows and where its output goes, for [`SpectatorWindow::open`].
pub struct Launch {
    pub feed: Feed,
    /// The GStreamer command that reads the PipeWire source's frames, when it publishes one.
    pub video: Option<std::process::Command>,
    /// The recording it makes, when it makes one.
    pub recording: Option<Recording>,
}

impl Launch {
    /// A window of `feed` and nothing else.
    pub fn new(feed: Feed) -> Self {
        Self {
            feed,
            video: None,
            recording: None,
        }
    }
}

/// The spectator's window, following its feed; and its PipeWire video source, when it has a producer, or its recording, when it has one, either with the window as its preview.
pub struct SpectatorWindow {
    renderer: Renderer,
    device: wgpu::Device,
    queue: wgpu::Queue,
    width: u32,
    height: u32,
    occluded: bool,
    first_frame: bool,
    spectator: SpectatorView,
    video: Option<Video>,
    /// The recording's render target, which is drawn into at the recording's size, and read back, instead of the window.
    capture: Option<Capture>,
    recording: Option<Recording>,
    /// When the PipeWire source or the recording draws next: they are driven by a timer, even while the window is hidden.
    next_video_frame: Option<Instant>,
    frame_interval: Duration,
}

impl SpectatorWindow {
    /// The spectator's window, as `options` ask, which Escape closes, at 1080p: decorated, with the platform's title bar, so that it can be moved, resized to any size and aspect and made full screen; or, if [`SpectatorOptions::borderless`], without decorations and at least 1080p, for a clean capture.
    pub fn settings(options: &SpectatorOptions) -> Settings {
        let borderless = options.borderless;
        // Request physical render pixels, not logical points, so desktop DPI scaling doesn't silently change the intended OBS capture resolution.
        let size = PhysicalSize::new(WIDTH, HEIGHT);
        Settings {
            title: TITLE,
            decorations: !borderless,
            size: size.into(),
            // Keep a borderless capture's content at least 1080p even when the desktop work area excludes a panel.
            min_size: borderless.then_some(size.into()),
        }
    }

    /// Show the spectator's view of `feed`, drawn with `gpu`; and publish it through `producer` too, when there is one, with this window as its preview.
    pub fn new(gpu: Gpu, feed: Feed, producer: Option<Producer>) -> Self {
        Self::build(gpu, feed, producer, None)
    }

    /// As [`Self::new`], recording the view to `recording` instead of publishing it, with this window as its preview: it draws at the recording's size, on its clock, as the PipeWire source draws at its own.
    pub fn recording(gpu: Gpu, feed: Feed, recording: Recording) -> Self {
        Self::build(gpu, feed, None, Some(recording))
    }

    fn build(
        gpu: Gpu,
        feed: Feed,
        producer: Option<Producer>,
        recording: Option<Recording>,
    ) -> Self {
        let fixed = producer.is_some() || recording.is_some();
        let render_format = if fixed {
            pipewire_video::FORMAT
        } else {
            gpu.format
        };
        let samples = gpu.samples(render_format);
        let video = producer.map(|producer| Video::new(&gpu.device, Some(gpu.format), producer));
        let capture = recording
            .clone()
            .map(|recording| Capture::new(&gpu.device, Some(gpu.format), recording));
        let (render_width, render_height) = match (&video, &capture) {
            (Some(_), _) => (pipewire_video::WIDTH, pipewire_video::HEIGHT),
            (None, Some(capture)) => capture.size(),
            (None, None) => (gpu.width, gpu.height),
        };
        let mut renderer = Renderer::with_sceneries(
            &gpu.device,
            &gpu.queue,
            render_format,
            render_width,
            render_height,
            samples,
            feed.sceneries(),
        );
        renderer.set_tier(crate::tier().tier);
        log::info!(
            "Desktop adapter: {} — {}x{}, {samples}x multisampling",
            gpu.adapter.get_info().name,
            gpu.width,
            gpu.height
        );
        log::info!("Spectator: Esc closes the window.");
        let next_video_frame = fixed.then(Instant::now);
        let frame_interval = match &recording {
            Some(recording) => FrameClock::new(recording.fps).interval(),
            None => Duration::from_secs_f64(1.0 / f64::from(pipewire_video::FPS)),
        };
        Self {
            renderer,
            device: gpu.device,
            queue: gpu.queue,
            width: gpu.width,
            height: gpu.height,
            occluded: false,
            first_frame: true,
            spectator: SpectatorView::new(feed),
            video,
            capture,
            recording,
            next_video_frame,
            frame_interval,
        }
    }

    /// [`new`](Self::new), or [`recording`](Self::recording), publishing to PipeWire through `launch.video`, the GStreamer command that reads the frames, when there is one; for [`desktop_window::show`](tomori_client_desktop::desktop_window::show), which starts it once the window and its device exist.
    pub fn open(gpu: Gpu, launch: Launch) -> Result<Self> {
        match launch.recording {
            Some(recording) => Ok(Self::recording(gpu, launch.feed, recording)),
            None => {
                let producer = launch.video.map(Producer::spawn).transpose()?;
                Ok(Self::new(gpu, launch.feed, producer))
            }
        }
    }

    /// Whether the window draws at a size of its own, not at the window's: the PipeWire source's or the recording's.
    fn fixed(&self) -> bool {
        self.video.is_some() || self.capture.is_some()
    }

    /// Ask for the recording, if there is one, to end, for `reason`.
    fn stop(&self, reason: &str) {
        if let Some(recording) = &self.recording {
            recording.stop.request(reason);
        }
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
        if let Some(capture) = &self.capture {
            let (recorded_width, recorded_height) = capture.size();
            log::info!(
                "Spectator preview size: {width}x{height} physical pixels (the recording remains {recorded_width}x{recorded_height})"
            );
        } else if self.video.is_some() {
            log::info!(
                "Spectator preview size: {width}x{height} physical pixels (PipeWire output remains {}x{})",
                pipewire_video::WIDTH,
                pipewire_video::HEIGHT
            );
        } else {
            log::info!("Spectator render size: {width}x{height} physical pixels");
        }
        surface.resize(width, height);
        if !self.fixed() {
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
        let fixed = match (&self.video, &self.capture) {
            (Some(video), _) => Some((
                video.view.clone(),
                Vec2::new(pipewire_video::WIDTH as f32, pipewire_video::HEIGHT as f32),
            )),
            (None, Some(capture)) => {
                let (width, height) = capture.size();
                Some((capture.view.clone(), Vec2::new(width as f32, height as f32)))
            }
            (None, None) => None,
        };
        let drawn = Instant::now();
        let (target, size) = match (fixed, &frame) {
            (Some(fixed), _) => fixed,
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
        if let (Some(capture), Some(recording)) = (&mut self.capture, &self.recording) {
            capture.capture(&self.queue, drawn);
            if let Some(preview) = &frame {
                capture.preview(&self.queue, preview);
            }
            recording.watch(&self.spectator.feed);
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
    /// The spectator's window only follows its feed; V cycles through the voices it plays, A turns the ambience on and
    /// off, D ducks under everyone's speech or the voices played alone, and Escape closes it.
    fn input(&mut self, input: Input, surface: &mut dyn Surface) -> Result<Flow> {
        match input {
            Input::Close => {
                self.stop("the window was closed");
                return Ok(Flow::Exit);
            }
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
            } => {
                self.stop("the window was closed");
                return Ok(Flow::Exit);
            }
            Input::Key {
                code: KeyCode::KeyV,
                pressed: true,
                repeat: false,
            } => {
                self.spectator.feed.next_voices();
            }
            Input::Key {
                code: KeyCode::KeyA,
                pressed: true,
                repeat: false,
            } => {
                self.spectator.feed.toggle_ambience();
            }
            Input::Key {
                code: KeyCode::KeyD,
                pressed: true,
                repeat: false,
            } => {
                self.spectator.feed.next_duck_under();
            }
            _ => {}
        }
        Ok(Flow::Continue)
    }

    fn redraw(&mut self, surface: &mut dyn Surface) -> Result<()> {
        // The PipeWire source and the recording are driven by their timer, even while the window is hidden.
        if self.fixed() || self.hidden() {
            return Ok(());
        }
        self.draw(surface)?;
        surface.request_redraw();
        Ok(())
    }

    fn about_to_wait(&mut self, surface: &mut dyn Surface) -> Result<Wake> {
        if self
            .recording
            .as_ref()
            .is_some_and(|recording| recording.stop.requested())
        {
            return Ok(Wake::Exit);
        }
        let Some(mut next) = self.next_video_frame else {
            return Ok(Wake::Wait);
        };
        if Instant::now() >= next {
            self.draw(surface)?;
            // An even cadence, without catching up with bursts after a stall.
            next = next_frame_time(next, Instant::now(), self.frame_interval);
            self.next_video_frame = Some(next);
        }
        Ok(Wake::At(next))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Nowhere, Silent};
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
            // V plays the agent's speech alone, as for a demonstration, and then cycles on.
            input(&mut demo, &mut surface, key(KeyCode::KeyV));
            assert_eq!(demo.spectator.feed.voices(), crate::Hearing::Agent);
            // A turns the ambience on, and off again, showing nothing.
            assert!(!demo.spectator.feed.ambience());
            input(&mut demo, &mut surface, key(KeyCode::KeyA));
            assert!(demo.spectator.feed.ambience());
            input(&mut demo, &mut surface, key(KeyCode::KeyA));
            assert!(!demo.spectator.feed.ambience());
            // D ducks under everyone's speech, and under the voices played again, showing nothing.
            assert_eq!(demo.spectator.feed.duck_under(), crate::DuckUnder::Played);
            input(&mut demo, &mut surface, key(KeyCode::KeyD));
            assert_eq!(demo.spectator.feed.duck_under(), crate::DuckUnder::Everyone);
            input(&mut demo, &mut surface, key(KeyCode::KeyD));
            assert_eq!(demo.spectator.feed.duck_under(), crate::DuckUnder::Played);
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

    /// A recording of `size` that goes nowhere but the frames it is read back into.
    fn recording(
        size: (u32, u32),
    ) -> (
        Recording,
        std::sync::mpsc::Receiver<(Duration, crate::recording::video::Frame)>,
    ) {
        let origin = crate::recording::Origin::default();
        let (frames, drawn) = crate::recording::video::FrameSender::new(
            origin.clone(),
            size.0 as usize * size.1 as usize * 4,
        );
        let recording = Recording {
            size,
            fps: 30,
            origin,
            frames,
            stop: crate::recording::Stop::default(),
        };
        (recording, drawn)
    }

    #[test]
    fn the_recording_draws_at_its_size_on_its_timer_with_the_window_as_its_preview() {
        with_gpu(FORMAT, |gpu| {
            let mut surface = TestSurface::new(&gpu.device, FORMAT);
            let (recording, drawn) = recording((96, 64));
            let mut demo = SpectatorWindow::recording(gpu, feed(), recording);
            // Redraws are left to the timer, as the PipeWire source's are.
            demo.redraw(&mut surface).unwrap();
            assert_eq!(surface.presented, 0);
            let next = demo.about_to_wait(&mut surface).unwrap();
            assert!(matches!(next, Wake::At(_)));
            assert_eq!(surface.presented, 1);
            // Not yet due: nothing more is drawn.
            assert_eq!(demo.about_to_wait(&mut surface).unwrap(), next);
            assert_eq!(surface.presented, 1);
            // The window's size changes the preview, not the recording.
            input(
                &mut demo,
                &mut surface,
                Input::Resized {
                    width: 64,
                    height: 48,
                },
            );
            assert_eq!(surface.sizes, [(64, 48)]);
            // Hidden, it still records, without a preview.
            input(&mut demo, &mut surface, Input::Occluded(true));
            for _ in 0..3 {
                std::thread::sleep(Duration::from_millis(40));
                demo.about_to_wait(&mut surface).unwrap();
            }
            assert_eq!(surface.presented, 1);
            demo.capture.as_mut().unwrap().finish();
            let frames: Vec<_> = drawn.try_iter().collect();
            assert!(frames.len() >= 2, "{}", frames.len());
            assert!(frames.iter().all(|(_, frame)| frame.len() == 96 * 64 * 4));
            // Nothing asked it to end.
            assert!(!demo.recording.as_ref().unwrap().stop.requested());
            // A frame that cannot be drawn stops the window.
            input(&mut demo, &mut surface, Input::Occluded(false));
            surface.fail = true;
            demo.next_video_frame = Some(Instant::now());
            assert!(demo.about_to_wait(&mut surface).is_err());
        });
    }

    #[test]
    fn closing_the_window_ends_the_recording_and_so_does_whatever_asks_for_it() {
        for close in [Input::Close, key(KeyCode::Escape)] {
            with_gpu(FORMAT, |gpu| {
                let mut surface = TestSurface::new(&gpu.device, FORMAT);
                let (recording, _drawn) = recording((64, 64));
                let stop = recording.stop.clone();
                let mut demo = SpectatorWindow::recording(gpu, feed(), recording);
                assert_eq!(input(&mut demo, &mut surface, close), Flow::Exit);
                assert_eq!(stop.reason().as_deref(), Some("the window was closed"));
            });
        }
        with_gpu(FORMAT, |gpu| {
            let mut surface = TestSurface::new(&gpu.device, FORMAT);
            let (recording, _drawn) = recording((64, 64));
            let stop = recording.stop.clone();
            let mut demo = SpectatorWindow::recording(gpu, feed(), recording);
            assert!(matches!(
                demo.about_to_wait(&mut surface).unwrap(),
                Wake::At(_)
            ));
            // A signal, or the server going away, asks from elsewhere: the window closes itself.
            stop.request("received SIGINT");
            assert_eq!(demo.about_to_wait(&mut surface).unwrap(), Wake::Exit);
        });
        // A window that records nothing has nothing to stop.
        with_gpu(FORMAT, |gpu| {
            let mut surface = TestSurface::new(&gpu.device, FORMAT);
            let mut demo = SpectatorWindow::new(gpu, feed(), None);
            assert_eq!(input(&mut demo, &mut surface, Input::Close), Flow::Exit);
        });
    }

    #[test]
    fn opening_with_a_recording_records_instead_of_publishing() {
        with_gpu(FORMAT, |gpu| {
            let (recording, _drawn) = recording((64, 64));
            let launch = Launch {
                recording: Some(recording),
                ..Launch::new(feed())
            };
            let window = SpectatorWindow::open(gpu, launch).unwrap();
            assert!(window.capture.is_some() && window.video.is_none());
        });
    }

    #[test]
    fn the_spectator_window_is_decorated_and_resizable_at_1080p_physical_pixels() {
        let window = SpectatorWindow::settings(&SpectatorOptions::new());
        assert_eq!(window.title, TITLE);
        assert!(window.decorations);
        assert_eq!(window.size, Size::Physical(PhysicalSize::new(1920, 1080)));
        assert_eq!(window.min_size, None);
    }

    #[test]
    fn a_borderless_spectator_window_is_at_least_1080p_physical_pixels() {
        let mut options = SpectatorOptions::new();
        options.borderless = true;
        let window = SpectatorWindow::settings(&options);
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
            let window = SpectatorWindow::open(gpu, Launch::new(feed())).unwrap();
            assert!(window.video.is_none());
        });
        with_gpu(FORMAT, |gpu| {
            let mut command = Command::new("cat");
            command.stdin(Stdio::piped()).stdout(Stdio::null());
            let launch = Launch {
                video: Some(command),
                ..Launch::new(feed())
            };
            let window = SpectatorWindow::open(gpu, launch).unwrap();
            assert!(window.video.is_some());
        });
        with_gpu(FORMAT, |gpu| {
            let missing = Command::new("/nonexistent/gst-launch-1.0");
            let launch = Launch {
                video: Some(missing),
                ..Launch::new(feed())
            };
            let error = SpectatorWindow::open(gpu, launch).err().unwrap();
            assert!(error.to_string().contains("starting gst-launch-1.0"));
        });
    }
}
