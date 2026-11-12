// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The rule for fixed time: a published panel whose time is fixed to a window, by its own setting or its wall's, shows
//! only content its side declared for exactly that window, a chart whose time axis is that window or a panel published
//! for it, keeping what it shows otherwise; until its side answers, it says it is waiting for that window's data, never
//! showing what it showed live. A chart held to a window has its axis labelled by the server, in clock times in UTC,
//! from the window alone. See `design/panel-context.md`.
use tomori_protocol::chart::{AxisLabel, Chart, Note, TimeAxis, Tone};

use super::context::Setting;
use super::published::Published;
use super::{Kept, Room, Stage};

/// What a panel held to a window shows until its side publishes content for it.
pub const WAITING: &str = "Waiting for this window's data";

/// Why a row of a panel waiting for its window's data is not chosen: what it showed is not what that window held.
pub const WAITING_ROWS: &str = "That panel is waiting for its window's data.";

/// How many labels the server writes along the axis of a chart held to a window, at most: its first point, its middle
/// and its last.
const LABELS: u32 = 3;

impl Published {
    /// Whether it is declared to show what the window from `from_ms` to `to_ms` held: a chart whose time axis is that
    /// window, or a panel published for it.
    pub(super) fn shows_window(&self, (from_ms, to_ms): (i64, i64)) -> bool {
        self.window == Some((from_ms, to_ms))
            || self.chart.as_ref().and_then(|chart| chart.time)
                == Some(TimeAxis::Fixed { from_ms, to_ms })
    }
}

/// The window `kept` is held to in `room`, as absolute instants in milliseconds since the Unix epoch in UTC: its own
/// time setting's, or else its wall's, where that is a window and it depends on time; none otherwise.
pub(super) fn window_in(room: &Room, kept: &Kept) -> Option<(i64, i64)> {
    if !kept.timed() {
        return None;
    }
    let wall = kept.wall().and_then(|wall| room.contexts.get(wall));
    let setting = kept
        .context
        .get(tomori_environment::TIME)
        .or_else(|| wall.and_then(|wall| wall.get(tomori_environment::TIME)));
    match setting {
        Some(Setting::Window { from_ms, to_ms }) => Some((*from_ms, *to_ms)),
        _ => None,
    }
}

/// The instant `ms` in UTC, as `format` writes it.
fn utc(ms: i64, format: &str) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map_or_else(|_| ms.to_string(), |at| at.strftime(format).to_string())
}

/// The window from `from_ms` to `to_ms` as a panel held to it says it: `2026-11-12 02:20–02:50 UTC` within a day, or
/// each end with its date, `2026-11-11 23:40–2026-11-12 00:10 UTC`.
pub fn described((from_ms, to_ms): (i64, i64)) -> String {
    let day = |ms: i64| ms.div_euclid(86_400_000);
    let to = if day(from_ms) == day(to_ms) {
        utc(to_ms, "%H:%M")
    } else {
        utc(to_ms, "%Y-%m-%d %H:%M")
    };
    format!("{}–{to} UTC", utc(from_ms, "%Y-%m-%d %H:%M"))
}

/// The labels of a chart of `points` points over the window from `from_ms` to `to_ms`: its first, middle and last
/// points' clock times in UTC, `02:20`, the last saying `UTC`; for a window across days, the date before the first and
/// before any on another day than the label before it, `11-12 02:20`; as many as it has points, at most [`LABELS`].
pub fn labels(points: u32, (from_ms, to_ms): (i64, i64)) -> Vec<AxisLabel> {
    let last = points.max(1) - 1;
    let count = LABELS.min(last + 1);
    let mut written = Vec::new();
    // Within a day, its labels are clock times alone; across days, each on another day than the one before says it.
    let one_day = from_ms.div_euclid(86_400_000) == to_ms.div_euclid(86_400_000);
    let mut day = one_day.then(|| from_ms.div_euclid(86_400_000));
    for index in 0..count {
        let at = if count == 1 {
            0
        } else {
            last * index / (count - 1)
        };
        let ms = if last == 0 {
            from_ms
        } else {
            from_ms + (to_ms - from_ms) * i64::from(at) / i64::from(last)
        };
        let today = ms.div_euclid(86_400_000);
        let mut text = if day == Some(today) {
            utc(ms, "%H:%M")
        } else {
            utc(ms, "%m-%d %H:%M")
        };
        day = Some(today);
        if index + 1 == count {
            text.push_str(" UTC");
        }
        written.push(AxisLabel::new(at, text));
    }
    written
}

impl Stage {
    /// The window the panel its environment published as `id` in `environment`, shown, is held to, if any.
    pub(super) fn held_window(&self, environment: &str, id: &str) -> Option<(i64, i64)> {
        let room = self.rooms.get(environment)?;
        room.panels
            .iter()
            .find(|kept| {
                kept.published
                    .as_ref()
                    .is_some_and(|published| published.id == id)
            })
            .and_then(|kept| window_in(room, kept))
    }

    /// `panel`, `kept` as the view shows it, held to the window it is fixed to in `environment`, if any: with its axis
    /// labelled in UTC where it shows that window, or else saying it is waiting for that window's data, a chart without
    /// values over the window, which says it has no data, or a body saying so, without rows.
    pub(super) fn held(
        &self,
        environment: &str,
        kept: &Kept,
        mut panel: tomori_protocol::Panel,
    ) -> tomori_protocol::Panel {
        let Some(window) = self
            .rooms
            .get(environment)
            .and_then(|room| window_in(room, kept))
        else {
            return panel;
        };
        let published = kept
            .published
            .as_ref()
            .expect("a panel held depends on time");
        if published.shows_window(window) {
            if let Some(chart) = &mut panel.chart {
                chart.labels = labels(chart.points, window);
            }
            return panel;
        }
        let waiting = format!("{WAITING}, {}.", described(window));
        if let Some(chart) = &mut panel.chart {
            let (from_ms, to_ms) = window;
            **chart = Chart::new(chart.points)
                .with_axis(chart.axis.clone())
                .with_time(TimeAxis::Fixed { from_ms, to_ms })
                .with_labels(labels(chart.points, window))
                .with_status(Note::new(waiting.clone(), Tone::Neutral));
        }
        panel.body = waiting;
        panel.rows.clear();
        panel.columns.clear();
        panel
    }
}

#[cfg(test)]
mod tests;
