// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Moving what a participant carries with the right stick: forward and back push it further along the ray or pull it
//! closer, smoothly, and left and right turn it about the vertical in snaps of [`YAW_STEP`], one a flick, from where it
//! faces the participant, which a click of the stick returns it to. Pure, so that it is tested without a device. A
//! projected panel uses it now; a 3D object carried later is to add its other axes as further snapped turns, with the same
//! gestures. See `design/projection.md`.
use glam::Vec2;

/// The nearest and farthest something carried goes along the ray, in metres.
pub const NEAREST: f32 = 0.3;
pub const FARTHEST: f32 = 5.0;
/// How fast a full push moves something carried along the ray, as a proportion of how far it is, each second: it moves by
/// the same proportion of its distance near or far, so that it feels alike at arm's length and across the room.
pub const RATE: f32 = 1.2;
/// How far one flick turns something carried, in radians: 10°.
pub const YAW_STEP: f32 = std::f32::consts::PI / 18.0;
/// The snaps in a whole turn.
const STEPS: i32 = 36;
/// How far forward or back the stick goes before it moves something carried, and how far sideways before it turns it.
const DEPTH: f32 = 0.3;
const TURN: f32 = 0.6;
/// How near the middle the stick returns to end a gesture.
const CENTRE: f32 = 0.2;

/// What the right stick is doing to something carried: each push is one gesture, owned by the axis that passed its
/// threshold first, until the stick returns to the middle, as with travel's left stick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Gesture {
    /// At the middle: the next push starts a gesture.
    #[default]
    Ready,
    /// Forward or back came first: it moves along the ray for as long as the stick is pushed.
    Depth,
    /// Sideways came first, and turned it: nothing more until the middle.
    Spent,
}

/// How something carried is held: how far along the ray, and how far it is turned from facing the participant, in snaps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Manipulation {
    distance: f32,
    steps: i32,
    gesture: Gesture,
}

impl Manipulation {
    /// Held `distance` along the ray, within [`NEAREST`] and [`FARTHEST`], facing the participant.
    pub fn new(distance: f32) -> Self {
        Self {
            distance: if distance.is_finite() {
                distance.clamp(NEAREST, FARTHEST)
            } else {
                NEAREST
            },
            steps: 0,
            gesture: Gesture::Ready,
        }
    }

    /// How far along the ray it is held, in metres.
    pub fn distance(&self) -> f32 {
        self.distance
    }

    /// How far it is turned from facing the participant, about the vertical, in radians: a whole number of
    /// [`YAW_STEP`]s, from just past half a turn one way to half a turn the other.
    pub fn yaw(&self) -> f32 {
        self.steps as f32 * YAW_STEP
    }

    /// Whether it faces the participant square, unturned.
    pub fn square(&self) -> bool {
        self.steps == 0
    }

    /// Turn it back to face the participant square, as a click of the stick does.
    pub fn straighten(&mut self) {
        self.steps = 0;
    }

