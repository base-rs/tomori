// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! How the agent and the Tomori wall stand one above the other at the agent's home, and where the agent stands when it
//! goes to someone, for the server, which stands them, and every client and environment, which test and draw them, to
//! agree on: the swarm above, the middle of the wall's row of slots [`BENEATH`] directly beneath the swarm's middle,
//! facing the way the agent faces, with a clear gap between the swarm's lowest reach and the top of the row, and both
//! clear of the floor. Seen from whoever the agent faces, the wall stands beneath the swarm, never behind it:
//! [`seen_beneath`]. Its name, and that of [`summoned`], are from when the wall and the workers travelled with the agent
//! as it was summoned; nothing travels with it now, and the wall stays home. See `design/environments.md` and
//! `design/projection.md`.
//!
//! ```
//! use tomori_protocol::{Pose, cluster::FLOOR, entourage};
//!
//! // The default layout: the agent 1.4 m up, 2.3 m ahead of home, facing home, and its wall beneath it, at 0.52 m.
//! let agent = Pose::at([0.0, 1.4, -2.3]).facing_towards([0.0, 1.4, 0.0]);
//! let wall = entourage::wall_beneath(&agent, FLOOR);
//! assert!((wall.position[1] - 0.52).abs() < 1e-5);
//! assert_eq!([wall.position[0], wall.position[2]], [0.0, -2.3]);
//! // Summoned by someone seated, their eyes 1 m up, the agent stands high enough for its wall beneath it.
//! assert_eq!(entourage::summoned_height(1.0, FLOOR), entourage::LOWEST_AGENT);
//! ```
use crate::projection::layout::{ARC_SCALE, SLOT_SIZE};
use crate::{Pose, Wall};

/// How far the swarm reaches about its middle, at most, in any of its shapes and as it drifts, in metres: across, to
/// either side of the way it faces; up and down; and towards and away from whoever it faces.
pub const SWARM: [f32; 3] = [1.35, 0.52, 0.42];

/// Half the height of the Tomori wall's row, in metres: a slot at the curved wall's scale, as it stays (see
/// `design/projection.md`).
pub const ROW_HALF: f32 = SLOT_SIZE.1 * ARC_SCALE / 2.0;

/// How far below the middle of the swarm the middle of the Tomori wall's row stands, in metres, beneath it: as the
/// default home's wall stands below the default agent, 0.52 m beneath 1.4 m.
pub const BENEATH: f32 = 0.88;

/// The clear gap between the swarm's lowest reach and the top of the Tomori wall's row beneath it, in metres.
pub const GAP: f32 = BENEATH - SWARM[1] - ROW_HALF;

/// The least height of the bottom edge of the Tomori wall's row above the floor, in metres.
pub const CLEARANCE: f32 = 0.3;

/// The lowest the middle of the Tomori wall's row stands above the floor, in metres: as the default home's, so that its
/// row's bottom edge stands about 0.39 m above the floor, and its label, below the row, about 0.34 m.
pub const LOWEST_WALL: f32 = 0.52;

/// The lowest the middle of a summoned agent stands above the floor, in metres: high enough for its wall beneath it, as
/// the default agent's home, 1.4 m.
pub const LOWEST_AGENT: f32 = LOWEST_WALL + BENEATH;

/// How far above the eyes of whoever summoned it the middle of a summoned agent stands, in metres: a little above, so
/// that its wall beneath stands at a comfortable reading height below those eyes, about 0.58 m, and is seen beneath it.
pub const RISE: f32 = 0.3;

/// How far from the head of whoever summoned it, horizontally, a summoned agent stands at least, in metres: as far as
/// the agent's default home stands from home, so that its entourage stands about them as it stands about home by
/// default: its workers on the arc through it, which needs that much room for their columns and signs to stand clear of
/// one another and of the swarm in front of them, and its wall beneath it, six slots across, read as from home and seen
/// below the swarm.
pub const SUMMONED: f32 = 2.3;

/// How far below the swarm's lowest reach, as seen from whoever it faces, the top of every slot of the wall beneath it
/// stands at least, in radians: 1°.
pub const SEEN_GAP: f32 = 0.017_453_292;

/// The home of the Tomori wall beneath the agent standing at `agent`, on a floor at `floor`: the middle of its row
/// [`BENEATH`] directly beneath the swarm's middle, upright and facing the way the agent faces, but never lower than
/// [`LOWEST_WALL`] above the floor.
pub fn wall_beneath(agent: &Pose, floor: f32) -> Pose {
    let [x, y, z] = agent.position;
    Pose::upright([x, (y - BENEATH).max(floor + LOWEST_WALL), z], agent.yaw())
}

/// The height the middle of a summoned agent stands at, for whoever summoned it with their eyes at `eyes`, on a floor at
/// `floor`: [`RISE`] above their eyes, but never lower than [`LOWEST_AGENT`] above the floor, however they sit or stand.
pub fn summoned_height(eyes: f32, floor: f32) -> f32 {
    (eyes + RISE).max(floor + LOWEST_AGENT)
}

