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
    /// The space's project folder, its directory relative to the workspace root it is in, such as `ioquatix/tomori`,
    /// or the root's name when it is the root itself, so that spaces in different projects are told apart. Absent
    /// from older servers. See `design/arrival.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// How long ago, in whole seconds, the space was last active: when its agent last worked in it or input was
    /// queued in it. Absent when not known, and from older servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_age: Option<u32>,
    /// The first words of the agent's newest reply in the space, on one line, cut short with "…". Absent when it has
    /// none, and from older servers. A visitor sees no more than joining the space shows them. See `design/arrival.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
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
    /// The avatar that represents the backend's agent, by identifier, which [`valid_avatar`] accepts; none for the
    /// swarm, and from older servers. See `design/avatars.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
}

/// The longest avatar identifier, in bytes, that a server sends and a client reads. See `design/avatars.md`.
pub const AVATAR_BYTES: usize = 128;

/// Whether `avatar` can name an avatar: not empty, at most [`AVATAR_BYTES`], and without control characters,
/// bidirectional controls or invisible formatting characters, so that it reads as it is when a client names it.
pub fn valid_avatar(avatar: &str) -> bool {
    !avatar.is_empty() && avatar.len() <= AVATAR_BYTES && !avatar.chars().any(hidden)
}

/// Whether `character` changes how text around it reads without being seen: a control character, a bidirectional
/// control, or a zero-width or invisible formatting character.
fn hidden(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'
                | '\u{2066}'..='\u{2069}'
                | '\u{061C}'
                | '\u{FEFF}'
        )
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
    /// Whether the agent's speech is still to come: segments of its reply queued for synthesis, or synthesized and not
    /// yet all sent, however long after its turn has ended, until none of it has made progress for a minute. A client
    /// shows the agent speaking meanwhile, through the gaps while the next sentence is synthesized, rather than what its
    /// session is doing. False from older servers. See `design/speech.md`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub speech_to_come: bool,
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
    /// Whether the server takes [`crate::ClientMessage::SetWall`] from participants, changing a wall's grid, slots' size,
    /// shape and, for an added wall, pose, and leaves a removed wall's panels floating. False from older servers, which
    /// a client sends no `SetWall` and offers no wall editing. See `design/projection.md`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub walls: bool,
    /// Whether the server takes [`crate::ClientMessage::CheckPanelItem`] from participants, ticking and unticking a
    /// text panel's checklist items. False from older servers, to which a client sends none and offers no ticking. See
    /// `design/projection.md`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub checks: bool,
    /// Whether the server takes [`crate::ClientMessage::ChoosePanelRow`] from participants, handing the choice of a
    /// choosable row of an environment's list panel to its server side. False from older servers, to which a client
    /// sends none and offers no choosing. See `design/panel-actions.md`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub choices: bool,
    /// Whether the server snaps walls into groups, taking [`crate::ClientMessage::AttachWall`] from participants, moving
    /// a group together as one of its walls is moved and sending each wall's [`crate::Wall::group`] and
    /// [`crate::Wall::joined`]. False from older servers, which a client sends no `AttachWall` and offers no snapping. See
    /// `design/projection.md`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub groups: bool,
    /// Whether the server locks walls: it says which are locked, with [`crate::Wall::locked`], refuses every change to a
    /// locked wall, takes [`crate::ClientMessage::LockWall`] from owners, and lets an environment's own wall, unlocked, be
    /// moved. False from older servers, whose environment walls stay where the environment puts them; a client offers
    /// no lock and moves no environment wall without it. See `design/projection.md`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub wall_locks: bool,
    /// Whether the server accepts [`crate::ClientMessage::DeleteSpace`] from an owner in the space. False from older
    /// servers; a client offers deleting the space only when this is set. See `design/spaces.md`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deletes: bool,
    /// Whether deleting the space can delete its agent session too, as its backend can. A client offers it only when
    /// this is set, as well as [`Self::deletes`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deletes_session: bool,
    /// Who gave the space its title, when someone did: absent while the title follows the space's agent session, and
    /// from older servers. A client that sees the title change while it is in the space says who changed it. See
    /// `design/spaces.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renamed_by: Option<Renamer>,
    /// Whether the server accepts [`crate::ClientMessage::RenameSpace`] from an owner, and an utterance marked as
    /// titling the space. False from older servers; a client offers renaming the space only when this is set.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub renames: bool,
    /// Where the space's environment has participants arrive, and puts the agent's home and the workers' arrangement,
    /// each only when its registration says. None from older servers, and when the server no longer offers the
    /// environment, which a client reads as the default layout. See `design/environments.md`.
    #[serde(default, skip_serializing_if = "Anchors::is_empty")]
    pub anchors: Anchors,
    /// Where the agent stands away from its home, in the space's coordinates, facing along its +Z, as it follows the
    /// speaker or was called or asked to go there: absent while it is at home, as it is when the space opens and after
    /// the agent or a participant sends it home. See `design/environments.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summoned: Option<crate::Pose>,
    /// Sent only by older servers, while the agent was summoned: where the eyes were of whoever it was summoned to, in
    /// the space's coordinates, whom its entourage gathered about, its workers on their arc about them. A server no
    /// longer sends it, since nothing travels with the agent, and a client reads its absence, with
    /// [`Self::movement`] sent, as the workers at home; whom the agent stands for is [`Self::partner`]. See
    /// `design/environments.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summoner: Option<[f32; 3]>,
    /// Whether the server keeps where a participant's trigger summons the agent to, taking
    /// [`crate::ClientMessage::PlaceAgent`] with a place from a participant. Set only by older servers: since the agent
    /// follows whoever speaks to it, a server no longer keeps summons, and a client sends none. See
    /// `design/environments.md`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub summons: bool,
    /// How the space's agent moves, an owner's setting, kept with the space: following whoever speaks to it, only
    /// when asked, or staying home, and how long after a conversation goes quiet it goes home. A server that sends it
    /// accepts [`crate::ClientMessage::CallAgent`], [`crate::ClientMessage::SetAgentMovement`] from an owner, and
    /// [`crate::ClientMessage::PlaceAgent`] without a place to send the agent home; a client sends those only to such a
    /// server. None from older servers. See `design/environments.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub movement: Option<AgentMovement>,
    /// Whether the agent is pinned where it stands, by a participant's **Call Tomori here** or **Tomori: stay here**, or
    /// the agent's own `move_agent stay`: it follows nobody and does not go home on its own until it is sent home or
    /// follows again. See `design/environments.md`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
    /// Whether the server takes [`crate::ClientMessage::PinAgent`], pinning the agent where it stands or ending its
    /// pin, which a client offers as **Tomori: stay here** and **Tomori: follow again**, and sends only to such a
    /// server. Absent from older servers. See `design/hand-menu.md`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pins: bool,
    /// Whether the server takes [`crate::ClientMessage::WalkAgent`], having the agent walk with a participant or ending
    /// its walk with them, which a client offers as **Tomori: walk with me** and **Tomori: stop walking with me**, and
    /// sends only to such a server. Absent from older servers. See `design/hand-menu.md`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub walks: bool,
    /// The number of the participant the agent walks with, [`Participant::source`], while it does: it keeps with them
    /// throughout. Absent otherwise, and from older servers. See `design/environments.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub walking: Option<u32>,
    /// Where the agent turns to face, in the space's coordinates, while it does, rather than the way it stands facing:
    /// the eyes of whoever began speaking to it. Turned only about the vertical. See `design/environments.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facing: Option<[f32; 3]>,
    /// The eyes of whoever the agent stands for while it is away from home, in the space's coordinates: whoever it
    /// followed, whoever called it, or whoever it went to as it asked. A client draws the swarm in within
    /// `avatar`'s near distance of them, so that it never fills their view. Absent at home, and from older servers.
    /// See `design/avatars.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partner: Option<[f32; 3]>,
    /// A point the agent passes through on its way to where it stands, in the space's coordinates, where the straight
    /// way there passes through someone, a wall or the environment's scenery. See `design/environments.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<[f32; 3]>,
    /// Whether the agent streams to where it stands as a flock, at a worker's flight speed, rather than gliding: far
    /// away, or where no way there is clear. See `design/environments.md`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stream: bool,
    /// The avatar that represents the space's agent, as its backend names it, by identifier: none for the swarm, from
    /// older servers, and when the server no longer offers the backend. A client shows the swarm for one its build
    /// lacks, and says so. See `design/avatars.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    /// How the server lays out each cluster anchor's cluster that a worker stands at: the span each is laid out for,
    /// and the slot each worker takes there, so that every client lays them out alike. Empty from older servers, whose
    /// clients give their workers slots at home themselves; an older client ignores it, and keeps every worker at home.
    /// See `design/environments.md`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clusters: Vec<crate::cluster::Layout>,
    /// The space's timers, scheduled notes its agent set itself, in the order they were set, for a client to draw
    /// beside the workers and point at as [`crate::Resource::Timer`]. Empty from older servers and where there are none.
    /// See `design/timers.md`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub timers: Vec<Timer>,
}

