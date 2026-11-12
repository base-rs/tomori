// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What the server side of a space's environment keeps and sends through the hub: its state, events, descriptions, actions, zones and context, and the panels it publishes onto its environment's walls, paced by their rates; and the sides started and stopped to match the spaces bound to their environments. See `design/environments.md`.
use super::*;

impl tomori_environment::Outlet for SpaceOutlet {
    fn share(&self, state: String) -> tomori_environment::Result<()> {
        self.hub
            .upgrade()
            .map_or(Ok(()), |hub| hub.channel_update(self, &state, None))
    }

    fn announce(&self, event: String, state: String) -> tomori_environment::Result<()> {
        self.hub
            .upgrade()
            .map_or(Ok(()), |hub| hub.channel_update(self, &state, Some(&event)))
    }

    fn describe(&self, objects: Vec<tomori_environment::Object>) -> tomori_environment::Result<()> {
        self.hub
            .upgrade()
            .map_or(Ok(()), |hub| hub.channel_describe(self, objects))
    }

    fn publish(&self, panel: tomori_environment::Panel) -> tomori_environment::Result<()> {
        self.hub
            .upgrade()
            .map_or(Ok(()), |hub| hub.channel_publish(self, panel))
    }

    fn retract(&self, id: &str) -> tomori_environment::Result<()> {
        self.hub
            .upgrade()
            .map_or(Ok(()), |hub| hub.channel_retract(self, id))
    }

    fn place_anchor(
        &self,
        anchor: tomori_protocol::cluster::Anchor,
    ) -> tomori_environment::Result<()> {
        self.hub
            .upgrade()
            .map_or(Ok(()), |hub| hub.channel_place_anchor(self, anchor))
    }

    fn remove_anchor(&self, name: &str) -> tomori_environment::Result<()> {
        self.hub
            .upgrade()
            .map_or(Ok(()), |hub| hub.channel_remove_anchor(self, name))
    }

    fn declare_actions(
        &self,
        actions: Vec<tomori_environment::Action>,
    ) -> tomori_environment::Result<()> {
        self.hub
            .upgrade()
            .map_or(Ok(()), |hub| hub.channel_declare_actions(self, actions))
    }

    fn declare_zones(
        &self,
        zones: Vec<tomori_environment::Zone>,
    ) -> tomori_environment::Result<()> {
        self.hub
            .upgrade()
            .map_or(Ok(()), |hub| hub.channel_declare_zones(self, zones))
    }

    fn set_context(
        &self,
        parameters: Vec<tomori_environment::ContextParameter>,
    ) -> tomori_environment::Result<()> {
        self.hub.upgrade().map_or(Ok(()), |hub| {
            hub.channel_set_context(self, parameters, Instant::now())
        })
    }
}

impl Hub {
    /// What the side of `outlet`'s binding keeps and sends: a new `state`, kept for whoever joins and sent to everyone in the space now unless an `event` goes with it, which is sent to them instead, except as the binding's first update, which is always sent as its state. A side of a binding that has ended, or of a space that has gone, is ignored. See `design/environments.md`.
    pub(super) fn channel_update(
        &self,
        outlet: &SpaceOutlet,
        state_text: &str,
        event_text: Option<&str>,
    ) -> tomori_environment::Result<()> {
        let kept = crate::channel::state(state_text)?;
        let event = event_text.map(crate::channel::event).transpose()?;
        let mut state = self.lock();
        let Some(record) = self.current_channel(&mut state, outlet) else {
            return Ok(());
        };
        // Held to the rate of updates a side may send, states and events alike. See `design/environments.md`.
        let now = Instant::now();
        record
            .updates
            .get_or_insert_with(|| crate::channel::Updates::new(now))
            .take(now)?;
        // Clients ignore an event until they have a state, so the first update of a binding is sent as its state, which already includes any event that goes with it.
        let first = record.state.replace(kept.clone()).is_none();
        let message = match event.filter(|_| !first) {
            Some(event) => ServerMessage::EnvironmentEvent {
                environment: outlet.environment.clone(),
                version: outlet.version,
                event,
            },
            None => ServerMessage::EnvironmentState {
                environment: outlet.environment.clone(),
                version: outlet.version,
                state: kept,
            },
        };
        let record = state.space(&outlet.space).expect("found above");
        let channel = record.channel.as_ref().expect("kept above");
        for outbox in state.channel_outboxes(record, channel) {
            outbox.message(message.clone());
        }
        Ok(())
    }

