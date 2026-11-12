// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Holding a panel or a wall with both hands: each hand keeps the point it took along its own ray, and what is held
//! follows the middle of the two, turns about the vertical as the line between the hands turns, and grows or shrinks
//! about that middle as the hands spread apart or close together. Pure, so that it is tested without a device. See
//! `design/projection.md`.
//!
//! The size follows how far apart the hands themselves are, not the points their rays hold: the hands' spread is the
//! same gesture near and far, while the points held on something across a room move apart many times faster than the
//! hands as the wrists turn, and shake as much more. So a panel pointed at from across the room grows as one held at
//! arm's length does, and a hand held still holds its size still.
use glam::{Mat4, Quat, Vec3};

use crate::pointing::Ray;

/// The least spread between the hands that sizes anything against, in metres: hands that closed nearly together are
/// taken as this far apart, so that the first few centimetres of spreading do not grow it many times over.
pub const LEAST_SPREAD: f32 = 0.1;

/// How long the line between the hands must be across, in metres, to turn what is held: held one above the other, the
/// hands say no direction to turn it to, and it keeps the way it faces.
const LEAST_ACROSS: f32 = 0.05;

/// How a size is stepped, as a proportion of its own: in steps of 5%, as a wall's corner sizes its slots.
pub const STEP: f32 = 0.05;

/// Where both hands took hold of something: how far along each ray, the middle of the two points, the way the line
/// between the hands faced and how far apart they were, as the second closed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hold {
    along: [f32; 2],
    middle: Vec3,
    yaw: f32,
    spread: f32,
    /// The way the line between the hands last faced, kept while it says none.
    last: f32,
}

/// Where something held with both hands is now, against where it was as the second hand closed: its middle moved from
/// `from` to `to`, turned by `turn` about the vertical, and `spread` times as far between the hands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moved {
    pub from: Vec3,
    pub to: Vec3,
    pub turn: Quat,
    pub spread: f32,
}

impl Moved {
    /// Where something that stood at `model` stands now, `ratio` times its size then, about the middle of the hands'
    /// points: moved and turned with them, and every point of it that much further from their middle.
    pub fn model(&self, model: Mat4, ratio: f32) -> Mat4 {
        Mat4::from_translation(self.to)
            * Mat4::from_quat(self.turn)
            * Mat4::from_scale(Vec3::splat(ratio))
            * Mat4::from_translation(-self.from)
            * model
    }

    /// Where something whose frame was `frame`, unscaled, stands now, `ratio` times as large about the middle of the
    /// hands' points, its frame still unscaled: its middle moved and turned as [`Self::model`] moves it, and turned
    /// with the hands.
    pub fn frame(&self, frame: Mat4, ratio: f32) -> Mat4 {
        let (_, rotation, middle) = frame.to_scale_rotation_translation();
        let at = self.to + self.turn * ((middle - self.from) * ratio);
        Mat4::from_rotation_translation(self.turn * rotation, at)
    }
}

/// The way the line from `from` to `to` faces about the vertical, in radians, if it is long enough across to say.
fn facing(from: Vec3, to: Vec3) -> Option<f32> {
    let across = (to - from).with_y(0.0);
    (across.length() >= LEAST_ACROSS).then(|| across.x.atan2(across.z))
}

impl Hold {
    /// Both hands taking hold: `rays`, the first hand's and the second's, `along` each as far as the point each took.
    pub fn new(rays: [Ray; 2], along: [f32; 2]) -> Self {
        let yaw = facing(rays[0].origin, rays[1].origin).unwrap_or(0.0);
        Self {
            along,
            middle: middle(rays, along),
            yaw,
            spread: rays[0].origin.distance(rays[1].origin).max(LEAST_SPREAD),
            last: yaw,
        }
    }

    /// How far along each ray, the first hand's and the second's, each took hold.
    pub fn along(&self) -> [f32; 2] {
        self.along
    }

