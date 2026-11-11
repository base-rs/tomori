// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Who asked for what an agent does with a security-sensitive tool, checked by the server rather than left to the agent's judgement. Each such tool requires `requested_by`, the participant whose words asked for it, and the server checks, in code, that the entry of input given to the agent last, which began its current turn, is theirs, was given within [`REQUEST_RECENT`], came from a participant's connection, has not been followed by input that did not come through Tomori, and that they are in the space now with the role the tool needs. See `design/security.md`.
use std::time::{Duration, Instant};

use tomori_protocol::ConnectionRole;

use super::{Hub, HubState};
use crate::space::Superseded;

/// How recently a participant's entry must have been given to the agent for a tool to act on it: long enough for an agent to call the tool a request asks for, which it does at once, and short enough that words said earlier, or a turn left running, are not a standing permission. See `design/security.md`.
pub const REQUEST_RECENT: Duration = Duration::from_secs(180);

/// The role a tool needs of whoever asked for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Needs {
    /// An owner, as for deciding a pairing, which can make a device an owner.
    Owner,
    /// An owner or a collaborator in the space, a participant, as for stopping the space's workers.
    Participant,
}

impl Needs {
    fn permits(self, role: ConnectionRole) -> bool {
        match self {
            Self::Owner => role == ConnectionRole::Owner,
            Self::Participant => {
                matches!(role, ConnectionRole::Owner | ConnectionRole::Collaborator)
            }
        }
    }

    /// Who may ask, as a refusal says it.
    fn who(self) -> &'static str {
        match self {
            Self::Owner => "an owner",
            Self::Participant => "an owner or a collaborator in this space",
        }
    }
}

/// Who asked, as the server checked it: their name as the server knows them now, and Tomori's number for their entry and how long ago it was given to the agent, for the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Requester {
    pub name: String,
    /// Their tag, which tells them apart in the log from anyone else of their name.
    pub tag: Option<u32>,
    pub entry: u64,
    pub ago: Duration,
}

impl Requester {
    /// The requester as the log and the console say it, by their handle, since names may repeat:
    /// `Ada#23 (entry 3, given 12 seconds ago)`.
    pub fn logged(&self) -> String {
        format!(
            "{} (entry {}, given {} seconds ago)",
            crate::presence::handle(&self.name, self.tag),
            self.entry,
            self.ago.as_secs()
        )
    }
}

/// The name of a role refused, as a refusal says it: never an owner's, since an owner may ask for anything these tools do.
fn role_name(role: ConnectionRole) -> &'static str {
    match role {
        ConnectionRole::Collaborator => "a collaborator",
        _ => "a visitor",
    }
}

impl Hub {
    /// [`HubState::requester`], within [`REQUEST_RECENT`] of now.
    pub(super) fn requester(
        &self,
        state: &HubState,
        space: &str,
        requested_by: Option<&str>,
        needs: Needs,
    ) -> Result<Requester, String> {
        state.requester(space, requested_by, needs, Instant::now())
    }
}

/// How long ago `given`, the entry given to the agent last, was given, if within [`REQUEST_RECENT`] of `now`, so that
/// words said earlier, or a turn left running, are no standing permission; or why not, for the agent. See
/// `design/security.md`.
pub(super) fn recent(given: &crate::space::Given, now: Instant) -> Result<Duration, String> {
    let ago = now.saturating_duration_since(given.at);
    if ago > REQUEST_RECENT {
        return Err(format!(
            "The words you were given last in this space are more than {} old, so Tomori will not act on them. Ask whoever wants this to say it again.",
            super::spoken(REQUEST_RECENT)
        ));
    }
    Ok(ago)
}

/// Whether `given`, the entry given to the agent last, still stands behind what it does: nothing reached its session
/// since that did not come through Tomori, Tomori gave it no notes of its own since, such as a timer firing, and Tomori
/// has not lost its connection to the session since. Returns why not, for the agent. See `design/security.md`.
pub(super) fn standing(given: &crate::space::Given) -> Result<(), String> {
    // Input that did not come through Tomori, typed into the session or reported by the agent's service, came after the words, so they are not what the agent acts on now.
    match given.superseded {
            Some(Superseded::Input) => {
                Err(
                    "Input that did not come through Tomori reached you after the words you were given last in this space, such as a message typed into your session or a sub-agent's report, so Tomori cannot tell that a participant asked for this. Ask whoever wants this to say it again, and wait for their words before you act."
                        .to_owned(),
                )
            }
            Some(Superseded::Event) => {
                Err(
                    "Tomori gave you its own notes after the words you were given last in this space, such as a timer firing, which no participant asked for, so Tomori cannot tell that a participant asked for this. Ask whoever wants this to say it again, and wait for their words before you act."
                        .to_owned(),
                )
            }
            Some(Superseded::Reconnected) => {
                Err(
                    "Tomori lost its connection to your session after the words you were given last in this space, so it cannot tell that nothing else reached you since. Ask whoever wants this to say it again, and wait for their words before you act."
                        .to_owned(),
                )
            }
            None => Ok(()),
        }
}

