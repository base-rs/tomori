// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use glam::{Quat, Vec3};

use super::*;
use crate::client_side::Hand;
use crate::people::{Joint, Joints};

const FRAME: f32 = 1.0 / 72.0;

/// Someone whose eyes are at `eyes`, turned `turn`, holding their controllers at `controllers`, left then right.
fn present(eyes: Vec3, turn: Quat, controllers: [Option<Vec3>; 2]) -> Present {
    Present {
        head: Hand::new(eyes, turn),
        controllers: controllers.map(|at| at.map(|at| Hand::new(at, Quat::IDENTITY))),
        joints: [None, None],
        pointing: [None, None],
    }
}

/// Someone standing at home, facing −Z, their hands as `controllers`.
fn standing(controllers: [Option<Vec3>; 2]) -> Present {
    present(Vec3::new(0.0, 1.6, 0.0), Quat::IDENTITY, controllers)
}

/// Close enough to be seen in full.
const EYES: Option<Vec3> = Some(Vec3::new(0.0, 1.6, -2.0));

fn cues(glow: f32, time: f32) -> Cues {
    Cues {
        colour: COLOUR,
        fade: 1.0,
        glow,
        eyes: EYES,
        time,
    }
}

/// The cubes of `present`, with `body`, as `cues` say.
fn drawn(present: &Present, body: &Body, cues: &Cues) -> Vec<Cube> {
    let mut cubes = Vec::new();
    look(&mut cubes, present, body, cues);
    cubes
}

fn at(cube: &Cube) -> Vec3 {
    Vec3::from_array(cube.position)
}

/// Each mote's place among the cubes drawn in full, with its part.
fn parts() -> impl Iterator<Item = (usize, Part)> {
    MOTES.iter().map(|mote| mote.part).enumerate()
}

#[test]
fn the_arm_reaches_the_hand_its_elbow_bending_towards_its_pole_and_never_inverting() {
    let shoulder = Vec3::new(0.18, 1.35, 0.0);
    let pole = Vec3::new(1.0, -0.7, 0.45).normalize();
    let mut dice = Dice(7);
    for _ in 0..2_000 {
        let reach = dice.within(0.05, (UPPER + LOWER) * STRETCH);
        let hand = shoulder + dice.direction() * reach;
        let (elbow, end) = elbow(shoulder, hand, pole, UPPER, LOWER);
        // The hand is reached, the arm stretched no more than it must be, its bones alike.
        assert!(end.distance(hand) < 1e-4, "{hand} {end}");
        let stretch = (reach / (UPPER + LOWER)).max(1.0);
        assert!((elbow.distance(shoulder) - UPPER * stretch).abs() < 1e-4);
        assert!((elbow.distance(end) - LOWER * stretch).abs() < 1e-4);
        // The elbow bends towards its pole, never away from it.
        let along = (hand - shoulder).normalize();
        let off = elbow - shoulder - along * (elbow - shoulder).dot(along);
        assert!(off.dot(pole) >= -1e-5, "{off} {pole}");
    }
    // At full reach the arm is straight, and as the hand comes nearer the elbow bends out smoothly, never jumping.
    let ahead = Vec3::new(0.0, 0.0, -1.0);
    let mut bent = 0.0;
    for step in (0..=100).rev() {
        let reach = (UPPER + LOWER) * step as f32 / 100.0;
        let (elbow, _) = elbow(
            shoulder,
            shoulder + ahead * reach.max(0.03),
            pole,
            UPPER,
            LOWER,
        );
        let out = (elbow - shoulder).dot(Vec3::new(pole.x, pole.y, 0.0).normalize());
        if step == 100 {
            assert!(out.abs() < 1e-3, "straight at full reach: {out}");
        }
        // Until it is nearly folded, as far as an arm bends.
        if step >= 30 {
            assert!(out >= bent - 1e-5, "{step}: {out} after {bent}");
        }
        bent = out;
    }
    // Beyond its stretch it reaches as far as it can towards the hand.
    let far = shoulder + ahead * 2.0;
    let (_, end) = elbow(shoulder, far, pole, UPPER, LOWER);
    assert!((end.distance(shoulder) - (UPPER + LOWER) * STRETCH).abs() < 1e-4);
    assert!((end - shoulder).normalize().distance(ahead) < 1e-5);
    // A hand at the shoulder folds the arm along its pole; and an arm along its pole still bends somewhere.
    let (folded, end) = elbow(shoulder, shoulder, pole, UPPER, LOWER);
    assert!((folded - shoulder).normalize().dot(pole) > 0.99);
    assert!(end.distance(shoulder) < 0.05);
    let (elbow, end) = elbow(shoulder, shoulder + pole * 0.4, pole, UPPER, LOWER);
    assert!((elbow.distance(shoulder) - UPPER).abs() < 1e-4);
    assert!(end.distance(shoulder + pole * 0.4) < 1e-4);
}

/// Whether `point` is inside the torso's core, in the body's coordinates of someone standing at home facing −Z.
fn inside_torso(point: Vec3) -> bool {
    let neck = Vec3::new(0.0, 1.6, 0.0) + NECK;
    let at = point - neck;
    (-0.62..=-0.2).contains(&at.y) && (at.x / 0.13).powi(2) + ((at.z - 0.02) / 0.07).powi(2) < 1.0
}

