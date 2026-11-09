// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::arrival::HOME;
use crate::projection::{Input as Frame, Projector};
use tomori_protocol::{Panel, Place, TOMORI_WALL};

const ADJUST: Rights = Rights {
    adjust: true,
    remove: false,
};

const OWNER: Rights = Rights {
    adjust: true,
    remove: true,
};

/// An added wall 2.5 m ahead of home, facing it, flat, three columns of two rows.
fn added() -> Wall {
    Wall::new(
        "wall-1",
        WallKind::Added,
        Pose::upright_facing([0.0, 1.5, -2.5], HOME.to_array()),
    )
}

/// The Tomori wall where it stands by default.
fn tomori() -> Wall {
    Wall::new(
        TOMORI_WALL,
        WallKind::Tomori,
        Pose::upright_facing([0.0, 0.52, -2.3], [0.0, 0.52, 0.0]),
    )
    .with_shape(WallShape::Arc { radius: 2.3 })
}

fn shown(walls: Vec<Wall>, panels: Vec<Panel>) -> Projection {
    Projection::new("s", "sky")
        .with_walls(walls)
        .with_panels(panels)
}

fn frame<'a>(
    projection: &'a Projection,
    ray: Option<Ray>,
    grip: f32,
    rights: Rights,
    now: Instant,
) -> Frame<'a> {
    Frame {
        projection: Some(projection),
        ray,
        grip: Some(grip),
        viewer: Some(HOME),
        active: true,
        pointing: true,
        nearer: f32::INFINITY,
        stick: Vec2::ZERO,
        straighten: false,
        controls: Controls::Controllers,
        scroll: None,
        rights,
        now,
        dt: 1.0 / 72.0,
    }
}

/// A ray from home at `point`.
fn at(point: Vec3) -> Option<Ray> {
    Ray::new(HOME, point - HOME)
}

/// Where `handle` of `wall`, standing where it says, is drawn, seen from home.
fn handle(wall: &Wall, handle: Handle) -> Vec3 {
    handle_point(wall, model(&wall.pose).unwrap(), handle, HOME).0
}

/// A point of `wall`'s board, `x` and `y` from its middle in its own frame, in the scene.
fn on(wall: &Wall, x: f32, y: f32) -> Vec3 {
    model(&wall.pose)
        .unwrap()
        .transform_point3(Vec3::new(x, y, 0.0))
}

/// The projector, holding `wall` taken hold of by a squeeze on its board's top edge, above its slots, or, curved, on its
/// first slot, empty, and let go.
fn holding(projection: &Projection, wall: &Wall, rights: Rights, now: Instant) -> Projector {
    let mut projector = Projector::default();
    let target = match wall.board() {
        Some((_, height)) => on(wall, 0.0, height / 2.0 - 0.005),
        None => model(&wall.pose)
            .unwrap()
            .transform_point3(Vec3::from_array(wall.slot_frames()[0].position)),
    };
    let ray = at(target);
    projector.update(frame(projection, ray, 1.0, rights, now));
    projector.update(frame(projection, ray, 0.0, rights, now));
    projector
}

#[test]
fn a_squeeze_on_a_wall_s_board_takes_hold_of_it_and_an_added_wall_is_carried_and_put_down_where_let_go()
 {
    let wall = added();
    let projection = shown(vec![wall.clone()], Vec::new());
    let now = Instant::now();
    let mut projector = Projector::default();
    // Between the top row and the label, on the board.
    let ray = at(on(&wall, 0.0, 0.42));
    let output = projector.update(frame(&projection, ray, 1.0, ADJUST, now));
    assert!(output.wall_ticked || output.wall.is_none());
    assert_eq!(projector.editing_wall(), Some("wall-1"));
    assert_eq!(projector.carried_wall(), Some("wall-1"));
    assert!(projector.carrying());
    // Carried a metre to the left, it follows the ray, drawn where it is carried, with the carrying hint.
    let left = at(on(&wall, -1.0, 0.42));
    let mut stick = frame(&projection, left, 1.0, ADJUST, now);
    stick.stick = Vec2::new(0.0, 1.0);
    stick.dt = 0.2;
    let output = projector.update(stick);
    let hint = output
        .sheets
        .last()
        .and_then(|sheet| sheet.canvas.as_ref())
        .unwrap()
        .plain();
    assert!(
        hint.contains("further / closer") && hint.contains("leave it here"),
        "{hint}"
    );
    assert!(!hint.contains("remove"), "only owners remove walls: {hint}");
    // Let go, the new pose is sent, upright, further away than it was, and drawn there until the server agrees.
    let output = projector.update(frame(&projection, left, 0.0, ADJUST, now));
    let change = output.wall.expect("a pose to send");
    assert_eq!(change.wall, "wall-1");
    let pose = change.pose.expect("moved");
    assert!(pose.position[0] < -0.5, "{pose:?}");
    assert!(
        Vec3::from_array(pose.position).distance(HOME)
            > Vec3::from_array(wall.pose.position).distance(HOME)
    );
    assert_eq!(
        (change.grid, change.scale, change.shape),
        (None, None, None)
    );
    assert!(!projector.carrying());
    assert_eq!(projector.editing_wall(), Some("wall-1"));
    let drawn = projector.editor.walls(&projection);
    assert_eq!(drawn[0].pose, pose);
    // Once the server agrees, it is drawn as the server says.
    let mut moved = wall.clone();
    moved.pose = pose;
    let agreed = shown(vec![moved.clone()], Vec::new());
    projector.update(frame(&agreed, None, 0.0, ADJUST, now));
    assert!(projector.editor.pending.is_none());
    // Without agreeing, after a while, the server's is drawn again.
    let mut projector = Projector::default();
    projector.update(frame(&projection, ray, 1.0, ADJUST, now));
    projector.update(frame(&projection, left, 0.0, ADJUST, now));
    projector.update(frame(&projection, None, 0.0, ADJUST, now + PENDING));
    assert_eq!(projector.editor.walls(&projection)[0], wall);
}

#[test]
fn a_panel_on_a_wall_is_taken_before_the_wall_and_a_participant_who_may_not_adjust_walls_takes_none()
 {
    let wall = added();
    let panel = Panel::new(
        "panel-1",
        "Build",
        "Green.",
        Place::Wall {
            wall: "wall-1".into(),
            slot: 0,
        },
    );
    let projection = shown(vec![wall.clone()], vec![panel]);
    let now = Instant::now();
    let slot = model(&wall.pose)
        .unwrap()
        .transform_point3(Vec3::from_array(wall.slot_frames()[0].position));
    let mut projector = Projector::default();
    projector.update(frame(&projection, at(slot), 1.0, OWNER, now));
    assert_eq!(projector.held(), Some("panel-1"));
    assert_eq!(projector.editing_wall(), None);
    // A visitor, or anyone where the server takes no wall changes, takes no wall.
    let mut projector = Projector::default();
    projector.update(frame(
        &projection,
        at(on(&wall, 0.0, 0.42)),
        1.0,
        Rights::default(),
        now,
    ));
    assert_eq!(projector.editing_wall(), None);
    // Nor does a squeeze while something nearer takes it, as a card does.
    let mut projector = Projector::default();
    let mut nearer = frame(&projection, at(on(&wall, 0.0, 0.42)), 1.0, ADJUST, now);
    nearer.nearer = 0.5;
    projector.update(nearer);
    assert_eq!(projector.editing_wall(), None);
}

