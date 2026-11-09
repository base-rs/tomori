// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The Tomori wall travelling with the agent: when the server moves it within the same space and environment, as when
//! a participant summons the agent and its wall goes with it, or sends it home, it folds up where it stood, its slots
//! and their panels collapsing into a small bundle, travels to where it now stands, and unfolds there. Everyone sees
//! the same, since each client follows the same projection; a wall that appears elsewhere, as a space is joined or
//! changes environment, stands there at once. See `design/projection.md`.
use std::time::Instant;

use glam::{Mat4, Quat, Vec3};
use tomori_protocol::{Pose, Projection, TOMORI_WALL};

/// How long the wall takes to fold up, and to unfold, in seconds.
pub const FOLD: f32 = 0.3;

/// How fast the folded wall travels, in metres a second, and the least and most time it takes, in seconds: about as
/// fast as the agent glides, a little faster across a room so as not to keep anyone waiting.
pub const SPEED: f32 = 1.5;
pub const TRAVEL: (f32, f32) = (0.4, 2.5);

/// How high above the straight line between where it was and where it goes the folded wall rises, at most, in metres,
/// so that it is seen to be carried rather than slid.
const HOP: f32 = 0.25;

/// How small each slot is drawn, folded, as a proportion of its size, and how far apart its slots are stacked, in
/// metres, front to back.
const BUNDLE_SCALE: f32 = 0.45;
const BUNDLE_GAP: f32 = 0.02;

/// A journey of the wall from one spot to another.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Journey {
    from: Pose,
    to: Pose,
    since: Instant,
    /// How far folded it already was as it set off, from 0, unfolded, to 1, folded, as for a journey that turns back.
    folded: f32,
}

impl Journey {
    /// How long it takes to travel, folded.
    fn travel(&self) -> f32 {
        let distance =
            Vec3::from_array(self.from.position).distance(Vec3::from_array(self.to.position));
        (distance / SPEED).clamp(TRAVEL.0, TRAVEL.1)
    }

    /// Where it stands and how far folded it is `elapsed` seconds in, or none once it has unfolded. Sent back to where
    /// it is before it has left, it unfolds there.
    fn at(&self, elapsed: f32) -> Option<(Pose, f32)> {
        if Vec3::from_array(self.from.position).distance(Vec3::from_array(self.to.position)) < 1e-3
        {
            let folded = self.folded - elapsed / FOLD;
            return (folded > 0.0).then_some((self.to, folded));
        }
        let folding = (1.0 - self.folded) * FOLD;
        if elapsed < folding {
            return Some((self.from, self.folded + elapsed / FOLD));
        }
        let elapsed = elapsed - folding;
        let travel = self.travel();
        if elapsed < travel {
            let t = smooth(elapsed / travel);
            let from = Vec3::from_array(self.from.position);
            let to = Vec3::from_array(self.to.position);
            let hop = HOP.min(from.distance(to) * 0.2) * (std::f32::consts::PI * t).sin();
            let position = from.lerp(to, t) + Vec3::Y * hop;
            let turn = Quat::from_array(self.from.orientation)
                .normalize()
                .slerp(Quat::from_array(self.to.orientation).normalize(), t);
            return Some((
                Pose {
                    position: position.to_array(),
                    orientation: turn.to_array(),
                },
                1.0,
            ));
        }
        let elapsed = elapsed - travel;
        (elapsed < FOLD).then(|| (self.to, 1.0 - elapsed / FOLD))
    }
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The Tomori wall as it was last projected, and its journey while it travels.
#[derive(Clone, Debug, Default)]
pub struct Travel {
    /// The space and environment it was last projected in, and where.
    seen: Option<(String, String, Pose)>,
    journey: Option<Journey>,
}

impl Travel {
    /// Follow `projection` at `now`: where the Tomori wall is drawn and how far folded it is, from 0 to 1, while it
    /// travels, or none while it stands where the projection says. A move within the same space and environment sets
    /// it travelling, from where it is drawn and as folded as it is, so that a summons that follows another turns it
    /// round smoothly; elsewhere it stands where it is at once.
    pub fn follow(&mut self, projection: &Projection, now: Instant) -> Option<(Pose, f32)> {
        let Some(wall) = projection.walls.iter().find(|wall| wall.id == TOMORI_WALL) else {
            *self = Self::default();
            return None;
        };
        let here = |(space, environment, _): &(String, String, Pose)| {
            *space == projection.space && *environment == projection.environment
        };
        match &self.seen {
            Some(seen) if here(seen) => {
                if seen.2 != wall.pose {
                    let (from, folded) = self.drawn(now).unwrap_or((seen.2, 0.0));
                    self.journey = Some(Journey {
                        from,
                        to: wall.pose,
                        since: now,
                        folded,
                    });
                }
            }
            _ => self.journey = None,
        }
        self.seen = Some((
            projection.space.clone(),
            projection.environment.clone(),
            wall.pose,
        ));
        let drawn = self.drawn(now);
        if drawn.is_none() {
            self.journey = None;
        }
        drawn
    }

