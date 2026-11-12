// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use glam::Quat;
use tomori_physics::message::{Holding as Changed, Landed, Shared, Snapshot};
use tomori_physics::{Encoded, Held};

/// Where stone 0 lies, on the beach's flat shingle, and stone 1 a little along from it.
fn lying(stone: u32) -> Placement {
    let point = beach::at(beach::NEAR + 0.5, stone as f32 * 0.6);
    let (_, height) = beach::size(stone as usize);
    Placement::new(
        Vec3::new(
            point.x,
            beach::height(point).unwrap() + height * 0.5,
            point.y,
        ),
        Quat::from_rotation_y(0.4),
    )
    .snapped(Precision::Rest)
}

fn state() -> State {
    State {
        bodies: (0..2)
            .map(|body| Shared {
                body,
                epoch: 3,
                at: lying(body).encoded(Precision::Rest),
                moving: false,
                held: None,
            })
            .collect(),
    }
}

fn shared() -> Stones {
    let mut stones = Stones::new();
    stones.share(&state());
    stones
}

fn hand_at(position: Vec3) -> Hand {
    Hand::new(position, Quat::from_rotation_y(0.4))
}

fn grab(stone: u32, hand: Hand, phase: Phase, side: Side) -> Grab {
    Grab::new(format!("stone-{stone}"), hand, phase).of(side)
}

#[test]
fn a_stone_is_pointed_at_where_its_faces_are_and_taken_within_reach_of_a_hand() {
    let stones = shared();
    let at = lying(0).position;
    let down = Ray::new(at + Vec3::Y * 1.5, Vec3::NEG_Y).unwrap();
    let picked = stones.pick(&down).unwrap();
    assert_eq!(picked.object, "stone-0");
    assert!(picked.actions.contains(Actions::GRAB));
    // Beyond the pull, not.
    assert_eq!(
        stones.pick(&Ray::new(at + Vec3::Y * 4.0, Vec3::NEG_Y).unwrap()),
        None
    );
    assert_eq!(stones.within_reach(at), Some("stone-0".to_owned()));
    assert_eq!(stones.within_reach(at + Vec3::Y * 0.3), None);
    // Of two, the nearer: along the ray, and to the hand.
    let other = lying(1).position;
    let along = Ray::new(
        at - (other - at).normalize() * 0.5,
        (other - at).normalize(),
    )
    .unwrap();
    assert_eq!(stones.pick(&along).unwrap().object, "stone-0");
    let mut close = Stones::new();
    let mut near = state();
    near.bodies[1].at =
        Placement::new(at + Vec3::new(0.12, 0.0, 0.0), Quat::IDENTITY).encoded(Precision::Rest);
    close.share(&near);
    assert_eq!(
        close.within_reach(at + Vec3::new(0.02, 0.0, 0.0)),
        Some("stone-0".to_owned())
    );
    assert_eq!(
        close.within_reach(at + Vec3::new(0.1, 0.0, 0.0)),
        Some("stone-1".to_owned())
    );
}

