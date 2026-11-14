// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::campsite::{FIRE, Pose};
use glam::Vec2;
use tomori_physics::message::{LetGo, Take};
use tomori_physics::{Encoded, Precision};

/// Someone, `participant`, whose right hand is at `hand`, their head a little behind and above it, seen at `at`.
fn holding(participant: u32, hand: Placement, at: Instant) -> Presence {
    Presence::new(
        participant,
        tomori_protocol::Pose::at((hand.position + Vec3::new(0.0, 0.5, 0.4)).to_array()),
        None,
        Some(tomori_protocol::Pose {
            position: hand.position.to_array(),
            orientation: hand.rotation.to_array(),
        }),
        at,
    )
}

/// Where a right hand holds stick `index` by its handle, as it leans on its log.
fn at_handle(sticks: &Sticks, index: usize) -> Placement {
    let body = sticks.world().placement(campsite::body(index)).unwrap();
    body.then(&campsite::grip().inverse())
}

/// Where a hand holds a stick, along its −Z, so that its marshmallow is at `marshmallow`.
fn toasting(marshmallow: Vec3) -> Placement {
    let back = campsite::STICK_LENGTH - campsite::MARSHMALLOW_INSET - campsite::HOLD;
    Placement::at(marshmallow + Vec3::Z * back)
}

fn take(index: usize, epoch: u32) -> String {
    serde_json::to_string(&Intent::Take(Take {
        body: campsite::body(index),
        epoch,
        hand: Side::Right,
        offset: campsite::grip().encoded(Precision::Rest),
    }))
    .unwrap()
}

fn let_go(index: usize, epoch: u32, at: Placement, velocity: [f32; 3]) -> String {
    serde_json::to_string(&Intent::LetGo(LetGo {
        body: campsite::body(index),
        epoch,
        at: at.encoded(Precision::Rest),
        velocity,
        spin: [0.0; 3],
        clock: 0,
    }))
    .unwrap()
}

/// Move `sticks` on from `now` by `seconds`, in steps of the side's tick, with `presence`: every event it said, and when
/// it ends.
fn run(
    sticks: &mut Sticks,
    presence: &[Presence],
    now: Instant,
    seconds: f32,
) -> (Vec<Event>, Instant) {
    let mut events = Vec::new();
    let mut at = now;
    for _ in 0..(seconds / 0.05).round() as usize {
        at += Duration::from_millis(50);
        events.extend(sticks.update(presence, at, 0));
    }
    (events, at)
}

/// Move `sticks` on until nothing moves, for at most `seconds`.
fn rest(sticks: &mut Sticks, now: Instant, seconds: f32) -> (Vec<Event>, Instant) {
    let mut events = Vec::new();
    let mut at = now;
    let mut taken = 0.0;
    while (sticks.world().moving() || taken < 0.2) && taken < seconds {
        at += Duration::from_millis(50);
        taken += 0.05;
        events.extend(sticks.update(&[], at, 0));
    }
    assert!(!sticks.world().moving(), "still moving after {seconds} s");
    (events, at)
}

/// Stick `index`, taken by participant 3 in their right hand at its handle, at `now`: its epoch now.
fn taken(sticks: &mut Sticks, index: usize, now: Instant) -> u32 {
    let ada = holding(3, at_handle(sticks, index), now);
    let epoch = sticks.world().epoch(campsite::body(index)).unwrap();
    assert!(sticks.ask(3, &take(index, epoch), &[ada]));
    assert_eq!(
        sticks
            .world()
            .held(campsite::body(index))
            .map(|held| held.by),
        Some(3)
    );
    epoch + 1
}

#[test]
fn the_sticks_settle_on_their_log_and_then_cost_nothing() {
    let now = Instant::now();
    let mut sticks = Sticks::new(now);
    assert!(!sticks.world().moving());
    for index in 0..campsite::STICKS {
        let body = campsite::body(index);
        assert!(sticks.world().at_home(body, campsite::RACKED));
        // Where they were leant, to within a centimetre: the log holds them.
        let at = sticks.world().placement(body).unwrap();
        let leant = campsite::home(index);
        assert!(
            at.position.distance(leant.position) < 0.01,
            "{index}: {at:?}"
        );
        assert_eq!(sticks.toast(index), 0.0);
    }
    let shared = sticks.shared();
    assert_eq!(
        shared.sticks.bodies.len(),
        campsite::STICKS + campsite::LOGS
    );
    assert_eq!(shared.toast, [0.0; campsite::STICKS]);
    // None alight: the side keeps nothing, so a server started again begins with every marshmallow fresh and unlit.
    assert_eq!(shared.burning, [None; campsite::STICKS]);
    let (events, _) = run(&mut sticks, &[], now, 2.0);
    assert!(events.is_empty(), "{events:?}");
}

