// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where the Tomori wall stands while the agent is summoned: its travel spot, beneath the summoned agent, before
//! whoever summoned it, as its home stands beneath the agent's home, so that it goes where the conversation went rather
//! than staying behind. The wall's home never changes; the travel spot is for the summons alone, and the wall goes back
//! home with the agent. See `design/projection.md`.
use tomori_environment::sightlines::{READING, Solid};
use tomori_protocol::{Pose, Wall, entourage};

/// How much nearer to whoever summoned the agent, or further, than the agent the middle of the Tomori wall's row may
/// stand, horizontally, in metres, where beneath the agent is not clear: the agent's own distance first.
const ALONG: [f32; 3] = [0.0, -0.3, 0.4];

/// The nearest to the eyes of whoever summoned the agent the middle of its row stands, horizontally, in metres.
const NEAREST: f32 = 1.0;

/// How far its middle may turn aside from the way the agent stands from those eyes, in steps, either way, in radians:
/// straight beneath the agent first, then 15°, 30° and 45° to either side.
const TURNS: [f32; 7] = [0.0, 15.0, -15.0, 30.0, -30.0, 45.0, -45.0];

/// How far about the agent's middle, across, up and down, the swarm keeps the wall away, in metres: its ring, as wide
/// as it grows and a little more, which faces the same way as the wall. It is sparse, and seen through, so it hides no
/// slot; the wall beneath it stands clear of it.
const SWARM: [f32; 3] = [1.05, 0.6, 1.05];

/// How thick another wall stands, in metres, about its slots, that the Tomori wall stays clear of.
const THICKNESS: f32 = 0.05;

/// Where the Tomori wall, laid out as `wall` is, stands while the agent stands at `agent`, summoned to someone whose eyes
/// are at `eyes`, or, unknown, where the agent faces from `entourage::SUMMONED` away, [`entourage::RISE`] below it, as a
/// participant's trigger leaves it: beneath the agent, the middle of its row [`entourage::BENEATH`] directly below the
/// swarm's middle, never lower than [`entourage::LOWEST_WALL`] above the floor, facing them, as its home stands beneath
/// the agent's home. Where that is not clear, it stands a little nearer them or further, then turns aside, up to 45°
/// either way, at the same height, at the first spot within `tomori_environment::sightlines::READING` of their eyes at
/// which no slot, along its edges or across its diagonals, passes through the environment's `solids`, the other walls
/// shown, `others`, or the swarm, nothing of the solids and walls is between their eyes and any slot's middle, and
/// every slot is seen beneath the swarm, never behind it ([`entourage::seen_beneath`]). None where no spot is clear: it
/// stays at home.
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
            [
                x + entourage::SUMMONED * yaw.sin(),
                y - entourage::RISE,
                z + entourage::SUMMONED * yaw.cos(),
            ]
        });
    let (x, z) = (agent.position[0] - eyes[0], agent.position[2] - eyes[2]);
    let away = x.hypot(z);
    // Towards the agent, or else the way the agent faces them from.
    let towards = if away > 1e-3 {
        x.atan2(z)
    } else {
        agent.yaw() + std::f32::consts::PI
    };
    let height = entourage::wall_beneath(agent, entourage::FLOOR).position[1];
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
        .flat_map(|turn| ALONG.iter().map(move |along| (*turn, *along)))
        .map(|(turn, along)| {
            let bearing = towards + turn.to_radians();
            let distance = (away + along).max(NEAREST);
            let middle = [
                eyes[0] + distance * bearing.sin(),
                height,
                eyes[2] + distance * bearing.cos(),
            ];
            Pose::upright_facing(middle, [eyes[0], middle[1], eyes[2]])
        })
        .filter_map(|pose| super::valid(&pose))
        .find(|pose| clear(eyes, agent, pose, wall, &blocking, &swarm))
}

/// Whether every slot of `wall`, standing at `pose`, stands clear of `solids` and `swarm`, seen from `eyes` past the
/// solids: no edge of a slot, nor either of its diagonals, passes through any of them, so that one standing across a
/// slot, between its corners, as another wall edge on does, counts too; within reading distance of the eyes; its row's
/// bottom edge at least [`entourage::CLEARANCE`] above the floor; and seen beneath the swarm of the agent at `agent`.
fn clear(
    eyes: [f32; 3],
    agent: &Pose,
    pose: &Pose,
    wall: &Wall,
    solids: &[Solid],
    swarm: &Solid,
) -> bool {
    let mut placed = wall.clone();
    placed.pose = *pose;
    let reach = (0..3)
        .map(|axis| (pose.position[axis] - eyes[axis]).powi(2))
        .sum::<f32>()
        .sqrt();
    if reach > READING
        || pose.position[1] - entourage::ROW_HALF < entourage::FLOOR + entourage::CLEARANCE
        || !entourage::seen_beneath(eyes, agent, &placed)
    {
        return false;
    }
    let crosses = |(from, to): ([f32; 3], [f32; 3])| {
        swarm.blocks(from, to) || solids.iter().any(|solid| solid.blocks(from, to))
    };
    let hidden = |point: [f32; 3]| solids.iter().any(|solid| solid.blocks(eyes, point));
    // Its slots stand in its frame, tilted as the space tilted it.
    let frame = placed.frame();
    placed.slot_frames().iter().all(|slot| {
        let (middle, [a, b, c, d]) = corners(slot, &frame);
        // Its edges, about it, and its diagonals, across it.
        let lines = [(a, b), (b, d), (d, c), (c, a), (a, d), (b, c)];
        !lines.into_iter().any(crosses) && !hidden(middle)
    })
}

