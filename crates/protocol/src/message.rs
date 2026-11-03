// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use serde::{Deserialize, Serialize};

use crate::{
    BackendInfo, ConnectionRole, EnvironmentInfo, Gate, KnockRequest, KnockState, SessionSummary,
    SpaceState, SpaceSummary, WorkspaceRoot,
};

/// A tracked pose in the space's coordinate system.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    pub position: [f32; 3],
    /// A unit quaternion `[x, y, z, w]`.
    pub orientation: [f32; 4],
}

/// A participant's decision on a pending tool permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    AllowOnce,
    /// Allow, and grant the approval's [`crate::ApprovalOption`] named by the decision's `option`, with the same effect
    /// as choosing it in the agent's own interface.
    Grant,
    Deny,
}

/// Control messages sent by a client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// The first message on every connection.
    Hello {
        protocol: u32,
        /// A stable identity chosen by the client, so a reconnect is recognized as the same participant. A secret of the device: the server never sends it to other clients. See `design/security.md`.
        participant: String,
        /// The display name used for speaker attribution.
        name: String,
    },
    ListSpaces,
    /// The existing sessions of `backend` in `directory`, for attaching. A backend without attaching answers with an error.
    ListSessions {
        backend: String,
        directory: String,
    },
    /// Create a space bound to a new session, or to an existing `session` where the backend supports binding.
    CreateSpace {
        title: String,
        backend: String,
        directory: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<String>,
        /// The identifier of one of the environments the server offers in `Welcome`, which the space is bound to for its lifetime. Absent from older clients, which did not choose one: the server then binds the first environment it offers.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        environment: Option<String>,
    },
    /// Join a space, leaving any other space.
    Join {
        space: String,
    },
    Leave,
    /// Change where this client's microphone audio goes. Applies to subsequent audio frames.
    Gate {
        gate: Gate,
    },
    /// Mark the start of push-to-talk audio for the agent. Audio frames with [`Gate::Agent`] belong to the open utterance.
    UtteranceBegin {
        utterance: String,
    },
    /// Finalize the utterance; the server finishes recognition and queues the transcript.
    UtteranceEnd {
        utterance: String,
    },
    UtteranceCancel {
        utterance: String,
    },
    /// Cancel a queued entry before it is sent to the agent. Any participant may cancel any entry.
    CancelQueued {
        entry: String,
    },
    Decide {
        request: String,
        decision: Decision,
        /// The id of the approval option to grant. Required with [`Decision::Grant`] and refused with any other decision.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        option: Option<String>,
    },
    /// Answer the current field of a pending question with spoken or typed text.
    Answer {
        question: String,
        field: String,
        text: String,
    },
    Presence {
        head: Pose,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        left: Option<Pose>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        right: Option<Pose>,
        /// Tracked hand skeletons, left then right. Absent from older clients.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hands: Option<Box<[Option<crate::presence::Hand>; 2]>>,
    },
    /// Turn speaker attribution on or off for the current space.
    SetAttribution {
        enabled: bool,
    },
    /// Switch the current space's session to one of the modes it offers. Any participant may switch it.
    SetMode {
        mode: String,
    },
    /// Ask to pair, committing to a random value: the SHA-256 of the value [`ClientMessage::PairReveal`] sends, as 64 hexadecimal digits. The server answers with [`ServerMessage::PairNonce`]. See `design/security.md`.
    PairBegin {
        commitment: String,
    },
    /// Reveal the value committed to, as 64 hexadecimal digits, once the server has sent its own. Both sides then derive the pairing code.
    PairReveal {
        nonce: String,
    },
    /// Stop pairing.
    PairCancel,
    /// Ask to collaborate in the space this visitor is watching. See `design/security.md`.
    Knock,
    /// Admit or decline a knock on the space this participant is in.
    DecideKnock {
        knock: String,
        admit: bool,
    },
    /// Play a retained reply again to this participant only, from its start, from [`crate::REPLAY_SOURCE`]: `None`
    /// for the latest reply. Live agent speech of a later reply stops it. Sent only to a server whose
    /// [`crate::SpaceState::speech`] is known. See `design/speech.md`.
    Replay {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Stop this participant's replay or catch-up, and follow live speech from here on.
    StopReplay,
    /// Whether this participant is wearing their device and attending, sent when it changes and once on entering a
    /// space. Sent only to a server whose [`crate::SpaceState::speech`] is known. See `design/spaces.md`.
    Engagement {
        state: crate::Engagement,
    },
    /// Change how much of each reply the current space speaks, for everyone in it. Sent only to a server whose
    /// [`crate::SpaceState::speech`] is known.
    SetSpeech {
        speech: crate::SpeechSetting,
    },
    /// Sent when nothing has been received for the keepalive interval. Valid before Hello. See [`crate::Keepalive`].
    Ping {
        nonce: u64,
    },
    /// The immediate answer to a [`ServerMessage::Ping`], echoing its nonce.
    Pong {
        nonce: u64,
    },
}

