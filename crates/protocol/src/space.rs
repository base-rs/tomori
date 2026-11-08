// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use serde::{Deserialize, Serialize};

/// A space in the lobby listing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpaceSummary {
    pub id: String,
    pub title: String,
    pub backend: String,
    pub directory: String,
    /// The identifier of the space's [`EnvironmentInfo`]. Empty from older servers, which did not name one.
    #[serde(default)]
    pub environment: String,
    pub activity: Activity,
    pub participants: usize,
    /// What about the space needs a participant, if anything; see [`Attention`]. Absent when nothing does, and from
    /// older servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention: Option<Attention>,
    /// How long, in whole seconds, the space has needed [`Self::attention`] at its current level, so the longest
    /// waiting comes first. Absent without attention, and from older servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention_age: Option<u32>,
}

/// What about a space needs a participant, worked out by the server for anyone, not for one participant. When several
/// apply, the first listed wins. Running, idle, queued and closed spaces need nothing by themselves. See
/// `design/spaces.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attention {
    /// An approval or question is waiting for an answer, or its answer is being delivered.
    Input,
    /// While nobody was in the space, input failed, the agent reported an error or the session closed. Cleared when
    /// someone joins.
    Problem,
    /// While nobody was in the space, the agent finished a turn that completed a reply. Cleared when someone joins.
    Reply,
    /// A level a newer server sends that this build does not know.
    #[serde(other)]
    Unknown,
}

/// An existing agent session a space can be bound to, reported by a backend that supports attaching.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    /// The session's own directory: the one listed, or one beneath it. A space bound to the session runs in it.
    pub directory: String,
    /// The agent is running a turn in the session. Absent from older servers, which listed no busy sessions apart.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub busy: bool,
}

/// A backend the server can bind spaces to, as offered to a client so it can show only working actions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendInfo {
    pub name: String,
    /// Existing sessions can be listed and attached through [`crate::ClientMessage::CreateSpace`]'s `session`.
    pub attach: bool,
}

/// A directory a space may run in, as offered to a client. A client cannot type a path, so it chooses from what the server sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRoot {
    /// The canonical path, ready to send in a `CreateSpace`.
    pub path: String,
    /// A short name for display, such as the path's final component.
    pub label: String,
}

/// An environment a space can be bound to, as offered to a client so it can choose one when creating a space. See `design/environments.md`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentInfo {
    /// An opaque identifier, such as `sky`, ready to send in a `CreateSpace`. A client that does not know it can still name it.
    pub id: String,
    /// A short name for display.
    pub label: String,
    /// The environment's protocol version, which a client must present it at. 1 from older servers, which had no
    /// versions.
    #[serde(default = "first_version")]
    pub version: u32,
}

/// An environment a client can present, at the protocol version it presents it at, as it lists it in
/// [`crate::ClientMessage::Hello`], so that the server offers it only the environments it lists at the same version.
/// See `design/environments.md`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EnvironmentVersion {
    /// The environment's identifier, as in [`EnvironmentInfo::id`].
    pub id: String,
    /// The version presented, read as 1 when absent.
    #[serde(default = "first_version")]
    pub version: u32,
}

impl EnvironmentVersion {
    /// The environment `id` at `version`.
    pub fn new(id: impl Into<String>, version: u32) -> Self {
        Self {
            id: id.into(),
            version,
        }
    }
}

/// An environment's version where a peer names none, as one from before versions: every environment then was at
/// version 1.
fn first_version() -> u32 {
    1
}

/// The agent's observed state. `Unknown` is never treated as idle or complete, and stands for any state a newer server sends that this build does not know.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    Idle,
    Running,
    /// A permission or question needs a participant's input.
    Waiting,
    /// The agent session has ended. The space keeps its state, but input is no longer accepted; `SpaceState::error` says why.
    Closed,
    #[default]
    #[serde(other)]
    Unknown,
}