#[test]
fn a_stone_taken_within_reach_is_held_where_the_hand_closed_on_it_and_thrown_as_fast_as_the_hand_moved()
 {
    let mut stones = shared();
    let start = Instant::now();
    let at = lying(0).position;
    let hand = hand_at(at + Vec3::new(0.0, 0.03, 0.0));
    let intents = stones.update(
        [&grab(0, hand, Phase::Began, Side::Left)].into_iter(),
        Some(5),
        start,
        1_000,
    );
    let Intent::Take(take) = intents[0] else {
        unreachable!()
    };
    assert_eq!(
        (take.body, take.epoch, take.hand),
        (0, 3, tomori_physics::Side::Left)
    );
    let offset = take.offset.placement(Precision::Rest);
    assert!(placed(&hand).then(&offset).position.distance(at) < 1e-3);
    // Held, it is drawn in the hand, as it moves, and is no longer pointed at or within reach.
    let shown = stones.shown(&[], &[], start, 1_000);
    assert!(shown[0].transform.w_axis.truncate().distance(at) < 1e-3);
    assert_eq!(stones.within_reach(at), None);
    // A second taking in the same hand, or of a stone held, takes nothing.
    assert!(
        stones
            .update(
                [&grab(1, hand, Phase::Began, Side::Left)].into_iter(),
                Some(5),
                start,
                1_000
            )
            .is_empty()
    );
    assert!(
        stones
            .update(
                [&grab(0, hand, Phase::Began, Side::Right)].into_iter(),
                Some(5),
                start,
                1_000
            )
            .is_empty()
    );
    // Moving the hand up and out at 2 m/s, it is let go moving so, and foreseen as it flies.
    let mut now = start;
    for step in 1..=6 {
        now += Duration::from_millis(20);
        let moved = hand_at(hand.position + Vec3::new(0.04, 0.0, 0.0) * step as f32);
        stones.update(
            [&grab(0, moved, Phase::Changed, Side::Left)].into_iter(),
            Some(5),
            now,
            1_000,
        );
    }
    let moved = hand_at(hand.position + Vec3::new(0.28, 0.0, 0.0));
    now += Duration::from_millis(20);
    let intents = stones.update(
        [&grab(0, moved, Phase::Ended, Side::Left)].into_iter(),
        Some(5),
        now,
        1_200,
    );
    let Intent::LetGo(let_go) = intents[0] else {
        unreachable!()
    };
    assert_eq!((let_go.body, let_go.epoch, let_go.clock), (0, 4, 1_200));
    let speed = Vec3::from(let_go.velocity);
    assert!(
        (speed.x - 2.0).abs() < 0.3 && speed.y.abs() < 0.1,
        "{speed}"
    );
    let flying = stones.shown(&[], &[], now, 1_300);
    assert!(flying[0].transform.w_axis.x > moved.position.x + 0.1);
}

#[test]
fn a_stone_pulled_along_the_ray_flies_to_the_hand_and_is_held_at_the_palm() {
    let mut stones = shared();
    let start = Instant::now();
    let at = lying(1).position;
    let hand = hand_at(at + Vec3::new(-1.2, 1.0, 0.0));
    let intents = stones.update(
        [&grab(1, hand, Phase::Began, Side::Right)].into_iter(),
        Some(5),
        start,
        0,
    );
    let Intent::Take(take) = intents[0] else {
        unreachable!()
    };
    let palm = take.offset.placement(Precision::Rest).position;
    assert!(palm.z < -0.03 && palm.length() < 0.2, "{palm}");
    // Where it lay, then half way, then in the hand.
    let drawn = |stones: &mut Stones, now| {
        stones.shown(&[], &[], now, 0)[1]
            .transform
            .w_axis
            .truncate()
    };
    assert!(drawn(&mut stones, start).distance(at) < 1e-3);
    let half = drawn(
        &mut stones,
        start + Duration::from_secs_f32(PULL_TIME * 0.5),
    );
    assert!(half.distance(at) > 0.3 && half.distance(hand.position) > 0.3);
    let held = drawn(&mut stones, start + Duration::from_secs_f32(PULL_TIME));
    assert!(held.distance(hand.position) < 0.2);
}

#[test]
fn a_taking_the_server_side_agrees_to_is_kept_and_one_another_won_or_that_went_unanswered_is_given_up()
 {
    let start = Instant::now();
    let at = lying(0).position;
    let hand = hand_at(at);
    let took = |holder: u32, epoch: u32| Update {
        holding: vec![Changed {
            body: 0,
            epoch,
            held: Some(Held {
                by: holder,
                hand: tomori_physics::Side::Right,
                offset: Encoded::default(),
            }),
        }],
        ..Default::default()
    };
    // Agreed, it is kept long after.
    let mut stones = shared();
    stones.update(
        [&grab(0, hand, Phase::Began, Side::Right)].into_iter(),
        Some(5),
        start,
        0,
    );
    stones.changed(10, &took(5, 4), 20);
    let later = start + UNAGREED * 2;
    stones.update(std::iter::empty(), Some(5), later, 0);
    assert!(stones.holding[1].is_some());
    // Another took it first: it is theirs.
    let mut stones = shared();
    stones.update(
        [&grab(0, hand, Phase::Began, Side::Right)].into_iter(),
        Some(5),
        start,
        0,
    );
    stones.changed(10, &took(7, 4), 20);
    assert!(stones.holding[1].is_none());
    // Unanswered, it is given up.
    let mut stones = shared();
    stones.update(
        [&grab(0, hand, Phase::Began, Side::Right)].into_iter(),
        Some(5),
        start,
        0,
    );
    stones.update(std::iter::empty(), Some(5), later, 0);
    assert!(stones.holding[1].is_none());
    // A visitor, a grip on something else or on a stone nobody knows, takes nothing.
    let mut stones = shared();
    assert!(
        stones
            .update(
                [&grab(0, hand, Phase::Began, Side::Right)].into_iter(),
                None,
                start,
                0
            )
            .is_empty()
    );
    let stick = Grab::new("stick-1", hand, Phase::Began);
    assert!(
        stones
            .update(
                [&stick, &grab(9, hand, Phase::Began, Side::Right)].into_iter(),
                Some(5),
                start,
                0
            )
            .is_empty()
    );
    // A grip's changes and end for a stone it does not hold change nothing.
    assert!(
        stones
            .update(
                [
                    &grab(0, hand, Phase::Changed, Side::Right),
                    &grab(0, hand, Phase::Ended, Side::Right)
                ]
                .into_iter(),
                Some(5),
                start,
                0
            )
            .is_empty()
    );
}

