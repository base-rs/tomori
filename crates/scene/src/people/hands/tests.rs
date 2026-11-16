// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use glam::{Quat, Vec3};

use super::*;
use crate::people::{Heard, Joint};

const FRAME: f32 = 1.0 / 72.0;
const COLOUR: [f32; 3] = super::super::swarm::COLOUR;
const EYES: Option<Vec3> = Some(Vec3::new(0.0, 1.6, -2.0));
const WRIST_AT: Vec3 = Vec3::new(0.25, 1.2, -0.35);
const GRIP_AT: Vec3 = Vec3::new(0.25, 1.15, -0.3);

/// `hand` as it is drawn.
fn joints(hand: &tomori_protocol::Hand) -> Joints {
    hand.joints.map(|joint| {
        joint.map(|joint| Joint {
            position: Vec3::from_array(joint.position),
            radius: joint.radius,
        })
    })
}

/// Someone at home whose right hand is held as `gesture`, if tracked, and whose right controller is at its grip, if
/// tracked.
fn present(gesture: Option<Gesture>, controller: bool) -> Present {
    Present {
        head: Hand::new(Vec3::new(0.0, 1.6, 0.0), Quat::IDENTITY),
        controllers: [
            None,
            controller.then(|| Hand::new(GRIP_AT, Quat::from_rotation_x(-0.6))),
        ],
        joints: [
            None,
            gesture.map(|gesture| joints(&posed(gesture, 1, WRIST_AT, Quat::IDENTITY))),
        ],
        pointing: [None, None],
        hidden: false,
    }
}

fn drawn(hands: &Hands, detail: Detail) -> Vec<Cube> {
    let mut cubes = Vec::new();
    draw(&mut cubes, hands, detail, COLOUR, 1.0, EYES);
    cubes
}

fn at(cube: &Cube) -> Vec3 {
    Vec3::from_array(cube.position)
}

/// `hands` followed through `present` for `seconds`, a frame at a time.
fn follow(hands: &mut Hands, present: &Present, seconds: f32) {
    for _ in 0..(seconds / FRAME).round() as usize {
        hands.follow(present, FRAME);
    }
}

#[test]
fn a_tracked_hand_is_drawn_rather_than_its_controller_and_a_controller_alone_as_itself() {
    // Where both are tracked, as with simultaneous hands and controllers, the hand.
    let both = Hands::new(&present(Some(Gesture::Open), true));
    assert_eq!(both.shown(1), (1.0, 0.0));
    assert_eq!(both.shown(0), (0.0, 0.0));
    assert_eq!(drawn(&both, Detail::Full).len(), HAND);
    // Each replaces the swarm's own cluster at the end of its arm; a hand neither tracks keeps it.
    assert_eq!(both.clusters(), [1.0, 0.0]);
    let controller = Hands::new(&present(None, true));
    assert_eq!(controller.shown(1), (0.0, 1.0));
    assert_eq!(controller.clusters(), [1.0, 0.0]);
    let cubes = drawn(&controller, Detail::Full);
    assert_eq!(cubes.len(), CONTROLLER);
    // A small controller, at its grip, turned with it.
    for cube in &cubes {
        assert!(at(cube).distance(GRIP_AT) < 0.13, "{cube:?}");
        assert!(cube.scale.iter().all(|size| *size > 0.0 && *size < 0.11));
    }
    assert!(
        Quat::from_array(cubes[0].orientation).angle_between(Quat::from_rotation_x(-0.6)) < 1e-4
    );
    // Neither, nothing.
    assert!(drawn(&Hands::new(&present(None, false)), Detail::Full).is_empty());
    // A hand of which no bone is seen is no hand: its controller is drawn.
    let mut one = present(None, true);
    let mut joints: Joints = [None; tomori_protocol::JOINT_COUNT];
    joints[WRIST] = Some(Joint {
        position: WRIST_AT,
        radius: 0.02,
    });
    one.joints[1] = Some(joints);
    assert_eq!(Hands::new(&one).shown(1), (0.0, 1.0));
}

