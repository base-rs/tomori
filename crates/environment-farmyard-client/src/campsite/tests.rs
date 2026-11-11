// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use glam::Quat;

use super::*;

/// The sticks, as the server side shares them fresh, with `change` made.
fn shared(change: impl FnOnce(&mut Vec<Stick>)) -> Shared {
    let mut sticks = vec![Stick::default(); STICKS];
    change(&mut sticks);
    Shared { sticks }
}

/// A hand holding a stick by its handle, as it lies racked.
fn hand_on(index: usize) -> Hand {
    let pose = Pose::racked(index);
    Hand::new(
        pose.butt + pose.along * campsite::HOLD,
        Quat::from_rotation_arc(Vec3::NEG_Z, pose.along),
    )
}

/// A hand holding a stick's marshmallow over the heart of the fire.
fn over_the_fire() -> Hand {
    let along = Vec3::new(1.0, 0.3, 0.0).normalize();
    let hand = campsite::heart()
        - along * (campsite::STICK_LENGTH - campsite::MARSHMALLOW_INSET - campsite::HOLD);
    Hand::new(hand, Quat::from_rotation_arc(Vec3::NEG_Z, along))
}

#[test]
fn a_ray_passing_near_a_free_stick_takes_it_with_the_grip_and_the_bonfire_is_pointed_at() {
    let mut sticks = Sticks::new();
    let at = |pose: Pose| Ray::new(pose.middle() + Vec3::new(0.02, 1.5, 0.0), Vec3::NEG_Y).unwrap();
    // Nothing before the server side says what there is.
    assert_eq!(sticks.pick(&at(Pose::racked(1))), None);
    sticks.share(shared(|_| {}));
    let picked = sticks.pick(&at(Pose::racked(1))).unwrap();
    assert_eq!(picked.object, "stick-1");
    assert!(picked.actions.contains(Actions::GRAB));
    assert!(!picked.selectable);
    // The bonfire, pointed at, takes nothing.
    let fire = Ray::new(campsite::heart() + Vec3::new(0.0, 0.2, 3.0), Vec3::NEG_Z).unwrap();
    let picked = sticks.pick(&fire).unwrap();
    assert_eq!(picked.object, "bonfire");
    assert_eq!(picked.actions, Actions::NONE);
    // A stick another holds is theirs, and is not pointed at to take.
    sticks.share(shared(|sticks| sticks[1].held = Some(9)));
    assert_ne!(
        sticks
            .pick(&at(Pose::racked(1)))
            .map(|picked| picked.object),
        Some("stick-1".to_owned())
    );
    assert_eq!(sticks.within_reach(hand_on(1).position), None);
    // A hand at a free stick's handle reaches it, the nearest; nowhere near, none.
    assert_eq!(
        sticks.within_reach(hand_on(2).position).as_deref(),
        Some("stick-2")
    );
    assert_eq!(sticks.within_reach(Vec3::new(0.0, 1.0, 0.0)), None);
}

