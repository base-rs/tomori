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
