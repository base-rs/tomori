// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The context of an environment's walls and panels: the parameters its side declares, with their values now, each
//! wall's and published panel's settings, following or fixed, a panel's own over its wall's, the chips that show them,
//! and how each panel is bound, which its side is told. Time is always absolute instants in UTC. See
//! `design/panel-context.md`.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tomori_environment::{
    Binding, CONTEXT_CHOICES, CONTEXT_LABEL_CHARS, CONTEXT_PARAMETERS, CONTEXT_VALUE_CHARS, TIME,
    Time,
};
use tomori_protocol::chart::{CHART_EARLIEST_MS, CHART_INSTANT_MS, Chart, TimeAxis};
use tomori_protocol::context::{ContextChip, ContextSetting, ContextValue};

use super::{Kept, Placed, Stage, TOMORI_WALL, Walls};

/// The longest live window a setting gives, in minutes: a week.
pub const LIVE_MINUTES: u32 = 7 * 24 * 60;

/// The widest fixed window a setting gives, in milliseconds: 31 days.
pub const WINDOW_MS: i64 = 31 * 86_400_000;

/// The most settings a wall or a panel holds: one for each parameter a side may declare, and time; those read back from
/// the state file are held to it too.
pub const SETTINGS: usize = CONTEXT_PARAMETERS + 1;

/// A parameter a side declared, cleaned: its id, its label, its value now and the values it may take.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Parameter {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) value: String,
    pub(crate) choices: Vec<String>,
}

/// `text` cleaned onto one line and cut to `limit` characters, as a parameter's label and values are.
fn cleaned(text: &str, limit: usize) -> String {
    tomori_protocol::text::one_line(text.trim(), limit)
        .trim()
        .to_owned()
}

/// The parameters a side declares, `parameters`, cleaned, or why they are refused: more than [`CONTEXT_PARAMETERS`], an
/// id [`tomori_environment::valid_context_id`] refuses or another has, or more than [`CONTEXT_CHOICES`] choices. A
/// label empty once cleaned takes its id.
pub(crate) fn declared(
    parameters: Vec<tomori_environment::ContextParameter>,
) -> Result<Vec<Parameter>, String> {
    if parameters.len() > CONTEXT_PARAMETERS {
        return Err(format!(
            "A side declares at most {CONTEXT_PARAMETERS} parameters."
        ));
    }
    let mut kept: Vec<Parameter> = Vec::new();
    for parameter in parameters {
        if !tomori_environment::valid_context_id(parameter.id()) {
            return Err(format!(
                "A parameter's id is 1 to {} ASCII letters, digits, - and _, and never time.",
                tomori_environment::CONTEXT_ID_BYTES
            ));
        }
        if kept.iter().any(|kept| kept.id == parameter.id()) {
            return Err("Two parameters have the same id.".to_owned());
        }
        if parameter.choices().len() > CONTEXT_CHOICES {
            return Err(format!(
                "A parameter has at most {CONTEXT_CHOICES} choices."
            ));
        }
        let label = cleaned(parameter.label(), CONTEXT_LABEL_CHARS);
        kept.push(Parameter {
            id: parameter.id().to_owned(),
            label: if label.is_empty() {
                parameter.id().to_owned()
            } else {
                label
            },
            value: cleaned(parameter.value(), CONTEXT_VALUE_CHARS),
            choices: parameter
                .choices()
                .iter()
                .map(|choice| cleaned(choice, CONTEXT_VALUE_CHARS))
                .collect(),
        });
    }
    Ok(kept)
}

/// What the parameters a published panel depends on, `depends`, may be, or why not: at most [`CONTEXT_PARAMETERS`] and
/// time, each an id [`tomori_environment::valid_context_id`] takes or [`TIME`], none twice.
pub(crate) fn dependencies(depends: &[String]) -> Result<(), String> {
    if depends.len() > SETTINGS {
        return Err(format!(
            "A panel depends on at most {CONTEXT_PARAMETERS} parameters and time."
        ));
    }
    for (index, parameter) in depends.iter().enumerate() {
        if !(tomori_environment::valid_context_id(parameter) || parameter == TIME) {
            return Err("A panel depends only on parameters by their ids, or time.".to_owned());
        }
        if depends[..index].contains(parameter) {
            return Err("A panel depends on a parameter once.".to_owned());
        }
    }
    Ok(())
}

