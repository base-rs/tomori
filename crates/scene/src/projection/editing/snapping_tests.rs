// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::arrival::HOME;
use crate::projection::{Input as Frame, Projector};
use tomori_protocol::{Align, Edge, Join};

const SNAPS: Rights = Rights {
    adjust: true,
    remove: false,
    snap: true,
    locks: false,
    lock: false,
};

/// A wall of `columns` by `rows` at `scale`, someone added, at `pose`.
fn wall(id: &str, pose: Pose, (columns, rows): (u32, u32), scale: f32) -> Wall {
    Wall::new(id, WallKind::Added, pose)
        .with_grid(Grid::new(columns, rows).unwrap())
        .with_scale(scale)
}

/// A two by two wall 2.5 m ahead of home, facing it, and a big one by one wall, as one would stand snapped to its right.
fn pair() -> (Wall, Wall) {
    let two = wall(
        "wall-1",
        Pose::upright_facing([-0.4, 1.5, -2.5], [-0.4, 1.5, 0.0]),
        (2, 2),
        0.95,
    );
    let mut big = wall("wall-2", two.pose, (1, 1), 1.9);
    big.pose = group::attached_pose(&two, &big, Edge::Right, Align::Start).unwrap();
    (two, big)
}

/// The two, grouped as the server sends them.
fn grouped() -> (Wall, Wall) {
    let (two, big) = pair();
    (
        two.with_group(
            Some("wall-1".into()),
            vec![Join::new(Edge::Right, "wall-2")],
        ),
        big.with_group(Some("wall-1".into()), vec![Join::new(Edge::Left, "wall-1")]),
    )
}

/// `pose` turned about the vertical through home by `angle` radians, keeping the way it faces.
fn swung(pose: Pose, angle: f32) -> Pose {
    let at = Quat::from_rotation_y(angle) * (Vec3::from_array(pose.position) - HOME) + HOME;
    Pose {
        position: at.to_array(),
        ..pose
    }
}

fn shown(walls: Vec<Wall>) -> Projection {
    Projection::new("s", "sky").with_walls(walls)
}

fn frame<'a>(projection: &'a Projection, ray: Option<Ray>, grip: f32, rights: Rights) -> Frame<'a> {
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
        left: crate::projection::Hold::default(),
        now: Instant::now(),
        dt: 1.0 / 72.0,
    }
}

/// Where `wall`'s board is taken hold of, at the middle of its top margin, in the scene, standing at `pose`.
fn grip_point(wall: &Wall, pose: &Pose) -> Vec3 {
    let (_, height) = wall.board().unwrap();
    model(pose)
        .unwrap()
        .transform_point3(Vec3::new(0.0, height / 2.0 - 0.005, 0.0))
}

fn at(point: Vec3) -> Option<Ray> {
    Ray::new(HOME, point - HOME)
}

/// The hint drawn last, as text.
fn hint(output: &crate::projection::Output) -> String {
    output
        .sheets
        .last()
        .and_then(|sheet| sheet.canvas.as_deref())
        .unwrap()
        .plain()
}

