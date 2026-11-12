// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use tomori_environment::chart::{Colour, Series, SeriesStyle};
use tomori_environment::{ContextParameter, Panel, Row};
use tomori_protocol::Pose;

use super::*;
use crate::stage::published::{Change, Publishing};
use crate::stage::{Destination, Walls};

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

fn console() -> Walls<'static> {
    Walls {
        environment: "console",
        tomori: crate::stage::default_stage(),
        declared: &DECLARED,
        status: crate::stage::Status::Beneath,
    }
}

/// 12 November 2026, 02:20 to 02:50 UTC.
const FROM: i64 = 1_794_450_000_000;
const TO: i64 = 1_794_451_800_000;
const WINDOW: Setting = Setting::Window {
    from_ms: FROM,
    to_ms: TO,
};

fn latency(id: &str, time: TimeAxis, values: usize) -> Publishing {
    let chart = Chart::new(181)
        .with_time(time)
        .with_labels(vec![AxisLabel::new(0, "-30m"), AxisLabel::new(180, "now")]);
    let chart = if values == 0 {
        chart.with_status(Note::new("no history kept", Tone::Warn))
    } else {
        chart.with_series(Series::new(
            "p99",
            Colour::Amber,
            SeriesStyle::Line,
            vec![Some(41.0); values],
        ))
    };
    Publishing::new(Panel::chart(id, "Latency", chart).depends_on(["region"])).unwrap()
}

const LIVE: TimeAxis = TimeAxis::Live { step_ms: 10_000 };
const FIXED: TimeAxis = TimeAxis::Fixed {
    from_ms: FROM,
    to_ms: TO,
};

/// A console showing a live latency chart, declaring its region, and a copy of it held to the window: the stage, the
/// chart's identity, and the copy's identity and its environment's id for it.
fn copied() -> (Stage, String, (String, String)) {
    let walls = console();
    let mut stage = Stage::default();
    stage.declare_parameters(
        "console",
        crate::stage::context::declared(vec![ContextParameter::new(
            "region", "Region", "US-EAST1",
        )])
        .unwrap(),
    );
    stage
        .publish(&walls, latency("latency", LIVE, 181), 0)
        .unwrap();
    let source = stage.view("s", &walls, 0).panels[0].id.clone();
    let to = Destination::Free(Pose::at([0.0, 1.5, -1.5]));
    let copy = stage
        .copy_panel(&walls, &source, vec![("time".into(), WINDOW)], &to)
        .unwrap();
    (stage, source, copy)
}

fn shown(stage: &Stage, copy: &str) -> tomori_protocol::Panel {
    stage.view("s", &console(), 0).panel(copy).unwrap().clone()
}

#[test]
fn windows_and_their_labels_are_written_in_utc() {
    assert_eq!(described((FROM, TO)), "2026-11-12 02:20–02:50 UTC");
    assert_eq!(
        described((FROM - 3 * 3_600_000, FROM - 2 * 3_600_000 - 20 * 60_000)),
        "2026-11-11 23:20–2026-11-12 00:00 UTC"
    );
    let texts = |points: u32, window: (i64, i64)| -> Vec<(u32, String)> {
        labels(points, window)
            .into_iter()
            .map(|label| (label.at, label.text))
            .collect()
    };
    assert_eq!(
        texts(181, (FROM, TO)),
        [
            (0, "02:20".into()),
            (90, "02:35".into()),
            (180, "02:50 UTC".into())
        ]
    );
    // Across midnight, a label on another day says its date.
    assert_eq!(
        texts(3, (FROM - 3 * 3_600_000, FROM - 2 * 3_600_000)),
        [
            (0, "11-11 23:20".into()),
            (1, "23:50".into()),
            (2, "11-12 00:20 UTC".into())
        ]
    );
    assert_eq!(texts(1, (FROM, TO)), [(0, "02:20 UTC".into())]);
    assert_eq!(
        texts(2, (FROM, TO)),
        [(0, "02:20".into()), (1, "02:50 UTC".into())]
    );
    // An instant past what a timestamp holds is written as its milliseconds.
    assert_eq!(utc(i64::MAX, "%H:%M"), i64::MAX.to_string());
}

