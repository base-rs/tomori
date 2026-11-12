// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The rule for fixed time: a published panel whose time is fixed to a window, by its own setting or its wall's, shows
//! only content its side declared for exactly that window, a chart whose time axis is that window or a panel published
//! for it, keeping what it shows otherwise; until its side answers, it says it is waiting for that window's data, never
//! showing what it showed live, and, after [`WAIT_SECS`] without it, that it has no data for that window. A chart held
//! to a window has its axis labelled by the server, in clock times in UTC, from the window alone. See
//! `design/panel-context.md`.
use serde::{Deserialize, Serialize};
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

/// How long a panel held to a window waits for its side to publish content for it before it says it has no data for that
/// window, in seconds: long enough for a side that keeps history to fetch it, short enough that a side that does not
/// answer, as an older one that ignores how its panels are bound, leaves no panel waiting for ever.
pub const WAIT_SECS: u64 = 45;

/// What a panel held to a window says once it has waited [`WAIT_SECS`] for content for it.
pub const NO_DATA: &str = "No data for this window";

/// How long a panel has waited for content for `window`: since `since`, in seconds since the epoch, and whether it has
/// waited [`WAIT_SECS`] already, so that it says it has no data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Waited {
    window: (i64, i64),
    since: u64,
    lapsed: bool,
}

/// Whether what a side published, `chart` or else a panel published for `window`, shows what the window from `from_ms`
/// to `to_ms` held: a chart whose time axis is that window, fixed, or another panel published for that window.
pub(super) fn declares(
    chart: Option<&Chart>,
    window: Option<(i64, i64)>,
    (from_ms, to_ms): (i64, i64),
) -> bool {
    match chart {
        Some(chart) => chart.time == Some(TimeAxis::Fixed { from_ms, to_ms }),
        None => window == Some((from_ms, to_ms)),
    }
}

impl Published {
    /// Whether it shows what the window `window` held, as [`declares`] says.
    fn shows_window(&self, window: (i64, i64)) -> bool {
        declares(self.chart.as_ref(), self.window, window)
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

/// The labels of a chart of `points` points over the window from `from_ms` to `to_ms`, against `today`, in days since
/// the Unix epoch in UTC: its first, middle and last points' clock times in UTC, `02:20`, the last saying `UTC`; the date
/// before the first where the window is not within today, `11-12 02:20`, and, across days, before any on another day than
/// the label before it; as many as it has points, at most [`LABELS`].
pub fn labels(points: u32, (from_ms, to_ms): (i64, i64), today: i64) -> Vec<AxisLabel> {
    let last = points.max(1) - 1;
    let count = LABELS.min(last + 1);
    let day = |ms: i64| ms.div_euclid(86_400_000);
    let mut written = Vec::new();
    // Within today, its labels are clock times alone; otherwise each on another day than the one before says it.
    let within_today = day(from_ms) == today && day(to_ms) == today;
    let mut shown = within_today.then_some(today);
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
        let mut text = if shown == Some(day(ms)) {
            utc(ms, "%H:%M")
        } else {
            utc(ms, "%m-%d %H:%M")
        };
        shown = Some(day(ms));
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

    /// Mark how long each panel in `environment` held to a window it shows nothing for has waited, at `now`, in seconds
    /// since the epoch: from now, where it begins to wait, as its window changes, for one held anew; and as having waited
    /// [`WAIT_SECS`], once it has, so that it says it has no data. Returns whether any panel now says so.
    pub(crate) fn lapse_waiting(&mut self, environment: &str, now: u64) -> bool {
        let Some(room) = self.rooms.get_mut(environment) else {
            return false;
        };
        let windows: Vec<Option<(i64, i64)>> = room
            .panels
            .iter()
            .map(|kept| window_in(room, kept))
            .collect();
        let mut lapsed = false;
        let published = room
            .panels
            .iter_mut()
            .zip(windows)
            .filter_map(|(kept, window)| {
                kept.published.as_mut().map(|published| (published, window))
            });
        for (published, window) in published {
            let waiting = window.filter(|window| !published.shows_window(*window));
            published.waited = match (waiting, published.waited) {
                (Some(window), Some(waited)) if waited.window == window => {
                    let due = !waited.lapsed && now.saturating_sub(waited.since) >= WAIT_SECS;
                    lapsed |= due;
                    Some(Waited {
                        lapsed: waited.lapsed || due,
                        ..waited
                    })
                }
                (Some(window), _) => Some(Waited {
                    window,
                    since: now,
                    lapsed: false,
                }),
                (None, _) => None,
            };
        }
        lapsed
    }

    /// `panel`, `kept` as the view shows it at `now`, in seconds since the epoch, held to the window it is fixed to in
    /// `environment`, if any: with its axis labelled in UTC where it shows that window, or else saying it is waiting for
    /// that window's data, or, once it has waited [`WAIT_SECS`], that it has none, a chart without values over the
    /// window, which says it has no data, or a body saying so, without rows.
    pub(super) fn held(
        &self,
        environment: &str,
        kept: &Kept,
        mut panel: tomori_protocol::Panel,
        now: u64,
    ) -> tomori_protocol::Panel {
        let Some(window) = self
            .rooms
            .get(environment)
            .and_then(|room| window_in(room, kept))
        else {
            return panel;
        };
        let today = (now / 86_400) as i64;
        let published = kept
            .published
            .as_ref()
            .expect("a panel held depends on time");
        if published.shows_window(window) {
            if let Some(chart) = &mut panel.chart {
                chart.labels = labels(chart.points, window, today);
            }
            return panel;
        }
        let lapsed = published
            .waited
            .is_some_and(|waited| waited.window == window && waited.lapsed);
        let (said, tone) = if lapsed {
            (NO_DATA, Tone::Warn)
        } else {
            (WAITING, Tone::Neutral)
        };
        let waiting = format!("{said}, {}.", described(window));
        if let Some(chart) = &mut panel.chart {
            let (from_ms, to_ms) = window;
            **chart = Chart::new(chart.points)
                .with_axis(chart.axis.clone())
                .with_time(TimeAxis::Fixed { from_ms, to_ms })
                .with_labels(labels(chart.points, window, today))
                .with_status(Note::new(waiting.clone(), tone));
        }
        panel.body = waiting;
        panel.rows.clear();
        panel.columns.clear();
        panel
    }
}

#[cfg(test)]
mod tests;
