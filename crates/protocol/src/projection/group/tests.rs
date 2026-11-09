// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::{Grid, WallKind, WallShape, wall as layout};

/// A wall of `columns` by `rows` at `scale`, standing at `pose`.
fn wall(id: &str, pose: Pose, (columns, rows): (u32, u32), scale: f32) -> Wall {
    Wall::new(id, WallKind::Added, pose)
        .with_grid(Grid::new(columns, rows).unwrap())
        .with_scale(scale)
}

/// Facing home from 2.4 m ahead, at eye height.
fn ahead(x: f32) -> Pose {
    Pose::upright_facing([x, 1.4, -2.4], [x, 1.4, 0.0])
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn a_flat_wall_snaps_beside_another_its_slots_one_gutter_apart_at_the_larger_scale() {
    let two = wall("wall-1", ahead(0.0), (2, 2), 0.9);
    let big = wall("wall-2", ahead(0.0), (1, 1), 1.6);
    // Carried 6 cm short of where it would stand on the right, a little high, turned 2° and 3 cm proud of the plane.
    let (u, y) = attached_offset(&two, &big, Edge::Right, Align::Start).unwrap();
    let mut carried = big.clone();
    carried.pose = along(&two.pose, None, u - 0.06, y + 0.03);
    carried.pose.position[2] += 0.03;
    let turn = 2.0_f32.to_radians() / 2.0;
    carried.pose.orientation = [0.0, turn.sin(), 0.0, turn.cos()];
    let snap = snap(&carried, &two).expect("it snaps");
    assert_eq!((snap.edge, snap.align), (Edge::Right, Align::Start));
    assert!(snap.distance < 0.08, "{}", snap.distance);
    let mut snapped = big.clone();
    snapped.pose = snap.pose;
    // Square in the same plane, their tops aligned, the gap between their slots one gutter at the larger scale.
    assert_eq!(snapped.pose.orientation, two.pose.orientation);
    let (two_slots, big_slots) = (two.extent(), seen_from(&two, &snapped));
    assert!(near(big_slots.top, two_slots.top));
    assert!(near(big_slots.left - two_slots.right, layout::GUTTER * 1.6));
    assert!(near(gap_between(&two, &snapped), big.gap()));
    // Joined across the right edge, where they meet, from the bigger's bottom to their tops; and across its left, back.
    let (edge, from, to) = joined(&two, &snapped).unwrap();
    assert_eq!(edge, Edge::Right);
    assert!(near(to, two_slots.top) && near(from, two_slots.bottom.max(big_slots.bottom)));
    assert_eq!(joined(&snapped, &two).unwrap().0, Edge::Left);
    assert!(!overlaps(&two, &snapped) && !overlaps(&snapped, &two));
    // A smaller wall's middle or its bottom aligned when they are nearest.
    let small = wall("wall-3", ahead(0.0), (1, 1), 1.0);
    let (u, y) = attached_offset(&two, &small, Edge::Right, Align::Start).unwrap();
    for (align, lower) in [
        (Align::Start, 0.05),
        (Align::Middle, 0.12),
        (Align::End, 0.3),
    ] {
        let mut carried = small.clone();
        carried.pose = along(&two.pose, None, u + 0.05, y - lower);
        let snap = super::snap(&carried, &two).unwrap();
        assert_eq!((snap.edge, snap.align), (Edge::Right, align), "{lower}");
    }
}

#[test]
fn a_wall_snaps_above_below_and_to_the_left_aligned_along_the_edge() {
    let base = wall("wall-1", ahead(0.3), (3, 1), 1.0);
    let other = wall("wall-2", ahead(0.3), (1, 1), 1.0);
    for (edge, align) in [
        (Edge::Left, Align::Start),
        (Edge::Top, Align::Start),
        (Edge::Top, Align::End),
        (Edge::Bottom, Align::Middle),
    ] {
        let (u, y) = attached_offset(&base, &other, edge, align).unwrap();
        let mut carried = other.clone();
        carried.pose = along(&base.pose, None, u + 0.04, y + 0.03);
        let snap = snap(&carried, &base).unwrap();
        assert_eq!((snap.edge, snap.align), (edge, align));
        carried.pose = snap.pose;
        assert_eq!(joined(&base, &carried).map(|joined| joined.0), Some(edge));
        assert_eq!(
            joined(&carried, &base).map(|joined| joined.0),
            Some(edge.opposite())
        );
        assert!(!overlaps(&base, &carried));
    }
    // Carried over the top right corner, within reach of both edges and meeting both, it snaps across the nearer: the
    // right, where it is aligned with the top, rather than the top, where it is most of a slot from either end.
    let (u, _) = attached_offset(&base, &other, Edge::Right, Align::Start).unwrap();
    let (_, above) = attached_offset(&base, &other, Edge::Top, Align::End).unwrap();
    let mut corner = other.clone();
    corner.pose = along(&base.pose, None, u - 0.05, above - 0.08);
    assert_eq!(snap(&corner, &base).unwrap().edge, Edge::Right);
    // Above, its bottom slots a gutter above the other's top, where its label was.
    let (_, y) = attached_offset(&base, &other, Edge::Top, Align::Start).unwrap();
    assert!(near(
        y + other.extent().bottom - base.extent().top,
        layout::GUTTER
    ));
}

/// `pose` turned `degrees` about the vertical through it, clockwise as seen from above for a positive angle.
fn turned(pose: Pose, degrees: f32) -> Pose {
    let half = -degrees.to_radians() / 2.0;
    Pose {
        orientation: multiply([0.0, half.sin(), 0.0, half.cos()], pose.orientation),
        ..pose
    }
}

#[test]
fn a_wall_25_cm_off_and_turned_25_degrees_snaps_lined_up_and_one_beyond_the_leeway_does_not() {
    let base = wall("wall-1", ahead(0.0), (2, 1), 1.0);
    let other = wall("wall-2", ahead(0.0), (1, 1), 1.0);
    let (u, y) = attached_offset(&base, &other, Edge::Right, Align::Start).unwrap();
    let placed = |u: f32, y: f32, depth: f32, turn: f32| {
        let mut carried = other.clone();
        carried.pose = turned(along(&base.pose, None, u, y), turn);
        carried.pose.position[2] += depth;
        snap(&carried, &base)
    };
    // 25 cm beyond the edge, turned 25° either way, as two and a half of the stick's turns leave it, it snaps beside it,
    // square in its plane, its top aligned: lined up for the participant.
    for turn in [25.0, -25.0] {
        let snap = placed(u + 0.25, y, 0.0, turn).expect("it snaps");
        assert_eq!((snap.edge, snap.align), (Edge::Right, Align::Start));
        assert!(near(snap.turn, 25.0_f32.to_radians()), "{}", snap.turn);
        let mut snapped = other.clone();
        snapped.pose = snap.pose;
        assert_eq!(
            snapped.pose,
            attached_pose(&base, &other, Edge::Right, Align::Start).unwrap()
        );
        let (_, _, depth, turn) = on_surface(&base.pose, None, &snapped.pose);
        assert!(near(depth, 0.0) && near(turn, 0.0));
        assert_eq!(
            joined(&base, &snapped).map(|joined| joined.0),
            Some(Edge::Right)
        );
    }
    // 20 cm in front of its plane, or behind it, and 25 cm short of the edge, overlapping it, it snaps too.
    assert!(placed(u, y, 0.2, 0.0).is_some() && placed(u, y, -0.2, 0.0).is_some());
    assert!(placed(u - 0.25, y, 0.0, 0.0).is_some());
    // Beyond the leeway: 35 cm off across, 30 cm out of the plane, turned 35°, or tilted 20°, it does not.
    assert!(placed(u + 0.35, y, 0.0, 0.0).is_none());
    assert!(placed(u, y, 0.3, 0.0).is_none());
    assert!(placed(u, y, 0.0, 35.0).is_none());
    let mut tilted = other.clone();
    let half = 20.0_f32.to_radians() / 2.0;
    tilted.pose = along(&base.pose, None, u, y);
    tilted.pose.orientation = multiply([half.sin(), 0.0, 0.0, half.cos()], tilted.pose.orientation);
    assert!(snap(&tilted, &base).is_none());
    tilted.pose.orientation = turned(along(&base.pose, None, u, y), 0.0).orientation;
    let half = 10.0_f32.to_radians() / 2.0;
    tilted.pose.orientation = multiply([half.sin(), 0.0, 0.0, half.cos()], tilted.pose.orientation);
    assert!(snap(&tilted, &base).is_some());
    // Below the other's bottom, meeting it nowhere along the edge, it does not snap beside it.
    assert!(placed(u, y - 1.2, 0.0, 0.0).is_none());
    // Facing the other way, it shares no surface.
    let mut behind = other.clone();
    behind.pose = Pose::upright_facing(along(&base.pose, None, u, y).position, [0.0, 1.4, -5.0]);
    assert!(snap(&behind, &base).is_none());
    // A curved wall and a flat one never snap.
    let mut curved = other.clone().with_shape(WallShape::Arc { radius: 2.4 });
    curved.pose = along(&base.pose, None, u, y);
    assert!(snap(&curved, &base).is_none() && !same_surface(&curved, &base));
    assert!(snap(&other.clone(), &curved).is_none());
    assert!(attached_offset(&base, &other, Edge::Unknown, Align::Start).is_none());
    assert!(attached_pose(&base, &other, Edge::Right, Align::Unknown).is_none());
    // Nor are they joined, sharing no surface.
    assert!(joined(&curved, &base).is_none());
    assert!(attached_offset(&base, &other, Edge::Right, Align::Unknown).is_none());
    // Only walls someone added are snapped to.
    assert!(snaps_to(&base));
    assert!(!snaps_to(&Wall::new(
        "environment-0",
        WallKind::Environment,
        ahead(0.0)
    )));
    assert!(!snaps_to(&Wall::new(
        crate::TOMORI_WALL,
        WallKind::Tomori,
        ahead(0.0)
    )));
}

#[test]
fn the_leeway_widens_with_distance_and_a_snap_shown_is_held_a_little_wider() {
    assert_eq!(Leeway::at(1.0), Leeway::NEAR);
    let far = Leeway::at(5.0);
    assert!(near(far.reach, 5.0 * SNAP_ANGLE.tan()) && far.reach > 0.5);
    assert_eq!((far.depth, far.turn), (SNAP_DEPTH, SNAP_TURN));
    let held = Leeway::NEAR.held();
    assert!(near(held.reach, SNAP_REACH * SNAP_HOLD) && near(held.turn, SNAP_TURN * SNAP_HOLD));
    assert!(near(held.depth, SNAP_DEPTH * SNAP_HOLD) && near(held.tilt, SNAP_TILT * SNAP_HOLD));
    assert!(near(Leeway::NEAR.within(0.1).depth, 0.1));
    assert!(near(Leeway::NEAR.within(1.0).depth, SNAP_DEPTH));
    let base = wall("wall-1", ahead(0.0), (2, 1), 1.0);
    let other = wall("wall-2", ahead(0.0), (1, 1), 1.0);
    let (u, y) = attached_offset(&base, &other, Edge::Right, Align::Start).unwrap();
    let mut carried = other.clone();
    // 34 cm off and turned 33°: beyond the leeway, but within it held, and within it far away.
    carried.pose = turned(along(&base.pose, None, u + 0.34, y), 33.0);
    assert!(snap(&carried, &base).is_none());
    assert!(snap_within(&carried, &base, Leeway::NEAR.held()).is_some());
    carried.pose = along(&base.pose, None, u + 0.45, y);
    assert!(snap(&carried, &base).is_none());
    assert!(snap_within(&carried, &base, far).is_some());
    // Held, it still lets go well beyond.
    carried.pose = along(&base.pose, None, u + 0.4, y);
    assert!(snap_within(&carried, &base, Leeway::NEAR.held()).is_none());
}

#[test]
fn of_two_neighbours_the_nearer_ranks_first_and_a_side_before_the_top_unless_clearly_above() {
    let left = wall("wall-1", ahead(-0.8), (1, 1), 1.0);
    let other = wall("wall-2", ahead(0.0), (1, 1), 1.0);
    let mut right = wall("wall-3", ahead(0.0), (1, 1), 1.0);
    // The two a little more than one wall's width and two gutters apart, the one carried between them.
    let (u, y) = attached_offset(&left, &other, Edge::Right, Align::Start).unwrap();
    right.pose = along(&left.pose, None, 2.0 * u + 0.2, 0.0);
    let ranks = |shift: f32| {
        let mut carried = other.clone();
        carried.pose = along(&left.pose, None, u + shift, y);
        let a = snap(&carried, &left).map(|snap| (snap.edge, snap.rank));
        let b = snap(&carried, &right).map(|snap| (snap.edge, snap.rank));
        (a.unwrap(), b.unwrap())
    };
    // Nearer the left wall's right edge, its snap ranks first; nearer the right wall's left edge, that one's.
    let ((edge, a), (other_edge, b)) = ranks(0.05);
    assert_eq!((edge, other_edge), (Edge::Right, Edge::Left));
    assert!(a < b, "{a} {b}");
    let ((_, a), (_, b)) = ranks(0.15);
    assert!(b < a, "{a} {b}");
    // Off a wall's top right corner, as near its right edge as its top, square slots, it goes beside it.
    let square = |id: &str| wall(id, ahead(0.0), (1, 1), 1.0).with_aspect(Some(1.0));
    let (base, other) = (square("wall-4"), square("wall-5"));
    let (beside, _) = attached_offset(&base, &other, Edge::Right, Align::Start).unwrap();
    let (_, above) = attached_offset(&base, &other, Edge::Top, Align::End).unwrap();
    let mut corner = other.clone();
    corner.pose = along(&base.pose, None, beside + 0.05, above + 0.05);
    assert_eq!(snap(&corner, &base).unwrap().edge, Edge::Right);
    // Clearly above it, it goes on top.
    corner.pose = along(&base.pose, None, 0.1, above + 0.05);
    assert_eq!(snap(&corner, &base).unwrap().edge, Edge::Top);
}

#[test]
fn a_curved_wall_snaps_along_the_same_arc_continuing_it() {
    let curve = |id: &str, columns: u32, radius: f32| {
        wall(
            id,
            Pose::upright_facing([0.0, 1.3, -radius], [0.0, 1.3, 0.0]),
            (columns, 1),
            0.8,
        )
        .with_shape(WallShape::Arc { radius })
    };
    let base = curve("wall-1", 3, 2.4);
    let other = curve("wall-2", 2, 2.4);
    let (u, y) = attached_offset(&base, &other, Edge::Right, Align::Start).unwrap();
    let mut carried = other.clone();
    carried.pose = along(&base.pose, Some(2.4), u - 0.05, y + 0.02);
    // As tall as each other, their tops are aligned.
    let snap = snap(&carried, &base).expect("it snaps along the arc");
    assert_eq!((snap.edge, snap.align), (Edge::Right, Align::Start));
    carried.pose = snap.pose;
    // About the same line, home, at the same radius, facing it: the arc continued.
    let (_, _, depth, turn) = on_surface(&base.pose, Some(2.4), &carried.pose);
    assert!(depth.abs() < 1e-4 && turn.abs() < 1e-4);
    let middle = centre(&carried.pose, 2.4);
    assert!(middle[0].hypot(middle[2]) < 1e-4, "{middle:?}");
    // Its slots one curved wall's gap from the other's along the arc, joined, and apart.
    let seen = seen_from(&base, &carried);
    assert!(near(seen.left - base.extent().right, base.gap()));
    assert_eq!(joined(&base, &carried).unwrap().0, Edge::Right);
    assert!(!overlaps(&base, &carried));
    assert_eq!(snap.radius, Some(2.4));
    // Curved about a line 40 cm away, it does not.
    let mut off = other.clone();
    off.pose = along(&base.pose, Some(2.4), u, y);
    off.pose.position[2] -= 0.4;
    assert!(super::snap(&off, &base).is_none());
    // Curved at another radius, turned 20° and 15 cm out, it takes the other's radius, continuing its arc about the
    // same line, so that the two read as one curve.
    let mut wider = curve("wall-3", 2, 3.2);
    wider.pose = turned(along(&base.pose, Some(2.4), u + 0.1, y - 0.05), 20.0);
    wider.pose.position[2] += 0.15;
    let snap = super::snap(&wider, &base).expect("it snaps onto the arc");
    assert_eq!(
        (snap.edge, snap.align, snap.radius),
        (Edge::Right, Align::Start, Some(2.4))
    );
    let mut snapped = wider.clone().with_shape(WallShape::Arc { radius: 2.4 });
    snapped.pose = snap.pose;
    let middle = centre(&snapped.pose, 2.4);
    assert!(middle[0].hypot(middle[2]) < 1e-4, "{middle:?}");
    let (_, _, depth, turn) = on_surface(&base.pose, Some(2.4), &snapped.pose);
    assert!(depth.abs() < 1e-4 && turn.abs() < 1e-4);
    assert_eq!(joined(&base, &snapped).unwrap().0, Edge::Right);
    // A curve too tight for it, which would take it round more than half a circle, it does not snap to.
    let tight = curve("wall-4", 1, 1.0);
    let mut long = curve("wall-5", 8, 3.0);
    long.pose = along(&tight.pose, Some(1.0), 1.0, 0.0);
    assert!(super::snap(&long, &tight).is_none());
}

#[test]
fn walls_overlap_only_when_their_slots_reach_into_each_other_on_one_surface() {
    let base = wall("wall-1", ahead(0.0), (2, 2), 1.0);
    let mut other = wall("wall-2", ahead(0.0), (1, 1), 1.0);
    assert!(overlaps(&base, &other));
    other.pose = along(&base.pose, None, 0.5, 0.0);
    assert!(overlaps(&base, &other));
    // Clear of it, beside it or in another plane.
    other.pose = along(&base.pose, None, 2.0, 0.0);
    assert!(!overlaps(&base, &other) && joined(&base, &other).is_none());
    other.pose = ahead(0.0);
    other.pose.position[2] += 0.5;
    assert!(!overlaps(&base, &other));
    // Around a curve, one a whole turn on is found.
    let arc = |id: &str, angle: f32| {
        let mut wall = wall(
            id,
            Pose::upright_facing([0.0, 1.3, -1.5], [0.0, 1.3, 0.0]),
            (1, 1),
            1.0,
        )
        .with_shape(WallShape::Arc { radius: 1.5 });
        wall.pose = along(&wall.pose.clone(), Some(1.5), angle * 1.5, 0.0);
        wall
    };
    let (first, last) = (arc("a", 3.0), arc("b", -3.05));
    assert!(overlaps(&first, &last));
    assert!(!overlaps(&arc("a", 0.0), &arc("b", 1.0)));
}

#[test]
fn edges_and_alignments_have_a_stable_wire_format_and_names() {
    let attach = Attach::new("wall-1", Edge::Right, Align::Start);
    let json = serde_json::json!({"to": "wall-1", "edge": "right", "align": "start"});
    assert_eq!(serde_json::to_value(&attach).unwrap(), json);
    assert_eq!(serde_json::from_value::<Attach>(json).unwrap(), attach);
    assert!(attach.known());
    let newer: Attach =
        serde_json::from_value(serde_json::json!({"to": "w", "edge": "front", "align": "spread"}))
            .unwrap();
    assert_eq!((newer.edge, newer.align), (Edge::Unknown, Align::Unknown));
    assert!(!newer.known());
    assert_eq!(Edge::parse(" Above "), Some(Edge::Top));
    assert_eq!(Edge::parse("below"), Some(Edge::Bottom));
    assert_eq!(Edge::parse("LEFT"), Some(Edge::Left));
    assert_eq!(Edge::parse("right"), Some(Edge::Right));
    assert_eq!(Edge::parse("behind"), None);
    assert_eq!(Align::parse("centre"), Some(Align::Middle));
    assert_eq!(Align::parse("Top"), Some(Align::Start));
    assert_eq!(Align::parse("bottom"), Some(Align::End));
    assert_eq!(Align::parse("spread"), None);
    assert_eq!(Edge::Unknown.opposite(), Edge::Unknown);
    for edge in Edge::ALL {
        assert_eq!(edge.opposite().opposite(), edge);
    }
    // A wall in a group says so, and which edges are joined; one alone says neither, as an older server's.
    let pose = ahead(0.0);
    let grouped = Wall::new("wall-2", WallKind::Added, pose)
        .with_group(Some("wall-1".into()), vec![Join::new(Edge::Left, "wall-1")]);
    let json = serde_json::to_value(&grouped).unwrap();
    assert_eq!(json["group"], "wall-1");
    assert_eq!(
        json["joined"],
        serde_json::json!([{"edge": "left", "wall": "wall-1"}])
    );
    assert_eq!(serde_json::from_value::<Wall>(json).unwrap(), grouped);
    let alone = serde_json::to_value(Wall::new("wall-3", WallKind::Added, pose)).unwrap();
    assert!(alone.get("group").is_none() && alone.get("joined").is_none());
    // Attaching a wall, and taking it out of its group.
    let cases = [
        (
            crate::ClientMessage::AttachWall {
                wall: "wall-2".into(),
                attach: Some(attach),
                alone: false,
                pose: None,
            },
            serde_json::json!({"type": "attach_wall", "wall": "wall-2",
                "attach": {"to": "wall-1", "edge": "right", "align": "start"}}),
        ),
        (
            crate::ClientMessage::AttachWall {
                wall: "wall-2".into(),
                attach: None,
                alone: true,
                pose: Some(pose),
            },
            serde_json::json!({"type": "attach_wall", "wall": "wall-2", "alone": true,
                "pose": serde_json::to_value(pose).unwrap()}),
        ),
    ];
    for (message, json) in cases {
        assert_eq!(serde_json::to_value(&message).unwrap(), json);
        assert_eq!(
            serde_json::from_value::<crate::ClientMessage>(json).unwrap(),
            message
        );
    }
}

#[test]
fn a_flat_wall_s_margins_are_its_gutter_its_label_and_what_rounds_its_board() {
    let flat = wall("w", ahead(0.0), (2, 2), 1.5);
    assert!(near(flat.margin(Edge::Left), layout::GUTTER * 1.5));
    assert!(near(flat.margin(Edge::Right), layout::GUTTER * 1.5));
    let (_, height) = flat.board().unwrap();
    let extent = flat.extent();
    assert!(near(
        flat.margin(Edge::Top) + flat.margin(Edge::Bottom) + extent.top - extent.bottom,
        height
    ));
    assert!(flat.margin(Edge::Bottom) >= layout::GUTTER * 1.5 - 1e-4);
    // A curved wall's slots are about its middle.
    let curved = flat.clone().with_shape(WallShape::Arc { radius: 3.0 });
    let extent = curved.extent();
    assert!(near(extent.top, -extent.bottom) && near(extent.right - extent.left, curved.size().0));
    assert!(near(
        Extent::shifted(&extent, 1.0, 0.0).left,
        extent.left + 1.0
    ));
    assert!(near(reach(&curved, Edge::Right), extent.right));
    assert!(near(reach(&curved, Edge::Left), extent.left));
    assert!(near(reach(&curved, Edge::Top), extent.top));
    assert!(near(reach(&curved, Edge::Bottom), extent.bottom));
    assert!(near(
        seam(&flat, &curved),
        flat.gap().max(curved.gap()) / 2.0
    ));
}

#[test]
fn a_wall_carried_with_another_keeps_where_it_stands_from_it() {
    let (from, other) = (ahead(0.0), ahead(1.0));
    // Turned a quarter, to face +X, and moved a metre to the left.
    let to = Pose::upright_facing([-1.0, 1.4, -2.4], [5.0, 1.4, -2.4]);
    let moved = carried_with(&from, &to, &other);
    // A metre to its right, as seen from in front, which is now towards -Z, facing as it faces.
    assert!(near(moved.position[0], -1.0) && near(moved.position[1], 1.4));
    assert!(near(moved.position[2], -3.4), "{moved:?}");
    let (u, y, depth, turn) = on_surface(&to, None, &moved);
    assert!(near(u, 1.0) && near(y, 0.0) && near(depth, 0.0) && near(turn, 0.0));
}

#[test]
fn a_locked_wall_is_no_snap_target_and_snaps_to_nothing() {
    let two = wall("wall-1", ahead(0.0), (2, 2), 0.9);
    let small = wall("wall-2", ahead(0.0), (1, 1), 1.0);
    let (u, y) = attached_offset(&two, &small, Edge::Right, Align::Start).unwrap();
    let mut carried = small.clone();
    carried.pose = along(&two.pose, None, u + 0.02, y);
    assert!(snaps_to(&two) && snap(&carried, &two).is_some());
    // Locked, the wall stood beside is no target, and a wall locked snaps to nothing.
    let locked = two.clone().with_locked(true);
    assert!(!snaps_to(&locked));
    assert!(snap(&carried, &locked).is_none());
    assert!(snap(&carried.clone().with_locked(true), &two).is_none());
    // The Tomori wall and an environment's walls are no targets either.
    let environment = Wall::new("environment-0", WallKind::Environment, ahead(0.0));
    assert!(!snaps_to(&environment));
}

#[test]
fn a_tilted_wall_is_no_snap_target() {
    let two = wall("wall-1", ahead(0.0), (2, 2), 0.9);
    assert!(snaps_to(&two));
    assert!(!snaps_to(&two.clone().with_tilt(Some(10.0))));
    assert!(snaps_to(&two.with_tilt(Some(0.0))));
}