#[test]
fn a_stone_another_holds_is_drawn_in_their_hand_or_where_it_was_last() {
    let mut stones = shared();
    let held = |hand| Update {
        holding: vec![Changed {
            body: 1,
            epoch: 4,
            held: Some(Held {
                by: 7,
                hand,
                offset: Placement::at(Vec3::new(0.0, 0.0, -0.05)).encoded(Precision::Rest),
            }),
        }],
        ..Default::default()
    };
    stones.changed(10, &held(tomori_physics::Side::Left), 20);
    let theirs = Hand::new(Vec3::new(10.5, 1.0, -5.0), Quat::IDENTITY);
    let shown = stones.shown(&[], &[(7, theirs)], Instant::now(), 30);
    assert!(
        shown[1]
            .transform
            .w_axis
            .truncate()
            .distance(Vec3::new(10.5, 1.0, -5.05))
            < 1e-3
    );
    // Their hand unreported, it stays where it was drawn.
    let again = stones.shown(&[], &[], Instant::now(), 40);
    assert_eq!(again[1], shown[1]);
    // In their right hand, from their right hand.
    stones.changed(50, &held(tomori_physics::Side::Right), 60);
    let right = Hand::new(Vec3::new(10.0, 1.2, -5.0), Quat::IDENTITY);
    let shown = stones.shown(&[(7, right)], &[], Instant::now(), 70);
    assert!(
        shown[1]
            .transform
            .w_axis
            .truncate()
            .distance(Vec3::new(10.0, 1.2, -5.05))
            < 1e-3
    );
    // Never drawn inside the shingle, though held there.
    let low = Hand::new(lying(0).position - Vec3::Y * 0.2, Quat::IDENTITY);
    let shown = stones.shown(&[(7, low)], &[], Instant::now(), 80);
    let point = shown[1].transform.w_axis.truncate();
    assert!(point.y > beach::height(Vec2::new(point.x, point.z)).unwrap());
    // Nothing known, nothing drawn.
    let mut unknown = Stones::new();
    unknown.changed(10, &held(tomori_physics::Side::Left), 20);
    assert!(
        unknown
            .shown(&[], &[(7, theirs)], Instant::now(), 30)
            .is_empty()
    );
}

#[test]
fn a_stone_that_lands_is_heard_a_clack_on_the_shingle_or_a_splash_in_the_water_louder_the_harder() {
    let mut stones = shared();
    let landed = |body, speed, water| Landed { body, speed, water };
    let update = Update {
        moving: vec![Snapshot::new(0, &lying(0), Precision::Moving)],
        landed: vec![
            landed(0, 3.0, false),
            landed(1, 2.0, true),
            landed(1, 0.1, false),
            landed(9, 3.0, false),
        ],
        ..Default::default()
    };
    let heard = stones.changed(10, &update, 20);
    assert_eq!(heard.len(), 2);
    let (clacked, splashed) = (&heard[0].0, &heard[1].0);
    assert_eq!(clacked.samples.len(), clack().len());
    assert_eq!(splashed.samples.len(), splash().len());
    assert!(splashed.samples.len() > clacked.samples.len());
    let peak = |sound: &Sound| {
        sound
            .samples
            .iter()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
    };
    assert!(peak(&sound(false, 4.0)) > peak(&sound(false, 0.5)) * 2.0);
    assert!(peak(clacked) > 0.0 && peak(clacked) <= 1.0);
}