#[test]
fn the_first_to_take_a_stick_holds_it_and_its_marshmallow_toasts_where_their_hand_holds_it_in_the_heat()
 {
    let now = Instant::now();
    let mut sticks = Sticks::new(now);
    let epoch = taken(&mut sticks, 1, now);
    // Bea, taking it too, at the epoch she saw, is refused: the first to arrive holds it.
    let bea = holding(4, at_handle(&sticks, 2), now);
    assert!(sticks.ask(4, &take(1, 0), &[bea]));
    assert_eq!(
        sticks.world().held(campsite::body(1)).map(|held| held.by),
        Some(3)
    );
    // Held over the heart of the fire for ten seconds, it toasts half way, said at most twice a second.
    let over = holding(3, toasting(campsite::heart()), now);
    let (events, later) = run(&mut sticks, std::slice::from_ref(&over), now, 10.0);
    let toast = sticks.toast(1);
    assert!(
        (toast - campsite::TOAST_RATE * 10.0).abs() < 0.01,
        "{toast}"
    );
    let said: Vec<f32> = events
        .iter()
        .flat_map(|event| &event.toast)
        .map(|toasted| {
            assert_eq!(toasted.body, campsite::body(1));
            toasted.toast
        })
        .collect();
    assert!((18..=21).contains(&said.len()), "{}", said.len());
    assert!(said.windows(2).all(|pair| pair[1] > pair[0]));
    assert!((said.last().unwrap() - toast).abs() < 0.03);
    assert_eq!(sticks.shared().toast[1], toast);
    // Toasted only where the hand holds it: half a metre further off, slower, and beyond the heat, not at all.
    let edge = holding(3, toasting(campsite::heart() + Vec3::X * 0.45), later);
    let (_, later) = run(&mut sticks, &[edge], later, 2.0);
    let slower = sticks.toast(1) - toast;
    assert!(
        slower > 0.0 && slower < campsite::TOAST_RATE * 2.0 * 0.5,
        "{slower}"
    );
    let away = holding(3, toasting(campsite::heart() + Vec3::X * 1.0), later);
    let before = sticks.toast(1);
    // What toasted at the edge is said once more, then nothing.
    let (_, later) = run(&mut sticks, std::slice::from_ref(&away), later, 0.6);
    let (events, later) = run(&mut sticks, &[away], later, 3.0);
    assert_eq!(sticks.toast(1), before);
    assert!(events.iter().all(|event| event.toast.is_empty()));
    // Where the server has not seen their hand, as on the desktop, it is held before their head, as the desktop carries
    // it: toasted as they look into the fire from beside it.
    let head = campsite::heart() + Vec3::new(0.0, 0.2, 0.5 + 0.81);
    let desktop = Presence::new(
        3,
        tomori_protocol::Pose::at(head.to_array()),
        None,
        None,
        later,
    );
    let held = Held {
        by: 3,
        hand: Side::Right,
        offset: campsite::grip().encoded(Precision::Rest),
    };
    let before_eyes = hand(std::slice::from_ref(&desktop), &held).unwrap();
    let marshmallow = Pose::of(&held.placement(&before_eyes)).marshmallow();
    assert!(campsite::heat(marshmallow) > 0.5, "{marshmallow}");
    let before = sticks.toast(1);
    run(&mut sticks, &[desktop], later, 1.0);
    assert!(sticks.toast(1) > before);
    // Nobody seen at all, nothing toasts.
    assert_eq!(hand(&[], &held), None);
    let before = sticks.toast(1);
    let (_, later) = run(&mut sticks, &[], later, 1.0);
    assert_eq!(sticks.toast(1), before);
    // Held in the left hand, it toasts where the left hand holds it.
    let left = Held {
        hand: Side::Left,
        ..held
    };
    let over = toasting(campsite::heart());
    let seen = Presence::new(
        3,
        tomori_protocol::Pose::at(head.to_array()),
        Some(tomori_protocol::Pose::at(over.position.to_array())),
        None,
        later,
    );
    let in_left = hand(std::slice::from_ref(&seen), &left).unwrap();
    assert_eq!(in_left.position, over.position);
    // A hand said to be in the fire from farther than an arm's length from the head, or nowhere at all, toasts nothing.
    for (head, hand_at) in [
        (head + Vec3::X * 3.0, over.position),
        (head, Vec3::NAN),
        (Vec3::NAN, over.position),
    ] {
        let unsound = Presence::new(
            3,
            tomori_protocol::Pose::at(head.to_array()),
            Some(tomori_protocol::Pose::at(hand_at.to_array())),
            Some(tomori_protocol::Pose::at(hand_at.to_array())),
            later,
        );
        assert_eq!(hand(std::slice::from_ref(&unsound), &left), None);
        let before = sticks.toast(1);
        run(&mut sticks, &[unsound], later, 1.0);
        assert_eq!(sticks.toast(1), before);
    }
    let _ = epoch;
}

#[test]
fn a_stick_let_go_falls_and_comes_to_rest_where_it_lies_as_toasted_as_it_was() {
    let now = Instant::now();
    let mut sticks = Sticks::new(now);
    let epoch = taken(&mut sticks, 0, now);
    let over = holding(3, toasting(campsite::heart()), now);
    let (_, later) = run(&mut sticks, std::slice::from_ref(&over), now, 4.0);
    let toast = sticks.toast(0);
    assert!(toast > 0.15);
    // Let go a metre above the clearing's earth, beside the fire, it falls and lies on it.
    let earth = Vec2::new(FIRE.x, FIRE.z) + Vec2::new(0.4, 1.0);
    let at = Placement::new(
        Vec3::new(earth.x, 1.0, earth.y),
        glam::Quat::from_rotation_y(0.5),
    );
    let near = holding(3, at, later);
    assert!(sticks.ask(3, &let_go(0, epoch, at, [0.5, 0.0, 0.0]), &[near]));
    assert_eq!(sticks.world().held(campsite::body(0)), None);
    let (events, _) = rest(&mut sticks, later, 10.0);
    let lying = sticks.world().placement(campsite::body(0)).unwrap();
    let pose = Pose::of(&lying);
    // Lying on the earth, about level, a few centimetres up, no further than it might roll.
    assert!(pose.along.y.abs() < 0.1, "{pose:?}");
    let clearing = crate::trail::CLEARING_HEIGHT;
    assert!(
        lying.position.y > clearing && lying.position.y < clearing + 0.05,
        "{lying:?}"
    );
    assert!(Vec2::new(lying.position.x, lying.position.z).distance(earth) < 0.6);
    assert!(!sticks.world().at_home(campsite::body(0), campsite::RACKED));
    // Said moving, then resting; as toasted as it was.
    assert!(events.iter().any(|event| !event.sticks.moving.is_empty()));
    assert!(events.iter().any(|event| !event.sticks.rested.is_empty()));
    assert_eq!(sticks.toast(0), toast);
    // Anyone may take it from where it lies.
    let bea = holding(4, lying.then(&campsite::grip().inverse()), later);
    let epoch = sticks.world().epoch(campsite::body(0)).unwrap();
    assert!(sticks.ask(4, &take(0, epoch), &[bea]));
    assert_eq!(
        sticks.world().held(campsite::body(0)).map(|held| held.by),
        Some(4)
    );
}

