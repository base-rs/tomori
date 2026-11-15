// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Moving what a participant carries with the right stick: forward and back push it further along the ray or pull it
//! closer, smoothly, and left and right turn it about the vertical, continuously for as long as the stick is held,
//! faster the further it is pushed, a short flick turning it on to the next whole [`YAW_STEP`], from where it faces the
//! participant, which a click of the stick returns it to. Pure, so that it is tested without a device. A projected panel
//! and a wall carried use it now; a 3D object carried later is to add its other axes with the same gestures. See
//! `design/projection.md`.
use glam::Vec2;

/// The nearest and farthest something carried goes along the ray, in metres: as far as the ray once reached to put a
/// panel on a wall, so that a panel docks on a wall about 9 m away, pushed to it. See `design/projection.md`.
pub const NEAREST: f32 = 0.3;
pub const FARTHEST: f32 = 8.0;
/// How fast a full push moves something carried along the ray, as a proportion of how far it is, each second: it moves by
/// the same proportion of its distance near or far, so that it feels alike at arm's length and across the room.
pub const RATE: f32 = 1.2;
/// The whole steps a turn ticks at, and a flick turns on to, in radians: 10°.
pub const YAW_STEP: f32 = std::f32::consts::PI / 18.0;
/// How fast the stick held all the way sideways turns something carried, in radians a second: 90°, half a turn in two
/// seconds, so that turning a panel round to see its back is quick but never a blur; half as far past the dead zone,
/// half as fast.
pub const TURN_RATE: f32 = std::f32::consts::FRAC_PI_2;
/// How long a push sideways may last and still be a flick, in seconds: let go within it, it turns on to the next whole
/// [`YAW_STEP`], as a snap did before turning was continuous.
pub const FLICK: f32 = 0.25;
/// How far forward or back the stick goes before it moves something carried, and how far sideways before it turns it:
/// a small dead zone, so that a stick resting off its middle turns nothing.
const DEPTH: f32 = 0.3;
const TURN: f32 = 0.25;
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
    /// Sideways came first: it turns for as long as the stick is pushed, from `from`, by `turned` so far, for `held`
    /// seconds, first to the left where `left`.
    Turn {
        from: f32,
        turned: f32,
        held: f32,
        left: bool,
    },
}

/// How something carried is held: how far along the ray, and how far it is turned from facing the participant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Manipulation {
    distance: f32,
    yaw: f32,
    gesture: Gesture,
}