#[test]
fn a_stone_let_go_unmoving_is_let_go_still() {
    let mut stones = shared();
    let start = Instant::now();
    let hand = hand_at(lying(0).position);
    stones.update(
        [&grab(0, hand, Phase::Began, Side::Right)].into_iter(),
        Some(5),
        start,
        0,
    );
    let intents = stones.update(
        [&grab(0, hand, Phase::Ended, Side::Right)].into_iter(),
        Some(5),
        start,
        0,
    );
    let Intent::LetGo(let_go) = intents[0] else {
        unreachable!()
    };
    assert_eq!((let_go.velocity, let_go.spin), ([0.0; 3], [0.0; 3]));
}

#[test]
fn a_stone_the_server_side_says_is_held_by_no_hand_of_this_participant_is_let_go_of_again() {
    let start = Instant::now();
    let at = lying(0).position;
    let hand = hand_at(at);
    let mut stones = shared();
    stones.update(
        [&grab(0, hand, Phase::Began, Side::Right)].into_iter(),
        Some(5),
        start,
        0,
    );
    stones.changed(
        10,
        &Update {
            holding: vec![Changed {
                body: 0,
                epoch: 4,
                held: Some(Held {
                    by: 5,
                    hand: tomori_physics::Side::Right,
                    offset: Encoded::default(),
                }),
            }],
            ..Default::default()
        },
        20,
    );
    // Let go, but the letting go never arrives, as when it was one intent too many: the server side still says they
    // hold it.
    let intents = stones.update(
        [&grab(0, hand, Phase::Ended, Side::Right)].into_iter(),
        Some(5),
        start,
        100,
    );
    assert!(matches!(intents[..], [Intent::LetGo(_)]));
    let lets_go = |intents: &[Intent]| {
        intents
            .iter()
            .filter_map(|intent| match intent {
                Intent::LetGo(let_go) => Some((let_go.body, let_go.epoch, let_go.velocity)),
                Intent::Take(_) => None,
            })
            .collect::<Vec<_>>()
    };
    // Not at once, while its own letting go may yet be answered; then, unmoving, at the epoch the server side says; and
    // again only a while after.
    let soon = start + ASTRAY / 2;
    assert!(
        stones
            .update(std::iter::empty(), Some(5), soon, 200)
            .is_empty()
    );
    let astray = start + ASTRAY;
    assert_eq!(
        lets_go(&stones.update(std::iter::empty(), Some(5), astray, 300)),
        [(0, 4, [0.0; 3])]
    );
    assert!(
        stones
            .update(std::iter::empty(), Some(5), astray + ASTRAY / 2, 400)
            .is_empty()
    );
    assert_eq!(
        lets_go(&stones.update(std::iter::empty(), Some(5), astray + ASTRAY, 500)).len(),
        1
    );
    // Once the server side says it is let go, nothing more.
    stones.changed(
        600,
        &Update {
            holding: vec![Changed {
                body: 0,
                epoch: 5,
                held: None,
            }],
            ..Default::default()
        },
        610,
    );
    assert!(
        stones
            .update(std::iter::empty(), Some(5), astray + ASTRAY * 3, 700)
            .is_empty()
    );
    // Taken, but agreed only after this client gave the taking up: let go of again, at the epoch it was agreed at.
    let mut late = shared();
    late.update(
        [&grab(0, hand, Phase::Began, Side::Right)].into_iter(),
        Some(5),
        start,
        0,
    );
    let given_up = start + UNAGREED;
    late.update(std::iter::empty(), Some(5), given_up, 0);
    late.changed(
        10,
        &Update {
            holding: vec![Changed {
                body: 0,
                epoch: 4,
                held: Some(Held {
                    by: 5,
                    hand: tomori_physics::Side::Right,
                    offset: Encoded::default(),
                }),
            }],
            ..Default::default()
        },
        20,
    );
    assert!(
        late.update(std::iter::empty(), Some(5), given_up, 0)
            .is_empty()
    );
    assert_eq!(
        lets_go(&late.update(std::iter::empty(), Some(5), given_up + ASTRAY, 0)),
        [(0, 4, [0.0; 3])]
    );
    // Someone else's stone is never theirs to let go of.
    let mut others = shared();
    others.changed(
        10,
        &Update {
            holding: vec![Changed {
                body: 1,
                epoch: 4,
                held: Some(Held {
                    by: 7,
                    hand: tomori_physics::Side::Left,
                    offset: Encoded::default(),
                }),
            }],
            ..Default::default()
        },
        20,
    );
    others.update(std::iter::empty(), Some(5), start, 0);
    assert!(
        others
            .update(std::iter::empty(), Some(5), start + ASTRAY * 2, 0)
            .is_empty()
    );
}

