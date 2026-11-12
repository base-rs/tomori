// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The world's tests, opening with the stacking harness the physics proposal asks for before anyone builds a cairn on
//! the headset: cairns of 3 to 10 stones stacked programmatically, required to settle and sleep within seconds and then
//! never move for ten simulated minutes; one more stone dropped on, required to wake a cairn without bursting it; and
//! stones let go with the speeds a hand gives.
use super::*;
use crate::message::Intent;
use crate::{Encoded, PULL, RELEASE_SPEED, Side};
use glam::{Quat, Vec2};

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
    for fastest in [-1.0, f32::NAN] {
        assert_eq!(
            refused(vec![], vec![stone(1).thrown_at_most(fastest)]),
            Some(Refused::Body(1))
        );
    }
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
fn a_body_whose_letting_go_never_arrived_is_taken_again_by_its_holder_alone() {
    let (mut world, seen, take) = one();
    assert_eq!(world.take(3, &take, Some(&seen)), Ok(1));
    world.step(0.0);
    // Their letting go at epoch 1 lost, their client takes it again at the epoch its letting go would have made, with
    // the other hand: it is theirs again, in that hand, at the epoch after it, as their client expects.
    let again = Take {
        epoch: 2,
        hand: Side::Left,
        ..take
    };
    assert_eq!(world.take(3, &again, Some(&seen)), Ok(3));
    assert_eq!(
        world.held(1).map(|held| (held.by, held.hand)),
        Some((3, Side::Left))
    );
    assert_eq!(
        world.step(0.0).holding,
        [Holding {
            body: 1,
            epoch: 3,
            held: world.held(1)
        }]
    );
    assert!(!world.moving());
    // Or at the epoch it holds it at, as a client that heard of its taking but whose letting go was lost.
    assert_eq!(
        world.take(3, &Take { epoch: 3, ..take }, Some(&seen)),
        Ok(4)
    );
    // Nobody else takes it, at any epoch, nor its holder at one long gone or far ahead, nor unseen.
    for epoch in [0, 4, 5] {
        assert_eq!(
            world.take(5, &Take { epoch, ..take }, Some(&seen)),
            Err(Refused::Held(1))
        );
    }
    for epoch in [0, 3, 6] {
        assert_eq!(
            world.take(3, &Take { epoch, ..take }, Some(&seen)),
            Err(Refused::Held(1))
        );
    }
    assert_eq!(
        world.take(3, &Take { epoch: 4, ..take }, None),
        Err(Refused::Reach(1))
    );
    // Let go at the epoch its taking again made, it is let go.
    let let_go = LetGo {
        body: 1,
        epoch: 4,
        at: Placement::at(Vec3::new(0.0, 0.3, 0.0)).encoded(Precision::Rest),
        velocity: [0.0; 3],
        spin: [0.0; 3],
        clock: 0,
    };
    assert_eq!(world.let_go(3, &let_go, Some(&seen)), Ok(5));
    assert_eq!(world.held(1), None);
    assert_eq!((world.epoch(1), world.epoch(9)), (Some(5), None));
}

#[test]
fn a_body_held_in_one_hand_is_passed_to_the_other_without_falling_and_only_by_its_holder() {
    let (mut world, mut seen, take) = one();
    seen.hands[0] = Some(Placement::at(Vec3::new(-0.1, 0.9, 0.3)));
    assert_eq!(world.take(3, &take, Some(&seen)), Ok(1));
    world.step(0.0);
    // Their free left hand grips it at the epoch they hold it at: passed to it, at the offset that hand took it at,
    // still held, nothing let go, moving or landing, however long the world steps.
    let offset = Placement::new(Vec3::new(0.05, 0.0, -0.1), Quat::from_rotation_y(0.4))
        .encoded(Precision::Rest);
    let pass = Take {
        epoch: 1,
        hand: Side::Left,
        offset,
        ..take
    };
    assert!(world.passing(3, &pass));
    assert!(!world.passing(4, &pass));
    assert!(!world.passing(3, &Take { epoch: 2, ..pass }));
    assert!(!world.passing(3, &Take { body: 9, ..pass }));
    assert!(!world.passing(
        3,
        &Take {
            hand: Side::Right,
            ..pass
        }
    ));
    // Out of the hand's reach, or seen nowhere, or not seen at all, it is refused, and stays in the right hand.
    let mut unsound = seen;
    unsound.hands[0] = Some(Placement::at(Vec3::NAN));
    let mut far = seen;
    far.hands[0] = Some(Placement::at(Vec3::new(0.0, 0.9, PULL + 2.0)));
    for seen in [Some(&unsound), Some(&far), None] {
        assert_eq!(world.take(3, &pass, seen), Err(Refused::Reach(1)));
    }
    assert_eq!(world.held(1).map(|held| held.hand), Some(Side::Right));
    assert_eq!(world.take(3, &pass, Some(&seen)), Ok(2));
    assert_eq!(
        world.held(1),
        Some(Held {
            by: 3,
            hand: Side::Left,
            offset
        })
    );
    assert_eq!(world.placement(1), None);
    let update = world.step(0.5);
    assert_eq!(
        update,
        Update {
            holding: vec![Holding {
                body: 1,
                epoch: 2,
                held: world.held(1)
            }],
            ..Default::default()
        }
    );
    assert!(!world.moving());
    // Passed back, by the right hand gripping it again.
    let back = Take {
        epoch: 2,
        hand: Side::Right,
        ..take
    };
    assert!(world.passing(3, &back));
    assert_eq!(world.take(3, &back, Some(&seen)), Ok(3));
    assert_eq!(world.held(1).map(|held| held.hand), Some(Side::Right));
    // Someone else's grip on it, at the epoch it is at, is refused: a handover from another's hand is stage 3's.
    let theirs = Take {
        epoch: 3,
        hand: Side::Left,
        ..take
    };
    assert!(!world.passing(4, &theirs));
    assert_eq!(world.take(4, &theirs, Some(&seen)), Err(Refused::Held(1)));
    assert_eq!(world.held(1).map(|held| held.by), Some(3));
    // Let go by the hand it was passed to, at the epoch the pass made, it is let go.
    let let_go = LetGo {
        body: 1,
        epoch: 3,
        at: Placement::at(Vec3::new(0.0, 0.3, 0.0)).encoded(Precision::Rest),
        velocity: [0.0; 3],
        spin: [0.0; 3],
        clock: 0,
    };
    assert_eq!(world.let_go(3, &let_go, Some(&seen)), Ok(4));
}

