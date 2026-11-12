// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The tools with which a space's agent reads and sets the context of its environment's walls and panels: `contexts`
//! lists the parameters the environment declares, every setting each wall and panel holds, those not shown marked, and
//! what each panel shows; `set_context` fixes a parameter, or time, on a wall or a panel, or lets it follow. Time is
//! always absolute instants in UTC. See `design/panel-context.md` and `design/mcp.md`.
use serde_json::{Value, json};
use tomori_protocol::context::{ContextChip, ContextSetting, ContextValue};

use super::Hub;
use super::agent_panels::{LISTING_BYTES, sent};

/// How many of [`LISTING_BYTES`] are left for what `contexts` says besides its walls and panels, in both forms, and for
/// the result around them.
const LISTING_ROOM: usize = 1024;
use crate::stage::context::{Setting, Settings, shown};

/// What a setting is told as, missing from the agent's `contexts` unless set: the parameter, and its value, its live
/// minutes or its window's ends, written as instants in UTC.
fn setting_json(parameter: &str, setting: &Setting) -> Value {
    let mut told = json!({ "parameter": parameter });
    match setting {
        Setting::Fixed { value } => told["value"] = json!(value),
        Setting::Live { minutes } => told["live_minutes"] = json!(minutes),
        Setting::Window { from_ms, to_ms } => {
            told["from"] = json!(instant(*from_ms));
            told["to"] = json!(instant(*to_ms));
        }
    }
    told
}

/// The instant `ms`, milliseconds since the Unix epoch, written in UTC, as `2026-11-12T02:20:00Z`.
pub(crate) fn instant(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms).map_or_else(|_| ms.to_string(), |at| at.to_string())
}

/// The instant `text` says, in milliseconds since the Unix epoch, which must say its offset from UTC, as
/// `2026-11-12T02:20:00Z` or `2026-11-12T13:20:00+11:00`, so that it is absolute; or why not.
pub(crate) fn parse_instant(text: &str, name: &str) -> Result<i64, String> {
    let text = text.trim();
    // A year past 9999, which no window reaches, written with more digits, as `10000-` or `+010000-`, says so.
    let year = text
        .trim_start_matches('+')
        .split('-')
        .next()
        .unwrap_or_default();
    if year.bytes().all(|byte| byte.is_ascii_digit())
        && year.parse::<u64>().is_ok_and(|year| year > 9999)
    {
        return Err(format!(
            "{name} is past the year 9999: a time window is within the years 0 to 9999."
        ));
    }
    text.parse::<jiff::Timestamp>()
        .map(|at| at.as_millisecond())
        .map_err(|_| {
            format!(
                "{name} is an instant with its offset from UTC, such as 2026-11-12T02:20:00Z, never a time of day alone."
            )
        })
}

/// What setting `parameter` to `setting`, or letting it follow with `None`, did, as a notice and the log say it: `region
/// to EU-WEST1`, `time to the last 30 minutes`, `time to 2026-11-12T02:20:00Z to 2026-11-12T02:50:00Z, UTC` or `region to
/// follow again`.
fn described(parameter: &str, setting: Option<&Setting>) -> String {
    match setting {
        Some(Setting::Fixed { value }) => format!("{parameter} to {value}"),
        Some(Setting::Live { minutes }) => format!("{parameter} to the last {minutes} minutes"),
        Some(Setting::Window { from_ms, to_ms }) => format!(
            "{parameter} to {} to {}, UTC",
            instant(*from_ms),
            instant(*to_ms)
        ),
        None => format!("{parameter} to follow again"),
    }
}

/// The kind of Tomori's notice that the agent set a context, one replacing the last.
const NOTICE: &str = "context";

/// Tomori's notice of a context the agent set, saying `body`.
fn notice(body: String) -> crate::stage::Content {
    crate::stage::Content {
        title: "Context".to_owned(),
        body,
        meta: vec![("source".to_owned(), "Tomori".to_owned())],
        code: None,
        slug: None,
        image: None,
        diagram: None,
        warnings: Vec::new(),
    }
}

