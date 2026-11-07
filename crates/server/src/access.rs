// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What each connection may do. A connection is admitted at the door with an [`Access`]; its role at any moment follows from that and the space it is in, and one table decides which client messages each role may send. See `design/security.md`.
use tomori_protocol::{ClientMessage, ConnectionRole};

/// What a connection was admitted as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Grant {
    /// An owner on the whole server.
    Owner,
    /// A collaborator in one space, and a visitor everywhere else.
    Collaborator { space: String },
    /// A visitor: admitted without credentials, to watch.
    Visitor,
    /// Admitted without credentials only to pair, on a server that refuses visitors.
    Pairing,
}

/// Who a connection is, as authenticated at the door, and what that admits it as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Access {
    /// The authenticated principal, if the connection presented credentials.
    pub principal: Option<String>,
    /// The display name kept with the principal, such as its token's name, which the server knows them by rather than the one their client suggests. See `design/security.md`.
    pub name: Option<String>,
    pub grant: Grant,
}

impl Access {
    /// Admitted without credentials, as everyone is in `--auth open` mode: an owner.
    pub fn open() -> Self {
        Self {
            principal: None,
            name: None,
            grant: Grant::Owner,
        }
    }

    /// Admitted without credentials, to watch.
    pub fn visitor() -> Self {
        Self {
            principal: None,
            name: None,
            grant: Grant::Visitor,
        }
    }

    /// Admitted without credentials only to pair.
    pub fn pairing() -> Self {
        Self {
            principal: None,
            name: None,
            grant: Grant::Pairing,
        }
    }

    /// The connection's role in `space`, or in the lobby.
    pub fn role(&self, space: Option<&str>) -> ConnectionRole {
        match &self.grant {
            Grant::Owner => ConnectionRole::Owner,
            Grant::Collaborator { space: admitted } if space == Some(admitted.as_str()) => {
                ConnectionRole::Collaborator
            }
            Grant::Collaborator { .. } | Grant::Visitor => ConnectionRole::Visitor,
            Grant::Pairing => ConnectionRole::Pairing,
        }
    }

    /// Admitted without credentials, as a visitor or only to pair: counted against the visitor limit, and not as a participant.
    pub fn is_visitor(&self) -> bool {
        matches!(self.grant, Grant::Visitor | Grant::Pairing)
    }
}