#[test]
fn handles_show_on_a_wall_taken_hold_of_with_a_hint_and_a_squeeze_at_nothing_or_b_lets_go_of_it() {
    let wall = added();
    let projection = shown(vec![wall.clone(), tomori()], Vec::new());
    let now = Instant::now();
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let output = projector.update(frame(&projection, None, 0.0, ADJUST, now));
    // Eleven handles and the hint, after the wall's own sheets.
    let faces: Vec<String> = output
        .sheets
        .iter()
        .rev()
        .take(12)
        .filter_map(|sheet| sheet.canvas.as_ref().map(Canvas::plain))
        .collect();
    let hint = &faces[0];
    for row in [
        "more / fewer slots",
        "size and shape",
        "one fewer / more",
        "pull to bend",
        "carry it",
        "done",
    ] {
        assert!(hint.contains(row), "{row}: {hint}");
    }
    assert_eq!(handles(&wall).len(), 11);
    assert!(faces[1..].iter().any(|face| face.contains('●')));
    // A handle the ray meets is highlighted, ticks, and ends the ray.
    let bend = handle(&wall, Handle::Bend);
    let output = projector.update(frame(&projection, at(bend), 0.0, ADJUST, now));
    assert!(output.wall_ticked);
    assert!(output.end.unwrap().distance(bend) < 0.02);
    assert_eq!(projector.editor.hovered, Some(Handle::Bend));
    // A squeeze at nothing lets go of the wall, and so does stopping, as B does.
    projector.update(frame(&projection, at(HOME + Vec3::Y), 1.0, ADJUST, now));
    projector.update(frame(&projection, None, 0.0, ADJUST, now));
    assert_eq!(projector.editing_wall(), None);
    let mut projector = holding(&projection, &wall, ADJUST, now);
    assert!(projector.stop_editing());
    assert!(!projector.stop_editing());
    // The Tomori wall has no corners or rows to change, its slots its own size in its one row.
    let tomori = tomori();
    assert_eq!(handles(&tomori).len(), 4);
    let hint = super::hint(Controls::Controllers, &tomori, false, false, None).plain();
    assert!(
        !hint.contains("size and shape") && !hint.contains("carry it"),
        "{hint}"
    );
    // The desktop window names its keys.
    let keys = super::hint(Controls::Keyboard, &wall, true, true, None).plain();
    assert!(
        keys.contains("PgUp PgDn") && keys.contains("Backspace") && keys.contains("Let go G"),
        "{keys}"
    );
    let keys = super::hint(Controls::Keyboard, &wall, false, false, None).plain();
    assert!(keys.contains("drag up to bend"), "{keys}");
}

/// Drag `handle` of `wall`, which `projector` holds, to `to`, and let go: what is sent.
fn drag(
    projector: &mut Projector,
    projection: &Projection,
    wall: &Wall,
    handle: Handle,
    to: Vec3,
    now: Instant,
) -> Option<Change> {
    let from = at(self::handle(wall, handle));
    projector.update(frame(projection, from, 0.0, ADJUST, now));
    let output = projector.update(frame(projection, from, 1.0, ADJUST, now));
    assert!(output.wall.is_none());
    assert_eq!(
        projector.editor.drag.as_ref().map(|drag| drag.handle),
        Some(handle)
    );
    projector.update(frame(projection, at(to), 1.0, ADJUST, now));
    projector
        .update(frame(projection, at(to), 0.0, ADJUST, now))
        .wall
}

#[test]
fn edges_give_a_wall_whole_columns_and_rows_and_corners_size_its_slots_in_steps() {
    let wall = added();
    let projection = shown(vec![wall.clone()], Vec::new());
    let now = Instant::now();
    // The right edge dragged out to where a fourth column's edge would stand, and the bottom edge up to one row.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let four = Wall::new("w", WallKind::Added, wall.pose).with_grid(Grid::new(4, 2).unwrap());
    let (width, _) = four.board().unwrap();
    let change = drag(
        &mut projector,
        &projection,
        &wall,
        Handle::Columns,
        on(&wall, width / 2.0, 0.0),
        now,
    )
    .expect("more columns");
    assert_eq!(change.grid, Grid::new(4, 2));
    assert_eq!(
        (change.scale, change.shape, change.pose),
        (None, None, None)
    );
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let change = drag(
        &mut projector,
        &projection,
        &wall,
        Handle::Rows,
        on(&wall, 0.0, -0.1),
        now,
    )
    .expect("fewer rows");
    assert_eq!(change.grid, Grid::new(3, 1));
    // Dragged back where it was, nothing is sent.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let still = handle(&wall, Handle::Rows);
    assert_eq!(
        drag(&mut projector, &projection, &wall, Handle::Rows, still, now),
        None
    );
    // A corner dragged half as far again from the middle makes the slots half as large again, in steps of 5%.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let corner = handle(&wall, Handle::Corner(1, 1));
    let middle = Vec3::from_array(wall.pose.position);
    let change = drag(
        &mut projector,
        &projection,
        &wall,
        Handle::Corner(1, 1),
        middle + (corner - middle) * 1.5,
        now,
    )
    .expect("larger slots");
    let scale = change.scale.unwrap();
    assert!((scale - 1.5).abs() < 0.051, "{scale}");
    assert!(((scale / SCALE_STEP).round() * SCALE_STEP - scale).abs() < 1e-5);
    // Never beyond the largest.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let change = drag(
        &mut projector,
        &projection,
        &wall,
        Handle::Corner(1, 1),
        middle + (corner - middle) * 5.0,
        now,
    );
    assert!(change.is_none_or(|change| change.scale.is_none_or(|scale| scale <= layout::SCALES.1)));
    // Dragged down alone, its slots grow taller, not wider, their text its size: nearly three times as tall snaps to
    // 9:16, twice as tall is its own shape, and wider alone makes them wide.
    let stretched = |across: f32, down: f32| {
        let mut projector = holding(&projection, &wall, ADJUST, now);
        let corner = handle(&wall, Handle::Corner(1, -1));
        let local = model(&wall.pose)
            .unwrap()
            .inverse()
            .transform_point3(corner);
        let to = on(&wall, local.x * across, local.y * down);
        drag(
            &mut projector,
            &projection,
            &wall,
            Handle::Corner(1, -1),
            to,
            now,
        )
        .expect("a new shape")
    };
    let tall = stretched(1.0, 2.95);
    assert_eq!(
        (tall.aspect, tall.scale),
        (Some(9.0 / 16.0), None),
        "{tall:?}"
    );
    let taller = stretched(1.0, 2.0);
    assert_eq!(taller.aspect, Some(0.84));
    let wide = stretched(1.9, 1.0);
    assert!(
        wide.aspect
            .is_some_and(|aspect| (aspect - 3.18).abs() < 0.02),
        "{wide:?}"
    );
    assert_eq!(wide.scale, None);
}

#[test]
fn a_slot_s_shape_snaps_to_common_shapes_near_them_its_text_as_large_as_its_narrower_side() {
    let (width, height) = layout::SLOT_SIZE;
    // As it was, a standard slot.
    assert_eq!(slot_shape(width, height), (1.0, None, Some("standard")));
    // Near 9:16, 16:9, 1:1, 4:3 and 3:4, it snaps to them.
    for (shape, name) in ASPECT_SNAPS
        .iter()
        .filter(|(shape, _)| *shape != layout::STANDARD_ASPECT)
    {
        for near in [0.975, 1.0, 1.025] {
            let (_, aspect, snapped) = slot_shape(height * shape * near, height);
            assert_eq!(
                (aspect, snapped),
                (Some(*shape), Some(*name)),
                "{name} at {near}"
            );
        }
        // Further off, it is its own shape, to a hundredth.
        let (_, aspect, snapped) = slot_shape(height * shape * 1.06, height);
        assert_eq!(snapped, None, "{name}");
        assert!(
            aspect.is_some_and(|aspect| ((aspect * 100.0).round() - aspect * 100.0).abs() < 1e-3)
        );
    }
    // Its text is as large as its narrower side allows against a standard slot's, in steps, within bounds.
    assert_eq!(slot_shape(width * 1.5, height * 4.0).0, 1.5);
    assert_eq!(slot_shape(width * 4.0, height * 1.5).0, 1.5);
    assert_eq!(slot_shape(width * 1.23, height * 3.0).0, 1.25);
    assert_eq!(slot_shape(width * 9.0, height * 9.0).0, layout::SCALES.1);
    assert_eq!(slot_shape(0.0, 0.0).0, layout::SCALES.0);
    // Never narrower or wider than a slot may be.
    assert_eq!(slot_shape(width, height * 40.0).1, Some(layout::ASPECTS.0));
    assert_eq!(slot_shape(width * 40.0, height).1, Some(layout::ASPECTS.1));
    // Dragged, the corner says what it makes.
    let tall = added().with_aspect(Some(9.0 / 16.0));
    assert_eq!(
        readout(Handle::Corner(1, 1), &tall).plain().trim(),
        "60 × 107 cm · 9:16"
    );
    let own = added().with_aspect(Some(0.84));
    assert!(
        readout(Handle::Corner(1, 1), &own)
            .plain()
            .contains("1:1.19")
    );
    assert!(
        readout(Handle::Corner(1, 1), &added().with_aspect(Some(2.5)))
            .plain()
            .contains("2.50:1")
    );
    assert!(
        readout(Handle::Columns, &added())
            .plain()
            .contains("3 × 2 slots")
    );
}

