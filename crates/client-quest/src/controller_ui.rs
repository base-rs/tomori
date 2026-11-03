// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The panels held by the controllers, shared by the headset's and the desktop window's frame loops: for now the recording indicator and the `LIVE` badge on the left controller. See `design/hand-menu.md`.

use std::time::Instant;

use glam::{Mat4, Vec3};

use crate::conversation::Conversation;
use crate::graphics::Renderer;
use crate::recording::{self, Anchor, Trackers};
use crate::ui_panel::Panel;

/// When a frame is, for the panels that move and animate.
#[derive(Clone, Copy, Debug)]
pub struct Moment {
    pub now: Instant,
    /// Seconds since the last frame.
    pub dt: f32,
    /// Seconds since the client started, for animation.
    pub time: f32,
}

/// Each panel's most vertices.
const CAPACITY: usize = 24_000;

/// The recording indicator and the `LIVE` badge, with where they are.
pub struct Indicator {
    card: Panel,
    badge: Panel,
    anchor: Anchor,
    badge_anchor: Anchor,
    /// The card's and the badge's models in the scene, while shown.
    shown: [Option<Mat4>; 2],
}

impl Indicator {
    pub fn new(device: &wgpu::Device, renderer: &Renderer) -> Self {
        Self {
            card: renderer.panel(device, CAPACITY),
            badge: renderer.panel(device, CAPACITY / 8),
            anchor: Anchor::default(),
            badge_anchor: Anchor::default(),
            shown: [None, None],
        }
    }

    /// Follow the conversation at `moment`, placing the indicator by `trackers` facing `head`, both in the scene.
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        conversation: &Conversation,
        trackers: Trackers,
        head: Option<Vec3>,
        moment: Moment,
    ) {
        let Moment { now, dt, time } = moment;
        let pulse = 0.5 + 0.5 * (time * 5.0).sin();
        let phase = conversation.recording_phase(now);
        let head = head.unwrap_or(crate::arrival::HOME);
        self.shown[0] = match (&phase, trackers.target(recording::GRIP_OFFSET)) {
            (Some(phase), Some(target)) => {
                let canvas = recording::canvas(phase, conversation.utterance_limit(), pulse);
                self.card.set(queue, &canvas);
                let (position, scale) = self.anchor.update(target, dt);
                Some(recording::facing(position, head, scale))
            }
            _ => {
                self.anchor.hide();
                None
            }
        };
        // The badge stays on the controller while the live microphone is on, except while the card shows.
        let badge = trackers
            .grip
            .and_then(|_| trackers.target(recording::BADGE_OFFSET));
        self.shown[1] = match badge {
            Some(target) if conversation.live() && self.shown[0].is_none() => {
                self.badge.set(queue, &recording::badge(pulse));
                let (position, scale) = self.badge_anchor.update(target, dt);
                Some(recording::facing(position, head, scale))
            }
            _ => {
                self.badge_anchor.hide();
                None
            }
        };
    }

    /// The panels to draw, with `scene_to_space` taking the scene to the eyes' space.
    pub fn panels(&self, scene_to_space: Mat4) -> impl Iterator<Item = (&Panel, Mat4)> {
        [&self.card, &self.badge]
            .into_iter()
            .zip(self.shown)
            .filter_map(move |(panel, model)| model.map(|model| (panel, scene_to_space * model)))
    }
}

/// Where the indicator goes when neither the controller nor the hand is tracked: low in the view of `head`, a scene transform.
pub fn view_spot(head: Mat4) -> Vec3 {
    head.transform_point3(Vec3::new(0.0, -0.22, -0.55))
}