#[test]
fn a_wall_carried_near_another_s_edge_outlines_where_it_snaps_and_let_go_snaps_there() {
    let (two, big) = pair();
    let snapped = big.pose;
    // Standing half a metre to the right of where it would snap, as far from home.
    let mut apart = big.clone();
    apart.pose = swung(snapped, -0.2);
    let projection = shown(vec![two.clone(), apart.clone()]);
    let mut projector = Projector::default();
    let start = at(grip_point(&apart, &apart.pose));
    projector.update(frame(&projection, start, 1.0, SNAPS));
    assert_eq!(projector.carried_wall(), Some("wall-2"));
    // Away from the other, nothing is outlined, and letting go leaves it here.
    let output = projector.update(frame(&projection, start, 1.0, SNAPS));
    assert!(!projector.editor.snapping());
    assert!(hint(&output).contains("leave it here"));
    // Brought within a few centimetres of where it snaps, with a tick, its slot outlined there and the hint saying so.
    let near = at(grip_point(&big, &snapped) + Vec3::new(0.03, 0.02, 0.0));
    let output = projector.update(frame(&projection, near, 1.0, SNAPS));
    assert!(projector.editor.snapping());
    assert!(output.wall_ticked);
    assert!(hint(&output).contains("snap it there"), "{}", hint(&output));
    let outline = output
        .sheets
        .iter()
        .filter(|sheet| {
            sheet
                .canvas
                .as_deref()
                .is_some_and(|canvas| canvas.background == super::super::GHOST)
        })
        .count();
    assert_eq!(outline, 1);
    // Let go, it is sent beside the other, aligned at their tops, its group coming with it, and drawn where it snaps.
    let output = projector.update(frame(&projection, near, 0.0, SNAPS));
    let change = output.wall.expect("a snap to send");
    assert_eq!(change.wall, "wall-2");
    assert_eq!(
        change.join,
        Some(Joining::Attach {
            attach: Attach::new("wall-1", Edge::Right, Align::Start),
            alone: false,
        })
    );
    assert_eq!(change.pose, None);
    let drawn = projector.editor.walls(&projection);
    let shown = drawn.iter().find(|wall| wall.id == "wall-2").unwrap();
    assert!(
        Vec3::from_array(shown.pose.position).distance(Vec3::from_array(snapped.position)) < 1e-4
    );
    // A participant in a space whose server snaps nothing carries it, but it snaps nowhere.
    let mut projector = Projector::default();
    let rights = Rights {
        snap: false,
        ..SNAPS
    };
    projector.update(frame(&projection, start, 1.0, rights));
    projector.update(frame(&projection, near, 1.0, rights));
    assert!(!projector.editor.snapping());
    let output = projector.update(frame(&projection, near, 0.0, rights));
    assert_eq!(output.wall.unwrap().join, None);
}

#[test]
fn a_wall_carried_carries_its_group_and_pulled_firmly_towards_the_participant_comes_out_alone() {
    let (two, big) = grouped();
    let projection = shown(vec![two.clone(), big.clone()]);
    let mut projector = Projector::default();
    let start = at(grip_point(&big, &big.pose));
    projector.update(frame(&projection, start, 1.0, SNAPS));
    // Carried half a metre to the left, the other comes with it, standing as it stood from it.
    let left = at(grip_point(&big, &big.pose) - Vec3::X * 0.5);
    let output = projector.update(frame(&projection, left, 1.0, SNAPS));
    assert!(
        hint(&output).contains("take it out alone"),
        "{}",
        hint(&output)
    );
    let drawn = projector.editor.walls(&projection);
    let (moved, carried) = (&drawn[0], &drawn[1]);
    assert!(
        moved.pose.position[0] < two.pose.position[0] - 0.1,
        "{moved:?}"
    );
    let (u, y, depth, turn) = group::on_surface(&moved.pose, None, &carried.pose);
    let (was_u, was_y, ..) = group::on_surface(&two.pose, None, &big.pose);
    assert!((u - was_u).abs() < 1e-3 && (y - was_y).abs() < 1e-3);
    assert!(depth.abs() < 1e-3 && turn.abs() < 1e-3);
    // Let go, the one carried is sent where it is, and the server moves its group; both are drawn there meanwhile.
    let output = projector.update(frame(&projection, left, 0.0, SNAPS));
    let change = output.wall.unwrap();
    assert_eq!((change.wall.as_str(), change.join), ("wall-2", None));
    assert!(change.pose.is_some());
    let drawn = projector.editor.walls(&projection);
    assert!(drawn[0].pose.position[0] < two.pose.position[0] - 0.1);
    // Carried again and pulled firmly towards the participant, it comes out with a tick, the other staying.
    let mut projector = Projector::default();
    projector.update(frame(&projection, start, 1.0, SNAPS));
    let mut ticked = false;
    for _ in 0..40 {
        let mut pull = frame(&projection, start, 1.0, SNAPS);
        pull.stick = Vec2::new(0.0, -1.0);
        pull.dt = 0.02;
        ticked |= projector.update(pull).wall_ticked;
        if projector.editor.detached {
            break;
        }
    }
    assert!(projector.editor.detached && ticked);
    let output = projector.update(frame(&projection, start, 1.0, SNAPS));
    assert!(!hint(&output).contains("take it out alone"));
    let drawn = projector.editor.walls(&projection);
    assert_eq!(drawn[0].pose, two.pose);
    assert!(drawn[1].group.is_none() && drawn[1].joined.is_empty());
    // Let go, it leaves its group where it is.
    let output = projector.update(frame(&projection, start, 0.0, SNAPS));
    let change = output.wall.unwrap();
    assert_eq!(change.join, Some(Joining::Leave));
    assert!(change.pose.is_some());
}

