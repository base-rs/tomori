// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Charts drawn on a canvas: a [`Chart`]'s plot, its lines, steps, bars and shaded areas, reference lines, axes, badge
//! and labels, laid out in font pixels as [`shapes`](crate::shapes) and [`Label`]s, so that a chart is drawn in its
//! panel's own mesh, by the same pipeline as its text, tested against depth and drawn alike in both eyes, flat or on a
//! curved wall's slot (see `design/rendering.md`).
//!
//! **Bounded.** A chart is drawn without trusting it: [`bounded`] holds every count and string to the spec's bounds and
//! every value to [`CHART_MAGNITUDE`] first, and the drawing holds what it makes to a fixed number of pieces, whatever
//! the chart, so that its mesh, with every label, always fits the vertex buffer a wall's panel has: each series' line to
//! [`LINE_VERTICES`] vertices, its points closer than [`SIMPLIFY`] to the line through their neighbours merged away and,
//! where that is not enough, each stretch of its points kept as its lowest and highest; a dashed line's dashes to one
//! for each period across the plot, its points, where too many, to each period's lowest and highest; every shaded
//! region and area together to [`FILL_QUADS`] quads; and each dashed reference line to [`REFERENCE_DASHES`].
//!
//! **Lines.** A segment between two points is a [`Shape::Stroke`], one piece, antialiased across its length as a rule
//! is, so that a thin line at any angle is as steady as a rule when seen from far away; segments that meet overlap by
//! half the line's width, so that a joint, even a step's right angle, shows no notch; a point joined to neither
//! neighbour is a [`Shape::Dot`], so that a lone sample shows. A gap, `None`, breaks the line, as does a break the
//! series gives; nothing is drawn as zero. A line is clipped to the plot: where it leaves past a fixed end of the value
//! axis, it ends at the plot's edge with a dot there, so that it never reads as lying at that end. **Areas.** A shaded
//! area is cut into pieces, each where its edges have a gap throughout, in one order, within one of equal columns
//! across the plot, so that nothing is shaded where the edges meet or cross; each piece is a [`Shape::Quad`] reaching
//! the widest gap in it, a spike's peak, level there and sloping to its ends, or two meeting at the peak, as many as
//! its share of [`FILL_QUADS`] allows, each shading the area between the edges within it, clipped to the plot; a step
//! series' gap is one level quad a piece; a series' area is shaded down, or up, to zero, or the plot's edge nearer it.
//! **Dashes** are laid across the plot, a fixed period by its width, each the part of the line within it.
use tomori_protocol::chart::{
    CHART_LABELS, CHART_MAGNITUDE, CHART_NAME_CHARS, CHART_POINTS, CHART_REFERENCES, CHART_SERIES,
    CHART_SHADES, CHART_STATUS_CHARS, Chart, Colour, Edge, LINE_WIDTH, LINE_WIDTHS, Note,
    SeriesStyle, Tone,
};
use tomori_protocol::text::one_line;

pub mod readout;

use crate::canvas::Label;
use crate::shapes::Shape;
use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH, Cell, Rgba, Span, Style, cut};

/// The colour of a chart's axis labels and grid's text.
pub const DIM: Rgba = [0.6, 0.65, 0.72, 1.0];
/// The colour of its grid lines and the plot behind them.
const GRID: Rgba = [0.55, 0.62, 0.7, 0.2];
const PLOT: Rgba = [0.004, 0.008, 0.016, 0.8];
/// How opaque a shaded area is, and a series' area, over the plot.
const SHADE_ALPHA: f32 = 0.38;
const AREA_ALPHA: f32 = 0.26;
/// How long a dash is, and the gap after it, in font pixels, at their shortest.
const DASH: (f32, f32) = (4.0, 3.0);
/// How wide a bar is, as a share of the distance between two points.
const BAR: f32 = 0.7;
/// How wide a grid line and a reference line are, in font pixels.
const GRID_WIDTH: f32 = 0.75;
const REFERENCE_WIDTH: f32 = 1.25;

/// The most vertices of one series' line, its dashes and its bars, after merging and thinning.
pub const LINE_VERTICES: usize = tomori_protocol::chart::STRETCHES;
/// The most quads of every shaded region and series' area of a chart together, shared alike among its regions: a
/// region's columns across the plot are half as many as its share, and each piece of it is one quad, or two.
pub const FILL_QUADS: usize = 512;
/// The most dashes of one dashed reference line.
pub const REFERENCE_DASHES: usize = 64;
/// The most dots one series' line shows where it leaves the plot.
pub const MARKERS: usize = 16;
/// The most dots one series' line rounds its sharpest joints with: those turning by more than a right angle, where the
/// overlapping ends of its parts would notch the outside of the turn or overshoot it.
pub const JOINTS: usize = 64;
/// How near, in font pixels, a point may be to the line through its neighbours and be merged away: less than a pixel
/// at any distance a panel is read from.
pub const SIMPLIFY: f32 = 0.2;

/// The colour of `colour`, opaque, or `alpha` opaque.
pub fn ink(colour: Colour, alpha: f32) -> Rgba {
    let [red, green, blue] = colour.rgb();
    [red, green, blue, alpha]
}

/// The colour a note of `tone` is drawn in: what it says never reads as well where it is not.
pub fn tone(tone: Tone) -> Rgba {
    match tone {
        Tone::Good => [0.42, 0.86, 0.52, 1.0],
        Tone::Warn => [0.98, 0.72, 0.3, 1.0],
        Tone::Bad => [1.0, 0.45, 0.4, 1.0],
        _ => DIM,
    }
}

/// A value as briefly as an axis says it: `3.4k`, `120`, `1.5`, `0.25`; a very large one in scientific notation.
pub fn brief(value: f32) -> String {
    let size = value.abs();
    if size >= 1.0e7 {
        format!("{value:.1e}")
    } else if size >= 10_000.0 {
        format!("{:.0}k", value / 1000.0)
    } else if size >= 1000.0 {
        format!("{:.1}k", value / 1000.0)
    } else if size >= 10.0 || value == value.round() {
        format!("{value:.0}")
    } else if size >= 1.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.2}")
    }
}

pub use tomori_protocol::chart::nice;

/// `value` held to [`CHART_MAGNITUDE`] either side of zero, or `None` where it is not a number.
fn sane(value: f32) -> Option<f32> {
    (!value.is_nan()).then(|| value.clamp(-CHART_MAGNITUDE, CHART_MAGNITUDE))
}

/// `chart` as a client draws it, whatever a server sent: from 2 to [`CHART_POINTS`] points; at most [`CHART_SERIES`]
/// series, each of at most as many values as points, every value held to [`CHART_MAGNITUDE`], its breaks inside it and
/// its width within [`LINE_WIDTHS`]; at most [`CHART_REFERENCES`] reference lines, [`CHART_SHADES`] shades and
/// [`CHART_LABELS`] labels at points it has; its axis's ends held as its values are; and every name, label, unit and
/// note on one line, cut as the server cuts them.
pub fn bounded(chart: &Chart) -> Chart {
    let points = chart.points.clamp(2, CHART_POINTS);
    let name = |text: &str| one_line(text, CHART_NAME_CHARS);
    let mut bounded = Chart::new(points);
    bounded.series = chart
        .series
        .iter()
        .take(CHART_SERIES)
        .map(|series| {
            let mut kept = series.clone();
            kept.name = name(&series.name);
            kept.values = series
                .values
                .iter()
                .take(points as usize)
                .map(|value| value.and_then(sane))
                .collect();
            kept.breaks = series
                .breaks
                .iter()
                .copied()
                .filter(|at| *at < points)
                .take(points as usize)
                .collect();
            kept.width = series
                .width
                .filter(|width| width.is_finite())
                .map(|width| width.clamp(LINE_WIDTHS.0, LINE_WIDTHS.1));
            kept
        })
        .collect();
    bounded.references = chart
        .references
        .iter()
        .take(CHART_REFERENCES)
        .map(|reference| {
            let mut kept = reference.clone();
            kept.value = sane(reference.value).unwrap_or_default();
            kept.label = name(&reference.label);
            kept
        })
        .collect();
    bounded.shades = chart.shades.iter().take(CHART_SHADES).cloned().collect();
    bounded.labels = chart
        .labels
        .iter()
        .filter(|label| label.at < points)
        .take(CHART_LABELS)
        .map(|label| tomori_protocol::chart::AxisLabel::new(label.at, name(&label.text)))
        .collect();
    bounded.axis = tomori_protocol::chart::ValueAxis::new(
        chart.axis.min.and_then(sane),
        chart.axis.max.and_then(sane),
        name(&chart.axis.unit),
    );
    bounded.legend = chart.legend;
    bounded.badge = chart
        .badge
        .as_ref()
        .map(|badge| Note::new(name(&badge.text), badge.tone));
    bounded.status = chart
        .status
        .as_ref()
        .map(|status| Note::new(one_line(&status.text, CHART_STATUS_CHARS), status.tone));
    bounded
}

