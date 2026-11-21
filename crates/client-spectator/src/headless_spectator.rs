// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Render the Quest spectator directly to PipeWire, or to a recording, without a display surface.
use crate::{
    graphics::Renderer,
    pipewire_video::{self, Producer, Video},
    recording::{self, Recording, capture::Capture, clock::FrameClock},
    spectator_feed::Feed,
    spectator_view::{RenderInputs, SpectatorView},
};
use anyhow::{Context, Result};
use std::time::{Duration, Instant};

/// Publish `feed` through `producer`, at the video's rate, until publishing fails.
pub fn run(feed: Feed, producer: Producer) -> Result<()> {
    pollster::block_on(gpu(wgpu::Backends::VULKAN, &Default::default()))
        .and_then(|(_, device, queue)| publish(State::new((device, queue), feed, producer)))
}

/// Draw and publish a frame at the video's rate, until publishing fails.
fn publish(mut state: State) -> Result<()> {
    let interval = Duration::from_secs_f64(1.0 / f64::from(pipewire_video::FPS));
    let mut next = Instant::now();
    log::info!("Headless spectator running; Ctrl-C stops the PipeWire source.");
    loop {
        // Drawing late, the next frame is due at once.
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
        state.draw()?;
        next = next_frame_time(next, Instant::now(), interval);
    }
}

pub(crate) fn next_frame_time(previous: Instant, now: Instant, interval: Duration) -> Instant {
    // Keep an even cadence without issuing catch-up bursts after a slow frame.
    (previous + interval).max(now)
}

/// Record `feed` to `recording`, without a window, at the recording's size and rate, until the recording is to end, drawing on the platform's own graphics, Vulkan or Metal.
pub fn record(feed: Feed, recording: Recording) -> Result<()> {
    record_on(wgpu::Backends::PRIMARY, feed, recording)
}

/// [`record`], on the adapter of `backends`.
fn record_on(backends: wgpu::Backends, feed: Feed, recording: Recording) -> Result<()> {
    pollster::block_on(gpu(backends, &Default::default()))
        .and_then(|gpu| record_frames(Recorder::new(gpu, feed, recording)))
}

/// Draw a frame at the recording's rate until the recording is to end or drawing fails.
fn record_frames(mut recorder: Recorder) -> Result<()> {
    let interval = FrameClock::new(recorder.recording.fps).interval();
    let mut next = Instant::now();
    log::info!("Headless spectator recording; Ctrl-C stops it.");
    while !recorder.recording.stop.requested() {
        // Drawing late, the next frame is due at once.
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
        recorder.draw();
        next = next_frame_time(next, Instant::now(), interval);
    }
    Ok(())
}

/// A device with no surface, on the preferred adapter of `backends`, as `descriptor` asks, and the adapter it is on.
async fn gpu(
    backends: wgpu::Backends,
    descriptor: &wgpu::DeviceDescriptor<'_>,
) -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            ..Default::default()
        })
        .await
        .context("finding a headless adapter")?;
    log::info!("Headless spectator adapter: {}", adapter.get_info().name);
    adapter
        .request_device(descriptor)
        .await
        .context("creating a headless spectator device")
        .map(|(device, queue)| (adapter, device, queue))
}

/// The spectator drawing for a recording, without a window.
struct Recorder {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    capture: Capture,
    spectator: SpectatorView,
    recording: Recording,
}

impl Recorder {
    fn new(
        (adapter, device, queue): (wgpu::Adapter, wgpu::Device, wgpu::Queue),
        feed: Feed,
        recording: Recording,
    ) -> Self {
        let samples = crate::graphics::supported_samples(
            &adapter,
            recording::FORMAT,
            crate::graphics::SAMPLES,
        );
        let (width, height) = recording.size;
        let mut renderer = Renderer::with_sceneries(
            &device,
            &queue,
            recording::FORMAT,
            width,
            height,
            samples,
            feed.sceneries(),
        );
        renderer.set_tier(crate::tier().tier);
        let capture = Capture::new(&device, None, recording.clone());
        Self {
            device,
            queue,
            renderer,
            capture,
            spectator: SpectatorView::new(feed),
            recording,
        }
    }

