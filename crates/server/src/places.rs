// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where workers were placed in a space, kept with it for each environment: at a cluster anchor, by name, or in a small
//! cluster of the server's own beside a wall or a person; who placed each; and the limits on how often the agent places
//! workers and moves itself, so that neither drifts about. See `design/environments.md` and `design/mcp.md`.
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tomori_protocol::Pose;
use tomori_protocol::cluster::{Anchor, OWN};

/// The most worker placements a space keeps for each environment.
pub const PLACEMENTS: usize = 64;

/// How long a placement at an anchor the environment's side has not placed again waits, after the side starts, before
/// it is forgotten, silently, since the place may simply be gone.
pub const GRACE: Duration = Duration::from_secs(60);

/// How far apart, in metres, two placements beside a person stand to share a cluster rather than make another.
const SHARED: f32 = 1.0;

/// Where a worker was placed, and by whom: at the anchor `at`, by name, which is one of the server's own, from
/// [`OWN`], beside a wall or a person, standing at `pose`; placed by the participant `by`, by name, or else by the agent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub session: String,
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pose: Option<Pose>,
    /// What the server's own cluster stands beside, as the agent is told: a wall's name or a person's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beside: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
}

/// The worker placements a space keeps, for each environment, by its identifier, oldest first.
pub type Kept = BTreeMap<String, Vec<Placement>>;

/// Keep `placement` among `placements`, one environment's, in place of any of its worker's; at most [`PLACEMENTS`],
/// the oldest forgotten first.
pub fn keep(placements: &mut Vec<Placement>, placement: Placement) {
    placements.retain(|kept| kept.session != placement.session);
    placements.push(placement);
    let excess = placements.len().saturating_sub(PLACEMENTS);
    placements.drain(..excess);
}

/// The name of the server's own cluster beside the wall `wall`, by its identity, or, where that is not a name an
/// anchor may have, by its place among the walls shown.
pub fn wall_cluster(wall: &str, index: usize) -> String {
    let name = format!("{OWN}wall/{wall}");
    if tomori_protocol::cluster::valid_name(&name) {
        name
    } else {
        format!("{OWN}wall/{index}")
    }
}

/// The name of the server's own cluster by the object `reference`, or, where that is not a name an anchor may have,
/// by its place among the objects described.
pub fn object_cluster(reference: &str, index: usize) -> String {
    let name = format!("{OWN}object/{reference}");
    if tomori_protocol::cluster::valid_name(&name) {
        name
    } else {
        format!("{OWN}object/{index}")
    }
}

/// How far before an object, in metres, towards whoever asked, the server's own cluster by it stands.
pub const BEFORE_OBJECT: f32 = 1.0;

/// Where the server's own cluster by an object at `object` stands, for whoever asked, whose eyes were at `eyes`:
/// [`BEFORE_OBJECT`] before it, towards them, at their eye height, facing them; or, standing where the object is, before
/// it along +Z.
pub fn before_object(object: [f32; 3], eyes: [f32; 3]) -> Pose {
    let (dx, dz) = (eyes[0] - object[0], eyes[2] - object[2]);
    let apart = dx.hypot(dz);
    let (x, z) = if apart > 1e-3 {
        (dx / apart, dz / apart)
    } else {
        (0.0, 1.0)
    };
    let at = [
        object[0] + x * BEFORE_OBJECT,
        eyes[1],
        object[2] + z * BEFORE_OBJECT,
    ];
    Pose::upright_facing(at, [at[0] + x, eyes[1], at[2] + z])
}

/// The name of the server's own cluster for a worker placed beside someone at `pose`: that of one of `placements`
/// standing within [`SHARED`] of it, so that workers placed beside the same person stand together, or else one of its
/// own, named from where it stands.
pub fn near_cluster(placements: &[Placement], pose: &Pose) -> (String, Pose) {
    let near = placements.iter().find_map(|placement| {
        let at = placement.pose?;
        let [x, y, z] = at.position;
        let [px, py, pz] = pose.position;
        (placement.at.starts_with(&format!("{OWN}near/"))
            && ((x - px).powi(2) + (y - py).powi(2) + (z - pz).powi(2)).sqrt() < SHARED)
            .then(|| (placement.at.clone(), at))
    });
    near.unwrap_or_else(|| {
        let [x, _, z] = pose.position;
        (format!("{OWN}near/{:.1}_{:.1}", x, z), *pose)
    })
}

