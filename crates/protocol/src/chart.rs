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
    /// What its points are in time, if it says: by which a client links its readout to another chart's at the same
    /// time, never by its labels, which are only shown. An older client ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<TimeAxis>,
}

/// The most milliseconds between a chart's points, a day.
pub const CHART_STEP_MS: u32 = 86_400_000;

/// The latest instant a chart's window reaches, in milliseconds since the Unix epoch: the last of the year 9999, UTC.
pub const CHART_INSTANT_MS: i64 = 253_402_300_799_999;

/// The earliest instant a chart's window reaches, in milliseconds since the Unix epoch: the first of the year 0, UTC.
pub const CHART_EARLIEST_MS: i64 = -62_167_219_200_000;

/// What a chart's points are in time: those of a live chart, whose last point is now and each `step_ms` milliseconds
/// after the one before, or those of a fixed window, whose first point is at `from_ms` and last at `to_ms`, absolute
/// instants in milliseconds since the Unix epoch, in UTC, the points evenly between. Two charts are linked at the same
/// time only by these, a live chart to another live chart by how long before now, a fixed window to another by the
/// instant, never by a time of day. `Unknown` stands for a kind a newer server sends, which links nothing. See the
/// linked time cursor in `design/projection.md`.
///
/// ```
/// use tomori_protocol::chart::TimeAxis;
///
/// assert_eq!(
///     serde_json::to_string(&TimeAxis::Live { step_ms: 5_000 }).unwrap(),
///     r#"{"kind":"live","step_ms":5000}"#
/// );
/// let newer: TimeAxis = serde_json::from_str(r#"{"kind": "sidereal"}"#).unwrap();
/// assert_eq!(newer, TimeAxis::Unknown);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TimeAxis {
    Live {
        step_ms: u32,
    },
    Fixed {
        from_ms: i64,
        to_ms: i64,
    },
    #[serde(other)]
    Unknown,
}

impl TimeAxis {
    /// It as a chart keeps it once cleaned: a live step from 1 ms to [`CHART_STEP_MS`], and a fixed window that ends
    /// after it begins, within the years 0 to 9999, from [`CHART_EARLIEST_MS`] to [`CHART_INSTANT_MS`], however few
    /// points it has, as a week's window of a few points is still that window; none otherwise.
    fn cleaned(self) -> Option<Self> {
        match self {
            Self::Live { step_ms } => (1..=CHART_STEP_MS).contains(&step_ms).then_some(self),
            Self::Fixed { from_ms, to_ms } => {
                (from_ms < to_ms && from_ms >= CHART_EARLIEST_MS && to_ms <= CHART_INSTANT_MS)
                    .then_some(self)
            }
            Self::Unknown => None,
        }
    }
}

/// A time on a chart's axis, as [`Chart::time_of`] finds it from its [`TimeAxis`]: so many milliseconds before now, on
/// a live chart, or an absolute instant, in milliseconds since the Unix epoch in UTC, on a fixed window.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ChartTime {
    BeforeNow { ms: f64 },
    Instant { utc_ms: f64 },
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
            time: None,
        }
    }

    /// The chart, its points in time as `time` says.
    pub fn with_time(mut self, time: TimeAxis) -> Self {
        self.time = Some(time);
        self
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
        self.time = self.time.and_then(TimeAxis::cleaned);
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
            time,
        } = self;
        *points == other.points
            && *time == other.time
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

/// How many parts a chart's time axis is read in, from its oldest point at 0 to its newest at `ACROSS`, where a
/// participant points at it: [`crate::Resource::Panel::across`], fine enough for every point of the longest chart, and
/// whatever its count of points. See `design/projection.md`.
pub const ACROSS: u16 = 10_000;

/// The most characters of what a chart reads at a time, [`Chart::read_at`]: its time, and each series' name and value.
pub const READING_CHARS: usize = 240;

/// A time an axis label says, as [`Chart::time_at`] reads it: so many seconds from now, such as `-15 min`, `now` or `2 h
/// ago`, written with its sign and unit; or, on a `clock`, in seconds since midnight, such as `14:05`, `precise` where it
/// says its seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Time {
    seconds: f64,
    clock: bool,
    precise: bool,
    unit: (&'static str, f64),
    minus: char,
}