#[test]
fn steppers_give_or_take_one_column_or_row_at_once_within_what_a_wall_may_have() {
    let wall = added();
    let projection = shown(vec![wall.clone(), tomori()], Vec::new());
    let now = Instant::now();
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let more = at(handle(&wall, Handle::Step(Axis::Columns, 1)));
    projector.update(frame(&projection, more, 0.0, ADJUST, now));
    let output = projector.update(frame(&projection, more, 1.0, ADJUST, now));
    assert_eq!(output.wall.unwrap().grid, Grid::new(4, 2));
    // Shown at once, as sent.
    assert_eq!(
        projector.editor.walls(&projection)[0].grid(),
        Grid::new(4, 2).unwrap()
    );
    let fewer = at(handle(&wall, Handle::Step(Axis::Rows, -1)));
    let mut projector = holding(&projection, &wall, ADJUST, now);
    projector.update(frame(&projection, fewer, 0.0, ADJUST, now));
    let output = projector.update(frame(&projection, fewer, 1.0, ADJUST, now));
    assert_eq!(output.wall.unwrap().grid, Grid::new(3, 1));
    // The Tomori wall stops at six in its row.
    let tomori = tomori();
    let mut projector = holding(&projection, &tomori, ADJUST, now);
    assert_eq!(projector.editing_wall(), Some(TOMORI_WALL));
    let more = at(handle(&tomori, Handle::Step(Axis::Columns, 1)));
    projector.update(frame(&projection, more, 0.0, ADJUST, now));
    let output = projector.update(frame(&projection, more, 1.0, ADJUST, now));
    assert_eq!(output.wall, None);
    let fewer = at(handle(&tomori, Handle::Step(Axis::Columns, -1)));
    projector.update(frame(&projection, fewer, 0.0, ADJUST, now));
    let output = projector.update(frame(&projection, fewer, 1.0, ADJUST, now));
    assert_eq!(output.wall.unwrap().grid, Grid::new(5, 1));
}

/// Take hold of `wall`'s knob, which `projector` holds, with the hand at home, move the hand by each of `moves` in turn,
/// and let go: what is drawn after each move, and what is sent.
fn bend(
    projector: &mut Projector,
    projection: &Projection,
    wall: &Wall,
    moves: &[Vec3],
    controls: Controls,
    now: Instant,
) -> (Vec<Option<f32>>, Option<Change>) {
    let knob = handle(wall, Handle::Bend);
    let from = at(knob);
    let step = |projector: &mut Projector, ray: Option<Ray>, grip: f32| {
        let mut frame = frame(projection, ray, grip, ADJUST, now);
        frame.controls = controls;
        projector.update(frame)
    };
    step(projector, from, 0.0);
    step(projector, from, 1.0);
    assert_eq!(
        projector.editor.drag.as_ref().map(|drag| drag.handle),
        Some(Handle::Bend)
    );
    let mut drawn = Vec::new();
    let mut ray = from;
    for moved in moves {
        // The hand moved, still aiming about where it took hold.
        ray = match controls {
            Controls::Controllers => Ray::new(HOME + *moved, knob - HOME),
            Controls::Keyboard => Ray::new(HOME, knob + *moved - HOME),
        };
        let output = step(projector, ray, 1.0);
        drawn.push(projector.editor.walls(projection)[0].radius());
        // While it bends, the ray ends on the knob, and what it makes of the wall shows in place of the hint.
        assert!(output.hit.is_some_and(|hit| hit.held));
    }
    (drawn, step(projector, ray, 0.0).wall)
}

#[test]
fn the_knob_drawn_towards_the_participant_bends_a_wall_evenly_and_back_flattens_it() {
    let wall = added();
    let projection = shown(vec![wall.clone()], Vec::new());
    let now = Instant::now();
    let towards = model(&wall.pose).unwrap().transform_vector3(Vec3::Z);
    // Drawn towards the participant step by step, it bends more at every step, evenly in its curvature, from flat to
    // as tightly as it may.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let moves: Vec<Vec3> = (1..=20).map(|step| towards * 0.02 * step as f32).collect();
    let (drawn, sent) = bend(
        &mut projector,
        &projection,
        &wall,
        &moves,
        Controls::Controllers,
        now,
    );
    let curvature: Vec<f32> = drawn
        .iter()
        .map(|radius| radius.map_or(0.0, |radius| 1.0 / radius))
        .collect();
    assert!(
        curvature.windows(2).all(|pair| pair[1] >= pair[0] - 1e-4),
        "{curvature:?}"
    );
    assert_eq!(drawn[0], None, "a little is flat");
    let most = most_bend(&wall);
    assert!(
        (curvature[19] - most).abs() < 0.02,
        "{curvature:?} to {most}"
    );
    // Even: halfway along, about half as curved.
    assert!((curvature[9] - most / 2.0).abs() < 0.03, "{curvature:?}");
    let Some(WallShape::Arc { radius }) = sent.as_ref().and_then(|change| change.shape) else {
        unreachable!("an arc, not {sent:?}");
    };
    assert!(radius >= layout::LEAST_RADIUS && Some(radius) == drawn[19]);
    // Back where it took hold, it is as it was, and nothing is sent; on the way, it flattens as it bent.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let there_and_back = [towards * 0.2, towards * 0.1, towards * 0.001];
    let (drawn, sent) = bend(
        &mut projector,
        &projection,
        &wall,
        &there_and_back,
        Controls::Controllers,
        now,
    );
    assert!(drawn[0].unwrap() < drawn[1].unwrap());
    assert_eq!((drawn[2], sent), (None, None));
    // A curved wall: back to where it took hold, its radius exactly as it was; pushed away, flatter, then flat, and no
    // further.
    let curved = wall.clone().with_shape(WallShape::Arc { radius: 3.0 });
    let projection = shown(vec![curved.clone()], Vec::new());
    let mut projector = holding(&projection, &curved, ADJUST, now);
    assert_eq!(projector.editing_wall(), Some("wall-1"));
    let moves = [
        towards * 0.1,
        Vec3::ZERO,
        -towards * 0.05,
        -towards * 0.3,
        -towards * 0.6,
    ];
    let (drawn, sent) = bend(
        &mut projector,
        &projection,
        &curved,
        &moves,
        Controls::Controllers,
        now,
    );
    assert!(drawn[0].unwrap() < 3.0);
    assert_eq!(drawn[1], Some(3.0));
    assert!(drawn[2].unwrap() > 3.0);
    assert_eq!((drawn[3], drawn[4]), (None, None));
    assert_eq!(sent.and_then(|change| change.shape), Some(WallShape::Grid));
    assert_eq!(bent(&curved, 0.0), Some(3.0));
    assert_eq!(bent(&curved, 0.004), Some(3.0));
    assert_eq!(bent(&wall, f32::NAN), None);
}

