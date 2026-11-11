// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The world's tests, opening with the stacking harness the physics proposal asks for before anyone builds a cairn on
//! the headset: cairns of 3 to 10 stones stacked programmatically, required to settle and sleep within seconds and then
//! never move for ten simulated minutes; one more stone dropped on, required to wake a cairn without bursting it; and
//! stones let go with the speeds a hand gives.
use super::*;
use crate::message::Intent;
use crate::{Encoded, PULL, RELEASE_SPEED, Side};
use glam::Vec2;

/// A flattish stone's hull, as one chosen for a cairn: a broad, gently domed top and bottom about a ring, `width`
/// across and `height` high, its points nudged by `seed`, its middle at its origin.
pub(crate) fn pebble(seed: u32, width: f32, height: f32) -> Vec<Vec3> {
    let nudge = |index: u32| {
        let mut value = (seed.wrapping_mul(2_654_435_761) ^ index.wrapping_mul(40_503))
            .wrapping_mul(2_246_822_519);
        value ^= value >> 15;
        (value % 1000) as f32 / 1000.0
    };
    let radius = width * 0.5;
    let mut points = Vec::new();
    for index in 0..6 {
        let turn = index as f32 / 6.0 * std::f32::consts::TAU;
        let reach = radius * (0.88 + nudge(index) * 0.12);
        points.push(Vec3::new(
            turn.cos() * reach,
            (nudge(index + 10) - 0.5) * height * 0.1,
            turn.sin() * reach * 0.82,
        ));
    }
    for (level, offset) in [(0.5, 0.0), (-0.5, 0.4)] {
        for index in 0..4 {
            let turn = (index as f32 + offset) / 4.0 * std::f32::consts::TAU;
            let reach = radius * 0.62;
            points.push(Vec3::new(
                turn.cos() * reach,
                level * height,
                turn.sin() * reach * 0.82,
            ));
        }
    }
    points
}

/// The ground: a box whose top is at zero.
fn ground() -> Static {
    Static::cuboid(
        Vec3::new(-20.0, -1.0, -20.0),
        Vec3::new(20.0, 0.0, 20.0),
        Quat::IDENTITY,
    )
}

/// A cairn of `count` stones, 20 cm to 13 cm across, each a third to a half as high, stacked a couple of millimetres
/// apart, a little turned and off centre, as a hand would place them, at `at` on the ground; and how high its top is.
fn cairn(count: u32, at: Vec2, first: u32) -> (Vec<Body>, f32) {
    let mut height = 0.0;
    let bodies = (0..count)
        .map(|index| {
            let width = 0.2 - index as f32 * 0.07 / count as f32;
            let tall = width * (0.36 + (index % 3) as f32 * 0.04);
            let hull = pebble(first + index, width, tall);
            let lowest = hull
                .iter()
                .map(|point| point.y)
                .fold(f32::INFINITY, f32::min);
            let highest = hull
                .iter()
                .map(|point| point.y)
                .fold(f32::NEG_INFINITY, f32::max);
            let offset = Vec2::new(
                ((index * 7) % 5) as f32 - 2.0,
                ((index * 3) % 5) as f32 - 2.0,
            ) * 0.003;
            let middle = Vec3::new(at.x + offset.x, height - lowest + 0.002, at.y + offset.y);
            height = middle.y + highest;
            Body::convex(first + index, hull).at(Placement::new(
                middle,
                Quat::from_rotation_y(index as f32 * 0.9),
            ))
        })
        .collect();
    (bodies, height)
}

fn world(bodies: Vec<Body>) -> World {
    World::new(Settings::default(), vec![ground()], Vec::new(), bodies).unwrap()
}

/// Where every body is.
fn placements(world: &World) -> Vec<Placement> {
    world
        .bodies()
        .map(|body| world.placement(body).unwrap())
        .collect()
}

/// Step `world` until nothing moves, for at most `seconds`, returning what it said and how long it took.
fn until_rest(world: &mut World, seconds: f32) -> (Vec<Update>, f32) {
    let mut updates = Vec::new();
    let mut taken = 0.0;
    while world.moving() && taken < seconds {
        updates.push(world.step(0.05));
        taken += 0.05;
    }
    (updates, taken)
}