/// What a connection may do where it is now: an owner anywhere, a collaborator in the space they were admitted to, and a visitor otherwise. `Unknown` stands for a role a newer server sends that this build does not know, and permits nothing beyond a visitor's actions. See `design/security.md`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionRole {
    /// Everything a collaborator can do, and creating spaces and listing an agent's sessions.
    Owner,
    /// Acting in the space: talking to the agent, deciding approvals, answering questions and the like.
    Collaborator,
    /// Watching: listing spaces, joining one to watch and hear it, leaving, pairing and knocking.
    Visitor,
    /// Only pairing, on a server that refuses visitors.
    Pairing,
    #[default]
    #[serde(other)]
    Unknown,
}

impl ConnectionRole {
    /// Whether this role makes its holder a participant in the space they are in, able to act on it.
    pub fn participates(self) -> bool {
        matches!(self, Self::Owner | Self::Collaborator)
    }
}

/// The complete shared state of a space, identical for every participant. See `design/spaces.md`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SpaceState {
    pub id: String,
    pub title: String,
    pub backend: String,
    pub directory: String,
    /// The identifier of the environment the space is bound to, the same for every participant. A client that does not know it shows the Minimal grid and names it. Empty from older servers, which did not name one.
    #[serde(default)]
    pub environment: String,
    /// The protocol version of [`Self::environment`], so that a client that presents it at another version, or not at
    /// all, shows the Minimal grid and names the version it needs. Absent from older servers, and when the server no
    /// longer offers the environment. See `design/environments.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_version: Option<u32>,
    pub activity: Activity,
    /// Recent conversation, oldest first, projected to displayable text.
    pub messages: Vec<Message>,
    pub queue: Vec<QueueEntry>,
    pub approval: Option<Approval>,
    pub question: Option<Question>,
    pub workers: Vec<Worker>,
    /// Whether absence from this list is evidence that a previously observed
    /// worker was removed. It does not assert that every worker was discovered.
    /// Absent when sent by an older server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workers_authoritative: Option<bool>,
    pub presentations: Vec<Presentation>,
    pub participants: Vec<Participant>,
    /// How many visitors are watching the space. See `design/security.md`.
    #[serde(default)]
    pub visitors: usize,
    pub attribution: bool,
    /// A space-level problem, such as a lost backend connection. Shared state is retained.
    pub error: Option<String>,
    /// The id of the session's current mode, as the agent last reported it. Absent from older servers.
    #[serde(default)]
    pub mode: Option<String>,
    /// The modes the session offers, in the order the agent's own interface steps through them. Absent from older
    /// servers.
    #[serde(default)]
    pub modes: Vec<Mode>,
    /// How much of each reply is spoken, shared by everyone in the space. `Unknown` from older servers, which speak
    /// every reply in full.
    ///
    /// A server that sends a known setting also retains speech, and accepts [`crate::ClientMessage::Replay`],
    /// [`crate::ClientMessage::StopReplay`], [`crate::ClientMessage::Engagement`] and
    /// [`crate::ClientMessage::SetSpeech`]; a client sends those only to such a server. See `design/speech.md`.
    #[serde(default)]
    pub speech: SpeechSetting,
    /// The instructions Tomori gives the agent in this space, in layers, so that everyone in it can see them. Absent
    /// when the space's backend takes no instructions, and from older servers. See `design/spaces.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<AgentInstructions>,
    /// Whether the server applies an utterance marked with [`crate::AnswerTarget`] as the answer to the presented question, rather than queueing it. False from older servers, which would queue it; a client marks an utterance as an answer only when this is set. See `design/spaces.md`.
    #[serde(default)]
    pub voice_answers: bool,
    /// Whether the server accepts [`crate::ClientMessage::SetEnvironment`] from an owner, changing the space's
    /// environment for everyone in it. False from older servers, which bind a space's environment for its lifetime; a
    /// client offers the change only when this is set. See `design/environments.md`.
    #[serde(default)]
    pub environment_changes: bool,
    /// Whether the space's environment shows cards, the GitHub environment's issues and pull requests, which the server
    /// sends a participant who asks with [`crate::ClientMessage::WatchCards`], and lets them place with
    /// [`crate::ClientMessage::PlaceCard`]. False from older servers and in other environments. See `design/github.md`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cards: bool,
    /// Whether the server projects panels in the space, sending [`crate::ServerMessage::Projection`] to a client whose
    /// Hello set `projection`, and taking [`crate::ClientMessage::AddWall`], [`crate::ClientMessage::RemoveWall`],
    /// [`crate::ClientMessage::PlacePanel`] and [`crate::ClientMessage::DismissPanel`]. False from older servers, which
    /// a client sends none of them. See `design/projection.md`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub projection: bool,
    /// Whether the server accepts [`crate::ClientMessage::DeleteSpace`] from an owner in the space. False from older
    /// servers; a client offers deleting the space only when this is set. See `design/spaces.md`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deletes: bool,
    /// Whether deleting the space can delete its agent session too, as its backend can. A client offers it only when
    /// this is set, as well as [`Self::deletes`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deletes_session: bool,
}