/// One of a space's timers as a client draws it: its name, its schedule and condition in short words, when it next
/// fires and how it stands now. See `design/timers.md`.
///
/// ```
/// use tomori_protocol::{Timer, TimerState};
///
/// let timer = Timer::new("status", "every hour", "when present", Some(1_800_000_000), TimerState::Paused);
/// assert_eq!(
///     serde_json::to_value(&timer).unwrap(),
///     serde_json::json!({
///         "name": "status",
///         "schedule": "every hour",
///         "condition": "when present",
///         "next": 1_800_000_000,
///         "state": "paused",
///     })
/// );
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Timer {
    /// Its name, which tells it from the space's other timers.
    pub name: String,
    /// Its schedule in short words, as `every hour` or `weekdays at 09:00`.
    pub schedule: String,
    /// Its condition in short words: `always`, `when present` or `when present or on arrival`.
    pub condition: String,
    /// When it next fires, in seconds since the Unix epoch, for a countdown; absent while it waits for someone to
    /// arrive with no occurrence ahead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<u64>,
    pub state: TimerState,
}

impl Timer {
    pub fn new(
        name: impl Into<String>,
        schedule: impl Into<String>,
        condition: impl Into<String>,
        next: Option<u64>,
        state: TimerState,
    ) -> Self {
        Self {
            name: name.into(),
            schedule: schedule.into(),
            condition: condition.into(),
            next,
            state,
        }
    }
}

