// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A chart panel's native chart: a small, bounded spec of up to [`CHART_SERIES`] series of values at evenly spaced
//! points along one time axis, built for live data: a fixed step, a gap where a value is missing, never a zero, and
//! everything bounded, so that a client draws it without trusting it. An environment's server side publishes one in a
//! panel (see `design/projection.md` and `design/environments.md`); the agent's `show_chart` (issue #82) is to reuse it.
//!
//! The server [cleans](Chart::cleaned) every chart before anyone is sent it, and a client draws only what is valid,
//! leaving out an edge, a label or a shade that names nothing, so that a chart from a newer server, or one cleaned by an
//! older one, still draws what it can.
use serde::{Deserialize, Serialize};

use crate::text::one_line;

/// The most points along a chart's axis: 15 minutes at 5 seconds is 181.
pub const CHART_POINTS: u32 = 256;
/// The most series of a chart.
pub const CHART_SERIES: usize = 4;
/// The most reference lines of a chart, such as a target.
pub const CHART_REFERENCES: usize = 4;
/// The most shaded regions of a chart.
pub const CHART_SHADES: usize = 4;
/// The most labels along a chart's axis.
pub const CHART_LABELS: usize = 8;
/// The most characters of a chart's names, labels, unit and badge, each on one line, before the "…" that cuts them.
pub const CHART_NAME_CHARS: usize = 32;
/// The most characters of a chart's status line.
pub const CHART_STATUS_CHARS: usize = 80;
/// The narrowest and widest a series' line is, in font pixels; a line says nothing of its width is
/// [`LINE_WIDTH`].
pub const LINE_WIDTHS: (f32, f32) = (0.5, 4.0);
/// How wide a series' line is unless it says otherwise, in font pixels.
pub const LINE_WIDTH: f32 = 1.5;
/// How many significant figures a value keeps once cleaned, so that a chart of every point is a few kilobytes.
pub const SIGNIFICANT: i32 = 4;
/// The largest value either side of zero a chart holds: one beyond it is held to it, so that every value, and the
/// distances between them that drawing it works out, are finite.
pub const CHART_MAGNITUDE: f32 = 1.0e15;

/// A colour of a chart's palette, or any other as RGB. On the wire, a name, such as `teal`, or `#rrggbb`; a name a
/// client does not know is [`Colour::Grey`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum Colour {
    Teal,
    Amber,
    Blue,
    Violet,
    Green,
    Red,
    #[default]
    Grey,
    White,
    Rgb([u8; 3]),
}

impl Colour {
    /// Every named colour, with its name.
    pub const NAMED: [(&'static str, Colour); 8] = [
        ("teal", Colour::Teal),
        ("amber", Colour::Amber),
        ("blue", Colour::Blue),
        ("violet", Colour::Violet),
        ("green", Colour::Green),
        ("red", Colour::Red),
        ("grey", Colour::Grey),
        ("white", Colour::White),
    ];

    /// Its red, green and blue, from 0 to 1, as every client draws it on a panel's dark face.
    pub fn rgb(self) -> [f32; 3] {
        match self {
            Self::Teal => [0.16, 0.74, 0.66],
            Self::Amber => [0.86, 0.62, 0.22],
            Self::Blue => [0.42, 0.6, 0.98],
            Self::Violet => [0.68, 0.52, 0.94],
            Self::Green => [0.3, 0.8, 0.44],
            Self::Red => [0.92, 0.36, 0.3],
            Self::Grey => [0.56, 0.6, 0.66],
            Self::White => [0.92, 0.93, 0.95],
            Self::Rgb(rgb) => rgb.map(|value| f32::from(value) / 255.0),
        }
    }
}

impl From<String> for Colour {
    fn from(text: String) -> Self {
        let text = text.trim().to_ascii_lowercase();
        if let Some(digits) = text.strip_prefix('#')
            && digits.len() == 6
            && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).unwrap_or(0);
            return Self::Rgb([channel(0), channel(2), channel(4)]);
        }
        Self::NAMED
            .iter()
            .find(|(name, _)| *name == text)
            .map_or(Self::Grey, |(_, colour)| *colour)
    }
}

impl From<Colour> for String {
    fn from(colour: Colour) -> Self {
        match colour {
            Colour::Rgb([red, green, blue]) => format!("#{red:02x}{green:02x}{blue:02x}"),
            named => Colour::NAMED
                .iter()
                .find(|(_, colour)| *colour == named)
                .map_or("grey", |(name, _)| name)
                .to_owned(),
        }
    }
}