/// The range of values `chart`'s plot spans, lowest and highest, as [`Chart::range`] finds it.
pub fn range(chart: &Chart) -> (f32, f32) {
    chart.range()
}

/// Where a chart's plot is, in font pixels: its left and top edges, its width and its height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// What draws a chart: its shapes, back to front, its labels, and where its plot is, which its readout reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drawing {
    pub shapes: Vec<Shape>,
    pub labels: Vec<Label>,
    pub plot: Option<PlotArea>,
}

/// Where a chart's plot is drawn, in font pixels, its left and top edges, its width and its height, the range of
/// values it spans, lowest and highest, and how many points it has along its time axis: what its readout needs to read
/// the time and the values under a point on it, without laying the chart out again. See [`readout`].
#[derive(Clone, Debug, PartialEq)]
pub struct PlotArea {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
    pub range: (f32, f32),
    pub points: u32,
    /// The lines thinned to be drawn, as they are drawn, each series' by its index, a run of points, in font pixels,
    /// for each part its line draws: those of more points than [`LINE_VERTICES`], which its readout reads where they
    /// are drawn rather than where their points are.
    pub thinned: Vec<(usize, Vec<Vec<[f32; 2]>>)>,
}

impl PlotArea {
    /// Where along the time axis `point`, in font pixels, is, as a point's index or a share of the way between two;
    /// `None` off the plot.
    pub fn at(&self, [x, y]: [f32; 2]) -> Option<f32> {
        let across = (x - self.left) / self.width;
        let down = (y - self.top) / self.height;
        ((0.0..=1.0).contains(&across) && (0.0..=1.0).contains(&down))
            .then(|| across * (self.points.max(2) - 1) as f32)
    }

    /// Where `at`, a point's index or a share of the way between two, is across the plot.
    pub fn x(&self, at: f32) -> f32 {
        self.left + self.width * at / (self.points.max(2) - 1) as f32
    }

    /// The value at `y` down the plot, as [`PlotArea::y`] places it.
    pub fn value(&self, y: f32) -> f32 {
        let (low, high) = (f64::from(self.range.0), f64::from(self.range.1));
        let share = 1.0 - (f64::from(y) - f64::from(self.top)) / f64::from(self.height);
        (low + (high - low) * share) as f32
    }

    /// The value of the thinned line `runs` where it is drawn at `x`, between the two of its points about it; `None`
    /// where none of its runs is drawn there.
    pub fn drawn(&self, runs: &[Vec<[f32; 2]>], x: f32) -> Option<f32> {
        runs.iter()
            .flat_map(|run| run.windows(2))
            .find(|pair| pair[0][0] <= x && x <= pair[1][0])
            .map(|pair| {
                let span = pair[1][0] - pair[0][0];
                let share = ((x - pair[0][0]) / span.max(f32::EPSILON)).min(1.0);
                self.value(pair[0][1] + (pair[1][1] - pair[0][1]) * share)
            })
    }

    /// Where `value` is down the plot, held within it.
    pub fn y(&self, value: f32) -> f32 {
        let (low, high) = (f64::from(self.range.0), f64::from(self.range.1));
        let share = ((f64::from(value) - low) / (high - low)).clamp(0.0, 1.0);
        (f64::from(self.top) + f64::from(self.height) * (1.0 - share)) as f32
    }
}

/// The cells of `text` in `color`.
fn cells(text: &str, color: Rgba) -> Vec<Cell> {
    cut(&[Span::new(text, Style::plain(color))], usize::MAX)
}

/// How `chart` is laid out in its area, in font pixels: its plot's edges, and the range of values it spans.
struct Plot<'a> {
    chart: &'a Chart,
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    range: (f32, f32),
}

impl Plot<'_> {
    /// Where point `index` is across the plot.
    fn x(&self, index: usize) -> f32 {
        let last = self.chart.points.saturating_sub(1).max(1) as f32;
        self.left + self.width * index as f32 / last
    }

    /// Where `value` is down the plot, above or below it where it is beyond the range, worked out in double precision
    /// so that it is finite for every value held to [`CHART_MAGNITUDE`].
    fn y(&self, value: f32) -> f32 {
        let (low, high) = (f64::from(self.range.0), f64::from(self.range.1));
        let share = (f64::from(value) - low) / (high - low);
        (f64::from(self.top) + f64::from(self.height) * (1.0 - share)) as f32
    }

    fn bottom(&self) -> f32 {
        self.top + self.height
    }

    /// The value of `edge` at point `index`, if it has one.
    fn edge(&self, edge: Edge, index: usize) -> Option<f32> {
        match edge {
            Edge::Series { index: series } => self
                .chart
                .series
                .get(series as usize)?
                .values
                .get(index)
                .copied()
                .flatten(),
            Edge::Reference { index: reference } => self
                .chart
                .references
                .get(reference as usize)
                .map(|reference| reference.value),
            _ => None,
        }
    }

    /// Whether `edge` is level from each point to the next, as a step series is.
    fn stepped(&self, edge: Edge) -> bool {
        matches!(edge, Edge::Series { index } if self.chart.series.get(index as usize).is_some_and(|series| series.style == SeriesStyle::Step))
    }

    /// Whether `edge`'s line breaks after point `index`.
    fn broken(&self, edge: Edge, index: usize) -> bool {
        matches!(edge, Edge::Series { index: series } if self.chart.series.get(series as usize).is_some_and(|series| series.breaks.contains(&(index as u32))))
    }
}

/// One edge of a region to fill: a series or reference line, or a level, such as an area's base.
#[derive(Clone, Copy)]
enum Bound {
    Edge(Edge),
    Level(f32),
}