/// A setting of a wall's or a panel's, fixing one parameter: a value of the side's, or, for time, live over the last
/// `minutes` up to now, or the window from `from_ms` to `to_ms`, absolute instants in milliseconds since the Unix epoch
/// in UTC. A parameter with no setting follows. Kept with the space across restarts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Setting {
    Fixed { value: String },
    Live { minutes: u32 },
    Window { from_ms: i64, to_ms: i64 },
}

/// A wall's or a panel's settings, by their parameters' ids.
pub(crate) type Settings = BTreeMap<String, Setting>;

/// `raw`, settings as the state file holds them, each kept only where it is a setting [`Setting::sound`] takes for its
/// parameter, cleaned as it would be set, at most [`SETTINGS`], so that a state file written by another version or
/// changed by hand never holds a wall or a panel to one the server would refuse.
fn sound_settings(raw: BTreeMap<String, serde_json::Value>) -> Settings {
    raw.into_iter()
        .filter_map(|(parameter, value)| {
            let setting = serde_json::from_value::<Setting>(value).ok()?;
            Some((parameter.clone(), setting.sound(&parameter).ok()?))
        })
        .take(SETTINGS)
        .collect()
}

/// A wall's or a panel's settings, read back from the state file, as [`sound_settings`] keeps them.
pub(crate) fn read<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Settings, D::Error> {
    Ok(sound_settings(BTreeMap::deserialize(deserializer)?))
}

/// Each wall's settings, by the wall's identity, read back from the state file, as [`sound_settings`] keeps them, a
/// wall left with none forgotten.
pub(crate) fn read_walls<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, Settings>, D::Error> {
    let raw: BTreeMap<String, BTreeMap<String, serde_json::Value>> =
        BTreeMap::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .map(|(wall, settings)| (wall, sound_settings(settings)))
        .filter(|(_, settings)| !settings.is_empty())
        .collect())
}

impl Setting {
    /// What a chip says of it.
    fn value(&self) -> ContextValue {
        match self {
            Self::Fixed { value } => ContextValue::Text {
                text: value.clone(),
            },
            Self::Live { minutes } => ContextValue::Live { minutes: *minutes },
            Self::Window { from_ms, to_ms } => ContextValue::Window {
                from_ms: *from_ms,
                to_ms: *to_ms,
            },
        }
    }

    /// It as a setting of `parameter` may be, whatever the side declares, or why not: time only live, from a minute to
    /// [`LIVE_MINUTES`], or a window that ends after it begins, within the years 0 to 9999, from [`CHART_EARLIEST_MS`]
    /// to [`CHART_INSTANT_MS`], and no wider than [`WINDOW_MS`]; another parameter, by an id
    /// [`tomori_environment::valid_context_id`] takes, only a value, cleaned onto one line and cut to
    /// [`CONTEXT_VALUE_CHARS`], not empty. What is read back from the state file is held to this too.
    fn sound(self, parameter: &str) -> Result<Self, &'static str> {
        if parameter == TIME {
            return match self {
                Self::Live { minutes } if (1..=LIVE_MINUTES).contains(&minutes) => Ok(self),
                Self::Live { .. } => Err("A live window is from a minute to a week."),
                Self::Window { from_ms, to_ms }
                    if from_ms < to_ms
                        && from_ms >= CHART_EARLIEST_MS
                        && to_ms <= CHART_INSTANT_MS
                        && to_ms - from_ms <= WINDOW_MS =>
                {
                    Ok(self)
                }
                Self::Window { .. } => Err(
                    "A time window ends after it begins, within the years 0 to 9999, and spans at most 31 days.",
                ),
                Self::Fixed { .. } => Err("Time is fixed to a live or an absolute window."),
            };
        }
        if !tomori_environment::valid_context_id(parameter) {
            return Err("The environment declares no such parameter.");
        }
        let Self::Fixed { value } = self else {
            return Err("Only time has a window.");
        };
        let value = cleaned(&value, CONTEXT_VALUE_CHARS);
        if value.is_empty() {
            return Err("A parameter is fixed to a value.");
        }
        Ok(Self::Fixed { value })
    }

    /// It as the parameter `parameter`, declared as `declared` says, keeps it, or why not: what [`Setting::sound`]
    /// refuses, a parameter its side does not declare now, and a value not among its choices where it declares any.
    pub(crate) fn checked(
        self,
        parameter: &str,
        declared: &[Parameter],
    ) -> Result<Self, &'static str> {
        let setting = self.sound(parameter)?;
        if parameter == TIME {
            return Ok(setting);
        }
        let Some(declared) = declared.iter().find(|declared| declared.id == parameter) else {
            return Err("The environment declares no such parameter.");
        };
        let offered = |choice: &String| {
            setting
                == Self::Fixed {
                    value: choice.clone(),
                }
        };
        if !declared.choices.is_empty() && !declared.choices.iter().any(offered) {
            return Err("That is not one of the parameter's choices.");
        }
        Ok(setting)
    }

    /// How a panel so set is bound to a parameter of its side's.
    fn binding(&self) -> Binding {
        match self {
            Self::Fixed { value } => Binding::Fixed(value.clone()),
            _ => Binding::Follows,
        }
    }

    /// How a panel so set is bound to time.
    fn time(&self) -> Time {
        match self {
            Self::Live { minutes } => Time::Live { minutes: *minutes },
            Self::Window { from_ms, to_ms } => Time::Window {
                from_ms: *from_ms,
                to_ms: *to_ms,
            },
            Self::Fixed { .. } => Time::Follows,
        }
    }
}

