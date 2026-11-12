// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::time::Duration;

use glam::Quat;
use tomori_physics::message::{Holding, Landed, Shared as Body, State, Update};
use tomori_physics::{Held, PULL_TIME, Precision};
use tomori_scene::client_side::{Actions, Phase, Side};

use super::*;
use crate::loose::placed;

/// The sticks, as the server side shares them, each on its log, fresh, with `change` made.
fn shared(change: impl FnOnce(&mut Shared)) -> Shared {
    let mut shared = Shared {
        sticks: State {
            bodies: (0..STICKS)
                .map(|index| Body {
                    body: campsite::body(index),
                    epoch: 0,
                    at: campsite::home(index).encoded(Precision::Rest),
                    moving: false,
                    held: None,
                })
                .collect(),
        },
        toast: vec![0.0; STICKS],
    };
    change(&mut shared);
    shared
}

/// Stick `index` held by participant `by`, in their right hand, by its handle.
fn held_by(by: u32) -> Option<Held> {
    Some(Held {
        by,
        hand: tomori_physics::Side::Right,
        offset: campsite::grip().encoded(Precision::Rest),
    })
}

/// A hand holding a stick by its handle, as it leans on its log.
fn hand_on(index: usize) -> Hand {
    let hand = campsite::home(index)
        .snapped(Precision::Rest)
        .then(&campsite::grip().inverse());
    Hand::new(hand.position, hand.rotation)
}

/// A hand holding a stick's marshmallow over the heart of the fire.
fn over_the_fire() -> Hand {
    let along = Vec3::new(1.0, 0.3, 0.0).normalize();
    let hand = campsite::heart()
        - along * (campsite::STICK_LENGTH - campsite::MARSHMALLOW_INSET - campsite::HOLD);
    Hand::new(hand, Quat::from_rotation_arc(Vec3::NEG_Z, along))
}

fn grab(index: usize, hand: Hand, phase: Phase, side: Side) -> Grab {
    Grab::new(format!("stick-{index}"), hand, phase).of(side)
}

/// What changed, as the server side says it, of stick `index`: who holds it at what epoch.
fn holding(index: usize, epoch: u32, held: Option<Held>) -> Event {
    Event {
        clock: 1,
        sticks: Update {
            holding: vec![Holding {
                body: campsite::body(index),
                epoch,
                held,
            }],
            ..Update::default()
        },
        toast: Vec::new(),
    }
}

#[test]
fn a_ray_passing_near_a_free_stick_takes_it_with_the_grip_and_the_bonfire_is_pointed_at() {
    let mut sticks = Sticks::new();
    let at = |index: usize| {
        let pose = Pose::of(&campsite::home(index));
        Ray::new(pose.middle() + Vec3::new(0.02, 1.5, 0.0), Vec3::NEG_Y).unwrap()
    };
    // Nothing before the server side says what there is.
    assert_eq!(sticks.pick(&at(1)), None);
    sticks.share(&shared(|_| {}));
    let picked = sticks.pick(&at(1)).unwrap();
    assert_eq!(picked.object, "stick-1");
    assert!(picked.actions.contains(Actions::GRAB));
    assert!(!picked.selectable);
    // The bonfire, pointed at, takes nothing.
    let fire = Ray::new(campsite::heart() + Vec3::new(0.0, 0.2, 3.0), Vec3::NEG_Z).unwrap();
    let picked = sticks.pick(&fire).unwrap();
    assert_eq!(picked.object, "bonfire");
    assert_eq!(picked.actions, Actions::NONE);
    // A ray through the bonfire towards a stick's marshmallow over its log meets the bonfire first, and one from beyond
    // the log the other way, the stick.
    let marshmallow = Pose::of(&campsite::home(1)).marshmallow();
    let towards = (marshmallow - campsite::heart()).normalize();
    let through = Ray::new(campsite::heart() - towards * 1.0, towards).unwrap();
    assert_eq!(sticks.pick(&through).unwrap().object, "bonfire");
    let back = Ray::new(marshmallow + towards * 1.0, -towards).unwrap();
    assert_eq!(sticks.pick(&back).unwrap().object, "stick-1");
    // A stick another holds is theirs, and is not pointed at to take, nor reached for.
    sticks.share(&shared(|shared| shared.sticks.bodies[1].held = held_by(9)));
    assert_ne!(
        sticks.pick(&at(1)).map(|picked| picked.object),
        Some("stick-1".to_owned())
    );
    assert_eq!(sticks.within_reach(hand_on(1).position), None);
    // A hand at a free stick's handle reaches it, the nearest; nowhere near, none.
    assert_eq!(
        sticks.within_reach(hand_on(2).position).as_deref(),
        Some("stick-2")
    );
    assert_eq!(sticks.within_reach(Vec3::new(0.0, 1.0, 0.0)), None);
    // Its names are its place on the log, and nothing else is a stick.
    assert_eq!(
        (Rack::number(campsite::body(3)), Rack::body(3)),
        (3, Some(campsite::body(3)))
    );
    assert_eq!(Rack::body(STICKS as u32), None);
}