/// Move `sticks` on from `now`, the farmyard's clock at `clock`, by `seconds`, in steps of the side's tick, with
/// `presence`: every event it said, and when, and at what clock, it ends.
fn burn(
    sticks: &mut Sticks,
    presence: &[Presence],
    now: Instant,
    clock: u64,
    seconds: f32,
) -> (Vec<Event>, Instant, u64) {
    let mut events = Vec::new();
    let (mut at, mut clock) = (now, clock);
    for _ in 0..(seconds / 0.05).round() as usize {
        at += Duration::from_millis(50);
        clock += 50;
        events.extend(sticks.update(presence, at, clock));
    }
    (events, at, clock)
}

/// What `events` said of stick `index` catching or going out, in order.
fn lit(events: &[Event], index: usize) -> Vec<Option<u64>> {
    events
        .iter()
        .flat_map(|event| &event.lit)
        .filter(|lit| lit.body == campsite::body(index))
        .map(|lit| lit.since)
        .collect()
}

#[test]
fn a_black_marshmallow_catches_in_the_heat_and_burns_on_out_of_it_for_everyone_until_it_goes_out() {
    let now = Instant::now();
    let mut sticks = Sticks::new(now);
    taken(&mut sticks, 1, now);
    // Just short of black, over the heart of the fire, it turns black within a tick and catches as it does, said at
    // once, as black as it is, since then.
    sticks.toast[1] = campsite::BLACK - 0.002;
    let over = holding(3, toasting(campsite::heart()), now);
    let (events, later, clock) = burn(&mut sticks, std::slice::from_ref(&over), now, 1_000, 0.05);
    assert!(sticks.toast(1) >= campsite::BLACK);
    assert_eq!(sticks.lit(1), Some(clock));
    assert_eq!(lit(&events, 1), [Some(clock)]);
    let said = events.iter().flat_map(|event| &event.toast).last().unwrap();
    assert_eq!(
        (said.body, said.toast),
        (campsite::body(1), sticks.toast(1))
    );
    assert_eq!(sticks.shared().burning[1], Some(clock));
    assert!(sticks.report().sticks[1].burning);
    assert_eq!(
        sticks.report().inspect("campsite/stick-1").unwrap()["burning"],
        true
    );
    let caught = clock;
    // Pulled out at once, it burns on, said nothing more, for as long as it may out of the heat, and then goes out,
    // said once.
    let away = holding(3, toasting(campsite::heart() + Vec3::X * 2.0), later);
    let (events, later, clock) = burn(
        &mut sticks,
        std::slice::from_ref(&away),
        later,
        clock,
        campsite::BURNS - 0.2,
    );
    assert!(lit(&events, 1).is_empty());
    assert_eq!(sticks.lit(1), Some(caught));
    let (events, later, clock) = burn(&mut sticks, std::slice::from_ref(&away), later, clock, 0.4);
    assert_eq!(lit(&events, 1), [None]);
    assert_eq!(sticks.lit(1), None);
    let out = events
        .iter()
        .find(|event| !event.lit.is_empty())
        .unwrap()
        .clock;
    let burned = (out - caught) as f32 / 1000.0;
    assert!((burned - campsite::BURNS).abs() <= 0.1, "{burned}");
    assert!(sticks.toast(1) >= campsite::BLACK);
    let (events, later, clock) = burn(&mut sticks, &[away], later, clock, 1.0);
    assert!(lit(&events, 1).is_empty());
    // Back in the heat, it catches again; and held under the pool's water, it goes out at once.
    let over = holding(3, toasting(campsite::heart()), later);
    let (events, later, clock) = burn(&mut sticks, &[over], later, clock, 0.05);
    assert_eq!(lit(&events, 1), [Some(clock)]);
    let under = crate::layout::POOL - Vec3::Y * 0.1;
    assert!(campsite::doused(under));
    let pool = holding(3, toasting(under), later);
    let (events, _, _) = burn(&mut sticks, &[pool], later, clock, 0.05);
    assert_eq!(lit(&events, 1), [None]);
}