#[test]
fn moving_the_hand_up_down_or_across_while_bending_changes_nothing_and_the_desktop_drags_up() {
    let wall = added();
    let projection = shown(vec![wall.clone()], Vec::new());
    let now = Instant::now();
    let towards = model(&wall.pose).unwrap().transform_vector3(Vec3::Z);
    let across = model(&wall.pose).unwrap().transform_vector3(Vec3::X);
    // Jitter up and down and across, however far, bends nothing.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let jitter = [
        Vec3::Y * 0.05,
        -Vec3::Y * 0.08,
        across * 0.1,
        Vec3::Y * 0.3 - across * 0.2,
    ];
    let (drawn, sent) = bend(
        &mut projector,
        &projection,
        &wall,
        &jitter,
        Controls::Controllers,
        now,
    );
    assert!(
        drawn.iter().all(Option::is_none) && sent.is_none(),
        "{drawn:?}"
    );
    // Drawn towards the participant with jitter, it bends as far as without.
    let mut steady = holding(&projection, &wall, ADJUST, now);
    let (still, _) = bend(
        &mut steady,
        &projection,
        &wall,
        &[towards * 0.2],
        Controls::Controllers,
        now,
    );
    let mut shaky = holding(&projection, &wall, ADJUST, now);
    let (shaken, _) = bend(
        &mut shaky,
        &projection,
        &wall,
        &[towards * 0.2 + Vec3::Y * 0.04 - across * 0.03],
        Controls::Controllers,
        now,
    );
    assert_eq!(still, shaken);
    assert!(still[0].is_some());
    // In the desktop window, whose ray begins at the eyes, dragging the knob up bends it, and down flattens it.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let (drawn, sent) = bend(
        &mut projector,
        &projection,
        &wall,
        &[Vec3::Y * 0.2, Vec3::ZERO],
        Controls::Keyboard,
        now,
    );
    assert!(
        drawn[0].is_some() && drawn[1].is_none() && sent.is_none(),
        "{drawn:?}"
    );
    // What it makes shows as it is dragged.
    assert_eq!(readout(Handle::Bend, &wall).plain().trim(), "flat");
    let curved = wall.with_shape(WallShape::Arc { radius: 2.4 });
    assert_eq!(
        readout(Handle::Bend, &curved).plain().trim(),
        "◠ radius 2.4 m"
    );
}

#[test]
fn b_removes_a_wall_carried_for_an_owner_asking_again_first_when_it_holds_panels() {
    let wall = added();
    let panel = Panel::new(
        "panel-1",
        "Build",
        "Green.",
        Place::Wall {
            wall: "wall-1".into(),
            slot: 2,
        },
    );
    let projection = shown(vec![wall.clone()], vec![panel]);
    let empty = shown(vec![wall.clone()], Vec::new());
    let now = Instant::now();
    let ray = at(on(&wall, 0.0, 0.42));
    let carry = |projection: &Projection| {
        let mut projector = Projector::default();
        projector.update(frame(projection, ray, 1.0, OWNER, now));
        assert_eq!(projector.carried_wall(), Some("wall-1"));
        projector
    };
    // Empty, one press removes it.
    let mut projector = carry(&empty);
    assert_eq!(
        projector.remove_wall(&empty, now),
        Some("wall-1".to_owned())
    );
    assert_eq!(projector.editing_wall(), None);
    // Holding a panel, the first press asks again, saying it will float, and a second within a while removes it.
    let mut projector = carry(&projection);
    assert_eq!(projector.remove_wall(&projection, now), None);
    let hint = projector
        .update(frame(&projection, ray, 1.0, OWNER, now))
        .sheets
        .last()
        .and_then(|sheet| sheet.canvas.as_ref())
        .unwrap()
        .plain();
    assert!(
        hint.contains("B again") && hint.contains("1 panel floats"),
        "{hint}"
    );
    assert_eq!(
        projector.remove_wall(&projection, now + Duration::from_secs(1)),
        Some("wall-1".to_owned())
    );
    // Too late, it asks again.
    let mut projector = carry(&projection);
    assert_eq!(projector.remove_wall(&projection, now), None);
    projector.update(frame(&projection, ray, 1.0, OWNER, now + CONFIRM));
    assert_eq!(projector.remove_wall(&projection, now + CONFIRM), None);
    // Nothing carried, nothing is removed.
    let mut projector = Projector::default();
    assert_eq!(projector.remove_wall(&projection, now), None);
    // An owner's carrying hint says B removes it, warm.
    let hint = super::hint(Controls::Controllers, &wall, true, true, None).plain();
    assert!(hint.contains("remove, for all"), "{hint}");
}

#[test]
fn a_wall_no_longer_shown_or_rights_lost_let_go_of_it() {
    let wall = added();
    let projection = shown(vec![wall.clone()], Vec::new());
    let now = Instant::now();
    let mut projector = holding(&projection, &wall, ADJUST, now);
    assert_eq!(projector.editing_wall(), Some("wall-1"));
    projector.update(frame(
        &shown(Vec::new(), Vec::new()),
        None,
        0.0,
        ADJUST,
        now,
    ));
    assert_eq!(projector.editing_wall(), None);
    let mut projector = holding(&projection, &wall, ADJUST, now);
    projector.update(frame(&projection, None, 0.0, Rights::default(), now));
    assert_eq!(projector.editing_wall(), None);
    // Leaving the space lets go of it too.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let mut away = frame(&projection, None, 0.0, ADJUST, now);
    away.projection = None;
    projector.update(away);
    assert_eq!(projector.editing_wall(), None);
}

#[test]
fn a_ray_along_or_away_from_a_wall_meets_its_plane_nowhere_and_a_drag_along_it_changes_nothing() {
    let wall = added();
    let frame = model(&wall.pose).unwrap();
    let along = Ray::new(on(&wall, 0.0, 0.0) + Vec3::Z * 0.5, Vec3::X).unwrap();
    assert_eq!(on_plane(frame, &along), None);
    let away = Ray::new(on(&wall, 0.0, 0.0) + Vec3::Z * 0.5, Vec3::Z).unwrap();
    assert_eq!(on_plane(frame, &away), None);
    // A corner held while the ray runs along the wall's plane leaves the wall as it was.
    let projection = shown(vec![wall.clone()], Vec::new());
    let now = Instant::now();
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let corner = at(handle(&wall, Handle::Corner(1, 1)));
    projector.update(frame_at(&projection, corner, 0.0, now));
    projector.update(frame_at(&projection, corner, 1.0, now));
    projector.update(frame_at(&projection, Some(along), 1.0, now));
    assert_eq!(
        projector
            .update(frame_at(&projection, Some(along), 0.0, now))
            .wall,
        None
    );
    // Nothing carried, nothing is removed.
    assert_eq!(Editor::default().remove(0, now), None);
}

/// A frame at `now` for a participant who may adjust walls.
fn frame_at<'a>(
    projection: &'a Projection,
    ray: Option<Ray>,
    grip: f32,
    now: Instant,
) -> Frame<'a> {
    frame(projection, ray, grip, ADJUST, now)
}

#[test]
fn a_change_sent_is_drawn_until_the_server_agrees_curved_or_not_or_a_while_passes() {
    let wall = added();
    let projection = shown(vec![wall.clone()], Vec::new());
    let now = Instant::now();
    let towards = model(&wall.pose).unwrap().transform_vector3(Vec3::Z);
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let (_, sent) = bend(
        &mut projector,
        &projection,
        &wall,
        &[towards * 0.3],
        Controls::Controllers,
        now,
    );
    let Some(Change {
        shape: Some(WallShape::Arc { radius }),
        ..
    }) = sent
    else {
        unreachable!("bent");
    };
    // The server not yet agreeing, it is drawn as sent.
    projector.update(frame_at(
        &projection,
        None,
        0.0,
        now + Duration::from_secs(1),
    ));
    assert_eq!(
        projector.editor.walls(&projection)[0].radius(),
        Some(radius)
    );
    // Curved at another radius, it does not agree; curved as sent, it does, saying its grid and scale, as the server
    // says them of a curved wall that is not its shape's own.
    let curved = |radius: f32| {
        wall.clone()
            .with_shape(WallShape::Arc { radius })
            .with_grid(wall.grid())
            .with_scale(1.0)
    };
    let other = shown(vec![curved(radius + 1.0)], Vec::new());
    projector.update(frame_at(&other, None, 0.0, now + Duration::from_secs(1)));
    assert!(projector.editor.pending.is_some());
    let agreed = shown(vec![curved(radius)], Vec::new());
    projector.update(frame_at(&agreed, None, 0.0, now + Duration::from_secs(1)));
    assert!(projector.editor.pending.is_none());
}

