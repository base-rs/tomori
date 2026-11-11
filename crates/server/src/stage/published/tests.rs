// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use tomori_environment::chart::Tone;
use tomori_environment::chart::{
    AxisLabel, Chart, Colour, Edge, Reference, Series, SeriesStyle, Shade,
};
use tomori_environment::{Panel, Row};
use tomori_protocol::{Place, TOMORI_WALL};

use super::*;
use crate::stage::published::{RESTING, RESTING_RUNS};

/// The most bytes of JSON a published panel's content and chart can take, everything in it bounded as it is: a chart of
/// every series at every point, its labels, summary and title, each at its longest, with room to spare. These tests
/// check that the largest there can be is within it.
const PUBLISHED_BYTES: usize = 24 * 1024;

/// The most bytes of JSON a published panel of a chart of every series at every point takes without its lines' breaks,
/// as a live chart is: the bytes a side may publish at once, [`crate::channel::PUBLISH_BYTES_BURST`], hold as many of
/// it as it may show.
const PLOTTED_BYTES: usize = 16 * 1024;
use crate::stage::{Destination, Placed, Target, WallChange};

fn pose(x: f32, z: f32) -> Pose {
    Pose {
        position: [x, 1.5, z],
        orientation: [0.0, 0.0, 0.0, 1.0],
    }
}

/// The `index`th of many spots in rows of twelve, 3 m apart, each row `step` further from `z`, so that as many as a
/// space holds stand within 20 m of its middle.
fn spread(index: usize, z: f32, step: f32) -> Pose {
    pose(
        (index % 12) as f32 * 3.0 - 16.5,
        z + (index / 12) as f32 * step,
    )
}