#[test]
fn a_stick_taken_is_held_in_the_hand_toasted_over_the_fire_and_let_go_where_the_server_side_will_rest_it()
 {
    let mut sticks = Sticks::new();
    sticks.share(shared(|_| {}));
    let start = Instant::now();
    let me = Some(3);
    // Taken, the server side is asked for it at the epoch seen, and it is drawn in the hand at once.
    let grab = |hand: Hand, phase| Grab::new("stick-2", hand, phase);
    let intents = sticks.update(Some(&grab(hand_on(2), Phase::Began)), me, start);
    assert_eq!(intents, [Intent::Grab { grab: 2, epoch: 0 }]);
    let shown = sticks.shown(&[]);
    let held = Pose::held(hand_on(2).position, hand_on(2).orientation);
    assert_eq!(shown[2].transform, held.transform());
    // Not taken twice.
    assert!(
        sticks
            .update(Some(&grab(hand_on(2), Phase::Began)), me, start)
            .is_empty()
    );
    // The server side agrees.
    sticks.changed(Event {
        clock: 1,
        stick: 2,
        now: Stick {
            epoch: 1,
            held: Some(3),
            ..Stick::default()
        },
    });
    // Held over the fire, it toasts, frame by frame, and says how toasted at most twice a second.
    let mut said = Vec::new();
    let mut now = start;
    for _ in 0..300 {
        now += Duration::from_millis(20);
        said.extend(sticks.update(Some(&grab(over_the_fire(), Phase::Changed)), me, now));
    }
    let levels: Vec<f32> = said
        .iter()
        .map(|intent| match intent {
            Intent::Toast {
                toast: 2,
                epoch: 1,
                level,
            } => *level,
            other => panic!("{other:?}"),
        })
        .collect();
    assert!((10..=13).contains(&levels.len()), "{levels:?}");
    assert!(levels.windows(2).all(|pair| pair[1] > pair[0]));
    // Six seconds over the heart: golden, as this participant sees it at once, and so the server side is told.
    let toast = *levels.last().unwrap();
    assert!(
        toast > campsite::GOLDEN - 0.05 && toast < campsite::BROWN,
        "{toast}"
    );
    let shown = sticks.shown(&[]);
    assert!(shown[2].shade >= campsite::shade(campsite::GOLDEN) - 1);
    assert!(!shown[2].burning);
    // Let go on the clearing's earth, it is asked to rest there, and foreseen there, until the server side agrees.
    let earth = Hand::new(
        campsite::FIRE + Vec3::new(0.6, 1.0, 0.9),
        Quat::from_rotation_y(0.5),
    );
    let intents = sticks.update(Some(&grab(earth, Phase::Ended)), me, now);
    let pose = Pose::held(earth.position, earth.orientation);
    let [
        Intent::Release {
            release: 2,
            epoch: 1,
            level,
            at,
            yaw,
        },
    ] = intents[..]
    else {
        panic!("{intents:?}");
    };
    assert!((level - sticks.letting[2].unwrap().toast).abs() < 1e-6);
    assert_eq!((at, yaw), (pose.middle().to_array(), pose.yaw()));
    let lying = campsite::rest(at, yaw).unwrap();
    assert_eq!(
        sticks.shown(&[])[2].transform,
        Pose::lying(&lying).transform()
    );
    // Nothing more is said of it, held or let go, until it is taken again, nor may it be taken until the server agrees.
    assert!(
        sticks
            .update(Some(&grab(earth, Phase::Changed)), me, now)
            .is_empty()
    );
    assert!(
        sticks
            .update(Some(&grab(earth, Phase::Ended)), me, now)
            .is_empty()
    );
    assert_eq!(sticks.within_reach(earth.position), None);
    sticks.changed(Event {
        clock: 2,
        stick: 2,
        now: Stick {
            epoch: 2,
            toast: level,
            held: None,
            lying: Some(lying),
        },
    });
    assert!(sticks.letting[2].is_none());
    assert_eq!(
        sticks.shown(&[])[2].transform,
        Pose::lying(&lying).transform()
    );
}

#[test]
fn a_marshmallow_left_in_the_fire_catches_and_a_stick_put_back_on_its_log_is_fresh() {
    let mut sticks = Sticks::new();
    sticks.share(shared(|sticks| sticks[0].toast = campsite::BROWN));
    let start = Instant::now();
    let grab = |hand: Hand, phase| Grab::new("stick-0", hand, phase);
    sticks.update(Some(&grab(hand_on(0), Phase::Began)), Some(1), start);
    sticks.changed(Event {
        clock: 1,
        stick: 0,
        now: Stick {
            epoch: 1,
            toast: campsite::BROWN,
            held: Some(1),
            lying: None,
        },
    });
    let mut now = start;
    for _ in 0..400 {
        now += Duration::from_millis(25);
        sticks.update(Some(&grab(over_the_fire(), Phase::Changed)), Some(1), now);
    }
    // Black, in the heat, it burns; drawn out of the heat, it does not.
    assert!(sticks.shown(&[])[0].burning);
    assert_eq!(sticks.shown(&[])[0].shade, campsite::SHADES - 1);
    sticks.update(Some(&grab(hand_on(1), Phase::Changed)), Some(1), now);
    assert!(!sticks.shown(&[])[0].burning);
    // Put back on its log, it is foreseen there with a fresh marshmallow.
    let (middle, ..) = campsite::seat();
    let log = Hand::new(middle + Vec3::Y * 0.6, Quat::from_rotation_y(1.2));
    sticks.update(Some(&grab(log, Phase::Ended)), Some(1), now);
    let shown = sticks.shown(&[]);
    assert_eq!(shown[0].transform, Pose::racked(0).transform());
    assert_eq!(shown[0].shade, 0);
}

