// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The backend-neutral interface between the Tomori server and coding agents.
//!
//! Backends report structured facts as [`Event`]s. They never produce text for
//! display or speech; the server owns that projection. See
//! `design/agent-backends.md`.
use std::fmt;
use std::future::Future;

use serde::{Deserialize, Serialize};

/// What a backend supports, so the server can offer only working actions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Existing sessions can be listed and attached.
    pub attach: bool,
    /// A running turn can be interrupted.
    pub interrupt: bool,
    /// Structured questions can be answered through [`Session::answer`].
    pub questions: bool,
    /// Delegated worker sessions are reported.
    pub workers: bool,
    /// Named instruction entries can be set on a session through [`Session::set_instruction`], and removed from one no
    /// space is bound to through [`Backend::forget_instructions`]. See `design/agent-backends.md`.
    #[serde(default)]
    pub instructions: bool,
    /// Tomori's MCP server can be registered with the agent for a directory through [`Backend::register_tools`], so
    /// that its sessions there can use Tomori's tools. See `design/mcp.md`.
    #[serde(default)]
    pub tools: bool,
}

/// Whether `key` can name an instruction entry: lowercase letters, digits, `.`, `_` and `-`, starting with a letter or
/// digit, as OpenCode requires.
pub fn valid_instruction_key(key: &str) -> bool {
    let mut characters = key.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && characters.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '_' | '-')
        })
}

/// An MCP server that Tomori hosts for agents, to register with an agent for a directory: its name, the URL of its
/// streamable HTTP endpoint, and the headers a request to it carries, such as its bearer token. See `design/mcp.md`.
///
/// The headers are secret, so they are never printed: [`fmt::Debug`] names them without their values.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ToolServer {
    /// The name the agent knows the server by, which prefixes its tools' names, such as `tomori`.
    pub name: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
}

impl ToolServer {
    /// The server named `name` at `url`, with no headers.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            headers: Vec::new(),
        }
    }

    /// The same server, with every request carrying the header `name` with `value`.
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

impl fmt::Debug for ToolServer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers: Vec<&str> = self.headers.iter().map(|(name, _)| name.as_str()).collect();
        formatter
            .debug_struct("ToolServer")
            .field("name", &self.name)
            .field("url", &self.url)
            .field("headers", &headers)
            .finish()
    }
}

fn unsupported_tools() -> Error {
    Error::new(
        ErrorKind::Unsupported,
        "the backend does not take Tomori's tools",
    )
}

fn unsupported_instructions() -> Error {
    Error::new(
        ErrorKind::Unsupported,
        "the backend does not take instructions",
    )
}

/// How to start a new session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSpec {
    /// The working directory of the agent.
    pub directory: String,
    pub title: String,
}

/// An existing session that can be attached.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub directory: String,
}

/// Input delivered to the agent as one user turn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prompt {
    /// Unique per queue entry. A backend that is asked to send the same key twice delivers it at most once.
    pub key: String,
    /// The text to deliver, already including speaker attribution if enabled. It is delivered as the content of a user
    /// message and never interpreted as an agent command, such as a slash command, whatever it begins with.
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

/// The outcome of a permission request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    AllowOnce,
    /// Allow, and grant the [`PermissionOption`] with this id, with the same effect as choosing it in the agent's own
    /// interface.
    Grant {
        option: String,
    },
    Deny,
}

/// Where a granted permission is kept, as the agent names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantScope {
    /// Until the agent session ends.
    Session,
    /// This project, for this user only.
    Local,
    /// This project, for everyone who uses it.
    Project,
    /// Every project of this user.
    User,
}

/// One standing permission granted by a [`PermissionOption`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Grant {
    /// Calls of `tool` matching `content`, in the agent's rule syntax, run without asking. Without `content`, every call
    /// of the tool does.
    Rule {
        tool: String,
        content: Option<String>,
        scope: GrantScope,
    },
    /// The agent may work in `path`.
    Directory { path: String, scope: GrantScope },
    /// The agent's permission mode changes, such as to one that accepts edits without asking.
    Mode { mode: String, scope: GrantScope },
}

/// A choice the agent offers beside allowing once and denying, such as "don't ask again". Choosing it allows the request
/// and applies every grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionOption {
    /// Stable for the life of the request; passed back in [`Decision::Grant`].
    pub id: String,
    pub grants: Vec<Grant>,
}

