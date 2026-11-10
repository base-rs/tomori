// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_protocol::chart::{Chart, Colour, Series, SeriesStyle};
use tomori_protocol::{Place, Pose, WallKind};

fn wall(id: &str) -> Wall {
    Wall::new(id, WallKind::Added, Pose::default())
}

fn panel(id: &str, body: &str, slot: u32) -> Panel {
    Panel::new(
        id,
        id,
        body,
        Place::Wall {
            wall: "wall-1".into(),
            slot,
        },
    )
}

fn chart(values: &[f32]) -> Chart {
    Chart::new(values.len() as u32).with_series(Series::new(
        "pressure",
        Colour::Teal,
        SeriesStyle::Line,
        values.iter().copied().map(Some).collect(),
    ))
}

fn projection(walls: Vec<Wall>, panels: Vec<Panel>) -> Projection {
    Projection::new("s", "sky")
        .with_walls(walls)
        .with_panels(panels)
}

/// Stamp `projection` as the space's at `revision`, as the hub builds it there, returning the revision it was stamped
/// at.
fn stamp(versions: &mut Versions, mut projection: Projection, revision: &Cell<u64>) -> Projection {
    versions.stamp(&mut projection, revision);
    projection.with_revision(revision.get())
}

#[test]
fn a_version_is_the_revision_of_the_last_change_and_live_data_has_its_own_seq() {
    let mut versions = Versions::default();
    let revision = Cell::new(10);
    let live = panel("panel-2", "", 1).with_chart(chart(&[1.0, 2.0, 3.0]));
    let first = stamp(
        &mut versions,
        projection(
            vec![wall("wall-1")],
            vec![panel("panel-1", "- [ ] One", 0), live.clone()],
        ),
        &revision,
    );
    // Everything is new, at the space's revision.
    assert_eq!(first.walls[0].version, 10);
    assert_eq!(first.panels[0].version, 10);
    assert_eq!(
        (first.panels[1].version, first.panels[1].data_seq),
        (10, 10)
    );
    assert_eq!(first.panels[0].data_seq, 0, "no live data");
    // Built again with nothing changed, nothing changes, whatever the panels' ages.
    revision.set(11);
    let again = stamp(
        &mut versions,
        projection(
            vec![wall("wall-1")],
            vec![
                panel("panel-1", "- [ ] One", 0).with_age(30),
                live.clone().with_age(30),
            ],
        ),
        &revision,
    );
    assert_eq!(
        again
            .panels
            .iter()
            .map(|panel| (panel.version, panel.data_seq))
            .collect::<Vec<_>>(),
        [(10, 0), (10, 10)]
    );
    assert_eq!(again.walls[0].version, 10);
    // A tick changes the panel's version to the revision it was made at; a move does too; the wall's own change, its.
    revision.set(12);
    let mut tilted = wall("wall-1");
    tilted.tilt = Some(10.0);
    let ticked = stamp(
        &mut versions,
        projection(
            vec![tilted.clone()],
            vec![panel("panel-1", "- [x] One", 0), live.clone()],
        ),
        &revision,
    );
    assert_eq!(
        (ticked.walls[0].version, ticked.panels[0].version),
        (12, 12)
    );
    revision.set(13);
    let moved = stamp(
        &mut versions,
        projection(
            vec![tilted.clone()],
            vec![
                panel("panel-1", "- [x] One", 0),
                live.clone().with_pinned(true),
            ],
        ),
        &revision,
    );
    assert_eq!((moved.panels[0].version, moved.panels[1].version), (12, 13));
    assert_eq!(moved.panels[1].data_seq, 10, "its data is as it was");
    // Its live data moving on changes its data seq alone, so that it refuses nobody's change.
    revision.set(14);
    let mut updated = live.clone().with_pinned(true);
    updated.chart = Some(Box::new(chart(&[2.0, 3.0, 4.0])));
    updated.body = "pressure: 4 now".into();
    let data = stamp(
        &mut versions,
        projection(
            vec![tilted.clone()],
            vec![panel("panel-1", "- [x] One", 0), updated.clone()],
        ),
        &revision,
    );
    assert_eq!((data.panels[1].version, data.panels[1].data_seq), (13, 14));
    // Its structure changing, as its title, changes its version, its data seq staying where its data did not change.
    revision.set(15);
    let titled = stamp(
        &mut versions,
        projection(
            vec![tilted.clone()],
            vec![panel("panel-1", "- [x] One", 0), {
                let mut titled = updated.clone();
                titled.title = "Scaling pressure".into();
                titled
            }],
        ),
        &revision,
    );
    assert_eq!(
        (titled.panels[1].version, titled.panels[1].data_seq),
        (15, 14)
    );
    // A panel gone and shown again, or a wall, is new.
    revision.set(16);
    stamp(&mut versions, projection(vec![], vec![]), &revision);
    revision.set(17);
    let back = stamp(
        &mut versions,
        projection(vec![tilted], vec![panel("panel-1", "- [x] One", 0)]),
        &revision,
    );
    assert_eq!((back.walls[0].version, back.panels[0].version), (17, 17));
}

#[test]
fn a_change_built_at_a_revision_already_given_raises_it_so_that_one_version_names_one_state() {
    let mut versions = Versions::default();
    let revision = Cell::new(5);
    stamp(
        &mut versions,
        projection(vec![wall("wall-1")], vec![panel("panel-1", "One", 0)]),
        &revision,
    );
    // Changed with nothing raising the revision since, as when the agent's tool reads its change before the hub sends
    // it, it is given a revision of its own.
    let changed = stamp(
        &mut versions,
        projection(vec![wall("wall-1")], vec![panel("panel-1", "Two", 0)]),
        &revision,
    );
    assert_eq!(revision.get(), 6);
    assert_eq!(
        (changed.walls[0].version, changed.panels[0].version),
        (5, 6)
    );
    // Built again unchanged, it raises nothing.
    stamp(
        &mut versions,
        projection(vec![wall("wall-1")], vec![panel("panel-1", "Two", 0)]),
        &revision,
    );
    assert_eq!(revision.get(), 6);
}

#[test]
fn a_run_begins_its_revisions_from_the_time() {
    let first = first_revision();
    // Milliseconds since the Unix epoch: well past November 2026's, and never 0, which means none.
    assert!(first > 1_790_000_000_000, "{first}");
    assert!(first_revision() >= first);
}
