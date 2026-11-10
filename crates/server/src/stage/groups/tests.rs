// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::stage::{Content, Destination, WallChange};
use tomori_protocol::{Grid, Place, Projection, TOMORI_WALL, wall as layout};

/// Sky, with its Tomori wall to one side and an environment wall of its own.
const SKY: Walls<'static> = Walls {
    environment: "sky",
    tomori: Pose {
        position: [2.0, 1.5, -2.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    },
    declared: &[crate::EnvironmentWall::new(Pose {
        position: [-4.0, 1.5, -4.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    })],
};

/// Facing home from 2.4 m ahead, at `x`.
fn ahead(x: f32) -> Pose {
    Pose::upright_facing([x, 1.4, -2.4], [x, 1.4, 0.0])
}

/// A stage with walls of `grids`, each of its scale, added ahead of home, a metre and a half apart.
fn added(grids: &[((u32, u32), f32)]) -> (Stage, Vec<String>) {
    let mut stage = Stage::default();
    let ids = grids
        .iter()
        .enumerate()
        .map(|(index, ((columns, rows), scale))| {
            let layout = Layout {
                grid: Grid::new(*columns, *rows).unwrap(),
                scale: *scale,
                aspect: None,
                radius: None,
                tilt: None,
            };
            stage
                .add_wall(&SKY, &ahead(index as f32 * 3.0 - 3.0), Some(layout))
                .unwrap()
        })
        .collect();
    (stage, ids)
}

fn view(stage: &Stage) -> Projection {
    stage.view("s", &SKY, 0)
}

fn shown<'a>(projection: &'a Projection, wall: &str) -> &'a Wall {
    projection.wall(wall).unwrap()
}

