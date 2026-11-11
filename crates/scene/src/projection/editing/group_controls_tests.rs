// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A group's one grab bar, and the handles of a wall of a group, stand over none of its walls' slots, however the group
//! is arranged and however far away it is seen.
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

/// A wall of `columns` by `rows` at `scale`, someone added, its slots `aspect` times as wide as they are tall, or
/// standard, curved at `radius` or flat.
fn wall(
    id: &str,
    (columns, rows): (u32, u32),
    scale: f32,
    aspect: Option<f32>,
    radius: Option<f32>,
) -> Wall {
    let shape = radius.map_or(WallShape::Grid, |radius| WallShape::Arc { radius });
    Wall::new(id, WallKind::Added, Pose::default())
        .with_grid(Grid::new(columns, rows).unwrap())
        .with_scale(scale)
        .with_aspect(aspect)
        .with_shape(shape)
}

/// `walls` as the server sends them: the first standing at `pose`, each after it attached to an earlier one as `attach`
/// says, by index, all one group, named by the first, each with the edges it is joined across.
fn group_of(pose: Pose, walls: Vec<Wall>, attach: &[(usize, Edge, Align)]) -> Vec<Wall> {
    let mut placed: Vec<Wall> = Vec::new();
    for (index, mut wall) in walls.into_iter().enumerate() {
        wall.pose = match index {
            0 => pose,
            _ => {
                let (to, edge, align) = attach[index - 1];
                group::attached_pose(&placed[to], &wall, edge, align).unwrap()
            }
        };
        placed.push(wall);
    }
    for (index, a) in placed.iter().enumerate() {
        for b in &placed[index + 1..] {
            assert!(!group::overlaps(a, b), "{} overlaps {}", a.id, b.id);
        }
    }
    let first = placed[0].id.clone();
    let joined: Vec<Vec<Join>> = placed
        .iter()
        .map(|wall| {
            placed
                .iter()
                .filter(|other| other.id != wall.id)
                .filter_map(|other| {
                    group::joined(wall, other).map(|(edge, ..)| Join::new(edge, other.id.clone()))
                })
                .collect()
        })
        .collect();
    placed
        .into_iter()
        .zip(joined)
        .map(|(wall, joined)| wall.with_group(Some(first.clone()), joined))
        .collect()
}

/// Facing home from `distance` metres straight ahead.
fn ahead(distance: f32) -> Pose {
    Pose::upright_facing([0.0, 1.5, -distance], [0.0, 1.5, 0.0])
}

/// The maintainer's arrangement: a wide wall at the bottom, and two tall walls snapped above it, at its two ends.
fn stacked(pose: Pose, scale: f32) -> Vec<Wall> {
    group_of(
        pose,
        vec![
            wall("wall-1", (4, 1), scale, None, None),
            wall("wall-2", (1, 1), scale, Some(0.5), None),
            wall("wall-3", (1, 1), scale, Some(0.5), None),
        ],
        &[(0, Edge::Top, Align::Start), (0, Edge::Top, Align::End)],
    )
}

/// Every arrangement tested, standing at `pose`, its walls' slots `scale` times a standard slot's.
fn arrangements(pose: Pose, scale: f32) -> Vec<(&'static str, Vec<Wall>)> {
    let flat = |id, grid| wall(id, grid, scale, None, None);
    let curved = |id, grid| wall(id, grid, scale, None, Some(4.0));
    vec![
        ("stacked", stacked(pose, scale)),
        (
            "side by side",
            group_of(
                pose,
                vec![
                    flat("wall-1", (2, 2)),
                    wall("wall-2", (1, 1), scale * 1.5, None, None),
                ],
                &[(0, Edge::Right, Align::Start)],
            ),
        ),
        (
            "one above another",
            group_of(
                pose,
                vec![flat("wall-1", (3, 1)), flat("wall-2", (2, 1))],
                &[(0, Edge::Top, Align::Middle)],
            ),
        ),
        (
            "three high",
            group_of(
                pose,
                vec![
                    flat("wall-1", (2, 1)),
                    flat("wall-2", (2, 1)),
                    flat("wall-3", (2, 1)),
                ],
                &[(0, Edge::Top, Align::Start), (1, Edge::Top, Align::Start)],
            ),
        ),
        (
            "L-shaped",
            group_of(
                pose,
                vec![
                    flat("wall-1", (2, 2)),
                    flat("wall-2", (1, 1)),
                    flat("wall-3", (2, 1)),
                ],
                &[
                    (0, Edge::Right, Align::Start),
                    (0, Edge::Bottom, Align::Start),
                ],
            ),
        ),
        (
            "curved side by side",
            group_of(
                pose,
                vec![curved("wall-1", (2, 1)), curved("wall-2", (2, 1))],
                &[(0, Edge::Right, Align::Start)],
            ),
        ),
        (
            "curved one above another",
            group_of(
                pose,
                vec![curved("wall-1", (2, 1)), curved("wall-2", (2, 1))],
                &[(0, Edge::Top, Align::Start)],
            ),
        ),
    ]
}