#[test]
fn a_stone_taken_again_before_its_letting_go_is_answered_is_taken_at_the_epoch_its_letting_go_makes()
 {
    let start = Instant::now();
    let at = lying(0).position;
    let hand = hand_at(at + Vec3::Y * 0.03);
    let said = |epoch: u32, by: Option<u32>| Update {
        holding: vec![Changed {
            body: 0,
            epoch,
            held: by.map(|by| Held {
                by,
                hand: tomori_physics::Side::Right,
                offset: Encoded::default(),
            }),
        }],
        ..Default::default()
    };
    let taken = |intents: &[Intent]| match intents {
        [Intent::Take(take)] => take.epoch,
        _ => panic!("{intents:?}"),
    };
    let mut stones = shared();
    let grip = |stones: &mut Stones, phase: Phase, now: Instant| {
        stones.update(
            [&grab(0, hand, phase, Side::Right)].into_iter(),
            Some(5),
            now,
            0,
        )
    };
    assert_eq!(taken(&grip(&mut stones, Phase::Began, start)), 3);
    stones.changed(10, &said(4, Some(5)), 20);
    assert!(matches!(
        grip(&mut stones, Phase::Ended, start)[..],
        [Intent::LetGo(LetGo { epoch: 4, .. })]
    ));
    // Squeezed on again before the side has said it was let go: taken at the epoch the letting go makes.
    assert_eq!(stones.within_reach(at), Some("stone-0".to_owned()));
    let soon = start + Duration::from_millis(100);
    assert_eq!(taken(&grip(&mut stones, Phase::Began, soon)), 5);
    // Its letting go said, the taking again waits; then agreed, it is kept, and never let go of again unasked.
    stones.changed(30, &said(5, None), 40);
    assert!(stones.holding[1].is_some());
    stones.changed(40, &said(6, Some(5)), 50);
    assert!(stones.holding[1].as_ref().is_some_and(|held| held.agreed));
    let later = start + ASTRAY * 3;
    assert!(
        stones
            .update(std::iter::empty(), Some(5), later, 0)
            .is_empty()
    );
    assert!(stones.holding[1].is_some());
}

#[test]
fn a_stone_the_server_side_says_this_participant_holds_with_no_hand_of_theirs_is_theirs_to_take_again()
 {
    let start = Instant::now();
    let at = lying(0).position;
    let hand = hand_at(at + Vec3::Y * 0.03);
    // As when their letting go was lost, the side says they hold it; another's is not theirs to take.
    let mut held = state();
    held.bodies[0].epoch = 4;
    held.bodies[0].held = Some(Held {
        by: 5,
        hand: tomori_physics::Side::Right,
        offset: Encoded::default(),
    });
    held.bodies[1].held = Some(Held {
        by: 7,
        hand: tomori_physics::Side::Right,
        offset: Encoded::default(),
    });
    let mut stones = Stones::new();
    stones.share(&held);
    stones.update(std::iter::empty(), Some(5), start, 0);
    stones.shown(&[], &[], start, 0);
    assert_eq!(stones.within_reach(at), Some("stone-0".to_owned()));
    assert_eq!(stones.within_reach(lying(1).position), None);
    let down = Ray::new(at + Vec3::Y, Vec3::NEG_Y).unwrap();
    assert_eq!(stones.pick(&down).unwrap().object, "stone-0");
    // Taken with the left hand, at the epoch its letting go would make, and agreed in that hand.
    let intents = stones.update(
        [&grab(0, hand, Phase::Began, Side::Left)].into_iter(),
        Some(5),
        start,
        0,
    );
    let [Intent::Take(take)] = intents[..] else {
        panic!("{intents:?}")
    };
    assert_eq!((take.epoch, take.hand), (5, tomori_physics::Side::Left));
    stones.changed(
        10,
        &Update {
            holding: vec![Changed {
                body: 0,
                epoch: 6,
                held: Some(Held {
                    by: 5,
                    hand: tomori_physics::Side::Left,
                    offset: take.offset,
                }),
            }],
            ..Default::default()
        },
        20,
    );
    assert!(stones.holding[0].as_ref().is_some_and(|held| held.agreed));
}