fn right(to: &str) -> Option<Attach> {
    Some(Attach::new(to, Edge::Right, Align::Start))
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

/// Where `b`'s slots start to the right of `a`'s, on `a`'s surface, and how much higher their tops are.
fn beside(a: &Wall, b: &Wall) -> (f32, f32) {
    let (u, y, _, _) = group::on_surface(&a.pose, a.radius(), &b.pose);
    let (mine, theirs) = (a.extent(), b.extent().shifted(u, y));
    (theirs.left - mine.right, theirs.top - mine.top)
}

#[test]
fn a_wall_sent_without_the_wall_it_is_joined_to_is_sent_without_the_join() {
    let (mut stage, ids) = added(&[((2, 2), 0.95), ((1, 1), 1.9)]);
    assert!(
        stage
            .attach_wall(&SKY, &ids[1], right(&ids[0]), false, None)
            .unwrap()
    );
    assert_eq!(shown(&view(&stage), &ids[0]).joined.len(), 1);
    // A client that draws three walls is sent the Tomori wall, the environment's and the first added, its edge drawn.
    let three = tomori_protocol::Limits::MOST.with_projection(3, 780, 120, 900);
    let projection = stage.view_within("s", &SKY, 0, &three);
    assert_eq!(projection.walls.len(), 3);
    let first = shown(&projection, &ids[0]);
    assert!(first.joined.is_empty());
    assert_eq!(first.group.as_deref(), Some(ids[0].as_str()));
}

#[test]
fn a_wall_snapped_beside_another_joins_it_a_gutter_apart_kept_with_the_space() {
    let (mut stage, ids) = added(&[((2, 2), 0.95), ((1, 1), 1.9)]);
    let (two, big) = (&ids[0], &ids[1]);
    assert!(
        stage
            .attach_wall(&SKY, big, right(two), false, None)
            .unwrap()
    );
    let projection = view(&stage);
    let (a, b) = (shown(&projection, two), shown(&projection, big));
    // Their tops aligned, the gap between their slots a gutter at the larger scale, as the projection says it.
    let (gap, top) = beside(a, b);
    assert!(
        near(gap, layout::GUTTER * 1.9) && near(top, 0.0),
        "{gap} {top}"
    );
    assert_eq!(a.group.as_deref(), Some(two.as_str()));
    assert_eq!(b.group.as_deref(), Some(two.as_str()));
    assert_eq!(a.joined, [Join::new(Edge::Right, big.clone())]);
    assert_eq!(b.joined, [Join::new(Edge::Left, two.clone())]);
    // The Tomori wall and the environment's are in no group.
    assert!(shown(&projection, TOMORI_WALL).group.is_none());
    assert!(shown(&projection, "environment-0").joined.is_empty());
    // Snapping it again where it is changes nothing.
    assert!(
        !stage
            .attach_wall(&SKY, big, right(two), false, None)
            .unwrap()
    );
    // Kept across a restart, and read where it stood by a server that knows no groups.
    let kept = serde_json::to_value(&stage).unwrap();
    let again: Stage = serde_json::from_value(kept.clone()).unwrap();
    assert_eq!(view(&again), projection);
    let walls = kept["rooms"]["sky"]["walls"].as_array().unwrap();
    assert_eq!(walls[1]["attach"]["to"], two.as_str());
    let stood: Pose = serde_json::from_value(walls[1]["pose"].clone()).unwrap();
    assert_eq!(stood, b.pose);
}

#[test]
fn a_group_moves_together_as_any_of_its_walls_is_carried_its_panels_with_it() {
    let (mut stage, ids) = added(&[((2, 1), 1.0), ((1, 1), 1.0)]);
    stage
        .attach_wall(&SKY, &ids[1], right(&ids[0]), false, None)
        .unwrap();
    let to = Destination::Wall {
        wall: ids[0].clone(),
        slot: Some(1),
    };
    let panel = stage.project_to(&SKY, content("Build"), &to, 0);
    let before = view(&stage);
    // The second carried a metre left and turned to face +X: the first comes with it.
    let pose = Pose::upright_facing([-1.0, 1.4, -2.0], [3.0, 1.4, -2.0]);
    let change = WallChange {
        pose: Some(pose),
        ..WallChange::default()
    };
    assert!(stage.set_wall(&SKY, &ids[1], change).unwrap());
    let after = view(&stage);
    let (first, second) = (shown(&after, &ids[0]), shown(&after, &ids[1]));
    assert!(stands_at(&second.pose, &pose), "{:?}", second.pose);
    let alike = first
        .pose
        .orientation
        .iter()
        .zip(second.pose.orientation)
        .all(|(a, b)| near(*a, b));
    assert!(alike, "{first:?} {second:?}");
    let (gap, top) = beside(first, second);
    assert!(near(gap, layout::GUTTER) && near(top, 0.0));
    assert_eq!(first.joined.len(), 1);
    // The panel is in its slot, which has moved with its wall.
    let moved = after.panel(&panel).unwrap();
    assert_eq!(moved.place, before.panel(&panel).unwrap().place);
    let was = shown(&before, &ids[0]).slot_pose(1).unwrap();
    let now = first.slot_pose(1).unwrap();
    assert_ne!(was.position, now.position);
}

#[test]
fn a_group_stays_joined_as_its_walls_change_and_is_bent_alike() {
    let (mut stage, ids) = added(&[((2, 1), 1.0), ((1, 1), 1.0), ((1, 1), 1.0)]);
    stage
        .attach_wall(&SKY, &ids[1], right(&ids[0]), false, None)
        .unwrap();
    // A third column for the first: the second moves along, still a gutter away.
    let grid = WallChange {
        grid: Some(Grid::new(3, 1).unwrap()),
        ..WallChange::default()
    };
    assert!(stage.set_wall(&SKY, &ids[0], grid).unwrap());
    let projection = view(&stage);
    let (gap, _) = beside(shown(&projection, &ids[0]), shown(&projection, &ids[1]));
    assert!(near(gap, layout::GUTTER));
    // Bending one bends them all, along one arc, still joined.
    let bend = WallChange {
        radius: Some(Some(3.0)),
        ..WallChange::default()
    };
    assert!(stage.set_wall(&SKY, &ids[1], bend).unwrap());
    let projection = view(&stage);
    let (first, second) = (shown(&projection, &ids[0]), shown(&projection, &ids[1]));
    assert_eq!((first.radius(), second.radius()), (Some(3.0), Some(3.0)));
    assert!(group::same_surface(first, second));
    assert_eq!(first.joined, [Join::new(Edge::Right, ids[1].clone())]);
    let (gap, _) = beside(first, second);
    assert!(near(gap, first.gap()));
    // Flattened again, and then grown so far that it would reach into the third wall, it is refused, and stays.
    let flat = WallChange {
        radius: Some(None),
        ..WallChange::default()
    };
    stage.set_wall(&SKY, &ids[0], flat).unwrap();
    let pose = view(&stage).wall(&ids[1]).unwrap().pose;
    let third = Pose {
        position: [pose.position[0] + 1.4, pose.position[1], pose.position[2]],
        ..pose
    };
    let change = WallChange {
        pose: Some(third),
        ..WallChange::default()
    };
    stage.set_wall(&SKY, &ids[2], change).unwrap();
    let before = view(&stage);
    let wide = WallChange {
        grid: Some(Grid::new(6, 1).unwrap()),
        ..WallChange::default()
    };
    assert_eq!(
        stage.set_wall(&SKY, &ids[0], wide),
        Err("Walls cannot overlap: move it beside the other, or make one smaller.")
    );
    assert_eq!(view(&stage), before);
}

#[test]
fn a_wall_taken_out_leaves_the_rest_together_where_they_still_touch() {
    // Four in a square: the second right of the first, the third below the first, the fourth right of the third.
    let (mut stage, ids) = added(&[((1, 1), 1.0); 4]);
    let below = |to: &str| Some(Attach::new(to, Edge::Bottom, Align::Start));
    stage
        .attach_wall(&SKY, &ids[1], right(&ids[0]), false, None)
        .unwrap();
    stage
        .attach_wall(&SKY, &ids[2], below(&ids[0]), false, None)
        .unwrap();
    stage
        .attach_wall(&SKY, &ids[3], right(&ids[2]), false, None)
        .unwrap();
    let projection = view(&stage);
    assert!(
        ids.iter()
            .all(|id| shown(&projection, id).group.as_deref() == Some(ids[0].as_str()))
    );
    // The fourth is joined to the second above it and the third beside it.
    assert_eq!(shown(&projection, &ids[3]).joined.len(), 2);
    // The third taken out, where it is: the fourth stays, joined to the second, so the rest are still one group.
    assert!(stage.attach_wall(&SKY, &ids[2], None, false, None).unwrap());
    let after = view(&stage);
    assert!(shown(&after, &ids[2]).group.is_none());
    for id in [&ids[0], &ids[1], &ids[3]] {
        assert!(shown(&after, id).group.is_some(), "{id}");
        assert_eq!(shown(&after, id).pose, shown(&projection, id).pose);
    }
    assert_eq!(
        shown(&after, &ids[2]).pose,
        shown(&projection, &ids[2]).pose
    );
    // In a row of three, the middle taken out to stand elsewhere leaves the ends apart, each alone.
    let (mut stage, ids) = added(&[((1, 1), 1.0); 3]);
    stage
        .attach_wall(&SKY, &ids[1], right(&ids[0]), false, None)
        .unwrap();
    stage
        .attach_wall(&SKY, &ids[2], right(&ids[1]), false, None)
        .unwrap();
    let before = view(&stage);
    let away = Pose::upright_facing([0.0, 1.4, 3.0], [0.0, 1.4, 0.0]);
    assert!(
        stage
            .attach_wall(&SKY, &ids[1], None, true, Some(away))
            .unwrap()
    );
    let after = view(&stage);
    assert_eq!(shown(&after, &ids[1]).pose, away);
    for id in &ids {
        assert!(shown(&after, id).group.is_none());
    }
    assert_eq!(shown(&after, &ids[2]).pose, shown(&before, &ids[2]).pose);
    // Removing a wall of a group takes it out first.
    let (mut stage, ids) = added(&[((1, 1), 1.0); 3]);
    stage
        .attach_wall(&SKY, &ids[1], right(&ids[0]), false, None)
        .unwrap();
    stage
        .attach_wall(&SKY, &ids[2], right(&ids[1]), false, None)
        .unwrap();
    stage.remove_wall(&SKY, &ids[0]).unwrap();
    let after = view(&stage);
    assert_eq!(
        shown(&after, &ids[1]).group.as_deref(),
        Some(ids[1].as_str())
    );
    assert_eq!(
        shown(&after, &ids[2]).group.as_deref(),
        Some(ids[1].as_str())
    );
}

#[test]
fn a_group_carried_to_another_wall_snaps_whole_or_a_wall_alone() {
    let (mut stage, ids) = added(&[((1, 1), 1.0); 3]);
    // The first two a group, then the second snapped to the third, its group coming with it.
    stage
        .attach_wall(&SKY, &ids[1], right(&ids[0]), false, None)
        .unwrap();
    let left = Some(Attach::new(&ids[2], Edge::Left, Align::Start));
    stage
        .attach_wall(&SKY, &ids[1], left.clone(), false, None)
        .unwrap();
    let projection = view(&stage);
    let group = shown(&projection, &ids[2]).group.clone();
    assert!(group.is_some());
    assert!(ids.iter().all(|id| shown(&projection, id).group == group));
    let (gap, _) = beside(shown(&projection, &ids[0]), shown(&projection, &ids[1]));
    assert!(near(gap, layout::GUTTER));
    let (gap, _) = beside(shown(&projection, &ids[1]), shown(&projection, &ids[2]));
    assert!(near(gap, layout::GUTTER));
    // Snapped within its own group, above the first, it comes alone, and the rest stay where they touch.
    let above = Some(Attach::new(&ids[0], Edge::Top, Align::Start));
    stage
        .attach_wall(&SKY, &ids[2], above, false, None)
        .unwrap();
    let after = view(&stage);
    let joined = &shown(&after, &ids[0]).joined;
    assert_eq!(joined.len(), 2);
    assert!(joined.contains(&Join::new(Edge::Top, ids[2].clone())));
    assert!(joined.contains(&Join::new(Edge::Right, ids[1].clone())));
    assert_eq!(
        shown(&after, &ids[1]).pose,
        shown(&projection, &ids[1]).pose
    );
}

#[test]
fn only_added_walls_snap_without_overlapping_all_flat_or_curved_alike() {
    let (mut stage, ids) = added(&[((1, 1), 1.0), ((3, 2), 1.0), ((1, 1), 1.0)]);
    for (wall, to) in [
        (TOMORI_WALL, ids[0].as_str()),
        (ids[0].as_str(), "environment-0"),
    ] {
        assert_eq!(
            stage.attach_wall(&SKY, wall, right(to), false, None),
            Err(
                "Only walls someone added snap together: the Tomori wall and an environment's walls stand where they are put."
            )
        );
    }
    assert_eq!(
        stage.attach_wall(&SKY, "wall-99", right(&ids[0]), false, None),
        Err("That wall is not here.")
    );
    assert_eq!(
        stage.attach_wall(&SKY, &ids[0], right("wall-99"), false, None),
        Err("That wall is not here.")
    );
    assert_eq!(
        stage.attach_wall(&SKY, &ids[0], right(&ids[0]), false, None),
        Err("A wall cannot be attached to itself.")
    );
    let unknown = Some(Attach::new(&ids[1], Edge::Unknown, Align::Start));
    assert_eq!(
        stage.attach_wall(&SKY, &ids[0], unknown, false, None),
        Err("Tomori does not know that edge or alignment.")
    );
    let far = Pose::upright_facing([30.0, 1.4, 0.0], [0.0, 1.4, 0.0]);
    assert_eq!(
        stage.attach_wall(&SKY, &ids[0], None, false, Some(far)),
        Err("A wall must stand within 20 m of the space's middle.")
    );
    // The third stands where the first would go below the second, so that snap is refused, and nothing moves.
    let below = |to: &str| Some(Attach::new(to, Edge::Bottom, Align::Start));
    let target = shown(&view(&stage), &ids[1]).clone();
    let first = shown(&view(&stage), &ids[0]).clone();
    let there = group::attached_pose(&target, &first, Edge::Bottom, Align::Start).unwrap();
    stage
        .set_wall(
            &SKY,
            &ids[2],
            WallChange {
                pose: Some(there),
                ..WallChange::default()
            },
        )
        .unwrap();
    let before = view(&stage);
    assert_eq!(
        stage.attach_wall(&SKY, &ids[0], below(&ids[1]), false, None),
        Err("Walls cannot overlap: move it beside the other, or make one smaller.")
    );
    assert_eq!(view(&stage), before);
    // A curved wall and a flat one are refused.
    let bend = WallChange {
        radius: Some(Some(3.0)),
        ..WallChange::default()
    };
    stage.set_wall(&SKY, &ids[0], bend).unwrap();
    assert_eq!(
        stage.attach_wall(&SKY, &ids[0], right(&ids[1]), false, None),
        Err("Walls snap together only when all are flat, or all curved.")
    );
    // A group beyond reach is refused too.
    let (mut stage, ids) = added(&[((1, 1), 1.0); 2]);
    let edge = Pose::upright_facing([19.6, 1.4, 0.0], [19.6, 1.4, 5.0]);
    stage
        .set_wall(
            &SKY,
            &ids[0],
            WallChange {
                pose: Some(edge),
                ..WallChange::default()
            },
        )
        .unwrap();
    let away = Some(Attach::new(&ids[0], Edge::Right, Align::Start));
    assert_eq!(
        stage.attach_wall(&SKY, &ids[1], away, false, None),
        Err("A group's walls must stand within 20 m of the space's middle.")
    );
    // Carried by a wall within reach so far that its group's first would stand beyond it, it is refused, and nothing
    // moves.
    let (mut stage, ids) = added(&[((1, 1), 1.0); 2]);
    stage
        .attach_wall(&SKY, &ids[1], right(&ids[0]), false, None)
        .unwrap();
    let before = view(&stage);
    let edge = Pose::upright_facing([-19.8, 1.4, -2.4], [-19.8, 1.4, 0.0]);
    assert_eq!(
        stage.set_wall(
            &SKY,
            &ids[1],
            WallChange {
                pose: Some(edge),
                ..WallChange::default()
            },
        ),
        Err("A group's walls must stand within 20 m of the space's middle.")
    );
    assert_eq!(view(&stage), before);
}

#[test]
fn an_attachment_to_a_wall_not_here_or_round_in_a_circle_is_forgotten() {
    let (mut stage, ids) = added(&[((1, 1), 1.0); 2]);
    let room = stage.rooms.get_mut("sky").unwrap();
    room.walls[0].attach = right(&ids[1]);
    room.walls[1].attach = right(&ids[0]);
    let before = room.clone();
    room.settle(&before).unwrap();
    assert_eq!(
        room.walls
            .iter()
            .filter(|wall| wall.attach.is_some())
            .count(),
        1
    );
    room.walls[0].attach = right("wall-99");
    room.walls[1].attach = None;
    room.settle(&before).unwrap();
    assert!(room.walls.iter().all(|wall| wall.attach.is_none()));
    // One kept by a newer server, across an edge this one does not know.
    room.walls[1].attach = Some(Attach::new(&ids[0], Edge::Unknown, Align::Start));
    room.settle(&before).unwrap();
    assert!(room.walls[1].attach.is_none());
}

#[test]
fn a_wall_curved_at_another_radius_snaps_onto_the_arc_and_a_group_bent_too_far_for_one_of_its_walls_is_not_bent()
 {
    let (mut stage, ids) = added(&[((1, 1), 1.0), ((8, 1), 1.0), ((1, 1), 1.0), ((2, 1), 1.5)]);
    let bend = |radius: f32| WallChange {
        radius: Some(Some(radius)),
        ..WallChange::default()
    };
    stage.set_wall(&SKY, &ids[0], bend(3.0)).unwrap();
    stage.set_wall(&SKY, &ids[2], bend(4.0)).unwrap();
    stage.set_wall(&SKY, &ids[3], bend(3.02)).unwrap();
    // Snapped beside it, each takes its radius exactly, larger slots or not, and stands about the same line: one arc.
    assert!(
        stage
            .attach_wall(&SKY, &ids[2], right(&ids[0]), false, None)
            .unwrap()
    );
    stage
        .attach_wall(&SKY, &ids[3], right(&ids[2]), false, None)
        .unwrap();
    let projection = view(&stage);
    let first = shown(&projection, &ids[0]);
    for id in &ids[2..] {
        let wall = shown(&projection, id);
        assert_eq!(wall.radius(), Some(3.0));
        let (_, _, depth, turn) = group::on_surface(&first.pose, Some(3.0), &wall.pose);
        assert!(depth.abs() < 1e-3 && turn.abs() < 1e-3, "{depth} {turn}");
    }
    let (gap, _) = beside(first, shown(&projection, &ids[2]));
    assert!(near(gap, first.gap()), "{gap}");
    // Bending the group bends them all alike, still about one line.
    stage.set_wall(&SKY, &ids[3], bend(2.5)).unwrap();
    let projection = view(&stage);
    let first = shown(&projection, &ids[0]);
    for id in &ids[2..] {
        let wall = shown(&projection, id);
        assert_eq!(wall.radius(), Some(2.5));
        let (_, _, depth, turn) = group::on_surface(&first.pose, Some(2.5), &wall.pose);
        assert!(depth.abs() < 1e-3 && turn.abs() < 1e-3, "{depth} {turn}");
    }
    // A wide wall snapped to a curve too tight for it is refused, and nothing moves.
    let (mut stage, ids) = added(&[((1, 1), 1.0), ((8, 1), 1.0)]);
    stage.set_wall(&SKY, &ids[0], bend(1.0)).unwrap();
    stage.set_wall(&SKY, &ids[1], bend(5.0)).unwrap();
    let before = view(&stage);
    assert_eq!(
        stage.attach_wall(&SKY, &ids[1], right(&ids[0]), false, None),
        Err(
            "Curved that tightly, its slots would go around more than half a circle: curve it less, or give it fewer columns."
        )
    );
    assert_eq!(view(&stage), before);
    let (mut stage, ids) = added(&[((1, 1), 1.0), ((8, 1), 1.0)]);
    // The first flat again, snapped beside the wide one: bending the group as tightly as the first could be bent would
    // take the wide one round more than half a circle.
    stage
        .set_wall(
            &SKY,
            &ids[0],
            WallChange {
                radius: Some(None),
                ..WallChange::default()
            },
        )
        .unwrap();
    stage
        .attach_wall(&SKY, &ids[0], right(&ids[1]), false, None)
        .unwrap();
    let before = view(&stage);
    assert_eq!(
        stage.set_wall(&SKY, &ids[0], bend(1.0)),
        Err(
            "Curved that tightly, its slots would go around more than half a circle: curve it less, or give it fewer columns."
        )
    );
    assert_eq!(view(&stage), before);
}

fn content(title: &str) -> Content {
    Content {
        title: title.into(),
        body: "Green.".into(),
        meta: Vec::new(),
        code: None,
        slug: None,
        image: None,
    }
}

#[test]
fn a_panel_on_a_wall_of_a_group_stays_in_its_slot_as_the_group_moves() {
    let (mut stage, ids) = added(&[((1, 1), 1.0); 2]);
    stage
        .attach_wall(&SKY, &ids[1], right(&ids[0]), false, None)
        .unwrap();
    let to = Destination::Wall {
        wall: ids[1].clone(),
        slot: None,
    };
    let panel = stage.project_to(&SKY, content("Tests"), &to, 0);
    let away = Pose::upright_facing([1.0, 1.4, -3.0], [1.0, 1.4, 0.0]);
    stage
        .set_wall(
            &SKY,
            &ids[0],
            WallChange {
                pose: Some(away),
                ..WallChange::default()
            },
        )
        .unwrap();
    let projection = view(&stage);
    assert_eq!(
        projection.panel(&panel).unwrap().place,
        Place::Wall {
            wall: ids[1].clone(),
            slot: 0
        }
    );
}

#[test]
fn a_group_s_walls_stand_upright_a_tilted_wall_snapped_standing_upright_and_none_snapped_to_a_tilted_one()
 {
    let (mut stage, ids) = added(&[((2, 1), 1.0), ((1, 1), 1.0), ((1, 1), 1.0)]);
    let tilted = |tilt: f32| WallChange {
        tilt: Some(Some(tilt)),
        ..WallChange::default()
    };
    // A wall tilted has none snapped to it, saying why, and nothing changes.
    assert_eq!(stage.set_wall(&SKY, &ids[0], tilted(20.0)), Ok(true));
    assert_eq!(
        stage.attach_wall(&SKY, &ids[1], right(&ids[0]), false, None),
        Err(
            "A group's walls stand upright, so a tilted wall has none snapped to it: stand it upright first."
        )
    );
    assert!(shown(&view(&stage), &ids[1]).group.is_none());
    // A tilted wall snapped to an upright one stands upright, kept so, as the group reads as one board.
    assert_eq!(stage.set_wall(&SKY, &ids[0], tilted(0.0)), Ok(true));
    assert_eq!(stage.set_wall(&SKY, &ids[1], tilted(-30.0)), Ok(true));
    assert!(
        stage
            .attach_wall(&SKY, &ids[1], right(&ids[0]), false, None)
            .unwrap()
    );
    let projection = view(&stage);
    assert_eq!(shown(&projection, &ids[1]).tilt, None);
    assert_eq!(
        stage.rooms["sky"].walls[1]
            .layout
            .and_then(|layout| layout.tilt),
        None
    );
    let (gap, top) = beside(shown(&projection, &ids[0]), shown(&projection, &ids[1]));
    assert!(near(gap, layout::GUTTER) && near(top, 0.0), "{gap} {top}");
    // A wall of a group is not tilted, saying why, and nothing changes; taken out of it, it tilts.
    for wall in &ids[..2] {
        assert_eq!(
            stage.set_wall(&SKY, wall, tilted(10.0)),
            Err(
                "A group's walls stand upright, so that it reads as one board: take this wall out of its group to tilt it."
            )
        );
    }
    assert_eq!(view(&stage), projection);
    assert!(stage.attach_wall(&SKY, &ids[1], None, false, None).unwrap());
    assert_eq!(stage.set_wall(&SKY, &ids[1], tilted(10.0)), Ok(true));
    assert_eq!(shown(&view(&stage), &ids[1]).tilt(), 10.0);
    // The wall alone it snaps to may be tilted and is then made upright with it, as its layout says.
    assert_eq!(stage.set_wall(&SKY, &ids[2], tilted(15.0)), Ok(true));
    let mut room = stage.rooms["sky"].clone();
    room.walls[2].attach = Some(Attach::new(ids[1].clone(), Edge::Right, Align::Start));
    room.walls[1].attach = None;
    let before = stage.rooms["sky"].clone();
    room.settle(&before).unwrap();
    assert!(
        room.walls[1..]
            .iter()
            .all(|wall| wall.layout.is_none_or(|layout| layout.tilt.is_none()))
    );
    // A wall laid out as every wall is unless changed, tilted and snapped, is kept as laid out so, upright.
    let flat = stage.add_wall(&SKY, &ahead(9.0), None).unwrap();
    assert_eq!(stage.set_wall(&SKY, &flat, tilted(25.0)), Ok(true));
    assert_eq!(stage.set_wall(&SKY, &ids[2], tilted(0.0)), Ok(true));
    assert!(
        stage
            .attach_wall(&SKY, &flat, right(&ids[2]), false, None)
            .unwrap()
    );
    assert_eq!(stage.rooms["sky"].walls[3].layout, None);
    assert_eq!(shown(&view(&stage), &flat).tilt, None);
}

#[test]
fn a_locked_wall_is_neither_attached_nor_moved_with_its_group() {
    let (mut stage, ids) = added(&[((2, 1), 1.0), ((1, 1), 1.0), ((1, 1), 1.0)]);
    let (first, second, third) = (&ids[0], &ids[1], &ids[2]);
    stage
        .attach_wall(&SKY, second, right(first), false, None)
        .unwrap();
    // The group's first wall locked, the second may not carry it, nor bend it, and stays where it was.
    assert_eq!(stage.lock_wall(&SKY, first, true, false), Ok(true));
    let before = view(&stage);
    let carried = WallChange {
        pose: Some(ahead(-1.0)),
        ..WallChange::default()
    };
    assert_eq!(
        stage.set_wall(&SKY, second, carried),
        Err(super::super::GROUP_LOCKED)
    );
    let bent = WallChange {
        radius: Some(Some(3.0)),
        ..WallChange::default()
    };
    assert_eq!(
        stage.set_wall(&SKY, second, bent),
        Err(super::super::GROUP_LOCKED)
    );
    assert_eq!(view(&stage).walls, before.walls);
    // What moves no locked wall still changes: the second, attached to it, is given another column.
    let wider = WallChange {
        grid: Some(Grid::from((2, 1))),
        ..WallChange::default()
    };
    assert_eq!(stage.set_wall(&SKY, second, wider), Ok(true));
    assert_eq!(shown(&view(&stage), first).pose, shown(&before, first).pose);
    // A locked wall is neither attached nor taken out; nor is its group carried to another wall.
    for attach in [right(third), None] {
        assert_eq!(
            stage.attach_wall(&SKY, first, attach, false, None),
            Err(super::super::LOCKED)
        );
    }
    assert_eq!(
        stage.attach_wall(
            &SKY,
            second,
            Some(Attach::new(third, Edge::Bottom, Align::Start)),
            false,
            None
        ),
        Err(super::super::GROUP_LOCKED)
    );
    assert_eq!(shown(&view(&stage), first).pose, shown(&before, first).pose);
    // Unlocked, the group is carried again.
    assert_eq!(stage.lock_wall(&SKY, first, false, false), Ok(true));
    assert_eq!(stage.set_wall(&SKY, second, carried), Ok(true));
}