#[test]
fn of_two_handles_or_two_walls_the_ray_meets_the_nearer_is_taken_and_a_press_without_a_ray_takes_none()
 {
    let wall = added();
    let behind = Wall::new(
        "wall-2",
        WallKind::Added,
        Pose::upright_facing([0.0, 1.5, -4.0], HOME.to_array()),
    );
    let projection = shown(vec![behind.clone(), wall.clone()], Vec::new());
    let now = Instant::now();
    // The ray meets both walls, through an empty slot of the nearer; the nearer is taken.
    let mut projector = Projector::default();
    let through = at(on(&wall, 0.0, 0.05)).unwrap();
    let frame = model(&behind.pose).unwrap();
    assert!(crate::projection::wall_hit(&behind, frame, &through).is_some());
    projector.update(frame_at(&projection, Some(through), 1.0, now));
    assert_eq!(projector.editing_wall(), Some("wall-1"));
    // Between the two column steppers, the ray meets neither, in the gap between them; a little to either side, the one
    // whose middle it passes nearer.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let (fewer, more) = (
        handle(&wall, Handle::Step(Axis::Columns, -1)),
        handle(&wall, Handle::Step(Axis::Columns, 1)),
    );
    projector.update(frame_at(&projection, at(fewer.lerp(more, 0.5)), 0.0, now));
    assert_eq!(projector.editor.hovered, None);
    projector.update(frame_at(&projection, at(fewer.lerp(more, 0.3)), 0.0, now));
    assert_eq!(
        projector.editor.hovered,
        Some(Handle::Step(Axis::Columns, -1))
    );
    projector.update(frame_at(&projection, at(fewer.lerp(more, 0.7)), 0.0, now));
    assert_eq!(
        projector.editor.hovered,
        Some(Handle::Step(Axis::Columns, 1))
    );
    // A squeeze while the ray is not tracked takes nothing, and lets go of nothing.
    projector.update(frame_at(&projection, None, 1.0, now));
    assert_eq!(projector.editing_wall(), Some("wall-1"));
}

#[test]
fn a_wall_carried_turns_with_the_stick_and_squares_up_to_face_the_participant() {
    let wall = Wall::new(
        "wall-1",
        WallKind::Added,
        // Turned a little away from home.
        Pose::upright_facing([0.0, 1.5, -2.5], [1.5, 1.5, 0.0]),
    );
    let projection = shown(vec![wall.clone()], Vec::new());
    let now = Instant::now();
    let ray = at(on(&wall, 0.0, 0.42));
    let mut projector = Projector::default();
    projector.update(frame_at(&projection, ray, 1.0, now));
    let mut flick = frame_at(&projection, ray, 1.0, now);
    flick.stick = Vec2::new(1.0, 0.0);
    assert!(projector.update(flick).wall_ticked);
    let mut square = frame_at(&projection, ray, 1.0, now);
    square.straighten = true;
    assert!(projector.update(square).wall_ticked);
    let change = projector
        .update(frame_at(&projection, ray, 0.0, now))
        .wall
        .unwrap();
    let pose = change.pose.unwrap();
    // Squared up, it faces home.
    let facing = glam::Quat::from_array(pose.orientation) * Vec3::Z;
    let towards = (HOME - Vec3::from_array(pose.position))
        .with_y(0.0)
        .normalize();
    assert!(facing.dot(towards) > 0.999, "{facing} {towards}");
}

#[test]
fn the_hint_counts_the_panels_that_would_float_and_names_the_keyboard_s_keys() {
    let wall = added();
    let confirm = super::hint(Controls::Controllers, &wall, true, true, Some(2)).plain();
    assert!(confirm.contains("remove; 2 panels float"), "{confirm}");
    let keys = super::hint(Controls::Keyboard, &wall, false, true, None).plain();
    assert!(
        keys.contains("G corner") && keys.contains("Backspace") && keys.contains("done"),
        "{keys}"
    );
}

#[test]
fn a_handle_the_ray_meets_is_pointed_at_rather_than_a_panel_behind_it() {
    let wall = added();
    let edge = handle(&wall, Handle::Columns);
    let beyond = edge + (edge - HOME).normalize() * 1.5;
    let floating = Panel::new(
        "panel-1",
        "Behind",
        "Far.",
        Place::Free {
            pose: Pose::upright_facing(beyond.to_array(), HOME.to_array()),
        },
    );
    let projection = shown(vec![wall.clone()], vec![floating]);
    let now = Instant::now();
    // Not taken hold of, the ray points at the panel behind where the handle would be.
    let mut projector = Projector::default();
    let output = projector.update(frame_at(&projection, at(edge), 0.0, now));
    assert_eq!(
        output.pointed.map(|(panel, ..)| panel),
        Some("panel-1".to_owned())
    );
    // Taken hold of, its handle is pointed at instead, and a squeeze takes the handle, not the panel.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let output = projector.update(frame_at(&projection, at(edge), 0.0, now));
    assert_eq!(output.pointed, None);
    projector.update(frame_at(&projection, at(edge), 1.0, now));
    assert_eq!(projector.held(), None);
    assert_eq!(
        projector.editor.drag.as_ref().map(|drag| drag.handle),
        Some(Handle::Columns)
    );
}

/// The walls whose handles are checked: every grid's extremes, sizes and shapes of slot, flat and curved.
fn walls_of_every_size() -> Vec<Wall> {
    let mut walls = Vec::new();
    for (columns, rows) in [(1, 1), (2, 1), (3, 2), (4, 1), (8, 3), (6, 4), (1, 4)] {
        for scale in [0.5, 1.0, 2.0] {
            for aspect in [None, Some(0.25), Some(0.5625), Some(4.0)] {
                for radius in [None, Some(1.5), Some(3.0)] {
                    let wall = added()
                        .with_grid(Grid::new(columns, rows).unwrap())
                        .with_scale(scale)
                        .with_aspect(aspect)
                        .with_shape(
                            radius.map_or(WallShape::Grid, |radius| WallShape::Arc { radius }),
                        );
                    if allowed(&wall) {
                        walls.push(wall);
                    }
                }
            }
        }
    }
    walls.push(tomori());
    walls.push(tomori().with_grid(Grid::new(1, 1).unwrap()));
    walls
}

/// Each handle of `wall` seen from `viewer`, as the rectangle its face takes from the eyes: its middle's angles across
/// and up from where the wall's middle is seen, and half its width and height, in radians.
fn seen(wall: &Wall, viewer: Vec3) -> Vec<(Handle, Vec2, Vec2)> {
    let frame = model(&wall.pose).unwrap();
    let ahead = (frame.transform_point3(Vec3::ZERO) - viewer).normalize();
    let right = ahead.cross(Vec3::Y).normalize();
    let up = right.cross(ahead);
    let (points, size) = handle_points(wall, frame, viewer);
    points
        .into_iter()
        .map(|(handle, at)| {
            let to = at - viewer;
            let distance = to.length();
            let to = to / distance;
            let face = handle_face(handle, size, false).size();
            (
                handle,
                Vec2::new(to.dot(right).atan2(to.dot(ahead)), to.dot(up).asin()),
                Vec2::new(
                    (face.0 / 2.0 / distance).atan(),
                    (face.1 / 2.0 / distance).atan(),
                ),
            )
        })
        .collect()
}