#[test]
fn the_hint_says_how_to_take_a_wall_out_of_its_group_and_that_letting_go_snaps_it() {
    let (_, big) = grouped();
    let grouped = Grouping {
        grouped: true,
        snapping: true,
    };
    let keys = hint_with(Controls::Keyboard, &big, true, false, None, grouped).plain();
    assert!(
        keys.contains("PgDn far") && keys.contains("take it out alone"),
        "{keys}"
    );
    assert!(keys.contains("snap it there"), "{keys}");
    let stick = hint_with(Controls::Controllers, &big, true, false, None, grouped).plain();
    assert!(stick.contains("Stick ↓ far"), "{stick}");
    assert!(
        !super::hint(Controls::Controllers, &big, true, false, None)
            .plain()
            .contains("take it out")
    );
}

#[test]
fn a_wall_pulled_out_of_its_group_snaps_elsewhere_alone() {
    let (two, big) = grouped();
    // A third wall, alone, a few centimetres to the right of where it would snap beside the second.
    let mut third = wall("wall-3", two.pose, (1, 1), 1.0);
    third.pose = group::attached_pose(&big, &third, Edge::Right, Align::Start).unwrap();
    third.pose = swung(third.pose, -0.025);
    let projection = shown(vec![two.clone(), big.clone(), third.clone()]);
    let mut projector = Projector::default();
    let start = at(grip_point(&big, &big.pose));
    projector.update(frame(&projection, start, 1.0, SNAPS));
    // Pulled out, it stands in front of the walls, and snaps to none.
    for _ in 0..40 {
        let mut pull = frame(&projection, start, 1.0, SNAPS);
        pull.stick = Vec2::new(0.0, -1.0);
        pull.dt = 0.02;
        projector.update(pull);
        if projector.editor.detached {
            break;
        }
    }
    assert!(projector.editor.detached && !projector.editor.snapping());
    // Pushed back out to where it was taken, and carried right to where it would stand beside the third, it snaps there,
    // alone.
    let carry = |projector: &Projector| projector.editor.carry.clone().unwrap();
    while carry(&projector).manipulation.distance() < carry(&projector).start - 1e-3 {
        let mut push = frame(&projection, start, 1.0, SNAPS);
        push.stick = Vec2::new(0.0, 0.5);
        push.dt = 0.002;
        projector.update(push);
    }
    let right = at(grip_point(&big, &big.pose) + Vec3::X * 0.065);
    projector.update(frame(&projection, right, 1.0, SNAPS));
    assert!(projector.editor.snapping());
    let output = projector.update(frame(&projection, right, 0.0, SNAPS));
    match output.wall.unwrap().join {
        Some(Joining::Attach { attach, alone }) => {
            assert!(alone);
            assert_eq!((attach.to.as_str(), attach.edge), ("wall-3", Edge::Left));
        }
        other => unreachable!("a snap, not {other:?}"),
    }
}