/// The instructions Tomori gives a space's agent, in layers, each its own instruction entry of the agent's session and
/// read by the agent in this order. An empty layer is not given. See `design/spaces.md`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInstructions {
    /// Tomori's own, the same in every space: how to converse in a shared, spoken space.
    #[serde(default)]
    pub tomori: String,
    /// The space's environment's, about what it presents and how to use it.
    #[serde(default)]
    pub environment: String,
    /// The space's own, given by an owner when creating it and changed by an owner with
    /// [`crate::ClientMessage::SetInstructions`].
    #[serde(default)]
    pub space: String,
    /// Why the agent may not have the layers shown, such as the agent refusing them. The space works without them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// How much of each reply a space speaks. See `design/speech.md`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechSetting {
    /// Every reply is spoken in full.
    Full,
    /// Each reply's first blocks are spoken, up to about 20 seconds, followed by "More on the panel."
    Brief,
    /// A setting a newer server sends that this build does not know, or none from an older server.
    #[default]
    #[serde(other)]
    Unknown,
}

/// Whether a participant is wearing their device and attending to Tomori, as their client reports it. See
/// `design/spaces.md`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engagement {
    /// Worn, and Tomori has input.
    Engaged,
    /// Worn, but something else, such as a system menu, has input for now. Still listening.
    Distracted,
    /// Removed, asleep or in the background: not listening.
    Away,
    /// A state a newer client or server sends that this build does not know.
    #[default]
    #[serde(other)]
    Unknown,
}

impl Engagement {
    /// Whether a participant in this state hears speech as it plays. Only `Away` does not.
    pub fn is_listening(self) -> bool {
        self != Self::Away
    }
}

impl SpaceState {
    /// The mode after the current one in the agent's order, wrapping around, as the agent's terminal key steps. A
    /// current mode that is not offered steps to the first. `None` when the session offers no modes.
    pub fn next_mode(&self) -> Option<&Mode> {
        let index = self
            .modes
            .iter()
            .position(|mode| Some(&mode.id) == self.mode.as_ref())
            .map_or(0, |index| (index + 1) % self.modes.len());
        self.modes.get(index)
    }

    /// The current mode, when it is one the session offers.
    pub fn current_mode(&self) -> Option<&Mode> {
        self.modes
            .iter()
            .find(|mode| Some(&mode.id) == self.mode.as_ref())
    }
}

/// A mode a session can be switched to, such as an OpenCode primary agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mode {
    /// The agent's identifier, sent back in [`crate::ClientMessage::SetMode`].
    pub id: String,
    /// The name the agent's interface shows.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub role: Role,
    pub text: String,
    /// False while an assistant message is still streaming.
    pub completed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueState {
    /// Waiting for the agent's turn to end. Can be cancelled.
    Queued,
    /// Being delivered. Can no longer be cancelled.
    Sending,
    Delivered,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueueEntry {
    pub id: String,
    /// The speaker's participant number, their public handle, so a client can recognize its own entries by the `source` in its `Welcome`. A participant's identity is never sent to other clients; see `design/security.md`.
    pub source: u32,
    /// The speaker's display name.
    pub speaker: String,
    pub text: String,
    pub state: QueueState,
}

/// A pending tool permission, with prompt text prepared by the server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Approval {
    pub id: String,
    pub title: String,
    pub text: String,
    /// Choices beside allowing once and denying that the agent's own interface offers, such as "don't ask again".
    /// Chosen with [`crate::Decision::Grant`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ApprovalOption>,
}