#[test]
fn cairns_of_three_to_ten_stones_settle_and_sleep_within_seconds_and_never_move_in_ten_minutes() {
    for count in [3, 5, 7, 10] {
        let (bodies, top) = cairn(count, Vec2::ZERO, 0);
        let placed: Vec<Placement> = bodies.iter().map(|body| body.home).collect();
        let mut world = world(bodies);
        assert!(world.settle(10.0), "{count} stones never all slept");
        let settled = world.clock();
        assert!(
            settled <= 4_000,
            "{count} stones took {settled} ms to sleep"
        );
        // Standing: no stone has crept more than a centimetre and a half from where it was placed, nor sunk more than
        // the gaps it was placed with beneath it and a millimetre more for each stone, as contacts settle.
        let rested = placements(&world);
        for (index, (placed, rested)) in placed.iter().zip(&rested).enumerate() {
            let crept = (rested.position - placed.position).with_y(0.0).length();
            let sank = placed.position.y - rested.position.y;
            assert!(
                crept < 0.015,
                "{count} stones: stone {index} crept {crept} m"
            );
            assert!(
                sank < (index + 1) as f32 * 0.003,
                "{count} stones: stone {index} sank {sank} m"
            );
        }
        let highest = world
            .bodies()
            .map(|body| world.placement(body).unwrap().position.y)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(highest < top, "{count} stones");
        // Asleep, nothing moves, nothing is said, and every stone stays put to the bit, for ten minutes.
        for _ in 0..(600 * 4) {
            assert!(world.step(0.25).is_empty(), "{count} stones woke");
        }
        assert_eq!(
            placements(&world),
            rested,
            "{count} stones moved while asleep"
        );
        assert!(!world.moving());
    }
}

#[test]
fn one_more_stone_dropped_on_wakes_a_cairn_without_bursting_it() {
    for count in [3, 6, 9] {
        let (mut bodies, top) = cairn(count, Vec2::ZERO, 0);
        let extra =
            Body::convex(100, pebble(100, 0.12, 0.05)).at(Placement::at(Vec3::new(1.0, 0.03, 0.0)));
        bodies.push(extra);
        let mut world = world(bodies);
        assert!(world.settle(10.0));
        let before = placements(&world);
        // Taken, and let go a few centimetres above the top, unmoving, by someone kneeling beside it.
        let seen = Seen {
            head: Vec3::new(0.3, 1.1, 0.3),
            hands: [None, Some(Placement::at(Vec3::new(0.3, 1.0, 0.0)))],
        };
        let offset = Placement::at(Vec3::new(0.0, 0.0, -0.05)).encoded(Precision::Rest);
        let take = Take {
            body: 100,
            epoch: 0,
            hand: Side::Right,
            offset,
        };
        let epoch = world.take(7, &take, Some(&seen)).unwrap();
        let let_go = LetGo {
            body: 100,
            epoch,
            at: Placement::at(Vec3::new(0.0, top + 0.06, 0.0)).encoded(Precision::Rest),
            velocity: [0.0; 3],
            spin: [0.0; 3],
            clock: 0,
        };
        world.let_go(7, &let_go, Some(&seen)).unwrap();
        let (updates, taken) = until_rest(&mut world, 10.0);
        assert!(taken < 6.0, "{count} stones: still moving after {taken} s");
        // Woken, moved, but not burst: every stone of the cairn within a few centimetres of where it stood, and the new
        // one on top of it.
        assert!(
            updates.iter().any(|update| update.moving.len() > 1),
            "{count} stones: the cairn never woke"
        );
        let after = placements(&world);
        for (index, (before, after)) in before.iter().zip(&after).take(count as usize).enumerate() {
            let moved = before.position.distance(after.position);
            assert!(
                moved < 0.03,
                "{count} stones: stone {index} moved {moved} m"
            );
        }
        let extra = world.placement(100).unwrap().position;
        assert!(
            extra.y > top - 0.03,
            "{count} stones: the extra stone fell off, at {extra}"
        );
    }
}