    fn draw(&mut self) {
        let drawn = Instant::now();
        let (width, height) = self.recording.size;
        self.spectator.render(RenderInputs {
            device: &self.device,
            queue: &self.queue,
            renderer: &mut self.renderer,
            target: self.capture.view.clone(),
            aspect: width as f32 / height as f32,
        });
        self.capture.capture(&self.queue, drawn);
        self.recording.watch(&self.spectator.feed);
    }
}

struct State {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    video: Video,
    spectator: SpectatorView,
}

impl State {
    fn new((device, queue): (wgpu::Device, wgpu::Queue), feed: Feed, producer: Producer) -> Self {
        let mut renderer = Renderer::with_sceneries(
            &device,
            &queue,
            pipewire_video::FORMAT,
            pipewire_video::WIDTH,
            pipewire_video::HEIGHT,
            1,
            feed.sceneries(),
        );
        renderer.set_tier(crate::tier().tier);
        let spectator = SpectatorView::new(feed);
        let video = Video::new(&device, None, producer);
        Self {
            device,
            queue,
            renderer,
            video,
            spectator,
        }
    }

    fn draw(&mut self) -> Result<()> {
        self.spectator.render(RenderInputs {
            device: &self.device,
            queue: &self.queue,
            renderer: &mut self.renderer,
            target: self.video.view.clone(),
            aspect: pipewire_video::WIDTH as f32 / pipewire_video::HEIGHT as f32,
        });
        self.video
            .capture(&self.device, &self.queue)
            .context("publishing headless spectator frame")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_an_adapter_there_is_no_device() {
        let error = pollster::block_on(gpu(wgpu::Backends::empty(), &Default::default()))
            .map(drop)
            .expect_err("no adapter");
        assert_eq!(error.to_string(), "finding a headless adapter");
        // A recording has none to draw on either.
        let (recording, _drawn) = recording();
        let error = record_on(wgpu::Backends::empty(), feed(), recording).unwrap_err();
        assert_eq!(error.to_string(), "finding a headless adapter");
    }

    #[test]
    fn frame_schedule_keeps_cadence_and_drops_missed_ticks() {
        let start = Instant::now();
        let interval = Duration::from_millis(33);
        assert_eq!(
            next_frame_time(start, start + Duration::from_millis(5), interval),
            start + interval
        );
        assert_eq!(
            next_frame_time(start, start + Duration::from_millis(200), interval),
            start + Duration::from_millis(200)
        );
    }

    fn feed() -> Feed {
        Feed::with_audio(
            crate::SpectatorOptions::new(),
            Box::new(crate::test_support::Silent),
            Box::new(crate::test_support::Nowhere),
        )
    }

    /// A recording of 64 pixels square at 30 frames a second, and the frames it is given.
    fn recording() -> (
        Recording,
        std::sync::mpsc::Receiver<(Duration, crate::recording::video::Frame)>,
    ) {
        let origin = crate::recording::Origin::default();
        let (frames, drawn) =
            crate::recording::video::FrameSender::new(origin.clone(), 64 * 64 * 4);
        let recording = Recording {
            size: (64, 64),
            fps: 30,
            origin,
            frames,
            stop: crate::recording::Stop::default(),
        };
        (recording, drawn)
    }

    #[test]
    fn a_recording_without_a_window_draws_at_its_rate_until_it_is_to_end() {
        tomori_render::test_gpu::with(|gpu| {
            let (recording, drawn) = recording();
            let stop = recording.stop.clone();
            let recorder = Recorder::new(
                (gpu.adapter.clone(), gpu.device.clone(), gpu.queue.clone()),
                feed(),
                recording,
            );
            let ender = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(250));
                stop.request("received SIGINT");
            });
            record_frames(recorder).unwrap();
            ender.join().unwrap();
            // About thirty a second for a quarter of a second, each the recording's size.
            let frames: Vec<_> = drawn.try_iter().collect();
            assert!((2..=12).contains(&frames.len()), "{}", frames.len());
            assert!(frames.iter().all(|(_, frame)| frame.len() == 64 * 64 * 4));
            assert!(frames.windows(2).all(|pair| pair[0].0 < pair[1].0));
        });
    }

    #[test]
    fn a_recording_asked_to_end_before_it_starts_draws_nothing() {
        tomori_render::test_gpu::with(|_| {
            let (recording, drawn) = recording();
            recording.stop.request("the server disconnected");
            record(feed(), recording).unwrap();
            assert!(drawn.try_recv().is_err());
        });
    }
}
