// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::chart::{Chart, Colour, Series, SeriesStyle, Tone};
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
    Panel::new(id, id, body, place).with_version(1)
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
                version: 5,
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
                "pinned": true, "after": "panel-3", "revision": 5},
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
                version: 1,
            },
        ]
    );

    // A panel whose content changed is sent whole; one that only moved, or was pinned, is moved; a new one is sent
    // after the one before it; one dismissed is named.
    let mut content = next.clone();
    content.panels[0].body = "One, ticked".into();
    content.panels[0].version = 2;
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
            version: 1,
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
            version: 1,
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
                version: 1,
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
                version: 1,
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
        assert_eq!(shown, base, "nothing changed");
    }
    // One refused partway, after ops that applied, leaves the projection as it was, not half patched.
    let mut shown = base.clone();
    let partway = ProjectionPatch::new(
        "s",
        "sky",
        3,
        4,
        vec![
            PatchOp::Wall {
                wall: added("wall-1", 1.0),
                after: None,
                first: false,
            },
            PatchOp::Panel {
                panel: panel("panel-2", "Two", on(TOMORI_WALL, 1)),
                after: None,
                first: false,
            },
            PatchOp::RemovePanel {
                panel: "panel-1".into(),
            },
            PatchOp::Unknown,
        ],
    );
    assert_eq!(partway.apply(&mut shown), Err(PatchError::Unknown));
    assert_eq!(shown, base, "nothing changed");
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

/// A chart of `points` points with `series` series, each value its series and point, moved along by `along`.
fn chart(points: usize, series: usize, along: usize) -> Chart {
    let mut chart = Chart::new(points as u32);
    for index in 0..series {
        chart = chart.with_series(Series::new(
            format!("series {index}"),
            Colour::Teal,
            SeriesStyle::Line,
            (0..points)
                .map(|point| Some((index * 1000 + point + along) as f32))
                .collect(),
        ));
    }
    chart
}

/// A chart panel of `chart`, its live data at `seq`.
fn charted(id: &str, chart: Chart, seq: u64) -> Panel {
    Panel {
        body: chart.summary(),
        ..panel(id, "", on(TOMORI_WALL, 0))
    }
    .with_chart(chart)
    .with_data_seq(seq)
}