/// A standing permission granted together with allowing the request, with its label prepared by the server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalOption {
    /// Passed back as the option of a [`crate::Decision::Grant`] decision.
    pub id: String,
    /// What choosing it allows and for how long, such as "Yes, and don't ask again for Bash(make) in this project".
    pub label: String,
}

/// The current field of a pending question, with prompt text prepared by the server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    pub title: String,
    pub text: String,
    /// The field the next answer applies to.
    pub field: String,
    /// False when the question must be completed outside Tomori.
    pub supported: bool,
}

/// A worker's observed state. `Unknown` stands for any state a newer server sends that this build does not know.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerState {
    Running,
    Waiting,
    Idle,
    Completed,
    Error,
    /// Ended before it finished without failing, such as when its agent process ended. It may be resumed.
    Stopped,
    #[default]
    #[serde(other)]
    Unknown,
}

/// A delegated worker session, displayed as a station.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Worker {
    pub id: String,
    /// The worker that delegated to this one, or `None` for a worker the space's own agent session started directly.
    pub parent: Option<String>,
    pub title: String,
    pub state: WorkerState,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationKind {
    #[default]
    Markdown,
    Table,
}

/// A result card published by the coordinator.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Presentation {
    pub id: String,
    pub source_session: String,
    pub source_message: String,
    pub title: String,
    pub kind: PresentationKind,
    pub text: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// A visitor's request to collaborate in a space, as its participants see it. See `design/security.md`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnockRequest {
    /// Names the knock when admitting or declining it.
    pub id: String,
    /// The visitor's display name.
    pub name: String,
}

