// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Patches of what a stage projects: each change a participant or the agent makes is the patch it should be, patches
//! applied in turn are the view sent whole, and how much smaller they are. See `design/projection.md`.
use tomori_protocol::{PatchOp, ProjectionPatch, ServerMessage, WALL_SLOTS};

use super::*;

fn pose(x: f32, z: f32) -> Pose {
    Pose {
        position: [x, 1.5, z],
        orientation: [0.0, 0.0, 0.0, 1.0],
    }
}

/// The `index`th of many poses within reach: twelve a row, three metres apart, from `z`, each row `step` on.
fn spread(index: usize, z: f32, step: f32) -> Pose {
    pose(
        (index % 12) as f32 * 3.0 - 16.5,
        z + (index / 12) as f32 * step,
    )
}

const SKY: Walls<'static> = Walls {
    environment: "sky",
    tomori: Pose {
        position: [2.0, 1.5, -2.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    },
    declared: &[],
    status: crate::stage::Status::Beneath,
};

/// A text panel as an agent writes one, of a few sentences of prose and a list.
fn prose(title: &str) -> Content {
    Content::parse(&format!(
        "title: {title}\nsource: CI\nowner: Ada\n---\nThe **build** of `tomori-server` failed on two targets after the \
         change to how walls snap together; the others pass.\n\n- Linux: 3 tests failed in `stage`.\n- Android: the \
         APK built, but the headset check is still to do.\n- macOS: passed.\n\nNext, fix the snapping tests and run \
         the headset check again before merging."
    ))
    .unwrap()
}

/// An agenda of checklist items, which participants tick.
fn agenda() -> Content {
    Content::parse(
        "title: Agenda\nid: agenda\n---\n- [ ] Decide the pairing window\n- [ ] Review the projection patches\n- [ ] \
         Plan the chunked snapshots\n- [ ] Check the headset build\n- [ ] Merge #277 and #279",
    )
    .unwrap()
}

/// Every panel as large as a block can make it, in characters of four bytes, as the bound on a frame is tested with.
fn largest() -> Content {
    let wide = |count: usize| "\u{1F600}".repeat(count);
    Content::parse(&format!(
        "title: {}\na: {}\nb: {}\nc: {}\nd: {}\n---\n{}",
        wide(200),
        wide(200),
        wide(200),
        wide(200),
        wide(200),
        vec![wide(40); 2 * blocks::BODY_LINES].join("\n")
    ))
    .unwrap()
}

fn whole(projection: &Projection) -> usize {
    serde_json::to_vec(&ServerMessage::Projection {
        projection: Box::new(projection.clone()),
    })
    .unwrap()
    .len()
}

fn patched(patch: &ProjectionPatch) -> usize {
    serde_json::to_vec(&ServerMessage::ProjectionPatch {
        patch: Box::new(patch.clone()),
    })
    .unwrap()
    .len()
}

/// `projection`, with every panel's age the same, as a client counts each on from when it was sent.
fn without_ages(mut projection: Projection) -> Projection {
    for panel in &mut projection.panels {
        panel.age = 0;
    }
    projection
}

/// What the stage shows at `revision`.
fn view(stage: &Stage, walls: &Walls<'_>, revision: u64) -> Projection {
    stage.view("s", walls, 1_000).with_revision(revision)
}

/// A space's first revision in a run, as the time in milliseconds makes it, so that what is measured is the size sent.
const FIRST: u64 = 1_794_000_000_000;

/// The patch `change` makes to what `stage` shows, versioned as the hub versions it, to a client that applies data ops,
/// which applied to what was shown is what is shown now, as the patch to an older client is too.
fn change(
    stage: &mut Stage,
    walls: &Walls<'_>,
    change: impl FnOnce(&mut Stage),
) -> ProjectionPatch {
    let mut versions = crate::versions::Versions::default();
    let revision = std::cell::Cell::new(FIRST);
    let mut before = view(stage, walls, FIRST);
    versions.stamp(&mut before, &revision);
    change(stage);
    revision.set(FIRST + 1);
    let mut after = view(stage, walls, FIRST + 1);
    versions.stamp(&mut after, &revision);
    // As a client that keeps the versions place ops carry, as Tomori's own do, is sent it, and as an older one is.
    let older = ProjectionPatch::between(&before, &after);
    let mut applied = before.clone();
    older.apply(&mut applied).unwrap();
    assert_eq!(without_ages(applied), without_ages(after.clone()));
    let patch = ProjectionPatch::between_live(&before, &after);
    let mut applied = before.clone();
    patch.apply(&mut applied).unwrap();
    assert_eq!(without_ages(applied), without_ages(after));
    patch
}