#[test]
fn a_body_is_passed_only_to_a_free_hand() {
    let bodies: Vec<Body> = (0..2)
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
        head: Vec3::new(0.15, 1.6, 0.5),
        hands: [
            Some(Placement::at(Vec3::new(0.0, 1.0, 0.3))),
            Some(Placement::at(Vec3::new(0.3, 1.0, 0.3))),
        ],
    };
    let take = |body, epoch, hand| Take {
        body,
        epoch,
        hand,
        offset: Encoded::default(),
    };
    assert_eq!(world.take(2, &take(0, 0, Side::Left), Some(&seen)), Ok(1));
    assert_eq!(world.take(2, &take(1, 0, Side::Right), Some(&seen)), Ok(1));
    assert!(world.passing(2, &take(0, 1, Side::Right)));
    assert_eq!(
        world.take(2, &take(0, 1, Side::Right), Some(&seen)),
        Err(Refused::Hands)
    );
    assert_eq!(world.held(0).map(|held| held.hand), Some(Side::Left));
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
fn a_heavy_body_is_let_go_no_faster_than_it_is_thrown_at_most() {
    // A big stone, thrown as hard as a hand lets go of a small one, leaves at the most it may, and no faster; let go
    // gently, it keeps the speed it was given.
    for (asked, left) in [(2.0 * RELEASE_SPEED, 2.0), (1.5, 1.5)] {
        let mut world = world(vec![
            Body::convex(1, pebble(1, 0.6, 0.12))
                .thrown_at_most(2.0)
                .at(Placement::at(Vec3::new(0.0, 0.07, 0.0))),
        ]);
        world.settle(10.0);
        let seen = Seen {
            head: Vec3::new(0.0, 1.6, 0.6),
            hands: [None, Some(Placement::at(Vec3::new(0.0, 0.5, 0.3)))],
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
            at: Placement::at(Vec3::new(0.0, 1.0, 0.0)).encoded(Precision::Rest),
            velocity: [asked, 0.0, 0.0],
            spin: [0.0; 3],
            clock: 0,
        };
        world.let_go(3, &let_go, Some(&seen)).unwrap();
        let before = world.placement(1).unwrap().position;
        world.step(1.0 / 60.0);
        let speed = (world.placement(1).unwrap().position - before).x * 60.0;
        assert!((speed - left).abs() < 0.05, "{asked}: {speed}");
    }
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
        let settings = Settings {
            lost_beyond: 2.0,
            ..Settings::default()
        };
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
        // Taken back once, from where it was lost to its home, and never said anywhere between.
        let returning: Vec<&Returning> = updates
            .iter()
            .flat_map(|update| &update.returning)
            .collect();
        assert_eq!(returning.len(), 1, "{to}");
        assert_eq!(returning[0].to, home.encoded(Precision::Rest));
        let from = returning[0].from.placement(Precision::Rest).position;
        assert!(from.distance(home.position) > 1.0, "{to}: {from}");
        let after = updates
            .iter()
            .position(|update| !update.returning.is_empty())
            .unwrap();
        for update in &updates[after..] {
            for snapshot in &update.moving {
                let at = snapshot.placement(Precision::Moving).position;
                assert!(at.distance(home.position) < 0.01, "{to}: {at}");
            }
        }
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
    // It soaks, then is taken back home, said once, fading where it is and then in at home, never said between, and
    // rests there.
    assert!(world.moving());
    let (updates, taken) = until_rest(&mut world, 30.0);
    assert!((5.0..20.0).contains(&taken), "{taken}");
    let returning: Vec<&Returning> = updates
        .iter()
        .flat_map(|update| &update.returning)
        .collect();
    assert_eq!(returning.len(), 1);
    assert_eq!(returning[0].from, rested.encoded(Precision::Rest));
    assert_eq!(returning[0].to, home.encoded(Precision::Rest));
    assert!(
        updates
            .iter()
            .flat_map(|update| &update.moving)
            .all(|snapshot| snapshot
                .placement(Precision::Moving)
                .position
                .distance(home.position)
                < 0.01)
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

/// A number from 0 to 1, the next of `state`'s, the same on every platform.
fn uniform(state: &mut u64) -> f32 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    (*state >> 40) as f32 / (1u64 << 24) as f32
}

/// Wake every body and step Rapier alone for `seconds`, without the world's stillness rule: how far the farthest moved.
fn unruled(world: &mut World, seconds: f32) -> f32 {
    let before: Vec<Vec3> = world
        .entries
        .iter()
        .map(|entry| world.placement_of(entry).position)
        .collect();
    for entry in &world.entries {
        world.physics.bodies[entry.handle].wake_up(true);
    }
    for _ in 0..(seconds / world.settings.step) as usize {
        world.physics.step();
    }
    world
        .entries
        .iter()
        .zip(before)
        .map(|(entry, before)| world.placement_of(entry).position.distance(before))
        .fold(0.0, f32::max)
}

#[test]
fn cairns_placed_by_less_careful_hands_stand_and_sleep_and_the_stillness_rule_holds_none_up() {
    for seed in 0..12u64 {
        let mut state = seed.wrapping_mul(7_919).wrapping_add(1);
        for count in 3..=7u32 {
            // Up to 4 mm off centre, 1 to 3 mm apart, any way round and tipped up to a degree.
            let mut height = 0.0;
            let bodies: Vec<Body> = (0..count)
                .map(|index| {
                    let width = 0.2 - index as f32 * 0.07 / count as f32;
                    let tall = width * (0.36 + uniform(&mut state) * 0.08);
                    let hull = pebble(seed as u32 * 100 + index, width, tall);
                    let lowest = hull
                        .iter()
                        .map(|point| point.y)
                        .fold(f32::INFINITY, f32::min);
                    let highest = hull
                        .iter()
                        .map(|point| point.y)
                        .fold(f32::NEG_INFINITY, f32::max);
                    let off =
                        Vec2::new(uniform(&mut state) - 0.5, uniform(&mut state) - 0.5) * 0.008;
                    let gap = 0.001 + uniform(&mut state) * 0.002;
                    let middle = Vec3::new(off.x, height - lowest + gap, off.y);
                    height = middle.y + highest;
                    let turn = Quat::from_rotation_y(uniform(&mut state) * std::f32::consts::TAU)
                        * Quat::from_rotation_x((uniform(&mut state) - 0.5) * 0.035);
                    Body::convex(index, hull).at(Placement::new(middle, turn))
                })
                .collect();
            let placed: Vec<Vec3> = bodies.iter().map(|body| body.home.position).collect();
            let mut world = world(bodies);
            assert!(
                world.settle(10.0),
                "seed {seed}, {count} stones never slept"
            );
            assert!(
                world.clock() <= 5_000,
                "seed {seed}, {count} stones took {} ms to sleep",
                world.clock()
            );
            for (index, (placed, rested)) in placed.iter().zip(placements(&world)).enumerate() {
                let crept = (rested.position - placed).with_y(0.0).length();
                assert!(
                    crept < 0.015,
                    "seed {seed}, {count} stones: stone {index} crept {crept} m"
                );
            }
            // Woken and left to Rapier alone, it stands as it was put to sleep: nothing the rule stopped was falling.
            let moved = unruled(&mut world, 3.0);
            assert!(
                moved < 0.005,
                "seed {seed}, {count} stones moved {moved} m without the rule"
            );
        }
    }
}

#[test]
fn a_stone_overhanging_its_support_falls_rather_than_being_put_to_sleep() {
    let base = pebble(1, 0.2, 0.07);
    let top = pebble(2, 0.14, 0.05);
    let lowest = |hull: &[Vec3]| {
        hull.iter()
            .map(|point| point.y)
            .fold(f32::INFINITY, f32::min)
    };
    let highest = base
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max);
    let under = -lowest(&base) + 0.001;
    let placed = under + highest - lowest(&top) + 0.002;
    // The base's top face reaches 62% of its half width: within it, the stone stands; well beyond it, it falls.
    for (overhang, stands) in [(0.3, true), (0.5, true), (0.75, false), (0.9, false)] {
        let mut world = world(vec![
            Body::convex(1, base.clone()).at(Placement::at(Vec3::new(0.0, under, 0.0))),
            Body::convex(2, top.clone()).at(Placement::at(Vec3::new(overhang * 0.1, placed, 0.0))),
        ]);
        assert!(world.settle(10.0), "{overhang}");
        let fell = placed - world.placement(2).unwrap().position.y;
        assert_eq!(
            fell < 0.01,
            stands,
            "overhanging by {overhang}, it fell {fell} m"
        );
        assert!(unruled(&mut world, 3.0) < 0.005, "{overhang}");
    }
}

