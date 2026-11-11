// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A space's cluster anchors: those its environment declares, and those its environment's server side places and
//! removes as it runs, within the limits `tomori-environment` exports; and where whatever stands at them stands, the
//! slots the server gives the members of each cluster, which the space's state sends. See `design/environments.md`.
use std::time::Instant;

use tomori_protocol::cluster::Anchor;

use tomori_protocol::Pose;

use super::{Hub, SpaceOutlet, SpaceRecord};
use crate::channel::{Change, Places};

/// The kind of Tomori's notice of what went with an anchor, one replacing the last.
const PLACES: &str = "places";

/// What stood at an anchor as it went: how many workers, and how many panels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Standing {
    workers: usize,
    panels: usize,
}

impl Standing {
    /// What the notice of the anchor `name` going says, such as "The environment closed rack/a12: 2 workers went home,
    /// 1 panel was left floating.", or `None` where nothing stood there.
    fn notice(&self, name: &str) -> Option<String> {
        let plural = |count: usize, one: &str, many: &str| {
            format!("{count} {}", if count == 1 { one } else { many })
        };
        let mut said = Vec::new();
        if self.workers > 0 {
            said.push(format!(
                "{} home",
                plural(self.workers, "worker went", "workers went")
            ));
        }
        if self.panels > 0 {
            said.push(format!(
                "{} left floating",
                plural(self.panels, "panel was", "panels were")
            ));
        }
        (!said.is_empty()).then(|| format!("The environment closed {name}: {}.", said.join(", ")))
    }
}

/// Tomori's notice, titled Places, that says `body`.
fn places_notice(body: String) -> crate::stage::Content {
    crate::stage::Content {
        title: "Places".to_owned(),
        ..super::tools::notice(body)
    }
}
use crate::environments::Environments;
use crate::seating::Seating;

impl SpaceRecord {
    /// The anchors the space's state sends: its environment's, carried into the space's coordinates, with those its
    /// environment's side placed for this binding, and the server's own beside the walls and people workers were placed
    /// beside, after them.
    pub(super) fn anchors(&self, environments: &Environments) -> tomori_protocol::Anchors {
        let mut anchors = environments.anchors(&self.model.environment);
        if let Some(channel) = self
            .channel
            .as_ref()
            .filter(|channel| channel.environment == self.model.environment)
        {
            anchors.clusters.extend(channel.anchors().iter().cloned());
        }
        // And the server's own, beside the walls and people workers were placed beside.
        anchors
            .clusters
            .extend(crate::places::own(self.placements()));
        anchors
    }

    /// Every cluster anchor of the space, in its coordinates: `home` first, about the workers anchor or else the agent's
    /// home, in the room its environment gives it, whether or not it gives one; then the rest, as [`Self::anchors`]
    /// gives them.
    pub(super) fn clusters(&self, environments: &Environments) -> Vec<Anchor> {
        let anchors = self.anchors(environments);
        let mut all = vec![crate::seating::home(&anchors)];
        all.extend(
            anchors
                .clusters
                .into_iter()
                .filter(|anchor| !anchor.is_home()),
        );
        all
    }

    /// Where the space's workers were placed in its environment now, oldest first.
    pub(super) fn placements(&self) -> &[crate::places::Placement] {
        self.places
            .get(&self.model.environment)
            .map_or(&[], Vec::as_slice)
    }

    /// Where the space's workers were placed in its environment now, to change.
    pub(super) fn placements_mut(&mut self) -> &mut Vec<crate::places::Placement> {
        self.places
            .entry(self.model.environment.clone())
            .or_default()
    }

    /// Who placed the worker `session` where it stands, where a participant did, by name.
    pub(super) fn placed_by(&self, session: &str) -> Option<String> {
        self.placements()
            .iter()
            .find(|placement| placement.session == session)
            .and_then(|placement| placement.by.clone())
    }