    /// The objects the side of `outlet`'s binding describes, for the agent; see [`Self::channel_update`].
    pub(super) fn channel_describe(
        &self,
        outlet: &SpaceOutlet,
        objects: Vec<tomori_environment::Object>,
    ) -> tomori_environment::Result<()> {
        // Where each stands, carried into the space's coordinates as its environment's anchors are.
        let environments = self.environments();
        let objects = crate::channel::objects(objects, |position| {
            environments
                .carry(&outlet.environment, &tomori_protocol::Pose::at(position))
                .map(|pose| pose.position)
        });
        drop(environments);
        let mut state = self.lock();
        let replaced = self
            .current_channel(&mut state, outlet)
            .map(|record| std::mem::replace(&mut record.objects, objects));
        // Those it described before, as many as a binding keeps, are freed once the hub is free.
        drop(state);
        drop(replaced);
        Ok(())
    }

    /// The actions the side of `outlet`'s binding declares on what it describes, replacing those it declared before, or
    /// why they are refused, keeping none of them. See `design/environment-actions.md`.
    pub(super) fn channel_declare_actions(
        &self,
        outlet: &SpaceOutlet,
        actions: Vec<tomori_environment::Action>,
    ) -> tomori_environment::Result<()> {
        let actions = crate::channel::actions(actions)?;
        let mut state = self.lock();
        if let Some(record) = self.current_channel(&mut state, outlet) {
            record.actions = actions;
        }
        Ok(())
    }

    /// The zones the side of `outlet`'s binding declares, replacing those it declared before, each told as it is at the
    /// next sweep, or why they are refused, keeping none of them. See `design/environments.md`.
    pub(super) fn channel_declare_zones(
        &self,
        outlet: &SpaceOutlet,
        zones: Vec<tomori_environment::Zone>,
    ) -> tomori_environment::Result<()> {
        let environments = self.environments();
        let zones = crate::zones::Zones::declare(zones, |point| {
            environments
                .carry(&outlet.environment, &tomori_protocol::Pose::at(point))
                .map(|pose| pose.position)
        })?;
        drop(environments);
        let mut state = self.lock();
        if let Some(record) = self.current_channel(&mut state, outlet) {
            record.zones.declare(zones, Instant::now());
        }
        Ok(())
    }

    /// The panel the side of `outlet`'s binding publishes onto its environment's own walls, or updates in place, for
    /// everyone in the space, held to the rates [`crate::channel::Publishes`] keeps: an update that comes too soon waits,
    /// replacing any update of the same panel waiting before it, and is made as soon as it may be, at the next change the
    /// side makes or the hub's next sweep, so that the latest always arrives. A new panel, which takes a slot, is kept
    /// with the space; an update in place, which its side makes again, is not written each time. See
    /// `design/environments.md`.
    pub(super) fn channel_publish(
        &self,
        outlet: &SpaceOutlet,
        panel: tomori_environment::Panel,
    ) -> tomori_environment::Result<()> {
        self.channel_publish_at(outlet, panel, Instant::now())
    }

    /// [`Self::channel_publish`] at `now`. Updates waiting that are due are made first, the oldest first, and a fresh one
    /// after them, at once where the changes left allow it, and otherwise waits behind them, while an update of a panel
    /// already waiting takes its place there, so that every panel's latest update is made in turn: with the changes a
    /// space may make, 16 a second, a side updating its 32 panels every second has each one's latest made within 2
    /// seconds, and at the next sweep after.
    pub(super) fn channel_publish_at(
        &self,
        outlet: &SpaceOutlet,
        panel: tomori_environment::Panel,
        now: Instant,
    ) -> tomori_environment::Result<()> {
        let publishing = crate::stage::Publishing::new(panel)?;
        let added = {
            let mut state = self.lock();
            Self::running(&state, outlet)?;
            let Some(channel) = self.current_channel(&mut state, outlet) else {
                return Ok(());
            };
            channel
                .publishes
                .get_or_insert_with(|| crate::channel::Publishes::new(now))
                .message(now)?;
            // Refused at once, as it would be made now, for what can be known before it is: its wall, its slot and
            // the most an environment shows, counting those waiting.
            let waiting: Vec<String> = channel
                .pending
                .iter()
                .map(|waiting| waiting.id().to_owned())
                .collect();
            {
                let record = state.space(&outlet.space).expect("its channel is current");
                let environments = self.environments();
                let walls = environments.walls(&record.model.environment);
                let waiting: Vec<&str> = waiting.iter().map(String::as_str).collect();
                record.stage.admits(&walls, &publishing, &waiting)?;
            }
            let channel = state
                .space_mut(&outlet.space)
                .and_then(|record| record.channel.as_mut())
                .expect("its channel is current");
            if let Some(waiting) = channel
                .pending
                .iter_mut()
                .find(|waiting| waiting.id() == publishing.id())
            {
                *waiting = publishing;
                self.flush_space(&mut state, &outlet.space, now)
            } else {
                // Those waiting that are due first; then this one, at once where the changes left allow it.
                let added = self.flush_space(&mut state, &outlet.space, now);
                self.publish_now(&mut state, &outlet.space, publishing, now)? || added
            }
        };
        if added {
            self.keep();
        }
        Ok(())
    }