impl Time {
    fn relative(seconds: f64, unit: (&'static str, f64), minus: char) -> Self {
        Self {
            seconds,
            clock: false,
            precise: false,
            unit,
            minus,
        }
    }
}

/// The units a relative time is written in: the words read, and how a reading is written in each, with its seconds.
const UNITS: [(&[&str], &str, f64); 4] = [
    (&["s", "sec", "secs", "second", "seconds"], "s", 1.0),
    (&["m", "min", "mins", "minute", "minutes"], "min", 60.0),
    (&["h", "hr", "hrs", "hour", "hours"], "h", 3600.0),
    (&["d", "day", "days"], "d", 86_400.0),
];

impl Time {
    /// The time `text` says, if it says one.
    fn read(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("now") {
            return Some(Self::relative(0.0, ("", 0.0), '-'));
        }
        if let Some(clock) = Self::clock(text) {
            return Some(clock);
        }
        let (text, ago) = match text.strip_suffix(" ago") {
            Some(text) => (text.trim_end(), true),
            None => (text, false),
        };
        let first = text.chars().next()?;
        let (sign, minus, text) = match first {
            '-' | '−' => (-1.0, first, &text[first.len_utf8()..]),
            '+' => (1.0, '-', &text[1..]),
            _ => (if ago { -1.0 } else { 1.0 }, '-', text),
        };
        let digits = text
            .find(|character: char| !(character.is_ascii_digit() || character == '.'))
            .unwrap_or(text.len());
        let number: f64 = text[..digits].parse().ok()?;
        let word = text[digits..].trim().to_ascii_lowercase();
        let (_, written, size) = UNITS
            .iter()
            .find(|(words, ..)| words.contains(&word.as_str()))?;
        Some(Self::relative(
            sign * number * size,
            (written, *size),
            minus,
        ))
    }

    /// The clock time `text` says, as `14:05` or `14:05:30`, if it says one.
    fn clock(text: &str) -> Option<Self> {
        let parts: Vec<&str> = text.split(':').collect();
        if !(2..=3).contains(&parts.len())
            || parts.iter().any(|part| {
                part.is_empty() || part.len() > 2 || !part.bytes().all(|byte| byte.is_ascii_digit())
            })
            || parts[1..].iter().any(|part| part.len() != 2)
        {
            return None;
        }
        let values: Vec<f64> = parts.iter().filter_map(|part| part.parse().ok()).collect();
        if values[0] >= 24.0 || values[1..].iter().any(|value| *value >= 60.0) {
            return None;
        }
        Some(Self {
            clock: true,
            precise: values.len() == 3,
            ..Self::relative(
                values[0] * 3600.0 + values[1] * 60.0 + values.get(2).unwrap_or(&0.0),
                ("", 0.0),
                '-',
            )
        })
    }

