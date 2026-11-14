// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What participants say: their utterances, the queue, decisions and answers to the agent's questions, attribution and replaying replies.

use super::*;

impl Hub {
    /// Act on `message`, a message about speech, from `participant` on `connection`, who is in `space` if any.
    pub(super) fn act_on_speech(
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
            ClientMessage::Gate { .. } => {
                // Each audio frame carries its own gate; the announcement needs no state yet.
            }
            ClientMessage::UtteranceBegin {
                utterance,
                answer,
                progress,
                naming,
                titling,
                attention,
                seen,
            } => {
                if naming && titling {
                    state.error(
                        participant,
                        "Words cannot say both your name and the space's title.",
                    );
                    return;
                }
                let checked =
                    limits::check(&utterance, limits::UTTERANCE_BYTES, "utterance identifier")
                        .and_then(|()| match &answer {
                            Some(target) => limits::check(
                                &target.question,
                                limits::IDENTIFIER_BYTES,
                                "question",
                            )
                            .and_then(|()| {
                                limits::check(&target.field, limits::IDENTIFIER_BYTES, "field")
                            }),
                            None => Ok(()),
                        })
                        .and_then(|()| match &seen {
                            Some(message) => {
                                limits::check(message, limits::IDENTIFIER_BYTES, "message")
                            }
                            None => Ok(()),
                        });
                if let Err(error) = checked {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                // Words for the agent, rather than an answer, a name or a title, turn a following agent to them.
                let question = answer.is_none() && !naming && !titling;
                if let Some(open) = state
                    .connections
                    .get_mut(&connection)
                    .and_then(|record| record.utterance.take())
                {
                    state.utterances.remove(&open.id);
                    deferred.push(Deferred::Cancel(open.id));
                    outbox.message(ServerMessage::TranscriptFailed {
                        utterance: open.client,
                        error: "A new utterance started before this one ended.".to_owned(),
                    });
                }
                // What the participant attends to is captured now, as they begin: only identities are taken from the
                // client, which the space resolves, ignoring those it does not have, cards only on the board of a
                // space that shows them. An answer goes to the question presented, and a name to the participant, so
                // either is about nothing attended to.
                // What they point at among the objects the environment describes, all of which it is resolved among,
                // ranks first in the agent's ambient list for their words, once those are queued. See `crate::ambient`.
                let pointed: Vec<String> = state
                    .space(&space)
                    .filter(|_| question)
                    .and_then(|record| {
                        record
                            .channel
                            .as_ref()
                            .filter(|channel| channel.environment == record.model.environment)
                    })
                    .map(|channel| {
                        attention
                            .iter()
                            .take(tomori_protocol::ATTENTION_LIMIT)
                            .filter_map(|resource| match resource {
                                tomori_protocol::Resource::Object { object, .. }
                                    if channel.objects.get(object).is_some() =>
                                {
                                    Some(object.clone())
                                }
                                _ => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let attention = match state.space(&space) {
                    Some(record) if answer.is_none() && !naming && !titling => {
                        let environments = self.environments();
                        let walls = environments.walls(&record.model.environment);
                        let now = crate::cards::unix_now();
                        record.model.resolve_with(
                            &attention,
                            state.cards_of(&space),
                            record.channel.as_ref(),
                            &|panel, at| record.stage.describe_at(&walls, panel, at, now),
                            &|anchor| record.gathered(&environments, anchor),
                        )
                    }
                    _ => Vec::new(),
                };
                // So is what they had seen of the agent's messages, which words for the agent follow, unless it says
                // more before they arrive.
                let seen = state
                    .space(&space)
                    .filter(|_| answer.is_none() && !naming && !titling)
                    .map(|record| record.model.seen(seen.as_deref()));
                let origin = state.participants[participant]
                    .seen
                    .as_ref()
                    .map(|seen| seen.head);
                state.next_utterance += 1;
                let id = UtteranceId(state.next_utterance);
                state.utterances.insert(
                    id,
                    UtteranceRecord {
                        participant: participant.to_owned(),
                        source: state.participants[participant].source,
                        client: utterance.clone(),
                        space: space.clone(),
                        // Words that name the participant answer nothing, since they are never queued.
                        answer,
                        naming,
                        titling,
                        attention,
                        pointed,
                        seen,
                        origin,
                        progress: progress.then_some(connection),
                        begun: Instant::now(),
                        ended: None,
                        waiting: None,
                        waited: Duration::ZERO,
                    },
                );
                let record = state
                    .connections
                    .get_mut(&connection)
                    .expect("checked above");
                record.straying = false;
                record.failed = None;
                record.utterance = Some(OpenUtterance {
                    client: utterance,
                    id,
                    expected: None,
                });
                // A reply that starts while they speak is held for them until they stop, rather than talking over them.
                state
                    .space_mut(&space)
                    .expect("checked above")
                    .speech
                    .recording(participant);
                // Everyone in it sees that someone is speaking to the agent.
                state.agent_changed(&space, Instant::now());
                if question && let Some(captured) = self.capture(state, &space, participant) {
                    state.agent_pressed(&space, participant, captured, Instant::now());
                }
                deferred.push(Deferred::Begin(id));
            }
            ClientMessage::UtteranceEnd { utterance, .. }
            | ClientMessage::UtteranceCancel { utterance }
                if state.connections[&connection].failed.as_ref() == Some(&utterance) =>
            {
                // Already failed and reported: the client sent this before it heard.
                state
                    .connections
                    .entry(connection)
                    .and_modify(|record| record.failed = None);
            }
            ClientMessage::UtteranceEnd { utterance, .. }
            | ClientMessage::UtteranceCancel { utterance }
                if !state.connections[&connection]
                    .utterance
                    .as_ref()
                    .is_some_and(|open| open.client == utterance) =>
            {
                state.error(participant, "That utterance is not open.");
            }
            ClientMessage::UtteranceEnd { pointed, .. } => {
                let open = state
                    .connections
                    .get_mut(&connection)
                    .and_then(|record| record.utterance.take())
                    .expect("checked by the previous arm");
                // A client from before attention began utterances names what it pointed at as they end instead. That
                // is taken, resolved now as the space resolves attention, only for an utterance that began attending to
                // nothing, for the agent and not an answer or a name, so that it can never replace what was captured
                // as the utterance began.
                let pointed = pointed.and_then(|resource| {
                    state
                        .utterances
                        .get(&open.id)
                        .filter(|record| {
                            record.attention.is_empty()
                                && record.answer.is_none()
                                && !record.naming
                                && !record.titling
                        })
                        .and_then(|record| {
                            state.space(&record.space).map(|space| {
                                let environments = self.environments();
                                let walls = environments.walls(&space.model.environment);
                                let now = crate::cards::unix_now();
                                space.model.resolve_with(
                                    &[resource],
                                    state.cards_of(&record.space),
                                    space.channel.as_ref(),
                                    &|panel, at| space.stage.describe_at(&walls, panel, at, now),
                                    &|anchor| space.gathered(&environments, anchor),
                                )
                            })
                        })
                });
                // They have let go of X, so what was held for them while they spoke plays after a short pause.
                let ended = state
                    .utterances
                    .get(&open.id)
                    .map(|record| (record.participant.clone(), record.space.clone()));
                if let Some((participant, space)) = &ended {
                    state.agent_released(space, participant, Instant::now());
                }
                state.released(ended, Instant::now());
                // Its frames and its end share one ordered stream, so all of its audio has arrived: the participant is told that it is being transcribed, before anything recognition reports.
                if let Some(record) = state.utterances.get_mut(&open.id) {
                    if let Some(attention) = pointed {
                        record.attention = attention;
                    }
                    record.ended = Some(Instant::now());
                    // The end arrives on the connection that began the utterance, as a reconnect closes it.
                    if record.progress.is_some() {
                        outbox.message(ServerMessage::TranscriptProgress {
                            utterance: open.client,
                            stage: tomori_protocol::TranscriptStage::Transcribing,
                            part: None,
                        });
                    }
                }
                deferred.push(Deferred::End(open.id));
            }
            ClientMessage::UtteranceCancel { .. } => {
                state.abandon_utterance(connection, deferred);
            }
            ClientMessage::CancelQueued { entry } => {
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                let record = state.space_mut(&space).expect("checked above");
                match record.model.cancel(&entry) {
                    Ok(()) => state.touch_space(&space),
                    Err(error) => state.error(participant, error),
                }
            }
            ClientMessage::Decide {
                request,
                decision,
                option,
            } => {
                let checked = limits::check(&request, limits::IDENTIFIER_BYTES, "request")
                    .and_then(|()| match &option {
                        Some(option) => limits::check(option, limits::IDENTIFIER_BYTES, "option"),
                        None => Ok(()),
                    });
                if let Err(error) = checked {
                    state.error(participant, error);
                    return;
                }
                let decision = match (decision, option) {
                    (tomori_protocol::Decision::AllowOnce, None) => {
                        tomori_backend::Decision::AllowOnce
                    }
                    (tomori_protocol::Decision::Deny, None) => tomori_backend::Decision::Deny,
                    (tomori_protocol::Decision::Grant, Some(option)) => {
                        tomori_backend::Decision::Grant { option }
                    }
                    (tomori_protocol::Decision::Grant, None) => {
                        state.error(participant, "Choose an option to grant.");
                        return;
                    }
                    (_, Some(_)) => {
                        state.error(participant, "Only a grant decision takes an option.");
                        return;
                    }
                };
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                let record = state.space_mut(&space).expect("checked above");
                let option = match &decision {
                    tomori_backend::Decision::Grant { option } => Some(option.as_str()),
                    _ => None,
                };
                match record.model.claim_approval(&request, option) {
                    Ok(()) => {
                        let sent = record.commands.try_send(SpaceCommand::Decide {
                            request: request.clone(),
                            decision,
                            participant: participant.to_owned(),
                        });
                        if sent.is_err() {
                            record.model.restore_approval(&request);
                            state.error(participant, "The agent session has stopped.");
                        }
                        state.touch_space(&space);
                    }
                    Err(error) => state.error(participant, error),
                }
            }
            ClientMessage::Answer {
                question,
                field,
                text,
            } => {
                let checked = limits::check(&text, limits::ANSWER_BYTES, "answer")
                    .and_then(|()| limits::check(&question, limits::IDENTIFIER_BYTES, "question"))
                    .and_then(|()| limits::check(&field, limits::IDENTIFIER_BYTES, "field"));
                if let Err(error) = checked {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                if let Err(error) = state.answer(participant, &space, &question, &field, &text) {
                    state.error(participant, error);
                }
            }
            ClientMessage::SetAttribution { enabled } => {
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                state
                    .space_mut(&space)
                    .expect("checked above")
                    .model
                    .set_attribution(enabled);
                state.touch_space(&space);
                deferred.push(Deferred::Keep);
            }
            ClientMessage::Replay { message } => {
                if let Some(message) = &message
                    && let Err(error) = limits::check(message, limits::IDENTIFIER_BYTES, "message")
                {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                let record = state.space_mut(&space).expect("checked above");
                match record.speech.replay(participant, message.as_deref()) {
                    Ok(()) => record.wake(),
                    Err(error) => state.error(participant, error),
                }
            }
            ClientMessage::StopReplay => {
                // Stopping is harmless when nothing plays, so a stop that crosses the end of a replay is not refused.
                if let Some(record) = space.and_then(|space| state.space_mut(space))
                    && record.speech.stop(participant)
                {
                    // A later reply waits for them, and is heard from its start now.
                    record.wake();
                }
            }
            // `act` hands this function only messages about speech.
            _ => unreachable!("not a message about speech"),
        }
    }
}