#[test]
fn a_hand_is_its_joints_and_bones_in_the_persons_colour_brighter_and_paler() {
    let hands = Hands::new(&present(Some(Gesture::Point), false));
    let cubes = drawn(&hands, Detail::Full);
    assert_eq!(cubes.len(), HAND);
    let hand = posed(Gesture::Point, 1, WRIST_AT, Quat::IDENTITY);
    // A cube on every joint but the palm's, where it is.
    for joint in hand.joints.iter().skip(WRIST).flatten() {
        let position = Vec3::from_array(joint.position);
        assert!(
            cubes.iter().any(|cube| at(cube).distance(position) < 1e-5),
            "{position}"
        );
    }
    // A rod along each bone, between its joints, longer than it is thick.
    for (start, end) in tomori_protocol::BONES {
        let (start, end) = (
            Vec3::from_array(hand.joints[start].unwrap().position),
            Vec3::from_array(hand.joints[end].unwrap().position),
        );
        let middle = (start + end) * 0.5;
        let rod = cubes
            .iter()
            .find(|cube| at(cube).distance(middle) < 1e-5)
            .expect("a rod along the bone");
        let along = Quat::from_array(rod.orientation) * Vec3::Z;
        assert!(along.dot((end - start).normalize()).abs() > 0.999);
        assert!(rod.scale[2] > rod.scale[0] && rod.scale[0] < 0.015);
    }
    // Pointing reads: the index's tip far from the wrist, the other fingertips curled in near the palm.
    let tip = |index: usize| Vec3::from_array(hand.joints[index].unwrap().position);
    assert!(tip(10).distance(WRIST_AT) > 0.17);
    for curled in [15, 20, 25] {
        assert!(tip(curled).distance(WRIST_AT) < 0.11, "{curled}");
    }
    // Tinted towards the person's colour, paler and a little brighter than it, but for the palm's dimmer gleam: never
    // another hue.
    for (_, cube) in cubes.iter().enumerate().filter(|(index, _)| *index != 1) {
        assert!(
            cube.color
                .iter()
                .zip(COLOUR)
                .all(|(drawn, colour)| *drawn >= colour * 0.9 - 1e-5),
            "{cube:?}"
        );
        assert!(cube.color[0] >= cube.color[1] && cube.color[1] >= cube.color[2]);
    }
    let mut blue = Vec::new();
    draw(&mut blue, &hands, Detail::Full, [0.2, 0.4, 1.0], 1.0, EYES);
    assert!(blue.iter().all(|cube| cube.color[2] >= cube.color[0]));
    // The palm a flat block turned with the hand, held palm down with its fingers ahead, and its gleam behind it as
    // seen, larger and dimmer.
    let (palm, gleam) = (&cubes[0], &cubes[1]);
    assert!(Quat::from_array(palm.orientation).angle_between(Quat::IDENTITY) < 0.1);
    assert!(palm.scale[1] < palm.scale[0] && palm.scale[1] < palm.scale[2]);
    assert!(gleam.scale[0] > palm.scale[0] && gleam.color[0] < palm.color[0]);
    let eyes = EYES.unwrap();
    assert!(at(gleam).distance(eyes) > at(palm).distance(eyes));
    // A left hand is a right hand's mirror, held alike.
    let mut left = present(None, false);
    left.joints[0] = Some(joints(&posed(Gesture::Open, 0, -WRIST_AT, Quat::IDENTITY)));
    let cubes = drawn(&Hands::new(&left), Detail::Full);
    assert!(Quat::from_array(cubes[0].orientation).angle_between(Quat::IDENTITY) < 0.1);
    // Turned as a thumb up, the thumb is above the fist.
    let up = Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2);
    let thumb = posed(Gesture::Thumb, 1, WRIST_AT, up);
    let tip = |index: usize| Vec3::from_array(thumb.joints[index].unwrap().position);
    assert!(
        [10, 15, 20, 25]
            .iter()
            .all(|finger| tip(5).y > tip(*finger).y + 0.03)
    );
}

