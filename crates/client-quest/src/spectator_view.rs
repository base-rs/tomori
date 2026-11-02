// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Shared scene rendering for the windowed and headless spectators.
//!
//! The view is driven by a [`crate::spectator_feed::Feed`], polled once per
//! rendered frame: the feed connects to a space the same way the headset does
//! and projects the followed participant's point of view into a [`Frame`].
use crate::{caption::Caption, graphics::Renderer, spectator_feed::Feed, swarm::Swarm};
use glam::{Mat4, Vec3};
use std::time::Duration;

pub(crate) struct SpectatorView {
    feed: Feed,
    caption: Caption,
}

pub(crate) struct RenderInputs<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub renderer: &'a mut Renderer,
    pub swarm: &'a mut Swarm,
    pub target: wgpu::TextureView,
    pub aspect: f32,
}

impl SpectatorView {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, feed: Feed) -> Self {
        Self {
            feed,
            caption: Caption::new(device, format),
        }
    }

    pub fn render(&mut self, input: RenderInputs<'_>) {
        let frame = self.feed.poll(input.aspect);
        input.renderer.set_environment(frame.environment);
        if input.swarm.agents.len() != frame.agents.len() {
            *input.swarm = Swarm::new();
        }
        for (agent, position) in input.swarm.agents.iter_mut().zip(&frame.agents) {
            agent.position = Vec3::from_array(*position);
        }
        input.swarm.visual = frame.interaction;
        let right_marker = if frame.controllers[crate::controller::RIGHT].is_some()
            || frame.hands[crate::controller::RIGHT].is_some()
        {
            None
        } else {
            frame.controller.map(Vec3::from_array)
        };
        input
            .renderer
            .update(input.queue, input.swarm, right_marker, frame.attracting);
        input
            .renderer
            .set_presentation_page(frame.presentation_page);
        input
            .renderer
            .set_presentation_content_page(frame.presentation_content_page);
        input.renderer.update_workspace(
            input.device,
            input.queue,
            &frame.worker_cubes,
            &frame.worker_stations,
            &frame.workspace,
        );
        input
            .renderer
            .update_controllers(input.queue, frame.controllers, frame.hands);
        self.caption.set_text(input.queue, &frame.caption);
        self.caption.set_recording_progress(
            input.queue,
            frame.recording_ms.map(Duration::from_millis),
            Duration::ZERO,
        );
        // Preserve the followed participant's vertical FOV instead of
        // stretching its view to the spectator output aspect ratio.
        let aspect_correction =
            Mat4::from_scale(Vec3::new(frame.view_aspect / input.aspect, 1.0, 1.0));
        let projection = aspect_correction * Mat4::from_cols_array(&frame.projection);
        input.renderer.render(
            input.device,
            input.queue,
            &[input.target],
            &[projection],
            Mat4::from_cols_array(&frame.scene_to_space),
            Some((&self.caption, Mat4::from_cols_array(&frame.head))),
        );
    }
}