    /// Why the side of `outlet`'s binding may change no panels: it has stopped, failed or finished, though a thread it
    /// left may still call.
    pub(super) fn running(
        state: &HubState,
        outlet: &SpaceOutlet,
    ) -> tomori_environment::Result<()> {
        if state
            .space(&outlet.space)
            .is_some_and(|record| record.stopped == Some(outlet.binding))
        {
            return Err("The environment's side has stopped in this space, so it changes no panels until it runs again.".into());
        }
        Ok(())
    }

    /// Make `publishing` in the space `space`, whose channel is current, at `now`, if its rates allow, or else let it wait,
    /// unless it would change nothing. Returns whether it put a new panel on a wall, or why the stage refused it.
    pub(super) fn publish_now(
        &self,
        state: &mut HubState,
        space: &str,
        publishing: crate::stage::Publishing,
        now: Instant,
    ) -> tomori_environment::Result<bool> {
        let environments = self.environments();
        let record = state.space_mut(space).expect("its channel is current");
        let walls = environments.walls(&record.model.environment);
        let channel = record.channel.as_mut().expect("its channel is current");
        let publishes = channel
            .publishes
            .as_mut()
            .expect("made as its side first published");
        let (id, bytes) = (publishing.id().to_owned(), publishing.bytes());
        if !publishes.ready(now, Some(&id), bytes) {
            if record.stage.unchanged(walls.environment, &publishing) {
                return Ok(false);
            }
            channel.pending.push(publishing);
            return Ok(false);
        }
        let change = record
            .stage
            .publish(&walls, publishing, crate::cards::unix_now())?;
        // Taken, so that a later refusal of it is said again.
        channel.accept(&id);
        if change == crate::stage::Change::Unchanged {
            return Ok(false);
        }
        channel
            .publishes
            .as_mut()
            .expect("made as its side first published")
            .charge(now, Some(&id), bytes);
        state.touch_projection_paced(space, now);
        Ok(change == crate::stage::Change::Added)
    }

    /// Make the updates waiting for the space `space`'s side that its rates allow at `now`, keeping the rest waiting, and
    /// dropping any the stage now refuses. Returns whether any put a new panel on a wall.
    pub(super) fn flush_space(&self, state: &mut HubState, space: &str, now: Instant) -> bool {
        let waiting = state
            .space_mut(space)
            .and_then(|record| record.channel.as_mut())
            .map(|channel| std::mem::take(&mut channel.pending))
            .unwrap_or_default();
        let mut added = false;
        for publishing in waiting {
            let id = publishing.id().to_owned();
            match self.publish_now(state, space, publishing, now) {
                Ok(new) => added |= new,
                // Refused for what changed while it waited, such as its wall filling: said once for each panel.
                Err(error) => {
                    let first = state
                        .space_mut(space)
                        .and_then(|record| record.channel.as_mut())
                        .is_some_and(|channel| channel.refuse(&id));
                    if first {
                        log::warn!(
                            "An update of panel {id} in space {space}, made once it could be, was refused: {error}"
                        );
                    }
                }
            }
        }
        added
    }

    /// Make every side's waiting panel updates that their rates allow at `now`, as the hub sweeps.
    pub(crate) fn flush_publishes(&self, now: Instant) {
        let added = {
            let mut state = self.lock();
            let waiting: Vec<String> = state
                .spaces
                .iter()
                .filter(|record| {
                    record
                        .channel
                        .as_ref()
                        .is_some_and(|channel| !channel.pending.is_empty())
                })
                .map(|record| record.model.id.clone())
                .collect();
            // Every space's, not stopping at the first that adds a panel.
            let mut added = false;
            for space in &waiting {
                added |= self.flush_space(&mut state, space, now);
            }
            added
        };
        if added {
            self.keep();
        }
    }

