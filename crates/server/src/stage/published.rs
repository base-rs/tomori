// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Panels an environment's server side publishes onto its own walls, by ids it chooses: text panels and chart panels,
//! kept with the space in that environment as the agent's panels are, moved, pointed at and dismissed as they are, but
//! updated only by their environment. See `design/projection.md` and `design/environments.md`.
use serde::{Deserialize, Serialize};
use tomori_protocol::Chart;

use super::{Content, DECLARED_WALLS, Kept, Placed, Pose, Stage, Walls, blocks};

/// The most panels an environment publishes in one space, as `tomori-environment` exports it.
pub const PUBLISHED: usize = tomori_environment::PANEL_LIMIT;

/// What makes a kept panel one its environment published: the environment's id for it, the reference the agent can
/// inspect it by, and its chart, if it is a chart panel. It is kept only while its environment's run in the space has
/// published it, never written: what the space keeps of it is a [`Resting`] panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Published {
    pub(super) id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) chart: Option<Chart>,
}

/// A panel its environment published that no run of its side has published since the space came back to the
/// environment, the server restarted or its side failed: only where it is and who put it there, kept, and written,
/// so that publishing it again puts it back there, but nothing of what it showed, which would no longer be current.
/// It holds no slot, and does not count towards [`PUBLISHED`]. One that [`RESTING_RUNS`] runs in turn which publish
/// others end without publishing is forgotten, so that a panel a side shows only now and then, as an alert, keeps its
/// place a while, but panels it stopped publishing never pile up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Resting {
    /// Its identity, so that it is the same panel when it comes back.
    pub(super) panel: String,
    /// The environment's id for it.
    pub(super) id: String,
    pub(super) created: u64,
    pub(super) place: Placed,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) pinned: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) arranged: bool,
    /// How many runs of its side that published others have ended without publishing it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub(super) missed: u32,
}

fn is_zero(missed: &u32) -> bool {
    *missed == 0
}

/// How many runs of a side that publish other panels may end without publishing one that rests before it is forgotten.
pub const RESTING_RUNS: u32 = 3;

/// The most panels resting in one environment of a space: those the run that ended published, and as many more from
/// runs before it; beyond that, those missed by the most runs are forgotten first.
pub const RESTING: usize = 2 * PUBLISHED;

/// What rests once a run that published `live` ends, where `earlier` rested before: `live`, and those of `earlier` it
/// did not publish again, each missed by one more run, forgotten after [`RESTING_RUNS`], held to [`RESTING`], those
/// missed by the most runs left out first; or, where the run published nothing, `earlier` as it was.
pub(super) fn rested(mut live: Vec<Resting>, earlier: &[Resting]) -> Vec<Resting> {
    if live.is_empty() {
        return earlier.to_vec();
    }
    live.extend(
        earlier
            .iter()
            .map(|earlier| Resting {
                missed: earlier.missed + 1,
                ..earlier.clone()
            })
            .filter(|earlier| earlier.missed <= RESTING_RUNS),
    );
    live.sort_by_key(|resting| resting.missed);
    live.truncate(RESTING);
    live
}

impl Resting {
    /// What is kept of `kept`, which its environment published as `id`.
    pub(super) fn of(kept: &Kept, id: &str) -> Self {
        Self {
            panel: kept.id.clone(),
            id: id.to_owned(),
            created: kept.created,
            place: kept.place.clone(),
            pinned: kept.pinned,
            arranged: kept.arranged,
            missed: 0,
        }
    }
}

/// A panel a side publishes, cleaned and bounded, as the stage takes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Publishing {
    id: String,
    content: Content,
    chart: Option<Chart>,
    wall: u32,
    slot: Option<u32>,
    reference: Option<String>,
}

impl Publishing {
    /// `panel`, cleaned and bounded as the agent's panels are, its chart cleaned as [`Chart::cleaned`] cleans it and its
    /// body, for a chart panel, the chart's summary; or why it is refused. A reference that
    /// [`tomori_environment::valid_reference`] refuses is left out, as an object's is.
    pub fn new(panel: tomori_environment::Panel) -> Result<Self, String> {
        let (id, title, body, chart, wall, slot, reference) = panel.into_parts();
        if !tomori_environment::valid_panel_id(&id) {
            return Err(format!(
                "A panel's id is 1 to {} ASCII letters, digits, - and _.",
                tomori_environment::PANEL_ID_BYTES
            ));
        }
        let chart = chart.map(Chart::cleaned).transpose()?;
        let body = chart.as_ref().map_or(body, Chart::summary);
        let publishing = Self {
            id,
            content: Content {
                title: blocks::title(&title),
                body: blocks::clean_body(&body),
                meta: Vec::new(),
                code: None,
                slug: None,
                image: None,
            },
            chart,
            wall,
            slot,
            reference: reference.filter(|reference| tomori_environment::valid_reference(reference)),
        };
        Ok(publishing)
    }

