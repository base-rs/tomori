// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

/// A head at `at` looking at `target`, as a head looks along its −Z.
fn head(at: [f32; 3], target: [f32; 3]) -> Pose {
    let along = sub(target, at);
    let yaw = (-along[0]).atan2(-along[2]);
    let pitch = along[1].atan2(level(along));
    let (a, c) = (yaw / 2.0).sin_cos();
    let (e, f) = (pitch / 2.0).sin_cos();
    Pose {
        position: at,
        orientation: [c * e, a * f, -a * e, c * f],
    }
}

/// Someone at `at` attending to `target`.
fn speaker(at: [f32; 3], target: [f32; 3]) -> Speaker {
    Speaker {
        head: head(at, target),
        focus: target,
    }
}

/// Nothing in the way but `heads`.
fn open(heads: &[[f32; 3]]) -> Scene {
    Scene::new(&[], &[], Vec::new(), heads.to_vec())
}

/// The bearing of `at` from `eyes`, in degrees clockwise from −Z.
fn degrees(eyes: [f32; 3], at: [f32; 3]) -> f32 {
    bearing(sub(at, eyes)).to_degrees()
}

const EYES: [f32; 3] = [0.0, 1.6, 0.0];

#[test]
fn the_spot_stands_beside_the_speaker_to_their_left_at_a_conversational_distance_facing_them() {
    let ada = speaker(EYES, [0.0, 1.6, -3.0]);
    let spot = spot(&ada, &open(&[EYES])).expect("clear");
    // 1.5 m away, 45° to her left, a little above her eyes.
    assert!((level(sub(spot.position, EYES)) - 1.5).abs() < 1e-4);
    assert!((degrees(EYES, spot.position) + 45.0).abs() < 1e-3);
    assert!((spot.position[1] - 1.7).abs() < 1e-5);
    // Facing her.
    let ahead = rotate(spot.orientation, [0.0, 0.0, 1.0]);
    let towards = sub(EYES, spot.position);
    assert!((bearing(ahead) - bearing(towards)).abs() < 1e-3);
    // Out of her way to what she attends to.
    assert!(!in_the_way(&spot, EYES, ada.focus));
    // What she attends to sets the line, not the way her head faces.
    let aside = Speaker {
        focus: [3.0, 1.6, 0.0],
        ..ada
    };
    let spot = super::spot(&aside, &open(&[EYES])).expect("clear");
    assert!((degrees(EYES, spot.position) - 45.0).abs() < 1e-3);
    // Attending to nothing beyond her eyes, the way her head faces.
    let close = Speaker { focus: EYES, ..ada };
    let spot = super::spot(&close, &open(&[EYES])).expect("clear");
    assert!((degrees(EYES, spot.position) + 45.0).abs() < 1e-3);
}

#[test]
fn the_spot_stands_at_a_face_s_height_for_anyone_seated_or_standing_within_its_bounds() {
    for (eyes, height) in [(1.0, 1.1), (1.2, 1.3), (1.75, 1.85), (2.5, 2.4)] {
        let at = [0.0, eyes, 0.0];
        let spot = spot(&speaker(at, [0.0, eyes, -3.0]), &open(&[at])).expect("clear");
        assert!((spot.position[1] - height).abs() < 1e-5, "{eyes}");
    }
}

