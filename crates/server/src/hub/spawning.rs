// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where people arrive as they join a space: the spawn point the server chooses for each, by their role and where the
//! others in the space stand, sent to them alone. See `design/environments.md`.
use super::*;

impl Hub {
    /// Tell `participant`, who has just joined the space they are in, or whose space has just changed environment,
    /// where they arrive: the spawn point of its environment that suits their role, where fewest stand, chosen as they
    /// arrive and never again until they next arrive. A reconnect, or a client joining again the space the server still
    /// keeps them in, as one reconnecting does, arrives where they arrived, counting nothing, so that only leaving the
    /// space, joining another or a change of environment chooses anew. With `anew`, as everyone in the space arrives in
    /// a new environment, where the others stood in the last means nothing, so only where they arrive counts. A client
    /// that did not say it reads it, and one that no point suits, is told nothing, and arrives at home. The participant
    /// is in a space, as the callers have just put them or found them there, and so connected.
    pub(super) fn arrive(&self, state: &mut HubState, participant: &str, anew: bool) {
        let record = &state.participants[participant];
        let space = record
            .space
            .clone()
            .expect("they have just joined, or are in, the space");
        let connection = &state.connections[&record
            .connection
            .expect("members of a space are connected, since disconnecting leaves it")];
        let reads = connection.spawns;
        let tag = tomori_protocol::spawn::tag(connection.access.role(Some(&space)));
        let outbox = connection.outbox.clone();
        // In this space, they have left any other, where they arrive anew.
        for other in state
            .spaces
            .iter_mut()
            .filter(|other| other.model.id != space)
        {
            other.arrivals.forget(participant);
        }
        if !reads {
            return;
        }
        let record = state.space(&space).expect("a space they are in is listed");
        let environment = record.model.environment.clone();
        let spawns = self.environments().anchors(&environment).spawns;
        if spawns.is_empty() {
            return;
        }
        let now = Instant::now();
        let others: Vec<(String, Option<[f32; 3]>)> = record
            .model
            .members()
            .filter(|member| *member != participant)
            .map(|member| {
                let head = state
                    .participants
                    .get(member)
                    .and_then(|other| other.seen.as_ref())
                    .filter(|seen| {
                        !anew && now.saturating_duration_since(seen.at) < crate::presence::STALE
                    })
                    .map(|seen| seen.head.position);
                (member.clone(), head)
            })
            .collect();
        let participants = &state.participants;
        let record = state
            .spaces
            .iter_mut()
            .find(|record| record.model.id == space)
            .expect("found above");
        let at = match record.arrivals.kept(participant) {
            Some(kept) => Some(kept),
            None => {
                // Those the server no longer keeps in the space, gone or away elsewhere, will arrive anew.
                record.arrivals.retain(|key| {
                    participants
                        .get(key)
                        .is_some_and(|kept| kept.last_space.as_deref() == Some(space.as_str()))
                });
                record.arrivals.choose(participant, tag, &spawns, &others)
            }
        };
        if let Some(at) = at {
            outbox.message(ServerMessage::Spawn {
                space,
                environment,
                at,
            });
        }
    }

    /// Everyone in `space` arrives anew, as it has changed environment, in the order they joined, participants first.
    pub(super) fn arrive_everyone(&self, state: &mut HubState, space: &str) {
        let record = state.space_mut(space).expect("the caller found it");
        record.arrivals.reset();
        let members: Vec<String> = record.model.members().cloned().collect();
        for member in members {
            self.arrive(state, &member, true);
        }
    }
}
