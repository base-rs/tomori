// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::time::{Duration, Instant};

use glam::{Quat, Vec3};

use super::*;

const FRAME: f32 = 1.0 / 72.0;

/// What someone's presence says: their head at `x` along the space, at standing eye height, facing −Z as an unturned
/// head does, and their right
/// controller a little in front of them, as their `reports`th report.
fn heard(x: f32, reports: u64) -> Heard {
    Heard {
        head: tomori_protocol::Pose::at([x, 1.6, 0.0]),
        controllers: [None, Some(tomori_protocol::Pose::at([x + 0.2, 1.2, -0.3]))],
        hands: [None, None],
        pointing: [None, None],
        reports,
    }
}

/// Eyes far enough from everyone not to make anyone fainter, behind them.
const AWAY: Vec3 = Vec3::new(0.0, 1.6, 10.0);

/// How far in each figure is drawn, by its number, from how large its head's first part is.
fn shown(people: &People) -> Vec<(u32, f32)> {
    people
        .figures()
        .iter()
        .map(|figure| (figure.source, figure.cubes[0].scale[0] / PARTS[0].1.x))
        .collect()
}

/// Where in a head's cubes its visor's halves, left and right, and its sheen are.
const VISOR_LEFT: usize = 6;
const VISOR_RIGHT: usize = 7;
const SHEEN_AT: usize = 8;

/// How lit the visor of the first figure is, from its sheen's colour.
fn lit(people: &People) -> f32 {
    let sheen = people.figures()[0].cubes[SHEEN_AT].color;
    (sheen[0] - SHEEN[0]) / (SHEEN_GLOW[0] - SHEEN[0])
}

#[test]
fn a_head_is_drawn_as_a_visor_block_facing_where_they_look_and_rays_drawn_in_full_end_in_a_dot() {
    let start = Instant::now();
    let mut people = People::default();
    let mut bob = heard(1.0, 0);
    let ray = |meets| {
        Some(tomori_protocol::Ray {
            aim: tomori_protocol::Pose::at([1.2, 1.2, -0.3]),
            length: 2.0,
            meets,
        })
    };
    bob.pointing = [ray(false), ray(true)];
    people.update(start, FRAME, 0, [(2, &bob)], Some(AWAY), None);
    let figure = &people.figures()[0];
    assert_eq!(figure.source, 2);
    // The head's parts, the ray meeting nothing, and the one meeting something with its dot.
    assert_eq!(figure.cubes.len(), PARTS.len() + 3);
    assert!(figure.cubes.len() <= CUBES);
    // The head stands about its eyes, a little behind them, its visor at its front, facing where they look.
    let middle = Vec3::from_array(figure.cubes[0].position);
    assert!(
        middle.distance(Vec3::new(1.0, 1.6, 0.05)) < 1e-4,
        "{middle}"
    );
    let visor = &figure.cubes[VISOR_LEFT];
    assert!(visor.position[2] < -0.02, "{:?}", visor.position);
    assert_eq!(visor.color, VISOR);
    let turned = Quat::from_array(visor.orientation);
    assert!((turned * Vec3::NEG_Z).distance(Vec3::NEG_Z) < 1e-4);
    // About 18 by 14 by 16 cm.
    let extent = |axis: usize| {
        figure.cubes[..3]
            .iter()
            .map(|cube| cube.scale[axis])
            .fold(0.0f32, f32::max)
    };
    assert_eq!([extent(0), extent(1), extent(2)], [0.18, 0.14, 0.16]);
    // Each ray reaches from its aim as far as it was drawn, fainter than this participant's own; only the one that
    // meets something is highlighted, and ends in a dot there.
    let rays = &figure.cubes[PARTS.len()..];
    let (idle, meets, dot) = (&rays[0], &rays[1], &rays[2]);
    assert!(Vec3::from_array(idle.position).distance(Vec3::new(1.2, 1.2, -1.3)) < 1e-4);
    assert!((idle.scale[2] - 2.0).abs() < 1e-5);
    assert_eq!(idle.color, RAY.map(|value| value * RAY_FAINTER));
    assert_eq!(meets.color, RAY_MEETS.map(|value| value * RAY_FAINTER));
    assert!(Vec3::from_array(dot.position).distance(Vec3::new(1.2, 1.2, -2.3)) < 1e-4);
    assert_eq!(dot.scale, [DOT; 3]);
    // A watcher seeing from inside their head draws their rays alone; with no rays, nothing of them.
    people.update(start, FRAME, 0, [(2, &bob)], Some(AWAY), Some(2));
    assert_eq!(people.figures()[0].cubes.len(), 3);
    bob.pointing = [None, None];
    bob.reports += 1;
    let later = |milliseconds| start + Duration::from_millis(milliseconds);
    people.update(later(20), FRAME, 0, [(2, &bob)], Some(AWAY), Some(2));
    // Drawn an update behind, as ever.
    assert_eq!(people.figures()[0].cubes.len(), 3);
    people.update(later(40), FRAME, 0, [(2, &bob)], Some(AWAY), Some(2));
    assert!(people.figures().is_empty());
}