    /// Where whatever stands at the space's anchors stands now, seated afresh for what stands there now, as every
    /// connection is sent it: each worker where it was placed, while that anchor is here, or else with the worker that
    /// delegated to it, or else at home; each panel floating where it was put at an anchor; and the agent, while it
    /// stands where it went to one.
    pub(super) fn seated(&self, environments: &Environments) -> std::cell::Ref<'_, Seating> {
        use crate::seating::Member;
        let anchors = self.clusters(environments);
        let placements = self.placements();
        let mut wanted = crate::seating::workers(self.model.workers(), &anchors, |session| {
            placements
                .iter()
                .find(|placement| placement.session == session)
                .map(|placement| placement.at.as_str())
        });
        let panels = self.stage.anchored(&self.model.environment);
        wanted.extend(
            panels
                .iter()
                .map(|(panel, anchor, _)| (Member::Panel(panel.clone()), anchor.as_str())),
        );
        if let Some(anchor) = self.agent_anchor() {
            wanted.push((Member::Agent, anchor));
        }
        self.seating.borrow_mut().seat(&anchors, &wanted);
        self.seating.borrow()
    }

    /// Float each of the space's panels at an anchor where its cluster stands it now, as the anchor moves or its
    /// cluster is laid out again, facing whoever the cluster gathers about. Returns whether any moved.
    pub(super) fn settle(&mut self, environments: &Environments) -> bool {
        let anchors = self.clusters(environments);
        let moving: Vec<(String, String, Pose)> = {
            let seating = self.seated(environments);
            self.stage
                .anchored(&self.model.environment)
                .into_iter()
                .filter_map(|(panel, anchor, was)| {
                    let (at, eyes) = seating
                        .position(&anchors, &crate::seating::Member::Panel(panel.clone()))?;
                    // As the stage keeps it, so that one already there is left there.
                    let pose = Pose::upright_facing(at, [eyes[0], at[1], eyes[2]]);
                    let pose = crate::stage::valid(&pose).unwrap_or(pose);
                    (pose != was).then_some((panel, anchor, pose))
                })
                .collect()
        };
        let walls = environments.walls(&self.model.environment);
        for (panel, anchor, pose) in &moving {
            let destination = crate::stage::Destination::Anchor {
                anchor: anchor.clone(),
                pose: *pose,
            };
            // A panel at an anchor floats, and is here to move.
            let _ = self.stage.relocate(&walls, panel, &destination);
        }
        !moving.is_empty()
    }

    /// What stood at the anchor `name`, which has just gone from among the space's anchors, by where its workers were
    /// placed and its panels put.
    fn standing(&self, environments: &Environments, name: &str) -> Standing {
        let mut anchors = self.clusters(environments);
        anchors.push(Anchor::new(name, Pose::HOME));
        let placements = self.placements();
        let workers = crate::seating::workers(self.model.workers(), &anchors, |session| {
            placements
                .iter()
                .find(|placement| placement.session == session)
                .map(|placement| placement.at.as_str())
        })
        .into_iter()
        .filter(|(_, at)| *at == name)
        .count();
        let panels = self
            .stage
            .anchored(&self.model.environment)
            .iter()
            .filter(|(_, anchor, _)| anchor == name)
            .count();
        Standing { workers, panels }
    }

    /// Where a new member arriving at the anchor `name` would stand, after `before` others arriving with it, facing
    /// whoever its cluster gathers about, and their eyes, if the space has that anchor.
    pub(super) fn arriving(
        &self,
        environments: &Environments,
        name: &str,
        before: usize,
    ) -> Option<(Pose, [f32; 3])> {
        let anchor = self
            .clusters(environments)
            .into_iter()
            .find(|anchor| anchor.name == name)?;
        let (at, eyes) = self.seated(environments).arriving(&anchor, before);
        Some((Pose::upright_facing(at, [eyes[0], at[1], eyes[2]]), eyes))
    }

    /// The anchor the agent stands at, by name, while it stands where it went to it.
    pub(super) fn agent_anchor(&self) -> Option<&str> {
        self.agent_at
            .as_ref()
            .filter(|(_, pose)| self.summoned == Some(*pose))
            .map(|(anchor, _)| anchor.as_str())
    }

    /// Where `place` puts a panel of the agent's in the space, after `before` others put at the same anchor with it: at
    /// an anchor, floating in its cluster's next free slot; or as [`crate::presence::destination`] says, for whoever
    /// asked, whose head was at `asker`, among `people`, in `projection`, clear of `solids`. Or why it cannot, as for an
    /// anchor not here.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn panel_destination(
        &self,
        environments: &Environments,
        place: &crate::stage::Placement,
        before: usize,
        asker: Option<&Pose>,
        people: &[crate::presence::Someone],
        projection: &tomori_protocol::Projection,
        solids: &[tomori_environment::sightlines::Solid],
    ) -> Result<crate::stage::Destination, String> {
        match place {
            crate::stage::Placement::Anchor(name) => self
                .arriving(environments, name, before)
                .map(|(pose, _)| crate::stage::Destination::Anchor {
                    anchor: name.clone(),
                    pose,
                })
                .ok_or_else(|| {
                    format!(
                        "There is no anchor called {:?} here: presence lists the anchors.",
                        crate::space::one_line(name, tomori_protocol::cluster::NAME_BYTES)
                    )
                }),
            place => crate::presence::destination(place, asker, people, projection, solids),
        }
    }

    /// The workers the "+N more" sign of the cluster at the anchor `name` gathers now, by session, in the order of
    /// their slots, or `None` where the space has no such anchor or the sign gathers none.
    pub(super) fn gathered(&self, environments: &Environments, name: &str) -> Option<Vec<String>> {
        let anchor = self
            .clusters(environments)
            .into_iter()
            .find(|anchor| anchor.name == name)?;
        let seating = self.seated(environments);
        let gathered: Vec<String> = seating
            .gathered(&anchor)
            .into_iter()
            .filter_map(|member| match member {
                crate::seating::Member::Worker(session) => Some(session.clone()),
                _ => None,
            })
            .collect();
        (!gathered.is_empty()).then_some(gathered)
    }
}