#[test]
fn no_two_handles_overlap_at_any_size_shape_or_distance_and_each_is_large_enough_to_aim_at() {
    let walls = walls_of_every_size();
    assert!(walls.len() > 100, "{}", walls.len());
    let mut least_gap = f32::INFINITY;
    let mut checked = 0;
    for wall in &walls {
        let middle = Vec3::from_array(wall.pose.position);
        // From nearby to across the space, straight on and from either side.
        for distance in [0.8, 1.5, 3.0, 6.0] {
            for side in [-50f32, -25.0, 0.0, 25.0, 50.0] {
                let facing = model(&wall.pose).unwrap().transform_vector3(Vec3::Z);
                let viewer = middle + Quat::from_rotation_y(side.to_radians()) * facing * distance;
                // Only where the whole wall is within view as its middle is looked at, as a participant editing it
                // would stand; nearer, they turn to its edges.
                let ahead = (middle - viewer).normalize();
                let frame = model(&wall.pose).unwrap();
                let [a, b, c, d] = corners(wall);
                if [a, b, c, d].iter().any(|corner| {
                    (frame.transform_point3(*corner) - viewer)
                        .normalize()
                        .dot(ahead)
                        < 55f32.to_radians().cos()
                }) {
                    continue;
                }
                checked += 1;
                let handles = seen(wall, viewer);
                for (index, (handle, at, half)) in handles.iter().enumerate() {
                    // About 1.5° across at least, as the eyes see it.
                    assert!(
                        half.x * 2.0 > 1.4f32.to_radians(),
                        "{handle:?} of {wall:?} at {distance} m"
                    );
                    for (other, other_at, other_half) in &handles[index + 1..] {
                        let gap = ((at.x - other_at.x).abs() - half.x - other_half.x)
                            .max((at.y - other_at.y).abs() - half.y - other_half.y);
                        least_gap = least_gap.min(gap);
                        assert!(
                            gap > 0.3f32.to_radians(),
                            "{handle:?} and {other:?} of {:?} {:?} at {distance} m from {side}°: {}°",
                            wall.grid(),
                            (wall.scale(), wall.aspect(), wall.radius()),
                            gap.to_degrees()
                        );
                    }
                }
            }
        }
    }
    assert!(least_gap.is_finite());
    assert!(checked > 1_000, "{checked}");
}

#[test]
fn a_ray_at_a_handle_s_edge_takes_it_and_never_its_neighbour() {
    let now = Instant::now();
    for wall in [
        added(),
        added().with_grid(Grid::new(1, 1).unwrap()).with_scale(0.5),
        added()
            .with_grid(Grid::new(8, 3).unwrap())
            .with_aspect(Some(0.5625))
            .with_scale(0.5),
        added().with_shape(WallShape::Arc { radius: 2.0 }),
    ] {
        let projection = shown(vec![wall.clone()], Vec::new());
        let frame = model(&wall.pose).unwrap();
        let (points, size) = handle_points(&wall, frame, HOME);
        let mut projector = holding(&projection, &wall, ADJUST, now);
        assert_eq!(projector.editing_wall(), Some("wall-1"));
        for (handle, middle) in &points {
            let towards = (*middle - HOME).normalize();
            let right = towards.cross(Vec3::Y).normalize();
            let up = right.cross(towards);
            // Within its face, to each edge, it is the handle taken.
            for edge in [right, -right, up, -up] {
                let point = *middle + edge * size * 0.45;
                projector.update(frame_at(&projection, at(point), 0.0, now));
                assert_eq!(
                    projector.editor.hovered,
                    Some(*handle),
                    "{handle:?} of {:?}",
                    wall.grid()
                );
                // Beyond it, in the gap, it is that handle or none, never another.
                let beyond = *middle + edge * size * 0.88;
                projector.update(frame_at(&projection, at(beyond), 0.0, now));
                assert!(
                    projector
                        .editor
                        .hovered
                        .is_none_or(|hovered| hovered == *handle),
                    "{handle:?} beyond, of {:?}: {:?}",
                    wall.grid(),
                    projector.editor.hovered
                );
            }
        }
    }
}

#[test]
fn the_ray_ends_on_a_handle_a_wall_or_a_panel_whichever_is_nearest_and_on_what_is_held() {
    let wall = added();
    let panel = Panel::new(
        "panel-1",
        "Build",
        "Green.",
        Place::Wall {
            wall: "wall-1".into(),
            slot: 0,
        },
    );
    let projection = shown(vec![wall.clone()], vec![panel]);
    let now = Instant::now();
    let length = |output: &crate::projection::Output, ray: Option<Ray>| {
        let hit = output.hit.expect("the ray ends on something");
        let ray = ray.unwrap();
        assert!((ray.at(hit.distance) - hit.point).length() < 1e-3);
        hit
    };
    // On the board, where the participant may take hold of the wall, at the board.
    let mut projector = Projector::default();
    let board = at(on(&wall, 0.0, 0.42));
    let output = projector.update(frame_at(&projection, board, 0.0, now));
    let hit = length(&output, board);
    assert!((hit.point - on(&wall, 0.0, 0.42)).length() < 1e-3 && !hit.held);
    // On a panel in a slot, at the panel, just in front of the wall.
    let slot = model(&wall.pose)
        .unwrap()
        .transform_point3(Vec3::from_array(wall.slot_frames()[0].position));
    let output = projector.update(frame_at(&projection, at(slot), 0.0, now));
    let hit = length(&output, at(slot));
    assert!((hit.distance - (slot - HOME).length()).abs() < 1e-3);
    assert_eq!(
        output.pointed.map(|(id, ..)| id),
        Some("panel-1".to_owned())
    );
    // A visitor takes hold of no wall, so the ray passes its board, but still ends on a panel.
    let output = projector.update(frame(&projection, board, 0.0, Rights::default(), now));
    assert_eq!(output.hit, None);
    let output = projector.update(frame(&projection, at(slot), 0.0, Rights::default(), now));
    assert!(output.hit.is_some());
    // On a handle of a wall taken hold of, at the handle, however near the wall behind it.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let corner = handle(&wall, Handle::Corner(1, 1));
    let output = projector.update(frame_at(&projection, at(corner), 0.0, now));
    let hit = length(&output, at(corner));
    assert!((hit.point - corner).length() < 1e-3 && !hit.held);
    // Dragged, it ends on the handle, held, wherever the ray goes.
    projector.update(frame_at(&projection, at(corner), 1.0, now));
    let output = projector.update(frame_at(&projection, at(corner + Vec3::X * 0.3), 1.0, now));
    assert!(output.hit.is_some_and(|hit| hit.held));
    projector.update(frame_at(&projection, at(corner + Vec3::X * 0.3), 0.0, now));
    // A wall carried, held, where it was taken.
    let mut projector = Projector::default();
    let added = shown(vec![wall.clone()], Vec::new());
    projector.update(frame_at(&added, board, 1.0, now));
    let output = projector.update(frame_at(&added, board, 1.0, now));
    let hit = length(&output, board);
    assert!(hit.held && (hit.point - on(&wall, 0.0, 0.42)).length() < 1e-3);
    // Nothing met, nothing ends it.
    let mut projector = Projector::default();
    let output = projector.update(frame_at(&projection, at(HOME + Vec3::Y), 0.0, now));
    assert_eq!(output.hit, None);
}

#[test]
fn of_two_handles_the_ray_meets_the_one_whose_middle_it_passes_nearest_is_taken() {
    // Two handles closer together than handles ever stand, both within reach of a ray between them.
    let ray = Ray::new(Vec3::ZERO, Vec3::NEG_Z).unwrap();
    let near = Vec3::new(0.01, 0.0, -1.0);
    let far = Vec3::new(-0.02, 0.0, -1.0);
    let points = [
        (Handle::Step(Axis::Columns, -1), far),
        (Handle::Step(Axis::Columns, 1), near),
    ];
    let taken = nearest_handle(&points, 0.05, &ray).unwrap();
    assert_eq!(taken.0, Handle::Step(Axis::Columns, 1));
    assert!((taken.1 - 1.0).abs() < 1e-5);
    // Nearer along the ray, but further from its middle, it is not taken first.
    let nearer = [
        (Handle::Bend, Vec3::new(0.03, 0.0, -0.9)),
        (Handle::Rows, near),
    ];
    assert_eq!(nearest_handle(&nearer, 0.05, &ray).unwrap().0, Handle::Rows);
    // Behind the ray, or beyond its reach, none.
    assert_eq!(
        nearest_handle(&[(Handle::Bend, Vec3::new(0.0, 0.0, 1.0))], 0.05, &ray),
        None
    );
    assert_eq!(
        nearest_handle(&[(Handle::Bend, Vec3::new(0.05, 0.0, -1.0))], 0.05, &ray),
        None
    );
}