#[test]
fn the_spot_keeps_clear_of_the_scenery_walls_panels_and_heads_trying_the_other_side_then_further() {
    let ada = speaker(EYES, [0.0, 1.6, -3.0]);
    // A solid to her left: it stands to her right.
    let left = Solid::new([-3.0, 0.0, -3.0], [-0.5, 3.0, 3.0]);
    let scene = Scene::new(&[left], &[], Vec::new(), vec![EYES]);
    let spot = spot(&ada, &scene).expect("clear");
    assert!((degrees(EYES, spot.position) - 45.0).abs() < 1e-3);
    // Someone standing there: to her right, clear of their head.
    let bob = [-1.06, 1.6, -1.06];
    let spot = super::spot(&ada, &open(&[EYES, bob])).expect("clear");
    assert!((degrees(EYES, spot.position) - 45.0).abs() < 1e-3);
    assert!(from_box(&spot, bob) >= HEADS);
    // A panel floating there, near her: the other side.
    let panel = Pose::upright_facing([-1.06, 1.6, -1.06], EYES);
    let scene = Scene::new(&[], &[], vec![panel], vec![EYES]);
    let spot = super::spot(&ada, &scene).expect("clear");
    assert!(degrees(EYES, spot.position) > 0.0);
    // One far from her is not in her way.
    let far = Pose::upright_facing([-6.0, 1.6, -6.0], EYES);
    let scene = Scene::new(&[], &[], vec![far], vec![EYES]);
    let spot = super::spot(&ada, &scene).expect("clear");
    assert!((degrees(EYES, spot.position) + 45.0).abs() < 1e-3);
    // A wall close in front of her, across both of the first bearings: further round, clear of it.
    let wall = Wall::new(
        "w",
        tomori_protocol::WallKind::Added,
        Pose::upright_facing([0.0, 1.5, -1.2], EYES),
    )
    .with_grid((2, 1).into());
    let scene = Scene::new(&[], &[wall], Vec::new(), vec![EYES]);
    let spot = super::spot(&ada, &scene).expect("clear");
    assert!(degrees(EYES, spot.position).abs() > 60.0, "{spot:?}");
    // With room nowhere near her, it stands as far as the summons stood it; with none at all, nowhere.
    let near = Solid::new([-2.0, 0.0, -2.0], [2.0, 3.0, -0.4]);
    let behind = Solid::new([-2.0, 0.0, 0.4], [2.0, 3.0, 2.0]);
    let sides = [
        Solid::new([-2.0, 0.0, -0.4], [-0.6, 3.0, 0.4]),
        Solid::new([0.6, 0.0, -0.4], [2.0, 3.0, 0.4]),
    ];
    let mut walled = vec![near, behind];
    walled.extend(sides);
    let scene = Scene::new(&walled, &[], Vec::new(), vec![EYES]);
    assert_eq!(super::spot(&ada, &scene), None);
    // Standing inside the scenery, nowhere.
    let inside = Scene::new(
        &[Solid::new([-2.0, 0.0, -2.0], [2.0, 3.0, 2.0])],
        &[],
        Vec::new(),
        vec![EYES],
    );
    assert_eq!(super::spot(&ada, &inside), None);
}

#[test]
fn a_spot_within_its_reach_of_the_line_to_what_they_attend_to_is_refused() {
    let focus = [0.0, 1.6, -3.0];
    let at = |turn: f32, distance: f32| {
        let towards = turn.to_radians();
        Pose::upright_facing(
            [distance * towards.sin(), 1.7, -distance * towards.cos()],
            EYES,
        )
    };
    // At 25°, its reach, 24° either side of its middle from 1.5 m, comes within 12° of the line; at 45° it does not.
    assert!(in_the_way(&at(-25.0, 1.5), EYES, focus));
    assert!(in_the_way(&at(25.0, 1.5), EYES, focus));
    assert!(!in_the_way(&at(-45.0, 1.5), EYES, focus));
    assert!(!in_the_way(&at(70.0, 1.2), EYES, focus));
    // Attending to what is straight above or below them, nothing is in the way.
    assert!(!in_the_way(&at(0.0, 1.5), EYES, [0.0, 0.0, 0.0]));
}

#[test]
fn it_stands_near_enough_to_need_no_move_only_within_reach_ahead_seen_and_out_of_the_way() {
    let ada = speaker(EYES, [0.0, 1.6, -3.0]);
    let scene = open(&[EYES]);
    let at = |turn: f32, distance: f32| {
        let towards = turn.to_radians();
        Pose::upright_facing(
            [distance * towards.sin(), 1.7, -distance * towards.cos()],
            EYES,
        )
    };
    assert!(near_enough(&at(-45.0, 1.5), &ada, &scene));
    assert!(near_enough(&at(40.0, 2.9), &ada, &scene));
    // Too far, too far round, in the way, or out of sight.
    assert!(!near_enough(&at(-45.0, 3.2), &ada, &scene));
    assert!(!near_enough(&at(-60.0, 1.5), &ada, &scene));
    assert!(!near_enough(&at(-10.0, 1.5), &ada, &scene));
    let screen = Solid::new([-2.0, 0.0, -1.0], [2.0, 3.0, -0.9]);
    let hidden = Scene::new(&[screen], &[], Vec::new(), vec![EYES]);
    assert!(!near_enough(&at(-45.0, 1.5), &ada, &hidden));
    assert!(!sees(&at(-45.0, 1.5), EYES, &hidden));
    assert!(sees(&at(-45.0, 1.5), EYES, &scene));
}