impl Plot<'_> {
    fn bound(&self, bound: Bound, index: usize) -> Option<f32> {
        match bound {
            Bound::Edge(edge) => self.edge(edge, index),
            Bound::Level(level) => Some(level),
        }
    }

    fn bound_stepped(&self, bound: Bound) -> bool {
        matches!(bound, Bound::Edge(edge) if self.stepped(edge))
    }

    fn bound_broken(&self, bound: Bound, index: usize) -> bool {
        matches!(bound, Bound::Edge(edge) if self.broken(edge, index))
    }

    /// The value of `bound` at `at`, a point's index or a share of the way between two, held to the plot's range: a
    /// straight line from one point to the next, or a step's value held from its point to the next; `None` where it is
    /// not measured at either end, or breaks between them.
    fn sample(&self, bound: Bound, at: f32) -> Option<f32> {
        let (low, high) = self.range;
        let last = self.chart.points.saturating_sub(1) as usize;
        let index = (at.max(0.0) as usize).min(last);
        let first = self.bound(bound, index)?;
        let share = at - index as f32;
        let value = if index == last || share <= 0.0 || self.bound_stepped(bound) {
            first
        } else {
            if self.bound_broken(bound, index) {
                return None;
            }
            let next = self.bound(bound, index + 1)?;
            first + (next - first) * share
        };
        Some(value.clamp(low, high))
    }

    /// How far `to` is above `from` at `at`, below it where negative, with the edge below.
    fn gap(&self, from: Bound, to: Bound, at: f32) -> Option<(f32, f32)> {
        let (f, g) = (self.sample(from, at)?, self.sample(to, at)?);
        Some((g - f, f.min(g)))
    }

    /// The quads that fill between `from` and `to`, or only where `to` is above `from`, `quads` of them at most. The
    /// region is cut into pieces, each where the edges are measured and have a gap throughout, in one order, within one
    /// of equal columns across the plot, half as many as `quads` and never narrower than a font pixel: so that a piece
    /// never covers a place, among fine samples of both edges and their points, where the edges have no gap, or swap,
    /// and how the plot is cut depends only on its width and how many regions share its quads. Each piece shades the
    /// area between the edges within it, reaching the widest gap in it, a spike's peak included: as one quad, level
    /// on top at that peak and sloping to its ends, or narrower about the peak where that would shade more; or, as
    /// many as the quads left allow, the widest first, as two quads that meet at the peak and slope to the gap at its
    /// ends. Where the pieces are more than `quads`, neighbours in one stretch of gap are joined, the narrowest first,
    /// and only then, the narrowest across where the gap closes. Between a step series and another edge, whose gap is
    /// level from point to point, a piece is one level quad as tall as the gap is on average. Everything is clipped to
    /// the plot.
    fn fill(&self, from: Bound, to: Bound, above: bool, quads: usize) -> Vec<[[f32; 2]; 4]> {
        let last = self.chart.points.saturating_sub(1).max(1) as f32;
        let stepped = self.bound_stepped(from) || self.bound_stepped(to);
        let count = (quads / 2).min(self.width.floor() as usize).max(1);
        let column = self.width / count as f32;
        let gap = |x: f32| self.gap(from, to, x / self.width * last);
        // A few samples to each font pixel, the columns' ends and the points, in order.
        let samples = (self.width * 4.0).ceil() as usize;
        let step = self.width / samples as f32;
        let mut places: Vec<f32> = (0..samples)
            .map(|sample| (sample as f32 + 0.5) * step)
            .collect();
        places.extend((0..=count).map(|index| index as f32 * column));
        places.extend((0..=last as usize).map(|index| index as f32 / last * self.width));
        places.sort_by(f32::total_cmp);
        places.dedup();
        let mut pieces: Vec<Piece> = Vec::new();
        let mut piece: Option<Piece> = None;
        let mut before: Option<(f32, f32)> = None;
        for &x in &places {
            let measured = gap(x);
            // How wide the gap is, and which way, or none.
            let here = measured.map(|(signed, below)| {
                let wide = if above { signed.max(0.0) } else { signed.abs() };
                (x, wide, below, signed > 0.0, signed)
            });
            let at_edge = (0..=count).any(|index| (index as f32 * column - x).abs() < 1e-4);
            match (here, piece.as_mut()) {
                (Some((x, wide, below, up, _)), Some(open)) if wide > 0.0 && up == open.up => {
                    open.add(x, wide, below);
                    if at_edge {
                        let mut next = Piece::new(x, wide, below, up, open.run);
                        next.area = 0.0;
                        pieces.push(std::mem::replace(open, next));
                    }
                }
                (here, _) => {
                    // Closed where the gap closes or swaps: the area on to where it closes, between two places.
                    let sliver = |step: f32, wide: f32, beyond: f32| {
                        step * wide * wide / (wide + beyond.abs()) / 2.0
                    };
                    if let Some(mut open) = piece.take() {
                        if let Some((x, .., signed)) = here {
                            open.area += sliver(x - open.right, open.end, signed);
                        }
                        pieces.push(open);
                    }
                    if let Some((x, wide, below, up, _)) = here.filter(|(_, wide, ..)| *wide > 0.0)
                    {
                        let run = pieces.last().map_or(0, |last| last.run + 1);
                        let mut open = Piece::new(x, wide, below, up, run);
                        if let Some((was, signed)) = before {
                            open.area += sliver(x - was, wide, signed);
                        }
                        piece = Some(open);
                    }
                }
            }
            before = measured.map(|(signed, _)| (x, signed));
        }
        pieces.extend(piece);
        pieces.retain(|piece| piece.area > 0.0);
        // Within the quads: neighbours in one stretch of gap joined first, then any, the narrowest first.
        while pieces.len() > quads.max(1) {
            let narrowest = |same: bool| {
                (0..pieces.len().saturating_sub(1))
                    .filter(|index| !same || pieces[*index].run == pieces[index + 1].run)
                    .min_by(|a, b| {
                        let width = |index: usize| pieces[index + 1].right - pieces[index].left;
                        width(*a).total_cmp(&width(*b))
                    })
            };
            let index = narrowest(true)
                .or_else(|| narrowest(false))
                .expect("more pieces than one");
            let next = pieces.remove(index + 1);
            pieces[index].join(next);
        }
        // Two quads to as many of the widest as the quads left allow.
        let mut order: Vec<usize> = (0..pieces.len()).collect();
        order.sort_by(|a, b| {
            let width = |index: usize| pieces[index].right - pieces[index].left;
            width(*b).total_cmp(&width(*a)).then(a.cmp(b))
        });
        let mut two = vec![false; pieces.len()];
        for index in order.into_iter().take(quads.saturating_sub(pieces.len())) {
            two[index] = !stepped;
        }
        let quad =
            |corners: [(f32, f32); 4]| corners.map(|(x, value)| [self.left + x, self.y(value)]);
        let lower = |x: f32, otherwise: f32| gap(x).map_or(otherwise, |(_, below)| below);
        let mut shaded = Vec::new();
        for (piece, two) in pieces.iter().zip(two) {
            let Piece {
                left,
                right,
                middle,
                peak,
                floor,
                area,
                ..
            } = *piece;
            if stepped {
                // Standing on the edge below it, weighted where the gap is widest, as tall as the gap on average.
                let width = (right - left).max(f32::EPSILON);
                let floor = if piece.weight > 0.0 {
                    piece.lower / piece.weight
                } else {
                    floor
                };
                let height = area / width;
                let top = floor + height;
                shaded.push(quad([
                    (left, top),
                    (right, top),
                    (left, floor),
                    (right, floor),
                ]));
                continue;
            }
            let (left, right) = narrowed((left, middle, right), peak, area);
            let (low_left, low_right) = (lower(left, floor), lower(right, floor));
            if two {
                let ends = [left, right].map(|x| {
                    gap(x).map_or(
                        0.0,
                        |(signed, _)| if above { signed.max(0.0) } else { signed.abs() },
                    )
                });
                let heights = sloped((left, middle, right), peak, (ends[0], ends[1]), area);
                if middle > left {
                    shaded.push(quad([
                        (left, low_left + heights.0),
                        (middle, floor + peak),
                        (left, low_left),
                        (middle, floor),
                    ]));
                }
                if right > middle {
                    shaded.push(quad([
                        (middle, floor + peak),
                        (right, low_right + heights.1),
                        (middle, floor),
                        (right, low_right),
                    ]));
                }
                continue;
            }
            // Level on top at the peak, as wide there as shades its area, about the peak.
            let width = right - left;
            let top = if width > 0.0 {
                ((2.0 * area / peak - width) / width).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (top_left, top_right) = (
                middle - (middle - left) * top,
                middle + (right - middle) * top,
            );
            shaded.push(quad([
                (top_left, floor + peak),
                (top_right, floor + peak),
                (left, low_left),
                (right, low_right),
            ]));
        }
        shaded
    }
}

/// A piece of a shaded region: where the edges have a gap throughout, in one order, within one column.
#[derive(Clone, Copy)]
struct Piece {
    /// Its first and last place with a gap, and where the gap is widest, how wide, and the edge below there.
    left: f32,
    right: f32,
    middle: f32,
    peak: f32,
    floor: f32,
    /// The gap at its last place, and the area it shades.
    end: f32,
    area: f32,
    /// The edge below, weighted by the gap, and the weight, for a level piece.
    lower: f32,
    weight: f32,
    /// Which way the gap is, and the stretch of gap it is in.
    up: bool,
    run: usize,
}