#[test]
fn a_body_sent_home_fades_there_through_whatever_lies_between_and_rests_where_it_began() {
    // Two stones on the ground, one moved off a metre and the other between it and where it began.
    let mut world = world(vec![
        Body::convex(1, pebble(1, 0.2, 0.08)).at(Placement::at(Vec3::new(0.0, 0.05, 0.0))),
        Body::convex(2, pebble(2, 0.2, 0.08)).at(Placement::at(Vec3::new(0.5, 0.05, 0.0))),
    ]);
    assert!(world.settle(10.0));
    let (home, between) = (world.home(1).unwrap(), world.placement(2).unwrap());
    assert!(world.at_home(1, 0.001));
    assert!(!world.at_home(9, 1.0));
    let seen = Seen {
        head: Vec3::new(0.8, 1.2, 0.3),
        hands: [None, Some(Placement::at(Vec3::new(0.3, 0.3, 0.0)))],
    };
    let take = Take {
        body: 1,
        epoch: 0,
        hand: Side::Right,
        offset: Encoded::default(),
    };
    let epoch = world.take(3, &take, Some(&seen)).unwrap();
    // Held, it is not sent anywhere.
    assert_eq!(world.send_home(1), Err(Refused::Held(1)));
    assert_eq!(world.send_home(9), Err(Refused::Unknown(9)));
    let let_go = LetGo {
        body: 1,
        epoch,
        at: Placement::at(Vec3::new(1.0, 0.3, 0.0)).encoded(Precision::Rest),
        velocity: [0.0; 3],
        spin: [0.0; 3],
        clock: 0,
    };
    world.let_go(3, &let_go, Some(&seen)).unwrap();
    until_rest(&mut world, 10.0);
    let away = world.placement(1).unwrap();
    assert!(away.position.x > 0.8);
    assert!(!world.at_home(1, 0.03));
    // Sent home, it is said to be on its way, from where it lies to where it began, its epoch kept; sent again, it goes
    // on its way.
    let (epoch, began) = (world.epoch(1).unwrap(), world.clock());
    world.send_home(1).unwrap();
    world.send_home(1).unwrap();
    assert!(world.moving());
    let said = world.step(0.0);
    assert_eq!(
        said.returning,
        [Returning {
            body: 1,
            from: away.encoded(Precision::Rest),
            to: home.encoded(Precision::Rest),
        }]
    );
    // Whoever joins sees it where it returns to, at rest.
    let shared = world.shared().bodies[0].clone();
    assert_eq!(
        (shared.at, shared.moving),
        (home.encoded(Precision::Rest), false)
    );
    let catching = |at: Vec3| Seen {
        head: Vec3::new(0.8, 1.2, 0.3),
        hands: [None, Some(Placement::at(at))],
    };
    let mut updates = vec![said];
    let mut jumped = None;
    let mut last = world.placement(1).unwrap().position;
    while world.moving() {
        // Fading, it is nobody's to take, where it was or where it returns to.
        let at = world.placement(1).unwrap().position;
        if matches!(world.entries[0].motion, Motion::Returning { .. }) {
            assert_eq!(
                world.take(3, &Take { epoch, ..take }, Some(&catching(at))),
                Err(Refused::Returning(1))
            );
            // Where it was for the fade out, then where it returns to: one jump, at once, and nowhere between.
            if at != last {
                assert!(jumped.is_none(), "it moved twice: {last} to {at}");
                assert!(at.distance(home.position) < 1e-4, "{at}");
                jumped = Some(world.clock());
            }
        }
        last = at;
        updates.push(world.step(1.0 / 60.0));
    }
    let jumped = jumped.unwrap();
    assert!(
        (began + FADE..began + FADE + 20).contains(&jumped),
        "{began} {jumped}"
    );
    assert!(
        updates
            .iter()
            .all(|update| update.moving.iter().all(|snapshot| snapshot
                .placement(Precision::Moving)
                .position
                .distance(home.position)
                < 0.01))
    );
    assert_eq!(world.epoch(1), Some(epoch));
    assert!(world.at_home(1, 0.003));
    assert!(world.placement(1).unwrap().position.distance(home.position) < 0.003);
    // The stone between was never pushed.
    assert!(
        world
            .placement(2)
            .unwrap()
            .position
            .distance(between.position)
            < 0.002
    );
    // Home, it is anyone's to take again.
    let at = world.placement(1).unwrap().position;
    world
        .take(3, &Take { epoch, ..take }, Some(&catching(at)))
        .unwrap();
    assert_eq!(world.held(1).map(|held| held.by), Some(3));
}