/// How a timer stands now, for a distinct look of each.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerState {
    /// Counting down to its next occurrence.
    Waiting,
    /// It fires only while someone is present, and nobody is.
    Paused,
    /// An occurrence fell while nobody was present, and it fires as the first participant arrives.
    Awaiting,
    /// It fired within the last minute.
    Fired,
    /// A state a newer server sends that this build does not know.
    #[default]
    #[serde(other)]
    Unknown,
}

/// Who gave a space its title, in place of its agent session's. See `design/spaces.md`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Renamer {
    /// The space's agent, through Tomori's tools, as someone in the space asked it.
    Agent,
    /// An owner in the space, by their display name as the server knew them.
    Participant { name: String },
    /// Someone a newer server names that this build does not know.
    #[serde(other)]
    Unknown,
}

/// Where a space's environment has participants arrive, and where it puts Tomori's agent and its delegated workers.
/// Every anchor faces along its pose's +Z, the way it looks: the participants arriving, the agent's eye, and the
/// workers' arrangement, as walls do (see [`crate::Pose::facing_towards`]). `agent` and `workers` are in the space's
/// coordinates, where home, the arrival, is the origin, facing −Z, each upright, turned only about the vertical;
/// `arrival` is in the environment's own coordinates, which a client that draws the environment carries into the
/// space's with [`crate::Pose::carriage`]. A client uses the default layout for each that is absent. See
/// `design/environments.md`.
///
/// ```
/// use tomori_protocol::{Anchors, Pose};
///
/// let pod = Pose::at([2.0, 1.2, -4.0]).facing_towards([0.0, 1.6, 0.0]);
/// let anchors = Anchors::new().with_agent(pod);
/// assert_eq!(anchors.agent, Some(pod));
/// assert!(anchors.workers.is_none());
/// assert!(Anchors::new().is_empty());
/// ```
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Anchors {
    /// The agent's home: where the swarm's middle stands, its eye facing along the pose's +Z, until it follows the
    /// speaker or is called or asked elsewhere, and again once it is sent home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<crate::Pose>,
    /// The middle of the workers' arrangement, which faces along the pose's +Z, towards whoever watches it, as the
    /// agent does; by default, the agent's home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workers: Option<crate::Pose>,
    /// Where participants arrive, in the environment's own coordinates, facing along its +Z: home. Sent only for an
    /// environment that arrives elsewhere than its own origin facing −Z.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrival: Option<crate::Pose>,
    /// The space's cluster anchors that a client lays out, in the space's coordinates: its environment's `home` anchor,
    /// where it gives the room the workers' arrangement may take, at [`Self::workers`] too, which an older client
    /// keeps today's arc about; its environment's other anchors, declared or placed by its server side as it runs; and
    /// those the server makes for workers placed beside a wall or a person, named from [`crate::cluster::OWN`]. A
    /// client lays out what stands at each by `tomori_protocol::cluster`. See `design/environments.md`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clusters: Vec<crate::cluster::Anchor>,
    /// Where people arrive as they join the space, its environment's spawn points, in the space's coordinates, each
    /// upright, from which the server chooses one for each person joining, sending it to them alone as
    /// [`crate::ServerMessage::Spawn`], and the spectator one for itself, by [`crate::spawn::choose`]. None where the
    /// environment declares none, when everyone arrives at home. See `design/environments.md`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spawns: Vec<crate::spawn::Spawn>,
}