#[test]
fn a_big_stone_pulled_to_the_hand_is_held_before_it_and_thrown_slower_than_a_small_one() {
    // Every stone, however turned in the hand, is held before the fist, reaching back into it no more than the fingers
    // closing round it would, and within what the side lets a hand hold.
    for stone in 0..beach::STONES as u32 {
        for turn in 0..6 {
            let turned = Quat::from_euler(glam::EulerRot::YXZ, turn as f32, turn as f32 * 0.7, 0.3);
            let held = palm(stone, turned);
            let back = beach::stone(stone as usize)
                .into_iter()
                .map(|point| held.transform_point(point).z)
                .fold(f32::NEG_INFINITY, f32::max);
            assert!(
                back < 0.04,
                "stone {stone} reaches {back} back into the hand"
            );
            assert!(held.position.length() <= tomori_physics::HOLD);
        }
    }
    // The heaviest leaves the hand at under half its speed, a small one at all of it.
    let heaviest = (0..beach::STONES as u32)
        .max_by(|a, b| beach::volume(*a as usize).total_cmp(&beach::volume(*b as usize)))
        .unwrap();
    assert!(beach::big(heaviest as usize));
    assert!((0.45..0.6).contains(&heft(heaviest)), "{}", heft(heaviest));
    assert_eq!(heft(0), 1.0);
    // Pulled along the ray and thrown as fast as a small one, it flies slower.
    let mut stones = Stones::new();
    let mut shared = state();
    shared.bodies[0].body = heaviest;
    stones.share(&shared);
    let start = Instant::now();
    stones.shown(&[], &[], start, 0);
    let hand = hand_at(lying(0).position + Vec3::new(0.0, 1.0, 1.0));
    let intents = stones.update(
        [&grab(heaviest, hand, Phase::Began, Side::Right)].into_iter(),
        Some(5),
        start,
        0,
    );
    let [Intent::Take(take)] = intents[..] else {
        panic!("{intents:?}")
    };
    assert!(take.offset.placement(Precision::Rest).position.z < -0.1);
    let mut now = start;
    for step in 1..=6 {
        now += Duration::from_millis(20);
        let moved = hand_at(hand.position + Vec3::new(0.04, 0.0, 0.0) * step as f32);
        stones.update(
            [&grab(heaviest, moved, Phase::Changed, Side::Right)].into_iter(),
            Some(5),
            now,
            0,
        );
    }
    now += Duration::from_millis(20);
    let moved = hand_at(hand.position + Vec3::new(0.28, 0.0, 0.0));
    let intents = stones.update(
        [&grab(heaviest, moved, Phase::Ended, Side::Right)].into_iter(),
        Some(5),
        now,
        200,
    );
    let [Intent::LetGo(let_go)] = intents[..] else {
        panic!("{intents:?}")
    };
    let speed = Vec3::from(let_go.velocity).length();
    assert!(speed > 0.8 && speed < 1.2, "{speed}");
}

/// The client side and the farmyard's own server side's stones, as one participant, 3, plays with them: what the client
/// asks reaches the side `latency` later, unless it is dropped, as the server drops an intent beyond its rate, and what
/// the side says reaches the client as long again.
mod played {
    use super::*;
    use std::collections::VecDeque;
    use tomori_environment::Presence;
    use tomori_environment_farmyard::side::Shingle;
    use tomori_protocol::Pose;

    struct Played {
        side: Shingle,
        client: Stones,
        start: Instant,
        now: Instant,
        latency: Duration,
        asked: VecDeque<(Instant, String)>,
        said: VecDeque<(Instant, u64, Update)>,
        /// Whether the client's next letting go is dropped on the way.
        drop_letting_go: bool,
    }

