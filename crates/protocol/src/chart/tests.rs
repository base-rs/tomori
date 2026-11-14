// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::{Panel, Place};

/// A scaling-pressure chart of 181 points, its greatest line with a gap, its mean entirely missing at the start.
fn pressure() -> Chart {
    let greatest: Vec<Option<f32>> = (0..181)
        .map(|index| (!(60..70).contains(&index)).then_some(0.8 + index as f32 / 200.0))
        .collect();
    Chart::new(181)
        .with_labels(vec![
            AxisLabel::new(0, "-15 min"),
            AxisLabel::new(180, "now"),
        ])
        .with_axis(ValueAxis::new(Some(0.0), None, ""))
        .with_series(Series::new(
            "mean",
            Colour::Teal,
            SeriesStyle::Line,
            vec![Some(0.73); 181],
        ))
        .with_series(
            Series::new("greatest", Colour::Amber, SeriesStyle::Line, greatest)
                .with_width(1.0)
                .with_dashed(true)
                .with_breaks(vec![100]),
        )
        .with_reference(Reference::new(1.0, "target", Colour::White))
        .with_shade(Shade::new(
            Edge::Reference { index: 0 },
            Edge::Series { index: 1 },
            true,
            Colour::Red,
        ))
        .with_badge(Note::new("SHORT 3", Tone::Warn))
        .with_status(Note::new("7 of 8 reporting", Tone::Neutral))
}

#[test]
fn a_chart_panel_has_a_stable_wire_format_and_an_older_reader_reads_its_summary() {
    let chart = Chart::new(3)
        .with_labels(vec![AxisLabel::new(0, "-10 s"), AxisLabel::new(2, "now")])
        .with_series(Series::new(
            "mean",
            Colour::Teal,
            SeriesStyle::Step,
            vec![Some(0.5), None, Some(1.25)],
        ))
        .with_reference(Reference::new(1.0, "target", Colour::Rgb([255, 128, 0])).with_dashed(true))
        .with_shade(Shade::new(
            Edge::Reference { index: 0 },
            Edge::Series { index: 0 },
            true,
            Colour::Red,
        ));
    let json = serde_json::json!({
        "points": 3,
        "labels": [{"at": 0, "text": "-10 s"}, {"at": 2, "text": "now"}],
        "axis": {},
        "series": [{"name": "mean", "colour": "teal", "style": "step", "values": [0.5, null, 1.25]}],
        "references": [{"value": 1.0, "label": "target", "colour": "#ff8000", "dashed": true}],
        "shades": [{"from": {"kind": "reference", "index": 0}, "to": {"kind": "series", "index": 0},
            "above": true, "colour": "red"}]
    });
    assert_eq!(serde_json::to_value(&chart).unwrap(), json);
    assert_eq!(serde_json::from_value::<Chart>(json).unwrap(), chart);
    let panel =
        Panel::new("panel-1", "Pressure", chart.summary(), Place::Unknown).with_chart(chart);
    let json = serde_json::to_value(&panel).unwrap();
    assert_eq!(json["chart"]["points"], 3);
    // An older reader, which knows no charts, reads its title and its summary.
    #[derive(Deserialize)]
    struct Older {
        title: String,
        body: String,
    }
    let older: Older = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(older.title, "Pressure");
    assert_eq!(older.body, "mean: 1.25 now\ntarget: 1");
    assert_eq!(serde_json::from_value::<Panel>(json).unwrap(), panel);
    // A text panel says nothing of charts.
    let text = Panel::new("panel-2", "t", "b", Place::Unknown);
    assert!(serde_json::to_value(&text).unwrap().get("chart").is_none());
}

#[test]
fn newer_styles_tones_edges_and_colours_are_read_as_what_a_client_can_draw() {
    let chart: Chart = serde_json::from_value(serde_json::json!({
        "points": 2,
        "series": [{"name": "a", "colour": "chartreuse", "style": "candles", "values": [1, 2],
            "smoothing": 3}],
        "shades": [{"from": {"kind": "band", "low": 1}, "to": {"kind": "series", "index": 0}}],
        "badge": {"text": "hot", "tone": "urgent"},
        "legend": false
    }))
    .unwrap();
    assert_eq!(chart.series[0].colour, Colour::Grey);
    assert_eq!(chart.series[0].style, SeriesStyle::Unknown);
    assert_eq!(chart.shades[0].from, Edge::Unknown);
    assert!(!chart.names(Edge::Unknown));
    assert_eq!(chart.badge.as_ref().unwrap().tone, Tone::Neutral);
    assert!(!chart.legend);
    assert_eq!(chart.clone().with_legend(true).legend, Chart::new(2).legend);
    assert!(!Chart::new(2).with_legend(false).legend);
    assert_eq!(serde_json::to_value(&chart).unwrap()["legend"], false);
    // Every named colour reads back as itself, RGB in any case, and a malformed one as grey.
    for (name, colour) in Colour::NAMED {
        assert_eq!(String::from(colour), name);
        assert_eq!(Colour::from(name.to_uppercase()), colour);
        assert!(colour.rgb().iter().all(|value| (0.0..=1.0).contains(value)));
    }
    assert_eq!(
        Colour::from("#A0b1C2".to_owned()),
        Colour::Rgb([0xa0, 0xb1, 0xc2])
    );
    for malformed in ["#12345", "#12345g", "#1234567", ""] {
        assert_eq!(
            Colour::from(malformed.to_owned()),
            Colour::Grey,
            "{malformed}"
        );
    }
    assert_eq!(Colour::default(), Colour::Grey);
    assert_eq!(SeriesStyle::default(), SeriesStyle::Line);
    assert_eq!(Note::default().tone, Tone::Neutral);
}