impl Anchors {
    /// None: the default layout.
    pub fn new() -> Self {
        Self::default()
    }

    /// These anchors, with the agent's home at `agent`.
    pub fn with_agent(mut self, agent: crate::Pose) -> Self {
        self.agent = Some(agent);
        self
    }

    /// These anchors, with the middle of the workers' arrangement at `workers`.
    pub fn with_workers(mut self, workers: crate::Pose) -> Self {
        self.workers = Some(workers);
        self
    }

    /// These anchors, with participants arriving at `arrival`, in the environment's own coordinates.
    pub fn with_arrival(mut self, arrival: crate::Pose) -> Self {
        self.arrival = Some(arrival);
        self
    }

    /// These anchors, with the cluster anchor `anchor` too, in place of one of its name.
    pub fn with_cluster(mut self, anchor: crate::cluster::Anchor) -> Self {
        self.clusters.retain(|kept| kept.name != anchor.name);
        self.clusters.push(anchor);
        self
    }

    /// These anchors, with people arriving at `spawn` too, in the space's coordinates.
    pub fn with_spawn(mut self, spawn: crate::spawn::Spawn) -> Self {
        self.spawns.push(spawn);
        self
    }

    /// The cluster anchor named `name`, if it gives one.
    pub fn cluster(&self, name: &str) -> Option<&crate::cluster::Anchor> {
        self.clusters.iter().find(|anchor| anchor.name == name)
    }

    /// Whether it says nothing, so that the default layout stands.
    pub fn is_empty(&self) -> bool {
        self.agent.is_none()
            && self.workers.is_none()
            && self.arrival.is_none()
            && self.clusters.is_empty()
            && self.spawns.is_empty()
    }
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

/// How a space's agent moves about the space, an owner's setting. See `design/environments.md`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Movement {
    /// It comes to whoever it is talking with, standing beside them, and goes home once the conversation is quiet.
    Follow,
    /// It moves only when someone asks it, by voice or with **Call Tomori here**, and stays where it goes until it is
    /// sent home. The default.
    #[default]
    Asked,
    /// It never leaves its home, as for a presentation or a cramped environment.
    Home,
    /// A setting a newer server sends that this build does not know.
    #[serde(other)]
    Unknown,
}