#[test]
fn stones_let_go_with_a_hand_s_speeds_and_spins_come_to_rest_on_the_ground() {
    let bodies: Vec<Body> = (0..6)
        .map(|index| {
            Body::convex(index, pebble(index, 0.18, 0.07)).at(Placement::at(Vec3::new(
                index as f32 * 0.4,
                0.04,
                0.0,
            )))
        })
        .collect();
    let mut world = world(bodies);
    assert!(world.settle(5.0));
    let seen = Seen {
        head: Vec3::new(0.0, 1.6, 1.0),
        hands: [
            Some(Placement::at(Vec3::new(-0.2, 1.1, 0.6))),
            Some(Placement::at(Vec3::new(0.2, 1.1, 0.6))),
        ],
    };
    let speeds = [(1.0, 2.0), (3.0, 5.0), (8.0, 20.0)];
    for (index, (speed, spin)) in speeds.into_iter().enumerate() {
        let body = index as u32 * 2;
        for (hand, side) in [(0, Side::Left), (1, Side::Right)] {
            let body = body + hand;
            let take = Take {
                body,
                epoch: 0,
                hand: side,
                offset: Encoded::default(),
            };
            let epoch = world.take(3, &take, Some(&seen)).unwrap();
            let let_go = LetGo {
                body,
                epoch,
                at: Placement::at(Vec3::new(hand as f32 * 0.4, 1.2, 0.5)).encoded(Precision::Rest),
                velocity: [0.3, 0.5, -speed],
                spin: [spin, 0.0, 0.0],
                clock: 0,
            };
            world.let_go(3, &let_go, Some(&seen)).unwrap();
        }
        let (updates, taken) = until_rest(&mut world, 20.0);
        assert!(
            taken < 15.0,
            "thrown at {speed} m/s, still moving after {taken} s"
        );
        assert!(
            updates.iter().any(|update| !update.landed.is_empty()),
            "never landed"
        );
        for body in [body, body + 1] {
            let at = world.placement(body).unwrap();
            assert!(
                at.position.y > 0.0 && at.position.y < 0.2,
                "thrown at {speed} m/s, resting at {at:?}"
            );
        }
    }
}

#[test]
fn a_world_takes_only_what_it_can_simulate() {
    let stone = |id| Body::convex(id, pebble(id, 0.2, 0.08));
    let refused = |statics: Vec<Static>, bodies: Vec<Body>| {
        World::new(Settings::default(), statics, Vec::new(), bodies).err()
    };
    assert_eq!(
        refused(vec![], (0..65).map(stone).collect()),
        Some(Refused::Bodies)
    );
    assert_eq!(
        refused(vec![], vec![stone(1), stone(1)]),
        Some(Refused::Duplicate(1))
    );
    assert_eq!(
        refused(vec![], vec![stone(1).with_density(0.0)]),
        Some(Refused::Body(1))
    );
    let flat = vec![Vec3::ZERO, Vec3::X, Vec3::Z, Vec3::new(1.0, 0.0, 1.0)];
    assert_eq!(
        refused(vec![], vec![Body::convex(2, flat.clone())]),
        Some(Refused::Hull(2))
    );
    assert_eq!(
        refused(vec![Static::convex(flat)], vec![]),
        Some(Refused::Static)
    );
    assert_eq!(
        refused(vec![Static::convex(vec![Vec3::ZERO; 3])], vec![]),
        Some(Refused::Static)
    );
    let many = Static::heights(Vec2::ZERO, 0.1, 100, &vec![0.0; 100 * 100]);
    assert_eq!(
        refused(vec![many.clone(), many], vec![]),
        Some(Refused::Triangles)
    );
    assert_eq!(
        refused(vec![], vec![Body::convex(3, vec![Vec3::X; 4])]),
        Some(Refused::Hull(3))
    );
    let mut world = World::new(
        Settings::default(),
        vec![
            ground(),
            Static::convex(pebble(9, 0.5, 0.2)),
            Static::heights(Vec2::ZERO, 1.0, 2, &[0.0; 4]),
        ],
        Vec::new(),
        vec![stone(4).at(Placement::at(Vec3::new(3.0, 0.05, 0.0)))],
    )
    .unwrap();
    assert_eq!(world.bodies().collect::<Vec<_>>(), [4]);
    assert_eq!(world.placement(5), None);
    assert_eq!((world.held(5), world.home(5)), (None, None));
    assert!(!world.moving());
    assert!(world.step(f32::NAN).is_empty());
    assert_eq!(world.clock(), 0);
}