/// The server's own clusters among `placements`, one environment's: one for each name, at the pose of the first placed
/// there, of the default room, a `wall` or a `person`, and labelled with what it stands beside.
pub fn own(placements: &[Placement]) -> Vec<Anchor> {
    let mut anchors: Vec<Anchor> = Vec::new();
    for placement in placements {
        let Some(pose) = placement.pose.filter(|_| placement.at.starts_with(OWN)) else {
            continue;
        };
        if anchors.iter().any(|anchor| anchor.name == placement.at) {
            continue;
        }
        let kind = if placement.at.starts_with(&format!("{OWN}wall/")) {
            "wall"
        } else if placement.at.starts_with(&format!("{OWN}object/")) {
            "object"
        } else {
            "person"
        };
        let mut anchor = Anchor::new(placement.at.clone(), pose).with_kind(kind);
        if let Some(beside) = &placement.beside {
            anchor = anchor.with_label(format!("Beside {beside}"));
        }
        anchors.push(anchor);
    }
    anchors
}

/// How often the agent may place each worker, and the space's together, of its own accord: each worker once a minute
/// and three times in ten minutes, its first placement not counted, and the space's twelve times a minute; and how often
/// the agent may move itself to an anchor of its own accord, once every 20 seconds and six times in ten minutes. A
/// placement or move asked for by whoever's words the agent answers is not limited here.
#[derive(Debug, Default)]
pub struct Rates {
    workers: BTreeMap<String, VecDeque<Instant>>,
    space: VecDeque<Instant>,
    moves: VecDeque<Instant>,
}

/// Whether `times`, the latest last, leave room at `now` for another: fewer than `most` within each `within`.
fn room(times: &VecDeque<Instant>, now: Instant, limits: &[(usize, Duration)]) -> bool {
    limits.iter().all(|(most, within)| {
        times
            .iter()
            .filter(|at| now.saturating_duration_since(**at) < *within)
            .count()
            < *most
    })
}

impl Rates {
    /// Whether the agent may place the worker `session` at `now` of its own accord, taking it if so, or why not: its first
    /// placement, as it starts, counts only towards the space's limit.
    pub fn place(&mut self, session: &str, now: Instant) -> Result<(), String> {
        let first = !self.workers.contains_key(session);
        let minute = Duration::from_secs(60);
        let ten = Duration::from_secs(600);
        self.space
            .retain(|at| now.saturating_duration_since(*at) < minute);
        if !room(&self.space, now, &[(12, minute)]) {
            return Err("Workers in this space have been placed twelve times in the last minute, so leave this one where it is for now.".to_owned());
        }
        let times = self.workers.entry(session.to_owned()).or_default();
        times.retain(|at| now.saturating_duration_since(*at) < ten);
        if !first && !room(times, now, &[(1, minute), (3, ten)]) {
            return Err("That worker was placed in the last minute, or three times in the last ten, so leave it where it is: move it again only when its work moves or someone asks.".to_owned());
        }
        if !first {
            times.push_back(now);
        }
        self.space.push_back(now);
        Ok(())
    }

    /// Whether the agent may move itself at `now` of its own accord, taking it if so, or why not.
    pub fn move_agent(&mut self, now: Instant) -> Result<(), String> {
        let ten = Duration::from_secs(600);
        self.moves
            .retain(|at| now.saturating_duration_since(*at) < ten);
        if !room(&self.moves, now, &[(1, Duration::from_secs(20)), (6, ten)]) {
            return Err("You moved yourself within the last 20 seconds, or six times in the last ten minutes, so stay where you are unless someone asks you to move.".to_owned());
        }
        self.moves.push_back(now);
        Ok(())
    }

    /// Forget the worker `session`'s placements, as it is removed.
    pub fn forget(&mut self, session: &str) {
        self.workers.remove(session);
    }
}

#[cfg(test)]
mod tests;