impl HubState {
    /// The entry given to the agent of `space` last, which began its current turn, or why there is none. See
    /// `design/security.md`.
    pub(super) fn given(&self, space: &str) -> Result<&crate::space::Given, String> {
        self.space(space)
            .and_then(|record| record.model.given())
            .ok_or_else(|| {
                "No participant's words have been given to you in this space since the server started, so Tomori cannot tell who asked. Ask them to say it.".to_owned()
            })
    }

    /// Check that `requested_by` asked for what the agent of `space` is about to do, as [`Needs`] says they must be able to: the entry given to the agent last, within [`REQUEST_RECENT`] of `now`, is theirs, said on a participant's connection, is still the agent's latest input, with nothing given to its session since that did not come through Tomori, and they are in the space now, connected, with the role needed. Returns who asked, or why the tool is refused, which names no one but whoever the agent named. See `design/security.md`.
    pub(super) fn requester(
        &self,
        space: &str,
        requested_by: Option<&str>,
        needs: Needs,
        now: Instant,
    ) -> Result<Requester, String> {
        // A name by the rules every display name follows, so that what the agent passes is bounded and on one line wherever a refusal repeats it.
        // A handle, `Ada#23`, is a name and a tag, the name by those rules, so that a name of the most characters has one.
        let valid = |name: &str| tomori_protocol::text::display_name(name).ok();
        let requested_by = requested_by
            .and_then(|named| {
                let handle = named.trim().rsplit_once('#').filter(|(_, tag)| {
                    (1..=4).contains(&tag.len()) && tag.bytes().all(|byte| byte.is_ascii_digit())
                });
                match handle.and_then(|(name, tag)| valid(name).zip(Some(tag))) {
                    Some((name, tag)) => Some(format!("{name}#{tag}")),
                    None => valid(named),
                }
            })
            .ok_or(
                "This tool needs requested_by: the name of the participant whose words asked for it, as Tomori gave you their words, such as Ada for \"Ada: …\".",
            )?;
        let given = self.given(space)?;
        let ago = recent(given, now)?;
        standing(given)?;
        // The speaker as the agent read them: the words that began its turn are the request it acts on. A handle,
        // `Ada#23`, names them by their tag too; a name alone must be theirs alone in the space.
        let key = tomori_protocol::text::name_key;
        let speaker = key(&given.speaker);
        let handle = requested_by
            .rsplit_once('#')
            .filter(|(name, tag)| {
                !tag.is_empty()
                    && tag.bytes().all(|byte| byte.is_ascii_digit())
                    && key(name) == speaker
            })
            .and_then(|(_, tag)| tag.parse::<u32>().ok());
        let not_theirs = "The words you were given last in this space are not from the participant you named, so Tomori cannot tell that they asked. Ask them to say it again, and wait for their words before you act.";
        if handle.is_none() && key(&requested_by) != speaker {
            return Err(not_theirs.to_owned());
        }
        let Some(participant) = self
            .participants
            .values()
            .find(|participant| participant.source == given.source)
        else {
            return Err(format!(
                "{requested_by} is no longer known to this server, so Tomori cannot check that they may ask for this."
            ));
        };
        match handle {
            Some(tag) if participant.tag != Some(tag) => return Err(not_theirs.to_owned()),
            Some(_) => {}
            None => {
                let mut called: Vec<String> = self
                    .heads(space)
                    .iter()
                    .filter(|person| key(&person.name) == speaker)
                    .map(crate::presence::Someone::handle)
                    .collect();
                // One identity on several devices is one person.
                called.sort();
                called.dedup();
                if called.len() > 1 {
                    return Err(format!(
                        "More than one person in this space is called {requested_by:?}: say which, as {}.",
                        called.join(" or ")
                    ));
                }
            }
        }
        // Only a participant's connection queues input, but a visitor's record is never trusted to have.
        let connection = participant
            .connection
            .and_then(|connection| self.connections.get(&connection))
            .filter(|_| !participant.visitor && participant.space.as_deref() == Some(space));
        let Some(connection) = connection else {
            return Err(format!(
                "{requested_by} is not in this space now, so Tomori will not act on their word. Ask them to come back and say it again."
            ));
        };
        let role = connection.access.role(Some(space));
        if !needs.permits(role) {
            return Err(format!(
                "{requested_by} is {} here, and only {} may ask for this.",
                role_name(role),
                needs.who()
            ));
        }
        Ok(Requester {
            name: participant.name.clone(),
            tag: participant.tag,
            entry: given.entry,
            ago,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_permit_only_their_roles() {
        assert!(Needs::Owner.permits(ConnectionRole::Owner));
        assert!(!Needs::Owner.permits(ConnectionRole::Collaborator));
        assert!(Needs::Participant.permits(ConnectionRole::Collaborator));
        assert!(!Needs::Participant.permits(ConnectionRole::Visitor));
        assert!(!Needs::Participant.permits(ConnectionRole::Pairing));
        assert_eq!(role_name(ConnectionRole::Pairing), "a visitor");
        let ada = Requester {
            name: "Ada".into(),
            tag: Some(23),
            entry: 3,
            ago: Duration::from_millis(12_500),
        };
        // By their handle, so that the log tells apart two people of one name.
        assert_eq!(ada.logged(), "Ada#23 (entry 3, given 12 seconds ago)");
        assert_eq!(
            Requester { tag: None, ..ada }.logged(),
            "Ada (entry 3, given 12 seconds ago)"
        );
    }
}