#[test]
fn cleaning_bounds_a_chart_keeping_gaps_as_gaps_and_the_newest_last() {
    let chart = pressure().cleaned().unwrap();
    assert_eq!(chart.series[1].values[65], None);
    assert_eq!(chart.series[1].values[0], Some(0.8));
    assert_eq!(chart.series[1].breaks, [100]);
    // Values are kept to four significant figures; a value that is not finite is a gap.
    assert_eq!(rounded(0.123_456), Some(0.1235));
    assert_eq!(rounded(-98_765.4), Some(-98_770.0));
    assert_eq!(rounded(0.0), Some(0.0));
    assert_eq!(rounded(f32::NAN), None);
    // One beyond what a chart holds is held to it, so that drawing it, and the distances between values, stay finite.
    assert_eq!(rounded(f32::MAX), Some(CHART_MAGNITUDE));
    assert_eq!(rounded(-3.0e38), Some(-CHART_MAGNITUDE));
    let mut wild = Chart::new(4).with_series(
        Series::new(
            "w".repeat(100),
            Colour::Blue,
            SeriesStyle::Bars,
            vec![Some(f32::INFINITY), Some(1.0)],
        )
        .with_width(f32::NAN)
        .with_breaks(vec![0, 0, 2, 3, u32::MAX]),
    );
    wild.series[0].width = Some(9.0);
    let cleaned = wild.clone().cleaned().unwrap();
    // Too few values are gaps before the first, and its breaks move with them, those past its last point dropped.
    assert_eq!(cleaned.series[0].values, [None, None, None, Some(1.0)]);
    assert_eq!(cleaned.series[0].breaks, [2]);
    assert_eq!(cleaned.series[0].width, Some(LINE_WIDTHS.1));
    assert_eq!(cleaned.series[0].name.chars().count(), CHART_NAME_CHARS + 1);
    wild.series[0].width = Some(f32::NAN);
    assert_eq!(wild.cleaned().unwrap().series[0].width, None);
    // Too many values keep the newest, and breaks among the oldest go with them.
    let long = Chart::new(2)
        .with_series(
            Series::new(
                "l",
                Colour::Red,
                SeriesStyle::Area,
                vec![Some(1.0), Some(2.0), Some(3.0)],
            )
            .with_breaks(vec![0, 1]),
        )
        .cleaned()
        .unwrap();
    assert_eq!(long.series[0].values, [Some(2.0), Some(3.0)]);
    assert_eq!(long.series[0].breaks, [0]);
    // Too many of everything is cut; shades that name nothing, or one edge twice, are left out.
    let mut crowded = Chart::new(256);
    for index in 0..10 {
        crowded = crowded
            .with_series(Series::new(
                "s",
                Colour::Green,
                SeriesStyle::Line,
                Vec::new(),
            ))
            .with_reference(Reference::new(index as f32, "\u{202e}r\nx", Colour::Violet))
            .with_shade(Shade::new(
                Edge::Series { index },
                Edge::Reference { index: 0 },
                false,
                Colour::Grey,
            ));
    }
    crowded.labels = (0..20).map(|at| AxisLabel::new(at * 20, "t")).collect();
    crowded.shades.insert(
        0,
        Shade::new(
            Edge::Series { index: 0 },
            Edge::Series { index: 0 },
            true,
            Colour::Grey,
        ),
    );
    crowded.axis.unit = "per\nsecond".into();
    crowded.badge = Some(Note::new("b\u{7}", Tone::Bad));
    crowded.status = Some(Note::new("s".repeat(200), Tone::Good));
    let crowded = crowded.cleaned().unwrap();
    assert_eq!(crowded.series.len(), CHART_SERIES);
    assert!(
        crowded
            .series
            .iter()
            .all(|series| series.values.len() == 256)
    );
    assert_eq!(crowded.references.len(), CHART_REFERENCES);
    assert_eq!(crowded.references[1].label, "r x");
    assert_eq!(crowded.shades.len(), CHART_SHADES);
    assert!(crowded.shades.iter().all(|shade| shade.from != shade.to));
    assert_eq!(crowded.labels.len(), CHART_LABELS);
    assert_eq!(crowded.axis.unit, "per second");
    assert_eq!(crowded.badge.unwrap().text, "b ");
    assert_eq!(
        crowded.status.unwrap().text.chars().count(),
        CHART_STATUS_CHARS + 1
    );
    let unlabelled = Chart::new(3)
        .with_labels(vec![AxisLabel::new(3, "past the end")])
        .cleaned()
        .unwrap();
    assert!(unlabelled.labels.is_empty());
    // Refused: too few or too many points, an axis upside down, a reference that is not finite.
    assert!(Chart::new(1).cleaned().is_err());
    assert!(Chart::new(CHART_POINTS + 1).cleaned().is_err());
    assert!(
        Chart::new(2)
            .with_axis(ValueAxis::new(Some(2.0), Some(1.0), ""))
            .cleaned()
            .is_err()
    );
    assert!(
        Chart::new(2)
            .with_reference(Reference::new(f32::NAN, "", Colour::Red))
            .cleaned()
            .is_err()
    );
    // An axis value that is not finite is found instead.
    let found = Chart::new(2)
        .with_axis(ValueAxis::new(Some(f32::NAN), Some(5.0), ""))
        .cleaned()
        .unwrap();
    assert_eq!((found.axis.min, found.axis.max), (None, Some(5.0)));
}

