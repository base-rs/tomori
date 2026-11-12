// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Both hands on one panel or wall: the second hand's grip, its ray on it however far away or the hand touching it,
//! takes hold of it too, and the two move, turn and stretch it between them. See `design/projection.md`.

use super::*;

/// Where the hands are: a little either side of the chest, as someone holds out both controllers.
const LEFT_HAND: Vec3 = Vec3::new(-0.2, 1.3, -0.3);
const RIGHT_HAND: Vec3 = Vec3::new(0.2, 1.3, -0.3);

/// A ray from `hand` towards `point`.
fn from(hand: Vec3, point: Vec3) -> Option<Ray> {
    Ray::new(hand, point - hand)
}

/// A frame of `projection` with the right hand's ray and grip and the left's, as given, for a participant with `rights`.
fn both<'a>(
    projection: &'a Projection,
    right: (Option<Ray>, f32),
    left: (Option<Ray>, f32),
    rights: editing::Rights,
    now: Instant,
) -> Input<'a> {
    Input {
        rights,
        left: Hold {
            ray: left.0,
            grip: Some(left.1),
            active: true,
            pointing: true,
            ..Hold::default()
        },
        ..input(projection, right.0, right.1, now)
    }
}

/// A projection of one panel, `p`, floating `distance` ahead of home, facing it, as large as `size`.
fn floating(distance: f32, size: Option<f32>) -> Projection {
    let at = Vec3::new(0.0, 1.5, -distance);
    let pose = Pose::upright_facing(at.to_array(), HOME.to_array());
    Projection::new("s", "sky")
        .with_walls(vec![wall(TOMORI_WALL, WallKind::Tomori, 0.0, 9.0)])
        .with_panels(vec![
            panel("p", Place::Free { pose })
                .with_size(size)
                .with_version(41),
        ])
}

/// The plain text of each of `output`'s sheets.
fn texts(output: &Output) -> Vec<String> {
    output
        .sheets
        .iter()
        .filter_map(|sheet| sheet.canvas.as_deref().map(Canvas::plain))
        .collect()
}

/// How many corner marks `output` draws.
fn marks(output: &Output) -> usize {
    output
        .sheets
        .iter()
        .filter(|sheet| {
            sheet
                .canvas
                .as_deref()
                .is_some_and(|canvas| *canvas == mark_face())
        })
        .count()
}

/// The middle of the panel `id` as drawn in `output`.
fn drawn(output: &Output, title: &str) -> Mat4 {
    output
        .sheets
        .iter()
        .find(|sheet| sheet.canvas.as_deref().unwrap().plain().starts_with(title))
        .and_then(|sheet| sheet.model)
        .expect("the panel is drawn")
}

/// How large `model` draws its face, against its own size.
fn scale_of(model: Mat4) -> f32 {
    model.x_axis.truncate().length()
}

#[test]
fn the_other_hand_s_ray_takes_hold_of_the_panel_one_carries_however_far_and_both_carry_it() {
    for distance in [1.0, 3.0, 6.0] {
        let shown = floating(distance, None);
        let middle = Vec3::new(0.0, 1.5, -distance);
        let now = Instant::now();
        let mut projector = Projector::default();
        let none = editing::Rights::default();
        let right = from(RIGHT_HAND, middle);
        let left = from(LEFT_HAND, middle);
        projector.update(both(&shown, (right, 0.0), (None, 0.0), none, now));
        projector.update(both(&shown, (right, 1.0), (None, 0.0), none, now));
        assert_eq!(projector.held_in(Side::Right), Some("p"));
        // The left ray on it says its grip would take hold of it too, and its squeeze does, felt on the left.
        let output = projector.update(both(&shown, (right, 1.0), (left, 0.0), none, now));
        assert!(projector.joins(Side::Left));
        assert_eq!(
            output.left.pointed.as_ref().map(|(id, ..)| id.as_str()),
            Some("p")
        );
        let output = projector.update(both(&shown, (right, 1.0), (left, 1.0), none, now));
        assert_eq!(output.joined, Some(Side::Left));
        assert!(projector.two_handed());
        assert!(projector.carrying_in(Side::Left) && projector.carrying_in(Side::Right));
        assert_eq!(projector.held_in(Side::Left), None, "{distance}");
        // Shown with its corners marked, and not yet its size, which has not changed.
        assert_eq!(marks(&output), 4);
        assert!(!texts(&output).iter().any(|text| text.contains('%')));
        // Both hands carried half a metre to the right, pointing as they did, it comes along, unturned and its size.
        let shift = Vec3::X * 0.5;
        let output = projector.update(both(
            &shown,
            (from(RIGHT_HAND + shift, middle + shift), 1.0),
            (from(LEFT_HAND + shift, middle + shift), 1.0),
            none,
            now,
        ));
        let model = drawn(&output, "Panel p");
        assert!(
            model.w_axis.truncate().distance(middle + shift) < 1e-3,
            "{distance}: {:?}",
            model.w_axis
        );
        assert!((scale_of(model) - 1.0).abs() < 1e-4);
        // The right hand drawn back and the left pushed forward, as a wheel turns, it turns with them about the
        // vertical, upright.
        let turned = Quat::from_rotation_y(0.4);
        let centre = (LEFT_HAND + RIGHT_HAND) / 2.0;
        let before = yaw_of(model);
        let output = projector.update(both(
            &shown,
            (from(centre + turned * (RIGHT_HAND - centre), middle), 1.0),
            (from(centre + turned * (LEFT_HAND - centre), middle), 1.0),
            none,
            now,
        ));
        let model = drawn(&output, "Panel p");
        assert!(((yaw_of(model) - before) - 0.4).abs() < 1e-3, "{distance}");
        assert!(model.y_axis.truncate().normalize().distance(Vec3::Y) < 1e-4);
    }
}