/// Whether a setting of `parameter` is shown, and binds, while its side declares `declared`: always for time; for
/// another parameter, only while its side declares it. One that is not is kept, as the side choosing not to declare a
/// parameter is its own to choose, and shown and bound again once it declares it again; the agent is told of it,
/// marked as not shown. A value no longer among a parameter's choices is still shown and bound as it is fixed, so that
/// what a chip shows is what the side is told.
pub(crate) fn shown(parameter: &str, declared: &[Parameter]) -> bool {
    parameter == TIME || declared.iter().any(|declared| declared.id == parameter)
}

/// Set `parameter` in `settings` to `setting`, or let it follow with `None`. Returns whether that changed anything, or
/// why it is refused: a setting [`Setting::checked`] refuses. Since only a declared parameter or time takes one, and a
/// side declares at most [`CONTEXT_PARAMETERS`], a wall or a panel holds at most [`SETTINGS`].
fn set(
    settings: &mut Settings,
    parameter: &str,
    setting: Option<Setting>,
    declared: &[Parameter],
) -> Result<bool, &'static str> {
    let Some(setting) = setting else {
        return Ok(settings.remove(parameter).is_some());
    };
    let setting = setting.checked(parameter, declared)?;
    Ok(settings.insert(parameter.to_owned(), setting.clone()) != Some(setting))
}

/// What a chart's own time axis says a panel following time shows: live over as many minutes as its points span,
/// rounded up, or its fixed window; nothing where it says neither.
fn shown_time(chart: &Chart) -> Option<ContextValue> {
    match chart.time? {
        TimeAxis::Live { step_ms } => Some(ContextValue::Live {
            minutes: (u64::from(chart.points.max(2) - 1) * u64::from(step_ms)).div_ceil(60_000)
                as u32,
        }),
        TimeAxis::Fixed { from_ms, to_ms } => Some(ContextValue::Window { from_ms, to_ms }),
        _ => None,
    }
}

impl Kept {
    /// Whether it depends on time: as a chart, or by naming it.
    fn timed(&self) -> bool {
        self.published.as_ref().is_some_and(|published| {
            published.chart.is_some() || published.depends.iter().any(|depends| depends == TIME)
        })
    }

    /// The wall it is on, if it is on one.
    fn wall(&self) -> Option<&str> {
        match &self.place {
            Placed::Wall { wall, .. } => Some(wall),
            Placed::Free { .. } => None,
        }
    }
}