/// Control messages sent by the server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Welcome {
        protocol: u32,
        server: String,
        /// The participant number used as [`crate::AudioFrame::source`] for this client's forwarded audio.
        source: u32,
        /// What this connection may do in the lobby; [`ServerMessage::Role`] follows whenever it changes. Unknown if absent.
        #[serde(default)]
        role: ConnectionRole,
        /// The server's backends, so a client shows only actions each supports. Empty unless the connection is an owner, since only owners create spaces.
        backends: Vec<BackendInfo>,
        /// The directories a space may run in, so a client can choose one without typing a path. Empty unless the connection is an owner.
        roots: Vec<WorkspaceRoot>,
        /// The environments a space may be bound to, the first being the one bound when a `CreateSpace` names none. Absent from older servers.
        #[serde(default)]
        environments: Vec<EnvironmentInfo>,
    },
    Spaces {
        spaces: Vec<SpaceSummary>,
    },
    /// What this connection may now do, sent whenever its role changes, such as on joining or leaving a space. See `design/security.md`.
    Role {
        role: ConnectionRole,
    },
    /// The answer to [`ClientMessage::ListSessions`].
    Sessions {
        sessions: Vec<SessionSummary>,
    },
    /// The complete state of the joined space. Sent on join and whenever it changes.
    Space {
        state: Box<SpaceState>,
    },
    Left,
    Transcript {
        utterance: String,
        text: String,
        /// True once the utterance is finalized and queued.
        #[serde(rename = "final")]
        finished: bool,
    },
    /// Recognition could not produce a transcript; nothing was queued.
    TranscriptFailed {
        utterance: String,
        error: String,
    },
    /// Announces the agent speech audio frames that follow, which use [`crate::AGENT_SOURCE`], or
    /// [`crate::REPLAY_SOURCE`] when `replay` is set.
    Speech {
        segment: u32,
        message: String,
        text: String,
        sample_rate: u32,
        /// Set when the segment is played again for this participant only, as a replay or catch-up, rather than
        /// live. Absent from older servers.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        replay: bool,
    },
    Presence {
        source: u32,
        head: Pose,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        left: Option<Pose>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        right: Option<Pose>,
        /// The participant's tracked hand skeletons, left then right. Absent from older servers.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hands: Option<Box<[Option<crate::presence::Hand>; 2]>>,
    },
    /// A request could not be completed. The connection stays open.
    Error {
        error: String,
        /// The space a refused `Join` named, so that a client can tell its join was refused rather than some other request. Absent for other errors.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        space: Option<String>,
    },
    /// The server's random value for a pairing, as 64 hexadecimal digits, sent once the device has committed to its own.
    PairNonce {
        nonce: String,
    },
    /// Both sides have derived the pairing code, which now waits for the operator or an owner to confirm it.
    PairWaiting,
    /// The pairing was confirmed: an owner token for this server, to present from now on, over TLS to the certificate this connection presented.
    Paired {
        token: String,
    },
    /// The pairing ended without a token: declined, withdrawn, or refused.
    PairFailed {
        error: String,
    },
    /// The knocks waiting on the space, sent to its participants whenever they change.
    Knocks {
        knocks: Vec<KnockRequest>,
    },
    /// What became of this visitor's knock.
    KnockState {
        state: KnockState,
    },
    /// This visitor was admitted to the space as a collaborator: a token to present from now on, over TLS to the certificate this connection presented. The connection is a collaborator's at once, and a [`ServerMessage::Role`] follows.
    Admitted {
        token: String,
    },
    /// Sent when nothing has been received for the keepalive interval. See [`crate::Keepalive`].
    Ping {
        nonce: u64,
    },
    /// The immediate answer to a [`ClientMessage::Ping`], echoing its nonce.
    Pong {
        nonce: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_use_a_snake_case_type_tag() {
        let json = serde_json::to_value(ClientMessage::Decide {
            request: "per_1".into(),
            decision: Decision::AllowOnce,
            option: None,
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"type": "decide", "request": "per_1", "decision": "allow_once"})
        );
        let grant = serde_json::json!({"type": "decide", "request": "per_1", "decision": "grant", "option": "suggestions"});
        assert_eq!(
            serde_json::from_value::<ClientMessage>(grant).unwrap(),
            ClientMessage::Decide {
                request: "per_1".into(),
                decision: Decision::Grant,
                option: Some("suggestions".into()),
            }
        );

        let json = serde_json::to_value(ServerMessage::Transcript {
            utterance: "u".into(),
            text: "hello".into(),
            finished: true,
        })
        .unwrap();
        assert_eq!(json["final"], true);
    }

    #[test]
    fn replay_engagement_and_speech_settings_have_a_stable_wire_format() {
        assert_eq!(
            serde_json::to_value(ClientMessage::Replay { message: None }).unwrap(),
            serde_json::json!({"type": "replay"})
        );
        assert_eq!(
            serde_json::to_value(ClientMessage::StopReplay).unwrap(),
            serde_json::json!({"type": "stop_replay"})
        );
        assert_eq!(
            serde_json::to_value(ClientMessage::Engagement {
                state: crate::Engagement::Away
            })
            .unwrap(),
            serde_json::json!({"type": "engagement", "state": "away"})
        );
        assert_eq!(
            serde_json::to_value(ClientMessage::SetSpeech {
                speech: crate::SpeechSetting::Brief
            })
            .unwrap(),
            serde_json::json!({"type": "set_speech", "speech": "brief"})
        );
        // A state a newer client sends is read as unknown, which still counts as listening.
        let newer: ClientMessage =
            serde_json::from_value(serde_json::json!({"type": "engagement", "state": "dozing"}))
                .unwrap();
        assert_eq!(
            newer,
            ClientMessage::Engagement {
                state: crate::Engagement::Unknown
            }
        );
        assert!(crate::Engagement::Unknown.is_listening());
        assert!(crate::Engagement::Distracted.is_listening());
        assert!(!crate::Engagement::Away.is_listening());

        // Live speech omits the replay flag, as older servers did; a replayed segment sets it.
        let live = ServerMessage::Speech {
            segment: 1,
            message: "m".into(),
            text: "Hello.".into(),
            sample_rate: 24_000,
            replay: false,
        };
        let json = serde_json::to_value(&live).unwrap();
        assert!(json.get("replay").is_none());
        assert_eq!(serde_json::from_value::<ServerMessage>(json).unwrap(), live);
        let replayed = ServerMessage::Speech {
            segment: 1,
            message: "m".into(),
            text: "Hello.".into(),
            sample_rate: 24_000,
            replay: true,
        };
        assert_eq!(serde_json::to_value(&replayed).unwrap()["replay"], true);
    }

    #[test]
    fn keepalive_messages_are_symmetric() {
        let ping = serde_json::json!({"type": "ping", "nonce": 7});
        let pong = serde_json::json!({"type": "pong", "nonce": 7});
        assert_eq!(
            serde_json::to_value(ClientMessage::Ping { nonce: 7 }).unwrap(),
            ping
        );
        assert_eq!(
            serde_json::to_value(ServerMessage::Ping { nonce: 7 }).unwrap(),
            ping
        );
        assert_eq!(
            serde_json::to_value(ClientMessage::Pong { nonce: 7 }).unwrap(),
            pong
        );
        assert_eq!(
            serde_json::to_value(ServerMessage::Pong { nonce: 7 }).unwrap(),
            pong
        );
        assert_eq!(
            serde_json::from_value::<ClientMessage>(ping.clone()).unwrap(),
            ClientMessage::Ping { nonce: 7 }
        );
        assert_eq!(
            serde_json::from_value::<ServerMessage>(pong).unwrap(),
            ServerMessage::Pong { nonce: 7 }
        );
        // The largest nonce survives a round trip through the frame encoding.
        let frame = crate::Frame::server(&ServerMessage::Ping { nonce: u64::MAX });
        assert_eq!(
            frame.to_server_message().unwrap(),
            ServerMessage::Ping { nonce: u64::MAX }
        );
    }

    #[test]
    fn environments_are_optional_for_older_peers() {
        // An older client chooses no environment, and the server binds its first.
        let create: ClientMessage = serde_json::from_value(serde_json::json!({
            "type": "create_space", "title": "Tomori", "backend": "fake", "directory": "/tmp",
        }))
        .unwrap();
        assert!(matches!(
            create,
            ClientMessage::CreateSpace {
                environment: None,
                ..
            }
        ));
        let json = serde_json::to_value(ClientMessage::CreateSpace {
            title: "Tomori".into(),
            backend: "fake".into(),
            directory: "/tmp".into(),
            session: None,
            environment: Some("sky".into()),
        })
        .unwrap();
        assert_eq!(json["environment"], "sky");

        // An older server offers none.
        let welcome: ServerMessage = serde_json::from_value(serde_json::json!({
            "type": "welcome", "protocol": 1, "server": "tomori", "source": 1, "backends": [], "roots": [],
        }))
        .unwrap();
        assert!(matches!(
            welcome,
            ServerMessage::Welcome { environments, .. } if environments.is_empty()
        ));
    }

    #[test]
    fn presence_hands_keep_their_wire_format() {
        use crate::presence::{Hand, JOINT_COUNT, Joint};

        let mut joints = [None; JOINT_COUNT];
        joints[1] = Some(Joint {
            position: [0.5, 1.0, -0.25],
            radius: 0.0625,
        });
        let hands = [Some(Hand { joints }), None];
        let head = Pose {
            position: [0.0, 1.5, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };

        // The boxed hands serialize exactly as the unboxed array did: a plain two-element array of hands.
        let mut joints_json = vec![serde_json::Value::Null; JOINT_COUNT];
        joints_json[1] = serde_json::json!({"position": [0.5, 1.0, -0.25], "radius": 0.0625});
        let hands_json = serde_json::json!([{"joints": joints_json}, null]);
        let head_json =
            serde_json::json!({"position": [0.0, 1.5, 0.0], "orientation": [0.0, 0.0, 0.0, 1.0]});

        let client = ClientMessage::Presence {
            head,
            left: None,
            right: None,
            hands: Some(Box::new(hands)),
        };
        let client_json =
            serde_json::json!({"type": "presence", "head": head_json, "hands": hands_json});
        assert_eq!(serde_json::to_value(&client).unwrap(), client_json);
        assert_eq!(
            serde_json::from_value::<ClientMessage>(client_json).unwrap(),
            client
        );

        let server = ServerMessage::Presence {
            source: 3,
            head,
            left: None,
            right: None,
            hands: Some(Box::new(hands)),
        };
        let server_json = serde_json::json!({"type": "presence", "source": 3, "head": head_json, "hands": hands_json});
        assert_eq!(serde_json::to_value(&server).unwrap(), server_json);
        assert_eq!(
            serde_json::from_value::<ServerMessage>(server_json).unwrap(),
            server
        );

        // Presence without hands, as from older clients, omits the field.
        let bare = ClientMessage::Presence {
            head,
            left: None,
            right: None,
            hands: None,
        };
        assert_eq!(
            serde_json::to_value(&bare).unwrap(),
            serde_json::json!({"type": "presence", "head": head_json})
        );
    }

    #[test]
    fn hello_carries_no_credentials() {
        // Credentials belong on the HTTP request that opens the connection; see `design/security.md`.
        let hello = ClientMessage::Hello {
            protocol: 2,
            participant: "p".into(),
            name: "Ada".into(),
        };
        assert_eq!(
            serde_json::to_value(&hello).unwrap(),
            serde_json::json!({"type": "hello", "protocol": 2, "participant": "p", "name": "Ada"})
        );
    }
}
