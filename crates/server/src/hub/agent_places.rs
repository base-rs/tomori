// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent placing its space's workers where their work is, with `place_worker`: at a cluster anchor, by the object
//! one stands by, beside a wall or beside a person, or back home; kept with the space for its environment, within the
//! limits that keep workers from drifting, a participant's placement winning over the agent's. See `design/mcp.md`.
use std::time::Instant;

use serde_json::{Value, json};

use super::provenance::Needs;
use super::{Hub, HubState};
use crate::places::Placement;
use crate::seating::Member;

/// Where `place_worker` puts a worker.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    /// `anchor NAME`: in that anchor's cluster.
    Anchor(String),
    /// `object ID`: in the cluster of the anchor that object stands by.
    Object(String),
    /// `wall WALL`: just beyond the end of that wall nearer whoever asked.
    Wall(String),
    /// `near [NAME]` or `beside [NAME]`: to that person's right, or whoever asked's.
    Near(Option<String>),
    /// `home`: back home.
    Home,
}

/// The longest place taken, in characters, after which it is cut.
const PLACE_CHARS: usize = 120;

/// Where `place` says, as the agent gives it to `place_worker`, or why Tomori does not know it.
fn target(place: &str) -> Result<Target, String> {
    let value = tomori_protocol::text::one_line(place, PLACE_CHARS);
    let value = value.trim();
    let (form, rest) = match value.find(|c: char| c == ':' || c.is_whitespace()) {
        Some(at) => (&value[..at], value[at..].trim_start_matches(':').trim()),
        None => (value, ""),
    };
    let named = (!rest.is_empty()).then(|| rest.to_owned());
    let known = match (form.to_ascii_lowercase().as_str(), named) {
        ("home", None) => Some(Target::Home),
        ("anchor", Some(name)) => Some(Target::Anchor(name)),
        ("object", Some(id)) => Some(Target::Object(id)),
        ("wall", Some(wall)) => Some(Target::Wall(wall)),
        ("near" | "beside", named) => Some(Target::Near(named)),
        _ => None,
    };
    known.ok_or_else(|| {
        format!(
            "Tomori does not know where {:?} is: say anchor NAME, object ID, wall WALL, near NAME or beside NAME, near or beside for whoever asked, or home.",
            crate::space::one_line(place, 80)
        )
    })
}

/// Up to three of `names` nearest `wanted`: those sharing the longest beginning with it, then those containing it.
fn nearest<'a>(wanted: &str, names: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let common = |name: &str| {
        name.bytes()
            .zip(wanted.bytes())
            .take_while(|(a, b)| a.eq_ignore_ascii_case(b))
            .count()
    };
    let mut scored: Vec<(usize, bool, &str)> = names
        .map(|name| (common(name), name.contains(wanted), name))
        .filter(|(common, contains, _)| *common > 0 || *contains)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(b.2)));
    scored.into_iter().take(3).map(|(.., name)| name).collect()
}

impl Hub {
    /// Place the worker `worker`, by its session or its title, of the space bound to `session`, where `place` says, as
    /// its agent asked: at an anchor, by the object an anchor stands by, beside a wall or beside someone, or back home,
    /// forgetting where it was placed. Kept with the space for its environment, and logged. Refused, saying why, for a
    /// worker not the space's, or finished; an anchor, object, wall or person not here; one a participant placed,
    /// unless `requested_by` names whoever's words asked, checked as for the gated tools, when it moves and stays theirs,
    /// with a notice saying what moved; and, of the agent's own accord, beyond the limits [`crate::places::Rates`]
    /// keeps. Returns where it stands now. See `design/mcp.md`.
    pub(crate) fn agent_place_worker(
        &self,
        session: &str,
        worker: &str,
        place: &str,
        requested_by: Option<&str>,
    ) -> Result<Value, String> {
        let target = target(place)?;
        let mut state = self.lock();
        let space = self.calling_space(&state, session)?;
        let asked = match requested_by {
            Some(_) => Some(self.requester(&state, &space, requested_by, Needs::Participant)?),
            None => None,
        };
        let placement = self.placement(&state, &space, worker, target)?;
        let found = placement.session.clone();
        let record = state.space_mut(&space).expect("found above");
        let previous = record
            .placements()
            .iter()
            .find(|placement| placement.session == found)
            .cloned();
        let by = previous.as_ref().and_then(|previous| previous.by.clone());
        if let Some(by) = by.as_deref().filter(|_| asked.is_none()) {
            return Err(format!(
                "{by} placed that worker where it is, so leave it there; move it only when someone asks, naming them in requested_by."
            ));
        }
        if asked.is_none() {
            record.rates.place(&found, Instant::now())?;
        }
        let at = placement.at.clone();
        let home = at == tomori_protocol::cluster::HOME;
        record.placements_mut().retain(|kept| kept.session != found);
        if !home {
            // A participant's placement stays theirs where it goes.
            crate::places::keep(
                record.placements_mut(),
                Placement {
                    by: by.clone(),
                    ..placement
                },
            );
        }
        let environments = self.environments();
        record.settle(&environments);
        let anchors = record.clusters(&environments);
        let position = record
            .seated(&environments)
            .position(&anchors, &Member::Worker(found.clone()))
            .map(|(at, _)| at);
        let title = record
            .model
            .workers()
            .iter()
            .find(|known| known.id == found)
            .map_or_else(String::new, |known| known.title.clone());
        match &asked {
            Some(asker) => log::info!(
                "The agent of space {space} placed worker {found} at {at}, as {} asked.",
                asker.logged()
            ),
            None => log::info!("The agent of space {space} placed worker {found} at {at}."),
        }
        // Someone placed it where it was, so everyone is told what moved, and where.
        if let Some(by) = by {
            let walls = environments.walls(&record.model.environment);
            let content = crate::stage::Content {
                title: "Moved".to_owned(),
                body: format!(
                    "Tomori moved worker '{}', which {} placed, to {}.",
                    crate::space::one_line(&title, 48),
                    crate::space::one_line(&by, 40),
                    if home { "home" } else { &at }
                ),
                ..super::tools::notice(String::new())
            };
            record
                .stage
                .project_notice(&walls, "moved", content, crate::cards::unix_now());
            state.touch_projection(&space);
        }
        state.touch_space(&space);
        drop(state);
        self.keep();
        Ok(json!({
            "space": space,
            "worker": found,
            "title": title,
            "at": at,
            "position": position,
            "say": "Say where you put it.",
        }))
    }

