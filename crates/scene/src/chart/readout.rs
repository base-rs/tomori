// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A chart's readout, while the ray or the mouse points at its plot: a thin line down the plot at the time pointed at,
//! a dot on each series at its value there, and a small label beside the line saying the time, as the axis says it,
//! and each series' name and value in its unit, in its colour, or that it has no data there, kept inside the panel. It
//! is drawn each frame as an overlay of its own over the chart's face, from where the face's plot is and the chart's
//! values, so that the chart is never laid out again as the ray scans it: a line, a dot or two for each series, a
//! backing and at most [`LINES`] lines of at most [`READOUT_CHARS`], the series that do not fit said as how many more,
//! from glyphs the atlas holds. Each participant sees their own. See `design/projection.md`.
use tomori_protocol::chart::{CHART_SERIES, Chart};

use super::{DIM, PlotArea, ink};
use crate::canvas::{Canvas, Label};
use crate::shapes::Shape;
use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH, Rgba, Row, Span, Style, cut};

/// The most characters of each line of a readout's label, its time or a series' name and value.
pub const READOUT_CHARS: usize = 40;
/// The most lines of a readout's label: its time, and one for each series.
pub const LINES: usize = 1 + CHART_SERIES;

/// The colour of the line down the plot, and of the label's time.
const CROSSHAIR: Rgba = [0.92, 0.93, 0.95, 0.8];
const TIME: Rgba = [0.92, 0.93, 0.95, 1.0];
/// The label's backing and frame, and the ring about each dot that keeps it clear of the lines under it.
const BACKING: Rgba = [0.02, 0.035, 0.06, 0.94];
const FRAME: Rgba = [0.55, 0.62, 0.7, 0.5];
/// How much fainter a linked readout's line and frame are than those of the chart pointed at.
const LINKED: f32 = 0.55;
/// How far the label stands from the line, and the space inside its frame, in font pixels.
const GAP: f32 = 6.0;
const INSET: (f32, f32) = (3.0, 2.0);

/// What a readout shows: where its line is across the plot, and from top to bottom; each series' dot, where it has a
/// value; and its label's lines, each in its colour, and where its backing is, its left and top edges, its width and
/// its height, all in font pixels from the face's top-left corner inside its padding.
#[derive(Clone, Debug, PartialEq)]
pub struct Readout {
    pub x: f32,
    pub span: (f32, f32),
    pub dots: Vec<([f32; 2], Rgba)>,
    pub lines: Vec<(String, Rgba)>,
    pub label: [f32; 4],
    /// Whether it is linked to the readout of another chart pointed at, sharing its time, rather than where the ray
    /// meets this one: drawn fainter, so that the chart pointed at stands out.
    pub linked: bool,
}

