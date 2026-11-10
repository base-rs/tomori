// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use tomori_environment::Panel;
use tomori_environment::chart::{
    AxisLabel, Chart, Colour, Edge, Reference, Series, SeriesStyle, Shade,
};
use tomori_protocol::{Place, TOMORI_WALL};

use super::*;
use crate::stage::published::{RESTING, RESTING_RUNS};

/// The most bytes of JSON a published panel's content and chart can take, everything in it bounded as it is: a chart of
/// every series at every point, its labels, summary and title, each at its longest, with room to spare. These tests
/// check that the largest there can be is within it.
const PUBLISHED_BYTES: usize = 24 * 1024;
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
    }
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
    // The largest a chart can be, with the longest title, is within the bound.
    let bytes = Publishing::new(Panel::chart("w", "\u{1F600}".repeat(80), wide))
        .unwrap()
        .bytes();
    assert!(bytes <= PUBLISHED_BYTES, "{bytes}");
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
    // Each change of what it shows is a revision of it, as an agent's update is; the same again is none.
    assert_eq!(view.panels[0].revision, 2);
    assert_eq!(stage.revision(&walls, &id), Some(2));
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
    // At most so many in a space.
    for index in 2..PUBLISHED {
        let panel = Panel::text(format!("n{index}"), "", "b").on_wall(1, None);
        assert_eq!(
            stage.publish(&walls, publishing(panel), 0),
            Ok(Change::Added)
        );
    }
    assert!(
        stage
            .publish(&walls, publishing(Panel::text("more", "", "b")), 0)
            .is_err()
    );
    // Retracted, it goes; again, nothing.
    assert!(stage.retract("yard", "rota"));
    assert!(!stage.retract("yard", "rota"));
    assert!(!stage.retract("elsewhere", "rota"));
    assert_eq!(shown(&stage, &walls).len(), PUBLISHED - 1);
    // Nothing of the environment's is told the agent as its own to update.
    assert!(stage.content(&walls, &id).is_none());
    assert_eq!(stage.published("yard", &id), Some(("pressure", None)));
    assert!(stage.published("yard", "panel-999").is_none());
}

#[test]
fn a_full_wall_gives_up_only_the_agent_s_panels_and_an_environment_s_given_up_stays_away() {
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
    assert_eq!(stage.publish(&walls, rota("a"), 0), Ok(Change::Added));
    // The agent's panels fill the wall, giving up only their own once it is full.
    for index in 0..7 {
        stage.project_to(&walls, note(index), &board, 0);
    }
    assert!(shown(&stage, &walls).iter().any(|(title, _)| title == "a"));
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
    // The agent's newcomer on a wall where every panel is arranged gives up the one there longest, which, its
    // environment's, stays away rather than coming back as it is next updated.
    stage.project_to(&walls, note(10), &board, 0);
    assert!(!shown(&stage, &walls).iter().any(|(title, _)| title == "a"));
    assert_eq!(stage.publish(&walls, rota("a"), 0), Ok(Change::Unchanged));
}

#[test]
fn a_dismissed_panel_stays_away_until_retracted_or_the_space_comes_back() {
    let walls = yard();
    let mut stage = Stage::default();
    let rota = |body: &str| publishing(Panel::text("rota", "Rota", body));
    stage.publish(&walls, rota("one"), 0).unwrap();
    let id = stage.view("s", &walls, 0).panels[0].id.clone();
    stage.dismiss("yard", &id).unwrap();
    assert_eq!(stage.publish(&walls, rota("two"), 0), Ok(Change::Unchanged));
    assert!(shown(&stage, &walls).is_empty());
    // Retracted and published again, it is back.
    assert!(!stage.retract("yard", "rota"));
    assert_eq!(stage.publish(&walls, rota("three"), 0), Ok(Change::Added));
    let id = stage.view("s", &walls, 0).panels[0].id.clone();
    stage.dismiss("yard", &id).unwrap();
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
    stage.dismiss("yard", &notes).unwrap();
    assert!(stage.dismiss("yard", &notes).is_err());
}