#[test]
fn spreading_the_hands_stretches_a_panel_about_their_middle_in_steps_within_its_bounds_and_closing_them_shrinks_it()
 {
    let shown = floating(2.0, None);
    let middle = Vec3::new(0.0, 1.5, -2.0);
    let now = Instant::now();
    let mut projector = Projector::default();
    let none = editing::Rights::default();
    let hands = |apart: f32| {
        let (left, right) = (
            Vec3::new(-apart / 2.0, 1.3, -0.3),
            Vec3::new(apart / 2.0, 1.3, -0.3),
        );
        (from(right, middle), from(left, middle))
    };
    let (right, left) = hands(0.4);
    projector.update(both(&shown, (right, 1.0), (None, 0.0), none, now));
    projector.update(both(&shown, (right, 1.0), (left, 0.0), none, now));
    projector.update(both(&shown, (right, 1.0), (left, 1.0), none, now));
    assert!(projector.two_handed());
    // Half as far apart again, it is half as large again, and says so, briefly.
    let (right, left) = hands(0.6);
    let output = projector.update(both(&shown, (right, 1.0), (left, 1.0), none, now));
    let model = drawn(&output, "Panel p");
    assert!((scale_of(model) - 1.5).abs() < 1e-4);
    assert!(
        texts(&output).iter().any(|text| text.trim() == "150%"),
        "{:?}",
        texts(&output)
    );
    let later = now + SIZE_SHOWN;
    let output = projector.update(both(&shown, (right, 1.0), (left, 1.0), none, later));
    assert!(!texts(&output).iter().any(|text| text.contains('%')));
    // In steps of 5%: a little further apart is the nearest step.
    let (right, left) = hands(0.635);
    let output = projector.update(both(&shown, (right, 1.0), (left, 1.0), none, later));
    assert!((scale_of(drawn(&output, "Panel p")) - 1.6).abs() < 1e-4);
    // Grown about the middle of the points the hands hold, which stays where it was.
    assert!(drawn(&output, "Panel p").w_axis.truncate().distance(middle) < 0.05);
    // No larger than the most, however far apart, ticking as it reaches it.
    let (right, left) = hands(4.0);
    let output = projector.update(both(&shown, (right, 1.0), (left, 1.0), none, later));
    assert_eq!(output.bounded, Some(Side::Left));
    assert_eq!(
        scale_of(drawn(&output, "Panel p")),
        tomori_protocol::FLOAT_SIZES.1
    );
    assert!(
        texts(&output)
            .iter()
            .any(|text| text.contains("400% · largest"))
    );
    let output = projector.update(both(&shown, (right, 1.0), (left, 1.0), none, later));
    assert_eq!(output.bounded, None, "it ticks once");
    // Closed together, it shrinks, to no smaller than the least, its text never smaller than half a standard slot's.
    let (right, left) = hands(0.01);
    let output = projector.update(both(&shown, (right, 1.0), (left, 1.0), none, later));
    assert!((scale_of(drawn(&output, "Panel p")) - 0.5).abs() < 1e-4);
    assert!(
        texts(&output)
            .iter()
            .any(|text| text.contains("50% · smallest"))
    );
    // Let go of both, it floats where it is at that size, sent with the version both hands took hold of it at.
    let output = projector.update(both(&shown, (right, 0.0), (left, 0.0), none, later));
    let Some((panel, Target::Sized(pose, resized))) = output.placed else {
        unreachable!(
            "expected a panel left floating at its size, not {:?}",
            output.placed
        );
    };
    assert_eq!(panel, "p");
    assert_eq!(
        resized,
        Resized {
            size: 0.5,
            version: 41
        }
    );
    assert!(Vec3::from_array(pose.position).distance(middle) < 0.05);
    assert!(!projector.two_handed() && projector.held().is_none());
    // Shown so until the server agrees.
    let output = projector.update(both(&shown, (None, 0.0), (None, 0.0), none, later));
    assert!((scale_of(drawn(&output, "Panel p")) - 0.5).abs() < 1e-4);
}

