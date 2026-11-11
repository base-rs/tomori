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

/// Someone at `at` looking at `target`.
fn speaker(at: [f32; 3], target: [f32; 3]) -> Speaker {
    Speaker {
        head: head(at, target),
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

/// Where the agent's home is by default, where it comes from.
const HOME: [f32; 3] = [0.0, 1.4, -2.3];

/// The horizontal bearing of `at` from `eyes` and that of `from`, in degrees, the second brought within half a turn of
/// the first.
fn bearings(eyes: [f32; 3], at: [f32; 3], from: [f32; 3]) -> (f32, f32) {
    let at = degrees(eyes, at);
    let from = degrees(eyes, from);
    (at, at + wrapped((from - at).to_radians()).to_degrees())
}

#[test]
fn the_spot_is_on_the_straight_line_from_the_agent_to_them_2_m_from_them_facing_them() {
    let ada = speaker(EYES, [0.0, 1.6, -3.0]);
    // From ahead of her, beside her and behind her, near and far: on the line from her eyes to where it is.
    for from in [
        HOME,
        [3.0, 1.4, -1.0],
        [-4.5, 1.4, 0.5],
        [0.5, 1.4, 4.0],
        [-2.0, 2.2, 2.0],
        [0.0, 1.4, -2.05],
    ] {
        let spot = spot(from, &ada, &open(&[EYES])).expect("clear");
        assert!(
            (level(sub(spot.position, EYES)) - CONVERSATION).abs() < 1e-4,
            "{from:?}: {spot:?}"
        );
        let (at, along) = bearings(EYES, spot.position, from);
        assert!((at - along).abs() < 1e-3, "{from:?}: {spot:?}");
        // At a face's height, a little above her eyes, facing her.
        assert!((spot.position[1] - 1.7).abs() < 1e-5);
        let ahead = rotate(spot.orientation, [0.0, 0.0, 1.0]);
        assert!(apart(bearing(ahead), bearing(sub(EYES, spot.position))) < 1e-3);
    }
    // Behind her, it stands behind her.
    let behind = spot([0.0, 1.4, 4.0], &ada, &open(&[EYES])).expect("clear");
    assert!(behind.position[2] > 1.9, "{behind:?}");
    // Within reach of her already, it would only come nearer along the same line, or back off to 2 m.
    let close = spot([1.0, 1.4, 0.0], &ada, &open(&[EYES])).expect("clear");
    assert!((close.position[0] - 2.0).abs() < 1e-4, "{close:?}");
    // Where it stands at her eyes, about the vertical, there is no line: the way her head faces.
    let above = spot([0.0, 2.6, 0.0], &ada, &open(&[EYES])).expect("clear");
    assert!(degrees(EYES, above.position).abs() < 1e-3, "{above:?}");
}

#[test]
fn the_spot_never_depends_on_the_way_they_face() {
    let scene = open(&[EYES]);
    let from = [3.0, 1.4, -1.0];
    let ahead = spot(from, &speaker(EYES, [0.0, 1.6, -3.0]), &scene).expect("clear");
    // Turning round on the spot, or looking up or down, moves nothing.
    for target in [
        [0.0, 1.6, 3.0],
        [3.0, 1.6, 0.0],
        [-3.0, 0.0, 0.0],
        [0.0, 4.0, -0.5],
        [3.0, 1.6, -1.0],
    ] {
        let turned = speaker(EYES, target);
        assert_eq!(spot(from, &turned, &scene), Some(ahead), "{target:?}");
    }
}

#[test]
fn the_spot_stands_at_a_face_s_height_for_anyone_seated_or_standing_within_its_bounds() {
    for (eyes, height) in [(1.0, 1.1), (1.2, 1.3), (1.75, 1.85), (2.5, 2.4)] {
        let at = [0.0, eyes, 0.0];
        let spot = spot(HOME, &speaker(at, [0.0, eyes, -3.0]), &open(&[at])).expect("clear");
        assert!((spot.position[1] - height).abs() < 1e-5, "{eyes}");
    }
}

#[test]
fn where_the_line_is_blocked_it_swings_round_them_by_the_least_it_must_at_the_same_distance() {
    let ada = speaker(EYES, [0.0, 1.6, 3.0]);
    // Coming from straight ahead of her, along −Z, the spot on the line is at bearing 0.
    let from = [0.0, 1.4, -4.0];
    let swung = |scene: &Scene| {
        let spot = spot(from, &ada, scene).expect("clear");
        assert!((level(sub(spot.position, EYES)) - CONVERSATION).abs() < 1e-4);
        assert!(clear(&spot, EYES, scene, &scene.panels_near(EYES)));
        degrees(EYES, spot.position)
    };
    // A post just to the right of the line, at 2 m: it swings left, the least it must, a whole number of swings.
    let post = Solid::new([0.05, 0.0, -2.6], [0.5, 3.0, -1.4]);
    let scene = Scene::new(&[post], &[], Vec::new(), vec![EYES]);
    let turn = swung(&scene);
    assert!(turn < 0.0, "{turn}");
    assert!(
        (turn / SWING - (turn / SWING).round()).abs() < 1e-3,
        "{turn}"
    );
    // Not a swing less would do.
    let less = turn + SWING;
    let towards = less.to_radians();
    let nearer = Pose::upright_facing(
        [
            CONVERSATION * towards.sin(),
            1.7,
            -CONVERSATION * towards.cos(),
        ],
        EYES,
    );
    assert!(!clear(&nearer, EYES, &scene, &[]));
    // Mirrored, it swings right as far.
    let mirrored = Solid::new([-0.5, 0.0, -2.6], [-0.05, 3.0, -1.4]);
    let scene = Scene::new(&[mirrored], &[], Vec::new(), vec![EYES]);
    assert!((swung(&scene) + turn).abs() < 1e-3);
    // Someone standing on the line, a panel floating there, or a wall: round them, clear of each.
    let bob = [0.0, 1.6, -2.2];
    let scene = open(&[EYES, bob]);
    assert!(swung(&scene).abs() >= SWING);
    let spot = spot(from, &ada, &scene).expect("clear");
    assert!(from_box(&spot, bob) >= HEADS);
    let panel = Pose::upright_facing([0.0, 1.6, -2.0], EYES);
    let scene = Scene::new(&[], &[], vec![panel], vec![EYES]);
    assert!(swung(&scene).abs() >= SWING);
    // One far from her is not in the way.
    let far = Pose::upright_facing([0.0, 1.6, -8.0], EYES);
    let scene = Scene::new(&[], &[], vec![far], vec![EYES]);
    assert_eq!(swung(&scene), 0.0);
    let wall = Wall::new(
        "w",
        tomori_protocol::WallKind::Added,
        Pose::upright_facing([0.0, 1.5, -1.2], EYES),
    )
    .with_grid((2, 1).into());
    let scene = Scene::new(&[], &[wall], Vec::new(), vec![EYES]);
    assert!(swung(&scene).abs() >= SWING);
    // Blocked all round but behind her, it goes behind her.
    let front = Solid::new([-3.0, 0.0, -3.0], [3.0, 3.0, -0.5]);
    let scene = Scene::new(&[front], &[], Vec::new(), vec![EYES]);
    assert!(swung(&scene).abs() > 90.0);
    // With room nowhere 2 m from her, nowhere.
    let near = Solid::new([-3.0, 0.0, -3.0], [3.0, 3.0, -0.4]);
    let behind = Solid::new([-3.0, 0.0, 0.4], [3.0, 3.0, 3.0]);
    let sides = [
        Solid::new([-3.0, 0.0, -0.4], [-0.6, 3.0, 0.4]),
        Solid::new([0.6, 0.0, -0.4], [3.0, 3.0, 0.4]),
    ];
    let mut walled = vec![near, behind];
    walled.extend(sides);
    let scene = Scene::new(&walled, &[], Vec::new(), vec![EYES]);
    assert_eq!(super::spot(from, &ada, &scene), None);
    // Standing inside the scenery, nowhere.
    let inside = Scene::new(
        &[Solid::new([-3.0, 0.0, -3.0], [3.0, 3.0, 3.0])],
        &[],
        Vec::new(),
        vec![EYES],
    );
    assert_eq!(super::spot(from, &ada, &inside), None);
}

#[test]
fn arrived_it_stands_well_within_reach_so_that_it_never_moves_again_at_the_edge() {
    let ada = speaker(EYES, [0.0, 1.6, -3.0]);
    const { assert!(CONVERSATION <= NEAR - 1.0) };
    let spot = spot([5.0, 1.4, 0.0], &ada, &open(&[EYES])).expect("clear");
    assert!(near_enough(&spot, &ada));
    // She may step a metre away, any way, and it only turns to her.
    for way in 0..8 {
        let way = (way as f32 * 45.0).to_radians();
        let stepped = [way.sin() * 0.99, 1.6, -way.cos() * 0.99];
        let moved = speaker(stepped, [stepped[0], 1.6, -3.0]);
        assert!(near_enough(&spot, &moved), "{stepped:?}");
    }
}

#[test]
fn beside_an_object_it_stands_to_the_asker_s_left_off_their_line_of_sight_to_it() {
    let ada = head(EYES, [0.0, 1.6, -3.0]);
    let object = [0.0, 1.6, -3.0];
    let spot = beside(&ada, object, &open(&[EYES])).expect("clear");
    // 1.5 m away, 45° to her left, a little above her eyes.
    assert!((level(sub(spot.position, EYES)) - 1.5).abs() < 1e-4);
    assert!((degrees(EYES, spot.position) + 45.0).abs() < 1e-3);
    assert!((spot.position[1] - 1.7).abs() < 1e-5);
    // Facing her.
    let ahead = rotate(spot.orientation, [0.0, 0.0, 1.0]);
    let towards = sub(EYES, spot.position);
    assert!((bearing(ahead) - bearing(towards)).abs() < 1e-3);
    // Out of her way to it.
    assert!(!in_the_way(&spot, EYES, object));
    // The object sets the line, not the way her head faces.
    let spot = super::beside(&ada, [3.0, 1.6, 0.0], &open(&[EYES])).expect("clear");
    assert!((degrees(EYES, spot.position) - 45.0).abs() < 1e-3);
    // An object at her eyes, about the vertical: the way her head faces.
    let spot = super::beside(&ada, [0.0, 0.5, 0.0], &open(&[EYES])).expect("clear");
    assert!((degrees(EYES, spot.position) + 45.0).abs() < 1e-3);
    // A solid to her left: to her right.
    let left = Solid::new([-3.0, 0.0, -3.0], [-0.5, 3.0, 3.0]);
    let scene = Scene::new(&[left], &[], Vec::new(), vec![EYES]);
    let spot = super::beside(&ada, object, &scene).expect("clear");
    assert!((degrees(EYES, spot.position) - 45.0).abs() < 1e-3);
    // With room nowhere near her, even as far as the summons stood it, nowhere; nor inside the scenery.
    let near = Solid::new([-2.0, 0.0, -2.0], [2.0, 3.0, -0.4]);
    let behind = Solid::new([-2.0, 0.0, 0.4], [2.0, 3.0, 2.0]);
    let sides = [
        Solid::new([-2.0, 0.0, -0.4], [-0.6, 3.0, 0.4]),
        Solid::new([0.6, 0.0, -0.4], [2.0, 3.0, 0.4]),
    ];
    let mut walled = vec![near, behind];
    walled.extend(sides);
    let scene = Scene::new(&walled, &[], Vec::new(), vec![EYES]);
    assert_eq!(super::beside(&ada, object, &scene), None);
    let inside = Scene::new(
        &[Solid::new([-2.0, 0.0, -2.0], [2.0, 3.0, 2.0])],
        &[],
        Vec::new(),
        vec![EYES],
    );
    assert_eq!(super::beside(&ada, object, &inside), None);
}

#[test]
fn the_agent_s_body_is_its_reach_about_it_upright_and_level_however_it_faces() {
    let ahead = body(&Pose::upright_facing([0.0, 1.7, -2.0], EYES));
    let half = REACH.map(|reach| reach + MARGIN);
    assert!(ahead.contains([half[0] - 0.01, 1.7, -2.0]));
    assert!(!ahead.contains([half[0] + 0.01, 1.7, -2.0]));
    assert!(ahead.contains([0.0, 1.7 + half[1] - 0.01, -2.0 + half[2] - 0.01]));
    assert!(!ahead.contains([0.0, 1.7, -2.0 + half[2] + 0.01]));
    // Turned, its box turns with it, about the vertical.
    let aside = body(&Pose::upright_facing([2.0, 1.7, 0.0], EYES));
    assert!(aside.contains([2.0, 1.7, half[0] - 0.01]));
    assert!(!aside.contains([2.0 + half[2] + 0.01, 1.7, 0.0]));
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
fn it_stands_near_enough_to_need_no_move_within_3_m_whichever_way_they_face_and_whatever_it_stands_before()
 {
    let ada = speaker(EYES, [0.0, 1.6, -3.0]);
    let at = |turn: f32, distance: f32| {
        let towards = turn.to_radians();
        Pose::upright_facing(
            [distance * towards.sin(), 1.7, -distance * towards.cos()],
            EYES,
        )
    };
    assert!(near_enough(&at(-45.0, 1.5), &ada));
    assert!(near_enough(&at(40.0, 2.9), &ada));
    // Close up, far round, behind her or in her way to what she attends to, it only turns.
    assert!(near_enough(&at(-60.0, 1.5), &ada));
    assert!(near_enough(&at(180.0, 2.0), &ada));
    assert!(near_enough(&at(-10.0, 1.5), &ada));
    assert!(in_the_way(&at(-10.0, 1.5), EYES, [0.0, 1.6, -3.0]));
    // Distance alone moves it.
    assert!(!near_enough(&at(-45.0, 3.2), &ada));
    let screen = Solid::new([-2.0, 0.0, -1.0], [2.0, 3.0, -0.9]);
    let hidden = Scene::new(&[screen], &[], Vec::new(), vec![EYES]);
    assert!(!sees(&at(-45.0, 1.5), EYES, &hidden));
    assert!(sees(&at(-45.0, 1.5), EYES, &open(&[EYES])));
}

#[test]
fn a_huddle_is_served_from_within_4_m_of_its_middle() {
    let ada = speaker([-0.5, 1.6, 0.0], [-0.5, 1.6, -3.0]);
    let bob = speaker([0.5, 1.6, 0.0], [0.5, 1.6, -3.0]);
    let at = |position: [f32; 3]| Pose::upright_facing(position, EYES);
    assert!(serves(&at([-3.0, 1.7, -2.0]), &[ada, bob]));
    assert!(!serves(&at([-4.0, 1.7, -2.0]), &[ada, bob]));
}

#[test]
fn bearings_are_measured_clockwise_from_ahead_either_way_round() {
    assert!((towards(EYES, [1.0, 1.6, 0.0]) - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    assert!(heading(&head(EYES, [0.0, 1.6, -3.0])).abs() < 1e-5);
    // The agent faces along its +Z, towards whoever it faces.
    let agent = Pose::upright_facing([0.0, 1.7, -1.5], EYES);
    assert!((apart(faces(&agent), towards(agent.position, EYES))).abs() < 1e-5);
    assert!((apart(3.0, -3.0) - (std::f32::consts::TAU - 6.0)).abs() < 1e-5);
}

#[test]
fn a_huddle_is_served_from_the_straight_line_to_their_middle_clear_of_each_and_facing_it() {
    // Ada and Bob stand a metre apart, both looking along −Z at a rack; the agent comes from behind them.
    let ada = speaker([-0.5, 1.6, 0.0], [-0.5, 1.6, -4.0]);
    let bob = speaker([0.5, 1.7, 0.0], [0.5, 1.6, -4.0]);
    let scene = open(&[ada.head.position, bob.head.position]);
    let from = [0.0, 1.4, 5.0];
    let spot = huddle(from, &[ada, bob], &scene).expect("clear");
    let middle = [0.0, 1.65, 0.0];
    // About 2 m from the nearer of them, at their eyes' height on average, facing their middle.
    let nearest = [ada, bob]
        .iter()
        .map(|speaker| level(sub(spot.position, speaker.head.position)))
        .fold(f32::INFINITY, f32::min);
    assert!((2.0..2.15).contains(&nearest), "{nearest}");
    assert!((spot.position[1] - 1.75).abs() < 1e-4);
    let ahead = rotate(spot.orientation, [0.0, 0.0, 1.0]);
    assert!((bearing(ahead) - bearing(sub(middle, spot.position))).abs() < 1e-3);
    // On the line from their middle to it, behind them, whichever way they face.
    assert!(
        spot.position[0].abs() < 1e-4 && spot.position[2] > 1.0,
        "{spot:?}"
    );
    let turned = [
        speaker(ada.head.position, [3.0, 1.6, 3.0]),
        speaker(bob.head.position, [-3.0, 1.6, 3.0]),
    ];
    assert_eq!(huddle(from, &turned, &scene), Some(spot));
    // Standing at their middle, the first of them sets the line.
    let carl = speaker([0.0, 1.6, 0.5], [0.0, 1.6, 4.0]);
    let dee = speaker([0.0, 1.6, -0.5], [0.0, 1.6, -4.0]);
    let scene = open(&[carl.head.position, dee.head.position]);
    assert!(huddle([0.0, 3.0, 0.0], &[carl, dee], &scene).is_some());
    // Something on the line: it swings round them.
    let post = Solid::new([-0.6, 0.0, 1.6], [0.6, 3.0, 2.6]);
    let blocked = Scene::new(&[post], &[], Vec::new(), vec![ada.head.position]);
    let swung = huddle(from, &[ada, bob], &blocked).expect("clear");
    assert!(swung.position[0].abs() > 0.3, "{swung:?}");
    // Walled in, nowhere serves them.
    let walls = Scene::new(
        &[Solid::new([-3.0, 0.0, -3.0], [3.0, 3.0, 3.0])],
        &[],
        Vec::new(),
        vec![ada.head.position],
    );
    assert_eq!(huddle(from, &[ada, bob], &walls), None);
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
fn heads_stand_together_within_4_m_and_a_point_is_as_near_a_segment_as_its_nearest_point() {
    assert!(together(EYES, [3.0, 1.0, 2.0]));
    assert!(!together(EYES, [3.0, 1.6, 3.0]));
    assert!((near_segment([0.0, 1.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]) - 1.0).abs() < 1e-6);
}