    /// Whether `other` is of the same kind, so that a time between the two is read from both.
    fn kin(self, other: Self) -> bool {
        self.clock == other.clock
    }
}

/// `seconds` written as so many of `unit`, with the rest in the next smaller unit where `fine`, as `-6 min 25 s`, or
/// `now` at none.
fn relative(seconds: f64, (written, size): (&str, f64), minus: char, fine: bool) -> String {
    if size <= 0.0 {
        return "now".to_owned();
    }
    let sign = if seconds < 0.0 {
        minus.to_string()
    } else {
        "+".to_owned()
    };
    let smaller = UNITS
        .iter()
        .rev()
        .find(|(.., each)| *each < size)
        .map(|(_, written, each)| (*written, *each));
    let whole = |unit: f64| (seconds.abs() / unit).round();
    match smaller.filter(|_| fine) {
        Some((rest, each)) => {
            let total = whole(each);
            let per = (size / each).round();
            let (most, remainder) = ((total / per).floor(), total % per);
            if most == 0.0 && remainder == 0.0 {
                "now".to_owned()
            } else if most == 0.0 {
                format!("{sign}{remainder} {rest}")
            } else if remainder == 0.0 {
                format!("{sign}{most} {written}")
            } else {
                format!("{sign}{most} {written} {remainder} {rest}")
            }
        }
        None if whole(size) == 0.0 => "now".to_owned(),
        None => format!("{sign}{} {written}", whole(size)),
    }
}

/// `value` as a reading says it, kept to [`SIGNIFICANT`] figures: `203,800`, `1,235,000,000`, `0.6834`, `7` or `-1.5`.
pub fn figure(value: f32) -> String {
    let Some(value) = rounded(value) else {
        return "no data".to_owned();
    };
    let size = f64::from(value.abs());
    if size < 1000.0 {
        return format!("{value}");
    }
    let magnitude = size.log10().floor() as i32;
    let kept = (size / 10f64.powi(magnitude - (SIGNIFICANT - 1))).round() as u64;
    let digits = format!(
        "{kept}{}",
        "0".repeat((magnitude - (SIGNIFICANT - 1)).max(0) as usize)
    );
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    if value < 0.0 {
        format!("-{grouped}")
    } else {
        grouped
    }
}

/// A round number at least `value`: 1, 1.5, 2, 2.5, 3, 4, 5, 6 or 8 times a power of ten.
pub fn nice(value: f32) -> f32 {
    if value <= 0.0 || !value.is_finite() {
        return 1.0;
    }
    let magnitude = 10f32.powf(value.log10().floor());
    [1.0, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0]
        .into_iter()
        .map(|step| step * magnitude)
        .find(|step| *step >= value * 0.999)
        .unwrap_or(10.0 * magnitude)
}

/// `value` held to [`CHART_MAGNITUDE`] either side of zero, or `None` where it is not a number.
fn sane(value: f32) -> Option<f32> {
    (!value.is_nan()).then(|| value.clamp(-CHART_MAGNITUDE, CHART_MAGNITUDE))
}

/// The most stretches a chart's bars are drawn in, and the most vertices of a series' line: a chart of more points
/// draws a bar for each stretch of them, as tall as the one farthest from its base, which its readout reads.
pub const STRETCHES: usize = 128;

impl Chart {
    /// The range of values `chart`'s plot spans, lowest and highest: each fixed where its axis says, or else from 0, or its
    /// lowest value where one is below 0, to a round number above its highest value, or reference line's with a third to
    /// spare, so that a target stands clearly inside the plot; and where its ends would be the same, or the wrong way
    /// round, from its lowest to a tenth of its size, or at least 1, above it. Its values are held to
    /// [`CHART_MAGNITUDE`], so that it is always finite.
    pub fn range(&self) -> (f32, f32) {
        let chart = self;
        let values = chart.series.iter().flat_map(|series| {
            series
                .values
                .iter()
                .flatten()
                .filter_map(|value| sane(*value))
        });
        let references = chart
            .references
            .iter()
            .filter_map(|reference| sane(reference.value));
        let lowest = values
            .clone()
            .chain(references.clone())
            .fold(0.0_f32, f32::min);
        let highest = values
            .chain(references.map(|value| value * 1.3))
            .fold(f32::MIN, f32::max);
        let low = chart.axis.min.and_then(sane).unwrap_or(lowest);
        let high = chart.axis.max.and_then(sane).unwrap_or_else(|| {
            if highest > low {
                low + nice(highest - low)
            } else {
                low + 1.0
            }
        });
        let size = f64::from(low.abs()).max(f64::from(high.abs()));
        if f64::from(high) - f64::from(low) > size * 1e-5 {
            (low, high)
        } else {
            (low, low + (low.abs() * 0.1).max(1.0))
        }
    }

    /// Where along its time axis `across`, in [`ACROSS`] parts, is: a point's index, or a share of the way between two.
    pub fn at_across(&self, across: u16) -> f32 {
        let last = self.points.max(2) - 1;
        f32::from(across.min(ACROSS)) / f32::from(ACROSS) * last as f32
    }

