// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The tools with which a space's agent reads its panels back and updates one in place: `panels` lists what each shows,
//! with its items, and `update_panel` changes its title or body, or ticks and unticks its items, without projecting a new
//! one. See `design/projection.md` and `design/mcp.md`.
use serde_json::{Value, json};
use tomori_protocol::{Panel, Place, Projection};

use super::Hub;
use crate::stage::checklist;

/// The most items one call ticks and unticks together.
pub(crate) const ITEMS: usize = 30;

/// An item of a panel's body, as the agent names it: by its number, from 1, or by its text.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Item {
    Number(u64),
    Text(String),
}

/// What `update_panel` is asked to change.
#[derive(Debug, Default)]
pub(crate) struct Update {
    /// The panel, by its id, its identity or its title.
    pub panel: String,
    pub title: Option<String>,
    pub body: Option<String>,
    pub check: Vec<Item>,
    pub uncheck: Vec<Item>,
    /// Where it moves, in the forms a `place` header takes.
    pub place: Option<String>,
    /// The panel's version the agent last read, as `panels` gives it: where the panel has changed since, as when a
    /// participant ticked an item, the update is refused, so that an item named by its number is never another.
    pub version: Option<u64>,
}

/// The identity of the panel `panel` names in `projection`: by the id the agent gave it, by its identity, or by its
/// title, which must name only one.
pub(crate) fn find(projection: &Projection, panel: &str) -> Result<String, String> {
    let wanted = panel.trim();
    if let Some(slug) = crate::stage::slug(wanted)
        && let Some(shown) = projection
            .panels
            .iter()
            .find(|shown| shown.slug.as_deref() == Some(slug.as_str()))
    {
        return Ok(shown.id.clone());
    }
    if let Some(shown) = projection.panel(wanted) {
        return Ok(shown.id.clone());
    }
    let titled: Vec<&Panel> = projection
        .panels
        .iter()
        .filter(|shown| crate::presence::title(shown).eq_ignore_ascii_case(wanted))
        .collect();
    match titled.as_slice() {
        [shown] => Ok(shown.id.clone()),
        [] => Err("There is no such panel here.".to_owned()),
        _ => Err("More than one panel here has that title, so name it by its id or identity, as panels lists them.".to_owned()),
    }
}

/// The list items of `body`, as the agent is told them: each one's number, from 1, how deep it is nested, whether it is
/// ticked, for a task item, and its text.
fn items(body: &str) -> Value {
    json!(
        checklist::items(body)
            .iter()
            .enumerate()
            .map(|(index, item)| {
                json!({
                    "item": index + 1,
                    "depth": item.depth,
                    "checked": item.checked,
                    "text": crate::space::one_line(item.text, 120),
                })
            })
            .collect::<Vec<_>>()
    )
}

/// The line of `body` the item `item` names begins on: by its number, or by its text, the one whose text is the same in
/// any case, or else, given digits, the item of that number, or else the only one whose text holds it.
fn line(body: &str, item: &Item) -> Result<usize, String> {
    let items = checklist::items(body);
    match item {
        Item::Number(number) => usize::try_from(*number)
            .ok()
            .and_then(|number| number.checked_sub(1))
            .and_then(|index| items.get(index))
            .map(|item| item.line)
            .ok_or_else(|| {
                format!(
                    "The panel has {} items, numbered from 1, so it has no item {number}.",
                    items.len()
                )
            }),
        Item::Text(text) => {
            let wanted = text.trim().to_lowercase();
            if let Some(found) = items.iter().find(|item| item.text.to_lowercase() == wanted) {
                return Ok(found.line);
            }
            // A number given as text, as a model may give one, names the item of that number.
            if let Ok(number) = wanted.parse::<u64>() {
                return line(body, &Item::Number(number));
            }
            let holding: Vec<usize> = items
                .iter()
                .filter(|item| !wanted.is_empty() && item.text.to_lowercase().contains(&wanted))
                .map(|item| item.line)
                .collect();
            match holding.as_slice() {
                [line] => Ok(*line),
                [] => Err(format!(
                    "No item of the panel says {:?}.",
                    crate::space::one_line(text, 80)
                )),
                _ => Err(format!(
                    "More than one item of the panel says {:?}, so name it by its number.",
                    crate::space::one_line(text, 80)
                )),
            }
        }
    }
}