#[test]
fn what_is_not_a_pose_or_a_ray_is_left_out_and_a_head_that_is_not_is_nobody() {
    let mut bob = heard(1.0, 0);
    bob.controllers[0] = Some(tomori_protocol::Pose {
        position: [f32::NAN, 0.0, 0.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    });
    let mut joints = [None; tomori_protocol::JOINT_COUNT];
    joints[1] = Some(tomori_protocol::Joint {
        position: [0.1, 1.0, -0.3],
        radius: 0.01,
    });
    bob.hands = [
        Some(tomori_protocol::Hand { joints }),
        Some(tomori_protocol::Hand {
            joints: [Some(tomori_protocol::Joint {
                position: [0.0; 3],
                radius: 5.0,
            }); tomori_protocol::JOINT_COUNT],
        }),
    ];
    bob.pointing = [
        Some(tomori_protocol::Ray {
            aim: tomori_protocol::Pose::at([0.0; 3]),
            length: -1.0,
            meets: false,
        }),
        None,
    ];
    let present = Present::of(&bob).expect("a head");
    assert_eq!(present.controllers[0], None);
    assert!(present.controllers[1].is_some());
    assert_eq!(
        present.joints[0].unwrap()[1],
        Some(Joint {
            position: Vec3::new(0.1, 1.0, -0.3),
            radius: 0.01
        })
    );
    assert_eq!(present.joints[1], None);
    assert_eq!(present.pointing, [None, None]);
    let unturned = Heard {
        head: tomori_protocol::Pose {
            position: [0.0; 3],
            orientation: [0.0; 4],
        },
        ..heard(0.0, 0)
    };
    assert_eq!(Present::of(&unturned), None);
    let mut people = People::default();
    people.update(Instant::now(), FRAME, 0, [(2, &unturned)], None, None);
    assert!(people.figures().is_empty() && people.drawn().is_empty());
}

#[test]
fn whole_presence_keeps_the_hands_figures_and_the_hand_drawn_is_the_one_a_client_side_holds_with() {
    // Bob walks along x at a metre a second, his right controller with him, sending at 72 Hz, his reports arriving late by
    // up to 20 ms, unevenly, as over Wi-Fi, each taken at the first frame after it arrives.
    let interval = 1000.0 / 72.0;
    let start = Instant::now();
    let late = |sent: usize| ((sent * 7919) % 23) as f32 * 20.0 / 22.0;
    let mut arrived = 0.0f32;
    let mut sends = (0..400)
        .map(|sent| {
            arrived = arrived.max(sent as f32 * interval + 3.0 + late(sent));
            (arrived, heard(sent as f32 * interval / 1000.0, sent as u64))
        })
        .peekable();
    let carriage = crate::anchors::Anchor {
        position: Vec3::new(0.0, 0.0, -2.0),
        orientation: Quat::from_rotation_y(0.5),
    };
    let mut people = People::default();
    let mut hands = crate::relayed::Relayed::<Hand>::new();
    let mut bob = heard(0.0, 0);
    let mut drawn = Vec::new();
    for frame in 0..360 {
        let now = frame as f32 * interval + 7.0;
        while let Some((_, arrived)) = sends.next_if(|(at, _)| *at <= now) {
            bob = arrived;
        }
        let at = start + Duration::from_secs_f32(now / 1000.0);
        people.update(at, FRAME, 0, [(2, &bob)], Some(AWAY), None);
        let present = people.drawn()[0].1;
        // The hand an environment's client side is given is the one drawn, carried into its coordinates.
        let (right, left) = crate::relayed::others(people.drawn(), carriage);
        let held = present.controllers[1].unwrap();
        assert_eq!(right[0].0, 2);
        assert!(
            right[0]
                .1
                .position
                .distance(carriage.unplace(held.position))
                < 1e-5
        );
        assert!(left.is_empty());
        // And it is drawn as the hand alone would be, between its updates.
        let alone = Present::of(&bob).unwrap().controllers[1].unwrap();
        let hand = hands.update(at, vec![(2, alone)])[0].1;
        assert!(hand.position.distance(held.position) < 1e-5);
        drawn.push(present.head.position.x);
    }
    // Drawn between, it never holds still, nor moves two frames' worth at once, 13.9 mm a frame.
    let steps: Vec<f32> = drawn
        .windows(2)
        .map(|pair| (pair[1] - pair[0]) * 1000.0)
        .collect();
    assert!(
        steps[20..].iter().all(|step| *step > 5.0 && *step < 25.0),
        "{steps:?}"
    );
}

#[test]
fn a_jump_fades_out_where_they_were_and_in_where_they_are_never_gliding_between() {
    let start = Instant::now();
    let at = |milliseconds: u64| start + Duration::from_millis(milliseconds);
    let mut people = People::default();
    for step in 0..10u64 {
        people.update(
            at(step * 14),
            FRAME,
            0,
            [(2, &heard(step as f32 * 0.014, step))],
            Some(AWAY),
            None,
        );
    }
    let before = people.drawn()[0].1.head.position.x;
    // Teleported four metres on.
    let far = heard(4.0, 10);
    let x = |people: &People| people.figures()[0].cubes[0].position[0];
    people.update(at(140), FRAME, 0, [(2, &far)], Some(AWAY), None);
    assert!((x(&people) - before).abs() < 1e-4, "where they were");
    assert!((shown(&people)[0].1 - 1.0).abs() < 1e-4);
    people.update(at(190), FRAME, 0, [(2, &far)], Some(AWAY), None);
    assert!((x(&people) - before).abs() < 1e-4);
    assert!((shown(&people)[0].1 - 0.5).abs() < 1e-3);
    // What they hold stays with the hand drawn.
    assert!((people.drawn()[0].1.head.position.x - before).abs() < 1e-4);
    // Then in where they are, never between.
    people.update(at(250), FRAME, 0, [(2, &far)], Some(AWAY), None);
    assert!((x(&people) - 4.0).abs() < 1e-4);
    assert!((shown(&people)[0].1 - 0.1).abs() < 1e-3);
    people.update(at(340), FRAME, 0, [(2, &far)], Some(AWAY), None);
    assert!((x(&people) - 4.0).abs() < 1e-4);
    assert_eq!(shown(&people), [(2, 1.0)]);
    // Moving a metre is no jump: drawn moving between.
    people.update(at(354), FRAME, 0, [(2, &heard(5.0, 11))], Some(AWAY), None);
    people.update(at(361), FRAME, 0, [(2, &heard(5.0, 11))], Some(AWAY), None);
    assert!(x(&people) > 4.0 && x(&people) < 5.0, "{}", x(&people));
}

#[test]
fn someone_silent_fades_out_and_back_in_as_they_report_again() {
    let start = Instant::now();
    let at = |milliseconds: u64| start + Duration::from_millis(milliseconds);
    let mut people = People::default();
    let still = heard(1.0, 0);
    people.update(at(0), FRAME, 0, [(2, &still)], Some(AWAY), None);
    // A report alike to the last is still heard.
    let again = heard(1.0, 1);
    people.update(at(1_500), FRAME, 0, [(2, &again)], Some(AWAY), None);
    people.update(at(3_400), FRAME, 0, [(2, &again)], Some(AWAY), None);
    assert_eq!(shown(&people), [(2, 1.0)]);
    // Two seconds after the last, they begin to fade, over half a second.
    people.update(at(3_750), FRAME, 0, [(2, &again)], Some(AWAY), None);
    assert!(
        (shown(&people)[0].1 - 0.5).abs() < 1e-3,
        "{:?}",
        shown(&people)
    );
    people.update(at(4_000), FRAME, 0, [(2, &again)], Some(AWAY), None);
    assert!(people.figures().is_empty());
    // Still given to an environment's client side, as what they hold stays in their hand.
    assert_eq!(people.drawn().len(), 1);
    people.update(at(4_100), FRAME, 0, [(2, &heard(1.0, 2))], Some(AWAY), None);
    assert_eq!(shown(&people), [(2, 1.0)]);
}

#[test]
fn someone_who_leaves_fades_where_they_last_were_and_a_number_forgotten_is_drawn_anew() {
    let start = Instant::now();
    let at = |milliseconds: u64| start + Duration::from_millis(milliseconds);
    let mut people = People::default();
    let (bob, cleo) = (heard(1.0, 0), heard(-1.0, 0));
    people.update(at(0), FRAME, 0, [(2, &bob), (3, &cleo)], Some(AWAY), None);
    people.update(at(14), FRAME, 0, [(3, &cleo)], Some(AWAY), None);
    // Bob fades out where he was over 0.3 s, given no longer to an environment's client side.
    assert_eq!(people.drawn().len(), 1);
    let leaving = |people: &People| {
        people
            .figures()
            .iter()
            .find(|figure| figure.source == 2)
            .map(|figure| {
                (
                    figure.cubes[0].position[0],
                    figure.cubes[0].scale[0] / PARTS[0].1.x,
                )
            })
    };
    assert_eq!(leaving(&people), Some((1.0, 1.0)));
    people.update(at(164), FRAME, 0, [(3, &cleo)], Some(AWAY), None);
    let (x, fade) = leaving(&people).unwrap();
    assert!(
        (x - 1.0).abs() < 1e-4 && (fade - 0.5).abs() < 1e-3,
        "{fade}"
    );
    people.update(at(314), FRAME, 0, [(3, &cleo)], Some(AWAY), None);
    assert_eq!(leaving(&people), None);
    // Forgotten, as on joining another space, nobody leaves: whoever has a number now is drawn where they first are.
    people.update(at(328), FRAME, 1, [(2, &heard(5.0, 0))], Some(AWAY), None);
    assert_eq!(shown(&people), [(2, 1.0)]);
    assert_eq!(people.figures()[0].cubes[0].position[0], 5.0);
}

#[test]
fn someone_close_to_the_eyes_is_drawn_fainter_and_not_at_all_closer_still() {
    let mut people = People::default();
    let now = Instant::now();
    let bob = heard(0.0, 0);
    let middle = Vec3::new(0.0, 1.6, 0.05);
    let seen_from = |people: &mut People, distance: f32| {
        people.update(
            now,
            FRAME,
            0,
            [(2, &bob)],
            Some(middle + Vec3::X * distance),
            None,
        );
        shown(people).first().map_or(0.0, |(_, fade)| *fade)
    };
    assert_eq!(seen_from(&mut people, 0.8), 1.0);
    assert!((seen_from(&mut people, NEAR) - 1.0).abs() < 1e-3);
    assert!((seen_from(&mut people, 0.45) - 0.5).abs() < 1e-3);
    assert_eq!(seen_from(&mut people, NEAREST), 0.0);
    assert_eq!(seen_from(&mut people, 0.1), 0.0);
    // Fainter, a figure shrinks about its middle.
    seen_from(&mut people, 0.45);
    let visor = Vec3::from_array(people.figures()[0].cubes[VISOR_LEFT].position);
    assert!((visor.z - (0.05 - 0.082 * 0.5)).abs() < 1e-4, "{visor}");
}

#[test]
fn a_visor_lights_while_its_head_faces_this_participant_within_a_cone_that_holds_at_its_edge() {
    let now = Instant::now();
    let mut people = People::default();
    // Bob, at home facing −Z, and this participant two metres ahead of him, at an angle from where he faces.
    let bob = heard(0.0, 0);
    let eyes = |degrees: f32| {
        Some(
            Vec3::new(0.0, 1.6, 0.0)
                + Quat::from_rotation_y(degrees.to_radians()) * Vec3::NEG_Z * 2.0,
        )
    };
    let look = |people: &mut People, degrees: f32, frames: usize| {
        for _ in 0..frames {
            people.update(now, FRAME, 0, [(2, &bob)], eyes(degrees), None);
        }
        lit(people)
    };
    // Twelve degrees off is not yet eye contact.
    assert_eq!(look(&mut people, 12.0, 30), 0.0);
    // Within ten it lights, over 0.2 s.
    let lighting = look(&mut people, 8.0, 5);
    assert!(
        (lighting - 5.0 * FRAME / GLOW_RISE).abs() < 1e-3,
        "{lighting}"
    );
    assert!((look(&mut people, 8.0, 10) - 1.0).abs() < 1e-5);
    // Glancing out to twelve degrees again, it holds; past fifteen, it goes out over 0.4 s.
    assert!((look(&mut people, 12.0, 30) - 1.0).abs() < 1e-5);
    let going = look(&mut people, 20.0, 9);
    assert!(
        (going - (1.0 - 9.0 * FRAME / GLOW_FALL)).abs() < 1e-3,
        "{going}"
    );
    assert_eq!(look(&mut people, 20.0, 30), 0.0);
    // Back at twelve, still out: it lights only within ten.
    assert_eq!(look(&mut people, 12.0, 30), 0.0);
    // A head wavering at the edge does not flicker, lit or not.
    for frame in 0..60 {
        assert_eq!(look(&mut people, [11.0, 14.5][frame % 2], 1), 0.0);
    }
    look(&mut people, 0.0, 30);
    for frame in 0..60 {
        let wavering = look(&mut people, [9.5, 14.5][frame % 2], 1);
        assert!((wavering - 1.0).abs() < 1e-5, "{wavering}");
    }
    // Lit, looking straight at this participant, the visor glows, both halves alike, with its sheen, the antenna's tip
    // and a little of its pods.
    look(&mut people, 0.0, 30);
    let cubes = &people.figures()[0].cubes;
    let near = |a: [f32; 3], b: [f32; 3]| Vec3::from_array(a).distance(Vec3::from_array(b)) < 1e-5;
    assert!(near(cubes[SHEEN_AT].color, SHEEN_GLOW));
    assert!(near(cubes[PARTS.len() - 1].color, SHEEN_GLOW));
    assert_eq!(cubes[VISOR_LEFT].color, cubes[VISOR_RIGHT].color);
    let half = 1.0 - GLOW_SIDE / 2.0;
    assert!(
        (cubes[VISOR_LEFT].color[0] - (VISOR[0] + (VISOR_GLOW[0] - VISOR[0]) * half)).abs() < 1e-5
    );
    assert!(cubes[4].color[0] > POD[0]);
    assert_eq!(cubes[0].color, SHELL);
    // Seen from their left, the left half is the lighter.
    look(&mut people, 8.0, 1);
    let cubes = &people.figures()[0].cubes;
    assert!(cubes[VISOR_LEFT].color[0] > cubes[VISOR_RIGHT].color[0]);
    // Only this participant, whom they look at, sees it: another, beside them, does not.
    let mut beside = People::default();
    for _ in 0..30 {
        beside.update(now, FRAME, 0, [(2, &bob)], eyes(30.0), None);
    }
    assert_eq!(lit(&beside), 0.0);
    // Nor someone looked at from farther than 8 m.
    let mut far = People::default();
    for _ in 0..30 {
        far.update(
            now,
            FRAME,
            0,
            [(2, &bob)],
            Some(Vec3::new(0.0, 1.6, -9.0)),
            None,
        );
    }
    assert_eq!(lit(&far), 0.0);
    let mut within = People::default();
    for _ in 0..30 {
        within.update(
            now,
            FRAME,
            0,
            [(2, &bob)],
            Some(Vec3::new(0.0, 1.6, -7.5)),
            None,
        );
    }
    assert!((lit(&within) - 1.0).abs() < 1e-5);
    // Without eyes, as before a head is tracked, nobody is looked at, and eyes at their head are not looked at either.
    people.update(now, 1.0, 0, [(2, &bob)], None, None);
    assert_eq!(lit(&people), 0.0);
    assert!(!gazes(
        &Present::of(&bob).unwrap().head,
        Some(Vec3::new(0.0, 1.6, 0.0)),
        true
    ));
}

#[test]
fn a_ray_between_updates_turns_and_reaches_between_and_whatever_one_lacks_is_the_latest() {
    let ray = |direction: Vec3, length: f32, meets: bool| Ray {
        origin: Vec3::ZERO,
        direction,
        length,
        meets,
    };
    let was = Present {
        pointing: [Some(ray(Vec3::NEG_Z, 1.0, false)), None],
        ..Present::of(&heard(0.0, 0)).unwrap()
    };
    let will = Present {
        pointing: [
            Some(ray(Vec3::X, 3.0, true)),
            Some(ray(Vec3::X, 1.0, false)),
        ],
        controllers: [None, None],
        ..Present::of(&heard(1.0, 1)).unwrap()
    };
    let between = was.between(&will, 0.5);
    let left = between.pointing[0].unwrap();
    assert!(
        left.direction
            .distance(Vec3::new(1.0, 0.0, -1.0).normalize())
            < 1e-5
    );
    assert_eq!((left.length, left.meets), (2.0, true));
    assert_eq!(between.pointing[1], will.pointing[1]);
    assert_eq!(between.controllers, [None, None]);
    assert!((between.head.position.x - 0.5).abs() < 1e-5);
    // Turned right round, it keeps the latest way.
    let back = Present {
        pointing: [Some(ray(Vec3::Z, 1.0, false)), None],
        ..was
    };
    assert_eq!(
        was.between(&back, 0.5).pointing[0].unwrap().direction,
        Vec3::Z
    );
    // Joints are drawn between where both have them.
    let joints = |x: f32| {
        let mut joints: Joints = [None; tomori_protocol::JOINT_COUNT];
        joints[0] = Some(Joint {
            position: Vec3::new(x, 1.0, 0.0),
            radius: 0.01 + x / 100.0,
        });
        joints[1] = Some(Joint {
            position: Vec3::ZERO,
            radius: 0.01,
        });
        Some(joints)
    };
    let mut lost = joints(1.0).unwrap();
    lost[1] = None;
    let hands = Present {
        joints: [joints(0.0), joints(1.0)],
        ..was
    }
    .between(
        &Present {
            joints: [joints(1.0), Some(lost)],
            ..was
        },
        0.5,
    );
    let [left, right] = hands.joints.map(Option::unwrap);
    assert!((left[0].unwrap().position.x - 0.5).abs() < 1e-6);
    assert!((left[0].unwrap().radius - 0.015).abs() < 1e-6);
    assert_eq!(right[1], None);
}