impl Hub {
    /// The anchor the side of `outlet`'s binding places, or moves, resizes or relabels in place, for everyone in the
    /// space, held to the rates [`Places`] keeps: a change that comes too soon waits, the latest of each anchor's
    /// replacing the one before, and is made at the hub's next sweep once it may be. See `design/environments.md`.
    pub(super) fn channel_place_anchor(
        &self,
        outlet: &SpaceOutlet,
        anchor: Anchor,
    ) -> tomori_environment::Result<()> {
        self.channel_place_anchor_at(outlet, anchor, Instant::now())
    }

    /// [`Self::channel_place_anchor`] at `now`.
    pub(super) fn channel_place_anchor_at(
        &self,
        outlet: &SpaceOutlet,
        anchor: Anchor,
        now: Instant,
    ) -> tomori_environment::Result<()> {
        let name = crate::space::one_line(&anchor.name, tomori_environment::ANCHOR_NAME_BYTES);
        let refused = |why: &str| format!("The anchor {name:?} is refused: {why}.");
        anchor.checked().map_err(refused)?;
        let environments = self.environments();
        if environments
            .declared(&outlet.environment)
            .contains(&anchor.name)
        {
            return Err(refused(
                "the environment declares an anchor of that name, so the place is its own",
            )
            .into());
        }
        let pose = environments
            .carry(&outlet.environment, &anchor.pose)
            .ok_or_else(|| {
                refused("its pose is not finite, has no orientation, or stands more than 20 m from home")
            })?;
        let mut anchor = crate::environments::cleaned(anchor);
        anchor.pose = pose;
        anchor.placed = true;
        let declared = environments
            .get(&outlet.environment)
            .map_or(0, |environment| environment.anchors().len());
        let mut state = self.lock();
        Self::running(&state, outlet)?;
        let Some(channel) = self.current_channel(&mut state, outlet) else {
            return Ok(());
        };
        let places = channel.places.get_or_insert_with(|| Places::new(now));
        places.call(now)?;
        if !places.has(&anchor.name)
            && declared + places.count() >= tomori_environment::ANCHOR_LIMIT
        {
            return Err(refused(&format!(
                "the space has {} anchors already, as many as it may",
                tomori_environment::ANCHOR_LIMIT
            ))
            .into());
        }
        let made = Self::change_anchor(places, Change::Place(anchor), now);
        if made.is_some() {
            self.anchors_changed(&mut state, &outlet.space, None, now);
        }
        Ok(())
    }

