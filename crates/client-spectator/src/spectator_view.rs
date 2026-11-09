// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Shared scene rendering for the windowed and headless spectators.
//!
//! The view is driven by a [`crate::spectator_feed::Feed`], polled once per
//! rendered frame: the feed connects to a space the same way the headset does
//! and projects the followed participant's point of view into a [`Frame`].
use crate::{graphics::Renderer, spectator_feed::Feed, swarm::Swarm};
use glam::{Mat4, Vec3};

pub(crate) struct SpectatorView {
    feed: Feed,
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
    pub fn new(feed: Feed) -> Self {
        Self { feed }
    }

    pub fn render(&mut self, input: RenderInputs<'_>) {
        let frame = self.feed.poll(input.aspect);
        input.renderer.set_environment(frame.environment);
        // Its scenery draws in its own coordinates, carried into the space's, as on the headset.
        input.renderer.set_carriage(self.feed.carriage());
        // The feed animates a swarm as large as the renderer's, or none while another avatar shows, which its own
        // scenery draws. See `design/avatars.md`.
        input
            .renderer
            .set_avatar(input.device, input.queue, self.feed.avatar().as_ref());
        input.swarm.agents.resize(
            frame.agents.len(),
            crate::swarm::Agent {
                position: Vec3::ZERO,
                velocity: Vec3::ZERO,
            },
        );
        for (agent, position) in input.swarm.agents.iter_mut().zip(&frame.agents) {
            agent.position = Vec3::from_array(*position);
        }
        input.swarm.visual = frame.interaction;
        // Its rings stand about the swarm wherever the space's environment puts it.
        input.swarm.set_anchor(self.feed.swarm_anchor());
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
            // Workers' signs face the followed participant, as they do on their own headset.
            Some(tomori_render::scenery::eye(
                Mat4::from_cols_array(&frame.projection)
                    * Mat4::from_cols_array(&frame.scene_to_space),
            ))
            .filter(|eye| eye.is_finite()),
        );
        input
            .renderer
            .update_controllers(input.queue, frame.controllers, frame.hands);
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
            // The headset's head-locked HUD would obstruct the spectator view.
            &[],
        );
    }
}
