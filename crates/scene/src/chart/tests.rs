// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_protocol::chart::{AxisLabel, Reference, Series, Shade, ValueAxis};

const AREA: Area = Area {
    x: 0.0,
    y: 36.0,
    width: 306.0,
    height: 132.0,
};

/// The strokes of `drawing`: each one's ends, width and colour.
fn strokes(drawing: &Drawing) -> Vec<([f32; 2], [f32; 2], f32, Rgba)> {
    drawing
        .shapes
        .iter()
        .filter_map(|shape| match *shape {
            Shape::Stroke {
                from,
                to,
                width,
                color,
            } => Some((from, to, width, color)),
            _ => None,
        })
        .collect()
}

fn quads(drawing: &Drawing) -> Vec<([[f32; 2]; 4], Rgba)> {
    drawing
        .shapes
        .iter()
        .filter_map(|shape| match *shape {
            Shape::Quad { corners, color } => Some((corners, color)),
            _ => None,
        })
        .collect()
}

fn dots(drawing: &Drawing) -> Vec<([f32; 2], f32, Rgba)> {
    drawing
        .shapes
        .iter()
        .filter_map(|shape| match *shape {
            Shape::Dot {
                centre,
                radius,
                color,
            } => Some((centre, radius, color)),
            _ => None,
        })
        .collect()
}

fn fills(drawing: &Drawing, color: Rgba) -> Vec<(f32, f32, f32, f32)> {
    drawing
        .shapes
        .iter()
        .filter_map(|shape| match *shape {
            Shape::Fill {
                x,
                y,
                width,
                height,
                color: filled,
            } if filled == color => Some((x, y, width, height)),
            _ => None,
        })
        .collect()
}

fn texts(drawing: &Drawing) -> Vec<String> {
    drawing
        .labels
        .iter()
        .map(|label| crate::ui_text::plain(&label.cells))
        .collect()
}

/// The plot's top, bottom, left and right, as its backing, the first shape, covers it.
fn plot(drawing: &Drawing) -> (f32, f32, f32, f32) {
    let Shape::Fill {
        x,
        y,
        width,
        height,
        ..
    } = drawing.shapes[0]
    else {
        unreachable!("the plot's backing comes first")
    };
    (y, y + height, x, x + width)
}

/// Every coordinate of every shape and label of `drawing`, which must all be finite.
fn finite(drawing: &Drawing) -> bool {
    let mut pieces = Vec::new();
    for shape in &drawing.shapes {
        shape.pieces(&mut pieces);
    }
    pieces
        .iter()
        .flat_map(|piece| piece.corners)
        .flatten()
        .chain(drawing.labels.iter().flat_map(|label| [label.x, label.y]))
        .all(f32::is_finite)
}

/// How many pieces draw `drawing`'s shapes.
fn pieces(drawing: &Drawing) -> usize {
    let mut pieces = Vec::new();
    for shape in &drawing.shapes {
        shape.pieces(&mut pieces);
    }
    pieces.len()
}

fn line(values: Vec<Option<f32>>) -> Series {
    Series::new("a", Colour::Teal, SeriesStyle::Line, values)
}

#[test]
fn values_are_said_briefly_and_scales_are_round() {
    for (value, said) in [
        (1.0e15, "1.0e15"),
        (12_345.0, "12k"),
        (2_840.0, "2.8k"),
        (-1_500.0, "-1.5k"),
        (120.0, "120"),
        (2.0, "2"),
        (1.25, "1.2"),
        (0.25, "0.25"),
    ] {
        assert_eq!(brief(value), said, "{value}");
    }
    assert_eq!(nice(1.3), 1.5);
    assert_eq!(nice(7.0), 8.0);
    assert_eq!(nice(9.5), 10.0);
    assert_eq!(nice(1.0), 1.0);
    assert_eq!(nice(0.0), 1.0);
    assert_eq!(nice(f32::NAN), 1.0);
    assert_eq!(tone(Tone::Neutral), DIM);
    assert_ne!(tone(Tone::Good), tone(Tone::Bad));
    assert_ne!(tone(Tone::Warn), DIM);
    assert_eq!(ink(Colour::White, 0.5)[3], 0.5);
}

#[test]
fn a_chart_s_range_is_fixed_where_its_axis_says_and_otherwise_round_above_what_it_shows() {
    assert_eq!(range(&fixtures::pressure()), (0.0, 2.0));
    assert_eq!(range(&fixtures::unmeasured()), (0.0, 1.5));
    assert_eq!(range(&fixtures::provisioning()), (0.0, 10.0));
    assert_eq!(range(&fixtures::sampler()), (-1.0, 3.0));
    let fixed = Chart::new(2).with_axis(ValueAxis::new(Some(-5.0), Some(5.0), ""));
    assert_eq!(range(&fixed), (-5.0, 5.0));
    assert_eq!(range(&Chart::new(2)), (0.0, 1.0));
    let high = Chart::new(2)
        .with_series(line(vec![Some(1.0), Some(2.0)]))
        .with_axis(ValueAxis::new(Some(3.0), None, ""));
    assert_eq!(range(&high), (3.0, 4.0));
    // Ends the same, or the wrong way round, a tenth of the lowest's size, or 1, apart, even at the greatest values.
    let mut upside = Chart::new(2);
    upside.axis = ValueAxis::new(Some(2.0), Some(1.0), "");
    assert_eq!(range(&upside), (2.0, 3.0));
    upside.axis = ValueAxis::new(Some(1.0e15), Some(1.0e15), "");
    assert_eq!(range(&upside), (1.0e15, 1.1e15));
    // Values beyond what a chart holds are held to it, so the range is finite.
    let wild = Chart::new(2).with_series(line(vec![Some(3.0e38), Some(-3.0e38)]));
    let (low, high) = range(&wild);
    assert!(low == -CHART_MAGNITUDE && high.is_finite() && high > low);
    let flat = Chart::new(3).with_series(line(vec![Some(5.0); 3]));
    assert_eq!(range(&flat), (0.0, 5.0));
}

#[test]
fn a_chart_is_drawn_without_trusting_it_bounded_whatever_a_server_sends() {
    // Enormous counts and strings, and values beyond any a chart holds, draw a bounded number of pieces, quickly.
    let mut hostile = Chart::new(u32::MAX);
    for _ in 0..50 {
        hostile = hostile
            .with_series(
                line(
                    (0..10_000)
                        .map(|index| Some(if index % 2 == 0 { 3.0e38 } else { -3.0e38 }))
                        .collect(),
                )
                .with_breaks((0..10_000).collect())
                .with_width(f32::INFINITY),
            )
            .with_reference(Reference::new(
                f32::INFINITY,
                "r".repeat(10_000),
                Colour::Red,
            ))
            .with_shade(Shade::new(
                Edge::Series { index: 0 },
                Edge::Series { index: 1 },
                false,
                Colour::Red,
            ));
    }
    hostile.labels = (0..10_000)
        .map(|at| AxisLabel::new(at, "x".repeat(1_000)))
        .collect();
    hostile.axis = ValueAxis::new(Some(f32::NEG_INFINITY), Some(f32::NAN), "u".repeat(1_000));
    hostile.badge = Some(Note::new("b".repeat(10_000), Tone::Bad));
    hostile.status = Some(Note::new("s".repeat(10_000), Tone::Warn));
    let held = bounded(&hostile);
    assert_eq!(held.points, CHART_POINTS);
    assert_eq!(held.series.len(), CHART_SERIES);
    assert!(
        held.series
            .iter()
            .all(|series| series.values.len() == CHART_POINTS as usize
                && series.breaks.len() == CHART_POINTS as usize
                && series.width.is_none())
    );
    assert!(
        held.series[0]
            .values
            .iter()
            .flatten()
            .all(|value| value.abs() <= CHART_MAGNITUDE)
    );
    assert_eq!(
        (held.references.len(), held.shades.len(), held.labels.len()),
        (4, 4, 8)
    );
    assert_eq!(held.references[0].value, CHART_MAGNITUDE);
    assert_eq!(
        held.references[0].label.chars().count(),
        CHART_NAME_CHARS + 1
    );
    assert_eq!(held.axis.min, Some(-CHART_MAGNITUDE));
    assert_eq!(held.axis.max, None);
    assert_eq!(
        held.badge.as_ref().unwrap().text.chars().count(),
        CHART_NAME_CHARS + 1
    );
    assert_eq!(
        held.status.as_ref().unwrap().text.chars().count(),
        CHART_STATUS_CHARS + 1
    );
    assert_eq!(bounded(&Chart::new(0)).points, 2);
    let started = std::time::Instant::now();
    let drawing = draw(&hostile, AREA);
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert!(finite(&drawing));
    assert!(pieces(&drawing) < 2_500, "{}", pieces(&drawing));
    let legend = legend(&hostile, 34);
    assert!(legend.len() <= LEGEND_ROWS && legend.iter().all(|row| row.len() <= 34));
}