/// What became of a visitor's knock. `Unknown` stands for a state a newer server sends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnockState {
    /// Waiting for a participant to admit or decline it.
    Pending,
    Declined,
    /// Nobody answered in time.
    Expired,
    /// The visitor left the space, so the knock was withdrawn.
    Withdrawn,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Participant {
    pub name: String,
    /// The participant's number: their public handle, and the source of their audio frames. A participant's identity is never sent to other clients; see `design/security.md`.
    pub source: u32,
    /// What the participant's client last reported, if it reports engagement at all. Absent from older servers and
    /// for clients that do not report it, which count as listening.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engagement: Option<Engagement>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modes(current: Option<&str>) -> SpaceState {
        SpaceState {
            mode: current.map(Into::into),
            modes: ["default", "acceptEdits", "plan", "auto"]
                .map(|id| Mode {
                    id: id.into(),
                    name: id.into(),
                    description: None,
                })
                .into(),
            ..SpaceState::default()
        }
    }

    #[test]
    fn the_next_mode_follows_the_agents_order_and_wraps() {
        let next = |current| modes(current).next_mode().map(|mode| mode.id.clone());
        assert_eq!(next(Some("default")).as_deref(), Some("acceptEdits"));
        assert_eq!(next(Some("plan")).as_deref(), Some("auto"));
        assert_eq!(next(Some("auto")).as_deref(), Some("default"));
        // A mode that is not offered, such as one set by the user's configuration, steps to the first.
        assert_eq!(next(Some("dontAsk")).as_deref(), Some("default"));
        assert_eq!(next(None).as_deref(), Some("default"));
        assert_eq!(SpaceState::default().next_mode(), None);
        assert_eq!(modes(Some("dontAsk")).current_mode(), None);
    }

    #[test]
    fn a_session_is_busy_only_when_said_and_idle_from_older_servers() {
        let older: SessionSummary = serde_json::from_value(
            serde_json::json!({"id": "s", "title": "T", "directory": "/work"}),
        )
        .unwrap();
        assert!(!older.busy);
        assert_eq!(
            serde_json::to_value(&older).unwrap(),
            serde_json::json!({"id": "s", "title": "T", "directory": "/work"})
        );
        let busy = SessionSummary {
            busy: true,
            ..older
        };
        assert_eq!(serde_json::to_value(&busy).unwrap()["busy"], true);
    }

    #[test]
    fn modes_are_optional_on_the_wire() {
        let mut json = serde_json::to_value(SpaceState::default()).unwrap();
        let object = json.as_object_mut().unwrap();
        object.remove("mode");
        object.remove("modes");
        object.insert("future".into(), serde_json::json!(true));
        let state: SpaceState = serde_json::from_value(json).unwrap();
        assert_eq!(state, SpaceState::default());
        let mode: Mode =
            serde_json::from_value(serde_json::json!({"id": "plan", "name": "Plan"})).unwrap();
        assert_eq!(mode.description, None);
    }

    #[test]
    fn speech_and_engagement_are_unknown_from_older_servers() {
        let mut json = serde_json::to_value(SpaceState {
            participants: vec![Participant {
                name: "Ada".into(),
                source: 1,
                engagement: None,
            }],
            ..SpaceState::default()
        })
        .unwrap();
        assert!(json["participants"][0].get("engagement").is_none());
        json.as_object_mut().unwrap().remove("speech");
        let state: SpaceState = serde_json::from_value(json).unwrap();
        assert_eq!(state.speech, SpeechSetting::Unknown);
        assert_eq!(state.participants[0].engagement, None);
        assert_eq!(
            serde_json::from_str::<SpeechSetting>("\"terse\"").unwrap(),
            SpeechSetting::Unknown
        );
        let participant: Participant = serde_json::from_value(
            serde_json::json!({"name": "Ada", "source": 1, "engagement": "away"}),
        )
        .unwrap();
        assert_eq!(participant.engagement, Some(Engagement::Away));
    }

    #[test]
    fn worker_completeness_is_optional_for_older_servers() {
        let legacy = serde_json::to_value(SpaceState::default()).unwrap();
        assert!(legacy.get("workers_authoritative").is_none());
        assert_eq!(
            serde_json::from_value::<SpaceState>(legacy)
                .unwrap()
                .workers_authoritative,
            None
        );
        for complete in [true, false] {
            let state = SpaceState {
                workers_authoritative: Some(complete),
                ..Default::default()
            };
            let json = serde_json::to_value(&state).unwrap();
            assert_eq!(json["workers_authoritative"], complete);
            assert_eq!(serde_json::from_value::<SpaceState>(json).unwrap(), state);
        }
    }

    #[test]
    fn approvals_list_options_only_when_offered() {
        let mut approval = Approval {
            id: "p".into(),
            title: "Bash".into(),
            text: "make".into(),
            options: Vec::new(),
        };
        let json = serde_json::to_value(&approval).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"id": "p", "title": "Bash", "text": "make"})
        );
        assert_eq!(serde_json::from_value::<Approval>(json).unwrap(), approval);

        approval.options.push(ApprovalOption {
            id: "suggestions".into(),
            label: "Yes, and don't ask again for Bash(make) in this project".into(),
        });
        let json = serde_json::to_value(&approval).unwrap();
        assert_eq!(json["options"][0]["id"], "suggestions");
        assert_eq!(serde_json::from_value::<Approval>(json).unwrap(), approval);
    }

    #[test]
    fn roles_a_newer_server_adds_are_read_as_unknown() {
        assert_eq!(
            serde_json::from_str::<ConnectionRole>("\"moderator\"").unwrap(),
            ConnectionRole::Unknown
        );
        assert_eq!(
            serde_json::to_value(ConnectionRole::Collaborator).unwrap(),
            "collaborator"
        );
        assert!(ConnectionRole::Owner.participates());
        assert!(ConnectionRole::Collaborator.participates());
        assert!(!ConnectionRole::Visitor.participates());
        assert!(!ConnectionRole::Pairing.participates());
        assert!(!ConnectionRole::Unknown.participates());
    }

    #[test]
    fn states_a_newer_server_adds_are_read_as_unknown() {
        assert_eq!(
            serde_json::from_str::<Activity>("\"compacting\"").unwrap(),
            Activity::Unknown
        );
        assert_eq!(
            serde_json::from_str::<Activity>("\"closed\"").unwrap(),
            Activity::Closed
        );
        assert_eq!(
            serde_json::from_str::<WorkerState>("\"paused\"").unwrap(),
            WorkerState::Unknown
        );
        assert_eq!(
            serde_json::from_str::<WorkerState>("\"stopped\"").unwrap(),
            WorkerState::Stopped
        );
        let summary: SpaceSummary = serde_json::from_value(serde_json::json!({
            "id": "space-1",
            "title": "Tomori",
            "backend": "fake",
            "directory": "/tmp",
            "activity": "compacting",
            "participants": 1,
        }))
        .unwrap();
        assert_eq!(summary.activity, Activity::Unknown);
    }

    #[test]
    fn attention_is_optional_and_levels_a_newer_server_adds_are_unknown() {
        let older = serde_json::json!({
            "id": "space-1",
            "title": "Tomori",
            "backend": "fake",
            "directory": "/tmp",
            "environment": "sky",
            "activity": "idle",
            "participants": 0,
        });
        let summary: SpaceSummary = serde_json::from_value(older.clone()).unwrap();
        assert_eq!((summary.attention, summary.attention_age), (None, None));
        // Without attention, nothing is added to the wire, so an older client reads exactly what it did before.
        assert_eq!(serde_json::to_value(&summary).unwrap(), older);

        let waiting = SpaceSummary {
            attention: Some(Attention::Input),
            attention_age: Some(42),
            ..summary
        };
        let json = serde_json::to_value(&waiting).unwrap();
        assert_eq!(json["attention"], "input");
        assert_eq!(json["attention_age"], 42);
        assert_eq!(
            serde_json::from_value::<SpaceSummary>(json).unwrap(),
            waiting
        );
        for (text, level) in [
            ("\"problem\"", Attention::Problem),
            ("\"reply\"", Attention::Reply),
            ("\"mentioned\"", Attention::Unknown),
        ] {
            assert_eq!(serde_json::from_str::<Attention>(text).unwrap(), level);
        }
    }

    #[test]
    fn spaces_name_their_environment_and_older_servers_name_none() {
        let summary: SpaceSummary = serde_json::from_value(serde_json::json!({
            "id": "space-1",
            "title": "Tomori",
            "backend": "fake",
            "directory": "/tmp",
            "environment": "sky",
            "activity": "idle",
            "participants": 1,
        }))
        .unwrap();
        assert_eq!(summary.environment, "sky");
        let state = SpaceState {
            environment: "observe".into(),
            ..SpaceState::default()
        };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["environment"], "observe");
        assert_eq!(serde_json::from_value::<SpaceState>(json).unwrap(), state);

        let mut older = serde_json::to_value(SpaceState::default()).unwrap();
        older.as_object_mut().unwrap().remove("environment");
        assert_eq!(
            serde_json::from_value::<SpaceState>(older)
                .unwrap()
                .environment,
            ""
        );
    }

    #[test]
    fn a_space_names_its_environments_version_unless_the_server_is_older() {
        let state = SpaceState {
            environment: "observe".into(),
            environment_version: Some(2),
            ..SpaceState::default()
        };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["environment_version"], 2);
        assert_eq!(serde_json::from_value::<SpaceState>(json).unwrap(), state);
        // An older server, or an environment no longer offered, names none, and sends no field.
        let unversioned = serde_json::to_value(SpaceState::default()).unwrap();
        assert!(unversioned.get("environment_version").is_none());
        assert_eq!(
            serde_json::from_value::<SpaceState>(unversioned)
                .unwrap()
                .environment_version,
            None
        );
        // An environment a client presents, at its version.
        let presented = EnvironmentVersion::new("observe", 2);
        assert_eq!((presented.id.as_str(), presented.version), ("observe", 2));
    }

    #[test]
    fn older_servers_do_not_change_environments() {
        let changing = SpaceState {
            environment_changes: true,
            ..SpaceState::default()
        };
        let mut json = serde_json::to_value(&changing).unwrap();
        assert_eq!(json["environment_changes"], true);
        assert_eq!(
            serde_json::from_value::<SpaceState>(json.clone()).unwrap(),
            changing
        );
        // An older server says nothing, and binds a space's environment for its lifetime.
        json.as_object_mut().unwrap().remove("environment_changes");
        assert!(
            !serde_json::from_value::<SpaceState>(json)
                .unwrap()
                .environment_changes
        );
    }
}
