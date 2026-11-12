// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What the tests of editing walls share: the rights of a participant who may adjust and snap walls, a ray from home,
//! the projection shown and a frame of the projector's input.
use std::time::Instant;

use glam::{Vec2, Vec3};
use tomori_protocol::{Panel, Projection, Wall};

use super::Rights;
use crate::arrival::HOME;
use crate::pointing::Ray;
use crate::projection::{Controls, Hold, Input};

/// A participant who may adjust walls and snap them together, but not remove them.
pub(super) const ADJUST: Rights = Rights {
    adjust: true,
    remove: false,
    snap: true,
    locks: false,
    lock: false,
};

/// A ray from home at `point`.
pub(super) fn at(point: Vec3) -> Option<Ray> {
    Ray::new(HOME, point - HOME)
}

/// The projection in Sky showing `walls` and `panels`.
pub(super) fn shown(walls: Vec<Wall>, panels: Vec<Panel>) -> Projection {
    Projection::new("s", "sky")
        .with_walls(walls)
        .with_panels(panels)
}

/// A frame of the projector's input at `now`, seen from home with controllers: `ray` and `grip` pointing at what
/// `projection` shows, for a participant with `rights`.
pub(super) fn frame(
    projection: &Projection,
    ray: Option<Ray>,
    grip: f32,
    rights: Rights,
    now: Instant,
) -> Input<'_> {
    Input {
        projection: Some(projection),
        ray,
        grip: Some(grip),
        viewer: Some(HOME),
        active: true,
        pointing: true,
        nearer: f32::INFINITY,
        stick: Vec2::ZERO,
        straighten: false,
        free: false,
        controls: Controls::Controllers,
        scroll: None,
        rights,
        surfaces: None,
        left: Hold::default(),
        now,
        dt: 1.0 / 72.0,
    }
}