/// Body `body` taken by participant 3 from where it lies, let go, unmoving, at `at`, and left to rest.
fn put(world: &mut World, body: u32, at: Vec3) {
    let seen = |hand: Vec3| Seen {
        head: hand + Vec3::new(0.0, 0.6, 0.3),
        hands: [None, Some(Placement::at(hand))],
    };
    let lying = world.placement(body).unwrap().position;
    let take = Take {
        body,
        epoch: world.epoch(body).unwrap(),
        hand: Side::Right,
        offset: Encoded::default(),
    };
    let epoch = world.take(3, &take, Some(&seen(lying))).unwrap();
    let let_go = LetGo {
        body,
        epoch,
        at: Placement::at(at).encoded(Precision::Rest),
        velocity: [0.0; 3],
        spin: [0.0; 3],
        clock: 0,
    };
    world.let_go(3, &let_go, Some(&seen(at))).unwrap();
    until_rest(world, 10.0);
}

/// Body `body` sent home: where it is said to return to, once it rests there.
fn sent_home(world: &mut World, body: u32) -> Placement {
    world.send_home(body).unwrap();
    let to = world.step(0.0).returning[0].to.placement(Precision::Rest);
    until_rest(world, 10.0);
    to
}

#[test]
fn a_body_whose_home_is_taken_returns_to_the_nearest_free_home_of_another() {
    // Three different stones along the ground, the second's home nearer the first's than the third's.
    let stone = |body: u32, x: f32| {
        Body::convex(body, pebble(body, 0.2, 0.08)).at(Placement::at(Vec3::new(x, 0.05, 0.0)))
    };
    let mut world = world(vec![stone(1, 0.5), stone(2, 0.75), stone(3, 0.0)]);
    assert!(world.settle(10.0));
    let homes: Vec<Placement> = (1..=3).map(|body| world.home(body).unwrap()).collect();
    // The first and second carried off, and the third left where the second began.
    put(&mut world, 1, Vec3::new(2.0, 0.1, 0.0));
    put(&mut world, 2, Vec3::new(-2.0, 0.1, 0.0));
    put(&mut world, 3, homes[1].position + Vec3::Y * 0.05);
    // Both sent home at once: the first to its own, free; the second's taken, and the first's, the nearest, being
    // returned to, to the third's, free, in its own turn, its home its own still, as the two are not alike.
    world.send_home(1).unwrap();
    world.send_home(2).unwrap();
    let said = world.step(0.0).returning;
    let to: Vec<Placement> = said
        .iter()
        .map(|returning| returning.to.placement(Precision::Rest))
        .collect();
    assert_eq!(said.len(), 2);
    assert_eq!(to[0], homes[0].snapped(Precision::Rest));
    assert!(
        to[1]
            .position
            .with_y(0.0)
            .distance(homes[2].position.with_y(0.0))
            < 1e-3,
        "{:?}",
        to[1]
    );
    assert!(to[1].rotation.angle_between(homes[1].rotation) < 1e-3);
    until_rest(&mut world, 10.0);
    assert!(world.at_home(1, 0.003));
    assert!(!world.at_home(2, 0.03));
    assert_eq!(world.home(2), Some(homes[1]));
    let lying = world.placement(2).unwrap();
    assert!(
        lying.position.distance(homes[2].position) < 0.03,
        "{lying:?}"
    );
}

