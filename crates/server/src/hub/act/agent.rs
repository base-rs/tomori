// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent's figure: sending it home, calling, pinning and walking it, and how it moves.

use super::*;

impl Hub {
    /// Act on `message`, a message about the agent's figure, from `participant`, who is in `space` if any.
    pub(super) fn act_on_agent(
        &self,
        state: &mut HubState,
        participant: &str,
        space: Option<&str>,
        message: ClientMessage,
        deferred: &mut Vec<Deferred>,
    ) {
        match message {
            ClientMessage::SendAgentHome => {
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                let name = state.participants[participant].name.clone();
                state.agent_home(&space, &format!("{name} sent it home"), Instant::now());
            }
            ClientMessage::CallAgent => {
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                if let Err(error) = self.call_agent(state, &space, participant, Instant::now()) {
                    state.error(participant, error);
                }
            }
            ClientMessage::PinAgent { pinned } => {
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                if let Err(error) = state.pin_agent(&space, participant, pinned, Instant::now()) {
                    state.error(participant, error);
                }
            }
            ClientMessage::WalkAgent { walking } => {
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                let now = Instant::now();
                let done = if walking {
                    let name = state.participants[participant].name.clone();
                    self.walk_with(
                        state,
                        &space,
                        participant,
                        &format!(", as {name} asked from the hand menu"),
                        now,
                    )
                } else {
                    state.stop_walking(&space, participant, now)
                };
                if let Err(error) = done {
                    state.error(participant, error);
                }
            }
            ClientMessage::SetAgentMovement { movement } => {
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                let record = state.space_mut(&space).expect("checked above");
                match record.model.set_movement(movement) {
                    Ok(changed) => {
                        if changed {
                            let movement = record.model.movement();
                            let name = state.participants[participant].name.clone();
                            log::info!(
                                target: LOG,
                                "{name} set the agent of space {space} to move {:?}, going home after {:?} s.",
                                movement.mode,
                                movement.return_after
                            );
                            state.movement_changed(&space, &name, Instant::now());
                            state.touch_space(&space);
                            deferred.push(Deferred::Keep);
                        }
                    }
                    Err(error) => state.error(participant, error),
                }
            }
            // `act` hands this function only messages about the agent's figure.
            _ => unreachable!("not a message about the agent's figure"),
        }
    }
}