#[test]
fn arms_never_pass_through_the_torso_and_elbows_stay_out_and_down_in_common_poses() {
    let head = Vec3::new(0.0, 1.6, 0.0);
    // Each pose's right hand from the eyes, mirrored for the left.
    let poses = [
        ("pointing ahead", Vec3::new(0.2, -0.2, -0.58)),
        ("gesturing at the chest", Vec3::new(0.15, -0.45, -0.32)),
        ("on a table", Vec3::new(0.18, -0.75, -0.4)),
        ("raised", Vec3::new(0.25, 0.25, -0.15)),
        (
            "across to the other shoulder",
            Vec3::new(-0.15, -0.35, -0.3),
        ),
        ("held low in front", Vec3::new(0.1, -0.85, -0.25)),
        ("out to the side", Vec3::new(0.65, -0.3, -0.05)),
        ("pointing up ahead", Vec3::new(0.15, 0.1, -0.5)),
    ];
    for (name, hand) in poses {
        let hands = [
            Some(head + Vec3::new(-hand.x, hand.y, hand.z)),
            Some(head + hand),
        ];
        let present = standing(hands);
        let body = Body::new(&present, EYES, 2);
        for (side, hand) in hands.into_iter().enumerate() {
            let [shoulder, elbow, end] = body.arm(&present, Quat::IDENTITY, side);
            assert!(end.distance(hand.unwrap()) < 1e-4, "{name}: reached");
            for (from, to) in [(shoulder, elbow), (elbow, end)] {
                for step in 0..=20 {
                    let point = from.lerp(to, step as f32 / 20.0);
                    assert!(!inside_torso(point), "{name}, side {side}: {point}");
                }
            }
            // The elbow is out from the line between shoulder and hand, and no higher than the higher of them.
            let along = (end - shoulder).normalize();
            let off = elbow - shoulder - along * (elbow - shoulder).dot(along);
            assert!(off.x * sign(side) >= -1e-4, "{name}, side {side}: {off}");
            assert!(elbow.y <= shoulder.y.max(end.y) + 0.02, "{name}: {elbow}");
        }
    }
    // Untracked, both hands rest at the sides, the arms hanging, a little bent.
    let present = standing([None, None]);
    let body = Body::new(&present, EYES, 2);
    for side in 0..2 {
        let [shoulder, elbow, end] = body.arm(&present, Quat::IDENTITY, side);
        assert!(
            end.y < shoulder.y - 0.45 && end.x * sign(side) > 0.15,
            "{end}"
        );
        assert!(
            elbow.x * sign(side) > shoulder.x * sign(side) - 1e-3,
            "{elbow}"
        );
    }
}

#[test]
fn the_body_turns_after_the_head_s_yaw_damped_and_never_with_its_pitch_or_roll() {
    let eyes = Vec3::new(0.0, 1.6, 0.0);
    let ahead = standing([None, None]);
    let mut body = Body::new(&ahead, EYES, 2);
    assert_eq!(body.yaw, 0.0);
    // Turning the head 40°, the body follows, about 63% of the way in FOLLOW and all of it within two seconds.
    let turned = present(
        eyes,
        Quat::from_rotation_y(40f32.to_radians()),
        [None, None],
    );
    body.follow(&turned, FRAME, EYES, false);
    let first = body.yaw.to_degrees();
    assert!(
        (first - 40.0 * (1.0 - (-FRAME / FOLLOW).exp())).abs() < 1e-3,
        "{first}"
    );
    let mut time = FRAME;
    while time < FOLLOW - 1e-4 {
        body.follow(&turned, FRAME, EYES, false);
        time += FRAME;
    }
    assert!(
        (body.yaw.to_degrees() - 40.0 * (1.0 - (-1f32).exp())).abs() < 0.5,
        "{}",
        body.yaw.to_degrees()
    );
    for _ in 0..144 {
        body.follow(&turned, FRAME, EYES, false);
    }
    assert!((body.yaw.to_degrees() - 40.0).abs() < 0.1);
    // A head turned quickly a long way leaves the body no more than TWIST behind.
    let mut body = Body::new(&ahead, EYES, 2);
    let round = present(
        eyes,
        Quat::from_rotation_y(120f32.to_radians()),
        [None, None],
    );
    body.follow(&round, FRAME, EYES, false);
    assert!(
        (body.yaw.to_degrees() - (120.0 - TWIST)).abs() < 1e-3,
        "{}",
        body.yaw.to_degrees()
    );
    // Across the back, it turns the short way round.
    let mut body = Body::new(
        &present(
            eyes,
            Quat::from_rotation_y(170f32.to_radians()),
            [None, None],
        ),
        EYES,
        2,
    );
    body.follow(
        &present(
            eyes,
            Quat::from_rotation_y(-170f32.to_radians()),
            [None, None],
        ),
        FRAME,
        EYES,
        false,
    );
    assert!(
        body.yaw.to_degrees() > 170.0 || body.yaw.to_degrees() < -179.0,
        "{}",
        body.yaw.to_degrees()
    );
    // Nodding and tilting the head about the neck leaves the body as it was, its shoulders where they were.
    let mut body = Body::new(&ahead, EYES, 2);
    let neck = eyes + NECK;
    let shoulders = |body: &Body, present: &Present| {
        [0, 1].map(|side| body.arm(present, Quat::from_rotation_y(body.yaw), side)[0])
    };
    let before = shoulders(&body, &ahead);
    for turn in [
        Quat::from_rotation_x(-40f32.to_radians()),
        Quat::from_rotation_x(25f32.to_radians()),
        Quat::from_rotation_z(30f32.to_radians()),
    ] {
        let nodding = present(neck - turn * NECK, turn, [None, None]);
        for _ in 0..72 {
            body.follow(&nodding, FRAME, EYES, false);
        }
        assert!(body.yaw.abs() < 1e-5, "{turn}");
        let after = shoulders(&body, &nodding);
        for side in 0..2 {
            assert!(
                after[side].distance(before[side]) < 1e-4,
                "{turn}: {}",
                after[side]
            );
        }
    }
    // A head turned and nodded turns the body as far as it turned.
    let both = Quat::from_rotation_y(0.7) * Quat::from_rotation_x(-0.5);
    assert!((yaw(both) - 0.7).abs() < 1e-5);
    // Either of the two rotations alike, turned most of the way round.
    let round = Quat::from_rotation_y(3.0);
    assert!((yaw(round) - 3.0).abs() < 1e-5 && (yaw(-round) - 3.0).abs() < 1e-5);
    // The torso faces where the body does, its cubes turned with it.
    let mut body = Body::new(&ahead, EYES, 2);
    body.yaw = 0.6;
    let cubes = drawn(&ahead, &body, &cues(0.0, 0.0));
    let (torso, _) = parts().find(|(_, part)| *part == Part::Torso).unwrap();
    assert!(
        Quat::from_array(cubes[torso].orientation)
            .angle_between(Quat::from_rotation_y(0.6) * MOTES[torso].turn)
            < 1e-3
    );
}

