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
        size: 0.0,
        burnt: Vec::new(),
        burning: vec![None; STICKS],
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
        size: None,
        burnt: Vec::new(),
        lit: Vec::new(),
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
            size: None,
            burnt: Vec::new(),
            lit: vec![campsite::Lit {
                body: campsite::body(1),
                since: Some(2),
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
    // Black, in the heat, it burns, as the side says.
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
        shared.burning[3] = Some(0);
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
            size: None,
            burnt: Vec::new(),
            lit: Vec::new(),
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
            size: None,
            burnt: Vec::new(),
            lit: Vec::new(),
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
        size: None,
        burnt: Vec::new(),
        lit: Vec::new(),
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
            size: None,
            burnt: Vec::new(),
            lit: Vec::new(),
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

/// Stick `index` taken by `participant`, this participant, with their right hand by its handle, as it leans on its log,
/// at `now`, the side agreeing.
fn take_mine(sticks: &mut Sticks, index: usize, participant: u32, now: Instant) {
    sticks.update(
        [grab(index, hand_on(index), Phase::Began, Side::Right)].iter(),
        Some(participant),
        now,
        0,
    );
    sticks.changed(&holding(index, 1, held_by(participant)), 0);
}

/// A frame at `now` of stick `index` in the right hand of `participant`, this participant, at `hand`: how it is drawn.
fn frame(sticks: &mut Sticks, index: usize, participant: u32, hand: Hand, now: Instant) -> Shown {
    sticks.update(
        [grab(index, hand, Phase::Changed, Side::Right)].iter(),
        Some(participant),
        now,
        0,
    );
    sticks.shown(&[], &[], now, 0)[index]
}

/// The side saying stick `index`'s marshmallow caught, `since`, or went out.
fn lit(index: usize, since: Option<u64>) -> Event {
    Event {
        clock: since.unwrap_or_default(),
        sticks: Update::default(),
        toast: Vec::new(),
        size: None,
        burnt: Vec::new(),
        lit: vec![campsite::Lit {
            body: campsite::body(index),
            since,
        }],
    }
}

/// Frames of stick `index` in the right hand of this participant, number 9, at `hand`, every 16 ms for `seconds` from
/// `now`, which it moves on: whether it is drawn burning in each.
fn held_for(sticks: &mut Sticks, hand: Hand, now: &mut Instant, seconds: f32) -> Vec<bool> {
    (0..(seconds / 0.016).round() as usize)
        .map(|_| {
            *now += Duration::from_millis(16);
            frame(sticks, 3, 9, hand, *now).burning
        })
        .collect()
}

/// A hand holding a stick's marshmallow two metres from the heart of the fire, beyond its heat.
fn away_from_the_fire() -> Hand {
    Hand::new(
        over_the_fire().position + Vec3::X * 2.0,
        over_the_fire().orientation,
    )
}

/// A hand holding a stick's marshmallow under the pool's water, and where it is.
fn in_the_pool() -> (Hand, Vec3) {
    let reach = campsite::STICK_LENGTH - campsite::MARSHMALLOW_INSET - campsite::HOLD;
    let along = Vec3::new(0.0, -0.6, 1.0).normalize();
    let under = tomori_environment_farmyard::layout::POOL - Vec3::Y * 0.1;
    let hand = Hand::new(
        under - along * reach,
        Quat::from_rotation_arc(Vec3::NEG_Z, along),
    );
    (hand, under)
}

#[test]
fn another_s_marshmallow_burns_as_the_side_says_wherever_it_is_and_for_one_joining_late() {
    let mut sticks = Sticks::new();
    sticks.share(&shared(|shared| {
        shared.sticks.bodies[3].held = held_by(9);
        shared.toast[3] = campsite::BLACK;
        shared.sticks.bodies[1].held = held_by(8);
        shared.toast[1] = campsite::BROWN;
    }));
    let start = Instant::now();
    // Black in the fire, it is not drawn alight until the side says it caught, as the side alone burns it.
    let hands = [(9, over_the_fire()), (8, over_the_fire())];
    let shown = sticks.shown(&hands, &[], start, 0);
    assert!(!shown[3].burning && !shown[1].burning);
    assert_eq!(shown[3].index, 3);
    sticks.changed(&lit(3, Some(1_200)), 1_200);
    assert_eq!(sticks.lit(3), Some(1_200));
    let shown = sticks.shown(&hands, &[], start, 1_200);
    assert!(shown[3].burning && !shown[1].burning);
    // Carried away from the fire, and into the pool, it burns on until the side says it went out.
    let away = [(9, away_from_the_fire()), (8, away_from_the_fire())];
    let later = start + Duration::from_secs(60);
    assert!(sticks.shown(&away, &[], later, 61_200)[3].burning);
    assert!(sticks.shown(&[(9, in_the_pool().0)], &[], later, 61_200)[3].burning);
    sticks.changed(&lit(3, None), 61_250);
    assert!(!sticks.shown(&away, &[], later, 61_250)[3].burning);
    // What is not a stick changes nothing.
    sticks.changed(
        &Event {
            lit: vec![campsite::Lit {
                body: 7,
                since: Some(0),
            }],
            ..lit(3, None)
        },
        61_300,
    );
    assert_eq!(sticks.lit(3), None);
    // Someone joining as one burns, away from the fire, sees it alight at once, as the side shares it; and one fading on
    // its way back, unlit.
    let mut late = Sticks::new();
    late.share(&shared(|shared| {
        shared.sticks.bodies[3].held = held_by(9);
        shared.toast[3] = campsite::BLACK;
        shared.burning[3] = Some(30_000);
        shared.burning.truncate(2);
    }));
    assert_eq!(late.lit(3), None);
    late.share(&shared(|shared| {
        shared.sticks.bodies[3].held = held_by(9);
        shared.toast[3] = campsite::BLACK;
        shared.burning[3] = Some(30_000);
    }));
    let shown = late.shown(&[(9, away_from_the_fire())], &[], later, 61_000);
    assert!(shown[3].burning);
    assert_eq!(campsite::heat(shown[3].marshmallow), 0.0);
    assert_eq!(shown.iter().filter(|stick| stick.burning).count(), 1);
}

#[test]
fn the_holder_foresees_their_marshmallow_catching_and_going_out_and_gives_way_to_the_side() {
    let mut sticks = Sticks::new();
    sticks.share(&shared(|shared| shared.toast[3] = campsite::BLACK));
    let mut now = Instant::now();
    take_mine(&mut sticks, 3, 9, now);
    // In the fire, it catches as soon as it is drawn in the heat, before the side says so; the side saying so within
    // the grace, it burns on.
    let frames = held_for(&mut sticks, over_the_fire(), &mut now, 0.2);
    let first = frames.iter().position(|burning| *burning).unwrap();
    assert!(frames[first..].iter().all(|burning| *burning));
    sticks.changed(&lit(3, Some(1_000)), 1_000);
    assert!(
        held_for(&mut sticks, over_the_fire(), &mut now, 1.0)
            .iter()
            .all(|burning| *burning)
    );
    // Carried out of the heat, it burns on; plunged into the pool, it goes out at once, before the side says so, and
    // stays out once it does.
    assert!(
        held_for(&mut sticks, away_from_the_fire(), &mut now, 1.0)
            .iter()
            .all(|burning| *burning)
    );
    let (pool, under) = in_the_pool();
    let frames = held_for(&mut sticks, pool, &mut now, 0.3);
    assert!(frames.iter().skip(1).all(|burning| !*burning), "{frames:?}");
    assert!(
        sticks.shown(&[], &[], now, 0)[3]
            .marshmallow
            .distance(under)
            < 1e-3
    );
    sticks.changed(&lit(3, None), 2_000);
    assert!(
        held_for(&mut sticks, away_from_the_fire(), &mut now, 1.0)
            .iter()
            .all(|burning| !*burning)
    );
    // Back in the fire, it catches again; but the side never saying so, as when the holder's foresight ran ahead of it,
    // it goes out once the grace has passed, as it is for everyone else.
    let frames = held_for(&mut sticks, over_the_fire(), &mut now, 1.0);
    let first = frames.iter().position(|burning| *burning).unwrap();
    let last = frames.iter().rposition(|burning| *burning).unwrap();
    let alight = (last + 1 - first) as f32 * 0.016;
    assert!((alight - GRACE.as_secs_f32()).abs() < 0.04, "{alight}");
    assert!(frames[last + 1..].iter().all(|burning| !*burning));
    // The side saying it caught while the holder foresees it out, it is drawn alight once the grace has passed, and
    // burns on as the side says.
    sticks.changed(&lit(3, Some(5_000)), 5_000);
    let frames = held_for(&mut sticks, away_from_the_fire(), &mut now, 1.0);
    let first = frames.iter().position(|burning| *burning).unwrap();
    assert!(
        (first as f32 * 0.016 - GRACE.as_secs_f32()).abs() < 0.04,
        "{first}"
    );
    assert!(frames[first..].iter().all(|burning| *burning));
    // A brown one, never alight.
    sticks.changed(
        &Event {
            toast: vec![campsite::Toasted {
                body: campsite::body(3),
                toast: campsite::BROWN,
            }],
            ..lit(3, None)
        },
        6_000,
    );
    assert!(
        held_for(&mut sticks, over_the_fire(), &mut now, 1.0)
            .iter()
            .all(|burning| !*burning)
    );
}

#[test]
fn another_s_burning_marshmallow_thrown_trails_unbroken_from_their_hand_on_its_snapshots() {
    use tomori_physics::message::Snapshot;
    let mut sticks = Sticks::new();
    sticks.share(&shared(|shared| {
        shared.toast[2] = campsite::BLACK;
        shared.burning[2] = Some(800);
    }));
    let body = campsite::body(2);
    // Held by someone else, swung through the fire, its marshmallow over its heart as they let go, and thrown on at
    // 2 m/s, a snapshot every twentieth of a second, the first as the side says they let go.
    let held = Held {
        by: 9,
        hand: tomori_physics::Side::Right,
        offset: Default::default(),
    };
    sticks.changed(&holding(2, 1, Some(held)), 900);
    let velocity = Vec3::new(2.0, 0.0, 0.0);
    let along = Quat::from_rotation_y(0.4);
    let reach = campsite::STICK_LENGTH - campsite::MARSHMALLOW_INSET - campsite::STICK_LENGTH * 0.5;
    let thrown = |clock: u64| {
        let seconds = (clock as f32 - 1_000.0) / 1000.0;
        let marshmallow = campsite::heart() + velocity * seconds;
        Placement::new(marshmallow - along * Vec3::NEG_Z * reach, along)
    };
    let mut embers = crate::embers::Embers::new();
    let start = Instant::now();
    let mut sent = 1_000;
    let mut drawn = Vec::new();
    for clock in (860..1_700).step_by(14) {
        while sent <= clock {
            sticks.changed(
                &Event {
                    clock: sent,
                    sticks: Update {
                        moving: vec![Snapshot::new(body, &thrown(sent), Precision::Moving)],
                        holding: holding(2, 2, None)
                            .sticks
                            .holding
                            .into_iter()
                            .filter(|_| sent == 1_000)
                            .collect(),
                        ..Update::default()
                    },
                    toast: Vec::new(),
                    size: None,
                    burnt: Vec::new(),
                    lit: Vec::new(),
                },
                sent,
            );
            sent += 50;
        }
        let now = start + Duration::from_millis(clock - 860);
        // In their hand until they let go; then drawn on from there, where its snapshots put it carried to the clock,
        // as fast as it moved in their hand, and burning all the way.
        let hand = thrown(clock);
        let hands = [(9, Hand::new(hand.position, hand.rotation))];
        let shown = sticks.shown(if clock < 1_000 { &hands } else { &[] }, &[], now, clock);
        let stick = shown[2];
        let expected = Pose::of(&thrown(clock)).marshmallow();
        // Foreseen falling from the frame it was last drawn in their hand, it is drawn within a centimetre.
        assert!(stick.marshmallow.distance(expected) < 1e-2, "{clock}");
        assert!(stick.burning, "{clock}");
        let burning: Vec<_> = shown
            .iter()
            .filter(|stick| stick.burning)
            .map(|stick| (stick.index, stick.marshmallow))
            .collect();
        embers.update((clock - 860) as f64 / 1000.0, &burning);
        drawn.push(stick.marshmallow);
    }
    // Its trail runs along where it was drawn, from their hand, every point on its way, joined, up to where it is drawn.
    let ribbon: Vec<_> = embers
        .glowing()
        .filter(|quad| quad[0].kind() == crate::embers::Kind::Ribbon)
        .map(|quad| (quad[0].at(), quad[2].at()))
        .collect();
    assert!(ribbon.len() > 60, "{}", ribbon.len());
    for (from, to) in &ribbon {
        for point in [from, to] {
            let off = (*point - campsite::heart())
                .cross(velocity.normalize())
                .length();
            assert!(off < 1e-2, "{point}");
        }
    }
    for pair in ribbon.windows(2) {
        assert_eq!(pair[0].1, pair[1].0);
    }
    assert!(ribbon[0].0.distance(drawn[0]) < 1e-4);
    assert!(ribbon.last().unwrap().1.distance(*drawn.last().unwrap()) < crate::embers::SPACING);
}

/// The sticks and logs, as the server side shares them, each in its place, fresh, the fire at its default, with `change`
/// made.
fn with_logs(change: impl FnOnce(&mut Shared)) -> Shared {
    shared(|shared| {
        shared.sticks.bodies.extend((0..LOGS).map(|index| Body {
            body: campsite::log_body(index),
            epoch: 0,
            at: campsite::log_home(index).encoded(Precision::Rest),
            moving: false,
            held: None,
        }));
        shared.burnt = vec![0.0; LOGS];
        change(shared);
    })
}

/// A ray from a little above and before log `index` where it lies on the woodpile, straight at its middle.
fn at_log(index: usize) -> Ray {
    let middle = campsite::log_home(index).position;
    let from = middle + Vec3::new(0.3, 0.8, 0.6);
    Ray::new(from, middle - from).unwrap()
}

#[test]
fn a_log_is_pointed_at_and_taken_where_the_hand_closes_on_it_but_not_one_burning_in_the_fire() {
    let mut sticks = Sticks::new();
    sticks.share(&with_logs(|shared| shared.burnt[7] = 0.2));
    // Pointed at, a log on the woodpile is the grip's to take, called by its place on it.
    let picked = sticks.pick(&at_log(6)).unwrap();
    assert_eq!(picked.object, "log-6");
    assert_eq!(picked.label, "a log");
    assert!(picked.actions.contains(Actions::GRAB));
    // Reached for, beside it.
    let beside = campsite::log_home(5).position + Vec3::Y * (campsite::LOG_RADIUS + 0.03);
    assert_eq!(sticks.within_reach(beside).as_deref(), Some("log-5"));
    // Burning, it is neither pointed at nor reached for.
    let burning = at_log(7);
    assert_ne!(
        sticks.pick(&burning).map(|picked| picked.object),
        Some("log-7".to_owned())
    );
    let on = campsite::log_home(7).position + Vec3::Y * (campsite::LOG_RADIUS + 0.03);
    assert_ne!(sticks.within_reach(on).as_deref(), Some("log-7"));
    // Taken where the hand closes on it, and let go as it moved.
    let now = Instant::now();
    let hand = Hand::new(beside, Quat::IDENTITY);
    let grab = Grab::new("log-5", hand, Phase::Began).of(Side::Right);
    let intents = sticks.update([&grab].into_iter(), Some(3), now, 10);
    let [Intent::Take(take)] = intents.as_slice() else {
        panic!("{intents:?}");
    };
    assert_eq!(take.body, campsite::log_body(5));
    let offset = take.offset.placement(Precision::Rest);
    let held = Placement::new(beside, Quat::IDENTITY).then(&offset);
    assert!(held.position.distance(campsite::log_home(5).position) < 1e-3);
    let moved = Hand::new(beside + Vec3::X * 0.05, Quat::IDENTITY);
    let release = Grab::new("log-5", moved, Phase::Ended).of(Side::Right);
    let later = now + Duration::from_millis(50);
    let intents = sticks.update([&release].into_iter(), Some(3), later, 60);
    let [Intent::LetGo(let_go)] = intents.as_slice() else {
        panic!("{intents:?}")
    };
    assert_eq!(let_go.body, campsite::log_body(5));
    // Pointed at from afar, it is pulled into the palm, across it, turned as it was; and its touch, as the other hand
    // takes it from the one that holds it, is how far the hand is from its sides.
    let far = Placement::new(beside + Vec3::new(0.0, 1.0, 1.5), Quat::IDENTITY);
    let (offset, pulled) = Pile::held(0, &campsite::log_home(4), &far, false);
    assert!(pulled && offset.position.length() <= tomori_physics::HOLD);
    assert_eq!(
        Pile::touch(0, &campsite::log_home(4), beside),
        Pile::reach(0, &campsite::log_home(4), beside)
    );
    // Heard landing, a knock softer than a stone's, or a splash, and nothing gently.
    assert!(Pile::sound(0, false, 3.0).is_some() && Pile::sound(0, true, 3.0).is_some());
    assert!(Pile::sound(0, false, 0.2).is_none());
    // A visitor takes none.
    let mut visiting = Sticks::new();
    visiting.share(&with_logs(|_| {}));
    let grab = Grab::new("log-5", hand, Phase::Began).of(Side::Right);
    assert!(
        visiting
            .update([&grab].into_iter(), None, now, 10)
            .is_empty()
    );
}

#[test]
fn a_burning_log_chars_glows_and_shrinks_and_fades_back_onto_the_woodpile_fresh() {
    let mut sticks = Sticks::new();
    sticks.share(&with_logs(|shared| {
        shared.burnt[6] = 0.5;
        shared.burnt[5] = 0.05;
    }));
    let now = Instant::now();
    let shown = sticks.logs_shown(&[], &[], now, 0);
    assert_eq!(shown.len(), LOGS);
    assert_eq!((shown[0].shade, shown[5].shade, shown[6].shade), (0, 1, 2));
    let size = |log: &ShownLog| log.transform.x_axis.truncate().length();
    assert!((size(&shown[0]) - 1.0).abs() < 1e-4);
    assert!((size(&shown[6]) - campsite::charred(0.5)).abs() < 1e-4);
    // Burnt away, it fades out where it lay, as burnt as it was, and in on the woodpile, fresh, the side saying so as it
    // begins its way.
    let from = Placement::at(campsite::FIRE + Vec3::Y * 0.1);
    let to = campsite::log_home(6).snapped(Precision::Rest);
    sticks.changed(
        &Event {
            clock: 1_000,
            sticks: Update {
                returning: vec![tomori_physics::message::Returning {
                    body: campsite::log_body(6),
                    from: from.encoded(Precision::Rest),
                    to: to.encoded(Precision::Rest),
                }],
                ..Update::default()
            },
            toast: Vec::new(),
            size: None,
            // And something that is no log, which changes nothing.
            burnt: [campsite::log_body(6), 7]
                .map(|body| campsite::Burnt { body, burnt: 0.0 })
                .to_vec(),
            lit: Vec::new(),
        },
        1_000,
    );
    assert_eq!(sticks.burnt(6), 0.0);
    let out = sticks.logs_shown(&[], &[], now, 1_100 + tomori_physics::FADE / 2)[6];
    assert_eq!(out.shade, 2);
    assert!((size(&out) - campsite::charred(0.5) * 0.5).abs() < 1e-3);
    assert!(out.transform.w_axis.truncate().distance(from.position) < 1e-4);
    let coming = sticks.logs_shown(&[], &[], now, 1_100 + tomori_physics::FADE * 3 / 2)[6];
    assert_eq!(coming.shade, 0);
    assert!(coming.transform.w_axis.truncate().distance(to.position) < 1e-4);
}

#[test]
fn the_fire_is_drawn_as_big_as_the_side_says_eased_there_toasting_and_crackling_the_more_the_bigger()
 {
    let mut sticks = Sticks::new();
    sticks.share(&with_logs(|_| {}));
    let now = Instant::now();
    sticks.shown(&[], &[], now, 0);
    assert_eq!(sticks.drawn_size(), 0.0);
    // At its default, it crackles only as the ambience does.
    let mut crackles = Vec::new();
    for step in 0..=100 {
        crackles.extend(sticks.crackles(step * 100));
    }
    assert!(crackles.is_empty());
    // The side says it is at its biggest: drawn growing there over a moment, never jumping.
    sticks.changed(
        &Event {
            clock: 10_000,
            sticks: Update::default(),
            toast: Vec::new(),
            size: Some(1.0),
            burnt: Vec::new(),
            lit: Vec::new(),
        },
        10_000,
    );
    assert_eq!(sticks.size(), 1.0);
    let mut last = 0.0;
    for frame in 1..=40 {
        sticks.shown(&[], &[], now + Duration::from_millis(50 * frame), 10_000);
        let drawn = sticks.drawn_size();
        assert!(drawn > last && drawn - last < 0.25, "{frame}: {drawn}");
        last = drawn;
    }
    assert!(last > 0.99);
    // It crackles, about three times a second, at its heart, louder than at a small size.
    let mut crackles = Vec::new();
    for step in 101..=200 {
        crackles.extend(sticks.crackles(step * 100));
    }
    let count = crackles.len();
    assert!((10..60).contains(&count), "{count}");
    for (sound, at) in &crackles {
        assert!(Vec3::from(*at).distance(campsite::heart_at(1.0)) < 0.4);
        assert!(!sound.samples.is_empty());
    }
    // A clock that jumps crackles nothing.
    assert!(sticks.crackles(60_000).is_empty());
    // A black marshmallow held further off than a small fire reaches is foreseen catching in a big one.
    sticks.share(&with_logs(|shared| {
        shared.size = 1.0;
        shared.toast[1] = campsite::BLACK;
    }));
    let along = Vec3::new(1.0, 0.0, 0.0);
    let marshmallow = campsite::heart_at(1.0) + Vec3::X * 0.9;
    assert_eq!(campsite::heat(marshmallow), 0.0);
    let hand = marshmallow
        - along * (campsite::STICK_LENGTH - campsite::MARSHMALLOW_INSET - campsite::HOLD);
    let hand = Hand::new(hand, Quat::from_rotation_arc(Vec3::NEG_Z, along));
    let mut later = now + Duration::from_secs(5);
    take_mine(&mut sticks, 1, 5, later);
    let mut caught = false;
    for _ in 0..30 {
        later += Duration::from_millis(16);
        caught |= frame(&mut sticks, 1, 5, hand, later).burning;
    }
    assert!(caught);
}

#[test]
fn what_moves_and_rests_of_the_campsite_is_shown_of_the_sticks_and_the_logs_alike() {
    let mut sticks = Sticks::new();
    sticks.share(&with_logs(|_| {}));
    let now = Instant::now();
    // A stick and a log moving, then each coming to rest, as the server side says, each drawn where it says.
    let moved = |body: u32, at: Placement| {
        tomori_physics::message::Snapshot::new(body, &at, Precision::Moving)
    };
    let (stick, log) = (
        Placement::at(campsite::FIRE + Vec3::new(1.0, 0.3, 0.5)),
        Placement::at(campsite::FIRE + Vec3::new(-1.0, 0.3, 0.5)),
    );
    let event = |clock: u64, moving: bool| Event {
        clock,
        sticks: Update {
            moving: if moving {
                vec![
                    moved(campsite::body(1), stick),
                    moved(campsite::log_body(2), log),
                ]
            } else {
                Vec::new()
            },
            rested: if moving {
                Vec::new()
            } else {
                vec![
                    tomori_physics::message::Snapshot::new(
                        campsite::body(1),
                        &stick,
                        Precision::Rest,
                    ),
                    tomori_physics::message::Snapshot::new(
                        campsite::log_body(2),
                        &log,
                        Precision::Rest,
                    ),
                ]
            },
            ..Update::default()
        },
        toast: Vec::new(),
        size: None,
        burnt: Vec::new(),
        lit: Vec::new(),
    };
    sticks.changed(&event(1_000, true), 1_000);
    sticks.changed(&event(1_100, false), 1_100);
    let shown = sticks.shown(&[], &[], now, 2_000);
    let logs = sticks.logs_shown(&[], &[], now, 2_000);
    assert!(
        shown[1]
            .transform
            .transform_point3(-Vec3::Z * campsite::STICK_LENGTH * 0.5)
            .distance(stick.position)
            < 1e-3
    );
    assert!(logs[2].transform.w_axis.truncate().distance(log.position) < 1e-3);
}

#[test]
fn played_against_the_side_one_pulled_out_as_it_turns_black_burns_for_everyone_and_for_one_joining_late()
 {
    use tomori_environment::Presence;
    use tomori_environment_farmyard::side;
    use tomori_protocol::Pose as Seen;
    let start = Instant::now();
    let mut side = side::Sticks::new(start);
    let (mut holder, mut watcher) = (Sticks::new(), Sticks::new());
    holder.share(&side.shared());
    watcher.share(&side.shared());
    let mut now = start;
    // A frame of the side's tick, the holder, participant 3, gripping stick 1 at `hand`, their presence bringing it to
    // the side as it is: how the holder and the watcher draw it.
    let mut frame = |side: &mut side::Sticks,
                     holder: &mut Sticks,
                     watcher: &mut Sticks,
                     hand: Hand,
                     phase: Phase|
     -> (bool, bool) {
        now += Duration::from_millis(50);
        let clock = now.duration_since(start).as_millis() as u64;
        let presence = [Presence::new(
            3,
            Seen::at((hand.position + Vec3::new(0.0, 0.5, 0.4)).to_array()),
            None,
            Some(Seen {
                position: hand.position.to_array(),
                orientation: hand.orientation.to_array(),
            }),
            now,
        )];
        for intent in holder.update(
            [grab(1, hand, phase, Side::Right)].iter(),
            Some(3),
            now,
            clock,
        ) {
            side.ask(3, &serde_json::to_string(&intent).unwrap(), &presence);
        }
        if let Some(event) = side.update(&presence, now, clock) {
            holder.changed(&event, clock);
            watcher.changed(&event, clock);
        }
        let mine = holder.shown(&[], &[], now, clock)[1].burning;
        let theirs = watcher.shown(&[(3, hand)], &[], now, clock)[1].burning;
        (mine, theirs)
    };
    frame(
        &mut side,
        &mut holder,
        &mut watcher,
        hand_on(1),
        Phase::Began,
    );
    // Held over the heart of the fire until it turns black, and pulled out the moment it does.
    let mut held = 0;
    while side.lit(1).is_none() {
        let (_, theirs) = frame(
            &mut side,
            &mut holder,
            &mut watcher,
            over_the_fire(),
            Phase::Changed,
        );
        assert_eq!(theirs, side.lit(1).is_some());
        held += 1;
        assert!(held < 400, "never caught");
    }
    assert!(side.toast(1) >= campsite::BLACK);
    // Out of the heat, it burns on for everyone, and for someone joining now.
    for _ in 0..100 {
        let (mine, theirs) = frame(
            &mut side,
            &mut holder,
            &mut watcher,
            away_from_the_fire(),
            Phase::Changed,
        );
        assert!(mine && theirs);
    }
    let mut late = Sticks::new();
    late.share(&side.shared());
    let shown = late.shown(&[(3, away_from_the_fire())], &[], Instant::now(), 0);
    assert!(shown[1].burning);
    assert_eq!(shown[1].shade, campsite::SHADES - 1);
    // Until it goes out, for everyone at once.
    let mut frames = 100;
    while side.lit(1).is_some() {
        let (_, theirs) = frame(
            &mut side,
            &mut holder,
            &mut watcher,
            away_from_the_fire(),
            Phase::Changed,
        );
        assert_eq!(theirs, side.lit(1).is_some());
        frames += 1;
        assert!(frames < 1_000, "never went out");
    }
    let burned = frames as f32 * 0.05;
    assert!((burned - campsite::BURNS).abs() <= 0.15, "{burned}");
    let (mine, theirs) = frame(
        &mut side,
        &mut holder,
        &mut watcher,
        away_from_the_fire(),
        Phase::Changed,
    );
    assert!(!mine && !theirs);
}