#[test]
fn a_burning_marshmallow_let_go_burns_on_and_goes_out_in_the_pool_or_fresh_on_its_log() {
    let now = Instant::now();
    let mut sticks = Sticks::new(now);
    let epoch = taken(&mut sticks, 1, now);
    sticks.toast[1] = campsite::BLACK;
    let over = holding(3, toasting(campsite::heart()), now);
    let (_, later, clock) = burn(&mut sticks, &[over], now, 0, 0.05);
    assert!(sticks.lit(1).is_some());
    // Let go above the pool, it burns on as it falls, and goes out as it floats in, its marshmallow under the water.
    let pool = crate::layout::POOL;
    let above = Placement::at(Vec3::new(pool.x, pool.y + 0.6, pool.z + 0.5));
    let ada = holding(3, above, later);
    assert!(sticks.ask(3, &let_go(1, epoch, above, [0.0; 3]), &[ada]));
    let (events, later, clock) = burn(&mut sticks, &[], later, clock, 0.1);
    assert!(lit(&events, 1).is_empty());
    let (events, later, clock) = burn(&mut sticks, &[], later, clock, 10.0);
    assert_eq!(lit(&events, 1), [None]);
    // Another, alight, put back gently by its log, fades onto it fresh and unlit, said as it begins its way.
    let epoch = taken(&mut sticks, 2, later);
    sticks.toast[2] = campsite::BLACK;
    let over = holding(3, toasting(campsite::heart()), later);
    let (_, later, clock) = burn(&mut sticks, &[over], later, clock, 0.05);
    assert!(sticks.lit(2).is_some());
    let (middle, ..) = campsite::seat();
    let at = Placement::at(middle + Vec3::new(0.0, 0.9, 0.0));
    assert!(sticks.ask(3, &let_go(2, epoch, at, [0.0; 3]), &[holding(3, at, later)]));
    let (events, _, _) = burn(&mut sticks, &[], later, clock, 0.2);
    let began = events
        .iter()
        .find(|event| !event.sticks.returning.is_empty())
        .unwrap();
    assert!(
        began
            .lit
            .iter()
            .any(|lit| lit.body == campsite::body(2) && lit.since.is_none())
    );
    assert_eq!((sticks.lit(2), sticks.toast(2)), (None, 0.0));
}

#[test]
fn a_stick_passed_to_the_other_hand_stays_as_toasted_and_toasts_on_from_that_hand() {
    let now = Instant::now();
    let mut sticks = Sticks::new(now);
    let epoch = taken(&mut sticks, 0, now);
    let over = toasting(campsite::heart());
    let (_, later) = run(&mut sticks, &[holding(3, over, now)], now, 4.0);
    let toast = sticks.toast(0);
    assert!(toast > 0.15);
    // Their left hand, beside the right, grips it by the shaft, where it took it: passed, nothing let go, as toasted.
    let both = |left: Placement, right: Placement, at: Instant| {
        let pose = |hand: Placement| tomori_protocol::Pose {
            position: hand.position.to_array(),
            orientation: hand.rotation.to_array(),
        };
        Presence::new(
            3,
            tomori_protocol::Pose::at((right.position + Vec3::new(0.0, 0.5, 0.4)).to_array()),
            Some(pose(left)),
            Some(pose(right)),
            at,
        )
    };
    let beside = Placement::at(over.position + Vec3::new(0.0, 0.0, -0.15));
    let stick = campsite::grip();
    let offset = beside.inverse().then(&over.then(&stick));
    let pass = serde_json::to_string(&Intent::Take(Take {
        body: campsite::body(0),
        epoch,
        hand: Side::Left,
        offset: offset.encoded(Precision::Rest),
    }))
    .unwrap();
    assert!(sticks.ask(3, &pass, &[both(beside, over, later)]));
    assert_eq!(
        sticks
            .world()
            .held(campsite::body(0))
            .map(|held| (held.by, held.hand)),
        Some((3, Side::Left))
    );
    assert_eq!(sticks.toast(0), toast);
    // Held over the fire by the left hand alone, the right gone elsewhere, it toasts on.
    let elsewhere = Placement::at(over.position + Vec3::new(0.6, -0.3, 0.3));
    run(&mut sticks, &[both(beside, elsewhere, later)], later, 2.0);
    assert!(sticks.toast(0) > toast, "{}", sticks.toast(0));
}

#[test]
fn a_stick_put_back_by_its_log_fades_onto_it_and_has_a_fresh_marshmallow() {
    let now = Instant::now();
    let mut sticks = Sticks::new(now);
    let epoch = taken(&mut sticks, 2, now);
    let over = holding(3, toasting(campsite::heart()), now);
    let (_, later) = run(&mut sticks, std::slice::from_ref(&over), now, 8.0);
    assert!(sticks.toast(2) > campsite::GOLDEN);
    // Let go just above its log, it fades out there and in on it, its marshmallow fresh, said as it begins its way.
    let (middle, ..) = campsite::seat();
    let at = Placement::at(middle + Vec3::new(0.0, 0.9, 0.0));
    let near = holding(3, at, later);
    assert!(sticks.ask(3, &let_go(2, epoch, at, [0.0; 3]), &[near]));
    let (events, _) = rest(&mut sticks, later, 5.0);
    assert!(sticks.world().at_home(campsite::body(2), campsite::RACKED));
    assert_eq!(sticks.toast(2), 0.0);
    let began = events
        .iter()
        .find(|event| {
            event
                .sticks
                .returning
                .iter()
                .any(|returning| returning.body == campsite::body(2))
        })
        .unwrap();
    assert!(
        began
            .toast
            .iter()
            .any(|toasted| toasted.body == campsite::body(2) && toasted.toast == 0.0)
    );
    // The others never moved.
    for index in [0, 1, 3] {
        assert!(
            sticks
                .world()
                .at_home(campsite::body(index), campsite::RACKED)
        );
    }
}