#[test]
fn what_a_run_that_ended_published_rests_where_it_was_holding_no_slot_until_published_again() {
    let walls = yard();
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
    stage.dismiss("yard", &agent).unwrap();
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
    let walls = yard();
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
    stage.dismiss("yard", &id).unwrap();
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
    assert!(stage.tick_item(&walls, &id, 0, true, None).is_err());
    assert!(stage.tick_item(&walls, &id, 0, true, Some(1)).is_err());
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
    let largest = crate::stage::Content::parse(&source).unwrap();
    let declared: Vec<crate::EnvironmentWall> = (0..DECLARED_WALLS)
        .map(|index| crate::EnvironmentWall::new(pose(index as f32 - 3.0, -5.0)))
        .collect();
    let walls = Walls {
        declared: &declared,
        ..yard()
    };
    let mut stage = Stage::default();
    let mut chart = Chart::new(256);
    for _ in 0..4 {
        chart = chart.with_series(Series::new(
            wide(32),
            Colour::Teal,
            SeriesStyle::Line,
            (0..256)
                .map(|point| Some(-1.234_5e-12 * point as f32 - 1.0e-20))
                .collect(),
        ));
    }
    chart = chart.with_labels((0..8).map(|at| AxisLabel::new(at, wide(32))).collect());
    for index in 0..PUBLISHED {
        let panel = Panel::chart(format!("chart-{index}"), wide(80), chart.clone())
            .on_wall(index as u32 % 2, None);
        stage.publish(&walls, publishing(panel), 0).unwrap();
    }
    for index in 0..crate::stage::FLOATING {
        let id = stage.project(&walls, largest.clone(), None, 0);
        stage
            .place(&walls, &id, Target::Free(spread(index, 0.0, 0.4)))
            .unwrap();
    }
    for _ in 0..crate::stage::ADDED_WALLS {
        stage.add_wall(&walls, &pose(0.0, 5.0), None).unwrap();
    }
    let near =
        std::iter::once(None).chain(walls.declared.iter().map(|wall| Some(wall.pose.position)));
    for origin in near {
        for _ in 0..tomori_protocol::WALL_SLOTS {
            stage.project(&walls, largest.clone(), origin, 0);
        }
    }
    let added: Vec<String> = stage
        .view("s", &walls, 0)
        .walls
        .iter()
        .filter(|wall| wall.kind == tomori_protocol::WallKind::Added)
        .map(|wall| wall.id.clone())
        .collect();
    for wall in &added {
        for _ in 0..tomori_protocol::WALL_SLOTS {
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
    let view = stage.view(&"s".repeat(64), &walls, u64::MAX);
    assert_eq!(
        view.panels
            .iter()
            .filter(|panel| panel.chart.is_some())
            .count(),
        PUBLISHED
    );
    let message = tomori_protocol::ServerMessage::Projection {
        projection: Box::new(view),
    };
    let bytes = serde_json::to_vec(&message).unwrap().len();
    // Within the longest frame, for a client that takes the most; and within today's for an older one, which is sent
    // as much as today's limits hold.
    assert!(bytes <= tomori_protocol::MOST_FRAME_BYTES, "{bytes} bytes");
    let today = stage.view_within("s", &walls, u64::MAX, &tomori_protocol::Limits::TODAY);
    let message = tomori_protocol::ServerMessage::Projection {
        projection: Box::new(today),
    };
    let bytes = serde_json::to_vec(&message).unwrap().len();
    assert!(bytes <= tomori_protocol::MAX_FRAME_BYTES, "{bytes} bytes");
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
    assert_eq!(
        (&view.panels[0].place, view.panels[0].revision),
        (&on("environment-0", 1), 2)
    );
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
    stage.dismiss("yard", &id).unwrap();
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
    stage.dismiss("yard", &first).unwrap();
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
    for index in 0..4 {
        stage
            .publish(&walls, text(&format!("s{index}")), 0)
            .unwrap();
    }
    let waiting = ["w0", "w1", "w2", "s0"];
    assert_eq!(stage.admits(&walls, &text("w3"), &waiting), Ok(()));
    let waiting = ["w0", "w1", "w2", "w3", "s0"];
    assert_eq!(
        stage.admits(&walls, &text("new"), &waiting),
        Err("An environment publishes at most 8 panels in a space.")
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