#[test]
fn a_chart_of_every_series_at_every_point_is_a_few_kilobytes() {
    let mut chart = Chart::new(CHART_POINTS);
    for index in 0..CHART_SERIES {
        let values = (0..CHART_POINTS)
            .map(|point| Some(-1_234.567_8 - (point as f32 * 7.0 + index as f32)))
            .collect();
        chart = chart.with_series(Series::new(
            "x".repeat(64),
            Colour::Teal,
            SeriesStyle::Line,
            values,
        ));
    }
    let bytes = serde_json::to_vec(&chart.cleaned().unwrap()).unwrap().len();
    assert!(bytes < 20 * 1024, "{bytes} bytes");
}

#[test]
fn a_summary_says_each_latest_value_and_never_that_all_is_well_without_data() {
    let chart = pressure().with_axis(ValueAxis::new(None, None, "x"));
    assert_eq!(
        chart.summary(),
        "7 of 8 reporting\nmean: 0.73 x now\ngreatest: 1.7 x now\ntarget: 1 x\nSHORT 3"
    );
    let mut gone = pressure();
    gone.series[0].values = vec![None; 181];
    *gone.series[1].values.last_mut().unwrap() = None;
    gone.references[0].label.clear();
    gone.status = None;
    gone.badge = None;
    assert_eq!(
        gone.summary(),
        "mean: no data\ngreatest: no value now\nreference: 1"
    );
    assert_eq!(
        Chart::new(2)
            .with_series(Series::new(
                "a",
                Colour::Red,
                SeriesStyle::Line,
                vec![None, None]
            ))
            .with_status(Note::new("not measured", Tone::Warn))
            .summary(),
        "not measured\nNo data.\na: no data"
    );
    assert!(!Chart::new(2).has_values());
    assert_eq!(
        Series::new("a", Colour::Red, SeriesStyle::Line, Vec::new()).latest(),
        None
    );
}

#[test]
fn a_fixed_window_s_summary_says_each_series_newest_value_and_when() {
    // 02:00 to 10:00 UTC on 14 November 2026, a point each 6 minutes, 81 points.
    let from_ms = 1_794_621_600_000;
    let window = |values: Vec<Option<f32>>| {
        Chart::new(81)
            .with_series(Series::new(
                "dw2 trial",
                Colour::Teal,
                SeriesStyle::Line,
                values,
            ))
            .with_time(TimeAxis::Fixed {
                from_ms,
                to_ms: from_ms + 8 * 3_600_000,
            })
    };
    // Filling until its end: read to 05:54, the rest a gap to come, never a value gone.
    let filling = window((0..81).map(|at| (at <= 39).then_some(0.194)).collect())
        .with_status(Note::new("filling until 10:00 UTC", Tone::Neutral));
    assert_eq!(
        filling.summary(),
        "filling until 10:00 UTC\ndw2 trial: 0.194 at 05:54"
    );
    assert_eq!(filling.current(&filling.series[0]), Some((39, 0.194)));
    // Ended and read whole, at its end; a series that stopped within it, when it stopped.
    let ended = window(vec![Some(0.2); 81]);
    assert_eq!(ended.summary(), "dw2 trial: 0.2 at its end, 10:00");
    let stopped = window((0..81).map(|at| (at < 40).then_some(0.5)).collect());
    assert_eq!(stopped.summary(), "dw2 trial: 0.5 at 05:54");
    // Off the minute, to the second; not begun, no data.
    let mut fine = window(vec![None; 81]);
    fine.time = Some(TimeAxis::Fixed {
        from_ms: from_ms + 15_000,
        to_ms: from_ms + 15_000 + 8 * 3_600_000,
    });
    fine.series[0].values[1] = Some(1.0);
    assert_eq!(fine.summary(), "dw2 trial: 1 at 02:06:15");
    assert_eq!(
        window(vec![None; 81]).summary(),
        "No data.\ndw2 trial: no data"
    );
    // A live chart whose last point has no value still says it has none now.
    let mut live = window((0..81).map(|at| (at <= 39).then_some(0.194)).collect());
    live.time = Some(TimeAxis::Live { step_ms: 360_000 });
    assert_eq!(live.summary(), "dw2 trial: no value now");
    assert_eq!(live.current(&live.series[0]), None);
    live.series[0].values[80] = Some(0.3);
    assert_eq!(live.summary(), "dw2 trial: 0.3 now");
}