#[test]
fn a_stick_dropped_in_the_fire_or_thrown_far_away_goes_back_on_its_log() {
    let now = Instant::now();
    let mut sticks = Sticks::new(now);
    let epoch = taken(&mut sticks, 3, now);
    let at = Placement::at(campsite::heart() + Vec3::Y * 0.3);
    let over = holding(3, at, now);
    assert!(sticks.ask(3, &let_go(3, epoch, at, [0.0; 3]), &[over]));
    let (_, now) = rest(&mut sticks, now, 15.0);
    assert!(sticks.world().at_home(campsite::body(3), campsite::RACKED));
    // Thrown away from the fire as hard as a hand may, it comes to rest beyond the campsite and goes back.
    let from = at_handle(&sticks, 3).then(&campsite::grip());
    let epoch = taken(&mut sticks, 3, now);
    let ada = holding(3, from, now);
    let velocity = (Vec3::new(-1.0, 0.6, 0.0).normalize() * 8.0).to_array();
    assert!(sticks.ask(3, &let_go(3, epoch, from, velocity), &[ada]));
    let mut far = 0.0f32;
    let mut at = now;
    for _ in 0..400 {
        at += Duration::from_millis(50);
        sticks.update(&[], at, 0);
        if let Some(placement) = sticks.world().placement(campsite::body(3)) {
            far = far.max(
                Vec2::new(placement.position.x, placement.position.z)
                    .distance(Vec2::new(FIRE.x, FIRE.z)),
            );
        }
        if !sticks.world().moving() {
            break;
        }
    }
    assert!(far > campsite::DROP_REACH, "{far}");
    assert!(sticks.world().at_home(campsite::body(3), campsite::RACKED));
}

#[test]
fn a_stick_let_go_in_the_pool_floats_and_goes_back_on_its_log_after_a_while() {
    let now = Instant::now();
    let mut sticks = Sticks::new(now);
    let epoch = taken(&mut sticks, 1, now);
    let pool = crate::layout::POOL;
    let over = Placement::at(Vec3::new(pool.x, pool.y + 0.6, pool.z + 0.5));
    let ada = Presence::new(
        3,
        tomori_protocol::Pose::at((over.position + Vec3::new(0.0, 0.6, 0.8)).to_array()),
        None,
        Some(tomori_protocol::Pose::at(over.position.to_array())),
        now,
    );
    assert!(sticks.ask(3, &let_go(1, epoch, over, [0.0; 3]), &[ada]));
    // It floats, at the water's surface, rather than sinking to the pool's bed, and comes to rest there, soaking.
    let (events, later) = run(&mut sticks, &[], now, 10.0);
    assert!(events.iter().any(|event| {
        event
            .sticks
            .rested
            .iter()
            .any(|rested| rested.body() == campsite::body(1))
    }));
    assert!(sticks.world().moving());
    let floating = sticks.world().placement(campsite::body(1)).unwrap();
    assert!(
        (floating.position.y - pool.y).abs() < 0.03,
        "{floating:?}, the water at {}",
        pool.y
    );
    assert!(campsite::in_pool(&floating));
    // Its marshmallow, the end it floats down by, is under the water, so a burning one goes out.
    assert!(campsite::doused(
        campsite::Pose::of(&floating).marshmallow()
    ));
    assert!(sticks.report().sticks[1].floating);
    // A minute later, it fades back to its log.
    let (_, later) = run(&mut sticks, &[], later, (campsite::RETURNS / 1000) as f32);
    rest(&mut sticks, later, 10.0);
    assert!(sticks.world().at_home(campsite::body(1), campsite::RACKED));
}

#[test]
fn a_stick_held_by_someone_who_leaves_drops_where_their_hand_was() {
    let now = Instant::now();
    let mut sticks = Sticks::new(now);
    taken(&mut sticks, 0, now);
    let earth = Vec2::new(FIRE.x, FIRE.z) + Vec2::new(-0.2, 1.1);
    let hand = toasting(Vec3::new(earth.x, 1.0, earth.y));
    let ada = holding(3, hand, now);
    // Until the server says who is there, and while they are, nothing changes.
    sticks.sweep(None, std::slice::from_ref(&ada), now);
    sticks.sweep(Some(&[3]), std::slice::from_ref(&ada), now);
    assert!(sticks.world().held(campsite::body(0)).is_some());
    // Gone, it is let go where their hand was, and falls.
    sticks.sweep(Some(&[]), &[ada], now);
    assert_eq!(sticks.world().held(campsite::body(0)), None);
    rest(&mut sticks, now, 10.0);
    let lying = sticks.world().placement(campsite::body(0)).unwrap();
    assert!(lying.position.y < 0.1, "{lying:?}");
}

#[test]
fn nobody_unseen_takes_a_stick_and_nothing_is_taken_of_the_campsite_s_own_old_intents() {
    let now = Instant::now();
    let mut sticks = Sticks::new(now);
    // Unseen, as a visitor, whose client sends no presence, and whose intents the server refuses before the side.
    assert!(sticks.ask(5, &take(0, 0), &[]));
    assert_eq!(sticks.world().held(campsite::body(0)), None);
    // Seen far from it, the same.
    let far = holding(5, Placement::at(FIRE + Vec3::new(6.0, 1.0, 0.0)), now);
    assert!(sticks.ask(5, &take(0, 0), &[far]));
    assert_eq!(sticks.world().held(campsite::body(0)), None);
    // The campsite's own intents, by which a client once took a stick from anywhere and said how toasted it was, are
    // not the sticks' to take, nor are the stones'.
    let ada = holding(3, at_handle(&sticks, 0), now);
    for old in [
        r#"{"grab":0,"epoch":0}"#,
        r#"{"toast":0,"epoch":1,"level":0.9}"#,
        r#"{"release":0,"epoch":1,"level":0.9,"at":[0,0,0],"yaw":0}"#,
    ] {
        assert!(!sticks.ask(3, old, std::slice::from_ref(&ada)), "{old}");
    }
    let stone = serde_json::to_string(&Intent::Take(Take {
        body: 2,
        epoch: 0,
        hand: Side::Right,
        offset: Encoded::default(),
    }))
    .unwrap();
    assert!(!sticks.ask(3, &stone, &[ada]));
    assert!(sticks.world().holders().is_empty());
    assert!((0..campsite::STICKS).all(|index| sticks.toast(index) == 0.0));
}

