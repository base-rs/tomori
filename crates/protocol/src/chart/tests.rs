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
    assert_eq!(Colour::Rgb([255, 0, 51]).rgb(), [1.0, 0.0, 0.2]);
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
    assert!(bytes < 12 * 1024, "{bytes} bytes");
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