impl Stage {
    /// Declare `parameters` for `environment`'s side's run, replacing those it declared before. Settings of parameters it
    /// no longer declares are kept, shown nowhere and binding nothing until it declares them again ([`shown`]). Returns
    /// whether that changed them.
    pub(crate) fn declare_parameters(
        &mut self,
        environment: &str,
        parameters: Vec<Parameter>,
    ) -> bool {
        let room = self.room(environment);
        let changed = room.parameters != parameters;
        room.parameters = parameters;
        changed
    }

    /// Every wall's settings in `environment`, by the wall's identity, every one stored, shown here now or not.
    pub(crate) fn wall_contexts(
        &self,
        environment: &str,
    ) -> impl Iterator<Item = (&String, &Settings)> {
        self.rooms
            .get(environment)
            .into_iter()
            .flat_map(|room| room.contexts.iter())
    }

    /// The settings of each panel resting in `environment` that holds any, by its identity and its environment's id for
    /// it, as it comes back with them.
    pub(crate) fn resting_settings(
        &self,
        environment: &str,
    ) -> impl Iterator<Item = (&str, &str, &Settings)> {
        self.rooms
            .get(environment)
            .into_iter()
            .flat_map(|room| room.resting.iter())
            .filter(|resting| !resting.context.is_empty())
            .map(|resting| {
                (
                    resting.panel.as_str(),
                    resting.id.as_str(),
                    &resting.context,
                )
            })
    }

    /// The panel `panel`'s own settings in `environment`, every one stored, if its environment published it.
    pub(crate) fn panel_settings(&self, environment: &str, panel: &str) -> Option<&Settings> {
        self.kept(environment, panel)
            .and_then(|kept| kept.published.as_ref().map(|_| &kept.context))
    }

    /// The parameters `environment`'s side declares now.
    pub(crate) fn parameters(&self, environment: &str) -> &[Parameter] {
        self.rooms
            .get(environment)
            .map_or(&[], |room| room.parameters.as_slice())
    }