#[test]
fn hands_come_up_as_they_are_tracked_and_fall_back_to_rest_as_they_are_lost() {
    let eyes = Vec3::new(0.0, 1.6, 0.0);
    let hand = eyes + Vec3::new(0.2, -0.3, -0.5);
    let holding = standing([None, Some(hand)]);
    let mut body = Body::new(&holding, EYES, 2);
    // Tracked from the first, the right hand is where it is, and the left at rest.
    let reached = |body: &Body, present: &Present, side| body.arm(present, Quat::IDENTITY, side)[2];
    assert!(reached(&body, &holding, 1).distance(hand) < 1e-4);
    let rest = eyes + NECK + Vec3::new(-REST.x, REST.y, REST.z);
    let left = reached(&body, &holding, 0);
    // Rest is within reach, so the left hand is there.
    assert!(left.distance(rest) < 1e-4, "{left} {rest}");
    // Lost, it falls back to rest over REACH_FALL, where it was first, moving with them as it falls.
    let lost = standing([None, None]);
    body.follow(&lost, 0.2, EYES, false);
    let falling = reached(&body, &lost, 1);
    assert!(falling.distance(hand) > 0.01 && falling.distance(eyes + NECK + REST) > 0.01);
    body.follow(&lost, 0.2, EYES, false);
    assert!(reached(&body, &lost, 1).distance(eyes + NECK + REST) < 1e-4);
    // Tracked again, it comes up over REACH_RISE.
    body.follow(&holding, 0.1, EYES, false);
    assert!(reached(&body, &holding, 1).distance(hand) > 0.01);
    body.follow(&holding, 0.15, EYES, false);
    assert!(reached(&body, &holding, 1).distance(hand) < 1e-4);
    // A tracked hand's palm is the hand rather than its controller, or its wrist where its palm is not seen.
    let mut joints: Joints = [None; tomori_protocol::JOINT_COUNT];
    let wrist = eyes + Vec3::new(0.25, -0.35, -0.4);
    joints[1] = Some(Joint {
        position: wrist,
        radius: 0.02,
    });
    let mut tracked = holding;
    tracked.joints[1] = Some(joints);
    assert_eq!(super::hand(&tracked, 1), Some(wrist));
    let palm = eyes + Vec3::new(0.26, -0.33, -0.45);
    joints[0] = Some(Joint {
        position: palm,
        radius: 0.02,
    });
    tracked.joints[1] = Some(joints);
    assert_eq!(super::hand(&tracked, 1), Some(palm));
    assert_eq!(super::hand(&tracked, 0), None);
    // The hand's cluster is about where it reaches, paler than the body.
    let body = Body::new(&tracked, EYES, 2);
    let cubes = drawn(&tracked, &body, &cues(0.0, 0.0));
    for (index, part) in parts() {
        if part == Part::Hand(1) {
            assert!(at(&cubes[index]).distance(palm) < 0.06);
            assert!(cubes[index].color[1] > COLOUR[1] && cubes[index].color[2] > COLOUR[2]);
        }
    }
}

#[test]
fn fewer_cubes_are_drawn_farther_away_and_a_head_alone_beyond_and_none_below_where_legs_would_be() {
    let full = cubes(Detail::Full);
    let fewer = cubes(Detail::Fewer);
    let head = cubes(Detail::Head);
    assert_eq!(full, MOTES.len());
    assert_eq!(full, MOST);
    assert!((200..=400).contains(&full), "{full}");
    assert!(fewer * 2 <= full, "{fewer} of {full}");
    assert!(head < 50, "{head}");
    assert!(
        MOTES
            .iter()
            .filter(|mote| mote.from == Detail::Head)
            .all(|mote| matches!(
                mote.part,
                Part::Core | Part::Shell | Part::Eye(_) | Part::Orbit
            ))
    );
    // As drawn, with each detail's cubes larger, to fill in for those left out.
    let person = standing([
        Some(Vec3::new(-0.2, 1.2, -0.4)),
        Some(Vec3::new(0.2, 1.2, -0.4)),
    ]);
    let mut body = Body::new(&person, EYES, 2);
    let mut largest = Vec::new();
    for (detail, count) in [
        (Detail::Full, full),
        (Detail::Fewer, fewer),
        (Detail::Head, head),
    ] {
        body.detail = detail;
        let cubes = drawn(&person, &body, &cues(1.0, 0.0));
        assert_eq!(cubes.len(), count);
        // The shell's largest, after the head's core, which is the same at every detail.
        let shell = MOTES.iter().filter(|mote| mote.from <= detail).zip(&cubes);
        largest.push(
            shell
                .filter(|(mote, _)| mote.part == Part::Shell)
                .map(|(_, cube)| cube.scale[0])
                .fold(0.0, f32::max),
        );
    }
    assert!(
        largest[0] < largest[1] && largest[1] < largest[2],
        "{largest:?}"
    );
    // Each detail by how far away they are, holding at the edge: fewer beyond 5 m, the head alone beyond 15 m.
    use Detail::*;
    for (distance, was, is) in [
        (4.9, Fewer, Fewer),
        (4.4, Fewer, Full),
        (5.4, Full, Full),
        (5.6, Full, Fewer),
        (15.4, Fewer, Fewer),
        (15.6, Fewer, Head),
        (14.6, Head, Head),
        (14.4, Head, Fewer),
        (2.0, Head, Full),
        (30.0, Full, Head),
    ] {
        assert_eq!(Detail::at(distance, was), is, "{distance} from {was:?}");
    }
    // Followed, the detail is theirs from the eyes; without eyes it is kept.
    body.follow(&person, FRAME, Some(Vec3::new(0.0, 1.6, -20.0)), false);
    assert_eq!(body.detail, Head);
    body.follow(&person, FRAME, None, false);
    assert_eq!(body.detail, Head);
    // No legs: nothing below where the falling cubes fade, about the waist.
    let body = Body::new(&person, EYES, 2);
    let neck = Vec3::new(0.0, 1.6, 0.0) + NECK;
    let mut lowest = f32::MAX;
    for step in 0..120 {
        for cube in drawn(&person, &body, &cues(0.0, step as f32 * 0.1)) {
            lowest = lowest.min(cube.position[1]);
        }
    }
    assert!(lowest > neck.y - 0.62 - FALL.1 - 0.08, "{lowest}");
    assert!(lowest < neck.y - 0.62 - FALL.1 * 0.6, "{lowest}");
    // And thinner below the chest than above it, its cubes there smaller and dimmer.
    let torso: Vec<f32> = MOTES
        .iter()
        .filter(|mote| mote.part == Part::Torso)
        .map(|mote| mote.at.y)
        .collect();
    let chest = torso.iter().filter(|y| **y > -0.36).count();
    assert!(
        chest > (torso.len() - chest) * 2,
        "{chest} of {}",
        torso.len()
    );
    let mean = |above: bool| {
        let chosen: Vec<&Mote> = MOTES
            .iter()
            .filter(|mote| mote.part == Part::Torso && (mote.at.y > -0.25) == above)
            .collect();
        let count = chosen.len() as f32;
        (
            chosen.iter().map(|mote| mote.size.x).sum::<f32>() / count,
            chosen.iter().map(|mote| mote.tone).sum::<f32>() / count,
        )
    };
    let ((chest_size, chest_tone), (waist_size, waist_tone)) = (mean(true), mean(false));
    assert!(waist_size < chest_size * 0.9, "{waist_size} {chest_size}");
    assert!(waist_tone < chest_tone * 0.9, "{waist_tone} {chest_tone}");
}