#[test]
fn two_taking_one_stick_the_loser_lets_it_go_and_sees_it_in_the_winner_s_hand() {
    let mut sticks = Sticks::new();
    sticks.share(shared(|_| {}));
    let start = Instant::now();
    let grab = |hand: Hand, phase| Grab::new("stick-1", hand, phase);
    // Bea takes it as Ada does; the server side says Ada was first.
    let bea = Some(2);
    sticks.update(Some(&grab(hand_on(1), Phase::Began)), bea, start);
    sticks.share(shared(|sticks| {
        sticks[1].epoch = 1;
        sticks[1].held = Some(1);
    }));
    // Bea holds nothing: she says nothing of it, and lets go of nothing.
    let later = start + Duration::from_secs(1);
    let changed = sticks.update(Some(&grab(over_the_fire(), Phase::Changed)), bea, later);
    assert!(changed.is_empty());
    assert!(
        sticks
            .update(Some(&grab(hand_on(1), Phase::Ended)), bea, later)
            .is_empty()
    );
    // It is drawn in Ada's hand, from her presence; and where it was last drawn, should her hand go unreported.
    let ada = over_the_fire();
    let shown = sticks.shown(&[(1, ada)]);
    let in_her_hand = Pose::held(ada.position, ada.orientation);
    assert_eq!(shown[1].transform, in_her_hand.transform());
    assert_eq!(sticks.shown(&[])[1].transform, in_her_hand.transform());
    assert_eq!(
        sticks.shown(&[(7, hand_on(3))])[1].transform,
        in_her_hand.transform()
    );
    // Before it is ever drawn in a hand, it is drawn where it rests.
    let mut unseen = Sticks::new();
    unseen.share(shared(|sticks| sticks[0].held = Some(1)));
    assert_eq!(unseen.shown(&[])[0].transform, Pose::racked(0).transform());
}

#[test]
fn a_grab_the_server_side_never_agrees_to_is_given_up_and_a_visitor_takes_nothing() {
    let mut sticks = Sticks::new();
    sticks.share(shared(|_| {}));
    let start = Instant::now();
    let grab = |phase| Grab::new("stick-3", hand_on(3), phase);
    // A visitor takes nothing, nor does a grab of what is not a stick, or not one there is.
    assert!(
        sticks
            .update(Some(&grab(Phase::Began)), None, start)
            .is_empty()
    );
    for object in ["hen-1", "stick-9", "stick-x"] {
        let other = Grab::new(object, hand_on(3), Phase::Began);
        assert!(sticks.update(Some(&other), Some(1), start).is_empty());
    }
    // Taken, but never agreed to, as when its intent was dropped, it is given up after a moment and drawn where it rests.
    assert_eq!(
        sticks
            .update(Some(&grab(Phase::Began)), Some(1), start)
            .len(),
        1
    );
    sticks.update(None, Some(1), start + Duration::from_millis(1400));
    assert!(sticks.holding.is_some());
    sticks.update(None, Some(1), start + Duration::from_millis(1600));
    assert!(sticks.holding.is_none());
    assert_eq!(sticks.shown(&[])[3].transform, Pose::racked(3).transform());
    // Agreed and then taken back by the server side, as when this participant was gone too long, it is given up too.
    sticks.update(Some(&grab(Phase::Began)), Some(1), start);
    sticks.share(shared(|sticks| {
        sticks[3].epoch = 1;
        sticks[3].held = Some(1);
    }));
    assert!(sticks.holding.as_ref().unwrap().agreed);
    sticks.share(shared(|sticks| sticks[3].epoch = 2));
    assert!(sticks.holding.is_none());
    // While the server side has not yet heard of a grab, it waits.
    sticks.update(Some(&grab(Phase::Began)), Some(1), start);
    sticks.share(shared(|sticks| sticks[3].epoch = 2));
    assert!(sticks.holding.is_some());
    // An event for a stick there is not changes nothing, and a share of fewer sticks forgets one held beyond them.
    sticks.changed(Event {
        clock: 0,
        stick: 9,
        now: Stick::default(),
    });
    sticks.share(Shared {
        sticks: vec![Stick::default(); 2],
    });
    assert!(sticks.holding.is_none());
    assert_eq!(sticks.shown(&[]).len(), 2);
}