    /// How many bytes of JSON its content and chart take.
    pub fn bytes(&self) -> usize {
        super::json_size(&self.content) + self.chart.as_ref().map_or(0, super::json_size)
    }

    /// The environment's id for it.
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// What publishing a panel changed: nothing, as for the same again or one dismissed, what it shows, in place, or which
/// panels are shown, as a new one is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Unchanged,
    Updated,
    Added,
}

impl Stage {
    /// Whether `publishing` could be made in `walls`' environment, whenever it is, or why not: onto a wall the
    /// environment declares, in a slot that wall has, and, for a panel not shown nor among the ids `waiting` to be
    /// published, within the [`PUBLISHED`] an environment may show, counting those waiting. What depends on what else is
    /// shown when it is made, such as a full wall, is not known until then.
    pub(crate) fn admits(
        &self,
        walls: &Walls<'_>,
        publishing: &Publishing,
        waiting: &[&str],
    ) -> Result<(), &'static str> {
        if publishing.wall as usize >= walls.declared.len().min(DECLARED_WALLS) {
            return Err("The environment declares no such wall.");
        }
        let wall = format!("environment-{}", publishing.wall);
        if publishing
            .slot
            .is_some_and(|slot| slot >= self.slots_of(walls, &wall))
        {
            return Err("That wall has no such slot.");
        }
        let shown: Vec<&str> = self
            .rooms
            .get(walls.environment)
            .into_iter()
            .flat_map(|room| &room.panels)
            .filter_map(|kept| {
                kept.published
                    .as_ref()
                    .map(|published| published.id.as_str())
            })
            .collect();
        let id = publishing.id();
        if shown.contains(&id) || waiting.contains(&id) {
            return Ok(());
        }
        let others = waiting
            .iter()
            .filter(|waiting| !shown.contains(waiting))
            .count();
        if shown.len() + others >= PUBLISHED {
            return Err("An environment publishes at most 8 panels in a space.");
        }
        Ok(())
    }

    /// Publish `publishing` at `now` onto the environment's own wall it names in `walls`' environment, in the slot it
    /// prefers if that is free, or else as the agent's panels go, arranged, so that the agent's newcomers make room
    /// before it; or, where the environment published one with its id in this run, update that one in place, keeping
    /// its identity, where it is and its age, as an agent's panel is updated; or, where one with its id is resting, put
    /// it back where it rested, in its slot if that is still there and free, or floating where it floated, or else as a
    /// new one goes. Returns what it changed, or why it was refused: a wall the environment does not declare, a slot it
    /// does not have, a full wall of panels participants arranged and the environment's own, or more than [`PUBLISHED`]
    /// panels. One a participant dismissed stays away, its updates ignored, until the environment retracts it, the space
    /// comes back to the environment or the server restarts.
    pub(crate) fn publish(
        &mut self,
        walls: &Walls<'_>,
        publishing: Publishing,
        now: u64,
    ) -> Result<Change, &'static str> {
        self.admits(walls, &publishing, &[])?;
        let wall = format!("environment-{}", publishing.wall);
        if self.unchanged(walls.environment, &publishing) {
            return Ok(Change::Unchanged);
        }
        let Publishing {
            id,
            content,
            chart,
            slot,
            reference,
            ..
        } = publishing;
        let room = self.room(walls.environment);
        let published = Published {
            id: id.clone(),
            reference,
            chart,
        };
        if let Some(kept) = room.panels.iter_mut().find(|kept| {
            kept.published
                .as_ref()
                .is_some_and(|published| published.id == id)
        }) {
            kept.updated = Some(now);
            kept.content = content;
            kept.published = Some(published);
            return Ok(Change::Updated);
        }
        let resting = room
            .resting
            .iter()
            .position(|resting| resting.id == id)
            .map(|index| room.resting.remove(index));
        let mut kept = Kept {
            id: String::new(),
            content,
            created: now,
            updated: None,
            placed: 0,
            place: Placed::Free {
                pose: Pose::default(),
            },
            pinned: false,
            arranged: false,
            published: Some(published),
        };
        // Back where it rested, where that is still there.
        if let Some(resting) = resting {
            kept.id = resting.panel;
            kept.created = resting.created;
            kept.pinned = resting.pinned;
            match resting.place {
                Placed::Free { pose } => {
                    self.float(walls, kept, pose);
                    return Ok(Change::Added);
                }
                Placed::Wall { wall: rested, slot } => {
                    let list = self.list(walls.environment, Some(&rested));
                    let free = !list
                        .iter()
                        .any(|other| other.slot_on(&rested) == Some(slot));
                    if rested != super::TOMORI_WALL && free && slot < self.slots_of(walls, &rested)
                    {
                        kept.placed = self.tick();
                        kept.arranged = resting.arranged;
                        kept.place = Placed::Wall { wall: rested, slot };
                        self.room(walls.environment).panels.push(kept);
                        return Ok(Change::Added);
                    }
                }
            }
        } else {
            kept.id = format!("panel-{}", self.tick());
        }
        // A full wall makes room only by giving up one of the agent's panels, never one a participant arranged or the
        // environment's own.
        let slots = self.slots_of(walls, &wall);
        let list = self.list(walls.environment, Some(&wall));
        let taken: Vec<&Kept> = list
            .iter()
            .filter(|kept| kept.slot_on(&wall).is_some())
            .collect();
        if taken.len() as u32 >= slots && taken.iter().all(|kept| kept.arranged) {
            return Err("That wall is full.");
        }
        let identity = kept.id.clone();
        self.put_at(walls, kept, &wall, slot);
        self.here_mut(walls.environment, &identity)
            .expect("a panel just put on its wall is kept")
            .arranged = true;
        self.bound(walls.environment);
        Ok(Change::Added)
    }

    /// Whether publishing `publishing` in `environment` would change nothing: the same as the panel shown with its id,
    /// or one a participant dismissed.
    pub(crate) fn unchanged(&self, environment: &str, publishing: &Publishing) -> bool {
        let Some(room) = self.rooms.get(environment) else {
            return false;
        };
        room.dismissed.contains(&publishing.id)
            || room.panels.iter().any(|kept| {
                kept.published.as_ref().is_some_and(|published| {
                    published.id == publishing.id
                        && published.chart == publishing.chart
                        && published.reference == publishing.reference
                }) && kept.content == publishing.content
            })
    }

    /// Take away the panel the environment published as `id` in `environment`, shown or resting, and forget that a
    /// participant dismissed it, so that publishing it again shows it. Returns whether it was shown.
    pub(crate) fn retract(&mut self, environment: &str, id: &str) -> bool {
        let Some(room) = self.rooms.get_mut(environment) else {
            return false;
        };
        room.dismissed.remove(id);
        room.resting.retain(|resting| resting.id != id);
        let before = room.panels.len();
        room.panels.retain(|kept| {
            kept.published
                .as_ref()
                .is_none_or(|published| published.id != id)
        });
        room.panels.len() < before
    }

    /// End the run of the environment's side in `environment`, as the space leaves it or its side stops: what the run
    /// published rests, shown no more until a side publishes it again, and, where it published anything, what rested
    /// before that it did not publish again is missed by one more run, forgotten after [`RESTING_RUNS`], while a run
    /// that published nothing, as one that failed at once, changes nothing. Returns whether that hid any.
    pub(crate) fn rest(&mut self, environment: &str) -> bool {
        let Some(room) = self.rooms.get_mut(environment) else {
            return false;
        };
        let mut resting = Vec::new();
        room.panels.retain(|kept| match &kept.published {
            Some(published) => {
                resting.push(Resting::of(kept, &published.id));
                false
            }
            None => true,
        });
        if resting.is_empty() {
            return false;
        }
        room.resting = rested(resting, &room.resting);
        true
    }

    /// The environment's id for the panel `panel` in `environment`, and the reference the agent can inspect it by, if
    /// its environment published it.
    pub(crate) fn published(&self, environment: &str, panel: &str) -> Option<(&str, Option<&str>)> {
        let published = self.kept(environment, panel)?.published.as_ref()?;
        Some((&published.id, published.reference.as_deref()))
    }

    /// What the agent is told of the panel shown in `environment` that its environment published with `reference`, as
    /// the `inspect` tool describes it: its title and what it shows, cleaned onto one line.
    pub(crate) fn referenced(&self, environment: &str, reference: &str) -> Option<String> {
        self.rooms
            .get(environment)?
            .panels
            .iter()
            .find(|kept| {
                kept.published
                    .as_ref()
                    .is_some_and(|published| published.reference.as_deref() == Some(reference))
            })
            .map(|kept| {
                crate::space::one_line(
                    &format!(
                        "{}: {}",
                        kept.content.title,
                        kept.content.body.replace('\n', "; ")
                    ),
                    blocks::BODY_CHARS,
                )
            })
    }
}

#[cfg(test)]
mod tests;