/// Whether the box about `points`, as they stand on `wall`'s surface, reaching `half` beyond them across and up, meets
/// its slots: a flat wall's, about them all; a curved wall's, each as it is drawn, flat, turned to face the line the
/// arc curves about, which reaches less far around than its length along the arc.
fn over_slots(wall: &Wall, points: &[Vec3], half: (f32, f32)) -> bool {
    // A tenth of a millimetre's leeway for rounding.
    let tiny = 1e-4;
    if wall.radius().is_some() {
        let frame = model(&wall.frame()).unwrap();
        return wall.slot_frames().iter().any(|slot| {
            let local = (frame
                * Mat4::from_translation(Vec3::from_array(slot.position))
                * Mat4::from_rotation_y(slot.yaw))
            .inverse();
            let (mut left, mut right, mut bottom, mut top) =
                (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
            for point in points.iter().map(|point| local.transform_point3(*point)) {
                (left, right, bottom, top) = (
                    left.min(point.x),
                    right.max(point.x),
                    bottom.min(point.y),
                    top.max(point.y),
                );
            }
            // Before it or behind it, not across the arc, as a slot of a wall curved more than half around is.
            let near = points.iter().all(|point| {
                local.transform_point3(*point).z.abs() < wall.radius().unwrap_or_default()
            });
            let (width, height) = (slot.size[0] / 2.0, slot.size[1] / 2.0);
            near && left - half.0 < width - tiny
                && right + half.0 > -width + tiny
                && bottom - half.1 < height - tiny
                && top + half.1 > -height + tiny
        });
    }
    let on = |point: &Vec3| {
        let (u, y, ..) = group::on_surface(
            &wall.pose,
            wall.radius(),
            &Pose {
                position: point.to_array(),
                orientation: wall.pose.orientation,
            },
        );
        (u, y)
    };
    let (mut left, mut right, mut bottom, mut top) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for (u, y) in points.iter().map(on) {
        (left, right, bottom, top) = (left.min(u), right.max(u), bottom.min(y), top.max(y));
    }
    let slots = wall.extent();
    left - half.0 < slots.right - tiny
        && right + half.0 > slots.left + tiny
        && bottom - half.1 < slots.top - tiny
        && top + half.1 > slots.bottom + tiny
}

/// Points along `bar`, from end to end, a little apart, so that on a curve its box follows the arc.
fn along(bar: &Bar) -> Vec<Vec3> {
    (0..=16)
        .map(|step| bar.middle + bar.half * (step as f32 / 8.0 - 1.0))
        .collect()
}

#[test]
fn a_group_shows_one_bar_over_none_of_its_slots_and_handles_over_none_however_far_and_large() {
    let mut raised = 0;
    // At eye height, and low, where a group's bar would stand in the floor and stands above it instead.
    for height in [1.5, 0.4] {
        for scale in [layout::SCALES.0, 1.0, 2.0, layout::SCALES.1] {
            let at = Pose::upright_facing([0.0, height, -3.0], [0.0, height, 0.0]);
            for (name, walls) in arrangements(at, scale) {
                let all: Vec<&Wall> = walls.iter().collect();
                let context = format!("{name} {height} m up at {scale}×");
                // One bar, of its bottom left wall, below the slots of every wall, or above them all where it would be
                // in the floor, over none of them and clear of the floor.
                let bars: Vec<(&Wall, Bar)> = walls
                    .iter()
                    .filter_map(|wall| {
                        group_bar(wall, model(&wall.pose).unwrap(), &all).map(|bar| (wall, bar))
                    })
                    .collect();
                assert_eq!(bars.len(), 1, "{context}");
                let (bearer, bar) = bars[0];
                let lowest = walls
                    .iter()
                    .map(|wall| {
                        let frame = model(&wall.pose).unwrap();
                        let slots = wall.extent();
                        frame.transform_point3(Vec3::Y * slots.bottom).y
                    })
                    .fold(f32::INFINITY, f32::min);
                let highest = walls
                    .iter()
                    .map(|wall| {
                        let frame = model(&wall.pose).unwrap();
                        frame.transform_point3(Vec3::Y * wall.extent().top).y
                    })
                    .fold(f32::NEG_INFINITY, f32::max);
                assert!(
                    bar.middle.y < lowest || bar.middle.y > highest,
                    "{context}: the bar of {}",
                    bearer.id
                );
                let floor = tomori_protocol::entourage::FLOOR + FLOOR_CLEARANCE;
                assert!(bar.middle.y - bar.size / 2.0 >= floor, "{context}");
                raised += usize::from(bar.middle.y > highest);
                for wall in &walls {
                    assert!(
                        !over_slots(wall, &along(&bar), (0.0, BAR_THICKNESS * bar.size / 2.0)),
                        "{context}: the bar of {} over {}",
                        bearer.id,
                        wall.id
                    );
                }
                // Each wall's handles, as it is taken hold of, over none of them either, as large as they are drawn.
                for held in &walls {
                    let (points, size) =
                        handle_points_among(held, model(&held.pose).unwrap(), &all, None);
                    for (handle, at) in &points {
                        assert!(
                            at.y - size / 2.0 >= floor,
                            "{context}: {handle:?} in the floor"
                        );
                        for wall in &walls {
                            assert!(
                                !over_slots(wall, &[*at], (size / 2.0, size / 2.0)),
                                "{context}: {handle:?} of {} over {}",
                                held.id,
                                wall.id
                            );
                        }
                    }
                }
            }
        }
    }
    assert!(raised > 0 && raised < 32, "{raised}");
}

#[test]
fn a_wall_of_a_group_keeps_the_handles_of_an_edge_joined_along_the_opposite_one() {
    let walls = stacked(ahead(2.5), 1.0);
    let all: Vec<&Wall> = walls.iter().collect();
    let frame = |wall: &Wall| model(&wall.pose).unwrap();
    let (bottom, tall) = (&walls[0], &walls[1]);
    // The wide wall below, joined across its top, bends from below it, and keeps the handles of its right edge, its
    // columns' and their steppers', and of its rows; its top corners are left out.
    let (points, _) = handle_points_among(bottom, frame(bottom), &all, None);
    let point = |points: &[(Handle, Vec3)], handle| {
        points
            .iter()
            .find(|(shown, _)| *shown == handle)
            .map(|(_, at)| *at)
    };
    let below = frame(bottom).transform_point3(Vec3::ZERO).y;
    assert!(point(&points, Handle::Bend).unwrap().y < below);
    for handle in [
        Handle::Columns,
        Handle::Step(Axis::Columns, -1),
        Handle::Step(Axis::Columns, 1),
        Handle::Rows,
        Handle::Corner(-1, -1),
        Handle::Corner(1, -1),
    ] {
        assert!(point(&points, handle).is_some(), "{handle:?}");
    }
    assert!(
        point(&points, Handle::Corner(-1, 1)).is_none()
            && point(&points, Handle::Corner(1, 1)).is_none()
    );
    // A tall wall above, joined across its bottom, has the handle of its rows above it, beyond its knob.
    let (points, _) = handle_points_among(tall, frame(tall), &all, None);
    let (rows, knob) = (
        point(&points, Handle::Rows).unwrap(),
        point(&points, Handle::Bend).unwrap(),
    );
    assert!(rows.y > knob.y && knob.y > frame(tall).transform_point3(Vec3::ZERO).y);
    // Alone, a wall's handles stand where they always have.
    let mut alone = wall("wall-9", (4, 1), 1.0, None, None);
    alone.pose = ahead(2.5);
    assert_eq!(
        handle_points_among(&alone, frame(&alone), &[&alone], None),
        handle_points(&alone, frame(&alone))
    );
    assert_eq!(
        group_bar(&alone, frame(&alone), &[&alone]),
        Some(bar(&alone, frame(&alone)))
    );
    // A wall joined across both its sides keeps the handles of neither; the handle dragged stays, wherever it is.
    let row = group_of(
        ahead(2.5),
        vec![
            wall("wall-1", (1, 1), 1.0, None, None),
            wall("wall-2", (1, 1), 1.0, None, None),
            wall("wall-3", (1, 1), 1.0, None, None),
        ],
        &[
            (0, Edge::Right, Align::Start),
            (1, Edge::Right, Align::Start),
        ],
    );
    let all: Vec<&Wall> = row.iter().collect();
    let middle = &row[1];
    let (points, _) = handle_points_among(middle, frame(middle), &all, None);
    assert!(
        point(&points, Handle::Columns).is_none()
            && point(&points, Handle::Step(Axis::Columns, 1)).is_none()
    );
    assert!(point(&points, Handle::Bend).is_some() && point(&points, Handle::Rows).is_some());
    assert_eq!(
        handle_point_among(middle, frame(middle), Handle::Columns, &all, None).0,
        frame(middle).transform_point3(Vec3::ZERO)
    );
    // Of large walls, those handles that would stand over another wall's slots are left out, but for the one being
    // dragged, which stays under the hand.
    let mut restored = 0;
    for (_, walls) in arrangements(ahead(15.0), layout::SCALES.1) {
        let all: Vec<&Wall> = walls.iter().collect();
        for wall in &walls {
            let (shown, _) = handle_points_among(wall, frame(wall), &all, None);
            for handle in handles(wall, LockControls::default()) {
                let (kept, _) = handle_points_among(wall, frame(wall), &all, Some(handle));
                assert!(shown.iter().all(|shown| kept.contains(shown)));
                if point(&shown, handle).is_none() && point(&kept, handle).is_some() {
                    restored += 1;
                }
            }
        }
    }
    assert!(restored > 0);
}

fn shown(walls: Vec<Wall>) -> Projection {
    Projection::new("s", "sky").with_walls(walls)
}

fn frame<'a>(projection: &'a Projection, ray: Option<Ray>, grip: f32) -> Frame<'a> {
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
        rights: SNAPS,
        now: Instant::now(),
        dt: 1.0 / 72.0,
    }
}