/// The most bytes `panels` lists a space's panels in, as its result is sent, as structured content and again as text,
/// so that the agent can always read it: about 4,000 tokens of each.
pub(crate) const LISTING_BYTES: usize = 32 * 1024;

/// How many of [`LISTING_BYTES`] are left for what a listing says besides its panels, in both forms, and for the
/// result around them.
const LISTING_ROOM: usize = 1024;

/// How many bytes `value` takes as a tool's result is sent: as its structured content, and again as its text, a JSON
/// string of it, whose quotes and backslashes are escaped. See [`crate::mcp::structured`].
pub(crate) fn sent(value: &Value) -> usize {
    let text = value.to_string();
    text.len() + Value::String(text).to_string().len()
}

/// The most characters of a panel's body `panels` lists it with, on one line.
const EXCERPT_CHARS: usize = 80;

/// What `panels` says of every panel, listed or read: its identity and id, its title, whether it is text, code or a
/// chart, where it is, its age and its version, whether it is one of the space's status panels, and, for an
/// environment's panel, its environment's id for it and the id the agent may inspect it by.
fn summarized(
    projection: &Projection,
    panel: &Panel,
    updated: Option<u64>,
    (published, status): (Option<(&str, Option<&str>)>, bool),
) -> Value {
    let kind = if panel.code.is_some() {
        "code"
    } else if panel.chart.is_some() {
        "chart"
    } else {
        "text"
    };
    let mut described = json!({
        "panel": panel.id,
        "title": crate::presence::title(panel),
        "kind": kind,
        "pinned": panel.pinned,
        "projected_seconds_ago": panel.age,
        "version": panel.version,
    });
    // A status panel says so: it goes with the space to the next environment's status wall.
    if status {
        described["status"] = json!(true);
    }
    // An environment's panel says so, with its environment's id for it and the id the agent may inspect it by.
    if let Some((id, reference)) = published {
        described["environment_panel"] = json!(id);
        described["inspect_id"] = json!(
            reference
                .map(|reference| crate::space::inspectable(&projection.environment, reference))
        );
    }
    if let Some(slug) = &panel.slug {
        described["id"] = json!(slug);
    }
    if let Some(updated) = updated {
        described["updated_seconds_ago"] = json!(updated);
    }
    match &panel.place {
        Place::Wall { wall, slot } => {
            described["wall"] = json!(wall);
            described["wall_name"] = json!(
                projection
                    .wall(wall)
                    .map(|shown| crate::presence::wall_name(projection, shown))
            );
            described["slot"] = json!(slot);
        }
        _ => described["floating"] = json!(true),
    }
    if let Some(code) = &panel.code {
        described["language"] = json!(code.language);
    }
    described
}

/// `panel`, as `panels` lists it among the others: what [`summarized`] says of it, how much it holds, as counts, its
/// lines, its list items and, for an environment's list, its rows, named apart from the items and rows a panel read
/// whole gives, and, with `excerpt`, the start of its text on one line, so that the agent can tell which to read
/// without being sent every panel's text.
fn listed(
    projection: &Projection,
    panel: &Panel,
    updated: Option<u64>,
    published: (Option<(&str, Option<&str>)>, bool),
    rows: Option<usize>,
    excerpt: bool,
) -> Value {
    let mut described = summarized(projection, panel, updated, published);
    described["line_count"] = match &panel.code {
        Some(code) => json!(code.lines),
        None => json!(panel.body.lines().count()),
    };
    if panel.code.is_none() {
        described["item_count"] = json!(checklist::items(&panel.body).len());
        if excerpt {
            described["excerpt"] = json!(crate::space::one_line(&panel.body, EXCERPT_CHARS));
        }
    }
    if let Some(rows) = rows {
        described["row_count"] = json!(rows);
    }
    described
}

