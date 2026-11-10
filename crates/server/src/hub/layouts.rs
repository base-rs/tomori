// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The tools with which a space's agent composes a layout of walls in one step, to show information laid out, and takes
//! one down again: `create_layout` builds several walls as one snapped group where someone can read it, and says each
//! wall's name and slots, which the agent then fills with `wall NAME slot N`; `remove_layout` removes a group's walls.
//! See `design/projection.md` and `design/mcp.md`.
use serde_json::{Value, json};

use super::Hub;
use crate::stage::{Placement, Planned};

/// How `wall` is described in the result of `create_layout`: the agent's name for it, its identity and name, its grid
/// and its slots, numbered as `presence` numbers them, which a `place` of `wall NAME slot N` fills, their size, and its
/// own.
fn built(
    projection: &tomori_protocol::Projection,
    named: &str,
    wall: &tomori_protocol::Wall,
) -> Value {
    let grid = wall.grid();
    let (width, height) = wall.size();
    let (slot_width, slot_height) = wall.slot_size();
    let round = |value: f32| (f64::from(value) * 100.0).round() / 100.0;
    json!({
        "id": named,
        "wall": wall.id,
        "name": crate::presence::wall_name(projection, wall),
        "columns": grid.columns,
        "rows": grid.rows,
        "slots": (0..grid.slots()).collect::<Vec<u32>>(),
        "slot_size": [round(slot_width), round(slot_height)],
        "size": [round(width), round(height)],
    })
}

impl Hub {
    /// Compose `planned` as one group of walls in the space bound to `session`, as its agent asked: for whoever asked, or
    /// someone `place` names, in front of them, `front` or `near`, or where they are looking, `looking`, the default,
    /// where they can read every slot, as `Stage::compose` places it, or refused whole. Everyone in the space sees it, a
    /// notice says so, and it is kept and logged. Returns each wall's names and slots.
    pub(crate) fn agent_create_layout(
        &self,
        session: &str,
        place: Option<&str>,
        planned: Vec<Planned>,
    ) -> Result<Value, String> {
        let placement = match place {
            None => Placement::Looking(None),
            Some(place) => super::tools::placement(place)?,
        };
        let (name, looking) = match placement {
            // A layout is too large to stand beside someone, so beside them it stands in front of them.
            Placement::Near(name) | Placement::Front(name) | Placement::Beside(name) => {
                (name, false)
            }
            Placement::Looking(name) => (name, true),
            Placement::Wall(..) | Placement::Anchor(_) => {
                return Err("A layout stands in front of someone or where they are looking: say front, near or looking, with a name or for whoever asked.".to_owned());
            }
        };
        let mut state = self.lock();
        let space = self.calling_space(&state, session)?;
        let people = state.heads(&space);
        let record = state.space_mut(&space).expect("found above");
        let environments = self.environments();
        let walls = environments.walls(&record.model.environment);
        let origin = record.model.origin();
        let head = crate::presence::head_of(name.as_deref(), origin.as_ref(), &people)?;
        let middles = |extent: tomori_protocol::group::Extent, radius: Option<f32>| {
            crate::presence::layout_middles(&head, looking, extent.right - extent.left, radius)
        };
        let ids = record
            .stage
            .compose(&walls, &planned, &middles, head.position)?;
        let now = crate::cards::unix_now();
        let after = record.stage.view(&space, &walls, now);
        let described: Vec<Value> = planned
            .iter()
            .zip(&ids)
            .map(|(wall, id)| {
                built(
                    &after,
                    &wall.id,
                    after.wall(id).expect("a wall built is shown"),
                )
            })
            .collect();
        let names: Vec<String> = described
            .iter()
            .map(|wall| {
                format!(
                    "{}, {} by {}",
                    wall["name"].as_str().unwrap_or_default(),
                    wall["columns"],
                    wall["rows"]
                )
            })
            .collect();
        let summary = names.join("; ");
        let content =
            super::tools::notice(format!("The agent built a layout of walls: {summary}."));
        let panel = record
            .stage
            .project_notice(&walls, super::tools::NOTICE, content, now);
        log::info!(
            "The agent of space {space} built a layout of {}; projected {panel}.",
            ids.join(", ")
        );
        state.touch_projection(&space);
        drop(environments);
        drop(state);
        self.keep();
        Ok(json!({
            "space": space,
            "group": ids[0],
            "walls": described,
            "next": "Fill each slot it should hold now, with a panel block whose header says place: wall NAME slot N, a code block with place=\"wall NAME slot N\", or show_image's place, by each wall's name and its slots as listed here; then tell everyone in the space what you built.",
        }))
    }

    /// Remove the walls of the group `group` names in the space bound to `session`, as its agent asked: by any of its
    /// walls' identity or name, or the one someone is looking at or is nearest. Their panels float where they were.
    /// Everyone in the space sees them go, a notice says so, and it is kept and logged.
    pub(crate) fn agent_remove_layout(&self, session: &str, group: &str) -> Result<Value, String> {
        let mut state = self.lock();
        let space = self.calling_space(&state, session)?;
        let people = state.heads(&space);
        let record = state.space_mut(&space).expect("found above");
        let environments = self.environments();
        let walls = environments.walls(&record.model.environment);
        let now = crate::cards::unix_now();
        let projection = record.stage.view(&space, &walls, now);
        let origin = record.model.origin();
        let id = crate::presence::which_wall(group, origin.as_ref(), &people, &projection)?;
        // In the order they are shown, as they are named.
        let mut members = record.stage.group_of(&walls, &id);
        members.sort_by_key(|member| projection.walls.iter().position(|wall| wall.id == *member));
        if members.is_empty() {
            return Err("Only walls someone added can be removed: the Tomori wall and an environment's walls stay.".to_owned());
        }
        // Each wall that may be removed: one that may not, such as one locked, stays.
        let (members, stayed): (Vec<String>, Vec<String>) = members
            .into_iter()
            .partition(|member| record.stage.remove_wall(&walls, member).is_ok());
        if members.is_empty() {
            return Err("Every wall of that layout is locked: ask an owner in the space to unlock them first.".to_owned());
        }
        let stayed: Vec<String> = stayed
            .iter()
            .filter_map(|member| projection.wall(member))
            .map(|wall| crate::presence::wall_name(&projection, wall))
            .collect();
        let stays = match stayed.len() {
            0 => String::new(),
            1 => format!("; {} stays, locked", stayed[0]),
            _ => format!("; {} stay, locked", stayed.join(", ")),
        };
        let names: Vec<String> = members
            .iter()
            .filter_map(|member| projection.wall(member))
            .map(|wall| crate::presence::wall_name(&projection, wall))
            .collect();
        let held = projection
            .panels
            .iter()
            .filter(|panel| matches!(&panel.place, tomori_protocol::Place::Wall { wall, .. } if members.contains(wall)))
            .count();
        let floated = match held {
            0 => String::new(),
            1 => "; the panel on them floats where it was".to_owned(),
            held => format!("; the {held} panels on them float where they were"),
        };
        let removed = names.join(", ");
        let floated = format!("{floated}{stays}");
        let content = super::tools::notice(format!("The agent removed {removed}{floated}."));
        let panel = record
            .stage
            .project_notice(&walls, super::tools::NOTICE, content, now);
        log::info!(
            "The agent of space {space} removed {}; projected {panel}.",
            members.join(", ")
        );
        state.touch_projection(&space);
        drop(environments);
        drop(state);
        self.keep();
        Ok(json!({
            "space": space,
            "removed": members,
            "floating": held,
            "next": format!("Tell everyone in the space that you removed {removed}{floated}."),
        }))
    }
}