    /// The value of the series `series` at `at`, a point's index or a share of the way between two, as it is drawn
    /// there: between two points its line joins, a line's or an area's, the straight line between them, and nothing past
    /// the last point of a run its line draws, before a gap or a break, but for a point that stands alone, drawn as a
    /// dot, read within half a point of it; a step's held from its point to the next; and a bar's, the bar drawn for
    /// the stretch of points nearest, as tall as the one of them farthest from its base where a chart of more than
    /// [`STRETCHES`] points draws one for each stretch. `None` where it has no value there, a gap, never zero; kept to
    /// [`SIGNIFICANT`] figures.
    pub fn value_at(&self, series: usize, at: f32) -> Option<f32> {
        let series = self.series.get(series)?;
        let points = self.points.clamp(2, CHART_POINTS) as usize;
        let last = (points - 1) as f32;
        let at = if at.is_nan() {
            0.0
        } else {
            at.clamp(0.0, last)
        };
        let value = |index: usize| series.values.get(index).copied().flatten();
        // Whether the point `index` is joined to the next by its line.
        let joined = |index: usize| {
            value(index).is_some()
                && value(index + 1).is_some()
                && !series.breaks.contains(&(index as u32))
        };
        let index = at.floor() as usize;
        let share = at - index as f32;
        let nearest = at.round() as usize;
        let value = match series.style {
            SeriesStyle::Step => value(index),
            SeriesStyle::Bars => {
                let stretches = points.min(STRETCHES);
                let stretch = (0..stretches)
                    .find(|stretch| nearest < (stretch + 1) * points / stretches)
                    .unwrap_or(stretches - 1);
                let base = 0.0_f32.clamp(self.range().0, self.range().1);
                series.values[(stretch * points / stretches).min(series.values.len())
                    ..((stretch + 1) * points / stretches).min(series.values.len())]
                    .iter()
                    .flatten()
                    .copied()
                    .max_by(|a, b| (a - base).abs().total_cmp(&(b - base).abs()))
            }
            _ if share == 0.0 => value(index),
            _ if joined(index) => {
                let (from, to) = (
                    value(index).unwrap_or_default(),
                    value(index + 1).unwrap_or_default(),
                );
                Some(
                    (f64::from(from) + (f64::from(to) - f64::from(from)) * f64::from(share)) as f32,
                )
            }
            // A point alone, joined to neither neighbour, is drawn as a dot, read within half a point of it.
            _ if !joined(nearest) && (nearest == 0 || !joined(nearest - 1)) => value(nearest),
            _ => None,
        };
        value.and_then(rounded)
    }

    /// The time at `at`, a point's index or a share of the way between two, as its axis labels say it: read from the
    /// two labels about it that say a time of one kind, or the two nearest it, and written as they are, `-6 min 25 s`
    /// or `now` between `-10 min` and `-5 min`, or `14:03:25` between `14:00` and `14:05`, to the second where its points
    /// are less than the unit apart; or else a label at the point nearest it, or which point it is, `point 121 of
    /// 181`.
    pub fn time_at(&self, at: f32) -> String {
        let last = (self.points.max(2) - 1) as f32;
        let at = if at.is_nan() {
            0.0
        } else {
            at.clamp(0.0, last)
        };
        let mut times: Vec<(f32, Time)> = self
            .labels
            .iter()
            .filter_map(|label| Time::read(&label.text).map(|time| (label.at as f32, time)))
            .collect();
        times.sort_by(|a, b| a.0.total_cmp(&b.0));
        times.dedup_by(|a, b| a.0 == b.0);
        // The pair about it, or else nearest it, of one kind.
        let pairs = times.windows(2).filter(|pair| pair[0].1.kin(pair[1].1));
        let pair = pairs.min_by(|a, b| {
            let distance = |pair: &&[(f32, Time)]| (pair[0].0 - at).max(at - pair[1].0).max(0.0);
            distance(a).total_cmp(&distance(b))
        });
        let nearest = (at.round() as u32).min(self.points.max(2) - 1);
        let Some([(from_at, from), (to_at, to)]) = pair.map(|pair| [pair[0], pair[1]]) else {
            return self
                .labels
                .iter()
                .find(|label| label.at == nearest)
                .map_or_else(
                    || format!("point {} of {}", nearest + 1, self.points.max(2)),
                    |label| label.text.clone(),
                );
        };
        let (start, mut end) = (from.seconds, to.seconds);
        if from.clock && end < start {
            end += 86_400.0;
        }
        let step = (end - start) / f64::from(to_at - from_at);
        let seconds = start + step * f64::from(at - from_at);
        if from.clock {
            let seconds = seconds.rem_euclid(86_400.0).round() as u64 % 86_400;
            let (hours, minutes) = (seconds / 3600, seconds / 60 % 60);
            return if from.precise || to.precise || step.abs() < 60.0 {
                format!("{hours:02}:{minutes:02}:{:02}", seconds % 60)
            } else {
                format!("{hours:02}:{minutes:02}")
            };
        }
        // In the larger unit of the two, `now` having none, and its sign.
        let larger = if to.unit.1 > from.unit.1 { to } else { from };
        relative(
            seconds,
            larger.unit,
            larger.minus,
            step.abs() < larger.unit.1,
        )
    }