/// Root admissions a minute, as a line, at 5 seconds over 15 minutes, with a gap and a break; and rejections as bars.
fn admissions() -> Chart {
    let admitted = (0..181)
        .map(|index| (!(40..44).contains(&index)).then_some(200_000.0 + 100.0 * index as f32))
        .collect();
    let rejected = (0..181)
        .map(|index| (index % 2 == 0).then_some(index as f32))
        .collect();
    Chart::new(181)
        .with_labels(vec![
            AxisLabel::new(0, "-15 min"),
            AxisLabel::new(60, "-10 min"),
            AxisLabel::new(120, "-5 min"),
            AxisLabel::new(180, "now"),
        ])
        .with_axis(ValueAxis::new(None, None, "a minute"))
        .with_series(
            Series::new("admitted", Colour::Blue, SeriesStyle::Line, admitted)
                .with_breaks(vec![100]),
        )
        .with_series(Series::new(
            "rejected",
            Colour::Red,
            SeriesStyle::Bars,
            rejected,
        ))
}

#[test]
fn a_place_across_the_time_axis_is_a_point_or_between_two() {
    let chart = admissions();
    assert_eq!(chart.at_across(0), 0.0);
    assert_eq!(chart.at_across(ACROSS), 180.0);
    assert_eq!(chart.at_across(ACROSS / 2), 90.0);
    // Beyond its end, it is its newest point; a chart of fewer than two points reads as two.
    assert_eq!(chart.at_across(u16::MAX), 180.0);
    assert_eq!(Chart::new(0).at_across(ACROSS), 1.0);
}

#[test]
fn a_line_is_read_between_its_points_and_steps_and_bars_at_theirs_and_a_gap_never_as_zero() {
    let chart = admissions();
    // Between two points of a line, the straight line between them, kept to four figures.
    assert_eq!(chart.value_at(0, 10.0), Some(201_000.0));
    assert_eq!(chart.value_at(0, 10.5), Some(201_100.0));
    // Within a gap, and past the last point drawn before one, none: never zero, and never a value the line does not draw.
    assert_eq!(chart.value_at(0, 41.0), None);
    assert_eq!(chart.value_at(0, 39.0), Some(203_900.0));
    assert_eq!(chart.value_at(0, 39.4), None);
    assert_eq!(chart.value_at(0, 39.6), None);
    // Across a break, nothing either, as no line is drawn there.
    assert_eq!(chart.value_at(0, 100.2), None);
    assert_eq!(chart.value_at(0, 100.8), None);
    // A point alone, drawn as a dot, is read within half a point of it.
    let lone = Chart::new(5).with_series(Series::new(
        "x",
        Colour::Teal,
        SeriesStyle::Line,
        vec![None, Some(5.0), None, Some(1.0), Some(2.0)],
    ));
    assert_eq!(lone.value_at(0, 0.6), Some(5.0));
    assert_eq!(lone.value_at(0, 1.4), Some(5.0));
    assert_eq!(lone.value_at(0, 2.6), None);
    assert_eq!(lone.value_at(0, 3.5), Some(1.5));
    // Bars of a chart of more than 128 points are drawn one to a stretch, as tall as its farthest from the base, and
    // read so; of fewer, at the nearest point, a missing one as none.
    assert_eq!(chart.value_at(1, 9.4), Some(10.0));
    assert_eq!(chart.value_at(1, 10.0), Some(10.0));
    assert_eq!(chart.value_at(1, 103.0), None);
    assert_eq!(chart.value_at(1, 180.0), Some(180.0));
    let few = Chart::new(4).with_series(Series::new(
        "bars",
        Colour::Red,
        SeriesStyle::Bars,
        vec![Some(-3.0), None, Some(2.0), Some(1.0)],
    ));
    assert_eq!(few.value_at(0, 0.4), Some(-3.0));
    assert_eq!(few.value_at(0, 1.2), None);
    assert_eq!(few.value_at(0, 1.6), Some(2.0));
    // The farthest from the base, below it too.
    let mut many = Chart::new(256);
    many.series.push(Series::new(
        "bars",
        Colour::Red,
        SeriesStyle::Bars,
        (0..256)
            .map(|index| Some(if index % 2 == 0 { 1.0 } else { -4.0 }))
            .collect(),
    ));
    assert_eq!(many.value_at(0, 0.0), Some(-4.0));
    // A step holds its value from its point to the next.
    let steps = Chart::new(3).with_series(Series::new(
        "ready",
        Colour::Green,
        SeriesStyle::Step,
        vec![Some(6.0), Some(8.0), None],
    ));
    assert_eq!(steps.value_at(0, 0.9), Some(6.0));
    assert_eq!(steps.value_at(0, 1.5), Some(8.0));
    assert_eq!(steps.value_at(0, 2.0), None);
    // Beyond either end, its end; nonsense, the oldest; a series it lacks, none.
    assert_eq!(chart.value_at(0, -5.0), Some(200_000.0));
    assert_eq!(chart.value_at(0, 900.0), Some(218_000.0));
    assert_eq!(chart.value_at(0, f32::NAN), Some(200_000.0));
    assert_eq!(chart.value_at(4, 1.0), None);
    // Fewer values than points, as from a server not to be trusted, reads none past them.
    let short = Chart::new(4).with_series(Series::new(
        "x",
        Colour::Teal,
        SeriesStyle::Area,
        vec![Some(1.0)],
    ));
    assert_eq!(short.value_at(0, 0.0), Some(1.0));
    assert_eq!(short.value_at(0, 2.5), None);
}

