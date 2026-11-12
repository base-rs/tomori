// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The things of a space a diagram's world references may name, gathered from what the agent's own tools read, in
//! the space alone: the objects its current environment's side describes, its cluster anchors, its walls and panels,
//! its workers, the participants in it who report presence, and the agent. See `design/diagrams.md`.
use std::time::Instant;

use tomori_protocol::world::Frame;
use tomori_protocol::{Place, Projection};

use super::{Environments, HubState};
use crate::diagram::world::{Thing, Things};

/// How large a participant's head is taken as, a worker's sign and the agent's swarm, in metres from their middles.
const HEAD: f32 = 0.15;
const SIGN: f32 = 0.3;
const SWARM: f32 = 0.35;

/// The most an anchor's ring reaches from its point, in metres, however much room its cluster takes.
const ANCHOR_RING: f32 = 1.5;

impl HubState {
    /// The things of `space` its diagrams' world references may name at `now`, with `projection` its walls and panels as
    /// shown, those in `carried` in someone's hand, and whoever its agent acts for, whose head is at `asker`, whom
    /// distances are told from.
    pub(super) fn world_things(
        &self,
        space: &str,
        environments: &Environments,
        projection: &Projection,
        carried: &[String],
        asker: Option<&tomori_protocol::Pose>,
        now: Instant,
    ) -> Things {
        let Some(record) = self.space(space) else {
            return Things::default();
        };
        let environment = record.model.environment.clone();
        let mut things = Things {
            environment: environment.clone(),
            ..Things::default()
        };
        // An object is named as `inspect` takes it, by its environment's id and its reference, and by its reference.
        if let Some(channel) = record
            .channel
            .as_ref()
            .filter(|channel| channel.environment == environment)
        {
            things.objects = channel
                .objects
                .iter()
                .filter_map(|object| {
                    let reference = object.reference.as_ref()?;
                    let label = object
                        .description
                        .split(',')
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .to_owned();
                    Some(Thing {
                        names: vec![
                            crate::space::inspectable(&environment, reference),
                            reference.clone(),
                        ],
                        label,
                        described: object.description.clone(),
                        at: object.position,
                        radius: object.extent,
                        frame: None,
                        absent: object
                            .position
                            .is_none()
                            .then(|| "where the environment does not say".to_owned()),
                    })
                })
                .collect();
        }
        let anchors = record.clusters(environments);
        things.anchors = anchors
            .iter()
            .map(|anchor| {
                let label = anchor.label.clone().unwrap_or_else(|| anchor.name.clone());
                let kind = anchor
                    .kind
                    .as_ref()
                    .map_or(String::new(), |kind| format!(" ({kind})"));
                Thing {
                    names: vec![anchor.name.clone()],
                    described: format!(
                        "anchor {} \"{}\"{kind}",
                        anchor.name,
                        crate::space::one_line(&label, 64)
                    ),
                    label,
                    // Its ring on the floor beneath its point, as where the floor would be beneath a swarm there.
                    at: Some({
                        let [x, y, z] = anchor.pose.position;
                        [x, y - tomori_protocol::cluster::RING_BELOW, z]
                    }),
                    radius: anchor.extent.map_or(0.5, |extent| {
                        (extent.width.max(extent.depth) / 2.0).clamp(0.3, ANCHOR_RING)
                    }),
                    frame: None,
                    absent: None,
                }
            })
            .collect();
        things.walls = projection
            .walls
            .iter()
            .map(|wall| {
                let name = crate::presence::wall_name(projection, wall);
                let (pose, size) = crate::presence::wall_frame(wall);
                let mut names = vec![wall.id.clone(), name.clone()];
                names.extend(wall.name.clone());
                if wall.status || wall.kind == tomori_protocol::WallKind::Tomori {
                    names.push("status wall".to_owned());
                }
                Thing {
                    names,
                    label: crate::presence::capitalised(&name),
                    described: crate::presence::the_wall(projection, wall),
                    at: Some(pose.position),
                    radius: size[0].max(size[1]) / 2.0,
                    frame: Some(Frame {
                        orientation: pose.orientation,
                        size,
                    }),
                    absent: None,
                }
            })
            .collect();
        things.panels = projection
            .panels
            .iter()
            .map(|panel| {
                let title = crate::presence::title(panel);
                let mut names = vec![panel.id.clone()];
                names.extend(panel.slug.clone());
                names.push(title.clone());
                let standing = match &panel.place {
                    Place::Wall { wall, slot } => projection
                        .walls
                        .iter()
                        .find(|shown| shown.id == *wall)
                        .and_then(|wall| crate::presence::slot_frame(wall, *slot)),
                    Place::Free { pose } => {
                        let (width, height) = tomori_protocol::wall::SLOT_SIZE;
                        Some((*pose, [width, height]))
                    }
                    _ => None,
                }
                .filter(|_| !carried.contains(&panel.id));
                Thing {
                    names,
                    described: format!(
                        "panel \"{}\" ({})",
                        crate::space::one_line(&title, 64),
                        panel.id
                    ),
                    label: title,
                    at: standing.map(|(pose, _)| pose.position),
                    radius: standing.map_or(0.3, |(_, size)| size[0].max(size[1]) / 2.0),
                    frame: standing.map(|(pose, size)| Frame {
                        orientation: pose.orientation,
                        size,
                    }),
                    absent: standing.is_none().then(|| "being carried".to_owned()),
                }
            })
            .collect();
        let seating = record.seated(environments);
        things.workers = record
            .model
            .workers()
            .iter()
            .map(|worker| {
                let at = seating
                    .position(&anchors, &crate::seating::Member::Worker(worker.id.clone()))
                    .map(|(at, _)| at);
                Thing {
                    names: vec![worker.id.clone(), worker.title.clone()],
                    label: worker.title.clone(),
                    described: format!(
                        "worker \"{}\" ({})",
                        crate::space::one_line(&worker.title, 64),
                        worker.id
                    ),
                    at,
                    radius: SIGN,
                    frame: None,
                    // Every worker is seated, so none stands nowhere.
                    absent: None,
                }
            })
            .collect();
        drop(seating);
        // Only the participants in the space, by what their presence says now: one who has not reported, or not for
        // [`crate::presence::STALE`], is away, and never drawn to where they were.
        things.people = record
            .model
            .participants
            .iter()
            .filter_map(|key| self.participants.get(key))
            .map(|participant| {
                let handle = crate::presence::handle(&participant.name, participant.tag);
                let head = participant
                    .seen
                    .as_ref()
                    .filter(|seen| now.saturating_duration_since(seen.at) < crate::presence::STALE)
                    .map(|seen| seen.head.position);
                Thing {
                    names: vec![handle.clone(), participant.name.clone()],
                    label: participant.name.clone(),
                    described: format!(
                        "{} ({handle})",
                        crate::space::one_line(&participant.name, 40)
                    ),
                    at: head,
                    radius: HEAD,
                    frame: None,
                    absent: head.is_none().then(|| "away".to_owned()),
                }
            })
            .collect();
        let home = super::agent_home(&environments.anchors(&environment));
        things.agent = Some(Thing {
            names: vec!["agent".to_owned()],
            label: "Tomori".to_owned(),
            described: "you, the agent".to_owned(),
            at: Some(record.summoned.unwrap_or(home).position),
            radius: SWARM,
            frame: None,
            absent: None,
        });
        // Whom the agent answers, by name, where their eyes were as they asked.
        let asked = self.asked_by(space, &record.model.asked(), now);
        things.asker = asked
            .and_then(|key| self.participants.get(&key))
            .zip(asker)
            .map(|(participant, head)| (participant.name.clone(), head.position));
        things
    }
}

