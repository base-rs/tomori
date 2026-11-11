// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! How the agent's place moves and turns when it goes somewhere, as a hovering creature would. A **lead** sets off
//! along the way at once, at up to [`SPEED`], or far away, streaming, at the speed a worker's flock flies, [`STREAM`],
//! through the waypoint the server gives where the straight way is blocked, slowing round it; the place follows the
//! lead on a critically damped spring, [`FOLLOW`], integrated exactly, so that it eases in, cruises no faster than the
//! lead and settles without overshooting, and a new place to go, given on its way, carries on from how it moves rather
//! than starting again from rest. Its turn is the same on its orientation, a quaternion: a lead turns along the
//! shortest arc at up to [`TURN_SPEED`], and the orientation follows it on a spring, [`TURN`], so that a turn eases in
//! and out and a new one carries on from how it turns. Going further than [`SIDESTEP`], it turns towards where it goes
//! as it sets off, and back to face the way it is to face as it arrives, from [`ARRIVE`] away; nearer, it sidesteps,
//! facing as it is to face. It depends only on where the server says it stands, never on when the server said so, so
//! every client converges on the same place. The participant is never moved, so this is an object moving in the
//! scene, not travel. See `design/environments.md`.
use glam::{Quat, Vec3};

use crate::anchors::Anchor;

/// How fast the agent glides, at most, in metres a second: as fast as the swarm's agents fly.
pub const SPEED: f32 = 0.85;

/// How fast the agent streams, at most, in metres a second: as fast as a worker's flock flies.
pub const STREAM: f32 = 6.0;

/// How closely the agent's place follows its lead as it glides: a critically damped spring's rate, per second, so
/// that it reaches nine tenths of its speed in about 0.8 s and trails its lead by `2 × SPEED / FOLLOW`, 0.34 m.
pub const FOLLOW: f32 = 5.0;

/// How closely it follows its lead as it streams, faster, so that it trails a fast lead by no more than 1.2 m.
pub const STREAM_FOLLOW: f32 = 10.0;

/// How far before and after a waypoint, in metres, the lead slows to round it, so that the place passes close by it
/// rather than cutting the corner.
pub const CORNER: f32 = 0.6;

/// The share of its speed the lead keeps at a waypoint.
pub const CORNER_SPEED: f32 = 0.3;

/// How fast the agent's lead turns, at most, in radians a second: 90° a second, so that a turn is gentle, never a
/// spin, before whoever it faces.
pub const TURN_SPEED: f32 = std::f32::consts::FRAC_PI_2;

/// How closely its orientation follows its lead's: a critically damped spring's rate, per second, so that a small turn,
/// as the one it makes to keep facing someone who moves, eases in and out over about 0.8 s.
pub const TURN: f32 = 6.0;

/// How far it goes, in metres, within which it sidesteps, facing as it is to face, rather than turning to where it
/// goes.
pub const SIDESTEP: f32 = 0.8;

/// How far from where it goes, in metres, it begins turning to face the way it is to face there, wholly by
/// [`SIDESTEP`].
pub const ARRIVE: f32 = 1.6;

/// How far its lead may stand ahead of it, about the vertical, in metres, before it faces where it goes: wholly from
/// [`AHEAD`], so that it never turns for a lead barely ahead of it, nor going straight up or down.
const ABREAST: f32 = 0.01;
const AHEAD: f32 = 0.06;

/// The longest step it is moved by at once, in seconds, so that it moves alike at any frame rate.
const STEP: f32 = 1.0 / 240.0;

/// How near, in metres, and how slowly, in metres a second, it stands and moves before it stands exactly where it
/// goes, a step too small to see; and the same, in radians and radians a second, for its turn.
const SETTLED: f32 = 3e-5;
const STILL: f32 = 3e-4;

/// Where the agent's place is as it moves, frame by frame.
#[derive(Clone, Debug)]
pub struct Glide {
    /// Where it is now, once it has been anywhere.
    at: Option<Anchor>,
    /// Where its lead is, and which way it faces.
    lead: Anchor,
    /// How far it stands from its lead, and how fast it moves.
    offset: Vec3,
    velocity: Vec3,
    /// How far it is turned from its lead, as an axis scaled by an angle, and how fast it turns.
    off: Vec3,
    spin: Vec3,
    /// Where it is going, and the waypoint it goes by, until its lead passes it.
    target: Option<Vec3>,
    via: Option<Vec3>,
    /// The waypoint its lead passed, until it is a [`CORNER`] beyond it.
    passed: Option<Vec3>,
}

impl Default for Glide {
    fn default() -> Self {
        Self {
            at: None,
            lead: Anchor::AGENT,
            offset: Vec3::ZERO,
            velocity: Vec3::ZERO,
            off: Vec3::ZERO,
            spin: Vec3::ZERO,
            target: None,
            via: None,
            passed: None,
        }
    }
}