/// `projection`, changed once at random, as the server's stage changes it, at the space's revision `revision`: a wall
/// added, removed, reshaped, moved, locked or grouped, its version then `revision`; a panel projected, dismissed,
/// updated, ticked, moved, pinned, its version then `revision`, or put elsewhere in the list or aged, which changes no
/// version; or a chart panel's live data moved along or replaced, its data seq then `revision` and its version as it
/// was.
fn mutate(projection: &mut Projection, random: &mut Random, serial: &mut u32, revision: u64) {
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
    let charts: Vec<usize> = (0..panels)
        .filter(|index| projection.panels[*index].chart.is_some())
        .collect();
    match random.below(15) {
        0 if walls < 8 => {
            let wall = added(&format!("wall-{serial}"), *serial as f32).with_version(revision);
            let at = 1 + random.below(walls);
            projection.walls.insert(at, wall);
        }
        1 if walls > 1 => {
            let index = 1 + random.below(walls - 1);
            let gone = projection.walls.remove(index).id;
            for panel in &mut projection.panels {
                if matches!(&panel.place, Place::Wall { wall, .. } if *wall == gone) {
                    panel.place = Place::Free { pose: pose(1.0) };
                    panel.version = revision;
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
                .with_locked(random.below(2) == 0)
                .with_version(revision);
        }
        3 => {
            let index = random.below(walls);
            projection.walls[index].pose = pose(random.below(100) as f32);
            if random.below(2) == 0 && walls > 1 {
                let group = projection.walls[0].id.clone();
                projection.walls[index].group = Some(group);
            }
            projection.walls[index].version = revision;
        }
        4 | 5 if panels < 40 => {
            let mut new = panel(
                &format!("panel-{serial}"),
                &"text ".repeat(random.below(50)),
                place(random, &projection.walls),
            )
            .with_version(revision);
            match random.below(4) {
                0 => new = new.with_code(Code::new("rust", 3)),
                1 => {
                    let points = 4 + random.below(40);
                    let chart = chart(points, 1 + random.below(4), 0);
                    new.body = chart.summary();
                    new = new.with_chart(chart).with_data_seq(revision);
                }
                _ => {}
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
            panel.title.push_str(" more");
            if panel.chart.is_none() {
                panel.body.push_str(" more");
            }
            panel.version = revision;
        }
        8 if panels > 0 => {
            let index = random.below(panels);
            projection.panels[index].place = place(random, &projection.walls);
            projection.panels[index].pinned = random.below(2) == 0;
            projection.panels[index].version = revision;
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
        // A chart's points move along, as a live chart's do, sometimes with a break and a new status line.
        12 | 13 if !charts.is_empty() => {
            let panel = &mut projection.panels[charts[random.below(charts.len())]];
            let chart = panel.chart.as_mut().unwrap();
            let points = chart.points as usize;
            let along = 1 + random.below(points - 1);
            for series in &mut chart.series {
                series.values.drain(..along);
                for _ in 0..along {
                    series
                        .values
                        .push((random.below(5) > 0).then_some(random.below(1000) as f32 / 10.0));
                }
                series.breaks = series
                    .breaks
                    .iter()
                    .filter(|at| **at as usize >= along)
                    .map(|at| at - along as u32)
                    .collect();
                if random.below(3) == 0 {
                    series.breaks.push(points as u32 - 2);
                    series.breaks.dedup();
                }
            }
            chart.status = (random.below(2) == 0)
                .then(|| crate::chart::Note::new(format!("{} of 8", random.below(9)), Tone::Good));
            panel.body = chart.summary();
            panel.data_seq = revision;
        }
        14 if !charts.is_empty() => {
            let panel = &mut projection.panels[charts[random.below(charts.len())]];
            let chart = panel.chart.as_mut().unwrap();
            for series in &mut chart.series {
                for value in &mut series.values {
                    *value = Some(random.below(100) as f32);
                }
                series.breaks = vec![random.below(chart.points as usize - 1) as u32];
            }
            chart.badge = Some(crate::chart::Note::new("SHORT 3", Tone::Bad));
            panel.body = chart.summary();
            panel.data_seq = revision;
        }
        _ => {}
    }
}

/// Apply `patch` to `client` as it reaches it, over the wire.
fn apply(patch: &ProjectionPatch, client: &mut Projection) -> Result<(), PatchError> {
    let patch: ProjectionPatch =
        serde_json::from_str(&serde_json::to_string(patch).unwrap()).unwrap();
    patch.apply(client)
}

#[test]
fn patches_applied_in_sequence_are_the_projection_sent_whole() {
    for seed in 1..=200u64 {
        let mut random = Random(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut serial = 0;
        let mut server = projection(vec![tomori()], Vec::new(), 1);
        // One client that applies data ops, and one that does not, each patched from what it holds.
        let mut live = server.clone();
        let mut older = server.clone();
        for step in 0..60 {
            // Several changes at once, as a burst before the server sends, coalesce into one patch.
            let before = server.clone();
            let revision = server.revision + 1;
            for _ in 0..1 + random.below(4) {
                mutate(&mut server, &mut random, &mut serial, revision);
            }
            server.revision = revision;
            let patch = ProjectionPatch::between_live(&before, &server);
            assert_eq!(apply(&patch, &mut live), Ok(()), "seed {seed}, step {step}");
            let patch = ProjectionPatch::between(&before, &server);
            assert!(
                !patch.ops.iter().any(|op| matches!(
                    op,
                    PatchOp::ChartAppend { .. } | PatchOp::ChartReplace { .. }
                )),
                "an older client is sent no data ops"
            );
            assert_eq!(
                apply(&patch, &mut older),
                Ok(()),
                "seed {seed}, step {step}"
            );
            for client in [&live, &older] {
                assert_eq!(
                    without_ages(client.clone()),
                    without_ages(server.clone()),
                    "seed {seed}, step {step}"
                );
            }
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
        let revision = server.revision + 1;
        mutate(&mut server, &mut random, &mut serial, revision);
        server.revision = revision;
    }
    for between in [ProjectionPatch::between, ProjectionPatch::between_live] {
        let mut client = first.clone();
        between(&first, &server).apply(&mut client).unwrap();
        assert_eq!(without_ages(client), without_ages(server.clone()));
    }
}

#[test]
fn a_live_panel_s_data_is_appended_or_replaced_apart_from_its_version() {
    let base = projection(vec![tomori()], vec![charted("chart", chart(6, 2, 0), 3)], 3);
    // Its points moved along by two: an append of the newest two of each series, from the seq held to the new one.
    let mut next = base.clone().with_revision(4);
    next.panels[0] = charted("chart", chart(6, 2, 2), 4);
    let patch = ProjectionPatch::between_live(&base, &next);
    assert_eq!(
        patch.ops,
        vec![PatchOp::ChartAppend {
            panel: "chart".into(),
            seq: 4,
            from: 3,
            values: vec![vec![Some(6.0), Some(7.0)], vec![Some(1006.0), Some(1007.0)]],
            breaks: None,
            status: None,
            badge: None,
            body: next.panels[0].body.clone(),
        }]
    );
    assert_eq!(patch.whole(), 0, "nothing is sent whole");
    let json = serde_json::to_value(&patch.ops[0]).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "op": "chart_append", "panel": "chart", "seq": 4, "from": 3,
            "values": [[6.0, 7.0], [1006.0, 1007.0]], "body": next.panels[0].body,
        })
    );
    let mut shown = base.clone();
    patch.apply(&mut shown).unwrap();
    assert_eq!(shown, next);
    // An older client is sent the panel whole, as before.
    assert!(matches!(
        ProjectionPatch::between(&base, &next).ops.as_slice(),
        [PatchOp::Panel { .. }]
    ));
    // Its points changed otherwise, with breaks and a badge: replaced whole, breaks and all.
    let mut replaced = base.clone().with_revision(5);
    let mut chart_now = chart(6, 2, 0);
    chart_now.series[0].values[2] = None;
    chart_now.series[1].breaks = vec![3];
    chart_now.badge = Some(crate::chart::Note::new("SHORT 3", Tone::Bad));
    replaced.panels[0] = charted("chart", chart_now, 5);
    let patch = ProjectionPatch::between_live(&base, &replaced);
    let [PatchOp::ChartReplace { seq, breaks, .. }] = patch.ops.as_slice() else {
        unreachable!("{:?}", patch.ops);
    };
    assert_eq!((*seq, breaks.clone()), (5, vec![vec![], vec![3]]));
    let mut shown = base.clone();
    patch.apply(&mut shown).unwrap();
    assert_eq!(shown, replaced);
    // Moved along with a break among the new points, the append says every series' breaks.
    let mut broken = base.clone().with_revision(6);
    let mut chart_now = chart(6, 2, 1);
    chart_now.series[1].breaks = vec![4];
    broken.panels[0] = charted("chart", chart_now, 6);
    let patch = ProjectionPatch::between_live(&base, &broken);
    let [PatchOp::ChartAppend { breaks, .. }] = patch.ops.as_slice() else {
        unreachable!("{:?}", patch.ops);
    };
    assert_eq!(breaks.clone(), Some(vec![vec![], vec![4]]));
    let mut shown = base.clone();
    patch.apply(&mut shown).unwrap();
    assert_eq!(shown, broken);
    // Moved and its data changed together: a place op carrying its version, and the data apart.
    let mut both = next.clone().with_revision(7);
    both.panels[0].place = Place::Free { pose: pose(1.0) };
    both.panels[0].version = 7;
    let patch = ProjectionPatch::between_live(&base, &both);
    assert!(matches!(
        patch.ops.as_slice(),
        [
            PatchOp::Place { version: 7, .. },
            PatchOp::ChartAppend { .. }
        ]
    ));
    let mut shown = base.clone();
    patch.apply(&mut shown).unwrap();
    assert_eq!(shown, both);
    // Its structure changed, as its series' names, it is sent whole.
    let mut renamed = next.clone();
    renamed.panels[0].chart.as_mut().unwrap().series[0].name = "renamed".into();
    renamed.panels[0].version = 4;
    assert!(matches!(
        ProjectionPatch::between_live(&base, &renamed)
            .ops
            .as_slice(),
        [PatchOp::Panel { .. }]
    ));
    // Data changed at the seq it was at, as no server sends, it is sent whole rather than taken for data held.
    let mut same_seq = next.clone();
    same_seq.panels[0].data_seq = 3;
    assert!(matches!(
        ProjectionPatch::between_live(&base, &same_seq)
            .ops
            .as_slice(),
        [PatchOp::Panel { .. }]
    ));
}

#[test]
fn a_data_op_already_held_is_dropped_and_one_that_does_not_fit_asks_for_the_projection_whole() {
    let base = projection(vec![tomori()], vec![charted("chart", chart(6, 1, 0), 5)], 5);
    let append = |seq: u64, from: u64, values: Vec<Vec<Option<f32>>>| PatchOp::ChartAppend {
        panel: "chart".into(),
        seq,
        from,
        values,
        breaks: None,
        status: None,
        badge: None,
        body: "new".into(),
    };
    let replace =
        |seq: u64, values: Vec<Vec<Option<f32>>>, breaks: Vec<Vec<u32>>| PatchOp::ChartReplace {
            panel: "chart".into(),
            seq,
            values,
            breaks,
            status: None,
            badge: None,
            body: "new".into(),
        };
    // Stale, as data the client already holds at its seq or later, it is dropped, and the rest of the patch applies.
    for stale in [
        append(5, 4, vec![vec![Some(1.0)]]),
        append(4, 3, vec![vec![Some(1.0)]]),
        replace(5, vec![vec![None; 6]], Vec::new()),
    ] {
        let mut shown = base.clone();
        ProjectionPatch::new("s", "sky", 5, 6, vec![stale])
            .apply(&mut shown)
            .unwrap();
        assert_eq!(shown, base.clone().with_revision(6));
    }
    // From another seq than it holds, or not fitting its series, or for a panel with no chart, it does not apply, and
    // nothing changes.
    let mut text = base.clone();
    text.panels[0].chart = None;
    for (op, shown) in [
        (append(7, 6, vec![vec![Some(1.0)]]), base.clone()),
        (
            append(6, 5, vec![vec![Some(1.0)], vec![Some(2.0)]]),
            base.clone(),
        ),
        (append(6, 5, vec![vec![]]), base.clone()),
        (append(6, 5, vec![vec![Some(1.0); 7]]), base.clone()),
        (
            PatchOp::ChartAppend {
                panel: "chart".into(),
                seq: 6,
                from: 5,
                values: vec![vec![Some(1.0)]],
                breaks: Some(vec![vec![], vec![]]),
                status: None,
                badge: None,
                body: String::new(),
            },
            base.clone(),
        ),
        (append(6, 5, vec![vec![Some(1.0)]]), text.clone()),
        (
            replace(6, vec![vec![None; 6], vec![None; 6]], Vec::new()),
            base.clone(),
        ),
        (
            replace(6, vec![vec![None; 6]], vec![vec![], vec![]]),
            base.clone(),
        ),
        (replace(6, vec![vec![None; 6]], Vec::new()), text.clone()),
    ] {
        let mut held = shown.clone();
        let error = ProjectionPatch::new("s", "sky", 5, 6, vec![op])
            .apply(&mut held)
            .unwrap_err();
        assert_eq!(error, PatchError::Data("chart".into()));
        assert!(error.to_string().contains("chart"));
        assert_eq!(held, shown, "nothing changed");
    }
    for op in [
        append(6, 5, vec![vec![Some(1.0)]]),
        replace(6, Vec::new(), Vec::new()),
    ] {
        let mut held = projection(vec![tomori()], Vec::new(), 5);
        assert_eq!(
            ProjectionPatch::new("s", "sky", 5, 6, vec![op]).apply(&mut held),
            Err(PatchError::Missing("chart".into()))
        );
    }
    // Replaced whole, with no breaks said, every series has none.
    let mut shown = base.clone();
    shown.panels[0].chart.as_mut().unwrap().series[0].breaks = vec![2];
    ProjectionPatch::new(
        "s",
        "sky",
        5,
        6,
        vec![replace(6, vec![vec![None; 6]], Vec::new())],
    )
    .apply(&mut shown)
    .unwrap();
    let held = &shown.panels[0];
    assert_eq!(
        (
            held.data_seq,
            held.body.as_str(),
            held.chart.as_ref().unwrap().series[0].breaks.len()
        ),
        (6, "new", 0)
    );
    // Its version is never a data op's to change.
    assert_eq!(held.version, 1);
}

#[test]
fn a_place_op_carries_the_panel_s_version_and_an_older_server_s_keeps_it() {
    let base = projection(
        vec![tomori()],
        vec![panel("panel-1", "One", on(TOMORI_WALL, 0))],
        3,
    );
    let mut moved = base.clone().with_revision(4);
    moved.panels[0].place = on(TOMORI_WALL, 2);
    moved.panels[0].version = 4;
    // To a client that keeps it, a place op with its version; to an older client, the panel whole.
    assert!(matches!(
        ProjectionPatch::between_live(&base, &moved).ops.as_slice(),
        [PatchOp::Place { version: 4, .. }]
    ));
    assert!(matches!(
        ProjectionPatch::between(&base, &moved).ops.as_slice(),
        [PatchOp::Panel { .. }]
    ));
    // Only its version changed, as for a panel shown again: a place op where it is.
    let mut versioned = base.clone().with_revision(4);
    versioned.panels[0].version = 4;
    let patch = ProjectionPatch::between_live(&base, &versioned);
    assert!(matches!(
        patch.ops.as_slice(),
        [PatchOp::Place { version: 4, .. }]
    ));
    let mut shown = base.clone();
    patch.apply(&mut shown).unwrap();
    assert_eq!(shown, versioned);
    // An older server's place op says none, and the panel keeps the version it had.
    let older: ProjectionPatch = serde_json::from_value(serde_json::json!({
        "space": "s", "environment": "sky", "from": 3, "to": 4,
        "ops": [{"op": "place", "panel": "panel-1", "place": {"kind": "wall", "wall": "tomori", "slot": 2}}]
    }))
    .unwrap();
    let mut shown = base.clone();
    older.apply(&mut shown).unwrap();
    assert_eq!(
        (shown.panels[0].version, &shown.panels[0].place),
        (1, &on(TOMORI_WALL, 2))
    );
}

#[test]
fn the_longest_run_kept_in_order_stays_where_it_is() {
    assert_eq!(longest_increasing(&[]), Vec::<usize>::new());
    assert_eq!(longest_increasing(&[2, 0, 1, 3]), vec![0, 1, 3]);
    assert_eq!(longest_increasing(&[1, 2, 3, 0]), vec![1, 2, 3]);
    assert_eq!(longest_increasing(&[3, 2, 1]).len(), 1);
}

#[test]
fn a_chart_s_breaks_move_along_with_its_points_and_its_data_whole_may_have_none() {
    // Its breaks moved along with its points, those dropped with them, the append says none of them.
    let mut old = chart(6, 1, 0);
    old.series[0].breaks = vec![0, 3];
    let mut new = chart(6, 1, 1);
    new.series[0].breaks = vec![2];
    let base = projection(vec![tomori()], vec![charted("chart", old, 3)], 3);
    let mut next = base.clone().with_revision(4);
    next.panels[0] = charted("chart", new, 4);
    let patch = ProjectionPatch::between_live(&base, &next);
    assert!(
        matches!(
            patch.ops.as_slice(),
            [PatchOp::ChartAppend { breaks: None, .. }]
        ),
        "{:?}",
        patch.ops
    );
    let mut shown = base.clone();
    patch.apply(&mut shown).unwrap();
    assert_eq!(shown, next);
    // Its points all changed, and none broken, its data is replaced whole, saying no breaks.
    let mut replaced = base.clone().with_revision(5);
    replaced.panels[0] = charted("chart", chart(6, 1, 100), 5);
    let patch = ProjectionPatch::between_live(&base, &replaced);
    assert!(
        matches!(patch.ops.as_slice(), [PatchOp::ChartReplace { breaks, .. }] if breaks.is_empty()),
        "{:?}",
        patch.ops
    );
    let mut shown = base.clone();
    patch.apply(&mut shown).unwrap();
    assert_eq!(shown, replaced);
    // An append of no series does not fit a chart that has one.
    let empty = PatchOp::ChartAppend {
        panel: "chart".into(),
        seq: 4,
        from: 3,
        values: Vec::new(),
        breaks: None,
        status: None,
        badge: None,
        body: String::new(),
    };
    let mut shown = base.clone();
    assert_eq!(
        ProjectionPatch::new("s", "sky", 3, 4, vec![empty]).apply(&mut shown),
        Err(PatchError::Data("chart".into()))
    );
}

#[test]
fn a_panel_s_structure_and_live_data_are_told_apart() {
    let text = panel("panel-1", "pressure: 1 now", on(TOMORI_WALL, 0));
    let live = charted("panel-1", chart(4, 1, 0), 2);
    // A text panel and a chart panel are never of one structure, and a text panel has no live data to differ in.
    assert!(!text.same_structure(&live) && !live.same_structure(&text));
    assert!(text.same_data(&live) && text.same_structure(&text.clone().with_version(9)));
    // A chart's points moving along is its data, not its structure; its series renamed is its structure.
    let moved = charted("panel-1", chart(4, 1, 1), 3);
    assert!(live.same_structure(&moved) && !live.same_data(&moved));
    let mut renamed = live.clone();
    renamed.chart.as_mut().unwrap().series[0].name = "renamed".into();
    assert!(!live.same_structure(&renamed) && live.same_data(&renamed));
    // Moved, pinned, aged or at another version, it is of the same structure.
    let mut placed = live.clone().with_pinned(true).with_age(30).with_version(7);
    placed.place = Place::Free { pose: pose(1.0) };
    assert!(live.same_structure(&placed));
}
