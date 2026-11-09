// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where the Tomori wall stands while the agent is summoned: its travel spot, before whoever summoned it, as its home
//! stands before the eyes at home, so that it goes where the conversation went rather than staying behind. The wall's
//! home never changes; the travel spot is for the summons alone, and the wall goes back home with the agent. See
//! `design/projection.md`.
use tomori_environment::sightlines::{READING, Solid};
use tomori_protocol::{Pose, Wall};

/// How far ahead of the eyes of whoever summoned the agent the middle of the Tomori wall's row stands, horizontally, in
/// metres, first: as far as its home stands ahead of the eyes at home, so that its curved row curves about them.
pub const DISTANCE: f32 = super::ARC_RADIUS;

/// The other distances it may stand at, nearer and further, where the first is not clear, in metres.
const DISTANCES: [f32; 3] = [DISTANCE, 1.9, 2.8];

/// How far below those eyes the middle of its row stands, in metres: as far as its home stands below the eyes at home.
pub const DROP: f32 = tomori_environment::sightlines::EYES - 0.52;

/// How far its middle may turn aside from the way the agent stands from those eyes, in steps, either way, in radians:
/// straight towards the agent first, then 15°, 30° and 45° to either side.
const TURNS: [f32; 7] = [0.0, 15.0, -15.0, 30.0, -30.0, 45.0, -45.0];

/// How far about the agent's middle, across, up and down, the swarm keeps the wall away, in metres: its ring, as wide
/// as it grows and a little more, which faces the same way as the wall. It is sparse, and seen through, so it hides no
/// slot.
const SWARM: [f32; 3] = [1.05, 0.6, 1.05];

/// How thick another wall stands, in metres, about its slots, that the Tomori wall stays clear of.
const THICKNESS: f32 = 0.05;

/// Where the Tomori wall, laid out as `wall` is, stands while the agent stands at `agent`, summoned to someone whose eyes
/// are at `eyes`, or, unknown, where the agent faces from 1.2 m away, as a participant's trigger leaves it: before them,
/// [`DISTANCE`] ahead and [`DROP`] below their eyes, towards the agent, facing them, so that its curved row curves about
/// them as it does about the eyes at home. Where that is not clear, it turns aside from the agent, up to 45° either way,
/// and stands nearer or further, at the first spot within [`READING`] of their eyes at which every slot, at its corners
/// and middle, stands outside the environment's `solids`, the other walls shown, `others`, and the swarm, and nothing of
/// the solids and walls is between their eyes and any slot's middle. None where no spot is clear: it stays at home.
pub fn spot(
    eyes: Option<[f32; 3]>,
    agent: &Pose,
    wall: &Wall,
    others: &[Wall],
    solids: &[Solid],
) -> Option<Pose> {
    let eyes = eyes
        .filter(|eyes| eyes.iter().all(|value| value.is_finite()))
        .unwrap_or_else(|| {
            let yaw = agent.yaw();
            let [x, y, z] = agent.position;
            [x + 1.2 * yaw.sin(), y + 0.2, z + 1.2 * yaw.cos()]
        });
    let (x, z) = (agent.position[0] - eyes[0], agent.position[2] - eyes[2]);
    // Towards the agent, or else the way the agent faces them from.
    let towards = if x.hypot(z) > 1e-3 {
        x.atan2(z)
    } else {
        agent.yaw() + std::f32::consts::PI
    };
    let mut blocking: Vec<Solid> = solids.to_vec();
    blocking.extend(others.iter().flat_map(|other| {
        boxes(other).map(|(low, high)| {
            Solid::new(
                low.map(|value| value - THICKNESS),
                high.map(|value| value + THICKNESS),
            )
        })
    }));
    let swarm = Solid::new(
        std::array::from_fn(|axis| agent.position[axis] - SWARM[axis]),
        std::array::from_fn(|axis| agent.position[axis] + SWARM[axis]),
    );
    TURNS
        .iter()
        .flat_map(|turn| DISTANCES.iter().map(move |distance| (*turn, *distance)))
        .map(|(turn, distance)| {
            let bearing = towards + turn.to_radians();
            let middle = [
                eyes[0] + distance * bearing.sin(),
                eyes[1] - DROP,
                eyes[2] + distance * bearing.cos(),
            ];
            Pose::upright_facing(middle, [eyes[0], middle[1], eyes[2]])
        })
        .filter_map(|pose| super::valid(&pose))
        .find(|pose| clear(eyes, pose, wall, &blocking, &swarm))
}