#[test]
fn the_largest_charts_are_drawn_in_a_bounded_number_of_pieces() {
    for chart in [fixtures::largest(false), fixtures::largest(true)] {
        let drawing = draw(&chart, AREA);
        // Each series' line and dashes, as many in all for eight series as for four, the fills together, the reference
        // lines' dashes, the dots where lines leave the plot, the marks of the last four and the rest.
        let marks = (CHART_SERIES - UNMARKED) * MARKS * 3;
        let most = UNMARKED * (2 * LINE_VERTICES + MARKERS)
            + marks
            + FILL_QUADS
            + 4 * REFERENCE_DASHES
            + 64;
        assert!(pieces(&drawing) <= most, "{} of {most}", pieces(&drawing));
        assert!(finite(&drawing));
    }
    // Lines thinned to their budget keep every swing's extremes: the zigzag reaches the plot's top and bottom.
    let zigzag = Chart::new(CHART_POINTS)
        .with_axis(ValueAxis::new(Some(0.0), Some(1.0), ""))
        .with_series(line(
            (0..CHART_POINTS)
                .map(|index| Some((index % 2) as f32))
                .collect(),
        ));
    let drawing = draw(&zigzag, AREA);
    let (top, bottom, ..) = plot(&drawing);
    let lines: Vec<_> = strokes(&drawing)
        .into_iter()
        .filter(|(.., color)| *color == ink(Colour::Teal, 1.0))
        .collect();
    assert!(lines.len() < LINE_VERTICES);
    let ends: Vec<f32> = lines
        .iter()
        .flat_map(|(from, to, ..)| [from[1], to[1]])
        .collect();
    assert!(
        ends.iter().any(|y| (y - top).abs() < 1.0) && ends.iter().any(|y| (y - bottom).abs() < 1.0)
    );
}

#[test]
fn a_line_breaks_at_every_gap_and_break_never_drawing_a_missing_value_as_zero() {
    let drawing = draw(&fixtures::pressure(), AREA);
    let teal = ink(Colour::Teal, 1.0);
    let mean: Vec<_> = strokes(&drawing)
        .into_iter()
        .filter(|(.., color)| *color == teal)
        .collect();
    // Merged where a point adds nothing, but never across the gap.
    assert!(mean.len() > 20 && mean.len() < 180);
    let (top, bottom, left, right) = plot(&drawing);
    let gap = (
        left + (right - left) * 57.0 / 180.0,
        left + (right - left) * 70.0 / 180.0,
    );
    assert!(
        mean.iter()
            .all(|(from, to, ..)| to[0] <= gap.0 + 2.0 || from[0] >= gap.1 - 2.0)
    );
    assert!(
        mean.iter().all(|(from, to, ..)| from[1] < bottom - 10.0
            && to[1] < bottom - 10.0
            && from[1] > top)
    );
    // Its parts overlap where they meet, by half its width, so that no joint shows a notch.
    let (first, second) = (mean[0], mean[1]);
    assert!(first.1[0] > second.0[0]);
    // A break the series gives ends a run too, its area with it.
    let admitted = draw(&fixtures::admissions(), AREA);
    let blue = ink(Colour::Blue, 1.0);
    let line: Vec<_> = strokes(&admitted)
        .into_iter()
        .filter(|(.., color)| *color == blue)
        .collect();
    let (.., left, right) = plot(&admitted);
    let middle = left + (right - left) * 90.5 / 180.0;
    assert!(
        line.iter()
            .all(|(from, to, ..)| from[0] > middle || to[0] < middle)
    );
    assert!(
        quads(&admitted)
            .iter()
            .all(|(corners, _)| corners[0][0] > middle || corners[1][0] < middle)
    );
}

#[test]
fn a_line_beyond_a_fixed_end_is_clipped_with_a_dot_where_it_leaves() {
    let chart = Chart::new(3)
        .with_axis(ValueAxis::new(Some(0.0), Some(1.0), ""))
        .with_series(line(vec![Some(0.5), Some(3.0), Some(0.5)]));
    let drawing = draw(&chart, AREA);
    let (top, ..) = plot(&drawing);
    let lines = strokes(&drawing)
        .into_iter()
        .filter(|(.., color)| *color == ink(Colour::Teal, 1.0))
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert!(
        lines
            .iter()
            .all(|(from, to, ..)| from[1] >= top - 1e-3 && to[1] >= top - 1e-3)
    );
    let marks = dots(&drawing);
    assert_eq!(marks.len(), 2);
    assert!(
        marks
            .iter()
            .all(|(centre, ..)| (centre[1] - top).abs() < 1e-3)
    );
    // Wholly beyond, nothing of it is drawn but where it is in range.
    let above = Chart::new(2)
        .with_axis(ValueAxis::new(Some(0.0), Some(1.0), ""))
        .with_series(line(vec![Some(2.0), Some(3.0)]));
    assert!(
        strokes(&draw(&above, AREA))
            .iter()
            .all(|(.., color)| *color != ink(Colour::Teal, 1.0))
    );
    // At most so many dots.
    let swings = Chart::new(64)
        .with_axis(ValueAxis::new(Some(0.0), Some(1.0), ""))
        .with_series(line(
            (0..64)
                .map(|index| Some(if index % 2 == 0 { -1.0 } else { 2.0 }))
                .collect(),
        ));
    assert_eq!(dots(&draw(&swings, AREA)).len(), MARKERS);
    // A level line along the plot's edge, and a lone point beyond it.
    let level = Chart::new(3)
        .with_axis(ValueAxis::new(Some(0.0), Some(1.0), ""))
        .with_series(line(vec![Some(1.0), Some(1.0), None]))
        .with_series(line(vec![None, None, Some(5.0)]))
        .with_series(line(vec![Some(-1.0), Some(-1.0), None]));
    let drawing = draw(&level, AREA);
    assert_eq!(
        strokes(&drawing)
            .iter()
            .filter(|(.., color)| *color == ink(Colour::Teal, 1.0))
            .count(),
        1
    );
    assert!(dots(&drawing).is_empty());
}

#[test]
fn shading_more_steps_than_its_budget_widens_its_steps() {
    let chart = Chart::new(CHART_POINTS)
        .with_series(line(
            (0..CHART_POINTS)
                .map(|index| Some((index % 2) as f32 * 2.0))
                .collect(),
        ))
        .with_series(line(
            (0..CHART_POINTS)
                .map(|index| Some(((index + 1) % 2) as f32 * 2.0))
                .collect(),
        ))
        .with_shade(Shade::new(
            Edge::Series { index: 0 },
            Edge::Series { index: 1 },
            false,
            Colour::Green,
        ));
    let shaded = quads(&draw(&chart, AREA));
    assert!(
        !shaded.is_empty() && shaded.len() <= FILL_QUADS,
        "{}",
        shaded.len()
    );
}

#[test]
fn steps_are_level_then_upright_and_shade_their_gap_level_too() {
    let drawing = draw(&fixtures::provisioning(), AREA);
    let blue = ink(Colour::Blue, 1.0);
    let desired: Vec<_> = strokes(&drawing)
        .into_iter()
        .filter(|(.., color)| *color == blue)
        .collect();
    // Each part level or upright, its ends overlapping by half its width where it meets the next.
    assert!(
        desired
            .iter()
            .all(|(from, to, ..)| (from[0] - to[0]).abs() < 1e-3 || (from[1] - to[1]).abs() < 1e-3)
    );
    assert!(
        desired
            .iter()
            .any(|(from, to, ..)| (from[0] - to[0]).abs() < 1e-3 && from[1] != to[1])
    );
    let amber = ink(Colour::Amber, SHADE_ALPHA);
    let gap: Vec<_> = quads(&drawing)
        .into_iter()
        .filter(|(_, color)| *color == amber)
        .collect();
    assert!(!gap.is_empty());
    assert!(
        gap.iter()
            .all(|(corners, _)| corners[0][1] == corners[1][1] && corners[2][1] == corners[3][1])
    );
    assert!(texts(&drawing).contains(&" SHORT 3 ".to_owned()));
    assert!(drawing.shapes.iter().any(|shape| matches!(shape, Shape::Rect { outline: Some(_), color, .. } if *color == tone(Tone::Warn))));
}