/// `panel`, as `panels` describes the one panel the agent names: what [`summarized`] says of it, and what it shows,
/// once: an environment's list its rows, each with its id, the id the agent may inspect it by, whether it is
/// highlighted, its tone, in a word, its value against its limit, from 0 to 1, or null, and its text; any other text
/// or chart panel its body, as participants see it, and its list items; a code panel only its language, as before.
fn detailed(
    projection: &Projection,
    panel: &Panel,
    updated: Option<u64>,
    published: (Option<(&str, Option<&str>)>, bool),
    rows: Option<(&[crate::stage::Row], Option<&str>)>,
) -> Value {
    let mut described = summarized(projection, panel, updated, published);
    match rows {
        Some((rows, highlighted)) => {
            described["rows"] = rows
                .iter()
                .map(|row| {
                    json!({
                        "id": row.id,
                        "inspect_id": row.reference.as_deref().map(|reference| {
                            crate::space::inspectable(&projection.environment, reference)
                        }),
                        "highlighted": highlighted == Some(row.id.as_str()),
                        "choosable": row.choosable,
                        "tone": row.tone_word().unwrap_or("neutral"),
                        "value": row.value.map(|value| (f64::from(value) * 1000.0).round() / 1000.0),
                        "text": row.text,
                    })
                })
                .collect();
        }
        None if panel.code.is_none() => {
            described["body"] = json!(panel.body);
            described["items"] = items(&panel.body);
        }
        None => {}
    }
    described
}

impl Hub {
    /// The panels shown in the space bound to `session`, in its current environment, as the `panels` tool tells its
    /// agent. Without `panel`, they are listed, from the `offset`th in the projection's order, each with where it is,
    /// its version and the start of its text, in at most [`LISTING_BYTES`] as the result is sent: where they would take
    /// more, without the start of their text, and then only as many as fit, saying how many there are in all and the
    /// offset the next begin at, so that the agent pages through them and every panel is listed on some page. With
    /// `panel`, by its id, identity or title, that panel alone with what it shows, its body and items or its rows, so
    /// that the agent can read the current state of a list before changing it. See `design/mcp.md`.
    pub(crate) fn tool_panels(
        &self,
        session: &str,
        panel: Option<&str>,
        offset: usize,
    ) -> Result<Value, String> {
        let state = self.lock();
        let space = self.calling_space(&state, session)?;
        let record = state.space(&space).expect("found above");
        let environments = self.environments();
        let walls = environments.walls(&record.model.environment);
        let now = crate::cards::unix_now();
        let projection = record.versioned(&environments, now);
        let updated = |panel: &Panel| {
            let updated = record.stage.updated(walls.environment, &panel.id);
            updated.map(|updated| now.saturating_sub(updated))
        };
        let published = |panel: &Panel| {
            (
                record.stage.published(walls.environment, &panel.id),
                record.stage.is_status(&panel.id),
            )
        };
        let rows = |panel: &Panel| record.stage.rows(walls.environment, &panel.id);
        let mut told = json!({
            "space": space,
            "environment": record.model.environment,
        });
        if let Some(panel) = panel {
            let id = find(&projection, panel)?;
            let panel = projection.panel(&id).expect("a panel found");
            let detail = detailed(
                &projection,
                panel,
                updated(panel),
                published(panel),
                rows(panel),
            );
            told["panels"] = json!([detail]);
            return Ok(told);
        }
        let total = projection.panels.len();
        let offset = offset.min(total);
        let list = |excerpt: bool| -> Vec<Value> {
            let panels = projection.panels[offset..].iter().map(|panel| {
                let rows = rows(panel).map(|(rows, _)| rows.len());
                listed(
                    &projection,
                    panel,
                    updated(panel),
                    published(panel),
                    rows,
                    excerpt,
                )
            });
            panels.collect()
        };
        // Each as it is sent, in both forms, with the comma after it.
        let size = |panels: &[Value]| panels.iter().map(|panel| sent(panel) + 2).sum::<usize>();
        let budget = LISTING_BYTES - LISTING_ROOM;
        let mut panels = list(true);
        if size(&panels) > budget {
            panels = list(false);
        }
        // As many as fit, the next in the projection's order: at least one, since each is far shorter than the room.
        let (mut bytes, mut fit) = (0, 0);
        for panel in &panels {
            bytes += sent(panel) + 2;
            if bytes > budget {
                break;
            }
            fit += 1;
        }
        panels.truncate(fit);
        told["panels"] = json!(panels);
        told["total"] = json!(total);
        told["offset"] = json!(offset);
        // Status panels the status wall has no slot for wait, kept, and take slots as they free.
        let waiting: Vec<Value> = record
            .stage
            .waiting(&walls)
            .into_iter()
            .map(|(panel, title)| json!({ "panel": panel, "title": title, "status": true }))
            .collect();
        if !waiting.is_empty() {
            told["waiting_for_a_slot"] = json!(waiting);
        }
        if offset + fit < total {
            let next = offset + fit;
            told["next"] = json!(next);
            told["note"] = json!(format!(
                "Panels {} to {} of {total} are listed. List those after with offset {next}, or read one whole by naming it in panel, by its id, identity or title.",
                offset + 1,
                next
            ));
        }
        Ok(told)
    }