#[test]
fn a_panel_floating_at_a_size_of_its_own_is_drawn_met_and_carried_one_handed_at_that_size() {
    let shown = floating(2.0, Some(2.0));
    let middle = Vec3::new(0.0, 1.5, -2.0);
    let now = Instant::now();
    let mut projector = Projector::default();
    let output = projector.update(input(&shown, None, 0.0, now));
    assert!((scale_of(drawn(&output, "Panel p")) - 2.0).abs() < 1e-4);
    // Met beyond where a panel of its own size would end.
    let (width, _) = fitted_size(&shown.panels[0], Fit::Text);
    let edge = middle + Vec3::X * width * 0.9;
    let output = projector.update(input(&shown, at(edge), 0.0, now));
    assert_eq!(
        output.pointed.as_ref().map(|(id, ..)| id.as_str()),
        Some("p")
    );
    // Carried with one hand and let go, it keeps its size, sent without one, so that the server keeps it too.
    projector.update(input(&shown, at(edge), 1.0, now));
    let output = projector.update(input(&shown, at(edge + Vec3::Y * 0.2), 1.0, now));
    assert!((scale_of(drawn(&output, "Panel p")) - 2.0).abs() < 1e-4);
    let output = projector.update(input(&shown, at(edge + Vec3::Y * 0.2), 0.0, now));
    assert!(matches!(output.placed, Some((_, Target::Free(_)))));
}

#[test]
fn the_other_hand_touching_the_panel_takes_hold_of_it_too_wherever_its_ray_points() {
    let shown = floating(0.8, None);
    let middle = Vec3::new(0.0, 1.5, -0.8);
    let now = Instant::now();
    let mut projector = Projector::default();
    let none = editing::Rights::default();
    let right = from(RIGHT_HAND, middle);
    projector.update(both(&shown, (right, 1.0), (None, 0.0), none, now));
    assert_eq!(projector.held_in(Side::Right), Some("p"));
    // The left hand at its left edge, pointing away from it, down at the floor.
    let (width, _) = fitted_size(&shown.panels[0], Fit::Text);
    let hand = middle - Vec3::X * (width / 2.0) + Vec3::Z * 0.05;
    let away = from(hand, hand - Vec3::Y);
    projector.update(both(&shown, (right, 1.0), (away, 0.0), none, now));
    assert!(projector.joins(Side::Left));
    let output = projector.update(both(&shown, (right, 1.0), (away, 1.0), none, now));
    assert_eq!(output.joined, Some(Side::Left));
    // A hand further off takes nothing of it.
    let mut projector = Projector::default();
    projector.update(both(&shown, (right, 1.0), (None, 0.0), none, now));
    let far = from(hand - Vec3::X * (TOUCH + 0.05), hand - Vec3::Y);
    projector.update(both(&shown, (right, 1.0), (far, 0.0), none, now));
    assert!(!projector.joins(Side::Left));
    let output = projector.update(both(&shown, (right, 1.0), (far, 1.0), none, now));
    assert_eq!(output.joined, None);
    assert!(!projector.two_handed());
}