/// How a space's agent moves, and when it goes home on its own: [`SpaceState::movement`]. See
/// `design/environments.md`.
///
/// ```
/// use tomori_protocol::{AgentMovement, Movement};
///
/// let movement = AgentMovement::default();
/// assert_eq!(movement.mode, Movement::Asked);
/// assert_eq!(movement.return_after, Some(90));
/// // Out of bounds, the delay is brought within them.
/// assert_eq!(AgentMovement::new(Movement::Follow, Some(5)).return_after, Some(30));
/// assert_eq!(AgentMovement::new(Movement::Follow, None).next_return().return_after, Some(30));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AgentMovement {
    #[serde(default)]
    pub mode: Movement,
    /// How long, in seconds, a conversation stays quiet before a following agent goes home, from
    /// [`RETURN_SHORTEST`] to [`RETURN_LONGEST`], or never. Absent means never.
    #[serde(default)]
    pub return_after: Option<u32>,
}

/// The shortest delay after which a following agent goes home, in seconds.
pub const RETURN_SHORTEST: u32 = 30;

/// The longest delay after which a following agent goes home, in seconds, short of never.
pub const RETURN_LONGEST: u32 = 600;

/// The delay after which a following agent goes home by default, in seconds.
pub const RETURN_AFTER: u32 = 90;

/// The delays a client steps through, in seconds, before never.
pub const RETURN_CHOICES: [u32; 6] = [30, 60, 90, 120, 300, 600];

impl Default for AgentMovement {
    fn default() -> Self {
        Self {
            mode: Movement::default(),
            return_after: Some(RETURN_AFTER),
        }
    }
}

impl AgentMovement {
    /// Moving as `mode` says, going home after `return_after` seconds, brought within its bounds, or never.
    pub fn new(mode: Movement, return_after: Option<u32>) -> Self {
        Self {
            mode,
            return_after: return_after.map(|delay| delay.clamp(RETURN_SHORTEST, RETURN_LONGEST)),
        }
    }

    /// This setting, with the next mode a client steps to: from only when asked, the default, to following, then
    /// staying home, then only when asked again.
    pub fn next_mode(self) -> Self {
        let mode = match self.mode {
            Movement::Asked | Movement::Unknown => Movement::Follow,
            Movement::Follow => Movement::Home,
            Movement::Home => Movement::Asked,
        };
        Self { mode, ..self }
    }

    /// This setting, with the next delay a client steps to, of [`RETURN_CHOICES`] and then never, and then the first
    /// again.
    pub fn next_return(self) -> Self {
        let return_after = match self.return_after {
            None => Some(RETURN_CHOICES[0]),
            Some(delay) => RETURN_CHOICES
                .iter()
                .copied()
                .find(|choice| *choice > delay),
        };
        Self {
            return_after,
            ..self
        }
    }
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
    /// Not finished, but it has shown no progress for longer than the server allows, as the server judged it. See
    /// `design/events.md`.
    Stalled,
    /// It was given a message and has shown no progress since, for longer than the server allows, as the server judged
    /// it. See `design/events.md`.
    Unanswered,
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