/// One stone on the ground, settled, someone seen beside it, and how to take it.
fn one() -> (World, Seen, Take) {
    let mut world = world(vec![
        Body::convex(1, pebble(1, 0.2, 0.08)).at(Placement::at(Vec3::new(0.0, 0.05, 0.0))),
    ]);
    assert!(world.settle(5.0));
    let seen = Seen {
        head: Vec3::new(0.0, 1.6, 0.6),
        hands: [None, Some(Placement::at(Vec3::new(0.1, 0.9, 0.3)))],
    };
    let take = Take {
        body: 1,
        epoch: 0,
        hand: Side::Right,
        offset: Placement::at(Vec3::new(0.0, 0.0, -0.06)).encoded(Precision::Rest),
    };
    (world, seen, take)
}

#[test]
fn a_body_is_taken_once_at_the_epoch_seen_within_reach_with_a_free_hand() {
    let (mut world, seen, take) = one();
    assert_eq!(
        world.take(3, &Take { body: 9, ..take }, Some(&seen)),
        Err(Refused::Unknown(9))
    );
    assert_eq!(
        world.take(3, &Take { epoch: 4, ..take }, Some(&seen)),
        Err(Refused::Epoch(1))
    );
    assert_eq!(world.take(3, &take, None), Err(Refused::Reach(1)));
    let far = Seen {
        head: Vec3::new(0.0, 1.6, 6.0),
        hands: [None, Some(Placement::at(Vec3::new(0.0, 1.0, PULL + 1.0)))],
    };
    assert_eq!(world.take(3, &take, Some(&far)), Err(Refused::Reach(1)));
    assert_eq!(world.take(3, &take, Some(&seen)), Ok(1));
    assert_eq!(
        world.held(1).map(|held| (held.by, held.hand)),
        Some((3, Side::Right))
    );
    assert_eq!(world.placement(1), None);
    let update = world.step(0.0);
    assert_eq!(
        update.holding,
        [Holding {
            body: 1,
            epoch: 1,
            held: world.held(1)
        }]
    );
    // The first to take it has it: a second taking, even at the epoch it is now, is refused.
    assert_eq!(
        world.take(4, &Take { epoch: 1, ..take }, Some(&seen)),
        Err(Refused::Held(1))
    );
    let shared = world.shared();
    assert_eq!(
        (
            shared.bodies[0].epoch,
            shared.bodies[0].held.map(|held| held.by)
        ),
        (1, Some(3))
    );
    // Held, it is out of the simulation: nothing moves.
    assert!(!world.moving());
}