#[test]
fn eyes_warm_and_brighten_clearly_as_the_head_faces_this_participant_never_turning_gold() {
    let person = standing([None, None]);
    let body = Body::new(&person, EYES, 2);
    let cool = drawn(&person, &body, &cues(0.0, 1.0));
    let warm = drawn(&person, &body, &cues(1.0, 1.0));
    let sum = |color: [f32; 3]| color.iter().sum::<f32>();
    // Facing straight at them, both eyes warm alike, nearly fully.
    let glow = 1.0 - GLOW_SIDE / 2.0;
    for (index, part) in parts() {
        let (cool, warm) = (&cool[index], &warm[index]);
        match part {
            Part::Eye(_) => {
                // A soft, warm off-white while they look elsewhere, a little of the face's colour in it, paler than it.
                assert_eq!(cool.color, eyes(COLOUR));
                assert!(
                    cool.color[0] > cool.color[1] && cool.color[1] > cool.color[2],
                    "{:?}",
                    cool.color
                );
                assert!(cool.color[1] > COLOUR[1] * 1.5 && cool.color[2] > COLOUR[2] * 2.0);
                assert!(cool.color.iter().all(|value| (0.55..0.95).contains(value)));
                // Looked at, clearly warmer and brighter, so that it reads from across a space, still a pale cream
                // rather than gold, and larger.
                let expected = mix(eyes(COLOUR), warm_eyes(COLOUR), glow);
                assert!(Vec3::from_array(warm.color).distance(Vec3::from_array(expected)) < 1e-5);
                let brighter = luminance(warm.color) / luminance(cool.color);
                assert!(brighter > 1.12 && brighter < 1.3, "{brighter}");
                assert!(sum(warm.color) > sum(cool.color));
                assert!(
                    warm.color[0] - warm.color[2] > cool.color[0] - cool.color[2] + 0.1,
                    "{:?}",
                    warm.color
                );
                let grown = warm.scale[0] / cool.scale[0];
                assert!(grown > 1.15 && grown < 1.25, "{grown}");
                assert!(
                    warm.color[2] > 0.5 && warm.color[1] > 0.75,
                    "{:?}",
                    warm.color
                );
                assert!((warm.scale[0] / cool.scale[0] - 1.0 - EYE_GROW).abs() < 1e-5);
            }
            _ => assert_eq!(warm.color, cool.color, "{part:?}"),
        }
    }
    // Eased in and out: a third of the way, less than a third warmed, and two thirds of the way, more than two thirds.
    let eye = |side| {
        parts()
            .find(|(_, part)| *part == Part::Eye(side))
            .unwrap()
            .0
    };
    let grown = |glow: f32| {
        let cubes = drawn(&person, &body, &cues(glow, 1.0));
        (cubes[eye(1)].scale[0] / cool[eye(1)].scale[0] - 1.0) / EYE_GROW
    };
    assert_eq!(grown(0.0), 0.0);
    assert!(grown(1.0 / 3.0) < 0.3 && grown(2.0 / 3.0) > 0.7);
    assert!((grown(1.5) - 1.0).abs() < 1e-5);
    // Seen from their left, the left eye warms more; without eyes to see them, neither is nearer.
    let from_left = Cues {
        eyes: Some(Vec3::new(-1.0, 1.6, -2.0)),
        ..cues(1.0, 1.0)
    };
    let cubes = drawn(&person, &body, &from_left);
    assert!(cubes[eye(0)].color[0] > cubes[eye(1)].color[0]);
    let unseen = drawn(
        &person,
        &body,
        &Cues {
            eyes: None,
            ..cues(1.0, 1.0)
        },
    );
    assert_eq!(unseen[eye(0)].color, unseen[eye(1)].color);
}