#[test]
fn letting_go_with_either_hand_leaves_the_other_carrying_it_at_its_new_size_without_a_jump() {
    for keeps in Side::BOTH {
        let shown = floating(2.0, None);
        let middle = Vec3::new(0.0, 1.5, -2.0);
        let now = Instant::now();
        let mut projector = Projector::default();
        let none = editing::Rights::default();
        let right = from(RIGHT_HAND, middle);
        let left = from(LEFT_HAND, middle);
        projector.update(both(&shown, (right, 1.0), (None, 0.0), none, now));
        projector.update(both(&shown, (right, 1.0), (left, 1.0), none, now));
        assert!(projector.two_handed());
        // Spread, it grows by a quarter.
        let wide = (
            from(RIGHT_HAND + Vec3::X * 0.05, middle),
            from(LEFT_HAND - Vec3::X * 0.05, middle),
        );
        let output = projector.update(both(&shown, (wide.0, 1.0), (wide.1, 1.0), none, now));
        let before = drawn(&output, "Panel p");
        assert!((scale_of(before) - 1.25).abs() < 1e-4);
        // One hand lets go: the other carries it on, where it was, as large.
        let grips = match keeps {
            Side::Right => (1.0, 0.0),
            Side::Left => (0.0, 1.0),
        };
        let output = projector.update(both(
            &shown,
            (wide.0, grips.0),
            (wide.1, grips.1),
            none,
            now,
        ));
        assert!(
            output.placed.is_none() && output.left.placed.is_none(),
            "{keeps:?}"
        );
        assert!(!projector.two_handed());
        assert_eq!(projector.held_in(keeps), Some("p"), "{keeps:?}");
        assert_eq!(projector.held_in(keeps.other()), None);
        let after = drawn(&output, "Panel p");
        assert!(
            after.w_axis.truncate().distance(before.w_axis.truncate()) < 0.01,
            "{keeps:?}: {:?} then {:?}",
            before.w_axis,
            after.w_axis
        );
        assert!((yaw_of(after) - yaw_of(before)).abs() < 1e-3);
        assert!((scale_of(after) - 1.25).abs() < 1e-4);
        // Carried on, it settles onto that hand's ray, and let go, it floats at its new size, sent with its version.
        let ray = match keeps {
            Side::Right => wide.0,
            Side::Left => wide.1,
        };
        for _ in 0..72 {
            projector.update(both(
                &shown,
                (wide.0, grips.0),
                (wide.1, grips.1),
                none,
                now,
            ));
        }
        let output = projector.update(both(
            &shown,
            (wide.0, grips.0),
            (wide.1, grips.1),
            none,
            now,
        ));
        let middle_now = drawn(&output, "Panel p").w_axis.truncate();
        let ray = ray.unwrap();
        let off = (middle_now - ray.origin).cross(ray.direction).length();
        assert!(off < 1e-3, "{keeps:?}: {off}");
        let output = projector.update(both(&shown, (wide.0, 0.0), (wide.1, 0.0), none, now));
        let placed = match keeps {
            Side::Right => output.placed,
            Side::Left => output.left.placed,
        };
        assert!(
            matches!(placed, Some((_, Target::Sized(_, Resized { size, version: 41 }))) if (size - 1.25).abs() < 1e-4),
            "{keeps:?}: {placed:?}"
        );
    }
}

#[test]
fn b_throws_away_a_panel_both_hands_hold() {
    let shown = floating(2.0, None);
    let middle = Vec3::new(0.0, 1.5, -2.0);
    let now = Instant::now();
    let mut projector = Projector::default();
    let none = editing::Rights::default();
    let (right, left) = (from(RIGHT_HAND, middle), from(LEFT_HAND, middle));
    projector.update(both(&shown, (right, 1.0), (None, 0.0), none, now));
    projector.update(both(&shown, (right, 1.0), (left, 1.0), none, now));
    assert!(projector.two_handed());
    assert_eq!(projector.throw(now).as_deref(), Some("p"));
    assert!(!projector.two_handed() && !projector.carrying());
    let output = projector.update(both(&shown, (right, 0.0), (left, 0.0), none, now));
    assert!(output.placed.is_none() && output.left.placed.is_none());
}

