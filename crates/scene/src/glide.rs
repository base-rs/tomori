// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! How the agent's place moves when it goes somewhere: a glide that eases in over [`EASE`], cruises at up to [`SPEED`]
//! and settles over [`SETTLE`], through the waypoint the server gives where the straight way is blocked, or, far away,
//! a stream at the speed a worker's flock flies, [`STREAM`]; a move turned back on its way turns round smoothly, never
//! jumping; and a turn to face someone, quick and eased. The participant is never moved, so this is an object moving
//! in the scene, not travel. See `design/environments.md`.
use glam::{Quat, Vec3};

use crate::anchors::Anchor;

/// How fast the agent glides, at most, in metres a second: as fast as the swarm's agents fly.
pub const SPEED: f32 = 0.85;

/// How fast the agent streams, at most, in metres a second: as fast as a worker's flock flies.
pub const STREAM: f32 = 6.0;

/// How long it takes to reach its speed, in seconds.
pub const EASE: f32 = 0.4;

/// How long it takes to settle from its speed, in seconds.
pub const SETTLE: f32 = 0.5;

/// How quickly it turns to face the way it is to face: the share of the turn left that remains after a second, as a
/// rate, so that most of a turn is made in about a quarter of a second.
pub const TURN: f32 = 12.0;

/// Where the agent's place is as it moves, frame by frame.
#[derive(Clone, Debug, Default)]
pub struct Glide {
    /// Where it is now, once it has been anywhere.
    at: Option<Anchor>,
    velocity: Vec3,
    /// Where it is going, and the waypoint it goes by, until it passes it.
    target: Option<Vec3>,
    via: Option<Vec3>,
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
        let Some(mut at) = self.at.filter(|_| !appear) else {
            self.velocity = Vec3::ZERO;
            self.target = Some(target.position);
            self.via = None;
            self.at = Some(target);
            return target;
        };
        // A new place to go: by its waypoint, if it has one.
        if self
            .target
            .is_none_or(|going| going.distance(target.position) > 1e-3)
        {
            self.target = Some(target.position);
            self.via = via.filter(|via| via.is_finite());
        }
        let fastest = if stream { STREAM } else { SPEED };
        let next = self.via.unwrap_or(target.position);
        let left =
            at.position.distance(next) + self.via.map_or(0.0, |via| via.distance(target.position));
        if left < 1e-4 {
            self.velocity = Vec3::ZERO;
            self.via = None;
            at.position = target.position;
        } else {
            // As fast as it may go while still settling by the end, turning towards where it goes.
            let settling = (2.0 * fastest / SETTLE * left).sqrt();
            let wanted = (next - at.position).normalize_or_zero() * fastest.min(settling);
            let change = (wanted - self.velocity).clamp_length_max(fastest / EASE * dt);
            self.velocity += change;
            let step = self.velocity * dt;
            let to_next = next - at.position;
            if step.length() >= to_next.length() && step.dot(to_next) > 0.0 {
                at.position = next;
                if self.via.take().is_none() {
                    self.velocity = Vec3::ZERO;
                }
            } else {
                at.position += step;
            }
        }
        at.orientation = turned(at.orientation, target.orientation, dt);
        self.at = Some(at);
        at
    }
}