impl Piece {
    fn new(x: f32, wide: f32, below: f32, up: bool, run: usize) -> Self {
        Self {
            left: x,
            right: x,
            middle: x,
            peak: wide,
            floor: below,
            end: wide,
            area: 0.0,
            lower: 0.0,
            weight: 0.0,
            up,
            run,
        }
    }

    /// It, reaching on to `x`, where the gap is `wide` above `below`.
    fn add(&mut self, x: f32, wide: f32, below: f32) {
        let step = x - self.right;
        self.area += step * (self.end + wide) / 2.0;
        self.lower += below * wide * step;
        self.weight += wide * step;
        self.right = x;
        self.end = wide;
        if wide > self.peak {
            (self.middle, self.peak, self.floor) = (x, wide, below);
        }
    }

    /// It and `next`, after it, as one.
    fn join(&mut self, next: Piece) {
        self.right = next.right;
        self.end = next.end;
        self.area += next.area;
        self.lower += next.lower;
        self.weight += next.weight;
        if next.peak > self.peak {
            (self.middle, self.peak, self.floor) = (next.middle, next.peak, next.floor);
        }
    }
}

/// Where a column's shading starts and ends, from `left` to `right` about its widest gap, `peak`, at `middle`: as wide as
/// its gap is, or narrower, about the same place, where even sloping down to nothing at both ends from `peak` would
/// shade more than its `area`.
fn narrowed((left, middle, right): (f32, f32, f32), peak: f32, area: f32) -> (f32, f32) {
    let least = peak * (right - left) / 2.0;
    if least <= area {
        return (left, right);
    }
    let share = area / least;
    (
        middle - (middle - left) * share,
        middle + (right - middle) * share,
    )
}

/// How high a column's shading stands at its two ends, from `left` to `right` about `peak` at `middle`, the gaps there
/// being `ends`: those gaps, lowered towards nothing or raised towards `peak` alike, so that it shades `area`, or as near
/// as it can.
fn sloped(
    (left, middle, right): (f32, f32, f32),
    peak: f32,
    ends: (f32, f32),
    area: f32,
) -> (f32, f32) {
    let shaded = |(a, b): (f32, f32)| {
        (middle - left) * (a + peak) / 2.0 + (right - middle) * (peak + b) / 2.0
    };
    let (low, high) = ((0.0, 0.0), (peak, peak));
    let (from, to) = if shaded(ends) > area {
        (low, ends)
    } else {
        (ends, high)
    };
    let (a, b) = (shaded(from), shaded(to));
    let share = if b > a {
        ((area - a) / (b - a)).clamp(0.0, 1.0)
    } else {
        1.0
    };
    (
        from.0 + (to.0 - from.0) * share,
        from.1 + (to.1 - from.1) * share,
    )
}

/// `points` with each point nearer than `tolerance` to the line through the points kept either side of it left out,
/// as Douglas and Peucker simplify a line: its ends always kept.
fn simplify(points: &[[f32; 2]], tolerance: f32) -> Vec<[f32; 2]> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0, points.len() - 1)];
    while let Some((first, last)) = stack.pop() {
        let ([x0, y0], [x1, y1]) = (points[first], points[last]);
        let (dx, dy) = (x1 - x0, y1 - y0);
        let length = dx.hypot(dy);
        let distance = |[x, y]: [f32; 2]| {
            if length <= f32::EPSILON {
                (x - x0).hypot(y - y0)
            } else {
                ((x - x0) * dy - (y - y0) * dx).abs() / length
            }
        };
        let farthest = (first + 1..last)
            .map(|index| (index, distance(points[index])))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((index, far)) = farthest
            && far > tolerance
        {
            keep[index] = true;
            stack.push((first, index));
            stack.push((index, last));
        }
    }
    points
        .iter()
        .zip(keep)
        .filter_map(|(point, kept)| kept.then_some(*point))
        .collect()
}

/// `points` thinned to about `most`: its ends, and, for each of `most / 2` stretches between, its highest and lowest
/// point, in order, so that a spike is never thinned away.
fn thin(points: &[[f32; 2]], most: usize) -> Vec<[f32; 2]> {
    if points.len() <= most.max(2) {
        return points.to_vec();
    }
    let inner = &points[1..points.len() - 1];
    let stretches = (most.saturating_sub(2) / 2).max(1);
    let mut kept = vec![points[0]];
    for stretch in 0..stretches {
        let part =
            &inner[stretch * inner.len() / stretches..(stretch + 1) * inner.len() / stretches];
        let by_height = |a: &(usize, &[f32; 2]), b: &(usize, &[f32; 2])| a.1[1].total_cmp(&b.1[1]);
        // Never empty: there are fewer stretches than points between the ends.
        let top = part
            .iter()
            .enumerate()
            .min_by(by_height)
            .expect("a stretch has points");
        let bottom = part
            .iter()
            .enumerate()
            .max_by(by_height)
            .expect("a stretch has points");
        let (first, second) = if top.0 <= bottom.0 {
            (top, bottom)
        } else {
            (bottom, top)
        };
        // Never the same point: a stretch has at least two, the first lowest and the last highest of equals.
        kept.push(*first.1);
        kept.push(*second.1);
    }
    kept.push(points[points.len() - 1]);
    kept
}

/// The part of the segment from `from` to `to` within the plot's top and bottom, with whether each end of it was cut
/// there; `None` where it is wholly beyond them.
fn clip(
    from: [f32; 2],
    to: [f32; 2],
    top: f32,
    bottom: f32,
) -> Option<([f32; 2], [f32; 2], bool, bool)> {
    let (dy, mut t0, mut t1) = (to[1] - from[1], 0.0_f32, 1.0_f32);
    for (limit, below) in [(top, false), (bottom, true)] {
        // The share along it where it meets `limit`, and which side of it is inside.
        if dy == 0.0 {
            let outside = if below {
                from[1] > limit
            } else {
                from[1] < limit
            };
            if outside {
                return None;
            }
            continue;
        }
        let t = (limit - from[1]) / dy;
        let entering = if below { dy < 0.0 } else { dy > 0.0 };
        if entering {
            t0 = t0.max(t);
        } else {
            t1 = t1.min(t);
        }
    }
    if t0 > t1 {
        return None;
    }
    let at = |t: f32| [from[0] + (to[0] - from[0]) * t, from[1] + dy * t];
    Some((at(t0), at(t1), t0 > 0.0, t1 < 1.0))
}

/// The period of a dashed line in a plot `width` font pixels wide, a dash and the gap after it: [`DASH`]'s, or, where a
/// level line across the plot would take more than half of [`LINE_VERTICES`] of them, longer in proportion, the gap
/// still three sevenths of it; fixed by the plot's size alone, never by the line.
fn dash_period(width: f32) -> f32 {
    (DASH.0 + DASH.1).max(width / (LINE_VERTICES / 2) as f32)
}

/// The dashes of `path`, its parts, laid across the plot from `left` in periods of `period`, a dash then a gap: in each
/// dash, the parts of `path` across it, so that the pattern depends only on the plot, however the line runs, its gaps
/// the same across it everywhere, a point added at its end changes only its end, and there are no more dashes than
/// periods across the plot; an upright part is drawn where it stands within a dash.
fn dashes(path: &[([f32; 2], [f32; 2])], left: f32, period: f32) -> Vec<([f32; 2], [f32; 2])> {
    let on = period * DASH.0 / (DASH.0 + DASH.1);
    let mut dashes = Vec::new();
    for &(from, to) in path {
        let ([x0, y0], [x1, y1]) = (from, to);
        let first = ((x0.min(x1) - left) / period).floor() as i64;
        let last = ((x0.max(x1) - left) / period).floor() as i64;
        for period_index in first..=last {
            let (start, end) = (
                left + period_index as f32 * period,
                left + period_index as f32 * period + on,
            );
            if (x1 - x0).abs() <= f32::EPSILON {
                if x0 >= start && x0 <= end {
                    dashes.push((from, to));
                }
                continue;
            }
            let share = |x: f32| ((x - x0) / (x1 - x0)).clamp(0.0, 1.0);
            let (a, b) = (share(start), share(end));
            let (t0, t1) = (a.min(b), a.max(b));
            if t1 - t0 > 1e-6 {
                let point = |t: f32| [x0 + (x1 - x0) * t, y0 + (y1 - y0) * t];
                dashes.push((point(t0), point(t1)));
            }
        }
    }
    dashes
}