    /// Where `target` places the worker `worker`, by its session or title, of the space `space`, as its agent asked: the
    /// placement it makes, at `home` for home; or why not.
    fn placement(
        &self,
        state: &HubState,
        space: &str,
        worker: &str,
        target: Target,
    ) -> Result<Placement, String> {
        let record = state.space(space).expect("the caller found it");
        let workers = record.model.workers();
        let known = match workers.iter().find(|known| known.id == worker) {
            Some(known) => known,
            None => {
                let titled: Vec<_> = workers
                    .iter()
                    .filter(|known| known.title.trim() == worker.trim())
                    .collect();
                match titled.as_slice() {
                    [known] => *known,
                    [] => {
                        return Err(format!(
                            "There is no worker {:?} in this space: workers lists them, by session and title.",
                            crate::space::one_line(worker, 80)
                        ));
                    }
                    _ => {
                        return Err(format!(
                            "More than one worker here is titled {:?}, so name it by its session.",
                            crate::space::one_line(worker, 80)
                        ));
                    }
                }
            }
        };
        if known.state == tomori_protocol::WorkerState::Completed {
            return Err(
                "That worker has finished, so remove it with remove_workers rather than place it."
                    .to_owned(),
            );
        }
        let environments = self.environments();
        let anchors = record.clusters(&environments);
        let placed = |at: String, pose, beside| Placement {
            session: known.id.clone(),
            at,
            pose,
            beside,
            by: None,
        };
        Ok(match target {
            Target::Home => placed(tomori_protocol::cluster::HOME.to_owned(), None, None),
            Target::Anchor(name) => {
                if !anchors.iter().any(|anchor| anchor.name == name) {
                    let near = nearest(&name, anchors.iter().map(|anchor| anchor.name.as_str()));
                    let suggest = if near.is_empty() {
                        String::new()
                    } else {
                        format!(" Nearest in name: {}.", near.join(", "))
                    };
                    return Err(format!(
                        "There is no anchor called {:?} here: presence lists the anchors.{suggest}",
                        crate::space::one_line(&name, tomori_protocol::cluster::NAME_BYTES)
                    ));
                }
                placed(name, None, None)
            }
            Target::Object(id) => {
                // By the id inspect takes, or its reference alone.
                let reference = id
                    .rsplit_once(':')
                    .map_or(id.as_str(), |(_, reference)| reference);
                if let Some(anchor) = anchors
                    .iter()
                    .find(|anchor| anchor.object.as_deref() == Some(reference))
                {
                    return Ok(placed(anchor.name.clone(), None, None));
                }
                // Else in a small cluster of its own, 1 m before where the object stands, towards whoever asked.
                let objects = record
                    .channel
                    .as_ref()
                    .filter(|channel| channel.environment == record.model.environment)
                    .map_or(&[][..], |channel| channel.objects.as_slice());
                let (index, object, position) = objects
                    .iter()
                    .enumerate()
                    .find(|(_, object)| {
                        object.reference.as_deref() == Some(reference) || object.id == id
                    })
                    .and_then(|(index, object)| Some((index, object, object.position?)))
                    .ok_or_else(|| {
                        format!(
                            "Nothing here stands by the object {:?}, and the environment does not say where it stands, so a worker has nowhere to stand by it: presence lists the anchors and what each stands by.",
                            crate::space::one_line(&id, 80)
                        )
                    })?;
                let asker = record
                    .model
                    .origin()
                    .map_or(tomori_protocol::cluster::EYES, |head| head.position);
                let pose = crate::places::before_object(position, asker);
                placed(
                    crate::places::object_cluster(
                        object.reference.as_deref().unwrap_or(&object.id),
                        index,
                    ),
                    Some(pose),
                    Some(crate::space::one_line(&id, 80)),
                )
            }
            Target::Wall(named) => {
                let walls = environments.walls(&record.model.environment);
                let projection = record.stage.view(space, &walls, crate::cards::unix_now());
                let people = state.heads(space);
                let origin = record.model.origin();
                let id =
                    crate::presence::which_wall(&named, origin.as_ref(), &people, &projection)?;
                let (index, wall) = projection
                    .walls
                    .iter()
                    .enumerate()
                    .find(|(_, wall)| wall.id == id)
                    .expect("a wall named is shown");
                let pose = crate::presence::beyond(wall, origin.map(|head| head.position));
                let name = crate::places::wall_cluster(&wall.id, index);
                placed(
                    name,
                    Some(pose),
                    Some(crate::presence::wall_name(&projection, wall)),
                )
            }
            Target::Near(name) => {
                let people = state.heads(space);
                let head = crate::presence::head_of(
                    name.as_deref(),
                    record.model.origin().as_ref(),
                    &people,
                )?;
                let (at, pose) = crate::places::near_cluster(
                    record.placements(),
                    &crate::presence::right_of(&head),
                );
                let beside = name.unwrap_or_else(|| "whoever asked".to_owned());
                placed(at, Some(pose), Some(beside))
            }
        })
    }
}