#[test]
fn an_empty_campsite_puts_its_sticks_back_after_an_hour() {
    let now = Instant::now();
    let mut sticks = Sticks::new(now);
    let epoch = taken(&mut sticks, 2, now);
    let earth = Vec2::new(FIRE.x, FIRE.z) + Vec2::new(0.6, 0.9);
    let at = Placement::at(Vec3::new(earth.x, 0.4, earth.y));
    let ada = holding(3, at, now);
    assert!(sticks.ask(
        3,
        &let_go(2, epoch, at, [0.0; 3]),
        std::slice::from_ref(&ada)
    ));
    let (_, later) = rest(&mut sticks, now, 10.0);
    assert!(!sticks.world().at_home(campsite::body(2), campsite::RACKED));
    // Nobody there; not a moment before the hour, nor if someone came back meanwhile.
    sticks.sweep(Some(&[]), &[], later);
    sticks.sweep(
        Some(&[]),
        &[],
        later + campsite::TIDY - Duration::from_secs(1),
    );
    sticks.sweep(
        Some(&[7]),
        &[],
        later + campsite::TIDY - Duration::from_millis(500),
    );
    sticks.sweep(Some(&[]), &[], later + campsite::TIDY);
    assert!(!sticks.world().moving());
    let empty = later + campsite::TIDY * 2;
    sticks.sweep(Some(&[]), &[], empty);
    assert!(sticks.world().moving());
    rest(&mut sticks, empty, 10.0);
    assert!(sticks.world().at_home(campsite::body(2), campsite::RACKED));
}

#[test]
fn the_agent_is_told_of_the_bonfire_the_fireside_log_and_each_stick_and_may_inspect_them() {
    let now = Instant::now();
    let mut sticks = Sticks::new(now);
    taken(&mut sticks, 2, now);
    let report = sticks.report();
    let objects = report.describe();
    assert_eq!(objects.len(), campsite::STICKS + 3);
    let (id, reference, description, at) = &objects[0];
    assert_eq!(
        (id.as_str(), reference.as_str()),
        ("bonfire", "campsite/bonfire")
    );
    assert!(description.contains("bonfire"));
    assert_eq!(*at, Some(campsite::heart()));
    // The woodpile, every log on it.
    let (id, reference, description, _) = &objects[1];
    assert_eq!(
        (id.as_str(), reference.as_str()),
        ("woodpile", "campsite/woodpile")
    );
    assert!(description.contains(&format!(
        "{} of its {} logs",
        campsite::LOGS,
        campsite::LOGS
    )));
    // The fireside log, on its seat, which pointing at seats one.
    let (id, reference, description, at) = &objects[2];
    assert_eq!(
        (id.as_str(), reference.as_str()),
        ("fireside-log", "campsite/fireside-log")
    );
    assert!(tomori_environment::valid_reference(reference));
    assert!(description.contains("seat") && description.contains("pointing at it"));
    let (seat, _, _, radius) = campsite::fireside();
    assert_eq!(*at, Some(seat + Vec3::Y * campsite::seat_top(radius)));
    let log = report.inspect("campsite/fireside-log").unwrap();
    assert_eq!(log["kind"], "log seat");
    for (index, (id, reference, description, at)) in objects[3..].iter().enumerate() {
        assert_eq!(*id, format!("stick-{index}"));
        assert_eq!(*reference, format!("campsite/stick-{index}"));
        assert!(tomori_environment::valid_reference(reference));
        assert!(description.contains("marshmallow"));
        // Where it rests, but not in a hand, which only presence says.
        if index == 2 {
            assert!(description.contains("held"));
            assert_eq!(*at, None);
        } else {
            assert!(description.contains("leaning on the log"));
            let home = sticks.world().home(campsite::body(index)).unwrap();
            assert!(at.unwrap().distance(Pose::of(&home).marshmallow()) < 0.01);
        }
    }
    let lying = Report {
        sticks: vec![Said::default()],
        ..Report::default()
    };
    assert!(lying.describe()[3].2.contains("lying"));
    let floating = Report {
        sticks: vec![Said {
            floating: true,
            ..Said::default()
        }],
        ..Report::default()
    };
    assert!(floating.describe()[3].2.contains("floating in the pool"));
    let bonfire = report.inspect("campsite/bonfire").unwrap();
    assert_eq!(bonfire["kind"], "bonfire");
    let stick = report.inspect("campsite/stick-2").unwrap();
    assert_eq!(stick["looks"], "fresh and white");
    assert_eq!(stick["burning"], false);
    assert_eq!(stick["held_by"], 3);
    assert_eq!(stick["on_its_log"], false);
    assert_eq!(
        report.inspect("campsite/stick-1").unwrap()["on_its_log"],
        true
    );
    assert_eq!(report.inspect("campsite/stick-9"), None);
    assert_eq!(report.inspect("campsite/stick-x"), None);
    assert_eq!(report.inspect("hen/pip"), None);
    // Its takings are logged as sticks and logs.
    assert_eq!((NOUN.one)(campsite::body(3)), "stick 3");
    assert_eq!((NOUN.one)(campsite::log_body(5)), "log 5");
    assert_eq!((NOUN.one)(7), "body 7");
    assert_eq!(
        sticks.loose().world().bodies().count(),
        campsite::STICKS + campsite::LOGS
    );
}