    /// Take away the panel the side of `outlet`'s binding published as `id`, and any update of it waiting, so that it
    /// may be published again at once; see [`Self::channel_publish`]. A retraction is never kept waiting: it costs a
    /// change, which later ones then wait for, but only where it took a panel away.
    pub(super) fn channel_retract(
        &self,
        outlet: &SpaceOutlet,
        id: &str,
    ) -> tomori_environment::Result<()> {
        self.channel_retract_at(outlet, id, Instant::now())
    }

    /// [`Self::channel_retract`] at `now`.
    pub(super) fn channel_retract_at(
        &self,
        outlet: &SpaceOutlet,
        id: &str,
        now: Instant,
    ) -> tomori_environment::Result<()> {
        {
            let mut state = self.lock();
            Self::running(&state, outlet)?;
            let Some(channel) = self.current_channel(&mut state, outlet) else {
                return Ok(());
            };
            let publishes = channel
                .publishes
                .get_or_insert_with(|| crate::channel::Publishes::new(now));
            publishes.message(now)?;
            publishes.forget(id);
            channel.pending.retain(|waiting| waiting.id() != id);
            channel.accept(id);
            let record = state.space_mut(&outlet.space).expect("found above");
            if !record.stage.retract(&outlet.environment, id) {
                return Ok(());
            }
            // A status panel waiting for the slot it freed takes it.
            record
                .stage
                .settle(&self.environments().walls(&outlet.environment));
            let channel = record.channel.as_mut().expect("found above");
            channel
                .publishes
                .as_mut()
                .expect("made above")
                .charge(now, None, 0);
            state.touch_projection_paced(&outlet.space, now);
        }
        self.keep();
        Ok(())
    }

    /// The parameters the side of `outlet`'s binding declares its panels depend on, with their values now, replacing
    /// those it declared before, at `now`, held to the rate its publishing is: a change of them is a change of what the
    /// space projects, its chips, told to everyone in the space as the side's changes are. See
    /// `design/panel-context.md`.
    pub(super) fn channel_set_context(
        &self,
        outlet: &SpaceOutlet,
        parameters: Vec<tomori_environment::ContextParameter>,
        now: Instant,
    ) -> tomori_environment::Result<()> {
        let parameters = crate::stage::context::declared(parameters)?;
        let mut state = self.lock();
        Self::running(&state, outlet)?;
        let Some(channel) = self.current_channel(&mut state, outlet) else {
            return Ok(());
        };
        let publishes = channel
            .publishes
            .get_or_insert_with(|| crate::channel::Publishes::new(now));
        publishes.message(now)?;
        let record = state.space_mut(&outlet.space).expect("found above");
        if record
            .stage
            .declare_parameters(&outlet.environment, parameters)
        {
            let channel = record.channel.as_mut().expect("found above");
            channel
                .publishes
                .as_mut()
                .expect("made above")
                .charge(now, None, 0);
            state.touch_projection_paced(&outlet.space, now);
        }
        Ok(())
    }

