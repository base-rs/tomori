// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A space's cluster anchors: those its environment declares, and those its environment's server side places and
//! removes as it runs, within the limits `tomori-environment` exports; and where whatever stands at them stands, the
//! slots the server gives the members of each cluster, which the space's state sends. See `design/environments.md`.
use std::time::Instant;

use tomori_protocol::cluster::Anchor;

use super::{Hub, SpaceOutlet, SpaceRecord};
use crate::channel::{Change, Places};
use crate::environments::Environments;
use crate::seating::Seating;

impl SpaceRecord {
    /// The anchors the space's state sends: its environment's, carried into the space's coordinates, with those its
    /// environment's side placed for this binding, after them.
    pub(super) fn anchors(&self, environments: &Environments) -> tomori_protocol::Anchors {
        let mut anchors = environments.anchors(&self.model.environment);
        if let Some(channel) = self
            .channel
            .as_ref()
            .filter(|channel| channel.environment == self.model.environment)
        {
            anchors.clusters.extend(channel.anchors().iter().cloned());
        }
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

    /// Where whatever stands at the space's anchors stands now, seated afresh for the workers it has now, as every
    /// connection is sent it: each worker at home, or with the worker that delegated to it.
    pub(super) fn seated(&self, environments: &Environments) -> std::cell::Ref<'_, Seating> {
        let anchors = self.clusters(environments);
        let wanted = crate::seating::workers(self.model.workers(), &anchors, |_| None);
        self.seating.borrow_mut().seat(&anchors, &wanted);
        self.seating.borrow()
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
            .map(|crate::seating::Member::Worker(session)| session.clone())
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
        let changed = Self::change_anchor(places, Change::Place(anchor), now);
        if changed {
            state.touch_space(&outlet.space);
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
        if Self::change_anchor(places, Change::Remove(name.to_owned()), now) {
            state.touch_space(&outlet.space);
        }
        Ok(())
    }

    /// Make `change` to the side's anchors now, if its rates allow at `now`, in place of any change of its anchor
    /// waiting; or else let it wait. Returns whether it changed them.
    fn change_anchor(places: &mut Places, change: Change, now: Instant) -> bool {
        let name = change.name().to_owned();
        let placed = places.anchors.iter().any(|anchor| anchor.name == name);
        // Removing one not placed only cancels its placement waiting.
        if matches!(change, Change::Remove(_)) && !placed {
            places.wait(change);
            return false;
        }
        if !places.ready(now, &name) {
            places.wait(change);
            return false;
        }
        places.waiting.retain(|waiting| waiting.name() != name);
        let changed = places.make(change);
        if changed {
            places.charge(now, &name);
        }
        changed
    }

    /// Make every side's anchor changes waiting that their rates allow at `now`, in the order they were asked for, as
    /// the hub sweeps.
    pub(crate) fn flush_anchors(&self, now: Instant) {
        let mut state = self.lock();
        let mut changed = Vec::new();
        for record in &mut state.spaces {
            let Some(places) = record
                .channel
                .as_mut()
                .and_then(|channel| channel.places.as_mut())
                .filter(|places| !places.waiting.is_empty())
            else {
                continue;
            };
            let mut any = false;
            for change in std::mem::take(&mut places.waiting) {
                any |= Self::change_anchor(places, change, now);
            }
            if any {
                changed.push(record.model.id.clone());
            }
        }
        for space in changed {
            state.touch_space(&space);
        }
    }
}