/// What `contexts` says where it leaves out walls and panels to fit [`LISTING_BYTES`].
fn cut(panels: usize, walls: usize) -> String {
    format!(
        "{panels} panels and {walls} walls are left out to fit; read them by setting what you need with set_context, or ask fewer to be set."
    )
}

/// Every setting in `settings`, each with whether it is shown, and, where it is not, why.
fn settings_json(settings: &Settings, declared: &[crate::stage::context::Parameter]) -> Vec<Value> {
    settings
        .iter()
        .map(|(parameter, setting)| {
            let mut told = setting_json(parameter, setting);
            let visible = shown(parameter, declared);
            told["shown"] = json!(visible);
            if !visible {
                told["note"] = json!("not shown: the environment doesn't declare it now");
            }
            told
        })
        .collect()
}

/// What a chip's `value` is, as a sentence says it: its value, the last so many minutes, or its window as instants in
/// UTC; nothing for a kind this server does not know.
pub(crate) fn spoken(value: &ContextValue) -> String {
    match value {
        ContextValue::Text { text } => text.clone(),
        ContextValue::Live { minutes } => format!("the last {minutes} minutes"),
        ContextValue::Window { from_ms, to_ms } => {
            format!("{} to {}, UTC", instant(*from_ms), instant(*to_ms))
        }
        _ => String::new(),
    }
}

/// What a panel's chip says, as the agent is told it: the parameter, its label, its value, its live minutes or its
/// window's ends in UTC, and whether it follows, is fixed by the panel's own setting or inherited from its wall's.
fn chip_json(chip: &ContextChip) -> Value {
    let mut told = json!({ "parameter": chip.parameter, "label": chip.label });
    match &chip.value {
        ContextValue::Text { text } => told["value"] = json!(text),
        ContextValue::Live { minutes } => told["live_minutes"] = json!(minutes),
        ContextValue::Window { from_ms, to_ms } => {
            told["from"] = json!(instant(*from_ms));
            told["to"] = json!(instant(*to_ms));
        }
        _ => {}
    }
    told["setting"] = json!(match chip.setting {
        ContextSetting::Fixed => "fixed",
        ContextSetting::Inherited => "inherited",
        _ => "follows",
    });
    told
}

/// Where `set_context` sets a parameter: on a wall, as the agent names one, or on a panel, as it names one.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Target {
    Wall(String),
    Panel(String),
}

impl Hub {
    /// The `contexts` tool: the parameters the environment of the calling session's space declares now, every setting
    /// of each of its walls and panels that holds any, those not shown marked so, and the context each panel that
    /// depends on any shows now.
    pub(crate) fn tool_contexts(&self, session: &str) -> Result<Value, String> {
        let state = self.lock();
        let space = self.calling_space(&state, session)?;
        let record = state.space(&space).expect("found above");
        let environments = self.environments();
        let environment = record.model.environment.as_str();
        let projection = record.versioned(&environments, crate::cards::unix_now());
        let stage = &record.stage;
        let declared = stage.parameters(environment);
        let parameters: Vec<Value> = declared
            .iter()
            .map(|parameter| {
                json!({
                    "id": parameter.id,
                    "label": parameter.label,
                    "value": parameter.value,
                    "choices": parameter.choices,
                })
            })
            .collect();
        // Every wall holding a setting, shown here or not, as one the environment no longer declares.
        let mut walls: Vec<Value> = stage
            .wall_contexts(environment)
            .map(|(id, settings)| {
                let shown = projection.wall(id);
                let mut told = json!({
                    "wall": shown.map_or_else(|| id.clone(), |wall| crate::presence::wall_name(&projection, wall)),
                    "id": id,
                    "settings": settings_json(settings, declared),
                });
                if shown.is_none() {
                    told["shown"] = json!(false);
                    told["note"] = json!("not shown: the environment doesn't show this wall now");
                }
                told
            })
            .collect();
        let mut panels: Vec<Value> = projection
            .panels
            .iter()
            .filter_map(|panel| {
                // One its environment published that shows and holds nothing of a context is left out.
                let settings = stage
                    .panel_settings(environment, &panel.id)
                    .filter(|settings| !(panel.context.is_empty() && settings.is_empty()))?;
                let mut told = json!({
                    "panel": panel.id,
                    "title": crate::presence::title(panel),
                    "settings": settings_json(settings, declared),
                    "shows": panel.context.iter().map(chip_json).collect::<Vec<_>>(),
                });
                if let Some(of) = stage.instance_of(environment, &panel.id) {
                    told["instance_of"] = json!(of);
                }
                Some(told)
            })
            .collect();
        // And each resting, as its side has not published it in this run, with the settings it comes back with.
        panels.extend(
            stage
                .resting_settings(environment)
                .map(|(panel, id, settings, of)| {
                    let mut told = json!({
                        "panel": panel,
                        "id": id,
                        "resting": true,
                        "note": "resting: its environment has not shown it since it last ran",
                        "settings": settings_json(settings, declared),
                    });
                    if let Some(of) = of {
                        told["instance_of"] = json!(of);
                    }
                    told
                }),
        );
        let mut told = json!({
            "space": space,
            "environment": environment,
            "parameters": parameters,
            "walls": walls,
            "panels": panels,
        });
        // Within its bound as it is sent, the last panels and then the last walls left out, saying how many.
        let (mut left_panels, mut left_walls) = (0, 0);
        while sent(&told) + LISTING_ROOM > LISTING_BYTES && !(panels.is_empty() && walls.is_empty())
        {
            if panels.pop().is_some() {
                left_panels += 1;
            } else {
                walls.pop();
                left_walls += 1;
            }
            told["walls"] = json!(walls);
            told["panels"] = json!(panels);
            told["cut"] = json!(cut(left_panels, left_walls));
        }
        Ok(told)
    }