#[test]
fn each_participant_holds_one_body_in_each_hand() {
    let bodies: Vec<Body> = (0..3)
        .map(|id| {
            Body::convex(id, pebble(id, 0.15, 0.06)).at(Placement::at(Vec3::new(
                id as f32 * 0.3,
                0.04,
                0.0,
            )))
        })
        .collect();
    let mut world = world(bodies);
    world.settle(5.0);
    let seen = Seen {
        head: Vec3::new(0.3, 1.6, 0.5),
        hands: [
            Some(Placement::at(Vec3::new(0.0, 1.0, 0.3))),
            Some(Placement::at(Vec3::new(0.6, 1.0, 0.3))),
        ],
    };
    let take = |body, hand| Take {
        body,
        epoch: 0,
        hand,
        offset: Encoded::default(),
    };
    assert_eq!(world.take(2, &take(0, Side::Left), Some(&seen)), Ok(1));
    assert_eq!(
        world.take(2, &take(1, Side::Left), Some(&seen)),
        Err(Refused::Hands)
    );
    assert_eq!(world.take(2, &take(1, Side::Right), Some(&seen)), Ok(1));
    assert_eq!(
        world.take(2, &take(2, Side::Right), Some(&seen)),
        Err(Refused::Hands)
    );
    // Someone else's hands are their own.
    assert_eq!(world.take(5, &take(2, Side::Right), Some(&seen)), Ok(1));
    assert_eq!(world.holders(), [2, 5]);
    // Let go of as they leave, where their hands were seen, unmoving, each said.
    let mut let_go = world.let_go_of(2, Some(&seen));
    let_go.sort();
    assert_eq!(let_go, [0, 1]);
    let update = world.step(0.0);
    assert_eq!(
        update
            .holding
            .iter()
            .filter(|holding| holding.held.is_none())
            .count(),
        2
    );
    assert!(
        world
            .placement(0)
            .unwrap()
            .position
            .distance(Vec3::new(0.0, 1.0, 0.3))
            < 1e-3
    );
    // Unseen, where each was taken.
    assert_eq!(world.let_go_of(5, None), [2]);
    assert!(
        world
            .placement(2)
            .unwrap()
            .position
            .distance(Vec3::new(0.6, 0.04, 0.0))
            < 0.05
    );
    assert!(world.let_go_of(9, None).is_empty());
    assert!(world.holders().is_empty());
    let (updates, _) = until_rest(&mut world, 10.0);
    assert!(updates.iter().flat_map(|update| &update.rested).count() >= 3);
}

#[test]
fn only_its_holder_lets_go_of_a_body_at_the_epoch_their_taking_made() {
    let (mut world, seen, take) = one();
    let epoch = world.take(3, &take, Some(&seen)).unwrap();
    world.step(0.0);
    let let_go = LetGo {
        body: 1,
        epoch,
        at: Placement::at(Vec3::new(0.0, 1.0, 0.0)).encoded(Precision::Rest),
        velocity: [0.0, 0.0, -2.0 * RELEASE_SPEED],
        spin: [0.0; 3],
        clock: 0,
    };
    assert_eq!(world.let_go(4, &let_go, Some(&seen)), Err(Refused::Held(1)));
    assert_eq!(
        world.let_go(3, &LetGo { epoch: 0, ..let_go }, Some(&seen)),
        Err(Refused::Epoch(1))
    );
    assert_eq!(
        world.let_go(3, &LetGo { body: 8, ..let_go }, Some(&seen)),
        Err(Refused::Unknown(8))
    );
    assert_eq!(world.let_go(3, &let_go, Some(&seen)), Ok(2));
    // Said at once where it was let go, moving, no faster than the bound.
    let update = world.step(0.0);
    assert_eq!(
        update.holding,
        [Holding {
            body: 1,
            epoch: 2,
            held: None
        }]
    );
    assert_eq!(update.moving.len(), 1);
    assert!(world.moving());
    assert!(world.shared().bodies[0].moving);
    let mut fastest: f32 = 0.0;
    let mut last = world.placement(1).unwrap().position;
    for _ in 0..10 {
        world.step(1.0 / 60.0);
        let now = world.placement(1).unwrap().position;
        fastest = fastest.max(now.distance(last) * 60.0);
        last = now;
    }
    assert!(fastest <= RELEASE_SPEED + 0.5, "{fastest}");
    // Not held, it is no one's to let go.
    assert_eq!(world.let_go(3, &let_go, Some(&seen)), Err(Refused::Held(1)));
    // Let go by someone the side no longer sees, it is let go where they say only within reach of where it was taken.
    let (mut world, seen, take) = one();
    let taken = world.placement(1).unwrap();
    let epoch = world.take(3, &take, Some(&seen)).unwrap();
    let unseen = LetGo {
        epoch,
        at: Placement::at(taken.position + Vec3::new(0.0, 3.0, 0.0)).encoded(Precision::Rest),
        ..let_go
    };
    world.let_go(3, &unseen, None).unwrap();
    assert!(
        world
            .placement(1)
            .unwrap()
            .position
            .distance(taken.position)
            < 0.03
    );
}