impl Glide {
    /// Move on by `dt` seconds towards `target`, by `via` where it is given, streaming where `stream` says, or appear at
    /// `target` at once where `appear` says, as when a space opens: where the agent's place is now.
    pub fn step(
        &mut self,
        target: Anchor,
        via: Option<Vec3>,
        stream: bool,
        appear: bool,
        dt: f32,
    ) -> Anchor {
        let dt = dt.clamp(0.0, 0.1);
        let Some(at) = self.at.filter(|_| !appear) else {
            *self = Self {
                at: Some(target),
                lead: target,
                target: Some(target.position),
                ..Self::default()
            };
            return target;
        };
        // A new place to go: by its waypoint, if it has one. Its lead sets off from wherever it is.
        if self
            .target
            .is_none_or(|going| going.distance(target.position) > 1e-3)
        {
            self.target = Some(target.position);
            self.via = via.filter(|via| via.is_finite());
        }
        let steps = (dt / STEP).ceil();
        let h = if steps > 0.0 { dt / steps } else { 0.0 };
        let mut at = at;
        for _ in 0..steps as usize {
            at = self.advance(target, stream, h);
        }
        self.at = Some(at);
        at
    }

    /// Move on by `h` seconds towards `target`. Where it stands and faces are kept as offsets from its lead, which keep
    /// their precision as they shrink, so that it settles exactly however far from the space's origin it stands.
    fn advance(&mut self, target: Anchor, stream: bool, h: f32) -> Anchor {
        let (fastest, follow) = if stream {
            (STREAM, STREAM_FOLLOW)
        } else {
            (SPEED, FOLLOW)
        };
        // The lead moves along the way, slowing about a waypoint.
        let next = self.via.unwrap_or(target.position);
        let ahead = next - self.lead.position;
        let distance = ahead.length();
        let mut corner = 1.0f32;
        if self.via.is_some() {
            corner = corner.min(distance / CORNER);
        }
        if let Some(passed) = self.passed {
            let beyond = self.lead.position.distance(passed);
            if beyond >= CORNER {
                self.passed = None;
            }
            corner = corner.min(beyond / CORNER);
        }
        let speed = fastest * corner.max(CORNER_SPEED);
        let moved = if distance > 0.0 {
            ahead * (speed * h / distance).min(1.0)
        } else {
            Vec3::ZERO
        };
        self.lead.position += moved;
        if self.lead.position.distance(next) < 1e-6 {
            self.lead.position = next;
            if let Some(via) = self.via.take() {
                self.passed = Some(via);
            }
        }
        (self.offset, self.velocity) =
            follow_spring(self.offset, self.velocity, moved / h, follow, h);
        let position = self.lead.position + self.offset;
        // It faces where it goes on its way, and the way it is to face as it arrives, or nearby.
        let next = self.via.unwrap_or(target.position);
        let left =
            position.distance(next) + self.via.map_or(0.0, |via| via.distance(target.position));
        let mut going = -self.offset;
        going.y = 0.0;
        let travel =
            smoothstep(SIDESTEP, ARRIVE, left) * smoothstep(ABREAST, AHEAD, going.length());
        let facing = if travel > 0.0 {
            let way = Quat::from_rotation_y(going.x.atan2(going.z));
            target.orientation.slerp(way, travel)
        } else {
            target.orientation
        };
        // The lead turns towards it by the shortest arc, as the slerp does, and the orientation follows the lead.
        let turn = rotation(facing * self.lead.orientation.inverse());
        let turned = turn.clamp_length_max(TURN_SPEED * h);
        let faces = turned == turn;
        self.lead.orientation = if faces {
            facing
        } else {
            (Quat::from_scaled_axis(turned) * self.lead.orientation).normalize()
        };
        (self.off, self.spin) = follow_spring(self.off, self.spin, turned / h, TURN, h);
        // Arrived and still, it stands exactly where it goes, facing exactly as it is to face.
        if self.lead.position == target.position
            && self.offset.length() < SETTLED
            && self.velocity.length() < STILL
        {
            self.offset = Vec3::ZERO;
            self.velocity = Vec3::ZERO;
        }
        if faces && self.off.length() < SETTLED && self.spin.length() < STILL {
            self.off = Vec3::ZERO;
            self.spin = Vec3::ZERO;
        }
        Anchor {
            position: self.lead.position + self.offset,
            orientation: if self.off == Vec3::ZERO {
                self.lead.orientation
            } else {
                (Quat::from_scaled_axis(self.off) * self.lead.orientation).normalize()
            },
        }
    }
}

/// A critically damped spring at `rate`, `offset` from a lead moving at `lead` with its own `velocity`, moved on by `h`
/// seconds, exactly while the lead moves steadily: its offset from where the lead is then, and its velocity.
fn follow_spring(offset: Vec3, velocity: Vec3, lead: Vec3, rate: f32, h: f32) -> (Vec3, Vec3) {
    // Offset by how far it trails a steady lead, it is a spring at rest at the lead.
    let trail = 2.0 * lead / rate;
    let (z, dz) = (offset + trail, velocity - lead);
    let decay = (-rate * h).exp();
    let c = dz + rate * z;
    let z = (z + c * h) * decay;
    let dz = (dz - rate * c * h) * decay;
    (z - trail, dz + lead)
}

/// The rotation `q` makes, by the shortest arc, as its axis scaled by its angle.
fn rotation(q: Quat) -> Vec3 {
    let q = if q.w < 0.0 { -q } else { q };
    let axis = Vec3::new(q.x, q.y, q.z);
    let sin = axis.length();
    if sin < 1e-7 {
        axis * 2.0
    } else {
        axis * (2.0 * sin.atan2(q.w) / sin)
    }
}

/// 0 below `from`, 1 above `to`, and eased between.
fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests;
