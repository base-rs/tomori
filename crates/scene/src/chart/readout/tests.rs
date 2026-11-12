// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::chart::fixtures;
use crate::chart::{Area, draw};
use crate::ui_text::Cell;

/// A slot's chart face: 34 columns, and its plot below a heading and a legend, as a wall's slot draws it.
const AREA: Area = Area {
    x: 0.0,
    y: 48.0,
    width: 306.0,
    height: 132.0,
};
const BOUNDS: (f32, f32) = (306.0, 180.0);

fn plotted(chart: &Chart) -> PlotArea {
    draw(chart, AREA).plot.expect("every chart has a plot")
}

fn text(cells: &[Cell]) -> String {
    cells.iter().map(|cell| cell.0).collect()
}

#[test]
fn a_place_on_the_plot_is_a_time_along_its_axis_and_off_it_none() {
    let chart = fixtures::pressure();
    let plot = plotted(&chart);
    // The plot drawn is where its backing is.
    let drawing = draw(&chart, AREA);
    assert!(drawing.shapes.iter().any(|shape| matches!(
        *shape,
        Shape::Fill { x, y, width, height, .. }
            if (x, y, width, height) == (plot.left, plot.top, plot.width, plot.height)
    )));
    let middle = plot.top + plot.height / 2.0;
    assert_eq!(plot.at([plot.left, middle]), Some(0.0));
    assert_eq!(plot.at([plot.left + plot.width, middle]), Some(180.0));
    let at = plot.at([plot.left + plot.width * 0.6, plot.top]).unwrap();
    assert!((at - 108.0).abs() < 1e-3, "{at}");
    assert!((plot.x(at) - (plot.left + plot.width * 0.6)).abs() < 1e-3);
    for off in [
        [plot.left - 1.0, middle],
        [plot.left + plot.width + 1.0, middle],
        [plot.left + 10.0, plot.top - 1.0],
        [plot.left + 10.0, plot.top + plot.height + 1.0],
        [f32::NAN, middle],
    ] {
        assert_eq!(plot.at(off), None, "{off:?}");
    }
    // A value is placed down the plot, held within it beyond its range.
    assert_eq!(plot.y(plot.range.0), plot.top + plot.height);
    assert_eq!(plot.y(plot.range.1), plot.top);
    assert_eq!(plot.y(1.0e9), plot.top);
    assert_eq!(plot.y(-1.0e9), plot.top + plot.height);
}

#[test]
fn a_readout_marks_each_series_at_its_value_and_says_the_time_and_values_in_their_colours() {
    let chart = fixtures::pressure();
    let plot = plotted(&chart);
    let readout = Readout::new(&chart, &plot, 103.0, BOUNDS);
    assert_eq!(readout.x, plot.x(103.0));
    assert_eq!(readout.span, (plot.top, plot.top + plot.height));
    let mean = chart.value_at(0, 103.0).unwrap();
    let greatest = chart.value_at(1, 103.0).unwrap();
    assert_eq!(
        readout.dots,
        [
            ([readout.x, plot.y(mean)], ink(chart.series[0].colour, 1.0)),
            (
                [readout.x, plot.y(greatest)],
                ink(chart.series[1].colour, 1.0)
            ),
        ]
    );
    assert_eq!(
        readout.lines,
        [
            ("-6 min 25 s".to_owned(), TIME),
            (
                format!("mean {}", tomori_protocol::chart::figure(mean)),
                ink(chart.series[0].colour, 1.0)
            ),
            (
                format!("greatest {}", tomori_protocol::chart::figure(greatest)),
                ink(chart.series[1].colour, 1.0)
            ),
        ]
    );
    // Between two points the line is read between them, so the dot is on the line drawn.
    let between = Readout::new(&chart, &plot, 102.5, BOUNDS);
    let (low, high) = (
        chart.value_at(0, 102.0).unwrap(),
        chart.value_at(0, 103.0).unwrap(),
    );
    let read = chart.value_at(0, 102.5).unwrap();
    assert!(read >= low.min(high) - 1e-3 && read <= low.max(high) + 1e-3);
    assert_eq!(between.dots[0].0, [plot.x(102.5), plot.y(read)]);
}

#[test]
fn in_a_gap_a_readout_says_no_data_and_marks_nothing_never_zero() {
    let chart = fixtures::pressure();
    let plot = plotted(&chart);
    let readout = Readout::new(&chart, &plot, 63.0, BOUNDS);
    assert!(readout.dots.is_empty());
    assert_eq!(readout.lines[1].0, "mean no data");
    assert_eq!(readout.lines[2].0, "greatest no data");
    // A chart with nothing measured reads no data for every series.
    let none = fixtures::unmeasured();
    let readout = Readout::new(&none, &plotted(&none), 10.0, BOUNDS);
    assert!(readout.dots.is_empty());
    assert_eq!(readout.lines[1].0, "mean no data");
}