#[test]
fn the_eyes_blink_every_few_seconds_each_person_at_times_of_their_own_squashing_and_opening_again()
{
    let person = standing([None, None]);
    // Over a minute, each blink begins 2 to 6 s after the last, at times of their own.
    let mut firsts = Vec::new();
    for who in [1, 2, 3, 40, u32::MAX] {
        let mut body = Body::new(&person, EYES, who);
        assert_eq!(body.blink.shut(), 0.0);
        let (mut began, mut time, mut since) = (Vec::new(), 0.0, body.blink.since);
        for _ in 0..72 * 60 {
            body.follow(&person, FRAME, EYES, false);
            time += FRAME;
            if body.blink.since < since {
                began.push(time - body.blink.since);
            }
            since = body.blink.since;
        }
        assert!(began[0] < BLINKS.1 + FRAME, "{began:?}");
        assert!((9..=31).contains(&began.len()), "{began:?}");
        let gaps: Vec<f32> = began.windows(2).map(|pair| pair[1] - pair[0]).collect();
        for gap in &gaps {
            assert!(*gap > BLINKS.0 - 1e-3 && *gap < BLINKS.1 + 1e-3, "{gaps:?}");
        }
        // Never a steady beat.
        let (shortest, longest) = gaps.iter().fold((f32::MAX, 0.0f32), |(low, high), gap| {
            (low.min(*gap), high.max(*gap))
        });
        assert!(longest - shortest > 1.5, "{gaps:?}");
        firsts.push(began[0]);
    }
    for (index, first) in firsts.iter().enumerate() {
        assert!(
            firsts[..index]
                .iter()
                .all(|other| (other - first).abs() > 0.05),
            "{firsts:?}"
        );
    }
    // A blink squashes each eye from above and below, never its width, nearly shut halfway through, open again after.
    let mut body = Body::new(&person, EYES, 2);
    let eye = parts().find(|(_, part)| *part == Part::Eye(0)).unwrap().0;
    let open = drawn(&person, &body, &cues(0.0, 1.0))[eye];
    let mut at = |since: f32| {
        body.blink.since = since;
        drawn(&person, &body, &cues(0.0, 1.0))[eye]
    };
    for (since, height) in [
        (0.0, 1.0),
        (BLINK * 0.25, 1.0 - BLINK_SHUT * 0.5f32.sqrt()),
        (BLINK * 0.5, 1.0 - BLINK_SHUT),
        (BLINK * 0.75, 1.0 - BLINK_SHUT * 0.5f32.sqrt()),
        (BLINK, 1.0),
        (1.0, 1.0),
    ] {
        let blinking = at(since);
        assert!(
            (blinking.scale[1] / open.scale[1] - height).abs() < 1e-4,
            "{since}: {:?}",
            blinking.scale
        );
        assert_eq!(blinking.scale[0], open.scale[0]);
        assert_eq!(blinking.position, open.position);
    }
    // A long frame never blinks twice at once, nor skips to the end of one.
    let mut blink = Blink::new(2);
    blink.since = 0.5;
    blink.next = 3.0;
    blink.after(30.0);
    assert!(blink.since <= BLINK && blink.shut() < 1e-5, "{blink:?}");
    assert!((BLINKS.0..BLINKS.1).contains(&blink.next));
}

#[test]
fn the_orbit_quickens_and_brightens_while_their_voice_plays_and_calms_after() {
    let person = standing([None, None]);
    let mut body = Body::new(&person, EYES, 2);
    let orbit = |body: &Body| {
        let cubes = drawn(&person, body, &cues(0.0, 0.0));
        let (index, _) = parts().find(|(_, part)| *part == Part::Orbit).unwrap();
        cubes[index]
    };
    let quiet = orbit(&body);
    // Quiet, it turns slowly.
    body.follow(&person, 0.1, EYES, false);
    assert!((body.orbit - ORBIT_TURN * 0.1).abs() < 1e-5);
    // As their voice plays, it quickens over SPEAK_RISE, its cubes larger and brighter.
    body.follow(&person, SPEAK_RISE / 2.0, EYES, true);
    assert!((body.speaking - 0.5).abs() < 1e-5);
    body.follow(&person, SPEAK_RISE, EYES, true);
    assert_eq!(body.speaking, 1.0);
    let before = body.orbit;
    body.follow(&person, 0.1, EYES, true);
    assert!((body.orbit - before - ORBIT_SPEAKING_TURN * 0.1).abs() < 1e-4);
    let speaking = orbit(&body);
    assert!((speaking.scale[0] / quiet.scale[0] - 1.35).abs() < 1e-4);
    assert!(
        speaking
            .color
            .iter()
            .zip(quiet.color)
            .all(|(speaking, quiet)| *speaking > quiet)
    );
    // The orbit is about the head, a little beyond it, and moves round it.
    let middle = Vec3::new(0.0, 1.6, 0.0) + MIDDLE;
    let radius = at(&speaking).distance(middle);
    assert!(radius > 0.15 && radius < 0.23, "{radius}");
    assert!(at(&speaking).distance(at(&quiet)) > 0.02);
    // A tidy ring: each of its cubes as far from its middle, and spaced evenly about it.
    let orbited = middle + Vec3::new(0.0, ORBIT_ABOVE, 0.0);
    let cubes = drawn(&person, &body, &cues(0.0, 0.0));
    let ring = Mat3::from_euler(glam::EulerRot::XYZ, ORBIT_TILT.0, 0.0, ORBIT_TILT.1).inverse();
    let mut angles: Vec<f32> = parts()
        .filter(|(_, part)| *part == Part::Orbit)
        .map(|(index, _)| {
            let from = ring * (at(&cubes[index]) - orbited);
            assert!((from.length() - ORBIT_RADIUS).abs() < 0.004, "{from}");
            assert!(from.y.abs() < 0.004, "{from}");
            from.z.atan2(from.x)
        })
        .collect();
    angles.sort_by(f32::total_cmp);
    let even = TAU / angles.len() as f32;
    for pair in angles.windows(2) {
        assert!((pair[1] - pair[0] - even).abs() < even * 0.3, "{angles:?}");
    }
    // Silent again, it calms over SPEAK_FALL, never flickering for a breath between words.
    body.follow(&person, 0.1, EYES, false);
    assert!(body.speaking > 0.75);
    body.follow(&person, SPEAK_FALL, EYES, false);
    assert_eq!(body.speaking, 0.0);
}