    /// The time at `at`, a point's index or a share of the way between two, held to its points, as its [`TimeAxis`] says:
    /// so many milliseconds before now on a live chart, or the instant on a fixed window; `None` where it says neither.
    /// Its labels are never read for it. See the linked time cursor in `design/projection.md`.
    ///
    /// ```
    /// use tomori_protocol::chart::{Chart, ChartTime, TimeAxis};
    ///
    /// let live = Chart::new(181).with_time(TimeAxis::Live { step_ms: 5_000 });
    /// assert_eq!(live.time_of(120.0), Some(ChartTime::BeforeNow { ms: 300_000.0 }));
    /// let hourly = Chart::new(61).with_time(TimeAxis::Live { step_ms: 60_000 });
    /// assert_eq!(hourly.at_time(ChartTime::BeforeNow { ms: 300_000.0 }), Some(55.0));
    /// assert_eq!(Chart::new(10).time_of(3.0), None);
    /// ```
    pub fn time_of(&self, at: f32) -> Option<ChartTime> {
        let last = f64::from(self.points.max(2) - 1);
        let at = if at.is_nan() {
            0.0
        } else {
            f64::from(at).clamp(0.0, last)
        };
        match self.time? {
            TimeAxis::Live { step_ms } => Some(ChartTime::BeforeNow {
                ms: (last - at) * f64::from(step_ms),
            }),
            TimeAxis::Fixed { from_ms, to_ms } => Some(ChartTime::Instant {
                utc_ms: from_ms as f64 + (to_ms as f64 - from_ms as f64) * at / last,
            }),
            TimeAxis::Unknown => None,
        }
    }

    /// Where along its time axis `time` is, a point's index or a share of the way between two, as its [`TimeAxis`] says:
    /// a time before now only on a live chart and an instant only on a fixed window; `None` for another kind, a chart
    /// that says neither, or a time before its first point or after its last by more than half a point, so that a chart
    /// that does not cover it shows nothing there.
    pub fn at_time(&self, time: ChartTime) -> Option<f32> {
        let last = f64::from(self.points.max(2) - 1);
        let at = match (self.time?, time) {
            (TimeAxis::Live { step_ms }, ChartTime::BeforeNow { ms }) => {
                last - ms / f64::from(step_ms)
            }
            (TimeAxis::Fixed { from_ms, to_ms }, ChartTime::Instant { utc_ms }) => {
                (utc_ms - from_ms as f64) / (to_ms as f64 - from_ms as f64) * last
            }
            _ => return None,
        };
        (at.is_finite() && (-0.5..=last + 0.5).contains(&at)).then(|| at.clamp(0.0, last) as f32)
    }

    /// How a value of the chart reads, `value` in its axis's unit, `203,800 a minute`; `no data` where it has none.
    pub fn amount(&self, value: Option<f32>) -> String {
        match value {
            Some(value) if self.axis.unit.is_empty() => figure(value),
            Some(value) => format!("{} {}", figure(value), self.axis.unit),
            None => "no data".to_owned(),
        }
    }

    /// What the chart reads at `at`, a point's index or a share of the way between two, in plain text: its time, and
    /// each series' name and value there, `-6 min: admitted 203,800 a minute, rejected no data`, on one line of at
    /// most [`READING_CHARS`].
    pub fn read_at(&self, at: f32) -> String {
        let values: Vec<String> = self
            .series
            .iter()
            .take(CHART_SERIES)
            .enumerate()
            .map(|(index, series)| {
                format!(
                    "{} {}",
                    one_line(&series.name, CHART_NAME_CHARS),
                    self.amount(self.value_at(index, at))
                )
            })
            .collect();
        one_line(
            &format!("{}: {}", self.time_at(at), values.join(", ")),
            READING_CHARS,
        )
    }
}

#[cfg(test)]
mod tests;