#[test]
fn a_stick_taken_with_either_hand_is_held_by_its_handle_along_the_fist_and_let_go_as_fast_as_the_hand_moved()
 {
    let mut sticks = Sticks::new();
    sticks.share(&shared(|_| {}));
    let start = Instant::now();
    let me = Some(3);
    // Taken with the right hand, the server side is asked for it at the epoch seen, held by its handle.
    let hand = hand_on(2);
    let intents = sticks.update(
        [grab(2, hand, Phase::Began, Side::Right)].iter(),
        me,
        start,
        1_000,
    );
    let [Intent::Take(take)] = intents.as_slice() else {
        panic!("{intents:?}")
    };
    assert_eq!(
        (take.body, take.epoch, take.hand),
        (campsite::body(2), 0, tomori_physics::Side::Right)
    );
    assert_eq!(
        take.offset.placement(Precision::Rest),
        campsite::grip().snapped(Precision::Rest)
    );
    // It settles into the fist as it is taken, and is drawn there.
    let later = start + Duration::from_secs_f32(PULL_TIME + 0.01);
    let shown = sticks.shown(&[], &[], later, 1_000);
    let in_hand = campsite::drawn(&placed(&hand).then(&campsite::grip().snapped(Precision::Rest)));
    assert!(shown[2].transform.abs_diff_eq(in_hand, 1e-4));
    // The left hand takes another as the right holds its own.
    let left = hand_on(0);
    let intents = sticks.update(
        [grab(0, left, Phase::Began, Side::Left)].iter(),
        me,
        later,
        1_000,
    );
    assert!(
        matches!(intents.as_slice(), [Intent::Take(take)] if take.hand == tomori_physics::Side::Left)
    );
    // Swung forward, and let go, it is let go where it was drawn, moving as the hand did.
    let mut now = later;
    let mut hand = hand;
    for _ in 0..5 {
        now += Duration::from_millis(16);
        hand.position += Vec3::new(0.0, 0.0, -0.032);
        sticks.update(
            [grab(2, hand, Phase::Changed, Side::Right)].iter(),
            me,
            now,
            1_100,
        );
    }
    let intents = sticks.update(
        [grab(2, hand, Phase::Ended, Side::Right)].iter(),
        me,
        now,
        1_100,
    );
    let [Intent::LetGo(let_go)] = intents.as_slice() else {
        panic!("{intents:?}")
    };
    assert_eq!((let_go.body, let_go.epoch), (campsite::body(2), 1));
    assert!(
        (Vec3::from(let_go.velocity).z + 2.0).abs() < 0.1,
        "{let_go:?}"
    );
    let at = let_go.at.placement(Precision::Rest);
    assert!(Pose::of(&at).from_handle(hand.position) < 0.01);
    // The intents are takings and lettings go alone: nothing says how toasted it is.
    for intent in sticks.update(
        [grab(0, left, Phase::Ended, Side::Left)].iter(),
        me,
        now,
        1_100,
    ) {
        let text = serde_json::to_string(&intent).unwrap();
        assert!(text.starts_with(r#"{"let_go":"#), "{text}");
    }
}

#[test]
fn a_marshmallow_held_in_the_fire_is_foreseen_toasting_and_eases_to_what_the_server_side_says() {
    // Over the heart of the fire, the foresight toasts by the side's own rule, but runs no further ahead of what the side
    // said than the side's half second and presence's flight explain.
    let heart = campsite::heart();
    let mut foreseen = 0.0;
    for _ in 0..10 {
        foreseen = foresee(foreseen, 0.0, heart, 0.05);
    }
    assert!(foreseen > 0.01 && foreseen <= AHEAD + 1e-4, "{foreseen}");
    for _ in 0..100 {
        foreseen = foresee(foreseen, 0.0, heart, 0.05);
    }
    assert!(foreseen <= AHEAD + 1e-4, "{foreseen}");
    // The side says more, as it would of a hand it saw nearer the heart: the foresight eases up to it within a second.
    for _ in 0..20 {
        foreseen = foresee(foreseen, 0.5, heart + Vec3::X * 2.0, 0.05);
    }
    assert!((foreseen - 0.5).abs() < BEHIND + 0.01, "{foreseen}");
    // Out of the heat, nothing more.
    assert_eq!(foresee(0.5, 0.5, heart + Vec3::X * 2.0, 1.0), 0.5);
    // As this participant holds one, it is drawn as they foresee it; once the side says more, as the side says.
    let mut sticks = Sticks::new();
    sticks.share(&shared(|_| {}));
    let start = Instant::now();
    let me = Some(3);
    sticks.update(
        [grab(1, hand_on(1), Phase::Began, Side::Right)].iter(),
        me,
        start,
        0,
    );
    sticks.changed(&holding(1, 1, held_by(3)), 0);
    let mut now = start;
    for _ in 0..60 {
        now += Duration::from_millis(16);
        sticks.update(
            [grab(1, over_the_fire(), Phase::Changed, Side::Right)].iter(),
            me,
            now,
            0,
        );
        sticks.shown(&[], &[], now, 0);
    }
    let foreseen = sticks.foreseen[&campsite::body(1)];
    assert!(foreseen > 0.0 && foreseen <= AHEAD + 1e-4, "{foreseen}");
    sticks.changed(
        &Event {
            clock: 2,
            sticks: Update::default(),
            toast: vec![campsite::Toasted {
                body: campsite::body(1),
                toast: campsite::BLACK,
            }],
        },
        0,
    );
    let mut shown = Vec::new();
    for _ in 0..90 {
        now += Duration::from_millis(16);
        sticks.update(
            [grab(1, over_the_fire(), Phase::Changed, Side::Right)].iter(),
            me,
            now,
            0,
        );
        shown = sticks.shown(&[], &[], now, 0);
    }
    assert_eq!(shown[1].shade, campsite::SHADES - 1);
    // Black, in the heat, it burns.
    assert!(shown[1].burning);
    assert_eq!(sticks.toast(1), campsite::BLACK);
}

#[test]
fn a_stick_passed_to_the_other_hand_is_held_where_that_hand_took_it_and_toasts_on_as_foreseen() {
    let mut sticks = Sticks::new();
    let ray = |from: Vec3, to: Vec3| Ray::new(from, (to - from).normalize()).unwrap();
    // Nothing before the server side says what there is.
    assert_eq!(sticks.pick_by(Side::Left, &ray(Vec3::ZERO, Vec3::X)), None);
    sticks.share(&shared(|_| {}));
    let start = Instant::now();
    let me = Some(3);
    sticks.update(
        [grab(1, hand_on(1), Phase::Began, Side::Right)].iter(),
        me,
        start,
        0,
    );
    sticks.changed(&holding(1, 1, held_by(3)), 0);
    let mut now = start;
    let right = over_the_fire();
    for _ in 0..60 {
        now += Duration::from_millis(16);
        sticks.update(
            [grab(1, right, Phase::Changed, Side::Right)].iter(),
            me,
            now,
            0,
        );
        sticks.shown(&[], &[], now, 0);
    }
    let foreseen = sticks.foreseen[&campsite::body(1)];
    assert!(foreseen > 0.0, "{foreseen}");
    let was = sticks.shown(&[], &[], now, 0)[1];
    // The left hand on its shaft, beyond the handle a free stick is taken by, takes it from the right hand there.
    let pose = Pose::of(&placed(&right).then(&campsite::grip()));
    let shaft = right.position + pose.along * 0.55;
    assert_eq!(sticks.within_reach(shaft), None);
    assert_eq!(sticks.within_reach_of(Side::Right, shaft), None);
    assert_eq!(
        sticks.within_reach_of(Side::Left, shaft).as_deref(),
        Some("stick-1")
    );
    // The left ray meets it too, near the right hand, away from the fire; the right's not.
    let handle = right.position + pose.along * 0.1;
    let beside = handle + Vec3::new(0.0, 0.5, 0.3);
    assert_eq!(
        sticks
            .pick_by(Side::Left, &ray(beside, handle))
            .map(|picked| picked.object),
        Some("stick-1".to_owned())
    );
    assert_ne!(
        sticks
            .pick_by(Side::Right, &ray(beside, handle))
            .map(|picked| picked.object),
        Some("stick-1".to_owned())
    );
    let left = Hand::new(shaft, right.orientation);
    now += Duration::from_millis(16);
    let intents = sticks.update([grab(1, left, Phase::Began, Side::Left)].iter(), me, now, 0);
    let [Intent::Take(take)] = intents.as_slice() else {
        panic!("{intents:?}")
    };
    assert_eq!((take.epoch, take.hand), (1, tomori_physics::Side::Left));
    // Held where the left hand took it, not slid back to its handle: drawn just where it was, as toasted as foreseen.
    let offset = take.offset.placement(Precision::Rest);
    assert!(offset.position.distance(campsite::grip().position) > 0.3);
    let shown = sticks.shown(&[], &[], now, 0)[1];
    assert!(shown.transform.abs_diff_eq(was.transform, 1e-3));
    assert!(sticks.foreseen[&campsite::body(1)] >= foreseen);
    assert_eq!(sticks.passed(), [Side::Left]);
    assert_eq!(sticks.loose().held_in(tomori_physics::Side::Right), None);
    // The right grip, still closed, lets go of nothing.
    assert!(
        sticks
            .update(
                [grab(1, right, Phase::Ended, Side::Right)].iter(),
                me,
                now,
                0
            )
            .is_empty()
    );
}

#[test]
fn a_marshmallow_another_holds_or_left_lying_is_as_toasted_as_the_server_side_says_and_fresh_once_put_back()
 {
    let mut sticks = Sticks::new();
    sticks.share(&shared(|shared| {
        shared.sticks.bodies[3].held = held_by(9);
        shared.toast[3] = campsite::BLACK;
        shared.toast[0] = campsite::GOLDEN;
    }));
    let now = Instant::now();
    // In their hand, from the hand their presence brings, over the fire, burning.
    let hand = over_the_fire();
    let shown = sticks.shown(&[(9, hand)], &[], now, 0);
    let in_hand = campsite::drawn(&placed(&hand).then(&campsite::grip().snapped(Precision::Rest)));
    assert!(shown[3].transform.abs_diff_eq(in_hand, 1e-4));
    assert!(shown[3].burning);
    // On its log, golden, but not burning; the others fresh.
    assert_eq!(shown[0].shade, campsite::shade(campsite::GOLDEN));
    assert!(!shown[0].burning);
    assert_eq!(shown[1].shade, 0);
    // Their hand unreported, it stays where it was last drawn.
    let again = sticks.shown(&[], &[], now, 0);
    assert_eq!(again[3].transform, shown[3].transform);
    // Put back, fresh, as the side says.
    sticks.changed(
        &Event {
            clock: 2,
            sticks: Update::default(),
            toast: vec![campsite::Toasted {
                body: campsite::body(0),
                toast: 0.0,
            }],
        },
        0,
    );
    assert_eq!(sticks.shown(&[], &[], now, 0)[0].shade, 0);
    // What is not a number, nor a stick, changes nothing.
    sticks.changed(
        &Event {
            clock: 3,
            sticks: Update::default(),
            toast: vec![
                campsite::Toasted {
                    body: campsite::body(3),
                    toast: f32::NAN,
                },
                campsite::Toasted {
                    body: 7,
                    toast: 0.5,
                },
            ],
        },
        0,
    );
    assert_eq!(sticks.toast(3), campsite::BLACK);
}

#[test]
fn two_taking_one_stick_the_loser_lets_it_go_and_sees_it_in_the_winner_s_hand() {
    let mut sticks = Sticks::new();
    sticks.share(&shared(|_| {}));
    let now = Instant::now();
    let mine = hand_on(1);
    sticks.update(
        [grab(1, mine, Phase::Began, Side::Right)].iter(),
        Some(3),
        now,
        0,
    );
    // Another's taking arrived first: this participant holds nothing, and sees it in their hand.
    sticks.changed(&holding(1, 1, held_by(9)), 0);
    assert_eq!(sticks.loose().held_in(tomori_physics::Side::Right), None);
    let theirs = over_the_fire();
    let shown = sticks.shown(&[(9, theirs)], &[], now, 0);
    let in_hand =
        campsite::drawn(&placed(&theirs).then(&campsite::grip().snapped(Precision::Rest)));
    assert!(shown[1].transform.abs_diff_eq(in_hand, 1e-4));
    // Letting go of the grip it lost asks nothing.
    assert!(
        sticks
            .update(
                [grab(1, mine, Phase::Ended, Side::Right)].iter(),
                Some(3),
                now,
                0
            )
            .is_empty()
    );
}

#[test]
fn a_visitor_takes_no_stick_and_what_lands_is_heard_lightly_or_splashing() {
    let mut sticks = Sticks::new();
    sticks.share(&shared(|_| {}));
    let now = Instant::now();
    assert!(
        sticks
            .update(
                [grab(2, hand_on(2), Phase::Began, Side::Right)].iter(),
                None,
                now,
                0
            )
            .is_empty()
    );
    assert_eq!(sticks.loose().held_in(tomori_physics::Side::Right), None);
    // A stick landing hard taps, more softly than a stone; one landing gently is not heard; one in the pool splashes.
    let landed = |speed: f32, water: bool| Event {
        clock: 2,
        sticks: Update {
            landed: vec![Landed {
                body: campsite::body(2),
                speed,
                water,
            }],
            ..Update::default()
        },
        toast: Vec::new(),
    };
    assert_eq!(sticks.changed(&landed(3.0, false), 0).len(), 1);
    assert!(sticks.changed(&landed(0.6, false), 0).is_empty());
    assert_eq!(sticks.changed(&landed(0.6, true), 0).len(), 1);
    let tap = Rack::sound(0, false, 3.0).unwrap();
    let clack = crate::stones::sound(false, 3.0);
    let loudest = |sound: &Sound| {
        sound
            .samples
            .iter()
            .fold(0.0f32, |most, sample| most.max(sample.abs()))
    };
    assert!(loudest(&tap) < loudest(&clack));
}

#[test]
fn a_stick_on_its_way_back_fades_out_as_toasted_as_it_was_and_in_on_its_log_fresh() {
    let mut sticks = Sticks::new();
    sticks.share(&shared(|shared| shared.toast[2] = campsite::BLACK));
    let now = Instant::now();
    let from = Placement::at(campsite::heart() + Vec3::Y * 0.1);
    let to = campsite::home(2).snapped(Precision::Rest);
    // On its way back from the fire, burnt, the side saying its marshmallow fresh as it begins.
    sticks.changed(
        &Event {
            clock: 1_000,
            sticks: Update {
                // And something that is no stick, which changes nothing.
                returning: [campsite::body(2), 7]
                    .map(|body| tomori_physics::message::Returning {
                        body,
                        from: from.encoded(Precision::Rest),
                        to: to.encoded(Precision::Rest),
                    })
                    .to_vec(),
                ..Update::default()
            },
            toast: vec![campsite::Toasted {
                body: campsite::body(2),
                toast: 0.0,
            }],
        },
        1_000,
    );
    // Fading out where it was, half as large about its middle, as black as it was, but alight no longer.
    let out = sticks.shown(&[], &[], now, 1_100 + tomori_physics::FADE / 2)[2];
    assert_eq!(out.shade, campsite::shade(campsite::BLACK));
    assert!(!out.burning);
    let middle = out
        .transform
        .transform_point3(-Vec3::Z * campsite::STICK_LENGTH * 0.5);
    assert!(middle.distance(from.position) < 1e-4, "{middle}");
    assert!((out.transform.x_axis.truncate().length() - 0.5).abs() < 1e-3);
    // Fading in on its log, fresh; and whole there once back.
    let coming = sticks.shown(&[], &[], now, 1_100 + tomori_physics::FADE * 3 / 2)[2];
    assert_eq!(coming.shade, 0);
    let middle = coming
        .transform
        .transform_point3(-Vec3::Z * campsite::STICK_LENGTH * 0.5);
    assert!(middle.distance(to.position) < 1e-4, "{middle}");
    let back = sticks.shown(&[], &[], now, 1_100 + tomori_physics::FADE * 2)[2];
    assert_eq!(back.transform, campsite::drawn(&to));
    assert_eq!(back.shade, 0);
}