/// Where a summoned agent stands, drawn towards `at` by someone whose eyes are at `eyes`, on a floor at `floor`: the way
/// `at` lies from them, at least [`SUMMONED`] from them horizontally, straight ahead of them where `at` is above or
/// below them, at [`summoned_height`], facing them.
pub fn summoned(at: [f32; 3], eyes: [f32; 3], floor: f32) -> Pose {
    let (x, z) = (at[0] - eyes[0], at[2] - eyes[2]);
    let across = x.hypot(z);
    let (dx, dz) = if across > 1e-4 {
        (x / across, z / across)
    } else {
        (0.0, -1.0)
    };
    let away = across.max(SUMMONED);
    let spot = [
        eyes[0] + dx * away,
        summoned_height(eyes[1], floor),
        eyes[2] + dz * away,
    ];
    Pose::at(spot).facing_towards(eyes)
}

/// Whether `wall` stands beneath the swarm of the agent at `agent` as seen from `eyes`: the top corners of every one of
/// its slots, as it is laid out and tilted, are seen at least [`SEEN_GAP`] lower than any part of the swarm, as far as it
/// reaches ([`SWARM`]), so that nothing of the swarm stands in front of or behind the wall's content. Not where the eyes
/// stand within the swarm's reach about its middle.
pub fn seen_beneath(eyes: [f32; 3], agent: &Pose, wall: &Wall) -> bool {
    match swarm_lowest(eyes, agent) {
        None => false,
        Some(lowest) => slot_corners(wall)
            .iter()
            .all(|corner| elevation(eyes, *corner) <= lowest - SEEN_GAP),
    }
}

/// The lowest any part of the swarm of the agent at `agent` is seen from `eyes`, as an elevation above the level of the
/// eyes, in radians: that of the nearest point, horizontally, of the bottom of the box it reaches within, which faces
/// the way the agent faces. None where the eyes stand within that box's footprint.
pub fn swarm_lowest(eyes: [f32; 3], agent: &Pose) -> Option<f32> {
    let yaw = agent.yaw();
    let (sin, cos) = yaw.sin_cos();
    let (dx, dz) = (eyes[0] - agent.position[0], eyes[2] - agent.position[2]);
    // The eyes in the agent's own frame, about the vertical: its +X across, its +Z the way it faces.
    let (across, ahead) = (dx * cos - dz * sin, dx * sin + dz * cos);
    let near_across = across.clamp(-SWARM[0], SWARM[0]);
    let near_ahead = ahead.clamp(-SWARM[2], SWARM[2]);
    let level = (across - near_across).hypot(ahead - near_ahead);
    if level < 1e-3 {
        return None;
    }
    let bottom = agent.position[1] - SWARM[1];
    // Where the bottom is above the eyes, its furthest corner is seen lowest.
    let level = if bottom >= eyes[1] {
        let far_across = SWARM[0] * if across > 0.0 { -1.0 } else { 1.0 };
        let far_ahead = SWARM[2] * if ahead > 0.0 { -1.0 } else { 1.0 };
        (across - far_across).hypot(ahead - far_ahead)
    } else {
        level
    };
    Some((bottom - eyes[1]).atan2(level))
}

/// The elevation of `point` above the level of `eyes`, as seen from them, in radians.
pub fn elevation(eyes: [f32; 3], point: [f32; 3]) -> f32 {
    (point[1] - eyes[1]).atan2((point[0] - eyes[0]).hypot(point[2] - eyes[2]))
}

