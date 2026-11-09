// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What a space's agent may do with a device pairing through Tomori's tools: list the pairing waiting without its code or the device's name, confirm it with the code a participant read aloud from the device, or decline it, always where everyone can see. See `design/security.md` and `design/mcp.md`.
use std::time::Instant;

use super::{Confirmation, Hub, HubState, PAIRING_DECISIONS, PairingEvent, REVEAL_TIMEOUT};
use serde_json::{Value, json};

/// What the tools say on a server that does not pair, as in `--auth open` mode.
pub(crate) const NO_PAIRING: &str = "This server does not pair devices: in --auth open mode everyone who connects is already an owner, so there is no pairing to list, confirm or decline.";

/// What the agent asked to do with the pairing waiting.
#[derive(Clone, Copy)]
pub(crate) enum Decision<'a> {
    /// Confirm it, if this is its code, as a participant read it aloud from the device.
    Confirm(&'a str),
    Decline,
}

/// Written without the code given, which may be the device's.
impl std::fmt::Debug for Decision<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Confirm(_) => "Confirm([redacted])",
            Self::Decline => "Decline",
        })
    }
}

impl Decision<'_> {
    /// What the agent asked, as the log and the space are told of a refusal.
    fn asked(self) -> &'static str {
        match self {
            Self::Confirm(_) => "asked to confirm a pairing",
            Self::Decline => "asked to decline a pairing",
        }
    }
}

/// What the agent did with a pairing, for the tool's result. It never names the device, whose name the device chose and could carry an attacker's instructions or code; see `design/security.md`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Decided {
    pub confirmed: bool,
}

/// What the agent is told of `reason`, a refusal Tomori has also put in its space.
fn announced(reason: &str) -> String {
    format!("{reason} Tomori has said so on the space's wall; tell everyone there too.")
}

/// Two or more `names`, as a sentence lists them: "Ada and Bob", "Ada, Bob and Cy".
fn listed(names: &[String]) -> String {
    let (last, rest) = names.split_last().expect("two or more names");
    format!("{} and {last}", rest.join(", "))
}

impl HubState {
    /// Who is in `space` now, as a notice names them: "while Ada was in the space", or "with nobody in the space".
    fn present_in(&self, space: &str) -> String {
        let names: Vec<String> = self
            .space(space)
            .into_iter()
            .flat_map(|record| &record.model.participants)
            .filter_map(|key| self.participants.get(key))
            .filter(|participant| participant.connection.is_some())
            .map(|participant| participant.name.clone())
            .collect();
        match names.len() {
            0 => "with nobody in the space".to_owned(),
            1 => format!("while {} was in the space", names[0]),
            _ => format!("while {} were in the space", listed(&names)),
        }
    }
}

impl Hub {
    /// Whether this server pairs devices: it does not in `--auth open` mode, where everyone is an owner.
    fn pairs(&self) -> bool {
        self.pairing.read().expect("never left poisoned").is_some()
    }

    /// The pairing waiting, as the `pairing` tool lists it to the agent of the space bound to `session`: only what the server knows itself, how long ago it began, how long until it is withdrawn and whether the device shows its code yet, and whether pairing is paused. Never the code, nor the values it is derived from, nor the device's name, which the device chose and could carry an attacker's code or instructions. The server knows nothing else of the device, such as its kind or platform. See `design/security.md`.
    pub(crate) fn tool_pairing(&self, session: &str) -> Result<Value, String> {
        let timeout = *self.pairing_timeout.read().expect("never left poisoned");
        let state = self.lock();
        self.calling_space(&state, session)?;
        if !self.pairs() {
            return Err(NO_PAIRING.to_owned());
        }
        let now = Instant::now();
        let pending = state.pairing.as_ref().map(|pairing| {
            let waited = now.saturating_duration_since(pairing.started);
            // A device that has not revealed its value is withdrawn sooner.
            let limit = if pairing.code.is_none() {
                REVEAL_TIMEOUT.min(timeout)
            } else {
                timeout
            };
            json!({
                "began_seconds_ago": waited.as_secs(),
                "expires_in_seconds": limit.saturating_sub(waited).as_secs(),
                "code_shown": pairing.code.is_some(),
            })
        });
        Ok(json!({
            "pending": pending,
            "paused": state.pairing_paused(now),
        }))
    }