/// A tool permission request, described with structured facts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Permission {
    pub id: String,
    /// The tool name, such as `Bash` or `Edit`.
    pub tool: String,
    /// The tool input as reported by the agent.
    pub input: serde_json::Value,
    /// The agent's own short description of the action, when it provides one.
    pub description: Option<String>,
    /// The standing permissions the agent's own interface offers with this request, if any.
    #[serde(default)]
    pub options: Vec<PermissionOption>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    /// Free text.
    Text,
    /// Exactly one of the options.
    Choice,
    /// Any number of the options.
    MultipleChoice,
    /// A field Tomori cannot collect; the question must be completed outside Tomori.
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldOption {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    pub key: String,
    pub title: String,
    pub description: Option<String>,
    pub kind: FieldKind,
    pub options: Vec<FieldOption>,
    /// A choice field also accepts free text.
    pub custom: bool,
}

/// A structured question from the agent, answered one field at a time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    pub title: String,
    pub fields: Vec<Field>,
}

/// The answer to one field. The server matches spoken text to options before calling the backend.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum AnswerValue {
    Text(String),
    Choices(Vec<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerState {
    Running,
    Waiting,
    Idle,
    Completed,
    Error,
    /// Ended before it finished without failing, such as when it was stopped or the agent process running it ended. The
    /// agent may resume it later.
    Stopped,
    Unknown,
}

/// A delegated worker, such as an OpenCode child session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worker {
    pub id: String,
    /// The worker that started this one, or `None` when the session itself started it.
    pub parent: Option<String>,
    pub title: String,
    pub state: WorkerState,
}

/// A mode a session can be switched to while it runs, as the agent's own interface offers it: for OpenCode, a primary
/// agent its terminal cycles through with Tab. See `design/agent-backends.md`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mode {
    /// The agent's own identifier, such as `acceptEdits` or `plan`; passed back to [`Session::set_mode`].
    pub id: String,
    /// The name the agent's interface shows for it.
    pub name: String,
    /// A short description of what the mode does.
    pub description: Option<String>,
}

/// The modes a session offers and the one it is in.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Modes {
    /// In the order the agent's own interface steps through them. Empty when the backend offers no modes.
    pub offered: Vec<Mode>,
    /// The identifier of the current mode, as the agent last reported it. It may be a mode that is not offered, such as
    /// one set by the user's configuration. `None` until the agent reports one.
    pub current: Option<String>,
}

/// A part of a message in a [`Snapshot`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text {
        text: String,
    },
    Reasoning {
        text: String,
    },
    Tool {
        id: String,
        tool: String,
        input: serde_json::Value,
        finished: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotMessage {
    pub id: String,
    pub role: Role,
    pub parts: Vec<Part>,
    pub completed: bool,
}

/// The current state of a session, used on attach and to resynchronize after a lost event.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub busy: Option<bool>,
    pub messages: Vec<SnapshotMessage>,
    pub permissions: Vec<Permission>,
    pub questions: Vec<Question>,
    /// The session's workers, oldest first as far as the backend knows, since a snapshot too large to send drops its
    /// oldest workers first.
    pub workers: Vec<Worker>,
    /// Why `workers` may not be every worker the session has, such as while the backend is still discovering them; `None`
    /// when they are. A worker it omits has then not departed: the rest follow as [`Event::WorkersPartial`] and
    /// [`Event::WorkersChanged`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workers_incomplete: Option<String>,
    #[serde(default)]
    pub modes: Modes,
}