fn at(point: Vec3) -> Option<Ray> {
    Ray::new(HOME, point - HOME)
}

#[test]
fn the_group_s_bar_carries_the_group_and_a_wall_below_it_still_comes_out_alone() {
    let walls = stacked(ahead(2.5), 1.0);
    let all: Vec<&Wall> = walls.iter().collect();
    let projection = shown(walls.clone());
    // The bar below the wide wall, the lowest, takes hold of it, and its group with it.
    let bar = group_bar(&walls[0], model(&walls[0].pose).unwrap(), &all).unwrap();
    assert!(group_bar(&walls[1], model(&walls[1].pose).unwrap(), &all).is_none());
    let mut projector = Projector::default();
    projector.update(frame(&projection, at(bar.middle), 0.0));
    projector.update(frame(&projection, at(bar.middle), 1.0));
    assert_eq!(projector.carried_wall(), Some("wall-1"));
    assert_eq!(projector.editor.members.len(), 2);
    // The wide wall below, taken hold of by an empty slot, is carried with its group, and pulled firmly towards the
    // participant comes out alone.
    let slot = model(&walls[0].pose)
        .unwrap()
        .transform_point3(Vec3::from_array(walls[0].slot_frames()[1].position));
    let mut projector = Projector::default();
    projector.update(frame(&projection, at(slot), 1.0));
    assert_eq!(projector.carried_wall(), Some("wall-1"));
    assert_eq!(projector.editor.members.len(), 2);
    for _ in 0..40 {
        let mut pull = frame(&projection, at(slot), 1.0);
        pull.stick = Vec2::new(0.0, -1.0);
        pull.dt = 0.02;
        projector.update(pull);
        if projector.editor.detached {
            break;
        }
    }
    assert!(projector.editor.detached);
    let output = projector.update(frame(&projection, at(slot), 0.0));
    assert_eq!(output.wall.unwrap().join, Some(Joining::Leave));
}