#[test]
fn a_hand_partly_seen_is_drawn_from_what_is_seen() {
    let mut partial: Joints = [None; tomori_protocol::JOINT_COUNT];
    // A thumb alone: no palm, wrist or knuckles to find its frame from, but bones to draw; one bone of no length.
    for (index, at) in [(2, 0.0), (3, 0.03), (4, 0.03), (5, 0.06)] {
        partial[index] = Some(Joint {
            position: WRIST_AT + Vec3::new(at, 0.0, -at),
            radius: 0.01,
        });
    }
    let mut thumb = present(None, false);
    thumb.joints[1] = Some(partial);
    let cubes = drawn(&Hands::new(&thumb), Detail::Full);
    // The palm where its first joint is, turned as a hand held out, its gleam, four joints and the two bones of length.
    assert_eq!(cubes.len(), 2 + 4 + 2);
    assert_eq!(at(&cubes[0]), WRIST_AT);
    assert_eq!(
        Quat::from_array(cubes[0].orientation),
        Quat::IDENTITY,
        "{:?}",
        cubes[0]
    );
    // A wrist and a middle knuckle, but neither side's: turned along them.
    let mut middle: Joints = [None; tomori_protocol::JOINT_COUNT];
    middle[WRIST] = Some(Joint {
        position: WRIST_AT,
        radius: 0.02,
    });
    middle[11] = Some(Joint {
        position: WRIST_AT + Vec3::new(0.0, 0.0, 0.08),
        radius: 0.01,
    });
    thumb.joints[1] = Some(middle);
    let cubes = drawn(&Hands::new(&thumb), Detail::Full);
    let ahead = Quat::from_array(cubes[0].orientation) * Vec3::NEG_Z;
    assert!(ahead.distance(Vec3::Z) < 1e-4, "{ahead}");
    // Its palm as large as a hand's at rest, the knuckles across not seen.
    assert!((cubes[0].scale[0] - PALM_REST.0).abs() < 1e-6);
}

#[test]
fn switching_between_a_controller_and_a_hand_cross_fades() {
    let controller = present(None, true);
    let hand = present(Some(Gesture::Open), true);
    let mut hands = Hands::new(&controller);
    // Halfway, both are drawn, each half shown, and the swarm's cluster not at all.
    follow(&mut hands, &hand, CROSS / 2.0);
    let (shown, held) = hands.shown(1);
    assert!(
        (shown - 0.5).abs() < 0.04 && (held - 0.5).abs() < 0.04,
        "{shown} {held}"
    );
    assert!(hands.clusters()[1] < 0.55);
    let cubes = drawn(&hands, Detail::Full);
    assert_eq!(cubes.len(), HAND + CONTROLLER);
    let palm = Vec3::from_array(
        posed(Gesture::Open, 1, WRIST_AT, Quat::IDENTITY).joints[0]
            .unwrap()
            .position,
    );
    let block = cubes
        .iter()
        .find(|cube| at(cube) == palm)
        .expect("the palm");
    assert!(block.scale[1] < PALM_THICK * 0.55 && block.scale[1] > PALM_THICK * 0.45);
    // Then the hand alone.
    follow(&mut hands, &hand, CROSS / 2.0 + FRAME);
    assert_eq!(hands.shown(1), (1.0, 0.0));
    assert_eq!(drawn(&hands, Detail::Full).len(), HAND);
    assert_eq!(hands.clusters()[1], 0.0);
    // And back, over as long: the hand fading as the controller takes over.
    follow(&mut hands, &controller, CROSS / 2.0);
    let (shown, held) = hands.shown(1);
    assert!(
        (shown - 0.5).abs() < 0.04 && (held - 0.5).abs() < 0.04,
        "{shown} {held}"
    );
    follow(&mut hands, &controller, CROSS / 2.0 + FRAME);
    assert_eq!(hands.shown(1), (0.0, 1.0));
    assert_eq!(drawn(&hands, Detail::Full).len(), CONTROLLER);
}