    /// Update the text panel `update.panel` names, in the space bound to `session`, in place, as its agent asked: its
    /// title and body replaced where given, then the items named ticked and unticked, and then, where `place` says, moved,
    /// unless a participant put it where it is or is carrying it. Everyone in the space sees the change. See `design/projection.md`.
    pub(crate) fn agent_update_panel(
        &self,
        session: &str,
        update: Update,
    ) -> Result<Value, String> {
        let placement = match &update.place {
            Some(place) => Some(crate::stage::Placement::parse(place).ok_or_else(|| {
                format!(
                    "Tomori does not know where {:?} is: say near NAME, looking NAME, front NAME, beside NAME or wall WALL, or near, looking, front or beside for whoever asked.",
                    crate::space::one_line(place, 80)
                )
            })?),
            None => None,
        };
        if update.title.is_none()
            && update.body.is_none()
            && update.check.is_empty()
            && update.uncheck.is_empty()
            && placement.is_none()
        {
            return Err("Say what to change: title, body, check, uncheck or place.".to_owned());
        }
        let mut state = self.lock();
        let space = self.calling_space(&state, session)?;
        let people = state.heads(&space);
        let origin = state.asking(&space);
        let carried = state.carried(&space, std::time::Instant::now());
        let record = state.space_mut(&space).expect("found above");
        let environments = self.environments();
        let walls = environments.walls(&record.model.environment);
        let now = crate::cards::unix_now();
        let projection = record.versioned(&environments, now);
        let id = find(&projection, &update.panel)?;
        if record.stage.published(walls.environment, &id).is_some() {
            return Err(
                "That panel is its environment's: only the environment updates it.".to_owned(),
            );
        }
        let mut content = record
            .stage
            .content(&walls, &id)
            .cloned()
            .ok_or("A code panel cannot be updated in place: project a new one.")?;
        // Changed since the version the agent read, the update is refused, rather than tick another item than it named.
        let current = projection.panel(&id).expect("a panel found").version;
        if let Some(read) = update.version
            && read != current
        {
            return Err(format!(
                "That panel has changed since version {read}, as someone ticked an item, moved it or its text was updated; it is at version {current} now. Read it again with panels, naming it, and redo the update from what it shows, with version {current} if it still applies."
            ));
        }
        if let Some(title) = &update.title {
            content.title = crate::stage::clean_title(title);
        }
        if let Some(body) = &update.body {
            content.body = crate::stage::clean_body(body);
        }
        // Each item is found in the body as it stands, its title and body replaced, and none both ticked and unticked.
        let ticked = update
            .check
            .iter()
            .map(|item| line(&content.body, item))
            .collect::<Result<Vec<_>, _>>()?;
        let unticked = update
            .uncheck
            .iter()
            .map(|item| line(&content.body, item))
            .collect::<Result<Vec<_>, _>>()?;
        if ticked.iter().any(|line| unticked.contains(line)) {
            return Err("An item cannot be both checked and unchecked.".to_owned());
        }
        for (lines, checked) in [(ticked, true), (unticked, false)] {
            for line in lines {
                content.body =
                    checklist::set(&content.body, line, checked).expect("an item is on its line");
            }
        }
        // A box given to an item that had none makes the body longer, so it is bounded again.
        content.body = crate::stage::clean_body(&content.body);
        if content.title.is_empty() && content.body.is_empty() {
            return Err("A panel needs a title or a body.".to_owned());
        }
        let to = match &placement {
            Some(placement) => Some(record.panel_destination(
                &environments,
                placement,
                0,
                origin.as_ref(),
                &people,
                &projection,
                &environments.solids(&record.model.environment),
            )?),
            None => None,
        };
        let pinned = projection.panel(&id).is_some_and(|panel| panel.pinned);
        // A panel someone is carrying stays in their hand, as `move_panel` leaves it: its text changes, and it does not
        // move.
        let carried = carried.contains(&id);
        let to = to.filter(|_| !carried);
        let changed = record
            .stage
            .revise(&walls, &id, content, to.as_ref(), now)
            .expect("a text panel shown is updated");
        let shown = record.versioned(&environments, now);
        let panel = shown.panel(&id).expect("a panel updated is shown");
        let mut result = json!({
            "space": space,
            "panel": id,
            "changed": changed,
            "place": panel.place,
            "items": items(&panel.body),
            "version": panel.version,
        });
        if let Some(slug) = &panel.slug {
            result["id"] = json!(slug);
        }
        if carried && placement.is_some() {
            result["stayed"] = json!(
                "Someone is carrying that panel now, so it stays in their hand; its text changed."
            );
        } else if to.is_some() && pinned {
            result["stayed"] = json!(
                "A participant put that panel where it is, so it stays there; its text changed."
            );
        }
        if changed {
            log::info!("The agent of space {space} updated {id} in place.");
            state.touch_projection(&space);
            drop(state);
            self.keep();
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_is_named_by_its_number_or_its_text() {
        let body = "- [ ] Pairing window\n- [x] Panel ids\n  - [ ] Pairing code\nNotes";
        assert_eq!(line(body, &Item::Number(1)), Ok(0));
        assert_eq!(line(body, &Item::Number(3)), Ok(2));
        assert!(
            line(body, &Item::Number(0))
                .unwrap_err()
                .contains("no item 0")
        );
        assert!(
            line(body, &Item::Number(4))
                .unwrap_err()
                .contains("has 3 items")
        );
        assert!(line(body, &Item::Number(u64::MAX)).is_err());
        // The same text in any case, else the only item holding it.
        assert_eq!(line(body, &Item::Text("panel IDS".into())), Ok(1));
        assert_eq!(line(body, &Item::Text("window".into())), Ok(0));
        assert!(
            line(body, &Item::Text("pairing".into()))
                .unwrap_err()
                .contains("More than one")
        );
        assert!(
            line(body, &Item::Text("notes".into()))
                .unwrap_err()
                .contains("No item")
        );
        assert!(line(body, &Item::Text(" ".into())).is_err());
        // Digits name the item of that number, unless an item says just that.
        assert_eq!(line(body, &Item::Text(" 2 ".into())), Ok(1));
        assert!(
            line(body, &Item::Text("7".into()))
                .unwrap_err()
                .contains("no item 7")
        );
        assert_eq!(line("- 2026\n- 1", &Item::Text("1".into())), Ok(1));
    }
}