#[test]
fn a_refused_size_is_drawn_as_the_server_has_it_at_once() {
    let shown = floating(2.0, None);
    let middle = Vec3::new(0.0, 1.5, -2.0);
    let now = Instant::now();
    let mut projector = Projector::default();
    let none = editing::Rights::default();
    let (right, left) = (from(RIGHT_HAND, middle), from(LEFT_HAND, middle));
    projector.update(both(&shown, (right, 1.0), (None, 0.0), none, now));
    projector.update(both(&shown, (right, 1.0), (left, 1.0), none, now));
    let wide = (
        from(RIGHT_HAND + Vec3::X * 0.2, middle),
        from(LEFT_HAND - Vec3::X * 0.2, middle),
    );
    projector.update(both(&shown, (wide.0, 1.0), (wide.1, 1.0), none, now));
    let output = projector.update(both(&shown, (wide.0, 0.0), (wide.1, 0.0), none, now));
    assert!(matches!(output.placed, Some((_, Target::Sized(..)))));
    let output = projector.update(both(&shown, (None, 0.0), (None, 0.0), none, now));
    assert!(scale_of(drawn(&output, "Panel p")) > 1.5);
    projector.refused_panel("p");
    let output = projector.update(both(&shown, (None, 0.0), (None, 0.0), none, now));
    assert!((scale_of(drawn(&output, "Panel p")) - 1.0).abs() < 1e-4);
}

/// An added wall, flat, three columns of two rows, 2.5 m ahead of home, facing it.
fn added_wall(locked: bool) -> Wall {
    Wall::new(
        "wall-1",
        WallKind::Added,
        Pose::upright_facing([0.0, 1.5, -2.5], HOME.to_array()),
    )
    .with_version(7)
    .with_locked(locked)
}

const ADJUST: editing::Rights = editing::Rights {
    adjust: true,
    remove: false,
    snap: true,
    locks: true,
    lock: false,
};

#[test]
fn both_hands_on_a_wall_carry_it_between_them_and_spreading_them_sizes_its_slots_sent_with_its_version()
 {
    let walled = Projection::new("s", "sky").with_walls(vec![added_wall(false)]);
    let wall = &walled.walls[0];
    let (_, height) = wall.board().unwrap();
    // Its board's top edge, above its slots, as a hand takes hold of a wall.
    let top = Vec3::new(0.0, 1.5 + height / 2.0 - 0.01, -2.5);
    let now = Instant::now();
    let mut projector = Projector::default();
    let right = from(RIGHT_HAND, top);
    let left = from(LEFT_HAND, top);
    projector.update(both(&walled, (right, 1.0), (None, 0.0), ADJUST, now));
    assert_eq!(projector.carried_wall(), Some("wall-1"));
    // The left ray on the wall the right carries takes hold of it too.
    projector.update(both(&walled, (right, 1.0), (left, 0.0), ADJUST, now));
    assert!(projector.joins(Side::Left));
    let output = projector.update(both(&walled, (right, 1.0), (left, 1.0), ADJUST, now));
    assert_eq!(output.joined, Some(Side::Left));
    assert!(projector.two_handed());
    assert_eq!(marks(&output), 4);
    // Its slots' size shows in place of its hint, as its corner's does.
    assert!(
        texts(&output).iter().any(|text| text.contains(" cm · ")),
        "{:?}",
        texts(&output)
    );
    // Both hands half as far apart again, and carried a little to the right: its slots half as large again, in a step,
    // and the wall comes with them.
    let wide = (
        from(RIGHT_HAND + Vec3::X * 0.2, top + Vec3::X * 0.1),
        from(LEFT_HAND, top + Vec3::X * 0.1),
    );
    let output = projector.update(both(&walled, (wide.0, 1.0), (wide.1, 1.0), ADJUST, now));
    assert!(output.wall.is_none(), "nothing is sent until it is let go");
    // Let go of the left, the right carries it on at its new size.
    let output = projector.update(both(&walled, (wide.0, 1.0), (wide.1, 0.0), ADJUST, now));
    assert!(output.wall.is_none());
    assert!(!projector.two_handed());
    assert_eq!(projector.carried_wall_in(Side::Right), Some("wall-1"));
    // Let go, its new size and where it stands are sent, with the version it had as both took hold of it.
    let output = projector.update(both(&walled, (wide.0, 0.0), (None, 0.0), ADJUST, now));
    let change = output.wall.expect("the change is sent");
    assert_eq!(change.wall, "wall-1");
    assert_eq!(change.scale, Some(1.5));
    assert!(change.pose.is_some());
    assert_eq!(change.version, Some(7));
    assert_eq!(change.grid, None);
}