    /// Remove the anchor the side of `outlet`'s binding placed as `name`, as [`Self::channel_place_anchor`] holds its
    /// changes to their rates; one not placed changes nothing, and cancels its placement waiting.
    pub(super) fn channel_remove_anchor(
        &self,
        outlet: &SpaceOutlet,
        name: &str,
    ) -> tomori_environment::Result<()> {
        self.channel_remove_anchor_at(outlet, name, Instant::now())
    }

    /// [`Self::channel_remove_anchor`] at `now`.
    pub(super) fn channel_remove_anchor_at(
        &self,
        outlet: &SpaceOutlet,
        name: &str,
        now: Instant,
    ) -> tomori_environment::Result<()> {
        let mut state = self.lock();
        Self::running(&state, outlet)?;
        let Some(channel) = self.current_channel(&mut state, outlet) else {
            return Ok(());
        };
        let places = channel.places.get_or_insert_with(|| Places::new(now));
        places.call(now)?;
        if Self::change_anchor(places, Change::Remove(name.to_owned()), now).is_some() {
            let record = state.space(&outlet.space).expect("its channel is current");
            let standing = record.standing(&self.environments(), name);
            self.anchors_changed(&mut state, &outlet.space, Some((name, standing)), now);
        }
        Ok(())
    }

    /// Make `change` to the side's anchors now, if its rates allow at `now`, in place of any change of its anchor
    /// waiting; or else let it wait. Returns the change, if it changed them.
    fn change_anchor(places: &mut Places, change: Change, now: Instant) -> Option<Change> {
        let name = change.name().to_owned();
        let placed = places.anchors.iter().any(|anchor| anchor.name == name);
        // Removing one not placed only cancels its placement waiting.
        if matches!(change, Change::Remove(_)) && !placed {
            places.wait(change);
            return None;
        }
        if !places.ready(now, &name) {
            places.wait(change);
            return None;
        }
        places.waiting.retain(|waiting| waiting.name() != name);
        let made = change.clone();
        if !places.make(change) {
            return None;
        }
        places.charge(now, &name);
        Some(made)
    }

    /// The space `space`'s anchors changed at `now`, the anchor `closed` among them removed, with what stood there as it
    /// went: everyone in it is told; what stands at its anchors moves with them; and, of one removed, its workers go
    /// home, forgetting where they were placed there, its panels stay floating where they stand, the agent stays where
    /// it is, and a notice on the status wall, titled Places, says what went and what moved, unless nothing stood there.
    /// See `design/environments.md`.
    fn anchors_changed(
        &self,
        state: &mut super::HubState,
        space: &str,
        closed: Option<(&str, Standing)>,
        now: Instant,
    ) {
        let environments = self.environments();
        let record = state.space_mut(space).expect("its channel is current");
        let mut projected = false;
        if let Some((name, standing)) = closed {
            record
                .placements_mut()
                .retain(|placement| placement.at != name);
            record
                .stage
                .unanchor(&record.model.environment.clone(), name);
            if record.agent_at.as_ref().is_some_and(|(at, _)| at == name) {
                record.agent_at = None;
            }
            if let Some(body) = standing.notice(name) {
                let walls = environments.walls(&record.model.environment);
                record.stage.project_notice(
                    &walls,
                    PLACES,
                    places_notice(body),
                    crate::cards::unix_now(),
                );
                projected = true;
            }
            log::info!("The environment of space {space} removed the anchor {name}.");
        }
        projected |= record.settle(&environments);
        state.touch_space(space);
        if projected {
            state.touch_projection_paced(space, now);
        }
    }