/// `angle` within half a turn either way, as a turn is kept.
fn wrapped(angle: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    (angle + PI).rem_euclid(TAU) - PI
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
            yaw: 0.0,
            gesture: Gesture::Ready,
        }
    }

    /// How far along the ray it is held, in metres.
    pub fn distance(&self) -> f32 {
        self.distance
    }

    /// How far it is turned from facing the participant, about the vertical, in radians, anticlockwise seen from above,
    /// within half a turn either way.
    pub fn yaw(&self) -> f32 {
        self.yaw
    }

    /// Whether it faces the participant square, unturned.
    pub fn square(&self) -> bool {
        self.yaw.abs() < 1e-6
    }

    /// Turn it back to face the participant square, as a click of the stick does.
    pub fn straighten(&mut self) {
        self.yaw = 0.0;
    }

    /// Follow the right `stick` for `dt` seconds. Returns whether it turned past a whole [`YAW_STEP`], or on to one, as
    /// a flick does, which the participant feels as a tick.
    pub fn update(&mut self, stick: Vec2, dt: f32) -> bool {
        // A stick or a frame that reads nonsense is at the middle, and stands still.
        let stick = if stick.is_finite() { stick } else { Vec2::ZERO };
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        if stick.length() < CENTRE {
            // A flick, let go as soon as it began, turns on to the next whole step, at least one from where it began.
            let flicked = match self.gesture {
                Gesture::Turn {
                    from,
                    turned,
                    held,
                    left,
                } if held < FLICK => {
                    let at = (from + turned) / YAW_STEP;
                    let step = if left {
                        (at + 1e-3).floor() + 1.0
                    } else {
                        (at - 1e-3).ceil() - 1.0
                    };
                    self.yaw = wrapped(step * YAW_STEP);
                    true
                }
                _ => false,
            };
            self.gesture = Gesture::Ready;
            return flicked;
        }
        let (across, along) = (stick.x.abs(), stick.y.abs());
        if self.gesture == Gesture::Ready {
            if along > DEPTH && along >= across {
                self.gesture = Gesture::Depth;
            } else if across > TURN && across > along {
                // Right turns it clockwise, seen from above, as a snap turn turns the participant.
                self.gesture = Gesture::Turn {
                    from: self.yaw,
                    turned: 0.0,
                    held: 0.0,
                    left: stick.x < 0.0,
                };
            }
        }
        match &mut self.gesture {
            Gesture::Depth => {
                // How far past the middle, from 0 to 1, moves it by that much of its rate, exponentially, so that two
                // short frames move it as far as one long one.
                let push = ((along - CENTRE) / (1.0 - CENTRE)).clamp(0.0, 1.0);
                let factor = (RATE * push * dt * stick.y.signum()).exp();
                self.distance = (self.distance * factor).clamp(NEAREST, FARTHEST);
                false
            }
            Gesture::Turn {
                from, turned, held, ..
            } => {
                // How far past the dead zone, from 0 to 1, turns it by that much of its rate, so that two short frames
                // turn it as far as one long one; a whole step passed ticks.
                let push = ((across - TURN) / (1.0 - TURN)).clamp(0.0, 1.0);
                let before = (*from + *turned) / YAW_STEP;
                *turned -= stick.x.signum() * TURN_RATE * push * dt;
                *held += dt;
                let after = (*from + *turned) / YAW_STEP;
                self.yaw = wrapped(*from + *turned);
                if after > before {
                    after.floor() > before.floor()
                } else {
                    after.ceil() < before.ceil()
                }
            }
            Gesture::Ready => false,
        }
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
    fn a_flick_sideways_turns_it_on_one_step_and_rearms_at_the_middle() {
        let mut held = Manipulation::new(1.0);
        assert!(!held.update(Vec2::new(0.9, 0.1), FRAME));
        assert!(
            held.update(Vec2::ZERO, FRAME),
            "a flick ticks as it turns on"
        );
        assert!((held.yaw() + YAW_STEP).abs() < 1e-6, "right is clockwise");
        // Rolled forward first, it moves along the ray and turns nothing.
        assert!(!held.update(Vec2::new(0.1, 0.9), 0.1));
        assert!(!held.update(Vec2::new(0.95, 0.1), 0.1));
        assert!(!held.update(Vec2::ZERO, FRAME));
        assert!((held.yaw() + YAW_STEP).abs() < 1e-6);
        // Left, back to square.
        held.update(Vec2::NEG_X, FRAME);
        held.update(Vec2::ZERO, FRAME);
        assert!(held.square());
        // Short of its dead zone, or more forward than sideways, it does not turn.
        assert!(!held.update(Vec2::new(0.24, 0.0), FRAME));
        assert!(!held.update(Vec2::ZERO, FRAME));
        assert!(!held.update(Vec2::new(0.6, 0.7), FRAME));
        assert!(!held.update(Vec2::ZERO, FRAME));
        assert!(held.square());
        // Part of the way back is not the middle: a flick is ended only there.
        held.update(Vec2::X, FRAME);
        held.update(Vec2::new(0.22, 0.0), FRAME);
        assert!(held.update(Vec2::new(0.1, 0.0), FRAME));
        assert!((held.yaw() + YAW_STEP).abs() < 1e-6);
    }

    #[test]
    fn held_sideways_it_turns_continuously_faster_the_further_it_is_pushed_ticking_at_each_step() {
        let mut held = Manipulation::new(1.0);
        // A full push for a second turns it a quarter, clockwise, whatever the frames, ticking at each step it passes:
        // 9 of them, the last as the quarter is reached or just after.
        let mut ticks = (0..72).filter(|_| held.update(Vec2::X, FRAME)).count();
        assert!((held.yaw() + TURN_RATE).abs() < 1e-4, "{}", held.yaw());
        let mut past = held;
        ticks += usize::from(past.update(Vec2::X, FRAME));
        assert_eq!(ticks, 9);
        // Let go after more than a flick, it stays where it turned to.
        assert!(!held.update(Vec2::ZERO, FRAME));
        assert!((held.yaw() + TURN_RATE).abs() < 1e-4);
        let mut once = Manipulation::new(1.0);
        once.update(Vec2::X, 1.0);
        assert!((once.yaw() - held.yaw()).abs() < 1e-4);
        // Halfway past the dead zone, half as fast; anticlockwise to the left.
        let mut half = Manipulation::new(1.0);
        half.update(Vec2::new(-(0.25 + 0.75 / 2.0), 0.0), 1.0);
        assert!(
            (half.yaw() - TURN_RATE / 2.0).abs() < 1e-4,
            "{}",
            half.yaw()
        );
        // Smoothly: no frame turns it by more than its rate allows.
        let mut smooth = Manipulation::new(1.0);
        let mut previous = smooth.yaw();
        for _ in 0..72 {
            smooth.update(Vec2::NEG_X, FRAME);
            assert!((smooth.yaw() - previous).abs() <= TURN_RATE * FRAME + 1e-5);
            previous = smooth.yaw();
        }
        // Swung across while held, it turns back the other way, as the same push.
        smooth.update(Vec2::X, 0.5);
        assert!((smooth.yaw() - TURN_RATE / 2.0).abs() < 1e-4);
        // Eased off, or with a frame or stick that reads nonsense, it stands still.
        let mut still = Manipulation::new(1.0);
        still.update(Vec2::X, f32::NAN);
        assert!(still.square());
        let mut nonsense = Manipulation::new(1.0);
        nonsense.update(Vec2::NAN, 1.0);
        assert!(nonsense.square());
    }

    #[test]
    fn a_flick_after_turning_freely_turns_on_to_the_next_whole_step_and_it_stays_within_half_a_turn()
     {
        let mut held = Manipulation::new(1.0);
        // Turned a little, then flicked the same way, it turns on to the next whole step past where it is.
        held.update(Vec2::NEG_X, 0.3);
        held.update(Vec2::ZERO, FRAME);
        let turned = held.yaw();
        assert!((turned - 0.3 * TURN_RATE).abs() < 1e-4);
        held.update(Vec2::NEG_X, FRAME);
        held.update(Vec2::ZERO, FRAME);
        assert!((held.yaw() - 3.0 * YAW_STEP).abs() < 1e-4, "{}", held.yaw());
        // Flicked back, one whole step back.
        held.update(Vec2::X, FRAME);
        held.update(Vec2::ZERO, FRAME);
        assert!((held.yaw() - 2.0 * YAW_STEP).abs() < 1e-4, "{}", held.yaw());
        // All the way round, it comes back to where it started, always within half a turn either way.
        for _ in 0..40 {
            held.update(Vec2::NEG_X, 0.1);
            assert!(held.yaw().abs() <= std::f32::consts::PI + 1e-5);
        }
        assert!((held.yaw() - 2.0 * YAW_STEP).abs() < 1e-3, "{}", held.yaw());
        held.update(Vec2::ZERO, FRAME);
        for _ in 0..36 {
            held.update(Vec2::X, FRAME);
            held.update(Vec2::ZERO, FRAME);
        }
        assert!((held.yaw() - 2.0 * YAW_STEP).abs() < 1e-3, "{}", held.yaw());
        // A click straightens it square.
        held.straighten();
        assert!(held.square());
        assert_eq!(held.yaw(), 0.0);
    }
}