#[test]
fn the_time_pointed_at_is_read_from_the_axis_labels_about_it_and_written_as_they_are() {
    let chart = admissions();
    // Six minutes and 25 seconds ago, between -10 min and -5 min, to the second, as its points are 5 s apart.
    assert_eq!(chart.time_at(103.0), "-6 min 25 s");
    assert_eq!(chart.time_at(108.0), "-6 min");
    assert_eq!(chart.time_at(0.0), "-15 min");
    assert_eq!(chart.time_at(179.0), "-5 s");
    assert_eq!(chart.time_at(180.0), "now");
    assert_eq!(chart.time_at(f32::NAN), "-15 min");
    // Hours, to the minute where its points are minutes apart, and a minus sign kept as written.
    let mut hours = admissions();
    hours.labels = vec![AxisLabel::new(0, "−3 h"), AxisLabel::new(180, "now")];
    assert_eq!(hours.time_at(30.0), "−2 h 30 min");
    // Whole units where points are a unit or more apart, and `ago`, `+` and other words for units.
    let mut days = admissions();
    days.labels = vec![AxisLabel::new(0, "4 days ago"), AxisLabel::new(4, "+0 d")];
    assert_eq!(days.time_at(1.0), "-3 d");
    assert_eq!(days.time_at(4.2), "now");
    let mut ahead = admissions();
    ahead.labels = vec![AxisLabel::new(0, "now"), AxisLabel::new(180, "+15 minutes")];
    assert_eq!(ahead.time_at(12.0), "+1 min");
    // Seconds alone.
    let mut seconds = admissions();
    seconds.labels = vec![AxisLabel::new(0, "-180 sec"), AxisLabel::new(180, "now")];
    assert_eq!(seconds.time_at(170.0), "-10 s");
    // Clock times, past midnight too, to the second where points are less than a minute apart or a label says one.
    let mut clock = admissions();
    clock.labels = vec![AxisLabel::new(0, "23:55"), AxisLabel::new(60, "00:00")];
    assert_eq!(clock.time_at(65.0), "00:00:25");
    clock.labels = vec![AxisLabel::new(0, "9:00"), AxisLabel::new(3, "12:00")];
    assert_eq!(clock.time_at(2.0), "11:00");
    clock.labels = vec![AxisLabel::new(0, "9:00:00"), AxisLabel::new(3, "12:00")];
    assert_eq!(clock.time_at(2.0), "11:00:00");
    // Labels of two kinds read only from those of one; labels of no time, or one, say a label there or the point.
    let mut mixed = admissions();
    mixed.labels = vec![
        AxisLabel::new(0, "09:00"),
        AxisLabel::new(60, "-10 min"),
        AxisLabel::new(180, "now"),
    ];
    assert_eq!(mixed.time_at(30.0), "-12 min 30 s");
    for nonsense in [
        "9:0",
        "25:00",
        "9:60",
        "1:2:3:4",
        "soon",
        "-5 parsecs",
        "",
        "-x min",
    ] {
        let mut chart = admissions();
        chart.labels = vec![AxisLabel::new(0, nonsense), AxisLabel::new(180, "now")];
        assert_eq!(chart.time_at(1.0), "point 2 of 181", "{nonsense:?}");
        assert_eq!(
            chart.time_at(0.0),
            if nonsense.is_empty() { "" } else { nonsense }
        );
    }
    // Two labels saying now, at different points, read as now between them.
    let mut nows = admissions();
    nows.labels = vec![AxisLabel::new(0, "now"), AxisLabel::new(180, "NOW")];
    assert_eq!(nows.time_at(90.0), "now");
}

