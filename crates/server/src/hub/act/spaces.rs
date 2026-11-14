// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Listing, creating, joining, leaving, changing and deleting spaces, and naming oneself.

use super::*;

impl Hub {
    /// Act on `message`, a message about spaces or the participant's own name, from `participant` on `connection`, who is in `space` if any.
    pub(super) fn act_on_spaces(
        &self,
        state: &mut HubState,
        connection: ConnectionId,
        participant: &str,
        space: Option<&str>,
        message: ClientMessage,
        deferred: &mut Vec<Deferred>,
    ) {
        let outbox = state.connections[&connection].outbox.clone();
        match message {
            ClientMessage::ListSpaces => outbox.spaces(),
            ClientMessage::SetName { name } => match limits::name(&name) {
                Ok(name) => {
                    state.rename_request(participant, &name, None, self.token_store(), deferred)
                }
                Err(error) => state.error(participant, format!("Name not changed: {error}")),
            },
            ClientMessage::CreateSpace {
                title,
                backend,
                directory,
                session,
                environment,
                instructions,
            } => {
                let instructions = instructions.unwrap_or_default();
                let checked = limits::check(&title, limits::TITLE_BYTES, "title")
                    .and_then(|()| {
                        limits::check(&instructions, limits::INSTRUCTIONS_BYTES, "instructions")
                    })
                    .and_then(|()| limits::check(&directory, limits::DIRECTORY_BYTES, "directory"))
                    .and_then(|()| match &session {
                        Some(session) => limits::check(session, limits::SESSION_BYTES, "session"),
                        None => Ok(()),
                    });
                if let Err(error) = checked {
                    state.error(participant, error);
                    return;
                }
                let Some(found) = self.backends.get(&backend) else {
                    let names: Vec<&str> = self.backends.names().collect();
                    state.error(
                        participant,
                        format!("Unknown backend; available: {}.", names.join(", ")),
                    );
                    return;
                };
                if session.is_some() && !found.capabilities().attach {
                    state.error(
                        participant,
                        format!("The {backend} backend cannot bind an existing session."),
                    );
                    return;
                }
                let environment = match self
                    .environments()
                    .choose_for(environment.as_deref(), state.presents(connection))
                {
                    Ok(environment) => environment,
                    Err(error) => {
                        state.error(participant, error);
                        return;
                    }
                };
                // Refuse early when the limit is already reached. The supervisor checks the directory outside the lock and only then takes a place, so that a refused directory never holds one.
                if self.slots.is_full() {
                    state.error(participant, self.slots.full_error());
                    return;
                }
                let request = CreateRequest {
                    title,
                    backend,
                    directory,
                    session,
                    environment,
                    instructions,
                    creator: participant.to_owned(),
                    creator_space: space.map(str::to_owned),
                };
                if self.jobs.try_send(Job::Create(request)).is_err() {
                    state.error(participant, "The server is shutting down.");
                }
            }
            ClientMessage::ListSessions { backend, directory } => {
                let checked = limits::check(&backend, limits::NAME_BYTES, "backend")
                    .and_then(|()| limits::check(&directory, limits::DIRECTORY_BYTES, "directory"));
                if let Err(error) = checked {
                    state.error(participant, error);
                    return;
                }
                let Some(found) = self.backends.get(&backend) else {
                    let names: Vec<&str> = self.backends.names().collect();
                    state.error(
                        participant,
                        format!("Unknown backend; available: {}.", names.join(", ")),
                    );
                    return;
                };
                if !found.capabilities().attach {
                    state.error(
                        participant,
                        format!("The {backend} backend cannot list existing sessions."),
                    );
                    return;
                }
                if self
                    .jobs
                    .try_send(Job::ListSessions {
                        participant: participant.to_owned(),
                        backend,
                        directory,
                    })
                    .is_err()
                {
                    state.error(participant, "The server is shutting down.");
                }
            }
            ClientMessage::Join { space: target } => {
                if let Err(error) =
                    limits::check(&target, limits::IDENTIFIER_BYTES, "space identifier")
                {
                    state.error(participant, error);
                    return;
                }
                if state.space(&target).is_none() {
                    // Naming the space lets the client tell this refusal from other errors; it is bounded above.
                    state.send(
                        participant,
                        ServerMessage::Error {
                            error: "That space does not exist.".to_owned(),
                            space: Some(target),
                            panel: None,
                            wall: None,
                        },
                    );
                    return;
                }
                // A participant who chooses to join sees what happened while nobody was there, for everyone. A visitor
                // watching does not, and being put back in a space on reconnecting does not count, since what happened
                // meanwhile was not seen. See `design/spaces.md`.
                if state.role(connection, Some(&target)).participates()
                    && let Some(record) = state.space_mut(&target)
                {
                    record.model.attend();
                }
                // The space is listed, checked above, and the participant connected, so they join it.
                state.join(participant, &target);
                self.arrive(state, participant, false);
            }
            ClientMessage::Leave => {
                state.abandon_utterance(connection, deferred);
                // Announce leaving before the lobby listing that leaving produces.
                outbox.clear_audio();
                outbox.message(ServerMessage::LEFT);
                // Leaving, they arrive anew when they next join, unlike a reconnect. See `design/environments.md`.
                for record in &mut state.spaces {
                    record.arrivals.forget(participant);
                }
                state.leave(participant);
                // A member is kept.
                state
                    .participants
                    .entry(participant.to_owned())
                    .and_modify(|record| record.last_space = None);
                state.update_role(participant);
                outbox.spaces();
            }
            ClientMessage::SetMode { mode } => {
                if let Err(error) = limits::check(&mode, limits::IDENTIFIER_BYTES, "mode") {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                let record = state.space_mut(&space).expect("checked above");
                if record.model.is_closed() {
                    state.error(participant, "The agent session has stopped.");
                } else if record.model.is_reconnecting() {
                    state.error(participant, crate::space::RECONNECTING);
                } else if !record.model.offers_mode(&mode) {
                    state.error(participant, "The agent does not offer that mode.");
                } else if record
                    .commands
                    .try_send(SpaceCommand::SetMode {
                        mode,
                        participant: participant.to_owned(),
                    })
                    .is_err()
                {
                    state.error(participant, "The agent session has stopped.");
                }
            }
            ClientMessage::SetInstructions { text } => {
                if let Err(error) = limits::check(&text, limits::INSTRUCTIONS_BYTES, "instructions")
                {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                let record = state.space_mut(&space).expect("checked above");
                match record.model.set_space_instructions(&text) {
                    Ok(()) => {
                        // While the agent has not taken them all, every layer is given again, so that taking this one alone does not hide that.
                        record.instruct(record.model.instructions_failed());
                        state.touch_space(&space);
                        deferred.push(Deferred::Keep);
                    }
                    Err(error) => state.error(participant, error),
                }
            }
            ClientMessage::DeleteSpace {
                space: target,
                session,
            } => {
                if let Err(error) =
                    limits::check(&target, limits::IDENTIFIER_BYTES, "space identifier")
                {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                // Only the space the owner is in, and named as the one they saw, so that a request meant for one space never deletes another. See `design/spaces.md`.
                if target != space {
                    state.error(participant, "Only the space you are in can be deleted.");
                    return;
                }
                let record = state.space(&space).expect("checked above");
                let (backend, bound) = (record.model.backend.clone(), record.model.session.clone());
                if session {
                    // Refused whole, deleting nothing, so that the owner can choose again.
                    let refusal = if !self
                        .backends
                        .get(&backend)
                        .is_some_and(|found| found.capabilities().delete)
                    {
                        Some(format!(
                            "The {backend} backend cannot delete its sessions, so nothing was deleted."
                        ))
                    } else if state.is_bound(&backend, &bound, Some(&space)) {
                        Some("Its agent session is bound to another space too, so nothing was deleted.".to_owned())
                    } else {
                        None
                    };
                    if let Some(refusal) = refusal {
                        state.error(participant, refusal);
                        return;
                    }
                }
                let asker = session.then(|| (participant.to_owned(), connection));
                self.delete(state, participant, &space, asker, deferred);
            }
            ClientMessage::RenameSpace { title } => {
                let title = match limits::title(&title) {
                    Ok(title) => title,
                    Err(error) => {
                        state.error(participant, format!("Title not changed: {error}"));
                        return;
                    }
                };
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                let by = state.participants[participant].name.clone();
                state.rename_space(
                    &space,
                    title,
                    tomori_protocol::Renamer::Participant { name: by },
                );
                deferred.push(Deferred::Keep);
            }
            ClientMessage::SetSpeech { speech } => {
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                let record = state.space_mut(&space).expect("checked above");
                match record.model.set_speech(speech) {
                    Ok(()) => {
                        state.touch_space(&space);
                        deferred.push(Deferred::Keep);
                    }
                    Err(error) => state.error(participant, error),
                }
            }
            // `act` hands this function only messages about spaces or the participant's own name.
            _ => unreachable!("not a message about spaces or the participant's own name"),
        }
    }
}
