// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Render the Quest spectator directly to PipeWire without a display surface.
use crate::{
    graphics::Renderer,
    pipewire_video::{self, Producer, Video},
    spectator_feed::Feed,
    spectator_view::{RenderInputs, SpectatorView},
    swarm::Swarm,
};
use anyhow::{Context, Result};
use std::time::{Duration, Instant};

/// Publish `feed` through `producer`, at the video's rate, until publishing fails.
pub fn run(feed: Feed, producer: Producer) -> Result<()> {
    pollster::block_on(device(wgpu::Backends::VULKAN, &Default::default()))
        .and_then(|gpu| publish(State::new(gpu, feed, producer)))
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

fn next_frame_time(previous: Instant, now: Instant, interval: Duration) -> Instant {
    // Keep an even cadence without issuing catch-up bursts after a slow frame.
    (previous + interval).max(now)
}

/// A device with no surface, on the preferred adapter of `backends`, as `descriptor` asks.
async fn device(
    backends: wgpu::Backends,
    descriptor: &wgpu::DeviceDescriptor<'_>,
) -> Result<(wgpu::Device, wgpu::Queue)> {
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
        .context("finding a headless Vulkan adapter")?;
    log::info!("Headless spectator adapter: {}", adapter.get_info().name);
    adapter
        .request_device(descriptor)
        .await
        .context("creating a headless spectator device")
}

struct State {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    video: Video,
    spectator: SpectatorView,
    swarm: Swarm,
}

impl State {
    fn new((device, queue): (wgpu::Device, wgpu::Queue), feed: Feed, producer: Producer) -> Self {
        let renderer = Renderer::new(
            &device,
            &queue,
            pipewire_video::FORMAT,
            pipewire_video::WIDTH,
            pipewire_video::HEIGHT,
        );
        let spectator = SpectatorView::new(feed);
        let video = Video::new(&device, None, producer);
        Self {
            device,
            queue,
            renderer,
            video,
            spectator,
            swarm: Swarm::default(),
        }
    }

    fn draw(&mut self) -> Result<()> {
        self.spectator.render(RenderInputs {
            device: &self.device,
            queue: &self.queue,
            renderer: &mut self.renderer,
            swarm: &mut self.swarm,
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
        let error = pollster::block_on(device(wgpu::Backends::empty(), &Default::default()))
            .map(drop)
            .expect_err("no adapter");
        assert_eq!(error.to_string(), "finding a headless Vulkan adapter");
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
}
