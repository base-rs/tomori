// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A One Euro filter ([Casiez, Roussel and Vogel, CHI 2012](https://gery.casiez.net/1euro/)) on a head's position and orientation: a first-order low-pass whose cutoff is `min_cutoff + beta × speed`, so that a still head is smoothed heavily, which is where tremor shows, and a moving one is followed, which is where lag does. The speed is itself low-passed at 1 Hz, so that one noisy frame does not open the filter.
//!
//! The orientation is filtered as a rotation: the filtered orientation moves towards the input by a fraction of the rotation between them, found along the shortest arc and expressed about the filtered orientation's own axes, and is renormalised, so that it is a unit quaternion whatever the input. The fraction for the part of the rotation that tilts the horizon, the roll, is the rest's times `tuning.roll`, less than one: roll is damped more than yaw and pitch, which keeps the horizon level without forcing the head upright, since a deliberate tilt still arrives, later.
//!
//! The roll is measured against the horizon, not the view axis: a turn about the world's vertical, at any pitch, has a component about the view axis that is the sine of the pitch, and damping that as roll would curve the turn and tilt the horizon. So the rotation is split about the axis in the view's Y–Z plane that tilts the camera's right vector out of the horizontal ([`horizon_axis`]), which is the view axis when looking level and turns towards the camera's own Y axis as it pitches up or down, and about the others, which tilt nothing. Looking straight up or down the split is still well defined, since the view axis is then a turn about the vertical, which is let through; only a camera rolled a quarter turn has no such axis, and then the view axis is used.
use super::{Placed, Tuning};
use glam::{Quat, Vec3};
use std::f32::consts::TAU;

/// The cutoff of the low-pass on the speed, in hertz.
const SPEED_CUTOFF: f32 = 1.0;

/// How far a first-order low-pass with this cutoff moves to its input in `dt` seconds.
fn factor(cutoff: f32, dt: f32) -> f32 {
    dt / (dt + 1.0 / (TAU * cutoff))
}

/// The rotation `q` as a rotation vector, its axis scaled by its angle along the shortest arc, in radians.
pub(super) fn rotation_vector(q: Quat) -> Vec3 {
    let q = if q.w < 0.0 { -q } else { q };
    let sine = q.xyz().length();
    if sine < 1e-6 {
        q.xyz() * 2.0
    } else {
        q.xyz() / sine * (2.0 * sine.atan2(q.w))
    }
}

/// The axis, in the view's own axes, that a rotation of the camera tilts its horizon about: the camera's right vector leaves the horizontal plane as the camera turns about it, by the rate of the rotation's part along it, and a turn about any axis at right angles to it, the right vector itself, or the world's vertical, leaves it be. The view axis, Z, when the camera is level; the Y axis when it looks straight up or down; and Z again when the camera lies on its side, where no axis does.
pub(super) fn horizon_axis(orientation: Quat) -> Vec3 {
    let up = orientation.conjugate() * Vec3::Y;
    let axis = Vec3::new(0.0, -up.z, up.y);
    let length = axis.length();
    if length < 1e-3 {
        Vec3::Z
    } else {
        axis / length
    }
}

/// The state of the filter on one head, from the last frame.
pub(super) struct Filter {
    /// Which stretch of continuous motion it filters: it starts over where the head was cut.
    pub epoch: u32,
    position: Vec3,
    raw_position: Vec3,
    /// The low-passed speed of the input, in metres a second.
    speed: f32,
    orientation: Quat,
    raw_orientation: Quat,
    /// The low-passed angular speed of the input, in radians a second.
    turn: f32,
}

impl Filter {
    /// A filter at rest at `head`, which is where its first output is.
    pub fn new(head: Placed, epoch: u32) -> Self {
        Self {
            epoch,
            position: head.position,
            raw_position: head.position,
            speed: 0.0,
            orientation: head.orientation,
            raw_orientation: head.orientation,
            turn: 0.0,
        }
    }

    /// Where the filter is.
    pub fn output(&self) -> Placed {
        Placed {
            position: self.position,
            orientation: self.orientation,
        }
    }

    /// Move `dt` seconds, `dt` above zero, to `head`, the input of that frame, and return where it is.
    pub fn step(&mut self, head: Placed, dt: f32, tuning: &Tuning) -> Placed {
        let speed = (head.position - self.raw_position).length() / dt;
        self.speed += factor(SPEED_CUTOFF, dt) * (speed - self.speed);
        self.raw_position = head.position;
        let cutoff = tuning.position.min_cutoff + tuning.position.beta * self.speed;
        self.position += factor(cutoff, dt) * (head.position - self.position);

        let turn =
            rotation_vector(self.raw_orientation.conjugate() * head.orientation).length() / dt;
        self.turn += factor(SPEED_CUTOFF, dt) * (turn - self.turn);
        self.raw_orientation = head.orientation;
        let cutoff = tuning.orientation.min_cutoff + tuning.orientation.beta * self.turn;
        let rest = factor(cutoff, dt);
        let roll = factor(cutoff * tuning.roll, dt);
        let remaining = rotation_vector(self.orientation.conjugate() * head.orientation);
        let tilting = horizon_axis(self.orientation);
        let tilt = remaining.dot(tilting);
        let moved = (remaining - tilting * tilt) * rest + tilting * (tilt * roll);
        self.orientation = (self.orientation * Quat::from_scaled_axis(moved)).normalize();
        self.output()
    }
}

#[cfg(test)]
mod tests;