#[test]
fn every_series_of_several_is_read_at_the_same_time_steps_held_in_their_units() {
    let chart = fixtures::provisioning();
    let plot = plotted(&chart);
    let readout = Readout::new(&chart, &plot, 100.0, BOUNDS);
    let lines: Vec<&str> = readout
        .lines
        .iter()
        .map(|(text, _)| text.as_str())
        .collect();
    assert_eq!(
        lines,
        [
            "-6 min 40 s",
            "desired 8 replicas",
            "created 8 replicas",
            "ready 7 replicas"
        ]
    );
    assert_eq!(readout.dots.len(), 3);
    let colours: Vec<Rgba> = readout.lines[1..].iter().map(|(_, color)| *color).collect();
    let expected: Vec<Rgba> = chart
        .series
        .iter()
        .map(|series| ink(series.colour, 1.0))
        .collect();
    assert_eq!(colours, expected);
}

#[test]
fn a_readout_s_label_stays_inside_the_panel_wherever_it_points() {
    let chart = fixtures::admissions();
    let plot = plotted(&chart);
    let inside = |readout: &Readout, (width, height): (f32, f32)| {
        let [left, top, across, down] = readout.label;
        left >= 0.0 && top >= 0.0 && left + across <= width + 1e-3 && top + down <= height + 1e-3
    };
    // Right of the line where there is room, and left of it near the right edge.
    let early = Readout::new(&chart, &plot, 10.0, BOUNDS);
    assert!(early.label[0] > early.x);
    let late = Readout::new(&chart, &plot, 178.0, BOUNDS);
    assert!(late.label[0] + late.label[2] < late.x);
    for at in (0..=180).map(|at| at as f32) {
        assert!(
            inside(&Readout::new(&chart, &plot, at, BOUNDS), BOUNDS),
            "{at}"
        );
    }
    // On a face too narrow and short for it, its lines are cut and fewer, and it still fits.
    let small = (120.0, 30.0);
    let cramped = Readout::new(&chart, &plot, 90.0, small);
    assert!(inside(&cramped, small));
    assert_eq!(cramped.lines.len(), 2);
    assert!(cramped.lines[1].0.ends_with('…'), "{:?}", cramped.lines);
    // Its longest names and units are cut to a line's most characters.
    let largest = fixtures::largest(true);
    let wide = Readout::new(&largest, &plotted(&largest), 3.0, (2000.0, 400.0));
    assert_eq!(wide.lines.len(), LINES);
    assert!(
        wide.lines
            .iter()
            .all(|(text, _)| text.chars().count() <= READOUT_CHARS)
    );
}

#[test]
fn a_readout_is_drawn_over_the_face_as_large_as_it_with_nothing_else_on_it() {
    let chart = fixtures::pressure();
    let plot = plotted(&chart);
    let readout = Readout::new(&chart, &plot, 103.0, BOUNDS);
    let mut face = Canvas::new(34, 0.001);
    face.padding = (6, 5);
    face.rows = vec![Row::default(); 15];
    let overlay = readout.overlay(&face);
    assert_eq!(overlay.size(), face.size());
    assert_eq!(overlay.background[3], 0.0);
    assert_eq!(overlay.border, None);
    assert!(overlay.shapes.contains(&Shape::Stroke {
        from: [readout.x, plot.top],
        to: [readout.x, plot.top + plot.height],
        width: 1.0,
        color: CROSSHAIR,
    }));
    let dots = overlay
        .shapes
        .iter()
        .filter(|shape| matches!(shape, Shape::Dot { .. }))
        .count();
    assert_eq!(dots, 2 * readout.dots.len());
    let labels: Vec<String> = overlay
        .labels
        .iter()
        .map(|label| text(&label.cells))
        .collect();
    let lines: Vec<String> = readout.lines.iter().map(|(line, _)| line.clone()).collect();
    assert_eq!(labels, lines);
    // Its text and shapes stand inside its label's backing.
    let [left, top, across, down] = readout.label;
    for label in &overlay.labels {
        assert!(label.x >= left && label.x + label.width() <= left + across);
        assert!(label.y >= top && label.y + CELL_HEIGHT as f32 <= top + down);
    }
}

#[test]
fn beside_a_gap_nothing_is_read_past_the_last_point_drawn() {
    let chart = fixtures::pressure();
    let plot = plotted(&chart);
    // The pressure's lines end at point 57, before its gap: on the point, read; past it, toward the gap, nothing.
    let on = Readout::new(&chart, &plot, 57.0, BOUNDS);
    assert_eq!(on.dots.len(), 2);
    let past = Readout::new(&chart, &plot, 57.4, BOUNDS);
    assert!(past.dots.is_empty());
    assert_eq!(past.lines[1].0, "mean no data");
}