/// Where a volume going to `to` will stand, as nearly as it is known before it is projected, so that its nodes are
/// pulled towards what it points at as it will stand: where `standing`, the volume it updates, stands, unless it is
/// placed afresh, `placed`; at the pose it floats at; in the slot or on the wall it goes to; or in front of whoever it
/// goes beside. `None` where it goes nowhere known, as onto a wall not shown.
pub(super) fn volume_pose(
    to: &crate::stage::Destination,
    placed: bool,
    standing: Option<&tomori_protocol::Panel>,
    projection: &Projection,
) -> Option<tomori_protocol::Pose> {
    use crate::stage::Destination;
    let wall_named = |id: &str| projection.walls.iter().find(|wall| wall.id == id);
    let on_wall = |id: &str, slot: Option<u32>| {
        wall_named(id).map(|wall| {
            slot.and_then(|slot| crate::presence::slot_frame(wall, slot))
                .map_or_else(|| crate::presence::wall_frame(wall).0, |(pose, _)| pose)
        })
    };
    if let Some(panel) = standing.filter(|_| !placed) {
        return match &panel.place {
            Place::Wall { wall, slot } => on_wall(wall, Some(*slot)),
            Place::Free { pose } => Some(*pose),
            _ => None,
        };
    }
    match to {
        Destination::Free(pose) | Destination::Anchor { pose, .. } => Some(*pose),
        Destination::Wall { wall, slot } => on_wall(wall, *slot),
        Destination::Nearest(point) => crate::presence::nearest_wall(&projection.walls, *point)
            .map(|(wall, _)| crate::presence::wall_frame(wall).0),
        Destination::Beside { head, .. } => Some(crate::presence::front_of(head)),
        Destination::Tomori => on_wall(tomori_protocol::TOMORI_WALL, None),
    }
}

/// Where a volume of `diagram` placed `near references` goes for whoever asked, whose head is at `asker`: floating where
/// [`crate::diagram::world::near_references`] stands it, or else beside them, clear of `solids`; or, with nobody asking,
/// as the agent's panels go by default.
pub(super) fn near_references(
    diagram: &tomori_protocol::diagram::Diagram,
    asker: Option<&tomori_protocol::Pose>,
    projection: &Projection,
    solids: &[tomori_environment::sightlines::Solid],
) -> crate::stage::Destination {
    match asker {
        Some(head) => crate::diagram::world::near_references(diagram, head).map_or_else(
            || crate::stage::Destination::Beside {
                head: *head,
                solids: solids.to_vec(),
            },
            crate::stage::Destination::Free,
        ),
        None => crate::presence::by_default(None, projection, solids),
    }
}
