// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The context a side's panels depend on: the parameters it declares, each with the value its own selection gives it
//! now, and Tomori's own time, and how each panel is bound to them, following or fixed, as the server tells the side.
//! See `design/panel-context.md`.

/// The most parameters a side declares at once, besides Tomori's own [`TIME`].
pub const CONTEXT_PARAMETERS: usize = 3;

/// The longest id of a parameter, in bytes, as a panel's.
pub const CONTEXT_ID_BYTES: usize = crate::PANEL_ID_BYTES;

/// The longest label of a parameter, in characters, once cleaned onto one line: a longer one is cut.
pub const CONTEXT_LABEL_CHARS: usize = 16;

/// The longest value of a parameter, or of one of its choices, in characters, once cleaned onto one line: a longer one
/// is cut.
pub const CONTEXT_VALUE_CHARS: usize = 32;

/// The most choices a parameter offers.
pub const CONTEXT_CHOICES: usize = 16;

/// The id of Tomori's own parameter, a panel's time: live, the last minutes up to now, or fixed to a window of absolute
/// instants in UTC. A side never declares it, and names it in [`crate::Panel::depends_on`] for a panel that is not a
/// chart but shows a time, as a list of events.
pub const TIME: &str = "time";

/// Whether `id` names an instance the server made of one of a side's panels, as `~7`: a `~` and 1 to 9 digits, which
/// [`crate::valid_panel_id`] refuses, so that a side never names one of its own panels so, and makes no instance
/// itself; it only publishes to, and retracts, an instance the server told it of ([`crate::Intent::context`]).
///
/// ```
/// use tomori_environment::is_instance;
///
/// assert!(is_instance("~7"));
/// assert!(!is_instance("latency") && !is_instance("~") && !is_instance("~x") && !is_instance("~1234567890"));
/// ```
pub fn is_instance(id: &str) -> bool {
    id.strip_prefix('~').is_some_and(|digits| {
        (1..=9).contains(&digits.len()) && digits.bytes().all(|byte| byte.is_ascii_digit())
    })
}

/// Whether `id` is one a side may declare a parameter by: 1 to [`CONTEXT_ID_BYTES`] ASCII letters, digits, `-` and
/// `_`, as a panel's id, but never [`TIME`], which is Tomori's.
///
/// ```
/// use tomori_environment::valid_context_id;
///
/// assert!(valid_context_id("region"));
/// assert!(!valid_context_id("time"));
/// assert!(!valid_context_id("a region"));
/// ```
pub fn valid_context_id(id: &str) -> bool {
    crate::valid_panel_id(id) && id != TIME
}

/// A parameter a side's panels depend on, as it declares it with [`crate::Space::set_context`]: its id, its label, as
/// a person reads it, the value its own selection gives it now, which a panel following it shows, and, optionally, the
/// values it may take. See `design/panel-context.md`.
///
/// ```
/// use tomori_environment::ContextParameter;
///
/// let region = ContextParameter::new("region", "Region", "US-EAST1").with_choices(["US-EAST1", "EU-WEST1"]);
/// assert_eq!((region.id(), region.label(), region.value()), ("region", "Region", "US-EAST1"));
/// assert_eq!(region.choices(), ["US-EAST1", "EU-WEST1"]);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ContextParameter {
    id: String,
    label: String,
    value: String,
    choices: Vec<String>,
}

impl ContextParameter {
    /// The parameter `id`, read as `label`, whose value is `value` now.
    pub fn new(id: impl Into<String>, label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            value: value.into(),
            choices: Vec::new(),
        }
    }

    /// The parameter, which may take each of `choices`, at most [`CONTEXT_CHOICES`].
    pub fn with_choices(mut self, choices: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.choices = choices.into_iter().map(Into::into).collect();
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// The value the side's own selection gives it now.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// The values it may take, if it said.
    pub fn choices(&self) -> &[String] {
        &self.choices
    }
}

/// How a panel is bound to one of the side's parameters: following the side's own value, or fixed to `value`, by its
/// own setting or its wall's.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Binding {
    Follows,
    Fixed(String),
}

/// How a panel is bound to Tomori's time: following what the side shows, live over the last `minutes` up to now, or
/// fixed to the window from `from_ms` to `to_ms`, absolute instants in milliseconds since the Unix epoch in UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Time {
    Follows,
    Live { minutes: u32 },
    Window { from_ms: i64, to_ms: i64 },
}