/// How a series is drawn. `Unknown` stands for a style a newer server sends, which a client draws as a line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SeriesStyle {
    /// A line through its points.
    #[default]
    Line,
    /// Level from each point to the next, then up or down to it, as a count of replicas changes.
    Step,
    /// A line, with the area below it, down to the axis's lowest value, filled faintly.
    Area,
    /// A bar from the axis's lowest value to each point.
    Bars,
    #[serde(other)]
    Unknown,
}

/// How a note, or a row of a list panel ([`crate::ListRow::tone`]), reads: neutral, good, a warning or bad, named for
/// what it means rather than a colour. `Neutral` stands for a tone a newer server sends too.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Tone {
    Good,
    Warn,
    Bad,
    #[default]
    #[serde(other)]
    Neutral,
}

/// A short note a chart shows: its badge, such as `SHORT 3`, or its status line, such as `7 of 8 reporting` or `stale 2
/// min`, in a tone.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Note {
    pub text: String,
    #[serde(default)]
    pub tone: Tone,
}

impl Note {
    pub fn new(text: impl Into<String>, tone: Tone) -> Self {
        Self {
            text: text.into(),
            tone,
        }
    }
}

/// A label along the axis, under the point `at`, from 0, such as `-15 min` under the first and `now` under the last.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AxisLabel {
    pub at: u32,
    pub text: String,
}

impl AxisLabel {
    pub fn new(at: u32, text: impl Into<String>) -> Self {
        Self {
            at,
            text: text.into(),
        }
    }
}

/// The value axis: its lowest and highest values, each fixed or, where it is `None`, found from the values and
/// reference lines shown, and its unit, such as `%` or `replicas`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ValueAxis {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
}

impl ValueAxis {
    /// The axis from `min` to `max`, each found where it is `None`, in `unit`.
    pub fn new(min: Option<f32>, max: Option<f32>, unit: impl Into<String>) -> Self {
        Self {
            min,
            max,
            unit: unit.into(),
        }
    }
}

/// One series: its name, colour, style and width, whether it is dashed, its value at each point, oldest first, `None`
/// where there is none, a gap, where its line breaks, and the points after which its line breaks too, though both are
/// measured, as where what it counts changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Series {
    pub name: String,
    #[serde(default)]
    pub colour: Colour,
    #[serde(default)]
    pub style: SeriesStyle,
    /// In font pixels; [`LINE_WIDTH`] where it says nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dashed: bool,
    pub values: Vec<Option<f32>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub breaks: Vec<u32>,
}

impl Series {
    /// The series `name`, drawn in `colour` as `style`, of `values`.
    pub fn new(
        name: impl Into<String>,
        colour: Colour,
        style: SeriesStyle,
        values: Vec<Option<f32>>,
    ) -> Self {
        Self {
            name: name.into(),
            colour,
            style,
            width: None,
            dashed: false,
            values,
            breaks: Vec::new(),
        }
    }

    /// The series, its line `width` font pixels wide.
    pub fn with_width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// The series, dashed or not.
    pub fn with_dashed(mut self, dashed: bool) -> Self {
        self.dashed = dashed;
        self
    }

    /// The series, its line broken after each of `breaks`.
    pub fn with_breaks(mut self, breaks: Vec<u32>) -> Self {
        self.breaks = breaks;
        self
    }

    /// Its latest value, if its last point has one.
    pub fn latest(&self) -> Option<f32> {
        self.values.last().copied().flatten()
    }
}

/// A level across the chart, such as a target of 1.0, with its label, in its colour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Reference {
    pub value: f32,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub colour: Colour,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dashed: bool,
}

impl Reference {
    pub fn new(value: f32, label: impl Into<String>, colour: Colour) -> Self {
        Self {
            value,
            label: label.into(),
            colour,
            dashed: false,
        }
    }

    /// The reference line, dashed or not.
    pub fn with_dashed(mut self, dashed: bool) -> Self {
        self.dashed = dashed;
        self
    }
}

/// One edge of a shaded region: a series or a reference line, by its index. `Unknown` stands for an edge a newer server
/// sends, which shades nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Edge {
    Series {
        index: u32,
    },
    Reference {
        index: u32,
    },
    #[serde(other)]
    Unknown,
}

/// A region shaded between two edges, where both have values, in a colour drawn faintly; with `above`, only where
/// `to` is above `from`, such as the pressure above its target, or the replicas wanted above those ready.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Shade {
    pub from: Edge,
    pub to: Edge,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub above: bool,
    #[serde(default)]
    pub colour: Colour,
}

impl Shade {
    pub fn new(from: Edge, to: Edge, above: bool, colour: Colour) -> Self {
        Self {
            from,
            to,
            above,
            colour,
        }
    }
}

fn shown() -> bool {
    true
}

fn is_shown(legend: &bool) -> bool {
    *legend
}