    /// Make every side's anchor changes waiting that their rates allow at `now`, in the order they were asked for, as
    /// the hub sweeps.
    pub(crate) fn flush_anchors(&self, now: Instant) {
        let mut state = self.lock();
        let environments = self.environments();
        let waiting: Vec<String> = state
            .spaces
            .iter()
            .filter(|record| {
                record
                    .channel
                    .as_ref()
                    .and_then(|channel| channel.places.as_ref())
                    .is_some_and(|places| !places.waiting.is_empty())
            })
            .map(|record| record.model.id.clone())
            .collect();
        for space in waiting {
            let record = state.space_mut(&space).expect("listed above");
            let places = record
                .channel
                .as_mut()
                .and_then(|channel| channel.places.as_mut())
                .expect("waiting above");
            let mut made = Vec::new();
            for change in std::mem::take(&mut places.waiting) {
                made.extend(Self::change_anchor(places, change, now));
            }
            for change in made {
                let closed = match &change {
                    Change::Remove(name) => {
                        // What stood there is found as it stood, before it went.
                        let record = state.space(&space).expect("listed above");
                        Some((name.as_str(), record.standing(&environments, name)))
                    }
                    Change::Place(_) => None,
                };
                self.anchors_changed(&mut state, &space, closed, now);
            }
        }
    }

    /// Forget, at `now`, where workers were placed that are no longer the space's, once its list of workers is
    /// complete, and those at an anchor that is not here [`crate::places::GRACE`] after the space was bound to its
    /// environment, silently, since the place may simply be gone; and float each panel at an anchor where its cluster
    /// stands it now. Called as the hub sweeps. See `design/environments.md`.
    pub(crate) fn sweep_places(&self, now: Instant) {
        let environments = self.environments();
        let mut forgot = false;
        {
            let mut state = self.lock();
            let ids: Vec<String> = state
                .spaces
                .iter()
                .map(|record| record.model.id.clone())
                .collect();
            for space in ids {
                let record = state.space_mut(&space).expect("listed above");
                let complete = record.model.workers_complete().0 && !record.restoring;
                let workers: Vec<String> = record
                    .model
                    .workers()
                    .iter()
                    .map(|worker| worker.id.clone())
                    .collect();
                let names: Vec<String> = record
                    .clusters(&environments)
                    .into_iter()
                    .map(|anchor| anchor.name)
                    .collect();
                // A closed space, or one restoring, runs no side, which is not a removal: its placements wait.
                let waited = !record.model.is_closed()
                    && !record.restoring
                    && now.saturating_duration_since(record.anchored) >= crate::places::GRACE;
                let before = record.placements().to_vec();
                let mut gone = Vec::new();
                record.placements_mut().retain(|placement| {
                    let here = !complete || workers.contains(&placement.session);
                    if !here {
                        gone.push(placement.session.clone());
                    }
                    here && (!waited || names.contains(&placement.at))
                });
                for session in gone {
                    record.rates.forget(&session);
                }
                let changed = record.placements() != before.as_slice();
                let moved = record.settle(&environments);
                if changed {
                    log::debug!("Space {space} forgot where some of its workers were placed.");
                    state.touch_space(&space);
                    forgot = true;
                }
                if moved {
                    state.touch_projection_paced(&space, now);
                }
            }
        }
        if forgot {
            self.keep();
        }
    }
    /// Tell the server side of each space who takes part in it now, by their numbers: its participants whose clients
    /// are connected and take part, not its visitors, so that a side can let go of what someone who left held, or tidy
    /// up once nobody is there. Called as the hub sweeps. See `tomori_environment::Space::present`.
    pub(crate) fn sweep_present(&self) {
        let state = self.lock();
        for record in &state.spaces {
            let space = &record.model.id;
            let present: Vec<u32> = record
                .model
                .participants
                .iter()
                .filter_map(|key| state.participants.get(key))
                .filter(|participant| {
                    participant.connection.is_some_and(|connection| {
                        state.role(connection, Some(space)).participates()
                    })
                })
                .map(|participant| participant.source)
                .collect();
            self.server_sides.present(space, present);
        }
    }

