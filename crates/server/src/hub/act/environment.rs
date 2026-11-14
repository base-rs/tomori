// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The space's environment: choosing it, its intents and its cards.

use super::*;

impl Hub {
    /// Act on `message`, a message about the space's environment, from `participant` on `connection`, who is in `space` if any.
    pub(super) fn act_on_environment(
        &self,
        state: &mut HubState,
        connection: ConnectionId,
        participant: &str,
        space: Option<&str>,
        message: ClientMessage,
        deferred: &mut Vec<Deferred>,
    ) {
        match message {
            ClientMessage::SetEnvironment { environment } => {
                if let Err(error) =
                    limits::check(&environment, limits::IDENTIFIER_BYTES, "environment")
                {
                    state.error(participant, error);
                    return;
                }
                // Only an environment offered to the requester: anyone else in the space who cannot present it shows the grid and says what it lacks. See `design/environments.md`.
                let chosen = self
                    .environments()
                    .choose_for(Some(&environment), state.presents(connection));
                let environment = match chosen {
                    Ok(environment) => environment,
                    Err(error) => {
                        state.error(participant, error);
                        return;
                    }
                };
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                match self.move_space(state, &space, environment) {
                    Ok(true) => {
                        log::info!(
                            target: LOG,
                            "Space {space} is now in the {} environment.",
                            state
                                .space(&space)
                                .expect("checked above")
                                .model
                                .environment
                        );
                        // It is kept across a restart.
                        deferred.push(Deferred::Keep);
                    }
                    Ok(false) => {}
                    Err(error) => state.error(participant, error),
                }
            }
            ClientMessage::WatchCards => {
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                if !state.shows_cards(&space) {
                    state.error(participant, "This space shows no cards.");
                    return;
                }
                let record = state
                    .connections
                    .get_mut(&connection)
                    .expect("the connection acting is listed");
                record.cards = true;
                record.outbox.cards();
                self.cards_watched.notify(usize::MAX);
            }
            // Ambient, so dropped quietly, as the server side would drop an intent it does not take: outside a space whose environment is the one named at its version, too large, or too soon after the last. The side validates the rest. See `design/environments.md`.
            ClientMessage::EnvironmentIntent {
                environment,
                version,
                intent,
            } => {
                let current = space
                    .and_then(|space| state.space(space))
                    .is_some_and(|record| {
                        record.model.environment == environment
                            && self.environments().version(&environment) == Some(version)
                    });
                let text = intent.text();
                let now = Instant::now();
                // The participant's own bucket, kept across their connections.
                let record = state
                    .participants
                    .get_mut(participant)
                    .expect("the participant acting is listed");
                let taken = current
                    && text.len() <= tomori_protocol::MESSAGE_LIMIT
                    && record.intents.take(now);
                let source = record.source;
                if !(taken
                    && self.server_sides.deliver(
                        space.expect("checked above"),
                        tomori_environment::Intent::new(source, text),
                    ))
                {
                    log::debug!(target: LOG, "An environment intent was dropped.");
                }
            }
            ClientMessage::PlaceCard { card, placement } => {
                if let Err(error) = limits::check(&card, limits::IDENTIFIER_BYTES, "card") {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                if !state.shows_cards(&space) {
                    state.error(participant, "This space shows no cards.");
                    return;
                }
                let HubState { spaces, board, .. } = &mut *state;
                let board = board.as_ref().expect("checked above");
                let record = spaces
                    .iter_mut()
                    .find(|record| record.model.id == space)
                    .expect("checked above");
                match crate::cards::place(&mut record.placements, board, &card, placement) {
                    Ok(true) => {
                        // Everyone watching sees it move, and the layout is kept with the space.
                        state.touch_cards(Some(&space));
                        deferred.push(Deferred::Keep);
                    }
                    Ok(false) => {}
                    Err(error) => state.error(participant, error),
                }
            }
            // `act` hands this function only messages about the space's environment.
            _ => unreachable!("not a message about the space's environment"),
        }
    }
}