    /// What the space of `outlet` keeps for its binding, begun anew for a new binding, or `None` if the binding has ended or the space has gone.
    pub(super) fn current_channel<'a>(
        &self,
        state: &'a mut HubState,
        outlet: &SpaceOutlet,
    ) -> Option<&'a mut crate::channel::Channel> {
        let current = self.server_sides.binding(&outlet.space) == Some(outlet.binding);
        let record = state
            .space_mut(&outlet.space)
            .filter(|record| current && record.model.environment == outlet.environment)?;
        let channel = record.channel.get_or_insert_with(|| {
            crate::channel::Channel::new(outlet.binding, &outlet.environment, outlet.version)
        });
        if channel.binding != outlet.binding {
            *channel =
                crate::channel::Channel::new(outlet.binding, &outlet.environment, outlet.version);
        }
        Some(channel)
    }

    /// Start and stop the server sides of the environments to match the spaces that are open and the environments they are bound to. Called whenever a space is bound, closes or changes environment, the environments offered change, or the hub shuts down, so that each is run for as it should be whichever came first. See `design/environments.md`.
    pub(super) fn reconcile_server_sides(&self, state: &mut HubState) {
        let wanted: Vec<crate::server_sides::Wanted> = if state.shutting_down || !state.serving {
            Vec::new()
        } else {
            state
                .spaces
                .iter()
                .filter(|record| !record.model.is_closed() && !record.restoring)
                .map(|record| crate::server_sides::Wanted {
                    id: record.model.id.clone(),
                    title: record.model.title.clone(),
                    directory: record.model.directory.clone(),
                    environment: record.model.environment.clone(),
                })
                .collect()
        };
        let failures = self.server_sides.reconcile(&self.environments(), &wanted);
        for failure in failures {
            self.show_side_failure(state, failure);
        }
        // What a side kept is forgotten with its binding, so that nobody joining is sent another environment's state, or an earlier run's.
        for record in &mut state.spaces {
            let binding = self.server_sides.binding(&record.model.id);
            if record
                .channel
                .as_ref()
                .is_some_and(|channel| binding != Some(channel.binding))
            {
                record.channel = None;
            }
            // A side begun anew, as the space reopens, places its anchors again, so placements wait for them from now.
            if binding != record.bound {
                record.bound = binding;
                record.anchored = Instant::now();
            }
        }
    }

    /// Move the space `space` to `environment`, one the server offers, for everyone in it, as an owner or its agent asked: whether it changed, or why it cannot, as for a closed space. Keeping the change is the caller's. See `design/environments.md` and `design/mcp.md`.
    pub(super) fn move_space(
        &self,
        state: &mut HubState,
        space: &str,
        environment: String,
    ) -> Result<bool, &'static str> {
        let record = state.space_mut(space).expect("the caller found it");
        let left = record.model.environment.clone();
        if !record.model.set_environment(environment)? {
            return Ok(false);
        }
        // What its stage keeps for the environment left is the last forgotten. See `design/projection.md`.
        record.stage.leave(&left);
        // The agent appears at the new environment's home for it, wherever it was summoned to in the last, and that
        // environment's placements wait for its anchors from now.
        record.summoned = None;
        record.agent_at = None;
        record.anchored = Instant::now();
        record.partner = None;
        // Nor does it follow anyone there, nor stay pinned.
        record.following = following::Following::default();
        // The environment's layer of the agent's instructions changes with it, so every layer is given again, as on binding.
        record.instruct(true);
        // Its server side, if it has one, runs for the space, and the previous environment's stops.
        self.reconcile_server_sides(state);
        // Everyone in the space, and the lobby's listing, see the change.
        state.touch_space(space);
        // The status wall is where the new environment puts it, and that environment's walls show, the status panels
        // taking the status wall's free slots there.
        let environments = self.environments();
        let record = state.space_mut(space).expect("the caller found it");
        let walls = environments.walls(&record.model.environment);
        record.stage.settle(&walls);
        drop(environments);
        state.touch_projection(space);
        Ok(true)
    }

    /// The server has begun serving: run the server sides of the spaces that were bound while it was being configured, and of those bound from now on. A kept space listed again waits until it is bound to its session again; see [`Hub::bound`].
    pub(crate) fn serving(&self) {
        let mut state = self.lock();
        state.serving = true;
        self.reconcile_server_sides(&mut state);
    }

    /// A server side failed after it started: show it in its space, if the space is still bound to the environment it ran for.
    pub(super) fn side_failed(&self, failure: crate::server_sides::Failure) {
        let mut state = self.lock();
        self.show_side_failure(&mut state, failure);
    }

    pub(super) fn show_side_failure(
        &self,
        state: &mut HubState,
        failure: crate::server_sides::Failure,
    ) {
        if !self
            .server_sides
            .is_current(&failure.space, failure.binding)
        {
            return;
        }
        let Some(record) = state.space_mut(&failure.space) else {
            return;
        };
        // A side that failed shows why; one that finished, nothing. Either changes no panels from now on, and what it
        // left waiting is dropped, so that nothing of it comes back until a side runs again.
        if let Some(message) = failure.message {
            record.model.fail_environment(message);
        }
        record.stopped = Some(failure.binding);
        if let Some(channel) = record.channel.as_mut() {
            channel.pending.clear();
        }
        // What the side published is no longer current, so it is shown no more until a side publishes it again.
        let environment = record.model.environment.clone();
        if record.stage.rest(&environment) {
            // A status panel waiting for a slot it freed takes it.
            record
                .stage
                .settle(&self.environments().walls(&environment));
            state.touch_projection(&failure.space);
        }
        state.touch_space(&failure.space);
    }
}