fn take_log(index: usize, epoch: u32) -> String {
    serde_json::to_string(&Intent::Take(Take {
        body: campsite::log_body(index),
        epoch,
        hand: Side::Right,
        offset: Placement::IDENTITY.encoded(Precision::Rest),
    }))
    .unwrap()
}

fn let_go_log(index: usize, epoch: u32, at: Placement) -> String {
    serde_json::to_string(&Intent::LetGo(LetGo {
        body: campsite::log_body(index),
        epoch,
        at: at.encoded(Precision::Rest),
        velocity: [0.0; 3],
        spin: [0.0; 3],
        clock: 0,
    }))
    .unwrap()
}

/// Log `index`, taken from where it lies by participant 3's right hand there, at `now`, and let go at `at`, unmoving,
/// their hand there.
fn carried(sticks: &mut Sticks, index: usize, at: Placement, now: Instant) {
    let body = campsite::log_body(index);
    let lies = sticks.world().placement(body).unwrap();
    let epoch = sticks.world().epoch(body).unwrap();
    assert!(sticks.ask(3, &take_log(index, epoch), &[holding(3, lies, now)]));
    assert_eq!(sticks.world().held(body).map(|held| held.by), Some(3));
    assert!(sticks.ask(3, &let_go_log(index, epoch + 1, at), &[holding(3, at, now)]));
    assert_eq!(sticks.world().held(body), None);
}

/// Over the fire, a little above its embers, lying across it.
fn over_the_fire(turn: f32) -> Placement {
    Placement::new(FIRE + Vec3::Y * 0.35, Quat::from_rotation_y(turn))
}

#[test]
fn the_woodpile_s_logs_rest_stacked_where_they_were_laid_and_never_creep_nor_tip() {
    let now = Instant::now();
    let mut sticks = Sticks::new(now);
    assert!(!sticks.world().moving());
    // Each log where it was laid, crossed on the pair beneath, to within a few millimetres and a degree.
    for index in 0..campsite::LOGS {
        let body = campsite::log_body(index);
        let at = sticks.world().placement(body).unwrap();
        let laid = campsite::log_home(index);
        assert!(
            at.position.distance(laid.position) < 0.005,
            "{index}: {at:?} {laid:?}"
        );
        assert!(at.rotation.angle_between(laid.rotation) < 0.02, "{index}");
        assert!(sticks.world().at_home(body, campsite::RACKED));
        assert_eq!(sticks.burnt(index), 0.0);
    }
    // For ten simulated minutes nothing moves and nothing is said: every log stays put to the bit.
    let before: Vec<Placement> = (0..campsite::LOGS)
        .map(|index| sticks.world().placement(campsite::log_body(index)).unwrap())
        .collect();
    let mut at = now;
    for _ in 0..600 {
        at += Duration::from_secs(1);
        assert_eq!(sticks.update(&[], at, 0), None);
    }
    for (index, placement) in before.iter().enumerate() {
        let after = sticks.world().placement(campsite::log_body(index)).unwrap();
        assert_eq!(after, *placement, "{index}");
    }
    // Its stone and its logs, solid, beyond the ring participants walk round the fire.
    let report = sticks.report();
    assert_eq!(
        (report.piled, report.burning, report.elsewhere),
        (campsite::LOGS, 0, 0)
    );
    assert_eq!(sticks.size(), 0.0);
}