#[test]
fn dashes_bars_and_lone_points_are_drawn_as_their_series_say() {
    let drawing = draw(&fixtures::sampler(), AREA);
    let teal = ink(Colour::Teal, 1.0);
    let lone: Vec<_> = dots(&drawing)
        .into_iter()
        .filter(|(.., color)| *color == teal)
        .collect();
    assert_eq!(lone.len(), 4);
    assert!(lone.iter().all(|(_, radius, _)| *radius == 1.5));
    let amber = ink(Colour::Amber, 1.0);
    let dashed: Vec<_> = strokes(&drawing)
        .into_iter()
        .filter(|(.., color)| *color == amber)
        .collect();
    assert!(dashed.len() > 20);
    assert!(
        dashed
            .iter()
            .all(|(from, to, ..)| (to[0] - from[0]).abs() <= DASH.0 + 1e-3)
    );
    // The bars: one for each value but the one at zero, within the plot.
    let bars = fills(&drawing, ink(Colour::Rgb([200, 200, 200]), 1.0));
    assert_eq!(bars.len(), 4);
    let (top, bottom, left, right) = plot(&drawing);
    assert!(bars.iter().all(|(x, y, width, height)| *x >= left
        && x + width <= right + 1e-3
        && *y >= top
        && y + height <= bottom + 1e-3));
    // More bars than its budget, a bar for each stretch, as tall as its tallest.
    let many = Chart::new(CHART_POINTS).with_series(Series::new(
        "b",
        Colour::Green,
        SeriesStyle::Bars,
        (0..CHART_POINTS)
            .map(|index| Some(if index == 100 { 9.0 } else { 1.0 }))
            .collect(),
    ));
    let drawing = draw(&many, AREA);
    let bars = fills(&drawing, ink(Colour::Green, 1.0));
    assert_eq!(bars.len(), LINE_VERTICES);
    let (top, ..) = plot(&drawing);
    let (shortest, tallest) = bars.iter().fold((f32::MAX, 0.0_f32), |(least, most), bar| {
        (least.min(bar.3), most.max(bar.3))
    });
    assert!(tallest > 5.0 * shortest && top < bars[0].1);
    // Beyond a fixed end, a bar is held to the plot.
    let tall = Chart::new(2)
        .with_axis(ValueAxis::new(Some(0.0), Some(1.0), ""))
        .with_series(Series::new(
            "b",
            Colour::Green,
            SeriesStyle::Bars,
            vec![Some(5.0), None],
        ));
    let drawing = draw(&tall, AREA);
    let (top, ..) = plot(&drawing);
    assert!((fills(&drawing, ink(Colour::Green, 1.0))[0].1 - top).abs() < 1e-3);
}

#[test]
fn lines_are_merged_thinned_clipped_and_dashed_within_their_bounds() {
    // Points on a straight line are merged away; a corner is kept; a point far off is kept.
    let straight: Vec<[f32; 2]> = (0..10)
        .map(|index| [index as f32, index as f32 * 2.0])
        .collect();
    assert_eq!(simplify(&straight, SIMPLIFY).len(), 2);
    assert_eq!(
        simplify(&[[0.0, 0.0], [5.0, 5.0], [10.0, 0.0]], SIMPLIFY).len(),
        3
    );
    assert_eq!(
        simplify(&[[0.0, 0.0], [0.0, 0.0], [0.0, 3.0], [0.0, 0.0]], SIMPLIFY).len(),
        3
    );
    assert_eq!(simplify(&[[0.0, 0.0]], SIMPLIFY).len(), 1);
    // Thinned, the ends and each stretch's highest and lowest are kept, in order.
    let zigzag: Vec<[f32; 2]> = (0..100)
        .map(|index| [index as f32, (index % 2) as f32 * 10.0])
        .collect();
    let thinned = thin(&zigzag, 20);
    assert!(thinned.len() <= 20);
    assert!(thinned.windows(2).all(|pair| pair[0][0] < pair[1][0]));
    assert_eq!(
        (thinned[0], *thinned.last().unwrap()),
        (zigzag[0], zigzag[99])
    );
    assert_eq!(thin(&zigzag[..5], 20).len(), 5);
    let falling: Vec<[f32; 2]> = (0..50)
        .map(|index| [index as f32, 100.0 - index as f32])
        .collect();
    let thinned = thin(&falling, 10);
    assert!(thinned.windows(2).all(|pair| pair[0][0] < pair[1][0]));
    let level: Vec<[f32; 2]> = (0..50).map(|index| [index as f32, 1.0]).collect();
    assert!(thin(&level, 10).len() <= 10);
    // Clipped to the plot's top and bottom.
    assert_eq!(
        clip([0.0, 5.0], [10.0, 5.0], 0.0, 10.0),
        Some(([0.0, 5.0], [10.0, 5.0], false, false))
    );
    assert_eq!(clip([0.0, -5.0], [10.0, -5.0], 0.0, 10.0), None);
    assert_eq!(clip([0.0, 15.0], [10.0, 15.0], 0.0, 10.0), None);
    assert_eq!(
        clip([0.0, -10.0], [10.0, 20.0], 0.0, 10.0).map(|(from, to, a, b)| (from[1], to[1], a, b)),
        Some((0.0, 10.0, true, true))
    );
    assert_eq!(
        clip([0.0, 20.0], [10.0, -10.0], 0.0, 10.0).map(|(.., a, b)| (a, b)),
        Some((true, true))
    );
    assert_eq!(clip([0.0, -10.0], [1.0, -20.0], 0.0, 10.0), None);
    // Dashes are laid across the plot: an upright part is drawn where it stands in a dash, and not in a gap.
    assert_eq!(
        dashes(
            &[([0.0, 0.0], [5.0, 0.0]), ([5.0, 0.0], [5.0, 5.0])],
            0.0,
            7.0
        ),
        [([0.0, 0.0], [4.0, 0.0])]
    );
    assert_eq!(dashes(&[([2.0, 0.0], [2.0, 5.0])], 0.0, 7.0).len(), 1);
    assert_eq!(dashes(&[([0.0, 0.0], [70.0, 0.0])], 0.0, 7.0).len(), 10);
    assert_eq!(dashes(&[([70.0, 0.0], [0.0, 0.0])], 0.0, 7.0).len(), 10);
    assert!(dashes(&[], 0.0, 7.0).is_empty());
}

#[test]
fn labels_name_the_range_the_references_and_the_axis_without_overlapping() {
    let drawing = draw(&fixtures::pressure(), AREA);
    let said = texts(&drawing);
    for text in ["2", "1", "0", "-15 min", "-5 min", "now"] {
        assert!(said.contains(&text.to_owned()), "{text}: {said:?}");
    }
    assert!(!said.contains(&"-10 min".to_owned()));
    let label = |text: &str| {
        drawing
            .labels
            .iter()
            .find(|label| crate::ui_text::plain(&label.cells) == text)
            .unwrap()
    };
    assert!((label("now").x + label("now").width() - AREA.width).abs() < 3.0);
    let crowded = Chart::new(2)
        .with_series(line(vec![Some(9.5), Some(9.0)]))
        .with_reference(Reference::new(9.9, "limit", Colour::Red).with_dashed(true))
        .with_reference(Reference::new(20.0, "beyond", Colour::Red))
        .with_labels(vec![AxisLabel::new(1, "now")])
        .with_axis(ValueAxis::new(None, Some(10.0), ""));
    let drawing = draw(&crowded, AREA);
    let said = texts(&drawing);
    assert!(
        said.contains(&"9.9".to_owned())
            && !said.contains(&"10".to_owned())
            && !said.contains(&"20".to_owned()),
        "{said:?}"
    );
    let red = ink(Colour::Red, 1.0);
    let dashes = strokes(&drawing)
        .into_iter()
        .filter(|(_, _, width, color)| *width == REFERENCE_WIDTH && *color == red)
        .count();
    assert!(dashes > 10 && dashes <= REFERENCE_DASHES + 1);
}

#[test]
fn a_chart_without_values_says_so_and_never_draws_an_empty_plot_as_quiet() {
    assert!(
        texts(&draw(&fixtures::unmeasured(), AREA)).contains(&"No data: not measured".to_owned())
    );
    assert!(texts(&draw(&Chart::new(2), AREA)).contains(&"No data".to_owned()));
    assert!(
        !texts(&draw(&fixtures::stale(), AREA))
            .iter()
            .any(|text| text.starts_with("No data"))
    );
}