#[test]
fn a_body_whose_home_is_taken_by_one_alike_takes_its_home_in_turn() {
    // Two alike, as two sticks on a log are.
    let hull = pebble(1, 0.2, 0.08);
    let mut world = world(vec![
        Body::convex(1, hull.clone()).at(Placement::at(Vec3::new(0.0, 0.05, 0.0))),
        Body::convex(2, hull).at(Placement::at(Vec3::new(0.5, 0.05, 0.0))),
    ]);
    assert!(world.settle(10.0));
    let homes = [world.home(1).unwrap(), world.home(2).unwrap()];
    // The first carried off, and the second left where the first began.
    put(&mut world, 1, Vec3::new(2.0, 0.1, 0.0));
    put(&mut world, 2, homes[0].position + Vec3::Y * 0.05);
    // Sent home, the first takes the second's, and the second the first's, where it lies: both at home.
    assert_eq!(sent_home(&mut world, 1), homes[1].snapped(Precision::Rest));
    assert_eq!(
        (world.home(1), world.home(2)),
        (Some(homes[1]), Some(homes[0]))
    );
    assert!(world.at_home(1, 0.003));
    assert!(world.at_home(2, 0.03));
}

#[test]
fn a_body_with_nowhere_free_to_return_to_is_let_go_on_whatever_lies_at_its_home() {
    let stone = |body: u32, x: f32| {
        Body::convex(body, pebble(body, 0.2, 0.08)).at(Placement::at(Vec3::new(x, 0.05, 0.0)))
    };
    let mut world = world(vec![stone(1, 0.0), stone(2, 0.25)]);
    assert!(world.settle(10.0));
    let home = world.home(1).unwrap();
    // The first carried off, and the second left across both their homes.
    put(&mut world, 1, Vec3::new(2.0, 0.1, 0.0));
    put(&mut world, 2, Vec3::new(0.125, 0.1, 0.0));
    assert_eq!(sent_home(&mut world, 1), home.snapped(Precision::Rest));
    // On the second, rather than within it.
    let lying = world.placement(1).unwrap();
    assert!(lying.position.y > home.position.y + 0.01, "{lying:?}");
    assert!(!world.occupied(0, &lying));
}

#[test]
fn a_body_lighter_than_water_floats_on_it_and_comes_to_rest_there() {
    // A pool a metre deep, and a plank of wood, a little over half as dense as water, let go above it.
    let bed = Static::cuboid(
        Vec3::new(-3.0, -2.0, -3.0),
        Vec3::new(3.0, -1.0, 3.0),
        Quat::IDENTITY,
    );
    let outline = vec![
        Vec2::new(-3.0, -3.0),
        Vec2::new(3.0, -3.0),
        Vec2::new(3.0, 3.0),
        Vec2::new(-3.0, 3.0),
    ];
    let plank: Vec<Vec3> = (0..8)
        .map(|corner| {
            let sign = |bit: u32| if corner & bit == 0 { -1.0 } else { 1.0 };
            Vec3::new(sign(1) * 0.3, sign(2) * 0.02, sign(4) * 0.05)
        })
        .collect();
    let mut world = World::new(
        Settings::default(),
        vec![bed],
        vec![Water::below(0.0, outline)],
        vec![
            Body::convex(1, plank)
                .with_density(600.0)
                .at(Placement::at(Vec3::new(0.0, 0.5, 0.0))),
        ],
    )
    .unwrap();
    world.settle(20.0);
    assert!(!world.moving());
    // It floats as deep as it is lighter than the water: 60% of its 4 cm below the surface, its middle 4 mm under.
    let floating = world.placement(1).unwrap();
    assert!((floating.position.y + 0.004).abs() < 0.004, "{floating:?}");
}