/// What draws `chart` in `area`, in font pixels: its plot's backing, grid and labels, shaded areas, bars, areas,
/// reference lines and lines, front last, its badge, and, with no value at all, that it has no data, its status line
/// with it where it has one, rather than an empty plot that could be read as quiet. It draws `chart` as [`bounded`]
/// holds it, within the bounds its module describes.
pub fn draw(chart: &Chart, area: Area) -> Drawing {
    let chart = &bounded(chart);
    let mut drawing = Drawing::default();
    let range = range(chart);
    // Labels down the left: the range's ends, dim, and each reference line's value within it, in its colour.
    let within = |value: f32| value >= range.0 && value <= range.1;
    let mut marks: Vec<(f32, String, Rgba)> = chart
        .references
        .iter()
        .filter(|reference| within(reference.value))
        .map(|reference| {
            (
                reference.value,
                brief(reference.value),
                ink(reference.colour, 1.0),
            )
        })
        .collect();
    marks.extend([
        (range.1, brief(range.1), DIM),
        (range.0, brief(range.0), DIM),
    ]);
    let gutter = marks
        .iter()
        .map(|(_, text, _)| cells(text, DIM).len())
        .max()
        .unwrap_or_default() as f32
        * CELL_WIDTH as f32
        + 4.0;
    let row = CELL_HEIGHT as f32;
    let plot = Plot {
        chart,
        left: area.x + gutter,
        top: area.y + row / 2.0,
        width: (area.width - gutter - 2.0).max(1.0),
        height: (area.height - 2.0 * row).max(1.0),
        range,
    };
    drawing.plot = Some(PlotArea {
        left: plot.left,
        top: plot.top,
        width: plot.width,
        height: plot.height,
        range,
        points: chart.points,
        thinned: Vec::new(),
    });
    let shapes = &mut drawing.shapes;
    shapes.push(Shape::Fill {
        x: plot.left,
        y: plot.top,
        width: plot.width,
        height: plot.height,
        color: PLOT,
    });
    let mut placed: Vec<f32> = Vec::new();
    for (value, text, color) in marks {
        let y = plot.y(value);
        if placed.iter().any(|other| (other - y).abs() < row) {
            continue;
        }
        placed.push(y);
        let label = cells(&text, color);
        let x = plot.left - 4.0 - (label.len() * CELL_WIDTH) as f32;
        drawing.labels.push(Label::new(x, y - row / 2.0, label));
    }
    // The grid: a line across at each end of the range, and one down at each axis label, its text below the plot.
    let line = |from: [f32; 2], to: [f32; 2], width: f32, color: Rgba| Shape::Stroke {
        from,
        to,
        width,
        color,
    };
    for y in [plot.top, plot.bottom()] {
        shapes.push(line(
            [plot.left, y],
            [plot.left + plot.width, y],
            GRID_WIDTH,
            GRID,
        ));
    }
    // The first label starts at its point and the last ends at its, the rest centred on theirs; one that would
    // overlap the label before it is left out.
    let mut end = f32::MIN;
    let last = chart.points.saturating_sub(1);
    for label in &chart.labels {
        let x = plot.x(label.at as usize);
        shapes.push(line([x, plot.top], [x, plot.bottom()], GRID_WIDTH, GRID));
        let text = cells(&label.text, DIM);
        let width = (text.len() * CELL_WIDTH) as f32;
        let left = match label.at {
            0 => x,
            at if at == last => x - width,
            _ => x - width / 2.0,
        }
        .max(area.x)
        .min(area.x + area.width - width);
        if left < end + CELL_WIDTH as f32 {
            continue;
        }
        end = left + width;
        drawing
            .labels
            .push(Label::new(left, plot.bottom() + 2.0, text));
    }
    // Shaded regions and areas, behind everything drawn on them, sharing their quads.
    let base = 0.0_f32.clamp(range.0, range.1);
    let mut regions: Vec<(Bound, Bound, bool, Rgba)> = chart
        .shades
        .iter()
        .map(|shade| {
            (
                Bound::Edge(shade.from),
                Bound::Edge(shade.to),
                shade.above,
                ink(shade.colour, SHADE_ALPHA),
            )
        })
        .collect();
    for (index, series) in chart.series.iter().enumerate() {
        if series.style == SeriesStyle::Area {
            let edge = Bound::Edge(Edge::Series {
                index: index as u32,
            });
            regions.push((
                Bound::Level(base),
                edge,
                false,
                ink(series.colour, AREA_ALPHA),
            ));
        }
    }
    let share = FILL_QUADS / regions.len().max(1);
    for (from, to, above, color) in regions {
        for corners in plot.fill(from, to, above, share) {
            shapes.push(Shape::Quad { corners, color });
        }
    }
    // Bars, then reference lines, then lines, the first series in front.
    for series in &chart.series {
        if series.style == SeriesStyle::Bars {
            bars(&plot, series, base, shapes);
        }
    }
    for reference in &chart.references {
        if !within(reference.value) {
            continue;
        }
        let y = plot.y(reference.value);
        let color = ink(reference.colour, 1.0);
        let path = [([plot.left, y], [plot.left + plot.width, y])];
        let segments = if reference.dashed {
            dashes(
                &path,
                plot.left,
                (DASH.0 + DASH.1).max(plot.width / REFERENCE_DASHES as f32),
            )
        } else {
            path.to_vec()
        };
        for (from, to) in segments {
            shapes.push(line(from, to, REFERENCE_WIDTH, color));
        }
    }
    for (index, series) in chart.series.iter().enumerate().rev() {
        if series.style != SeriesStyle::Bars
            && let Some(runs) = series_line(&plot, series, shapes)
            && let Some(area) = drawing.plot.as_mut()
        {
            area.thinned.insert(0, (index, runs));
        }
    }
    // Its badge, at the plot's top right, on a backing of its tone.
    if let Some(Note {
        text, tone: kind, ..
    }) = &chart.badge
    {
        let color = tone(*kind);
        let label = cells(&format!(" {text} "), color);
        let width = (label.len() * CELL_WIDTH) as f32;
        let (x, y) = (plot.left + plot.width - width - 2.0, plot.top + 2.0);
        shapes.push(Shape::Rect {
            x,
            y,
            width,
            height: row,
            color: [0.02, 0.035, 0.06, 0.92],
            radius: 2.0,
            outline: None,
        });
        shapes.push(Shape::frame(x, y, [width, row], 1.0, color));
        drawing.labels.push(Label::new(x, y, label));
    }
    // With nothing measured, it says so in the middle of the plot, never an empty plot.
    if !chart.has_values() {
        let text = chart
            .status
            .as_ref()
            .map_or("No data".to_owned(), |status| {
                format!("No data: {}", status.text)
            });
        let label = cells(&text, tone(Tone::Warn));
        let width = (label.len() * CELL_WIDTH) as f32;
        let x = (plot.left + (plot.width - width) / 2.0).max(plot.left);
        drawing
            .labels
            .push(Label::new(x, plot.top + (plot.height - row) / 2.0, label));
    }
    drawing
}