/// The top two corners of each of `wall`'s slots where it stands, as it is laid out and tilted, in the space.
pub fn slot_corners(wall: &Wall) -> Vec<[f32; 3]> {
    let frame = wall.frame();
    wall.slot_frames()
        .iter()
        .flat_map(|slot| {
            let [_, _, left, right] = slot.corners(&frame);
            [left, right]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::FLOOR;
    use crate::wall::{ARC_GRID, ARC_SCALE};
    use crate::{TOMORI_WALL, WallKind, WallShape};

    /// The Tomori wall at `pose`, as it stands unless a space changes it: six slots curved along an arc of 2.3 m.
    fn tomori(pose: Pose) -> Wall {
        Wall::new(TOMORI_WALL, WallKind::Tomori, pose)
            .with_shape(WallShape::Arc { radius: 2.3 })
            .with_grid(ARC_GRID)
            .with_scale(ARC_SCALE)
    }

    /// The wall beneath the agent at `agent`, facing `eyes`, as it stands while the agent is summoned.
    fn beneath_facing(agent: &Pose, eyes: [f32; 3]) -> Wall {
        let home = wall_beneath(agent, FLOOR);
        tomori(Pose::upright_facing(
            home.position,
            [eyes[0], home.position[1], eyes[2]],
        ))
    }

    #[test]
    fn the_wall_stands_beneath_the_swarm_with_a_gap_and_clear_of_the_floor() {
        const { assert!(GAP > 0.2) };
        const { assert!(LOWEST_WALL - ROW_HALF >= CLEARANCE + 0.05) };
        assert!((LOWEST_AGENT - 1.4).abs() < 1e-6);
        // The default layout, from home.
        let eyes = [0.0, 1.6, 0.0];
        let agent = Pose::at([0.0, 1.4, -2.3]).facing_towards(eyes);
        let home = wall_beneath(&agent, FLOOR);
        assert_eq!(home.orientation, agent.orientation);
        assert!(seen_beneath(eyes, &agent, &tomori(home)));
        // An agent too low for its wall beneath it: the wall stays clear of the floor.
        let low = Pose::at([0.0, 0.9, -2.3]).facing_towards(eyes);
        assert_eq!(wall_beneath(&low, FLOOR).position[1], LOWEST_WALL);
        assert_eq!(wall_beneath(&low, 2.0).position[1], 2.0 + LOWEST_WALL);
    }

    #[test]
    fn a_summoned_agent_stands_above_the_eyes_and_its_wall_beneath_it_at_any_height_of_eyes() {
        // From someone seated low, about 1 m, to someone tall, about 1.8 m, standing.
        for tenth in 9..=19 {
            let eyes = [0.4, tenth as f32 / 10.0, 2.0];
            for (towards, distance) in [([0.0, -1.0], 1.2), ([0.6, 0.8], 1.5), ([-1.0, 0.0], 2.5)] {
                let at = [
                    eyes[0] + towards[0] * distance,
                    0.3,
                    eyes[2] + towards[1] * distance,
                ];
                let agent = summoned(at, eyes, FLOOR);
                let level = (agent.position[0] - eyes[0]).hypot(agent.position[2] - eyes[2]);
                assert!(level >= SUMMONED - 1e-4 && (level - distance.max(SUMMONED)).abs() < 1e-4);
                assert!(agent.position[1] >= eyes[1] + RISE - 1e-5);
                assert!(agent.position[1] >= LOWEST_AGENT - 1e-5);
                let wall = beneath_facing(&agent, eyes);
                // Clear of the floor, by its row's bottom edge, and beneath the swarm with the gap.
                assert!(wall.pose.position[1] - ROW_HALF >= CLEARANCE, "{eyes:?}");
                assert!(
                    agent.position[1] - SWARM[1] - (wall.pose.position[1] + ROW_HALF) >= GAP - 1e-5
                );
                // Seen beneath the swarm, never behind it, and below their eyes, as a wall is read.
                assert!(seen_beneath(eyes, &agent, &wall), "{eyes:?} {at:?}");
                assert!(wall.pose.position[1] < eyes[1]);
            }
        }
        // Straight above or below them, it stands straight ahead.
        let ahead = summoned([0.0, 3.0, 0.0], [0.0, 1.6, 0.0], FLOOR);
        assert_eq!(ahead.position, [0.0, 1.6 + RISE, -SUMMONED]);
    }

    #[test]
    fn a_wall_behind_the_swarm_or_level_with_it_is_not_seen_beneath_it() {
        let eyes = [0.0, 1.6, 0.0];
        let agent = Pose::at([0.0, 1.4, -1.5]).facing_towards(eyes);
        // Behind it, 2.3 m before the eyes and 1.08 m below them, as the travel spot stood before.
        let behind = tomori(Pose::upright_facing([0.0, 0.52, -2.3], [0.0, 0.52, 0.0]));
        assert!(!seen_beneath(eyes, &agent, &behind));
        // Level with it.
        let level = tomori(Pose::upright_facing([0.0, 1.4, -1.5], [0.0, 1.4, 0.0]));
        assert!(!seen_beneath(eyes, &agent, &level));
        // Eyes within the swarm see nothing beneath it.
        assert_eq!(swarm_lowest([0.0, 1.4, -1.6], &agent), None);
        assert!(!seen_beneath([0.0, 1.4, -1.6], &agent, &behind));
        // A swarm above the eyes is seen lowest at its furthest corner.
        let high = Pose::at([0.0, 2.5, -2.0]).facing_towards(eyes);
        let lowest = swarm_lowest(eyes, &high).unwrap();
        let far = (2.5 - SWARM[1] - 1.6f32).atan2(SWARM[0].hypot(2.0 + SWARM[2]));
        assert!((lowest - far).abs() < 1e-5);
        // From behind it and off to one side, the same: its furthest corner, on its other side.
        let away = Pose::at([0.0, 2.5, -2.0]).facing_towards([0.0, 2.5, -4.0]);
        let lowest = swarm_lowest([-1.0, 1.6, 0.0], &away).unwrap();
        let far = (2.5 - SWARM[1] - 1.6f32).atan2((1.0 + SWARM[0]).hypot(2.0 + SWARM[2]));
        assert!((lowest - far).abs() < 1e-5, "{lowest} {far}");
    }
}