#[test]
fn a_body_floating_with_its_middle_above_the_water_soaks_in_it_and_goes_home_but_a_stone_beside_it_does_not()
 {
    // A pool a metre deep, which takes back what rests in it after a second; a plank of wood hanging 10 cm below its
    // middle, as a stick's middle is above the water when it floats with its marshmallow down; and a stone on a ledge
    // beside the pool, its foot in the water and its middle out of it.
    let bed = Static::cuboid(
        Vec3::new(-3.0, -2.0, -3.0),
        Vec3::new(3.0, -1.0, 3.0),
        Quat::IDENTITY,
    );
    let ledge = Static::cuboid(
        Vec3::new(1.0, -1.0, -0.5),
        Vec3::new(2.0, -0.02, 0.5),
        Quat::IDENTITY,
    );
    let outline = vec![
        Vec2::new(-3.0, -3.0),
        Vec2::new(3.0, -3.0),
        Vec2::new(3.0, 3.0),
        Vec2::new(-3.0, 3.0),
    ];
    let plank: Vec<Vec3> = (0..8)
        .map(|corner| {
            let sign = |bit: u32| if corner & bit == 0 { -1.0 } else { 1.0 };
            Vec3::new(sign(1) * 0.3, sign(2) * 0.02 - 0.1, sign(4) * 0.05)
        })
        .collect();
    let home = Placement::at(Vec3::new(-1.0, 0.5, 0.0));
    let stone = Placement::at(Vec3::new(1.5, 0.08, 0.0));
    let pebble_body = Body::convex(2, pebble(2, 0.1, 0.06)).at(stone);
    let mut world = World::new(
        Settings::default(),
        vec![bed, ledge],
        vec![Water::below(0.0, outline).returning(1_000)],
        vec![
            Body::convex(1, plank).with_density(600.0).at(home),
            pebble_body.clone(),
        ],
    )
    .unwrap();
    world.settle(5.0);
    let lying = world.placement(2).unwrap();
    assert!(
        lying.position.y > 0.0 && pebble_body.lowest(&lying) < 0.0,
        "{lying:?}"
    );
    let let_go = LetGo {
        body: 1,
        epoch: 0,
        at: Placement::at(Vec3::new(-1.0, 0.3, 0.0)).encoded(Precision::Rest),
        velocity: [0.0; 3],
        spin: [0.0; 3],
        clock: 0,
    };
    let seen = Seen {
        head: Vec3::new(-1.0, 1.0, 0.5),
        hands: [None, Some(Placement::at(Vec3::new(-1.0, 0.3, 0.0)))],
    };
    let take = Take {
        body: 1,
        epoch: 0,
        hand: Side::Right,
        offset: Encoded::default(),
    };
    let epoch = world.take(3, &take, Some(&seen)).unwrap();
    world
        .let_go(3, &LetGo { epoch, ..let_go }, Some(&seen))
        .unwrap();
    // It floats, its middle above the surface, and rests there, soaking, and then goes home.
    let mut rested = None;
    for _ in 0..(60 * 10) {
        let update = world.step(1.0 / 60.0);
        if update.rested.iter().any(|snapshot| snapshot.body() == 1) {
            rested = world.placement(1);
            break;
        }
    }
    let rested = rested.expect("it rests afloat");
    assert!(rested.position.y > 0.05, "{rested:?}");
    assert!(world.moving());
    let (updates, _) = until_rest(&mut world, 10.0);
    assert!(updates.iter().any(|update| !update.returning.is_empty()));
    assert!(
        world
            .placement(1)
            .unwrap()
            .position
            .distance(world.home(1).unwrap().position)
            < 0.03
    );
    // The stone, its foot in the water and its middle out, stays.
    assert_eq!(world.placement(2).unwrap(), lying);
}

#[test]
fn a_grabbable_body_sent_home_fades_there_and_rests_without_falling() {
    let hook = Body::convex(1, pebble(1, 0.1, 0.04))
        .kinematic()
        .at(Placement::at(Vec3::new(0.0, 1.5, 0.0)));
    let mut world = world(vec![hook]);
    let seen = Seen {
        head: Vec3::new(0.0, 1.6, 0.5),
        hands: [None, Some(Placement::at(Vec3::new(0.0, 1.4, 0.0)))],
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
        at: Placement::at(Vec3::new(0.5, 1.2, 0.0)).encoded(Precision::Rest),
        velocity: [0.0; 3],
        spin: [0.0; 3],
        clock: 0,
    };
    world.let_go(3, &let_go, Some(&seen)).unwrap();
    assert!(!world.moving());
    world.send_home(1).unwrap();
    let (updates, _) = until_rest(&mut world, 5.0);
    // Back where it began, in the air, said once at rest there.
    assert!(world.at_home(1, 0.001));
    assert!(updates.iter().any(|update| !update.rested.is_empty()));
    assert!(
        world
            .placement(1)
            .unwrap()
            .position
            .distance(Vec3::new(0.0, 1.5, 0.0))
            < 0.001
    );
}