/// A chart: `points` evenly spaced points along one time axis, labelled along it, a value axis, its series, reference
/// lines and shaded regions, whether it shows a legend, and a badge and a status line. A chart with no value at all
/// shows that it has none, with its status line if it has one, and never an empty plot that could be read as quiet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Chart {
    pub points: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<AxisLabel>,
    #[serde(default)]
    pub axis: ValueAxis,
    #[serde(default)]
    pub series: Vec<Series>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<Reference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shades: Vec<Shade>,
    #[serde(default = "shown", skip_serializing_if = "is_shown")]
    pub legend: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<Note>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Note>,
}

/// `value` kept to [`SIGNIFICANT`] figures, so that its JSON is short; `None` where it is not finite.
/// One beyond [`CHART_MAGNITUDE`] either side of zero is held to it.
pub fn rounded(value: f32) -> Option<f32> {
    if !value.is_finite() {
        return None;
    }
    let value = value.clamp(-CHART_MAGNITUDE, CHART_MAGNITUDE);
    if value == 0.0 {
        return Some(0.0);
    }
    let magnitude = f64::from(value.abs()).log10().floor() as i32;
    let scale = 10f64.powi(SIGNIFICANT - 1 - magnitude);
    let kept = ((f64::from(value) * scale).round() / scale) as f32;
    kept.is_finite().then_some(kept)
}

impl Chart {
    /// A chart of `points` points, with nothing on it.
    pub fn new(points: u32) -> Self {
        Self {
            points,
            labels: Vec::new(),
            axis: ValueAxis::default(),
            series: Vec::new(),
            references: Vec::new(),
            shades: Vec::new(),
            legend: true,
            badge: None,
            status: None,
        }
    }

    /// The chart, labelled `labels` along its axis.
    pub fn with_labels(mut self, labels: Vec<AxisLabel>) -> Self {
        self.labels = labels;
        self
    }

    /// The chart, with `axis` for its values.
    pub fn with_axis(mut self, axis: ValueAxis) -> Self {
        self.axis = axis;
        self
    }

    /// The chart, with `series` added.
    pub fn with_series(mut self, series: Series) -> Self {
        self.series.push(series);
        self
    }

    /// The chart, with `reference` added.
    pub fn with_reference(mut self, reference: Reference) -> Self {
        self.references.push(reference);
        self
    }

    /// The chart, with `shade` added.
    pub fn with_shade(mut self, shade: Shade) -> Self {
        self.shades.push(shade);
        self
    }

    /// The chart, showing a legend or not.
    pub fn with_legend(mut self, legend: bool) -> Self {
        self.legend = legend;
        self
    }

    /// The chart, with `badge`.
    pub fn with_badge(mut self, badge: Note) -> Self {
        self.badge = Some(badge);
        self
    }

    /// The chart, with `status`.
    pub fn with_status(mut self, status: Note) -> Self {
        self.status = Some(status);
        self
    }

    /// Whether any series has a value at any point.
    pub fn has_values(&self) -> bool {
        self.series
            .iter()
            .any(|series| series.values.iter().any(Option::is_some))
    }

    /// Whether `edge` names a series or reference line of the chart.
    pub fn names(&self, edge: Edge) -> bool {
        match edge {
            Edge::Series { index } => (index as usize) < self.series.len(),
            Edge::Reference { index } => (index as usize) < self.references.len(),
            Edge::Unknown => false,
        }
    }