    /// The `set_context` tool: set `parameter` on the wall or panel `target` names in the calling session's space to
    /// `setting`, or let it follow with `None`, for everyone in it, its side told of each panel whose binding changes.
    pub(crate) fn agent_set_context(
        &self,
        session: &str,
        target: Target,
        parameter: &str,
        setting: Option<Setting>,
    ) -> Result<Value, String> {
        let mut state = self.lock();
        let space = self.calling_space(&state, session)?;
        let people = state.heads(&space);
        let origin = state.asking(&space);
        let record = state.space_mut(&space).expect("found above");
        let environments = self.environments();
        let walls = environments.walls(&record.model.environment);
        let projection = record.versioned(&environments, crate::cards::unix_now());
        let (named, changed) = match &target {
            Target::Wall(wall) => {
                let id = crate::presence::which_wall(wall, origin.as_ref(), &people, &projection)?;
                let shown = projection.wall(&id).expect("a wall named is shown");
                let named = crate::presence::wall_name(&projection, shown);
                let changed = record
                    .stage
                    .set_wall_context(&walls, &id, parameter, setting.clone())
                    .map_err(str::to_owned)?;
                (json!({ "wall": named, "id": id }), changed)
            }
            Target::Panel(panel) => {
                let id = super::agent_panels::find(&projection, panel)?;
                let title =
                    crate::presence::title(projection.panel(&id).expect("a panel found is shown"));
                let changed = record
                    .stage
                    .set_panel_context(&walls, &id, parameter, setting.clone())
                    .map_err(str::to_owned)?;
                (json!({ "panel": id, "title": title }), changed)
            }
        };
        if changed {
            let what = match &target {
                Target::Wall(_) => format!(
                    "{} of {}",
                    described(parameter, setting.as_ref()),
                    named["wall"].as_str().unwrap_or_default()
                ),
                Target::Panel(_) => format!(
                    "{} of the panel \"{}\"",
                    described(parameter, setting.as_ref()),
                    crate::space::one_line(named["title"].as_str().unwrap_or_default(), 80)
                ),
            };
            let content = notice(format!("The agent set {what}."));
            let panel =
                record
                    .stage
                    .project_notice(&walls, NOTICE, content, crate::cards::unix_now());
            log::info!("The agent of space {space} set {what}; projected {panel}.");
            drop(environments);
            state.touch_projection(&space);
            drop(state);
            self.keep();
        }
        let mut result = named;
        result["space"] = json!(space);
        result["changed"] = json!(changed);
        result["setting"] = match &setting {
            Some(setting) => setting_json(parameter, setting),
            None => json!({ "parameter": parameter, "follows": true }),
        };
        result["next"] = json!(if changed {
            "Everyone sees it on the chips, and a notice on the status wall says what you set; tell everyone in the space in a sentence what you set."
        } else {
            "It was already so, so nothing changed and there is nothing to tell."
        });
        Ok(result)
    }
}