/// `from` turned towards `to` by `dt` seconds of [`TURN`].
fn turned(from: Quat, to: Quat, dt: f32) -> Quat {
    let share = 1.0 - (-TURN * dt).exp();
    from.slerp(to, share).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(position: Vec3) -> Anchor {
        Anchor {
            position,
            orientation: Quat::IDENTITY,
        }
    }

    /// The places `glide` passes through, frame by frame at 72 Hz, for `seconds`, going to `target`.
    fn frames(
        glide: &mut Glide,
        target: Anchor,
        via: Option<Vec3>,
        stream: bool,
        seconds: f32,
    ) -> Vec<Anchor> {
        let dt = 1.0 / 72.0;
        (0..(seconds / dt) as usize)
            .map(|_| glide.step(target, via, stream, false, dt))
            .collect()
    }

    #[test]
    fn it_appears_where_it_first_stands_and_where_it_is_told_to() {
        let mut glide = Glide::default();
        let home = at(Vec3::new(0.0, 1.4, -2.3));
        assert_eq!(glide.step(home, None, false, false, 0.1), home);
        let yard = at(Vec3::new(4.0, 1.4, -6.0));
        assert_eq!(glide.step(yard, None, false, true, 0.1), yard);
    }

    #[test]
    fn it_eases_in_cruises_at_its_speed_and_settles_without_overshooting() {
        let mut glide = Glide::default();
        let home = at(Vec3::new(0.0, 1.4, -2.3));
        glide.step(home, None, false, false, 0.0);
        let there = at(Vec3::new(3.0, 1.4, -2.3));
        let frames = frames(&mut glide, there, None, false, 6.0);
        let dt = 1.0 / 72.0;
        let speeds: Vec<f32> = frames
            .windows(2)
            .map(|pair| pair[0].position.distance(pair[1].position) / dt)
            .collect();
        // Never faster than a glide, gaining speed gently, and never past the place.
        assert!(speeds.iter().all(|speed| *speed <= SPEED + 1e-3));
        assert!(speeds[0] < 0.1);
        assert!(speeds.iter().any(|speed| *speed > SPEED - 0.01));
        assert!(frames.iter().all(|frame| frame.position.x <= 3.0 + 1e-5));
        assert_eq!(frames.last().unwrap().position, there.position);
        // A stream is faster.
        let mut glide = Glide::default();
        glide.step(home, None, false, false, 0.0);
        let far = at(Vec3::new(9.0, 1.4, -2.3));
        let streamed = frames_until(&mut glide, far, None, true);
        assert!(streamed < 3.0, "{streamed}");
    }

    /// How long `glide` takes to reach `target`, in seconds.
    fn frames_until(glide: &mut Glide, target: Anchor, via: Option<Vec3>, stream: bool) -> f32 {
        let dt = 1.0 / 72.0;
        let mut seconds = 0.0;
        while glide.step(target, via, stream, false, dt).position != target.position {
            seconds += dt;
            assert!(seconds < 60.0, "never arrived");
        }
        seconds
    }

    #[test]
    fn it_goes_by_its_waypoint_and_turns_round_smoothly_when_sent_back() {
        let mut glide = Glide::default();
        let home = at(Vec3::new(0.0, 1.4, -2.3));
        glide.step(home, None, false, false, 0.0);
        let there = at(Vec3::new(2.0, 1.4, -2.3));
        let over = Vec3::new(1.0, 2.6, -2.3);
        let path = frames(&mut glide, there, Some(over), false, 8.0);
        // It rises to the waypoint before coming down to its place.
        assert!(path.iter().any(|frame| frame.position.y > 2.5));
        assert_eq!(path.last().unwrap().position, there.position);
        // Sent back on its way, it slows and turns round rather than reversing at once.
        let mut glide = Glide::default();
        glide.step(home, None, false, false, 0.0);
        frames(&mut glide, there, None, false, 1.5);
        let before = glide.step(there, None, false, false, 1.0 / 72.0);
        let back = frames(&mut glide, home, None, false, 0.2);
        assert!(back[0].position.x >= before.position.x - 1e-3);
        let path = frames(&mut glide, home, None, false, 8.0);
        assert_eq!(path.last().unwrap().position, home.position);
        // A waypoint that is not finite is ignored.
        let mut glide = Glide::default();
        glide.step(home, None, false, false, 0.0);
        let path = frames(&mut glide, there, Some(Vec3::NAN), false, 8.0);
        assert!(path.iter().all(|frame| frame.position.y == 1.4));
    }

    #[test]
    fn it_turns_quickly_to_face_the_way_it_is_to_face() {
        let mut glide = Glide::default();
        let home = at(Vec3::new(0.0, 1.4, -2.3));
        glide.step(home, None, false, false, 0.0);
        let turned = Anchor {
            orientation: Quat::from_rotation_y(1.0),
            ..home
        };
        let frames = frames(&mut glide, turned, None, false, 0.5);
        let angle = |frame: &Anchor| frame.orientation.angle_between(turned.orientation);
        assert!(angle(&frames[0]) > 0.8);
        assert!(angle(frames.last().unwrap()) < 0.01);
    }
}