#[test]
fn a_wall_held_with_both_hands_moved_but_not_sized_is_sent_as_carried() {
    let walled = Projection::new("s", "sky").with_walls(vec![added_wall(false)]);
    let (_, height) = walled.walls[0].board().unwrap();
    let top = Vec3::new(0.0, 1.5 + height / 2.0 - 0.01, -2.5);
    let now = Instant::now();
    let mut projector = Projector::default();
    let (right, left) = (from(RIGHT_HAND, top), from(LEFT_HAND, top));
    projector.update(both(&walled, (right, 1.0), (None, 0.0), ADJUST, now));
    projector.update(both(&walled, (right, 1.0), (left, 1.0), ADJUST, now));
    assert!(projector.two_handed());
    let shift = Vec3::X * 0.3;
    let moved = (
        from(RIGHT_HAND + shift, top + shift),
        from(LEFT_HAND + shift, top + shift),
    );
    projector.update(both(&walled, (moved.0, 1.0), (moved.1, 1.0), ADJUST, now));
    let output = projector.update(both(&walled, (moved.0, 0.0), (moved.1, 0.0), ADJUST, now));
    let change = output.wall.expect("the change is sent");
    assert_eq!(change.scale, None);
    assert_eq!(
        change.version, None,
        "a move alone is the participant's own"
    );
    let pose = change.pose.unwrap();
    assert!(Vec3::from_array(pose.position).distance(Vec3::new(0.3, 1.5, -2.5)) < 0.02);
}

#[test]
fn a_locked_wall_and_the_tomori_wall_are_not_stretched_with_both_hands() {
    let now = Instant::now();
    // A locked wall is held by nobody, so the other hand has nothing to take hold of with it.
    let locked = Projection::new("s", "sky").with_walls(vec![added_wall(true)]);
    let (_, height) = locked.walls[0].board().unwrap();
    let top = Vec3::new(0.0, 1.5 + height / 2.0 - 0.01, -2.5);
    let mut projector = Projector::default();
    let (right, left) = (from(RIGHT_HAND, top), from(LEFT_HAND, top));
    projector.update(both(&locked, (right, 1.0), (None, 0.0), ADJUST, now));
    assert_eq!(projector.editing_wall(), None);
    projector.update(both(&locked, (right, 1.0), (left, 0.0), ADJUST, now));
    assert!(!projector.joins(Side::Left));
    let output = projector.update(both(&locked, (right, 1.0), (left, 1.0), ADJUST, now));
    assert_eq!(output.joined, None);
    assert!(!projector.two_handed());
    // The Tomori wall, taken hold of by its slots and still squeezed, is neither carried nor sized by both hands: its
    // slots are its own size.
    let tomori = projection(Vec::new());
    let slot = slot_centre(&tomori, 0);
    let mut projector = Projector::default();
    let (right, left) = (from(RIGHT_HAND, slot), from(LEFT_HAND, slot));
    projector.update(both(&tomori, (right, 1.0), (None, 0.0), ADJUST, now));
    assert_eq!(projector.editing_wall(), Some(TOMORI_WALL));
    let output = projector.update(both(&tomori, (right, 1.0), (left, 1.0), ADJUST, now));
    assert_eq!(output.joined, None);
    assert!(!projector.two_handed());
}

#[test]
fn both_hands_size_the_slots_of_a_wall_held_but_not_carried_about_its_middle() {
    // Where the server does not lock walls, an environment's own is taken hold of, not carried, and still sized.
    let mut own = added_wall(false);
    own.kind = WallKind::Environment;
    let walled = Projection::new("s", "sky").with_walls(vec![own]);
    let (_, height) = walled.walls[0].board().unwrap();
    let top = Vec3::new(0.0, 1.5 + height / 2.0 - 0.01, -2.5);
    let rights = editing::Rights {
        locks: false,
        ..ADJUST
    };
    let now = Instant::now();
    let mut projector = Projector::default();
    let (right, left) = (from(RIGHT_HAND, top), from(LEFT_HAND, top));
    projector.update(both(&walled, (right, 1.0), (None, 0.0), rights, now));
    assert_eq!(projector.editing_wall(), Some("wall-1"));
    assert_eq!(projector.carried_wall(), None);
    let output = projector.update(both(&walled, (right, 1.0), (left, 1.0), rights, now));
    assert_eq!(output.joined, Some(Side::Left));
    // Closed to half as far apart, its slots are half as large, and it stays where it stands.
    let close = (
        from(Vec3::new(0.1, 1.3, -0.3), top),
        from(Vec3::new(-0.1, 1.3, -0.3), top),
    );
    projector.update(both(&walled, (close.0, 1.0), (close.1, 1.0), rights, now));
    let output = projector.update(both(&walled, (close.0, 0.0), (close.1, 0.0), rights, now));
    let change = output.wall.expect("the change is sent");
    assert_eq!(change.scale, Some(0.5));
    assert_eq!(change.pose, None);
    assert_eq!(change.version, Some(7));
}