/// Where `handle` of `wall` stands among `walls`, with the padlock shown, on `wall`'s surface: across and up from its
/// middle, in metres.
fn padlock_on(wall: &Wall, walls: &[Wall]) -> Vec<(Handle, f32, f32)> {
    let frame = model(&wall.frame()).unwrap();
    let all: Vec<&Wall> = walls.iter().collect();
    let controls = LockControls {
        padlock: true,
        reset: false,
    };
    let (points, _) = handle_points_among_with(wall, frame, &all, None, controls);
    points
        .into_iter()
        .map(|(handle, at)| {
            let local = frame.inverse().transform_point3(at);
            (handle, local.x, local.y)
        })
        .collect()
}

#[test]
fn a_wall_s_padlock_in_a_group_stands_clear_of_the_walls_joined_to_it_and_always_shows() {
    // Alone, it stands above the top left corner.
    let mut alone = vec![wall("a", (2, 1), 1.0, None, None).with_locked(true)];
    alone[0].pose = ahead(3.0);
    let (width, height) = alone[0].size();
    let (_, x, y) = padlock_on(&alone[0], &alone)[0];
    assert!(x < -width / 2.0 && y > height / 2.0, "{x} {y}");
    // With a wall joined across its top, below its bottom left corner, as the knob is.
    let stacked = group_of(
        ahead(3.0),
        vec![
            wall("a", (2, 1), 1.0, None, None),
            wall("b", (2, 1), 1.0, None, None),
        ],
        &[(0, Edge::Top, Align::Start)],
    );
    let points = padlock_on(&stacked[0], &stacked);
    let (_, x, y) = *points
        .iter()
        .find(|(handle, ..)| *handle == Handle::Lock)
        .unwrap();
    assert!(x < -width / 2.0 && y < -height / 2.0, "{x} {y}");
    // With a wall joined across its left, above its top right corner, as the handle of its columns moves.
    let beside = group_of(
        ahead(3.0),
        vec![
            wall("a", (2, 1), 1.0, None, None),
            wall("b", (2, 1), 1.0, None, None),
        ],
        &[(0, Edge::Left, Align::Start)],
    );
    let points = padlock_on(&beside[0], &beside);
    let (_, x, y) = *points
        .iter()
        .find(|(handle, ..)| *handle == Handle::Lock)
        .unwrap();
    assert!(x > width / 2.0 && y > height / 2.0, "{x} {y}");
    // Joined on every side, it still shows, so that a wall that will not change says why.
    let surrounded = group_of(
        ahead(3.0),
        vec![
            wall("a", (1, 1), 1.0, None, None),
            wall("l", (1, 1), 1.0, None, None),
            wall("r", (1, 1), 1.0, None, None),
            wall("t", (3, 1), 1.0, None, None),
            wall("b", (3, 1), 1.0, None, None),
        ],
        &[
            (0, Edge::Left, Align::Start),
            (0, Edge::Right, Align::Start),
            (1, Edge::Top, Align::Start),
            (1, Edge::Bottom, Align::Start),
        ],
    );
    let points = padlock_on(&surrounded[0].clone().with_locked(true), &surrounded);
    assert_eq!(
        points
            .iter()
            .map(|(handle, ..)| *handle)
            .collect::<Vec<_>>(),
        [Handle::Lock]
    );
}