/// Whether `wall`, standing at `pose`, is within reading distance of `eyes`, and every slot of it stands clear of
/// `solids` and `swarm`, seen from `eyes` past the solids.
fn clear(eyes: [f32; 3], pose: &Pose, wall: &Wall, solids: &[Solid], swarm: &Solid) -> bool {
    let distance = |a: [f32; 3], b: [f32; 3]| {
        (0..3)
            .map(|axis| (a[axis] - b[axis]).powi(2))
            .sum::<f32>()
            .sqrt()
    };
    if distance(eyes, pose.position) > READING {
        return false;
    }
    let mut placed = wall.clone();
    placed.pose = *pose;
    let inside =
        |point: [f32; 3]| swarm.contains(point) || solids.iter().any(|solid| solid.contains(point));
    let hidden = |point: [f32; 3]| solids.iter().any(|solid| solid.blocks(eyes, point));
    placed.slot_frames().iter().all(|slot| {
        let (middle, corners) = corners(slot, pose);
        !inside(middle) && !corners.into_iter().any(inside) && !hidden(middle)
    })
}

/// The middle and four corners of `slot`, on a wall standing at `wall`.
fn corners(slot: &tomori_protocol::wall::SlotFrame, wall: &Pose) -> ([f32; 3], [[f32; 3]; 4]) {
    let at = slot.pose(wall);
    let yaw = at.yaw();
    let right = [yaw.cos(), 0.0, -yaw.sin()];
    let [width, height] = slot.size;
    let corner = |across: f32, up: f32| {
        std::array::from_fn(|axis| {
            at.position[axis]
                + right[axis] * across * width / 2.0
                + if axis == 1 { up * height / 2.0 } else { 0.0 }
        })
    };
    (
        at.position,
        [
            corner(-1.0, -1.0),
            corner(1.0, -1.0),
            corner(-1.0, 1.0),
            corner(1.0, 1.0),
        ],
    )
}