#[test]
fn a_log_put_in_the_fire_burns_making_the_fire_bigger_up_to_its_biggest_and_nobody_takes_it_back() {
    let now = Instant::now();
    let mut sticks = Sticks::new(now);
    // The top log, let go over the fire, falls into it, catches, and feeds it.
    carried(&mut sticks, 7, over_the_fire(0.3), now);
    let (events, now) = rest(&mut sticks, now, 10.0);
    assert!(sticks.burnt(7) > 0.0);
    assert!(events.iter().any(|event| {
        event
            .burnt
            .iter()
            .any(|burnt| burnt.body == campsite::log_body(7) && burnt.burnt > 0.0)
    }));
    let report = sticks.report();
    assert_eq!((report.piled, report.burning), (campsite::LOGS - 1, 1));
    assert!(report.describe()[0].2.contains("a log burning on it"));
    // Burning, it is nobody's to take, nor let go of, as nobody holds it.
    let lies = sticks.world().placement(campsite::log_body(7)).unwrap();
    let epoch = sticks.world().epoch(campsite::log_body(7)).unwrap();
    assert!(sticks.ask(3, &take_log(7, epoch), &[holding(3, lies, now)]));
    assert_eq!(sticks.world().held(campsite::log_body(7)), None);
    assert!(sticks.ask(3, &let_go_log(7, epoch, lies), &[holding(3, lies, now)]));
    assert_eq!(sticks.world().epoch(campsite::log_body(7)), Some(epoch));
    // Over the time it catches, the fire grows by its fuel, less what it dies back meanwhile, and says so as it does.
    let (events, now) = run(&mut sticks, &[], now, campsite::CATCH + 1.0);
    let grown = sticks.size();
    let most = campsite::FUEL - campsite::CATCH / campsite::DIES_BACK;
    assert!(grown > most - 0.08 && grown <= most + 1e-3, "{grown}");
    let said: Vec<f32> = events.iter().filter_map(|event| event.size).collect();
    assert!(
        said.len() > 5 && said.windows(2).all(|pair| pair[1] > pair[0]),
        "{said:?}"
    );
    assert!((said.last().unwrap() - grown).abs() < campsite::FUEL * 0.1);
    // Three more logs, one after another, make it as big as it grows, and no bigger.
    let (mut now, mut events) = (now, Vec::new());
    for (index, turn) in [(6, 1.2), (5, 2.0), (4, 2.8)] {
        carried(&mut sticks, index, over_the_fire(turn), now);
        let (rested, at) = rest(&mut sticks, now, 10.0);
        assert!(sticks.burnt(index) > 0.0, "{index}");
        let (ran, at) = run(&mut sticks, &[], at, 2.0);
        events.extend(rested.into_iter().chain(ran));
        now = at;
    }
    let (ran, now) = run(&mut sticks, &[], now, campsite::CATCH);
    events.extend(ran);
    let said: Vec<f32> = events.iter().filter_map(|event| event.size).collect();
    assert_eq!(said.iter().copied().fold(0.0, f32::max), 1.0, "{said:?}");
    assert!(sticks.size() > 0.98);
    assert_eq!(sticks.shared().size, sticks.size());
    let report = sticks.report();
    assert_eq!(report.burning, 4);
    let bonfire = &report.describe()[0].2;
    assert!(bonfire.contains("blazing") && bonfire.contains("4 logs burning on it"));
    let inspected = report.inspect("campsite/woodpile").unwrap();
    assert_eq!(
        (&inspected["on_the_pile"], &inspected["burning"]),
        (
            &serde_json::json!(campsite::LOGS - 4),
            &serde_json::json!(4)
        )
    );
    assert_eq!(
        report.inspect("campsite/bonfire").unwrap()["looks"],
        "blazing"
    );
    // A marshmallow held well beyond where a fire at its default toasts it toasts by the big fire.
    let far = campsite::heart_at(1.0) + Vec3::X * 0.9;
    assert_eq!(campsite::heat(far), 0.0);
    let epoch = taken(&mut sticks, 0, now);
    let _ = epoch;
    let hand = toasting(far);
    let ada = holding(3, hand, now);
    (_, _) = run(&mut sticks, &[ada], now, 2.0);
    assert!(sticks.toast(0) > 0.0);
}

#[test]
fn a_burnt_log_comes_back_to_the_woodpile_and_the_fire_dies_back_to_its_default() {
    let now = Instant::now();
    let mut sticks = Sticks::new(now);
    carried(&mut sticks, 6, over_the_fire(1.0), now);
    let (_, now) = rest(&mut sticks, now, 10.0);
    assert!(sticks.burnt(6) > 0.0);
    // It chars and shrinks as it burns, said as it goes, and once burnt away fades back onto the woodpile, fresh.
    let (events, now) = run(&mut sticks, &[], now, campsite::BURN);
    let burnt: Vec<f32> = events
        .iter()
        .flat_map(|event| &event.burnt)
        .map(|burnt| burnt.burnt)
        .collect();
    assert!(burnt.len() > 20, "{burnt:?}");
    let returning: Vec<_> = events
        .iter()
        .flat_map(|event| &event.sticks.returning)
        .collect();
    assert_eq!(returning.len(), 1);
    assert_eq!(returning[0].body, campsite::log_body(6));
    assert_eq!(*burnt.last().unwrap(), 0.0);
    let (_, now) = rest(&mut sticks, now, 5.0);
    assert_eq!(sticks.burnt(6), 0.0);
    let report = sticks.report();
    assert_eq!((report.piled, report.burning), (campsite::LOGS, 0));
    // The fire dies back to its default over a few minutes, without more wood, and says so, then nothing more.
    assert!(sticks.size() > 0.0);
    let (events, now) = run(&mut sticks, &[], now, campsite::DIES_BACK * campsite::FUEL);
    assert_eq!(sticks.size(), 0.0);
    assert_eq!(
        events.iter().filter_map(|event| event.size).next_back(),
        Some(0.0)
    );
    let (events, _) = run(&mut sticks, &[], now, 5.0);
    assert!(events.is_empty(), "{events:?}");
}

#[test]
fn a_log_lying_about_the_fire_stays_and_one_carried_off_goes_back_on_the_woodpile() {
    let now = Instant::now();
    let mut sticks = Sticks::new(now);
    // Put down on the clearing beside the fire, it lies there for anyone to put on it.
    let beside = Placement::at(FIRE + Vec3::new(1.0, 0.3, 0.6));
    carried(&mut sticks, 7, beside, now);
    let (_, now) = rest(&mut sticks, now, 10.0);
    let lies = sticks.world().placement(campsite::log_body(7)).unwrap();
    assert!(lies.position.distance(beside.position) < 0.6);
    assert_eq!(sticks.burnt(7), 0.0);
    assert_eq!(sticks.report().elsewhere, 1);
    let (events, now) = run(&mut sticks, &[], now, 3.0);
    assert!(events.is_empty());
    // Carried off along the trail, beyond reach of the fire, it fades back onto the woodpile.
    let away = Placement::at(FIRE + Vec3::new(campsite::DROP_REACH + 1.0, 0.4, 1.5));
    carried(&mut sticks, 6, away, now);
    let (events, _) = rest(&mut sticks, now, 10.0);
    assert!(
        events
            .iter()
            .flat_map(|event| &event.sticks.returning)
            .any(|returning| returning.body == campsite::log_body(6))
    );
    let report = sticks.report();
    assert_eq!((report.piled, report.elsewhere), (campsite::LOGS - 1, 1));
}