    /// Follow both hands' `rays`: where what is held stands now against where it stood as the second closed.
    pub fn follow(&mut self, rays: [Ray; 2]) -> Moved {
        if let Some(yaw) = facing(rays[0].origin, rays[1].origin) {
            self.last = yaw;
        }
        Moved {
            from: self.middle,
            to: middle(rays, self.along),
            turn: Quat::from_rotation_y(self.last - self.yaw),
            spread: rays[0].origin.distance(rays[1].origin).max(LEAST_SPREAD) / self.spread,
        }
    }
}

/// The middle of the points `along` each of `rays`.
fn middle(rays: [Ray; 2], along: [f32; 2]) -> Vec3 {
    (rays[0].at(along[0]) + rays[1].at(along[1])) / 2.0
}

/// `size` stepped to the nearest [`STEP`], within `bounds`.
pub fn stepped(size: f32, bounds: (f32, f32)) -> f32 {
    let size = if size.is_finite() { size } else { 1.0 };
    ((size / STEP).round() * STEP).clamp(bounds.0, bounds.1)
}

/// How far past halfway to the next step the hands must spread or close before the size shown steps, as a proportion of
/// a step, so that hands held still at the edge between two steps do not flicker between them.
const HYSTERESIS: f32 = 0.25;

/// The size to show as the hands ask for `size`, `shown` shown now: `shown` while `size` is within half a step and a
/// little of it, and otherwise `size` [`stepped`], within `bounds`. So a size taken hold of between steps, as the agent
/// may set one, is kept until the hands move it.
pub fn settled(shown: f32, size: f32, bounds: (f32, f32)) -> f32 {
    if size.is_finite() && (size - shown).abs() < STEP * (0.5 + HYSTERESIS) {
        shown
    } else {
        stepped(size, bounds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ray(origin: Vec3, toward: Vec3) -> Ray {
        Ray::new(origin, toward - origin).unwrap()
    }

    #[test]
    fn what_both_hands_hold_follows_their_middle_and_turns_with_them() {
        let target = Vec3::new(0.0, 1.5, -2.0);
        let left = Vec3::new(-0.2, 1.2, 0.0);
        let right = Vec3::new(0.2, 1.2, 0.0);
        let rays = [ray(left, target), ray(right, target)];
        let along = [left.distance(target), right.distance(target)];
        let mut hold = Hold::new(rays, along);
        // Held still, nothing moves.
        let moved = hold.follow(rays);
        assert!(moved.from.distance(target) < 1e-5 && moved.to.distance(target) < 1e-5);
        assert!(moved.turn.angle_between(Quat::IDENTITY) < 1e-5);
        assert!((moved.spread - 1.0).abs() < 1e-5);
        // Both hands carried half a metre to the right, it comes along, unturned.
        let shift = Vec3::X * 0.5;
        let moved = hold.follow([
            ray(left + shift, target + shift),
            ray(right + shift, target + shift),
        ]);
        assert!(moved.to.distance(target + shift) < 1e-4);
        let model = Mat4::from_translation(target);
        assert!(
            moved
                .model(model, 1.0)
                .w_axis
                .truncate()
                .distance(target + shift)
                < 1e-4
        );
        // The right hand drawn back and the left pushed forward, as a wheel is turned, it turns the same way.
        let turned = Quat::from_rotation_y(0.3);
        let middle = (left + right) / 2.0;
        let hands = [
            middle + turned * (left - middle),
            middle + turned * (right - middle),
        ];
        let moved = hold.follow([ray(hands[0], target), ray(hands[1], target)]);
        assert!(moved.turn.angle_between(turned) < 1e-4);
        // Held one above the other, the hands say no way to face, and it keeps the last.
        let moved = hold.follow([
            ray(middle - Vec3::Y * 0.2, target),
            ray(middle + Vec3::Y * 0.2, target),
        ]);
        assert!(moved.turn.angle_between(turned) < 1e-4);
    }

    #[test]
    fn spreading_the_hands_grows_what_they_hold_about_their_middle_whatever_its_distance() {
        for distance in [0.5, 3.0, 8.0] {
            let target = Vec3::new(0.0, 1.5, -distance);
            let left = Vec3::new(-0.15, 1.2, 0.0);
            let right = Vec3::new(0.15, 1.2, 0.0);
            let along = [left.distance(target), right.distance(target)];
            let mut hold = Hold::new([ray(left, target), ray(right, target)], along);
            // Half as far apart again, pointing where they did, it is half as large again, however far away it is.
            let wider = [left - Vec3::X * 0.075, right + Vec3::X * 0.075];
            let moved = hold.follow([ray(wider[0], target), ray(wider[1], target)]);
            assert!((moved.spread - 1.5).abs() < 1e-4, "{distance}");
            // A point of it a metre to the right of the middle is then 1.5 m from it.
            let model = Mat4::from_translation(target);
            let grown = moved.model(model, 1.5);
            let middle = moved.to;
            assert!(
                grown
                    .transform_point3(Vec3::X)
                    .distance(middle + (target - moved.from) * 1.5 + Vec3::X * 1.5)
                    < 1e-4
            );
            // An unscaled frame keeps unscaled, its middle where the scaling about the hands' middle puts it.
            let frame = Mat4::from_translation(target + Vec3::Y);
            let framed = moved.frame(frame, 1.5);
            assert!((framed.x_axis.truncate().length() - 1.0).abs() < 1e-5);
            assert!(
                framed
                    .w_axis
                    .truncate()
                    .distance(middle + (target + Vec3::Y - moved.from) * 1.5)
                    < 1e-4
            );
        }
        // Hands that closed nearly together are taken as a little apart.
        let target = Vec3::new(0.0, 1.5, -2.0);
        let near = [Vec3::new(-0.01, 1.2, 0.0), Vec3::new(0.01, 1.2, 0.0)];
        let mut hold = Hold::new([ray(near[0], target), ray(near[1], target)], [2.0, 2.0]);
        let apart = [Vec3::new(-0.1, 1.2, 0.0), Vec3::new(0.1, 1.2, 0.0)];
        let moved = hold.follow([ray(apart[0], target), ray(apart[1], target)]);
        assert!((moved.spread - 2.0).abs() < 1e-4);
    }

    #[test]
    fn sizes_step_by_five_percent_within_their_bounds() {
        assert_eq!(stepped(1.23, (0.5, 4.0)), 1.25);
        assert!((stepped(1.22, (0.5, 4.0)) - 1.2).abs() < 1e-6);
        assert_eq!(stepped(0.1, (0.5, 4.0)), 0.5);
        assert_eq!(stepped(9.0, (0.5, 4.0)), 4.0);
        assert_eq!(stepped(f32::NAN, (0.5, 4.0)), 1.0);
    }

    #[test]
    fn a_size_at_the_edge_between_two_steps_holds_still_until_the_hands_move_past_it() {
        let bounds = (0.5, 4.0);
        // Just past halfway to the next step, either way, it stays.
        assert_eq!(settled(1.2, 1.226, bounds), 1.2);
        assert_eq!(settled(1.25, 1.224, bounds), 1.25);
        // Well past it, it steps.
        assert_eq!(settled(1.2, 1.24, bounds), 1.25);
        assert!((settled(1.25, 1.21, bounds) - 1.2).abs() < 1e-6);
        // Taken hold of between steps, it is kept until moved.
        assert_eq!(settled(1.33, 1.33, bounds), 1.33);
        assert_eq!(settled(1.33, 1.4, bounds), 1.4);
        // Within its bounds, and nonsense steps to its own size.
        assert_eq!(settled(3.95, 9.0, bounds), 4.0);
        assert_eq!(settled(1.2, f32::NAN, bounds), 1.0);
    }
}