    /// Set the wall `wall` in `walls`' environment to `setting` for `parameter`, or let it follow with `None`. Returns
    /// whether that changed anything, or why it is refused: a wall not shown there, the Tomori wall, which holds no
    /// environment's panels, and what [`Setting::checked`] refuses or one setting too many.
    pub(crate) fn set_wall_context(
        &mut self,
        walls: &Walls<'_>,
        wall: &str,
        parameter: &str,
        setting: Option<Setting>,
    ) -> Result<bool, &'static str> {
        if wall == TOMORI_WALL {
            return Err("The Tomori wall holds no environment's panels.");
        }
        if !self.walls(walls).iter().any(|shown| shown.id == wall) {
            return Err("That wall is not here.");
        }
        let room = self.room(walls.environment);
        let declared = room.parameters.clone();
        let settings = room.contexts.entry(wall.to_owned()).or_default();
        let changed = set(settings, parameter, setting, &declared);
        if settings.is_empty() {
            room.contexts.remove(wall);
        }
        changed
    }

    /// Set the panel `panel` shown in `walls`' environment to `setting` for `parameter`, or let it follow with `None`.
    /// Returns whether that changed anything, or why it is refused: a panel not shown there, one its environment did not
    /// publish, a parameter it does not depend on, and what [`Setting::checked`] refuses or one setting too many.
    pub(crate) fn set_panel_context(
        &mut self,
        walls: &Walls<'_>,
        panel: &str,
        parameter: &str,
        setting: Option<Setting>,
    ) -> Result<bool, &'static str> {
        if self.view("", walls, 0).panel(panel).is_none() {
            return Err("That panel is not here.");
        }
        let declared = self.parameters(walls.environment).to_vec();
        let kept = self
            .here_mut(walls.environment, panel)
            .expect("a panel shown is kept");
        let published = kept
            .published
            .as_ref()
            .ok_or("That panel is not its environment's.")?;
        let depends = if parameter == TIME {
            kept.timed()
        } else {
            published.depends.iter().any(|depends| depends == parameter)
        };
        if !depends {
            return Err("That panel does not depend on that parameter.");
        }
        set(&mut kept.context, parameter, setting, &declared)
    }

    /// The chips of the wall `wall`'s context in `environment`: one for each setting of its own that is shown
    /// ([`shown`]), labelled as its side declares the parameter, in the order of their ids.
    pub(crate) fn wall_chips(&self, environment: &str, wall: &str) -> Vec<ContextChip> {
        let Some(room) = self.rooms.get(environment) else {
            return Vec::new();
        };
        room.contexts
            .get(wall)
            .into_iter()
            .flatten()
            .filter(|(parameter, _)| shown(parameter, &room.parameters))
            .map(|(parameter, setting)| {
                let label = room
                    .parameters
                    .iter()
                    .find(|declared| declared.id == *parameter)
                    .map_or("Time", |declared| declared.label.as_str());
                ContextChip::new(parameter, label, setting.value(), ContextSetting::Fixed)
            })
            .collect()
    }

    /// The chips of `kept`'s context in `environment`, a panel its environment published: one for each parameter it
    /// depends on that its side declares, in the order the side declares them, and one for time, where it depends on
    /// time and a setting or its chart says what it shows; each its own setting's value, or its wall's, or else the
    /// side's value now, or what its chart shows.
    pub(super) fn panel_chips(&self, environment: &str, kept: &Kept) -> Vec<ContextChip> {
        let (Some(room), Some(published)) = (self.rooms.get(environment), &kept.published) else {
            return Vec::new();
        };
        let wall = kept.wall().and_then(|wall| room.contexts.get(wall));
        let setting = |parameter: &str| {
            kept.context
                .get(parameter)
                .map(|setting| (setting, ContextSetting::Fixed))
                .or_else(|| {
                    wall.and_then(|wall| wall.get(parameter))
                        .map(|setting| (setting, ContextSetting::Inherited))
                })
        };
        let mut chips: Vec<ContextChip> = room
            .parameters
            .iter()
            .filter(|declared| published.depends.contains(&declared.id))
            .map(|declared| {
                let (value, chosen) = match setting(&declared.id) {
                    Some((setting, chosen)) => (setting.value(), chosen),
                    None => (
                        ContextValue::Text {
                            text: declared.value.clone(),
                        },
                        ContextSetting::Follows,
                    ),
                };
                ContextChip::new(&declared.id, &declared.label, value, chosen)
            })
            .collect();
        if kept.timed() {
            let time = match setting(TIME) {
                Some((setting, chosen)) => Some((setting.value(), chosen)),
                None => published
                    .chart
                    .as_ref()
                    .and_then(shown_time)
                    .map(|value| (value, ContextSetting::Follows)),
            };
            chips.extend(time.map(|(value, chosen)| ContextChip::new(TIME, "Time", value, chosen)));
        }
        chips
    }

    /// How each panel its environment published in `environment` that depends on anything is bound now, by the
    /// environment's id for it: each parameter it depends on that its side declares now ([`shown`]), but time, fixed by
    /// its own setting, or else its wall's,
    /// or following, and time likewise, where it depends on time.
    pub(crate) fn contexts(
        &self,
        environment: &str,
    ) -> BTreeMap<String, tomori_environment::Context> {
        let Some(room) = self.rooms.get(environment) else {
            return BTreeMap::new();
        };
        room.panels
            .iter()
            .filter_map(|kept| {
                let published = kept.published.as_ref()?;
                if published.depends.is_empty() && !kept.timed() {
                    return None;
                }
                let wall = kept.wall().and_then(|wall| room.contexts.get(wall));
                let setting = |parameter: &str| {
                    kept.context
                        .get(parameter)
                        .or_else(|| wall.and_then(|wall| wall.get(parameter)))
                };
                // A parameter its side does not declare now is its own to leave out: it is told no binding for it.
                let bindings = published
                    .depends
                    .iter()
                    .filter(|parameter| *parameter != TIME && shown(parameter, &room.parameters))
                    .map(|parameter| {
                        (
                            parameter.clone(),
                            setting(parameter).map_or(Binding::Follows, Setting::binding),
                        )
                    })
                    .collect();
                let time = if kept.timed() {
                    setting(TIME).map_or(Time::Follows, Setting::time)
                } else {
                    Time::Follows
                };
                Some((
                    published.id.clone(),
                    tomori_environment::Context::new(&published.id, bindings, time),
                ))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