/// A dressed stone's hull, as one split along its bedding and roughly squared, for building: a box `width` along X,
/// `depth` along Z and `height` high, its edges bevelled and its corners nudged by `seed`, its bottom a face of four,
/// flat, and its top a face of four, level, with a ring of eight about its middle.
fn dressed(seed: u32, width: f32, depth: f32, height: f32) -> Vec<Vec3> {
    let nudge = |index: u32| pebble(seed, 1.0, 1.0)[(index % 14) as usize].x.abs() * 0.1;
    let (x, z) = (width * 0.5, depth * 0.5);
    let corners = [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)];
    let mut points = Vec::new();
    for level in [-0.5, 0.5] {
        for (across, along) in corners {
            points.push(Vec3::new(
                across * x * 0.88,
                level * height,
                along * z * 0.88,
            ));
        }
    }
    for (corner, (across, along)) in corners.into_iter().enumerate() {
        for (side, (wide, deep)) in [(1.0, 0.76), (0.76, 1.0)].into_iter().enumerate() {
            let at = corner as u32 * 2 + side as u32;
            points.push(Vec3::new(
                across * x * wide * (1.0 - nudge(at) * 0.5),
                (nudge(at + 7) - 0.025) * height,
                along * z * deep * (1.0 - nudge(at + 3) * 0.5),
            ));
        }
    }
    points
}

/// One stone of a structure: its hull, where its middle stands across the ground, turned how far about the upright, and
/// which of those before it it rests on, or, none, the ground.
struct Part {
    hull: Vec<Vec3>,
    at: Vec2,
    turn: f32,
    on: Vec<usize>,
}

fn part(hull: Vec<Vec3>, at: Vec2, turn: f32, on: &[usize]) -> Part {
    Part {
        hull,
        at,
        turn,
        on: on.to_vec(),
    }
}

/// A block, 38 by 30 cm and 34 high, a pillar's or a wall's; and a flagstone, 72 by 48 cm and 12 thick, a lintel's.
fn block(seed: u32) -> Vec<Vec3> {
    dressed(seed, 0.38, 0.3, 0.34)
}

fn flagstone(seed: u32) -> Vec<Vec3> {
    dressed(seed, 0.72, 0.48, 0.12)
}

/// `parts` built on `ground`, each placed a couple of millimetres above what it rests on, in a world of them simulated
/// by `settings` and settled for at most ten seconds; and where each was placed.
fn built(settings: Settings, ground: Static, parts: &[Part]) -> (World, Vec<Placement>) {
    let mut placed: Vec<Placement> = Vec::new();
    let span = |hull: &[Vec3], at: &Placement| {
        hull.iter()
            .map(|point| at.transform_point(*point).y)
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), y| {
                (low.min(y), high.max(y))
            })
    };
    for part in parts {
        let mut at = Placement::new(
            Vec3::new(part.at.x, 0.0, part.at.y),
            Quat::from_rotation_y(part.turn),
        );
        let beneath = part
            .on
            .iter()
            .map(|below| span(&parts[*below].hull, &placed[*below]).1)
            .fold(ground.lowest().max(0.0), f32::max);
        at.position.y = beneath + 0.002 - span(&part.hull, &at).0;
        placed.push(at);
    }
    let bodies = parts
        .iter()
        .zip(&placed)
        .enumerate()
        .map(|(number, (part, at))| Body::convex(number as u32, part.hull.clone()).at(*at))
        .collect();
    let mut world = World::new(settings, vec![ground], Vec::new(), bodies).unwrap();
    world.settle(10.0);
    (world, placed)
}