    impl Played {
        fn new(latency: u64) -> Self {
            let start = Instant::now();
            let side = Shingle::new(start);
            let mut client = Stones::new();
            client.share(&side.shared());
            Self {
                side,
                client,
                start,
                now: start,
                latency: Duration::from_millis(latency),
                asked: VecDeque::new(),
                said: VecDeque::new(),
                drop_letting_go: false,
            }
        }

        /// A frame of 14 ms, the grip `phase` on stone `stone`, if any, with the hand at `hand` and the head beside it.
        fn frame(&mut self, grip: Option<(u32, Phase)>, hand: Vec3) {
            self.now += Duration::from_millis(14);
            let clock = self.now.duration_since(self.start).as_millis() as u64;
            while self.said.front().is_some_and(|(at, ..)| *at <= self.now) {
                let (_, sent, update) = self.said.pop_front().unwrap();
                self.client.changed(sent, &update, clock);
            }
            let grabs: Vec<Grab> = grip
                .map(|(stone, phase)| grab(stone, hand_at(hand), phase, Side::Right))
                .into_iter()
                .collect();
            for intent in self.client.update(grabs.iter(), Some(3), self.now, clock) {
                if matches!(intent, Intent::LetGo(_)) && std::mem::take(&mut self.drop_letting_go) {
                    continue;
                }
                let asked = serde_json::to_string(&intent).unwrap();
                self.asked.push_back((self.now + self.latency, asked));
            }
            let presence = [Presence::new(
                3,
                Pose::at((hand + Vec3::new(-0.3, 0.6, 0.0)).to_array()),
                None,
                Some(Pose::at(hand.to_array())),
                self.now,
            )];
            while self.asked.front().is_some_and(|(at, _)| *at <= self.now) {
                let (_, asked) = self.asked.pop_front().unwrap();
                assert!(self.side.ask(3, &asked, &presence));
            }
            self.side.sweep(Some(&[3]), &presence, self.now);
            if let Some(update) = self.side.update(self.now) {
                self.said
                    .push_back((self.now + self.latency, clock, update));
            }
            self.client.shown(&[], &[], self.now, clock);
        }

        /// Stone `stone` taken within reach, lifted `lift` and let go, then, `wait` frames on, taken again where it is
        /// drawn and held for two seconds: whether the side then says participant 3 holds it, and the client agrees.
        fn taken_again(&mut self, stone: u32, lift: f32, wait: usize) -> bool {
            let at = self.side.world().placement(stone).unwrap().position;
            let mut hand = at + Vec3::Y * 0.03;
            self.frame(Some((stone, Phase::Began)), hand);
            for _ in 0..30 {
                hand += Vec3::Y * lift / 30.0;
                self.frame(Some((stone, Phase::Changed)), hand);
            }
            self.frame(Some((stone, Phase::Ended)), hand);
            for _ in 0..wait {
                self.frame(None, hand);
            }
            let hand = self.client.drawn[&stone].position + Vec3::Y * 0.03;
            assert_eq!(
                self.client.within_reach(hand),
                Some(format!("stone-{stone}"))
            );
            self.frame(Some((stone, Phase::Began)), hand);
            for _ in 0..140 {
                self.frame(Some((stone, Phase::Changed)), hand);
            }
            self.side.world().held(stone).map(|held| held.by) == Some(3)
                && self.client.holding[1]
                    .as_ref()
                    .is_some_and(|held| held.agreed)
        }
    }

    #[test]
    fn a_stone_dropped_is_taken_again_however_soon_and_however_far_the_server_is() {
        for latency in [0, 60, 150] {
            for wait in [0, 5, 20, 100] {
                for lift in [0.0, 0.8] {
                    let mut played = Played::new(latency);
                    assert!(
                        played.taken_again(5, lift, wait),
                        "taken again {wait} frames after it was dropped from {lift} m, {latency} ms away"
                    );
                }
            }
        }
    }

    #[test]
    fn a_stone_whose_letting_go_was_dropped_on_the_way_is_taken_again() {
        for latency in [0, 60] {
            for wait in [0, 20, 100] {
                let mut played = Played::new(latency);
                played.drop_letting_go = true;
                assert!(
                    played.taken_again(12, 0.3, wait),
                    "taken again {wait} frames after its letting go was lost, {latency} ms away"
                );
            }
        }
    }
}