/// The models of the ghost outlines drawn where the wall carried would snap.
fn ghosts(output: &crate::projection::Output) -> Vec<Mat4> {
    output
        .sheets
        .iter()
        .filter(|sheet| {
            sheet
                .canvas
                .as_deref()
                .is_some_and(|canvas| canvas.background == super::super::GHOST)
        })
        .filter_map(|sheet| sheet.model)
        .collect()
}

/// Where `wall`'s slots are outlined standing where it stands, as a ghost is drawn.
fn outlined(wall: &Wall) -> Vec<Mat4> {
    super::super::slot_frames(wall, model(&wall.pose).unwrap())
        .into_iter()
        .map(|slot| slot * Mat4::from_translation(Vec3::Z * LIFT))
        .collect()
}

fn same(a: &[Mat4], b: &[Mat4]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.abs_diff_eq(*b, 1e-4))
}

/// `pose` turned `degrees` about the vertical through its middle, keeping where it is.
fn turned(pose: Pose, degrees: f32) -> Pose {
    let turn = Quat::from_rotation_y(degrees.to_radians()) * Quat::from_array(pose.orientation);
    Pose {
        orientation: turn.to_array(),
        ..pose
    }
}

/// The way `wall` faces to its right, in the space.
fn right_of(wall: &Wall) -> Vec3 {
    model(&wall.pose)
        .unwrap()
        .transform_vector3(Vec3::X)
        .normalize()
}

#[test]
fn a_wall_25_cm_off_and_turned_25_degrees_shows_where_it_snaps_lined_up_and_snaps_exactly_there() {
    let (two, big) = pair();
    let (u, y) = group::attached_offset(&two, &big, Edge::Right, Align::Start).unwrap();
    for degrees in [25.0, -25.0] {
        let mut apart = big.clone();
        apart.pose = turned(group::along(&two.pose, None, u + 0.25, y), degrees);
        let projection = shown(vec![two.clone(), apart.clone()]);
        let mut projector = Projector::default();
        let ray = at(grip_point(&apart, &apart.pose));
        projector.update(frame(&projection, ray, 1.0, SNAPS));
        let output = projector.update(frame(&projection, ray, 1.0, SNAPS));
        assert!(projector.editor.snapping(), "{degrees}°");
        assert!(hint(&output).contains("snap it there"), "{}", hint(&output));
        // The ghost stands where it will snap, square in the other's plane, beside it.
        let shown_at = projector.editor.snap.clone().unwrap().snap.pose;
        assert_eq!(shown_at, big.pose);
        let mut lined_up = big.clone();
        lined_up.pose = shown_at;
        assert!(same(&ghosts(&output), &outlined(&lined_up)));
        // Let go, it snaps there, drawn exactly where the ghost was, lined up with the other.
        let output = projector.update(frame(&projection, ray, 0.0, SNAPS));
        assert_eq!(
            output.wall.unwrap().join,
            Some(Joining::Attach {
                attach: Attach::new("wall-1", Edge::Right, Align::Start),
                alone: false,
            })
        );
        let drawn = projector.editor.walls(&projection);
        let placed = drawn.iter().find(|wall| wall.id == "wall-2").unwrap();
        assert!(
            Vec3::from_array(placed.pose.position).distance(Vec3::from_array(shown_at.position))
                < 1e-4
        );
        assert!(
            Quat::from_array(placed.pose.orientation)
                .angle_between(Quat::from_array(two.pose.orientation))
                < 1e-3
        );
    }
    // Turned 25° by the stick as it is carried, from square, it snaps all the same.
    let mut apart = big.clone();
    apart.pose = turned(group::along(&two.pose, None, u + 0.2, y), -5.0);
    let projection = shown(vec![two.clone(), apart.clone()]);
    let mut projector = Projector::default();
    let ray = at(grip_point(&apart, &apart.pose));
    projector.update(frame(&projection, ray, 1.0, SNAPS));
    for _ in 0..2 {
        let mut flick = frame(&projection, ray, 1.0, SNAPS);
        flick.stick = Vec2::X;
        projector.update(flick);
        projector.update(frame(&projection, ray, 1.0, SNAPS));
    }
    let preview = projector.editor.preview.clone().unwrap();
    let (.., turn) = group::on_surface(&two.pose, None, &preview.pose);
    assert!(
        (turn.abs().to_degrees() - 25.0).abs() < 0.5,
        "{}",
        turn.to_degrees()
    );
    assert!(projector.editor.snapping());
    // 35 cm beyond the edge, it snaps nowhere.
    let mut beyond = big.clone();
    beyond.pose = group::along(&two.pose, None, u + 0.35, y);
    let projection = shown(vec![two.clone(), beyond.clone()]);
    let mut projector = Projector::default();
    let ray = at(grip_point(&beyond, &beyond.pose));
    projector.update(frame(&projection, ray, 1.0, SNAPS));
    let output = projector.update(frame(&projection, ray, 1.0, SNAPS));
    assert!(!projector.editor.snapping() && ghosts(&output).is_empty());
}

