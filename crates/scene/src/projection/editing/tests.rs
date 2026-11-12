// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::arrival::HOME;
use crate::projection::{Input as Frame, Projector};
use crate::sides::Side;
use tomori_protocol::{Panel, Place, TOMORI_WALL};

const ADJUST: Rights = Rights {
    adjust: true,
    remove: false,
    snap: true,
    locks: false,
    lock: false,
};

const OWNER: Rights = Rights {
    adjust: true,
    remove: true,
    snap: true,
    locks: false,
    lock: false,
};

/// An added wall 2.5 m ahead of home, facing it, flat, three columns of two rows.
fn added() -> Wall {
    Wall::new(
        "wall-1",
        WallKind::Added,
        Pose::upright_facing([0.0, 1.5, -2.5], HOME.to_array()),
    )
    .with_version(7)
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
        free: false,
        controls: Controls::Controllers,
        scroll: None,
        rights,
        surfaces: None,
        left: crate::projection::Hold::default(),
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
    handle_point(wall, model(&wall.frame()).unwrap(), handle).0
}

/// Where a ray from home at `point` meets `wall`'s plane, in the scene: where a handle off the wall, such as a corner,
/// is taken hold of, as its drag is measured.
fn met(wall: &Wall, point: Vec3) -> Vec3 {
    let frame = model(&wall.frame()).unwrap();
    frame.transform_point3(on_plane(frame, &at(point).unwrap()).unwrap())
}

/// A point of `wall`'s board, `x` and `y` from its middle in its own frame, in the scene.
fn on(wall: &Wall, x: f32, y: f32) -> Vec3 {
    model(&wall.frame())
        .unwrap()
        .transform_point3(Vec3::new(x, y, 0.0))
}