/// The bars of `series`, from `base` to each value, held within the plot: one for each point, or, for more than
/// [`LINE_VERTICES`], one for each stretch of points, as tall as the one of them farthest from `base`.
fn bars(
    plot: &Plot<'_>,
    series: &tomori_protocol::chart::Series,
    base: f32,
    shapes: &mut Vec<Shape>,
) {
    let color = ink(series.colour, 1.0);
    let points = plot.chart.points as usize;
    let stretches = points.min(LINE_VERTICES);
    let pitch = plot.width / plot.chart.points.saturating_sub(1).max(1) as f32;
    let (low, high) = plot.range;
    let right = plot.left + plot.width;
    for stretch in 0..stretches {
        let part = stretch * points / stretches..(stretch + 1) * points / stretches;
        let Some(value) = series.values
            [part.start.min(series.values.len())..part.end.min(series.values.len())]
            .iter()
            .flatten()
            .copied()
            .max_by(|a, b| (a - base).abs().total_cmp(&(b - base).abs()))
        else {
            continue;
        };
        let middle = (plot.x(part.start) + plot.x(part.end - 1)) / 2.0;
        let width = (pitch * part.len() as f32 * BAR).max(1.0);
        let left = (middle - width / 2.0).max(plot.left);
        let across = (middle + width / 2.0).min(right) - left;
        let (y0, y1) = (plot.y(value.clamp(low, high)), plot.y(base));
        if y0 == y1 {
            continue;
        }
        shapes.push(Shape::Fill {
            x: left,
            y: y0.min(y1),
            width: across,
            height: (y1 - y0).abs(),
            color,
        });
    }
}

/// A part of a line within the plot: its ends, and whether each was cut at the plot's edge.
type Part = ([f32; 2], [f32; 2], bool, bool);

/// The line of `series`: each run of its points joined to the next, merged and thinned to [`LINE_VERTICES`], a step
/// series' by columns so that it keeps its square corners, clipped to the plot with a dot where it leaves it; dashed,
/// in a pattern laid across the plot by its width, the part of the line within each dash, not merged but, past
/// [`LINE_VERTICES`] points, thinned to each period's highest and lowest; or whole, its parts overlapping where they
/// meet, or, where they turn by more than a right
/// angle, rounded with a dot; and a dot for each lone point.
fn series_line(
    plot: &Plot<'_>,
    series: &tomori_protocol::chart::Series,
    shapes: &mut Vec<Shape>,
) -> Option<Vec<Vec<[f32; 2]>>> {
    let color = ink(series.colour, 1.0);
    let width = series.width.unwrap_or(LINE_WIDTH);
    let stepped = series.style == SeriesStyle::Step;
    // Each run of points joined to the next: a gap, or a break after a point, ends one.
    let mut runs: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut run: Vec<[f32; 2]> = Vec::new();
    for (index, value) in series.values.iter().enumerate() {
        match value {
            Some(value) => {
                run.push([plot.x(index), plot.y(*value)]);
                if series.breaks.contains(&(index as u32)) {
                    runs.push(std::mem::take(&mut run));
                }
            }
            None => runs.push(std::mem::take(&mut run)),
        }
    }
    runs.push(run);
    runs.retain(|run| !run.is_empty());
    // Points nearly in line merged away, but for a dashed line's, so that its pattern, which runs along it, depends on
    // its points alone: merging weighs every point, and one added at the end could move dashes anywhere along it.
    let drawn = |run: &[[f32; 2]]| {
        let run = if stepped { steps(run) } else { run.to_vec() };
        if series.dashed {
            run
        } else {
            simplify(&run, SIMPLIFY)
        }
    };
    let mut lines: Vec<Vec<[f32; 2]>> = runs.iter().map(|run| drawn(run)).collect();
    let total: usize = lines.iter().map(Vec::len).sum();
    let period = dash_period(plot.width);
    if series.dashed && !stepped {
        // Dashed, past [`LINE_VERTICES`] thinned to each period of its dashes' highest and lowest points, so that a
        // point added at its end thins only its end, and a line however steep and noisy is a few parts in each dash.
        if total > LINE_VERTICES {
            lines = runs
                .iter()
                .map(|run| gridded(run, plot.left, period))
                .collect();
        }
    } else if total > LINE_VERTICES {
        for (line, run) in lines.iter_mut().zip(&runs) {
            let most = (line.len() * LINE_VERTICES / total).max(2);
            *line = if stepped {
                simplify(&columns(run, most), SIMPLIFY)
            } else {
                thin(line, most)
            };
        }
    }
    let thinned = (total > LINE_VERTICES && !stepped).then(|| lines.clone());
    let (top, bottom) = (plot.top, plot.bottom());
    let inside = |[_, y]: [f32; 2]| y >= top && y <= bottom;
    let mut markers = 0;
    let mut marker = |shapes: &mut Vec<Shape>, centre: [f32; 2]| {
        if markers < MARKERS {
            markers += 1;
            shapes.push(Shape::Dot {
                centre,
                radius: width.max(1.5),
                color,
            });
        }
    };
    // Clipped to the plot, each run's parts, with whether each end was cut there.
    let mut paths: Vec<Vec<Part>> = Vec::new();
    for line in lines {
        if let [point] = line[..] {
            if inside(point) {
                shapes.push(Shape::Dot {
                    centre: point,
                    radius: width.max(1.5),
                    color,
                });
            }
            continue;
        }
        let mut path = Vec::new();
        for pair in line.windows(2) {
            if let Some((from, to, cut_from, cut_to)) = clip(pair[0], pair[1], top, bottom) {
                for (cut, at) in [(cut_from, from), (cut_to, to)] {
                    if cut {
                        marker(shapes, at);
                    }
                }
                path.push((from, to, cut_from, cut_to));
            }
        }
        paths.push(path);
    }
    // Dashed, the pattern laid across the plot, so that the same points draw the same dashes from one update to the
    // next, a point added at its end changes only its end, and it is dashed all along however noisy it is.
    if series.dashed {
        let whole: Vec<_> = paths
            .iter()
            .flatten()
            .map(|(from, to, ..)| (*from, *to))
            .collect();
        for (from, to) in dashes(&whole, plot.left, period) {
            shapes.push(Shape::Stroke {
                from,
                to,
                width,
                color,
            });
        }
        return thinned;
    }
    let mut joints = 0;
    for path in paths {
        // Each part's ends overlap its neighbours' where they meet, by half the line's width, where it turns by a right
        // angle or less; past that, the ends meet at the joint, rounded with a dot.
        let direction = |&(from, to, ..): &Part| {
            let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
            let length = dx.hypot(dy).max(f32::EPSILON);
            [dx / length, dy / length]
        };
        let joined = |index: usize| {
            let (here, next) = (&path[index], &path[index + 1]);
            if here.3 || next.2 {
                return None;
            }
            let ([ax, ay], [bx, by]) = (direction(here), direction(next));
            Some(ax * bx + ay * by >= 0.0)
        };
        let count = path.len();
        for (index, part) in path.iter().enumerate() {
            let (from, to, ..) = *part;
            let [dx, dy] = direction(part);
            let back = index > 0 && joined(index - 1) == Some(true);
            let ahead = index + 1 < count && joined(index) == Some(true);
            let reach = |extended: bool| if extended { width / 2.0 } else { 0.0 };
            let (back, ahead) = (reach(back), reach(ahead));
            shapes.push(Shape::Stroke {
                from: [from[0] - dx * back, from[1] - dy * back],
                to: [to[0] + dx * ahead, to[1] + dy * ahead],
                width,
                color,
            });
            if index + 1 < count && joined(index) == Some(false) && joints < JOINTS {
                joints += 1;
                shapes.push(Shape::Dot {
                    centre: to,
                    radius: width / 2.0,
                    color,
                });
            }
        }
    }
    thinned
}

/// The points that draw a step series through `points`: level from each to the next, then upright to it.
fn steps(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut drawn: Vec<[f32; 2]> = Vec::with_capacity(2 * points.len());
    for point in points {
        if let Some(&[_, level]) = drawn.last() {
            drawn.push([point[0], level]);
        }
        drawn.push(*point);
    }
    drawn
}