/// `vector` turned by the quaternion `rotation`.
pub(super) fn rotate([x, y, z, w]: [f32; 4], vector: [f32; 3]) -> [f32; 3] {
    let length = (x * x + y * y + z * z + w * w).sqrt().max(f32::EPSILON);
    let (x, y, z, w) = (x / length, y / length, z / length, w / length);
    // v + 2w(q × v) + 2q × (q × v)
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let q = [x, y, z];
    let t = cross(q, vector).map(|value| 2.0 * value);
    let u = cross(q, t);
    std::array::from_fn(|axis| vector[axis] + w * t[axis] + u[axis])
}

/// The middle and four corners of `slot`, on a wall standing at `wall`, as it is turned: bottom left, bottom right,
/// top left and top right.
fn corners(slot: &tomori_protocol::wall::SlotFrame, wall: &Pose) -> ([f32; 3], [[f32; 3]; 4]) {
    let at = slot.pose(wall);
    let right = rotate(at.orientation, [1.0, 0.0, 0.0]);
    let up = rotate(at.orientation, [0.0, 1.0, 0.0]);
    let [width, height] = slot.size;
    let corner = |across: f32, rise: f32| {
        std::array::from_fn(|axis| {
            at.position[axis] + right[axis] * across * width / 2.0 + up[axis] * rise * height / 2.0
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

/// The box about each slot of `wall`, where it stands, tilted as it tilts, aligned with the space's axes.
pub(super) fn boxes(wall: &Wall) -> impl Iterator<Item = ([f32; 3], [f32; 3])> + '_ {
    wall.slot_frames().into_iter().map(move |slot| {
        let (middle, corners) = corners(&slot, &wall.frame());
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
    use tomori_environment::sightlines::{READING, Spot, readable};
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

    /// The agent summoned by someone whose eyes are at `eyes`, drawn towards `at`, as a client and the server stand it.
    fn summoned(at: [f32; 3], eyes: [f32; 3]) -> Pose {
        entourage::summoned(at, eyes, entourage::FLOOR)
    }

    /// `tomori()` standing at `pose`.
    fn placed(pose: Pose) -> Wall {
        let mut wall = tomori();
        wall.pose = pose;
        wall
    }

    #[test]
    fn it_stands_beneath_the_summoned_agent_facing_whoever_summoned_it_as_its_home_stands_beneath_the_agent_s_home()
     {
        // Someone 3 m to the right of home, facing +X, summons the agent ahead of them.
        let eyes = [3.0, 1.6, 0.0];
        let agent = summoned([4.2, 1.0, 0.0], eyes);
        let spot = spot(Some(eyes), &agent, &tomori(), &[], &[]).expect("clear");
        // Directly beneath the swarm's middle, 0.88 m below it, facing them.
        assert!(across(spot.position, agent.position) < 1e-4, "{spot:?}");
        assert!((agent.position[1] - spot.position[1] - entourage::BENEATH).abs() < 1e-5);
        let facing = Spot::facing(spot.position, eyes).yaw;
        assert!((spot.yaw() - facing).abs() < 1e-4);
        // Seen beneath the swarm, never behind it, with the gap between them.
        assert!(entourage::seen_beneath(eyes, &agent, &placed(spot)));
        // Its home, by the same rule from home, is where it is.
        let home = super::super::default_stage();
        let default = Pose::at(super::super::AGENT).facing_towards([0.0, 1.6, 0.0]);
        let from_home = super::spot(Some([0.0, 1.6, 0.0]), &default, &tomori(), &[], &[]).unwrap();
        assert!(across(from_home.position, home.position) < 1e-4);
        assert!((from_home.position[1] - home.position[1]).abs() < 1e-4);
        // So the rule every environment's home keeps holds of it: readable from where they stand.
        let standing = Spot::new([eyes[0], 0.0, eyes[2]], 0.0);
        assert_eq!(
            readable(&standing, &Spot::new(spot.position, spot.yaw()), &[]),
            Ok(())
        );
    }

    #[test]
    fn it_never_stands_in_the_floor_nor_behind_the_swarm_however_tall_whoever_summoned_it_is() {
        // From someone seated, their eyes 1 m up, to someone tall standing, 1.8 m, summoning the agent near and far.
        for tenth in 10..=18 {
            let eyes = [0.5, tenth as f32 / 10.0, 2.0];
            for (towards, distance) in [([0.0, -1.0], 1.0), ([0.8, -0.6], 1.6), ([-1.0, 0.0], 2.4)]
            {
                let at = [
                    eyes[0] + towards[0] * distance,
                    eyes[1] - 0.9,
                    eyes[2] + towards[1] * distance,
                ];
                let agent = summoned(at, eyes);
                let spot = spot(Some(eyes), &agent, &tomori(), &[], &[]).expect("clear");
                let wall = placed(spot);
                // Beneath it, its row's bottom edge, and every slot's, above the floor by the clearance.
                assert!(
                    across(spot.position, agent.position) < 1e-4,
                    "{eyes:?} {at:?}"
                );
                for slot in wall.slot_frames() {
                    let (_, [low_left, low_right, ..]) = corners(&slot, &wall.frame());
                    for corner in [low_left, low_right] {
                        assert!(corner[1] >= entourage::CLEARANCE, "{eyes:?} {corner:?}");
                    }
                }
                // The gap between the swarm's lowest reach and the top of its row, and nothing of the swarm seen over
                // it.
                let top = spot.position[1] + entourage::ROW_HALF;
                assert!(agent.position[1] - entourage::SWARM[1] - top >= entourage::GAP - 1e-5);
                assert!(
                    entourage::seen_beneath(eyes, &agent, &wall),
                    "{eyes:?} {at:?}"
                );
                // Below their eyes, within reading distance of them.
                assert!(spot.position[1] < eyes[1]);
                assert!(
                    (0..3)
                        .map(|axis| (spot.position[axis] - eyes[axis]).powi(2))
                        .sum::<f32>()
                        .sqrt()
                        <= READING
                );
            }
        }
    }

    #[test]
    fn it_moves_or_turns_aside_for_scenery_other_walls_and_the_swarm_and_stays_home_where_nothing_is_clear()
     {
        let eyes = [0.0, 1.6, 0.0];
        let agent = summoned([0.0, 1.2, -1.5], eyes);
        let beneath = spot_with(eyes, &agent, &[], &[]);
        // A shed where it would stand, beneath the agent: it moves, still readable, clear of it and seen beneath the
        // swarm.
        let [x, y, z] = beneath.position;
        let shed = Solid::new([x - 0.6, 0.0, z - 0.2], [x + 0.6, y + 0.08, z + 0.2]);
        let spot = spot(Some(eyes), &agent, &tomori(), &[], &[shed]).expect("clear");
        assert!(across(spot.position, beneath.position) > 0.2, "{spot:?}");
        let wall = placed(spot);
        for slot in wall.slot_frames() {
            let (middle, corners) = corners(&slot, &spot);
            assert!(!shed.contains(middle) && !corners.iter().any(|corner| shed.contains(*corner)));
            assert!(!shed.blocks(eyes, middle));
        }
        assert!(entourage::seen_beneath(eyes, &agent, &wall));
        assert!((spot.position[1] - beneath.position[1]).abs() < 1e-5);
        // A thin board standing across a slot, between its corners and its middle, as another wall seen edge on: it
        // moves too, no edge or diagonal of any slot passing through it.
        let board = Solid::new([x + 0.1, 0.0, z - 0.1], [x + 0.12, y + 0.4, z + 0.05]);
        let thin = spot_with(eyes, &agent, &[], &[board]);
        assert!(across(thin.position, beneath.position) > 0.2, "{thin:?}");
        let wall = placed(thin);
        for slot in wall.slot_frames() {
            let (_, [a, b, c, d]) = corners(&slot, &thin);
            for (from, to) in [(a, b), (b, d), (d, c), (c, a), (a, d), (b, c)] {
                assert!(!board.blocks(from, to));
            }
        }
        // A wall someone added there: the same.
        let added = Wall::new("wall-1", WallKind::Added, beneath);
        let beside = spot_with(eyes, &agent, std::slice::from_ref(&added), &[]);
        assert!(boxes(&placed(beside)).all(|(low, high)| {
            boxes(&added).all(|(other_low, other_high)| {
                (0..3).any(|axis| {
                    high[axis] < other_low[axis] - THICKNESS
                        || low[axis] > other_high[axis] + THICKNESS
                })
            })
        }));
        // A swarm too low for its wall beneath it, as an older client might leave it: nowhere is beneath it, and the
        // wall stays home.
        let low = Pose::at([0.0, 0.9, -1.5]).facing_towards(eyes);
        assert_eq!(super::spot(Some(eyes), &low, &tomori(), &[], &[]), None);
        // Walled in on every side: it stays home.
        let around = Solid::new([-5.0, -1.0, -5.0], [5.0, 3.0, -0.5]);
        assert_eq!(
            super::spot(Some(eyes), &agent, &tomori(), &[], &[around]),
            None
        );
    }

    fn spot_with(eyes: [f32; 3], agent: &Pose, others: &[Wall], solids: &[Solid]) -> Pose {
        spot(Some(eyes), agent, &tomori(), others, solids).expect("clear")
    }

    #[test]
    fn every_spot_it_stands_at_is_within_reading_distance_of_the_eyes() {
        let eyes = [0.0, 1.6, 0.0];
        // Summoned 4.2 m away, beneath it is beyond reading distance: it stands nearer, beneath the swarm still.
        let agent = summoned([0.0, 1.0, -4.2], eyes);
        let spot = spot_with(eyes, &agent, &[], &[]);
        let distance = (0..3)
            .map(|axis| (spot.position[axis] - eyes[axis]).powi(2))
            .sum::<f32>()
            .sqrt();
        assert!(distance <= READING, "{distance}");
        assert!(across(spot.position, eyes) < across(agent.position, eyes));
        assert!(entourage::seen_beneath(eyes, &agent, &placed(spot)));
    }

    #[test]
    fn without_their_eyes_it_stands_beneath_the_agent_before_whoever_it_faces() {
        // The agent faces +X, towards someone 2.3 m away, as a summons leaves it.
        let agent = Pose::at([1.0, 1.9, 0.0]).facing_towards([3.0, 1.9, 0.0]);
        let spot = spot(None, &agent, &tomori(), &[], &[]).expect("clear");
        assert!(across(spot.position, agent.position) < 1e-4, "{spot:?}");
        // Facing them.
        let facing = Spot::facing(spot.position, [2.5, 0.0, 0.0]).yaw;
        assert!((spot.yaw() - facing).abs() < 1e-4);
        // Not finite, the same.
        let nonsense = super::spot(Some([f32::NAN, 0.0, 0.0]), &agent, &tomori(), &[], &[]);
        assert_eq!(nonsense, Some(spot));
        // Straight above their eyes, the swarm hides everything beneath it: it stays home.
        let above = Pose::at([1.0, 2.2, 0.0]).facing_towards([3.0, 2.2, 0.0]);
        assert_eq!(
            super::spot(Some([1.0, 1.6, 0.0]), &above, &tomori(), &[], &[]),
            None
        );
    }

    #[test]
    fn a_tilted_wall_is_kept_clear_as_it_tilts() {
        let eyes = [0.0, 1.6, 0.0];
        let agent = summoned([0.0, 1.2, -1.5], eyes);
        let beneath = spot_with(eyes, &agent, &[], &[]);
        // A low shelf just in front of the top of its middle slots, which the wall upright stands behind.
        let [x, y, z] = beneath.position;
        let top = y + entourage::ROW_HALF;
        let shelf = Solid::new(
            [x - 0.3, top - 0.056, z + 0.08],
            [x + 0.3, top + 0.054, z + 0.13],
        );
        let upright = spot_with(eyes, &agent, &[], &[shelf]);
        assert!(across(upright.position, beneath.position) < 1e-4);
        // Tilted with its top forward, the top of its middle slots would stand in the shelf: it moves.
        let tilted = tomori().with_tilt(Some(45.0));
        let aside = spot(Some(eyes), &agent, &tilted, &[], &[shelf]).expect("clear");
        assert!(across(aside.position, beneath.position) > 0.2, "{aside:?}");
        // Another wall tilted towards where it would stand is kept clear of as it tilts too.
        let mut leaning = tomori().with_tilt(Some(45.0));
        leaning.pose = Pose::at([0.0, 0.52, -2.3]).facing_towards(eyes);
        assert!(boxes(&leaning).all(|(low, high)| high[2] - low[2] > 0.1));
    }

    #[test]
    fn a_wall_the_space_reshaped_travels_as_it_is() {
        // Two columns, flat: it still stands beneath the agent, and its slots are those checked.
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
        let agent = summoned([0.0, 1.2, -2.0], eyes);
        let spot = spot(Some(eyes), &agent, &wall, &[], &[]).expect("clear");
        assert!(across(spot.position, agent.position) < 1e-4);
        // Too far for any spot to be read: beyond the reach of walls, it stays home.
        let far = Pose::at([30.0, 1.4, 0.0]);
        assert_eq!(
            super::spot(Some([30.0, 1.6, 2.0]), &far, &wall, &[], &[]),
            None
        );
    }
}