    /// The chart, bounded and cleaned as the server sends it, or why it cannot be one: from 2 to [`CHART_POINTS`]
    /// points; at most [`CHART_SERIES`] series, each of exactly `points` values, the newest last, those it lacks gaps
    /// before its first, those beyond dropped from the oldest, and every value finite, kept to [`SIGNIFICANT`] figures,
    /// or a gap; its breaks inside it, in order, once each; a width within [`LINE_WIDTHS`]; at most
    /// [`CHART_REFERENCES`] finite reference lines, [`CHART_SHADES`] shades whose edges name something, and
    /// [`CHART_LABELS`] labels at points it has; an axis whose fixed values are finite, the lowest below the highest;
    /// and every name, label, unit and note on one line, cut to [`CHART_NAME_CHARS`], or [`CHART_STATUS_CHARS`] for the
    /// status line.
    pub fn cleaned(mut self) -> Result<Self, &'static str> {
        if !(2..=CHART_POINTS).contains(&self.points) {
            return Err("A chart has from 2 to 256 points.");
        }
        let points = self.points as usize;
        let name = |text: &str| one_line(text, CHART_NAME_CHARS);
        self.series.truncate(CHART_SERIES);
        for series in &mut self.series {
            series.name = name(&series.name);
            let values = std::mem::take(&mut series.values);
            let skipped = values.len().saturating_sub(points);
            let missing = points.saturating_sub(values.len());
            series.values = std::iter::repeat_n(None, missing)
                .chain(
                    values
                        .into_iter()
                        .skip(skipped)
                        .map(|value| value.and_then(rounded)),
                )
                .collect();
            let shift = missing as i64 - skipped as i64;
            let mut breaks: Vec<u32> = series
                .breaks
                .iter()
                .filter_map(|at| u32::try_from(i64::from(*at) + shift).ok())
                .filter(|at| (*at as usize) + 1 < points)
                .collect();
            breaks.sort_unstable();
            breaks.dedup();
            series.breaks = breaks;
            series.width = series
                .width
                .filter(|width| width.is_finite())
                .map(|width| width.clamp(LINE_WIDTHS.0, LINE_WIDTHS.1));
        }
        self.references.truncate(CHART_REFERENCES);
        for reference in &mut self.references {
            reference.value =
                rounded(reference.value).ok_or("A chart's reference lines are finite.")?;
            reference.label = name(&reference.label);
        }
        let shades = std::mem::take(&mut self.shades);
        self.shades = shades
            .into_iter()
            .filter(|shade| {
                self.names(shade.from) && self.names(shade.to) && shade.from != shade.to
            })
            .take(CHART_SHADES)
            .collect();
        self.labels.retain(|label| label.at < self.points);
        self.labels.truncate(CHART_LABELS);
        for label in &mut self.labels {
            label.text = name(&label.text);
        }
        let fixed = |value: Option<f32>| value.and_then(rounded);
        self.axis.min = fixed(self.axis.min);
        self.axis.max = fixed(self.axis.max);
        if let (Some(min), Some(max)) = (self.axis.min, self.axis.max)
            && min >= max
        {
            return Err("A chart's lowest value is below its highest.");
        }
        self.axis.unit = name(&self.axis.unit);
        if let Some(badge) = &mut self.badge {
            badge.text = name(&badge.text);
        }
        if let Some(status) = &mut self.status {
            status.text = one_line(&status.text, CHART_STATUS_CHARS);
        }
        Ok(self)
    }

    /// Whether it and `other` are the same chart but for their live data, [`Chart::same_data`]: their points, labels,
    /// value axis, reference lines, shaded regions and legend, and their series' names, colours, styles, widths and
    /// dashes, in the same order. A panel's version covers these, and its data seq the rest. See
    /// `design/projection.md`.
    pub fn same_structure(&self, other: &Chart) -> bool {
        let Chart {
            points,
            labels,
            axis,
            series,
            references,
            shades,
            legend,
            badge: _,
            status: _,
        } = self;
        *points == other.points
            && *labels == other.labels
            && *axis == other.axis
            && *references == other.references
            && *shades == other.shades
            && *legend == other.legend
            && series.len() == other.series.len()
            && series.iter().zip(&other.series).all(|(a, b)| {
                let Series {
                    name,
                    colour,
                    style,
                    width,
                    dashed,
                    values: _,
                    breaks: _,
                } = a;
                *name == b.name
                    && *colour == b.colour
                    && *style == b.style
                    && *width == b.width
                    && *dashed == b.dashed
            })
    }

    /// Whether it and `other` show the same live data, as an environment's server side changes it from moment to
    /// moment: each series' values and breaks, in order, its badge and its status line.
    pub fn same_data(&self, other: &Chart) -> bool {
        self.badge == other.badge
            && self.status == other.status
            && self.series.len() == other.series.len()
            && self
                .series
                .iter()
                .zip(&other.series)
                .all(|(a, b)| a.values == b.values && a.breaks == b.breaks)
    }

    /// What the chart shows now, in plain text, for a reader that cannot draw it, as an older client, and for the
    /// agent: each series' latest value, or that it has none, each reference line, its badge and its status, a line
    /// each; or that it has no data.
    pub fn summary(&self) -> String {
        let unit = if self.axis.unit.is_empty() {
            String::new()
        } else {
            format!(" {}", self.axis.unit)
        };
        let mut lines = Vec::new();
        if let Some(status) = &self.status {
            lines.push(status.text.clone());
        }
        if !self.has_values() {
            lines.push("No data.".to_owned());
        }
        for series in &self.series {
            lines.push(match series.latest() {
                Some(value) => format!("{}: {value}{unit} now", series.name),
                None if series.values.iter().any(Option::is_some) => {
                    format!("{}: no value now", series.name)
                }
                None => format!("{}: no data", series.name),
            });
        }
        for reference in &self.references {
            let label = if reference.label.is_empty() {
                "reference"
            } else {
                &reference.label
            };
            lines.push(format!("{label}: {}{unit}", reference.value));
        }
        if let Some(badge) = &self.badge {
            lines.push(badge.text.clone());
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests;