/// `points` thinned by columns of the plot `width` font pixels wide, from `left`: its ends, and, in each column, its
/// highest and lowest point, in order, so that a spike is never thinned away, and which points are kept in one column
/// depends on its points alone.
fn gridded(points: &[[f32; 2]], left: f32, width: f32) -> Vec<[f32; 2]> {
    let (&first, rest) = points.split_first().expect("a run has a point");
    let mut kept = vec![first];
    let column = |[x, _]: [f32; 2]| ((x - left) / width).floor() as i64;
    let inner = &rest[..rest.len().saturating_sub(1)];
    for part in inner.chunk_by(|a, b| column(*a) == column(*b)) {
        let by_height = |a: &&[f32; 2], b: &&[f32; 2]| a[1].total_cmp(&b[1]);
        let (top, bottom) = (
            part.iter().min_by(by_height).expect("a column has points"),
            part.iter().max_by(by_height).expect("a column has points"),
        );
        let index = |point: &[f32; 2]| part.iter().position(|other| std::ptr::eq(other, point));
        let (one, two) = if index(top) <= index(bottom) {
            (top, bottom)
        } else {
            (bottom, top)
        };
        kept.push(*one);
        if !std::ptr::eq(one, two) {
            kept.push(*two);
        }
    }
    if let Some(&last) = rest.last() {
        kept.push(last);
    }
    kept
}

/// The points that draw a step series through `points` thinned to about `most`, by columns: for each of `most / 4`
/// stretches of its points, upright at the stretch's first point from where the line was to the stretch's highest and
/// lowest, then to where it ends, and level to the next stretch, so that its corners stay square and no spike is
/// thinned away.
fn columns(points: &[[f32; 2]], most: usize) -> Vec<[f32; 2]> {
    let stretches = (most / 4).max(1).min(points.len());
    let mut drawn: Vec<[f32; 2]> = Vec::new();
    for stretch in 0..stretches {
        let part =
            &points[stretch * points.len() / stretches..(stretch + 1) * points.len() / stretches];
        let x = part[0][0];
        let entry = drawn.last().map_or(part[0][1], |&[_, level]| level);
        let (least, most) = part
            .iter()
            .fold((f32::MAX, f32::MIN), |(least, most), [_, y]| {
                (least.min(*y), most.max(*y))
            });
        let exit = part[part.len() - 1][1];
        // The nearer extreme first, so that the column is drawn once up and once down at most.
        let (first, second) = if (entry - least).abs() <= (entry - most).abs() {
            (least, most)
        } else {
            (most, least)
        };
        drawn.extend([[x, entry], [x, first], [x, second], [x, exit]]);
    }
    let [x, _] = points[points.len() - 1];
    let exit = drawn.last().map_or(0.0, |&[_, level]| level);
    drawn.push([x, exit]);
    drawn.dedup();
    drawn
}

/// The legend of `chart`, in a row `columns` wide: each series' key, a short line in its colour, its name and its latest
/// value, `--` where its last point has none, or `no data` where it has none at all, dim; then each reference line's
/// label and value; each whole, as many as fit, the series first, since a reference line's value is labelled beside the
/// plot too. It shows `chart` as [`bounded`] holds it.
pub fn legend(chart: &Chart, columns: usize) -> Vec<Cell> {
    let chart = &bounded(chart);
    let mut items: Vec<Vec<Span>> = Vec::new();
    for series in &chart.series {
        let color = ink(series.colour, 1.0);
        let (text, color) = match series.latest() {
            Some(value) => (format!(" {} {}", series.name, brief(value)), color),
            None if series.values.iter().any(Option::is_some) => {
                (format!(" {} --", series.name), color)
            }
            None => (format!(" {} no data", series.name), DIM),
        };
        // Its key a short line, or, for a dashed series, two dashes, so that the legend always says its style.
        let key = Style::plain(ink(series.colour, 1.0));
        let mut item = vec![crate::shapes::Mark::Rule.span(1, key)];
        item.push(if series.dashed {
            Span::new(" ", key)
        } else {
            crate::shapes::Mark::Rule.span(1, key)
        });
        if series.dashed {
            item.push(crate::shapes::Mark::Rule.span(1, key));
        }
        item.push(Span::new(text, Style::plain(color)));
        items.push(item);
    }
    for reference in &chart.references {
        let color = ink(reference.colour, 1.0);
        items.push(vec![Span::new(
            format!("{} {}", reference.label, brief(reference.value)),
            Style::plain(color),
        )]);
    }
    let mut cells: Vec<Cell> = Vec::new();
    for item in items {
        let item = cut(&item, usize::MAX);
        let gap = if cells.is_empty() { 0 } else { 2 };
        if cells.len() + gap + item.len() > columns {
            // The first item is shown, cut, rather than none.
            if cells.is_empty() {
                cells = cut(
                    &[Span::new(crate::ui_text::plain(&item), item[0].1)],
                    columns,
                );
            }
            break;
        }
        cells.extend(std::iter::repeat_n((' ', Style::plain(DIM)), gap));
        cells.extend(item);
    }
    cells
}

/// Charts in the states the tests and offscreen renders need: invented data, of the shapes an operations console
/// shows.
#[cfg(any(test, feature = "testing"))]
pub mod fixtures {
    use tomori_protocol::chart::{
        AxisLabel, Chart, Colour, Edge, Note, Reference, Series, SeriesStyle, Shade, Tone,
        ValueAxis,
    };

    /// 15 minutes at 5 seconds.
    pub const POINTS: u32 = 181;

    /// The time axis's labels: every five minutes, "now" at the newest.
    pub fn labels() -> Vec<AxisLabel> {
        vec![
            AxisLabel::new(0, "-15 min"),
            AxisLabel::new(60, "-10 min"),
            AxisLabel::new(120, "-5 min"),
            AxisLabel::new(180, "now"),
        ]
    }

    /// Scaling pressure: a teal mean below the 1.0 target, a muted amber greatest line rising above it now and then,
    /// the area above the target shaded, and a gap where polls failed.
    pub fn pressure() -> Chart {
        let at = |index: usize| index as f32 / 9.0;
        let gap = |index: usize| (58..70).contains(&index);
        let mean = (0..POINTS as usize)
            .map(|index| (!gap(index)).then(|| 0.68 + 0.12 * at(index).sin()))
            .collect();
        let greatest = (0..POINTS as usize)
            .map(|index| {
                (!gap(index))
                    .then(|| 0.95 + 0.45 * (at(index) / 2.3).sin() + 0.12 * at(index).cos())
            })
            .collect();
        Chart::new(POINTS)
            .with_labels(labels())
            .with_axis(ValueAxis::new(Some(0.0), None, ""))
            .with_series(
                Series::new("mean", Colour::Teal, SeriesStyle::Line, mean).with_width(1.75),
            )
            .with_series(
                Series::new("greatest", Colour::Amber, SeriesStyle::Line, greatest)
                    .with_width(1.25),
            )
            .with_reference(Reference::new(1.0, "target", Colour::White))
            .with_shade(Shade::new(
                Edge::Reference { index: 0 },
                Edge::Series { index: 1 },
                true,
                Colour::Red,
            ))
            .with_status(Note::new("7 of 8 reporting", Tone::Neutral))
    }

    /// Provisioning: desired, created and ready replicas as steps, the gap between ready and desired shaded, and a
    /// shortfall badge.
    pub fn provisioning() -> Chart {
        let step = |index: usize, levels: &[(usize, f32)]| {
            levels
                .iter()
                .rev()
                .find(|(from, _)| index >= *from)
                .map(|(_, level)| *level)
        };
        let series = |levels: &[(usize, f32)]| -> Vec<Option<f32>> {
            (0..POINTS as usize)
                .map(|index| step(index, levels))
                .collect()
        };
        Chart::new(POINTS)
            .with_labels(labels())
            .with_axis(ValueAxis::new(Some(0.0), None, "replicas"))
            .with_series(Series::new(
                "desired",
                Colour::Blue,
                SeriesStyle::Step,
                series(&[(0, 6.0), (70, 8.0), (140, 9.0)]),
            ))
            .with_series(Series::new(
                "created",
                Colour::Violet,
                SeriesStyle::Step,
                series(&[(0, 6.0), (80, 8.0), (150, 9.0)]),
            ))
            .with_series(Series::new(
                "ready",
                Colour::Green,
                SeriesStyle::Step,
                series(&[(0, 6.0), (95, 7.0), (110, 8.0), (165, 6.0)]),
            ))
            .with_shade(Shade::new(
                Edge::Series { index: 2 },
                Edge::Series { index: 0 },
                true,
                Colour::Amber,
            ))
            .with_badge(Note::new("SHORT 3", Tone::Warn))
    }