#[test]
fn a_snap_shown_is_held_a_little_beyond_where_it_is_first_shown() {
    let (two, big) = pair();
    let (u, y) = group::attached_offset(&two, &big, Edge::Right, Align::Start).unwrap();
    let mut apart = big.clone();
    apart.pose = group::along(&two.pose, None, u + 0.26, y);
    let projection = shown(vec![two.clone(), apart.clone()]);
    let mut projector = Projector::default();
    let grip = grip_point(&apart, &apart.pose);
    let right = right_of(&two);
    projector.update(frame(&projection, at(grip), 1.0, SNAPS));
    // Where it is beyond the edge, `shift` from where it was taken.
    let mut carried = |shift: f32| {
        let output = projector.update(frame(&projection, at(grip + right * shift), 1.0, SNAPS));
        (projector.editor.snapping(), output.wall_ticked)
    };
    // Shown at 26 cm, as it was taken hold of; held at 34 cm; gone at 42 cm; not shown again at 34 cm, until within
    // 30 cm, with a tick.
    assert!(carried(0.0).0);
    assert_eq!(carried(0.08), (true, false));
    assert!(!carried(0.16).0);
    assert!(!carried(0.08).0);
    assert_eq!(carried(0.02), (true, true));
}

#[test]
fn of_two_neighbours_the_nearer_is_shown_and_the_one_shown_stays_until_the_other_is_clearly_nearer()
{
    let left = wall(
        "wall-1",
        Pose::upright_facing([-0.8, 1.5, -2.5], [-0.8, 1.5, 0.0]),
        (1, 1),
        1.0,
    );
    let mut middle = wall("wall-2", left.pose, (1, 1), 1.0);
    let mut right = wall("wall-3", left.pose, (1, 1), 1.0);
    let (u, y) = group::attached_offset(&left, &middle, Edge::Right, Align::Start).unwrap();
    right.pose = group::along(&left.pose, None, 2.0 * u + 0.2, 0.0);
    middle.pose = group::along(&left.pose, None, u + 0.05, y);
    let projection = shown(vec![left.clone(), middle.clone(), right.clone()]);
    let mut projector = Projector::default();
    let grip = grip_point(&middle, &middle.pose);
    let across = right_of(&left);
    projector.update(frame(&projection, at(grip), 1.0, SNAPS));
    let mut towards = |at_shift: f32| {
        let output = projector.update(frame(
            &projection,
            at(grip + across * (at_shift - 0.05)),
            1.0,
            SNAPS,
        ));
        let snap = projector.editor.snap.clone().unwrap();
        (snap.to, snap.snap.edge, output.wall_ticked)
    };
    // 5 cm from the left one's edge and 15 cm from the right one's, the left one's.
    assert_eq!(towards(0.05), ("wall-1".into(), Edge::Right, false));
    // Just past halfway, still the left one's; well past, the right one's, with a tick; and back just past halfway, the
    // right one's still.
    assert_eq!(towards(0.104), ("wall-1".into(), Edge::Right, false));
    assert_eq!(towards(0.15), ("wall-3".into(), Edge::Left, true));
    assert_eq!(towards(0.096), ("wall-3".into(), Edge::Left, false));
    assert_eq!(towards(0.04).0, "wall-1");
}