    /// Confirm or decline the pairing waiting, as the agent of the space bound to `session` asked. A code that does not match declines the pairing, through the same check as the console's; one that is not six digits is refused and changes nothing. Every decision and refusal is logged with the space and session, shown on the console and the status page, and put on the space's Tomori wall for everyone in it. See `design/security.md`.
    pub(crate) fn agent_decide_pairing(
        &self,
        session: &str,
        decision: Decision<'_>,
    ) -> Result<Decided, String> {
        let mut state = self.lock();
        let space = match self.calling_space(&state, session) {
            Ok(space) => space,
            Err(error) => {
                // There is no space to tell, but the operator is told.
                drop(state);
                self.tell_operator(format!(
                    "Agent session {session}, bound to no open space, {} and was refused.",
                    decision.asked()
                ));
                return Err(error);
            }
        };
        let refuse = |state: std::sync::MutexGuard<'_, HubState>, reason: &str| {
            drop(state);
            self.announce_pairing(
                &space,
                session,
                &format!(
                    "{} and was refused: {}",
                    decision.asked(),
                    reason.trim_end_matches('.')
                ),
            );
            Err(announced(reason))
        };
        if !self.pairs() {
            return refuse(state, NO_PAIRING);
        }
        if state.pairing.is_none() {
            return refuse(state, "No pairing is waiting.");
        }
        let present = state.present_in(&space);
        let code = match decision {
            Decision::Decline => {
                state.decline_pairing(None);
                drop(state);
                self.announce_pairing(
                    &space,
                    session,
                    &format!("declined the pairing waiting {present}"),
                );
                return Ok(Decided { confirmed: false });
            }
            Decision::Confirm(code) => code,
        };
        // A code misheard or mistyped, not six digits, tells nothing about the device's code, so it is refused without ending the pairing.
        let Some(code) = tomori_security::pairing::typed(code) else {
            return refuse(
                state,
                "A pairing code is the six digits the device shows, such as 123 456, so nothing was confirmed.",
            );
        };
        let revealed = state
            .pairing
            .as_ref()
            .is_some_and(|pairing| pairing.code.is_some());
        let pairing = match state.take_confirmed(Confirmation::Agent(&code)) {
            Ok(pairing) => pairing,
            Err(error) if revealed => {
                drop(state);
                self.announce_pairing(
                    &space,
                    session,
                    "gave a code that did not match the one the device shows, so its pairing was declined",
                );
                return Err(announced(error));
            }
            Err(error) => return refuse(state, error),
        };
        drop(state);
        match self.issue_pairing(pairing) {
            Ok(_) => {
                self.announce_pairing(
                    &space,
                    session,
                    &format!("confirmed the pairing waiting {present}; the device is now an owner"),
                );
                Ok(Decided { confirmed: true })
            }
            Err(error) => {
                self.announce_pairing(
                    &space,
                    session,
                    &format!(
                        "confirmed the pairing waiting {present}, but {}",
                        error.trim_end_matches('.').to_lowercase()
                    ),
                );
                Err(announced(error))
            }
        }
    }

    /// Tell the operator, on the console and in the log, and owners, on the status page, `line`, about what a space's agent did with a pairing.
    fn tell_operator(&self, line: String) {
        log::info!("{line}");
        {
            let mut state = self.lock();
            if state.pairing_decisions.len() == PAIRING_DECISIONS {
                state.pairing_decisions.pop_front();
            }
            state.pairing_decisions.push_back(line.clone());
        }
        if let Some(prompt) = &*self.pairing_prompt.read().expect("never left poisoned") {
            prompt(&PairingEvent::Agent { notice: line });
        }
    }

    /// Say that the agent of `space`, called from `session`, `did` something with a pairing: to the operator and owners, naming the space and the session, and to everyone in the space, on its Tomori wall, where it stays until someone dismisses it. See `design/security.md`.
    fn announce_pairing(&self, space: &str, session: &str, did: &str) {
        let mut state = self.lock();
        let environments = self.environments();
        // The space may have been forgotten since the decision, as its token was issued; the operator is told anyway.
        let title = state
            .space_mut(space)
            .map(|record| {
                let walls = environments.walls(&record.model.environment);
                let content = crate::stage::Content {
                    title: "Pairing".to_owned(),
                    body: format!("The agent {did}."),
                    meta: vec![("source".to_owned(), "Tomori".to_owned())],
                    code: None,
                    slug: None,
                };
                let panel = record.stage.project_to(
                    &walls,
                    content,
                    &crate::stage::Destination::Tomori,
                    crate::cards::unix_now(),
                );
                log::info!("Space {space} projected {panel}.");
                record.model.title.clone()
            })
            .unwrap_or_default();
        state.touch_projection(space);
        drop(state);
        self.keep();
        self.tell_operator(format!(
            "The agent of space {space} ({title:?}), session {session}, {did}."
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_listed_as_a_sentence_lists_them() {
        let names = |names: &[&str]| -> Vec<String> {
            names.iter().map(|name| (*name).to_owned()).collect()
        };
        assert_eq!(listed(&names(&["Ada", "Bob"])), "Ada and Bob");
        assert_eq!(listed(&names(&["Ada", "Bob", "Cy"])), "Ada, Bob and Cy");
        assert_eq!(Decision::Confirm("1").asked(), "asked to confirm a pairing");
        assert_eq!(Decision::Decline.asked(), "asked to decline a pairing");
    }
}