#[test]
fn a_legend_shows_whole_items_as_they_fit_and_says_what_is_missing() {
    let plain = |rows: Vec<Vec<Cell>>| {
        rows.iter()
            .map(|row| crate::ui_text::plain(row))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(
        plain(legend(&fixtures::pressure(), 34)),
        "── mean 0.79  ── greatest 1.3"
    );
    assert_eq!(
        plain(legend(&fixtures::pressure(), 60)),
        "── mean 0.79  ── greatest 1.3  target 1"
    );
    assert_eq!(
        plain(legend(&fixtures::stale(), 60)),
        "── mean --  ── greatest no data  target 1"
    );
    assert_eq!(plain(legend(&fixtures::pressure(), 6)), "── me…\n── gr…");
    assert!(legend(&Chart::new(2), 10).is_empty());
}

/// A series swinging between 0.9 and 1.3 at every point about a target of 1.0, shaded only above it, of `points`
/// points, with `others` more shades beside it.
fn swinging(points: u32, others: usize) -> Chart {
    let mut chart = Chart::new(points)
        .with_axis(ValueAxis::new(Some(0.0), Some(2.0), ""))
        .with_series(line(
            (0..points)
                .map(|index| Some(if index % 2 == 0 { 0.9 } else { 1.3 }))
                .collect(),
        ))
        .with_reference(Reference::new(1.0, "target", Colour::White))
        .with_shade(Shade::new(
            Edge::Reference { index: 0 },
            Edge::Series { index: 0 },
            true,
            Colour::Red,
        ));
    for _ in 0..others {
        chart = chart.with_shade(Shade::new(
            Edge::Reference { index: 0 },
            Edge::Series { index: 0 },
            false,
            Colour::Grey,
        ));
    }
    chart
}

#[test]
fn sharp_turns_are_rounded_with_a_dot_and_right_angles_overlap() {
    let zigzag = Chart::new(9)
        .with_axis(ValueAxis::new(Some(0.0), Some(1.0), ""))
        .with_series(line((0..9).map(|index| Some((index % 2) as f32)).collect()));
    let drawing = draw(&zigzag, AREA);
    assert_eq!(dots(&drawing).len(), 7);
    // At most so many.
    let many = Chart::new(CHART_POINTS)
        .with_axis(ValueAxis::new(Some(0.0), Some(1.0), ""))
        .with_series(line(
            (0..CHART_POINTS)
                .map(|index| Some((index % 2) as f32))
                .collect(),
        ));
    assert!(dots(&draw(&many, AREA)).len() <= JOINTS);
    // Steps turn by right angles: overlapped, no dots.
    assert!(dots(&draw(&fixtures::provisioning(), AREA)).is_empty());
}

/// The area `drawing` shades in `color`, in square font pixels.
fn shaded(drawing: &Drawing, color: Rgba) -> f32 {
    quads(drawing)
        .into_iter()
        .filter(|(_, filled)| *filled == color)
        .map(|(corners, _)| {
            // Its corners in order around it, top left, top right, bottom right and bottom left.
            let [a, b, d, c] = corners;
            let around = [a, b, c, d];
            (0..4)
                .map(|index| {
                    let ([x0, y0], [x1, y1]) = (around[index], around[(index + 1) % 4]);
                    x0 * y1 - x1 * y0
                })
                .sum::<f32>()
                .abs()
                / 2.0
        })
        .sum()
}

/// The area between `from` and `to`, or only where `to` is above `from`, as a plot of `drawing`'s size from `low` to
/// `high` shows it, worked out here apart from the drawing: both lines straight from point to point, clipped to the
/// range, summed over many thin strips.
fn exact(
    drawing: &Drawing,
    from: &[Option<f32>],
    to: &[Option<f32>],
    above: bool,
    (low, high): (f32, f32),
) -> f32 {
    let (top, bottom, left, right) = plot(drawing);
    let last = (from.len() - 1) as f32;
    let strips = 20_000;
    let width = (right - left) / strips as f32;
    let at = |values: &[Option<f32>], u: f32| {
        let index = (u as usize).min(values.len() - 1);
        let next = (index + 1).min(values.len() - 1);
        let (a, b) = (values[index]?, values[next]?);
        Some((a + (b - a) * (u - index as f32)).clamp(low, high))
    };
    (0..strips)
        .filter_map(|strip| {
            let u = (strip as f32 + 0.5) / strips as f32 * last;
            let (f, g) = (at(from, u)?, at(to, u)?);
            let gap = if above {
                (g - f).max(0.0)
            } else {
                (g - f).abs()
            };
            Some(gap / (high - low) * (bottom - top) * width)
        })
        .sum()
}

/// Ready and desired replicas swinging together between 0.2 and 1.8 at every one of `points` points, desired 0.1 above
/// ready, the gap shaded, with `others` more shades of it beside.
fn swinging_gap(points: u32, others: usize) -> (Chart, Vec<Option<f32>>, Vec<Option<f32>>) {
    let ready: Vec<Option<f32>> = (0..points)
        .map(|index| Some(if index % 2 == 0 { 0.2 } else { 1.8 }))
        .collect();
    let desired: Vec<Option<f32>> = ready
        .iter()
        .map(|value| value.map(|value| value + 0.1))
        .collect();
    let mut chart = Chart::new(points)
        .with_axis(ValueAxis::new(Some(0.0), Some(2.0), ""))
        .with_series(line(ready.clone()))
        .with_series(Series::new(
            "desired",
            Colour::Blue,
            SeriesStyle::Line,
            desired.clone(),
        ));
    for index in 0..=others {
        let colour = if index == 0 {
            Colour::Amber
        } else {
            Colour::Grey
        };
        chart = chart.with_shade(Shade::new(
            Edge::Series { index: 0 },
            Edge::Series { index: 1 },
            true,
            colour,
        ));
    }
    (chart, ready, desired)
}

#[test]
fn a_shaded_band_is_the_band_however_many_points_or_shades_and_never_lost() {
    for points in [61, 97, 101, 181, 256] {
        for others in [0, 1, 3] {
            let (chart, ready, desired) = swinging_gap(points, others);
            let drawing = draw(&chart, AREA);
            let amber = ink(Colour::Amber, SHADE_ALPHA);
            let count = quads(&drawing)
                .into_iter()
                .filter(|(_, color)| *color == amber)
                .count();
            assert!(
                count > 0 && count <= FILL_QUADS / (1 + others),
                "{points} {others}: {count}"
            );
            let (area, truth) = (
                shaded(&drawing, amber),
                exact(&drawing, &ready, &desired, true, (0.0, 2.0)),
            );
            assert!(
                (area - truth).abs() <= 0.05 * truth,
                "{points} {others}: {area} of {truth}"
            );
            // Only above a target, the swinging pressure too, never lost.
            let pressure = swinging(points, others);
            let drawing = draw(&pressure, AREA);
            let red = ink(Colour::Red, SHADE_ALPHA);
            let target = vec![Some(1.0); points as usize];
            let (area, truth) = (
                shaded(&drawing, red),
                exact(
                    &drawing,
                    &target,
                    &pressure.series[0].values,
                    true,
                    (0.0, 2.0),
                ),
            );
            assert!(
                area > 0.0 && (area - truth).abs() <= 0.05 * truth,
                "{points} {others}: {area} of {truth}"
            );
            // Never below the target.
            let (top, bottom, ..) = plot(&drawing);
            let line = bottom - (bottom - top) / 2.0;
            assert!(
                quads(&drawing)
                    .iter()
                    .filter(|(_, color)| *color == red)
                    .all(|(corners, _)| corners.iter().all(|[_, y]| *y <= line + 1e-3))
            );
        }
    }
    // The console's swinging pressure alike, above its target, shaded twice in the one colour.
    let chart = fixtures::swinging();
    let drawing = draw(&chart, AREA);
    let target = vec![Some(1.0); chart.points as usize];
    let truth = 2.0 * exact(&drawing, &target, &chart.series[0].values, true, (0.0, 2.0));
    let area = shaded(&drawing, ink(Colour::Red, SHADE_ALPHA));
    assert!((area - truth).abs() <= 0.05 * truth, "{area} of {truth}");
    // The console's tracking chart alike, both its shades the narrow band.
    let chart = fixtures::tracking();
    let drawing = draw(&chart, AREA);
    let truth = exact(
        &drawing,
        &chart.series[0].values,
        &chart.series[1].values,
        true,
        (0.0, 2.0),
    );
    for colour in [Colour::Amber, Colour::Red] {
        let area = shaded(&drawing, ink(colour, SHADE_ALPHA));
        assert!((area - truth).abs() <= 0.05 * truth, "{area} of {truth}");
    }
}

#[test]
fn a_live_chart_s_shading_changes_little_as_it_gains_a_point() {
    // A point more, about where an exact shading would once have given way to columns, moves the shaded area a little.
    for (points, others) in [(96, 0), (97, 0), (100, 1), (101, 1), (180, 1), (181, 3)] {
        let (before, ..) = swinging_gap(points, others);
        let (after, ..) = swinging_gap(points + 1, others);
        let amber = ink(Colour::Amber, SHADE_ALPHA);
        let (a, b) = (
            shaded(&draw(&before, AREA), amber),
            shaded(&draw(&after, AREA), amber),
        );
        assert!((a - b).abs() < 0.1 * a, "{points} {others}: {a} then {b}");
    }
}

#[test]
fn shading_is_clipped_to_the_plot_and_shows_nothing_where_the_edges_have_no_gap() {
    // Two lines that cross, between them, everywhere but where they meet; only above, half.
    let crossing = |above: bool, max: f32| {
        Chart::new(2)
            .with_axis(ValueAxis::new(Some(0.0), Some(max), ""))
            .with_series(line(vec![Some(0.0), Some(4.0)]))
            .with_series(line(vec![Some(4.0), Some(0.0)]))
            .with_shade(Shade::new(
                Edge::Series { index: 0 },
                Edge::Series { index: 1 },
                above,
                Colour::Green,
            ))
    };
    let green = ink(Colour::Green, SHADE_ALPHA);
    let both = draw(&crossing(false, 4.0), AREA);
    let half = draw(&crossing(true, 4.0), AREA);
    let (a, b) = (shaded(&both, green), shaded(&half, green));
    assert!((a / b - 2.0).abs() < 0.05, "{a} {b}");
    let truth = exact(
        &both,
        &[Some(0.0), Some(4.0)],
        &[Some(4.0), Some(0.0)],
        false,
        (0.0, 4.0),
    );
    assert!((a - truth).abs() < 0.02 * truth);
    // Clipped to a lower fixed end, held within the plot, and the area so clipped.
    let clipped = draw(&crossing(false, 3.0), AREA);
    let (top, ..) = plot(&clipped);
    assert!(
        quads(&clipped)
            .iter()
            .all(|(corners, _)| corners.iter().all(|[_, y]| *y >= top - 1e-3))
    );
    let truth = exact(
        &clipped,
        &[Some(0.0), Some(4.0)],
        &[Some(4.0), Some(0.0)],
        false,
        (0.0, 3.0),
    );
    assert!((shaded(&clipped, green) - truth).abs() < 0.02 * truth);
    // Wholly beyond the plot, or with no gap at all, nothing.
    let beyond = Chart::new(2)
        .with_axis(ValueAxis::new(Some(0.0), Some(1.0), ""))
        .with_series(line(vec![Some(2.0), Some(3.0)]))
        .with_reference(Reference::new(5.0, "", Colour::White))
        .with_shade(Shade::new(
            Edge::Series { index: 0 },
            Edge::Reference { index: 0 },
            false,
            Colour::Green,
        ));
    assert!(quads(&draw(&beyond, AREA)).is_empty());
    // An edge that names nothing, or a break, nothing there.
    let mut named = Chart::new(3)
        .with_series(line(vec![Some(1.0), Some(2.0), Some(3.0)]).with_breaks(vec![0]))
        .with_reference(Reference::new(0.5, "", Colour::White))
        .with_shade(Shade::new(
            Edge::Reference { index: 0 },
            Edge::Series { index: 0 },
            false,
            Colour::Green,
        ));
    let drawing = draw(&named, AREA);
    let (.., left, right) = plot(&drawing);
    let middle = (left + right) / 2.0;
    assert!(
        quads(&drawing)
            .iter()
            .all(|(corners, _)| corners[0][0] >= middle - 1e-3)
    );
    named.shades[0].to = Edge::Unknown;
    assert!(quads(&draw(&named, AREA)).is_empty());
    named.shades[0].from = Edge::Series { index: 7 };
    named.shades[0].to = Edge::Reference { index: 0 };
    assert!(quads(&draw(&named, AREA)).is_empty());
    // A gap leaves its stretch unshaded, never shaded as zero.
    let gappy = Chart::new(3)
        .with_series(line(vec![Some(1.0), None, Some(3.0)]))
        .with_reference(Reference::new(0.5, "", Colour::White))
        .with_shade(Shade::new(
            Edge::Reference { index: 0 },
            Edge::Series { index: 0 },
            false,
            Colour::Green,
        ));
    assert!(quads(&draw(&gappy, AREA)).is_empty());
}

#[test]
fn an_area_is_shaded_to_zero_on_either_side_of_it() {
    let chart = Chart::new(2).with_series(Series::new(
        "a",
        Colour::Blue,
        SeriesStyle::Area,
        vec![Some(-2.0), Some(2.0)],
    ));
    let drawing = draw(&chart, AREA);
    let blue = ink(Colour::Blue, AREA_ALPHA);
    let truth = exact(
        &drawing,
        &[Some(0.0), Some(0.0)],
        &[Some(-2.0), Some(2.0)],
        false,
        (-2.0, 2.0),
    );
    assert!((shaded(&drawing, blue) - truth).abs() < 0.02 * truth);
    // Each column stands where the area is: those on the left below zero, those on the right above it.
    let (top, bottom, left, right) = plot(&drawing);
    let zero = (top + bottom) / 2.0;
    for (corners, _) in quads(&drawing) {
        let middle = (corners[0][0] + corners[1][0]) / 2.0;
        if middle < (left + right) / 2.0 - 2.0 {
            assert!(corners[0][1] >= zero - 1e-3, "{corners:?}");
        } else if middle > (left + right) / 2.0 + 2.0 {
            assert!(corners[2][1] <= zero + 1e-3, "{corners:?}");
        }
    }
}

#[test]
fn a_step_series_of_many_changes_keeps_its_square_corners_line_and_shading_alike() {
    let levels: Vec<Option<f32>> = (0..CHART_POINTS)
        .map(|index| Some(((index * 7919) % 10) as f32))
        .collect();
    let chart = Chart::new(CHART_POINTS)
        .with_axis(ValueAxis::new(Some(0.0), Some(10.0), ""))
        .with_series(Series::new(
            "steps",
            Colour::Blue,
            SeriesStyle::Step,
            levels,
        ))
        .with_reference(Reference::new(5.0, "", Colour::White))
        .with_shade(Shade::new(
            Edge::Reference { index: 0 },
            Edge::Series { index: 0 },
            true,
            Colour::Amber,
        ))
        .with_shade(Shade::new(
            Edge::Reference { index: 0 },
            Edge::Series { index: 0 },
            false,
            Colour::Grey,
        ));
    let drawing = draw(&chart, AREA);
    let blue = ink(Colour::Blue, 1.0);
    let lines: Vec<_> = strokes(&drawing)
        .into_iter()
        .filter(|(.., color)| *color == blue)
        .collect();
    assert!(lines.len() <= LINE_VERTICES, "{}", lines.len());
    assert!(
        lines
            .iter()
            .all(|(from, to, ..)| (from[0] - to[0]).abs() < 1e-3 || (from[1] - to[1]).abs() < 1e-3)
    );
    // Its shading in upright, level columns.
    let amber = ink(Colour::Amber, SHADE_ALPHA);
    let shades: Vec<_> = quads(&drawing)
        .into_iter()
        .filter(|(_, color)| *color == amber)
        .collect();
    assert!(!shades.is_empty());
    assert!(
        shades
            .iter()
            .all(|(corners, _)| corners[0][1] == corners[1][1] && corners[0][0] == corners[2][0])
    );
    assert_eq!(super::columns(&[[1.0, 2.0]], 8), [[1.0, 2.0]]);
    // Thinned by columns of the plot: its ends, and each column's lowest and highest, once where there is one point.
    assert_eq!(
        gridded(
            &[
                [0.0, 0.0],
                [1.0, 5.0],
                [5.0, 1.0],
                [6.0, 9.0],
                [7.0, 4.0],
                [11.0, 3.0]
            ],
            0.0,
            4.0
        ),
        [[0.0, 0.0], [1.0, 5.0], [5.0, 1.0], [6.0, 9.0], [11.0, 3.0]]
    );
    // The console's many steps alike.
    let drawing = draw(&fixtures::stepping(), AREA);
    let green = ink(Colour::Green, 1.0);
    let lines: Vec<_> = strokes(&drawing)
        .into_iter()
        .filter(|(.., color)| *color == green)
        .collect();
    assert!(!lines.is_empty() && lines.len() <= LINE_VERTICES);
    assert!(
        lines
            .iter()
            .all(|(from, to, ..)| (from[0] - to[0]).abs() < 1e-3 || (from[1] - to[1]).abs() < 1e-3)
    );
}

#[test]
fn a_dashed_line_is_always_dashed_its_dashes_bounded_and_its_legend_says_so() {
    // Four series swinging across the plot at every one of 256 points, and a noisy one, each dashed all along: every
    // dash of the plot's pattern drawn, nothing in its gaps, and no more dashes than the plot has periods.
    let colours = [Colour::Teal, Colour::Amber, Colour::Blue, Colour::Green];
    let zigzag = |points: u32| {
        let mut chart = Chart::new(points).with_axis(ValueAxis::new(Some(0.0), Some(1.0), ""));
        for (index, colour) in colours.into_iter().enumerate() {
            let values = (0..points)
                .map(|point| Some(((point as usize + index) % 2) as f32))
                .collect();
            let series = Series::new("a", colour, SeriesStyle::Line, values).with_dashed(true);
            chart = chart.with_series(series.with_width(4.0));
        }
        chart
    };
    let noisy = |spread: f32| {
        let values = (0..CHART_POINTS as usize)
            .map(|point| Some(1.0 + spread * (((point * 7919) % 101) as f32 / 50.0 - 1.0)))
            .collect();
        Chart::new(CHART_POINTS)
            .with_axis(ValueAxis::new(Some(0.0), Some(2.0), ""))
            .with_series(
                Series::new("a", Colour::Teal, SeriesStyle::Line, values).with_dashed(true),
            )
    };
    for area in [AREA, WIDE] {
        for (chart, colours) in [
            (zigzag(CHART_POINTS), &colours[..]),
            (noisy(0.4), &colours[..1]),
            (noisy(0.8), &colours[..1]),
        ] {
            let drawing = draw(&chart, area);
            let (.., left, right) = plot(&drawing);
            let period = dash_period(right - left, LINE_VERTICES);
            let on = period * DASH.0 / (DASH.0 + DASH.1);
            let periods = ((right - left) / period).ceil() as usize;
            for colour in colours {
                let dashes: Vec<_> = strokes(&drawing)
                    .into_iter()
                    .filter(|(.., color)| *color == ink(*colour, 1.0))
                    .collect();
                assert!(dashes.len() <= 2 * LINE_VERTICES, "{}", dashes.len());
                let within = |x: f32| ((x - left) % period) <= on + 1e-3;
                assert!(
                    dashes
                        .iter()
                        .all(|(from, to, ..)| within(from[0]) && within(to[0]))
                );
                for index in 0..periods {
                    let start = left + index as f32 * period;
                    let end = (start + on).min(right);
                    let drawn = dashes.iter().any(|(from, to, ..)| {
                        from[0].min(to[0]) < end - 1e-3 && from[0].max(to[0]) > start + 1e-3
                    });
                    assert!(drawn, "{} {index} of {periods}", area.width);
                }
            }
        }
    }
    // A point added at its end changes only its end: the dashes before it are the same.
    let measured = |count: usize| {
        let values = (0..CHART_POINTS as usize)
            .map(|index| (index < count).then(|| (index as f32 / 7.0).sin()))
            .collect();
        Chart::new(CHART_POINTS)
            .with_axis(ValueAxis::new(Some(-1.0), Some(1.0), ""))
            .with_series(line(values).with_dashed(true))
    };
    let teal = ink(Colour::Teal, 1.0);
    let before = |count: usize| {
        let drawing = draw(&measured(count), AREA);
        let (.., left, right) = plot(&drawing);
        let edge = left + (right - left) * (count - 2) as f32 / (CHART_POINTS - 1) as f32;
        strokes(&drawing)
            .into_iter()
            .filter(|(from, to, _, color)| *color == teal && from[0] < edge && to[0] < edge)
            .collect::<Vec<_>>()
    };
    for count in [60, 100, 140] {
        let (one, two) = (before(count), before(count + 1));
        assert!(one.len() > 5);
        assert_eq!(one[..one.len() - 1], two[..one.len() - 1], "{count}");
    }
    // A short dashed line keeps its dashes, a short gap apart across the plot.
    let drawing = draw(&fixtures::sampler(), AREA);
    let amber = ink(Colour::Amber, 1.0);
    let dashed: Vec<_> = strokes(&drawing)
        .into_iter()
        .filter(|(.., color)| *color == amber)
        .collect();
    for pair in dashed.windows(2) {
        let (end, start) = (pair[0].1, pair[1].0);
        assert!((start[0] - end[0]).abs() <= DASH.1 + 1e-3);
    }
    // A flat dashed line across the widest plot is dashed, its dashes as the plot's size makes them.
    let flat = Chart::new(2).with_series(line(vec![Some(1.0), Some(1.0)]).with_dashed(true));
    let drawing = draw(&flat, WIDE);
    let (.., left, right) = plot(&drawing);
    let dashes: Vec<_> = strokes(&drawing)
        .into_iter()
        .filter(|(.., color)| *color == teal)
        .collect();
    let expected = ((right - left) / dash_period(right - left, LINE_VERTICES)).ceil() as usize;
    assert_eq!(dashes.len(), expected);
    assert!(expected > 50 && expected <= LINE_VERTICES / 2 + 1);
    // Its key in the legend is two dashes, a solid series' a line.
    let legend = crate::ui_text::plain(&legend(&zigzag(4), 34)[0]);
    assert!(legend.starts_with("─ ─ a"), "{legend}");
    assert!(
        crate::ui_text::plain(&super::legend(&fixtures::pressure(), 34)[0]).starts_with("── mean")
    );
}

/// Pressure at 0.8 but for a spike to 1.9 at one point in the middle of `points`, shaded above a 1.0 target, with
/// `others` more shades of it beside.
fn spike(points: u32, others: usize) -> (Chart, usize) {
    let middle = points as usize / 2;
    let values = (0..points as usize)
        .map(|index| Some(if index == middle { 1.9 } else { 0.8 }))
        .collect();
    let mut chart = Chart::new(points)
        .with_axis(ValueAxis::new(Some(0.0), Some(2.0), ""))
        .with_series(line(values))
        .with_reference(Reference::new(1.0, "", Colour::White));
    for index in 0..=others {
        let colour = if index == 0 {
            Colour::Red
        } else {
            Colour::Grey
        };
        chart = chart.with_shade(Shade::new(
            Edge::Reference { index: 0 },
            Edge::Series { index: 0 },
            true,
            colour,
        ));
    }
    (chart, middle)
}

/// A 4:1 slot's plot, as wide as one of a wall five times a standard slot's size.
const WIDE: Area = Area {
    x: 0.0,
    y: 36.0,
    width: 1_300.0,
    height: 132.0,
};

/// How a spike is drawn: how far above the target its line and its shading reach, in font pixels; from where to where
/// across its shading is, and its line is above the target.
struct Reach {
    line: f32,
    shading: f32,
    across: (f32, f32),
    excursion: (f32, f32),
}

/// How the spike of `points` with `others` more shades is drawn in `area`.
fn spike_reach(points: u32, others: usize, area: Area) -> Reach {
    let (chart, middle) = spike(points, others);
    let drawing = draw(&chart, area);
    let (top, bottom, left, right) = plot(&drawing);
    let target = bottom - (bottom - top) / 2.0;
    let teal = ink(Colour::Teal, 1.0);
    let peak = strokes(&drawing)
        .into_iter()
        .filter(|(.., color)| *color == teal)
        .flat_map(|(from, to, ..)| [from[1], to[1]])
        .fold(f32::MAX, f32::min);
    let red = ink(Colour::Red, SHADE_ALPHA);
    let shaded: Vec<[f32; 2]> = quads(&drawing)
        .into_iter()
        .filter(|(_, color)| *color == red)
        .flat_map(|(corners, _)| corners)
        .collect();
    let highest = shaded.iter().map(|[_, y]| *y).fold(f32::MAX, f32::min);
    let across = shaded
        .iter()
        .map(|[x, _]| *x)
        .fold((f32::MAX, f32::MIN), |(low, high), x| {
            (low.min(x), high.max(x))
        });
    // The line is above the target from a fifth of the way from the point before the spike to a fifth of the way back.
    let step = (right - left) / (points - 1) as f32;
    let x = left + step * middle as f32;
    Reach {
        line: target - peak,
        shading: target - highest,
        across,
        excursion: (x - step * 0.9 / 1.1, x + step * 0.9 / 1.1),
    }
}

#[test]
fn a_spike_s_shading_reaches_its_line_s_peak_and_spreads_as_far_as_it_within_a_pixel() {
    for area in [AREA, WIDE] {
        for points in [61, 97, 101, 181, 256] {
            for others in [0, 3] {
                let reach = spike_reach(points, others, area);
                let case = format!("{} {points} {others}", area.width);
                assert!(
                    (reach.line - reach.shading).abs() <= 1.0,
                    "{case}: {} of {}",
                    reach.shading,
                    reach.line
                );
                let ((low, high), (from, to)) = (reach.across, reach.excursion);
                assert!(
                    (low - from).abs() <= 1.0 && (high - to).abs() <= 1.0,
                    "{case}: {low}..{high} of {from}..{to}"
                );
            }
        }
    }
}

/// The places across `drawing`'s plot, a tenth of a font pixel apart, at which `value` is at most `level`: where a
/// region shaded above `level` has no gap.
fn closed(drawing: &Drawing, values: &[Option<f32>], level: f32) -> Vec<f32> {
    let (.., left, right) = plot(drawing);
    let last = values.len() - 1;
    (0..((right - left) * 10.0) as usize)
        .map(|tenth| left + tenth as f32 / 10.0)
        .filter(|x| {
            let at = (x - left) / (right - left) * last as f32;
            let index = (at as usize).min(last - 1);
            let (a, b) = (values[index].unwrap(), values[index + 1].unwrap());
            a + (b - a) * (at - index as f32) <= level
        })
        .collect()
}

/// Whether any quad of `drawing` in `color` reaches across `x`.
fn covers(drawing: &Drawing, color: Rgba, x: f32) -> bool {
    quads(drawing)
        .into_iter()
        .filter(|(_, filled)| *filled == color)
        .any(|(corners, _)| {
            let (low, high) = corners
                .iter()
                .fold((f32::MAX, f32::MIN), |(low, high), [x, _]| {
                    (low.min(*x), high.max(*x))
                });
            x > low + 1e-3 && x < high - 1e-3
        })
}

#[test]
fn nothing_is_shaded_where_the_gap_closes_however_many_points_or_shades() {
    let red = ink(Colour::Red, SHADE_ALPHA);
    for area in [AREA, WIDE] {
        for points in [61, 97, 101, 181, 256] {
            for others in [0, 3] {
                let chart = swinging(points, others);
                let drawing = draw(&chart, area);
                let closed = closed(&drawing, &chart.series[0].values, 1.0);
                assert!(!closed.is_empty());
                assert!(
                    closed.iter().all(|x| !covers(&drawing, red, *x)),
                    "{} {points} {others}",
                    area.width
                );
            }
        }
    }
}

#[test]
fn two_spikes_with_a_dip_between_are_each_shaded_to_their_peak_and_the_dip_not_at_all() {
    // Two points apart, back below the target between, in a plot 1,500 font pixels wide with four shades.
    let wide = Area {
        width: 1_560.0,
        ..WIDE
    };
    let (mut chart, middle) = spike(181, 3);
    chart.series[0].values[middle + 2] = Some(1.9);
    let drawing = draw(&chart, wide);
    let (top, bottom, left, right) = plot(&drawing);
    let red = ink(Colour::Red, SHADE_ALPHA);
    let closed = closed(&drawing, &chart.series[0].values, 1.0);
    let step = (right - left) / 180.0;
    let dip = left + step * (middle + 1) as f32;
    assert!(closed.iter().any(|x| (x - dip).abs() < step * 0.1));
    assert!(closed.iter().all(|x| !covers(&drawing, red, *x)));
    // Each reaches its line's peak, 0.9 above the target, within a pixel.
    let peak = (bottom - top) * 0.9 / 2.0;
    let target = bottom - (bottom - top) / 2.0;
    for spike in [middle, middle + 2] {
        let x = left + step * spike as f32;
        let highest = quads(&drawing)
            .into_iter()
            .filter(|(_, color)| *color == red)
            .flat_map(|(corners, _)| corners)
            .filter(|[at, _]| (at - x).abs() < step)
            .map(|[_, y]| target - y)
            .fold(0.0_f32, f32::max);
        assert!(
            (highest - peak).abs() <= 1.0,
            "{spike}: {highest} of {peak}"
        );
    }
}

#[test]
fn the_palette_s_colours_are_legible_on_the_plot() {
    // Each of the series' colours, and white, the reference lines', at least 4.5 times as bright as the plot over the
    // face, as text must be to be legible. How far apart they are is `tomori-protocol`'s to check.
    let luminance = |[r, g, b]: [f32; 3]| 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let face = [0.02, 0.035, 0.06];
    let plot =
        std::array::from_fn(|channel| PLOT[channel] * PLOT[3] + face[channel] * (1.0 - PLOT[3]));
    for (name, colour) in Colour::NAMED {
        let contrast = (luminance(colour.rgb()) + 0.05) / (luminance(plot) + 0.05);
        assert!(contrast >= 4.5, "{name}: {contrast}");
    }
}

#[test]
fn the_series_after_the_fourth_are_each_marked_along_their_line_by_a_marker_of_their_own() {
    assert_eq!(
        (0..CHART_SERIES).map(Marker::of).collect::<Vec<_>>(),
        [
            None,
            None,
            None,
            None,
            Some(Marker::Dot),
            Some(Marker::Ring),
            Some(Marker::Ringed),
            Some(Marker::Square)
        ]
    );
    assert_eq!(Marker::of(CHART_SERIES), None);
    let chart = fixtures::regions();
    let drawing = draw(&chart, AREA);
    let (top, bottom, left, right) = plot(&drawing);
    let area = drawing.plot.clone().unwrap();
    let rings: Vec<_> = drawing
        .shapes
        .iter()
        .filter_map(|shape| match *shape {
            Shape::Ring { centre, color, .. } => Some((centre, color)),
            _ => None,
        })
        .collect();
    let squares = drawing
        .shapes
        .iter()
        .filter(|shape| {
            matches!(
                shape,
                Shape::Rect {
                    outline: Some(_),
                    ..
                }
            )
        })
        .count();
    let colour = |index: usize| ink(chart.series[index].colour, 1.0);
    // A ring for each mark of the sixth and seventh, a square for the eighth's and a dot for the fifth's, one in each
    // column across the plot, where its line is drawn there, within the plot.
    let sixth: Vec<_> = rings
        .iter()
        .filter(|(_, color)| *color == colour(5))
        .collect();
    assert_eq!(sixth.len(), MARKS);
    assert_eq!(
        rings
            .iter()
            .filter(|(_, color)| *color == colour(6))
            .count(),
        MARKS
    );
    assert_eq!(squares, MARKS);
    let fifth: Vec<_> = dots(&drawing)
        .into_iter()
        .filter(|(_, radius, color)| *color == colour(4) && *radius == 3.0)
        .collect();
    assert_eq!(fifth.len(), MARKS);
    let column = (right - left) / MARKS as f32;
    for (mark, (centre, _)) in sixth.iter().enumerate() {
        assert!((centre[0] - (left + column * (mark as f32 + 1.5 / 4.0))).abs() < 1e-3);
        assert!(centre[1] >= top && centre[1] <= bottom);
        let at = area.at([centre[0], top]).unwrap();
        // A step's held from its point, as its line is drawn.
        let value = chart.series[5].values[at.floor() as usize].unwrap();
        assert!((centre[1] - area.y(value)).abs() < 1e-3, "{mark}");
    }
    // None for the first four, nor in any colour of theirs but their lines'.
    for index in 0..UNMARKED {
        assert!(rings.iter().all(|(_, color)| *color != colour(index)));
    }
    // None across a gap, beyond a fixed end of the axis, nor for bars, which are told apart by their colour.
    let mut gapped = fixtures::regions();
    gapped.series[4].values = vec![None; fixtures::POINTS as usize];
    gapped.series[5].values = vec![Some(1.0e6); fixtures::POINTS as usize];
    gapped.axis.max = Some(2000.0);
    gapped.series[6].style = SeriesStyle::Bars;
    let drawing = draw(&gapped, AREA);
    assert!(
        drawing
            .shapes
            .iter()
            .all(|shape| !matches!(shape, Shape::Ring { .. }))
    );
    assert!(
        dots(&drawing)
            .iter()
            .all(|(_, radius, color)| *color != colour(4) || *radius != 3.0)
    );
    // Nor across a break: a few points, each its own run, too few to be thinned.
    let mut broken = Chart::new(9);
    for (index, colour) in Colour::SERIES.into_iter().enumerate() {
        broken = broken.with_series(
            Series::new("a", colour, SeriesStyle::Line, vec![Some(1.0); 9]).with_breaks(
                if index == 7 {
                    (0..9).collect()
                } else {
                    Vec::new()
                },
            ),
        );
    }
    let squares = |drawing: &Drawing| {
        drawing
            .shapes
            .iter()
            .filter(|shape| {
                matches!(
                    shape,
                    Shape::Rect {
                        outline: Some(_),
                        ..
                    }
                )
            })
            .count()
    };
    assert_eq!(squares(&draw(&broken, AREA)), 0);
    broken.series[7].breaks.clear();
    assert_eq!(squares(&draw(&broken, AREA)), MARKS);
    // A line thinned to be drawn is marked where it is drawn.
    let mut thinned = fixtures::regions();
    for series in &mut thinned.series {
        series.values = (0..fixtures::POINTS as usize)
            .map(|point| Some(if point % 2 == 0 { 100.0 } else { 900.0 }))
            .collect();
    }
    let drawing = draw(&thinned, AREA);
    let area = drawing.plot.clone().unwrap();
    let (_, runs) = area.thinned.iter().find(|(index, _)| *index == 7).unwrap();
    let frames: Vec<_> = drawing
        .shapes
        .iter()
        .filter_map(|shape| match *shape {
            Shape::Rect {
                x,
                y,
                width,
                outline: Some(_),
                ..
            } => Some([x + width / 2.0, y + width / 2.0]),
            _ => None,
        })
        .collect();
    assert_eq!(frames.len(), MARKS);
    for [x, y] in frames {
        assert!((area.y(area.drawn(runs, x).unwrap()) - y).abs() < 1e-2);
    }
}

#[test]
fn eight_series_are_drawn_in_as_many_pieces_as_four_their_lines_thinned_to_a_share() {
    assert_eq!(share(LINE_VERTICES, 1), LINE_VERTICES);
    assert_eq!(share(LINE_VERTICES, UNMARKED), LINE_VERTICES);
    assert_eq!(share(LINE_VERTICES, CHART_SERIES), LINE_VERTICES / 2);
    // Eight series zigzagging at every one of 256 points: each thinned to its share, keeping every swing's extremes.
    let mut chart = Chart::new(CHART_POINTS).with_axis(ValueAxis::new(Some(0.0), Some(1.0), ""));
    for (index, colour) in Colour::SERIES.into_iter().enumerate() {
        chart = chart.with_series(Series::new(
            "a",
            colour,
            SeriesStyle::Line,
            (0..CHART_POINTS)
                .map(|point| Some(((point as usize + index) % 2) as f32))
                .collect(),
        ));
    }
    let drawing = draw(&chart, AREA);
    let (top, bottom, ..) = plot(&drawing);
    let area = drawing.plot.clone().unwrap();
    assert_eq!(area.thinned.len(), CHART_SERIES);
    for (index, runs) in &area.thinned {
        assert!(runs.iter().map(Vec::len).sum::<usize>() <= share(LINE_VERTICES, CHART_SERIES) + 2);
        let ys: Vec<f32> = runs.iter().flatten().map(|point| point[1]).collect();
        assert!(ys.iter().any(|y| (y - top).abs() < 1.0), "{index}");
        assert!(ys.iter().any(|y| (y - bottom).abs() < 1.0), "{index}");
    }
    let four = {
        let mut four = chart.clone();
        four.series.truncate(UNMARKED);
        pieces(&draw(&four, AREA))
    };
    let marks = (CHART_SERIES - UNMARKED) * MARKS * 3;
    assert!(
        pieces(&drawing) <= four + marks + 16,
        "{} of {four}",
        pieces(&drawing)
    );
}

#[test]
fn several_series_of_bars_stand_side_by_side_each_at_least_two_font_pixels_wide() {
    // One alone keeps a bar for each stretch of 128.
    let one = fixtures::bars(1);
    let drawing = draw(&one, AREA);
    assert_eq!(
        fills(&drawing, ink(one.series[0].colour, 1.0)).len(),
        LINE_VERTICES
    );
    assert_eq!(drawing.plot.unwrap().stretches, LINE_VERTICES);
    for count in [2, 4, 8] {
        for area in [AREA, WIDE] {
            let chart = fixtures::bars(count);
            let drawing = draw(&chart, area);
            let (_, _, left, right) = plot(&drawing);
            let plotted = drawing.plot.clone().unwrap();
            let stretches = plotted.stretches;
            assert_eq!(stretches, bar_stretches(256, right - left, count));
            // Thinned only where a stretch for each point would make them narrower: eight in a slot are a few dozen.
            assert!(
                (4..=LINE_VERTICES).contains(&stretches),
                "{count}: {stretches}"
            );
            if count == 8 && area == AREA {
                assert!(stretches < 32, "{stretches}");
            }
            // Each series a bar in each stretch, each at least two font pixels wide, within the plot, in its place.
            let mut all = Vec::new();
            for (place, series) in chart.series.iter().enumerate() {
                let drawn = fills(&drawing, ink(series.colour, 1.0));
                assert_eq!(drawn.len(), stretches, "{count}: {place}");
                for (x, _, width, _) in &drawn {
                    assert!(*width >= BAR_WIDTH - 0.05, "{count}: {width}");
                    assert!(*x >= left - 1e-3 && x + width <= right + 1e-3);
                }
                all.extend(drawn.into_iter().map(|(x, _, width, _)| (x, width, place)));
            }
            // None overlapping another, and in each stretch the series in their order, left to right.
            all.sort_by(|a, b| a.0.total_cmp(&b.0));
            for pair in all.windows(2) {
                assert!(
                    pair[0].0 + pair[0].1 <= pair[1].0 + 1e-3,
                    "{count}: {pair:?}"
                );
            }
            for group in all.chunks(count) {
                assert!(
                    group.iter().map(|bar| bar.2).eq(0..count),
                    "{count}: {group:?}"
                );
            }
            // The readout reads each the bar drawn: its stretch's farthest from the base, a dot at its top.
            let readout = super::readout::Readout::new(&chart, &plotted, 100.0, (2000.0, 400.0));
            for (index, series) in chart.series.iter().enumerate() {
                let stretch = 100 * stretches / 256;
                let want = series.values
                    [stretch * 256 / stretches..(stretch + 1) * 256 / stretches]
                    .iter()
                    .flatten()
                    .copied()
                    .fold(f32::MIN, f32::max);
                let read = bar_at(&chart, index, 100.0, stretches).unwrap();
                assert!((read - want).abs() <= want * 1e-3, "{read} {want}");
                assert_eq!(readout.dots[index].0[1], plotted.y(read));
            }
        }
    }
    // Too narrow a plot for two-pixel bars even in one stretch draws them in one, as narrow as they must be.
    assert_eq!(bar_stretches(256, 4.0, 8), 1);
    assert_eq!(bar_stretches(256, 300.0, 0), LINE_VERTICES);
    assert_eq!(bar_stretches(6, 300.0, 3), 6);
    // A readout of nothing reads nothing; nonsense reads the oldest.
    let chart = fixtures::bars(2);
    assert_eq!(bar_at(&chart, 5, 1.0, 10), None);
    assert_eq!(bar_at(&chart, 0, f32::NAN, 10), bar_at(&chart, 0, 0.0, 10));
    let mut gap = fixtures::bars(2);
    gap.series[1].values = vec![None; 256];
    assert_eq!(bar_at(&gap, 1, 3.0, 10), None);
}

#[test]
fn a_legend_of_eight_series_wraps_onto_a_second_row_and_says_how_many_more_do_not_fit() {
    let plain = |rows: &[Vec<Cell>]| {
        rows.iter()
            .map(|row| crate::ui_text::plain(row))
            .collect::<Vec<_>>()
    };
    let chart = fixtures::regions();
    // In a standard slot, as many as fit in two rows, then how many more.
    let slot = legend(&chart, 34);
    assert_eq!(
        plain(&slot),
        [
            "── eu-west-1 112  ── us-east-1 161",
            "── ap-south-1 324  +5",
        ]
    );
    // In a wide slot, every series whole, the fifth to the eighth with their markers in their keys, and its limit
    // where it fits.
    let wide = legend(&chart, 140);
    assert_eq!(wide.len(), 2);
    let said = plain(&wide).join("  ");
    for name in chart.series.iter().map(|series| series.name.as_str()) {
        assert!(said.contains(name), "{said}");
    }
    assert!(
        said.contains("─●─ us-west-2") && said.contains("─○─ eu-central-1"),
        "{said}"
    );
    assert!(
        said.contains("─◉─ ap-northeast-1") && said.contains("─☐─ af-south-1"),
        "{said}"
    );
    assert!(wide.iter().all(|row| row.len() <= 140));
    // Floating, or wide enough for all in a row, one row with its limit.
    let row = legend(&chart, 400);
    assert_eq!(row.len(), 1);
    assert!(crate::ui_text::plain(&row[0]).ends_with("limit 1.0k"));
    // However narrow, never more than two rows, none wider than the legend.
    for columns in 1..60 {
        let rows = legend(&chart, columns);
        assert!(rows.len() <= LEGEND_ROWS, "{columns}");
        assert!(
            rows.iter().all(|row| row.len() <= columns),
            "{columns}: {:?}",
            plain(&rows)
        );
    }
    assert_eq!(plain(&legend(&chart, 1)), ["…", "…"]);
}
