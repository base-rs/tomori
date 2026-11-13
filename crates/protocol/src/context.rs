// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The context a wall or a panel shows: what its environment's panels on it depend on, such as a region and a time
//! window, each following the environment's selection or fixed, as chips a client draws read-only along a wall's top
//! edge and in a panel's chrome, and the agent reads. Time is always absolute instants in UTC, never a time of day. See
//! `design/panel-context.md`.
use serde::{Deserialize, Serialize};

/// The most chips a wall or a panel has: its environment's parameters and time.
pub const CONTEXT_CHIPS: usize = 4;

/// The id of the chip for Tomori's own parameter, time.
pub const TIME: &str = "time";

/// What a chip says its parameter is: a value of the environment's, as `US-EAST1`; live, the last `minutes` up to now;
/// or a window from `from_ms` to `to_ms`, absolute instants in milliseconds since the Unix epoch in UTC, which a client
/// writes in UTC, saying so. `Unknown` stands for a kind a newer server sends, which a client draws as its label alone.
///
/// ```
/// use tomori_protocol::context::ContextValue;
///
/// let window = ContextValue::Window { from_ms: 1_794_450_000_000, to_ms: 1_794_451_800_000 };
/// assert_eq!(
///     serde_json::to_string(&window).unwrap(),
///     r#"{"kind":"window","from_ms":1794450000000,"to_ms":1794451800000}"#
/// );
/// let newer: ContextValue = serde_json::from_str(r#"{"kind": "season"}"#).unwrap();
/// assert_eq!(newer, ContextValue::Unknown);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContextValue {
    Text {
        text: String,
    },
    Live {
        minutes: u32,
    },
    Window {
        from_ms: i64,
        to_ms: i64,
    },
    #[serde(other)]
    Unknown,
}

/// Where a chip's value comes from: the environment's selection, which it follows; a setting of its own, which fixes
/// it; or, on a panel, its wall's setting, which it inherits. `Follows` stands for a kind a newer server sends too.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContextSetting {
    Fixed,
    Inherited,
    #[default]
    #[serde(other)]
    Follows,
}

/// One chip of a wall's or a panel's context: the parameter's id, as the environment declares it or [`TIME`], its
/// label, its value and where that comes from. A wall has a chip for each setting of its own; a panel its environment
/// published, for each parameter it depends on and time, as it is shown now. See `design/panel-context.md`.
///
/// ```
/// use tomori_protocol::context::{ContextChip, ContextSetting, ContextValue};
///
/// let chip = ContextChip::new("region", "Region", ContextValue::Text { text: "US-EAST1".into() }, ContextSetting::Fixed);
/// assert_eq!(
///     serde_json::to_string(&chip).unwrap(),
///     r#"{"parameter":"region","label":"Region","value":{"kind":"text","text":"US-EAST1"},"setting":"fixed"}"#
/// );
/// // Following, it says nothing of its setting.
/// let follows = ContextChip::new("region", "Region", ContextValue::Text { text: "US-EAST1".into() }, ContextSetting::Follows);
/// assert!(serde_json::to_value(&follows).unwrap().get("setting").is_none());
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ContextChip {
    pub parameter: String,
    pub label: String,
    pub value: ContextValue,
    #[serde(default, skip_serializing_if = "is_follows")]
    pub setting: ContextSetting,
    /// What pressing it does, pointing at it and pressing A, Enter or a click, or pulling the trigger, if anything: on a
    /// copy's time chip, freezing it or unfreezing it, which the client asks with [`crate::ClientMessage::FreezePanel`].
    /// An older server says nothing, and an older client ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub press: Option<ChipPress>,
}

/// What pressing a chip does: freezing its panel's time to the window it shows, or unfreezing it to follow again.
/// `Unknown` stands for what a newer server sends, which a client offers nothing for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChipPress {
    Freeze,
    Unfreeze,
    #[serde(other)]
    Unknown,
}

fn is_follows(setting: &ContextSetting) -> bool {
    *setting == ContextSetting::Follows
}

impl ContextChip {
    pub fn new(
        parameter: impl Into<String>,
        label: impl Into<String>,
        value: ContextValue,
        setting: ContextSetting,
    ) -> Self {
        Self {
            parameter: parameter.into(),
            label: label.into(),
            value,
            setting,
            press: None,
        }
    }

