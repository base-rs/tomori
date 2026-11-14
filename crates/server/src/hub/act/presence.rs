// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where participants are and whether they are engaged.

use super::*;

impl Hub {
    /// Act on `message`, a message about presence, from `participant` on `connection`, who is in `space` if any.
    pub(super) fn act_on_presence(
        &self,
        state: &mut HubState,
        connection: ConnectionId,
        participant: &str,
        space: Option<&str>,
        message: ClientMessage,
    ) {
        match message {
            ClientMessage::Presence {
                head,
                left,
                right,
                hands,
                carrying,
                carried,
                rays,
            } => {
                let Some(id) = space.filter(|space| state.space(space).is_some()) else {
                    return;
                };
                let space = state.space(id).expect("found above");
                let source = state.participants[participant].source;
                // The panel carried is the server's to know, and not relayed.
                let seen = crate::presence::Seen::reported(
                    head,
                    left,
                    right,
                    hands.as_deref(),
                    Instant::now(),
                )
                .map(|seen| seen.carrying_all(carrying, carried).showing(rays));
                for other in space.model.members() {
                    if other != participant {
                        state.send(
                            other,
                            ServerMessage::Presence {
                                source,
                                head,
                                left,
                                right,
                                hands: hands.clone(),
                            },
                        );
                    }
                }
                let record = state
                    .participants
                    .get_mut(participant)
                    .expect("a member is a participant");
                record.headset |= seen
                    .as_ref()
                    .is_some_and(crate::presence::Seen::tracks_hands);
                // Where a participant is, for the environment's server side, in its own coordinates, from the poses as
                // the server sounded them, so that nothing a client sends makes a side's checks fail open; a report whose
                // head is no pose tells it nothing. A visitor's presence is refused before it reaches here (`access.rs`).
                let sounded = seen
                    .as_ref()
                    .map(|seen| (seen.head, seen.controllers[0], seen.controllers[1]));
                record.seen = seen;
                let engagement = record.engagement;
                if let Some((head, left, right)) = sounded {
                    let environment = &state.space(id).expect("found above").model.environment;
                    let environments = self.environments();
                    let uncarry =
                        |pose: &tomori_protocol::Pose| environments.uncarry(environment, pose);
                    let presence = tomori_environment::Presence::new(
                        source,
                        uncarry(&head),
                        left.as_ref().map(uncarry),
                        right.as_ref().map(uncarry),
                        Instant::now(),
                    )
                    .with_engagement(engagement);
                    self.server_sides.presence(id, presence);
                }
                // A following agent keeps with whoever it talks with as they move. See `design/environments.md`.
                self.agent_seen(state, id, participant, Instant::now());
            }
            ClientMessage::Engagement { state: engagement } => {
                // A visitor's engagement is ignored: visitors follow live speech and show no presence. See `design/security.md`.
                if !state.role(connection, space).participates() {
                    return;
                }
                // A member is kept; only a change is applied.
                let Some(record) = state
                    .participants
                    .get_mut(participant)
                    .filter(|record| record.engagement != Some(engagement))
                else {
                    return;
                };
                log::debug!(target: LOG, "Participant {} is now {engagement:?}.", record.source);
                record.engagement = Some(engagement);
                let source = record.source;
                if let Some(space) = space {
                    // At once, whatever the presence throttle, since a headset lifted off says it is away and then stops
                    // reporting. See `tomori_environment::Space::presence`.
                    self.server_sides.engagement(space, source, engagement);
                }
                if let Some(space) = space
                    && let Some(record) = state.space_mut(space)
                {
                    if record.speech.engagement(participant, engagement)
                        || record.speech.held_changed()
                    {
                        record.wake();
                    }
                    state.touch_space(space);
                }
            }
            // `act` hands this function only messages about presence.
            _ => unreachable!("not a message about presence"),
        }
    }
}
