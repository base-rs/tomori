// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Shared scene rendering for the windowed and headless spectators.
//!
//! The view is driven by a [`crate::spectator_feed::Feed`], polled once per
//! rendered frame: the feed runs the frame step a participant's client runs, as
//! a watcher, and the view draws the frame it returns as a participant's client
//! does, with [`tomori_render::scene_graphics::draw`], from the feed's camera.
//! See `design/spectator.md`.
use crate::{graphics::Renderer, spectator_feed::Feed};
use glam::Mat4;
use std::time::Instant;
use tomori_render::scene_graphics::{self, Panels};

pub(crate) struct SpectatorView {
    pub(crate) feed: Feed,
    /// The GPU buffers of the frame's panels, made with the renderer's first frame.
    panels: Option<Panels>,
}

pub(crate) struct RenderInputs<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub renderer: &'a mut Renderer,
    pub target: wgpu::TextureView,
    pub aspect: f32,
}

impl SpectatorView {
    pub fn new(feed: Feed) -> Self {
        Self { feed, panels: None }
    }

    /// Draw a frame, with the camera as of `at`, the time on the clock that paces the frames: the recording's slot, whose frames are an even interval apart however long each took, or the time now. See [`Feed::poll_at`].
    pub fn render(&mut self, input: RenderInputs<'_>, at: Instant) {
        let watched = self.feed.poll_at(input.aspect, Instant::now(), at);
        let panels = self
            .panels
            .get_or_insert_with(|| Panels::new(input.device, input.renderer));
        scene_graphics::draw(
            input.renderer,
            input.device,
            input.queue,
            panels,
            &watched.frame,
        );
        // The followed participant's controllers and hands, as theirs.
        input
            .renderer
            .update_controllers(input.queue, watched.controllers, watched.hands);
        // A watcher's rig never moves, so its scene is the space's.
        input.renderer.render(
            input.device,
            input.queue,
            &[input.target],
            &[watched.projection],
            Mat4::IDENTITY,
            &panels.list(Mat4::IDENTITY),
        );
    }
}