#[test]
fn cubes_drift_about_where_they_rest_shrink_where_they_rest_as_they_fade_and_fall_below_the_chest()
{
    let person = standing([None, Some(Vec3::new(0.2, 1.2, -0.4))]);
    let body = Body::new(&person, EYES, 2);
    let still = drawn(&person, &body, &cues(0.0, 0.0));
    let later = drawn(&person, &body, &cues(0.0, 0.7));
    let mut drifted = 0;
    for ((index, part), (was, is)) in parts().zip(still.iter().zip(&later)) {
        let moved = at(was).distance(at(is));
        match part {
            Part::Drift => {}
            Part::Eye(_) => assert_eq!(moved, 0.0),
            Part::Core if index == 0 => assert_eq!(moved, 0.0),
            _ => {
                assert!(moved < 0.04, "{index} {part:?} {moved}");
                drifted += usize::from(moved > 1e-4);
            }
        }
    }
    assert!(drifted > 200, "{drifted}");
    // The cubes below the chest fall, shrinking as they go, and begin again.
    let (index, mote) = MOTES
        .iter()
        .enumerate()
        .find(|(_, mote)| mote.part == Part::Drift)
        .unwrap();
    let falling = |time: f32| drawn(&person, &body, &cues(0.0, time))[index];
    let start = (1.0 - mote.offset) * mote.fall;
    let (born, top, low) = (
        falling(start + 0.01),
        falling(start + mote.fall * BORN),
        falling(start + mote.fall * 0.9),
    );
    assert!(top.position[1] - low.position[1] > mote.drop * 0.7);
    // Growing in from nothing, so that none appears at once, and shrinking and fading as it falls.
    assert!(born.scale[0] < top.scale[0] * 0.1);
    assert!(low.scale[0] < top.scale[0] * 0.5);
    assert!(low.color[1] < top.color[1] * 0.8);
    // Fading, each cube shrinks where it rests.
    let faded = drawn(
        &person,
        &body,
        &Cues {
            fade: 0.5,
            ..cues(0.0, 0.7)
        },
    );
    for (faded, whole) in faded.iter().zip(&later) {
        assert_eq!(faded.position, whole.position);
        assert!((faded.scale[0] - whole.scale[0] * 0.5).abs() < 1e-6);
    }
    // And every client draws everyone alike.
    assert_eq!(
        format!("{:?}", drawn(&person, &body, &cues(0.0, 0.7))),
        format!("{later:?}")
    );
}

#[test]
fn a_person_reads_as_one_a_head_above_shoulders_above_a_torso_with_the_eyes_where_they_look() {
    let turn = Quat::from_rotation_y(1.1);
    let eyes = Vec3::new(2.0, 1.7, -1.0);
    let person = present(eyes, turn, [None, None]);
    let body = Body::new(&person, EYES, 2);
    let cubes = drawn(&person, &body, &cues(0.0, 0.0));
    let mean = |which: fn(Part) -> bool| {
        let chosen: Vec<Vec3> = parts()
            .filter(|(_, part)| which(*part))
            .map(|(index, _)| at(&cubes[index]))
            .collect();
        chosen.iter().sum::<Vec3>() / chosen.len() as f32
    };
    let head = mean(|part| part == Part::Shell);
    let eyes_at = mean(|part| matches!(part, Part::Eye(_)));
    let torso = mean(|part| part == Part::Torso);
    assert!(head.distance(eyes + turn * MIDDLE) < 0.02, "{head}");
    // The eyes are on the side the head faces.
    assert!((eyes_at - head).normalize().dot(turn * Vec3::NEG_Z) > 0.9);
    assert!(
        head.y - torso.y > 0.3 && head.y - torso.y < 0.6,
        "{}",
        head.y - torso.y
    );
    // About as wide as a person at the shoulders, and every colour within what a cube may be.
    let across = turn * Vec3::X;
    let width = parts()
        .filter(|(_, part)| *part == Part::Torso)
        .map(|(index, _)| (at(&cubes[index]) - torso).dot(across).abs())
        .fold(0.0, f32::max);
    assert!(width > 0.15 && width < 0.25, "{width}");
    assert!(
        cubes
            .iter()
            .all(|cube| cube.color.iter().all(|value| (0.0..=1.0).contains(value)))
    );
}

#[test]
fn each_cube_glows_brightest_at_the_middle_of_its_part_as_seen_dimmer_and_smaller_at_its_outline_and_shimmers()
 {
    let person = standing([
        Some(Vec3::new(-0.25, 1.15, -0.3)),
        Some(Vec3::new(0.25, 1.15, -0.3)),
    ]);
    let body = Body::new(&person, EYES, 2);
    let cubes = drawn(&person, &body, &cues(0.0, 0.0));
    // How bright each cube of the head's shell and the torso is for its own tone, and how large for its own size, by
    // which way its part's outside faces the eyes, ahead along −Z: its middle as seen, or its outline.
    let ahead = (EYES.unwrap() - (Vec3::new(0.0, 1.6, 0.0) + NECK)).normalize();
    for part in [Part::Shell, Part::Torso] {
        let mean = |middle: bool| {
            let chosen: Vec<(f32, f32)> = MOTES
                .iter()
                .zip(&cubes)
                .filter(|(mote, _)| mote.part == part)
                .filter(|(mote, _)| {
                    let facing = mote.out.dot(ahead);
                    if middle {
                        facing > 0.7
                    } else {
                        facing.abs() < 0.2
                    }
                })
                .map(|(mote, cube)| (cube.color[1] / mote.tone, cube.scale[0] / mote.size.x))
                .collect();
            let count = chosen.len() as f32;
            assert!(count >= 3.0, "{part:?}: {count}");
            (
                chosen.iter().map(|(light, _)| light).sum::<f32>() / count,
                chosen.iter().map(|(_, large)| large).sum::<f32>() / count,
            )
        };
        let ((middle, middle_size), (outline, outline_size)) = (mean(true), mean(false));
        assert!(outline < middle * 0.6, "{part:?}: {outline} {middle}");
        assert!(
            outline_size < middle_size * 0.92,
            "{part:?}: {outline_size} {middle_size}"
        );
    }
    // Seen from behind, their back is the brighter.
    let behind = Cues {
        eyes: Some(Vec3::new(0.0, 1.6, 2.0)),
        ..cues(0.0, 0.0)
    };
    let from_behind = drawn(&person, &body, &behind);
    let back = |cubes: &[Cube]| {
        MOTES
            .iter()
            .zip(cubes)
            .filter(|(mote, _)| mote.part == Part::Torso && mote.out.z > 0.8)
            .map(|(_, cube)| cube.color[1])
            .sum::<f32>()
    };
    assert!(back(&from_behind) > back(&cubes) * 1.5);
    // Each shimmers a little over time, about its own brightness, no more than SHIMMER either way.
    let later = drawn(&person, &body, &cues(0.0, 0.4));
    let mut shimmered = 0;
    for ((mote, was), is) in MOTES.iter().zip(&cubes).zip(&later) {
        if mote.part == Part::Core {
            let (was, is) = (was.color[1], is.color[1]);
            assert!(is / was < (1.0 + SHIMMER) / (1.0 - SHIMMER) + 1e-4);
            shimmered += usize::from((is - was).abs() > 1e-3);
        }
    }
    assert!(shimmered > 5, "{shimmered}");
    // Every colour is the person's, so that their own replaces it alone: a cube of the core is their colour, as bright as
    // the core is.
    let colour = [0.1, 0.3, 0.45];
    let theirs = drawn(
        &person,
        &body,
        &Cues {
            colour,
            ..cues(0.0, 0.0)
        },
    );
    for (mote, cube) in MOTES.iter().zip(&theirs) {
        if matches!(mote.part, Part::Core | Part::Torso | Part::Upper(_)) {
            let ratio = cube.color[2] / colour[2];
            assert!(
                (cube.color[1] - colour[1] * ratio).abs() < 1e-5,
                "{:?}",
                cube.color
            );
        }
    }
    assert!(theirs[0].color[1] / colour[1] > CORE * 0.8);
    assert_ne!(theirs[MOTES.len() - 1].color, cubes[MOTES.len() - 1].color);
}

