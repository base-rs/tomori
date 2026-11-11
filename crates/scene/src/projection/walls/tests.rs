// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::{
    arrival::HOME,
    pointing::Ray,
    projection::{Controls, Input, Projector, editing},
};
use glam::{Vec2, Vec3};
use std::time::Instant;
use tomori_protocol::{
    Edge, Grid, Join, Panel, Place, Projection, TOMORI_WALL, WallKind, WallShape,
};

fn flat(id: &str, x: f32) -> Wall {
    Wall::new(
        id,
        WallKind::Added,
        Pose::upright_facing([x, 1.5, -3.0], [x, 1.5, 0.0]),
    )
}

fn curved(id: &str) -> Wall {
    Wall::new(
        id,
        WallKind::Environment,
        Pose::upright_facing([0.0, 1.0, -2.4], [0.0, 1.0, 0.0]),
    )
    .with_shape(WallShape::Arc { radius: 2.4 })
    .with_name(Some("Details".into()))
}

/// Two flat walls grouped side by side, joined as the server sends them.
fn pair() -> (Wall, Wall) {
    let left = flat("wall-1", -1.0);
    let mut right = flat("wall-2", 0.0);
    right.pose = tomori_protocol::group::attached_pose(
        &left,
        &right,
        Edge::Right,
        tomori_protocol::Align::Start,
    )
    .unwrap();
    (
        left.with_group(
            Some("wall-1".into()),
            vec![Join::new(Edge::Right, "wall-2")],
        ),
        right.with_group(Some("wall-1".into()), vec![Join::new(Edge::Left, "wall-1")]),
    )
}

/// How many faces `draw` draws afresh.
fn drawn(draw: impl FnOnce()) -> usize {
    crate::frame_timing::take();
    draw();
    crate::frame_timing::take().faces
}

#[test]
fn a_walls_faces_are_kept_while_what_they_are_drawn_from_is_the_same() {
    let mut faces = WallFaces::default();
    let wall = flat("wall-1", 0.0);
    let board = faces.board(&wall);
    assert_eq!(*board, wall_face(&wall), "as drawn afresh");
    // Moved, or at another version, the same face.
    let mut moved = wall.clone();
    moved.pose.position[0] += 2.0;
    moved.version = 7;
    assert_eq!(
        drawn(|| assert!(Arc::ptr_eq(&faces.board(&moved), &board))),
        0
    );
    // Its grid changed, drawn again.
    let bigger = wall.clone().with_grid(Grid::new(4, 2).unwrap());
    let mut again = None;
    assert_eq!(drawn(|| again = Some(faces.board(&bigger))), 1);
    assert!(!Arc::ptr_eq(&again.unwrap(), &board));
    // A curved wall's slot board, the same for every slot, and its label, each drawn once.
    let arc = curved("wall-3");
    let slot = faces.slot(&arc);
    assert_eq!(*slot, slot_board(wall_border(&arc), Cells::of(&arc)));
    assert!(Arc::ptr_eq(&faces.slot(&arc), &slot));
    let label = faces.label(&arc);
    assert_eq!(*label, arc_label(&arc));
    assert!(Arc::ptr_eq(&faces.label(&arc), &label));
    // Renamed, its label is drawn again, and its slot board kept.
    let renamed = arc.clone().with_name(Some("Notes".into()));
    assert!(!Arc::ptr_eq(&faces.label(&renamed), &label));
    assert!(Arc::ptr_eq(&faces.slot(&renamed), &slot));
    // Its content smaller, each slot holds more cells: a new slot board.
    let smaller = renamed.clone().with_content_scale(Some(0.5));
    assert!(!Arc::ptr_eq(&faces.slot(&smaller), &slot));
    // The outline of a slot, by its cells.
    let ghost = faces.ghost(&arc);
    assert_eq!(*ghost, super::super::ghost(Cells::of(&arc)));
    assert!(Arc::ptr_eq(&faces.ghost(&arc), &ghost));
}

