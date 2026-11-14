// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Acting on what a participant's client sends, once it is admitted: each family of messages in a module of its own.

mod agent;
mod environment;
mod pairing;
mod panels;
mod presence;
mod spaces;
mod speech;
mod walls;

use super::*;

/// The target of what is logged here: the hub's, whose module this was once in.
const LOG: &str = "tomori_server::hub";

/// The current space, `space`, if it is listed; else `participant` is told to join one first.
fn in_space(state: &HubState, participant: &str, space: Option<&str>) -> Option<String> {
    match space {
        Some(space) if state.space(space).is_some() => Some(space.to_owned()),
        _ => {
            state.error(participant, "Join a space first.");
            None
        }
    }
}

impl Hub {
    /// Act on `message` from `participant` on `connection`, who is in `space` if any, handing it to its family's function.
    pub(super) fn act(
        &self,
        state: &mut HubState,
        connection: ConnectionId,
        participant: &str,
        space: Option<&str>,
        message: ClientMessage,
        deferred: &mut Vec<Deferred>,
    ) {
        match message {
            ClientMessage::Hello { .. } => unreachable!("handled by hello"),
            ClientMessage::Ping { .. } | ClientMessage::Pong { .. } => {
                unreachable!("handled before membership")
            }
            message @ (ClientMessage::ListSpaces
            | ClientMessage::SetName { .. }
            | ClientMessage::CreateSpace { .. }
            | ClientMessage::ListSessions { .. }
            | ClientMessage::Join { .. }
            | ClientMessage::Leave
            | ClientMessage::SetMode { .. }
            | ClientMessage::SetInstructions { .. }
            | ClientMessage::DeleteSpace { .. }
            | ClientMessage::RenameSpace { .. }
            | ClientMessage::SetSpeech { .. }) => {
                self.act_on_spaces(state, connection, participant, space, message, deferred);
            }
            message @ (ClientMessage::Gate { .. }
            | ClientMessage::UtteranceBegin { .. }
            | ClientMessage::UtteranceEnd { .. }
            | ClientMessage::UtteranceCancel { .. }
            | ClientMessage::CancelQueued { .. }
            | ClientMessage::Decide { .. }
            | ClientMessage::Answer { .. }
            | ClientMessage::SetAttribution { .. }
            | ClientMessage::Replay { .. }
            | ClientMessage::StopReplay) => {
                self.act_on_speech(state, connection, participant, space, message, deferred);
            }
            message @ (ClientMessage::Presence { .. } | ClientMessage::Engagement { .. }) => {
                self.act_on_presence(state, connection, participant, space, message);
            }
            message @ (ClientMessage::PairBegin { .. }
            | ClientMessage::PairReveal { .. }
            | ClientMessage::Knock
            | ClientMessage::DecideKnock { .. }
            | ClientMessage::PairCancel
            | ClientMessage::OpenPairing
            | ClientMessage::ClosePairing) => {
                self.act_on_pairing(state, connection, participant, space, message, deferred);
            }
            message @ (ClientMessage::AddWall { .. }
            | ClientMessage::SetWall { .. }
            | ClientMessage::AttachWall { .. }
            | ClientMessage::LockWall { .. }
            | ClientMessage::RemoveWall { .. }) => {
                self.act_on_walls(state, participant, space, message, deferred);
            }
            message @ (ClientMessage::PlacePanel { .. }
            | ClientMessage::ResyncProjection { .. }
            | ClientMessage::FetchImage { .. }
            | ClientMessage::DismissPanel { .. }
            | ClientMessage::CheckPanelItem { .. }
            | ClientMessage::FreezePanel { .. }
            | ClientMessage::ChoosePanelRow { .. }) => {
                self.act_on_panels(state, connection, participant, space, message, deferred);
            }
            message @ (ClientMessage::SetEnvironment { .. }
            | ClientMessage::EnvironmentIntent { .. }
            | ClientMessage::WatchCards
            | ClientMessage::PlaceCard { .. }) => {
                self.act_on_environment(state, connection, participant, space, message, deferred);
            }
            message @ (ClientMessage::PlaceAgent { .. }
            | ClientMessage::CallAgent
            | ClientMessage::PinAgent { .. }
            | ClientMessage::WalkAgent { .. }
            | ClientMessage::SetAgentMovement { .. }) => {
                self.act_on_agent(state, participant, space, message, deferred);
            }
        }
    }
}