#[test]
fn a_hand_that_loses_tracking_dissolves_where_it_was_rather_than_freezing() {
    let mut hands = Hands::new(&present(Some(Gesture::Point), false));
    // It moves while tracked, and is lost where it last was.
    let mut moved = present(Some(Gesture::Point), false);
    moved.joints[1] = Some(joints(&posed(
        Gesture::Point,
        1,
        WRIST_AT + Vec3::X * 0.1,
        Quat::IDENTITY,
    )));
    hands.follow(&moved, FRAME);
    let last = drawn(&hands, Detail::Full);
    let lost = present(None, false);
    let mut was = 1.0;
    for frame in 1..=((LOST / FRAME).ceil() as usize) {
        hands.follow(&lost, FRAME);
        let (shown, _) = hands.shown(1);
        assert!(shown < was, "fading, not frozen");
        was = shown;
        let t = frame as f32 * FRAME / LOST;
        assert!((shown - (1.0 - t).max(0.0)).abs() < 1e-4, "{shown} {t}");
        let cubes = drawn(&hands, Detail::Full);
        if shown > 0.0 {
            // Each cube where it was, shrinking.
            assert_eq!(cubes.len(), HAND);
            for (cube, before) in cubes.iter().zip(&last) {
                assert_eq!(cube.position, before.position);
                assert!((cube.scale[0] - before.scale[0] * shown).abs() < 1e-6);
            }
        } else {
            assert!(cubes.is_empty());
        }
    }
    assert_eq!(hands.shown(1), (0.0, 0.0));
    // The swarm's own cluster comes back as it goes.
    assert_eq!(hands.clusters(), [1.0, 1.0]);
    // Tracked again, it comes back where it is now.
    follow(&mut hands, &moved, CROSS);
    assert_eq!(drawn(&hands, Detail::Full).len(), HAND);
    // A controller lost fades too, over a cross-fade.
    let mut controller = Hands::new(&present(None, true));
    follow(&mut controller, &lost, CROSS / 2.0);
    assert!((controller.shown(1).1 - 0.5).abs() < 0.04);
    follow(&mut controller, &lost, CROSS / 2.0 + FRAME);
    assert!(drawn(&controller, Detail::Full).is_empty());
}

#[test]
fn far_away_a_hand_is_one_cluster_at_its_palm_and_farther_neither_is_drawn() {
    let hand = Hands::new(&present(Some(Gesture::Open), false));
    let palm = Vec3::from_array(
        posed(Gesture::Open, 1, WRIST_AT, Quat::IDENTITY).joints[0]
            .unwrap()
            .position,
    );
    let cubes = drawn(&hand, Detail::Fewer);
    assert_eq!(cubes.len(), CLUSTER);
    assert!(cubes.iter().all(|cube| at(cube).distance(palm) < 0.08));
    let controller = Hands::new(&present(None, true));
    assert_eq!(drawn(&controller, Detail::Fewer).len(), BODY.len());
    assert!(drawn(&hand, Detail::Head).is_empty());
    assert!(drawn(&controller, Detail::Head).is_empty());
    // Fewer than in full, so that a crowded space stays cheap.
    const { assert!(CLUSTER < HAND && BODY.len() < CONTROLLER) };
}

#[test]
fn hands_are_drawn_in_the_swarm_of_someone_heard_tracking_them() {
    // Through `People`, from what presence says: a hand where both are sent, then its controller once it is lost.
    let now = std::time::Instant::now();
    let mut people = crate::people::People::default();
    let mut heard = Heard {
        head: tomori_protocol::Pose::at([0.0, 1.6, 0.0]),
        controllers: [None, Some(tomori_protocol::Pose::at(GRIP_AT.to_array()))],
        hands: [
            None,
            Some(posed(Gesture::Open, 1, WRIST_AT, Quat::IDENTITY)),
        ],
        pointing: [None, None],
        hidden: false,
        reports: 0,
    };
    people.update(now, FRAME, 0, [(2, &heard)], EYES, None);
    let full = people.figures()[0].cubes.len();
    // The swarm without its right hand's cluster, and the hand.
    let cluster = super::super::swarm::MOTES
        .iter()
        .filter(|mote| {
            matches!(
                mote.part,
                super::super::swarm::Part::Hand(1) | super::super::swarm::Part::Gleam(1)
            )
        })
        .count();
    assert_eq!(full, super::super::swarm::MOST - cluster + HAND);
    assert!(full <= crate::people::CUBES);
    heard.hands = [None, None];
    for frame in 1..=40 {
        heard.reports = frame;
        people.update(
            now + std::time::Duration::from_secs_f32(frame as f32 * FRAME),
            FRAME,
            0,
            [(2, &heard)],
            EYES,
            None,
        );
    }
    assert_eq!(
        people.figures()[0].cubes.len(),
        super::super::swarm::MOST - cluster + CONTROLLER
    );
}