/// How one of the side's panels is bound to its context now, as the server tells the side, its own setting over its
/// wall's: the side's id for the panel, how it is bound to each parameter it depends on, in the order it named them,
/// and to time. The side shows in the panel what its context says, the selection's value where a parameter follows,
/// and the value fixed where one is. See `design/panel-context.md`.
///
/// ```
/// use tomori_environment::{Binding, Context, Time};
///
/// let context = Context::new("latency", vec![("region".into(), Binding::Fixed("EU-WEST1".into()))], Time::Follows);
/// assert_eq!(context.binding("region"), Some(&Binding::Fixed("EU-WEST1".into())));
/// assert_eq!((context.panel(), context.binding("pod"), context.time()), ("latency", None, Time::Follows));
/// assert!(!context.follows());
/// assert!(Context::new("latency", vec![("region".into(), Binding::Follows)], Time::Follows).follows());
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Context {
    panel: String,
    bindings: Vec<(String, Binding)>,
    time: Time,
    of: Option<String>,
}

impl Context {
    /// The panel `panel`, bound to each parameter as `bindings` say and to time as `time` says.
    pub fn new(panel: impl Into<String>, bindings: Vec<(String, Binding)>, time: Time) -> Self {
        Self {
            panel: panel.into(),
            bindings,
            time,
            of: None,
        }
    }

    /// The instance `panel` of the side's panel `of`, an id the server minted, as [`Context::is_instance`] says, bound
    /// as `bindings` and `time` say: the side publishes under `panel` what `of` shows in that context, kept updated, and
    /// retracts it once it has no more use for it. See `design/panel-context.md`.
    ///
    /// ```
    /// use tomori_environment::{Binding, Context, Time};
    ///
    /// let copy = Context::instance("~7", "latency", vec![("region".into(), Binding::Fixed("EU-WEST1".into()))], Time::Follows);
    /// assert_eq!((copy.panel(), copy.of()), ("~7", Some("latency")));
    /// assert_eq!(Context::new("latency", Vec::new(), Time::Follows).of(), None);
    /// ```
    pub fn instance(
        panel: impl Into<String>,
        of: impl Into<String>,
        bindings: Vec<(String, Binding)>,
        time: Time,
    ) -> Self {
        Self {
            of: Some(of.into()),
            ..Self::new(panel, bindings, time)
        }
    }

    /// The side's own panel this is an instance of, by its id, for an instance the server made, as with the agent's
    /// `copy_panel`; none for one of the side's own panels.
    pub fn of(&self) -> Option<&str> {
        self.of.as_deref()
    }

    /// The side's id for the panel, as it published it.
    pub fn panel(&self) -> &str {
        &self.panel
    }

    /// How it is bound to each parameter it depends on, by the parameter's id, in the order it named them.
    pub fn bindings(&self) -> &[(String, Binding)] {
        &self.bindings
    }

    /// How it is bound to the parameter `id`, if it depends on it.
    pub fn binding(&self, id: &str) -> Option<&Binding> {
        self.bindings
            .iter()
            .find(|(parameter, _)| parameter == id)
            .map(|(_, binding)| binding)
    }

    /// How it is bound to time.
    pub fn time(&self) -> Time {
        self.time
    }

    /// Whether it follows everything, as every panel does until a setting fixes something.
    pub fn follows(&self) -> bool {
        self.time == Time::Follows
            && self
                .bindings
                .iter()
                .all(|(_, binding)| *binding == Binding::Follows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_space_made_without_an_outlet_keeps_no_context() {
        let space = crate::Space::new("space-1-1", "Fleet", "/");
        assert_eq!(
            space.set_context(vec![ContextParameter::new("region", "Region", "US-EAST1")]),
            Ok(())
        );
        let parameter =
            ContextParameter::new("region", "Region", "US-EAST1").with_choices(["US-EAST1"]);
        assert_eq!(
            (parameter.id(), parameter.label(), parameter.value()),
            ("region", "Region", "US-EAST1")
        );
        assert_eq!(parameter.choices(), ["US-EAST1"]);
        assert!(valid_context_id("region") && !valid_context_id(TIME));
        let context = Context::new(
            "latency",
            vec![("region".into(), Binding::Fixed("EU-WEST1".into()))],
            Time::Follows,
        );
        assert_eq!(
            (context.panel(), context.bindings().len(), context.time()),
            ("latency", 1, Time::Follows)
        );
        assert_eq!((context.binding("pod"), context.follows()), (None, false));
        let live = Context::new("latency", Vec::new(), Time::Live { minutes: 30 });
        assert!(!live.follows());
        let intent = crate::Intent::context_of(3, context.clone());
        assert_eq!(
            (intent.context(), intent.participant()),
            (Some(&context), 3)
        );
        assert_eq!(crate::Intent::new(3, "{}").context(), None);
        let copy = Context::instance("~7", "latency", Vec::new(), Time::Follows);
        assert_eq!(
            (copy.panel(), copy.of(), context.of()),
            ("~7", Some("latency"), None)
        );
        assert!(
            is_instance("~7") && !is_instance("~") && !is_instance("~a") && !is_instance("latency")
        );
        assert!(!is_instance("~1234567890"));
        let panel = crate::Panel::text("latency", "Latency", "").depends_on(["region"]);
        assert_eq!(panel.depends(), ["region"]);
    }
}