impl Hub {
    /// Take away, in every space, each copy of a panel its environment withdrew longer ago than
    /// `stage::published::WITHDRAWN_SECS` before `now`, in seconds since the epoch, its side told so.
    pub(crate) fn sweep_withdrawn(&self, now: u64) {
        let environments = self.environments();
        let mut state = self.lock();
        let ids: Vec<String> = state
            .spaces
            .iter()
            .map(|record| record.model.id.clone())
            .collect();
        for space in ids {
            let record = state.space_mut(&space).expect("listed above");
            let environment = record.model.environment.clone();
            if record.stage.lapse_withdrawn(&environment, now) {
                record.stage.settle(&environments.walls(&environment));
                log::info!("Copies of withdrawn panels in space {space} were taken away.");
                state.touch_projection(&space);
            }
        }
    }

    /// The `copy_panel` tool: copy the panel `panel` names in the calling session's space as an instance of it with
    /// `settings` of its own, to where `place` says, as `move_panel` takes it, or else the wall it is on, or beside
    /// whoever asked where it floats, for everyone in it, a notice saying so and its side told of the instance.
    pub(crate) fn agent_copy_panel(
        &self,
        session: &str,
        panel: &str,
        place: Option<&str>,
        settings: Vec<(String, Setting)>,
    ) -> Result<Value, String> {
        let mut state = self.lock();
        let space = self.calling_space(&state, session)?;
        let people = state.heads(&space);
        let origin = state.asking(&space);
        let record = state.space_mut(&space).expect("found above");
        let environments = self.environments();
        let walls = environments.walls(&record.model.environment);
        let projection = record.versioned(&environments, crate::cards::unix_now());
        let id = super::agent_panels::find(&projection, panel)?;
        let shown = projection.panel(&id).expect("a panel found is shown");
        let title = crate::presence::title(shown);
        let to = match (place, &shown.place) {
            (None, tomori_protocol::Place::Wall { wall, .. }) => crate::stage::Destination::Wall {
                wall: wall.clone(),
                slot: None,
            },
            (place, _) => {
                let placement = super::tools::placement(place.unwrap_or("beside"))?;
                record.panel_destination(
                    &environments,
                    &placement,
                    0,
                    super::anchors::Surroundings {
                        asker: origin.as_ref(),
                        people: &people,
                        projection: &projection,
                        solids: &record.panel_solids(&environments),
                    },
                )?
            }
        };
        let described: Vec<String> = settings
            .iter()
            .map(|(parameter, setting)| described(parameter, Some(setting)))
            .collect();
        let (copy, instance) = record
            .stage
            .copy_panel(&walls, &id, settings, &to)
            .map_err(str::to_owned)?;
        let title = crate::space::one_line(&title, 80);
        // A copy is made only with a setting.
        let with = format!(", with {}", described.join(" and "));
        let content = notice(format!("The agent copied the panel \"{title}\"{with}."));
        let now = crate::cards::unix_now();
        let notice = record.stage.project_notice(&walls, NOTICE, content, now);
        log::info!(
            "The agent of space {space} copied {id} as {copy}, instance {instance}{with}; projected {notice}."
        );
        let after = record.stage.view(&space, &walls, now);
        let place = json!(after.panel(&copy).map(|panel| &panel.place));
        drop(environments);
        state.touch_projection(&space);
        drop(state);
        self.keep();
        Ok(json!({
            "space": space,
            "panel": copy,
            "instance": instance,
            "of": id,
            "title": title,
            "place": place,
            "next": "Everyone sees the copy and its chips, and a notice on the status wall says what you copied; tell everyone in the space in a sentence what you copied, and where.",
        }))
    }
}

#[cfg(test)]
mod tests;
