// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The environment state channel as a client keeps it: the state the server side of the joined space's environment
//! last sent, and its events since, until the environment's client side takes them. See `design/environments.md`.
use tomori_protocol::Payload;

/// The most updates kept for a client side that has not taken them, such as the console's, which never does: the
/// latest, after the latest state.
pub const KEPT: usize = 64;

/// What the server side of an environment sent: its state, which replaces whatever was sent before, or an event.
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    State(Payload),
    Event(Payload),
}

/// Which space, environment and version a replica is for.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Key {
    space: String,
    environment: String,
    version: u32,
}

/// The state and events of one space's environment, as the server sent them, waiting to be taken.
#[derive(Debug, Default)]
pub struct Replica {
    key: Option<Key>,
    updates: Vec<Update>,
}

impl Replica {
    /// The server sent `update` for `environment` at `version` in `space`. A state for another space, environment or
    /// version begins anew; an event for one this replica has no state for is dropped, as it changes nothing the client
    /// has.
    pub fn receive(&mut self, space: &str, environment: &str, version: u32, update: Update) {
        let key = Key {
            space: space.to_owned(),
            environment: environment.to_owned(),
            version,
        };
        match update {
            Update::State(_) => {
                self.key = Some(key);
                self.updates.clear();
            }
            Update::Event(_) if self.key.as_ref() != Some(&key) => return,
            Update::Event(_) => {
                // The oldest event after the latest state goes first; the state stays, so that the client side has
                // something to begin from.
                if self.updates.len() == KEPT {
                    let first = usize::from(matches!(self.updates.first(), Some(Update::State(_))));
                    self.updates.remove(first);
                }
            }
        }
        self.updates.push(update);
    }

    /// Whether the server has sent a state for `environment` at `version` in `space`, so that it runs the environment's
    /// server side and takes intents for it.
    pub fn has(&self, space: &str, environment: &str, version: u32) -> bool {
        self.key.as_ref().is_some_and(|key| {
            key.space == space && key.environment == environment && key.version == version
        })
    }

    /// The updates for `environment` at `version` in `space` since they were last taken, in order; none for another.
    pub fn take(&mut self, space: &str, environment: &str, version: u32) -> Vec<Update> {
        if self.has(space, environment, version) {
            std::mem::take(&mut self.updates)
        } else {
            Vec::new()
        }
    }

    /// Forget everything, as on leaving a space.
    pub fn clear(&mut self) {
        self.key = None;
        self.updates.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(value: u32) -> Payload {
        Payload::new(serde_json::json!(value))
    }

    #[test]
    fn a_state_begins_anew_and_events_follow_it() {
        let mut replica = Replica::default();
        // An event before any state changes nothing the client has.
        replica.receive("s", "garden", 1, Update::Event(payload(0)));
        assert!(!replica.has("s", "garden", 1));
        replica.receive("s", "garden", 1, Update::State(payload(1)));
        replica.receive("s", "garden", 1, Update::Event(payload(2)));
        // An event of another space, environment or version is not this one's.
        replica.receive("t", "garden", 1, Update::Event(payload(3)));
        replica.receive("s", "sky", 1, Update::Event(payload(3)));
        replica.receive("s", "garden", 2, Update::Event(payload(3)));
        assert!(replica.take("s", "garden", 2).is_empty());
        assert_eq!(
            replica.take("s", "garden", 1),
            [Update::State(payload(1)), Update::Event(payload(2))]
        );
        assert!(replica.take("s", "garden", 1).is_empty());
        assert!(replica.has("s", "garden", 1));
        // A state for another space begins anew.
        replica.receive("t", "garden", 1, Update::Event(payload(4)));
        replica.receive("t", "garden", 1, Update::State(payload(5)));
        assert!(!replica.has("s", "garden", 1));
        assert_eq!(replica.take("t", "garden", 1), [Update::State(payload(5))]);
        replica.clear();
        assert!(!replica.has("t", "garden", 1));
    }

    #[test]
    fn untaken_updates_keep_the_state_and_the_latest_events() {
        let mut replica = Replica::default();
        replica.receive("s", "garden", 1, Update::State(payload(0)));
        for event in 1..=KEPT as u32 + 5 {
            replica.receive("s", "garden", 1, Update::Event(payload(event)));
        }
        let updates = replica.take("s", "garden", 1);
        assert_eq!(updates.len(), KEPT);
        assert_eq!(updates[0], Update::State(payload(0)));
        assert_eq!(updates[1], Update::Event(payload(7)));
        // Taken, only events wait, and the oldest goes first.
        for event in 0..=KEPT as u32 {
            replica.receive("s", "garden", 1, Update::Event(payload(event)));
        }
        let updates = replica.take("s", "garden", 1);
        assert_eq!(updates.len(), KEPT);
        assert_eq!(updates[0], Update::Event(payload(1)));
    }
}