    /// Follow the right `stick` for `dt` seconds. Returns whether it turned a snap.
    pub fn update(&mut self, stick: Vec2, dt: f32) -> bool {
        // A stick or a frame that reads nonsense is at the middle, and stands still.
        let stick = if stick.is_finite() { stick } else { Vec2::ZERO };
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        if stick.length() < CENTRE {
            self.gesture = Gesture::Ready;
            return false;
        }
        let mut turned = false;
        if self.gesture == Gesture::Ready {
            let (across, along) = (stick.x.abs(), stick.y.abs());
            if along > DEPTH && along >= across {
                self.gesture = Gesture::Depth;
            } else if across > TURN && across > along {
                self.gesture = Gesture::Spent;
                // Right turns it clockwise, seen from above, as a snap turn turns the participant.
                self.steps = (self.steps - stick.x.signum() as i32 + STEPS / 2 - 1)
                    .rem_euclid(STEPS)
                    - STEPS / 2
                    + 1;
                turned = true;
            }
        }
        if self.gesture == Gesture::Depth {
            // How far past the middle, from 0 to 1, moves it by that much of its rate, exponentially, so that two short
            // frames move it as far as one long one.
            let push = ((stick.y.abs() - CENTRE) / (1.0 - CENTRE)).clamp(0.0, 1.0);
            let factor = (RATE * push * dt * stick.y.signum()).exp();
            self.distance = (self.distance * factor).clamp(NEAREST, FARTHEST);
        }
        turned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f32 = 1.0 / 72.0;

    #[test]
    fn it_is_held_within_reach() {
        assert_eq!(Manipulation::new(2.0).distance(), 2.0);
        assert_eq!(Manipulation::new(0.1).distance(), NEAREST);
        assert_eq!(Manipulation::new(9.0).distance(), FARTHEST);
        assert_eq!(Manipulation::new(f32::NAN).distance(), NEAREST);
        let held = Manipulation::new(2.0);
        assert!(held.square());
        assert_eq!(held.yaw(), 0.0);
    }

    #[test]
    fn forward_pushes_it_away_and_back_pulls_it_closer_smoothly_within_reach() {
        let mut held = Manipulation::new(1.0);
        // A full push for a second moves it by its rate, exponentially, whatever the frames.
        for _ in 0..72 {
            assert!(!held.update(Vec2::Y, FRAME));
        }
        assert!((held.distance() - RATE.exp()).abs() < 1e-3);
        let mut once = Manipulation::new(1.0);
        once.update(Vec2::Y, 1.0);
        assert!((once.distance() - held.distance()).abs() < 1e-3);
        // Smoothly: no frame moves it by more than its rate allows.
        let mut previous = held.distance();
        for _ in 0..72 {
            held.update(Vec2::NEG_Y, FRAME);
            let step = (held.distance() / previous).ln().abs();
            assert!(step <= RATE * FRAME + 1e-5, "{step}");
            previous = held.distance();
        }
        assert!((held.distance() - 1.0).abs() < 1e-3);
        // Halfway past the middle, it moves half as fast; eased off but not back to the middle, still a little.
        let mut half = Manipulation::new(1.0);
        half.update(Vec2::new(0.0, 0.6), 1.0);
        assert!((half.distance() - (RATE * 0.5).exp()).abs() < 1e-3);
        let before = half.distance();
        half.update(Vec2::new(0.0, 0.25), 1.0);
        assert!(half.distance() > before);
        // Never past either end.
        let mut far = Manipulation::new(4.0);
        far.update(Vec2::Y, 10.0);
        assert_eq!(far.distance(), FARTHEST);
        far.update(Vec2::ZERO, FRAME);
        far.update(Vec2::NEG_Y, 10.0);
        assert_eq!(far.distance(), NEAREST);
        // Short of its threshold, or with a frame or stick that reads nonsense, it stands still.
        let mut still = Manipulation::new(1.0);
        still.update(Vec2::new(0.0, 0.28), 1.0);
        still.update(Vec2::ZERO, FRAME);
        still.update(Vec2::Y, f32::NAN);
        still.update(Vec2::NAN, 1.0);
        assert_eq!(still.distance(), 1.0);
    }

    #[test]
    fn a_flick_sideways_turns_it_one_snap_and_rearms_at_the_middle() {
        let mut held = Manipulation::new(1.0);
        assert!(held.update(Vec2::new(0.9, 0.1), FRAME));
        assert!((held.yaw() + YAW_STEP).abs() < 1e-6, "right is clockwise");
        // Held, eased off or rolled forward, it turns no more and moves nowhere.
        for rolled in [Vec2::X, Vec2::new(0.4, 0.0), Vec2::new(0.0, 0.9)] {
            assert!(!held.update(rolled, 1.0), "{rolled}");
        }
        assert_eq!(held.distance(), 1.0);
        // Part of the way back is not the middle.
        held.update(Vec2::new(0.25, 0.0), FRAME);
        assert!(!held.update(Vec2::X, FRAME));
        held.update(Vec2::ZERO, FRAME);
        assert!(held.update(Vec2::NEG_X, FRAME));
        assert!(held.square());
        // Short of its threshold, or more forward than sideways, it does not turn.
        held.update(Vec2::ZERO, FRAME);
        assert!(!held.update(Vec2::new(0.55, 0.0), FRAME));
        held.update(Vec2::ZERO, FRAME);
        assert!(!held.update(Vec2::new(0.6, 0.7), FRAME));
        // A push along the ray owns it: rolling sideways then turns nothing.
        assert!(!held.update(Vec2::new(0.95, 0.1), FRAME));
        assert!(held.square());
    }

    #[test]
    fn its_turn_stays_in_snaps_within_a_turn_and_straightens_square() {
        let mut held = Manipulation::new(1.0);
        let flick = |held: &mut Manipulation, x: f32| {
            held.update(Vec2::ZERO, FRAME);
            held.update(Vec2::new(x, 0.0), FRAME)
        };
        for _ in 0..4 {
            flick(&mut held, -1.0);
        }
        assert!((held.yaw() - 4.0 * YAW_STEP).abs() < 1e-5);
        // All the way round, it comes back to where it started, always a whole number of snaps within half a turn.
        for _ in 0..STEPS {
            flick(&mut held, -1.0);
            assert!(held.yaw().abs() <= std::f32::consts::PI + 1e-5);
            let snaps = held.yaw() / YAW_STEP;
            assert!((snaps - snaps.round()).abs() < 1e-4);
        }
        assert!((held.yaw() - 4.0 * YAW_STEP).abs() < 1e-5);
        for _ in 0..STEPS {
            flick(&mut held, 1.0);
        }
        assert!((held.yaw() - 4.0 * YAW_STEP).abs() < 1e-5);
        // A click straightens it square.
        held.straighten();
        assert!(held.square());
        assert_eq!(held.yaw(), 0.0);
    }
}