#[test]
fn a_reading_says_the_time_and_each_series_value_in_its_unit_or_that_it_has_none() {
    let chart = admissions();
    assert_eq!(
        chart.read_at(103.0),
        "-6 min 25 s: admitted 210,300 a minute, rejected no data"
    );
    assert_eq!(
        chart.read_at(chart.at_across(ACROSS)),
        "now: admitted 218,000 a minute, rejected 180 a minute"
    );
    let mut bare = admissions();
    bare.axis.unit.clear();
    assert_eq!(bare.amount(Some(0.25)), "0.25");
    assert_eq!(bare.amount(None), "no data");
    // Figures kept to four, grouped by thousands, either side of zero.
    assert_eq!(figure(203_812.0), "203,800");
    assert_eq!(figure(1_234_567_890.0), "1,235,000,000");
    assert_eq!(figure(-1500.0), "-1,500");
    assert_eq!(figure(0.683_449), "0.6834");
    assert_eq!(figure(7.0), "7");
    assert_eq!(figure(f32::NAN), "no data");
    // Every series and name at its longest still reads on one bounded line.
    let mut long = admissions();
    long.series = (0..8)
        .map(|_| {
            Series::new(
                "W".repeat(200),
                Colour::Teal,
                SeriesStyle::Line,
                vec![Some(1.0e15); 181],
            )
        })
        .collect();
    long.axis.unit = "W".repeat(200);
    let reading = long.read_at(5.0);
    assert!(reading.chars().count() <= READING_CHARS + 1, "{reading}");
    assert!(!reading.contains('\n'));
}

#[test]
fn a_chart_s_range_is_fixed_where_its_axis_says_and_otherwise_round_above_what_it_shows() {
    assert_eq!(
        [nice(1.3), nice(7.0), nice(9.5), nice(0.0), nice(f32::NAN)],
        [1.5, 8.0, 10.0, 1.0, 1.0]
    );
    // From zero to a round number above the highest value, or a reference line's with a third to spare.
    assert_eq!(pressure().range(), (0.0, 2.0));
    let mut fixed = pressure();
    fixed.axis = ValueAxis::new(Some(-1.0), Some(3.0), "");
    assert_eq!(fixed.range(), (-1.0, 3.0));
    // Ends that would be the same are a tenth of its size apart, or at least one.
    let mut flat = Chart::new(2);
    flat.axis = ValueAxis::new(Some(5.0), Some(5.0), "");
    assert_eq!(flat.range(), (5.0, 6.0));
    assert_eq!(Chart::new(2).range(), (0.0, 1.0));
}

#[test]
fn a_time_on_one_chart_is_found_on_another_of_its_kind_by_its_time_axis_never_its_labels() {
    let live = |points: u32, step_ms: u32| Chart::new(points).with_time(TimeAxis::Live { step_ms });
    // Ten minutes ago on 15 minutes at 5 s is the 50th minute of an hour's chart at a minute, and covered by neither 5
    // minutes nor a chart that says nothing of its time, whatever its labels say.
    let quarter = live(181, 5_000);
    let ten = quarter.time_of(60.0).unwrap();
    assert_eq!(ten, ChartTime::BeforeNow { ms: 600_000.0 });
    assert_eq!(live(61, 60_000).at_time(ten), Some(50.0));
    assert_eq!(live(61, 5_000).at_time(ten), None);
    let labelled =
        Chart::new(61).with_labels(vec![AxisLabel::new(0, "-1 h"), AxisLabel::new(60, "now")]);
    assert_eq!(labelled.at_time(ten), None);
    assert_eq!(labelled.time_of(30.0), None);
    // Within half a point of its ends it is its end; nonsense is the first point.
    assert_eq!(
        live(61, 60_000).at_time(ChartTime::BeforeNow { ms: -20_000.0 }),
        Some(60.0)
    );
    assert_eq!(
        quarter.time_of(f32::NAN),
        Some(ChartTime::BeforeNow { ms: 900_000.0 })
    );
    // A fixed window, 02:20 to 02:50 UTC on 12 November 2026, links only by the instant, to a window that overlaps it,
    // never to a live chart, and never to the same time of day on another day.
    const DAY: i64 = 86_400_000;
    let at = |hours: i64, minutes: i64| 1_794_441_600_000 + hours * 3_600_000 + minutes * 60_000;
    let fixed = |from: i64, to: i64| {
        Chart::new(181).with_time(TimeAxis::Fixed {
            from_ms: from,
            to_ms: to,
        })
    };
    let east = fixed(at(2, 20), at(2, 50));
    let instant = east.time_of(102.0).unwrap();
    assert_eq!(
        instant,
        ChartTime::Instant {
            utc_ms: at(2, 37) as f64
        }
    );
    assert_eq!(fixed(at(2, 30), at(3, 0)).at_time(instant), Some(42.0));
    assert_eq!(
        fixed(at(2, 30) - DAY, at(3, 0) - DAY).at_time(instant),
        None
    );
    assert_eq!(fixed(at(3, 0), at(3, 30)).at_time(instant), None);
    assert_eq!(quarter.at_time(instant), None);
    assert_eq!(east.at_time(ten), None);
    // A window of no width, or a step of none, as an untrusted server might send, finds nothing; an unknown kind
    // neither.
    assert_eq!(fixed(at(2, 20), at(2, 20)).at_time(instant), None);
    assert_eq!(live(61, 0).at_time(ten), None);
    let mut unknown = live(61, 60_000);
    unknown.time = Some(TimeAxis::Unknown);
    assert_eq!((unknown.time_of(1.0), unknown.at_time(ten)), (None, None));
}

