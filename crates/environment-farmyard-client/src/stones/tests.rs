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