#[test]
fn a_panel_both_hands_hold_holds_still_while_a_hand_is_not_tracked_and_is_let_go_as_it_is_dismissed()
 {
    let shown = floating(2.0, None);
    let middle = Vec3::new(0.0, 1.5, -2.0);
    let now = Instant::now();
    let mut projector = Projector::default();
    let none = editing::Rights::default();
    let (right, left) = (from(RIGHT_HAND, middle), from(LEFT_HAND, middle));
    projector.update(both(&shown, (right, 1.0), (None, 0.0), none, now));
    projector.update(both(&shown, (right, 1.0), (left, 1.0), none, now));
    assert!(projector.two_handed());
    // The left controller lost for a frame, its grip still held, it stays where it was, held with both.
    let shift = Vec3::X * 0.4;
    let output = projector.update(both(
        &shown,
        (from(RIGHT_HAND + shift, middle + shift), 1.0),
        (None, 1.0),
        none,
        now,
    ));
    assert!(projector.two_handed());
    assert!(drawn(&output, "Panel p").w_axis.truncate().distance(middle) < 1e-3);
    // Dismissed by someone else, it is held by neither.
    let gone = Projection::new("s", "sky").with_walls(shown.walls.clone());
    projector.update(both(&gone, (right, 1.0), (left, 1.0), none, now));
    assert!(!projector.two_handed() && !projector.carrying());
}

/// A projection of the added wall, holding a panel in its first slot, and a panel floating `before` the wall's middle,
/// between it and home, where given.
fn wall_with_panels(before: Option<f32>) -> Projection {
    let mut panels = vec![panel("on", on("wall-1", 0))];
    if let Some(before) = before {
        let at = Vec3::new(0.0, 1.5, -2.5 + before);
        panels.push(panel(
            "near",
            Place::Free {
                pose: Pose::upright_facing(at.to_array(), HOME.to_array()),
            },
        ));
    }
    Projection::new("s", "sky")
        .with_walls(vec![added_wall(false)])
        .with_panels(panels)
}

#[test]
fn the_other_ray_on_a_panel_on_the_wall_one_hand_carries_takes_hold_of_the_wall_but_one_nearer_is_pulled_off()
 {
    let now = Instant::now();
    let shown = wall_with_panels(None);
    let wall = &shown.walls[0];
    let (_, height) = wall.board().unwrap();
    let top = Vec3::new(0.0, 1.5 + height / 2.0 - 0.01, -2.5);
    let slot = model(&wall.frame())
        .unwrap()
        .transform_point3(slot_offset(wall, 0));
    let mut projector = Projector::default();
    let right = from(RIGHT_HAND, top);
    projector.update(both(&shown, (right, 1.0), (None, 0.0), ADJUST, now));
    assert_eq!(projector.carried_wall_in(Side::Right), Some("wall-1"));
    let left = from(LEFT_HAND, slot);
    projector.update(both(&shown, (right, 1.0), (left, 0.0), ADJUST, now));
    assert!(projector.joins(Side::Left));
    let output = projector.update(both(&shown, (right, 1.0), (left, 1.0), ADJUST, now));
    assert_eq!(output.joined, Some(Side::Left));
    assert!(projector.carrying_in(Side::Left));
    assert_eq!(projector.held_in(Side::Left), None);
    // A panel floating in front of the wall, which the left ray meets first, is the left grip's to pull off.
    let shown = wall_with_panels(Some(1.0));
    let mut projector = Projector::default();
    projector.update(both(&shown, (right, 1.0), (None, 0.0), ADJUST, now));
    let middle = Vec3::new(0.0, 1.5, -1.5);
    let left = from(LEFT_HAND, middle);
    projector.update(both(&shown, (right, 1.0), (left, 0.0), ADJUST, now));
    assert!(!projector.joins(Side::Left));
    let output = projector.update(both(&shown, (right, 1.0), (left, 1.0), ADJUST, now));
    assert_eq!(output.joined, None);
    assert_eq!(projector.held_in(Side::Left), Some("near"));
}