#[test]
fn cleaning_keeps_a_time_axis_only_where_it_is_sound_and_it_is_part_of_a_chart_s_structure() {
    let with = |time: TimeAxis| Chart::new(181).with_time(time).cleaned().unwrap().time;
    assert_eq!(
        with(TimeAxis::Live { step_ms: 5_000 }),
        Some(TimeAxis::Live { step_ms: 5_000 })
    );
    assert_eq!(
        with(TimeAxis::Live {
            step_ms: CHART_STEP_MS
        }),
        Some(TimeAxis::Live {
            step_ms: CHART_STEP_MS
        })
    );
    assert_eq!(with(TimeAxis::Live { step_ms: 0 }), None);
    assert_eq!(
        with(TimeAxis::Live {
            step_ms: CHART_STEP_MS + 1
        }),
        None
    );
    let fixed = TimeAxis::Fixed {
        from_ms: 0,
        to_ms: 1_800_000,
    };
    assert_eq!(with(fixed), Some(fixed));
    assert_eq!(
        with(TimeAxis::Fixed {
            from_ms: 5,
            to_ms: 5
        }),
        None
    );
    assert_eq!(
        with(TimeAxis::Fixed {
            from_ms: 9,
            to_ms: 5
        }),
        None
    );
    assert_eq!(
        with(TimeAxis::Fixed {
            from_ms: CHART_EARLIEST_MS - 1,
            to_ms: 0
        }),
        None
    );
    assert_eq!(
        with(TimeAxis::Fixed {
            from_ms: 0,
            to_ms: CHART_INSTANT_MS + 1
        }),
        None
    );
    assert!(
        with(TimeAxis::Fixed {
            from_ms: CHART_EARLIEST_MS,
            to_ms: CHART_EARLIEST_MS + 1
        })
        .is_some()
    );
    // A window wider than its points at a day apart is still that window: a month on a chart of two points.
    let month = 31 * i64::from(CHART_STEP_MS);
    assert!(
        Chart::new(2)
            .with_time(TimeAxis::Fixed {
                from_ms: 0,
                to_ms: month
            })
            .cleaned()
            .unwrap()
            .time
            .is_some()
    );
    assert_eq!(with(TimeAxis::Unknown), None);
    // On the wire, absent where it says nothing, and read as nothing from an older server.
    let fixed_chart = Chart::new(181).with_time(fixed);
    let wire = serde_json::to_value(&fixed_chart).unwrap();
    assert_eq!(
        wire["time"],
        serde_json::json!({"kind": "fixed", "from_ms": 0, "to_ms": 1_800_000})
    );
    assert!(
        serde_json::to_value(Chart::new(181))
            .unwrap()
            .get("time")
            .is_none()
    );
    let older: Chart = serde_json::from_str(r#"{"points": 181}"#).unwrap();
    assert_eq!(older.time, None);
    // A window moved is another structure, as its labels moved are.
    assert!(fixed_chart.same_structure(&fixed_chart.clone()));
    assert!(!fixed_chart.same_structure(&Chart::new(181)));
}

/// A chart of `count` series, each named after its place, of 181 points.
fn of_series(count: usize) -> Chart {
    (0..count).fold(Chart::new(181), |chart, index| {
        chart.with_series(Series::new(
            format!("region {}", index + 1),
            Colour::NAMED[index % Colour::NAMED.len()].1,
            SeriesStyle::Line,
            vec![Some(index as f32); 181],
        ))
    })
}

#[test]
fn a_chart_keeps_eight_series_and_leaves_out_a_ninth() {
    assert_eq!(CHART_SERIES, 8);
    let eight = of_series(8).cleaned().unwrap();
    assert_eq!(eight, of_series(8));
    let nine = of_series(9).cleaned().unwrap();
    assert_eq!(nine.series, eight.series);
    assert!(nine.summary().contains("region 8: 7 now"));
    assert!(!nine.summary().contains("region 9"));
    // A shade may name the eighth, and not the ninth.
    let shaded = of_series(9)
        .with_shade(Shade::new(
            Edge::Series { index: 0 },
            Edge::Series { index: 7 },
            false,
            Colour::Grey,
        ))
        .with_shade(Shade::new(
            Edge::Series { index: 0 },
            Edge::Series { index: 8 },
            false,
            Colour::Grey,
        ))
        .cleaned()
        .unwrap();
    assert_eq!(shaded.shades.len(), 1);
}

/// How far apart two colours are as seen, in CIELAB, each linear RGB as a client draws it, seen through `sight`, a
/// matrix in linear RGB of Machado, Oliveira and Fernandes's simulation of colour blindness at its most severe.
fn seen_apart(a: [f32; 3], b: [f32; 3], sight: [[f32; 3]; 3]) -> f32 {
    let lab = |rgb: [f32; 3]| {
        let [r, g, b] =
            sight.map(|row| (row[0] * rgb[0] + row[1] * rgb[1] + row[2] * rgb[2]).clamp(0.0, 1.0));
        let f = |t: f32| {
            if t > 0.008_856 {
                t.cbrt()
            } else {
                7.787 * t + 16.0 / 116.0
            }
        };
        let (x, y, z) = (
            f((0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.950_47),
            f(0.2126 * r + 0.7152 * g + 0.0722 * b),
            f((0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.088_83),
        );
        [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
    };
    let (a, b) = (lab(a), lab(b));
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

#[test]
fn the_palette_has_a_distinct_colour_for_each_series() {
    // A colour for each series, none of them white, which reference lines such as a target are drawn in.
    assert_eq!(Colour::SERIES.len(), CHART_SERIES);
    assert!(!Colour::SERIES.contains(&Colour::White));
    for colour in Colour::SERIES {
        assert!(Colour::NAMED.iter().any(|(_, named)| *named == colour));
    }
    // Each series' colour, and white, told apart from every other, at least so far apart in CIELAB, with normal sight
    // and with each kind of colour blindness simulated; before eight series, the named colours were 2.2 apart with
    // protanopia and 2.5 with deuteranopia.
    let sights = [
        (
            "normal",
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            35.0,
        ),
        (
            "deuteranopia",
            [
                [0.367_322, 0.860_646, -0.227_968],
                [0.280_085, 0.672_501, 0.047_413],
                [-0.011_82, 0.042_94, 0.968_881],
            ],
            15.0,
        ),
        (
            "protanopia",
            [
                [0.152_286, 1.052_583, -0.204_868],
                [0.114_503, 0.786_281, 0.099_216],
                [-0.003_882, -0.048_116, 1.051_998],
            ],
            15.0,
        ),
        (
            "tritanopia",
            [
                [1.255_528, -0.076_749, -0.178_779],
                [-0.078_411, 0.930_809, 0.147_602],
                [0.004_733, 0.691_367, 0.303_9],
            ],
            12.0,
        ),
    ];
    let palette: Vec<Colour> = Colour::SERIES.into_iter().chain([Colour::White]).collect();
    for (sight, matrix, least) in sights {
        for (index, a) in palette.iter().enumerate() {
            for b in palette.iter().skip(index + 1) {
                let apart = seen_apart(a.rgb(), b.rgb(), matrix);
                assert!(apart >= least, "{sight}: {a:?} and {b:?} {apart}");
            }
        }
    }
    // Green is green, its green well above its red and blue, as teal's blue is above green's.
    let [red, green, blue] = Colour::Green.rgb();
    assert!(green > 1.5 * red && green > 1.5 * blue);
}

#[test]
fn a_colour_is_sent_as_srgb_and_drawn_linear_a_named_one_as_its_bytes_are() {
    // Each named colour is drawn as `#rrggbb` of its bytes is, so `#36d9bd` as teal.
    for (_, colour) in Colour::NAMED {
        let written = String::from(Colour::Rgb(colour.srgb()));
        assert_eq!(Colour::from(written).rgb(), colour.rgb());
    }
    assert_eq!(Colour::from("#36d9bd".to_owned()).rgb(), Colour::Teal.rgb());
    // Bytes are sRGB, decoded to linear: black and white their ends, the middle grey about a fifth.
    assert_eq!(Colour::Rgb([0, 0, 0]).rgb(), [0.0; 3]);
    assert_eq!(Colour::Rgb([255, 255, 255]).rgb(), [1.0; 3]);
    let [middle, ..] = Colour::Rgb([128, 128, 128]).rgb();
    assert!((middle - 0.2158).abs() < 1e-3, "{middle}");
    let [dark, ..] = Colour::Rgb([10, 10, 10]).rgb();
    assert!((dark - 10.0 / 255.0 / 12.92).abs() < 1e-6, "{dark}");
}

#[test]
fn a_reading_of_eight_series_says_each_whole_or_how_many_more() {
    assert_eq!(
        of_series(8).read_at(10.0),
        "point 11 of 181: region 1 0, region 2 1, region 3 2, region 4 3, region 5 4, region 6 5, region 7 6, \
         region 8 7"
    );
    // Names and units at their longest: as many whole as fit, then how many more, within the bound.
    let mut long = of_series(8);
    for series in &mut long.series {
        series.name = "W".repeat(CHART_NAME_CHARS);
    }
    long.axis.unit = "u".repeat(CHART_NAME_CHARS);
    let reading = long.read_at(10.0);
    assert!(reading.chars().count() <= READING_CHARS, "{reading}");
    assert!(reading.ends_with(", +5 more"), "{reading}");
    assert_eq!(reading.matches(&"W".repeat(CHART_NAME_CHARS)).count(), 3);
    // None that fit at all says only how many there are.
    let mut longest = of_series(8);
    longest.axis.unit = "u".repeat(READING_CHARS);
    assert_eq!(longest.read_at(10.0), "point 11 of 181: +8 more");
}
