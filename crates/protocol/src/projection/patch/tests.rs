// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::{Code, Grid, Meta, Pose, TOMORI_WALL, WallKind, WallShape};

fn pose(x: f32) -> Pose {
    Pose {
        position: [x, 1.5, -2.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    }
}

fn on(wall: &str, slot: u32) -> Place {
    Place::Wall {
        wall: wall.into(),
        slot,
    }
}

fn panel(id: &str, body: &str, place: Place) -> Panel {
    Panel::new(id, id, body, place).with_revision(1)
}

fn projection(walls: Vec<Wall>, panels: Vec<Panel>, revision: u64) -> Projection {
    Projection::new("s", "sky")
        .with_walls(walls)
        .with_panels(panels)
        .with_revision(revision)
}

fn tomori() -> Wall {
    Wall::new(TOMORI_WALL, WallKind::Tomori, pose(0.0))
}

fn added(id: &str, x: f32) -> Wall {
    Wall::new(id, WallKind::Added, pose(x))
}

/// The ops of the patch from `before` to `after`, which it applies to make `after`.
fn ops(before: &Projection, after: &Projection) -> Vec<PatchOp> {
    let patch = ProjectionPatch::between(before, after);
    let mut applied = before.clone();
    patch.apply(&mut applied).unwrap();
    assert_eq!(without_ages(applied), without_ages(after.clone()));
    patch.ops
}

/// `projection`, with every panel's age the same, as a client counts each on from when it was sent.
fn without_ages(mut projection: Projection) -> Projection {
    for panel in &mut projection.panels {
        panel.age = 0;
    }
    projection
}

#[test]
fn a_patch_has_a_stable_wire_format_and_reads_a_newer_op_as_unknown() {
    let patch = ProjectionPatch::new(
        "s",
        "sky",
        4,
        6,
        vec![
            PatchOp::RemovePanel {
                panel: "panel-1".into(),
            },
            PatchOp::Place {
                panel: "panel-2".into(),
                place: on(TOMORI_WALL, 3),
                pinned: true,
                after: Some("panel-3".into()),
                first: false,
            },
            PatchOp::RemoveWall {
                wall: "wall-2".into(),
            },
        ],
    );
    let json = serde_json::json!({
        "space": "s", "environment": "sky", "from": 4, "to": 6,
        "ops": [
            {"op": "remove_panel", "panel": "panel-1"},
            {"op": "place", "panel": "panel-2", "place": {"kind": "wall", "wall": "tomori", "slot": 3},
                "pinned": true, "after": "panel-3"},
            {"op": "remove_wall", "wall": "wall-2"},
        ]
    });
    assert_eq!(serde_json::to_value(&patch).unwrap(), json);
    assert_eq!(
        serde_json::from_value::<ProjectionPatch>(json).unwrap(),
        patch
    );
    // A patch that changes nothing says so in as little.
    assert_eq!(
        serde_json::to_string(&ProjectionPatch::new("s", "sky", 1, 2, Vec::new())).unwrap(),
        r#"{"space":"s","environment":"sky","from":1,"to":2}"#
    );
    let newer: ProjectionPatch = serde_json::from_value(serde_json::json!({
        "space": "s", "environment": "sky", "from": 1, "to": 2,
        "ops": [{"op": "series_append", "panel": "chart", "points": [1, 2]}]
    }))
    .unwrap();
    assert_eq!(newer.ops, vec![PatchOp::Unknown]);
    let mut shown = projection(vec![tomori()], Vec::new(), 1);
    assert_eq!(newer.apply(&mut shown), Err(PatchError::Unknown));
    assert_eq!(shown.revision, 1);
}

#[test]
fn every_kind_of_change_makes_its_op() {
    let base = projection(
        vec![tomori(), added("wall-1", 2.0)],
        vec![
            panel("panel-1", "One", on(TOMORI_WALL, 0)),
            panel("panel-2", "Two", on(TOMORI_WALL, 1)),
        ],
        3,
    );
    let mut next = base.clone();
    next.revision = 4;
    // Nothing changed: the patch only says the revision, so that a client undoes what it showed at once.
    assert_eq!(ops(&base, &next), Vec::new());

    // A wall added goes last; one changed is sent whole, in its place; one removed is named.
    let mut walls = next.clone();
    walls.walls.push(added("wall-2", 4.0));
    assert_eq!(
        ops(&base, &walls),
        vec![PatchOp::Wall {
            wall: added("wall-2", 4.0),
            after: Some("wall-1".into()),
            first: false,
        }]
    );
    let mut grid = next.clone();
    grid.walls[1] = added("wall-1", 2.0)
        .with_grid(Grid::from((4, 1)))
        .with_scale(1.5)
        .with_tilt(Some(10.0))
        .with_locked(true)
        .with_shape(WallShape::Arc { radius: 3.0 })
        .with_group(Some("wall-1".into()), Vec::new());
    assert_eq!(
        ops(&base, &grid),
        vec![PatchOp::Wall {
            wall: grid.walls[1].clone(),
            after: None,
            first: false,
        }]
    );
    let mut removed = next.clone();
    removed.walls.pop();
    removed.panels[1].place = Place::Free { pose: pose(2.0) };
    assert_eq!(
        ops(&base, &removed),
        vec![
            PatchOp::RemoveWall {
                wall: "wall-1".into()
            },
            PatchOp::Place {
                panel: "panel-2".into(),
                place: Place::Free { pose: pose(2.0) },
                pinned: false,
                after: None,
                first: false,
            },
        ]
    );

    // A panel whose content changed is sent whole; one that only moved, or was pinned, is moved; a new one is sent
    // after the one before it; one dismissed is named.
    let mut content = next.clone();
    content.panels[0].body = "One, ticked".into();
    content.panels[0].revision = 2;
    content.panels[0].age = 30;
    assert_eq!(
        ops(&base, &content),
        vec![PatchOp::Panel {
            panel: content.panels[0].clone(),
            after: None,
            first: false,
        }]
    );
    let mut aged = next.clone();
    aged.panels[0].age = 30;
    assert_eq!(ops(&base, &aged), Vec::new(), "age alone is no change");
    let mut moved = next.clone();
    moved.panels[1].place = on("wall-1", 2);
    moved.panels[1].pinned = true;
    assert_eq!(
        ops(&base, &moved),
        vec![PatchOp::Place {
            panel: "panel-2".into(),
            place: on("wall-1", 2),
            pinned: true,
            after: None,
            first: false,
        }]
    );
    let mut new = next.clone();
    new.panels
        .insert(0, panel("panel-3", "Three", on(TOMORI_WALL, 2)));
    new.panels.remove(2);
    assert_eq!(
        ops(&base, &new),
        vec![
            PatchOp::RemovePanel {
                panel: "panel-2".into()
            },
            PatchOp::Panel {
                panel: new.panels[0].clone(),
                after: None,
                first: true,
            },
        ]
    );
    // A panel moved to the end of the list, as a panel placed is, is one op, its content not sent again.
    let mut three = next.clone();
    three
        .panels
        .push(panel("panel-3", "Three", on(TOMORI_WALL, 2)));
    let mut last = three.clone().with_revision(5);
    last.panels.rotate_left(1);
    assert_eq!(
        ops(&three, &last),
        vec![PatchOp::Place {
            panel: "panel-1".into(),
            place: on(TOMORI_WALL, 0),
            pinned: false,
            after: Some("panel-3".into()),
            first: false,
        }]
    );
}

#[test]
fn a_patch_applies_only_to_its_projection_at_its_revision() {
    let base = projection(
        vec![tomori()],
        vec![panel("panel-1", "One", on(TOMORI_WALL, 0))],
        3,
    );
    let mut next = base.clone().with_revision(5);
    next.panels[0].body = "Changed".into();
    let patch = ProjectionPatch::between(&base, &next);
    assert_eq!((patch.from, patch.to), (3, 5));

    let mut later = base.clone().with_revision(4);
    assert_eq!(
        patch.apply(&mut later),
        Err(PatchError::Revision { have: 4, from: 3 })
    );
    assert_eq!(later, base.clone().with_revision(4), "nothing changed");
    let mut other = Projection::new("s", "grid").with_revision(3);
    assert_eq!(patch.apply(&mut other), Err(PatchError::Elsewhere));
    let mut other = Projection::new("t", "sky").with_revision(3);
    assert_eq!(patch.apply(&mut other), Err(PatchError::Elsewhere));

    // One naming what is not there is refused, its revision kept, for the client to ask for the whole projection.
    let mut missing = projection(vec![tomori()], Vec::new(), 3);
    let remove = ProjectionPatch::new(
        "s",
        "sky",
        3,
        4,
        vec![PatchOp::RemovePanel {
            panel: "panel-1".into(),
        }],
    );
    let error = remove.apply(&mut missing).unwrap_err();
    assert_eq!(error, PatchError::Missing("panel-1".into()));
    assert_eq!(missing.revision, 3);
    assert!(error.to_string().contains("panel-1"));
    for (op, missing) in [
        (
            PatchOp::RemoveWall {
                wall: "wall-9".into(),
            },
            "wall-9",
        ),
        (
            PatchOp::Panel {
                panel: panel("panel-2", "Two", on(TOMORI_WALL, 1)),
                after: Some("panel-9".into()),
                first: false,
            },
            "panel-9",
        ),
        (
            PatchOp::Place {
                panel: "panel-9".into(),
                place: on(TOMORI_WALL, 1),
                pinned: false,
                after: None,
                first: false,
            },
            "panel-9",
        ),
        (
            PatchOp::Place {
                panel: "panel-1".into(),
                place: on(TOMORI_WALL, 1),
                pinned: false,
                after: Some("panel-8".into()),
                first: false,
            },
            "panel-8",
        ),
        (
            PatchOp::Wall {
                wall: added("wall-1", 1.0),
                after: Some("wall-9".into()),
                first: false,
            },
            "wall-9",
        ),
    ] {
        let mut shown = base.clone();
        let patch = ProjectionPatch::new("s", "sky", 3, 4, vec![op]);
        assert_eq!(
            patch.apply(&mut shown),
            Err(PatchError::Missing(missing.into()))
        );
    }
    for error in [
        PatchError::Elsewhere,
        PatchError::Revision { have: 1, from: 2 },
        PatchError::Unknown,
    ] {
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn a_wall_or_panel_new_without_where_it_stands_goes_last() {
    let mut shown = projection(
        vec![tomori()],
        vec![panel("panel-1", "One", on(TOMORI_WALL, 0))],
        1,
    );
    let patch = ProjectionPatch::new(
        "s",
        "sky",
        1,
        2,
        vec![
            PatchOp::Wall {
                wall: added("wall-1", 1.0),
                after: None,
                first: false,
            },
            PatchOp::Panel {
                panel: panel("panel-2", "Two", on("wall-1", 0)),
                after: None,
                first: false,
            },
        ],
    );
    assert_eq!(patch.apply(&mut shown), Ok(()));
    assert_eq!(
        shown,
        projection(
            vec![tomori(), added("wall-1", 1.0)],
            vec![
                panel("panel-1", "One", on(TOMORI_WALL, 0)),
                panel("panel-2", "Two", on("wall-1", 0)),
            ],
            2
        )
    );
}

#[test]
fn a_patch_counts_what_it_sends_whole() {
    let base = projection(
        vec![tomori()],
        vec![panel("panel-1", "One", on(TOMORI_WALL, 0))],
        1,
    );
    let mut next = base.clone().with_revision(2);
    next.walls.push(added("wall-1", 1.0));
    next.panels[0].place = on("wall-1", 0);
    next.panels
        .push(panel("panel-2", "Two", on(TOMORI_WALL, 0)));
    let patch = ProjectionPatch::between(&base, &next);
    assert_eq!(patch.whole(), 2, "{:?}", patch.ops);
}

/// A small, fast generator of numbers, seeded, so that each run tries the same sequences.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

/// `projection`, changed once at random, as the server's stage changes it: a wall added, removed, reshaped, moved,
/// locked or grouped; a panel projected, dismissed, updated, ticked, moved, pinned, put elsewhere in the list or aged.
fn mutate(projection: &mut Projection, random: &mut Random, serial: &mut u32) {
    *serial += 1;
    let walls = projection.walls.len();
    let panels = projection.panels.len();
    let place = |random: &mut Random, walls: &[Wall]| {
        if random.below(4) == 0 {
            Place::Free {
                pose: pose(random.below(10) as f32),
            }
        } else {
            on(&walls[random.below(walls.len())].id, random.below(6) as u32)
        }
    };
    match random.below(12) {
        0 if walls < 8 => {
            let wall = added(&format!("wall-{serial}"), *serial as f32);
            let at = 1 + random.below(walls);
            projection.walls.insert(at, wall);
        }
        1 if walls > 1 => {
            let index = 1 + random.below(walls - 1);
            let gone = projection.walls.remove(index).id;
            for panel in &mut projection.panels {
                if matches!(&panel.place, Place::Wall { wall, .. } if *wall == gone) {
                    panel.place = Place::Free { pose: pose(1.0) };
                }
            }
        }
        2 => {
            let wall = &mut projection.walls[random.below(walls)];
            *wall = wall
                .clone()
                .with_grid(Grid::from((
                    1 + random.below(8) as u32,
                    1 + random.below(3) as u32,
                )))
                .with_tilt(Some(random.below(30) as f32))
                .with_locked(random.below(2) == 0);
        }
        3 => {
            let index = random.below(walls);
            projection.walls[index].pose = pose(random.below(100) as f32);
            if random.below(2) == 0 && walls > 1 {
                let group = projection.walls[0].id.clone();
                projection.walls[index].group = Some(group);
            }
        }
        4 | 5 if panels < 40 => {
            let mut new = panel(
                &format!("panel-{serial}"),
                &"text ".repeat(random.below(50)),
                place(random, &projection.walls),
            );
            if random.below(3) == 0 {
                new = new.with_code(Code::new("rust", 3));
            }
            new = new.with_meta(vec![Meta::new("source", "CI")]);
            let at = random.below(panels + 1);
            projection.panels.insert(at, new);
        }
        6 if panels > 0 => {
            projection.panels.remove(random.below(panels));
        }
        7 if panels > 0 => {
            let panel = &mut projection.panels[random.below(panels)];
            panel.body.push_str(" more");
            panel.revision += 1;
        }
        8 if panels > 0 => {
            let index = random.below(panels);
            projection.panels[index].place = place(random, &projection.walls);
            projection.panels[index].pinned = random.below(2) == 0;
        }
        9 if panels > 1 => {
            // Put elsewhere in the list, as a panel placed goes last.
            let moved = projection.panels.remove(random.below(panels));
            let at = random.below(panels);
            projection.panels.insert(at, moved);
        }
        10 if panels > 0 => {
            projection.panels[random.below(panels)].age += 1;
        }
        11 if walls > 2 => {
            projection.walls[1..].rotate_left(1);
        }
        _ => {}
    }
}

#[test]
fn patches_applied_in_sequence_are_the_projection_sent_whole() {
    for seed in 1..=200u64 {
        let mut random = Random(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut serial = 0;
        let mut server = projection(vec![tomori()], Vec::new(), 1);
        let mut client = server.clone();
        for step in 0..60 {
            // Several changes at once, as a burst before the server sends, coalesce into one patch.
            let before = server.clone();
            for _ in 0..1 + random.below(4) {
                mutate(&mut server, &mut random, &mut serial);
            }
            server.revision += 1;
            let patch = ProjectionPatch::between(&before, &server);
            // Each patch goes over the wire.
            let patch: ProjectionPatch =
                serde_json::from_str(&serde_json::to_string(&patch).unwrap()).unwrap();
            assert_eq!(patch.apply(&mut client), Ok(()), "seed {seed}, step {step}");
            assert_eq!(
                without_ages(client.clone()),
                without_ages(server.clone()),
                "seed {seed}, step {step}"
            );
        }
    }
}

#[test]
fn a_patch_over_several_revisions_is_their_patches_together() {
    let mut random = Random(77);
    let mut serial = 0;
    let first = projection(vec![tomori()], Vec::new(), 1);
    let mut server = first.clone();
    for _ in 0..40 {
        mutate(&mut server, &mut random, &mut serial);
        server.revision += 1;
    }
    let mut client = first.clone();
    ProjectionPatch::between(&first, &server)
        .apply(&mut client)
        .unwrap();
    assert_eq!(without_ages(client), without_ages(server));
}

#[test]
fn the_longest_run_kept_in_order_stays_where_it_is() {
    assert_eq!(longest_increasing(&[]), Vec::<usize>::new());
    assert_eq!(longest_increasing(&[2, 0, 1, 3]), vec![0, 1, 3]);
    assert_eq!(longest_increasing(&[1, 2, 3, 0]), vec![1, 2, 3]);
    assert_eq!(longest_increasing(&[3, 2, 1]).len(), 1);
}