    #[test]
    fn timers_are_carried_in_the_state_and_absent_from_an_older_server() {
        let older: SpaceState =
            serde_json::from_value(serde_json::to_value(SpaceState::default()).unwrap()).unwrap();
        assert!(older.timers.is_empty());
        assert!(
            serde_json::to_value(SpaceState::default())
                .unwrap()
                .get("timers")
                .is_none()
        );
        let state = SpaceState {
            timers: vec![Timer::new(
                "status",
                "every hour",
                "always",
                None,
                TimerState::Fired,
            )],
            ..SpaceState::default()
        };
        let read: SpaceState =
            serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();
        assert_eq!(read.timers, state.timers);
        // A state a newer server sends, unknown here.
        let timer: Timer = serde_json::from_value(serde_json::json!({
            "name": "x", "schedule": "every hour", "condition": "always", "state": "snoozing"
        }))
        .unwrap();
        assert_eq!((timer.next, timer.state), (None, TimerState::Unknown));
    }

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
    fn who_renamed_a_space_is_optional_and_one_a_newer_server_names_is_unknown() {
        // From an older server, a space neither says who renamed it nor that it renames.
        let older: SpaceState = serde_json::from_value(serde_json::json!({
            "id": "s", "title": "T", "backend": "b", "directory": "/", "activity": "idle", "messages": [],
            "queue": [], "approval": null, "question": null, "workers": [], "presentations": [],
            "participants": [], "attribution": true, "error": null,
        }))
        .unwrap();
        assert_eq!((older.renamed_by.clone(), older.renames), (None, false));
        // A space that follows its session adds nothing to the wire.
        let json = serde_json::to_value(&older).unwrap();
        assert!(json.get("renamed_by").is_none() && json.get("renames").is_none());

        let renamed = SpaceState {
            renamed_by: Some(Renamer::Participant { name: "Ada".into() }),
            renames: true,
            ..older
        };
        let json = serde_json::to_value(&renamed).unwrap();
        assert_eq!(
            json["renamed_by"],
            serde_json::json!({"kind": "participant", "name": "Ada"})
        );
        assert_eq!(json["renames"], true);
        assert_eq!(serde_json::from_value::<SpaceState>(json).unwrap(), renamed);
        for (json, renamer) in [
            (serde_json::json!({"kind": "agent"}), Renamer::Agent),
            (serde_json::json!({"kind": "operator"}), Renamer::Unknown),
        ] {
            assert_eq!(serde_json::from_value::<Renamer>(json).unwrap(), renamer);
        }
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
        assert_eq!(
            (
                summary.folder.as_deref(),
                summary.active_age,
                summary.latest.as_deref()
            ),
            (None, None, None)
        );
        // What a newer server says of each space is read back, and an older client skips it as unknown fields.
        let described = SpaceSummary {
            folder: Some("ioquatix/tomori".into()),
            active_age: Some(300),
            latest: Some("All 42 tests pass…".into()),
            ..summary.clone()
        };
        let json = serde_json::to_value(&described).unwrap();
        assert_eq!(json["folder"], "ioquatix/tomori");
        assert_eq!(json["active_age"], 300);
        assert_eq!(json["latest"], "All 42 tests pass…");
        assert_eq!(
            serde_json::from_value::<SpaceSummary>(json).unwrap(),
            described
        );

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
    fn anchors_are_sent_only_when_given_and_absent_from_older_servers() {
        let pose = crate::Pose {
            position: [1.0, 1.5, -3.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let state = SpaceState {
            anchors: Anchors::new()
                .with_agent(pose)
                .with_workers(pose)
                .with_arrival(pose),
            summoned: Some(pose),
            summoner: Some([0.5, 1.5, 0.0]),
            summons: true,
            ..SpaceState::default()
        };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["summoner"][1], 1.5);
        assert_eq!(json["anchors"]["agent"]["position"][0], 1.0);
        assert_eq!(json["anchors"]["arrival"]["position"][1], 1.5);
        assert_eq!(json["summoned"]["position"][2], -3.0);
        assert_eq!(json["summons"], true);
        assert!(!Anchors::new().with_arrival(pose).is_empty());
        let spawned = Anchors::new().with_spawn(crate::spawn::Spawn::new(pose));
        assert!(!spawned.is_empty());
        assert_eq!(
            serde_json::to_value(&spawned).unwrap()["spawns"][0]["pose"]["position"][0],
            1.0
        );
        assert!(
            serde_json::to_value(Anchors::new())
                .unwrap()
                .get("spawns")
                .is_none()
        );
        assert_eq!(json["anchors"]["workers"]["position"][2], -3.0);
        assert_eq!(serde_json::from_value::<SpaceState>(json).unwrap(), state);
        // Only those given are sent.
        let agent = serde_json::to_value(Anchors::new().with_agent(pose)).unwrap();
        assert!(agent.get("workers").is_none());
        // None at all sends no field, and an older server's state, without it, reads as none.
        let plain = serde_json::to_value(SpaceState::default()).unwrap();
        assert!(plain.get("anchors").is_none());
        assert!(plain.get("summoned").is_none() && plain.get("summons").is_none());
        assert!(plain.get("summoner").is_none());
        let read = serde_json::from_value::<SpaceState>(plain).unwrap();
        assert!(read.anchors.is_empty());
        assert!(!state.anchors.is_empty());
    }

    #[test]
    fn the_server_s_layout_of_clusters_is_sent_beside_the_workers_and_an_older_client_ignores_it() {
        use crate::cluster::Layout;
        let state = SpaceState {
            workers: vec![Worker {
                id: "ses_1".into(),
                parent: None,
                title: "Rack A12".into(),
                state: WorkerState::Running,
            }],
            clusters: vec![Layout::new("rack/a12", 1).with_seat("ses_1", 0)],
            ..SpaceState::default()
        };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["clusters"][0]["seats"][0]["worker"], "ses_1");
        assert_eq!(
            serde_json::from_value::<SpaceState>(json.clone()).unwrap(),
            state
        );
        // From an older server, none: its clients give their workers slots themselves.
        let mut older = json.clone();
        older.as_object_mut().unwrap().remove("clusters");
        assert!(
            serde_json::from_value::<SpaceState>(older)
                .unwrap()
                .clusters
                .is_empty()
        );
        assert!(
            serde_json::to_value(SpaceState::default())
                .unwrap()
                .get("clusters")
                .is_none()
        );
        // An older client, reading only what it knows, keeps its workers.
        #[derive(Deserialize)]
        struct Older {
            workers: Vec<Worker>,
        }
        let older: Older = serde_json::from_value(json).unwrap();
        assert_eq!(older.workers, state.workers);
    }

    #[test]
    fn the_home_cluster_s_room_is_sent_beside_the_workers_anchor_and_an_older_client_ignores_it() {
        use crate::cluster::{Anchor, Extent, HOME};
        let pose = crate::Pose::at([0.0, 1.4, -4.0]);
        let home = Anchor::new(HOME, pose).with_extent(5.0, 2.5, 2.0);
        let anchors = Anchors::new()
            .with_workers(pose)
            .with_cluster(Anchor::new(HOME, pose))
            .with_cluster(home.clone());
        // One of a name: the last given.
        assert_eq!(anchors.clusters, std::slice::from_ref(&home));
        assert_eq!(
            anchors.cluster(HOME).and_then(|home| home.extent),
            Some(Extent::new(5.0, 2.5, 2.0))
        );
        assert_eq!(anchors.cluster("aisle-3"), None);
        assert!(!Anchors::new().with_cluster(home).is_empty());
        let json = serde_json::to_value(&anchors).unwrap();
        assert_eq!(json["clusters"][0]["name"], "home");
        assert_eq!(json["clusters"][0]["extent"]["width"], 5.0);
        assert_eq!(
            serde_json::from_value::<Anchors>(json.clone()).unwrap(),
            anchors
        );
        // Without room, none is written; and a state from an older server, without them, reads as none.
        let bare = serde_json::to_value(Anchor::new(HOME, pose)).unwrap();
        assert!(bare.get("extent").is_none());
        assert!(
            serde_json::to_value(Anchors::new().with_workers(pose))
                .unwrap()
                .get("clusters")
                .is_none()
        );
        // An older client, reading only what it knows, keeps the workers anchor.
        #[derive(Deserialize)]
        struct Older {
            workers: Option<crate::Pose>,
        }
        let older: Older = serde_json::from_value(json).unwrap();
        assert_eq!(older.workers, Some(pose));
    }

    #[test]
    fn how_the_agent_moves_is_sent_only_by_newer_servers_and_steps_through_its_choices() {
        let state = SpaceState {
            movement: Some(AgentMovement::new(Movement::Follow, Some(120))),
            pinned: true,
            pins: true,
            walks: true,
            walking: Some(11),
            facing: Some([1.0, 1.6, 2.0]),
            via: Some([0.0, 2.5, -1.0]),
            stream: true,
            ..SpaceState::default()
        };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["pins"], true);
        assert_eq!(
            (&json["walks"], &json["walking"]),
            (&serde_json::json!(true), &serde_json::json!(11))
        );
        assert_eq!(json["movement"]["mode"], "follow");
        assert_eq!(json["movement"]["return_after"], 120);
        assert_eq!(
            (json["pinned"].as_bool(), json["stream"].as_bool()),
            (Some(true), Some(true))
        );
        assert_eq!(json["facing"][2], 2.0);
        assert_eq!(json["via"][1], 2.5);
        assert_eq!(serde_json::from_value::<SpaceState>(json).unwrap(), state);
        // From an older server, none of it, and none of it is written.
        let plain = serde_json::to_value(SpaceState::default()).unwrap();
        for field in [
            "movement", "pinned", "pins", "walks", "walking", "facing", "via", "stream",
        ] {
            assert!(plain.get(field).is_none(), "{field}");
        }
        // A way a newer server names reads as unknown; never going home is no delay.
        let newer: AgentMovement =
            serde_json::from_value(serde_json::json!({ "mode": "wander" })).unwrap();
        assert_eq!((newer.mode, newer.return_after), (Movement::Unknown, None));
        assert_eq!(
            serde_json::from_value::<AgentMovement>(serde_json::json!({}))
                .unwrap()
                .mode,
            Movement::Asked
        );
        // A client steps the mode round, and the delay through its choices to never and round again.
        let asked = AgentMovement::default();
        assert_eq!(asked.next_mode().mode, Movement::Follow);
        assert_eq!(asked.next_mode().next_mode().mode, Movement::Home);
        assert_eq!(newer.next_mode().mode, Movement::Follow);
        assert_eq!(asked.next_mode().next_mode().next_mode(), asked);
        let mut delays = Vec::new();
        let mut stepped = AgentMovement::new(Movement::Follow, Some(RETURN_SHORTEST));
        for _ in 0..7 {
            delays.push(stepped.return_after);
            stepped = stepped.next_return();
        }
        assert_eq!(
            delays,
            [
                Some(30),
                Some(60),
                Some(90),
                Some(120),
                Some(300),
                Some(600),
                None
            ]
        );
        assert_eq!(stepped.return_after, Some(30));
        assert_eq!(
            AgentMovement::new(Movement::Follow, Some(9000)).return_after,
            Some(RETURN_LONGEST)
        );
        assert_eq!(AgentMovement::default().return_after, Some(RETURN_AFTER));
    }