/// The box about each slot of `wall`, where it stands, aligned with the space's axes.
fn boxes(wall: &Wall) -> impl Iterator<Item = ([f32; 3], [f32; 3])> + '_ {
    wall.slot_frames().into_iter().map(move |slot| {
        let (middle, corners) = corners(&slot, &wall.pose);
        corners
            .into_iter()
            .fold((middle, middle), |(low, high), corner| {
                (
                    std::array::from_fn(|axis| low[axis].min(corner[axis])),
                    std::array::from_fn(|axis| high[axis].max(corner[axis])),
                )
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomori_environment::sightlines::{Spot, readable};
    use tomori_protocol::{TOMORI_WALL, WallKind};

    fn tomori() -> Wall {
        super::super::Layout::TOMORI.apply(Wall::new(
            TOMORI_WALL,
            WallKind::Tomori,
            super::super::default_stage(),
        ))
    }

    /// The horizontal distance between `a` and `b`.
    fn across(a: [f32; 3], b: [f32; 3]) -> f32 {
        (a[0] - b[0]).hypot(a[2] - b[2])
    }

    #[test]
    fn it_stands_before_whoever_summoned_the_agent_as_its_home_stands_before_home() {
        // Someone 3 m to the right of home, facing +X, summons the agent 1.5 m ahead of them.
        let eyes = [3.0, 1.6, 0.0];
        let agent = Pose::at([4.5, 1.4, 0.0]).facing_towards(eyes);
        let spot = spot(Some(eyes), &agent, &tomori(), &[], &[]).expect("clear");
        // 2.3 m ahead of their eyes, towards the agent, 1.08 m below them, facing them, as its home is before home's.
        assert!((across(spot.position, eyes) - DISTANCE).abs() < 1e-4);
        assert!((spot.position[1] - (eyes[1] - DROP)).abs() < 1e-5);
        assert!(spot.position[0] > eyes[0] + 2.0);
        let facing = Spot::facing(spot.position, eyes).yaw;
        assert!((spot.yaw() - facing).abs() < 1e-4);
        // Its home, by the same rule from home, is where it is.
        let home = super::super::default_stage();
        let from_home = spot_from([0.0, 1.6, 0.0], [0.0, 1.4, -1.6]);
        assert!(across(from_home.position, home.position) < 1e-4);
        assert!((from_home.position[1] - home.position[1]).abs() < 1e-4);
        // So the rule every environment's home keeps holds of it: readable from where they stand.
        let standing = Spot::new([eyes[0], 0.0, eyes[2]], 0.0);
        assert_eq!(
            readable(&standing, &Spot::new(spot.position, spot.yaw()), &[]),
            Ok(())
        );
    }

    fn spot_from(eyes: [f32; 3], agent: [f32; 3]) -> Pose {
        spot(
            Some(eyes),
            &Pose::at(agent).facing_towards(eyes),
            &tomori(),
            &[],
            &[],
        )
        .expect("clear")
    }

    #[test]
    fn it_turns_aside_or_moves_for_scenery_other_walls_and_the_swarm_and_stays_home_where_nothing_is_clear()
     {
        let eyes = [0.0, 1.6, 0.0];
        let agent = Pose::at([0.0, 1.4, -3.0]).facing_towards(eyes);
        // A shed where it would stand: it turns aside, still readable and clear of it.
        let shed = Solid::new([-1.0, 0.0, -3.0], [1.0, 2.5, -2.0]);
        let spot = spot(Some(eyes), &agent, &tomori(), &[], &[shed]).expect("clear");
        let mut placed = tomori();
        placed.pose = spot;
        for slot in placed.slot_frames() {
            let (middle, corners) = corners(&slot, &spot);
            assert!(!shed.contains(middle) && !corners.iter().any(|corner| shed.contains(*corner)));
            assert!(!shed.blocks(eyes, middle));
        }
        assert!(across(spot.position, eyes) <= READING);
        // A wall someone added there: the same.
        let added = Wall::new(
            "wall-1",
            WallKind::Added,
            Pose::at([0.0, 0.6, -2.3]).facing_towards(eyes),
        );
        let beside = spot_with(eyes, &agent, std::slice::from_ref(&added), &[]);
        let mut placed = tomori();
        placed.pose = beside;
        assert!(boxes(&placed).all(|(low, high)| {
            boxes(&added).all(|(other_low, other_high)| {
                (0..3).any(|axis| {
                    high[axis] < other_low[axis] - THICKNESS
                        || low[axis] > other_high[axis] + THICKNESS
                })
            })
        }));
        // The swarm hovering low where it would stand: it stands elsewhere, outside the swarm's ring.
        let low = Pose::at([0.0, 0.7, -3.3]).facing_towards(eyes);
        let elsewhere = spot_with(eyes, &low, &[], &[]);
        assert!(across(elsewhere.position, [0.0, 0.0, -DISTANCE]) > 0.3);
        let ring = Solid::new(
            std::array::from_fn(|axis| low.position[axis] - SWARM[axis]),
            std::array::from_fn(|axis| low.position[axis] + SWARM[axis]),
        );
        let mut placed = tomori();
        placed.pose = elsewhere;
        for slot in placed.slot_frames() {
            let (middle, corners) = corners(&slot, &elsewhere);
            assert!(!ring.contains(middle) && !corners.iter().any(|corner| ring.contains(*corner)));
        }
        // Hovering just in front of it, at eye height, the swarm hides nothing: it stands where it would.
        let before = Pose::at([0.0, 1.6, -1.2]).facing_towards(eyes);
        assert!(
            (across(spot_with(eyes, &before, &[], &[]).position, eyes) - DISTANCE).abs() < 1e-4
        );
        // Walled in on every side: it stays home.
        let around = Solid::new([-5.0, -1.0, -5.0], [5.0, 3.0, -1.0]);
        assert_eq!(
            super::spot(Some(eyes), &agent, &tomori(), &[], &[around]),
            None
        );
    }

    fn spot_with(eyes: [f32; 3], agent: &Pose, others: &[Wall], solids: &[Solid]) -> Pose {
        spot(Some(eyes), agent, &tomori(), others, solids).expect("clear")
    }

    #[test]
    fn without_their_eyes_it_stands_before_whoever_the_agent_faces() {
        // The agent faces +X, towards someone 1.2 m away, as a trigger leaves it.
        let agent = Pose::at([1.0, 1.4, 0.0]).facing_towards([3.0, 1.4, 0.0]);
        let spot = spot(None, &agent, &tomori(), &[], &[]).expect("clear");
        let eyes = [2.2, 1.6, 0.0];
        assert!((across(spot.position, eyes) - DISTANCE).abs() < 1e-3);
        // Towards the agent from them, facing them.
        assert!(spot.position[0] < eyes[0] - 2.0);
        // Not finite, the same.
        let nonsense = super::spot(Some([f32::NAN, 0.0, 0.0]), &agent, &tomori(), &[], &[]);
        assert_eq!(nonsense, Some(spot));
        // Straight below the agent, the way it faces them from: before them, beneath it.
        let above = Pose::at([1.0, 2.0, 0.0]).facing_towards([3.0, 2.0, 0.0]);
        let below = super::spot(Some([1.0, 1.6, 0.0]), &above, &tomori(), &[], &[]).expect("clear");
        assert!(
            (below.position[0] - (1.0 - DISTANCE)).abs() < 1e-3,
            "{below:?}"
        );
    }

    #[test]
    fn a_wall_the_space_reshaped_travels_as_it_is() {
        // Two columns, flat: it still stands clear, and its slots are those checked.
        let wall = super::super::Layout {
            grid: tomori_protocol::Grid::new(2, 1).unwrap(),
            radius: None,
            ..super::super::Layout::TOMORI
        }
        .apply(Wall::new(
            TOMORI_WALL,
            WallKind::Tomori,
            super::super::default_stage(),
        ));
        let eyes = [0.0, 1.6, 0.0];
        let agent = Pose::at([0.0, 1.4, -2.0]).facing_towards(eyes);
        let spot = spot(Some(eyes), &agent, &wall, &[], &[]).expect("clear");
        assert!((across(spot.position, eyes) - DISTANCE).abs() < 1e-4);
        // Too far for any spot to be read: beyond the reach of walls, it stays home.
        let far = Pose::at([30.0, 1.4, 0.0]);
        assert_eq!(
            super::spot(Some([30.0, 1.6, 2.0]), &far, &wall, &[], &[]),
            None
        );
    }
}
