// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Panels a server side publishes onto its environment's own walls, for everyone in its space: see [`Panel`] and
//! [`crate::Space::publish`].
use tomori_protocol::Chart;

/// The longest id of a published panel, in bytes.
pub const PANEL_ID_BYTES: usize = 32;

/// The most panels a side shows in its space with [`crate::Space::publish`], counting those whose updates wait: one
/// more is refused, saying so.
pub const PANEL_LIMIT: usize = 8;

/// Whether `id` is one a side may name a panel by: 1 to [`PANEL_ID_BYTES`] ASCII letters, digits, `-` and `_`, as the
/// agent names its panels.
///
/// ```
/// use tomori_environment::valid_panel_id;
///
/// assert!(valid_panel_id("scaling-pressure"));
/// assert!(!valid_panel_id(""));
/// assert!(!valid_panel_id("scaling pressure"));
/// assert!(!valid_panel_id("pressure/1"));
/// ```
pub fn valid_panel_id(id: &str) -> bool {
    (1..=PANEL_ID_BYTES).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// A panel a side publishes onto one of its environment's own walls, by an id it chooses: a text panel, its body
/// Markdown-lite text, drawn as text and never as HTML, or a chart panel, with its [`Chart`]. It names the wall by its
/// place in the order the environment declares its walls, from 0, and may say which slot it prefers. See
/// `design/environments.md`.
///
/// ```
/// use tomori_environment::Panel;
/// use tomori_environment::chart::{Chart, Colour, Series, SeriesStyle};
///
/// let notice = Panel::text("rota", "Rota", "- [ ] Feed the hens");
/// assert_eq!((notice.id(), notice.wall(), notice.slot()), ("rota", 0, None));
/// let chart = Chart::new(2).with_series(Series::new("eggs", Colour::Amber, SeriesStyle::Bars, vec![Some(3.0), None]));
/// let eggs = Panel::chart("eggs", "Eggs", chart).on_wall(1, Some(2)).with_reference("chart/eggs");
/// assert_eq!((eggs.wall(), eggs.slot(), eggs.reference()), (1, Some(2), Some("chart/eggs")));
/// assert!(eggs.chart_spec().is_some());
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Panel {
    id: String,
    title: String,
    body: String,
    chart: Option<Chart>,
    wall: u32,
    slot: Option<u32>,
    reference: Option<String>,
}

impl Panel {
    /// The text panel `id`, titled `title`, showing `body`, on the environment's first wall.
    pub fn text(id: impl Into<String>, title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            body: body.into(),
            chart: None,
            wall: 0,
            slot: None,
            reference: None,
        }
    }

    /// The chart panel `id`, titled `title`, showing `chart`, on the environment's first wall. Its body is the summary
    /// the server makes of the chart, which a client that cannot draw it shows.
    pub fn chart(id: impl Into<String>, title: impl Into<String>, chart: Chart) -> Self {
        Self {
            chart: Some(chart),
            ..Self::text(id, title, "")
        }
    }

    /// The panel, on the environment's own wall `wall`, from 0 in the order it declares them, in `slot` if it is free.
    pub fn on_wall(mut self, wall: u32, slot: Option<u32>) -> Self {
        self.wall = wall;
        self.slot = slot;
        self
    }

    /// The panel, which the space's agent can inspect by `reference`, as an [`crate::Object`] it describes.
    pub fn with_reference(mut self, reference: impl Into<String>) -> Self {
        self.reference = Some(reference.into());
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    /// Its chart, if it is a chart panel.
    pub fn chart_spec(&self) -> Option<&Chart> {
        self.chart.as_ref()
    }

    /// The environment's wall it goes on, from 0.
    pub fn wall(&self) -> u32 {
        self.wall
    }

    /// The slot it prefers, if it says.
    pub fn slot(&self) -> Option<u32> {
        self.slot
    }

    /// The reference the agent can inspect it by, if it has one.
    pub fn reference(&self) -> Option<&str> {
        self.reference.as_deref()
    }

    /// Its parts, to keep: its id, title, body, chart, wall, slot and reference.
    #[allow(clippy::type_complexity)]
    pub fn into_parts(
        self,
    ) -> (
        String,
        String,
        String,
        Option<Chart>,
        u32,
        Option<u32>,
        Option<String>,
    ) {
        (
            self.id,
            self.title,
            self.body,
            self.chart,
            self.wall,
            self.slot,
            self.reference,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panel_is_what_it_was_given() {
        let panel = Panel::text("rota", "Rota", "Feed the hens").on_wall(2, Some(1));
        assert_eq!(
            (
                panel.title(),
                panel.body(),
                panel.chart_spec(),
                panel.reference()
            ),
            ("Rota", "Feed the hens", None, None)
        );
        let chart = Panel::chart("eggs", "Eggs", Chart::new(2)).with_reference("chart/eggs");
        assert_eq!(chart.body(), "");
        let (id, title, body, spec, wall, slot, reference) = chart.into_parts();
        assert_eq!(
            (id.as_str(), title.as_str(), body.as_str()),
            ("eggs", "Eggs", "")
        );
        assert_eq!((spec, wall, slot), (Some(Chart::new(2)), 0, None));
        assert_eq!(reference.as_deref(), Some("chart/eggs"));
        assert_eq!(
            (panel.wall(), panel.slot(), panel.id()),
            (2, Some(1), "rota")
        );
    }

    #[test]
    fn a_panel_id_is_a_short_slug() {
        assert!(valid_panel_id(&"a".repeat(PANEL_ID_BYTES)));
        assert!(!valid_panel_id(&"a".repeat(PANEL_ID_BYTES + 1)));
        for refused in ["", "a b", "a:b", "a/b", "ä", "a\n"] {
            assert!(!valid_panel_id(refused), "{refused:?}");
        }
        assert!(valid_panel_id("Scaling_Pressure-2"));
    }
}