#[test]
fn the_eyes_are_small_soft_rounds_on_the_face_s_surface_a_little_low_and_set_wide() {
    for side in 0..2 {
        let ahead = eye_direction(side);
        // Each eye is three flat boxes facing out of the head where the eye looks, about one middle, together a round, a
        // little taller than wide, its corners cut.
        let boxes: Vec<&Mote> = MOTES
            .iter()
            .filter(|mote| mote.part == Part::Eye(side))
            .collect();
        assert_eq!(boxes.len(), 3);
        for mote in &boxes {
            assert_eq!(mote.at, boxes[0].at);
            assert!((mote.turn * Vec3::NEG_Z).distance(ahead) < 1e-4);
            assert!(mote.size.z < mote.size.x.max(mote.size.y) * 0.5);
        }
        let wide = boxes.iter().map(|mote| mote.size.x).fold(0.0, f32::max);
        let high = boxes.iter().map(|mote| mote.size.y).fold(0.0, f32::max);
        assert!(wide < 0.02 && high < 0.024 && high > wide, "{wide} {high}");
        assert!(
            boxes
                .iter()
                .all(|mote| mote.size.x < wide * 0.9 || mote.size.y < high * 0.9)
        );
        // A little below the head's middle, and set wide.
        let eye = boxes[0];
        assert!(eye.at.y < -0.01 && eye.at.y > -0.03, "{}", eye.at);
        assert!((0.08..0.09).contains(&(eye.at.x.abs() * 2.0)), "{}", eye.at);
        // On the face's surface, in no socket: just in front of the shell's cubes about it, which none stands in front
        // of, even grown as it warms, and well in front of the core.
        let axis = eye.at.normalize();
        let reach = EYE.1 * (1.0 + EYE_GROW) / 2.0;
        let (mut about, mut front) = (0, f32::MIN);
        for mote in MOTES.iter().filter(|mote| mote.part == Part::Shell) {
            let half = mote.size.x * 0.6;
            let (depth, lateral) = (
                mote.at.dot(axis),
                (mote.at - axis * mote.at.dot(axis)).length(),
            );
            if depth > 0.0 && lateral - half < reach {
                assert!(
                    depth + half < eye.at.dot(axis) - eye.size.z / 2.0,
                    "{mote:?}"
                );
            }
            if depth > 0.0 && lateral < depth * 25f32.to_radians().tan() {
                about += 1;
                front = front.max(depth + half);
            }
        }
        assert!(about >= 3, "{about}");
        let proud = eye.at.dot(axis) - front;
        assert!(proud < 0.01, "{proud}");
        for mote in MOTES.iter().filter(|mote| mote.part == Part::Core) {
            assert!(mote.at.dot(axis) + reaching(mote, axis) < eye.at.dot(axis) - 0.02);
        }
    }
    // The head's shell is dense, its cubes packed on its outside so that its outline is round.
    let shell: Vec<&Mote> = MOTES
        .iter()
        .filter(|mote| mote.part == Part::Shell)
        .collect();
    assert!(shell.len() > 100, "{}", shell.len());
    for mote in shell {
        let on = mote.at / HEAD;
        assert!((0.94..=1.0).contains(&on.length()), "{on}");
    }
}

#[test]
fn the_arms_are_rods_along_each_bone_tapering_to_the_wrist_with_loose_cubes_about_them_stretching_as_it_does()
 {
    let reaching = |hand: Vec3| {
        let person = standing([None, Some(hand)]);
        let body = Body::new(&person, EYES, 2);
        let [shoulder, elbow, end] = body.arm(&person, Quat::IDENTITY, 1);
        (
            drawn(&person, &body, &cues(0.0, 0.0)),
            [shoulder, elbow, end],
        )
    };
    let (cubes, joints) = reaching(Vec3::new(0.3, 1.25, -0.45));
    let mut widths = [0.0; 2];
    let mut lengths = [0.0; 2];
    for bone in 0..2 {
        let (start, end) = (joints[bone], joints[bone + 1]);
        let along = (end - start).normalize();
        let length = start.distance(end);
        let rods: Vec<&Cube> = parts()
            .filter(|(index, part)| *part == Part::Rod(1, bone) && MOTES[*index].at.y > 0.01)
            .map(|(index, _)| &cubes[index])
            .collect();
        assert_eq!(rods.len(), 3);
        let mut covered = 0.0;
        for rod in &rods {
            // Along the bone, within a few degrees, and on it.
            let axis = Quat::from_array(rod.orientation) * Vec3::Y;
            assert!(axis.dot(along).abs() > 0.99, "{axis} {along}");
            let from = at(rod) - start;
            assert!((from - along * from.dot(along)).length() < 0.02);
            assert!((0.0..=length).contains(&from.dot(along)));
            // Longer than wide, but no stick: 3 to 5 cm across.
            assert!(rod.scale[1] > rod.scale[0] * 1.6, "{:?}", rod.scale);
            assert!((0.028..0.05).contains(&rod.scale[0]), "{:?}", rod.scale);
            covered += rod.scale[1];
            widths[bone] += rod.scale[0] / 3.0;
        }
        // Together most of the bone, with gaps between them.
        assert!(
            covered > length * 0.6 && covered < length,
            "{covered} of {length}"
        );
        lengths[bone] = rods.iter().map(|rod| rod.scale[1]).sum::<f32>();
    }
    // Thinner towards the wrist.
    assert!(widths[1] < widths[0], "{widths:?}");
    // A few loose cubes about each bone, outside its rods, so that it reads as part of the swarm.
    for (bone, part) in [Part::Upper(1), Part::Lower(1)].into_iter().enumerate() {
        let (start, end) = (joints[bone], joints[bone + 1]);
        let along = (end - start).normalize();
        let loose: Vec<Vec3> = parts()
            .filter(|(_, is)| *is == part)
            .map(|(index, _)| at(&cubes[index]))
            .collect();
        assert_eq!(loose.len(), LOOSE);
        for cube in loose {
            let from = cube - start;
            let off = (from - along * from.dot(along)).length();
            assert!((0.025..0.09).contains(&off), "{off}");
        }
    }
    // With an elbow at the forearm's start.
    let (elbow, _) = parts()
        .find(|(index, part)| *part == Part::Rod(1, 1) && MOTES[*index].at.y == 0.0)
        .unwrap();
    assert!(at(&cubes[elbow]).distance(joints[1]) < 0.01);
    // Reaching farther than the arm, each rod stretches as its bone does.
    let (stretched, _) = reaching(Vec3::new(0.3, 1.0, -1.5));
    for (bone, length) in lengths.into_iter().enumerate() {
        let long: f32 = parts()
            .filter(|(index, part)| *part == Part::Rod(1, bone) && MOTES[*index].at.y > 0.01)
            .map(|(index, _)| stretched[index].scale[1])
            .sum();
        assert!((long / length - STRETCH).abs() < 0.01, "{long}");
    }
}