/// A panel in every slot of `wall`.
fn filling(wall: &Wall) -> Vec<Panel> {
    (0..wall.grid().slots())
        .map(|slot| {
            Panel::new(
                format!("panel-{slot}"),
                "Notes",
                "Full.",
                Place::Wall {
                    wall: wall.id.clone(),
                    slot,
                },
            )
        })
        .collect()
}

/// `wall`'s bar, standing where it says, seen from home.
fn bar_of(wall: &Wall) -> Bar {
    bar(wall, model(&wall.pose).unwrap(), HOME)
}

/// The bars drawn in `output`: faces without text, filled.
fn bars(output: &crate::projection::Output) -> Vec<&Canvas> {
    output
        .sheets
        .iter()
        .filter_map(|sheet| sheet.canvas.as_ref())
        .filter(|canvas| canvas.columns == 0 && canvas.rows.is_empty())
        .collect()
}

/// The viewers a wall is checked from: nearby to across the space, straight on and from either side, where the whole
/// wall is within view as its middle is looked at, as a participant would stand.
fn viewers(wall: &Wall) -> Vec<Vec3> {
    let frame = model(&wall.pose).unwrap();
    let middle = Vec3::from_array(wall.pose.position);
    let facing = frame.transform_vector3(Vec3::Z);
    let mut viewers = Vec::new();
    for distance in [0.8, 1.5, 3.0, 6.0] {
        for side in [-50f32, -25.0, 0.0, 25.0, 50.0] {
            let viewer = middle + Quat::from_rotation_y(side.to_radians()) * facing * distance;
            let ahead = (middle - viewer).normalize();
            if corners(wall).iter().all(|corner| {
                (frame.transform_point3(*corner) - viewer)
                    .normalize()
                    .dot(ahead)
                    >= 55f32.to_radians().cos()
            }) {
                viewers.push(viewer);
            }
        }
    }
    viewers
}

#[test]
fn a_wall_s_bar_is_met_along_its_length_and_across_its_thickness_flat_or_curved_near_or_far() {
    let mut checked = 0;
    for wall in walls_of_every_size() {
        let frame = model(&wall.pose).unwrap();
        for viewer in viewers(&wall) {
            checked += 1;
            let bar = bar(&wall, frame, viewer);
            let up = frame.transform_vector3(Vec3::Y);
            let distance = bar.middle.distance(viewer);
            // At least about 1.7° to aim at across it, and three handles along it, as the eyes see it.
            let across = 2.0 * (bar.size * HANDLE_REACH / distance).atan();
            assert!(across > 1.7f32.to_radians(), "{wall:?} from {viewer}");
            let (left, right) = (bar.middle - bar.half, bar.middle + bar.half);
            let along = (left - viewer)
                .normalize()
                .angle_between((right - viewer).normalize());
            assert!(
                along > 2.5 * HANDLE_ANGLE * 0.9,
                "{:?} {:?} from {viewer}: {}°",
                wall.grid(),
                (wall.scale(), wall.aspect(), wall.radius()),
                along.to_degrees()
            );
            // Along it, end to end, and up and down within most of a handle, the ray meets it where it passes it.
            for s in [-0.95, -0.5, 0.0, 0.5, 0.95] {
                for v in [-0.7, 0.0, 0.7] {
                    let point = bar.middle + bar.half * s + up * bar.size * v;
                    let ray = Ray::new(viewer, point - viewer).unwrap();
                    let met = bar
                        .hit(&ray)
                        .unwrap_or_else(|| panic!("{s} {v} of {:?} from {viewer}", wall.grid()));
                    assert!((met - point.distance(viewer)).abs() < bar.size, "{met}");
                }
            }
            // Well beyond it, above or below, as seen from in front of it, it is not met.
            let facing = frame.transform_vector3(Vec3::Z);
            if (viewer - bar.middle).normalize().dot(facing) > 0.99 {
                for v in [-1.5, 1.5] {
                    let point = bar.middle + up * bar.size * v;
                    assert_eq!(bar.hit(&Ray::new(viewer, point - viewer).unwrap()), None);
                }
            }
            // Nor is it behind the ray.
            assert_eq!(
                bar.hit(&Ray::new(viewer, viewer - bar.middle).unwrap()),
                None
            );
        }
    }
    assert!(checked > 1_000, "{checked}");
    // A ray along the bar, from beyond its end, meets it.
    let bar = bar_of(&added());
    let length = bar.half.length();
    let met = bar
        .hit(&Ray::new(bar.middle + bar.half * 3.0, -bar.half).unwrap())
        .unwrap();
    assert!(
        met >= 2.0 * length - 1e-3 && met <= 3.0 * length + 1e-3,
        "{met}"
    );
}

#[test]
fn the_bar_stands_clear_of_every_handle_and_its_drawn_face_within_where_it_is_met() {
    let mut checked = 0;
    for wall in walls_of_every_size() {
        let frame = model(&wall.pose).unwrap();
        for viewer in viewers(&wall) {
            checked += 1;
            let (points, size) = handle_points(&wall, frame, viewer);
            let bar = bar(&wall, frame, viewer);
            assert!((bar.size - size).abs() < 1e-6);
            // A ray at any handle's middle, or near its edge, meets no bar; and one anywhere along the bar meets no
            // handle, so that the two are never confused.
            for (handle, middle) in &points {
                let towards = (*middle - viewer).normalize();
                let right = towards.cross(Vec3::Y).normalize();
                let up = right.cross(towards);
                for edge in [Vec3::ZERO, right, -right, up, -up] {
                    let point = *middle + edge * size * 0.45;
                    let ray = Ray::new(viewer, point - viewer).unwrap();
                    assert_eq!(
                        bar.hit(&ray),
                        None,
                        "{handle:?} of {:?} {:?} from {viewer}",
                        wall.grid(),
                        (wall.scale(), wall.aspect(), wall.radius())
                    );
                }
            }
            let up = frame.transform_vector3(Vec3::Y);
            for step in 0..=20 {
                let s = -1.0 + step as f32 / 10.0;
                for v in [-0.4, 0.0, 0.4] {
                    let point = bar.middle + bar.half * s + up * size * v;
                    let ray = Ray::new(viewer, point - viewer).unwrap();
                    assert_eq!(
                        nearest_handle(&points, size, &ray),
                        None,
                        "{s} along the bar of {:?} {:?} from {viewer}",
                        wall.grid(),
                        (wall.scale(), wall.aspect(), wall.radius())
                    );
                }
            }
            // Drawn thin, as long as it is, within where the ray meets it.
            let face = bar_face(&bar, false).size();
            // To within a font pixel, an eighth of its thickness, at either end.
            assert!(
                (face.0 - 2.0 * bar.half.length()).abs() <= face.1 / 8.0 * 1.01,
                "{face:?}"
            );
            assert!(
                face.1 < 2.0 * size * HANDLE_REACH && face.1 > 0.3 * size,
                "{face:?} {size}"
            );
        }
    }
    assert!(checked > 1_000, "{checked}");
}