/// A fact reported by a session, in order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// The agent started working on a turn.
    Busy,
    /// The agent finished its turn and will accept the next prompt.
    Idle,
    /// A prompt with this key was accepted by the agent.
    Delivered {
        key: String,
    },
    /// Sent before any other event that names the message.
    MessageStarted {
        message: String,
        role: Role,
    },
    /// Assistant text, streamed as it is generated, after the message's [`Event::MessageStarted`]. Concatenating the deltas of a message gives its text.
    TextDelta {
        message: String,
        text: String,
    },
    MessageCompleted {
        message: String,
    },
    ToolStarted {
        message: String,
        tool: String,
        id: String,
        input: serde_json::Value,
    },
    ToolFinished {
        id: String,
        error: Option<String>,
    },
    PermissionRequested(Permission),
    /// The permission was resolved, by Tomori or by the agent itself.
    PermissionResolved {
        id: String,
    },
    QuestionAsked(Question),
    QuestionResolved {
        id: String,
    },
    /// Every worker the backend knows. A worker already reported keeps its place in the space, so the order matters
    /// only for new ones, which come oldest first.
    WorkersChanged {
        workers: Vec<Worker>,
    },
    /// Every worker the backend knows, as for [`Event::WorkersChanged`], while it knows that they may not be all the
    /// session has, such as while it is still discovering them: omissions must not imply deletion. `message` says why.
    WorkersPartial {
        workers: Vec<Worker>,
        message: String,
    },
    /// Worker observation became incomplete; omissions must not imply deletion.
    WorkersUnavailable {
        message: String,
    },
    /// The session's offered modes or its current mode changed, as the agent confirmed: after a mode request, when the
    /// agent changes mode itself, or once the offered modes are known.
    ModesChanged(Modes),
    /// The agent refused to enter `mode`, either as the starting mode or when asked through [`Session::set_mode`]. The
    /// session stays in the mode reported by [`Event::ModesChanged`]. `reason` is the agent's own explanation, if any.
    ModeRefused {
        mode: String,
        reason: Option<String>,
    },
    /// A recoverable problem. The session continues.
    Error {
        message: String,
    },
    /// The backend's connection to the agent was lost, so events are being missed. The session's state is uncertain
    /// until [`Event::Resynchronized`] follows, or [`Event::Closed`] if the session cannot be read again. `reason` says
    /// what happened, for participants.
    Reconnecting {
        reason: String,
    },
    /// The backend lost events, such as when its connection to the agent dropped, and has read the session again.
    /// The snapshot replaces the conversation, running state, modes and pending requests, as when the session was
    /// first opened; requests the snapshot does not list are no longer pending. Events reported after this one follow
    /// it. Its `workers` are not used: workers keep travelling as [`Event::WorkersChanged`],
    /// [`Event::WorkersPartial`] and [`Event::WorkersUnavailable`], so a resynchronization never retires one.
    Resynchronized {
        snapshot: Snapshot,
    },
    /// A message whose streamed text was partly missed while the backend reconnected, as the agent recorded it once
    /// it ended. It replaces that message's text and tools; a message the space does not hold is ignored.
    MessageRestored {
        message: SnapshotMessage,
    },
    /// The session ended and produces no more events.
    Closed {
        reason: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The session, request or question does not exist or is no longer pending.
    NotFound,
    /// The backend does not support the operation.
    Unsupported,
    /// The request is invalid, such as an answer for the wrong field.
    Invalid,
    /// The agent process or service is unavailable.
    Unavailable,
    /// The session has closed.
    Closed,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// A kind of agent, such as OpenCode.
pub trait Backend: Send + Sync {
    type Session: Session;

    /// The stable identifier used in [`SessionSpec`]-creating requests, such as `opencode`.
    fn name(&self) -> &str;

    fn capabilities(&self) -> Capabilities;

    /// Existing sessions that can be attached. Empty when attaching is unsupported.
    fn sessions(&self, directory: &str)
    -> impl Future<Output = Result<Vec<SessionSummary>>> + Send;

    /// Start a new session.
    fn create(&self, spec: SessionSpec) -> impl Future<Output = Result<Self::Session>> + Send;

    /// Bind to an existing session.
    fn attach(
        &self,
        id: &str,
        directory: &str,
    ) -> impl Future<Output = Result<Self::Session>> + Send;

    /// Remove the instruction entries `keys` from the session `id`, which no space is bound to any more, as when the
    /// server forgets a closed space. A key the session does not have is not an error. A session the backend has bound
    /// again by the time this is performed keeps its entries, which are then the new binding's. Fails with
    /// [`ErrorKind::Unsupported`] unless the backend has [`Capabilities::instructions`].
    fn forget_instructions(
        &self,
        id: &str,
        keys: Vec<String>,
    ) -> impl Future<Output = Result<()>> + Send {
        let _ = (id, keys);
        std::future::ready(Err(unsupported_instructions()))
    }

    /// Register `server` with the agent for `directory`, so that its sessions there can use its tools, replacing any
    /// server registered there under the same name. Registering it again is harmless. The agent may forget it when it
    /// restarts, so the server registers it again whenever it binds a session there and after a resynchronization.
    /// Fails with [`ErrorKind::Unsupported`] unless the backend has [`Capabilities::tools`]. See `design/mcp.md`.
    fn register_tools(
        &self,
        directory: &str,
        server: &ToolServer,
    ) -> impl Future<Output = Result<()>> + Send {
        let _ = (directory, server);
        std::future::ready(Err(unsupported_tools()))
    }

    /// Remove the server named `name` from the agent for `directory`. One the agent does not have is not an error.
    /// Fails with [`ErrorKind::Unsupported`] unless the backend has [`Capabilities::tools`].
    fn unregister_tools(
        &self,
        directory: &str,
        name: &str,
    ) -> impl Future<Output = Result<()>> + Send {
        let _ = (directory, name);
        std::future::ready(Err(unsupported_tools()))
    }

    /// The server is shutting down: stop waiting for the agent. Requests still waiting for it, such as for a
    /// connection to return, fail at once, and so do later ones, so that shutdown never waits on the agent. Sessions
    /// are closed separately. Must not block.
    fn shutdown(&self) {}
}

/// One live agent session, bound to one space.
///
/// Methods take `&self` so that one task can wait on [`Session::next_event`] while others send commands.
pub trait Session: Send + Sync + 'static {
    fn id(&self) -> String;

    /// The next event, waiting until one is available. Returns [`Event::Closed`] once, then errors with [`ErrorKind::Closed`].
    fn next_event(&self) -> impl Future<Output = Result<Event>> + Send;

    fn snapshot(&self) -> impl Future<Output = Result<Snapshot>> + Send;

    /// Deliver a prompt as a user message. Only called while the session is idle; the server queues input otherwise. No
    /// prompt text runs an agent command; see [`Prompt::text`].
    fn send(&self, prompt: Prompt) -> impl Future<Output = Result<()>> + Send;

    fn decide(
        &self,
        permission: &str,
        decision: Decision,
    ) -> impl Future<Output = Result<()>> + Send;

    fn answer(
        &self,
        question: &str,
        answers: Vec<(String, AnswerValue)>,
    ) -> impl Future<Output = Result<()>> + Send;

    fn interrupt(&self) -> impl Future<Output = Result<()>> + Send;

    /// Ask the agent to switch to the offered mode `mode`, the way its own interface does. Only called when a participant
    /// chooses a mode. Returns once the request is sent; the change is reported by [`Event::ModesChanged`] when the agent
    /// confirms it, or [`Event::ModeRefused`] when it does not.
    fn set_mode(&self, mode: &str) -> impl Future<Output = Result<()>> + Send;

    /// Set the session's instruction entry `key`, which [`valid_instruction_key`] accepts, to `value`, or remove it when
    /// `value` is `None`. The agent keeps its entries with the session and gives them to the model alongside its own
    /// instructions from its next step; they never change its configuration or files. The server sets its entries
    /// again whenever it binds a session and after [`Event::Resynchronized`], so a backend need not remember them.
    /// Fails with [`ErrorKind::Unsupported`] unless the backend has [`Capabilities::instructions`].
    fn set_instruction(
        &self,
        key: &str,
        value: Option<&str>,
    ) -> impl Future<Output = Result<()>> + Send {
        let _ = (key, value);
        std::future::ready(Err(unsupported_instructions()))
    }

    /// Stop the session's process or connection. The agent's own history is kept.
    fn close(&self) -> impl Future<Output = Result<()>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruction_keys_follow_opencode() {
        for key in ["tomori", "tomori-space", "tomori.environment", "a_1", "9"] {
            assert!(valid_instruction_key(key), "{key}");
        }
        for key in [
            "",
            "-tomori",
            ".tomori",
            "Tomori",
            "tomori/space",
            "tomori space",
            "é",
        ] {
            assert!(!valid_instruction_key(key), "{key}");
        }
    }

    #[test]
    fn capabilities_from_an_older_record_take_no_instructions() {
        let capabilities: Capabilities = serde_json::from_value(serde_json::json!({
            "attach": true, "interrupt": true, "questions": true, "workers": true
        }))
        .unwrap();
        assert!(!capabilities.instructions);
        assert!(!capabilities.tools);
    }

    #[test]
    fn a_tool_server_never_shows_its_headers() {
        let server = ToolServer::new("tomori", "http://127.0.0.1:1/mcp")
            .with_header("Authorization", "Bearer tomori_secret");
        assert_eq!(
            server.headers,
            [(
                "Authorization".to_owned(),
                "Bearer tomori_secret".to_owned()
            )]
        );
        let shown = format!("{server:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert_eq!(
            shown,
            r#"ToolServer { name: "tomori", url: "http://127.0.0.1:1/mcp", headers: ["Authorization"] }"#
        );
    }
}