/// The yard's two walls: a notice board and a console.
const DECLARED: [crate::EnvironmentWall; 2] = [
    crate::EnvironmentWall::new(Pose {
        position: [3.0, 1.5, -4.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    }),
    crate::EnvironmentWall::new(Pose {
        position: [-3.0, 1.5, -4.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    }),
];

fn yard() -> Walls<'static> {
    Walls {
        environment: "yard",
        tomori: crate::stage::default_stage(),
        declared: &DECLARED,
        status: crate::stage::Status::Beneath,
    }
}

/// The yard's two walls, each of 8 by 3 slots, so that they have room for every panel an environment may publish.
fn roomy() -> Vec<crate::EnvironmentWall> {
    const { assert!(PUBLISHED <= 48) };
    DECLARED
        .iter()
        .map(|wall| wall.with_grid(tomori_protocol::Grid::from((8, 3))))
        .collect()
}

/// The largest chart there can be: every series at every point, each value as long as one is written, its line broken
/// after every point, with every label, reference line and shaded region, a unit, a badge and a status line, each as
/// long as it may be in characters of four bytes.
fn largest_chart() -> Chart {
    use tomori_environment::chart::{Note, ValueAxis};
    use tomori_protocol::chart::{
        CHART_LABELS, CHART_NAME_CHARS, CHART_POINTS, CHART_REFERENCES, CHART_SERIES, CHART_SHADES,
        CHART_STATUS_CHARS,
    };
    let wide = |count: usize| "\u{1F600}".repeat(count);
    let mut chart = Chart::new(CHART_POINTS)
        .with_axis(ValueAxis::new(
            Some(-1.234e-12),
            Some(1.234e-12),
            wide(CHART_NAME_CHARS),
        ))
        .with_labels(
            (0..CHART_LABELS as u32)
                .map(|at| AxisLabel::new(CHART_POINTS - 1 - at, wide(CHART_NAME_CHARS)))
                .collect(),
        )
        .with_badge(Note::new(wide(CHART_NAME_CHARS), Tone::Neutral))
        .with_status(Note::new(wide(CHART_STATUS_CHARS), Tone::Neutral));
    for index in 0..CHART_SERIES as u32 {
        let series = Series::new(
            wide(CHART_NAME_CHARS),
            Colour::Rgb([254, 254, 254]),
            SeriesStyle::Bars,
            (0..CHART_POINTS)
                .map(|point| Some(-1.234_5e-12 * (point + 1) as f32 - 1.0e-20))
                .collect(),
        )
        .with_width(1.25)
        .with_dashed(true)
        .with_breaks((0..CHART_POINTS).collect());
        chart = chart.with_series(series);
        chart = chart.with_reference(
            Reference::new(
                -1.234e-12,
                wide(CHART_NAME_CHARS),
                Colour::Rgb([254, 254, 254]),
            )
            .with_dashed(true),
        );
        chart = chart.with_shade(Shade::new(
            Edge::Reference { index },
            Edge::Series { index },
            true,
            Colour::Rgb([254, 254, 254]),
        ));
    }
    let chart = chart.cleaned().unwrap();
    assert_eq!(
        (
            chart.references.len(),
            chart.shades.len(),
            chart.labels.len()
        ),
        (CHART_REFERENCES, CHART_SHADES, CHART_LABELS)
    );
    assert_eq!(chart.series[0].breaks.len(), CHART_POINTS as usize - 1);
    chart
}

fn pressure(latest: f32) -> Chart {
    let mut values = vec![Some(0.8); 181];
    values[60..70].fill(None);
    values[180] = Some(latest);
    Chart::new(181)
        .with_labels(vec![
            AxisLabel::new(0, "-15 min"),
            AxisLabel::new(180, "now"),
        ])
        .with_series(Series::new(
            "greatest",
            Colour::Amber,
            SeriesStyle::Line,
            values,
        ))
        .with_reference(Reference::new(1.0, "target", Colour::White))
        .with_shade(Shade::new(
            Edge::Reference { index: 0 },
            Edge::Series { index: 0 },
            true,
            Colour::Red,
        ))
}

fn publishing(panel: Panel) -> Publishing {
    Publishing::new(panel).unwrap()
}

/// The panels shown, by title, with where each is.
fn shown(stage: &Stage, walls: &Walls<'_>) -> Vec<(String, Place)> {
    stage
        .view("s", walls, 0)
        .panels
        .into_iter()
        .map(|panel| (panel.title, panel.place))
        .collect()
}

fn on(wall: &str, slot: u32) -> Place {
    Place::Wall {
        wall: wall.into(),
        slot,
    }
}

#[test]
fn a_published_panel_is_cleaned_bounded_and_a_chart_s_body_is_its_summary() {
    let chart = publishing(
        Panel::chart("pressure", "Scaling\npressure \u{202e}", pressure(1.25))
            .with_reference("chart/pressure"),
    );
    assert_eq!(chart.id(), "pressure");
    assert_eq!(chart.content.title, "Scaling pressure");
    assert_eq!(chart.content.body, "greatest: 1.25 now\ntarget: 1");
    assert_eq!(chart.reference.as_deref(), Some("chart/pressure"));
    assert!(chart.bytes() > 1000 && chart.bytes() < PUBLISHED_BYTES);
    // A text panel keeps its body, bounded; a reference the agent could not name is left out.
    let text =
        publishing(Panel::text("rota", "Rota", "x\n".repeat(100)).with_reference("no spaces"));
    assert!(text.content.body.lines().count() <= blocks::BODY_LINES + 1);
    assert!(text.content.body.ends_with('…'));
    assert_eq!(text.reference, None);
    // Refused: an id that is not a slug, a chart cleaning refuses, a panel too large.
    assert!(Publishing::new(Panel::text("a b", "", "")).is_err());
    assert!(Publishing::new(Panel::chart("c", "", Chart::new(1))).is_err());
    let mut wide = Chart::new(256);
    for _ in 0..4 {
        wide = wide.with_series(Series::new(
            "s",
            Colour::Teal,
            SeriesStyle::Line,
            (0..256)
                .map(|point| Some(-1.234_5e-12 * point as f32 - 1.0e-20))
                .collect(),
        ));
    }
    wide = wide.with_labels(
        (0..8)
            .map(|at| AxisLabel::new(at, "\u{1F600}".repeat(32)))
            .collect(),
    );
    // A chart of every series at every point, with the longest title, is within what a side may publish at once for
    // each panel it may show; the largest a chart can be, in every part, is within the bound.
    let bytes = Publishing::new(Panel::chart("w", "\u{1F600}".repeat(80), wide))
        .unwrap()
        .bytes();
    assert!(bytes <= PLOTTED_BYTES, "{bytes}");
    assert!(
        (PLOTTED_BYTES * PUBLISHED) as f32 <= crate::channel::PUBLISH_BYTES_BURST,
        "every one is put up at once"
    );
    let bytes = Publishing::new(Panel::chart("w", "\u{1F600}".repeat(80), largest_chart()))
        .unwrap()
        .bytes();
    assert!(bytes > PLOTTED_BYTES && bytes <= PUBLISHED_BYTES, "{bytes}");
    // A live chart of three series over 15 minutes at 5 seconds is under 4 KiB, so that as many as a side may show,
    // each a point newer every second, are within what a space's panels may change by each second.
    let mut live = Chart::new(181);
    for (name, above) in [("mean", 0.0), ("greatest", 0.4), ("p99", 1.4)] {
        let values = (0..181).map(|point| Some(0.7 + above + 0.15 * (point as f32 / 9.0).sin()));
        live = live.with_series(Series::new(
            name,
            Colour::Teal,
            SeriesStyle::Line,
            values.collect(),
        ));
    }
    let bytes = publishing(Panel::chart("live", "Scaling pressure", live)).bytes();
    assert!((3000..4096).contains(&bytes), "{bytes}");
    assert!((bytes * PUBLISHED) as f32 <= crate::channel::PUBLISH_BYTES_PER_SECOND);
}

#[test]
fn an_environment_publishes_onto_its_own_walls_updates_in_place_and_retracts() {
    let walls = yard();
    let mut stage = Stage::default();
    let pressure_panel = |latest| {
        publishing(Panel::chart("pressure", "Pressure", pressure(latest)).on_wall(1, Some(2)))
    };
    assert_eq!(
        stage.publish(&walls, pressure_panel(1.1), 100),
        Ok(Change::Added)
    );
    assert_eq!(
        shown(&stage, &walls),
        [("Pressure".into(), on("environment-1", 2))]
    );
    let view = stage.view("s", &walls, 160);
    let id = view.panels[0].id.clone();
    assert!(view.panels[0].chart.is_some());
    assert_eq!(view.panels[0].age, 60);
    // An update keeps its identity, place and age, and the same again changes nothing.
    assert_eq!(
        stage.publish(&walls, pressure_panel(1.3), 200),
        Ok(Change::Updated)
    );
    assert_eq!(
        stage.publish(&walls, pressure_panel(1.3), 300),
        Ok(Change::Unchanged)
    );
    let view = stage.view("s", &walls, 300);
    assert_eq!(
        (view.panels[0].id.as_str(), view.panels[0].age),
        (id.as_str(), 200)
    );
    assert_eq!(view.panels[0].body, "greatest: 1.3 now\ntarget: 1");
    assert_eq!(stage.updated("yard", &id), Some(200));
    // Moved by a participant, an update leaves it where they put it.
    stage
        .place(&walls, &id, Target::Slot("environment-0".into(), 4))
        .unwrap();
    stage.publish(&walls, pressure_panel(1.4), 400).unwrap();
    assert_eq!(shown(&stage, &walls)[0].1, on("environment-0", 4));
    // A slot taken goes into the wall's most prominent free slot; one past the wall's last, or a wall the environment
    // does not declare, is refused.
    stage
        .publish(
            &walls,
            publishing(Panel::text("rota", "Rota", "Feed the hens").on_wall(0, Some(4))),
            0,
        )
        .unwrap();
    assert_eq!(
        shown(&stage, &walls)[1],
        ("Rota".into(), on("environment-0", 0))
    );
    assert!(
        stage
            .publish(
                &walls,
                publishing(Panel::text("x", "", "").on_wall(0, Some(6))),
                0
            )
            .is_err()
    );
    assert!(
        stage
            .publish(
                &walls,
                publishing(Panel::text("x", "", "").on_wall(2, None)),
                0
            )
            .is_err()
    );
    // Retracted, it goes; again, nothing.
    assert!(stage.retract("yard", "rota"));
    assert!(!stage.retract("yard", "rota"));
    assert!(!stage.retract("elsewhere", "rota"));
    assert_eq!(shown(&stage, &walls).len(), 1);
    // Nothing of the environment's is told the agent as its own to update.
    assert!(stage.content(&walls, &id).is_none());
    assert_eq!(stage.published("yard", &id), Some(("pressure", None)));
    assert!(stage.published("yard", "panel-999").is_none());
}

#[test]
fn an_environment_publishes_at_most_so_many_panels_in_a_space() {
    let declared = roomy();
    let walls = Walls {
        declared: &declared,
        ..yard()
    };
    let mut stage = Stage::default();
    let text = |id: String, body: &str| publishing(Panel::text(id, "", body).on_wall(1, None));
    // As many as it may show, 32, each on a wall it names, the second wall's 24 slots filling before it is refused one.
    assert_eq!(PUBLISHED, 32);
    for index in 0..PUBLISHED {
        let panel = Panel::text(format!("n{index}"), "", "b").on_wall(index as u32 / 24, None);
        assert_eq!(
            stage.publish(&walls, publishing(panel), 0),
            Ok(Change::Added)
        );
    }
    assert_eq!(shown(&stage, &walls).len(), PUBLISHED);
    // One more is refused, saying how many it may show, wherever it would go.
    let refused = format!("An environment publishes at most {PUBLISHED} panels in a space.");
    assert_eq!(
        stage.publish(&walls, text("more".into(), "b"), 0),
        Err(refused.as_str())
    );
    assert_eq!(shown(&stage, &walls).len(), PUBLISHED);
    // Every one is still updated in place, and one retracted makes room for another.
    for index in 0..PUBLISHED {
        assert_eq!(
            stage.publish(&walls, text(format!("n{index}"), "c"), 1),
            Ok(Change::Updated)
        );
    }
    assert!(stage.retract("yard", "n31"));
    assert_eq!(
        stage.publish(&walls, text("more".into(), "b"), 2),
        Ok(Change::Added)
    );
    // A wall with no free slot, every panel on it the environment's own, refuses another, saying so, rather than
    // putting it elsewhere: its first wall has 24 slots, each taken.
    assert!(stage.retract("yard", "more"));
    assert_eq!(
        stage.publish(&walls, publishing(Panel::text("crowded", "", "b")), 3),
        Err("That wall is full.")
    );
    assert_eq!(shown(&stage, &walls).len(), PUBLISHED - 1);
}

#[test]
fn a_full_wall_gives_up_only_the_agent_s_panels_and_never_its_environment_s() {
    let walls = yard();
    let mut stage = Stage::default();
    let note = |index: usize| {
        crate::stage::Content::parse(&format!("title: Note {index}\n---\nx")).unwrap()
    };
    let board = Destination::Wall {
        wall: "environment-0".into(),
        slot: None,
    };
    let rota = |id: &str| publishing(Panel::text(id, id, "x"));
    let titles = |stage: &Stage, wall: &str| -> Vec<String> {
        let shown = shown(stage, &walls).into_iter();
        let on =
            shown.filter(|(_, place)| matches!(place, Place::Wall { wall: on, .. } if on == wall));
        on.map(|(title, _)| title).collect()
    };
    assert_eq!(stage.publish(&walls, rota("a"), 0), Ok(Change::Added));
    // The agent's panels fill the wall, giving up only their own once it is full.
    for index in 0..7 {
        stage.project_to(&walls, note(index), &board, 0);
    }
    assert!(titles(&stage, "environment-0").contains(&"a".to_owned()));
    // The agent's panels make room for the environment's.
    assert_eq!(stage.publish(&walls, rota("b"), 0), Ok(Change::Added));
    // A wall of a participant's and the environment's own refuses another.
    let ids: Vec<String> = stage
        .view("s", &walls, 0)
        .panels
        .into_iter()
        .map(|panel| panel.id)
        .collect();
    for id in &ids {
        stage
            .place(&walls, id, Target::Wall("environment-0".into()))
            .unwrap();
    }
    assert_eq!(
        stage.publish(&walls, rota("c"), 0),
        Err("That wall is full.")
    );
    // The agent's newcomer on a wall where every panel is arranged gives up the one there longest of those that are
    // not the environment's, which stay, though they have been there longest of all.
    let before = titles(&stage, "environment-0");
    stage.project_to(&walls, note(10), &board, 0);
    let after = titles(&stage, "environment-0");
    assert_eq!(after.len(), 6);
    assert!(
        ["a", "b", "Note 10"]
            .iter()
            .all(|title| after.contains(&title.to_string()))
    );
    let gone: Vec<&String> = before
        .iter()
        .filter(|title| !after.contains(title))
        .collect();
    assert_eq!(gone.len(), 1);
    assert!(gone[0].starts_with("Note "));
    // Once every slot of the wall holds the environment's own, it gives none of them up: the agent's newcomer goes
    // onto the Tomori wall instead, as when nobody asked, and a panel moved onto the wall, by hand or by the agent, is
    // refused, saying so, unless it goes into one of its slots, changing places with the panel there.
    for kept in &after {
        if kept.starts_with("Note ") {
            let id = stage.view("s", &walls, 0);
            let id = id.panels.iter().find(|panel| panel.title == *kept).unwrap();
            stage.dismiss(&yard(), &id.id.clone()).unwrap();
        }
    }
    for id in ["c", "d", "e", "f"] {
        assert_eq!(stage.publish(&walls, rota(id), 0), Ok(Change::Added));
    }
    assert_eq!(
        titles(&stage, "environment-0"),
        ["a", "b", "c", "d", "e", "f"]
    );
    let newcomer = stage.project_to(&walls, note(11), &board, 0);
    assert_eq!(
        titles(&stage, "environment-0"),
        ["a", "b", "c", "d", "e", "f"]
    );
    assert_eq!(titles(&stage, TOMORI_WALL), ["Note 11"]);
    assert_eq!(
        stage.place(&walls, &newcomer, Target::Wall("environment-0".into())),
        Err(
            "That wall is full of its environment's own panels, which stay on it: put it on another wall, or leave it floating."
        )
    );
    assert_eq!(
        stage.place(&walls, &newcomer, Target::Slot("environment-0".into(), 0)),
        Err("That panel is its environment's, and stays on its walls or floating in it.")
    );
    assert_eq!(titles(&stage, TOMORI_WALL), ["Note 11"]);
    assert_eq!(
        titles(&stage, "environment-0"),
        ["a", "b", "c", "d", "e", "f"]
    );
    // One of the environment's own put back on the wall it is on stays in its slot; and nothing was dismissed, so
    // each is still updated in place.
    let a = stage.view("s", &walls, 0);
    let a = a.panels.iter().find(|panel| panel.title == "a").unwrap();
    assert_eq!(
        stage.place(&walls, &a.id, Target::Wall("environment-0".into())),
        Ok(false)
    );
    assert_eq!(
        stage.publish(&walls, publishing(Panel::text("a", "a", "y")), 1),
        Ok(Change::Updated)
    );
    // Left floating, the agent's panel changes places with the environment's in a slot, which floats where it was,
    // still in its environment; and from the Tomori wall a panel goes into a slot of the Tomori wall, or into a slot
    // of another wall of the environment's that has a free one, the panel there moving along.
    stage
        .place(&walls, &newcomer, Target::Free(pose(0.0, -1.0)))
        .unwrap();
    assert_eq!(
        stage.place(&walls, &newcomer, Target::Slot("environment-0".into(), 0)),
        Ok(true)
    );
    assert_eq!(titles(&stage, "environment-0").len(), 6);
    assert!(titles(&stage, TOMORI_WALL).is_empty());
    assert_eq!(shown(&stage, &walls).len(), 7);
    let other = stage.project(&walls, note(12), None, 0);
    assert_eq!(
        stage.place(&walls, &other, Target::Slot(TOMORI_WALL.into(), 3)),
        Ok(true)
    );
    let elsewhere = publishing(Panel::text("g", "g", "x").on_wall(1, Some(1)));
    assert_eq!(stage.publish(&walls, elsewhere, 2), Ok(Change::Added));
    assert_eq!(
        stage.place(&walls, &other, Target::Slot("environment-1".into(), 1)),
        Ok(true)
    );
    assert!(titles(&stage, TOMORI_WALL).is_empty());
    assert_eq!(titles(&stage, "environment-1").len(), 2);
    assert_eq!(
        stage.place(&walls, "panel-999", Target::Wall("environment-0".into())),
        Err("That panel is not here.")
    );
}

#[test]
fn a_dismissed_panel_stays_away_until_retracted_or_the_space_comes_back() {
    let walls = yard();
    let mut stage = Stage::default();
    let rota = |body: &str| publishing(Panel::text("rota", "Rota", body));
    stage.publish(&walls, rota("one"), 0).unwrap();
    let id = stage.view("s", &walls, 0).panels[0].id.clone();
    stage.dismiss(&yard(), &id).unwrap();
    assert_eq!(stage.publish(&walls, rota("two"), 0), Ok(Change::Unchanged));
    assert!(shown(&stage, &walls).is_empty());
    // Retracted and published again, it is back.
    assert!(!stage.retract("yard", "rota"));
    assert_eq!(stage.publish(&walls, rota("three"), 0), Ok(Change::Added));
    let id = stage.view("s", &walls, 0).panels[0].id.clone();
    stage.dismiss(&yard(), &id).unwrap();
    // Leaving the environment forgets it was dismissed; on returning, its side publishes it again.
    stage.leave("yard");
    assert_eq!(stage.publish(&walls, rota("four"), 0), Ok(Change::Added));
    assert_eq!(shown(&stage, &walls).len(), 1);
    // An agent's panel dismissed is simply gone.
    let notes = stage.project(
        &walls,
        crate::stage::Content::parse("title: Notes\n---\nx").unwrap(),
        None,
        0,
    );
    stage.dismiss(&yard(), &notes).unwrap();
    assert!(stage.dismiss(&yard(), &notes).is_err());
}

#[test]
fn what_a_run_that_ended_published_rests_where_it_was_holding_no_slot_until_published_again() {
    let declared = roomy();
    let walls = Walls {
        declared: &declared,
        ..yard()
    };
    let mut stage = Stage::default();
    let chart = |latest: f32| {
        publishing(
            Panel::chart("pressure", "Pressure", pressure(latest))
                .on_wall(0, Some(3))
                .with_reference("chart/pressure"),
        )
    };
    stage.publish(&walls, chart(1.1), 0).unwrap();
    let id = stage.view("s", &walls, 0).panels[0].id.clone();
    stage
        .place(&walls, &id, Target::Slot("environment-0".into(), 5))
        .unwrap();
    // Written as where it rests, with nothing of what it showed, so that no stale value can come back.
    let json = serde_json::to_value(&stage).unwrap();
    let room = &json["rooms"]["yard"];
    assert!(room.get("panels").is_none(), "{room}");
    assert_eq!(room["resting"][0]["id"], "pressure");
    assert_eq!(room["resting"][0]["panel"], id.as_str());
    assert!(!json.to_string().contains("greatest"));
    let mut stage: Stage = serde_json::from_value(json).unwrap();
    assert!(shown(&stage, &walls).is_empty());
    assert!(stage.referenced("yard", "chart/pressure").is_none());
    // Resting, it holds no slot: the agent's panels take its slot, and it counts towards no limit.
    let board = Destination::Wall {
        wall: "environment-0".into(),
        slot: Some(5),
    };
    let note = crate::stage::Content::parse("title: Note\n---\nx").unwrap();
    let agent = stage.project_to(&walls, note, &board, 0);
    assert_eq!(
        shown(&stage, &walls),
        [("Note".into(), on("environment-0", 5))]
    );
    for index in 0..PUBLISHED {
        let panel = Panel::text(format!("n{index}"), "", "b")
            .on_wall(index as u32 % 2, Some(index as u32 / 2));
        assert_eq!(
            stage.publish(&walls, publishing(panel), 0),
            Ok(Change::Added)
        );
    }
    for index in 0..PUBLISHED {
        assert!(stage.retract("yard", &format!("n{index}")));
    }
    // Published again, it is the same panel, back in its slot where that is free, or else as a new one goes.
    assert_eq!(stage.publish(&walls, chart(1.2), 50), Ok(Change::Added));
    let view = stage.view("s", &walls, 50);
    let back = view.panel(&id).unwrap();
    assert_eq!(back.place, on("environment-0", 3));
    assert_eq!(back.body, "greatest: 1.2 now\ntarget: 1");
    assert_eq!(
        stage.referenced("yard", "chart/pressure").unwrap(),
        "Pressure: greatest: 1.2 now; target: 1"
    );
    assert!(stage.referenced("yard", "chart/other").is_none());
    assert!(stage.referenced("elsewhere", "chart/pressure").is_none());
    stage.dismiss(&yard(), &agent).unwrap();
    // Its side failing rests it again, once; back in its own slot, now free; floating, it floats again.
    assert!(stage.rest("yard"));
    assert!(!stage.rest("yard"));
    assert!(!stage.rest("elsewhere"));
    assert!(shown(&stage, &walls).is_empty());
    stage.publish(&walls, chart(1.3), 60).unwrap();
    assert_eq!(
        stage.view("s", &walls, 60).panel(&id).unwrap().place,
        on("environment-0", 3)
    );
    stage
        .place(&walls, &id, Target::Free(pose(0.0, -1.0)))
        .unwrap();
    stage.rest("yard");
    stage.publish(&walls, chart(1.4), 70).unwrap();
    assert!(matches!(
        stage.view("s", &walls, 70).panel(&id).unwrap().place,
        Place::Free { .. }
    ));
    // On the Tomori wall by another path, or a slot gone, it comes back as a new one goes.
    stage.rest("yard");
    stage.rooms.get_mut("yard").unwrap().resting[0].place = Placed::Wall {
        wall: crate::stage::TOMORI_WALL.into(),
        slot: 0,
    };
    stage.publish(&walls, chart(1.5), 80).unwrap();
    assert_eq!(
        stage.view("s", &walls, 80).panel(&id).unwrap().place,
        on("environment-0", 3)
    );
}

#[test]
fn a_resting_panel_keeps_its_place_for_a_few_runs_that_do_not_publish_it() {
    let walls = yard();
    let mut stage = Stage::default();
    let alert = || publishing(Panel::text("alert", "Alert", "x").on_wall(0, Some(4)));
    let steady = |body: &str| publishing(Panel::text("steady", "Steady", body).on_wall(1, None));
    stage.publish(&walls, alert(), 0).unwrap();
    stage.publish(&walls, steady("x"), 0).unwrap();
    let id = stage.view("s", &walls, 0).panels[0].id.clone();
    stage.leave("yard");
    // A conditional panel absent for a run, or as many as are allowed, comes back where it was.
    for run in 0..RESTING_RUNS {
        stage
            .publish(&walls, steady(&format!("run {run}")), 0)
            .unwrap();
        stage.leave("yard");
    }
    stage.publish(&walls, alert(), 0).unwrap();
    let back = stage.view("s", &walls, 0);
    assert_eq!(back.panel(&id).unwrap().place, on("environment-0", 4));
    stage.leave("yard");
    // Absent for one run more than that, it is forgotten.
    for run in 0..=RESTING_RUNS {
        stage
            .publish(&walls, steady(&format!("again {run}")), 0)
            .unwrap();
        stage.leave("yard");
    }
    let resting: Vec<&str> = stage.rooms["yard"]
        .resting
        .iter()
        .map(|resting| resting.id.as_str())
        .collect();
    assert_eq!(resting, ["steady"]);
    // A run that publishes nothing misses nothing.
    assert!(!stage.rest("yard"));
    assert_eq!(stage.rooms["yard"].resting[0].missed, 0);
    // A retraction forgets one resting too.
    assert!(!stage.retract("yard", "steady"));
    assert!(stage.rooms["yard"].resting.is_empty());
}

#[test]
fn at_most_so_many_panels_rest_those_missed_most_forgotten_first() {
    let declared = roomy();
    let walls = Walls {
        declared: &declared,
        ..yard()
    };
    let mut stage = Stage::default();
    for round in 0..3 {
        for index in 0..PUBLISHED {
            let panel =
                Panel::text(format!("r{round}p{index}"), "", "x").on_wall(index as u32 % 2, None);
            stage.publish(&walls, publishing(panel), 0).unwrap();
        }
        stage.leave("yard");
    }
    let resting = &stage.rooms["yard"].resting;
    assert_eq!(resting.len(), RESTING);
    assert!(resting.iter().all(|resting| resting.missed <= 1));
    assert!(resting.iter().any(|resting| resting.id == "r2p0"));
    // Written with how many runs missed each, and read so.
    let kept: Stage = serde_json::from_value(serde_json::to_value(&stage).unwrap()).unwrap();
    assert_eq!(kept.rooms["yard"].resting, stage.rooms["yard"].resting);
    // With as many shown again, what is written is as many as may rest, as they would rest were the run to end now.
    for index in 0..PUBLISHED {
        let panel = Panel::text(format!("r3p{index}"), "", "x").on_wall(index as u32 % 2, None);
        stage.publish(&walls, publishing(panel), 0).unwrap();
    }
    let written = serde_json::to_value(&stage).unwrap();
    assert_eq!(
        written["rooms"]["yard"]["resting"]
            .as_array()
            .unwrap()
            .len(),
        RESTING
    );
    let kept: Stage = serde_json::from_value(written).unwrap();
    stage.rest("yard");
    assert_eq!(kept.rooms["yard"].resting, stage.rooms["yard"].resting);
}

#[test]
fn an_environment_s_ids_and_the_agent_s_are_apart() {
    let walls = yard();
    let mut stage = Stage::default();
    stage
        .publish(&walls, publishing(Panel::text("agenda", "Theirs", "x")), 0)
        .unwrap();
    // The agent's panel of the same id is its own, and updating it leaves the environment's alone, and the other way.
    let mine = crate::stage::Content::parse("id: agenda\ntitle: Mine\n---\none").unwrap();
    let ids = stage.project_placed(&walls, vec![(mine, Destination::Tomori, false)], 0);
    assert!(!ids[0].1);
    let again = crate::stage::Content::parse("id: agenda\ntitle: Mine\n---\ntwo").unwrap();
    let updated = stage.project_placed(&walls, vec![(again, Destination::Tomori, false)], 0);
    assert_eq!(updated[0], (ids[0].0.clone(), true));
    stage
        .publish(&walls, publishing(Panel::text("agenda", "Theirs", "y")), 0)
        .unwrap();
    let titles: Vec<(String, String)> = stage
        .view("s", &walls, 0)
        .panels
        .into_iter()
        .map(|panel| (panel.title, panel.body))
        .collect();
    assert_eq!(
        titles,
        [("Mine".into(), "two".into()), ("Theirs".into(), "y".into())]
    );
    assert_eq!(stage.named("yard", "agenda"), Some(ids[0].0.clone()));
}

#[test]
fn a_panel_dismissed_stays_away_through_its_side_failing_and_one_floated_away_too() {
    let walls = yard();
    let mut stage = Stage::default();
    stage
        .publish(&walls, publishing(Panel::text("rota", "Rota", "x")), 0)
        .unwrap();
    let id = stage.view("s", &walls, 0).panels[0].id.clone();
    stage.dismiss(&yard(), &id).unwrap();
    stage.rest("yard");
    assert_eq!(
        stage.publish(&walls, publishing(Panel::text("rota", "Rota", "y")), 0),
        Ok(Change::Unchanged)
    );
    // One floating longest, given up as another is left floating, stays away as one a full wall gives up does.
    stage
        .publish(&walls, publishing(Panel::text("float", "Float", "x")), 0)
        .unwrap();
    let float = stage
        .view("s", &walls, 0)
        .panels
        .into_iter()
        .find(|panel| panel.title == "Float")
        .unwrap()
        .id;
    stage
        .place(&walls, &float, Target::Free(pose(0.0, -1.0)))
        .unwrap();
    for index in 0..crate::stage::FLOATING {
        let note = crate::stage::Content::parse(&format!("title: N{index}\n---\nx")).unwrap();
        let panel = stage.project(&walls, note, None, 0);
        stage
            .place(&walls, &panel, Target::Free(spread(index, 1.0, 0.4)))
            .unwrap();
    }
    assert!(stage.view("s", &walls, 0).panel(&float).is_none());
    assert_eq!(
        stage.publish(&walls, publishing(Panel::text("float", "Float", "y")), 0),
        Ok(Change::Unchanged)
    );
}

#[test]
fn an_environment_s_panel_stays_in_its_environment_and_only_it_changes_what_it_shows() {
    let walls = yard();
    let mut stage = Stage::default();
    stage
        .publish(
            &walls,
            publishing(Panel::text("rota", "Rota", "- [ ] Feed the hens")),
            0,
        )
        .unwrap();
    let id = stage.view("s", &walls, 0).panels[0].id.clone();
    // Not onto the Tomori wall, by a participant or the agent, which would take it out of its environment.
    assert!(
        stage
            .place(&walls, &id, Target::Wall(TOMORI_WALL.into()))
            .is_err()
    );
    assert!(
        stage
            .place(&walls, &id, Target::Slot(TOMORI_WALL.into(), 0))
            .is_err()
    );
    assert!(stage.relocate(&walls, &id, &Destination::Tomori).is_err());
    // Anywhere else in it.
    assert_eq!(
        stage.relocate(&walls, &id, &Destination::Free(pose(1.0, -1.0))),
        Ok(true)
    );
    assert_eq!(
        stage.place(&walls, &id, Target::Wall("environment-1".into())),
        Ok(true)
    );
    // Neither the agent's update nor a participant's tick changes it.
    let content = crate::stage::Content::parse("title: Mine\n---\nx").unwrap();
    assert!(stage.revise(&walls, &id, content, None, 0).is_err());
    assert!(stage.tick_item(&walls, &id, 0, true).is_err());
    // An agent's panel still goes onto the Tomori wall.
    let notes = stage.project(
        &walls,
        crate::stage::Content::parse("title: Notes\n---\nx").unwrap(),
        Some([3.0, 1.5, -4.0]),
        0,
    );
    assert_eq!(
        stage.place(&walls, &notes, Target::Wall(TOMORI_WALL.into())),
        Ok(true)
    );
}

#[test]
fn a_published_panel_is_described_to_the_agent_as_its_environment_s() {
    let walls = yard();
    let mut stage = Stage::default();
    stage
        .publish(
            &walls,
            publishing(
                Panel::chart("pressure", "Pressure", pressure(1.25))
                    .with_reference("chart/pressure"),
            ),
            0,
        )
        .unwrap();
    stage
        .publish(&walls, publishing(Panel::text("rota", "Rota", "Feed")), 0)
        .unwrap();
    let ids: Vec<String> = stage
        .view("s", &walls, 0)
        .panels
        .into_iter()
        .map(|panel| panel.id)
        .collect();
    assert_eq!(
        stage.describe(&walls, &ids[0], None, 120).unwrap(),
        "chart panel \"Pressure\" (on a wall, published 2 minutes ago; the yard environment's, id yard:chart/pressure; \
         shows: \"greatest: 1.25 now, target: 1\")"
    );
    assert_eq!(
        stage.describe(&walls, &ids[1], None, 0).unwrap(),
        "panel \"Rota\" (on a wall, published just now; the yard environment's)"
    );
}

/// The projection whole, as JSON, of a space at every limit whose environment publishes as many panels of `chart` as
/// it may, on the first of its sixteen walls, of 4 by 2 slots so that they hold them, within the slots an older client
/// is sent, every other slot and floating place holding
/// `largest`: as the server holds it, and as an older client is sent it, each as it is sent, in a message, its walls and
/// panels at the longest versions and its charts at the longest data seqs there can be, with how many of them it shows
/// as charts, every one of them sent to both.
fn at_every_limit(chart: &Chart, largest: &crate::stage::Content) -> [(usize, usize); 2] {
    let declared: Vec<crate::EnvironmentWall> = (0..DECLARED_WALLS)
        .map(|index| {
            crate::EnvironmentWall::new(pose(index as f32 - 3.0, -5.0))
                .with_grid(tomori_protocol::Grid::from((4, 2)))
        })
        .collect();
    let walls = Walls {
        declared: &declared,
        ..yard()
    };
    let mut stage = Stage::default();
    for index in 0..PUBLISHED {
        let title = "\u{1F600}".repeat(blocks::TITLE_CHARS);
        let panel = Panel::chart(format!("chart-{index}"), title, chart.clone())
            .on_wall((index / 8) as u32, None);
        stage.publish(&walls, publishing(panel), 0).unwrap();
    }
    let published: Vec<String> = stage
        .view("s", &walls, 0)
        .panels
        .into_iter()
        .map(|panel| panel.id)
        .collect();
    assert_eq!(published.len(), PUBLISHED);
    for index in 0..crate::stage::FLOATING {
        let id = stage.project(&walls, largest.clone(), None, 0);
        stage
            .place(&walls, &id, Target::Free(spread(index, 0.0, 0.4)))
            .unwrap();
    }
    for _ in 0..crate::stage::ADDED_WALLS {
        stage.add_wall(&walls, &pose(0.0, 5.0), None).unwrap();
    }
    for _ in 0..tomori_protocol::WALL_SLOTS {
        stage.project(&walls, largest.clone(), None, 0);
    }
    // Every other wall, its environment's own, where the charts leave room, and those added, by name.
    let others: Vec<(String, u32)> = stage
        .view("s", &walls, 0)
        .walls
        .iter()
        .filter(|wall| wall.kind != tomori_protocol::WallKind::Tomori)
        .map(|wall| (wall.id.clone(), wall.slots()))
        .collect();
    for (wall, slots) in &others {
        for _ in 0..*slots {
            stage.project_to(
                &walls,
                largest.clone(),
                &Destination::Wall {
                    wall: wall.clone(),
                    slot: None,
                },
                0,
            );
        }
    }
    let most = stage.view(&"s".repeat(64), &walls, u64::MAX);
    let today = stage.view_within(
        &"s".repeat(64),
        &walls,
        u64::MAX,
        &tomori_protocol::Limits::TODAY,
    );
    assert_eq!(
        today.panels.len(),
        tomori_protocol::PROJECTION_PANELS as usize
    );
    [most, today].map(|mut view| {
        assert!(published.iter().all(|id| view.panel(id).is_some()));
        let charts = view
            .panels
            .iter()
            .filter(|panel| panel.chart.is_some())
            .count();
        // As the hub sends it: versioned after it is fitted, at a revision.
        for wall in &mut view.walls {
            wall.version = u64::MAX;
        }
        for panel in &mut view.panels {
            panel.version = u64::MAX;
            panel.data_seq = if panel.chart.is_some() { u64::MAX } else { 0 };
        }
        let message = tomori_protocol::ServerMessage::Projection {
            projection: Box::new(view.with_revision(u64::MAX)),
        };
        (serde_json::to_vec(&message).unwrap().len(), charts)
    })
}

#[test]
fn a_projection_with_every_published_chart_at_its_largest_fits_in_one_frame() {
    // Every slot of every wall full of the largest text panels there are, but those the environment's charts take.
    let wide = |count: usize| "\u{1F600}".repeat(count);
    let source = format!(
        "title: {}\na: {}\nb: {}\nc: {}\nd: {}\n---\n{}",
        wide(200),
        wide(200),
        wide(200),
        wide(200),
        wide(200),
        vec![wide(40); 2 * blocks::BODY_LINES].join("\n")
    );
    let text = crate::stage::Content::parse(&source).unwrap();
    // Or of the largest code panels, as many of them sent short as the frame needs, each longer than a text panel.
    let lines = tomori_protocol::CODE_LINES;
    let body = format!(
        "{}…",
        vec![wide(tomori_protocol::CODE_CHARS / lines); lines].join("\n")
    );
    let length = body.chars().count() as u32;
    let spans = (0..tomori_protocol::CODE_SPANS as u32)
        .map(|index| {
            tomori_protocol::Span::new(
                length - 2 * tomori_protocol::CODE_SPANS as u32 + 2 * index,
                1,
                tomori_protocol::TokenKind::Punctuation,
            )
        })
        .collect();
    let code = crate::stage::Content {
        title: format!("{}…", wide(blocks::TITLE_CHARS)),
        body,
        meta: Vec::new(),
        code: Some(
            tomori_protocol::Code::new(format!("{}…", wide(blocks::META_KEY_CHARS)), u32::MAX)
                .with_spans(spans),
        ),
        slug: Some("a".repeat(blocks::ID_CHARS)),
        image: None,
    };
    // A chart of every series at every point, as a live chart is, its lines unbroken.
    let mut plotted = largest_chart();
    for series in &mut plotted.series {
        series.breaks.clear();
    }
    // Every chart is shown, to a client that takes the most and to an older one, which is sent as much as today's
    // limits hold, its environment's walls before those owners add and every panel on a wall: the message whole, with
    // every version and data seq at its longest, within each one's frame. The largest chart there can be, its lines
    // broken after every point, is larger still, and so many among the largest text panels could not all be sent to
    // an older client: it is sent 29 of them as charts, as many as fit, the frame then within 8 KB of full, and the
    // other three as their summaries.
    let largest = largest_chart();
    for (chart, others, charts, least) in [
        (&plotted, &text, PUBLISHED, 900_000),
        (&plotted, &code, PUBLISHED, 900_000),
        (
            &largest,
            &text,
            29,
            tomori_protocol::MAX_FRAME_BYTES - 8_000,
        ),
        (&largest, &code, PUBLISHED, 900_000),
    ] {
        let [(most, shown), (today, sent)] = at_every_limit(chart, others);
        assert_eq!((shown, sent), (PUBLISHED, charts));
        assert!(most <= tomori_protocol::MOST_FRAME_BYTES, "{most} bytes");
        let frames = least..=tomori_protocol::MAX_FRAME_BYTES;
        assert!(frames.contains(&today), "{today} bytes");
    }
}

#[test]
fn charts_that_do_not_fit_in_a_frame_are_sent_as_their_summaries_the_oldest_first() {
    let walls = yard();
    let mut stage = Stage::default();
    let chart = |index: usize, latest: f32| {
        publishing(Panel::chart(
            format!("chart-{index}"),
            format!("Chart {index}"),
            pressure(latest),
        ))
    };
    for index in 0..3 {
        stage.publish(&walls, chart(index, 1.0), 0).unwrap();
    }
    let whole = stage.view("s", &walls, 0);
    let size = |projection: &tomori_protocol::Projection| crate::stage::json_size(projection);
    let charts = |projection: &tomori_protocol::Projection| -> Vec<bool> {
        projection
            .panels
            .iter()
            .map(|panel| panel.chart.is_some())
            .collect()
    };
    // The first chart published was put where it is longest ago.
    let placed: Vec<u64> = (0..whole.panels.len() as u64).collect();
    // With room for all but one chart, the one put where it is longest ago is sent as its summary: the same panel, a
    // text panel an older client draws, where it is, at its version.
    let mut fitted = whole.clone();
    crate::stage::fit(&mut fitted, &placed, size(&whole) - 1);
    assert_eq!(charts(&fitted), [false, true, true]);
    assert!(size(&fitted) < size(&whole));
    let (was, short) = (&whole.panels[0], &fitted.panels[0]);
    assert_eq!(short.body, "greatest: 1 now\ntarget: 1");
    let mut summary = was.clone();
    summary.chart = None;
    assert_eq!(*short, summary);
    // With room for none, every one is; with room for all, none.
    let mut fitted = whole.clone();
    crate::stage::fit(&mut fitted, &placed, 0);
    assert_eq!(charts(&fitted), [false, false, false]);
    let mut fitted = whole.clone();
    crate::stage::fit(&mut fitted, &placed, size(&whole));
    assert_eq!(fitted, whole);
    // A client that applies data ops, sent a chart as its summary, is sent its next point as the panel whole, which
    // applied makes what it is sent whole.
    let mut before = whole.clone().with_revision(1);
    crate::stage::fit(&mut before, &placed, 0);
    stage.publish(&walls, chart(0, 1.5), 5).unwrap();
    let mut after = stage.view("s", &walls, 0).with_revision(2);
    after.panels[0].data_seq = 2;
    crate::stage::fit(&mut after, &placed, 0);
    let patch = tomori_protocol::ProjectionPatch::between_live(&before, &after);
    assert_eq!(patch.whole(), 1);
    let mut applied = before.clone();
    patch.apply(&mut applied).unwrap();
    assert_eq!(applied, after);
    assert_eq!(applied.panels[0].body, "greatest: 1.5 now\ntarget: 1");
}

/// Whether the wall `wall` is locked, as the space shows it.
fn locked(stage: &Stage, walls: &Walls<'_>, wall: &str) -> bool {
    let view = stage.view("s", walls, 0);
    view.walls
        .iter()
        .find(|shown| shown.id == wall)
        .unwrap()
        .is_locked()
}

#[test]
fn a_locked_wall_takes_its_environment_s_panels_their_updates_and_participants_moves_as_any_wall() {
    let walls = yard();
    let mut stage = Stage::default();
    assert!(locked(&stage, &walls, "environment-0"));
    // Published onto it, updated in place, as onto an unlocked wall, while the wall itself changes for nobody.
    let chart = |latest: f32| {
        publishing(Panel::chart("pressure", "Pressure", pressure(latest)).on_wall(0, Some(1)))
    };
    assert_eq!(stage.publish(&walls, chart(1.1), 0), Ok(Change::Added));
    let id = stage.view("s", &walls, 0).panels[0].id.clone();
    let grow = WallChange {
        grid: Some(tomori_protocol::Grid::from((4, 2))),
        ..WallChange::default()
    };
    assert_eq!(
        stage.set_wall(&walls, "environment-0", grow),
        Err(crate::stage::LOCKED)
    );
    assert_eq!(stage.publish(&walls, chart(1.2), 10), Ok(Change::Updated));
    let view = stage.view("s", &walls, 10);
    assert_eq!(&view.panels[0].place, &on("environment-0", 1));
    // Filled to its last slot, each panel published goes up, and one more is refused only as a full wall is.
    for slot in 0..5 {
        let panel = Panel::text(format!("n{slot}"), "", "b").on_wall(0, None);
        assert_eq!(
            stage.publish(&walls, publishing(panel), 0),
            Ok(Change::Added)
        );
    }
    let more = publishing(Panel::text("n5", "", "b").on_wall(0, None));
    assert_eq!(stage.publish(&walls, more, 0), Err("That wall is full."));
    // Full and locked, its panels still update in place.
    assert_eq!(stage.publish(&walls, chart(1.3), 20), Ok(Change::Updated));
    for slot in 0..5 {
        assert!(stage.retract("yard", &format!("n{slot}")));
    }
    // A participant moves it within the locked wall, off it and back, and dismisses it, as on any wall: the lock is the
    // wall's, not what is on it.
    stage
        .place(&walls, &id, Target::Slot("environment-0".into(), 4))
        .unwrap();
    stage
        .place(&walls, &id, Target::Free(pose(0.0, -1.0)))
        .unwrap();
    stage
        .place(&walls, &id, Target::Slot("environment-0".into(), 2))
        .unwrap();
    assert_eq!(shown(&stage, &walls)[0].1, on("environment-0", 2));
    // Resting, it comes back to its slot on the locked wall when published again.
    assert!(stage.rest("yard"));
    assert_eq!(stage.publish(&walls, chart(1.4), 30), Ok(Change::Added));
    assert_eq!(
        stage.view("s", &walls, 30).panel(&id).unwrap().place,
        on("environment-0", 2)
    );
    assert!(locked(&stage, &walls, "environment-0"));
    stage.dismiss(&yard(), &id).unwrap();
    assert_eq!(stage.publish(&walls, chart(1.5), 40), Ok(Change::Unchanged));
    assert!(shown(&stage, &walls).is_empty());
    // Retracted on the locked wall, it goes.
    assert!(!stage.retract("yard", "pressure"));
    assert_eq!(stage.publish(&walls, chart(1.6), 50), Ok(Change::Added));
    assert!(stage.retract("yard", "pressure"));
    assert!(shown(&stage, &walls).is_empty());
}

#[test]
fn a_wall_unlocked_and_moved_is_kept_with_what_rests_on_it() {
    let walls = yard();
    let mut stage = Stage::default();
    stage
        .publish(
            &walls,
            publishing(Panel::text("rota", "Rota", "Feed the hens").on_wall(1, Some(3))),
            0,
        )
        .unwrap();
    assert_eq!(
        stage.lock_wall(&walls, "environment-1", false, false),
        Ok(true)
    );
    let moved = WallChange {
        pose: Some(pose(-2.0, -5.0)),
        ..WallChange::default()
    };
    assert_eq!(stage.set_wall(&walls, "environment-1", moved), Ok(true));
    // Written and read, the wall stays unlocked where it was moved, and the panel rests in its slot there.
    let mut kept: Stage = serde_json::from_value(serde_json::to_value(&stage).unwrap()).unwrap();
    assert!(!locked(&kept, &walls, "environment-1"));
    assert!(locked(&kept, &walls, "environment-0"));
    let wall = kept
        .view("s", &walls, 0)
        .walls
        .into_iter()
        .find(|wall| wall.id == "environment-1")
        .unwrap();
    assert_eq!(wall.pose.position, pose(-2.0, -5.0).position);
    stage.rest("yard");
    assert_eq!(kept.rooms["yard"].resting, stage.rooms["yard"].resting);
    kept.publish(
        &walls,
        publishing(Panel::text("rota", "Rota", "Collect eggs").on_wall(1, None)),
        0,
    )
    .unwrap();
    assert_eq!(
        shown(&kept, &walls),
        [("Rota".into(), on("environment-1", 3))]
    );
}

#[test]
fn an_agent_s_panel_floated_beside_someone_stands_clear_of_an_environment_s_and_never_moves_it() {
    let walls = yard();
    let mut stage = Stage::default();
    // Someone across the yard from its walls, looking away from them.
    let head = Pose::at([5.0, 1.6, 6.0]).facing_towards([5.0, 1.6, 7.0]);
    let beside = Destination::Beside {
        head,
        solids: Vec::new(),
    };
    let floating = |stage: &Stage, id: &str| {
        let view = stage.view("s", &walls, 0);
        let Some(Place::Free { pose }) = view.panel(id).map(|panel| &panel.place) else {
            unreachable!("{id} floats");
        };
        *pose
    };
    // Where the agent's first panel beside them would stand, a participant has floated the environment's.
    let note =
        |title: &str| crate::stage::Content::parse(&format!("title: {title}\n---\nx")).unwrap();
    let first = stage.project_to(&walls, note("First"), &beside, 0);
    let spot = floating(&stage, &first);
    stage.dismiss(&yard(), &first).unwrap();
    stage
        .publish(&walls, publishing(Panel::text("rota", "Rota", "x")), 0)
        .unwrap();
    let rota = stage.view("s", &walls, 0).panels[0].id.clone();
    stage.place(&walls, &rota, Target::Free(spot)).unwrap();
    // The agent's goes to the next spot beside them, and the environment's stays where it was put, still updated.
    let next = stage.project_to(&walls, note("Next"), &beside, 0);
    assert_ne!(floating(&stage, &next).position, spot.position);
    assert_eq!(floating(&stage, &rota), spot);
    stage
        .publish(&walls, publishing(Panel::text("rota", "Rota", "y")), 1)
        .unwrap();
    assert_eq!(floating(&stage, &rota), spot);
}

#[test]
fn what_could_never_be_published_is_known_before_it_waits_those_waiting_counted() {
    let walls = yard();
    let mut stage = Stage::default();
    let text = |id: &str| publishing(Panel::text(id, "", "x"));
    assert_eq!(
        stage.admits(
            &walls,
            &publishing(Panel::text("a", "", "x").on_wall(2, None)),
            &[]
        ),
        Err("The environment declares no such wall.")
    );
    assert_eq!(
        stage.admits(
            &walls,
            &publishing(Panel::text("a", "", "x").on_wall(0, Some(6))),
            &[]
        ),
        Err("That wall has no such slot.")
    );
    // Shown and waiting together, at most so many; an update of one shown or waiting is always taken.
    let declared = roomy();
    let walls = Walls {
        declared: &declared,
        ..yard()
    };
    let half = PUBLISHED / 2;
    for index in 0..half {
        stage
            .publish(&walls, text(&format!("s{index}")), 0)
            .unwrap();
    }
    // One fewer waiting than would make as many as it may show, with one shown waiting too, which counts once.
    let mut ids: Vec<String> = (0..PUBLISHED - half - 1)
        .map(|index| format!("w{index}"))
        .collect();
    ids.push("s0".into());
    let waiting: Vec<&str> = ids.iter().map(String::as_str).collect();
    assert_eq!(stage.admits(&walls, &text("last"), &waiting), Ok(()));
    ids.push("last".into());
    let waiting: Vec<&str> = ids.iter().map(String::as_str).collect();
    assert_eq!(
        stage.admits(&walls, &text("new"), &waiting),
        Err("An environment publishes at most 32 panels in a space.")
    );
    assert_eq!(stage.admits(&walls, &text("w1"), &waiting), Ok(()));
    assert_eq!(stage.admits(&walls, &text("s2"), &waiting), Ok(()));
}

#[test]
fn a_chart_s_update_is_patched_as_that_panel_alone() {
    let walls = yard();
    let mut stage = Stage::default();
    for (index, title) in ["Agenda", "Notes"].into_iter().enumerate() {
        let notice = Panel::text(format!("notice-{index}"), title, "- [ ] Feed the hens");
        stage.publish(&walls, publishing(notice), 0).unwrap();
    }
    let chart =
        |latest| Panel::chart("pressure", "Scaling pressure", pressure(latest)).on_wall(1, None);
    stage.publish(&walls, publishing(chart(0.9)), 0).unwrap();
    let before = stage.view("s", &walls, 10).with_revision(1);
    // The next poll's point, and so its summary, changes; nothing else does.
    assert_eq!(
        stage.publish(&walls, publishing(chart(1.4)), 5),
        Ok(Change::Updated)
    );
    let after = stage.view("s", &walls, 10).with_revision(2);
    let patch = tomori_protocol::ProjectionPatch::between(&before, &after);
    let mut applied = before.clone();
    patch.apply(&mut applied).unwrap();
    assert_eq!(applied, after);
    let updated = after
        .panels
        .iter()
        .find(|panel| panel.chart.is_some())
        .unwrap();
    assert_eq!(
        updated.chart.as_deref(),
        Some(&pressure(1.4).cleaned().unwrap())
    );
    assert_eq!(
        patch.ops,
        [tomori_protocol::PatchOp::Panel {
            panel: updated.clone(),
            after: None,
            first: false,
        }]
    );
    // The chart's panel alone, of 181 points, rather than the projection whole, and within what one may be.
    let size =
        |message: tomori_protocol::ServerMessage| serde_json::to_vec(&message).unwrap().len();
    let patched = size(tomori_protocol::ServerMessage::ProjectionPatch {
        patch: Box::new(patch),
    });
    let whole = size(tomori_protocol::ServerMessage::Projection {
        projection: Box::new(after),
    });
    assert!(patched < PUBLISHED_BYTES, "{patched} bytes");
    assert!(whole - patched > 400, "{patched} of {whole} bytes");
}

/// A pressure chart of 15 minutes at 5 seconds, 181 points, `polls` polls in: each poll moves its points along one.
fn window(polls: usize) -> Chart {
    let values = (polls..polls + 181)
        .map(|at| (at % 37 != 0).then(|| 0.5 + (at % 17) as f32 / 20.0))
        .collect();
    Chart::new(181)
        .with_labels(vec![
            AxisLabel::new(0, "-15 min"),
            AxisLabel::new(180, "now"),
        ])
        .with_series(Series::new(
            "greatest",
            Colour::Amber,
            SeriesStyle::Line,
            values,
        ))
        .with_reference(Reference::new(1.0, "target", Colour::White))
}

#[test]
fn a_live_chart_s_next_point_is_appended_apart_from_its_version() {
    let walls = yard();
    let mut stage = Stage::default();
    let chart =
        |polls| Panel::chart("pressure", "Scaling pressure", window(polls)).on_wall(1, None);
    stage.publish(&walls, publishing(chart(0)), 0).unwrap();
    // Versioned as the hub versions what it sends.
    let mut versions = crate::versions::Versions::default();
    let revision = std::cell::Cell::new(1);
    let mut versioned = |stage: &Stage| {
        let mut shown = stage.view("s", &walls, 10);
        versions.stamp(&mut shown, &revision);
        shown.with_revision(revision.get())
    };
    let before = versioned(&stage);
    // The next poll moves its points along one, and changes its summary: its data alone.
    assert_eq!(
        stage.publish(&walls, publishing(chart(1)), 5),
        Ok(Change::Updated)
    );
    revision.set(2);
    let after = versioned(&stage);
    let (was, now) = (&before.panels[0], &after.panels[0]);
    assert_eq!((now.version, now.data_seq), (was.version, 2));
    let live = tomori_protocol::ProjectionPatch::between_live(&before, &after);
    let [
        tomori_protocol::PatchOp::ChartAppend {
            values, from, seq, ..
        },
    ] = live.ops.as_slice()
    else {
        unreachable!("{:?}", live.ops);
    };
    assert_eq!((values.len(), values[0].len(), *from, *seq), (1, 1, 1, 2));
    let mut applied = before.clone();
    live.apply(&mut applied).unwrap();
    assert_eq!(applied, after);
    // An older client is sent the panel whole, as before.
    let older = tomori_protocol::ProjectionPatch::between(&before, &after);
    assert!(matches!(
        older.ops.as_slice(),
        [tomori_protocol::PatchOp::Panel { .. }]
    ));
    let size = |patch: tomori_protocol::ProjectionPatch| {
        serde_json::to_vec(&tomori_protocol::ServerMessage::ProjectionPatch {
            patch: Box::new(patch),
        })
        .unwrap()
        .len()
    };
    let (appended, whole) = (size(live), size(older));
    println!("A live chart's next point: {appended} bytes appended, {whole} bytes whole.");
    assert!(appended < 400, "{appended} bytes");
    assert!(appended * 4 < whole, "{appended} of {whole} bytes");
    // Retitled, its version changes, and it is sent whole to every client.
    let mut retitled = Panel::chart("pressure", "Pressure", window(1)).on_wall(1, None);
    retitled = retitled.on_wall(1, None);
    stage.publish(&walls, publishing(retitled), 6).unwrap();
    revision.set(3);
    let titled = versioned(&stage);
    assert_eq!(
        (titled.panels[0].version, titled.panels[0].data_seq),
        (3, 2)
    );
    assert!(matches!(
        tomori_protocol::ProjectionPatch::between_live(&after, &titled)
            .ops
            .as_slice(),
        [tomori_protocol::PatchOp::Panel { .. }]
    ));
}

/// A queue of `count` rows, `pod0` and on, each inspectable by `pod/N`.
fn queue(count: usize) -> Vec<Row> {
    (0..count)
        .map(|index| {
            Row::new(
                format!("pod{index}"),
                format!("pod{index} · {index} waiting"),
            )
            .with_reference(format!("pod/{index}"))
        })
        .collect()
}

#[test]
fn a_list_panel_s_body_is_a_list_item_a_row_its_highlight_a_pointer() {
    let list = publishing(Panel::list("queue", "Queue", queue(3)).highlighting("pod1"));
    assert_eq!(
        list.content.body,
        "- pod0 · 0 waiting\n- **▸** pod1 · 1 waiting\n- pod2 · 2 waiting"
    );
    assert_eq!(list.highlighted.as_deref(), Some("pod1"));
    assert_eq!(
        list.rows
            .iter()
            .flatten()
            .map(|row| (row.id.as_str(), row.reference.as_deref()))
            .collect::<Vec<_>>(),
        [
            ("pod0", Some("pod/0")),
            ("pod1", Some("pod/1")),
            ("pod2", Some("pod/2"))
        ]
    );
    // Every row is an item of its body, on its own line, as clients find what they point at.
    let items = crate::stage::checklist::items(&list.content.body);
    assert_eq!(
        items.iter().map(|item| item.line).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    // Its text cleaned onto one line and cut, its id where that leaves nothing; a reference refused or repeated left
    // out, the row kept.
    let cleaned = publishing(Panel::list(
        "q",
        "",
        vec![
            Row::new("a", "one\ntwo \u{202e}").with_reference("no spaces"),
            Row::new("b", "\u{200b} ").with_reference("ref/b"),
            Row::new("c", "x".repeat(200)).with_reference("ref/b"),
        ],
    ));
    assert_eq!(
        cleaned.content.body,
        format!("- one two\n- b\n- {}…", "x".repeat(80))
    );
    assert_eq!(
        cleaned
            .rows
            .iter()
            .flatten()
            .map(|row| row.reference.as_deref())
            .collect::<Vec<_>>(),
        [None, Some("ref/b"), None]
    );
    // Rows past the body's lines, or its characters, are left out whole, and a highlight of one left out highlights
    // nothing.
    let long = (0..30)
        .map(|index| Row::new(format!("r{index}"), "x".repeat(60)))
        .collect();
    let long = publishing(Panel::list("q", "", long).highlighting("r29"));
    assert_eq!(long.rows.as_ref().unwrap().len(), 19);
    assert_eq!(
        long.content.body.lines().count(),
        long.rows.as_ref().unwrap().len()
    );
    assert!(!long.content.body.contains('…'));
    assert!(long.content.body.chars().filter(|c| *c != '\n').count() <= blocks::BODY_CHARS);
    assert_eq!(long.highlighted, None);
    let short = (0..30)
        .map(|index| Row::new(format!("r{index}"), "x"))
        .collect();
    let short = publishing(Panel::list("q", "", short));
    assert_eq!(short.rows.as_ref().unwrap().len(), blocks::BODY_LINES);
    // An empty list is still a list, of no rows.
    let empty = publishing(Panel::list("q", "", Vec::new()));
    assert_eq!(
        (empty.content.body.as_str(), empty.rows),
        ("", Some(Vec::new()))
    );
    // A row that begins with a checkbox is never a checklist item: its brackets are parentheses.
    let boxed = publishing(Panel::list(
        "q",
        "",
        vec![
            Row::new("a", "[ ] open"),
            Row::new("b", "[x] done"),
            Row::new("c", "[X]"),
            Row::new("d", "[x]done"),
        ],
    ));
    assert_eq!(
        boxed.content.body,
        "- ( ) open\n- (x) done\n- (X)\n- [x]done"
    );
    assert!(
        crate::stage::checklist::items(&boxed.content.body)
            .iter()
            .all(|item| item.checked.is_none())
    );
}

#[test]
fn a_list_panel_is_refused_past_its_rows_ids_and_highlight() {
    let refused = |panel: Panel| Publishing::new(panel).unwrap_err();
    assert_eq!(
        refused(Panel::list("q", "", queue(31))),
        "A list panel has at most 30 rows."
    );
    assert_eq!(
        refused(Panel::list("q", "", vec![Row::new("a b", "")])),
        "A row's id is 1 to 64 ASCII letters, digits, -, _, . and /."
    );
    assert_eq!(
        refused(Panel::list(
            "q",
            "",
            vec![Row::new("a", "1"), Row::new("a", "2")]
        )),
        "Two rows of a list panel have the same id."
    );
    assert_eq!(
        refused(Panel::list("q", "", queue(2)).highlighting("pod9")),
        "A list panel highlights only a row it has."
    );
    assert_eq!(
        refused(Panel::text("q", "", "- a").highlighting("a")),
        "Only a list panel highlights a row."
    );
    assert!(Publishing::new(Panel::list("a b", "", queue(1))).is_err());
}

#[test]
fn a_list_panel_s_highlight_moves_in_place_and_its_rows_are_found() {
    let walls = yard();
    let mut stage = Stage::default();
    let list = |highlighted: &str| {
        publishing(
            Panel::list("queue", "Queue", queue(3))
                .highlighting(highlighted)
                .with_reference("panel/queue"),
        )
    };
    assert_eq!(stage.publish(&walls, list("pod0"), 0), Ok(Change::Added));
    let id = stage.view("s", &walls, 0).panels[0].id.clone();
    assert_eq!(
        stage.publish(&walls, list("pod0"), 1),
        Ok(Change::Unchanged)
    );
    assert_eq!(stage.publish(&walls, list("pod2"), 2), Ok(Change::Updated));
    let (rows, highlighted) = stage.rows("yard", &id).unwrap();
    assert_eq!((rows.len(), highlighted), (3, Some("pod2")));
    assert!(
        stage
            .view("s", &walls, 2)
            .panel(&id)
            .unwrap()
            .body
            .ends_with("- **▸** pod2 · 2 waiting")
    );
    // A row's reference changing alone updates it too.
    let mut changed = queue(3);
    changed[1] = Row::new("pod1", "pod1 · 1 waiting").with_reference("pod/one");
    let changed = Panel::list("queue", "Queue", changed)
        .highlighting("pod2")
        .with_reference("panel/queue");
    assert_eq!(
        stage.publish(&walls, publishing(changed), 3),
        Ok(Change::Updated)
    );
    // The panel's reference is found before a row's; a row's is described by the panel's title and its text.
    assert_eq!(
        stage.referenced("yard", "pod/one").unwrap(),
        "Queue: pod1 · 1 waiting"
    );
    assert!(
        stage
            .referenced("yard", "panel/queue")
            .unwrap()
            .starts_with("Queue: - pod0")
    );
    assert!(stage.referenced("yard", "pod/1").is_none());
    // A panel that is not a list, or not its environment's, has no rows.
    stage
        .publish(&walls, publishing(Panel::text("rota", "Rota", "- a")), 0)
        .unwrap();
    let rota = stage
        .view("s", &walls, 0)
        .panels
        .into_iter()
        .find(|panel| panel.title == "Rota")
        .unwrap()
        .id;
    assert!(stage.rows("yard", &rota).is_none());
    stage
        .publish(
            &walls,
            publishing(Panel::list("empty", "Empty", Vec::new())),
            0,
        )
        .unwrap();
    let empty = stage
        .view("s", &walls, 0)
        .panels
        .into_iter()
        .find(|panel| panel.title == "Empty")
        .unwrap()
        .id;
    assert_eq!(stage.rows("yard", &empty), Some((&[][..], None)));
    assert!(stage.rows("yard", "panel-0").is_none());
    assert!(stage.rows("elsewhere", &id).is_none());
}

#[test]
fn pointing_at_a_row_of_a_list_panel_describes_the_row() {
    let walls = yard();
    let mut stage = Stage::default();
    let mut rows = queue(2);
    rows.push(Row::new("pod2", "pod2 \"held\""));
    stage
        .publish(
            &walls,
            publishing(Panel::list("queue", "Queue", rows).highlighting("pod1")),
            0,
        )
        .unwrap();
    let id = stage.view("s", &walls, 0).panels[0].id.clone();
    let describe = |line: Option<u32>| stage.describe(&walls, &id, line, 0).unwrap();
    assert_eq!(
        describe(Some(1)),
        "panel \"Queue\" (on a wall, published just now; the yard environment's; row 2 of 3, highlighted, id \
         yard:pod/1: \"pod1 · 1 waiting\")"
    );
    assert!(describe(Some(0)).ends_with("; row 1 of 3, id yard:pod/0: \"pod0 · 0 waiting\")"));
    // A row without a reference has no id, and its text is cleaned as an item's is.
    assert!(describe(Some(2)).ends_with("; row 3 of 3: \"pod2 'held'\")"));
    // Past its rows, or nowhere on them, the panel alone.
    assert_eq!(
        describe(Some(3)),
        "panel \"Queue\" (on a wall, published just now; the yard environment's)"
    );
    assert_eq!(describe(None), describe(Some(3)));
}

#[test]
fn a_row_s_tone_and_value_reach_clients_beside_its_body_and_the_agent_in_words() {
    let walls = yard();
    let mut stage = Stage::default();
    let mut rows = queue(5);
    rows[0] = rows[0].clone().with_tone(Tone::Good);
    rows[1] = rows[1].clone().with_tone(Tone::Bad).with_value(0.8234);
    rows[2] = rows[2].clone().with_tone(Tone::Warn).with_value(f32::NAN);
    rows[3] = rows[3].clone().with_value(7.0);
    stage
        .publish(
            &walls,
            publishing(Panel::list("queue", "Queue", rows).highlighting("pod1")),
            0,
        )
        .unwrap();
    let panel = stage.view("s", &walls, 0).panels[0].clone();
    // The body is the list an older client drew, its tones and values nowhere in it.
    assert_eq!(
        panel.body,
        "- pod0 · 0 waiting\n- **▸** pod1 · 1 waiting\n- pod2 · 2 waiting\n- pod3 · 3 waiting\n- pod4 · 4 waiting"
    );
    // A client that draws rows is sent the highlight, each tone and each value, to the nearest thousandth, held within
    // 0 to 1, and none where it was not finite.
    assert_eq!(
        panel.rows,
        [
            tomori_protocol::ListRow::new("pod0", false).with_tone(Tone::Good),
            tomori_protocol::ListRow::new("pod1", false)
                .with_highlight(true)
                .with_tone(Tone::Bad)
                .with_value(0.823),
            tomori_protocol::ListRow::new("pod2", false).with_tone(Tone::Warn),
            tomori_protocol::ListRow::new("pod3", false).with_value(1.0),
            tomori_protocol::ListRow::new("pod4", false),
        ]
    );
    // Pointing at a row tells the agent its tone and value in words, and nothing of a neutral row's tone.
    let describe = |line: u32| stage.describe(&walls, &panel.id, Some(line), 0).unwrap();
    assert!(
        describe(1).ends_with(
            "; row 2 of 5, highlighted, tone bad, at 82% of its limit, id yard:pod/1: \"pod1 · 1 waiting\")"
        ),
        "{}",
        describe(1)
    );
    assert!(describe(0).contains("; row 1 of 5, tone good, id"));
    assert!(describe(2).contains("; row 3 of 5, tone warning, id"));
    assert!(describe(3).contains("; row 4 of 5, at 100% of its limit, id"));
    assert!(describe(4).contains("; row 5 of 5, id"));
    // Changing only a tone or a value updates the panel in place; the same again changes nothing.
    let mut rows = queue(5);
    rows[0] = rows[0].clone().with_tone(Tone::Bad);
    let changed = stage.publish(
        &walls,
        publishing(Panel::list("queue", "Queue", rows.clone()).highlighting("pod1")),
        0,
    );
    assert_eq!(changed, Ok(Change::Updated));
    let after = stage.view("s", &walls, 0).panels[0].clone();
    assert_eq!(
        (after.id.as_str(), after.rows[0].tone),
        (panel.id.as_str(), Tone::Bad)
    );
    let again = stage.publish(
        &walls,
        publishing(Panel::list("queue", "Queue", rows).highlighting("pod1")),
        0,
    );
    assert_eq!(again, Ok(Change::Unchanged));
}

#[test]
fn a_row_s_tone_is_told_in_a_word_and_its_value_as_a_percentage() {
    let row = |tone: Tone, value: Option<f32>| super::Row {
        id: "pod".into(),
        text: "pod".into(),
        reference: None,
        choosable: false,
        tone,
        value,
    };
    let words =
        [Tone::Neutral, Tone::Good, Tone::Warn, Tone::Bad].map(|tone| row(tone, None).tone_word());
    assert_eq!(words, [None, Some("good"), Some("warning"), Some("bad")]);
    assert_eq!(row(Tone::Neutral, Some(0.456)).percent(), Some(46));
    assert_eq!(row(Tone::Neutral, None).percent(), None);
    // Kept with the space, a row from before tones and values reads as neutral, without one.
    let older: super::Row = serde_json::from_str(r#"{"id": "pod", "text": "pod"}"#).unwrap();
    assert_eq!(older, row(Tone::Neutral, None));
}

/// The rows of a list as large as a list can be, of the longest ids, each in a tone with a value of three decimals and
/// one highlighted, as a client is sent them: within 4 KiB, so that 32 of them add 128 KiB at most to a projection.
#[test]
fn the_largest_list_s_rows_with_tones_and_values_are_within_their_bound() {
    let walls = yard();
    let mut stage = Stage::default();
    let rows = (0..tomori_environment::ROW_LIMIT)
        .map(|index| {
            Row::new(format!("{index:02}{}", "r".repeat(62)), "x")
                .choosable()
                .with_tone(Tone::Warn)
                .with_value(0.123)
        })
        .collect::<Vec<_>>();
    let highlighted = rows[7].id().to_owned();
    stage
        .publish(
            &walls,
            publishing(Panel::list("queue", "Queue", rows).highlighting(&highlighted)),
            0,
        )
        .unwrap();
    let panel = stage.view("s", &walls, 0).panels[0].clone();
    assert_eq!(panel.rows.len(), tomori_environment::ROW_LIMIT);
    let bytes = serde_json::to_vec(&panel.rows).unwrap().len();
    assert!(bytes <= 4096, "{bytes} bytes");
}

#[test]
fn pointing_at_a_row_by_its_id_describes_that_row_wherever_it_now_is() {
    let walls = yard();
    let mut stage = Stage::default();
    stage
        .publish(
            &walls,
            publishing(Panel::list("queue", "Queue", queue(3))),
            0,
        )
        .unwrap();
    let id = stage.view("s", &walls, 0).panels[0].id.clone();
    let describe = |line: Option<u32>, row: Option<&str>| {
        stage.describe_at(&walls, &id, line, row, 0).unwrap()
    };
    // Named by its id, the row is the one the participant saw, whatever is on that line now.
    assert!(
        describe(Some(0), Some("pod2"))
            .ends_with("; row 3 of 3, id yard:pod/2: \"pod2 · 2 waiting\")")
    );
    // Gone, it says so, rather than describe whatever row is on that line.
    assert!(describe(Some(0), Some("pod9")).ends_with("; a row since gone from the list)"));
    // Without one, as from a client of a server before ids, it goes by the line.
    assert!(describe(Some(1), None).contains("; row 2 of 3"));
}

#[test]
fn a_row_is_chosen_only_on_its_environment_s_list_while_it_is_there_and_choosable() {
    let walls = yard();
    let mut stage = Stage::default();
    let mut rows = queue(2);
    rows[1] = rows[1].clone().choosable();
    stage
        .publish(&walls, publishing(Panel::list("queue", "Queue", rows)), 0)
        .unwrap();
    stage
        .publish(&walls, publishing(Panel::text("rota", "Rota", "- a")), 0)
        .unwrap();
    let ids: Vec<String> = stage
        .view("s", &walls, 0)
        .panels
        .into_iter()
        .map(|panel| panel.id)
        .collect();
    // Its rows reach every client, with which may be chosen.
    let view = stage.view("s", &walls, 0);
    assert_eq!(
        view.panel(&ids[0]).unwrap().rows,
        [
            tomori_protocol::ListRow::new("pod0", false),
            tomori_protocol::ListRow::new("pod1", true)
        ]
    );
    assert!(view.panel(&ids[1]).unwrap().rows.is_empty());
    assert_eq!(stage.chosen(&walls, &ids[0], "pod1"), Ok(("queue", "pod1")));
    assert_eq!(
        stage.chosen(&walls, &ids[0], "pod0"),
        Err("That row cannot be chosen.")
    );
    assert_eq!(
        stage.chosen(&walls, &ids[0], "pod9"),
        Err("That row is no longer on the list.")
    );
    assert_eq!(
        stage.chosen(&walls, &ids[1], "a"),
        Err("That panel has no rows to choose.")
    );
    let elsewhere = Walls {
        environment: "elsewhere",
        ..yard()
    };
    assert_eq!(
        stage.chosen(&elsewhere, &ids[0], "pod1"),
        Err("That panel is not here.")
    );
    // Kept, but on a wall the environment no longer declares, it is not shown, and not chosen.
    let fewer = Walls {
        declared: &DECLARED[..0],
        ..yard()
    };
    assert!(stage.view("s", &fewer, 0).panel(&ids[0]).is_none());
    assert_eq!(
        stage.chosen(&fewer, &ids[0], "pod1"),
        Err("That panel is not here.")
    );
    // An agent's panel has none either.
    let note = crate::stage::Content::parse("title: Note\n---\n- a").unwrap();
    let board = Destination::Wall {
        wall: "environment-1".into(),
        slot: None,
    };
    let agent = stage.project_to(&walls, note, &board, 0);
    assert_eq!(
        stage.chosen(&walls, &agent, "a"),
        Err("That panel has no rows to choose.")
    );
    // Marking a row choosable is a change of the panel.
    let mut again = queue(2);
    again[0] = again[0].clone().choosable();
    again[1] = again[1].clone().choosable();
    assert_eq!(
        stage.publish(&walls, publishing(Panel::list("queue", "Queue", again)), 1),
        Ok(Change::Updated)
    );
}