/// The projector, holding `wall` taken hold of by a squeeze on its board's top edge, above its slots, or, curved, on its
/// first slot, empty, and let go.
fn holding(projection: &Projection, wall: &Wall, rights: Rights, now: Instant) -> Projector {
    let mut projector = Projector::default();
    let target = match wall.board() {
        Some((_, height)) => on(wall, 0.0, height / 2.0 - 0.005),
        None => model(&wall.frame())
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
        .and_then(|sheet| sheet.canvas.as_deref())
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
    let slot = model(&wall.frame())
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
    // Thirteen handles and the hint, after the wall's own sheets.
    let faces: Vec<String> = output
        .sheets
        .iter()
        .rev()
        .take(14)
        .filter_map(|sheet| sheet.canvas.as_deref().map(Canvas::plain))
        .collect();
    let hint = &faces[0];
    for row in [
        "more / fewer slots",
        "size and shape",
        "one fewer / more",
        "knob",
        "↔ bend",
        "hinge",
        "↕ tilt",
        "aA",
        "↔ text size",
        "carry it",
        "done",
    ] {
        assert!(hint.contains(row), "{row}: {hint}");
    }
    assert_eq!(handles(&wall, LockControls::default()).len(), 13);
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
    // The Tomori wall has no corners or rows to change, its slots its own size in its one row, but its text handle.
    let tomori = tomori();
    assert_eq!(handles(&tomori, LockControls::default()).len(), 6);
    assert!(handles(&tomori, LockControls::default()).contains(&Handle::Text));
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
    assert!(
        keys.contains("G knob")
            && keys.contains("↔ bend")
            && keys.contains("G hinge")
            && keys.contains("↕ tilt"),
        "{keys}"
    );
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
    // The right edge dragged out as far as a fourth column's edge would stand beyond it, and the bottom edge up to
    // where one row's would, each moved from where its handle was taken hold of.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let four = Wall::new("w", WallKind::Added, wall.pose).with_grid(Grid::new(4, 2).unwrap());
    let ((width, height), (three, _)) = (four.board().unwrap(), wall.board().unwrap());
    let change = drag(
        &mut projector,
        &projection,
        &wall,
        Handle::Columns,
        handle(&wall, Handle::Columns) + Vec3::X * (width - three) / 2.0,
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
        handle(&wall, Handle::Rows) + Vec3::Y * (height / 2.0 - 0.1),
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
    // A corner dragged a little over a fifth as far again from the middle makes the slots half as large again, as the
    // square of how far it moved, in steps of 5%.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let corner = met(&wall, handle(&wall, Handle::Corner(1, 1)));
    let middle = Vec3::from_array(wall.pose.position);
    let change = drag(
        &mut projector,
        &projection,
        &wall,
        Handle::Corner(1, 1),
        middle + (corner - middle) * 1.5f32.sqrt(),
        now,
    )
    .expect("larger slots");
    let scale = change.scale.unwrap();
    assert!((scale - 1.5).abs() < 0.051, "{scale}");
    assert!(((scale / SCALE_STEP).round() * SCALE_STEP - scale).abs() < 1e-5);
    // From a standard slot to the largest, five times as large, in one drag to a little over twice as far, and never
    // beyond it.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let change = drag(
        &mut projector,
        &projection,
        &wall,
        Handle::Corner(1, 1),
        middle + (corner - middle) * 5f32.sqrt(),
        now,
    )
    .expect("the largest slots");
    assert_eq!(change.scale, Some(layout::SCALES.1));
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let change = drag(
        &mut projector,
        &projection,
        &wall,
        Handle::Corner(1, 1),
        middle + (corner - middle) * 3.0,
        now,
    );
    assert!(change.is_none_or(|change| change.scale.is_none_or(|scale| scale <= layout::SCALES.1)));
    // And back from the largest to a standard slot in one drag back.
    let large = wall.clone().with_scale(layout::SCALES.1);
    let large_projection = shown(vec![large.clone()], Vec::new());
    let mut projector = holding(&large_projection, &large, ADJUST, now);
    let corner = met(&large, handle(&large, Handle::Corner(1, 1)));
    let change = drag(
        &mut projector,
        &large_projection,
        &large,
        Handle::Corner(1, 1),
        middle + (corner - middle) * 0.2f32.sqrt(),
        now,
    )
    .expect("standard slots");
    assert!((change.scale.unwrap() - 1.0).abs() < 0.051, "{change:?}");
    // Dragged down alone, its slots grow taller, not wider, their text its size: nearly three times as tall snaps to
    // 9:16, twice as tall is its own shape, and wider alone makes them wide, each as the square of how far it moved.
    let stretched = |across: f32, down: f32| {
        let mut projector = holding(&projection, &wall, ADJUST, now);
        // Where the ray at the corner meets the wall's plane, as its drag is measured.
        let corner = met(&wall, handle(&wall, Handle::Corner(1, -1)));
        let local = model(&wall.frame())
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
    let tall = stretched(1.0, 2.95f32.sqrt());
    assert_eq!(
        (tall.aspect, tall.scale),
        (Some(9.0 / 16.0), None),
        "{tall:?}"
    );
    let taller = stretched(1.0, 2f32.sqrt());
    assert_eq!(taller.aspect, Some(0.84));
    let wide = stretched(1.9f32.sqrt(), 1.0);
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

/// Take hold of `wall`'s `handle`, the knob or the hinge, which `projector` holds, with the hand at home, move the hand
/// by each of `moves` in turn, and let go: the wall drawn after each move, whether each ticked, and what is sent.
fn move_hand(
    projector: &mut Projector,
    projection: &Projection,
    wall: &Wall,
    handle: Handle,
    moves: &[Vec3],
    controls: Controls,
    now: Instant,
) -> (Vec<Wall>, Vec<bool>, Option<Change>) {
    let point = self::handle(wall, handle);
    let from = at(point);
    let step = |projector: &mut Projector, ray: Option<Ray>, grip: f32| {
        let mut frame = frame(projection, ray, grip, ADJUST, now);
        frame.controls = controls;
        projector.update(frame)
    };
    step(projector, from, 0.0);
    step(projector, from, 1.0);
    assert_eq!(
        projector.editor.drag.as_ref().map(|drag| drag.handle),
        Some(handle)
    );
    let mut drawn = Vec::new();
    let mut ticks = Vec::new();
    let mut ray = from;
    for moved in moves {
        // The hand moved, still aiming about where it took hold.
        ray = match controls {
            Controls::Controllers => Ray::new(HOME + *moved, point - HOME),
            Controls::Keyboard => Ray::new(HOME, point + *moved - HOME),
        };
        let output = step(projector, ray, 1.0);
        drawn.push(projector.editor.walls(projection)[0].clone());
        ticks.push(output.wall_ticked);
        // While it is dragged, the ray ends on the handle, and what it makes of the wall shows in place of the hint.
        assert!(output.hit.is_some_and(|hit| hit.held));
    }
    (drawn, ticks, step(projector, ray, 0.0).wall)
}

/// Take hold of `wall`'s knob, which `projector` holds, move the hand by each of `moves` and let go: the radius drawn
/// after each move, and what is sent.
fn bend(
    projector: &mut Projector,
    projection: &Projection,
    wall: &Wall,
    moves: &[Vec3],
    controls: Controls,
    now: Instant,
) -> (Vec<Option<f32>>, Option<Change>) {
    let (drawn, _, sent) = move_hand(
        projector,
        projection,
        wall,
        Handle::Bend,
        moves,
        controls,
        now,
    );
    (drawn.iter().map(Wall::radius).collect(), sent)
}

/// The curvature of each of `radii`, flat as 0.
fn curvatures(radii: &[Option<f32>]) -> Vec<f32> {
    radii
        .iter()
        .map(|radius| radius.map_or(0.0, |radius| 1.0 / radius))
        .collect()
}

#[test]
fn the_ease_is_fine_at_first_quicker_at_the_end_even_either_way_and_undone_exactly() {
    assert_eq!((ease(0.0), ease(1.0), ease(-1.0)), (0.0, 1.0, -1.0));
    // Half as fast as evenly at first, and half as fast again at the end.
    assert!((ease(0.01) / 0.01 - 0.5).abs() < 0.01);
    assert!(((ease(1.0) - ease(0.99)) / 0.01 - 1.5).abs() < 0.01);
    let steps: Vec<f32> = (-20..=20).map(|step| ease(step as f32 / 20.0)).collect();
    assert!(steps.windows(2).all(|pair| pair[1] > pair[0]), "{steps:?}");
    for step in -20..=20 {
        let along = step as f32 / 20.0;
        assert!((unease(ease(along)) - along).abs() < 1e-5, "{along}");
        assert_eq!(ease(-along), -ease(along));
    }
}

#[test]
fn one_movement_across_of_a_fixed_travel_bends_a_wall_fully_and_back_flattens_it() {
    let wall = added();
    let projection = shown(vec![wall.clone()], Vec::new());
    let now = Instant::now();
    // Facing the wall, the participant's right is +X.
    let full = TRAVEL + DEAD_ZONE;
    // Moved to the right step by step, it bends more at every step, from flat to as tightly as it may, in one movement
    // of the travel, finely at first.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let moves: Vec<Vec3> = (1..=20)
        .map(|step| Vec3::X * full * step as f32 / 20.0)
        .collect();
    let (drawn, ticks, sent) = move_hand(
        &mut projector,
        &projection,
        &wall,
        Handle::Bend,
        &moves,
        Controls::Controllers,
        now,
    );
    let radii: Vec<Option<f32>> = drawn.iter().map(Wall::radius).collect();
    let curvature = curvatures(&radii);
    assert!(
        curvature.windows(2).all(|pair| pair[1] >= pair[0] - 1e-4),
        "{curvature:?}"
    );
    assert_eq!(radii[0], None, "a little is flat");
    let most = most_bend(&wall);
    assert_eq!(radii[19], bent(&wall, 1e3), "{curvature:?} to {most}");
    assert!((curvature[19] - most).abs() < 0.01, "{curvature:?}");
    // Eased: halfway across, less than halfway bent, as the ease says.
    let halfway = ease((full / 2.0 - DEAD_ZONE) / TRAVEL);
    assert!(
        (curvature[9] - most * halfway).abs() < 0.03 && curvature[9] < most / 2.0,
        "{curvature:?}"
    );
    // Reaching the end ticks, as a stop, once.
    assert_eq!(ticks.iter().filter(|tick| **tick).count(), 1, "{ticks:?}");
    assert!(ticks[19]);
    let Some(WallShape::Arc { radius }) = sent.as_ref().and_then(|change| change.shape) else {
        unreachable!("an arc, not {sent:?}");
    };
    assert!(radius >= layout::LEAST_RADIUS && Some(radius) == radii[19]);
    // Fully bent, one movement back to the left flattens it, less at every step.
    let tightest = wall.clone().with_shape(WallShape::Arc { radius });
    let projection = shown(vec![tightest.clone()], Vec::new());
    let mut projector = holding(&projection, &tightest, ADJUST, now);
    let moves: Vec<Vec3> = (1..=20)
        .map(|step| -Vec3::X * full * step as f32 / 20.0)
        .collect();
    let (radii, sent) = bend(
        &mut projector,
        &projection,
        &tightest,
        &moves,
        Controls::Controllers,
        now,
    );
    let curvature = curvatures(&radii);
    assert!(
        curvature.windows(2).all(|pair| pair[1] <= pair[0] + 1e-4),
        "{curvature:?}"
    );
    assert_eq!(radii[19], None);
    assert_eq!(sent.and_then(|change| change.shape), Some(WallShape::Grid));
    // Back where it took hold, it is as it was, and nothing is sent; on the way, it flattens as it bent.
    let projection = shown(vec![wall.clone()], Vec::new());
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let there_and_back = [Vec3::X * 0.2, Vec3::X * 0.1, Vec3::X * 0.005];
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
    // A curved wall: back to where it took hold, its radius exactly as it was; moved left, flatter, then flat, and no
    // further.
    let curved = wall.clone().with_shape(WallShape::Arc { radius: 3.0 });
    let projection = shown(vec![curved.clone()], Vec::new());
    let mut projector = holding(&projection, &curved, ADJUST, now);
    assert_eq!(projector.editing_wall(), Some("wall-1"));
    let moves = [
        Vec3::X * 0.05,
        Vec3::ZERO,
        -Vec3::X * 0.03,
        -Vec3::X * 0.2,
        -Vec3::X * 0.5,
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
    // Within the dead zone either way, it is as it was; a little beyond it, it snaps flat.
    assert_eq!(bent(&curved, 0.0), Some(3.0));
    assert_eq!(bent(&curved, DEAD_ZONE * 0.9), Some(3.0));
    assert_eq!(bent(&curved, -DEAD_ZONE * 0.9), Some(3.0));
    assert_eq!(bent(&wall, f32::NAN), None);
    assert_eq!(bent(&wall, DEAD_ZONE + 0.02), None);
    assert!(bent(&wall, DEAD_ZONE + 0.05).is_some());
    // Of every size, the same movement takes it from flat to as tightly as it may bend and back.
    for wall in walls_of_every_size() {
        let flat = wall.clone().with_shape(WallShape::Grid);
        let tightest = bent(&flat, 1e3).expect("every wall bends");
        let Some(full) = bent(&flat, TRAVEL + DEAD_ZONE) else {
            unreachable!("{:?} bends", flat.grid());
        };
        // To the centimetre the radius is rounded to.
        assert!((full - tightest).abs() <= 0.011, "{full} {tightest}");
        let curved = flat.with_shape(WallShape::Arc { radius: tightest });
        assert_eq!(bent(&curved, -(TRAVEL + DEAD_ZONE)), None);
    }
}

#[test]
fn the_knob_follows_the_hand_across_as_the_participant_sees_it_and_not_up_down_or_towards_the_wall()
{
    let wall = added();
    let projection = shown(vec![wall.clone()], Vec::new());
    let now = Instant::now();
    let towards = model(&wall.frame()).unwrap().transform_vector3(Vec3::Z);
    // Moving up or down, or towards the wall, however far, and across within the dead zone, bends nothing.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let jitter = [
        towards * 0.3,
        -towards * 0.3,
        Vec3::Y * 0.4,
        -Vec3::Y * 0.4,
        Vec3::X * 0.008 - Vec3::Y * 0.2,
        -Vec3::X * 0.009 + towards * 0.1,
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
    // Moved to the right with drift up and down and towards the wall, it bends as far as without; to the left of flat,
    // it stays flat.
    let mut steady = holding(&projection, &wall, ADJUST, now);
    let (still, _) = bend(
        &mut steady,
        &projection,
        &wall,
        &[Vec3::X * 0.15],
        Controls::Controllers,
        now,
    );
    let mut shaky = holding(&projection, &wall, ADJUST, now);
    let (shaken, _) = bend(
        &mut shaky,
        &projection,
        &wall,
        &[Vec3::X * 0.15 + towards * 0.04 - Vec3::Y * 0.1],
        Controls::Controllers,
        now,
    );
    assert_eq!(still, shaken);
    assert!(still[0].is_some());
    let mut leftward = holding(&projection, &wall, ADJUST, now);
    let (drawn, sent) = bend(
        &mut leftward,
        &projection,
        &wall,
        &[-Vec3::X * 0.2],
        Controls::Controllers,
        now,
    );
    assert_eq!((drawn[0], sent), (None, None));
    // A wall to the participant's left, seen turned towards it: their right is forwards, which bends it, and
    // towards the wall, to their left, does nothing.
    let aside = Wall::new(
        "wall-1",
        WallKind::Added,
        Pose::upright_facing([-2.5, 1.5, 0.0], HOME.to_array()),
    );
    let projection = shown(vec![aside.clone()], Vec::new());
    let mut projector = holding(&projection, &aside, ADJUST, now);
    let (drawn, _) = bend(
        &mut projector,
        &projection,
        &aside,
        &[Vec3::X * 0.2, -Vec3::Z * 0.15],
        Controls::Controllers,
        now,
    );
    assert!(drawn[0].is_none() && drawn[1].is_some(), "{drawn:?}");
    // In the desktop window, whose ray begins at the eyes, dragging the knob right bends it, and back flattens it.
    let projection = shown(vec![wall.clone()], Vec::new());
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let (drawn, sent) = bend(
        &mut projector,
        &projection,
        &wall,
        &[Vec3::X * 0.15, Vec3::ZERO],
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
fn the_hinge_moved_down_tilts_a_wall_towards_the_participant_within_the_travel_and_snaps_upright() {
    let wall = added();
    let projection = shown(vec![wall.clone()], Vec::new());
    let now = Instant::now();
    let full = TRAVEL + DEAD_ZONE;
    // Moved down step by step through half the travel, it tilts its top towards the participant more at every step,
    // from upright to the most it may, snapping upright at first, finely at first.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let half = DEAD_ZONE + TRAVEL / 2.0;
    let moves: Vec<Vec3> = (1..=20)
        .map(|step| -Vec3::Y * half * step as f32 / 20.0)
        .collect();
    let (drawn, ticks, sent) = move_hand(
        &mut projector,
        &projection,
        &wall,
        Handle::Tilt,
        &moves,
        Controls::Controllers,
        now,
    );
    let tilts: Vec<f32> = drawn.iter().map(Wall::tilt).collect();
    assert!(tilts.windows(2).all(|pair| pair[1] >= pair[0]), "{tilts:?}");
    assert_eq!(tilts[0], 0.0, "a little is upright");
    assert_eq!(tilts[19], layout::MOST_TILT);
    // Eased: halfway down, less than halfway, as the ease says.
    let halfway = layout::MOST_TILT * ease(2.0 * (half / 2.0 - DEAD_ZONE) / TRAVEL);
    assert!(
        (tilts[9] - halfway).abs() <= 1.0 && tilts[9] < layout::MOST_TILT / 2.0,
        "{tilts:?}"
    );
    assert!(tilts.iter().all(|tilt| tilt.fract() == 0.0));
    // Tilted, its top comes towards the participant, and its face looks down.
    let standing = model(&drawn[19].frame()).unwrap();
    assert!(standing.transform_vector3(Vec3::Z).y < -0.7);
    assert!(standing.transform_vector3(Vec3::Y).z > 0.7);
    // Reaching the most ticks, as a stop, and so does leaving upright, never in between.
    assert!(
        ticks[19] && ticks.iter().filter(|tick| **tick).count() == 1,
        "{ticks:?}"
    );
    assert_eq!(
        sent,
        Some(Change {
            wall: "wall-1".into(),
            grid: None,
            scale: None,
            aspect: None,
            shape: None,
            pose: None,
            tilt: Some(layout::MOST_TILT),
            content_scale: None,
            join: None,
            // Made against the wall as its hinge was taken hold of, at its version then.
            version: Some(7),
        })
    );
    // From the most it tilts down, one movement up of the travel takes it to the most it tilts up, upright on the way,
    // which ticks.
    let down = wall.clone().with_tilt(Some(layout::MOST_TILT));
    let projection = shown(vec![down.clone()], Vec::new());
    let mut projector = holding(&projection, &down, ADJUST, now);
    let moves: Vec<Vec3> = (1..=40)
        .map(|step| Vec3::Y * full * step as f32 / 40.0)
        .collect();
    let (drawn, ticks, sent) = move_hand(
        &mut projector,
        &projection,
        &down,
        Handle::Tilt,
        &moves,
        Controls::Controllers,
        now,
    );
    let tilts: Vec<f32> = drawn.iter().map(Wall::tilt).collect();
    assert!(tilts.windows(2).all(|pair| pair[1] <= pair[0]), "{tilts:?}");
    assert_eq!(tilts[39], -layout::MOST_TILT);
    let upright = tilts.iter().position(|tilt| *tilt == 0.0).unwrap();
    assert!(ticks[upright] && ticks[39], "{ticks:?}");
    assert_eq!(
        sent.and_then(|change| change.tilt),
        Some(-layout::MOST_TILT)
    );
    // Back where it took hold, within the dead zone, it is as it was and nothing is sent; near upright, it is upright.
    let projection = shown(vec![wall.clone()], Vec::new());
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let (drawn, _, sent) = move_hand(
        &mut projector,
        &projection,
        &wall,
        Handle::Tilt,
        &[-Vec3::Y * 0.1, Vec3::Y * 0.004],
        Controls::Controllers,
        now,
    );
    assert!(drawn[0].tilt() > 10.0 && drawn[1].tilt() == 0.0 && sent.is_none());
    assert_eq!(tilted(&wall, DEAD_ZONE * 0.9), 0.0);
    // Just short of where the ease reaches the snap, upright.
    let snap = unease(UPRIGHT_SNAP / layout::MOST_TILT) * TRAVEL / 2.0;
    assert_eq!(tilted(&wall, DEAD_ZONE + snap * 0.9), 0.0);
    assert!(tilted(&wall, DEAD_ZONE + snap * 1.2) >= UPRIGHT_SNAP);
    assert_eq!(tilted(&wall, f32::NAN), 0.0);
    let ten = wall.clone().with_tilt(Some(10.0));
    assert_eq!(tilted(&ten, 0.005), 10.0);
    assert!(tilted(&ten, 0.05) > 10.0 && tilted(&ten, -0.05) < 10.0);
    // In the desktop window, dragging it down tilts it down too.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let (drawn, _, _) = move_hand(
        &mut projector,
        &projection,
        &wall,
        Handle::Tilt,
        &[-Vec3::Y * 0.1],
        Controls::Keyboard,
        now,
    );
    assert!(drawn[0].tilt() > 10.0);
    // Moving across or towards the wall tilts nothing, and moving down with drift across tilts it as far as without.
    let towards = model(&wall.frame()).unwrap().transform_vector3(Vec3::Z);
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let (drawn, _, sent) = move_hand(
        &mut projector,
        &projection,
        &wall,
        Handle::Tilt,
        &[towards * 0.3, Vec3::X * 0.3, -Vec3::X * 0.3],
        Controls::Controllers,
        now,
    );
    assert!(drawn.iter().all(|wall| wall.tilt() == 0.0) && sent.is_none());
    let tilt_by = |moved: Vec3| {
        let mut projector = holding(&projection, &wall, ADJUST, now);
        move_hand(
            &mut projector,
            &projection,
            &wall,
            Handle::Tilt,
            &[moved],
            Controls::Controllers,
            now,
        )
        .0[0]
            .tilt()
    };
    assert_eq!(
        tilt_by(-Vec3::Y * 0.08),
        tilt_by(-Vec3::Y * 0.08 + Vec3::X * 0.1)
    );
    // Its readout says how it tilts.
    assert_eq!(readout(Handle::Tilt, &wall).plain().trim(), "upright");
    assert_eq!(
        readout(Handle::Tilt, &ten).plain().trim(),
        "tilted 10° down"
    );
    assert_eq!(
        readout(Handle::Tilt, &wall.clone().with_tilt(Some(-20.0)))
            .plain()
            .trim(),
        "tilted 20° up"
    );
}

#[test]
fn the_hinge_stands_on_the_line_a_flat_wall_tilts_about_and_a_tilted_wall_is_held_drawn_and_carried_tilted()
 {
    let now = Instant::now();
    // Flat, its hinge stays where it was as the wall tilts, so that the ray stays on it.
    let wall = added();
    let before = handle(&wall, Handle::Tilt);
    for tilt in [-45.0, -20.0, 30.0, 45.0] {
        let tilted = wall.clone().with_tilt(Some(tilt));
        assert!(
            handle(&tilted, Handle::Tilt).distance(before) < 0.02,
            "{tilt}"
        );
    }
    // A tilted wall is taken hold of where it is drawn, its handles about it, and carried tilted, its pose sent upright
    // and its tilt kept.
    let tilted = wall.clone().with_tilt(Some(30.0));
    let projection = shown(vec![tilted.clone()], Vec::new());
    let mut projector = holding(&projection, &tilted, ADJUST, now);
    assert_eq!(projector.editing_wall(), Some("wall-1"));
    let ray = at(on(&tilted, 0.3, 0.2));
    projector.update(frame(&projection, ray, 1.0, ADJUST, now));
    let moved = Ray::new(HOME, on(&tilted, 0.3, 0.2) + Vec3::X * 0.5 - HOME);
    projector.update(frame(&projection, moved, 1.0, ADJUST, now));
    let shown = projector.editor.walls(&projection)[0].clone();
    assert_eq!(shown.tilt(), 30.0);
    let [x, _, z, _] = shown.pose.orientation;
    assert!(x.abs() < 1e-4 && z.abs() < 1e-4, "{:?}", shown.pose);
    let sent = projector
        .update(frame(&projection, moved, 0.0, ADJUST, now))
        .wall
        .expect("moved");
    assert_eq!(sent.tilt, None);
    let pose = sent.pose.expect("a pose");
    assert!((pose.position[0] - 0.5).abs() < 0.05, "{pose:?}");
    let [x, _, z, _] = pose.orientation;
    assert!(x.abs() < 1e-4 && z.abs() < 1e-4, "{pose:?}");
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
        .and_then(|sheet| sheet.canvas.as_deref())
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
    let frame = model(&wall.frame()).unwrap();
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
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let (_, sent) = bend(
        &mut projector,
        &projection,
        &wall,
        &[Vec3::X * 0.15],
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
    // Refused by the server, as made against a version since changed, it is drawn as the server has it at once; a
    // refusal of another wall changes nothing.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    bend(
        &mut projector,
        &projection,
        &wall,
        &[Vec3::X * 0.15],
        Controls::Controllers,
        now,
    );
    projector.refused_wall("wall-99");
    assert!(projector.editor.pending.is_some());
    projector.refused_wall(&wall.id);
    assert!(projector.editor.pending.is_none());
    projector.update(frame_at(
        &projection,
        None,
        0.0,
        now + Duration::from_secs(1),
    ));
    assert_eq!(projector.editor.walls(&projection)[0], wall);
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
    let frame = model(&behind.frame()).unwrap();
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

/// Whether `viewer` stands in front of every slot of `wall`, as a participant editing it stands, not behind the end of
/// a wall curved around them.
fn before(wall: &Wall, viewer: Vec3) -> bool {
    let frame = model(&wall.frame()).unwrap();
    crate::projection::slot_frames(wall, frame)
        .iter()
        .all(|slot| {
            (viewer - slot.transform_point3(Vec3::ZERO)).dot(slot.transform_vector3(Vec3::Z)) > 0.0
        })
}

/// The walls whose handles are checked: every grid's extremes, sizes and shapes of slot, flat, curved and curved as
/// tightly as they may bend, the largest a big wall's, and tilted either way as far as they tilt.
fn walls_of_every_size() -> Vec<Wall> {
    let mut walls = Vec::new();
    for (columns, rows) in [(1, 1), (2, 1), (3, 2), (4, 1), (8, 3), (6, 4), (1, 4)] {
        for scale in [0.5, 1.0, 2.0, layout::SCALES.1] {
            for aspect in [None, Some(0.25), Some(0.5625), Some(4.0)] {
                let flat = added()
                    .with_grid(Grid::new(columns, rows).unwrap())
                    .with_scale(scale)
                    .with_aspect(aspect);
                // And curved as tightly as it may bend.
                let tightest = bent(&flat, 1e3);
                for radius in [None, Some(1.5), Some(3.0), tightest] {
                    let wall = flat.clone().with_shape(
                        radius.map_or(WallShape::Grid, |radius| WallShape::Arc { radius }),
                    );
                    if allowed(&wall) {
                        walls.push(wall);
                    }
                }
            }
        }
    }
    let tilted: Vec<Wall> = walls
        .iter()
        .filter(|wall| wall.aspect.is_none() && [1, 3, 4].contains(&wall.grid().columns))
        .flat_map(|wall| {
            [layout::MOST_TILT, -layout::MOST_TILT, 20.0]
                .map(|tilt| wall.clone().with_tilt(Some(tilt)))
        })
        .collect();
    walls.extend(tilted);
    walls.push(tomori());
    walls.push(tomori().with_grid(Grid::new(1, 1).unwrap()));
    walls.push(tomori().with_tilt(Some(-30.0)));
    walls
}

/// Each handle of `wall`, with the lock's controls too, as it stands where the wall says: its face's model, unscaled,
/// and how large handles are.
fn faces_of(wall: &Wall) -> (Vec<(Handle, Mat4)>, f32) {
    let lock = LockControls {
        padlock: true,
        reset: true,
    };
    handle_models(wall, model(&wall.frame()).unwrap(), &[], None, lock)
}

/// Where `point` stands from the face at `model`, across it and up it, and how far in front of it.
fn from_face(model: Mat4, point: Vec3) -> Vec3 {
    model.inverse().transform_point3(point)
}

#[test]
fn no_two_handles_overlap_nor_the_bar_nor_the_wall_s_slots_at_any_size_or_shape() {
    let walls = walls_of_every_size();
    assert!(walls.len() > 100, "{}", walls.len());
    let mut checked = 0;
    for wall in &walls {
        let frame = model(&wall.frame()).unwrap();
        let (faces, size) = faces_of(wall);
        // Some left out where they would stand in the floor, below a large wall tilted far.
        assert!(!faces.is_empty(), "{wall:?}");
        assert!((size - handle_size(wall)).abs() < 1e-6);
        let bar = bar(wall, frame);
        for (index, (handle, model)) in faces.iter().enumerate() {
            // Each faces as the wall faces where it stands, upright on it.
            let facing = model.transform_vector3(Vec3::Z);
            let up = model.transform_vector3(Vec3::Y);
            assert!((up - frame.transform_vector3(Vec3::Y)).length() < 1e-4);
            assert!(facing.dot(frame.transform_vector3(Vec3::Z)) > 0.0);
            // Its middle at least the spacing from every other's, across or up its face, so that neither their faces
            // nor where the ray meets them overlap, from wherever they are seen.
            for (other, at) in &faces[index + 1..] {
                let off = from_face(*model, at.w_axis.truncate());
                checked += 1;
                assert!(
                    off.x.abs().max(off.y.abs()) >= SPACING * size * 0.99
                        && (wall.radius().is_some() || off.z.abs() < 1e-4),
                    "{handle:?} and {other:?} of {:?} {:?} {}°: {off}",
                    wall.grid(),
                    (wall.scale(), wall.aspect(), wall.radius()),
                    wall.tilt()
                );
            }
            // Where the ray meets it clear of where it meets the bar: as far from the bar's middle line, up or down, as
            // both reach, or beyond its end.
            // On a curved wall, those along the arc beside the bar, behind its plane, are as far along the arc.
            let off = from_face(bar.model, model.w_axis.truncate());
            let reach = HANDLE_REACH * 2.0 * size;
            assert!(
                off.y.abs() >= reach * 0.99
                    || off.x.abs() >= bar.half.length() + reach * 0.99
                    || (wall.radius().is_some() && off.z.abs() > size),
                "{handle:?} of {:?} {:?} by the bar: {off}",
                wall.grid(),
                (wall.scale(), wall.aspect(), wall.radius())
            );
            // Its face, and the bar's, over none of the wall's slots.
            for slot in crate::projection::slot_frames(wall, frame) {
                // Its place, unscaled by the size of its text.
                let (_, turn, middle) = slot.to_scale_rotation_translation();
                let slot = Mat4::from_rotation_translation(turn, middle);
                let (width, height) = wall.slot_size();
                let off = from_face(slot, model.w_axis.truncate());
                // Across the arc from a slot of a wall curved more than half around is not over it.
                assert!(
                    off.z.abs() > size
                        || off.x.abs() >= (width + size) / 2.0 - 1e-4
                        || off.y.abs() >= (height + size) / 2.0 - 1e-4,
                    "{handle:?} over a slot of {:?} {:?}: {off}",
                    wall.grid(),
                    (wall.scale(), wall.aspect(), wall.radius())
                );
                let off = from_face(slot, bar.middle);
                assert!(off.y.abs() >= (height + BAR_THICKNESS * size) / 2.0 - 1e-4);
            }
        }
    }
    assert!(checked > 10_000, "{checked}");
}

#[test]
fn each_handle_stands_at_its_fixed_place_about_the_wall() {
    // A flat wall of three columns of two rows, at scale 1: its handles on a frame OUTSET handles beyond its board.
    let wall = added();
    let (width, height) = wall.size();
    let size = handle_size(&wall);
    assert!((size - HANDLE_SIZE).abs() < 1e-6);
    let (pitch, out) = (SPACING * size, OUTSET * size);
    let (side, top, bottom) = (width / 2.0 + out, height / 2.0 + out, -(height / 2.0 + out));
    let half = (width / 6.0).clamp(1.5 * size, 2.5 * size);
    let local = |shown: Handle| {
        let at = handle(&wall, shown);
        model(&wall.frame()).unwrap().inverse().transform_point3(at)
    };
    let table = [
        (Handle::Corner(-1, 1), Vec2::new(-side, top)),
        (Handle::Corner(1, 1), Vec2::new(side, top)),
        (Handle::Corner(-1, -1), Vec2::new(-side, bottom)),
        (Handle::Corner(1, -1), Vec2::new(side, bottom)),
        (Handle::Bend, Vec2::new(0.0, top)),
        (Handle::Columns, Vec2::new(side, 0.0)),
        (
            Handle::Step(Axis::Columns, -1),
            Vec2::new(side + pitch, 0.0),
        ),
        (
            Handle::Step(Axis::Columns, 1),
            Vec2::new(side + 2.0 * pitch, 0.0),
        ),
        (Handle::Tilt, Vec2::new(-side, 0.0)),
        (Handle::Text, Vec2::new(-(half + pitch), bottom)),
        (
            Handle::Step(Axis::Rows, -1),
            Vec2::new(half + pitch, bottom),
        ),
        (Handle::Rows, Vec2::new(half + 2.0 * pitch, bottom)),
        (
            Handle::Step(Axis::Rows, 1),
            Vec2::new(half + 3.0 * pitch, bottom),
        ),
    ];
    for (handle, expected) in table {
        let at = local(handle);
        assert!(
            (Vec2::new(at.x, at.y) - expected).length() < 1e-4 && at.z.abs() < 1e-4,
            "{handle:?}: {at} not {expected}"
        );
    }
    // The bar centred below its bottom edge, level with its bottom corners, a third of it long within bounds.
    let bar = bar_of(&wall);
    let middle = model(&wall.frame())
        .unwrap()
        .inverse()
        .transform_point3(bar.middle);
    assert!(
        (middle - Vec3::new(0.0, bottom, 0.0)).length() < 1e-4,
        "{middle}"
    );
    assert!((bar.half.length() - half).abs() < 1e-4);
    // The padlock a row beyond its top left corner, and the control that puts it back beside it.
    let (faces, _) = handle_models(
        &wall,
        model(&wall.frame()).unwrap(),
        &[],
        None,
        LockControls {
            padlock: true,
            reset: true,
        },
    );
    let point = |handle: Handle| {
        let at = faces
            .iter()
            .find(|(shown, _)| *shown == handle)
            .unwrap()
            .1
            .w_axis;
        model(&wall.frame())
            .unwrap()
            .inverse()
            .transform_point3(at.truncate())
    };
    assert!((point(Handle::Lock) - Vec3::new(-side, top + pitch, 0.0)).length() < 1e-4);
    assert!((point(Handle::Reset) - Vec3::new(pitch - side, top + pitch, 0.0)).length() < 1e-4);
    // Handles grow with the wall's slots, from a standard slot's, within bounds.
    for (scale, expected) in [
        (0.5, HANDLE_SIZES.0),
        (2.0, 2.0 * HANDLE_SIZE),
        (5.0, HANDLE_SIZES.1),
    ] {
        assert!((handle_size(&added().with_scale(scale)) - expected).abs() < 1e-6);
    }
    // A small wall's frame is as wide and tall as its handles need, so that none comes near another.
    let small = added().with_grid(Grid::new(1, 1).unwrap()).with_scale(0.5);
    let (faces, size) = faces_of(&small);
    let at = |handle: Handle| {
        let at = faces
            .iter()
            .find(|(shown, _)| *shown == handle)
            .unwrap()
            .1
            .w_axis;
        model(&small.frame())
            .unwrap()
            .inverse()
            .transform_point3(at.truncate())
    };
    assert!(at(Handle::Corner(1, 1)).x >= 2.0 * SPACING * size - 1e-5);
    // Its rows' handle and their steppers, with no room beside its bar, beyond its bottom corner, on the bar's row.
    let corner = at(Handle::Corner(1, -1));
    for (handle, beyond) in [
        (Handle::Step(Axis::Rows, -1), 1.0),
        (Handle::Rows, 2.0),
        (Handle::Step(Axis::Rows, 1), 3.0),
    ] {
        assert!(
            (at(handle) - corner - Vec3::X * beyond * SPACING * size).length() < 1e-4,
            "{handle:?}"
        );
    }
    assert!(at(Handle::Corner(1, 1)).y >= SPACING * size - 1e-5);
}

#[test]
fn nothing_a_wall_shows_stands_in_the_floor_its_bar_and_row_raised_above_a_low_wall() {
    let floor = tomori_protocol::entourage::FLOOR + FLOOR_CLEARANCE;
    let lock = LockControls {
        padlock: true,
        reset: true,
    };
    let low = |height: f32| {
        Wall::new(
            "wall-1",
            WallKind::Added,
            Pose::upright_facing([0.0, height, -2.5], [0.0, height, 0.0]),
        )
    };
    // The Tomori wall's bar, and the text handle beside it, stand below it, clear of the floor.
    let tomori = tomori();
    let frame = model(&tomori.frame()).unwrap();
    let bar = bar_of(&tomori);
    assert!(bar.middle.y - bar.size / 2.0 >= floor, "{}", bar.middle.y);
    let (points, size) = handle_points_with(&tomori, frame, lock);
    let text = points
        .iter()
        .find(|(handle, _)| *handle == Handle::Text)
        .unwrap()
        .1;
    assert!((text.y - bar.middle.y).abs() < 1e-4 && text.y < frame.transform_point3(Vec3::ZERO).y);
    assert!(points.iter().all(|(_, at)| at.y - size / 2.0 >= floor));
    // A wall standing low has its bar and the handles beside it a row above its top, the padlock a row above them,
    // and leaves out its bottom corners, which would be in the floor; one a little higher keeps its bar below it.
    for (height, raised) in [(0.45, true), (0.3, true), (1.5, false)] {
        let wall = low(height);
        let frame = model(&wall.frame()).unwrap();
        let bar = bar_of(&wall);
        let (points, size) = handle_points_with(&wall, frame, lock);
        let point = |shown: Handle| {
            points
                .iter()
                .find(|(handle, _)| *handle == shown)
                .map(|(_, at)| *at)
        };
        let top = point(Handle::Corner(-1, 1)).unwrap().y;
        assert_eq!(bar.middle.y > top, raised, "{height}");
        assert!(bar.middle.y - bar.size / 2.0 >= floor, "{height}");
        assert!(
            points.iter().all(|(_, at)| at.y - size / 2.0 >= floor),
            "{height}"
        );
        for handle in [Handle::Rows, Handle::Text, Handle::Step(Axis::Rows, 1)] {
            assert!(
                (point(handle).unwrap().y - bar.middle.y).abs() < 1e-4,
                "{handle:?}"
            );
        }
        if raised {
            assert!(point(Handle::Lock).unwrap().y > bar.middle.y);
            assert_eq!(point(Handle::Corner(1, -1)), None, "{height}");
        } else {
            assert!(point(Handle::Corner(1, -1)).is_some());
        }
        // No two overlap there either, nor the bar and a handle.
        let off = |at: Vec3| {
            frame.inverse().transform_point3(at) - frame.inverse().transform_point3(bar.middle)
        };
        for (index, (handle, at)) in points.iter().enumerate() {
            let from = off(*at);
            assert!(
                from.y.abs() >= 1.6 * size - 1e-4
                    || from.x.abs() >= bar.half.length() + 1.6 * size - 1e-4,
                "{handle:?} by the bar at {height}"
            );
            for (other, there) in &points[index + 1..] {
                let apart = frame.inverse().transform_point3(*there)
                    - frame.inverse().transform_point3(*at);
                assert!(
                    apart.x.abs().max(apart.y.abs()) >= SPACING * size * 0.99,
                    "{handle:?} {other:?}"
                );
            }
        }
    }
}

#[test]
fn a_curved_wall_s_handles_follow_its_slots_and_the_tomori_wall_s_bar_stands_below_its_label() {
    // Beside a curved wall's ends, along the slot at that end, as the wall would go on.
    let wall = added().with_shape(WallShape::Arc { radius: 1.5 });
    let frame = model(&wall.frame()).unwrap();
    let slots = crate::projection::slot_frames(&wall, frame);
    let last = slots[wall.grid().columns as usize - 1];
    for shown in [Handle::Columns, Handle::Corner(1, 1), Handle::Corner(1, -1)] {
        let off = from_face(last, handle(&wall, shown));
        assert!(
            off.z.abs() < 0.02 && off.x > wall.slot_size().0 / 2.0,
            "{shown:?}: {off}"
        );
    }
    // The knob above the middle of its arc, facing as its middle faces.
    let knob = from_face(frame, handle(&wall, Handle::Bend));
    assert!(
        knob.x.abs() < 1e-4 && knob.z.abs() < 0.05 && knob.y > wall.size().1 / 2.0,
        "{knob}"
    );
    // The Tomori wall's bar below its label, which stands below its slots, clear of it.
    let tomori = tomori();
    let bar = bar_of(&tomori);
    let below = from_face(model(&tomori.frame()).unwrap(), bar.middle);
    let label =
        crate::projection::ARC_LABEL_DROP + crate::projection::arc_label(&tomori).size().1 / 2.0;
    assert!(
        below.y <= -(tomori.size().1 / 2.0 + label) - BAR_THICKNESS * bar.size,
        "{below}"
    );
}

#[test]
fn handles_and_bars_stay_put_as_the_head_walks_and_turns_and_their_faces_are_drawn_once() {
    // A room: the Tomori wall, and three walls alone, one tilted and one curved.
    let lone = added();
    let tilted = Wall::new(
        "wall-2",
        WallKind::Added,
        Pose::upright_facing([2.0, 2.2, -1.5], HOME.to_array()),
    )
    .with_tilt(Some(-20.0));
    let curved = Wall::new(
        "wall-3",
        WallKind::Added,
        Pose::upright_facing([-2.5, 1.5, -1.0], HOME.to_array()),
    )
    .with_grid(Grid::new(4, 1).unwrap())
    .with_shape(WallShape::Arc { radius: 2.0 });
    let projection = shown(
        vec![tomori(), lone.clone(), tilted.clone(), curved.clone()],
        Vec::new(),
    );
    let now = Instant::now();
    for held in [&lone, &tilted, &curved] {
        let mut projector = holding(&projection, held, ADJUST, now);
        assert_eq!(projector.editing_wall(), Some(held.id.as_str()));
        // The handles' and bars' faces, as drawn, and where: every face but the hint, which turns to the eyes.
        let drawn = |projector: &mut Projector, viewer: Vec3, yaw: f32| {
            let mut input = frame(&projection, None, 0.0, ADJUST, now);
            input.viewer = Some(viewer);
            // Looking about, the ray at the sky, so that it meets nothing.
            input.ray = Ray::new(
                viewer,
                Quat::from_rotation_y(yaw) * Vec3::new(0.0, 1.0, -0.2),
            );
            let output = projector.update(input);
            output
                .sheets
                .into_iter()
                .filter_map(|sheet| Some((sheet.canvas?, sheet.model?)))
                .filter(|(canvas, _)| {
                    canvas.columns <= 1 && canvas.rows.len() <= 1 && canvas.padding.0 <= 8
                })
                .filter(|(canvas, _)| canvas.columns == 0 || canvas.padding == (4, 2))
                .collect::<Vec<_>>()
        };
        let first = drawn(&mut projector, HOME, 0.0);
        assert!(first.len() > 8, "{}", first.len());
        crate::frame_timing::take();
        for (step, yaw) in [-60f32, -30.0, 30.0, 60.0].into_iter().enumerate() {
            let viewer = HOME + Vec3::new(0.4 * step as f32 - 0.6, 0.0, 0.5);
            let again = drawn(&mut projector, viewer, yaw.to_radians());
            // No face drawn again but the hint, which turns to the eyes.
            assert_eq!(crate::frame_timing::take().faces, 1);
            assert_eq!(again.len(), first.len());
            for ((canvas, model), (kept, at)) in again.iter().zip(&first) {
                assert!(
                    model.abs_diff_eq(*at, 1e-6),
                    "moved from {viewer} at {yaw}°"
                );
                // The same face, drawn once and handed over again.
                assert!(std::sync::Arc::ptr_eq(canvas, kept));
            }
        }
    }
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
        let frame = model(&wall.frame()).unwrap();
        let (faces, size) = handle_models(&wall, frame, &[], None, LockControls::default());
        let mut projector = holding(&projection, &wall, ADJUST, now);
        assert_eq!(projector.editing_wall(), Some("wall-1"));
        for (handle, face) in &faces {
            let middle = &face.w_axis.truncate();
            let (right, up) = (face.x_axis.truncate(), face.y_axis.truncate());
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
    let slot = model(&wall.frame())
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
fn of_two_handles_the_ray_meets_the_nearer_is_taken_and_only_within_reach_of_its_face() {
    let ray = Ray::new(Vec3::ZERO, Vec3::NEG_Z).unwrap();
    let face = |at: Vec3| Mat4::from_translation(at);
    // Two handles one behind the other, both met: the nearer along the ray is taken.
    let faces = [
        (
            Handle::Step(Axis::Columns, -1),
            face(Vec3::new(0.0, 0.0, -1.2)),
        ),
        (
            Handle::Step(Axis::Columns, 1),
            face(Vec3::new(0.01, 0.0, -1.0)),
        ),
    ];
    let taken = nearest_handle(&faces, 0.05, &ray).unwrap();
    assert_eq!(taken.0, Handle::Step(Axis::Columns, 1));
    assert!((taken.1 - 1.0).abs() < 1e-5);
    // Within its reach across or up its face, as a square, and not beyond.
    let reach = 0.05 * HANDLE_REACH;
    for (off, met) in [
        (Vec3::new(reach * 0.99, reach * 0.99, -1.0), true),
        (Vec3::new(reach * 1.01, 0.0, -1.0), false),
        (Vec3::new(0.0, -reach * 1.01, -1.0), false),
    ] {
        let faces = [(Handle::Bend, face(-off.with_z(0.0) + Vec3::Z * off.z))];
        assert_eq!(nearest_handle(&faces, 0.05, &ray).is_some(), met, "{off}");
    }
    // Behind the ray, or with the ray along its face, none.
    assert_eq!(
        nearest_handle(&[(Handle::Bend, face(Vec3::Z))], 0.05, &ray),
        None
    );
    let edge_on = Mat4::from_rotation_translation(
        Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        Vec3::NEG_Z,
    );
    assert_eq!(nearest_handle(&[(Handle::Bend, edge_on)], 0.05, &ray), None);
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
    bar(wall, model(&wall.frame()).unwrap())
}

/// The bars drawn in `output`: faces without text, filled.
fn bars(output: &crate::projection::Output) -> Vec<&Canvas> {
    output
        .sheets
        .iter()
        .filter_map(|sheet| sheet.canvas.as_deref())
        .filter(|canvas| canvas.columns == 0 && canvas.rows.is_empty())
        .collect()
}

/// The viewers a wall is checked from: nearby to across the space, straight on and from either side, where the whole
/// wall is within view as its middle is looked at, as a participant would stand.
fn viewers(wall: &Wall) -> Vec<Vec3> {
    let frame = model(&wall.frame()).unwrap();
    let middle = Vec3::from_array(wall.pose.position);
    let facing = frame.transform_vector3(Vec3::Z);
    let mut viewers = Vec::new();
    for distance in [0.8, 1.5, 3.0, 6.0] {
        for side in [-50f32, -25.0, 0.0, 25.0, 50.0] {
            let viewer = middle + Quat::from_rotation_y(side.to_radians()) * facing * distance;
            let ahead = (middle - viewer).normalize();
            if before(wall, viewer)
                && corners(wall).iter().all(|corner| {
                    (frame.transform_point3(*corner) - viewer)
                        .normalize()
                        .dot(ahead)
                        >= 55f32.to_radians().cos()
                })
            {
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
        let frame = model(&wall.frame()).unwrap();
        for viewer in viewers(&wall) {
            checked += 1;
            let bar = bar(&wall, frame);
            let up = frame.transform_vector3(Vec3::Y);
            // Three to five handles long, and as large as its wall's handles.
            let length = 2.0 * bar.half.length();
            assert!(
                length >= BAR_LEAST * bar.size - 1e-5 && length <= BAR_MOST * bar.size + 1e-5,
                "{:?} {:?}: {length}",
                wall.grid(),
                (wall.scale(), wall.aspect(), wall.radius())
            );
            assert!((bar.size - handle_size(&wall)).abs() < 1e-6);
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
    // A ray along the bar, in its wall's plane, meets it nowhere; beyond its ends, by more than a handle's reach, none.
    let bar = bar_of(&added());
    assert_eq!(
        bar.hit(&Ray::new(bar.middle + bar.half * 3.0, -bar.half).unwrap()),
        None
    );
    let beyond = bar.middle + bar.half * 1.01 + bar.half.normalize() * bar.size * HANDLE_REACH;
    assert_eq!(bar.hit(&Ray::new(HOME, beyond - HOME).unwrap()), None);
}

#[test]
fn the_bar_stands_clear_of_every_handle_and_its_drawn_face_within_where_it_is_met() {
    let mut checked = 0;
    for wall in walls_of_every_size() {
        let frame = model(&wall.frame()).unwrap();
        for viewer in viewers(&wall) {
            checked += 1;
            let (faces, size) = handle_models(&wall, frame, &[], None, LockControls::default());
            let points: Vec<(Handle, Vec3)> = faces
                .iter()
                .map(|(handle, face)| (*handle, face.w_axis.truncate()))
                .collect();
            let bar = bar(&wall, frame);
            assert!((bar.size - size).abs() < 1e-6);
            // A ray at any handle's middle, or near its edge, meets no bar; and one anywhere along the bar meets no
            // handle, so that the two are never confused: on a flat wall, where they stand in one plane, from wherever
            // it is seen, and on a curved wall, whose handles turn as its slots do, from straight in front of it. Seen from far to the side of a wall curved tightly, a handle may come in line with the bar, and is then
            // taken first, as a handle always is.
            let facing = bar.model.transform_vector3(Vec3::Z).normalize();
            if wall.radius().is_some() && (viewer - bar.middle).normalize().dot(facing) < 0.97 {
                continue;
            }
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
                        nearest_handle(&faces, size, &ray),
                        None,
                        "{s} along the bar of {:?} {:?} from {viewer}",
                        wall.grid(),
                        (wall.scale(), wall.aspect(), wall.radius())
                    );
                }
            }
            // Drawn thin, as long as it is, within where the ray meets it.
            let (key, scale) = bar_key(&bar, false);
            let face = bar_face(key).size();
            let face = (face.0 * scale, face.1 * scale);
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
    let slot = model(&wall.frame())
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
            .as_deref()
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
    let below = model(&tomori.frame()).unwrap().transform_point3(Vec3::new(
        0.0,
        -tomori.size().1 / 2.0,
        0.0,
    ));
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
                    .as_deref()
                    .map(|canvas| (sheet.model, canvas.plain()))
            })
            .find(|(_, text)| text.contains("edit this wall"))
    };
    let (model, text) = label(&output).expect("what the grip does");
    assert!(
        text.contains("Grip") && text.contains("carry and edit"),
        "{text}"
    );
    // Below the bar, as the bar is below its wall.
    assert!(model.unwrap().w_axis.y < bar.middle.y);
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

#[test]
fn of_two_bars_the_ray_meets_the_nearer_takes_hold_of_its_wall() {
    let now = Instant::now();
    // Two walls one behind the other, so that a ray from home at the nearer's bar passes through the farther's too.
    let wall = |id: &str, at: Vec3| {
        Wall::new(
            id,
            WallKind::Added,
            Pose::upright_facing(at.to_array(), [at.x, at.y, 0.0]),
        )
    };
    let left = wall("wall-1", Vec3::new(-0.3, 1.5, -2.5));
    let near = bar_of(&left);
    let below = near.middle - Vec3::from_array(left.pose.position);
    let right = wall("wall-2", HOME + (near.middle - HOME) * 1.8 - below);
    let far = bar_of(&right);
    let ray = Ray::new(HOME, near.middle - HOME);
    assert!(near.hit(&ray.unwrap()).is_some() && far.hit(&ray.unwrap()).is_some());
    let projection = shown(vec![right.clone(), left.clone()], Vec::new());
    let mut projector = Projector::default();
    let output = projector.update(frame_at(&projection, ray, 1.0, now));
    assert!(output.wall_ticked);
    assert_eq!(projector.carried_wall(), Some("wall-1"));
}

/// Where the server locks walls, as a collaborator and as an owner.
const LOCKS: Rights = Rights {
    adjust: true,
    remove: false,
    snap: true,
    locks: true,
    lock: false,
};

const LOCKER: Rights = Rights {
    adjust: true,
    remove: true,
    snap: true,
    locks: true,
    lock: true,
};

/// The farmyard's notice board 2.5 m ahead of home, facing it, flat, two slots wide and one high, locked as the
/// environment has it.
fn notice_board() -> Wall {
    Wall::new(
        "environment-0",
        WallKind::Environment,
        Pose::upright_facing([0.0, 1.5, -2.5], HOME.to_array()),
    )
    .with_grid(Grid::new(2, 1).unwrap())
    .with_locked(true)
}

/// The faces of the handles and the hint `output` draws, the hint first, as they read.
fn faces(output: &crate::projection::Output, handles: usize) -> Vec<String> {
    output
        .sheets
        .iter()
        .rev()
        .take(handles + 1)
        .filter_map(|sheet| sheet.canvas.as_deref().map(Canvas::plain))
        .collect()
}

/// Where each sheet `output` draws stands, but `wall`'s board.
fn beside(output: &crate::projection::Output, wall: &Wall) -> Vec<Vec3> {
    output
        .sheets
        .iter()
        .filter_map(|sheet| sheet.model.map(|model| model.w_axis.truncate()))
        .filter(|at| at.distance(on(wall, 0.0, 0.0)) > 1e-4)
        .collect()
}

#[test]
fn a_locked_wall_shows_no_bar_nor_handles_and_only_an_owner_near_it_its_padlock_which_unlocks_it() {
    let wall = notice_board();
    let projection = shown(vec![wall.clone()], Vec::new());
    let now = Instant::now();
    let frame_of = model(&wall.pose).unwrap();
    // Its padlock alone, where it stands beyond the top left corner, a row above it, level with the knob.
    let lock = LockControls::of(&wall, LOCKER);
    assert_eq!(
        lock,
        LockControls {
            padlock: true,
            reset: false
        }
    );
    let (points, _) = handle_points_with(&wall, frame_of, lock);
    assert_eq!(
        points.iter().map(|(handle, _)| *handle).collect::<Vec<_>>(),
        [Handle::Lock]
    );
    let unlocked = wall.clone().with_locked(false);
    let (all, size) = handle_points_with(&unlocked, frame_of, LockControls::of(&unlocked, LOCKER));
    let padlock = points[0].1;
    assert_eq!(
        all.iter()
            .find(|(handle, _)| *handle == Handle::Lock)
            .unwrap()
            .1,
        padlock
    );
    // A row beyond its top left corner, within reach of home.
    let local = frame_of.inverse().transform_point3(padlock);
    let corner = frame_of.inverse().transform_point3(
        all.iter()
            .find(|(handle, _)| *handle == Handle::Corner(-1, 1))
            .unwrap()
            .1,
    );
    assert!(
        (local - corner - Vec3::Y * SPACING * size).length() < 1e-4,
        "{local} {corner}"
    );
    assert!(padlock.distance(HOME) < PADLOCK_NEAR);
    // It has no bar, where an unlocked wall has one.
    assert!(group_bar(&wall, frame_of, &[&wall]).is_none());
    let bar = group_bar(&unlocked, frame_of, &[&unlocked]).unwrap();
    for rights in [ADJUST, LOCKS, LOCKER] {
        let mut projector = Projector::default();
        // Nothing is drawn for it but its board and, for an owner near it, its padlock, closed, where it stands.
        let output = projector.update(frame(&projection, None, 0.0, rights, now));
        let drawn = beside(&output, &wall);
        if rights.lock {
            assert_eq!(drawn.len(), 1, "{drawn:?}");
            assert!(drawn[0].distance(padlock) < 1e-4, "{drawn:?}");
        } else {
            assert!(drawn.is_empty(), "{rights:?}: {drawn:?}");
        }
        // Squeezed on its board or where its bar would be, it is not taken hold of, nor anything sent.
        for point in [on(&wall, 0.0, 0.2), bar.middle] {
            let output = projector.update(frame(&projection, at(point), 1.0, rights, now));
            assert!(!output.wall_ticked && output.wall.is_none() && output.lock.is_none());
            projector.update(frame(&projection, at(point), 0.0, rights, now));
            assert_eq!(projector.editing_wall(), None);
        }
        // The ray at its padlock, for an owner, highlights it, ticks, ends there and says the grip unlocks it; for
        // anyone else it meets nothing.
        let output = projector.update(frame(&projection, at(padlock), 0.0, rights, now));
        let label = output
            .sheets
            .iter()
            .filter_map(|sheet| sheet.canvas.as_deref().map(Canvas::plain))
            .find(|text| text.contains("unlock this wall"));
        assert_eq!(output.wall_ticked, rights.lock);
        assert_eq!(output.end.is_some(), rights.lock);
        assert_eq!(label.is_some(), rights.lock);
        // A squeeze on it, for an owner, unlocks it, for everyone, and takes hold of it, shown unlocked at once with
        // every handle, the padlock open among them, and `↺`; for anyone else it does nothing.
        let output = projector.update(frame(&projection, at(padlock), 1.0, rights, now));
        projector.update(frame(&projection, at(padlock), 0.0, rights, now));
        if rights.lock {
            assert!(output.end.unwrap().distance(padlock) < 0.02);
            assert_eq!(
                output.lock,
                Some(LockChange {
                    wall: "environment-0".into(),
                    locked: false,
                    reset: false
                })
            );
            assert_eq!(projector.editing_wall(), Some("environment-0"));
            assert!(!projector.editor.walls(&projection)[0].is_locked());
            let output = projector.update(frame(&projection, None, 0.0, rights, now));
            let hint = &faces(&output, 15)[0];
            for row in [
                "more / fewer slots",
                "size and shape",
                "carry it",
                "lock it as it is",
                "put back, locked",
            ] {
                assert!(hint.contains(row), "{row}: {hint}");
            }
            // Until the server agrees, or a while passes.
            let agreed = shown(vec![unlocked.clone()], Vec::new());
            projector.update(frame(&agreed, None, 0.0, rights, now));
            assert!(projector.editor.pending.is_none());
            // Its padlock, squeezed again, locks it as it is, for everyone, and lets go of it, its padlock alone left.
            let output = projector.update(frame(&agreed, at(padlock), 1.0, rights, now));
            assert_eq!(
                output.lock,
                Some(LockChange {
                    wall: "environment-0".into(),
                    locked: true,
                    reset: false
                })
            );
            assert_eq!(projector.editing_wall(), None);
            let output = projector.update(frame(&agreed, None, 0.0, rights, now));
            assert_eq!(beside(&output, &wall).len(), 1);
        } else {
            assert_eq!(output.lock, None);
            assert_eq!(projector.editing_wall(), None);
            assert!(projector.editor.walls(&projection)[0].is_locked());
        }
    }
    // Of two padlocks the ray meets, the nearer is taken: another locked wall behind it, its padlock on the same ray.
    let mut behind = wall.clone();
    behind.id = "environment-1".into();
    let shift = (padlock - HOME).normalize() * 0.3;
    behind.pose.position = (Vec3::from_array(wall.pose.position) + shift).to_array();
    let projection = shown(vec![behind, wall.clone()], Vec::new());
    let mut projector = Projector::default();
    let output = projector.update(frame(&projection, at(padlock), 1.0, LOCKER, now));
    assert_eq!(output.lock.unwrap().wall, "environment-0");
    // An owner further from its padlock than `PADLOCK_NEAR` sees nothing of it, and the ray there meets nothing.
    let mut far = wall.clone();
    far.pose.position[2] = -6.0;
    let far_padlock = handle_points_with(&far, model(&far.pose).unwrap(), lock).0[0].1;
    assert!(far_padlock.distance(HOME) > PADLOCK_NEAR);
    let projection = shown(vec![far.clone()], Vec::new());
    let mut projector = Projector::default();
    let output = projector.update(frame(&projection, at(far_padlock), 1.0, LOCKER, now));
    assert!(beside(&output, &far).is_empty());
    assert!(output.lock.is_none() && output.end.is_none() && !output.wall_ticked);
    // Where the server locks no walls, as an older one, a wall shows no padlock, and an environment's own is not carried.
    let older = wall.clone().with_locked(false);
    assert_eq!(LockControls::of(&older, OWNER), LockControls::default());
    assert!(!carried(&older, OWNER));
    assert!(carried(&older, LOCKS));
    assert!(!carried(&wall, LOCKER));
    assert!(carried(&added(), OWNER) && !carried(&added().with_locked(true), LOCKER));
}

/// A deck at `height`, 8 m square about the notice board, with the main floor beneath it, as an environment declares
/// them.
fn deck_over_the_board(height: f32) -> crate::locomotion::Locomotion {
    use crate::locomotion::{Locomotion, Surface};
    Locomotion::teleport(vec![
        Surface::disc(Vec3::ZERO, 20.0),
        Surface::rectangle(Vec3::new(0.0, height, -2.5), Vec2::splat(8.0)),
    ])
}

#[test]
fn a_locked_wall_s_padlock_beneath_a_deck_is_neither_shown_to_nor_taken_by_an_owner_on_it_however_near()
 {
    let wall = notice_board();
    let projection = shown(vec![wall.clone()], Vec::new());
    let now = Instant::now();
    let lock = LockControls::of(&wall, LOCKER);
    let padlock = handle_points_with(&wall, model(&wall.pose).unwrap(), lock).0[0].1;
    // An owner on a deck 25 cm above the padlock, standing over the wall a metre out from it, within reach of it.
    let deck = deck_over_the_board(padlock.y + 0.25);
    let eye = Vec3::new(padlock.x, padlock.y + 0.25 + 1.6, padlock.z + 1.0);
    assert!(eye.distance(padlock) < PADLOCK_NEAR);
    let above = |ray: Option<Ray>, grip: f32, surfaces| Frame {
        viewer: Some(eye),
        surfaces,
        ..frame(&projection, ray, grip, LOCKER, now)
    };
    let towards = Ray::new(eye, padlock - eye);
    // Where nothing is declared, as before, the padlock within 3 m shows through the deck's floor and is taken.
    let mut projector = Projector::default();
    let output = projector.update(above(None, 0.0, None));
    assert_eq!(beside(&output, &wall).len(), 1);
    let output = projector.update(above(towards, 0.0, None));
    assert!(output.end.is_some() && output.wall_ticked);
    // With the deck declared, its floor hides the padlock: it is not drawn, the ray at it ends nowhere, says nothing and
    // a squeeze there unlocks nothing.
    let mut projector = Projector::default();
    let output = projector.update(above(None, 0.0, Some(&deck)));
    assert!(beside(&output, &wall).is_empty());
    let output = projector.update(above(towards, 0.0, Some(&deck)));
    assert!(output.end.is_none() && !output.wall_ticked);
    assert!(
        !output
            .sheets
            .iter()
            .filter_map(|sheet| sheet.canvas.as_deref().map(Canvas::plain))
            .any(|text| text.contains("unlock") || text.contains("Locked")),
    );
    let output = projector.update(above(towards, 1.0, Some(&deck)));
    assert!(output.lock.is_none());
    assert_eq!(projector.editing_wall(), None);
    // A deck just below the padlock, the padlock poking up through it, still hides it, as the wall it locks is beneath.
    let low = deck_over_the_board(padlock.y - 0.05);
    let eye_low = Vec3::new(padlock.x, padlock.y - 0.05 + 1.6, padlock.z + 1.0);
    let mut projector = Projector::default();
    let output = projector.update(Frame {
        viewer: Some(eye_low),
        surfaces: Some(&low),
        ..frame(&projection, None, 0.0, LOCKER, now)
    });
    assert!(beside(&output, &wall).is_empty());
    // From the main floor, in clear view within 3 m, the floor beneath and the deck above hide nothing: it is drawn and
    // taken as ever.
    let mut projector = Projector::default();
    let output = projector.update(Frame {
        surfaces: Some(&deck),
        ..frame(&projection, None, 0.0, LOCKER, now)
    });
    let drawn = beside(&output, &wall);
    assert!(
        drawn.len() == 1 && drawn[0].distance(padlock) < 1e-4,
        "{drawn:?}"
    );
    let output = projector.update(Frame {
        surfaces: Some(&deck),
        ..frame(&projection, at(padlock), 1.0, LOCKER, now)
    });
    assert!(output.lock.is_some_and(|lock| !lock.locked));
}

#[test]
fn the_ray_takes_no_padlock_nor_says_locked_beyond_a_floor_the_eyes_see_past() {
    let wall = notice_board();
    let projection = shown(vec![wall.clone()], Vec::new());
    let now = Instant::now();
    let frame_of = model(&wall.pose).unwrap();
    let padlock = handle_points_with(&wall, frame_of, LockControls::of(&wall, LOCKER)).0[0].1;
    let board = on(&wall, 0.0, 0.2);
    // A hand low and to the right, and a plate 20 cm across halfway along its ray to each point, below the eyes' lines
    // to the padlock and the wall, so that the eyes see both and only the ray crosses a floor.
    let hand = |point: Vec3| point + Vec3::new(1.5, -1.0, 0.5);
    let plate = |point: Vec3| {
        let middle = (hand(point) + point) * 0.5;
        crate::locomotion::Locomotion::teleport(vec![crate::locomotion::Surface::rectangle(
            middle,
            Vec2::splat(0.2),
        )])
    };
    let (over_padlock, over_board) = (plate(padlock), plate(board));
    for plate in [&over_padlock, &over_board] {
        assert!(!hidden(Some(plate), HOME, padlock));
        assert!(!hidden(
            Some(plate),
            HOME,
            Vec3::from_array(wall.pose.position)
        ));
    }
    let through = |point: Vec3, surfaces, rights, grip| Frame {
        surfaces,
        ..frame(
            &projection,
            Ray::new(hand(point), point - hand(point)),
            grip,
            rights,
            now,
        )
    };
    let said = |output: &crate::projection::Output| {
        output
            .sheets
            .iter()
            .filter_map(|sheet| sheet.canvas.as_deref().map(Canvas::plain))
            .find(|text| text.contains("Locked"))
    };
    // The padlock shows, as the eyes see it, and the ray at it from the hand takes it, but not across the plate.
    let mut projector = Projector::default();
    let output = projector.update(through(padlock, None, LOCKER, 0.0));
    assert!(output.end.is_some() && output.wall_ticked);
    let mut projector = Projector::default();
    let output = projector.update(through(padlock, Some(&over_padlock), LOCKER, 0.0));
    assert_eq!(beside(&output, &wall).len(), 1);
    assert!(output.end.is_none() && !output.wall_ticked);
    let output = projector.update(through(padlock, Some(&over_padlock), LOCKER, 1.0));
    assert!(output.lock.is_none());
    // Its board says `Locked` to a collaborator at the end of the ray, but not across the plate.
    let mut projector = Projector::default();
    let output = projector.update(through(board, None, LOCKS, 0.0));
    assert!(said(&output).is_some());
    let mut projector = Projector::default();
    let output = projector.update(through(board, Some(&over_board), LOCKS, 0.0));
    assert_eq!(said(&output), None);
    assert!(output.end.is_none());
}

#[test]
fn an_owner_whose_view_of_a_padlock_a_deck_hides_is_told_only_that_its_wall_is_locked() {
    let wall = notice_board();
    let projection = shown(vec![wall.clone()], Vec::new());
    let now = Instant::now();
    let padlock = handle_points_with(
        &wall,
        model(&wall.pose).unwrap(),
        LockControls::of(&wall, LOCKER),
    )
    .0[0]
        .1;
    let said = |output: &crate::projection::Output| {
        output
            .sheets
            .iter()
            .filter_map(|sheet| sheet.canvas.as_deref().map(Canvas::plain))
            .find(|text| text.contains("Locked"))
            .map(|text| text.trim().to_owned())
    };
    // The eyes on a deck over the wall, the ray from a hand beneath it at the board: coming closer would not show the
    // padlock, so it says only `Locked`.
    let deck = deck_over_the_board(padlock.y + 0.25);
    let eye = Vec3::new(padlock.x, padlock.y + 0.25 + 1.6, padlock.z + 1.0);
    let board = at(on(&wall, 0.0, 0.2));
    let mut projector = Projector::default();
    let output = projector.update(Frame {
        viewer: Some(eye),
        surfaces: Some(&deck),
        ..frame(&projection, board, 0.0, LOCKER, now)
    });
    assert!(
        beside(&output, &wall)
            .iter()
            .all(|at| at.distance(padlock) > 1e-3)
    );
    assert_eq!(said(&output).as_deref(), Some("Locked"));
    // Far from it on the floor, in clear view, it says to come closer.
    let mut projector = Projector::default();
    let output = projector.update(Frame {
        viewer: Some(HOME + Vec3::Z * 3.0),
        surfaces: Some(&deck),
        ..frame(&projection, board, 0.0, LOCKER, now)
    });
    assert_eq!(
        said(&output).as_deref(),
        Some("Locked: come closer to unlock")
    );
}

#[test]
fn the_ray_takes_no_panel_on_a_wall_beyond_a_floor() {
    let wall = added().with_grid(Grid::new(1, 1).unwrap());
    let panel = Panel::new(
        "panel-0",
        "Notes",
        "A note.",
        Place::Wall {
            wall: "wall-1".into(),
            slot: 0,
        },
    );
    let projection = shown(vec![wall.clone()], vec![panel]);
    let now = Instant::now();
    let top = wall.pose.position[1] + wall.board().unwrap().1 * 0.5;
    let deck = deck_over_the_board(top + 0.6);
    let eye = Vec3::new(0.0, top + 0.6 + 1.6, -1.0);
    let point = on(&wall, 0.0, 0.0);
    for (surfaces, pointed) in [(None, true), (Some(&deck), false)] {
        let mut projector = Projector::default();
        let output = projector.update(Frame {
            viewer: Some(eye),
            surfaces,
            ..frame(&projection, Ray::new(eye, point - eye), 0.0, OWNER, now)
        });
        assert_eq!(
            output.pointed.as_ref().map(|(id, ..)| id.as_str()),
            pointed.then_some("panel-0"),
            "{surfaces:?}"
        );
    }
}

#[test]
fn the_ray_takes_no_wall_bar_nor_handle_beyond_a_floor() {
    let wall = added();
    let projection = shown(vec![wall.clone()], Vec::new());
    let now = Instant::now();
    // A deck well above the wall's top and every handle it shows, an owner on it a metre and a half out from the wall.
    let top = wall.pose.position[1] + wall.board().unwrap().1 * 0.5;
    let deck = deck_over_the_board(top + 0.6);
    let eye = Vec3::new(0.0, top + 0.6 + 1.6, -1.0);
    let board = on(&wall, 0.0, 0.2);
    let bar = group_bar(&wall, model(&wall.frame()).unwrap(), &[&wall]).unwrap();
    for (point, surfaces, taken) in [
        (board, None, true),
        (board, Some(&deck), false),
        (bar.middle, None, true),
        (bar.middle, Some(&deck), false),
    ] {
        let mut projector = Projector::default();
        let output = projector.update(Frame {
            viewer: Some(eye),
            surfaces,
            ..frame(&projection, Ray::new(eye, point - eye), 1.0, OWNER, now)
        });
        assert_eq!(output.end.is_some(), taken, "{point} {surfaces:?}");
        assert_eq!(
            projector.editing_wall().is_some(),
            taken,
            "{point} {surfaces:?}"
        );
    }
    // Held, its handles beneath the deck are not taken from on it, though one in clear view is from the floor.
    let mut projector = holding(&projection, &wall, OWNER, now);
    let (points, _) = handle_points(&wall, model(&wall.frame()).unwrap());
    let (handle, point) = points
        .into_iter()
        .min_by(|a, b| a.1.y.total_cmp(&b.1.y))
        .unwrap();
    let output = projector.update(Frame {
        viewer: Some(eye),
        surfaces: Some(&deck),
        ..frame(&projection, Ray::new(eye, point - eye), 0.0, OWNER, now)
    });
    assert!(output.end.is_none() && !output.wall_ticked, "{handle:?}");
    let output = projector.update(Frame {
        surfaces: Some(&deck),
        ..frame(&projection, at(point), 0.0, OWNER, now)
    });
    assert!(output.end.is_some() && output.wall_ticked, "{handle:?}");
}

#[test]
fn an_environment_s_own_wall_unlocked_is_carried_and_put_down_but_never_removed_and_an_owner_puts_it_back()
 {
    let wall = notice_board().with_locked(false);
    let projection = shown(vec![tomori(), wall.clone()], Vec::new());
    let now = Instant::now();
    // Taken hold of by its board, it is carried, and let go a metre to the left, its new pose is sent.
    let mut projector = Projector::default();
    let ray = at(on(&wall, 0.0, 0.2));
    projector.update(frame(&projection, ray, 1.0, LOCKER, now));
    assert_eq!(projector.carried_wall(), Some("environment-0"));
    let output = projector.update(frame(&projection, ray, 1.0, LOCKER, now));
    let hint = &faces(&output, 0)[0];
    assert!(
        hint.contains("further / closer") && !hint.contains("remove"),
        "{hint}"
    );
    // B removes no environment's wall, even an owner's.
    assert_eq!(projector.editor.remove(0, now), None);
    assert_eq!(projector.carried_wall(), Some("environment-0"));
    let left = at(on(&wall, -1.0, 0.2));
    projector.update(frame(&projection, left, 1.0, LOCKER, now));
    let output = projector.update(frame(&projection, left, 0.0, LOCKER, now));
    let pose = output.wall.expect("a pose to send").pose.expect("moved");
    assert!(pose.position[0] < -0.5, "{pose:?}");
    // Its padlock, open, and the control that puts it back stand beside it for an owner; squeezed, that puts it back as
    // the environment has it, locked, and lets go of it, as it goes where the server says.
    let frame_of = model(&wall.pose).unwrap();
    let mut projector = holding(&projection, &wall, LOCKER, now);
    assert_eq!(projector.carried_wall(), None);
    let projector_frame = frame(&projection, None, 0.0, LOCKER, now);
    projector.update(projector_frame);
    let (points, size) = handle_points_with(&wall, frame_of, LockControls::of(&wall, LOCKER));
    assert_eq!(points.len(), 15);
    let reset = points
        .iter()
        .find(|(handle, _)| *handle == Handle::Reset)
        .unwrap()
        .1;
    let lock = points
        .iter()
        .find(|(handle, _)| *handle == Handle::Lock)
        .unwrap()
        .1;
    // The control that puts it back stands beside the padlock, inward.
    let local = |point: Vec3| frame_of.inverse().transform_point3(point);
    assert!(
        (local(reset) - local(lock) - Vec3::X * SPACING * size).length() < 1e-4,
        "{} {}",
        local(reset),
        local(lock)
    );
    let output = projector.update(frame(&projection, at(reset), 1.0, LOCKER, now));
    assert_eq!(
        output.lock,
        Some(LockChange {
            wall: "environment-0".into(),
            locked: true,
            reset: true
        })
    );
    assert_eq!(projector.editing_wall(), None);
    // A collaborator sees its padlock, open, but no control to put it back, and locks nothing with it.
    assert_eq!(
        LockControls::of(&wall, LOCKS),
        LockControls {
            padlock: true,
            reset: false
        }
    );
    let mut projector = holding(&projection, &wall, LOCKS, now);
    let output = projector.update(frame(&projection, at(lock), 1.0, LOCKS, now));
    assert_eq!(output.lock, None);
    // An added wall has no control to put it back; the Tomori wall's padlock stands beside its own handles.
    assert!(!LockControls::of(&added(), LOCKER).reset);
    let tomori = tomori();
    let (points, _) = handle_points_with(
        &tomori,
        model(&tomori.pose).unwrap(),
        LockControls::of(&tomori, LOCKER),
    );
    assert_eq!(points.len(), 7);
}

#[test]
fn a_locked_wall_s_padlock_says_the_grip_unlocks_it_and_a_bar_what_it_does() {
    let wall = notice_board();
    let label = |rights: Rights| bar_label_for(Controls::Controllers, &wall, rights).plain();
    assert!(
        label(LOCKER).contains("Grip  unlock this wall"),
        "{}",
        label(LOCKER)
    );
    // Unlocked, its bar carries and edits it, as an added wall's does; where the server locks none, it edits it.
    let unlocked = wall.clone().with_locked(false);
    assert!(
        bar_label_for(Controls::Controllers, &unlocked, LOCKS)
            .plain()
            .contains("carry and edit this wall")
    );
    assert!(
        bar_label(Controls::Controllers, &unlocked)
            .plain()
            .contains("edit this wall")
    );
    // The desktop names its key.
    assert!(
        bar_label_for(Controls::Keyboard, &wall, LOCKER)
            .plain()
            .contains("G ")
    );
}

#[test]
fn the_padlock_is_drawn_of_shapes_closed_while_locked_and_open_while_not() {
    let corner = handle_face(Handle::Corner(-1, 1), 0.05, false, false)
        .mesh()
        .len();
    let closed = handle_face(Handle::Lock, 0.05, false, true);
    let open = handle_face(Handle::Lock, 0.05, false, false);
    assert!(
        closed.mesh().len() > corner,
        "{} {corner}",
        closed.mesh().len()
    );
    assert_ne!(closed.shapes, open.shapes);
    assert_eq!(closed.size(), open.size());
    // Every symbol but the knob's dot is drawn of shapes, centred on its cell, and so on its handle (the renderer's
    // `ui_panel::tests::handles` checks each by its ink).
    let middle = Vec2::new(
        crate::ui_text::CELL_WIDTH as f32 / 2.0,
        crate::ui_text::CELL_HEIGHT as f32 / 2.0,
    );
    for handle in [
        Handle::Columns,
        Handle::Rows,
        Handle::Step(Axis::Columns, -1),
        Handle::Step(Axis::Rows, 1),
        Handle::Tilt,
        Handle::Reset,
    ] {
        let face = handle_face(handle, 0.05, false, false);
        assert_eq!(face.plain().trim(), "", "{handle:?}");
        // Its lines' ends, about the middle.
        let ends: Vec<Vec2> = face
            .shapes
            .iter()
            .filter_map(|shape| match shape {
                crate::shapes::Shape::Rule { from, to, .. } => Some([from, to]),
                _ => None,
            })
            .flatten()
            .map(|end| Vec2::from_array(*end))
            .collect();
        // The reset's are its arrow's head, beside its ring.
        let centre = ends.iter().sum::<Vec2>() / ends.len() as f32;
        assert!(
            handle == Handle::Reset || centre.distance(middle) < 1e-4,
            "{handle:?}: {centre}"
        );
    }
    assert_eq!(glyph(Handle::Bend), "●");
    assert!(
        handle_face(Handle::Corner(1, 1), 0.05, false, false)
            .shapes
            .is_empty()
    );
}

#[test]
fn a_wall_locked_while_carried_is_let_go_of_and_a_group_with_a_locked_wall_is_not_carried() {
    let wall = notice_board().with_locked(false);
    let projection = shown(vec![tomori(), wall.clone()], Vec::new());
    let now = Instant::now();
    let mut projector = Projector::default();
    let ray = at(on(&wall, 0.0, 0.2));
    projector.update(frame(&projection, ray, 1.0, LOCKS, now));
    assert_eq!(projector.carried_wall(), Some("environment-0"));
    // An owner elsewhere locks it: it is let go of, as a locked wall is never held, letting go sends nothing, and it
    // shows nothing to a collaborator.
    let locked = shown(vec![tomori(), wall.clone().with_locked(true)], Vec::new());
    let left = at(on(&wall, -1.0, 0.2));
    projector.update(frame(&locked, left, 1.0, LOCKS, now));
    assert_eq!(projector.carried_wall(), None);
    assert_eq!(projector.editing_wall(), None);
    let output = projector.update(frame(&locked, left, 0.0, LOCKS, now));
    assert_eq!(output.wall, None);
    // A wall of a group with a wall locked in it is held, but not carried, since the locked wall would move with it.
    let group = Some("wall-1".to_owned());
    let first = added()
        .with_locked(true)
        .with_group(group.clone(), Vec::new());
    let mut second = added().with_group(group, Vec::new());
    second.id = "wall-2".into();
    second.pose.position[0] += 2.5;
    let projection = shown(vec![tomori(), first, second.clone()], Vec::new());
    let mut projector = Projector::default();
    let ray = at(on(&second, 0.0, 0.0));
    projector.update(frame(&projection, ray, 1.0, LOCKS, now));
    assert_eq!(projector.editing_wall(), Some("wall-2"));
    assert_eq!(projector.carried_wall(), None);
}

/// How far each of `wall`'s handles stands from the nearest point of what the wall shows, its flat
/// board or its curved wall's slots, each flat, as they are drawn, in handles.
fn handle_distances(wall: &Wall) -> Vec<(Handle, f32)> {
    let frame = model(&wall.frame()).unwrap();
    let lock = LockControls {
        padlock: true,
        reset: true,
    };
    let (points, size) = handle_points_with(wall, frame, lock);
    // Each rectangle the wall shows, as its middle, and from it to its right edge and its top, in the space.
    let rectangles: Vec<(Vec3, Vec3, Vec3)> = match wall.board() {
        Some((width, height)) => vec![(
            frame.transform_point3(Vec3::ZERO),
            frame.transform_vector3(Vec3::X * width / 2.0),
            frame.transform_vector3(Vec3::Y * height / 2.0),
        )],
        None => crate::projection::slot_frames(wall, frame)
            .iter()
            .zip(wall.slot_frames())
            .map(|(slot, of)| {
                (
                    slot.transform_point3(Vec3::ZERO),
                    slot.transform_vector3(Vec3::X * of.size[0] / 2.0),
                    slot.transform_vector3(Vec3::Y * of.size[1] / 2.0),
                )
            })
            .collect(),
    };
    let nearest = |at: Vec3| {
        rectangles
            .iter()
            .map(|(middle, across, up)| {
                let to = at - *middle;
                let x = (to.dot(*across) / across.length_squared()).clamp(-1.0, 1.0);
                let y = (to.dot(*up) / up.length_squared()).clamp(-1.0, 1.0);
                at.distance(*middle + *across * x + *up * y)
            })
            .fold(f32::INFINITY, f32::min)
    };
    points
        .iter()
        .map(|(handle, at)| (*handle, nearest(*at) / size))
        .collect()
}

#[test]
fn every_handle_stands_near_its_wall_s_edge_flat_curved_tightly_or_tilted_however_large() {
    let mut checked = 0;
    for (columns, rows) in [(1, 1), (2, 1), (3, 2)] {
        for scale in [0.5, 1.0, 2.0, layout::SCALES.1] {
            let flat = added()
                .with_grid(Grid::new(columns, rows).unwrap())
                .with_scale(scale);
            let tightest = bent(&flat, 1e3).unwrap();
            for radius in [None, Some(tightest), Some(1.5), Some(3.0), Some(8.0)] {
                for tilt in [None, Some(30.0), Some(-layout::MOST_TILT)] {
                    let wall = flat
                        .clone()
                        .with_shape(
                            radius.map_or(WallShape::Grid, |radius| WallShape::Arc { radius }),
                        )
                        .with_tilt(tilt);
                    if !allowed(&wall) {
                        continue;
                    }
                    checked += 1;
                    // Below a curved wall, beyond its label too.
                    let label = match wall.radius() {
                        Some(_) => {
                            (crate::projection::ARC_LABEL_DROP
                                + crate::projection::arc_label(&wall).size().1)
                                / handle_size(&wall)
                        }
                        None => 0.0,
                    };
                    for (handle, far) in handle_distances(&wall) {
                        // Within nine handles of what it draws, the steppers of its columns, beyond the
                        // handle of its columns, and those of a small wall's rows, beyond its corner, the furthest, however it is curved or tilted, as one slot five times a standard
                        // one, curved tightly, stands a metre in front of its slot's edges.
                        assert!(
                            far < 9.0 + label,
                            "{handle:?} of {columns}x{rows} at {scale}×, {:?}, tilted {tilt:?}: {far} handles",
                            wall.radius()
                        );
                    }
                }
            }
        }
    }
    assert!(checked > 100, "{checked}");
}

#[test]
fn the_text_handle_moved_across_scales_a_wall_s_content_within_its_slots_in_steps_and_sends_it_from_its_version()
 {
    let wall = added();
    let projection = shown(vec![wall.clone()], Vec::new());
    let now = Instant::now();
    let full = TRAVEL + DEAD_ZONE;
    // Moved to the left step by step through half the travel, its text is drawn smaller at every step, from as large as
    // its slots give it to half, in tenths, its slots as they were, each showing more and longer lines.
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let half = DEAD_ZONE + TRAVEL / 2.0;
    let moves: Vec<Vec3> = (1..=20)
        .map(|step| -Vec3::X * half * step as f32 / 20.0)
        .collect();
    let (drawn, ticks, sent) = move_hand(
        &mut projector,
        &projection,
        &wall,
        Handle::Text,
        &moves,
        Controls::Controllers,
        now,
    );
    let scales: Vec<f32> = drawn.iter().map(Wall::content_scale).collect();
    assert!(
        scales.windows(2).all(|pair| pair[1] <= pair[0]),
        "{scales:?}"
    );
    assert_eq!(scales[19], layout::CONTENT_SCALES.0);
    assert!(
        scales
            .iter()
            .all(|scale| ((scale * 10.0).round() - scale * 10.0).abs() < 1e-4),
        "{scales:?}"
    );
    for drawn in &drawn {
        assert_eq!(drawn.slot_frames().len(), wall.slot_frames().len());
        for (before, after) in wall.slot_frames().iter().zip(drawn.slot_frames()) {
            assert_eq!((before.position, before.size), (after.position, after.size));
        }
    }
    let (standard, smaller) = (Cells::of(&wall), Cells::of(&drawn[19]));
    assert!(
        smaller.columns >= 2 * standard.columns && smaller.body >= 2 * standard.body,
        "{smaller:?}"
    );
    // Reaching the least ticks, as a stop, never in between.
    let least = scales
        .iter()
        .position(|scale| *scale == layout::CONTENT_SCALES.0)
        .unwrap();
    assert!(
        ticks[least] && ticks.iter().filter(|tick| **tick).count() == 1,
        "{ticks:?}"
    );
    // Let go, only its content scale is sent, made against the wall as its handle was taken hold of.
    assert_eq!(
        sent,
        Some(Change {
            wall: "wall-1".into(),
            grid: None,
            scale: None,
            aspect: None,
            shape: None,
            pose: None,
            tilt: None,
            content_scale: Some(layout::CONTENT_SCALES.0),
            join: None,
            version: Some(7),
        })
    );
    // Drawn so until the server agrees, or, refused as made against a version since changed, at once as it has it.
    assert_eq!(projector.editor.walls(&projection)[0].content_scale(), 0.5);
    let agreed = shown(vec![wall.clone().with_content_scale(Some(0.5))], Vec::new());
    projector.update(frame(&agreed, None, 0.0, ADJUST, now));
    assert!(projector.editor.pending.is_none());
    let mut projector = holding(&projection, &wall, ADJUST, now);
    move_hand(
        &mut projector,
        &projection,
        &wall,
        Handle::Text,
        &[-Vec3::X * 0.1],
        Controls::Controllers,
        now,
    );
    assert!(projector.editor.walls(&projection)[0].content_scale() < 1.0);
    projector.refused_wall(&wall.id);
    assert_eq!(projector.editor.walls(&projection)[0], wall);
    // From half, about one movement of the travel to the right, as the participant sees it at the handle beside the
    // wall's left corner, takes it to twice, through as large as its slots give it, which ticks, its slots then showing
    // fewer, larger lines.
    let small = wall.clone().with_content_scale(Some(0.5));
    let projection = shown(vec![small.clone()], Vec::new());
    let mut projector = holding(&projection, &small, ADJUST, now);
    let moves: Vec<Vec3> = (1..=40)
        .map(|step| Vec3::X * 1.1 * full * step as f32 / 40.0)
        .collect();
    let (drawn, ticks, sent) = move_hand(
        &mut projector,
        &projection,
        &small,
        Handle::Text,
        &moves,
        Controls::Controllers,
        now,
    );
    let scales: Vec<f32> = drawn.iter().map(Wall::content_scale).collect();
    assert!(
        scales.windows(2).all(|pair| pair[1] >= pair[0]),
        "{scales:?}"
    );
    assert_eq!(scales[39], layout::CONTENT_SCALES.1);
    let same = scales.iter().position(|scale| *scale == 1.0).unwrap();
    let most = scales
        .iter()
        .position(|scale| *scale == layout::CONTENT_SCALES.1)
        .unwrap();
    assert!(
        ticks[same] && ticks[most] && ticks.iter().filter(|tick| **tick).count() == 2,
        "{ticks:?}"
    );
    assert_eq!(sent.and_then(|change| change.content_scale), Some(2.0));
    let larger = Cells::of(&drawn[39]);
    assert!(larger.columns < standard.columns && larger.body < standard.body);
    assert!(larger.columns >= 12 && larger.body >= 2, "{larger:?}");
    // Within the dead zone, or moving up or down, it is as it was and nothing is sent; drifting so while moving across
    // scales it as far as without.
    let projection = shown(vec![wall.clone()], Vec::new());
    let mut projector = holding(&projection, &wall, ADJUST, now);
    let (drawn, _, sent) = move_hand(
        &mut projector,
        &projection,
        &wall,
        Handle::Text,
        &[Vec3::X * 0.008, Vec3::Y * 0.3, -Vec3::Y * 0.3],
        Controls::Controllers,
        now,
    );
    assert!(drawn.iter().all(|wall| wall.content_scale.is_none()) && sent.is_none());
    let scale_by = |moved: Vec3, controls: Controls| {
        let mut projector = holding(&projection, &wall, ADJUST, now);
        move_hand(
            &mut projector,
            &projection,
            &wall,
            Handle::Text,
            &[moved],
            controls,
            now,
        )
        .0[0]
            .content_scale()
    };
    assert_eq!(
        scale_by(-Vec3::X * 0.06, Controls::Controllers),
        scale_by(-Vec3::X * 0.06 + Vec3::Y * 0.1, Controls::Controllers)
    );
    // In the desktop window, dragging it right draws the text larger too.
    assert!(scale_by(Vec3::X * 0.06, Controls::Keyboard) > 1.0);
    assert_eq!(content_scaled(&wall, f32::NAN), 1.0);
    // The Tomori wall has its text handle too, below its bar, but its text never smaller than half a standard
    // slot's: at its 0.7, 0.72.
    let tomori = tomori().with_version(3);
    let projection = shown(vec![tomori.clone()], Vec::new());
    let mut projector = holding(&projection, &tomori, ADJUST, now);
    let (drawn, _, sent) = move_hand(
        &mut projector,
        &projection,
        &tomori,
        Handle::Text,
        &[-Vec3::X * full],
        Controls::Controllers,
        now,
    );
    assert_eq!(drawn[0].content_scale(), 0.72);
    assert_eq!(sent.and_then(|change| change.content_scale), Some(0.72));
    let text = handle(&tomori, Handle::Text);
    let bar = bar_of(&tomori);
    assert!(
        (text.y - bar.middle.y).abs() < 1e-4 && text.x < bar.middle.x,
        "{text}"
    );
    // A locked wall shows its padlock alone, so no text handle; an owner unlocks it first.
    let padlock = LockControls {
        padlock: true,
        reset: false,
    };
    assert_eq!(
        handles(&wall.clone().with_locked(true), padlock),
        vec![Handle::Lock]
    );
    assert!(handles(&wall, padlock).contains(&Handle::Text));
    // Its readout says how large its text is, and how many columns and rows of a panel's body each slot shows.
    assert_eq!(
        readout(Handle::Text, &wall).plain().trim(),
        format!(
            "Text 100% · {} columns × {} rows",
            crate::projection::COLUMNS,
            crate::projection::BODY_ROWS
        )
    );
    let eighty = wall.clone().with_content_scale(Some(0.8));
    let cells = Cells::of(&eighty);
    assert_eq!(
        readout(Handle::Text, &eighty).plain().trim(),
        format!("Text 80% · {} columns × {} rows", cells.columns, cells.body)
    );
    // Its hint names it, in the desktop window by its key.
    let keys = super::hint(Controls::Keyboard, &wall, false, false, None).plain();
    assert!(
        keys.contains("G aA") && keys.contains("↔ text size"),
        "{keys}"
    );
}

#[test]
fn a_locked_wall_pointed_at_says_faintly_that_it_is_locked_and_an_owner_too_far_to_come_closer() {
    let wall = notice_board();
    let now = Instant::now();
    let said = |output: &crate::projection::Output| {
        output
            .sheets
            .iter()
            .filter_map(|sheet| sheet.canvas.as_deref().map(Canvas::plain))
            .find(|text| text.contains("Locked"))
    };
    // Near it, a collaborator is told it is locked; an owner, who sees its padlock, nothing more.
    let projection = shown(vec![wall.clone()], Vec::new());
    let board = at(on(&wall, 0.0, 0.2));
    let mut projector = Projector::default();
    let output = projector.update(frame(&projection, board, 0.0, LOCKS, now));
    let text = said(&output).unwrap();
    assert!(
        text.contains("Locked") && !text.contains("closer"),
        "{text}"
    );
    let mut projector = Projector::default();
    assert_eq!(
        said(&projector.update(frame(&projection, board, 0.0, LOCKER, now))),
        None
    );
    // Unlocked, it says nothing, nor to a visitor, who adjusts no walls.
    let unlocked = shown(vec![wall.clone().with_locked(false)], Vec::new());
    assert_eq!(
        said(&projector.update(frame(&unlocked, board, 0.0, LOCKER, now))),
        None
    );
    let mut projector = Projector::default();
    assert_eq!(
        said(&projector.update(frame(&projection, board, 0.0, Rights::default(), now))),
        None
    );
    // Too far for its padlock to show, an owner is told to come closer to unlock it.
    let mut far = wall.clone();
    far.pose.position[2] = -6.0;
    let projection = shown(vec![far.clone()], Vec::new());
    let mut projector = Projector::default();
    let output = projector.update(frame(&projection, at(on(&far, 0.0, 0.2)), 0.0, LOCKER, now));
    assert!(
        said(&output)
            .unwrap()
            .contains("Locked: come closer to unlock")
    );
}

#[test]
fn an_owner_s_padlock_is_laid_out_once_and_again_only_as_its_wall_changes() {
    let wall = notice_board();
    let now = Instant::now();
    let projection = shown(vec![wall.clone()], Vec::new());
    let mut projector = Projector::default();
    projector.update(frame(&projection, None, 0.0, LOCKER, now));
    let padlock = handle_points_with(
        &wall,
        model(&wall.pose).unwrap(),
        LockControls::of(&wall, LOCKER),
    )
    .0[0]
        .1;
    assert_eq!(projector.editor.padlock_layouts.len(), 1);
    // Kept, as it was laid out, while the wall is the same: drawn where what was kept puts it.
    let elsewhere = padlock + Vec3::X * 0.1;
    projector
        .editor
        .padlock_layouts
        .get_mut("environment-0")
        .unwrap()
        .models[0]
        .w_axis = elsewhere.extend(1.0);
    let output = projector.update(frame(&projection, None, 0.0, LOCKER, now));
    assert!(
        beside(&output, &wall)
            .iter()
            .any(|at| at.distance(elsewhere) < 1e-4)
    );
    // The wall moved, it is laid out again, where the wall now puts it.
    let mut moved = wall.clone();
    moved.pose.position[0] += 0.5;
    let projection = shown(vec![moved.clone()], Vec::new());
    let output = projector.update(frame(&projection, None, 0.0, LOCKER, now));
    let drawn = beside(&output, &moved);
    assert!(
        drawn
            .iter()
            .any(|at| at.distance(padlock + Vec3::X * 0.5) < 1e-4),
        "{drawn:?}"
    );
    // Unlocked, it is forgotten.
    let projection = shown(vec![moved.with_locked(false)], Vec::new());
    projector.update(frame(&projection, None, 0.0, LOCKER, now));
    assert!(projector.editor.padlock_layouts.is_empty());
}

/// `frame`, with the right hand idle and the left hand's ray and grip.
fn left_handed<'a>(
    projection: &'a Projection,
    ray: Option<Ray>,
    grip: f32,
    rights: Rights,
    now: Instant,
) -> Frame<'a> {
    Frame {
        ray: None,
        grip: Some(0.0),
        left: crate::projection::Hold {
            ray,
            grip: Some(grip),
            active: true,
            pointing: true,
            nearer: f32::INFINITY,
            free: false,
            stick: Vec2::ZERO,
        },
        ..frame(projection, None, 0.0, rights, now)
    }
}

#[test]
fn the_left_grip_takes_hold_of_and_carries_a_wall_as_the_right_does_and_the_right_meanwhile_takes_nothing_of_it()
 {
    let wall = added();
    let projection = shown(vec![wall.clone()], Vec::new());
    let now = Instant::now();
    let mut projector = Projector::default();
    let ray = at(on(&wall, 0.0, 0.42));
    // The left ray meets the wall's board, where it ends, as the right's would.
    let output = projector.update(left_handed(&projection, ray, 0.0, ADJUST, now));
    assert!(output.left.hit.is_some_and(|hit| !hit.held));
    assert_eq!(output.hit, None);
    // Squeezed, the left grip takes hold of it and carries it along the left ray, and the editor follows the left.
    let output = projector.update(left_handed(&projection, ray, 1.0, ADJUST, now));
    assert_eq!(output.wall_hand, Side::Left);
    assert_eq!(projector.carried_wall_in(Side::Left), Some("wall-1"));
    assert_eq!(projector.carried_wall_in(Side::Right), None);
    assert!(projector.carrying_in(Side::Left) && !projector.carrying_in(Side::Right));
    let aside = at(on(&wall, -1.0, 0.42));
    let output = projector.update(left_handed(&projection, aside, 1.0, ADJUST, now));
    assert!(output.left.hit.is_some_and(|hit| hit.held));
    // The right ray meets the wall carried, and its squeeze takes nothing: the wall stays the left's.
    let right = at(on(&wall, -1.0, 0.42));
    let both = |grip: f32| Frame {
        ray: right,
        grip: Some(grip),
        ..left_handed(&projection, aside, 1.0, ADJUST, now)
    };
    let output = projector.update(both(0.0));
    assert!(
        output.hit.is_some(),
        "the right ray ends on the wall carried"
    );
    projector.update(both(1.0));
    assert_eq!(projector.carried_wall_in(Side::Left), Some("wall-1"));
    // Let go by the left, it is put down where the left hand left it.
    let output = projector.update(left_handed(&projection, aside, 0.0, ADJUST, now));
    let change = output.wall.expect("a pose to send");
    assert!(change.pose.expect("moved").position[0] < -0.5);
    assert!(!projector.carrying());
    // Still taken hold of, its handles show; the right ray, which the editor does not follow, meets one, ending on it.
    let moved = projector.editor.walls(&projection)[0].clone();
    let frame_of = model(&moved.frame()).unwrap();
    let (models, _) = handle_models(
        &moved,
        frame_of,
        &[&moved],
        None,
        LockControls::of(&moved, ADJUST),
    );
    let handle = models[0].1.w_axis.truncate();
    let output = projector.update(Frame {
        ray: at(handle),
        ..left_handed(&projection, None, 0.0, ADJUST, now)
    });
    assert!(
        output
            .hit
            .is_some_and(|hit| hit.point.distance(handle) < 0.05),
        "{:?}",
        output.hit
    );
    // A hand that is not followed meets nothing where the participant may not adjust walls.
    let none = Rights {
        adjust: false,
        ..ADJUST
    };
    let output = projector.update(Frame {
        ray: at(on(&wall, 0.0, 0.42)),
        ..left_handed(&projection, None, 0.0, none, now)
    });
    assert_eq!(output.hit, None);
}