impl Readout {
    /// `chart`'s readout at `at`, a point's index or a share of the way between two, on its plot drawn at `plot`, on a
    /// face `bounds` font pixels wide and tall inside its padding: its line at `at`, a dot on each series where it has
    /// a value as it is drawn, a thinned line's on the line drawn, held within the plot, and its label to the right of
    /// the line, or left of it where there is no room, at the plot's top or its bottom, whichever covers less of the
    /// series about it, held inside the face, its lines cut to fit across it.
    pub fn new(chart: &Chart, plot: &PlotArea, at: f32, (width, height): (f32, f32)) -> Self {
        let x = plot.x(at);
        let mut dots = Vec::new();
        let mut lines = vec![(chart.time_at(at), TIME)];
        let read = |index: usize, x: f32, at: f32| match plot
            .thinned
            .iter()
            .find(|(thinned, _)| *thinned == index)
        {
            Some((_, runs)) => plot
                .drawn(runs, x)
                .and_then(tomori_protocol::chart::rounded),
            None if chart.series[index].style == tomori_protocol::chart::SeriesStyle::Bars => {
                super::bar_at(chart, index, at, plot.stretches)
            }
            None => chart.value_at(index, at),
        };
        for (index, series) in chart.series.iter().take(CHART_SERIES).enumerate() {
            let color = ink(series.colour, 1.0);
            let value = read(index, x, at);
            if let Some(value) = value {
                dots.push(([x, plot.y(value)], color));
            }
            lines.push((format!("{} {}", series.name, chart.amount(value)), color));
        }
        // As many lines as fit down the face, each as many characters as fit across it, the series that do not fit
        // said in the last as how many more.
        let rows = ((height - 2.0 * INSET.1) / CELL_HEIGHT as f32).max(1.0) as usize;
        let most = rows.min(LINES);
        if lines.len() > most {
            let shown = most.saturating_sub(2);
            let more = lines.len() - 1 - shown;
            lines.truncate(1 + shown);
            if most >= 2 {
                lines.push((format!("+{more} more"), DIM));
            }
        }
        let most =
            (((width - 2.0 * INSET.0) / CELL_WIDTH as f32).max(1.0) as usize).min(READOUT_CHARS);
        let cells: Vec<usize> = lines
            .iter_mut()
            .map(|(text, color)| {
                let cells = cut(&[Span::new(text.as_str(), Style::plain(*color))], most);
                *text = cells.iter().map(|cell| cell.0).collect();
                cells.len()
            })
            .collect();
        let across =
            cells.into_iter().max().unwrap_or_default() as f32 * CELL_WIDTH as f32 + 2.0 * INSET.0;
        let down = lines.len() as f32 * CELL_HEIGHT as f32 + 2.0 * INSET.1;
        let right = x + GAP;
        let left = if right + across <= width {
            right
        } else {
            x - GAP - across
        };
        let left = left.min(width - across).max(0.0);
        // At the plot's top or bottom, whichever covers fewer of the series' values under it.
        let under = |top: f32| {
            let top = top.min(height - down).max(0.0);
            let covered = (0..=8)
                .map(|step| left + across * step as f32 / 8.0)
                .filter_map(|x| plot.at([x, plot.top]).map(|at| (x, at)))
                .flat_map(|(x, at)| {
                    (0..chart.series.len().min(CHART_SERIES))
                        .filter_map(move |index| read(index, x, at))
                })
                .filter(|value| (top - 2.0..=top + down + 2.0).contains(&plot.y(*value)))
                .count();
            (covered, top)
        };
        let (high, low) = (
            under(plot.top + 2.0),
            under(plot.top + plot.height - down - 2.0),
        );
        let top = if low.0 < high.0 { low.1 } else { high.1 };
        Self {
            x,
            span: (plot.top, plot.top + plot.height),
            dots,
            lines,
            label: [left, top, across, down],
            linked: false,
        }
    }

    /// It linked to the readout of another chart pointed at, at the same time: its line and frame fainter.
    pub fn linked(self) -> Self {
        Self {
            linked: true,
            ..self
        }
    }

    /// It drawn over `face`, the chart's face: a canvas as large as the face, with nothing on it but the readout, so
    /// that drawn in the face's plane, over it, each part stands where it reads.
    pub fn overlay(&self, face: &Canvas) -> Canvas {
        let mut canvas = Canvas::new(face.columns, face.pixel);
        canvas.padding = face.padding;
        canvas.background = [0.0; 4];
        canvas.rows = vec![Row::default(); face.rows.len()];
        let (top, bottom) = self.span;
        let faint = |[red, green, blue, alpha]: Rgba| {
            [
                red,
                green,
                blue,
                if self.linked { alpha * LINKED } else { alpha },
            ]
        };
        canvas.shapes.push(Shape::Stroke {
            from: [self.x, top],
            to: [self.x, bottom],
            width: 1.0,
            color: faint(CROSSHAIR),
        });
        for (centre, color) in &self.dots {
            canvas.shapes.push(Shape::Dot {
                centre: *centre,
                radius: 3.5,
                color: BACKING,
            });
            canvas.shapes.push(Shape::Dot {
                centre: *centre,
                radius: 2.5,
                color: *color,
            });
        }
        let [left, top, across, down] = self.label;
        canvas.shapes.push(Shape::Rect {
            x: left,
            y: top,
            width: across,
            height: down,
            color: BACKING,
            radius: 2.0,
            outline: None,
        });
        canvas
            .shapes
            .push(Shape::frame(left, top, [across, down], 1.0, faint(FRAME)));
        for (row, (text, color)) in self.lines.iter().enumerate() {
            canvas.labels.push(Label::new(
                left + INSET.0,
                top + INSET.1 + (row * CELL_HEIGHT) as f32,
                cut(
                    &[Span::new(text.as_str(), Style::plain(*color))],
                    usize::MAX,
                ),
            ));
        }
        canvas
    }
}

#[cfg(test)]
mod tests;