    /// Look, at `now`, at who is near the anchors of each space that notify its agent, and tell its agent, through the
    /// space's event queue, of each participant who came within one, as [`crate::proximity::Proximity`] decides: only
    /// participants, whose clients report where they are, named by their handle where another in the space shares
    /// their name, and only while the agent can be told, as its timers are; what happens meanwhile is not told later.
    /// Called as the hub sweeps. See `design/environments.md`.
    pub(crate) fn sweep_proximity(&self, now: Instant) {
        let environments = self.environments();
        let mut state = self.lock();
        let ids: Vec<String> = state
            .spaces
            .iter()
            .map(|record| record.model.id.clone())
            .collect();
        for space in ids {
            let record = state.space(&space).expect("listed above");
            let participants: Vec<(&String, &super::ParticipantRecord)> = record
                .model
                .participants
                .iter()
                .filter_map(|key| {
                    state
                        .participants
                        .get(key)
                        .map(|participant| (key, participant))
                })
                .filter(|(_, participant)| {
                    participant.connection.is_some_and(|connection| {
                        state.role(connection, Some(&space)).participates()
                    })
                })
                .collect();
            // Owned, since the space is changed below, each by their key, name and where their head is.
            let people: Vec<(String, String, [f32; 3])> = participants
                .iter()
                .filter_map(|(key, participant)| {
                    let shared = participants
                        .iter()
                        .any(|(other, them)| other != key && them.name == participant.name);
                    let name = if shared {
                        crate::presence::handle(&participant.name, participant.tag)
                    } else {
                        participant.name.clone()
                    };
                    participant
                        .seen
                        .as_ref()
                        .map(|seen| ((*key).clone(), name, seen.head.position))
                })
                .collect();
            if people.is_empty() && record.proximity.is_empty() {
                continue;
            }
            let anchors: Vec<Anchor> = record
                .clusters(&environments)
                .into_iter()
                .filter(|anchor| anchor.notify_within.is_some())
                .collect();
            let told =
                !record.restoring && !record.model.is_closed() && !record.model.is_reconnecting();
            let people: Vec<crate::proximity::Person<'_>> = people
                .iter()
                .map(|(key, name, at)| crate::proximity::Person { key, name, at: *at })
                .collect();
            let record = state.space_mut(&space).expect("listed above");
            let notes = record
                .proximity
                .observe(now, &record.model.environment, &people, &anchors);
            if !told || notes.is_empty() {
                continue;
            }
            // Who came near where is not logged, as nothing of presence is. See `design/mcp.md`.
            log::debug!("Space {space} tells its agent of someone coming near its anchors.");
            for (key, note) in notes {
                // Whom it is about, by their key, which the note's key begins: a reply to it keeps with them.
                let about = people.iter().map(|person| person.key).find(|person| {
                    key.strip_prefix(*person)
                        .and_then(|rest| rest.strip_prefix('/'))
                        .is_some_and(|anchor| anchors.iter().any(|known| known.name == anchor))
                });
                record.model.tell_proximity(&key, about, note);
            }
            state.pump(&space);
        }
    }
}