    /// Where its journey has it at `now`, if it is travelling.
    fn drawn(&self, now: Instant) -> Option<(Pose, f32)> {
        let journey = self.journey?;
        journey.at(now.saturating_duration_since(journey.since).as_secs_f32())
    }

    /// Whether it is travelling.
    pub fn travelling(&self) -> bool {
        self.journey.is_some()
    }
}

/// How large a flat wall's board is drawn, folded by `fold`, as a proportion of its size.
pub fn board_scale(fold: f32) -> f32 {
    1.0 - (1.0 - BUNDLE_SCALE) * smooth(fold)
}

/// `slots`, the frames of a wall's slots standing at `frame`, folded by `fold`, from 0, as they stand, to 1, a small
/// bundle at its middle: each drawn smaller and drawn in towards the middle, turned to face as the wall faces, and
/// stacked a little apart, front to back.
pub fn fold(slots: &[Mat4], frame: Mat4, fold: f32) -> Vec<Mat4> {
    let fold = smooth(fold);
    if fold <= 0.0 {
        return slots.to_vec();
    }
    let (_, facing, middle) = frame.to_scale_rotation_translation();
    let normal = facing * Vec3::Z;
    let count = slots.len();
    slots
        .iter()
        .enumerate()
        .map(|(index, slot)| {
            let (scale, rotation, position) = slot.to_scale_rotation_translation();
            let stacked =
                middle + normal * ((index as f32 - (count as f32 - 1.0) / 2.0) * BUNDLE_GAP);
            Mat4::from_scale_rotation_translation(
                scale * (1.0 - (1.0 - BUNDLE_SCALE) * fold),
                rotation.slerp(facing, fold),
                position.lerp(stacked, fold),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tomori_protocol::{Wall, WallKind};

    fn projection(space: &str, environment: &str, pose: Pose) -> Projection {
        Projection::new(space, environment).with_walls(vec![Wall::new(
            TOMORI_WALL,
            WallKind::Tomori,
            pose,
        )])
    }

    fn home() -> Pose {
        Pose::at([0.0, 0.52, -2.3]).facing_towards([0.0, 0.52, 0.0])
    }

    fn away() -> Pose {
        Pose::at([3.0, 0.52, 0.0]).facing_towards([0.0, 0.52, 0.0])
    }

    #[test]
    fn moved_it_folds_travels_and_unfolds_where_it_now_stands() {
        let now = Instant::now();
        let at = |seconds: f32| now + Duration::from_secs_f32(seconds);
        let mut travel = Travel::default();
        // First seen, it stands where it is.
        assert_eq!(
            travel.follow(&projection("space", "sky", home()), now),
            None
        );
        assert!(!travel.travelling());
        // Moved, it folds where it stood.
        let moved = projection("space", "sky", away());
        let (pose, folded) = travel.follow(&moved, now).expect("travelling");
        assert_eq!((pose, folded), (home(), 0.0));
        let (pose, folded) = travel.follow(&moved, at(FOLD / 2.0)).unwrap();
        assert_eq!(pose, home());
        assert!((folded - 0.5).abs() < 1e-3);
        // Folded, it travels, rising a little on the way, then unfolds where it now stands.
        let distance =
            Vec3::from_array(home().position).distance(Vec3::from_array(away().position));
        let journey = (distance / SPEED).clamp(TRAVEL.0, TRAVEL.1);
        let (pose, folded) = travel.follow(&moved, at(FOLD + journey / 2.0)).unwrap();
        assert_eq!(folded, 1.0);
        let middle = (Vec3::from_array(home().position) + Vec3::from_array(away().position)) / 2.0;
        let drawn = Vec3::from_array(pose.position);
        assert!(
            (drawn.x - middle.x).hypot(drawn.z - middle.z) < 1e-3,
            "{drawn}"
        );
        assert!(drawn.y > middle.y + 0.1, "{drawn}");
        let (pose, folded) = travel
            .follow(&moved, at(FOLD + journey + FOLD / 2.0))
            .unwrap();
        assert_eq!(pose, away());
        assert!((folded - 0.5).abs() < 1e-3);
        // Unfolded, it stands there.
        assert_eq!(travel.follow(&moved, at(FOLD * 2.0 + journey + 0.01)), None);
        assert!(!travel.travelling());
    }

    #[test]
    fn sent_back_on_the_way_it_turns_round_from_where_it_is_and_elsewhere_it_stands_at_once() {
        let now = Instant::now();
        let at = |seconds: f32| now + Duration::from_secs_f32(seconds);
        let mut travel = Travel::default();
        travel.follow(&projection("space", "sky", home()), now);
        travel.follow(&projection("space", "sky", away()), now);
        // Halfway through folding, it is sent home: it unfolds where it is.
        let (pose, folded) = travel
            .follow(&projection("space", "sky", home()), at(FOLD / 2.0))
            .unwrap();
        assert_eq!(pose, home());
        assert!((folded - 0.5).abs() < 1e-3);
        // Mid-journey, sent elsewhere, it sets off from where it is drawn, still folded.
        travel.follow(&projection("space", "sky", away()), at(1.0));
        let (drawn, _) = travel
            .follow(&projection("space", "sky", away()), at(1.0 + FOLD + 0.2))
            .unwrap();
        let elsewhere = Pose::at([-3.0, 0.52, 0.0]).facing_towards([0.0, 0.52, 0.0]);
        let (from, folded) = travel
            .follow(&projection("space", "sky", elsewhere), at(1.0 + FOLD + 0.2))
            .unwrap();
        assert_eq!((from.position, folded), (drawn.position, 1.0));
        assert!(
            Quat::from_array(from.orientation).angle_between(Quat::from_array(drawn.orientation))
                < 1e-3
        );
        // Another environment, or another space, and it stands where it is, at once.
        assert_eq!(
            travel.follow(&projection("space", "farmyard", home()), at(2.0)),
            None
        );
        assert_eq!(
            travel.follow(&projection("other", "farmyard", away()), at(2.1)),
            None
        );
        // With no Tomori wall, it forgets it.
        assert_eq!(
            travel.follow(&Projection::new("other", "farmyard"), at(2.2)),
            None
        );
        assert_eq!(
            travel.follow(&projection("other", "farmyard", home()), at(2.3)),
            None
        );
    }

    #[test]
    fn folded_its_slots_are_a_small_bundle_at_its_middle_facing_as_it_faces() {
        let frame =
            Mat4::from_rotation_translation(Quat::from_rotation_y(0.4), Vec3::new(1.0, 0.5, -2.0));
        let slots: Vec<Mat4> = (0..6)
            .map(|index| {
                frame
                    * Mat4::from_translation(Vec3::new(index as f32 * 0.4 - 1.0, 0.0, 0.1))
                    * Mat4::from_rotation_y(0.2 - index as f32 * 0.08)
                    * Mat4::from_scale(Vec3::splat(0.7))
            })
            .collect();
        // Unfolded, as they stand.
        assert_eq!(fold(&slots, frame, 0.0), slots);
        let folded = fold(&slots, frame, 1.0);
        let middle = frame.w_axis.truncate();
        for slot in &folded {
            let (scale, rotation, position) = slot.to_scale_rotation_translation();
            assert!((scale.x - 0.7 * BUNDLE_SCALE).abs() < 1e-4);
            assert!(rotation.angle_between(Quat::from_rotation_y(0.4)) < 1e-3);
            assert!(position.distance(middle) <= BUNDLE_GAP * 3.0);
        }
        // Halfway, each is between.
        let half = fold(&slots, frame, 0.5);
        for (slot, before) in half.iter().zip(&slots) {
            let position = slot.w_axis.truncate();
            assert!(position.distance(middle) < before.w_axis.truncate().distance(middle));
        }
    }
}