#[test]
fn the_first_hand_letting_go_of_a_wall_both_hold_leaves_the_other_carrying_it() {
    let walled = Projection::new("s", "sky").with_walls(vec![added_wall(false)]);
    let (_, height) = walled.walls[0].board().unwrap();
    let top = Vec3::new(0.0, 1.5 + height / 2.0 - 0.01, -2.5);
    let now = Instant::now();
    let mut projector = Projector::default();
    let (right, left) = (from(RIGHT_HAND, top), from(LEFT_HAND, top));
    projector.update(both(&walled, (right, 1.0), (None, 0.0), ADJUST, now));
    projector.update(both(&walled, (right, 1.0), (left, 1.0), ADJUST, now));
    assert!(projector.two_handed());
    let output = projector.update(both(&walled, (right, 0.0), (left, 1.0), ADJUST, now));
    assert!(output.wall.is_none());
    assert!(!projector.two_handed());
    assert_eq!(projector.carried_wall_in(Side::Left), Some("wall-1"));
    // Carried on by the left, it follows the left ray from where it stood.
    let shift = Vec3::X * 0.5;
    projector.update(both(
        &walled,
        (None, 0.0),
        (from(LEFT_HAND, top + shift), 1.0),
        ADJUST,
        now,
    ));
    let output = projector.update(both(
        &walled,
        (None, 0.0),
        (from(LEFT_HAND, top + shift), 0.0),
        ADJUST,
        now,
    ));
    let change = output.wall.expect("the change is sent");
    assert_eq!(output.wall_hand, Side::Left);
    let pose = change.pose.unwrap();
    assert!(Vec3::from_array(pose.position).x > 0.3, "{pose:?}");
}

#[test]
fn a_wall_is_sized_by_both_hands_no_larger_than_a_wall_may_be_and_one_of_a_group_not_at_all() {
    let now = Instant::now();
    // Eight columns of the largest slots would be wider than a wall may be: it stops at the largest it may be.
    let wide = added_wall(false).with_grid((8, 1).into());
    let walled = Projection::new("s", "sky").with_walls(vec![wide]);
    let (_, height) = walled.walls[0].board().unwrap();
    let top = Vec3::new(0.0, 1.5 + height / 2.0 - 0.01, -2.5);
    let mut projector = Projector::default();
    let (right, left) = (from(RIGHT_HAND, top), from(LEFT_HAND, top));
    projector.update(both(&walled, (right, 1.0), (None, 0.0), ADJUST, now));
    projector.update(both(&walled, (right, 1.0), (left, 1.0), ADJUST, now));
    let spread = |apart: f32| {
        (
            from(Vec3::new(apart / 2.0, 1.3, -0.3), top),
            from(Vec3::new(-apart / 2.0, 1.3, -0.3), top),
        )
    };
    let (right, left) = spread(0.8);
    let output = projector.update(both(&walled, (right, 1.0), (left, 1.0), ADJUST, now));
    assert!(!output.wall_ticked);
    let (right, left) = spread(2.0);
    let output = projector.update(both(&walled, (right, 1.0), (left, 1.0), ADJUST, now));
    assert!(output.wall_ticked, "reaching the most ticks");
    let output = projector.update(both(&walled, (right, 0.0), (left, 0.0), ADJUST, now));
    let scale = output.wall.and_then(|change| change.scale).unwrap();
    assert!((1.5..5.0).contains(&scale), "{scale}");
    let width = scale * tomori_protocol::wall::SLOT_SIZE.0 * 8.0;
    assert!(width <= tomori_protocol::wall::MOST_SIZE.0, "{width}");
    // A wall of a group is carried between both hands, its slots as they were, so that its group stays aligned.
    let grouped = added_wall(false).with_group(Some("group-1".into()), Vec::new());
    let walled = Projection::new("s", "sky").with_walls(vec![grouped]);
    let mut projector = Projector::default();
    let (right, left) = (from(RIGHT_HAND, top), from(LEFT_HAND, top));
    projector.update(both(&walled, (right, 1.0), (None, 0.0), ADJUST, now));
    let output = projector.update(both(&walled, (right, 1.0), (left, 1.0), ADJUST, now));
    assert_eq!(output.joined, Some(Side::Left));
    assert!(!texts(&output).iter().any(|text| text.contains(" cm · ")));
    let (right, left) = spread(1.2);
    projector.update(both(&walled, (right, 1.0), (left, 1.0), ADJUST, now));
    let output = projector.update(both(&walled, (right, 0.0), (left, 0.0), ADJUST, now));
    assert_eq!(output.wall.and_then(|change| change.scale), None);
}