#[test]
fn a_groups_boards_are_kept_until_a_wall_of_the_group_changes() {
    let mut faces = WallFaces::default();
    let (left, right) = pair();
    let walls = [&left, &right];
    let sheets = faces.group(&left, &walls);
    let fresh = boards::sheets(&left, Mat4::IDENTITY, &walls);
    assert_eq!(sheets.len(), fresh.len());
    for ((face, model), sheet) in sheets.iter().zip(&fresh) {
        assert_eq!(Some(face), sheet.canvas.as_ref());
        assert_eq!(Some(*model), sheet.model);
    }
    let again = faces.group(&left, &walls);
    assert!(Arc::ptr_eq(&again[0].0, &sheets[0].0));
    // Carried together, turned and moved, the group keeps its faces, and draws none.
    let carry = |wall: &Wall| {
        let mut carried = wall.clone();
        let turn = glam::Quat::from_rotation_y(0.3);
        let at = turn * Vec3::from_array(wall.pose.position) + Vec3::new(0.7, 0.0, -0.4);
        carried.pose.position = at.to_array();
        carried.pose.orientation =
            (turn * glam::Quat::from_array(wall.pose.orientation)).to_array();
        carried.version += 1;
        carried
    };
    let (moved_left, moved_right) = (carry(&left), carry(&right));
    let mut carried = Vec::new();
    assert_eq!(
        drawn(|| carried = faces.group(&moved_left, &[&moved_left, &moved_right])),
        0
    );
    assert!(Arc::ptr_eq(&carried[0].0, &sheets[0].0));
    // Another wall joining its group draws it again.
    let mut third = flat("wall-5", 3.0);
    third.group = left.group.clone();
    let joined = faces.group(&left, &[&left, &right, &third]);
    assert!(!Arc::ptr_eq(&joined[0].0, &sheets[0].0));
    // And a member changed in its layout.
    let wider = right.clone().with_grid(Grid::new(4, 2).unwrap());
    let changed = faces.group(&left, &[&left, &wider]);
    assert!(!Arc::ptr_eq(&changed[0].0, &joined[0].0));
    // Its neighbour moved away, its edge is drawn: drawn again.
    let mut away = right.clone();
    away.pose.position[0] += 1.0;
    let moved = faces.group(&left, &[&left, &away]);
    assert!(!Arc::ptr_eq(&moved[0].0, &sheets[0].0));
}

#[test]
fn the_faces_of_walls_no_longer_shown_are_forgotten() {
    let mut faces = WallFaces::default();
    let (one, two) = (flat("wall-1", -1.0), flat("wall-2", 1.0));
    let first = faces.board(&one);
    faces.board(&two);
    faces.retain(&[&two]);
    assert_eq!(faces.walls.len(), 1);
    // Shown again, its face is drawn again.
    assert!(!Arc::ptr_eq(&faces.board(&one), &first));
}

fn input(projection: &Projection, viewer: Vec3, ray: Option<Ray>, grip: f32) -> Input<'_> {
    Input {
        projection: Some(projection),
        ray,
        grip: Some(grip),
        viewer: Some(viewer),
        active: true,
        pointing: true,
        nearer: f32::INFINITY,
        stick: Vec2::ZERO,
        straighten: false,
        free: false,
        controls: Controls::Controllers,
        scroll: None,
        rights: editing::Rights::default(),
        now: Instant::now(),
        dt: 1.0 / 72.0,
    }
}

#[test]
fn the_projector_hands_over_the_same_wall_faces_as_the_head_moves_and_forgets_those_removed() {
    let tomori = Wall::new(
        TOMORI_WALL,
        WallKind::Tomori,
        Pose::upright_facing([0.0, 0.52, -2.3], [0.0, 0.52, 0.0]),
    )
    .with_shape(WallShape::Arc { radius: 2.3 });
    let (left, right) = pair();
    let mut projection = Projection::new("s", "sky")
        .with_walls(vec![
            tomori,
            flat("wall-3", 3.0),
            curved("wall-4"),
            left,
            right,
        ])
        .with_panels(vec![Panel::new(
            "panel-1",
            "Notes",
            "A note.",
            Place::Wall {
                wall: TOMORI_WALL.into(),
                slot: 0,
            },
        )]);
    let mut projector = Projector::default();
    let canvases = |projector: &mut Projector, projection: &Projection, viewer: Vec3| {
        let mut canvases: Vec<usize> = projector
            .update(input(projection, viewer, None, 0.0))
            .sheets
            .iter()
            .filter_map(|sheet| sheet.canvas.as_ref())
            .map(|canvas| Arc::as_ptr(canvas) as usize)
            .collect();
        canvases.sort();
        canvases
    };
    let first = canvases(&mut projector, &projection, HOME);
    crate::frame_timing::take();
    for step in 0..10 {
        let viewer = HOME + Vec3::new(0.3 * (step as f32).sin(), 0.0, 0.2);
        assert_eq!(canvases(&mut projector, &projection, viewer), first);
    }
    assert_eq!(crate::frame_timing::take().faces, 0);
    // A wall removed has its faces forgotten.
    projection.walls.retain(|wall| wall.id != "wall-3");
    projector.update(input(&projection, HOME, None, 0.0));
    assert!(!projector.walls.walls.contains_key("wall-3"));
    assert!(projector.walls.walls.contains_key("wall-4"));
    // And every wall's, as the projection is no longer shown.
    projector.update(Input {
        projection: None,
        ..input(&projection, HOME, None, 0.0)
    });
    assert!(projector.walls.walls.is_empty());
}