/// Why `role` may not send `message`, or `None` if it may. Every message is listed, so that a new one needs a decision.
pub fn refusal(role: ConnectionRole, message: &ClientMessage) -> Option<&'static str> {
    let owner = role == ConnectionRole::Owner;
    let participant = role.participates();
    let watcher = role != ConnectionRole::Pairing;
    let permitted = match message {
        ClientMessage::Hello { .. }
        | ClientMessage::Ping { .. }
        | ClientMessage::Pong { .. }
        | ClientMessage::PairBegin { .. }
        | ClientMessage::PairReveal { .. }
        | ClientMessage::PairCancel => true,
        // Ignored from a visitor, whose client may report it before it knows its role. See `design/spaces.md`.
        ClientMessage::Engagement { .. } => watcher,
        // Everyone renames only themselves: a token holder's name is kept with their token, and a visitor's is theirs while they are kept. See `design/security.md`.
        ClientMessage::ListSpaces
        | ClientMessage::Join { .. }
        | ClientMessage::Leave
        | ClientMessage::Knock
        | ClientMessage::SetName { .. } => watcher,
        ClientMessage::CreateSpace { .. }
        | ClientMessage::ListSessions { .. }
        | ClientMessage::SetInstructions { .. }
        | ClientMessage::SetEnvironment { .. } => owner,
        ClientMessage::Gate { .. }
        | ClientMessage::UtteranceBegin { .. }
        | ClientMessage::UtteranceEnd { .. }
        | ClientMessage::UtteranceCancel { .. }
        | ClientMessage::CancelQueued { .. }
        | ClientMessage::Decide { .. }
        | ClientMessage::Answer { .. }
        | ClientMessage::Presence { .. }
        | ClientMessage::SetAttribution { .. }
        | ClientMessage::SetMode { .. }
        | ClientMessage::SetSpeech { .. }
        | ClientMessage::Replay { .. }
        | ClientMessage::StopReplay
        | ClientMessage::DecideKnock { .. }
        // Visitors see no cards in this prototype, as a repository may be private. See `design/github.md`.
        | ClientMessage::WatchCards
        | ClientMessage::PlaceCard { .. }
        // Visitors watch an environment, but ask nothing of it. See `design/environments.md`.
        | ClientMessage::EnvironmentIntent { .. } => participant,
    };
    if permitted {
        None
    } else if participant {
        Some("Only owners can do that.")
    } else if watcher {
        Some("Visitors can watch, but not act in a space.")
    } else {
        Some("This server admits only paired devices: pair to join.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomori_protocol::{Decision, Gate, Pose};

    fn every_message() -> Vec<ClientMessage> {
        vec![
            ClientMessage::Hello {
                protocol: 2,
                participant: "p".into(),
                name: "n".into(),
                names: false,
                environments: None,
                environment_state: false,
            },
            ClientMessage::Ping { nonce: 1 },
            ClientMessage::Pong { nonce: 1 },
            ClientMessage::ListSpaces,
            ClientMessage::Join { space: "s".into() },
            ClientMessage::Leave,
            ClientMessage::CreateSpace {
                title: "t".into(),
                backend: "fake".into(),
                directory: "/tmp".into(),
                session: None,
                environment: None,
                instructions: None,
            },
            ClientMessage::ListSessions {
                backend: "fake".into(),
                directory: "/tmp".into(),
            },
            ClientMessage::Gate { gate: Gate::Live },
            ClientMessage::UtteranceBegin {
                utterance: "u".into(),
                answer: None,
                progress: false,
                naming: false,
                attention: Vec::new(),
            },
            ClientMessage::UtteranceEnd {
                utterance: "u".into(),
                pointed: None,
            },
            ClientMessage::UtteranceCancel {
                utterance: "u".into(),
            },
            ClientMessage::CancelQueued { entry: "e".into() },
            ClientMessage::Decide {
                request: "r".into(),
                decision: Decision::AllowOnce,
                option: None,
            },
            ClientMessage::Answer {
                question: "q".into(),
                field: "f".into(),
                text: "t".into(),
            },
            ClientMessage::Presence {
                head: Pose::default(),
                left: None,
                right: None,
                hands: None,
            },
            ClientMessage::SetAttribution { enabled: true },
            ClientMessage::SetMode {
                mode: "plan".into(),
            },
            ClientMessage::Replay { message: None },
            ClientMessage::StopReplay,
            ClientMessage::Engagement {
                state: tomori_protocol::Engagement::Away,
            },
            ClientMessage::SetSpeech {
                speech: tomori_protocol::SpeechSetting::Full,
            },
            ClientMessage::SetInstructions { text: "t".into() },
            ClientMessage::SetEnvironment {
                environment: "sky".into(),
            },
            ClientMessage::PairBegin {
                commitment: "c".into(),
            },
            ClientMessage::PairReveal { nonce: "n".into() },
            ClientMessage::PairCancel,
            ClientMessage::Knock,
            ClientMessage::SetName { name: "n".into() },
            ClientMessage::DecideKnock {
                knock: "k".into(),
                admit: true,
            },
            ClientMessage::EnvironmentIntent {
                environment: "garden".into(),
                version: 1,
                intent: tomori_protocol::Payload::new(serde_json::json!({})),
            },
        ]
    }

    fn permitted(role: ConnectionRole) -> Vec<String> {
        every_message()
            .iter()
            .filter(|message| refusal(role, message).is_none())
            .map(|message| {
                let json = serde_json::to_value(message).unwrap();
                json["type"].as_str().unwrap().to_owned()
            })
            .collect()
    }

    #[test]
    fn each_role_sends_only_what_it_may() {
        let watching = [
            "hello",
            "ping",
            "pong",
            "list_spaces",
            "join",
            "leave",
            "engagement",
            "pair_begin",
            "pair_reveal",
            "pair_cancel",
            "knock",
            "set_name",
        ];
        assert_eq!(permitted(ConnectionRole::Visitor), watching);
        assert_eq!(permitted(ConnectionRole::Unknown), watching);
        assert_eq!(
            permitted(ConnectionRole::Pairing),
            [
                "hello",
                "ping",
                "pong",
                "pair_begin",
                "pair_reveal",
                "pair_cancel"
            ]
        );
        assert_eq!(
            refusal(ConnectionRole::Pairing, &ClientMessage::ListSpaces),
            Some("This server admits only paired devices: pair to join.")
        );
        let collaborating: Vec<String> = every_message()
            .iter()
            .map(|message| {
                serde_json::to_value(message).unwrap()["type"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .filter(|kind| {
                kind != "create_space"
                    && kind != "list_sessions"
                    && kind != "set_instructions"
                    && kind != "set_environment"
            })
            .collect();
        assert_eq!(permitted(ConnectionRole::Collaborator), collaborating);
        assert_eq!(
            permitted(ConnectionRole::Owner).len(),
            every_message().len()
        );

        let create = &every_message()[6];
        assert_eq!(
            refusal(ConnectionRole::Collaborator, create),
            Some("Only owners can do that.")
        );
        assert_eq!(
            refusal(ConnectionRole::Visitor, create),
            Some("Visitors can watch, but not act in a space.")
        );
    }

    #[test]
    fn a_collaborator_is_one_only_in_their_space() {
        let collaborator = Access {
            principal: Some("token:1".into()),
            name: None,
            grant: Grant::Collaborator {
                space: "studio".into(),
            },
        };
        assert_eq!(
            collaborator.role(Some("studio")),
            ConnectionRole::Collaborator
        );
        assert_eq!(collaborator.role(Some("other")), ConnectionRole::Visitor);
        assert_eq!(collaborator.role(None), ConnectionRole::Visitor);
        assert!(!collaborator.is_visitor());
        assert_eq!(Access::open().role(None), ConnectionRole::Owner);
        assert_eq!(
            Access::visitor().role(Some("studio")),
            ConnectionRole::Visitor
        );
        assert!(Access::visitor().is_visitor());
        assert_eq!(Access::pairing().role(None), ConnectionRole::Pairing);
        assert!(Access::pairing().is_visitor());
    }
}