    /// The chip, which pressing does `press`.
    ///
    /// ```
    /// use tomori_protocol::context::{ChipPress, ContextChip, ContextSetting, ContextValue};
    ///
    /// let time = ContextChip::new("time", "Time", ContextValue::Live { minutes: 15 }, ContextSetting::Follows)
    ///     .with_press(ChipPress::Freeze);
    /// assert!(serde_json::to_string(&time).unwrap().ends_with(r#""press":"freeze"}"#));
    /// let newer: ChipPress = serde_json::from_str(r#""thaw""#).unwrap();
    /// assert_eq!(newer, ChipPress::Unknown);
    /// ```
    pub fn with_press(mut self, press: ChipPress) -> Self {
        self.press = Some(press);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Panel, Place, Pose, Wall, WallKind};

    fn chip() -> ContextChip {
        ContextChip::new(
            "region",
            "Region",
            ContextValue::Text {
                text: "EU-WEST1".into(),
            },
            ContextSetting::Inherited,
        )
    }

    #[test]
    fn a_chip_says_what_pressing_it_does_and_an_older_server_says_nothing() {
        let time = ContextChip::new(
            "time",
            "Time",
            ContextValue::Live { minutes: 15 },
            ContextSetting::Follows,
        );
        assert_eq!(time.press, None);
        for (press, said) in [
            (ChipPress::Freeze, "freeze"),
            (ChipPress::Unfreeze, "unfreeze"),
        ] {
            let pressed = time.clone().with_press(press);
            let sent = serde_json::to_value(&pressed).unwrap();
            assert_eq!(sent["press"], said);
            assert_eq!(
                serde_json::from_value::<ContextChip>(sent).unwrap(),
                pressed
            );
        }
        let older: ContextChip = serde_json::from_str(
            r#"{"parameter":"time","label":"Time","value":{"kind":"live","minutes":15}}"#,
        )
        .unwrap();
        assert_eq!(older, time);
        let newer: ChipPress = serde_json::from_str(r#""thaw""#).unwrap();
        assert_eq!(newer, ChipPress::Unknown);
    }

    #[test]
    fn a_wall_s_and_a_panel_s_context_is_sent_only_where_it_says_something_and_is_part_of_their_versions()
     {
        let panel = Panel::new("panel-1", "Latency", "", Place::Free { pose: Pose::HOME });
        let wall = Wall::new("environment-0", WallKind::Environment, Pose::HOME);
        assert!(
            serde_json::to_value(&panel)
                .unwrap()
                .get("context")
                .is_none()
        );
        assert!(
            serde_json::to_value(&wall)
                .unwrap()
                .get("context")
                .is_none()
        );
        let chipped = panel.clone().with_context(vec![chip()]);
        let fixed = wall.clone().with_context(vec![chip()]);
        assert_eq!(
            serde_json::to_value(&chipped).unwrap()["context"][0]["setting"],
            "inherited"
        );
        assert_eq!(
            serde_json::from_value::<Panel>(serde_json::to_value(&chipped).unwrap()).unwrap(),
            chipped
        );
        assert_eq!(
            serde_json::from_value::<Wall>(serde_json::to_value(&fixed).unwrap()).unwrap(),
            fixed
        );
        // An older server sends none; a newer setting reads as following.
        let older: Panel = serde_json::from_str(
            r#"{"id": "panel-1", "title": "", "body": "", "place": {"kind": "free", "pose": {"position": [0, 0, 0], "orientation": [0, 0, 0, 1]}}}"#,
        )
        .unwrap();
        assert!(older.context.is_empty());
        let newer: ContextChip = serde_json::from_str(
            r#"{"parameter": "region", "label": "Region", "value": {"kind": "text", "text": "x"}, "setting": "borrowed"}"#,
        )
        .unwrap();
        assert_eq!(newer.setting, ContextSetting::Follows);
        // A change of context is a change of structure, and of what a wall says.
        assert!(!chipped.same_structure(&panel));
        assert!(chipped.same_structure(&chipped.clone()));
        assert!(!fixed.same_but_version(&wall));
    }
}