#[test]
fn structures_of_blocks_and_flagstones_stand_and_sleep_and_the_stillness_rule_holds_none_up() {
    let (left, right) = (Vec2::new(-0.24, 0.0), Vec2::new(0.24, 0.0));
    let quarter = std::f32::consts::FRAC_PI_2;
    let (cairn, _) = cairn(4, Vec2::ZERO, 0);
    let mut on_a_cairn: Vec<Part> = cairn
        .into_iter()
        .enumerate()
        .map(|(level, body)| {
            let on: Vec<usize> = level.checked_sub(1).into_iter().collect();
            part(body.hull, Vec2::ZERO, level as f32 * 0.9, &on)
        })
        .collect();
    on_a_cairn.push(part(dressed(20, 0.5, 0.36, 0.09), Vec2::ZERO, 0.3, &[3]));
    let structures = [
        // Two pillars and a lintel across them.
        (
            "dolmen",
            vec![
                part(block(1), left, quarter, &[]),
                part(block(2), right, quarter, &[]),
                part(flagstone(3), Vec2::ZERO, 0.0, &[0, 1]),
            ],
        ),
        ("flagstone on a cairn", on_a_cairn),
        // Three blocks in a row and a coping stone along them.
        (
            "wall",
            vec![
                part(block(4), Vec2::ZERO, 0.0, &[]),
                part(block(5), Vec2::new(-0.39, 0.0), 0.0, &[]),
                part(block(6), Vec2::new(0.39, 0.0), 0.0, &[]),
                part(flagstone(7), Vec2::ZERO, 0.0, &[0, 1, 2]),
            ],
        ),
        // Three blocks on end, each turned across the one below, and a flagstone on top.
        (
            "tower",
            vec![
                part(block(8), Vec2::ZERO, 0.0, &[]),
                part(block(9), Vec2::ZERO, quarter, &[0]),
                part(block(10), Vec2::ZERO, 0.1, &[1]),
                part(flagstone(11), Vec2::ZERO, quarter + 0.2, &[2]),
            ],
        ),
    ];
    // As Rapier solves contacts by default, and each against its own triangle, as the farmyard's beach does.
    for ((name, parts), settings) in structures.iter().flat_map(|structure| {
        [
            (structure, Settings::default()),
            (structure, Settings::default().unclustered()),
        ]
    }) {
        let (mut world, placed) = built(settings, ground(), parts);
        assert!(!world.moving(), "the {name} never slept");
        assert!(
            world.clock() <= 5_000,
            "the {name} took {} ms to sleep",
            world.clock()
        );
        // Standing: none crept a centimetre and a half, nor sank onto what it rests on more than the gap it was placed
        // with and a little more, as contacts settle under a heavy stone.
        let rested = placements(&world);
        let sank: Vec<f32> = placed
            .iter()
            .zip(&rested)
            .map(|(placed, rested)| placed.position.y - rested.position.y)
            .collect();
        for (number, (placed, rested)) in placed.iter().zip(&rested).enumerate() {
            let crept = (rested.position - placed.position).with_y(0.0).length();
            let beneath = parts[number]
                .on
                .iter()
                .map(|below| sank[*below])
                .fold(0.0, f32::max);
            let lower = sank[number] - beneath;
            assert!(crept < 0.015, "the {name}'s stone {number} crept {crept} m");
            assert!(lower < 0.01, "the {name}'s stone {number} sank {lower} m");
        }
        for _ in 0..(600 * 4) {
            assert!(world.step(0.25).is_empty(), "the {name} woke");
        }
        assert_eq!(placements(&world), rested, "the {name} moved asleep");
        // Woken and left to Rapier alone, it stands as it was put to sleep: nothing the rule stopped was falling.
        let moved = unruled(&mut world, 3.0);
        assert!(moved < 0.005, "the {name} moved {moved} m without the rule");
    }
}

#[test]
fn a_lintel_beyond_its_one_pillar_falls_rather_than_being_put_to_sleep() {
    // On the pillar's top, within it, it stands; with its middle beyond the pillar's edge, it tips off.
    for (beyond, stands) in [(0.0, true), (0.12, true), (0.2, false), (0.3, false)] {
        let parts = [
            part(block(1), Vec2::ZERO, 0.0, &[]),
            part(flagstone(2), Vec2::new(beyond, 0.0), 0.0, &[0]),
        ];
        let (mut world, placed) = built(Settings::default().unclustered(), ground(), &parts);
        assert!(!world.moving(), "{beyond}");
        let fell = placed[1].position.y - world.placement(1).unwrap().position.y;
        assert_eq!(fell < 0.01, stands, "{beyond}: it fell {fell} m");
        assert!(unruled(&mut world, 3.0) < 0.005, "{beyond}");
    }
}

#[test]
fn a_broad_flat_stone_on_ground_bent_a_degree_or_two_settles_rather_than_creeping() {
    // A mesh of quarter-metre cells, flat and then sloping away more steeply, as a shingle beach does where it leaves
    // the meadow: a block or a flagstone lying across a bend rests, solving each contact against its own triangle, rather
    // than creeping a few millimetres a second for ever, as it does when Rapier solves those within 5° as one plane.
    let profile = |x: f32| {
        let bends = [
            (-2.0, 0.015),
            (0.0, 0.0128),
            (0.25, 0.008),
            (0.5, -0.0064),
            (0.75, -0.0252),
            (4.0, -0.4),
        ];
        let index = bends
            .iter()
            .position(|(at, _)| x <= *at)
            .unwrap_or(bends.len() - 1)
            .max(1);
        let ((a, low), (b, high)) = (bends[index - 1], bends[index]);
        low + (high - low) * ((x - a) / (b - a)).clamp(0.0, 1.0)
    };
    let columns = 17;
    let heights: Vec<f32> = (0..columns * columns)
        .map(|index| profile(-2.0 + (index % columns) as f32 * 0.25))
        .collect();
    let bent = Static::heights(Vec2::splat(-2.0), 0.25, columns, &heights);
    for step in 0..12 {
        let at = Vec2::new(-0.1 + step as f32 * 0.05, step as f32 * 0.07 - 0.4);
        let hull = if step % 2 == 0 {
            block(step)
        } else {
            flagstone(step)
        };
        let mut world = World::new(
            Settings::default().unclustered(),
            vec![bent.clone()],
            Vec::new(),
            vec![Body::convex(1, hull.clone()).at(Placement::new(
                Vec3::new(
                    at.x,
                    profile(at.x - 0.36).max(profile(at.x + 0.36)) + 0.2,
                    at.y,
                ),
                Quat::from_rotation_y(step as f32 * 0.4),
            ))],
        )
        .unwrap();
        assert!(world.settle(10.0), "{at}");
        assert!(world.clock() <= 3_000, "{at}: {} ms", world.clock());
    }
}