#[test]
fn a_copy_held_to_a_window_waits_for_it_and_takes_only_content_for_exactly_it() {
    let walls = console();
    let (mut stage, source, (copy, instance)) = copied();
    // Until its side answers, a chart over the window without values, saying it waits, labelled in UTC.
    let waiting = shown(&stage, &copy);
    let chart = waiting.chart.as_deref().unwrap();
    assert!(!chart.has_values());
    assert_eq!(chart.time, Some(FIXED));
    assert_eq!(chart.points, 181);
    assert!(
        chart.status.as_ref().unwrap().text.starts_with(WAITING),
        "{:?}",
        chart.status
    );
    assert!(
        waiting.body.contains("2026-11-12 02:20–02:50 UTC"),
        "{}",
        waiting.body
    );
    assert_eq!(chart.labels.last().unwrap().text, "02:50 UTC");
    // Live data, or another window's, is never taken for it.
    assert_eq!(
        stage.publish(&walls, latency(&instance, LIVE, 181), 1),
        Ok(Change::Unchanged)
    );
    let other = TimeAxis::Fixed {
        from_ms: FROM - 60_000,
        to_ms: TO,
    };
    assert_eq!(
        stage.publish(&walls, latency(&instance, other, 181), 1),
        Ok(Change::Unchanged)
    );
    assert!(!shown(&stage, &copy).chart.unwrap().has_values());
    // Its window's own content is shown, its labels the server's, in UTC, never the side's.
    assert_eq!(
        stage.publish(&walls, latency(&instance, FIXED, 181), 2),
        Ok(Change::Updated)
    );
    let held = shown(&stage, &copy).chart.unwrap();
    assert!(held.has_values());
    assert_eq!(held.labels, labels(181, (FROM, TO)));
    // Then nothing live replaces it: it keeps showing exactly that window.
    assert_eq!(
        stage.publish(&walls, latency(&instance, LIVE, 181), 3),
        Ok(Change::Unchanged)
    );
    assert_eq!(shown(&stage, &copy).chart.unwrap(), held);
    // The panel it is a copy of follows on, live, its labels its side's.
    assert_eq!(
        stage.publish(&walls, latency("latency", LIVE, 120), 4),
        Ok(Change::Updated)
    );
    let live = stage
        .view("s", &walls, 0)
        .panel(&source)
        .unwrap()
        .chart
        .clone()
        .unwrap();
    assert_eq!(live.time, Some(LIVE));
    assert_eq!(live.labels[1].text, "now");
}

#[test]
fn an_environment_without_history_says_honestly_it_has_no_data_for_the_window() {
    let walls = console();
    let (mut stage, _, (copy, instance)) = copied();
    stage
        .publish(&walls, latency(&instance, FIXED, 0), 1)
        .unwrap();
    let chart = shown(&stage, &copy).chart.unwrap();
    assert!(!chart.has_values());
    assert_eq!(chart.status.unwrap().text, "no history kept");
    assert_eq!(chart.time, Some(FIXED));
}

#[test]
fn a_list_on_a_wall_fixed_to_a_window_waits_without_rows_until_published_for_it() {
    let walls = console();
    let mut stage = Stage::default();
    let pods = |window: Option<(i64, i64)>| {
        let panel = Panel::list(
            "pods",
            "Pods",
            vec![Row::new("pod7", "pod7 · 41 waiting").choosable()],
        )
        .depends_on(["time"])
        .on_wall(1, None);
        let panel = match window {
            Some((from, to)) => panel.for_window(from, to),
            None => panel,
        };
        Publishing::new(panel).unwrap()
    };
    stage.publish(&walls, pods(None), 0).unwrap();
    let id = stage.view("s", &walls, 0).panels[0].id.clone();
    assert_eq!(stage.chosen(&walls, &id, "pod7"), Ok(("pods", "pod7")));
    stage
        .set_wall_context(&walls, "environment-1", "time", Some(WINDOW))
        .unwrap();
    let panel = stage.view("s", &walls, 0).panel(&id).unwrap().clone();
    assert!(panel.rows.is_empty());
    assert!(panel.body.starts_with(WAITING), "{}", panel.body);
    assert_eq!(stage.chosen(&walls, &id, "pod7"), Err(WAITING_ROWS));
    // Live rows are not taken; those published for its window are, and are chosen.
    assert_eq!(stage.publish(&walls, pods(None), 1), Ok(Change::Unchanged));
    assert_eq!(
        stage.publish(&walls, pods(Some((FROM, TO))), 2),
        Ok(Change::Updated)
    );
    let panel = stage.view("s", &walls, 0).panel(&id).unwrap().clone();
    assert_eq!(panel.rows.len(), 1);
    assert_eq!(stage.chosen(&walls, &id, "pod7"), Ok(("pods", "pod7")));
    // A live window holds nothing; following again, it takes what it is sent.
    stage
        .set_wall_context(
            &walls,
            "environment-1",
            "time",
            Some(Setting::Live { minutes: 30 }),
        )
        .unwrap();
    assert_eq!(stage.publish(&walls, pods(None), 3), Ok(Change::Updated));
    assert!(stage.held_window("console", "pods").is_none());
    assert!(stage.held_window("nowhere", "pods").is_none());
}

#[test]
fn a_window_is_refused_where_it_does_not_end_after_it_begins_or_reaches_past_9999() {
    let refused = |from: i64, to: i64| {
        Publishing::new(Panel::text("pods", "Pods", "").for_window(from, to)).unwrap_err()
    };
    for (from, to) in [
        (FROM, FROM),
        (TO, FROM),
        (FROM, tomori_protocol::chart::CHART_INSTANT_MS + 1),
        (tomori_protocol::chart::CHART_EARLIEST_MS - 1, FROM),
    ] {
        assert!(refused(from, to).contains("window ends after it begins"));
    }
}