#[test]
fn a_body_let_go_inside_another_or_the_ground_is_lifted_clear() {
    let (mut world, seen, take) = one();
    // Kneeling beside it.
    let seen = Seen {
        head: Vec3::new(0.0, 0.3, 0.5),
        ..seen
    };
    let epoch = world.take(3, &take, Some(&seen)).unwrap();
    let let_go = LetGo {
        body: 1,
        epoch,
        at: Placement::at(Vec3::new(0.0, 0.0, 0.0)).encoded(Precision::Rest),
        velocity: [0.0; 3],
        spin: [0.0; 3],
        clock: 0,
    };
    world.let_go(3, &let_go, Some(&seen)).unwrap();
    let lifted = world.placement(1).unwrap();
    assert!(lifted.position.y >= 0.03, "{lifted:?}");
    assert!(lifted.position.y <= 0.06, "{lifted:?}");
    // Deeper than it lifts, it is let go as far as it lifts, and the solver does the rest.
    let epoch = world
        .take(3, &Take { epoch: 2, ..take }, Some(&seen))
        .unwrap();
    let buried = LetGo {
        body: 1,
        epoch,
        at: Placement::at(Vec3::new(0.0, -0.7, 0.4)).encoded(Precision::Rest),
        ..let_go
    };
    world.let_go(3, &buried, Some(&seen)).unwrap();
    assert!((world.placement(1).unwrap().position.y - (-0.7 + CLEAR)).abs() < 0.02);
}

#[test]
fn a_grabbable_body_rests_where_it_is_let_go() {
    let hook = Body::convex(1, pebble(1, 0.2, 0.08))
        .kinematic()
        .at(Placement::at(Vec3::new(0.0, 1.0, 0.0)));
    let mut world = world(vec![hook]);
    assert!(world.settle(1.0));
    let seen = Seen {
        head: Vec3::new(0.0, 1.6, 0.6),
        hands: [None, Some(Placement::at(Vec3::new(0.0, 1.2, 0.3)))],
    };
    let take = Take {
        body: 1,
        epoch: 0,
        hand: Side::Right,
        offset: Encoded::default(),
    };
    let epoch = world.take(3, &take, Some(&seen)).unwrap();
    let let_go = LetGo {
        body: 1,
        epoch,
        at: Placement::at(Vec3::new(0.3, 1.3, 0.2)).encoded(Precision::Rest),
        velocity: [0.0, 0.0, -3.0],
        spin: [0.0; 3],
        clock: 0,
    };
    world.let_go(3, &let_go, Some(&seen)).unwrap();
    let update = world.step(0.5);
    assert_eq!(update.rested.len(), 1);
    assert!(update.moving.is_empty());
    assert!(!world.moving());
    assert_eq!(
        world.placement(1),
        Some(Placement::at(Vec3::new(0.3, 1.3, 0.2)).snapped(Precision::Rest))
    );
}

#[test]
fn what_moves_is_said_at_most_twenty_times_a_second_and_where_it_rests_once_exactly() {
    let (mut world, seen, take) = one();
    let epoch = world.take(3, &take, Some(&seen)).unwrap();
    let let_go = LetGo {
        body: 1,
        epoch,
        at: Placement::at(Vec3::new(0.0, 1.0, 0.0)).encoded(Precision::Rest),
        velocity: [0.0; 3],
        spin: [1.0, 2.0, 0.0],
        clock: 0,
    };
    world.let_go(3, &let_go, Some(&seen)).unwrap();
    world.step(0.0);
    let mut said = 0;
    let mut rested = Vec::new();
    for _ in 0..(60 * 6) {
        let update = world.step(1.0 / 60.0);
        said += !update.moving.is_empty() as u32;
        rested.extend(update.rested);
    }
    let seconds = world.clock() as f32 / 1000.0 - 5.0;
    assert!(
        said as f32 <= SNAPSHOT_RATE as f32 * 6.0 + 1.0,
        "{said} in {seconds} s"
    );
    assert!(said >= 10);
    assert_eq!(rested.len(), 1);
    // It rests exactly where it was said to, as it was snapped there.
    assert_eq!(
        world.placement(1),
        Some(rested[0].placement(Precision::Rest))
    );
    assert_eq!(
        world.shared().bodies[0].at,
        world.placement(1).unwrap().encoded(Precision::Rest)
    );
}

