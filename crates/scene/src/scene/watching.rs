// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The frame step in watcher mode, as a spectator runs it: the same [`Step::update`] that a participant's frame runs,
//! seen from a camera, with no input, so that a watcher draws what a participant does except what is a participant's
//! own, [`PERSONAL`]. See `design/spectator.md`.
use std::time::Instant;

use glam::Mat4;
use tomori_client::feedback::Feedback;

use super::{Frame, Marker, Senses, Step};
use crate::{controller::Pose, controller_ui::Sheet, conversation::Conversation};

/// Each thing a [`Frame`] draws or reports, so that what a watcher leaves out of a participant's frame is declared, by
/// [`PERSONAL`], and everything else is drawn by both. [`Frame::elements`] names every field of a frame as one of
/// these.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Element {
    Scenery,
    Swarm,
    Marker,
    Attracting,
    Card,
    Content,
    Cubes,
    Stations,
    Dials,
    SignFacing,
    Workspace,
    Arrival,
    Pill,
    Indicator,
    Wrist,
    Listener,
    Agent,
    Presence,
    Cards,
    Projection,
    Images,
    Volumes,
    Landing,
    Blink,
    Pads,
    Environment,
    Carriage,
    Avatar,
    HomeRing,
}

/// What a watcher leaves out of a participant's frame, as the participant's own: the attractor, arrival's objects with
/// the hand menu and the rays among them, the status pill, the recording indicator, the wrist's transcript, the
/// presence a participant reports, and the teleport being aimed, its blink and the pads it may take. Everything else a
/// participant's frame holds, a watcher's holds too. See `design/spectator.md`.
pub const PERSONAL: [Element; 10] = [
    Element::Marker,
    Element::Attracting,
    Element::Arrival,
    Element::Pill,
    Element::Indicator,
    Element::Wrist,
    Element::Presence,
    Element::Landing,
    Element::Blink,
    Element::Pads,
];