#[test]
fn each_hand_is_a_small_bright_cluster_with_a_larger_dimmer_gleam_behind_it_as_seen() {
    let hand = Vec3::new(0.3, 1.25, -0.45);
    let person = standing([None, Some(hand)]);
    let body = Body::new(&person, EYES, 2);
    let seen = |cues: &Cues| {
        let cubes = drawn(&person, &body, cues);
        let hands: Vec<Cube> = parts()
            .filter(|(_, part)| *part == Part::Hand(1))
            .map(|(index, _)| cubes[index])
            .collect();
        let gleams: Vec<Cube> = parts()
            .filter(|(_, part)| *part == Part::Gleam(1))
            .map(|(index, _)| cubes[index])
            .collect();
        (hands, gleams)
    };
    let (hands, gleams) = seen(&cues(0.0, 0.0));
    let eyes = EYES.unwrap();
    let largest = hands.iter().map(|cube| cube.scale[0]).fold(0.0, f32::max);
    let dimmest = hands.iter().map(|cube| cube.color[1]).fold(1.0, f32::min);
    assert!(hands.iter().all(|cube| at(cube).distance(hand) < 0.06));
    // Paler and brighter than the person's colour.
    assert!(hands.iter().all(|cube| cube.color[1] > COLOUR[1] * 1.2));
    assert_eq!(gleams.len(), 3);
    for gleam in &gleams {
        assert!(gleam.scale[0] > largest);
        assert!(gleam.color[1] < dimmest);
        // Behind the hand as seen, about HALO_BEHIND farther from the eyes than where it rests.
        assert!(at(gleam).distance(hand) < 0.06);
    }
    let far = |cubes: &[Cube]| {
        cubes
            .iter()
            .map(|cube| at(cube).distance(eyes))
            .sum::<f32>()
            / cubes.len() as f32
    };
    assert!(far(&gleams) > far(&hands) + HALO_BEHIND * 0.5);
    // Without eyes to see it, behind where the head faces.
    let (hands, gleams) = seen(&Cues {
        eyes: None,
        ..cues(0.0, 0.0)
    });
    let ahead = |cubes: &[Cube]| {
        cubes.iter().map(|cube| -cube.position[2]).sum::<f32>() / cubes.len() as f32
    };
    assert!(ahead(&gleams) < ahead(&hands));
}

/// The relative luminance of `color`, in linear light.
fn luminance(color: [f32; 3]) -> f32 {
    0.2126 * color[0] + 0.7152 * color[1] + 0.0722 * color[2]
}

/// How far `mote` reaches from where it rests along `axis`, turned as it is.
fn reaching(mote: &Mote, axis: Vec3) -> f32 {
    [Vec3::X, Vec3::Y, Vec3::Z]
        .into_iter()
        .zip(mote.size.to_array())
        .map(|(edge, size)| (mote.turn * edge).dot(axis).abs() * size / 2.0)
        .sum()
}

#[test]
fn the_head_s_middle_is_a_block_filling_it_within_its_shell_so_nothing_behind_shows_through() {
    // The first cube, still at the head's middle, unturned, at every detail.
    let block = &MOTES[0];
    assert_eq!(block.part, Part::Core);
    assert_eq!(block.at, Vec3::ZERO);
    assert_eq!(block.turn, Quat::IDENTITY);
    assert_eq!(block.wobble, Vec3::ZERO);
    assert_eq!(block.from, Detail::Head);
    assert_eq!(block.size, MIDDLE_BLOCK);
    // Across most of the head's width and height, and between, and a good part of its depth, so that from ahead, and
    // from well to either side, the gaps between the shell's cubes show its glow rather than what is behind the head.
    for (across, most) in [
        (Vec3::X, 0.55),
        (Vec3::Y, 0.55),
        (Vec3::new(1.0, 0.0, 1.0), 0.6),
        (Vec3::new(1.0, 0.0, -1.0), 0.6),
        (Vec3::Z, 0.35),
    ] {
        let across = across.normalize();
        let head = (across * HEAD).length();
        assert!(reaching(block, across) > head * most, "{across}");
    }
    // Within the inside of the shell everywhere, so that it never shows as a block.
    let inside = MOTES
        .iter()
        .filter(|mote| mote.part == Part::Shell)
        .map(|mote| (mote.at / HEAD).length())
        .fold(f32::MAX, f32::min);
    let corner = (MIDDLE_BLOCK / 2.0 / (HEAD * inside)).length();
    assert!(corner < 1.0, "{corner} {inside}");
}