    /// Admissions a minute: an area of bars' worth of values, its line breaking where its count of reporters changes.
    pub fn admissions() -> Chart {
        let values = (0..POINTS as usize)
            .map(|index| Some(2600.0 + 400.0 * (index as f32 / 13.0).sin()))
            .collect();
        Chart::new(POINTS)
            .with_labels(labels())
            .with_series(
                Series::new("admitted", Colour::Blue, SeriesStyle::Area, values)
                    .with_breaks(vec![90]),
            )
            .with_axis(ValueAxis::new(None, None, "a minute"))
    }

    /// Nothing measured: a chart that says so.
    pub fn unmeasured() -> Chart {
        Chart::new(POINTS)
            .with_labels(labels())
            .with_series(Series::new(
                "mean",
                Colour::Teal,
                SeriesStyle::Line,
                vec![None; POINTS as usize],
            ))
            .with_reference(Reference::new(1.0, "target", Colour::White))
            .with_status(Note::new("not measured", Tone::Warn))
    }

    /// Stale: pressure whose greatest line is entirely missing and whose newest points are gaps, saying how stale.
    pub fn stale() -> Chart {
        let mut chart = pressure();
        chart.series[1].values = vec![None; POINTS as usize];
        for value in &mut chart.series[0].values[157..] {
            *value = None;
        }
        chart.status = Some(Note::new("stale 2 min", Tone::Warn));
        chart.legend = true;
        chart
    }

    /// A chart at every bound of the spec, as many pieces as one can be drawn with: four dashed series of 256 points
    /// swinging from one end of the range to the other at every point, two lines, a step and an area, four shades between
    /// them that cross at every step, four dashed reference lines, and every label, name, badge and status at its
    /// longest. Without `gaps`, every swing passes beyond both fixed ends of the value axis, so that every segment is
    /// clipped; with them, every third point of each series is missing, so that its line is in many short runs.
    pub fn largest(gaps: bool) -> Chart {
        let long = |count: usize| "W".repeat(count);
        let mut chart = Chart::new(super::CHART_POINTS)
            .with_axis(ValueAxis::new(Some(0.0), Some(100.0), long(32)))
            .with_badge(Note::new(long(32), Tone::Bad))
            .with_status(Note::new(long(80), Tone::Warn))
            .with_labels(
                (0..8)
                    .map(|index| AxisLabel::new(index * 36, long(32)))
                    .collect(),
            );
        let styles = [
            SeriesStyle::Line,
            SeriesStyle::Step,
            SeriesStyle::Area,
            SeriesStyle::Line,
        ];
        for (index, style) in styles.into_iter().enumerate() {
            let values = (0..super::CHART_POINTS as usize)
                .map(|point| {
                    let high = (point + index) % 2 == 0;
                    let (top, bottom) = if gaps { (100.0, 0.0) } else { (150.0, -50.0) };
                    (!(gaps && point % 3 == 2)).then_some(if high { top } else { bottom })
                })
                .collect();
            chart = chart
                .with_series(
                    Series::new(long(32), Colour::Teal, style, values)
                        .with_width(4.0)
                        .with_dashed(true),
                )
                .with_reference(
                    Reference::new(20.0 * index as f32 + 10.0, long(32), Colour::White)
                        .with_dashed(true),
                )
                .with_shade(Shade::new(
                    Edge::Series {
                        index: index as u32,
                    },
                    Edge::Series {
                        index: (index as u32 + 1) % 4,
                    },
                    false,
                    Colour::Red,
                ));
        }
        chart
    }

    /// Pressure swinging between 0.9 and 1.3 at every one of its points about a 1.0 target, shaded only above it, twice:
    /// a piece of shading for each swing above the target, and none where it is below.
    pub fn swinging() -> Chart {
        let mut chart = Chart::new(POINTS)
            .with_labels(labels())
            .with_axis(ValueAxis::new(Some(0.0), Some(2.0), ""))
            .with_series(Series::new(
                "greatest",
                Colour::Amber,
                SeriesStyle::Line,
                (0..POINTS)
                    .map(|index| Some(if index % 2 == 0 { 0.9 } else { 1.3 }))
                    .collect(),
            ))
            .with_reference(Reference::new(1.0, "target", Colour::White));
        for _ in 0..2 {
            chart = chart.with_shade(Shade::new(
                Edge::Reference { index: 0 },
                Edge::Series { index: 0 },
                true,
                Colour::Red,
            ));
        }
        chart
    }

    /// Replicas stepping between many levels at almost every point, more steps than its line may have vertices, so
    /// that it is thinned by columns, its gap to a target shaded.
    pub fn stepping() -> Chart {
        Chart::new(POINTS)
            .with_labels(labels())
            .with_axis(ValueAxis::new(Some(0.0), Some(10.0), "replicas"))
            .with_series(Series::new(
                "ready",
                Colour::Green,
                SeriesStyle::Step,
                (0..super::CHART_POINTS)
                    .map(|index| Some(((index * 7919) % 9 + 1) as f32))
                    .collect(),
            ))
            .with_reference(Reference::new(6.0, "desired", Colour::Blue))
            .with_shade(Shade::new(
                Edge::Series { index: 0 },
                Edge::Reference { index: 0 },
                true,
                Colour::Amber,
            ))
    }

    /// Ready and desired replicas swinging together between 0.2 and 1.8 at every one of its points, desired 0.1 above
    /// ready, their gap shaded twice, and a dashed limit across, so that the band shaded is narrow however they swing.
    pub fn tracking() -> Chart {
        let ready: Vec<Option<f32>> = (0..POINTS)
            .map(|index| Some(if index % 2 == 0 { 0.2 } else { 1.8 }))
            .collect();
        let desired = ready
            .iter()
            .map(|value| value.map(|value| value + 0.1))
            .collect();
        let mut chart = Chart::new(POINTS)
            .with_labels(labels())
            .with_axis(ValueAxis::new(Some(0.0), Some(2.0), "replicas"))
            .with_series(Series::new(
                "ready",
                Colour::Green,
                SeriesStyle::Line,
                ready,
            ))
            .with_series(Series::new(
                "desired",
                Colour::Blue,
                SeriesStyle::Line,
                desired,
            ))
            .with_series(
                Series::new(
                    "limit",
                    Colour::Amber,
                    SeriesStyle::Line,
                    vec![Some(0.6); POINTS as usize],
                )
                .with_dashed(true),
            );
        for colour in [Colour::Amber, Colour::Red] {
            chart = chart.with_shade(Shade::new(
                Edge::Series { index: 0 },
                Edge::Series { index: 1 },
                true,
                colour,
            ));
        }
        chart
    }

    /// A few points, some joined, one alone, and dashed and stepped series and bars, for the shapes each draws.
    pub fn sampler() -> Chart {
        Chart::new(6)
            .with_series(
                Series::new(
                    "line",
                    Colour::Teal,
                    SeriesStyle::Line,
                    vec![Some(1.0), Some(2.0), None, Some(3.0), None, Some(1.0)],
                )
                .with_breaks(vec![0]),
            )
            .with_series(
                Series::new(
                    "dashed",
                    Colour::Amber,
                    SeriesStyle::Unknown,
                    vec![
                        Some(0.5),
                        Some(0.5),
                        Some(2.5),
                        Some(2.5),
                        Some(0.5),
                        Some(0.5),
                    ],
                )
                .with_dashed(true),
            )
            .with_series(Series::new(
                "bars",
                Colour::Rgb([200, 200, 200]),
                SeriesStyle::Bars,
                vec![Some(1.0), None, Some(-1.0), Some(2.0), Some(0.0), Some(1.5)],
            ))
    }
}

#[cfg(test)]
mod tests;