#[test]
fn a_curved_wall_carried_near_a_curved_one_snaps_onto_its_arc_at_its_radius() {
    let curve = |id: &str, columns: u32, radius: f32| {
        wall(
            id,
            Pose::upright_facing([0.0, 1.5, -2.5], [0.0, 1.5, 0.0]),
            (columns, 1),
            1.0,
        )
        .with_shape(WallShape::Arc { radius })
    };
    let target = curve("wall-1", 2, 2.5);
    let mut carried = curve("wall-2", 2, 3.5);
    let (u, y) = group::attached_offset(
        &target,
        &carried.clone().with_shape(WallShape::Arc { radius: 2.5 }),
        Edge::Right,
        Align::Start,
    )
    .unwrap();
    carried.pose = turned(
        group::along(&target.pose, Some(2.5), u + 0.12, y + 0.04),
        -12.0,
    );
    let projection = shown(vec![target.clone(), carried.clone()]);
    let mut projector = Projector::default();
    let ray = at(grip_point_curved(&carried));
    projector.update(frame(&projection, ray, 1.0, SNAPS));
    assert_eq!(projector.carried_wall(), Some("wall-2"));
    let output = projector.update(frame(&projection, ray, 1.0, SNAPS));
    let snap = projector
        .editor
        .snap
        .clone()
        .expect("it snaps onto the arc")
        .snap;
    assert_eq!((snap.edge, snap.radius), (Edge::Right, Some(2.5)));
    // The ghost is the wall at the other's radius, continuing its arc about home.
    let mut on_arc = carried.clone().with_shape(WallShape::Arc { radius: 2.5 });
    on_arc.pose = snap.pose;
    assert!(same(&ghosts(&output), &outlined(&on_arc)));
    let (_, _, depth, turn) = group::on_surface(&target.pose, Some(2.5), &on_arc.pose);
    assert!(depth.abs() < 1e-4 && turn.abs() < 1e-4);
    // Let go, it is drawn there, at that radius, until the server agrees.
    projector.update(frame(&projection, ray, 0.0, SNAPS));
    let drawn = projector.editor.walls(&projection);
    let placed = drawn.iter().find(|wall| wall.id == "wall-2").unwrap();
    assert_eq!(placed.radius(), Some(2.5));
    assert!(same(&outlined(placed), &outlined(&on_arc)));
}

/// Where a curved wall is taken hold of: its bar, above its middle slot.
fn grip_point_curved(wall: &Wall) -> Vec3 {
    bar(wall, model(&wall.pose).unwrap()).middle
}

