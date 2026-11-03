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
}

/// An existing agent session a space can be bound to, reported by a backend that supports attaching.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub directory: String,
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
}