    #[test]
    fn avatars_are_sent_only_when_named_and_absent_from_older_servers() {
        let state = SpaceState {
            avatar: Some("river".into()),
            ..SpaceState::default()
        };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["avatar"], "river");
        assert_eq!(serde_json::from_value::<SpaceState>(json).unwrap(), state);
        // None sends no field, and an older server's state, without it, reads as none: the swarm.
        let plain = serde_json::to_value(SpaceState::default()).unwrap();
        assert!(plain.get("avatar").is_none());
        assert_eq!(
            serde_json::from_value::<SpaceState>(plain).unwrap().avatar,
            None
        );
        // A backend's too.
        let backend = BackendInfo {
            name: "river".into(),
            attach: false,
            avatar: Some("river".into()),
        };
        let json = serde_json::to_value(&backend).unwrap();
        assert_eq!(json["avatar"], "river");
        assert_eq!(
            serde_json::from_value::<BackendInfo>(json).unwrap(),
            backend
        );
        let older: BackendInfo =
            serde_json::from_value(serde_json::json!({"name": "opencode", "attach": true}))
                .unwrap();
        assert_eq!(older.avatar, None);
        assert!(
            serde_json::to_value(&older)
                .unwrap()
                .get("avatar")
                .is_none()
        );
    }

    #[test]
    fn an_avatar_is_named_by_a_short_printable_identifier() {
        assert!(valid_avatar("swarm"));
        assert!(valid_avatar("river-2 ✨"));
        assert!(valid_avatar(&"a".repeat(AVATAR_BYTES)));
        assert!(!valid_avatar(""));
        assert!(!valid_avatar(&"a".repeat(AVATAR_BYTES + 1)));
        assert!(!valid_avatar("river\n"));
        assert!(!valid_avatar("\u{7}"));
        // Nothing that reads otherwise than it is: bidirectional controls and invisible formatting characters.
        for hidden in [
            '\u{202A}', '\u{202E}', '\u{2066}', '\u{2069}', '\u{200E}', '\u{200F}', '\u{061C}',
            '\u{200B}', '\u{200C}', '\u{200D}', '\u{2060}', '\u{FEFF}',
        ] {
            assert!(!valid_avatar(&format!("riv{hidden}er")), "{hidden:?}");
        }
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