#[test]
fn a_huddle_is_served_from_the_side_most_of_them_face_clear_of_each_and_facing_their_middle() {
    // Ada and Bob stand a metre apart, both looking along −Z at a rack.
    let ada = speaker([-0.5, 1.6, 0.0], [-0.5, 1.6, -4.0]);
    let bob = speaker([0.5, 1.7, 0.0], [0.5, 1.6, -4.0]);
    let scene = open(&[ada.head.position, bob.head.position]);
    let spot = huddle(&[ada, bob], &scene).expect("clear");
    let middle = [0.0, 1.65, 0.0];
    // About 1.5 m from the nearer of them, at their eyes' height on average, facing their middle.
    let nearest = [ada, bob]
        .iter()
        .map(|speaker| level(sub(spot.position, speaker.head.position)))
        .fold(f32::INFINITY, f32::min);
    assert!((1.5..1.65).contains(&nearest), "{nearest}");
    assert!((spot.position[1] - 1.75).abs() < 1e-4);
    let ahead = rotate(spot.orientation, [0.0, 0.0, 1.0]);
    assert!((bearing(ahead) - bearing(sub(middle, spot.position))).abs() < 1e-3);
    // Out of the way of what each attends to, off the side they face.
    assert!(!in_the_way(&spot, ada.head.position, ada.focus));
    assert!(!in_the_way(&spot, bob.head.position, bob.focus));
    assert!(spot.position[2] < 0.0);
    // Facing every way at once, the first of them sets the side.
    let carl = speaker([0.0, 1.6, 0.5], [0.0, 1.6, 4.0]);
    let dee = speaker([0.0, 1.6, -0.5], [0.0, 1.6, -4.0]);
    let scene = open(&[carl.head.position, dee.head.position]);
    assert!(huddle(&[carl, dee], &scene).is_some());
    // Walled in, nowhere serves them.
    let walls = Scene::new(
        &[Solid::new([-3.0, 0.0, -3.0], [3.0, 3.0, 3.0])],
        &[],
        Vec::new(),
        vec![ada.head.position],
    );
    assert_eq!(huddle(&[ada, bob], &walls), None);
}

#[test]
fn the_way_there_is_straight_raised_over_heads_or_aside_of_the_scenery_else_streamed() {
    let from = [0.0, 1.4, -2.3];
    let to = [-1.06, 1.7, -1.06];
    assert_eq!(route(from, to, &open(&[EYES])), Route::Straight);
    // Someone standing in the way: it rises over their head.
    let bob = [0.97, 1.6, -1.68];
    match route([3.0, 1.4, -2.3], to, &open(&[EYES, bob])) {
        Route::Via(via) => assert!((via[1] - (1.6 + OVER)).abs() < 1e-4, "{via:?}"),
        other => panic!("{other:?}"),
    }
    // A pillar in the way, too tall to rise over: it goes round it.
    let pillar = Solid::new([-0.8, 0.0, -1.9], [-0.3, 4.0, -1.4]);
    let scene = Scene::new(&[pillar], &[], Vec::new(), vec![EYES]);
    match route(from, to, &scene) {
        Route::Via(via) => assert!((via[1] - (from[1] + to[1]) / 2.0).abs() < 1e-4),
        other => panic!("{other:?}"),
    }
    // A wall it cannot pass either way, or far, it streams.
    let wall = Solid::new([-5.0, 0.0, -1.8], [5.0, 5.0, -1.6]);
    let scene = Scene::new(&[wall], &[], Vec::new(), vec![EYES]);
    assert_eq!(route(from, to, &scene), Route::Stream);
    assert_eq!(route(from, [0.0, 1.6, 4.0], &open(&[])), Route::Stream);
    // From where it already is, it is there.
    assert_eq!(route(to, to, &open(&[])), Route::Straight);
}

#[test]
fn what_someone_attends_to_with_nothing_else_known_is_ahead_of_their_head() {
    let ada = head(EYES, [0.0, 1.6, -3.0]);
    let ahead = gaze(&ada);
    assert!(distance(ahead, [0.0, 1.6, -2.0]) < 1e-4);
    assert!(together(EYES, [3.0, 1.0, 2.0]));
    assert!(!together(EYES, [3.0, 1.6, 3.0]));
    assert!((near_segment([0.0, 1.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]) - 1.0).abs() < 1e-6);
}