#[test]
fn a_thinned_line_is_read_where_it_is_drawn_and_bars_as_the_stretch_drawn() {
    // A line of 256 points swinging at every one, thinned to be drawn: its dot is on the line drawn, and its value the
    // line's there.
    let swinging = Chart::new(256)
        .with_axis(tomori_protocol::chart::ValueAxis::new(
            Some(0.0),
            Some(100.0),
            "",
        ))
        .with_series(tomori_protocol::chart::Series::new(
            "noisy",
            tomori_protocol::chart::Colour::Teal,
            tomori_protocol::chart::SeriesStyle::Line,
            (0..256)
                .map(|index| Some(((index * 37) % 101) as f32))
                .collect(),
        ));
    let plot = plotted(&swinging);
    assert_eq!(plot.thinned.len(), 1);
    let runs = &plot.thinned[0].1;
    assert!(runs.iter().map(Vec::len).sum::<usize>() <= crate::chart::LINE_VERTICES);
    for at in [10.3, 77.0, 200.6] {
        let readout = Readout::new(&swinging, &plot, at, BOUNDS);
        let drawn = plot.drawn(runs, readout.x).unwrap();
        assert!((readout.dots[0].0[1] - plot.y(drawn)).abs() < 0.05, "{at}");
        assert_eq!(
            readout.lines[1].0,
            format!(
                "noisy {}",
                tomori_protocol::chart::figure(tomori_protocol::chart::rounded(drawn).unwrap())
            )
        );
    }
    // Off every run drawn, nothing; a chart not thinned keeps none.
    assert_eq!(plot.drawn(runs, plot.left - 5.0), None);
    assert!(plotted(&fixtures::pressure()).thinned.is_empty());
    // Bars of 256 points, drawn one to a stretch as tall as its farthest from zero: read as the bar drawn under the line.
    let bars = Chart::new(256)
        .with_axis(tomori_protocol::chart::ValueAxis::new(
            Some(-5.0),
            Some(5.0),
            "",
        ))
        .with_series(tomori_protocol::chart::Series::new(
            "bars",
            tomori_protocol::chart::Colour::Red,
            tomori_protocol::chart::SeriesStyle::Bars,
            (0..256)
                .map(|index| Some(if index % 2 == 0 { 1.0 } else { -4.0 }))
                .collect(),
        ));
    let plot = plotted(&bars);
    let readout = Readout::new(&bars, &plot, 100.0, BOUNDS);
    assert_eq!(readout.lines[1].0, "bars -4");
    let drawn = draw(&bars, AREA);
    let under = drawn.shapes.iter().find_map(|shape| match *shape {
        Shape::Fill {
            x,
            y,
            width,
            height,
            ..
        } if x <= readout.x && readout.x <= x + width && width < plot.width => Some((y, height)),
        _ => None,
    });
    let (y, height) = under.expect("a bar is drawn under the line");
    assert!((y + height - plot.y(-4.0)).abs() < 1e-3);
}

#[test]
fn the_label_stands_where_it_covers_less_of_the_series_about_the_time_pointed_at() {
    let level = |value: f32| {
        Chart::new(10)
            .with_axis(tomori_protocol::chart::ValueAxis::new(
                Some(0.0),
                Some(10.0),
                "",
            ))
            .with_series(tomori_protocol::chart::Series::new(
                "level",
                tomori_protocol::chart::Colour::Teal,
                tomori_protocol::chart::SeriesStyle::Line,
                vec![Some(value); 10],
            ))
    };
    // High values: below them, at the plot's bottom; low ones, or none, above them, at its top.
    let high = level(9.5);
    let plot = plotted(&high);
    let below = Readout::new(&high, &plot, 3.0, BOUNDS);
    assert!(
        below.label[1] + below.label[3] > plot.top + plot.height - 4.0,
        "{:?}",
        below.label
    );
    let low = level(0.5);
    let above = Readout::new(&low, &plotted(&low), 3.0, BOUNDS);
    assert_eq!(above.label[1], plot.top + 2.0);
    // Held inside the face, however short.
    let short = Readout::new(&high, &plot, 3.0, (306.0, 40.0));
    assert!(short.label[1] >= 0.0 && short.label[1] + short.label[3] <= 40.0);
}

#[test]
fn a_linked_readout_reads_as_its_chart_does_with_its_line_and_frame_fainter() {
    let chart = fixtures::pressure();
    let plot = plotted(&chart);
    let readout = Readout::new(&chart, &plot, 60.0, BOUNDS);
    let linked = readout.clone().linked();
    assert!(!readout.linked && linked.linked);
    assert_eq!(
        (&linked.lines, &linked.dots, linked.x),
        (&readout.lines, &readout.dots, readout.x)
    );
    let face = Canvas::new(34, 0.01);
    let alpha = |canvas: &Canvas| match canvas.shapes[0] {
        Shape::Stroke { color, .. } => color[3],
        _ => panic!("its line first"),
    };
    let (own, faint) = (readout.overlay(&face), linked.overlay(&face));
    assert!((alpha(&faint) - alpha(&own) * LINKED).abs() < 1e-6);
    assert_eq!(own.labels, faint.labels);
}