impl Frame<'_> {
    /// Every element of the frame, with what it holds in words, for comparing one frame with another: each field is
    /// named, so that a field added to [`Frame`] is named here too, and a watcher draws it unless [`PERSONAL`] says
    /// otherwise. What an environment's client side or an avatar shows is the client side's own, and is told only by
    /// whether there is any.
    pub fn elements(&self) -> [(Element, String); 29] {
        let Frame {
            scenery,
            swarm,
            marker,
            attracting,
            card,
            content,
            cubes,
            stations,
            dials,
            sign_facing,
            workspace,
            arrival,
            panels:
                super::Panels {
                    pill,
                    indicator,
                    wrist,
                },
            listener,
            agent,
            presence,
            cards,
            projection,
            images,
            volumes,
            landing,
            blink,
            pads,
            environment,
            carriage,
            avatar,
            home_ring,
        } = self;
        let agents: Vec<_> = swarm.agents.iter().map(|agent| agent.position).collect();
        [
            (Element::Scenery, format!("{scenery:?}")),
            (
                Element::Swarm,
                format!("{agents:?} {:?} {:?}", swarm.visual, swarm.anchor()),
            ),
            (Element::Marker, format!("{marker:?}")),
            (Element::Attracting, format!("{attracting:?}")),
            (Element::Card, format!("{card:?}")),
            (Element::Content, format!("{content:?}")),
            (Element::Cubes, format!("{cubes:?}")),
            (Element::Stations, format!("{stations:?}")),
            (Element::Dials, format!("{dials:?}")),
            (Element::SignFacing, format!("{sign_facing:?}")),
            (Element::Workspace, format!("{workspace:?}")),
            (Element::Arrival, format!("{arrival:?}")),
            (Element::Pill, format!("{pill:?}")),
            (Element::Indicator, format!("{indicator:?}")),
            (Element::Wrist, format!("{wrist:?}")),
            (Element::Listener, format!("{listener:?}")),
            (Element::Agent, format!("{agent:?}")),
            (Element::Presence, format!("{presence:?}")),
            (Element::Cards, format!("{cards:?}")),
            (Element::Projection, format!("{projection:?}")),
            (Element::Images, format!("{images:?}")),
            (Element::Volumes, format!("{volumes:?}")),
            (Element::Landing, format!("{landing:?}")),
            (Element::Blink, format!("{blink:?}")),
            (Element::Pads, format!("{pads:?}")),
            (Element::Environment, format!("{:?}", environment.is_some())),
            (Element::Carriage, format!("{carriage:?}")),
            (
                Element::Avatar,
                format!(
                    "{:?}",
                    avatar
                        .as_ref()
                        .map(|shown| (shown.avatar, shown.space, shown.drawn.is_some()))
                ),
            ),
            (Element::HomeRing, format!("{home_ring:?}")),
        ]
    }

    /// Leave `element` out of the frame, as a watcher does each of [`PERSONAL`], returning whether it is one a frame
    /// can be without; what every viewer of the space shares it cannot.
    pub fn withhold(&mut self, element: Element) -> bool {
        match element {
            Element::Marker => self.marker = None,
            Element::Attracting => self.attracting = false,
            Element::Arrival => self.arrival = None,
            Element::Pill => self.panels.pill = Sheet::default(),
            Element::Indicator => self.panels.indicator = Default::default(),
            Element::Wrist => self.panels.wrist = Sheet::default(),
            Element::Presence => self.presence = None,
            Element::Landing => self.landing = None,
            Element::Blink => self.blink = 0.0,
            Element::Pads => self.pads.clear(),
            Element::Scenery
            | Element::Swarm
            | Element::Card
            | Element::Content
            | Element::Cubes
            | Element::Stations
            | Element::Dials
            | Element::SignFacing
            | Element::Workspace
            | Element::Listener
            | Element::Agent
            | Element::Cards
            | Element::Projection
            | Element::Images
            | Element::Volumes
            | Element::Environment
            | Element::Carriage
            | Element::Avatar
            | Element::HomeRing => return false,
        }
        true
    }
}

/// What a watcher senses: its camera, `camera`, as its head and the viewer that what faces a viewer faces, focused so
/// that it sees the agent's activity, and nothing else: no rays, triggers, grips, palms, controllers or hands, and no
/// attractor drawn.
fn senses(camera: Mat4) -> Senses {
    Senses {
        focused: true,
        head: Some(camera),
        head_pose: Pose::from_transform(camera),
        viewer: Some(camera.w_axis.truncate()),
        space_head: camera,
        marker: Marker::Hidden,
        ..Senses::default()
    }
}

impl Step {
    /// One frame as a watcher sees it, such as a spectator, from `camera`, the head of the participant it follows or
    /// the place a participant joining the space stands: [`Self::update`], the participant's own frame step, with no
    /// input, so that nothing is pointed at, hovered, chosen or carried, and without what [`PERSONAL`] names. As a
    /// watcher, it also holds no rights over walls, whatever its role, so that it draws no handles or padlocks; its
    /// environment's client side is told no participant, what that asks its server side is dropped, its sounds are
    /// not played and its passes not felt; and it plays no ambience. A step that has watched stays a watcher. See
    /// `design/spectator.md`.
    pub fn watch(
        &mut self,
        camera: Mat4,
        conversation: &mut Conversation,
        feedback: &mut Feedback,
        now: Instant,
        dt: f32,
    ) -> Frame<'_> {
        self.watching = true;
        let mut frame = self.update(&senses(camera), conversation, feedback, now, dt);
        for element in PERSONAL {
            frame.withhold(element);
        }
        frame
    }

    /// How many intents its environment's client sides have asked while it watched, all dropped, since a watcher asks
    /// nothing.
    pub fn dropped(&self) -> usize {
        self.dropped
    }
}

#[cfg(test)]
mod tests;
