// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent's `set_my_name`: whoever spoke renames the device they speak from by saying so, as in "my name is Ayoko", such as when someone else puts on a headset. Only the speaker's own name changes, as the server checks who spoke, never anyone else's, and only their name: their identity, token, role and what they may do stay as they were, so a person borrowing an owner's headset still acts as that owner. See `design/security.md`.
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use super::provenance::Needs;
use super::{Hub, Unkept};
use crate::limits;

/// Who may rename themselves by asking the agent: an owner, or a collaborator in the space, the roles whose words reach it, who may already rename themselves in the hand menu, so that asking the agent grants nothing new. A visitor's words never reach the agent, and a connection admitted only to pair has none. See `design/security.md`.
pub(super) const RENAMES: Needs = Needs::Participant;

/// At most this many renames through the agent, for one identity, in [`RENAME_WINDOW`]: ample for a household passing a headset round, and few enough that a turn the agent is misled in cannot keep renaming someone, writing the token store and telling everyone each time. See `design/security.md`.
pub const RENAME_LIMIT: usize = 3;

/// The window [`RENAME_LIMIT`] counts renames in.
pub const RENAME_WINDOW: Duration = Duration::from_secs(60);

/// When each identity was renamed through the agent within [`RENAME_WINDOW`], by its principal, or by its hub key for a participant without a token.
#[derive(Default)]
pub(super) struct Renames(HashMap<String, VecDeque<Instant>>);

impl Renames {
    /// Count a rename of `who` at `now`, if they have had fewer than [`RENAME_LIMIT`] in the [`RENAME_WINDOW`] before it, forgetting every rename older than that, so that what is kept stays bounded by the renames of the last minute.
    fn admit(&mut self, who: &str, now: Instant) -> bool {
        self.0.retain(|_, times| {
            while times
                .front()
                .is_some_and(|at| now.saturating_duration_since(*at) >= RENAME_WINDOW)
            {
                times.pop_front();
            }
            !times.is_empty()
        });
        let times = self.0.entry(who.to_owned()).or_default();
        if times.len() >= RENAME_LIMIT {
            return false;
        }
        times.push_back(now);
        true
    }
}

/// What `set_my_name` did, for the tool's result.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Renamed {
    /// The space whose agent renamed them.
    pub space: String,
    /// Their name now, cleaned as every name is.
    pub name: String,
    /// Their handle now, as `presence` gives it, such as `Ayoko#23`.
    pub id: String,
    /// Whether their name changed: the name they have already changes nothing.
    pub changed: bool,
    /// The handles of the others in the space who share their name now, so that the agent names each by their handle.
    pub sharing: Vec<String>,
}

impl Hub {
    /// Rename whoever's words the agent of the space bound to `session` was given last, `requested_by`, to `name`, as they asked, on their own device: their token's name, kept in the token store and so on every device presenting it and after reconnecting, or, without a token, as in `--auth open` mode, for as long as the server keeps them. The server checks who spoke, as for the gated tools, that they are an owner or a collaborator in the space ([`RENAMES`]), and that they have been renamed this way fewer than [`RENAME_LIMIT`] times in the last [`RENAME_WINDOW`]. Their role and permissions are unchanged. Logged at info. See `design/security.md`.
    pub(crate) fn agent_set_my_name(
        &self,
        session: &str,
        name: &str,
        requested_by: Option<&str>,
    ) -> Result<Renamed, String> {
        // As a name said in the hand menu is taken, without the full stop recognition may end it with.
        let name = limits::name(super::spoken_name(name))
            .map_err(|error| format!("Name not changed: {error}"))?;
        let mut state = self.lock();
        let space = self.calling_space(&state, session)?;
        let requester = self.requester(&state, &space, requested_by, RENAMES)?;
        // The speaker, whom the check above found under this lock; only they are renamed, whoever the name is.
        let source = state
            .given(&space)
            .map(|given| given.source)
            .expect("the words checked are given");
        let (key, principal, previous) = state
            .participants
            .iter()
            .find(|(_, participant)| participant.source == source)
            .map(|(key, participant)| {
                (
                    key.clone(),
                    participant.principal.clone(),
                    participant.name.clone(),
                )
            })
            .expect("whoever asked is known, as the check found");
        let changed = previous != name;
        if changed {
            if !state
                .renames
                .admit(principal.as_deref().unwrap_or(&key), Instant::now())
            {
                log::info!(
                    "The agent of space {space}, as {} asked, was refused a rename to {name:?}: the most renames in a minute.",
                    requester.logged()
                );
                return Err(format!(
                    "{previous} has been renamed {RENAME_LIMIT} times in the last minute, the most Tomori allows, so their name stays {previous}. Ask them to wait a minute and say it again."
                ));
            }
            match (principal, self.token_store()) {
                (Some(principal), Some(tokens)) => {
                    drop(state);
                    let (again, kept) = self.write_name(&tokens, &principal, &name);
                    state = again;
                    kept.map_err(|unkept| match unkept {
                        Unkept::Revoked => format!(
                            "Name not changed: {previous}'s token is no longer issued."
                        ),
                        Unkept::Unwritten => format!(
                            "Name not changed: Tomori could not keep {previous}'s new name. Ask them to say it again."
                        ),
                    })?;
                }
                // Anyone without a token, as in `--auth open` mode, is renamed only while the server keeps them.
                _ => state.rename(|candidate, _| candidate == key, &name),
            }
            log::info!(
                "The agent of space {space}, as {} asked, renamed them from {previous:?} to {name:?}.",
                requester.logged()
            );
        }
        let tag = state
            .participants
            .get(&key)
            .and_then(|participant| participant.tag);
        let id = crate::presence::handle(&name, tag);
        let folded = tomori_protocol::text::name_key(&name);
        let mut sharing: Vec<String> = state
            .heads(&space)
            .iter()
            .filter(|person| {
                tomori_protocol::text::name_key(&person.name) == folded && person.tag != tag
            })
            .map(crate::presence::Someone::handle)
            .collect();
        sharing.sort();
        sharing.dedup();
        Ok(Renamed {
            space,
            name,
            id,
            changed,
            sharing,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_are_limited_for_each_identity_within_the_window() {
        let mut renames = Renames::default();
        let start = Instant::now();
        for n in 0..RENAME_LIMIT {
            assert!(renames.admit("token:ada", start + Duration::from_secs(n as u64)));
        }
        // The next within the window is refused, and refusing counts nothing.
        assert!(!renames.admit("token:ada", start + Duration::from_secs(30)));
        assert!(!renames.admit(
            "token:ada",
            start + RENAME_WINDOW - Duration::from_millis(1)
        ));
        // Someone else is counted apart.
        assert!(renames.admit("token:bob", start + Duration::from_secs(30)));
        // A minute after the first, it no longer counts, and the oldest are forgotten.
        assert!(renames.admit("token:ada", start + RENAME_WINDOW));
        assert!(!renames.admit("token:ada", start + RENAME_WINDOW));
        // Once every rename is older than the window, nothing is kept of anyone.
        assert!(renames.admit("token:cy", start + RENAME_WINDOW * 3));
        assert_eq!(renames.0.len(), 1);
    }
}