/// The kinds of op of `patch`, in order, as their wire names.
fn kinds(patch: &ProjectionPatch) -> Vec<String> {
    patch
        .ops
        .iter()
        .map(|op| {
            serde_json::to_value(op).unwrap()["op"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[test]
fn the_stage_s_bounds_are_those_a_welcome_states() {
    let limits = crate::hub::LIMITS;
    assert_eq!(1 + DECLARED_WALLS + ADDED_WALLS, limits.walls as usize);
    assert_eq!(SLOTS, limits.slots as usize);
    assert_eq!(SLOTS + FLOATING, limits.panels as usize);
    assert_eq!(FLOATING, limits.floating as usize);
    // Ten times today's, which an older client is sent within.
    let today = tomori_protocol::Limits::TODAY;
    assert_eq!(SLOTS, 10 * today.slots as usize);
    assert_eq!(FLOATING, 10 * today.floating as usize);
}

#[test]
fn each_change_to_a_stage_is_its_patch() {
    let walls = SKY;
    let mut stage = Stage::default();
    let mut agenda_id = String::new();
    // The agent projects a panel: it is sent whole.
    let patch = change(&mut stage, &walls, |stage| {
        agenda_id = stage.project(&walls, agenda(), None, 0);
    });
    assert_eq!(kinds(&patch), ["panel"]);
    let notes = stage.project(&walls, prose("Notes"), None, 0);
    // A participant ticks an item: the panel is sent whole, at its next revision.
    let patch = change(&mut stage, &walls, |stage| {
        stage
            .tick_item(&walls, &agenda_id, 0, true)
            .unwrap()
            .unwrap();
    });
    assert_eq!(kinds(&patch), ["panel"]);
    let PatchOp::Panel { panel, .. } = &patch.ops[0] else {
        unreachable!("a panel");
    };
    assert!(panel.body.starts_with("- [x]"));
    // The agent updates it in place.
    let patch = change(&mut stage, &walls, |stage| {
        let mut updated = agenda();
        updated.title = "Agenda, revised".into();
        assert!(stage.revise(&walls, &agenda_id, updated, None, 5).unwrap());
    });
    assert_eq!(kinds(&patch), ["panel"]);
    // A participant pulls one off and leaves it floating: it is moved, not sent again.
    let patch = change(&mut stage, &walls, |stage| {
        stage
            .place(&walls, &notes, Target::Free(pose(1.0, 1.0)))
            .unwrap();
    });
    assert_eq!(kinds(&patch), ["place"]);
    // An owner adds a wall, and the panel goes onto it.
    let mut wall = String::new();
    let patch = change(&mut stage, &walls, |stage| {
        wall = stage.add_wall(&walls, &pose(0.0, -4.0), None).unwrap();
    });
    assert_eq!(kinds(&patch), ["wall"]);
    let patch = change(&mut stage, &walls, |stage| {
        stage
            .place(&walls, &notes, Target::Slot(wall.clone(), 4))
            .unwrap();
    });
    assert_eq!(kinds(&patch), ["place"]);
    // A participant changes the wall's grid: only the wall is sent, its panel keeping its column and row.
    let patch = change(&mut stage, &walls, |stage| {
        let change = WallChange::new(Grid::new(4, 2), None, None, None, None, Some(10.0)).unwrap();
        assert!(stage.set_wall(&walls, &wall, change).unwrap());
    });
    assert_eq!(kinds(&patch), ["wall", "place"], "{:?}", patch.ops);
    // An owner locks it.
    let patch = change(&mut stage, &walls, |stage| {
        assert!(stage.lock_wall(&walls, &wall, true, false).unwrap());
    });
    assert_eq!(kinds(&patch), ["wall"]);
    let patch = change(&mut stage, &walls, |stage| {
        assert!(stage.lock_wall(&walls, &wall, false, false).unwrap());
    });
    assert_eq!(kinds(&patch), ["wall"]);
    // An owner removes it, its panel left floating where its slot stood.
    let patch = change(&mut stage, &walls, |stage| {
        stage.remove_wall(&walls, &wall).unwrap();
    });
    assert_eq!(kinds(&patch), ["remove_wall", "place"]);
    // A participant dismisses a panel.
    let patch = change(&mut stage, &walls, |stage| {
        stage.dismiss(&walls, &notes).unwrap();
    });
    assert_eq!(kinds(&patch), ["remove_panel"]);
}

/// A small, fast generator of numbers, seeded, so that each run tries the same sequences.
struct Random(u64);

impl Random {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound as u64) as usize
    }
}

#[test]
fn a_stage_s_changes_patched_in_turn_are_what_it_shows() {
    let walls = SKY;
    for seed in 1..=40u64 {
        let mut random = Random(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut stage = Stage::default();
        // Versioned as the hub versions what it sends, each wall and panel at the revision it last changed.
        let mut versions = crate::versions::Versions::default();
        let revision = std::cell::Cell::new(1);
        let versioned = |stage: &Stage, versions: &mut crate::versions::Versions| {
            let mut shown = view(stage, &walls, 0);
            versions.stamp(&mut shown, &revision);
            shown.with_revision(revision.get())
        };
        let mut before = versioned(&stage, &mut versions);
        // One client that applies data ops and keeps the versions place ops carry, and one that does not.
        let mut live = before.clone();
        let mut client = before.clone();
        for step in 0..80 {
            // A burst of changes, sent together.
            for _ in 0..1 + random.below(3) {
                let shown = stage.view("s", &walls, 0);
                let panel = (!shown.panels.is_empty())
                    .then(|| shown.panels[random.below(shown.panels.len())].id.clone());
                let wall = shown.walls[random.below(shown.walls.len())].id.clone();
                let _ = match random.below(10) {
                    0 | 1 => {
                        let content = if random.below(2) == 0 {
                            agenda()
                        } else {
                            prose(&format!("Panel {step}"))
                        };
                        stage.project(&walls, content, None, step as u64);
                        Ok(true)
                    }
                    2 => panel.map_or(Ok(false), |panel| {
                        stage.place(&walls, &panel, Target::Free(pose(step as f32 / 10.0, 1.0)))
                    }),
                    3 => panel.map_or(Ok(false), |panel| {
                        stage.place(&walls, &panel, Target::Slot(wall, random.below(6) as u32))
                    }),
                    4 => panel.map_or(Ok(false), |panel| {
                        stage.dismiss(&walls, &panel).map(|()| true)
                    }),
                    5 => stage
                        .add_wall(&walls, &pose(random.below(10) as f32, -5.0), None)
                        .map(|_| true),
                    6 => stage.remove_wall(&walls, &wall).map(|()| true),
                    7 => {
                        let grid =
                            Grid::new(1 + random.below(4) as u32, 1 + random.below(2) as u32);
                        let change = WallChange::new(grid, None, None, None, None, None).unwrap();
                        stage.set_wall(&walls, &wall, change)
                    }
                    8 => panel.map_or(Ok(false), |panel| {
                        stage
                            .tick_item(&walls, &panel, random.below(5) as u32, true)
                            .map(|ticked| ticked.is_some())
                    }),
                    _ => stage.lock_wall(&walls, &wall, random.below(2) == 0, false),
                };
            }
            revision.set(revision.get() + 1);
            let after = versioned(&stage, &mut versions);
            for (held, between) in [
                (&mut live, ProjectionPatch::between_live as fn(&_, &_) -> _),
                (&mut client, ProjectionPatch::between),
            ] {
                let patch: ProjectionPatch =
                    serde_json::from_slice(&serde_json::to_vec(&between(&before, &after)).unwrap())
                        .unwrap();
                assert_eq!(patch.apply(held), Ok(()), "seed {seed}, step {step}");
                assert_eq!(
                    without_ages(held.clone()),
                    without_ages(after.clone()),
                    "seed {seed}, step {step}"
                );
            }
            // Each version is the revision at which its wall or panel last changed, never later than now.
            assert!(
                after
                    .walls
                    .iter()
                    .map(|wall| wall.version)
                    .chain(after.panels.iter().map(|panel| panel.version))
                    .all(|version| (1..=after.revision).contains(&version)),
                "seed {seed}, step {step}"
            );
            before = after;
        }
    }
}

/// What a tick, a panel's update and a wall's edit send, whole and as a patch, in a typical space and in one at every
/// limit: the numbers the change was measured by.
#[test]
fn patches_are_far_smaller_than_the_projection_whole() {
    let declared: Vec<crate::EnvironmentWall> = (0..DECLARED_WALLS)
        .map(|index| pose(index as f32 - 3.0, -5.0).into())
        .collect();
    let full = Walls {
        declared: &declared,
        ..SKY
    };
    // A typical space: the Tomori wall and one added, eight text panels, one of them an agenda.
    let mut typical = Stage::default();
    let added = typical.add_wall(&SKY, &pose(0.0, -4.0), None).unwrap();
    let agenda_typical = typical.project(&SKY, agenda(), None, 0);
    for index in 0..7 {
        let origin = (index >= 3).then_some(pose(0.0, -4.0).position);
        typical.project(&SKY, prose(&format!("Notes {index}")), origin, 0);
    }
    // A space at every limit: every slot of sixty-seven walls, and every floating panel, of the largest text panels.
    let mut most = Stage::default();
    for index in 0..FLOATING {
        let id = most.project(&full, largest(), None, 0);
        most.place(&full, &id, Target::Free(spread(index, 0.0, 0.4)))
            .unwrap();
    }
    let mut added_most = Vec::new();
    for index in 0..ADDED_WALLS {
        added_most.push(
            most.add_wall(&full, &spread(index, 5.0, 2.5), None)
                .unwrap(),
        );
    }
    let origins = std::iter::once(None).chain(
        declared
            .iter()
            .map(|declared| declared.pose.position)
            .chain((0..ADDED_WALLS).map(|index| spread(index, 5.0, 2.5).position))
            .map(Some),
    );
    for origin in origins {
        for _ in 0..WALL_SLOTS {
            most.project(&full, largest(), origin, 0);
        }
    }
    // The agenda last, on the Tomori wall, in place of the panel there longest.
    let agenda_most = most.project(&full, agenda(), None, 0);
    assert_eq!(
        most.view("s", &full, 0).panels.len(),
        WALL_SLOTS as usize * (1 + DECLARED_WALLS + ADDED_WALLS) + FLOATING
    );

    let mut rows = Vec::new();
    for (name, stage, walls, listed, wall) in [
        ("typical", &mut typical, SKY, agenda_typical, added),
        (
            "every limit",
            &mut most,
            full,
            agenda_most,
            added_most[0].clone(),
        ),
    ] {
        let tick = change(stage, &walls, |stage| {
            stage.tick_item(&walls, &listed, 2, true).unwrap();
        });
        let update = change(stage, &walls, |stage| {
            let mut updated = agenda();
            updated.body.push_str("\n- [ ] Write the release notes");
            stage.revise(&walls, &listed, updated, None, 5).unwrap();
        });
        let edit = change(stage, &walls, |stage| {
            let change = WallChange::new(None, None, None, None, None, Some(8.0)).unwrap();
            stage.set_wall(&walls, &wall, change).unwrap();
        });
        let mut shown = view(stage, &walls, FIRST);
        crate::versions::Versions::default().stamp(&mut shown, &std::cell::Cell::new(FIRST));
        for (what, patch) in [
            ("tick", tick),
            ("panel update", update),
            ("wall edit", edit),
        ] {
            let (whole, patched) = (whole(&shown), patched(&patch));
            assert!(
                patched * 4 < whole,
                "{name}, {what}: {patched} bytes patched against {whole} whole"
            );
            rows.push(format!(
                "| {name} | {what} | {whole} | {patched} | {:.0}× |",
                whole as f64 / patched as f64
            ));
        }
    }
    println!(
        "| Space | Change | Whole (bytes) | Patch (bytes) | Smaller |\n|---|---|---|---|---|\n{}",
        rows.join("\n")
    );
}