#[test]
fn a_full_curved_wall_is_taken_hold_of_and_carried_again_by_its_bar_while_its_panels_still_take_the_grip()
 {
    let now = Instant::now();
    let wall = added()
        .with_grid(Grid::new(4, 2).unwrap())
        .with_shape(WallShape::Arc { radius: 2.0 });
    let projection = shown(vec![wall.clone(), tomori()], filling(&wall));
    let bar = bar_of(&wall);
    // A panel pointed at takes the grip, as ever.
    let slot = model(&wall.pose)
        .unwrap()
        .transform_point3(Vec3::from_array(wall.slot_frames()[5].position));
    let mut projector = Projector::default();
    projector.update(frame_at(&projection, at(slot), 1.0, now));
    assert_eq!(projector.held(), Some("panel-5"));
    assert_eq!(projector.editing_wall(), None);
    // The bar, at either end and its middle, takes hold of the wall and, as it was added, carries it.
    for s in [-0.9, 0.0, 0.9] {
        let mut projector = Projector::default();
        let ray = at(bar.middle + bar.half * s);
        let output = projector.update(frame_at(&projection, ray, 1.0, now));
        assert!(output.wall_ticked);
        assert_eq!(projector.held(), None);
        assert_eq!(projector.carried_wall(), Some("wall-1"), "{s}");
        // Let go, its handles show, the bar still where it was, and it carries the wall again.
        projector.update(frame_at(&projection, ray, 0.0, now));
        assert_eq!(projector.editing_wall(), Some("wall-1"));
        assert_eq!(projector.carried_wall(), None);
        let output = projector.update(frame_at(&projection, ray, 0.0, now));
        let hint = output
            .sheets
            .last()
            .unwrap()
            .canvas
            .as_ref()
            .unwrap()
            .plain();
        assert!(
            hint.contains("Grip bar") && hint.contains("carry it"),
            "{hint}"
        );
        projector.update(frame_at(&projection, ray, 1.0, now));
        assert_eq!(projector.carried_wall(), Some("wall-1"));
    }
    // The Tomori wall, every slot full, is taken hold of by its bar below it, and not carried.
    let tomori = tomori();
    let projection = shown(vec![tomori.clone()], filling(&tomori));
    let bar = bar_of(&tomori);
    let below =
        model(&tomori.pose)
            .unwrap()
            .transform_point3(Vec3::new(0.0, -tomori.size().1 / 2.0, 0.0));
    assert!(
        bar.middle.y < below.y && bar.middle.y > 0.15,
        "{:?}",
        bar.middle
    );
    let mut projector = Projector::default();
    projector.update(frame_at(&projection, at(bar.middle), 1.0, now));
    assert_eq!(projector.editing_wall(), Some(TOMORI_WALL));
    assert_eq!(projector.carried_wall(), None);
    assert_eq!(projector.held(), None);
}

#[test]
fn a_panel_in_front_of_a_bar_takes_the_grip_and_the_bar_in_front_of_one_takes_it_and_ends_the_ray()
{
    let now = Instant::now();
    let wall = added();
    let bar = bar_of(&wall);
    let towards = (bar.middle - HOME).normalize();
    let floating = |at: Vec3| {
        Panel::new(
            "panel-1",
            "Floating",
            "Here.",
            Place::Free {
                pose: Pose::upright_facing(at.to_array(), HOME.to_array()),
            },
        )
    };
    // Floating in front of it, the panel is pointed at and taken.
    let projection = shown(vec![wall.clone()], vec![floating(HOME + towards * 1.0)]);
    let mut projector = Projector::default();
    let output = projector.update(frame_at(&projection, at(bar.middle), 0.0, now));
    assert_eq!(
        output.pointed.map(|(id, ..)| id),
        Some("panel-1".to_owned())
    );
    projector.update(frame_at(&projection, at(bar.middle), 1.0, now));
    assert_eq!(projector.held(), Some("panel-1"));
    assert_eq!(projector.editing_wall(), None);
    // Floating behind it, the bar is met first: pointed at rather than the panel, ending the ray, and taken.
    let projection = shown(
        vec![wall.clone()],
        vec![floating(bar.middle + towards * 1.0)],
    );
    let mut projector = Projector::default();
    let ray = at(bar.middle);
    let output = projector.update(frame_at(&projection, ray, 0.0, now));
    assert!(output.wall_ticked);
    assert_eq!(output.pointed, None);
    let hit = output.hit.expect("the ray ends on the bar");
    assert!(!hit.held);
    assert!(
        (hit.distance - bar.middle.distance(HOME)).abs() < 0.01,
        "{hit:?}"
    );
    assert!((ray.unwrap().at(hit.distance) - hit.point).length() < 1e-3);
    projector.update(frame_at(&projection, ray, 1.0, now));
    assert_eq!(projector.held(), None);
    assert_eq!(projector.carried_wall(), Some("wall-1"));
    // Something nearer that the grip would take, such as a card, takes it instead.
    let mut projector = Projector::default();
    let mut nearer = frame_at(&projection, ray, 1.0, now);
    nearer.nearer = 0.5;
    projector.update(nearer);
    assert_eq!(projector.editing_wall(), None);
}

#[test]
fn bars_show_only_to_those_who_may_adjust_walls_highlighted_with_what_the_grip_does_while_the_ray_meets_one()
 {
    let now = Instant::now();
    let wall = added();
    let projection = shown(vec![wall.clone(), tomori()], Vec::new());
    let bar = bar_of(&wall);
    let up = Vec3::Y * 2.0;
    // A visitor, or anyone where the server takes no wall changes, sees none, and the ray passes through them.
    let mut projector = Projector::default();
    let output = projector.update(frame(
        &projection,
        at(bar.middle),
        0.0,
        Rights::default(),
        now,
    ));
    assert!(bars(&output).is_empty());
    assert_eq!(output.hit, None);
    projector.update(frame(
        &projection,
        at(bar.middle),
        1.0,
        Rights::default(),
        now,
    ));
    assert_eq!(projector.editing_wall(), None);
    // A participant who may sees one on each wall, subtly.
    let mut projector = Projector::default();
    let output = projector.update(frame_at(&projection, at(HOME + up), 0.0, now));
    let shown = bars(&output);
    assert_eq!(shown.len(), 2);
    assert!(shown.iter().all(|face| face.background[3] < 0.5));
    // The ray on one highlights it and says what the grip does, ticking once as it arrives.
    let output = projector.update(frame_at(&projection, at(bar.middle), 0.0, now));
    assert!(output.wall_ticked);
    assert_eq!(
        bars(&output)
            .iter()
            .filter(|face| face.background == HIGHLIGHT)
            .count(),
        1
    );
    let label = |output: &crate::projection::Output| {
        output
            .sheets
            .iter()
            .filter_map(|sheet| {
                sheet
                    .canvas
                    .as_ref()
                    .map(|canvas| (sheet.model, canvas.plain()))
            })
            .find(|(_, text)| text.contains("edit this wall"))
    };
    let (model, text) = label(&output).expect("what the grip does");
    assert!(
        text.contains("Grip") && text.contains("carry and edit"),
        "{text}"
    );
    assert!(model.unwrap().w_axis.y > bar.middle.y);
    let output = projector.update(frame_at(&projection, at(bar.middle), 0.0, now));
    assert!(!output.wall_ticked);
    // On the Tomori wall's, below it, which is edited but not carried.
    let low = bar_of(&tomori());
    let output = projector.update(frame_at(&projection, at(low.middle), 0.0, now));
    let (model, text) = label(&output).expect("what the grip does");
    assert!(!text.contains("carry"), "{text}");
    assert!(model.unwrap().w_axis.y < low.middle.y);
    // The desktop window names its key.
    assert!(
        bar_label(Controls::Keyboard, &wall)
            .plain()
            .starts_with("G  ")
    );
    // Carried, its bar is highlighted; held, its bar still shows, and what the grip does there is the hint's to say.
    projector.update(frame_at(&projection, at(bar.middle), 1.0, now));
    let output = projector.update(frame_at(
        &projection,
        at(bar.middle + Vec3::X * 0.2),
        1.0,
        now,
    ));
    assert_eq!(
        bars(&output)
            .iter()
            .filter(|face| face.background == HIGHLIGHT)
            .count(),
        1
    );
    projector.update(frame_at(
        &projection,
        at(bar.middle + Vec3::X * 0.2),
        0.0,
        now,
    ));
    assert_eq!(projector.editing_wall(), Some("wall-1"));
    let output = projector.update(frame_at(&projection, at(HOME + up), 0.0, now));
    assert_eq!(bars(&output).len(), 2);
    assert!(label(&output).is_none());
}