#[test]
fn the_trigger_held_keeps_a_wall_carried_from_snapping_and_let_go_it_stays_where_it_is() {
    let (two, big) = pair();
    let (u, y) = group::attached_offset(&two, &big, Edge::Right, Align::Start).unwrap();
    let mut apart = big.clone();
    apart.pose = group::along(&two.pose, None, u + 0.1, y);
    let projection = shown(vec![two.clone(), apart.clone()]);
    let mut projector = Projector::default();
    let ray = at(grip_point(&apart, &apart.pose));
    projector.update(frame(&projection, ray, 1.0, SNAPS));
    let output = projector.update(frame(&projection, ray, 1.0, SNAPS));
    assert!(projector.editor.snapping());
    let rows = hint(&output);
    assert!(
        rows.contains("Hold trigger") && rows.contains("don't snap"),
        "{rows}"
    );
    // Held, nothing is outlined and the hint says it is left here.
    let free = |grip: f32| Frame {
        free: true,
        ..frame(&projection, ray, grip, SNAPS)
    };
    let output = projector.update(free(1.0));
    assert!(!projector.editor.snapping() && ghosts(&output).is_empty());
    assert!(hint(&output).contains("leave it here"), "{}", hint(&output));
    // Carried a little nearer the other and let go held, it stays where it is, beside the other, in no group.
    let nearer = at(grip_point(&apart, &apart.pose) - right_of(&two) * 0.05);
    let moved = |grip: f32| Frame {
        free: true,
        ..frame(&projection, nearer, grip, SNAPS)
    };
    projector.update(moved(1.0));
    assert!(!projector.editor.snapping());
    let carried = projector.editor.preview.clone().unwrap();
    let output = projector.update(moved(0.0));
    let change = output.wall.unwrap();
    assert_eq!(change.join, None);
    assert_eq!(change.pose, Some(carried.pose));
    let (left, ..) = group::on_surface(&two.pose, None, &carried.pose);
    assert!((left - (u + 0.05)).abs() < 0.01, "{left}");
    // Let go of the trigger, it snaps again.
    let mut projector = Projector::default();
    projector.update(frame(&projection, ray, 1.0, SNAPS));
    projector.update(free(1.0));
    projector.update(frame(&projection, ray, 1.0, SNAPS));
    assert!(projector.editor.snapping());
    // The desktop window names its key.
    let keys = hint_with(
        Controls::Keyboard,
        &big,
        true,
        false,
        None,
        Grouping {
            grouped: false,
            snapping: true,
        },
    )
    .plain();
    assert!(
        keys.contains("Hold click") && keys.contains("don't snap"),
        "{keys}"
    );
}

#[test]
fn a_tilted_wall_snaps_upright_none_snaps_to_one_tilted_and_a_wall_of_a_group_has_no_hinge() {
    let (two, big) = pair();
    let snapped = big.pose;
    // Tilted, carried by its board where it is drawn, from half a metre away to within a few centimetres of where it
    // snaps: its slots are outlined there upright, as the group's walls stand, and let go, it is drawn so.
    let mut apart = big.clone().with_tilt(Some(20.0));
    apart.pose = swung(snapped, -0.2);
    let projection = shown(vec![two.clone(), apart.clone()]);
    let mut projector = Projector::default();
    let start = at(grip_point(&apart, &apart.frame()));
    projector.update(frame(&projection, start, 1.0, SNAPS));
    assert_eq!(projector.carried_wall(), Some("wall-2"));
    let near = at(grip_point(&big, &snapped) + Vec3::new(0.03, 0.02, 0.0));
    let output = projector.update(frame(&projection, near, 1.0, SNAPS));
    assert!(projector.editor.snapping());
    assert!(same(&ghosts(&output), &outlined(&big)));
    let output = projector.update(frame(&projection, near, 0.0, SNAPS));
    assert!(output.wall.unwrap().join.is_some());
    let drawn = projector.editor.walls(&projection);
    let shown_now = drawn.iter().find(|wall| wall.id == "wall-2").unwrap();
    assert_eq!(shown_now.tilt, None);
    // A wall carried near a tilted one snaps to nothing.
    let projection = shown(vec![
        two.clone().with_tilt(Some(20.0)),
        apart.with_tilt(None),
    ]);
    let mut projector = Projector::default();
    let start = at(grip_point(&big, &swung(snapped, -0.2)));
    projector.update(frame(&projection, start, 1.0, SNAPS));
    assert_eq!(projector.carried_wall(), Some("wall-2"));
    projector.update(frame(&projection, near, 1.0, SNAPS));
    assert!(!projector.editor.snapping());
    // A wall of a group stands upright, and so has no hinge; alone, it has one.
    let (two, big) = grouped();
    assert!(!handles(&two, LockControls::default()).contains(&Handle::Tilt));
    let all = [&two, &big];
    let (points, _) = handle_points_among(&two, model(&two.pose).unwrap(), &all, None);
    assert!(points.iter().all(|(handle, _)| *handle != Handle::Tilt));
    let (alone, _) = pair();
    assert!(handles(&alone, LockControls::default()).contains(&Handle::Tilt));
}