#[test]
fn a_body_lost_below_the_world_or_beyond_its_reach_goes_back_where_it_began() {
    for (to, velocity) in [
        (Vec3::new(0.0, 1.0, 0.0), [0.0, 0.0, 8.0]),
        (Vec3::new(0.0, 1.2, 0.0), [0.0; 3]),
    ] {
        let mut settings = Settings::default();
        settings.lost_beyond = 2.0;
        // The ground ends a metre and a half away, and it falls off there, or is thrown beyond reach.
        let edge = Static::cuboid(
            Vec3::new(-1.5, -0.5, -1.5),
            Vec3::new(1.5, 0.0, 1.5),
            Quat::IDENTITY,
        );
        let stone =
            Body::convex(1, pebble(1, 0.2, 0.08)).at(Placement::at(Vec3::new(0.0, 0.05, 0.0)));
        let mut world = World::new(settings, vec![edge], Vec::new(), vec![stone]).unwrap();
        world.settle(5.0);
        let home = world.home(1).unwrap();
        let seen = Seen {
            head: Vec3::new(0.0, 1.6, 0.6),
            hands: [None, Some(Placement::at(Vec3::new(0.0, 1.0, 0.3)))],
        };
        let take = Take {
            body: 1,
            epoch: 0,
            hand: Side::Right,
            offset: Encoded::default(),
        };
        let epoch = world.take(3, &take, Some(&seen)).unwrap();
        let mut at = Placement::at(to);
        if velocity[2] == 0.0 {
            // Let go beside the ground's edge, it falls.
            at.position.x = 1.45;
            let seen = Seen {
                head: Vec3::new(1.4, 1.6, 0.6),
                ..seen
            };
            let let_go = LetGo {
                body: 1,
                epoch,
                at: at.encoded(Precision::Rest),
                velocity,
                spin: [0.0; 3],
                clock: 0,
            };
            world.let_go(3, &let_go, Some(&seen)).unwrap();
            // Nudged off it.
            world.physics.bodies[world.entries[0].handle]
                .set_linvel(Vec3::new(1.0, 0.0, 0.0), true);
        } else {
            let let_go = LetGo {
                body: 1,
                epoch,
                at: at.encoded(Precision::Rest),
                velocity,
                spin: [0.0; 3],
                clock: 0,
            };
            world.let_go(3, &let_go, Some(&seen)).unwrap();
        }
        let (updates, _) = until_rest(&mut world, 10.0);
        let rested: Vec<&Snapshot> = updates.iter().flat_map(|update| &update.rested).collect();
        let back = rested.last().unwrap().placement(Precision::Rest);
        assert!(
            back.position.distance(home.position) < 0.001,
            "{to}: {back:?} {home:?}"
        );
        assert!(world.placement(1).unwrap().position.distance(home.position) < 0.001);
    }
}

#[test]
fn water_slows_and_buoys_what_enters_it_and_takes_back_what_rests_in_it() {
    // A pool a metre deep beside the shore, which takes back a stone that rests in it after ten seconds.
    let shore = Static::cuboid(
        Vec3::new(-2.0, -1.0, -2.0),
        Vec3::new(0.0, 0.0, 2.0),
        Quat::IDENTITY,
    );
    let bed = Static::cuboid(
        Vec3::new(0.0, -2.0, -2.0),
        Vec3::new(4.0, -1.0, 2.0),
        Quat::IDENTITY,
    );
    let outline = vec![
        Vec2::new(0.0, -2.0),
        Vec2::new(4.0, -2.0),
        Vec2::new(4.0, 2.0),
        Vec2::new(0.0, 2.0),
    ];
    let pool = Water::below(-0.1, outline).returning(10_000);
    let stone = Body::convex(1, pebble(1, 0.2, 0.08)).at(Placement::at(Vec3::new(-1.0, 0.05, 0.0)));
    let mut world = World::new(
        Settings::default(),
        vec![shore, bed],
        vec![pool],
        vec![stone],
    )
    .unwrap();
    world.settle(5.0);
    let home = world.home(1).unwrap();
    let seen = Seen {
        head: Vec3::new(-0.5, 1.6, 0.0),
        hands: [None, Some(Placement::at(Vec3::new(-0.3, 1.0, 0.0)))],
    };
    let take = Take {
        body: 1,
        epoch: 0,
        hand: Side::Right,
        offset: Encoded::default(),
    };
    let epoch = world.take(3, &take, Some(&seen)).unwrap();
    let let_go = LetGo {
        body: 1,
        epoch,
        at: Placement::at(Vec3::new(0.2, 1.0, 0.0)).encoded(Precision::Rest),
        velocity: [4.0, 2.0, 0.0],
        spin: [0.0; 3],
        clock: 0,
    };
    world.let_go(3, &let_go, Some(&seen)).unwrap();
    // Into the water: a splash, slowed as it sinks rather than gathering speed as it would in air, and resting on the
    // bed.
    let mut splashed = false;
    let (mut entering, mut deep) = (None, None);
    let mut last = world.placement(1).unwrap().position;
    for _ in 0..(60 * 8) {
        let update = world.step(1.0 / 60.0);
        splashed |= update.landed.iter().any(|landed| landed.water);
        let now = world.placement(1).unwrap().position;
        let speed = (last.y - now.y) * 60.0;
        if now.y < -0.15 {
            entering.get_or_insert(speed);
        }
        if now.y < -0.8 {
            deep.get_or_insert(speed);
        }
        last = now;
    }
    assert!(splashed);
    let (entering, deep) = (entering.unwrap(), deep.unwrap());
    assert!(
        deep < entering * 0.8,
        "entering at {entering} m/s, deep at {deep} m/s"
    );
    let rested = world.placement(1).unwrap();
    assert!(
        rested.position.y < -0.9 && rested.position.x > 0.5,
        "{rested:?}"
    );
    // It soaks, then glides home, saying where it goes as it glides, and rests there.
    assert!(world.moving());
    let (updates, taken) = until_rest(&mut world, 30.0);
    assert!((5.0..20.0).contains(&taken), "{taken}");
    assert!(
        updates
            .iter()
            .filter(|update| !update.moving.is_empty())
            .count()
            > 20
    );
    let back = world.placement(1).unwrap();
    assert!(
        back.position.distance(home.position) < 0.03,
        "{back:?} {home:?}"
    );
}

#[test]
fn taking_a_stone_from_beneath_others_wakes_them_and_they_fall() {
    let (bodies, _) = cairn(3, Vec2::ZERO, 0);
    let mut world = world(bodies);
    world.settle(10.0);
    let seen = Seen {
        head: Vec3::new(0.0, 1.6, 0.6),
        hands: [None, Some(Placement::at(Vec3::new(0.0, 0.5, 0.3)))],
    };
    let take = Take {
        body: 0,
        epoch: 0,
        hand: Side::Right,
        offset: Encoded::default(),
    };
    let tops: Vec<f32> = (1..3)
        .map(|body| world.placement(body).unwrap().position.y)
        .collect();
    world.take(3, &take, Some(&seen)).unwrap();
    assert!(world.moving());
    until_rest(&mut world, 10.0);
    for (body, top) in (1..3).zip(tops) {
        assert!(
            world.placement(body).unwrap().position.y < top - 0.02,
            "{body} did not fall"
        );
    }
    let _ = Intent::Take(take);
}
