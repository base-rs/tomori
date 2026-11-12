// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Shared server state: participants, connections, spaces and open utterances, behind one lock.
//!
//! Every client action and backend event is applied synchronously under the lock. Work that may call out of the server — transcriber calls and backend operations — is deferred until the lock is released, or sent to the space's driver task.
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, Weak};
use std::time::{Duration, Instant, SystemTime};

use base_scheduler::Cancellation;
use event_listener::{Event as Notify, EventListener};
use tomori_backend::{AnswerValue, Backend, ErrorKind, Event, Prompt, Snapshot};
use tomori_discovery::{AttentionCounts, Counts};
use tomori_protocol::{
    Attention, AudioFrame, ClientMessage, ConnectionRole, Engagement, EnvironmentVersion, Frame,
    FrameKind, Gate, Keepalive, Limits, PROTOCOL_VERSION, Participant, ServerMessage,
    SpeechSetting, WorkspaceRoot,
};
use tomori_security::pairing::ChosenName;

use crate::access::{self, Access, Grant};
use crate::backends::Backends;
use crate::environments::Environments;
use crate::inbound::Inbound;
use crate::limits;
use crate::outbox::{Outbox, Outgoing};
use crate::retention::{Delivery, Item, Retention};
use crate::space::{AnswerOutcome, SpaceModel};
use crate::transcriber::{Transcriber, TranscriptSink, UtteranceId};
use crate::workspace::SpaceSlots;

pub(crate) mod agent_actions;
pub(crate) mod agent_context;
pub(crate) mod agent_inspect;
mod agent_names;
mod agent_pairing;
pub(crate) mod agent_panels;
mod agent_places;
mod agent_timers;
pub(crate) mod agent_workers;
mod anchors;
mod following;
mod layouts;
mod pairing_window;
mod provenance;
mod sides;
// Only the speech feature's voice path hands the hub the agent's speech.
#[cfg_attr(not(feature = "speech"), allow(dead_code))]
mod speech;
pub(crate) mod tools;
mod world;
#[cfg(all(test, feature = "speech"))]
pub(crate) use following::STALLED as SPEECH_STALLED;
pub(crate) use following::home as agent_home;

pub(crate) use agent_pairing::Decision;
pub use pairing_window::{
    Closing, Opener, PAIRING_OPEN_FOR, PairingState, REFUSALS_TOLD_EVERY, spoken,
};
pub use provenance::REQUEST_RECENT;

pub type ConnectionId = u64;

/// A registered connection: its identifier, its outbox, the flag it sets when data arrives, and where it reports its writes. See [`Hub::connect`].
pub type Connected = (ConnectionId, Arc<Outbox>, Arc<AtomicBool>, Arc<Output>);

/// The shortest time between snapshots of one space caused by agent events, so that a streaming reply sends about ten full snapshots a second however fast its text arrives. See `design/server.md`.
pub const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(100);

/// How long after telling a space's connections of a change its environment's side made to what it projects the next
/// such change is told: at most four times a second, however often its panels change. A change within the interval is
/// told at its end, with every change since, the newest state winning, so that one change is told at once and a stream
/// of them costs each connection, and the hub's lock, which every space shares, at most four builds of its projection a
/// second. See `design/projection.md`.
pub const PROJECTION_INTERVAL: Duration = Duration::from_millis(250);

/// What this server states in its `Welcome` that it sends at most, and sends each client within the lesser of and what
/// the client states: the most any connection is bound by, as its stage holds a space's projection within. See
/// `design/projection.md`.
pub const LIMITS: Limits = Limits::MOST;

/// The most closed spaces kept that nobody is in. Beyond this, the oldest are forgotten, so that sessions ending repeatedly cannot grow the server's memory without bound. See `design/spaces.md`.
pub const CLOSED_SPACES: usize = 32;

/// Who is where, for the status page. See `design/security.md`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub spaces: Vec<SpaceStatus>,
    /// The display names and roles of the connected participants in the lobby.
    pub lobby: Vec<(String, ConnectionRole)>,
    /// Visitors connected, wherever they are.
    pub visitors: usize,
    /// What spaces' agents recently did with pairings, through Tomori's tools, newest last. See `design/security.md`.
    pub pairing_decisions: Vec<String>,
}

/// One space on the status page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpaceStatus {
    pub title: String,
    pub activity: tomori_protocol::Activity,
    /// The display names and roles of its participants.
    pub participants: Vec<(String, ConnectionRole)>,
    /// How many visitors are watching it.
    pub visitors: usize,
}

/// How long a knock waits for a participant to answer before it expires. See `design/security.md`.
pub const KNOCK_TIMEOUT: Duration = Duration::from_secs(120);

/// How long a visitor waits after a declined or expired knock before knocking on the same space again.
pub const KNOCK_COOLDOWN: Duration = Duration::from_secs(60);

/// The most knocks waiting on one space.
pub const KNOCKS_PER_SPACE: usize = 8;

/// A visitor's knock on a space, waiting for a participant to answer.
#[derive(Clone)]
struct KnockRecord {
    id: String,
    space: String,
    /// The visitor, by hub key.
    participant: String,
    connection: ConnectionId,
    /// The knocking connection's outbox, told what became of the knock. Told after the connection goes, it hears nothing.
    outbox: Arc<Outbox>,
    /// The tokens that admit the visitor, which the server has, or it would not take the knock.
    tokens: Arc<tomori_security::TokenStore>,
    name: String,
    started: Instant,
}

/// How long a pairing waits to be confirmed before it is withdrawn. See `design/security.md`.
pub const PAIRING_TIMEOUT: Duration = Duration::from_secs(120);

/// How long a device has to reveal its value once the server has sent its own.
pub const REVEAL_TIMEOUT: Duration = Duration::from_secs(10);

/// The most pairings that begin within [`PAIRING_WINDOW`], so that a relay cannot keep trying values until the two codes happen to match.
pub const PAIRING_ATTEMPTS: usize = 3;
pub const PAIRING_WINDOW: Duration = Duration::from_secs(600);

/// The most of what spaces' agents did with pairings that the status page shows.
pub const PAIRING_DECISIONS: usize = 8;

/// A pairing as the status page shows it. Only the status page, shown only to owners, shows its code; the console and the log never do. See `design/security.md`.
#[derive(Clone, PartialEq, Eq)]
pub struct PairingView {
    /// Names this pairing, so that the status page confirms the one it showed.
    pub serial: u64,
    /// The device, by what is left of the name it chose for itself; never the name as it chose it. See `design/security.md`.
    pub device: ChosenName,
    /// The code, written as the device shows it, once the device has revealed its value.
    pub code: Option<String>,
}

/// Written without its code, so that no debugging output holds it.
impl std::fmt::Debug for PairingView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PairingView")
            .field("serial", &self.serial)
            .field("device", &self.device)
            .field("code", &self.code.as_ref().map(|_| Redacted))
            .finish()
    }
}

/// Stands for a pairing code, or text that may be one, in debugging output.
struct Redacted;

impl std::fmt::Debug for Redacted {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted]")
    }
}

/// How a pairing is confirmed: by typing the code the device shows, on the console; by the pairing the status page showed, whose code an owner compared; or by the code a space's agent was given, read aloud from the device. A typed or given code that does not match declines the pairing. See `design/security.md`.
#[derive(Clone, Copy)]
pub enum Confirmation<'a> {
    Typed(&'a str),
    Shown(u64),
    Agent(&'a str),
}

/// Written without the code typed or given, which may be the device's.
impl std::fmt::Debug for Confirmation<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Typed(_) => formatter.debug_tuple("Typed").field(&Redacted).finish(),
            Self::Shown(serial) => formatter.debug_tuple("Shown").field(serial).finish(),
            Self::Agent(_) => formatter.debug_tuple("Agent").field(&Redacted).finish(),
        }
    }
}

/// A device pairing: its commitment, the server's value, and the code once the device has revealed its own value.
struct PendingPairing {
    serial: u64,
    connection: ConnectionId,
    /// The pairing connection's outbox, told how the pairing ended. Told after the connection goes, it hears nothing.
    outbox: Arc<Outbox>,
    /// What pairs the device, which the server has, or it would not begin the pairing.
    setup: PairingSetup,
    /// The device, by what is left of the name it chose for itself, as every display of the pairing shows it. See `design/security.md`.
    device: ChosenName,
    /// The name the device chose, as it chose it, which names the token it receives once confirmed, as a token is named; never shown while it pairs.
    token_name: String,
    commitment: String,
    server: tomori_security::pairing::Nonce,
    code: Option<String>,
    started: Instant,
}

/// What pairing needs from the server: its certificate's fingerprint, which the code is derived from, and the token store, which issues the token. Without them, as in `--auth open` mode, the server does not pair.
#[derive(Clone)]
pub struct PairingSetup {
    pub fingerprint: tomori_security::Fingerprint,
    pub tokens: Arc<tomori_security::TokenStore>,
}

/// Something about pairing to show on the server's console, as its [`Display`](std::fmt::Display) says it. It never holds the pairing's code, since the console's output may be kept where others can read it, such as in a log: the operator confirms by typing the code the device shows. See `design/security.md`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairingEvent {
    /// A device began pairing.
    Began { device: ChosenName },
    /// A pairing waits to be confirmed: the device shows its code. It began at `began`, and is withdrawn at `expires` unless confirmed.
    Waiting {
        device: ChosenName,
        began: SystemTime,
        expires: SystemTime,
    },
    /// Pairings are paused, since too many began recently.
    Paused,
    /// A space's agent confirmed or declined a pairing through Tomori's tools, or was refused, as `notice` says, naming the space and the calling session.
    Agent { notice: String },
    /// `by` opened pairing for `length`, until `closes`.
    Opened {
        by: Opener,
        length: Duration,
        closes: SystemTime,
    },
    /// Pairing closed, as `closing` says why.
    Closed { closing: Closing },
    /// `count` requests to pair were refused since pairing is closed, since this was last told.
    Refused { count: u32 },
}

/// What the console tells the operator.
impl std::fmt::Display for PairingEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Began { device } => write!(formatter, "Pairing began from {device}."),
            Self::Waiting {
                device,
                began,
                expires,
            } => write!(
                formatter,
                "Pairing request from {device}, begun at {}; it is withdrawn at {} unless confirmed. To confirm it, type here the 6-digit code shown on the device you are pairing, never one from anywhere else, which declines it if it does not match, or type \"decline\"; owners can also confirm on the status page.",
                clock(*began),
                clock(*expires)
            ),
            Self::Paused => formatter.write_str(
                "Pairing is paused, since too many pairings began recently. Type \"allow\" to allow pairing again.",
            ),
            Self::Agent { notice } => formatter.write_str(notice),
            Self::Opened { by, length, closes } => write!(
                formatter,
                "{} opened pairing {} for {}, until {}: a device may ask to pair until then, and the first to pair closes it. Type \"close\" here to close it sooner.",
                by.who,
                by.place,
                pairing_window::spoken(*length),
                clock(*closes)
            ),
            Self::Closed { closing } => write!(formatter, "Pairing is closed{closing}"),
            Self::Refused { count: 1 } => {
                formatter.write_str("Refused a request to pair, since pairing is closed.")
            }
            Self::Refused { count } => write!(
                formatter,
                "Refused {count} requests to pair, since pairing is closed."
            ),
        }
    }
}

/// The time of day at `time`, in UTC, as the server's log gives it: `"03:12:05 UTC"`.
fn clock(time: SystemTime) -> String {
    let seconds = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    format!(
        "{:02}:{:02}:{:02} UTC",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// Shows [`PairingEvent`]s on the server's console.
pub type PairingPrompt = Box<dyn Fn(&PairingEvent) + Send + Sync>;

/// The most disconnected visitors remembered. Anyone can connect as a visitor with a new identity, so beyond this the longest gone are forgotten, rather than kept for the whole retention. See `design/spaces.md`.
pub const RETAINED_VISITORS: usize = 256;

/// Work for a space's driver, which owns the backend session.
#[derive(Debug)]
pub enum SpaceCommand {
    Send(Prompt),
    Decide {
        request: String,
        decision: tomori_backend::Decision,
        participant: String,
    },
    Answer {
        question: String,
        answers: Vec<(String, AnswerValue)>,
        participant: String,
    },
    /// Switch the session to `mode`, which the space offers. The space's mode changes when the agent confirms it.
    SetMode {
        mode: String,
        participant: String,
    },
    /// Give the agent the space's instructions as they are when the driver performs this, from [`Hub::instructions_due`]. At most one is queued for each space at a time. See `design/spaces.md`.
    Instructions,
    Close,
    /// The space was deleted: close its session, then drop the deletion, which does what is left to do with the session.
    Delete(Deletion),
}

/// What is left to do with a deleted space's session once nothing holds it any more: delete it, as its owner chose, or
/// else remove Tomori's instruction entries from it, as for a forgotten space. See `design/spaces.md`.
#[derive(Debug)]
pub struct Deleted {
    pub backend: String,
    pub session: String,
    /// The owner who asked for the session to be deleted too, by hub key, and their connection, which is told what
    /// became of it; `None` to keep the session.
    pub asker: Option<(String, ConnectionId)>,
}

/// A deleted space's [`Deleted`], handed to the supervisor when this is dropped: by the space's driver once it has
/// closed the session, or with the command that carried it once no driver is left to take it, as for a space whose
/// session had already closed. So the session is deleted, or its entries removed, only once the space's own handle to
/// it is closed, whatever the driver was doing when the space was deleted.
#[derive(Debug)]
pub struct Deletion {
    jobs: async_channel::Sender<Job>,
    deleted: Deleted,
}

impl Drop for Deletion {
    fn drop(&mut self) {
        let deleted = Deleted {
            backend: std::mem::take(&mut self.deleted.backend),
            session: std::mem::take(&mut self.deleted.session),
            asker: self.deleted.asker.take(),
        };
        // Once shutting down, the jobs are closed, and the session is kept.
        let _ = self.jobs.try_send(Job::Delete(deleted));
    }
}

/// A request to start a space, handled by the supervisor because starting a session is asynchronous.
#[derive(Debug)]
pub struct CreateRequest {
    pub title: String,
    pub backend: String,
    pub directory: String,
    pub session: Option<String>,
    /// The identifier of the offered environment the space is bound to.
    pub environment: String,
    /// The space's own instructions to the agent, empty for none.
    pub instructions: String,
    /// The participant who asked; they join the space once it exists, unless they have joined another space meanwhile.
    pub creator: String,
    /// The space the creator was in when they asked.
    pub creator_space: Option<String>,
}

/// A task for the supervisor, which runs outside the hub lock so it can call backends.
#[derive(Debug)]
pub enum Job {
    Create(CreateRequest),
    /// List the existing sessions of `backend` in `directory` for `participant`.
    ListSessions {
        participant: String,
        backend: String,
        directory: String,
    },
    /// Remove Tomori's instruction entries from `session` of `backend`, whose space was forgotten.
    ForgetInstructions {
        backend: String,
        session: String,
    },
    /// Finish deleting a space, as [`Deleted`] says.
    Delete(Deleted),
}

/// A space listed again after a restart, for the supervisor to bind to its session. See `design/spaces.md`.
#[derive(Debug)]
pub struct Restore {
    pub id: String,
    pub backend: String,
    pub directory: String,
    pub session: String,
    /// The space's commands, which wait until it is bound.
    pub commands: async_channel::Receiver<SpaceCommand>,
}

/// What a space listed again after a restart says until it is bound to its session.
pub const RESTORING: &str = "Reconnecting to the agent after the server restarted.";

/// Where the server side of one binding of a space to its environment sends its state, events and descriptions: the hub, which keeps them for the space and sends them to the clients in it that present the environment. See `design/environments.md`.
struct SpaceOutlet {
    hub: Weak<Hub>,
    space: String,
    binding: u64,
    environment: String,
    version: u32,
}

pub struct Hub {
    state: Mutex<HubState>,
    pub(crate) backends: Backends,
    transcriber: Arc<dyn Transcriber>,
    jobs: async_channel::Sender<Job>,
    /// Spaces whose sessions are starting or running, against the limit.
    slots: Arc<SpaceSlots>,
    /// The directories a space may run in, sent to clients in `Welcome` so they can choose one without typing a path.
    roots: RwLock<Vec<WorkspaceRoot>>,
    /// The environments a space may be bound to, sent to clients in `Welcome`. See `design/environments.md`.
    environments: RwLock<Environments>,
    /// The server sides of the environments, run for the spaces bound to them. See `design/environments.md`.
    server_sides: Arc<crate::server_sides::ServerSides>,
    /// Notified when a space defers a snapshot, or an agent's next move is due sooner, so that the server's snapshot
    /// timer can wake for it.
    deferred_snapshot: Arc<Notify>,
    /// Notified when [`Hub::counts`] changes, so that the server can advertise the new counts.
    counts_changed: Notify,
    /// Notified when someone starts watching the cards, so that the server reads them if they are due. See `design/github.md`.
    cards_watched: Notify,
    /// The most visitors connected at once; see `design/security.md`.
    max_visitors: AtomicUsize,
    /// Issues collaborator tokens to admitted visitors; see [`Hub::set_tokens`].
    tokens: RwLock<Option<Arc<tomori_security::TokenStore>>>,
    /// Where the spaces are kept across restarts; see [`Hub::restore`].
    kept: RwLock<Option<Arc<crate::kept::SpaceStore>>>,
    /// The images of image panels, by their SHA-256, in the state directory once spaces are kept there. See
    /// `design/projection.md`.
    pub(crate) images: Arc<crate::images::ImageStore>,
    /// Whether the server serves only TLS, which pairing needs: the token it issues must not cross the network in the clear.
    encrypted: AtomicBool,
    pairing: RwLock<Option<PairingSetup>>,
    pairing_prompt: RwLock<Option<PairingPrompt>>,
    pairing_timeout: RwLock<Duration>,
    /// How long pairing stays open once opened; see [`PAIRING_OPEN_FOR`].
    pairing_open_for: RwLock<Duration>,
    knock_timeout: RwLock<Duration>,
    /// How long the agent waits for an environment's server side to answer an inspection; see [`agent_inspect::INSPECT_TIMEOUT`].
    inspect_timeout: RwLock<Duration>,
    /// How long the agent waits for an environment's server side to answer an action; see [`agent_actions::ACT_TIMEOUT`].
    act_timeout: RwLock<Duration>,
    /// Tomori's MCP server and its registrations with backends; see `design/mcp.md`.
    tools: tools::Tools,
    /// What highlights the agent's code panels, outside the lock; see [`crate::highlighting`].
    highlighter: RwLock<crate::highlighting::Highlighter>,
    /// The time a test sets for the agent's timers, in place of the clock's. See `design/timers.md`.
    clock: RwLock<Option<jiff::Timestamp>>,
    /// The zone a timer's times of day are in unless the agent names one: this machine's. See `design/timers.md`.
    zone: RwLock<String>,
    this: Weak<Hub>,
}

#[derive(Default)]
struct HubState {
    /// The environments' server sides, the hub's own, which are told where their panels go.
    sides: Arc<crate::server_sides::ServerSides>,
    /// Set while a participant's message is taken, so that where it moves an environment's panels is told to its side
    /// once, as theirs, when it is done, rather than as the layout's at each change.
    acting: Cell<bool>,
    next_connection: ConnectionId,
    next_source: u32,
    /// Random for each server run and part of every space identifier, so a client restoring a space from an earlier run never joins an unrelated space that reused its number.
    run: String,
    next_space: u64,
    next_utterance: u64,
    next_ping: u64,
    /// Orders closed spaces by when they closed; see [`SpaceRecord::closed`].
    next_closed: u64,
    /// Orders disconnections; see [`ParticipantRecord::departure`].
    next_departure: u64,
    connections: HashMap<ConnectionId, ConnectionRecord>,
    participants: HashMap<String, ParticipantRecord>,
    /// In creation order.
    spaces: Vec<SpaceRecord>,
    utterances: HashMap<UtteranceId, UtteranceRecord>,
    /// Set once shutdown has begun: no further spaces are registered.
    shutting_down: bool,
    /// Whether the server is serving, which is when environments' server sides may run: not while it is still being configured, as when kept spaces are listed again.
    serving: bool,
    /// Set when a space was forgotten, so that the kept spaces are written once the lock is released.
    unkept: bool,
    /// The backends and sessions of spaces forgotten since the supervisor was last asked to remove their instruction entries; see [`Hub::recount`].
    forgotten: Vec<(String, String)>,
    /// The one pairing pending, if any; see `design/security.md`.
    pairing: Option<PendingPairing>,
    next_pairing: u64,
    /// When recent pairings began, for [`PAIRING_ATTEMPTS`].
    pairing_attempts: std::collections::VecDeque<Instant>,
    /// What spaces' agents recently did with pairings, newest last, for the status page; at most [`PAIRING_DECISIONS`].
    pairing_decisions: std::collections::VecDeque<String>,
    /// The window in which a device may ask to pair, while one is open; pairing is closed otherwise, also on a server with no owner yet. See `design/security.md`.
    window: Option<pairing_window::Window>,
    /// Requests to pair refused while pairing was closed, not yet told.
    refusals: pairing_window::Refusals,
    /// Knocks waiting for an answer, oldest first.
    knocks: Vec<KnockRecord>,
    next_knock: u64,
    /// When a visitor, by hub key, may next knock on a space, after a knock there was declined or expired.
    knock_cooldowns: HashMap<(String, String), Instant>,
    /// When each identity was last renamed through its space's agent, for [`agent_names::RENAME_LIMIT`].
    renames: agent_names::Renames,
    /// The counts when they were last compared, to notice a change.
    counts: Counts,
    /// The cards the server shows in their environment's spaces, if it shows any. See `design/github.md`.
    board: Option<crate::cards::Board>,
    /// Notified when an agent's next move is due sooner, so that the server's snapshot timer wakes for it: the hub's
    /// own. See `design/environments.md`.
    woken: Arc<Notify>,
}

struct ConnectionRecord {
    outbox: Arc<Outbox>,
    /// What the connection was admitted as at the door.
    access: Access,
    /// The role the client was last told, so that it is told again only when it changes.
    role: Option<ConnectionRole>,
    /// Whether the client said in Hello that it reads [`ServerMessage::Name`].
    names: bool,
    /// The environments the client listed in Hello, at most [`tomori_protocol::ENVIRONMENT_LIMIT`], or `None` when it listed none, as one from before versions. See `design/environments.md`.
    presents: Option<Vec<EnvironmentVersion>>,
    participant: Option<String>,
    utterance: Option<OpenUtterance>,
    /// The client's id for the utterance the server last closed by failing it, so that an end or cancel already on its way is ignored rather than refused.
    failed: Option<String>,
    /// Push-to-talk frames that arrived while no utterance was open, which are relayed but not transcribed.
    stray_frames: u64,
    /// Whether the last push-to-talk frame was stray, so that each run of them is logged once.
    straying: bool,
    liveness: Liveness,
    /// Whether the client watches the cards of the space it is in, until it leaves it. See `design/github.md`.
    cards: bool,
    /// Whether the client said in Hello that it reads environments' state and events. See `design/environments.md`.
    environment_state: bool,
    /// Whether the client said in Hello that it reads what is projected in its space. See `design/projection.md`.
    projection: bool,
    /// Whether the client said in Hello that it reads [`ServerMessage::PairingWindow`]. See `design/security.md`.
    pairing_window: bool,
    /// Whether the client said in Hello that it reads [`ServerMessage::SpeechHeld`]. See `design/speech.md`.
    speech_held: bool,
    /// Whether the client said in Hello that it shows image panels, fetching their images. See `design/projection.md`.
    images: bool,
    /// Whether the client said in Hello that it draws volumes, sent their diagrams within its limits. See
    /// `design/diagrams.md`.
    diagrams: bool,
    /// Whether the client said in Hello that it draws volumes' world edges, sent their targets with them. See
    /// `design/diagrams.md`.
    world_edges: bool,
    /// Whether the client said in Hello that it reads what is projected and applies patches to it. See
    /// `design/projection.md`.
    patches: bool,
    /// Whether the client said in Hello that it also applies a patch's data ops to live panels and keeps the version a
    /// place op carries. See `design/projection.md`.
    live_panels: bool,
    /// What the connection is bound by: the lesser of what the client said in Hello that it takes at most, today's from an
    /// older one, and [`LIMITS`]. See `design/projection.md`.
    limits: Limits,
    /// The projection this client was last sent, as it was sent, at its revision, which the next is sent as a patch
    /// from, for a client that takes patches: `None` until it is sent one, and once it asks for one whole again or
    /// joins another space. See `design/projection.md`.
    projected: Option<Arc<tomori_protocol::Projection>>,
    /// The revision of its space's projection it was last sent at, whole or patched, or 0 once it joins a space, so
    /// that a change told late, at the end of its interval, is not sent again to a connection already sent it.
    projection_sent: u64,
}

impl ConnectionRecord {
    /// Tell the connection its role in `space`, or in the lobby, if it changed. Returns the role.
    fn update_role(&mut self, space: Option<&str>) -> ConnectionRole {
        let role = self.access.role(space);
        if self.role != Some(role) {
            self.role = Some(role);
            self.outbox.message(ServerMessage::Role { role });
        }
        role
    }
}

/// Whether the HTTP server is writing a connection's output, shared by the connection body and the keepalive sweep, and
/// the connection's frame limit, which the hub sets from its Hello and the body reads and writes frames within.
#[derive(Default)]
pub struct Output {
    /// True from when the body hands out a chunk until the HTTP server asks for the next one, which it does once the chunk is written.
    pending: AtomicBool,
    /// Chunks written so far.
    written: AtomicU64,
    /// The longest frame's payload the server sends, as negotiated from the client's Hello: 0, read as
    /// [`tomori_protocol::MAX_FRAME_BYTES`], until it says Hello. See `design/server.md`.
    frame_bytes: AtomicUsize,
    /// Whether what the client sends is taken as an owner's or a collaborator's, [`Inbound::Participating`], from
    /// when it says Hello, or a knock admits it; until then, and for a visitor or a connection admitted only to pair,
    /// [`Inbound::Watching`]. See `design/security.md`.
    participating: AtomicBool,
}

impl Output {
    /// The longest frame's payload the server sends on this connection: [`tomori_protocol::MAX_FRAME_BYTES`] until the
    /// client says Hello, and then the lesser of what it and this server state. What the server takes from the client
    /// is [`Self::inbound`]'s, however long this is.
    pub fn frame_limit(&self) -> usize {
        tomori_protocol::frame_limit(self.frame_bytes.load(Ordering::Acquire))
    }

    fn set_frame_limit(&self, limit: usize) {
        self.frame_bytes.store(limit, Ordering::Release);
    }

    /// What the client's frames are taken as: how long they may be, and how often they may come.
    pub fn inbound(&self) -> Inbound {
        if self.participating.load(Ordering::Acquire) {
            Inbound::Participating
        } else {
            Inbound::Watching
        }
    }

    fn set_inbound(&self, inbound: Inbound) {
        self.participating
            .store(inbound == Inbound::Participating, Ordering::Release);
    }

    /// The body handed out a chunk to write.
    pub fn handed_out(&self) {
        self.pending.store(true, Ordering::Release);
    }

    /// The HTTP server asked for more, so the previous chunk, if any, was written.
    pub fn asked(&self) {
        if self.pending.swap(false, Ordering::AcqRel) {
            self.written.fetch_add(1, Ordering::AcqRel);
        }
    }
}

/// What the keepalive sweep knows about one connection.
struct Liveness {
    /// Set by the connection whenever data arrives, and cleared by the sweep.
    received: Arc<AtomicBool>,
    /// When the sweep last saw data arrive, or first saw the connection.
    last: Option<Instant>,
    /// When the sweep first saw the connection, until it says Hello.
    opened: Option<Instant>,
    greeted: bool,
    pinged: Option<Instant>,
    output: Arc<Output>,
    /// Chunks written when the sweep last looked.
    written: u64,
    /// When the sweep first saw the current write pending without progress.
    stalled: Option<Instant>,
    /// Ends this connection's HTTP exchange, including a write blocked on a peer that is gone.
    cancellation: Cancellation,
}

struct OpenUtterance {
    client: String,
    id: UtteranceId,
    /// The sequence number of the next frame, after an agent frame of this utterance. A gap between two agent frames means agent audio was lost mid-utterance; see `design/speech.md`. A frame of another gate clears it, so the agent frame after it is not checked.
    ///
    /// The first frame is not checked against frames before `UtteranceBegin`: the client legitimately skips sequence numbers there, by not sending frames while the gate is off and by dropping congested live audio. It never drops frames of an open utterance, and frames and control share one ordered stream, so the first frame after `UtteranceBegin` is the utterance's first.
    expected: Option<u32>,
}

struct ParticipantRecord {
    /// Admitted as a visitor, so forgotten sooner when many have come and gone; see [`RETAINED_VISITORS`].
    visitor: bool,
    /// The order in which participants last disconnected, so that the longest gone are forgotten first.
    departure: u64,
    /// The principal they authenticated as, whose token keeps their name. See `design/security.md`.
    principal: Option<String>,
    name: String,
    /// The number that tells them apart from others of the same name, as in `Ada#23`: their token's, kept with it, or,
    /// without one, one no token or other participant has, kept while the server keeps them. See `design/security.md`.
    tag: Option<u32>,
    source: u32,
    connection: Option<ConnectionId>,
    space: Option<String>,
    /// The space to rejoin when this participant reconnects.
    last_space: Option<String>,
    /// When the participant's last connection ended, while it has none.
    departed: Option<Instant>,
    /// What the participant's current connection last reported, if anything. See `design/spaces.md`.
    engagement: Option<Engagement>,
    /// The environment intents they may still send, kept across their connections, so that reconnecting does not refill the burst. See `design/environments.md`.
    intents: crate::channel::Intents,
    /// Where they last were in the space they are in, from their presence: their head, which decides the wall the agent
    /// projects onto in answer to them, and their controllers and hands, which its `presence` tool reports. See
    /// `design/projection.md` and `design/mcp.md`.
    seen: Option<crate::presence::Seen>,
    /// Whether their current connection has reported a controller or a hand, as only a headset does.
    headset: bool,
}

/// Where a space's environment's panels were last told to its side to be, by the environment's ids, for the
/// environment named and its side's binding, and the participant whose moves past their rate are yet to be told, if any.
#[derive(Default)]
struct Told {
    environment: String,
    binding: Option<u64>,
    places: std::collections::BTreeMap<String, tomori_environment::PanelPlace>,
    owed: Option<u32>,
    /// How each of the environment's panels was last told it is bound to its context, by the environment's id for it;
    /// one not here follows everything. See `design/panel-context.md`.
    contexts: std::collections::BTreeMap<String, tomori_environment::Context>,
}

struct SpaceRecord {
    model: SpaceModel,
    commands: async_channel::Sender<SpaceCommand>,
    /// When participants were last told that the space changed. Cells, because telling them only needs shared access to the state.
    touched: Cell<Option<Instant>>,
    /// When participants are due to be told of a change caused by agent events, deferred to the end of the [`SNAPSHOT_INTERVAL`] since they were last told.
    deferred: Cell<Option<Instant>>,
    /// Once the space has closed, when relative to other spaces, so that the least recently closed are forgotten first.
    closed: Option<u64>,
    /// Closed, but kept as open, and not forgotten, so that it is bound again when the server next starts: closed by shutdown, or not bound again after a restart for a reason that may pass. See `design/spaces.md`.
    kept_open: bool,
    /// The attention the space was last listed as needing, and since when, so the listing can say how long it has
    /// waited. Stamped after each agent event and whenever participants are told the space changed.
    attention: Cell<Option<(Attention, Instant)>>,
    /// The space's agent speech, kept for those who missed it. See `design/speech.md`.
    speech: Retention,
    /// Wakes the space's speech pacer, so that it paces a replay or catch-up that just started.
    waker: Option<Box<dyn Fn() + Send>>,
    /// While an instructions command is queued for the driver, whether it gives every layer rather than the space's own alone, so that changes made meanwhile join it rather than queue without bound.
    instructing: Cell<Option<bool>>,
    /// The keys and texts of the latest spoken answers applied, so that a repeated finalization of one is acknowledged rather than answered again; see [`ANSWERED`].
    answered: std::collections::VecDeque<(String, String)>,
    /// Where the space's participants put its cards, by card. See `design/github.md`.
    placements: std::collections::BTreeMap<String, tomori_protocol::Pose>,
    /// Listed again after a restart and not yet bound to its session, so its directory has not yet been resolved within the workspace roots again: its environment's server side waits until it is. See `design/environments.md`.
    restoring: bool,
    /// What the server side of the space's environment keeps for it, for its current binding only. See `design/environments.md`.
    channel: Option<crate::channel::Channel>,
    /// How many more actions the space's agent may take on what its environment shows, once it has taken one, kept
    /// with the space rather than its binding, so that changing environment refills nothing. See
    /// `design/environment-actions.md`.
    acts: Option<crate::channel::Acts>,
    /// The binding of the space's environment whose server side has stopped, failed or finished, which changes no
    /// panels from then on, any thread it left running included, until a side is run again for a new binding.
    stopped: Option<u64>,
    /// The space's projection walls and the panels on them. See `design/projection.md`.
    stage: crate::stage::Stage,
    /// The space's projection revision, raised with each change to what it projects, which patches are sent from and
    /// to, and each wall's and panel's version names; for this run only, since a client sent one connects again, and is
    /// sent the projection whole, after a restart, but begun from the time, so that no version a run gives is one an
    /// earlier run gave (see [`crate::versions::first_revision`]). A cell, because raising it only needs shared access
    /// to the state. See `design/projection.md`.
    projection_revision: Cell<u64>,
    /// Where its environment's panels were last told to its side to be. See [`HubState::tell_places`].
    told: std::cell::RefCell<Told>,
    /// The versions of its walls and panels, found from each projection built. See [`SpaceRecord::versioned`].
    versions: std::cell::RefCell<crate::versions::Versions>,
    /// When its connections were last told of a change its environment's side made to what it projects, and when they
    /// are due to be told of one made within [`PROJECTION_INTERVAL`] of that, deferred to the interval's end.
    projection_told: Cell<Option<Instant>>,
    projection_deferred: Cell<Option<Instant>>,
    /// What it projects as last built, for each kind of connection sent it, kept while it is at the same revision in
    /// the same second, so that every connection told of one change shares one build. See [`SpaceRecord::built`].
    built: std::cell::RefCell<Option<Built>>,
    /// Where a participant or the agent summoned the agent to, in the space's coordinates, while it is not at its home:
    /// for this run and this environment only, so the agent is at home whenever the space opens. See
    /// `design/environments.md`.
    summoned: Option<tomori_protocol::Pose>,
    /// While the agent is away, the eyes of whoever it stands for, as it was placed from them, so that clients draw the
    /// swarm in near them. See `design/avatars.md`.
    partner: Option<[f32; 3]>,
    /// How the agent follows whoever it is talking with, and whether it is pinned, for this run and this environment
    /// only. See `design/environments.md`.
    following: following::Following,
    /// Where whatever stands at the space's cluster anchors stands: each member's slot, and the span each cluster is
    /// laid out for, seated afresh as it is sent, for this run only. See `design/environments.md`.
    seating: std::cell::RefCell<crate::seating::Seating>,
    /// Where its workers were placed, at the anchors of each environment, by environment, kept with the space.
    places: crate::places::Kept,
    /// When the space was last bound to its environment, or its side began a binding, from which a placement at an
    /// anchor its environment's side has not placed again waits [`crate::places::GRACE`].
    anchored: Instant,
    /// The binding its environment's side last ran for, if one did, so that a side begun anew, as the space reopens,
    /// starts the grace again.
    bound: Option<u64>,
    /// The anchor the agent went to of its own accord, by name, and where it stands there, while it still stands there.
    agent_at: Option<(String, tomori_protocol::Pose)>,
    /// How often the agent has placed workers and moved itself of its own accord, for this run only.
    rates: crate::places::Rates,
    /// Who is within the space's anchors that notify its agent, and when each was last told, for this run only. See
    /// `design/environments.md`.
    proximity: crate::proximity::Proximity,
    /// When the space was last active, in seconds since the Unix epoch, to the [`ACTIVE_RESOLUTION`]: when it was
    /// created, its agent last worked in it, or input was last queued in it. Kept across a restart. See
    /// `design/arrival.md`.
    active: Option<u64>,
}

/// How finely when a space was last active is kept, in seconds, so that a busy space's record of spaces is written at
/// most once a minute for it.
const ACTIVE_RESOLUTION: u64 = 60;

/// How many spoken answers each space remembers, to acknowledge a repeated finalization. A client repeats only its latest utterance, after reconnecting.
const ANSWERED: usize = 16;

/// The most kinds of connection, by the limits they state, whose projection a space keeps built: Tomori's own state
/// two, the headset's and the desktop window's. One beyond them is built for itself each time.
const BUILT: usize = 8;

/// What a space projects as it was last built: at its `revision`, at `now`, in seconds since the epoch, since each
/// panel's age is counted from then; whole, and within each of the `limits` a connection sent it stated.
struct Built {
    revision: u64,
    now: u64,
    whole: Arc<tomori_protocol::Projection>,
    within: Vec<(Limits, Arc<tomori_protocol::Projection>)>,
}

/// When participants are told of a change to a space caused by an agent event; see [`SNAPSHOT_INTERVAL`].
enum Pace {
    Now,
    /// At the end of the interval, as no snapshot was deferred yet.
    Deferred,
    /// At the end of the interval, with the snapshot already deferred.
    AlreadyDeferred,
}

impl SpaceRecord {
    /// What the space projects in its environment, among `environments`, at `now`, as clients are sent it: with each
    /// wall's and panel's version, the projection revision at which it last changed, and each live panel's data seq,
    /// found by comparing it with the last built, so that a change a participant or the agent makes against what they
    /// saw is checked against what is shown now. See `design/projection.md`.
    fn versioned(
        &self,
        environments: &crate::environments::Environments,
        now: u64,
    ) -> tomori_protocol::Projection {
        let walls = environments.walls(&self.model.environment);
        let mut projection = self.stage.view(&self.model.id, &walls, now);
        self.versions
            .borrow_mut()
            .stamp(&mut projection, &self.projection_revision);
        projection
    }

    /// What the space projects at its revision now, at `now`, as [`SpaceRecord::versioned`] finds it, saying that
    /// revision, as a connection that takes at most `limits` is sent it: within them, each wall and panel at the version
    /// it has in the whole, so that every client is sent one version of each, and one sent less is never sent a wall or
    /// panel it was not sent before as changed. It is built once for every connection of the same limits told of the
    /// same change, and kept only while the space is at that revision in that second, so that a change, which raises
    /// the revision, and each panel's age are always followed. See `design/projection.md`.
    fn built(
        &self,
        environments: &crate::environments::Environments,
        now: u64,
        limits: &Limits,
    ) -> Arc<tomori_protocol::Projection> {
        let revision = self.projection_revision.get();
        let mut built = self.built.borrow_mut();
        let current = built
            .take()
            .filter(|built| built.revision == revision && built.now == now);
        let built = built.insert(current.unwrap_or_else(|| Built {
            revision,
            now,
            whole: Arc::new(self.versioned(environments, now).with_revision(revision)),
            within: Vec::new(),
        }));
        if *limits == Limits::MOST {
            return built.whole.clone();
        }
        if let Some((_, projection)) = built.within.iter().find(|(kept, _)| kept == limits) {
            return projection.clone();
        }
        let walls = environments.walls(&self.model.environment);
        let mut projection = self.stage.view_within(&self.model.id, &walls, now, limits);
        crate::versions::Versions::label(&built.whole, &mut projection);
        let projection = Arc::new(projection.with_revision(revision));
        if built.within.len() < BUILT {
            built.within.push((*limits, projection.clone()));
        }
        projection
    }

    /// When participants are told of a change caused by an agent event at `now`: at once, unless they were told within [`SNAPSHOT_INTERVAL`], in which case it is deferred to the interval's end.
    fn pace(&self, now: Instant) -> Pace {
        if self.deferred.get().is_some() {
            return Pace::AlreadyDeferred;
        }
        match self
            .touched
            .get()
            .map(|touched| touched + SNAPSHOT_INTERVAL)
            .filter(|due| now < *due)
        {
            Some(due) => {
                self.deferred.set(Some(due));
                Pace::Deferred
            }
            None => Pace::Now,
        }
    }

    /// Note when the space's attention changed level, at `now`.
    fn stamp_attention(&self, now: Instant) {
        let level = self.model.attention();
        if self.attention.get().map(|(stamped, _)| stamped) != level {
            self.attention.set(level.map(|level| (level, now)));
        }
    }

    /// The space as listed in the lobby at `now`, with how long it has needed its attention.
    /// The space was active at `unix`, in seconds since the Unix epoch. Returns whether that changed when it is kept as
    /// last active, and so the record of spaces is to be written again.
    fn stamp_active(&mut self, unix: u64) -> bool {
        let active = Some(unix - unix % ACTIVE_RESOLUTION);
        let changed = self.active != active;
        self.active = active;
        changed
    }

    /// The space as the lobby lists it at `now`, which is `unix` in seconds since the Unix epoch, with its folder
    /// beneath one of `roots`.
    fn summary(
        &self,
        now: Instant,
        unix: u64,
        roots: &[WorkspaceRoot],
    ) -> tomori_protocol::SpaceSummary {
        let mut summary = self.model.summary();
        summary.folder = Some(crate::workspace::folder(roots, &self.model.directory));
        summary.active_age = self
            .active
            .map(|active| u32::try_from(unix.saturating_sub(active)).unwrap_or(u32::MAX));
        summary.attention_age = summary.attention.map(|level| match self.attention.get() {
            Some((stamped, since)) if stamped == level => {
                u32::try_from(now.saturating_duration_since(since).as_secs()).unwrap_or(u32::MAX)
            }
            // Changed since it was last stamped, which happens when participants are next told.
            _ => 0,
        });
        summary
    }

    /// Give the agent the space's instructions, every layer if `all` and otherwise the space's own, before any input queued after this. A command already queued gives them as they are when it is performed, so it is widened rather than another queued. A driver that has stopped needs none.
    fn instruct(&self, all: bool) {
        if !self.model.takes_instructions() || self.model.is_closed() {
            return;
        }
        if let Some(queued) = self.instructing.get() {
            self.instructing.set(Some(queued || all));
            return;
        }
        self.instructing.set(Some(all));
        if self.commands.try_send(SpaceCommand::Instructions).is_err() {
            self.instructing.set(None);
        }
    }

    /// Wake the space's speech pacer, if it has one, to pace a replay or catch-up that just started.
    fn wake(&self) {
        if let Some(waker) = &self.waker {
            waker();
        }
    }
}

/// An utterance whose transcript has not been reported yet.
struct UtteranceRecord {
    participant: String,
    /// The participant's number, their public handle, which keys their queued input.
    source: u32,
    client: String,
    space: String,
    /// The question and field the transcript answers, rather than being queued.
    answer: Option<tomori_protocol::AnswerTarget>,
    /// Whether the transcript is the participant's new display name, rather than being queued or answering.
    naming: bool,
    /// Whether the transcript is the space's new title, rather than being queued or answering. See `design/hand-menu.md`.
    titling: bool,
    /// What the participant attended to as the utterance began, as the space knew it then. Their client names only
    /// identities. See `design/spaces.md`.
    attention: Vec<crate::space::Attended>,
    /// The environment's objects, by identity, that the participant pointed at as the utterance began, which rank first
    /// in the agent's ambient list for their words once those are queued, and not before. See [`crate::ambient`].
    pointed: Vec<String>,
    /// What the participant had seen of the agent's messages as the utterance began, as the space knew it then, for
    /// words for the agent. Their client names only the message. See `design/interaction.md`.
    seen: Option<crate::space::Seen>,
    /// Where the participant's head was, and the way it faced, as the utterance began, so that what the agent projects
    /// in answer goes where they were looking, or onto the wall nearest them. See `design/projection.md`.
    origin: Option<tomori_protocol::Pose>,
    /// The connection whose client asked for [`ServerMessage::TranscriptProgress`] as it began the utterance, if it did. Progress is sent only there, as the participant may reconnect with an older client, which would refuse it.
    progress: Option<ConnectionId>,
    /// When it began, after which it holds Tomori's notes back for at most [`crate::events::HOLD`]. See
    /// `design/timers.md`.
    begun: Instant,
    /// When its end arrived, and with it the last of its audio, from which the time it takes to transcribe is logged.
    ended: Option<Instant>,
    /// Since when it has waited behind another utterance, if it waits now, and how long it waited before.
    waiting: Option<Instant>,
    waited: Duration,
}

impl UtteranceRecord {
    /// Who recorded it, and in which space: what [`HubState::recorded`] needs once it is no longer recorded.
    fn recorder(self) -> (String, String) {
        (self.participant, self.space)
    }

    /// Log how long the utterance took to transcribe after its end arrived, and how much of that it waited, once it has `outcome`, so that the time can be seen per utterance. Nothing is logged for one that failed while it was spoken.
    fn log_outcome(&self, id: UtteranceId, outcome: &str) {
        if let Some(ended) = self.ended {
            let now = Instant::now();
            let waited = self.waited
                + self
                    .waiting
                    .map_or(Duration::ZERO, |since| now.saturating_duration_since(since));
            log::info!(
                "Utterance {} from participant {} {outcome} {:.2} s after its end arrived, having waited {:.2} s for the recognizer.",
                id.0,
                self.source,
                now.saturating_duration_since(ended).as_secs_f64(),
                waited.as_secs_f64()
            );
        }
    }
}

/// A transcriber call to make once the lock is released.
enum Deferred {
    Begin(UtteranceId),
    Push(UtteranceId, AudioFrame),
    End(UtteranceId),
    Cancel(UtteranceId),
    /// Show something about pairing on the console.
    PairingPrompt(PairingEvent),
    /// Write the kept spaces, if they changed.
    Keep,
    /// Issue a collaborator token for an admitted knock, outside the lock, and admit its visitor.
    Admit(KnockRecord),
    /// Keep a principal's new name with their token, outside the lock, and then call everyone holding it by it.
    Rename(Rename),
    /// Send the image with this SHA-256 to a connection that asked for it, read outside the lock, as it may be read from
    /// the state directory.
    Image(ConnectionId, [u8; 32]),
}

/// A token holder's new name, to keep with their token. See [`Hub::keep_name`].
struct Rename {
    tokens: Arc<tomori_security::TokenStore>,
    principal: String,
    name: String,
    /// The participant who asked, by hub key, and the utterance that said the name, if they spoke it, to answer once it is kept. A name kept on their behalf, as when their token had none, answers nobody.
    asked: Option<(String, Option<String>)>,
}

/// Why a token holder's new name was not kept with their token.
enum Unkept {
    /// Their token is no longer issued.
    Revoked,
    /// The token store could not be written.
    Unwritten,
}

/// A name said aloud, as recognition writes it, without the full stop or other mark it may end with: "Ada." is Ada.
fn spoken_name(text: &str) -> &str {
    text.trim()
        .trim_end_matches(['.', '!', '?', ',', ';', ':'])
        .trim_end()
}

/// A token for this server run, so space identifiers are not reused after a restart: 64 random bits as hexadecimal. `RandomState` is seeded from the operating system's random source.
fn run_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    format!("{:016x}", hasher.finish())
}

/// The key under which the hub keeps a participant: the identity they said in Hello, together with the principal they authenticated as, if any. The same identity without that principal is a different participant, so it cannot take over an authenticated participant's connection, space or audio source. See `design/security.md`.
///
/// The principal's length comes first, so that no principal and identity join to another's key; an unauthenticated key starts with `/`, which a length cannot.
fn participant_key(principal: Option<&str>, identity: &str) -> String {
    match principal {
        Some(principal) => format!("{}:{principal}/{identity}", principal.len()),
        None => format!("/{identity}"),
    }
}

/// An error reply, cut to [`limits::ERROR_BYTES`].
/// The folder of the state directory that image panels' images are kept in. See `design/projection.md`.
pub(crate) const IMAGES_FOLDER: &str = "images";

fn error_message(error: impl Into<String>) -> ServerMessage {
    ServerMessage::Error {
        error: limits::truncate(&error.into(), limits::ERROR_BYTES),
        space: None,
        panel: None,
        wall: None,
    }
}

impl HubState {
    /// The spaces listed, which includes closed spaces until they are forgotten, and the greeted owners and collaborators with a connection, whether in a space or in the lobby. Visitors are not counted. A space still being created is not listed, so it is not counted.
    fn counts(&self) -> Counts {
        let count = |count: usize| u32::try_from(count).unwrap_or(u32::MAX);
        let needing = |level: Attention| {
            count(
                self.spaces
                    .iter()
                    .filter(|space| space.model.attention() == Some(level))
                    .count(),
            )
        };
        Counts {
            spaces: count(self.spaces.len()),
            participants: count(
                self.connections
                    .values()
                    .filter(|record| record.participant.is_some() && !record.access.is_visitor())
                    .count(),
            ),
            attention: Some(AttentionCounts {
                waiting: needing(Attention::Input),
                problems: needing(Attention::Problem),
                replies: needing(Attention::Reply),
            }),
        }
    }

    /// Tell a participant's connection its role if it changed, as they join, are admitted or leave, which they do connected. Returns the role, or `None` for a participant without a connection, who has no role to be told.
    fn update_role(&mut self, participant: &str) -> Option<ConnectionRole> {
        let Self {
            participants,
            connections,
            ..
        } = self;
        participants.get(participant).and_then(|record| {
            record
                .connection
                .and_then(|id| connections.get_mut(&id))
                .map(|connection| connection.update_role(record.space.as_deref()))
        })
    }

    /// The role of a participant's connection where they are now.
    fn role(&self, connection: ConnectionId, space: Option<&str>) -> ConnectionRole {
        self.connections
            .get(&connection)
            .map_or(ConnectionRole::Visitor, |record| record.access.role(space))
    }

    fn outbox(&self, participant: &str) -> Option<&Arc<Outbox>> {
        let connection = self.participants.get(participant)?.connection?;
        Some(&self.connections.get(&connection)?.outbox)
    }

    fn send(&self, participant: &str, message: ServerMessage) {
        if let Some(outbox) = self.outbox(participant) {
            outbox.message(message);
        }
    }

    fn error(&self, participant: &str, error: impl Into<String>) {
        self.send(participant, error_message(error));
    }

    /// The outboxes of everyone in `space` whose client reads environments' state and presents the space's environment at the version its side runs for, participants and visitors alike. See `design/environments.md`.
    fn channel_outboxes(
        &self,
        record: &SpaceRecord,
        channel: &crate::channel::Channel,
    ) -> Vec<Arc<Outbox>> {
        let presented = EnvironmentVersion::new(channel.environment.clone(), channel.version);
        record
            .model
            .members()
            .filter_map(|member| self.participants.get(member))
            .filter_map(|participant| participant.connection)
            .filter_map(|connection| self.connections.get(&connection))
            .filter(|connection| {
                connection.environment_state
                    && connection
                        .presents
                        .as_ref()
                        .is_some_and(|presents| presents.contains(&presented))
            })
            .map(|connection| connection.outbox.clone())
            .collect()
    }

    /// The environments `connection` listed in Hello, or `None` when it listed none.
    fn presents(&self, connection: ConnectionId) -> Option<&[EnvironmentVersion]> {
        self.connections
            .get(&connection)
            .and_then(|record| record.presents.as_deref())
    }

    fn space(&self, id: &str) -> Option<&SpaceRecord> {
        self.spaces.iter().find(|space| space.model.id == id)
    }

    fn space_mut(&mut self, id: &str) -> Option<&mut SpaceRecord> {
        self.spaces.iter_mut().find(|space| space.model.id == id)
    }

    /// Whether the space `id` shows the board's cards.
    fn shows_cards(&self, id: &str) -> bool {
        self.space(id)
            .zip(self.board.as_ref())
            .is_some_and(|(space, board)| space.model.environment == board.environment())
    }

    /// The board, if the space `id` shows its cards.
    fn cards_of(&self, id: &str) -> Option<&crate::cards::Board> {
        self.board.as_ref().filter(|_| self.shows_cards(id))
    }

    /// Send the cards again to everyone watching them in `space`, or in every space that shows them.
    fn touch_cards(&self, space: Option<&str>) {
        for record in self.spaces.iter().filter(|record| {
            space.is_none_or(|space| record.model.id == space) && self.shows_cards(&record.model.id)
        }) {
            for member in record.model.members() {
                if let Some(connection) = self
                    .participants
                    .get(member)
                    .and_then(|participant| participant.connection)
                    .and_then(|connection| self.connections.get(&connection))
                    .filter(|connection| connection.cards)
                {
                    connection.outbox.cards();
                }
            }
        }
    }

    /// Send what is projected in `space` again to everyone in it whose client reads it, participants and visitors alike.
    /// See `design/projection.md`.
    /// What has changed of where `record`'s environment's panels are since its side, bound as `binding`, was last told: each that is on
    /// another wall, floating, or, new to what was told, not on the wall it asked for, as where it rested, by the
    /// environment's id, with where it is now; and each gone that stays away, dismissed by a participant or given up by
    /// the layout, as dismissed; never one moved between the slots of one wall, nor one retracted or resting. With where
    /// they all are now, to tell next time.
    fn moved_places(
        record: &SpaceRecord,
        binding: Option<u64>,
    ) -> (
        Vec<(String, tomori_environment::PanelPlace)>,
        std::collections::BTreeMap<String, tomori_environment::PanelPlace>,
    ) {
        use tomori_environment::PanelPlace;
        // The wall a panel at `place` is on, by its place among the environment's own, `Some(None)` for another wall,
        // and `None` floating.
        let walled = |place: PanelPlace| match place {
            PanelPlace::Wall { wall, .. } => Some(Some(wall)),
            PanelPlace::OtherWall => Some(None),
            _ => None,
        };
        let environment = &record.model.environment;
        let told = record.told.borrow();
        // A side bound anew, as one started again, knows nothing of where its panels were: each is new to it.
        let before =
            (told.environment == *environment && told.binding == binding).then_some(&told.places);
        let now = record.stage.published_places(environment);
        let mut moved = Vec::new();
        for (id, (place, asked)) in &now {
            let changed = match before.and_then(|before| before.get(id)) {
                Some(was) => walled(*was) != walled(*place),
                None => walled(*place) != Some(Some(*asked as usize)),
            };
            if changed {
                moved.push((id.clone(), *place));
            }
        }
        for id in before.into_iter().flat_map(|before| before.keys()) {
            if !now.contains_key(id) && record.stage.dismissed(environment, id) {
                moved.push((id.clone(), PanelPlace::Dismissed));
            }
        }
        let places = now
            .into_iter()
            .map(|(id, (place, _))| (id, place))
            .collect();
        (moved, places)
    }

    /// Tell `space`'s environment's side of every change of where its panels are since it was last told (see
    /// [`HubState::moved_places`]), each as an intent the server makes, `Intent::moved`, as `cause`'s, the participant's
    /// number, or `0`, the number the server gives nobody, for the layout's, the agent's or the environment's own; or as
    /// the participant's whose moves were held back by their rate, if any were. Nothing while a participant's message is
    /// taken, which [`HubState::tell_places_for`] tells once it is done. See `design/panel-actions.md`.
    fn tell_places(&self, space: &str, cause: u32) {
        if self.acting.get() {
            return;
        }
        for record in self.space(space).into_iter() {
            self.tell_record_places(record, cause);
        }
    }

    /// Tell `record`'s side, as [`HubState::tell_places`] does.
    fn tell_record_places(&self, record: &SpaceRecord, cause: u32) {
        let space = record.model.id.as_str();
        let binding = self.sides.binding(space);
        let (moved, places) = Self::moved_places(record, binding);
        let mut told = record.told.borrow_mut();
        // Moves held back are told as their mover's, unless the side is new to them.
        let owed = told.owed.take();
        let cause = match owed {
            Some(owed)
                if told.environment == record.model.environment && told.binding == binding =>
            {
                owed
            }
            _ => cause,
        };
        for (id, place) in moved {
            let delivered = self.sides.deliver(
                space,
                tomori_environment::Intent::move_of(
                    cause,
                    tomori_environment::Moved::new(id.as_str(), place),
                ),
            );
            log::debug!(
                "Panel {id} of space {space} is now {place:?}, by {cause}; told to its side: {delivered}."
            );
        }
        // How each panel is bound to its context, where that changed, or, for a side new to it, wherever it follows
        // anything less than everything, as `cause`'s, as a move is: the participant whose message changed it, as by
        // moving the panel onto another wall, or 0 for nobody's.
        let contexts = record.stage.contexts(&record.model.environment);
        let same = told.environment == record.model.environment && told.binding == binding;
        for (id, context) in &contexts {
            let before = same.then(|| told.contexts.get(id)).flatten();
            let changed = match before {
                Some(before) => before != context,
                // An instance is told whatever its context, as its side publishes nothing to it until it is.
                None => !context.follows() || context.of().is_some(),
            };
            if changed {
                let delivered = self.sides.deliver(
                    space,
                    tomori_environment::Intent::context_of(cause, context.clone()),
                );
                log::debug!(
                    "Panel {id} of space {space} is bound to {context:?}, by {cause}; told to its side: {delivered}."
                );
            }
        }
        told.contexts = contexts;
        told.environment.clone_from(&record.model.environment);
        told.binding = binding;
        told.places = places;
    }

    /// Tell `space`'s environment's side what the participant `participant`'s message changed of where its panels are,
    /// as theirs, taking one of their intents if it changed any: past their rate, nothing is told yet, and what it
    /// changed is told as theirs with the next telling, as the hub sweeps if nothing else changes first.
    fn tell_places_for(&mut self, space: &str, participant: &str) {
        let changed = self.space(space).is_some_and(|record| {
            !Self::moved_places(record, self.sides.binding(space))
                .0
                .is_empty()
        });
        if !changed {
            return;
        }
        let participant = self
            .participants
            .get_mut(participant)
            .expect("the participant acting is listed");
        let source = participant.source;
        if participant.intents.take(Instant::now()) {
            self.tell_places(space, source);
        } else {
            log::debug!("Moves of panels in space {space} wait for their mover's rate.");
            self.space(space)
                .expect("checked above")
                .told
                .borrow_mut()
                .owed = Some(source);
        }
    }

    fn touch_projection(&self, space: &str) {
        self.tell_places(space, 0);
        // Every change is a revision, even one that leaves the projection as it was, so that a client that showed a
        // change at once, such as a tick the server refused, is sent a patch that undoes it.
        for record in self.space(space).into_iter() {
            record
                .projection_revision
                .set(record.projection_revision.get() + 1);
            self.tell_projection(record);
        }
    }

    /// Tell everyone in `record`'s space whose client reads what it projects, and was not sent it at its revision now,
    /// that it changed: each is sent it as it is when sent, so that anything its environment's side changed that waited
    /// to be told is told with it, and one sent it since the last change is not sent it again.
    fn tell_projection(&self, record: &SpaceRecord) {
        record.projection_deferred.set(None);
        let revision = record.projection_revision.get();
        for member in record.model.members() {
            if let Some(connection) = self
                .participants
                .get(member)
                .and_then(|participant| participant.connection)
                .and_then(|connection| self.connections.get(&connection))
                .filter(|connection| connection.projection && connection.projection_sent < revision)
            {
                connection.outbox.projection();
            }
        }
    }

    /// A change the environment's side of `space` made, at `now`, to what it projects: a revision, as every change is,
    /// told to everyone in the space at once, unless they were told of one within [`PROJECTION_INTERVAL`], when it is
    /// deferred to the interval's end, and told then with every change since, the server's snapshot timer woken for
    /// it, as for an agent's next move, so that it is told then though nothing else happens.
    fn touch_projection_paced(&self, space: &str, now: Instant) {
        self.tell_places(space, 0);
        for record in self.space(space).into_iter() {
            record
                .projection_revision
                .set(record.projection_revision.get() + 1);
            if record.projection_deferred.get().is_some() {
                continue;
            }
            let due = record
                .projection_told
                .get()
                .map(|told| told + PROJECTION_INTERVAL)
                .filter(|due| now < *due);
            match due {
                Some(due) => {
                    record.projection_deferred.set(Some(due));
                    self.woken.notify(usize::MAX);
                }
                None => {
                    record.projection_told.set(Some(now));
                    self.tell_projection(record);
                }
            }
        }
    }

    /// Give the space `space`, which is listed, the title `title`, cleaned by [`limits::title`], as `by` asked, in place
    /// of its session's, or let it follow its session's title again when `title` is empty, telling everyone in it and
    /// the lobby. Returns whether the title shown changed. The caller keeps the change. See `design/spaces.md`.
    fn rename_space(&mut self, space: &str, title: String, by: tomori_protocol::Renamer) -> bool {
        let Some(record) = self.space_mut(space) else {
            unreachable!("only a listed space is renamed");
        };
        let before = record.model.renamed_by().cloned();
        let changed = record.model.rename(title, by.clone());
        let following = record.model.renamed_by().is_none();
        if changed || before.as_ref() != record.model.renamed_by() {
            let who = match &by {
                tomori_protocol::Renamer::Agent => "its agent".to_owned(),
                tomori_protocol::Renamer::Participant { .. }
                | tomori_protocol::Renamer::Unknown => "an owner".to_owned(),
            };
            if following {
                log::info!(
                    "Space {space} follows its session's title again, as {who} asked: {:?}.",
                    record.model.title
                );
            } else {
                log::info!(
                    "Space {space} is now titled {:?}, as {who} asked.",
                    record.model.title
                );
            }
            self.touch_space(space);
        }
        changed
    }

    /// Summon the agent of the space `id` to `at`, for whoever's eyes are at `partner`, or send it home without it, for
    /// everyone in the space; whether that changed where it stands. Nothing goes with it: its status wall and its
    /// workers stay at home. See `design/environments.md`.
    fn summon(
        &mut self,
        id: &str,
        at: Option<tomori_protocol::Pose>,
        partner: Option<[f32; 3]>,
    ) -> bool {
        let partner = partner.filter(|_| at.is_some());
        let changed = self.space_mut(id).is_some_and(|record| {
            let changed = record.summoned != at;
            let told = changed || record.partner != partner;
            record.summoned = at;
            record.partner = partner;
            told
        });
        if changed {
            self.touch_space(id);
        }
        changed
    }

    /// Tell everyone in the space, and the lobby, that the space changed.
    fn touch_space(&self, id: &str) {
        if let Some(space) = self.space(id) {
            let now = Instant::now();
            space.touched.set(Some(now));
            space.deferred.set(None);
            space.stamp_attention(now);
            // Members are connected, since disconnecting leaves the space.
            for outbox in space
                .model
                .members()
                .filter_map(|member| self.outbox(member))
            {
                outbox.space();
            }
        }
        self.touch_lobby();
    }

    /// Tell the space's participants that it changed because of an agent event, at most once per [`SNAPSHOT_INTERVAL`]. A change within the interval is deferred to its end, when the snapshot rendered then carries the latest state; returns true if so.
    fn touch_space_paced(&self, id: &str, now: Instant) -> bool {
        match self.space(id).map(|space| space.pace(now)) {
            Some(Pace::Now) => {
                self.touch_space(id);
                false
            }
            Some(Pace::Deferred) => true,
            // A space that is no longer listed has nobody to tell.
            Some(Pace::AlreadyDeferred) | None => false,
        }
    }

    /// Record when spaces closed since this was last called.
    fn stamp_closed(&mut self) {
        for space in &mut self.spaces {
            if space.closed.is_none() && space.model.is_closed() {
                space.closed = Some(self.next_closed);
                self.next_closed += 1;
                // Closing the sessions as the server stops does not close their spaces.
                space.kept_open |= self.shutting_down;
            }
        }
    }

    /// What is kept of the spaces listed.
    fn kept(&self) -> Vec<crate::kept::KeptSpace> {
        self.spaces
            .iter()
            .map(|space| {
                let model = &space.model;
                crate::kept::KeptSpace {
                    id: model.id.clone(),
                    title: model.title.clone(),
                    renamed_by: model.renamed_by().cloned(),
                    backend: model.backend.clone(),
                    directory: model.directory.clone(),
                    session: model.session.clone(),
                    environment: model.environment.clone(),
                    attribution: model.attribution(),
                    brief: model.speech() == SpeechSetting::Brief,
                    movement: Some(model.movement())
                        .filter(|movement| *movement != tomori_protocol::AgentMovement::default()),
                    instructions: model.space_instructions().to_owned(),
                    closed: model
                        .closed_reason()
                        .filter(|_| !space.kept_open)
                        .map(str::to_owned),
                    closed_order: space.closed.filter(|_| !space.kept_open),
                    placements: space.placements.clone(),
                    stage: space.stage.clone(),
                    places: space.places.clone(),
                    active: space.active,
                    timers: model.timers().kept(),
                }
            })
            .collect()
    }

    /// Forget the least recently closed spaces that nobody is in, beyond [`CLOSED_SPACES`]. Returns whether any were forgotten.
    fn forget_closed(&mut self) -> bool {
        self.stamp_closed();
        let occupied: HashSet<&str> = self
            .participants
            .values()
            .filter_map(|record| record.space.as_deref())
            .collect();
        let mut forgettable: Vec<(u64, String)> = self
            .spaces
            .iter()
            .filter(|space| {
                !space.kept_open
                    && space.model.members().next().is_none()
                    && !occupied.contains(space.model.id.as_str())
            })
            .filter_map(|space| Some((space.closed?, space.model.id.clone())))
            .collect();
        forgettable.sort_unstable();
        let Some(excess) = forgettable.len().checked_sub(CLOSED_SPACES) else {
            return false;
        };
        let forgotten: HashSet<&String> = forgettable[..excess].iter().map(|(_, id)| id).collect();
        for space in &self.spaces {
            if forgotten.contains(&space.model.id) && space.model.takes_instructions() {
                self.forgotten
                    .push((space.model.backend.clone(), space.model.session.clone()));
            }
        }
        self.spaces
            .retain(|space| !forgotten.contains(&space.model.id));
        self.unkept |= excess > 0;
        excess > 0
    }

    /// Whether a listed space other than `except` is bound to `session` of `backend`, open or closed.
    fn is_bound(&self, backend: &str, session: &str, except: Option<&str>) -> bool {
        self.spaces.iter().any(|space| {
            Some(space.model.id.as_str()) != except
                && space.model.backend == backend
                && space.model.session == session
        })
    }

    /// Tell every greeted participant outside a space that the listing changed.
    fn touch_lobby(&self) {
        for record in self.participants.values() {
            if record.space.is_none()
                && let Some(connection) = record.connection.and_then(|id| self.connections.get(&id))
                // A connection admitted only to pair learns nothing about the spaces.
                && connection.access.grant != Grant::Pairing
            {
                connection.outbox.spaces();
            }
        }
    }

    /// Hand the next queued input to the driver, if the space can accept it.
    fn pump(&mut self, id: &str) {
        // Tomori's own notes wait while anyone in the space is speaking, since their words will take them, but not for
        // words begun longer ago than anyone speaks: a client gone takes its open words with it, and one that never
        // ends them holds nothing back for long.
        let now = Instant::now();
        let hold = self.utterances.values().any(|utterance| {
            utterance.space == id
                && now.saturating_duration_since(utterance.begun) < crate::events::HOLD
        });
        let Some(space) = self.space_mut(id) else {
            return;
        };
        if let Some(prompt) = space.model.next_prompt_at(now, hold) {
            let key = prompt.key.clone();
            // Nobody's words began a prompt of Tomori's notes alone, so the agent goes to nobody for it.
            let source = space
                .model
                .given()
                .filter(|_| !crate::events::is_prompt(&key))
                .map(|given| given.source);
            if space.commands.try_send(SpaceCommand::Send(prompt)).is_err() {
                space
                    .model
                    .sent(&key, Err("the agent session has stopped".to_owned()));
            } else if let Some(source) = source {
                // A following agent goes to whoever it begins answering.
                self.agent_given(id, source, Instant::now());
            }
        }
    }

    /// The panels the participants in `space` are carrying at `now`, as their clients last reported, so that its agent
    /// leaves them in their hands. Visitors carry nothing.
    pub(crate) fn carried(&self, space: &str, now: Instant) -> Vec<String> {
        self.space(space).map_or_else(Vec::new, |record| {
            record
                .model
                .participants
                .iter()
                .filter_map(|key| self.participants.get(key))
                .filter_map(|participant| participant.seen.as_ref())
                .flat_map(|seen| seen.carried(now).iter().cloned())
                .collect()
        })
    }

    /// The names of the participants in `space` and where their heads are, as they last reported, for placing what its
    /// agent projects near them or where they look. Visitors send no presence.
    /// The participants of `space`, by name and tag, each with their head as they last reported it, if they have, so
    /// that the agent's tools find whom a name or handle means. Visitors report nothing, and are not among them.
    fn heads(&self, space: &str) -> Vec<crate::presence::Someone> {
        self.space(space).map_or_else(Vec::new, |record| {
            record
                .model
                .participants
                .iter()
                .filter_map(|key| self.participants.get(key))
                .map(|participant| crate::presence::Someone {
                    name: participant.name.clone(),
                    tag: participant.tag,
                    head: participant.seen.as_ref().map(|seen| seen.head),
                })
                .collect()
        })
    }

    /// Which participant of `space`, by their key, its agent acts for at `now`, as `asked` says: while its turn answers
    /// someone's words, the speaker; while it answers nobody's, as after a sub-agent's report, a timer or a message from
    /// outside Tomori, whom it talks with, as [`Self::talks_with`] finds them, or else whoever spoke last. None once
    /// they have left. See `design/projection.md`.
    pub(crate) fn asked_by(
        &self,
        space: &str,
        asked: &crate::space::Asked,
        now: Instant,
    ) -> Option<String> {
        let record = self.space(space).expect("the caller found it");
        let numbered = |source: u32| {
            record.model.participants.iter().find(|key| {
                self.participants
                    .get(*key)
                    .is_some_and(|participant| participant.source == source)
            })
        };
        let partner = (!asked.answering)
            .then(|| self.talks_with(space, now))
            .flatten();
        partner.or_else(|| asked.source.and_then(numbered).cloned())
    }

    /// Where the head of whoever the agent of `space` acts for at `now` is, as `asked` says who that is, by
    /// [`Self::asked_by`]: while its turn answers their words, where they were as those words began, or where they are
    /// now once they have walked far from there or the words are old, as [`crate::presence::asking`] says; otherwise
    /// where they are now. See `design/projection.md`.
    pub(crate) fn asker(
        &self,
        space: &str,
        asked: &crate::space::Asked,
        now: Instant,
    ) -> Option<tomori_protocol::Pose> {
        let head = self
            .asked_by(space, asked, now)
            .and_then(|key| self.participants.get(&key))
            .and_then(|participant| participant.seen.as_ref())
            .map(|seen| seen.head);
        if asked.answering {
            crate::presence::asking(asked.origin, head, now)
        } else {
            head
        }
    }

    /// Where the head of whoever the agent of `space` acts for now is, as [`Self::asker`] finds it, for its tools.
    pub(crate) fn asking(&self, space: &str) -> Option<tomori_protocol::Pose> {
        let asked = self
            .space(space)
            .expect("the caller found it")
            .model
            .asked();
        self.asker(space, &asked, Instant::now())
    }

    /// The least tag, from [`tomori_security::FIRST_TAG`], that neither a token of `tokens` nor any participant the
    /// server keeps has, for a participant without a token. See `design/security.md`.
    fn free_tag(&self, tokens: Option<&tomori_security::TokenStore>) -> Option<u32> {
        let mut used: std::collections::HashSet<u32> = self
            .participants
            .values()
            .filter_map(|record| record.tag)
            .collect();
        used.extend(
            tokens
                .map(tomori_security::TokenStore::entries)
                .unwrap_or_default()
                .iter()
                .filter_map(|entry| entry.tag),
        );
        (tomori_security::FIRST_TAG..=tomori_security::LAST_TAG).find(|tag| !used.contains(tag))
    }

    fn participants(&self, space: &SpaceModel) -> Vec<Participant> {
        space
            .participants
            .iter()
            .filter_map(|id| self.participants.get(id))
            .map(|record| Participant {
                name: record.name.clone(),
                source: record.source,
                engagement: record.engagement,
            })
            .collect()
    }

    /// Take a participant out of their space. A closed space left empty may be forgotten; see [`CLOSED_SPACES`].
    fn leave(&mut self, participant: &str) {
        self.depart(participant);
        self.forget_closed();
    }

    fn depart(&mut self, participant: &str) {
        let Self {
            participants,
            spaces,
            ..
        } = self;
        // Nothing to leave for a participant in the lobby. A space is forgotten only once nobody is in it, so the space they are in is listed; were it not, there would be nothing to leave either.
        let Some((space, record)) = participants
            .get_mut(participant)
            .and_then(|record| record.space.take())
            .and_then(|space| {
                spaces
                    .iter_mut()
                    .find(|record| record.model.id == space)
                    .map(|record| (space, record))
            })
        else {
            return;
        };
        record.model.participants.retain(|id| id != participant);
        record.model.visitors.retain(|id| id != participant);
        record.speech.left(participant);
        // Told that nothing is held for them any more, so that a hold on joining again is news.
        if record.speech.held_changed() {
            record.wake();
        }
        // Watching the cards ends with leaving their space.
        if let Some(connection) = participants
            .get(participant)
            .and_then(|record| record.connection)
            .and_then(|connection| self.connections.get_mut(&connection))
        {
            connection.cards = false;
        }
        self.withdraw_knock(participant, &space);
        self.touch_space(&space);
        // Its agent walks with them no more.
        if self
            .space(&space)
            .and_then(|record| record.following.walks_with())
            == Some(participant)
        {
            self.walk_no_more(&space, "they left the space", Instant::now());
        }
    }

    /// Put `participant` in `space`, taking them out of the space they were in. Returns false, changing nothing, if the space is no longer listed, such as one forgotten while a participant who was last in it was away, or if the participant has no connection, as one joining always has.
    fn join(&mut self, participant: &str, space: &str) -> bool {
        let Some((connection, engagement)) = self
            .space(space)
            .and(self.participants.get(participant))
            .and_then(|record| {
                record
                    .connection
                    .and_then(|id| self.connections.get(&id))
                    .map(|connection| (connection, record.engagement))
            })
        else {
            return false;
        };
        let participates = connection.access.role(Some(space)).participates();
        let outbox = connection.outbox.clone();
        // Not forgotten until the participant is in the space they join, which might be a closed one.
        self.depart(participant);
        // Departing forgets neither spaces nor participants, so both are still listed.
        let Some(record) = self.space_mut(space) else {
            unreachable!("found above, and departing forgets no space");
        };
        // Departed above, the participant is in neither list.
        if participates {
            record.model.participants.push(participant.to_owned());
        } else {
            record.model.visitors.push(participant.to_owned());
        }
        // A participant who missed part of the latest reply hears it from its start. Visitors follow live speech; see `design/speech.md`.
        if participates
            && (record.speech.joined(participant, engagement) || record.speech.held_changed())
        {
            record.wake();
        }
        let Some(record) = self.participants.get_mut(participant) else {
            unreachable!("found above, and departing forgets no participant");
        };
        record.space = Some(space.to_owned());
        record.last_space = Some(space.to_owned());
        // Where they stood in another space means nothing in this one.
        record.seen = None;
        outbox.clear_audio();
        self.update_role(participant);
        if participates {
            let knocks = self.knocks_for(space);
            self.send(participant, ServerMessage::Knocks { knocks });
        }
        self.forget_closed();
        self.touch_space(space);
        // After the space's state, what is projected in it, whole, for a client that reads it.
        if let Some(connection) = self
            .participants
            .get(participant)
            .and_then(|record| record.connection)
            .and_then(|connection| self.connections.get_mut(&connection))
            .filter(|connection| connection.projection)
        {
            connection.projected = None;
            connection.projection_sent = 0;
            outbox.projection();
        }
        // After the space's state, the state its environment's side keeps for it, so that the client knows which space and environment it is for.
        let record = self.space(space).expect("joined above");
        if let Some((channel, state)) = record
            .channel
            .as_ref()
            .and_then(|channel| channel.state.clone().map(|state| (channel, state)))
            && self
                .channel_outboxes(record, channel)
                .iter()
                .any(|joined| Arc::ptr_eq(joined, &outbox))
        {
            outbox.message(ServerMessage::EnvironmentState {
                environment: channel.environment.clone(),
                version: channel.version,
                state,
            });
        }
        true
    }

    /// The knocks waiting on `space`, as its participants see them.
    fn knocks_for(&self, space: &str) -> Vec<tomori_protocol::KnockRequest> {
        self.knocks
            .iter()
            .filter(|knock| knock.space == space)
            .map(|knock| tomori_protocol::KnockRequest {
                id: knock.id.clone(),
                name: knock.name.clone(),
            })
            .collect()
    }

    /// Tell the participants of `space` the knocks waiting on it.
    fn announce_knocks(&self, space: &str) {
        let knocks = self.knocks_for(space);
        // A knock is withdrawn as its visitor leaves the space, which is listed until nobody is in it, so a space knocked on is listed; were it not, it would have nobody to tell.
        for participant in self
            .space(space)
            .into_iter()
            .flat_map(|record| &record.model.participants)
        {
            self.send(
                participant,
                ServerMessage::Knocks {
                    knocks: knocks.clone(),
                },
            );
        }
    }

    /// Call `name` everyone `named` picks out by hub key and record: the participants holding a principal's token, on every device, or one participant without one. Each whose client reads names is told, and so is everyone in the spaces they are in, and the participants of spaces they knock on. Their queued input keeps the name it was spoken under. See `design/security.md`.
    fn rename(&mut self, named: impl Fn(&str, &ParticipantRecord) -> bool, name: &str) {
        let renamed: Vec<String> = self
            .participants
            .iter_mut()
            .filter(|(key, record)| named(key, record) && record.name != name)
            .map(|(key, record)| {
                record.name = name.to_owned();
                key.clone()
            })
            .collect();
        let mut spaces = Vec::new();
        for key in &renamed {
            let record = &self.participants[key];
            if let Some(connection) = record.connection.and_then(|id| self.connections.get(&id))
                && connection.names
            {
                connection.outbox.message(ServerMessage::Name {
                    name: name.to_owned(),
                });
            }
            // A visitor's name is in no snapshot, so only the spaces they take part in are told.
            spaces.extend(record.space.clone().filter(|space| {
                self.space(space)
                    .is_some_and(|space| space.model.participants.contains(key))
            }));
        }
        let mut knocked = Vec::new();
        for knock in &mut self.knocks {
            if renamed.contains(&knock.participant) {
                knock.name = name.to_owned();
                knocked.push(knock.space.clone());
            }
        }
        for space in spaces {
            self.touch_space(&space);
        }
        for space in knocked {
            self.announce_knocks(&space);
        }
    }

    /// Rename `participant`, as they asked, from a message or an utterance's transcript, `utterance`. A name others have is theirs to take too, since each participant's tag tells them apart. A token holder's name is kept with their token, which the server writes outside the lock, and everyone holding it is renamed once it is written; anyone else's is kept only while the server keeps them. Renaming to the name they have already changes nothing. See `design/security.md`.
    fn rename_request(
        &mut self,
        participant: &str,
        name: &str,
        utterance: Option<String>,
        tokens: Option<Arc<tomori_security::TokenStore>>,
        deferred: &mut Vec<Deferred>,
    ) {
        let (principal, unchanged) = self
            .participants
            .get(participant)
            .map(|record| (record.principal.clone(), record.name == name))
            .unwrap_or_default();
        match (principal, tokens) {
            (Some(principal), Some(tokens)) if !unchanged => {
                deferred.push(Deferred::Rename(Rename {
                    tokens,
                    principal,
                    name: name.to_owned(),
                    asked: Some((participant.to_owned(), utterance)),
                }));
                return;
            }
            // The name they have already, which is not written again or told.
            _ if unchanged => {}
            // A visitor, or anyone on a server that keeps no tokens, as in `--auth open` mode.
            _ => {
                self.rename(|key, _| key == participant, name);
                log::info!("A participant without a token is now called {name:?}.");
            }
        }
        if let Some(utterance) = utterance {
            self.send(
                participant,
                ServerMessage::Transcript {
                    utterance,
                    text: name.to_owned(),
                    finished: true,
                },
            );
        }
    }

    /// Withdraw the knock of `participant` on `space`, if any, telling them if they are still connected.
    fn withdraw_knock(&mut self, participant: &str, space: &str) {
        let before = self.knocks.len();
        self.knocks
            .retain(|knock| !(knock.participant == participant && knock.space == space));
        if self.knocks.len() != before {
            self.send(
                participant,
                ServerMessage::KnockState {
                    state: tomori_protocol::KnockState::Withdrawn,
                },
            );
            self.announce_knocks(space);
        }
    }

    /// Admit the visitor of `knock`, on its connection, as a collaborator in its space under `principal`, telling it `token`. The participant is kept under its new key, so that it keeps its number and place. Returns false if the visitor has gone.
    fn admit(
        &mut self,
        knock: &KnockRecord,
        principal: String,
        tag: Option<u32>,
        token: String,
    ) -> bool {
        let still_here = self
            .connections
            .get(&knock.connection)
            .is_some_and(|record| {
                record.participant.as_deref() == Some(knock.participant.as_str())
            });
        // A connected participant is kept, so one still here is found.
        let Some((identity, mut record)) = knock
            .participant
            .strip_prefix('/')
            .filter(|_| still_here)
            .and_then(|identity| {
                self.participants
                    .remove(&knock.participant)
                    .map(|record| (identity, record))
            })
        else {
            return false;
        };
        let key = participant_key(Some(&principal), identity);
        // A knock made again since the decision belongs to the old key, which is gone.
        let stale: Vec<String> = self
            .knocks
            .iter()
            .filter(|waiting| waiting.participant == knock.participant)
            .map(|waiting| waiting.space.clone())
            .collect();
        self.knocks
            .retain(|waiting| waiting.participant != knock.participant);
        record.visitor = false;
        record.principal = Some(principal.clone());
        // Their token's tag from now on, or, without one, the one they had.
        record.tag = tag.or(record.tag);
        let space = record.space.clone();
        self.participants.insert(key.clone(), record);
        let connection = self
            .connections
            .get_mut(&knock.connection)
            .expect("checked above");
        connection.participant = Some(key.clone());
        connection.access = Access {
            principal: Some(principal),
            // The token is named after the visitor, as they were called when they knocked.
            name: Some(knock.name.clone()),
            grant: Grant::Collaborator {
                space: knock.space.clone(),
            },
        };
        connection
            .liveness
            .output
            .set_inbound(Inbound::greeted(&connection.access));
        connection.outbox.message(ServerMessage::Admitted { token });
        if let Some(space) = &space
            && let Some(record) = self.space_mut(space)
        {
            record.model.visitors.retain(|id| *id != knock.participant);
            if *space == knock.space {
                record.model.participants.push(key.clone());
            } else {
                record.model.visitors.push(key.clone());
            }
        }
        self.update_role(&key);
        for space in stale {
            self.announce_knocks(&space);
        }
        if let Some(space) = space {
            self.touch_space(&space);
            self.announce_knocks(&space);
        }
        true
    }

    /// End the pending pairing, of `connection` if given, telling its device why. Returns whether one ended.
    fn end_pairing(&mut self, connection: Option<ConnectionId>, error: &str) -> bool {
        let Some(pairing) = self.pairing.take_if(|pairing| {
            connection.is_none_or(|connection| pairing.connection == connection)
        }) else {
            return false;
        };
        pairing.outbox.message(ServerMessage::PairFailed {
            error: error.to_owned(),
        });
        true
    }

    /// Take the pairing waiting, if `confirmation` confirms it, for its device to be issued a token. A code typed on the console, or given to the agent, that does not match declines the pairing, telling its device, so that each pairing gives a code from anywhere but the device one try: neither the console nor the agent is shown the code, and whatever can type on the console, such as through a pipe, must not search for it. See `design/security.md`.
    fn take_confirmed(
        &mut self,
        confirmation: Confirmation<'_>,
    ) -> Result<PendingPairing, &'static str> {
        let pairing = self.pairing.as_ref().ok_or("No pairing is waiting.")?;
        let code = pairing
            .code
            .as_deref()
            .ok_or("The device has not shown its code yet.")?;
        let confirmed = match confirmation {
            Confirmation::Typed(typed) | Confirmation::Agent(typed) => typed == code,
            Confirmation::Shown(serial) => serial == pairing.serial,
        };
        if !confirmed {
            return Err(match confirmation {
                Confirmation::Typed(_) | Confirmation::Agent(_) => {
                    if matches!(confirmation, Confirmation::Typed(_)) {
                        log::info!(
                            "Declined the pairing of {}, since the code typed did not match the one it shows.",
                            pairing.device
                        );
                    }
                    self.end_pairing(
                        None,
                        "The code given did not match, so the pairing was declined.",
                    );
                    "That code does not match, so the pairing was declined."
                }
                Confirmation::Shown(_) => {
                    "A different pairing is waiting now, so nothing was confirmed."
                }
            });
        }
        Ok(self.pairing.take().expect("checked above"))
    }

    /// Whether pairings are paused at `now`, since [`PAIRING_ATTEMPTS`] began within [`PAIRING_WINDOW`].
    fn pairing_paused(&self, now: Instant) -> bool {
        self.pairing_attempts
            .iter()
            .filter(|began| now.saturating_duration_since(**began) < PAIRING_WINDOW)
            .count()
            >= PAIRING_ATTEMPTS
    }

    /// Decline the pairing waiting, telling its device: the one with `serial` if given, as the status page showed it, or whichever is waiting. Returns whether one was declined.
    fn decline_pairing(&mut self, serial: Option<u64>) -> bool {
        if serial.is_some_and(|serial| {
            self.pairing
                .as_ref()
                .is_none_or(|pairing| pairing.serial != serial)
        }) {
            return false;
        }
        self.end_pairing(None, "The pairing was declined.")
    }

    /// Forget the longest gone disconnected visitors beyond [`RETAINED_VISITORS`].
    fn forget_excess_visitors(&mut self) {
        let mut departed: Vec<(u64, String)> = self
            .participants
            .iter()
            .filter(|(_, record)| record.visitor && record.connection.is_none())
            .map(|(key, record)| (record.departure, key.clone()))
            .collect();
        let Some(excess) = departed.len().checked_sub(RETAINED_VISITORS) else {
            return;
        };
        departed.sort_unstable();
        for (_, key) in departed.into_iter().take(excess) {
            self.participants.remove(&key);
        }
    }

    /// Answer the current field of `space`'s presented question as `participant`, sending the answers to the agent once the question is complete.
    fn answer(
        &mut self,
        participant: &str,
        space: &str,
        question: &str,
        field: &str,
        text: &str,
    ) -> Result<(), String> {
        // A member answers in a space that is listed, since one is forgotten only once nobody is in it; were it not, there would be nothing to answer.
        let (record, outcome) = self
            .space_mut(space)
            .ok_or("Join a space first.".to_owned())
            .and_then(|record| {
                record
                    .model
                    .answer(question, field, text)
                    .map(|outcome| (record, outcome))
            })?;
        match outcome {
            AnswerOutcome::Recorded => {}
            AnswerOutcome::Complete(answers) => {
                let sent = record.commands.try_send(SpaceCommand::Answer {
                    question: question.to_owned(),
                    answers,
                    participant: participant.to_owned(),
                });
                if sent.is_err() {
                    record.model.restore_question(question);
                    self.touch_space(space);
                    return Err("The agent session has stopped.".to_owned());
                }
            }
        }
        self.touch_space(space);
        Ok(())
    }

    /// The greeted participant of a connection and their current space.
    fn member(&self, connection: ConnectionId) -> Option<(String, Option<String>)> {
        let participant = self.connections.get(&connection)?.participant.as_ref()?;
        // A connection names a participant who is kept; were it not, it would be treated as not yet greeted.
        self.participants
            .get(participant)
            .map(|record| (participant.clone(), record.space.clone()))
    }

    /// Close the open utterance of a connection, if any.
    fn abandon_utterance(&mut self, connection: ConnectionId, deferred: &mut Vec<Deferred>) {
        if let Some(open) = self
            .connections
            .get_mut(&connection)
            .and_then(|record| record.utterance.take())
        {
            let ended = self
                .utterances
                .remove(&open.id)
                .map(UtteranceRecord::recorder);
            // A following agent's move for them stops where it is.
            ended.iter().for_each(|(participant, space)| {
                self.agent_cancelled(space, participant, Instant::now());
            });
            self.recorded(ended);
            deferred.push(Deferred::Cancel(open.id));
        }
    }

    /// The participant and space of an utterance that stopped being recorded other than by its end, if there was one,
    /// as by cancelling or failing: the speech held for them while they recorded plays now. See `design/speech.md`.
    fn recorded(&mut self, ended: Option<(String, String)>) {
        self.stopped_recording(ended, None);
    }

    /// The participant and space of an utterance whose end arrived, as its participant let go of X, if there was one:
    /// the speech held for them while they recorded plays after [`crate::retention::RELEASE_PAUSE`], from `now`.
    fn released(&mut self, ended: Option<(String, String)>, now: Instant) {
        self.stopped_recording(ended, Some(now));
    }

    fn stopped_recording(&mut self, ended: Option<(String, String)>, released: Option<Instant>) {
        if let Some((participant, space)) = ended
            && let Some(record) = self.space_mut(&space)
            && match released {
                Some(now) => record.speech.released(&participant, now),
                None => record.speech.recorded(&participant),
            } | record.speech.held_changed()
        {
            record.wake();
        }
    }
}

impl Hub {
    pub fn new(
        backends: Backends,
        transcriber: Arc<dyn Transcriber>,
        jobs: async_channel::Sender<Job>,
        roots: Vec<WorkspaceRoot>,
        slots: Arc<SpaceSlots>,
    ) -> Arc<Self> {
        let deferred_snapshot = Arc::new(Notify::new());
        let server_sides = Arc::new(crate::server_sides::ServerSides::default());
        let hub = Arc::new_cyclic(|this| Self {
            state: Mutex::new(HubState {
                sides: server_sides.clone(),
                run: run_token(),
                woken: deferred_snapshot.clone(),
                ..HubState::default()
            }),
            backends,
            transcriber,
            jobs,
            slots,
            roots: RwLock::new(roots),
            environments: RwLock::new(Environments::default()),
            server_sides,
            deferred_snapshot,
            counts_changed: Notify::new(),
            cards_watched: Notify::new(),
            max_visitors: AtomicUsize::new(usize::MAX),
            tokens: RwLock::new(None),
            kept: RwLock::new(None),
            images: Arc::new(crate::images::ImageStore::new()),
            encrypted: AtomicBool::new(false),
            pairing: RwLock::new(None),
            pairing_prompt: RwLock::new(None),
            pairing_timeout: RwLock::new(PAIRING_TIMEOUT),
            pairing_open_for: RwLock::new(PAIRING_OPEN_FOR),
            knock_timeout: RwLock::new(KNOCK_TIMEOUT),
            inspect_timeout: RwLock::new(agent_inspect::INSPECT_TIMEOUT),
            act_timeout: RwLock::new(agent_actions::ACT_TIMEOUT),
            tools: tools::Tools::default(),
            highlighter: RwLock::new(crate::highlighting::highlighter()),
            clock: RwLock::new(None),
            zone: RwLock::new(crate::timers::system_zone()),
            this: this.clone(),
        });
        let this = Arc::downgrade(&hub);
        // A failure after the hub is gone has no space to show it in.
        hub.server_sides.on_failure(move |failure| {
            let _ = this.upgrade().map(|hub| hub.side_failed(failure));
        });
        let this = Arc::downgrade(&hub);
        hub.server_sides
            .on_outlet(move |space, binding, environment| {
                Arc::new(SpaceOutlet {
                    hub: this.clone(),
                    space: space.to_owned(),
                    binding,
                    environment: environment.id().to_owned(),
                    version: environment.version(),
                })
            });
        hub
    }

    /// Offer `environments` for spaces, as in [`crate::Server::with_environments`].
    pub(crate) fn set_environments(&self, environments: Environments) {
        *self
            .environments
            .write()
            .expect("environments are never left poisoned") = environments;
        // Spaces bound to an environment that was not offered before, or whose side has changed, are run for now.
        let mut state = self.lock();
        self.reconcile_server_sides(&mut state);
    }

    /// Offer `environment` in place of the one with its identifier, or else after the others, as [`Self::set_environments`] offers them, as in [`crate::Server::with_environment`].
    pub(crate) fn set_environment(&self, environment: crate::Environment) {
        let environments = self.environments().clone().with(environment);
        self.set_environments(environments);
    }

    /// The environments offered for spaces, in order.
    pub(crate) fn environments_offered(&self) -> Vec<crate::Environment> {
        self.environments().as_slice().to_vec()
    }

    fn environments(&self) -> std::sync::RwLockReadGuard<'_, Environments> {
        self.environments
            .read()
            .expect("environments are never left poisoned")
    }

    /// Set the workspace roots a client can choose from, as in [`crate::Server::with_workspace_roots`].
    pub(crate) fn set_roots(&self, roots: Vec<WorkspaceRoot>) {
        *self
            .roots
            .write()
            .expect("workspace roots are never left poisoned") = roots;
    }

    /// Pair devices with this setup; see `design/security.md`.
    pub(crate) fn set_pairing(&self, setup: PairingSetup) {
        *self.pairing.write().expect("never left poisoned") = Some(setup);
    }

    pub(crate) fn set_pairing_prompt(&self, prompt: PairingPrompt) {
        *self.pairing_prompt.write().expect("never left poisoned") = Some(prompt);
    }

    pub(crate) fn set_pairing_timeout(&self, timeout: Duration) {
        *self.pairing_timeout.write().expect("never left poisoned") = timeout;
    }

    pub(crate) fn set_knock_timeout(&self, timeout: Duration) {
        *self.knock_timeout.write().expect("never left poisoned") = timeout;
    }

    /// Admit visitors who knock with collaborator tokens from `tokens`.
    pub(crate) fn set_tokens(&self, tokens: Arc<tomori_security::TokenStore>) {
        *self.tokens.write().expect("never left poisoned") = Some(tokens);
    }

    /// The state of the space `id`, as its participants would see it.
    #[cfg(test)]
    pub(crate) fn space_state(&self, id: &str) -> tomori_protocol::SpaceState {
        self.lock()
            .space(id)
            .expect("the space is listed")
            .model
            .state(Vec::new(), "")
    }

    /// The session the space `id` is bound to.
    #[cfg(test)]
    pub(crate) fn space_session(&self, id: &str) -> String {
        self.lock()
            .space(id)
            .expect("the space is listed")
            .model
            .session
            .clone()
    }

    /// Whether the space `id` is listed, so that a collaborator token for a space that is gone can be revoked.
    pub fn has_space(&self, id: &str) -> bool {
        self.lock().space(id).is_some()
    }

    /// Issue a collaborator token for `knock` and admit its visitor on the same connection, if it is still connected. Called outside the lock, since issuing writes the token store.
    fn admit(&self, knock: KnockRecord) {
        let tokens = knock.tokens.clone();
        let (entry, token) = match tokens.issue(
            &knock.name,
            tomori_security::Grant::Collaborator {
                space: knock.space.clone(),
            },
        ) {
            Ok(issued) => issued,
            Err(error) => {
                log::error!("Could not issue a token for an admitted knock: {error}.");
                if let Some(record) = self.lock().connections.get(&knock.connection) {
                    record.outbox.message(ServerMessage::Error {
                        error: "You were admitted, but the token could not be issued. Knock again."
                            .to_owned(),
                        space: None,
                        panel: None,
                        wall: None,
                    });
                }
                return;
            }
        };
        let admitted = {
            let mut state = self.lock();
            let admitted = state.admit(&knock, entry.principal().id, entry.tag, token);
            if !admitted && let Some(record) = state.connections.get(&knock.connection) {
                // Still connected, but no longer the visitor who knocked, such as after reconnecting.
                record.outbox.message(ServerMessage::KnockState {
                    state: tomori_protocol::KnockState::Withdrawn,
                });
            }
            self.recount(&mut state);
            admitted
        };
        if admitted {
            log::info!(
                "Admitted {:?} to {} as collaborator token {}.",
                knock.name,
                knock.space,
                entry.id
            );
        } else {
            // The visitor left while it was issued.
            crate::door::revoke(&tokens, &entry.id);
        }
    }

    /// Mark the server as serving only TLS, so that it pairs devices.
    pub(crate) fn set_encrypted(&self, encrypted: bool) {
        self.encrypted.store(encrypted, Ordering::Relaxed);
    }

    /// The pairing pending, revealed or not, if any.
    pub fn pending_pairing(&self) -> Option<PairingView> {
        let state = self.lock();
        let pairing = state.pairing.as_ref()?;
        Some(PairingView {
            serial: pairing.serial,
            device: pairing.device.clone(),
            code: pairing
                .code
                .as_deref()
                .map(tomori_security::pairing::display),
        })
    }

    /// Confirm the pairing waiting, issuing its device an owner token. Returns the device, as pairing shows it.
    pub fn confirm_pairing(
        &self,
        confirmation: Confirmation<'_>,
    ) -> Result<ChosenName, &'static str> {
        let pairing = self.lock().take_confirmed(confirmation)?;
        self.issue_pairing(pairing)
    }

    /// Issue the device of `pairing`, which was confirmed, an owner token, telling it. Returns the device, as pairing shows it. The token is issued outside the hub's lock, since issuing writes the token store.
    fn issue_pairing(&self, pairing: PendingPairing) -> Result<ChosenName, &'static str> {
        let outbox = pairing.outbox.clone();
        let issued = pairing
            .setup
            .tokens
            .issue(&pairing.token_name, tomori_security::Grant::Owner)
            .map_err(|error| {
                log::error!("Could not issue a token for a pairing: {error}.");
                "The token could not be issued."
            });
        match issued {
            Ok((entry, token)) => {
                log::info!("Paired {} as owner token {}.", pairing.device, entry.id);
                outbox.message(ServerMessage::Paired { token });
                // One pairing for each window, so that an opening is used once. See `design/security.md`.
                self.paired_closes_pairing();
                Ok(pairing.device)
            }
            Err(error) => {
                outbox.message(ServerMessage::PairFailed {
                    error: format!("{error} Try again."),
                });
                Err(error)
            }
        }
    }

    /// Whether pairings are paused, since [`PAIRING_ATTEMPTS`] began within [`PAIRING_WINDOW`].
    pub fn pairing_paused(&self) -> bool {
        self.lock().pairing_paused(Instant::now())
    }

    /// Allow pairings again, forgetting those that began recently, as the operator or an owner may when pairing is paused, such as by someone using up the attempts.
    pub fn allow_pairing(&self) {
        self.lock().pairing_attempts.clear();
    }

    /// Decline the pairing waiting, telling its device: the one with `serial` if given, as the status page showed it, or whichever is waiting. Returns whether one was declined.
    pub fn decline_pairing(&self, serial: Option<u64>) -> bool {
        let mut state = self.lock();
        let device = state.pairing.as_ref().map(|pairing| pairing.device.clone());
        let declined = state.decline_pairing(serial);
        if declined && let Some(device) = device {
            log::info!("Declined the pairing of {device}.");
        }
        declined
    }

    /// Change the most visitors connected at once; see [`Hub::connect`].
    pub(crate) fn set_max_visitors(&self, limit: usize) {
        self.max_visitors.store(limit, Ordering::Relaxed);
    }

    /// The hub's state, locked. Should a request ever panic while holding it, the next takes it as it was left, logging that, so that one request fails rather than every later one.
    fn lock(&self) -> MutexGuard<'_, HubState> {
        self.state.lock().unwrap_or_else(|poisoned| {
            log::error!(
                "A request failed while holding the hub's state; continuing with the state it left."
            );
            self.state.clear_poison();
            poisoned.into_inner()
        })
    }

    /// How many spaces are listed and participants connected, as advertised; see `design/server.md`.
    pub fn counts(&self) -> Counts {
        self.lock().counts()
    }

    /// Who is where now, for the status page.
    pub fn status(&self) -> Status {
        let state = self.lock();
        // Members are kept and connected, since disconnecting leaves the space; one that were not would not be listed.
        let role = |key: &str, space: Option<&str>| {
            state.participants.get(key).and_then(|record| {
                record
                    .connection
                    .and_then(|connection| state.connections.get(&connection))
                    .map(|connection| (record.name.clone(), connection.access.role(space)))
            })
        };
        let spaces = state
            .spaces
            .iter()
            .map(|space| SpaceStatus {
                title: space.model.title.clone(),
                activity: space.model.activity(),
                participants: space
                    .model
                    .participants
                    .iter()
                    .filter_map(|key| role(key, Some(&space.model.id)))
                    .collect(),
                visitors: space.model.visitors.len(),
            })
            .collect();
        // In the lobby, people are listed by what they were admitted as; visitors are only counted.
        let lobby = state
            .participants
            .values()
            .filter(|record| record.space.is_none())
            // Those who left are kept for a while, but not listed.
            .filter_map(|record| {
                record
                    .connection
                    .and_then(|connection| state.connections.get(&connection))
                    .map(|connection| (record, connection))
            })
            .filter_map(|(record, connection)| {
                let role = match connection.access.grant {
                    Grant::Owner => ConnectionRole::Owner,
                    Grant::Collaborator { .. } => ConnectionRole::Collaborator,
                    Grant::Visitor | Grant::Pairing => return None,
                };
                Some((record.name.clone(), role))
            })
            .collect();
        let visitors = state
            .connections
            .values()
            .filter(|record| record.participant.is_some() && record.access.is_visitor())
            .count();
        Status {
            spaces,
            lobby,
            visitors,
            pairing_decisions: state.pairing_decisions.iter().cloned().collect(),
        }
    }

    /// Wait for [`counts`](Self::counts) to change. Check them after creating the listener to avoid a lost wakeup.
    pub fn listen_counts(&self) -> EventListener {
        self.counts_changed.listen()
    }

    /// Notify listeners if the counts changed since this was last called. Called after every change that can add or remove a space or a connected participant.
    fn recount(&self, state: &mut HubState) {
        let counts = state.counts();
        if counts != state.counts {
            state.counts = counts;
            self.counts_changed.notify(usize::MAX);
        }
        self.forget_instructions(state);
    }

    /// Ask the supervisor to remove Tomori's instruction entries from the sessions of forgotten spaces, which are Tomori's alone, unless another space is bound to the same session. Called with [`Self::recount`], which follows every change that can forget a space. See `design/spaces.md`.
    fn forget_instructions(&self, state: &mut HubState) {
        for (backend, session) in std::mem::take(&mut state.forgotten) {
            let bound = state
                .spaces
                .iter()
                .any(|space| space.model.backend == backend && space.model.session == session);
            if !bound && !session.is_empty() {
                // Once shutting down, the jobs are closed, and the entries stay until the space's session is used again.
                let _ = self
                    .jobs
                    .try_send(Job::ForgetInstructions { backend, session });
            }
        }
    }

    fn run(&self, deferred: Vec<Deferred>) {
        for action in deferred {
            match action {
                Deferred::Begin(id) => self.transcriber.begin(
                    id,
                    TranscriptSink {
                        hub: self.this.clone(),
                        utterance: id,
                    },
                ),
                Deferred::Push(id, frame) => self.transcriber.push(id, &frame),
                Deferred::End(id) => self.transcriber.end(id),
                Deferred::Cancel(id) => self.transcriber.cancel(id),
                Deferred::Admit(knock) => self.admit(knock),
                Deferred::Rename(rename) => self.keep_name(rename),
                Deferred::Keep => {
                    self.keep();
                }
                Deferred::Image(connection, hash) => self.send_image(connection, hash),
                Deferred::PairingPrompt(event) => {
                    if let Some(prompt) = &*self.pairing_prompt.read().expect("never left poisoned")
                    {
                        prompt(&event);
                    }
                }
            }
        }
    }

    /// Keep `name` with the token of `principal`, writing the token store, and then call everyone holding it by it, on every device. Called outside the lock, since it writes the token store; returns the lock, taken again, and whether the name was kept.
    fn write_name(
        &self,
        tokens: &tomori_security::TokenStore,
        principal: &str,
        name: &str,
    ) -> (MutexGuard<'_, HubState>, Result<(), Unkept>) {
        let id = principal.strip_prefix("token:").unwrap_or_default();
        // Another token's name is no bar: tags tell their holders apart.
        let kept = match tokens.rename(id, name, |_| false) {
            Ok(tomori_security::Renamed::Renamed) => Ok(()),
            Ok(_) => Err(Unkept::Revoked),
            Err(error) => {
                log::warn!("Could not keep the name of token {id}: {error}.");
                Err(Unkept::Unwritten)
            }
        };
        let mut state = self.lock();
        if kept.is_ok() {
            log::info!("Token {id} is now called {name:?}.");
            state.rename(
                |_, record| record.principal.as_deref() == Some(principal),
                name,
            );
        }
        (state, kept)
    }

    /// Keep a token holder's new name with their token, then call everyone holding it by it, on every device, and answer whoever asked. Called outside the lock, since renaming writes the token store.
    fn keep_name(&self, rename: Rename) {
        let Rename {
            tokens,
            principal,
            name,
            asked,
        } = rename;
        let (state, kept) = self.write_name(&tokens, &principal, &name);
        let kept = kept.map_err(|unkept| match unkept {
            Unkept::Revoked => "Your token is no longer issued.",
            Unkept::Unwritten => "Your name could not be kept. Try again.",
        });
        let Some((participant, utterance)) = asked else {
            return;
        };
        let reply = match (kept, utterance) {
            (Ok(()), Some(utterance)) => ServerMessage::Transcript {
                utterance,
                text: name,
                finished: true,
            },
            (Ok(()), None) => return,
            (Err(error), Some(utterance)) => ServerMessage::TranscriptFailed {
                utterance,
                error: error.to_owned(),
            },
            (Err(error), None) => error_message(format!("Name not changed: {error}")),
        };
        state.send(&participant, reply);
    }

    /// Call each token holder by the name kept with their token, where it changed, such as by `tomori-server auth rename` in another process. `kept` gives a principal's name, or `None` once their token is revoked. A name that is not one the server shows is ignored. See `design/security.md`.
    pub fn refresh_names(&self, kept: impl Fn(&str) -> Option<String>) {
        let mut state = self.lock();
        let mut changed: Vec<(String, String)> = state
            .participants
            .values()
            .filter_map(|record| {
                let principal = record.principal.as_deref()?;
                let name = kept(principal)?;
                let name = limits::name(&name).ok()?;
                (name != record.name).then(|| (principal.to_owned(), name))
            })
            .collect();
        changed.sort_unstable();
        changed.dedup();
        for (principal, name) in changed {
            log::info!("The token of {principal} is now called {name:?}.");
            state.rename(
                |_, record| record.principal.as_deref() == Some(principal.as_str()),
                &name,
            );
        }
    }

    /// The token store, if the server keeps tokens, so that a token holder's name is kept with their token.
    fn token_store(&self) -> Option<Arc<tomori_security::TokenStore>> {
        self.tokens.read().expect("never left poisoned").clone()
    }

    /// Register a connection admitted at the door as `access`. `cancellation` must end only this connection's exchange: the keepalive sweep cancels it when the connection goes silent. The connection sets the returned flag whenever data arrives, and reports its writes to the returned [`Output`]. Returns `None` for a visitor while the most visitors are connected; see `design/security.md`.
    pub fn connect(&self, cancellation: Cancellation, access: Access) -> Option<Connected> {
        let mut state = self.lock();
        if access.is_visitor() {
            let visitors = state
                .connections
                .values()
                .filter(|record| record.access.is_visitor())
                .count();
            if visitors >= self.max_visitors.load(Ordering::Relaxed) {
                return None;
            }
        }
        state.next_connection += 1;
        let id = state.next_connection;
        let outbox = Arc::new(Outbox::new());
        let received = Arc::new(AtomicBool::new(false));
        let output = Arc::new(Output::default());
        state.connections.insert(
            id,
            ConnectionRecord {
                outbox: outbox.clone(),
                access,
                role: None,
                names: false,
                presents: None,
                participant: None,
                utterance: None,
                failed: None,
                stray_frames: 0,
                straying: false,
                liveness: Liveness {
                    received: received.clone(),
                    last: None,
                    opened: None,
                    greeted: false,
                    pinged: None,
                    output: output.clone(),
                    written: 0,
                    stalled: None,
                    cancellation,
                },
                cards: false,
                environment_state: false,
                projection: false,
                pairing_window: false,
                speech_held: false,
                images: false,
                diagrams: false,
                world_edges: false,
                patches: false,
                live_panels: false,
                limits: Limits::TODAY,
                projected: None,
                projection_sent: 0,
            },
        );
        Some((id, outbox, received, output))
    }

    /// End the connections of principals that are no longer valid, such as when their token was revoked, as if they had disconnected.
    pub fn revoke(&self, valid: impl Fn(&str) -> bool) {
        let revoked: Vec<(ConnectionId, Cancellation)> = self
            .lock()
            .connections
            .iter()
            .filter(|(_, record)| {
                record
                    .access
                    .principal
                    .as_deref()
                    .is_some_and(|principal| !valid(principal))
            })
            .map(|(&id, record)| (id, record.liveness.cancellation.clone()))
            .collect();
        for (id, cancellation) in revoked {
            log::info!("Connection {id} presented a credential that was revoked.");
            self.disconnect(id);
            cancellation.cancel();
        }
    }

    /// Ping connections that have been quiet for the keepalive interval, and release those that have received nothing for the keepalive timeout as if they had disconnected. Release, in the same way, connections whose current write has made no progress for the keepalive timeout: a peer that does not read can receive neither audio nor state. Also release connections that have not said Hello within `hello_timeout`, so that a peer cannot hold a connection slot by keeping a connection alive without joining. Called periodically with the current time; a connection's quiet time is measured from the first sweep that sees it or the last that saw data arrive.
    pub fn sweep(&self, now: Instant, keepalive: &Keepalive, hello_timeout: Duration) {
        let mut expired = Vec::new();
        let mut window_events = Vec::new();
        {
            let mut state = self.lock();
            let HubState {
                connections,
                next_ping,
                ..
            } = &mut *state;
            for (&id, record) in connections.iter_mut() {
                let liveness = &mut record.liveness;
                if liveness.received.swap(false, Ordering::AcqRel) || liveness.last.is_none() {
                    liveness.last = Some(now);
                }
                let quiet = now.saturating_duration_since(liveness.last.unwrap_or(now));
                let opened = *liveness.opened.get_or_insert(now);
                let written = liveness.output.written.load(Ordering::Acquire);
                if liveness.output.pending.load(Ordering::Acquire) && written == liveness.written {
                    liveness.stalled.get_or_insert(now);
                } else {
                    liveness.stalled = None;
                }
                liveness.written = written;
                let stalled = liveness
                    .stalled
                    .is_some_and(|since| now.saturating_duration_since(since) >= keepalive.timeout);
                if !liveness.greeted && now.saturating_duration_since(opened) >= hello_timeout {
                    log::info!("Connection {id} did not say Hello in time.");
                    expired.push((id, liveness.cancellation.clone()));
                } else if quiet >= keepalive.timeout {
                    log::info!("Connection {id} received nothing within the keepalive timeout.");
                    expired.push((id, liveness.cancellation.clone()));
                } else if stalled {
                    log::info!("Connection {id} accepted no output within the keepalive timeout.");
                    expired.push((id, liveness.cancellation.clone()));
                } else if quiet >= keepalive.interval
                    && liveness.pinged.is_none_or(|pinged| {
                        now.saturating_duration_since(pinged) >= keepalive.interval
                    })
                {
                    *next_ping += 1;
                    record.outbox.ping(*next_ping);
                    liveness.pinged = Some(now);
                }
            }
        }
        {
            let timeout = *self.pairing_timeout.read().expect("never left poisoned");
            let knock_timeout = *self.knock_timeout.read().expect("never left poisoned");
            let mut state = self.lock();
            let expired = state.pairing.as_ref().and_then(|pairing| {
                let waited = now.saturating_duration_since(pairing.started);
                if pairing.code.is_none() && waited >= REVEAL_TIMEOUT {
                    Some("The pairing was not completed in time.")
                } else if waited >= timeout {
                    Some("The pairing was not confirmed in time.")
                } else {
                    None
                }
            });
            if let Some(error) = expired {
                log::info!("A pairing was withdrawn: {error}");
                state.end_pairing(None, error);
            }
            self.expire_window(&mut state, now, &mut window_events);
            let (expired, waiting): (Vec<KnockRecord>, Vec<KnockRecord>) =
                std::mem::take(&mut state.knocks)
                    .into_iter()
                    .partition(|knock| {
                        now.saturating_duration_since(knock.started) >= knock_timeout
                    });
            state.knocks = waiting;
            state.knock_cooldowns.retain(|_, until| now < *until);
            for knock in expired {
                state.knock_cooldowns.insert(
                    (knock.participant.clone(), knock.space.clone()),
                    now + KNOCK_COOLDOWN,
                );
                knock.outbox.message(ServerMessage::KnockState {
                    state: tomori_protocol::KnockState::Expired,
                });
                state.announce_knocks(&knock.space);
            }
        }
        self.run(window_events);
        for (id, cancellation) in expired {
            // Release the participant now; cancelling also ends the exchange, even one blocked writing to a peer that is gone.
            self.disconnect(id);
            cancellation.cancel();
        }
        // Panel updates an environment's side made too soon, now that they may be made, and its anchors' changes.
        self.flush_publishes(now);
        self.flush_anchors(now);
        self.sweep_places(now);
        self.sweep_proximity(now);
        self.sweep_zones(now);
        self.sweep_present();
        self.sweep_withdrawn(crate::cards::unix_now());
        // Such as after a closed space was forgotten.
        self.keep();
    }

    /// Forget participants whose last connection ended at least `retention` before `now`. Until then, a participant who reconnects keeps their audio source and rejoins their space; afterwards they are a new participant. See `design/spaces.md`.
    pub fn forget_departed(&self, now: Instant, retention: Duration) {
        let mut state = self.lock();
        let before = state.participants.len();
        state.participants.retain(|_, record| {
            record.connection.is_some()
                || record
                    .departed
                    .is_none_or(|departed| now.saturating_duration_since(departed) < retention)
        });
        let forgotten = before - state.participants.len();
        if forgotten > 0 {
            log::info!("Forgot {forgotten} participants who left more than {retention:?} ago.");
        }
    }

    pub fn disconnect(&self, connection: ConnectionId) {
        let mut deferred = Vec::new();
        {
            let mut state = self.lock();
            state.abandon_utterance(connection, &mut deferred);
            if state
                .pairing
                .as_ref()
                .is_some_and(|pairing| pairing.connection == connection)
            {
                log::info!("A pairing was withdrawn because its device disconnected.");
                state.pairing = None;
            }
            if let Some(record) = state.connections.remove(&connection)
                && let Some(participant) = record.participant
            {
                if record.stray_frames > 0 {
                    log::info!(
                        "{participant} sent {} push-to-talk frames outside any utterance on this connection.",
                        record.stray_frames
                    );
                }
                // A connection superseded by the participant's next names nobody, so this is the participant's current one.
                state.leave(&participant);
                state.next_departure += 1;
                let departure = state.next_departure;
                // A connected participant is kept.
                state.participants.entry(participant).and_modify(|record| {
                    record.connection = None;
                    record.departed = Some(Instant::now());
                    record.departure = departure;
                });
                state.forget_excess_visitors();
            }
            self.recount(&mut state);
            if std::mem::take(&mut state.unkept) {
                deferred.push(Deferred::Keep);
            }
        }
        self.run(deferred);
    }

    /// Apply one frame received from a client.
    pub fn receive(&self, connection: ConnectionId, frame: Frame) {
        match frame.kind {
            FrameKind::Control => match frame.to_client_message() {
                Ok(message) => self.handle(connection, message),
                Err(error) => self.reply_error(connection, format!("Invalid message: {error}")),
            },
            FrameKind::Audio => match frame.to_audio() {
                Ok(audio) => self.audio(connection, audio),
                Err(error) => self.reply_error(connection, format!("Invalid audio: {error}")),
            },
            // Images go only from the server to clients.
            FrameKind::Image => self.reply_error(connection, "Clients send no images.".to_owned()),
        }
    }

    fn reply_error(&self, connection: ConnectionId, error: String) {
        if let Some(record) = self.lock().connections.get(&connection) {
            record.outbox.message(error_message(error));
        }
    }

    fn handle(&self, connection: ConnectionId, message: ClientMessage) {
        match message {
            // Keepalive is answered whether or not the connection has said hello. Receiving it already counted as liveness.
            ClientMessage::Ping { nonce } => {
                if let Some(record) = self.lock().connections.get(&connection) {
                    record.outbox.pong(nonce);
                }
                return;
            }
            ClientMessage::Pong { .. } => return,
            _ => {}
        }
        let mut deferred = Vec::new();
        {
            let mut state = self.lock();
            if let ClientMessage::Hello {
                protocol,
                participant,
                name,
                names,
                environments,
                environment_state,
                projection,
                pairing_window,
                speech_held,
                images,
                diagrams,
                world_edges,
                projection_patches,
                live_panels,
                limits,
            } = message
            {
                // Before Hello rejoins the space they were in, so that they are sent its environment's state and what is projected in it.
                if let Some(record) = state.connections.get_mut(&connection) {
                    record.environment_state = environment_state;
                    record.projection = projection;
                    record.pairing_window = pairing_window;
                    record.speech_held = speech_held;
                    record.images = images;
                    record.diagrams = diagrams;
                    record.world_edges = world_edges;
                    record.patches = projection && projection_patches;
                    record.live_panels = record.patches && live_panels;
                    // The lesser of what it and this server state, an older client's read as today's, whatever it
                    // says, within the most any connection takes; frames each way from the next on. See
                    // `design/projection.md` and `design/server.md`.
                    record.limits = Limits::negotiate(LIMITS, limits);
                    record
                        .liveness
                        .output
                        .set_frame_limit(record.limits.frame_limit());
                    // What it sends, by what it was admitted as, from the next frame on, whatever it reads. See
                    // `design/security.md`.
                    record
                        .liveness
                        .output
                        .set_inbound(Inbound::greeted(&record.access));
                }
                self.hello(
                    &mut state,
                    connection,
                    protocol,
                    (participant, name, names, environments),
                    &mut deferred,
                );
            } else if let Some((participant, space)) = state.member(connection) {
                let role = state.role(connection, space.as_deref());
                match access::refusal(role, &message) {
                    Some(refusal) => state.error(&participant, refusal),
                    None => {
                        // Whatever of where an environment's panels are this changes is told to its side as this
                        // participant's, once it is done, within their rate of intents.
                        state.acting.set(true);
                        self.act(
                            &mut state,
                            connection,
                            &participant,
                            space.as_deref(),
                            message,
                            &mut deferred,
                        );
                        state.acting.set(false);
                        if let Some(space) = state.member(connection).and_then(|(_, space)| space) {
                            state.tell_places_for(&space, &participant);
                        }
                    }
                }
            } else if let Some(record) = state.connections.get(&connection) {
                record.outbox.message(error_message("Send hello first."));
            }
            // Hello connects a participant, and joining or leaving may forget a closed space.
            self.recount(&mut state);
            if std::mem::take(&mut state.unkept) {
                deferred.push(Deferred::Keep);
            }
        }
        self.run(deferred);
    }

    fn hello(
        &self,
        state: &mut HubState,
        connection: ConnectionId,
        protocol: u32,
        (participant, name, names, mut presents): (
            String,
            String,
            bool,
            Option<Vec<EnvironmentVersion>>,
        ),
        deferred: &mut Vec<Deferred>,
    ) {
        let Some(outbox) = state
            .connections
            .get(&connection)
            .map(|record| record.outbox.clone())
        else {
            return;
        };
        if state.connections[&connection].participant.is_some() {
            outbox.message(error_message("This connection already said hello."));
            return;
        }
        if protocol != PROTOCOL_VERSION {
            outbox.message(error_message(format!(
                "Unsupported protocol {protocol}; this server speaks {PROTOCOL_VERSION}."
            )));
            outbox.close();
            return;
        }
        let suggested = if participant.is_empty() {
            Err("A participant identity is required.".to_owned())
        } else {
            limits::suggested_name(&name).and_then(|name| {
                limits::check(
                    &participant,
                    limits::PARTICIPANT_BYTES,
                    "participant identity",
                )
                .map(|()| name)
            })
        };
        let suggested = match suggested {
            Ok(suggested) => suggested,
            Err(error) => {
                outbox.message(error_message(error));
                outbox.close();
                return;
            }
        };
        let access = state.connections[&connection].access.clone();
        // A token holder is called by the name kept with their token, whatever their device suggests. A token without a name the server shows, as no token issued by this server is, takes the suggestion as its name. See `design/security.md`.
        let kept = access
            .name
            .as_deref()
            .and_then(|kept| limits::name(kept).ok());
        if kept.is_none()
            && let Some(principal) = &access.principal
            && let Some(tokens) = self.token_store()
        {
            deferred.push(Deferred::Rename(Rename {
                tokens,
                principal: principal.clone(),
                name: suggested.clone(),
                asked: None,
            }));
        }
        let name = kept.unwrap_or(suggested);
        let tokens = self.token_store();
        // A token holder's tag is their token's; anyone else's is one nobody has, kept while the server keeps them.
        let token_tag = access
            .principal
            .as_deref()
            .zip(tokens.as_deref())
            .and_then(|(principal, tokens)| tokens.entry(principal))
            .and_then(|entry| entry.tag);
        let participant = participant_key(access.principal.as_deref(), &participant);
        let tag = token_tag.or_else(|| {
            state
                .participants
                .get(&participant)
                .and_then(|record| record.tag)
                .or_else(|| state.free_tag(tokens.as_deref()))
        });
        if !state.participants.contains_key(&participant) {
            state.next_source += 1;
            let source = state.next_source;
            state.participants.insert(
                participant.clone(),
                ParticipantRecord {
                    visitor: access.is_visitor(),
                    departure: 0,
                    principal: access.principal.clone(),
                    name: String::new(),
                    tag: None,
                    source,
                    connection: None,
                    space: None,
                    last_space: None,
                    departed: None,
                    engagement: None,
                    intents: crate::channel::Intents::new(Instant::now()),
                    seen: None,
                    headset: false,
                },
            );
        }
        let record = state
            .participants
            .get_mut(&participant)
            .expect("inserted above");
        record.name = name;
        record.tag = tag;
        record.departed = None;
        // A new connection may be from a client that reports nothing, which stays at the live edge, and from another device.
        record.engagement = None;
        record.headset = false;
        let source = record.source;
        let previous = record.connection.replace(connection);
        let rejoin = record.space.clone().or_else(|| record.last_space.clone());
        // A reconnect supersedes the previous connection of the same participant.
        if let Some(previous) = previous.filter(|previous| *previous != connection) {
            state.abandon_utterance(previous, deferred);
            // A participant's connection is listed until it disconnects, which also forgets it here.
            state.connections.entry(previous).and_modify(|old| {
                old.participant = None;
                old.outbox.close();
            });
        }
        let record = state
            .connections
            .get_mut(&connection)
            .expect("checked above");
        record.participant = Some(participant.clone());
        record.names = names;
        // An identifier no environment could have, empty or longer than any the server takes, is skipped before the list is bounded.
        if let Some(presents) = &mut presents {
            presents.retain(|presented| {
                !presented.id.is_empty() && presented.id.len() <= limits::IDENTIFIER_BYTES
            });
            presents.truncate(tomori_protocol::ENVIRONMENT_LIMIT);
        }
        let environments = self.environments().presentable(presents.as_deref()).info();
        record.presents = presents;
        record.liveness.greeted = true;
        let role = access.role(None);
        record.role = Some(role);
        // Only owners create spaces, so only they are told what they could be created with.
        let (backends, roots) = if role == ConnectionRole::Owner {
            let roots = self
                .roots
                .read()
                .expect("workspace roots are never left poisoned")
                .clone();
            (self.backends.info(), roots)
        } else {
            (Vec::new(), Vec::new())
        };
        outbox.message(ServerMessage::Welcome {
            protocol: PROTOCOL_VERSION,
            server: crate::SERVER_NAME.to_owned(),
            source,
            role,
            backends,
            roots,
            environments,
            utterance_limit: self
                .transcriber
                .utterance_limit()
                .map(|limit| limit.as_secs().min(u64::from(u32::MAX)) as u32),
            name: Some(state.participants[&participant].name.clone()),
            // What this server sends at most, which each client is sent within the lesser of and what it states. See
            // `design/projection.md`.
            limits: Some(LIMITS),
        });
        self.tell_window_on_hello(state, connection);
        // The space they were in, unless it was forgotten meanwhile.
        let rejoined = rejoin.is_some_and(|space| state.join(&participant, &space));
        if !rejoined && role != ConnectionRole::Pairing {
            outbox.spaces();
        }
    }

    fn act(
        &self,
        state: &mut HubState,
        connection: ConnectionId,
        participant: &str,
        space: Option<&str>,
        message: ClientMessage,
        deferred: &mut Vec<Deferred>,
    ) {
        let outbox = state.connections[&connection].outbox.clone();
        // Actions on the current space.
        let in_space = |state: &HubState| -> Option<String> {
            match space {
                Some(space) if state.space(space).is_some() => Some(space.to_owned()),
                _ => {
                    state.error(participant, "Join a space first.");
                    None
                }
            }
        };
        match message {
            ClientMessage::Hello { .. } => unreachable!("handled by hello"),
            ClientMessage::Ping { .. } | ClientMessage::Pong { .. } => {
                unreachable!("handled before membership")
            }
            ClientMessage::ListSpaces => outbox.spaces(),
            ClientMessage::SetName { name } => match limits::name(&name) {
                Ok(name) => {
                    state.rename_request(participant, &name, None, self.token_store(), deferred)
                }
                Err(error) => state.error(participant, format!("Name not changed: {error}")),
            },
            ClientMessage::CreateSpace {
                title,
                backend,
                directory,
                session,
                environment,
                instructions,
            } => {
                let instructions = instructions.unwrap_or_default();
                let checked = limits::check(&title, limits::TITLE_BYTES, "title")
                    .and_then(|()| {
                        limits::check(&instructions, limits::INSTRUCTIONS_BYTES, "instructions")
                    })
                    .and_then(|()| limits::check(&directory, limits::DIRECTORY_BYTES, "directory"))
                    .and_then(|()| match &session {
                        Some(session) => limits::check(session, limits::SESSION_BYTES, "session"),
                        None => Ok(()),
                    });
                if let Err(error) = checked {
                    state.error(participant, error);
                    return;
                }
                let Some(found) = self.backends.get(&backend) else {
                    let names: Vec<&str> = self.backends.names().collect();
                    state.error(
                        participant,
                        format!("Unknown backend; available: {}.", names.join(", ")),
                    );
                    return;
                };
                if session.is_some() && !found.capabilities().attach {
                    state.error(
                        participant,
                        format!("The {backend} backend cannot bind an existing session."),
                    );
                    return;
                }
                let environment = match self
                    .environments()
                    .choose_for(environment.as_deref(), state.presents(connection))
                {
                    Ok(environment) => environment,
                    Err(error) => {
                        state.error(participant, error);
                        return;
                    }
                };
                // Refuse early when the limit is already reached. The supervisor checks the directory outside the lock and only then takes a place, so that a refused directory never holds one.
                if self.slots.is_full() {
                    state.error(participant, self.slots.full_error());
                    return;
                }
                let request = CreateRequest {
                    title,
                    backend,
                    directory,
                    session,
                    environment,
                    instructions,
                    creator: participant.to_owned(),
                    creator_space: space.map(str::to_owned),
                };
                if self.jobs.try_send(Job::Create(request)).is_err() {
                    state.error(participant, "The server is shutting down.");
                }
            }
            ClientMessage::ListSessions { backend, directory } => {
                let checked = limits::check(&backend, limits::NAME_BYTES, "backend")
                    .and_then(|()| limits::check(&directory, limits::DIRECTORY_BYTES, "directory"));
                if let Err(error) = checked {
                    state.error(participant, error);
                    return;
                }
                let Some(found) = self.backends.get(&backend) else {
                    let names: Vec<&str> = self.backends.names().collect();
                    state.error(
                        participant,
                        format!("Unknown backend; available: {}.", names.join(", ")),
                    );
                    return;
                };
                if !found.capabilities().attach {
                    state.error(
                        participant,
                        format!("The {backend} backend cannot list existing sessions."),
                    );
                    return;
                }
                if self
                    .jobs
                    .try_send(Job::ListSessions {
                        participant: participant.to_owned(),
                        backend,
                        directory,
                    })
                    .is_err()
                {
                    state.error(participant, "The server is shutting down.");
                }
            }
            ClientMessage::Join { space: target } => {
                if let Err(error) =
                    limits::check(&target, limits::IDENTIFIER_BYTES, "space identifier")
                {
                    state.error(participant, error);
                    return;
                }
                if state.space(&target).is_none() {
                    // Naming the space lets the client tell this refusal from other errors; it is bounded above.
                    state.send(
                        participant,
                        ServerMessage::Error {
                            error: "That space does not exist.".to_owned(),
                            space: Some(target),
                            panel: None,
                            wall: None,
                        },
                    );
                    return;
                }
                // A participant who chooses to join sees what happened while nobody was there, for everyone. A visitor
                // watching does not, and being put back in a space on reconnecting does not count, since what happened
                // meanwhile was not seen. See `design/spaces.md`.
                if state.role(connection, Some(&target)).participates()
                    && let Some(record) = state.space_mut(&target)
                {
                    record.model.attend();
                }
                state.join(participant, &target);
            }
            ClientMessage::Leave => {
                state.abandon_utterance(connection, deferred);
                // Announce leaving before the lobby listing that leaving produces.
                outbox.clear_audio();
                outbox.message(ServerMessage::LEFT);
                state.leave(participant);
                // A member is kept.
                state
                    .participants
                    .entry(participant.to_owned())
                    .and_modify(|record| record.last_space = None);
                state.update_role(participant);
                outbox.spaces();
            }
            ClientMessage::Gate { .. } => {
                // Each audio frame carries its own gate; the announcement needs no state yet.
            }
            ClientMessage::UtteranceBegin {
                utterance,
                answer,
                progress,
                naming,
                titling,
                attention,
                seen,
            } => {
                if naming && titling {
                    state.error(
                        participant,
                        "Words cannot say both your name and the space's title.",
                    );
                    return;
                }
                let checked =
                    limits::check(&utterance, limits::UTTERANCE_BYTES, "utterance identifier")
                        .and_then(|()| match &answer {
                            Some(target) => limits::check(
                                &target.question,
                                limits::IDENTIFIER_BYTES,
                                "question",
                            )
                            .and_then(|()| {
                                limits::check(&target.field, limits::IDENTIFIER_BYTES, "field")
                            }),
                            None => Ok(()),
                        })
                        .and_then(|()| match &seen {
                            Some(message) => {
                                limits::check(message, limits::IDENTIFIER_BYTES, "message")
                            }
                            None => Ok(()),
                        });
                if let Err(error) = checked {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state) else {
                    return;
                };
                // Words for the agent, rather than an answer, a name or a title, turn a following agent to them.
                let question = answer.is_none() && !naming && !titling;
                if let Some(open) = state
                    .connections
                    .get_mut(&connection)
                    .and_then(|record| record.utterance.take())
                {
                    state.utterances.remove(&open.id);
                    deferred.push(Deferred::Cancel(open.id));
                    outbox.message(ServerMessage::TranscriptFailed {
                        utterance: open.client,
                        error: "A new utterance started before this one ended.".to_owned(),
                    });
                }
                // What the participant attends to is captured now, as they begin: only identities are taken from the
                // client, which the space resolves, ignoring those it does not have, cards only on the board of a
                // space that shows them. An answer goes to the question presented, and a name to the participant, so
                // either is about nothing attended to.
                // What they point at among the objects the environment describes, all of which it is resolved among,
                // ranks first in the agent's ambient list for their words, once those are queued. See `crate::ambient`.
                let pointed: Vec<String> = state
                    .space(&space)
                    .filter(|_| question)
                    .and_then(|record| {
                        record
                            .channel
                            .as_ref()
                            .filter(|channel| channel.environment == record.model.environment)
                    })
                    .map(|channel| {
                        attention
                            .iter()
                            .take(tomori_protocol::ATTENTION_LIMIT)
                            .filter_map(|resource| match resource {
                                tomori_protocol::Resource::Object { object, .. }
                                    if channel.objects.get(object).is_some() =>
                                {
                                    Some(object.clone())
                                }
                                _ => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let attention = match state.space(&space) {
                    Some(record) if answer.is_none() && !naming && !titling => {
                        let environments = self.environments();
                        let walls = environments.walls(&record.model.environment);
                        let now = crate::cards::unix_now();
                        record.model.resolve_with(
                            &attention,
                            state.cards_of(&space),
                            record.channel.as_ref(),
                            &|panel, at| record.stage.describe_at(&walls, panel, at, now),
                            &|anchor| record.gathered(&environments, anchor),
                        )
                    }
                    _ => Vec::new(),
                };
                // So is what they had seen of the agent's messages, which words for the agent follow, unless it says
                // more before they arrive.
                let seen = state
                    .space(&space)
                    .filter(|_| answer.is_none() && !naming && !titling)
                    .map(|record| record.model.seen(seen.as_deref()));
                let origin = state.participants[participant]
                    .seen
                    .as_ref()
                    .map(|seen| seen.head);
                state.next_utterance += 1;
                let id = UtteranceId(state.next_utterance);
                state.utterances.insert(
                    id,
                    UtteranceRecord {
                        participant: participant.to_owned(),
                        source: state.participants[participant].source,
                        client: utterance.clone(),
                        space: space.clone(),
                        // Words that name the participant answer nothing, since they are never queued.
                        answer,
                        naming,
                        titling,
                        attention,
                        pointed,
                        seen,
                        origin,
                        progress: progress.then_some(connection),
                        begun: Instant::now(),
                        ended: None,
                        waiting: None,
                        waited: Duration::ZERO,
                    },
                );
                let record = state
                    .connections
                    .get_mut(&connection)
                    .expect("checked above");
                record.straying = false;
                record.failed = None;
                record.utterance = Some(OpenUtterance {
                    client: utterance,
                    id,
                    expected: None,
                });
                // A reply that starts while they speak is held for them until they stop, rather than talking over them.
                state
                    .space_mut(&space)
                    .expect("checked above")
                    .speech
                    .recording(participant);
                if question && let Some(captured) = self.capture(state, &space, participant) {
                    state.agent_pressed(&space, participant, captured, Instant::now());
                }
                deferred.push(Deferred::Begin(id));
            }
            ClientMessage::UtteranceEnd { utterance, .. }
            | ClientMessage::UtteranceCancel { utterance }
                if state.connections[&connection].failed.as_ref() == Some(&utterance) =>
            {
                // Already failed and reported: the client sent this before it heard.
                state
                    .connections
                    .entry(connection)
                    .and_modify(|record| record.failed = None);
            }
            ClientMessage::UtteranceEnd { utterance, .. }
            | ClientMessage::UtteranceCancel { utterance }
                if !state.connections[&connection]
                    .utterance
                    .as_ref()
                    .is_some_and(|open| open.client == utterance) =>
            {
                state.error(participant, "That utterance is not open.");
            }
            ClientMessage::UtteranceEnd { pointed, .. } => {
                let open = state
                    .connections
                    .get_mut(&connection)
                    .and_then(|record| record.utterance.take())
                    .expect("checked by the previous arm");
                // A client from before attention began utterances names what it pointed at as they end instead. That
                // is taken, resolved now as the space resolves attention, only for an utterance that began attending to
                // nothing, for the agent and not an answer or a name, so that it can never replace what was captured
                // as the utterance began.
                let pointed = pointed.and_then(|resource| {
                    state
                        .utterances
                        .get(&open.id)
                        .filter(|record| {
                            record.attention.is_empty()
                                && record.answer.is_none()
                                && !record.naming
                                && !record.titling
                        })
                        .and_then(|record| {
                            state.space(&record.space).map(|space| {
                                let environments = self.environments();
                                let walls = environments.walls(&space.model.environment);
                                let now = crate::cards::unix_now();
                                space.model.resolve_with(
                                    &[resource],
                                    state.cards_of(&record.space),
                                    space.channel.as_ref(),
                                    &|panel, at| space.stage.describe_at(&walls, panel, at, now),
                                    &|anchor| space.gathered(&environments, anchor),
                                )
                            })
                        })
                });
                // They have let go of X, so what was held for them while they spoke plays after a short pause.
                let ended = state
                    .utterances
                    .get(&open.id)
                    .map(|record| (record.participant.clone(), record.space.clone()));
                if let Some((participant, space)) = &ended {
                    state.agent_released(space, participant, Instant::now());
                }
                state.released(ended, Instant::now());
                // Its frames and its end share one ordered stream, so all of its audio has arrived: the participant is told that it is being transcribed, before anything recognition reports.
                if let Some(record) = state.utterances.get_mut(&open.id) {
                    if let Some(attention) = pointed {
                        record.attention = attention;
                    }
                    record.ended = Some(Instant::now());
                    // The end arrives on the connection that began the utterance, as a reconnect closes it.
                    if record.progress.is_some() {
                        outbox.message(ServerMessage::TranscriptProgress {
                            utterance: open.client,
                            stage: tomori_protocol::TranscriptStage::Transcribing,
                            part: None,
                        });
                    }
                }
                deferred.push(Deferred::End(open.id));
            }
            ClientMessage::UtteranceCancel { .. } => {
                state.abandon_utterance(connection, deferred);
            }
            ClientMessage::CancelQueued { entry } => {
                let Some(space) = in_space(state) else {
                    return;
                };
                let record = state.space_mut(&space).expect("checked above");
                match record.model.cancel(&entry) {
                    Ok(()) => state.touch_space(&space),
                    Err(error) => state.error(participant, error),
                }
            }
            ClientMessage::Decide {
                request,
                decision,
                option,
            } => {
                let checked = limits::check(&request, limits::IDENTIFIER_BYTES, "request")
                    .and_then(|()| match &option {
                        Some(option) => limits::check(option, limits::IDENTIFIER_BYTES, "option"),
                        None => Ok(()),
                    });
                if let Err(error) = checked {
                    state.error(participant, error);
                    return;
                }
                let decision = match (decision, option) {
                    (tomori_protocol::Decision::AllowOnce, None) => {
                        tomori_backend::Decision::AllowOnce
                    }
                    (tomori_protocol::Decision::Deny, None) => tomori_backend::Decision::Deny,
                    (tomori_protocol::Decision::Grant, Some(option)) => {
                        tomori_backend::Decision::Grant { option }
                    }
                    (tomori_protocol::Decision::Grant, None) => {
                        state.error(participant, "Choose an option to grant.");
                        return;
                    }
                    (_, Some(_)) => {
                        state.error(participant, "Only a grant decision takes an option.");
                        return;
                    }
                };
                let Some(space) = in_space(state) else {
                    return;
                };
                let record = state.space_mut(&space).expect("checked above");
                let option = match &decision {
                    tomori_backend::Decision::Grant { option } => Some(option.as_str()),
                    _ => None,
                };
                match record.model.claim_approval(&request, option) {
                    Ok(()) => {
                        let sent = record.commands.try_send(SpaceCommand::Decide {
                            request: request.clone(),
                            decision,
                            participant: participant.to_owned(),
                        });
                        if sent.is_err() {
                            record.model.restore_approval(&request);
                            state.error(participant, "The agent session has stopped.");
                        }
                        state.touch_space(&space);
                    }
                    Err(error) => state.error(participant, error),
                }
            }
            ClientMessage::Answer {
                question,
                field,
                text,
            } => {
                let checked = limits::check(&text, limits::ANSWER_BYTES, "answer")
                    .and_then(|()| limits::check(&question, limits::IDENTIFIER_BYTES, "question"))
                    .and_then(|()| limits::check(&field, limits::IDENTIFIER_BYTES, "field"));
                if let Err(error) = checked {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state) else {
                    return;
                };
                if let Err(error) = state.answer(participant, &space, &question, &field, &text) {
                    state.error(participant, error);
                }
            }
            ClientMessage::Presence {
                head,
                left,
                right,
                hands,
                carrying,
                carried,
                rays,
            } => {
                let Some(id) = space.filter(|space| state.space(space).is_some()) else {
                    return;
                };
                let space = state.space(id).expect("found above");
                let source = state.participants[participant].source;
                // The panel carried is the server's to know, and not relayed.
                let seen = crate::presence::Seen::reported(
                    head,
                    left,
                    right,
                    hands.as_deref(),
                    Instant::now(),
                )
                .map(|seen| seen.carrying_all(carrying, carried).showing(rays));
                for other in space.model.members() {
                    if other != participant {
                        state.send(
                            other,
                            ServerMessage::Presence {
                                source,
                                head,
                                left,
                                right,
                                hands: hands.clone(),
                            },
                        );
                    }
                }
                let record = state
                    .participants
                    .get_mut(participant)
                    .expect("a member is a participant");
                record.headset |= seen
                    .as_ref()
                    .is_some_and(crate::presence::Seen::tracks_hands);
                // Where a participant is, for the environment's server side, in its own coordinates, from the poses as
                // the server sounded them, so that nothing a client sends makes a side's checks fail open; a report whose
                // head is no pose tells it nothing. A visitor's presence is refused before it reaches here (`access.rs`).
                let sounded = seen
                    .as_ref()
                    .map(|seen| (seen.head, seen.controllers[0], seen.controllers[1]));
                record.seen = seen;
                let engagement = record.engagement;
                if let Some((head, left, right)) = sounded {
                    let environment = &state.space(id).expect("found above").model.environment;
                    let environments = self.environments();
                    let uncarry =
                        |pose: &tomori_protocol::Pose| environments.uncarry(environment, pose);
                    let presence = tomori_environment::Presence::new(
                        source,
                        uncarry(&head),
                        left.as_ref().map(uncarry),
                        right.as_ref().map(uncarry),
                        Instant::now(),
                    )
                    .with_engagement(engagement);
                    self.server_sides.presence(id, presence);
                }
                // A following agent keeps with whoever it talks with as they move. See `design/environments.md`.
                self.agent_seen(state, id, participant, Instant::now());
            }
            ClientMessage::SetAttribution { enabled } => {
                let Some(space) = in_space(state) else {
                    return;
                };
                state
                    .space_mut(&space)
                    .expect("checked above")
                    .model
                    .set_attribution(enabled);
                state.touch_space(&space);
                deferred.push(Deferred::Keep);
            }
            ClientMessage::PairBegin { commitment } => {
                if tomori_security::pairing::Nonce::parse(&commitment).is_none() {
                    state.error(
                        participant,
                        "A pairing commitment is 64 hexadecimal digits.",
                    );
                    return;
                }
                let failed = |error: &str| ServerMessage::PairFailed {
                    error: error.to_owned(),
                };
                let Some(setup) = self.pairing.read().expect("never left poisoned").clone() else {
                    outbox.message(failed("This server does not pair devices."));
                    return;
                };
                if !self.encrypted.load(Ordering::Relaxed) {
                    outbox.message(failed("Pairing needs a connection over TLS."));
                    return;
                }
                let now = Instant::now();
                // Closed, also on a server with no owner yet, except during a window an owner or the operator opened; refused at once, and not counted against the attempts, so that requests while closed cannot pause pairing for the owner who opens it. See `design/security.md`.
                if !self.pairing_open(state, now) {
                    outbox.message(failed(tomori_protocol::PAIRING_CLOSED));
                    self.refuse_closed(state, now, deferred);
                    return;
                }
                // Refused even to the same connection, which must not get a fresh value from the server once it has seen one; see `design/security.md`.
                if state.pairing.is_some() {
                    outbox.message(failed(
                        "A pairing is already in progress; try again shortly.",
                    ));
                    return;
                }
                while state
                    .pairing_attempts
                    .front()
                    .is_some_and(|began| now.saturating_duration_since(*began) >= PAIRING_WINDOW)
                {
                    state.pairing_attempts.pop_front();
                }
                if state.pairing_attempts.len() >= PAIRING_ATTEMPTS {
                    outbox.message(failed(
                        "Too many pairings began recently; try again in a few minutes.",
                    ));
                    deferred.push(Deferred::PairingPrompt(PairingEvent::Paused));
                    return;
                }
                state.pairing_attempts.push_back(now);
                state.next_pairing += 1;
                let server = tomori_security::pairing::Nonce::generate();
                // The name the device chose enters pairing here, made safe to show once, so that every display of the pairing shows the same; see `design/security.md`.
                let token_name = state.participants[participant].name.clone();
                let device = ChosenName::new(&token_name);
                log::info!("Pairing began from {device}.");
                deferred.push(Deferred::PairingPrompt(PairingEvent::Began {
                    device: device.clone(),
                }));
                state.pairing = Some(PendingPairing {
                    serial: state.next_pairing,
                    connection,
                    outbox: outbox.clone(),
                    setup,
                    device,
                    token_name,
                    commitment: commitment.to_ascii_lowercase(),
                    server,
                    code: None,
                    started: now,
                });
                outbox.message(ServerMessage::PairNonce {
                    nonce: server.to_hex(),
                });
            }
            ClientMessage::PairReveal { nonce } => {
                let Some(pairing) = state
                    .pairing
                    .as_mut()
                    .filter(|pairing| pairing.connection == connection && pairing.code.is_none())
                else {
                    state.error(participant, "No pairing is waiting for that.");
                    return;
                };
                let Some(device) = tomori_security::pairing::Nonce::parse(&nonce)
                    .filter(|device| device.commitment() == pairing.commitment)
                else {
                    state.end_pairing(
                        Some(connection),
                        "The pairing could not be checked; try again.",
                    );
                    return;
                };
                let code = tomori_security::pairing::code(
                    pairing.setup.fingerprint,
                    device,
                    pairing.server,
                );
                let device = pairing.device.clone();
                pairing.code = Some(code);
                outbox.message(ServerMessage::PairWaiting);
                // Never the code: the log and the console may be read by others on the server's machine, such as its agents, and the operator confirms with the code the device shows. See `design/security.md`.
                log::info!(
                    "Pairing request from {device}: it waits to be confirmed with the code its device shows."
                );
                let timeout = *self.pairing_timeout.read().expect("never left poisoned");
                let now = SystemTime::now();
                let began = now
                    .checked_sub(Instant::now().saturating_duration_since(pairing.started))
                    .unwrap_or(now);
                deferred.push(Deferred::PairingPrompt(PairingEvent::Waiting {
                    device,
                    began,
                    // Only a library's timeout too long for any clock could not be added.
                    expires: began.checked_add(timeout).unwrap_or(began),
                }));
            }
            ClientMessage::Knock => {
                let Some(space) = in_space(state) else {
                    return;
                };
                if state.role(connection, Some(&space)).participates() {
                    state.error(participant, "You already take part in this space.");
                    return;
                }
                // One token admits a device as one thing; a collaborator elsewhere asks an owner instead.
                if state.connections[&connection].access.principal.is_some() {
                    state.error(
                        participant,
                        "You are admitted elsewhere on this server; ask an owner to let you in here.",
                    );
                    return;
                }
                let Some(tokens) = self.tokens.read().expect("never left poisoned").clone() else {
                    state.error(participant, "This server does not admit by knocking.");
                    return;
                };
                if !self.encrypted.load(Ordering::Relaxed) {
                    state.error(participant, "Knocking needs a connection over TLS.");
                    return;
                }
                if state
                    .knocks
                    .iter()
                    .any(|knock| knock.participant == participant)
                {
                    state.error(participant, "Your knock is already waiting.");
                    return;
                }
                let now = Instant::now();
                let key = (participant.to_owned(), space.clone());
                if state
                    .knock_cooldowns
                    .get(&key)
                    .is_some_and(|until| now < *until)
                {
                    state.error(participant, "You knocked recently; try again in a minute.");
                    return;
                }
                if state
                    .knocks
                    .iter()
                    .filter(|knock| knock.space == space)
                    .count()
                    >= KNOCKS_PER_SPACE
                {
                    state.error(
                        participant,
                        "Too many knocks are waiting on this space; try again shortly.",
                    );
                    return;
                }
                state.next_knock += 1;
                let knock = KnockRecord {
                    id: format!("knock-{}", state.next_knock),
                    space: space.clone(),
                    participant: participant.to_owned(),
                    connection,
                    outbox: outbox.clone(),
                    tokens,
                    name: state.participants[participant].name.clone(),
                    started: now,
                };
                log::info!("{:?} knocked on {space}.", knock.name);
                state.knocks.push(knock);
                outbox.message(ServerMessage::KnockState {
                    state: tomori_protocol::KnockState::Pending,
                });
                state.announce_knocks(&space);
            }
            ClientMessage::DecideKnock { knock, admit } => {
                if let Err(error) = limits::check(&knock, limits::IDENTIFIER_BYTES, "knock") {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state) else {
                    return;
                };
                // The first decision resolves a knock; later ones find it gone.
                let Some(index) = state
                    .knocks
                    .iter()
                    .position(|waiting| waiting.id == knock && waiting.space == space)
                else {
                    state.error(participant, "That knock was already answered.");
                    return;
                };
                let knock = state.knocks.remove(index);
                state.announce_knocks(&space);
                if admit {
                    deferred.push(Deferred::Admit(knock));
                } else {
                    log::info!("{:?} was declined from {space}.", knock.name);
                    state.knock_cooldowns.insert(
                        (knock.participant.clone(), space),
                        Instant::now() + KNOCK_COOLDOWN,
                    );
                    knock.outbox.message(ServerMessage::KnockState {
                        state: tomori_protocol::KnockState::Declined,
                    });
                }
            }
            ClientMessage::PairCancel => {
                state.end_pairing(Some(connection), "The pairing was cancelled.");
            }
            // Owners only: visitors, collaborators and agents never open pairing. See `design/security.md`.
            ClientMessage::OpenPairing => {
                let name = state.participants[participant].name.clone();
                if let Err(error) = self.open_window(state, Opener::owner(&name), deferred) {
                    state.error(participant, error);
                }
            }
            ClientMessage::ClosePairing => {
                let name = state.participants[participant].name.clone();
                if !self.close_window(state, format!("{name} closed it."), deferred) {
                    state.error(participant, "Pairing is not open.");
                }
            }
            ClientMessage::SetMode { mode } => {
                if let Err(error) = limits::check(&mode, limits::IDENTIFIER_BYTES, "mode") {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state) else {
                    return;
                };
                let record = state.space_mut(&space).expect("checked above");
                if record.model.is_closed() {
                    state.error(participant, "The agent session has stopped.");
                } else if record.model.is_reconnecting() {
                    state.error(participant, crate::space::RECONNECTING);
                } else if !record.model.offers_mode(&mode) {
                    state.error(participant, "The agent does not offer that mode.");
                } else if record
                    .commands
                    .try_send(SpaceCommand::SetMode {
                        mode,
                        participant: participant.to_owned(),
                    })
                    .is_err()
                {
                    state.error(participant, "The agent session has stopped.");
                }
            }
            ClientMessage::SetInstructions { text } => {
                if let Err(error) = limits::check(&text, limits::INSTRUCTIONS_BYTES, "instructions")
                {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state) else {
                    return;
                };
                let record = state.space_mut(&space).expect("checked above");
                match record.model.set_space_instructions(&text) {
                    Ok(()) => {
                        // While the agent has not taken them all, every layer is given again, so that taking this one alone does not hide that.
                        record.instruct(record.model.instructions_failed());
                        state.touch_space(&space);
                        deferred.push(Deferred::Keep);
                    }
                    Err(error) => state.error(participant, error),
                }
            }
            ClientMessage::Replay { message } => {
                if let Some(message) = &message
                    && let Err(error) = limits::check(message, limits::IDENTIFIER_BYTES, "message")
                {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state) else {
                    return;
                };
                let record = state.space_mut(&space).expect("checked above");
                match record.speech.replay(participant, message.as_deref()) {
                    Ok(()) => record.wake(),
                    Err(error) => state.error(participant, error),
                }
            }
            ClientMessage::StopReplay => {
                // Stopping is harmless when nothing plays, so a stop that crosses the end of a replay is not refused.
                if let Some(record) = space.and_then(|space| state.space_mut(space))
                    && record.speech.stop(participant)
                {
                    // A later reply waits for them, and is heard from its start now.
                    record.wake();
                }
            }
            ClientMessage::Engagement { state: engagement } => {
                // A visitor's engagement is ignored: visitors follow live speech and show no presence. See `design/security.md`.
                if !state.role(connection, space).participates() {
                    return;
                }
                // A member is kept; only a change is applied.
                let Some(record) = state
                    .participants
                    .get_mut(participant)
                    .filter(|record| record.engagement != Some(engagement))
                else {
                    return;
                };
                log::debug!("Participant {} is now {engagement:?}.", record.source);
                record.engagement = Some(engagement);
                let source = record.source;
                if let Some(space) = space {
                    // At once, whatever the presence throttle, since a headset lifted off says it is away and then stops
                    // reporting. See `tomori_environment::Space::presence`.
                    self.server_sides.engagement(space, source, engagement);
                }
                if let Some(space) = space
                    && let Some(record) = state.space_mut(space)
                {
                    if record.speech.engagement(participant, engagement)
                        || record.speech.held_changed()
                    {
                        record.wake();
                    }
                    state.touch_space(space);
                }
            }
            ClientMessage::SetEnvironment { environment } => {
                if let Err(error) =
                    limits::check(&environment, limits::IDENTIFIER_BYTES, "environment")
                {
                    state.error(participant, error);
                    return;
                }
                // Only an environment offered to the requester: anyone else in the space who cannot present it shows the grid and says what it lacks. See `design/environments.md`.
                let chosen = self
                    .environments()
                    .choose_for(Some(&environment), state.presents(connection));
                let environment = match chosen {
                    Ok(environment) => environment,
                    Err(error) => {
                        state.error(participant, error);
                        return;
                    }
                };
                let Some(space) = in_space(state) else {
                    return;
                };
                match self.move_space(state, &space, environment) {
                    Ok(true) => {
                        log::info!(
                            "Space {space} is now in the {} environment.",
                            state
                                .space(&space)
                                .expect("checked above")
                                .model
                                .environment
                        );
                        // It is kept across a restart.
                        deferred.push(Deferred::Keep);
                    }
                    Ok(false) => {}
                    Err(error) => state.error(participant, error),
                }
            }
            ClientMessage::DeleteSpace {
                space: target,
                session,
            } => {
                if let Err(error) =
                    limits::check(&target, limits::IDENTIFIER_BYTES, "space identifier")
                {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state) else {
                    return;
                };
                // Only the space the owner is in, and named as the one they saw, so that a request meant for one space never deletes another. See `design/spaces.md`.
                if target != space {
                    state.error(participant, "Only the space you are in can be deleted.");
                    return;
                }
                let record = state.space(&space).expect("checked above");
                let (backend, bound) = (record.model.backend.clone(), record.model.session.clone());
                if session {
                    // Refused whole, deleting nothing, so that the owner can choose again.
                    let refusal = if !self
                        .backends
                        .get(&backend)
                        .is_some_and(|found| found.capabilities().delete)
                    {
                        Some(format!(
                            "The {backend} backend cannot delete its sessions, so nothing was deleted."
                        ))
                    } else if state.is_bound(&backend, &bound, Some(&space)) {
                        Some("Its agent session is bound to another space too, so nothing was deleted.".to_owned())
                    } else {
                        None
                    };
                    if let Some(refusal) = refusal {
                        state.error(participant, refusal);
                        return;
                    }
                }
                let asker = session.then(|| (participant.to_owned(), connection));
                self.delete(state, participant, &space, asker, deferred);
            }
            ClientMessage::AddWall { pose } => {
                let Some(space) = in_space(state) else {
                    return;
                };
                let environments = self.environments();
                let record = state.space_mut(&space).expect("checked above");
                let walls = environments.walls(&record.model.environment);
                match record.stage.add_wall(&walls, &pose, None) {
                    Ok(_) => {
                        state.touch_projection(&space);
                        deferred.push(Deferred::Keep);
                    }
                    Err(error) => state.error(participant, error),
                }
            }
            ClientMessage::SetWall {
                wall,
                grid,
                scale,
                aspect,
                shape,
                pose,
                tilt,
                content_scale,
                revision,
            } => {
                if let Err(error) = limits::check(&wall, limits::IDENTIFIER_BYTES, "wall") {
                    state.error(participant, error);
                    return;
                }
                let change = match crate::stage::WallChange::new(
                    grid,
                    scale,
                    aspect,
                    shape,
                    pose,
                    tilt,
                    content_scale,
                ) {
                    Ok(change) => change,
                    Err(error) => {
                        state.error(participant, error);
                        return;
                    }
                };
                let Some(space) = in_space(state) else {
                    return;
                };
                let environments = self.environments();
                let record = state.space_mut(&space).expect("checked above");
                // Changed since the version they saw, as by another participant's change, it is refused, naming the
                // wall, so that their client cues it, rather than undo the other change; one that says none, as a
                // carry's or an older client's, goes by what it says alone.
                let shown = record
                    .versioned(&environments, crate::cards::unix_now())
                    .wall(&wall)
                    .map(|shown| shown.version);
                if let Some(seen) = revision
                    && shown.is_some_and(|shown| shown != seen)
                {
                    state.send(
                        participant,
                        ServerMessage::Error {
                            error: crate::stage::STALE_WALL.to_owned(),
                            space: None,
                            panel: None,
                            wall: Some(wall),
                        },
                    );
                    state.touch_projection(&space);
                    return;
                }
                let walls = environments.walls(&record.model.environment);
                match record.stage.set_wall(&walls, &wall, change) {
                    Ok(true) => {
                        state.touch_projection(&space);
                        deferred.push(Deferred::Keep);
                    }
                    // Already so, it is sent again, so that a client showing it otherwise meanwhile shows it as it is.
                    Ok(false) => state.touch_projection(&space),
                    Err(error) => {
                        state.error(participant, error);
                        // Sent again, so that a client that showed the change at once shows the wall as it is.
                        state.touch_projection(&space);
                    }
                }
            }
            ClientMessage::AttachWall {
                wall,
                attach,
                alone,
                pose,
            } => {
                let checked =
                    limits::check(&wall, limits::IDENTIFIER_BYTES, "wall").and_then(|()| {
                        attach.as_ref().map_or(Ok(()), |attach| {
                            limits::check(&attach.to, limits::IDENTIFIER_BYTES, "wall")
                        })
                    });
                if let Err(error) = checked {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state) else {
                    return;
                };
                let environments = self.environments();
                let record = state.space_mut(&space).expect("checked above");
                let walls = environments.walls(&record.model.environment);
                match record.stage.attach_wall(&walls, &wall, attach, alone, pose) {
                    Ok(true) => {
                        state.touch_projection(&space);
                        deferred.push(Deferred::Keep);
                    }
                    // Sent again either way, so that a client that showed the snap at once shows the walls as they are.
                    Ok(false) => state.touch_projection(&space),
                    Err(error) => {
                        state.error(participant, error);
                        state.touch_projection(&space);
                    }
                }
            }
            ClientMessage::LockWall {
                wall,
                locked,
                reset,
            } => {
                if let Err(error) = limits::check(&wall, limits::IDENTIFIER_BYTES, "wall") {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state) else {
                    return;
                };
                let environments = self.environments();
                let record = state.space_mut(&space).expect("checked above");
                let walls = environments.walls(&record.model.environment);
                match record.stage.lock_wall(&walls, &wall, locked, reset) {
                    Ok(true) => {
                        log::info!(
                            "An owner {} {wall} in space {space}.",
                            match (reset, locked) {
                                (true, _) => "put back and locked",
                                (false, true) => "locked",
                                (false, false) => "unlocked",
                            }
                        );
                        state.touch_projection(&space);
                        deferred.push(Deferred::Keep);
                    }
                    Ok(false) => state.touch_projection(&space),
                    Err(error) => {
                        state.error(participant, error);
                        state.touch_projection(&space);
                    }
                }
            }
            ClientMessage::RemoveWall { wall } => {
                if let Err(error) = limits::check(&wall, limits::IDENTIFIER_BYTES, "wall") {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state) else {
                    return;
                };
                let environments = self.environments();
                let record = state.space_mut(&space).expect("checked above");
                let walls = environments.walls(&record.model.environment);
                match record.stage.remove_wall(&walls, &wall) {
                    Ok(()) => {
                        state.touch_projection(&space);
                        deferred.push(Deferred::Keep);
                    }
                    Err(error) => state.error(participant, error),
                }
            }
            ClientMessage::PlacePanel {
                panel,
                wall,
                slot,
                pose,
                size,
                revision,
            } => {
                let checked =
                    limits::check(&panel, limits::IDENTIFIER_BYTES, "panel").and_then(|()| {
                        wall.as_deref().map_or(Ok(()), |wall| {
                            limits::check(wall, limits::IDENTIFIER_BYTES, "wall")
                        })
                    });
                if let Err(error) = checked {
                    state.error(participant, error);
                    return;
                }
                let target = match (wall, slot, pose) {
                    (Some(wall), Some(slot), _) => crate::stage::Target::Slot(wall, slot),
                    (Some(wall), None, _) => crate::stage::Target::Wall(wall),
                    (None, _, Some(pose)) => crate::stage::Target::Free(pose),
                    (None, _, None) => {
                        state.error(participant, "Say where to put the panel.");
                        return;
                    }
                };
                let Some(space) = in_space(state) else {
                    return;
                };
                let environments = self.environments();
                let record = state.space_mut(&space).expect("checked above");
                // Stretched or shrunk with both hands, as a panel left floating, the change is refused where the panel
                // changed since the version the participant saw as they took hold of it with the second hand, as by the
                // agent rewriting it or another participant moving it, rather than lose that change. See
                // `design/projection.md`.
                let size = size.filter(|_| matches!(target, crate::stage::Target::Free(_)));
                let stale = size.is_some()
                    && revision.is_some_and(|seen| {
                        record
                            .versioned(&environments, crate::cards::unix_now())
                            .panel(&panel)
                            .is_some_and(|shown| shown.version != seen)
                    });
                let walls = environments.walls(&record.model.environment);
                let placed = if stale {
                    Err(crate::stage::STALE_SIZE)
                } else {
                    record.stage.place_sized(&walls, &panel, target, size)
                };
                match placed {
                    Ok(true) => {
                        state.touch_projection(&space);
                        deferred.push(Deferred::Keep);
                    }
                    // Already there, it is sent again, so that a client showing it elsewhere meanwhile puts it back.
                    Ok(false) => state.touch_projection(&space),
                    // Refused as made against a version since changed, it names the panel, so that the client shows it as
                    // the server has it, and the projection is sent again.
                    Err(error) if stale => {
                        state.send(
                            participant,
                            ServerMessage::Error {
                                error: error.to_owned(),
                                space: None,
                                panel: Some(panel),
                                wall: None,
                            },
                        );
                        state.touch_projection(&space);
                    }
                    Err(error) => state.error(participant, error),
                }
            }
            // A client that could not apply a patch is sent the projection whole, at the space's revision now; one in no
            // space, or that reads no projection, is sent nothing, as a patch on its way as it left asks innocently. So is
            // one already waiting for it whole, as on joining or having asked, so that asking again and again is sent it
            // once, and one that applies no patches, which is sent it whole at every change anyway.
            ClientMessage::ResyncProjection { .. } => {
                if space.is_some()
                    && let Some(record) = state
                        .connections
                        .get_mut(&connection)
                        .filter(|record| record.projection && record.patches)
                    && record.projected.take().is_some()
                {
                    log::debug!("A client asked for the projection whole.");
                    record.outbox.projection();
                }
            }
            // Only an image a panel shown in the client's space has: what a client may see is what its space shows. A
            // refusal is not said, since it happens innocently, as when the panel is dismissed while its image is asked
            // for, and the participant can do nothing about it; the client gives up after a few tries.
            ClientMessage::FetchImage { image } => {
                let environments = self.environments();
                let shown = tomori_protocol::hash_bytes(&image).filter(|_| {
                    space
                        .and_then(|space| state.space(space))
                        .is_some_and(|record| {
                            let walls = environments.walls(&record.model.environment);
                            record.stage.shows_image(&walls, &image)
                        })
                });
                match shown {
                    Some(hash) => deferred.push(Deferred::Image(connection, hash)),
                    None => {
                        log::debug!("Refused an image that no panel the client's space shows has.")
                    }
                }
            }
            ClientMessage::DismissPanel { panel } => {
                if let Err(error) = limits::check(&panel, limits::IDENTIFIER_BYTES, "panel") {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state) else {
                    return;
                };
                let record = state.space_mut(&space).expect("checked above");
                let environments = self.environments();
                let walls = environments.walls(&record.model.environment);
                let dismissed = record.stage.dismiss(&walls, &panel);
                drop(environments);
                match dismissed {
                    Ok(()) => {
                        state.touch_projection(&space);
                        deferred.push(Deferred::Keep);
                    }
                    Err(error) => state.error(participant, error),
                }
            }
            // A participant ticks or unticks a checklist item they point at, for everyone, by the same rule as
            // `update_panel`, and the agent is told before the next words spoken in the space, without a turn of its own.
            ClientMessage::CheckPanelItem {
                panel,
                line,
                checked,
                revision,
            } => {
                if let Err(error) = limits::check(&panel, limits::IDENTIFIER_BYTES, "panel") {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state) else {
                    return;
                };
                let who = state.participants[participant].name.clone();
                let environments = self.environments();
                let record = state.space_mut(&space).expect("checked above");
                // Changed since the version they saw, as by the agent's rewrite, another participant's tick or a move,
                // the tick is refused, rather than tick whatever item is on that line now. An older client says none.
                let shown = record
                    .versioned(&environments, crate::cards::unix_now())
                    .panel(&panel)
                    .map(|shown| shown.version);
                let ticked = match revision {
                    Some(seen) if shown.is_some_and(|shown| shown != seen) => {
                        Err(crate::stage::STALE_TICK)
                    }
                    _ => {
                        let walls = environments.walls(&record.model.environment);
                        record.stage.tick_item(&walls, &panel, line, checked)
                    }
                };
                match ticked {
                    Ok(Some(ticked)) => {
                        record.model.note_tick(&panel, line, ticked.note(&who));
                        log::info!(
                            "A participant {} item {} of {panel} in space {space}.",
                            if checked { "ticked" } else { "unticked" },
                            ticked.number
                        );
                        state.touch_projection(&space);
                        deferred.push(Deferred::Keep);
                    }
                    // Already so, it is sent again, so that a client that showed the change at once shows it as it is.
                    Ok(None) => state.touch_projection(&space),
                    // Refused, as when the panel changed since the revision they saw, it names the panel, so that their
                    // client cues it, and the projection is sent again, so that they see the panel as it is and look again.
                    Err(error) => {
                        state.send(
                            participant,
                            ServerMessage::Error {
                                error: error.to_owned(),
                                space: None,
                                panel: Some(panel),
                                wall: None,
                            },
                        );
                        state.touch_projection(&space);
                    }
                }
            }
            // A participant chooses a row of an environment's list panel, which is handed to that environment's server side
            // as a choice only the server makes, by the row's id, never its place, within the participant's rate of
            // intents. See `design/panel-actions.md`.
            ClientMessage::ChoosePanelRow {
                panel,
                row,
                revision,
                key,
            } => {
                if let Err(error) = limits::check(&panel, limits::IDENTIFIER_BYTES, "panel") {
                    state.error(participant, error);
                    return;
                }
                if !tomori_environment::valid_row_id(&row) {
                    state.error(participant, "That is not a row's id.");
                    return;
                }
                let Some(space) = in_space(state) else {
                    return;
                };
                let environments = self.environments();
                let record = state.space(&space).expect("checked above");
                let shown = record
                    .versioned(&environments, crate::cards::unix_now())
                    .panel(&panel)
                    .map(|shown| shown.version);
                let walls = environments.walls(&record.model.environment);
                let chosen = record
                    .stage
                    .chosen(&walls, &panel, &row)
                    .and_then(|(id, chosen)| {
                        // Chosen from the panel as it is now, unless it changed since the version they saw; a client
                        // that says none is taken at its word.
                        let current = revision.is_none_or(|seen| shown == Some(seen));
                        // A row of a list with keys is handed on with its key only where that is the key the participant
                        // saw: by the revision of its key they say, or, from a client that says none, by the panel's version,
                        // or at its word where it says neither; otherwise it is refused as stale, never resolved to another
                        // key. A client never sends a key, so none is forged.
                        let (revised, fields) =
                            record.stage.row_key(walls.environment, &panel, &row);
                        let seen = match (revised, key) {
                            (None, _) => true,
                            (Some(revised), Some(seen)) => revised == seen,
                            (Some(_), None) => current,
                        };
                        if !seen {
                            return Err(crate::stage::STALE_ROW);
                        }
                        Ok(tomori_environment::Chosen::new(id, chosen, current)
                            .with_fields(fields.to_vec()))
                    });
                match chosen {
                    Ok(chosen) => {
                        let record = state
                            .participants
                            .get_mut(participant)
                            .expect("the participant acting is listed");
                        let taken = record.intents.take(Instant::now());
                        let source = record.source;
                        let said = format!("row {} of {}", chosen.row(), chosen.panel());
                        // Logged only once taken, so that a flood past the rate fills no log.
                        if taken
                            && self
                                .server_sides
                                .deliver(&space, tomori_environment::Intent::choice(source, chosen))
                        {
                            log::info!("A participant chose {said} in space {space}.");
                        } else {
                            log::debug!("A choice of {said} was dropped.");
                        }
                    }
                    Err(error) => state.send(
                        participant,
                        ServerMessage::Error {
                            error: error.to_owned(),
                            space: None,
                            panel: Some(panel),
                            wall: None,
                        },
                    ),
                }
            }
            ClientMessage::WatchCards => {
                let Some(space) = in_space(state) else {
                    return;
                };
                if !state.shows_cards(&space) {
                    state.error(participant, "This space shows no cards.");
                    return;
                }
                let record = state
                    .connections
                    .get_mut(&connection)
                    .expect("the connection acting is listed");
                record.cards = true;
                record.outbox.cards();
                self.cards_watched.notify(usize::MAX);
            }
            // Ambient, so dropped quietly, as the server side would drop an intent it does not take: outside a space whose environment is the one named at its version, too large, or too soon after the last. The side validates the rest. See `design/environments.md`.
            ClientMessage::EnvironmentIntent {
                environment,
                version,
                intent,
            } => {
                let current = space
                    .and_then(|space| state.space(space))
                    .is_some_and(|record| {
                        record.model.environment == environment
                            && self.environments().version(&environment) == Some(version)
                    });
                let text = intent.text();
                let now = Instant::now();
                // The participant's own bucket, kept across their connections.
                let record = state
                    .participants
                    .get_mut(participant)
                    .expect("the participant acting is listed");
                let taken = current
                    && text.len() <= tomori_protocol::MESSAGE_LIMIT
                    && record.intents.take(now);
                let source = record.source;
                if !(taken
                    && self.server_sides.deliver(
                        space.expect("checked above"),
                        tomori_environment::Intent::new(source, text),
                    ))
                {
                    log::debug!("An environment intent was dropped.");
                }
            }
            ClientMessage::PlaceCard { card, placement } => {
                if let Err(error) = limits::check(&card, limits::IDENTIFIER_BYTES, "card") {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state) else {
                    return;
                };
                if !state.shows_cards(&space) {
                    state.error(participant, "This space shows no cards.");
                    return;
                }
                let HubState { spaces, board, .. } = &mut *state;
                let board = board.as_ref().expect("checked above");
                let record = spaces
                    .iter_mut()
                    .find(|record| record.model.id == space)
                    .expect("checked above");
                match crate::cards::place(&mut record.placements, board, &card, placement) {
                    Ok(true) => {
                        // Everyone watching sees it move, and the layout is kept with the space.
                        state.touch_cards(Some(&space));
                        deferred.push(Deferred::Keep);
                    }
                    Ok(false) => {}
                    Err(error) => state.error(participant, error),
                }
            }
            ClientMessage::RenameSpace { title } => {
                let title = match limits::title(&title) {
                    Ok(title) => title,
                    Err(error) => {
                        state.error(participant, format!("Title not changed: {error}"));
                        return;
                    }
                };
                let Some(space) = in_space(state) else {
                    return;
                };
                let by = state.participants[participant].name.clone();
                state.rename_space(
                    &space,
                    title,
                    tomori_protocol::Renamer::Participant { name: by },
                );
                deferred.push(Deferred::Keep);
            }
            ClientMessage::PlaceAgent { at: Some(_) } => {
                // The trigger no longer summons the agent: one from a client that ignores `summons` changes nothing.
                log::debug!("A summons from {participant} was dropped, as this server keeps none.");
            }
            ClientMessage::PlaceAgent { at: None } => {
                let Some(space) = in_space(state) else {
                    return;
                };
                let name = state.participants[participant].name.clone();
                state.agent_home(&space, &format!("{name} sent it home"), Instant::now());
            }
            ClientMessage::CallAgent => {
                let Some(space) = in_space(state) else {
                    return;
                };
                if let Err(error) = self.call_agent(state, &space, participant, Instant::now()) {
                    state.error(participant, error);
                }
            }
            ClientMessage::PinAgent { pinned } => {
                let Some(space) = in_space(state) else {
                    return;
                };
                if let Err(error) = state.pin_agent(&space, participant, pinned, Instant::now()) {
                    state.error(participant, error);
                }
            }
            ClientMessage::WalkAgent { walking } => {
                let Some(space) = in_space(state) else {
                    return;
                };
                let now = Instant::now();
                let done = if walking {
                    let name = state.participants[participant].name.clone();
                    self.walk_with(
                        state,
                        &space,
                        participant,
                        &format!(", as {name} asked from the hand menu"),
                        now,
                    )
                } else {
                    state.stop_walking(&space, participant, now)
                };
                if let Err(error) = done {
                    state.error(participant, error);
                }
            }
            ClientMessage::SetAgentMovement { movement } => {
                let Some(space) = in_space(state) else {
                    return;
                };
                let record = state.space_mut(&space).expect("checked above");
                match record.model.set_movement(movement) {
                    Ok(changed) => {
                        if changed {
                            let movement = record.model.movement();
                            let name = state.participants[participant].name.clone();
                            log::info!(
                                "{name} set the agent of space {space} to move {:?}, going home after {:?} s.",
                                movement.mode,
                                movement.return_after
                            );
                            state.movement_changed(&space, &name, Instant::now());
                            state.touch_space(&space);
                            deferred.push(Deferred::Keep);
                        }
                    }
                    Err(error) => state.error(participant, error),
                }
            }
            ClientMessage::SetSpeech { speech } => {
                let Some(space) = in_space(state) else {
                    return;
                };
                let record = state.space_mut(&space).expect("checked above");
                match record.model.set_speech(speech) {
                    Ok(()) => {
                        state.touch_space(&space);
                        deferred.push(Deferred::Keep);
                    }
                    Err(error) => state.error(participant, error),
                }
            }
        }
    }

    /// Delete `space`, which `participant`, an owner, is in, from the server: everyone in it is returned to the lobby, told who deleted it, and it is no longer listed, kept, projected or spoken in. Its session is closed, and then deleted if `asker` asked for that, or else kept with Tomori's instruction entries removed. See `design/spaces.md`.
    fn delete(
        &self,
        state: &mut HubState,
        participant: &str,
        space: &str,
        asker: Option<(String, ConnectionId)>,
        deferred: &mut Vec<Deferred>,
    ) {
        let (by, source) = state
            .participants
            .get(participant)
            .map(|record| (record.name.clone(), record.source))
            .expect("the participant acting is listed");
        let left = ServerMessage::Left {
            deleted: Some(tomori_protocol::SpaceDeletion {
                by,
                session: asker.is_some(),
            }),
        };
        let members: Vec<String> = state
            .space(space)
            .expect("the space acted on is listed")
            .model
            .members()
            .cloned()
            .collect();
        // Everyone in it leaves it, as they would by leaving, and is told why before the listing that follows.
        for member in &members {
            let connection = state.participants[member]
                .connection
                .expect("members are connected, since disconnecting leaves the space");
            state.abandon_utterance(connection, deferred);
            let outbox = &state.connections[&connection].outbox;
            outbox.clear_audio();
            outbox.message(left.clone());
            state.depart(member);
            state.update_role(member);
        }
        // Nobody rejoins it on reconnecting.
        for record in state.participants.values_mut() {
            record.last_space = record.last_space.take().filter(|last| last != space);
        }
        let index = state
            .spaces
            .iter()
            .position(|record| record.model.id == space)
            .expect("departing forgets no space");
        let record = state.spaces.remove(index);
        let with = if asker.is_some() {
            ", with its session"
        } else {
            ""
        };
        log::info!(
            "Space {space} {:?} was deleted by participant {source}{with}.",
            record.model.title
        );
        // Its environment's server side stops, and what it kept goes with the record, as do its walls, panels and speech.
        self.reconcile_server_sides(state);
        // Nor needs Tomori's tools, unless another space in its directory does. See `design/mcp.md`.
        self.tools_due(
            state.shutting_down,
            &record.model.backend,
            &record.model.directory,
        );
        state.unkept = true;
        // Everyone in the lobby, those it was deleted under among them, is sent the listing without it.
        state.touch_lobby();
        // Its driver closes its session, then drops the deletion; a space with no driver left drops it at once.
        let deletion = Deletion {
            jobs: self.jobs.clone(),
            deleted: Deleted {
                backend: record.model.backend.clone(),
                session: record.model.session.clone(),
                asker,
            },
        };
        let _ = record.commands.try_send(SpaceCommand::Delete(deletion));
    }

    /// Whether a listed space is bound to `session` of `backend`.
    pub(crate) fn binds(&self, backend: &str, session: &str) -> bool {
        self.lock().is_bound(backend, session, None)
    }

    /// Tell the owner who asked to delete a space's session what became of it, if they are still on the connection that asked.
    pub(crate) fn session_deleted(
        &self,
        (participant, connection): &(String, ConnectionId),
        result: Result<(), String>,
    ) {
        let state = self.lock();
        if state
            .participants
            .get(participant)
            .is_some_and(|record| record.connection == Some(*connection))
        {
            state.send(
                participant,
                ServerMessage::SessionDeleted {
                    error: result
                        .err()
                        .map(|error| limits::truncate(&error, limits::ERROR_BYTES)),
                },
            );
        }
    }

    /// Relay a participant's audio to the rest of their space, and hand push-to-talk audio to recognition.
    fn audio(&self, connection: ConnectionId, frame: AudioFrame) {
        let mut deferred = Vec::new();
        {
            let mut state = self.lock();
            let Some(record) = state.connections.get_mut(&connection) else {
                return;
            };
            let gap = match record.utterance.as_mut() {
                Some(open) if frame.gate == Gate::Agent => {
                    let gap = open
                        .expected
                        .is_some_and(|expected| expected != frame.sequence);
                    open.expected = Some(frame.sequence.wrapping_add(1));
                    gap
                }
                Some(open) => {
                    // Only agent audio is recognized, so only a gap between agent frames loses part of the utterance.
                    open.expected = None;
                    false
                }
                None => false,
            };
            let Some((participant, Some(space))) = state.member(connection) else {
                return;
            };
            // Visitors send no audio; anything they send is neither relayed nor transcribed. See `design/security.md`.
            if !state.role(connection, Some(&space)).participates() {
                log::debug!(
                    "Dropped audio from {participant}, who does not participate in {space}."
                );
                return;
            }
            let record = state
                .connections
                .get_mut(&connection)
                .expect("member has a connection");
            if frame.gate == Gate::Off {
                return;
            }
            if frame.gate == Gate::Agent && record.utterance.is_none() {
                record.stray_frames += 1;
                if !record.straying {
                    record.straying = true;
                    log::warn!(
                        "Push-to-talk audio from {participant} arrived outside an utterance; it is relayed but not transcribed."
                    );
                }
            }
            if frame.gate == Gate::Agent
                && let Some(open) = record.utterance.as_mut()
            {
                if gap {
                    let open = record.utterance.take().expect("matched above");
                    record.failed = Some(open.client.clone());
                    let ended = state.utterances.remove(&open.id);
                    state.recorded(ended.map(UtteranceRecord::recorder));
                    deferred.push(Deferred::Cancel(open.id));
                    state.send(
                        &participant,
                        ServerMessage::TranscriptFailed {
                            utterance: open.client,
                            error: "Some of the recording was lost, so nothing was sent. Please try again."
                                .to_owned(),
                        },
                    );
                } else {
                    deferred.push(Deferred::Push(open.id, frame.clone()));
                }
            }
            let source = state.participants[&participant].source;
            // The relayed frame is the size of the frame received, which was within the limit, and an occupied space is listed. Members are connected, since disconnecting leaves the space. Should any of that change, the frame is not relayed to whom it cannot be.
            let encoded = Frame::audio(&AudioFrame { source, ..frame }).try_encode();
            for (outbox, encoded) in state
                .space(&space)
                .into_iter()
                .flat_map(|space| space.model.members())
                .filter(|other| **other != participant)
                .filter_map(|other| state.outbox(other))
                .flat_map(|outbox| encoded.iter().map(move |encoded| (outbox, encoded)))
            {
                outbox.audio(encoded.clone());
            }
        }
        self.run(deferred);
    }

    pub(crate) fn transcript_partial(&self, utterance: UtteranceId, text: &str) {
        let state = self.lock();
        if let Some(record) = state.utterances.get(&utterance) {
            state.send(
                &record.participant,
                ServerMessage::Transcript {
                    utterance: record.client.clone(),
                    text: limits::truncate(text, limits::QUEUED_TEXT_BYTES),
                    finished: false,
                },
            );
        }
    }

    /// What recognition is doing with an utterance that has ended, told to its participant if their client reads it.
    pub(crate) fn transcript_progress(
        &self,
        utterance: UtteranceId,
        stage: tomori_protocol::TranscriptStage,
        part: Option<tomori_protocol::TranscriptPart>,
    ) {
        let mut state = self.lock();
        let state = &mut *state;
        let Some(record) = state.utterances.get_mut(&utterance) else {
            return;
        };
        let now = Instant::now();
        if stage == tomori_protocol::TranscriptStage::Waiting {
            record.waiting.get_or_insert(now);
        } else if let Some(since) = record.waiting.take() {
            record.waited += now.saturating_duration_since(since);
        }
        // Only the connection that asked is told: a later one of the participant's may be an older client's. One superseded by a reconnect is closed, and so ignores it, and one that has gone is no longer listed.
        let Some(connection) = record
            .progress
            .and_then(|connection| state.connections.get(&connection))
        else {
            return;
        };
        connection
            .outbox
            .message(ServerMessage::TranscriptProgress {
                utterance: record.client.clone(),
                stage,
                part,
            });
    }

    pub(crate) fn transcript_finished(&self, utterance: UtteranceId, text: &str) {
        let mut state = self.lock();
        let Some(record) = state.utterances.remove(&utterance) else {
            return;
        };
        let text = text.trim();
        // Recognition reports an utterance with no speech in it as empty, whatever the backend; see `design/speech.md`.
        record.log_outcome(
            utterance,
            if text.is_empty() {
                "was found to have no speech in it, so nothing was sent,"
            } else {
                "was transcribed"
            },
        );
        // Words with nothing in them move a following agent nowhere: a move begun for them stops where it is.
        if text.is_empty() {
            state.agent_cancelled(&record.space, &record.participant, Instant::now());
        }
        // Words that say the participant's new name rename them, wherever they are now, and are never queued. Renaming again to the same name, as a repeated finalization does, changes nothing. Words with no speech in them fail below, as any others do.
        if record.naming && !text.is_empty() {
            match limits::name(spoken_name(text)) {
                Ok(name) => {
                    let mut deferred = Vec::new();
                    let tokens = self.token_store();
                    state.rename_request(
                        &record.participant,
                        &name,
                        Some(record.client),
                        tokens,
                        &mut deferred,
                    );
                    drop(state);
                    self.run(deferred);
                }
                Err(error) => state.send(
                    &record.participant,
                    ServerMessage::TranscriptFailed {
                        utterance: record.client,
                        error,
                    },
                ),
            }
            return;
        }
        // Words that say the space's new title rename the space they were said in, if it is still listed, and are
        // never queued, even in a space whose session has closed. Saying the title it has already changes nothing.
        if record.titling && !text.is_empty() {
            let title = limits::title(spoken_name(text)).and_then(|title| {
                if title.is_empty() {
                    Err("No title was heard.".to_owned())
                } else {
                    Ok(title)
                }
            });
            // The space may have been forgotten while the words were transcribed.
            let renamed = title.and_then(|title| {
                if state.space(&record.space).is_none() {
                    return Err("That space is no longer listed.".to_owned());
                }
                let by = state
                    .participants
                    .get(&record.participant)
                    .map_or_else(String::new, |participant| participant.name.clone());
                state.rename_space(
                    &record.space,
                    title.clone(),
                    tomori_protocol::Renamer::Participant { name: by },
                );
                Ok(title)
            });
            match renamed {
                Ok(title) => {
                    state.send(
                        &record.participant,
                        ServerMessage::Transcript {
                            utterance: record.client,
                            text: title,
                            finished: true,
                        },
                    );
                    drop(state);
                    self.keep();
                }
                Err(error) => state.send(
                    &record.participant,
                    ServerMessage::TranscriptFailed {
                        utterance: record.client,
                        error,
                    },
                ),
            }
            return;
        }
        // The key makes a repeated finalization of the same client utterance idempotent: it is acknowledged with the text queued the first time, and nothing is queued again. It begins with the participant's number, which contains no `/`, so that no two participants' utterances share a key whatever their utterance identifiers contain. The key is the entry's identifier, which every client sees, so it never contains the participant's identity; see `design/security.md`.
        let key = format!("{}/{}", record.source, record.client);
        // A repeated finalization of an answer already applied is acknowledged in the same way.
        let answered = record.answer.as_ref().and_then(|_| {
            state.space(&record.space).and_then(|space| {
                space
                    .answered
                    .iter()
                    .find(|(answered, _)| *answered == key)
                    .map(|(_, text)| text.as_str())
            })
        });
        if let Some(queued) = state
            .space(&record.space)
            .and_then(|space| space.model.queued_text(&key))
            .or(answered)
        {
            let text = queued.to_owned();
            state.send(
                &record.participant,
                ServerMessage::Transcript {
                    utterance: record.client,
                    text,
                    finished: true,
                },
            );
            return;
        }
        let failure = if state
            .space(&record.space)
            .is_some_and(|space| space.model.is_closed())
        {
            Some("The agent session has ended, so nothing was sent.")
        } else if text.is_empty() {
            Some(tomori_protocol::NO_SPEECH)
        } else if text.len()
            > if record.answer.is_some() {
                limits::ANSWER_BYTES
            } else {
                limits::QUEUED_TEXT_BYTES
            }
        {
            Some("The recording was too long, so nothing was sent. Please say it in shorter parts.")
        } else if state
            .participants
            .get(&record.participant)
            .is_none_or(|participant| participant.space.as_deref() != Some(&record.space))
            || state.space(&record.space).is_none()
        {
            Some("You left the space before the transcript was ready, so nothing was sent.")
        } else if record.answer.is_none()
            && state
                .space(&record.space)
                .is_some_and(|space| space.model.waiting() >= limits::QUEUED_ENTRIES)
        {
            Some("The queue is full, so nothing was sent. Please wait for the agent to catch up.")
        } else {
            None
        };
        if let Some(error) = failure {
            state.send(
                &record.participant,
                ServerMessage::TranscriptFailed {
                    utterance: record.client,
                    error: error.to_owned(),
                },
            );
            return;
        }
        // A spoken answer goes to the presented question, never into the queue, where it would wait behind the question it answers. See `design/spaces.md`.
        if let Some(target) = &record.answer {
            let message = match state.answer(
                &record.participant,
                &record.space,
                &target.question,
                &target.field,
                text,
            ) {
                Ok(()) => {
                    let Some(space) = state.space_mut(&record.space) else {
                        unreachable!("answering just above found the space listed");
                    };
                    if space.answered.len() == ANSWERED {
                        space.answered.pop_front();
                    }
                    space.answered.push_back((key.clone(), text.to_owned()));
                    ServerMessage::Transcript {
                        utterance: record.client,
                        text: text.to_owned(),
                        finished: true,
                    }
                }
                Err(error) => ServerMessage::TranscriptFailed {
                    utterance: record.client,
                    error: limits::truncate(&error, limits::ERROR_BYTES),
                },
            };
            state.send(&record.participant, message);
            return;
        }
        let speaker = state.participants[&record.participant].name.clone();
        let Some(space) = state.space_mut(&record.space) else {
            unreachable!("checked listed above");
        };
        let queued = space.model.enqueue_attending(
            key.clone(),
            record.source,
            speaker,
            text.to_owned(),
            record.attention,
        );
        debug_assert!(queued, "repeats are acknowledged above");
        // What they pointed at ranks first in the agent's ambient list for their words, and lately, now that the words
        // are the agent's to answer; an utterance cancelled or never transcribed changes nothing.
        let environment = &space.model.environment;
        if let Some(channel) = space
            .channel
            .as_mut()
            .filter(|channel| &channel.environment == environment)
        {
            channel.ambient.focus(&record.participant, record.pointed);
        }
        if let Some(head) = record.origin {
            space.model.originate(
                &key,
                crate::space::Origin {
                    head,
                    at: record.begun,
                },
            );
        }
        space.model.saw(&key, record.seen);
        // Input queued in it makes the space active.
        let active = space.stamp_active(crate::cards::unix_now());
        state.send(
            &record.participant,
            ServerMessage::Transcript {
                utterance: record.client,
                text: text.to_owned(),
                finished: true,
            },
        );
        state.pump(&record.space);
        state.touch_space(&record.space);
        if active {
            drop(state);
            self.keep();
        }
    }

    pub(crate) fn transcript_failed(&self, utterance: UtteranceId, error: &str) {
        let mut deferred = Vec::new();
        {
            let mut state = self.lock();
            let Some(record) = state.utterances.remove(&utterance) else {
                return;
            };
            record.log_outcome(utterance, "failed to transcribe");
            state.agent_cancelled(&record.space, &record.participant, Instant::now());
            // Recognition can fail while the utterance is still open: close it, so that the client may forget it without cancelling and its next utterance does not fail this one again. The transcriber is still told, as every utterance it began ends with an end or a cancel.
            let mut open = false;
            for connection in state.connections.values_mut() {
                if let Some(closed) = connection.utterance.take_if(|open| open.id == utterance) {
                    connection.failed = Some(closed.client);
                    deferred.push(Deferred::Cancel(utterance));
                    open = true;
                }
            }
            state.recorded(open.then(|| (record.participant.clone(), record.space.clone())));
            state.send(
                &record.participant,
                ServerMessage::TranscriptFailed {
                    utterance: record.client,
                    error: error.to_owned(),
                },
            );
        }
        self.run(deferred);
    }

    /// Add a space for a newly started session, and move its creator into it unless they have joined another space since asking. Returns `None`, registering nothing, once shutdown has begun: the caller must close the session.
    pub(crate) fn register_space(
        &self,
        request: &CreateRequest,
        session: String,
        snapshot: Snapshot,
        commands: async_channel::Sender<SpaceCommand>,
    ) -> Option<String> {
        let mut state = self.lock();
        if state.shutting_down {
            return None;
        }
        state.next_space += 1;
        let id = format!("space-{}-{}", state.run, state.next_space);
        let mut model = SpaceModel::new(
            id.clone(),
            request.title.clone(),
            request.backend.clone(),
            request.directory.clone(),
            request.environment.clone(),
            session,
            snapshot,
        );
        model.set_instructions(
            request.instructions.trim().to_owned(),
            self.takes_instructions(&request.backend),
        );
        log::info!(
            "Space {id} {:?} is bound to {} session {} in the {} environment.",
            model.title,
            model.backend,
            model.session,
            model.environment
        );
        state.spaces.push(SpaceRecord {
            model,
            commands,
            touched: Cell::new(None),
            projection_revision: Cell::new(crate::versions::first_revision()),
            told: Default::default(),
            versions: Default::default(),
            projection_told: Cell::new(None),
            projection_deferred: Cell::new(None),
            built: Default::default(),
            deferred: Cell::new(None),
            closed: None,
            kept_open: false,
            attention: Cell::new(None),
            speech: Retention::default(),
            waker: None,
            answered: std::collections::VecDeque::new(),
            instructing: Cell::new(None),
            placements: std::collections::BTreeMap::new(),
            restoring: false,
            channel: None,
            acts: None,
            stopped: None,
            stage: crate::stage::Stage::default(),
            summoned: None,
            partner: None,
            following: following::Following::default(),
            seating: Default::default(),
            places: Default::default(),
            anchored: Instant::now(),
            bound: None,
            agent_at: None,
            rates: Default::default(),
            proximity: Default::default(),
            active: None,
        });
        // Created now, it is active from now.
        state
            .spaces
            .last_mut()
            .expect("pushed above")
            .stamp_active(crate::cards::unix_now());
        // Tomori's tools are registered for its directory, where the agent says its session is, if they are not already.
        let directory = state
            .spaces
            .last()
            .expect("pushed above")
            .model
            .directory
            .clone();
        self.tools_due(state.shutting_down, &request.backend, &directory);
        // Given before any input, so that the agent has them for its first turn.
        state.spaces.last().expect("pushed above").instruct(true);
        // The environment's server side, if it has one, runs for the space from when it is bound.
        self.reconcile_server_sides(&mut state);
        // Kept before anyone is told of it, so that no client holds an identifier a crash would lose.
        drop(state);
        self.keep();
        let mut state = self.lock();
        let creator_waiting = state
            .participants
            .get(&request.creator)
            .is_some_and(|record| {
                record.connection.is_some()
                    && (record.space.is_none() || record.space == request.creator_space)
            });
        if creator_waiting {
            state.join(&request.creator, &id);
        }
        state.pump(&id);
        state.forget_closed();
        state.touch_lobby();
        self.recount(&mut state);
        drop(state);
        self.keep();
        Some(id)
    }

    /// The session of `space` now goes by `session`, as an agent may say once it starts, so that the space is bound to it after a restart.
    pub(crate) fn session_changed(&self, space: &str, session: String) {
        {
            let mut state = self.lock();
            let Some(record) = state.space_mut(space) else {
                return;
            };
            log::info!("Space {space} is now bound to session {session}.");
            record.model.session = session;
        }
        self.keep();
    }

    /// Begin shutting down, as the server stops, so that the spaces closing are kept open; see [`Self::shutdown`].
    pub(crate) fn stopping(&self) {
        self.lock().shutting_down = true;
    }

    /// List again the spaces `store` keeps, and keep spaces there from now on. Each space that was open waits as reconnecting until the supervisor binds it to its session, so that clients restoring it, and collaborators admitted to it, find it meanwhile; each that was closed is listed closed. Returns the spaces to bind. See `design/spaces.md`.
    pub(crate) fn restore(&self, store: Arc<crate::kept::SpaceStore>) -> Vec<Restore> {
        let mut restores = Vec::new();
        let mut kept = store.spaces();
        let order: Vec<String> = kept.iter().map(|space| space.id.clone()).collect();
        // Closed spaces are stamped in the order they closed.
        kept.sort_by_key(|space| space.closed_order);
        {
            let mut state = self.lock();
            for kept in kept {
                if state.space(&kept.id).is_some() {
                    continue;
                }
                let mut model = SpaceModel::new(
                    kept.id.clone(),
                    kept.title,
                    kept.backend.clone(),
                    kept.directory.clone(),
                    kept.environment,
                    kept.session.clone(),
                    Snapshot::default(),
                );
                // Renamed, it keeps its title over its session's once bound again.
                model.keep_title(kept.renamed_by);
                model.set_attribution(kept.attribution);
                if kept.brief {
                    model
                        .set_speech(SpeechSetting::Brief)
                        .expect("brief is a known setting");
                }
                // A setting this build does not know, from a newer server, leaves the default.
                if let Some(movement) = kept.movement
                    && model.set_movement(movement).is_err()
                {
                    log::warn!(
                        "Space {} keeps a way for its agent to move that this server does not know.",
                        kept.id
                    );
                }
                let takes = self.takes_instructions(&kept.backend);
                model.set_instructions(kept.instructions, takes);
                *model.timers_mut() = crate::timers::Timers::restore(&kept.timers, &kept.id);
                let (commands, receiver) = async_channel::unbounded();
                let closed = match kept.closed {
                    Some(reason) => {
                        model.apply(Event::Closed {
                            reason: Some(reason),
                        });
                        // It closed before the restart; whether anyone saw it is not known.
                        model.attend();
                        state.next_closed += 1;
                        Some(state.next_closed - 1)
                    }
                    None => {
                        model.apply(Event::Reconnecting {
                            reason: RESTORING.to_owned(),
                        });
                        restores.push(Restore {
                            id: kept.id,
                            backend: kept.backend,
                            directory: kept.directory,
                            session: kept.session,
                            commands: receiver,
                        });
                        None
                    }
                };
                // Panels kept in slots a wall no longer has, as when an environment lays out its own wall anew, are put
                // where they show.
                let mut stage = kept.stage;
                {
                    let environments = self.environments();
                    for environment in stage.environments() {
                        stage.fit_slots(&environments.walls(&environment));
                    }
                    // The space's status panels take the status wall's free slots where it is now, as a space kept
                    // before environments had status walls of their own does on first opening in one.
                    stage.settle(&environments.walls(&model.environment));
                }
                state.spaces.push(SpaceRecord {
                    model,
                    commands,
                    touched: Cell::new(None),
                    projection_revision: Cell::new(crate::versions::first_revision()),
                    told: Default::default(),
                    versions: Default::default(),
                    projection_told: Cell::new(None),
                    projection_deferred: Cell::new(None),
                    built: Default::default(),
                    deferred: Cell::new(None),
                    closed,
                    kept_open: false,
                    attention: Cell::new(None),
                    speech: Retention::default(),
                    waker: None,
                    answered: std::collections::VecDeque::new(),
                    instructing: Cell::new(None),
                    placements: kept.placements,
                    restoring: true,
                    channel: None,
                    acts: None,
                    stopped: None,
                    stage,
                    summoned: None,
                    partner: None,
                    following: following::Following::default(),
                    seating: Default::default(),
                    places: kept.places,
                    anchored: Instant::now(),
                    bound: None,
                    agent_at: None,
                    rates: Default::default(),
                    proximity: Default::default(),
                    active: kept.active,
                });
            }
            // Listed in the order they were created, as before the restart.
            state.spaces.sort_by_key(|space| {
                order
                    .iter()
                    .position(|id| *id == space.model.id)
                    .unwrap_or(usize::MAX)
            });
            state.touch_lobby();
            self.recount(&mut state);
        }
        // Image panels' images are kept beside the spaces, and those no kept panel shows any more removed.
        let images = store.path().with_file_name(IMAGES_FOLDER);
        *self.kept.write().expect("never left poisoned") = Some(store);
        match self.images.keep_in(images) {
            Ok(()) => self.sweep_images(),
            Err(error) => {
                log::warn!("Image panels' images are kept only until the server stops: {error}.")
            }
        }
        restores
    }

    /// Bind the space `id`, listed again after a restart, to its session, in `directory`, where it was attached, which may not be where it was kept, since the agent may have moved it, and then where its snapshot says it is. Returns false, binding nothing, once shutdown has begun: the caller must close the session.
    pub(crate) fn bound(
        &self,
        id: &str,
        session: String,
        directory: &str,
        snapshot: Snapshot,
    ) -> bool {
        let mut state = self.lock();
        if state.shutting_down {
            return false;
        }
        let Some(record) = state.space_mut(id) else {
            return false;
        };
        let kept = record.model.directory.clone();
        record.model.follow_directory(directory);
        record.model.bind(session, snapshot);
        log::info!(
            "Space {id} {:?} is bound again to {} session {}.",
            record.model.title,
            record.model.backend,
            record.model.session
        );
        // Given again before any input, as the agent may have lost them while the server was stopped.
        record.instruct(true);
        // Its directory has been resolved within the workspace roots again, so its environment's server side runs for it as for a space just created.
        record.restoring = false;
        self.reconcile_server_sides(&mut state);
        let model = &state.space(id).expect("found above").model;
        self.tools_due(state.shutting_down, &model.backend, &model.directory);
        // Moved while the server was stopped, its tools are removed from where it was.
        if model.directory != kept {
            tools::moved(id, &kept, &model.directory);
            self.tools_due(state.shutting_down, &model.backend, &kept);
        }
        state.pump(id);
        state.touch_space(id);
        drop(state);
        self.keep();
        true
    }

    /// The space `id`, listed again after a restart, could not be bound to its session: list it as closed, with `reason`. Unless the failure is `lasting`, such as the session no longer existing, it is kept as open, to be bound again when the server next starts.
    pub(crate) fn restore_failed(&self, id: &str, reason: String, lasting: bool) {
        log::warn!("Space {id} could not be bound again: {reason}");
        if !lasting && let Some(record) = self.lock().space_mut(id) {
            record.kept_open = true;
        }
        self.space_ended(id, Some(reason));
        // Not being bound again is the server's restart, not something that happened in the space's work, so it needs no
        // attention; see `design/spaces.md`.
        let mut state = self.lock();
        if let Some(record) = state.space_mut(id) {
            record.model.attend();
        }
        state.touch_space(id);
        self.recount(&mut state);
    }

    /// Write the kept spaces, if there is somewhere to keep them and they changed. Spaces closed once shutdown has begun are kept as open; see `stamp_closed`. Returns whether what is kept is as the spaces are now.
    pub(crate) fn keep(&self) -> bool {
        let Some(store) = self.kept.read().expect("never left poisoned").clone() else {
            self.sweep_images();
            return true;
        };
        let written = store.keep(|| Some(self.lock().kept()));
        if let Err(error) = &written {
            log::error!(
                "Could not keep the spaces in {}: {error}.",
                store.path().display()
            );
        }
        self.sweep_images();
        written.is_ok()
    }

    /// Remove the images that no panel any space keeps shows, if those it shows changed. See `design/projection.md`.
    fn sweep_images(&self) {
        let shown = self
            .lock()
            .spaces
            .iter()
            .flat_map(|record| record.stage.images())
            .map(str::to_owned)
            .collect();
        self.images.sweep(shown);
    }

    /// Send the image `hash` to `connection`, which asked for it, if it is kept. A request for one already on its way, or
    /// beyond what may wait to be sent, is dropped: a client asks for a few at a time, and again after a while.
    fn send_image(&self, connection: ConnectionId, hash: [u8; 32]) {
        let png = self.images.get(&tomori_protocol::hash_text(&hash));
        let state = self.lock();
        let Some(record) = state.connections.get(&connection) else {
            return;
        };
        match png {
            Some(png) => {
                if !record.outbox.image(hash, png) {
                    log::debug!(
                        "An image asked for again, or beyond those waiting, was not queued."
                    );
                }
            }
            None => log::warn!("An image a panel shows is no longer kept, so it was not sent."),
        }
    }

    pub(crate) fn create_failed(&self, creator: &str, error: String) {
        self.lock().error(creator, error);
    }

    /// Report `backend`'s session list to `participant`, its failures as an error. A session already bound to a space
    /// that has not closed is left out, since binding it again would be refused, as is one whose identifier or
    /// directory is too long to be sent back to bind it; the list is then fitted to [`limits::SNAPSHOT_BUDGET`]. See
    /// `design/arrival.md`.
    pub(crate) fn sessions(
        &self,
        participant: &str,
        backend: &str,
        result: Result<Vec<tomori_backend::SessionSummary>, String>,
    ) {
        let state = self.lock();
        let message = match result {
            Ok(sessions) => {
                let bound: HashSet<&str> = state
                    .spaces
                    .iter()
                    .filter(|space| space.model.backend == backend && !space.model.is_closed())
                    .map(|space| space.model.session.as_str())
                    .collect();
                let sessions = sessions
                    .into_iter()
                    .filter(|session| !bound.contains(session.id.as_str()))
                    .map(|session| tomori_protocol::SessionSummary {
                        id: session.id,
                        title: session.title,
                        directory: session.directory,
                        busy: session.busy,
                    })
                    .collect();
                ServerMessage::Sessions {
                    sessions: limits::sessions(sessions),
                }
            }
            Err(error) => ServerMessage::Error {
                error: limits::truncate(&error, limits::ERROR_BYTES),
                space: None,
                panel: None,
                wall: None,
            },
        };
        state.send(participant, message);
    }

    /// Apply `event`, from the agent session of `space`. Returns the panels of the replies it completed, if any, whose
    /// code is to be highlighted outside the lock, with [`crate::highlighting::highlight`], and which are then projected
    /// with [`Self::project`], before the space's next event, so that each reply's panels are projected in order.
    #[must_use]
    pub(crate) fn space_event(
        &self,
        space: &str,
        event: Event,
    ) -> Option<crate::stage::Projecting> {
        let mut state = self.lock();
        let shutting_down = state.shutting_down;
        let record = state.space_mut(space)?;
        let resynchronized = matches!(event, Event::Resynchronized { .. });
        let was_open = !record.model.is_closed();
        let title = record.model.title.clone();
        let directory = record.model.directory.clone();
        // The agent working in the space keeps when it was last active, written again at most once a minute.
        let active =
            crate::space::is_activity(&event) && record.stamp_active(crate::cards::unix_now());
        record.model.apply(event);
        // Following its session, the space took its new title: everyone sees it, and it is kept across a restart.
        let retitled = record.model.title != title;
        if retitled {
            log::info!(
                "Space {space} follows its session's title, now {:?}.",
                record.model.title
            );
        }
        let replies = record.model.take_projected();
        // Following its session, which the agent moved, the space is in its new directory: Tomori's tools go with it,
        // removed from where it was, unless another space is open there, and it is kept across a restart.
        let relocated = record.model.directory != directory;
        if relocated {
            tools::moved(space, &directory, &record.model.directory);
            self.tools_due(shutting_down, &record.model.backend, &directory);
            self.tools_due(
                shutting_down,
                &record.model.backend,
                &record.model.directory,
            );
        }
        if resynchronized {
            // The agent was read again after its connection was lost, perhaps after it restarted: give it the instructions again, before any input released by reading it.
            record.instruct(true);
            // It forgets Tomori's tools when it restarts, too.
            self.tools_due(
                shutting_down,
                &record.model.backend,
                &record.model.directory,
            );
        }
        // Stamped now rather than when the snapshot is sent, so a level that ends and starts again within one
        // snapshot interval starts its age again.
        record.stamp_attention(Instant::now());
        state.stamp_closed();
        // A space that closed no longer runs its environment's server side.
        if was_open
            && state
                .space(space)
                .is_some_and(|record| record.closed.is_some())
        {
            self.reconcile_server_sides(&mut state);
        }
        // Each reply's panels go for whoever it was for, where they are as it completes.
        let now = Instant::now();
        let replies: Vec<_> = replies
            .into_iter()
            .map(|(panels, asked)| (panels, state.asker(space, &asked, now)))
            .collect();
        state.pump(space);
        if state.touch_space_paced(space, Instant::now()) {
            self.deferred_snapshot.notify(usize::MAX);
        }
        // The space's attention may have changed.
        self.recount(&mut state);
        if retitled || relocated || active {
            drop(state);
            self.keep();
        }
        (!replies.is_empty()).then(|| crate::stage::Projecting::new(replies))
    }

    /// Project the panels of `replies`, which events of `space` completed, highlighted: each where its block says, near
    /// or where someone in the space is looking, or on a wall, or else where whoever asked for it was looking, or else
    /// the Tomori wall, its code panels replacing those of the replies before. See `design/projection.md`.
    pub(crate) fn project(&self, space: &str, replies: crate::stage::Replies) {
        let mut state = self.lock();
        let people = state.heads(space);
        let carried = state.carried(space, Instant::now());
        let environments = self.environments();
        // What volumes' world references name, as the space has them as they are projected, where any has one.
        let referring = replies.iter().any(|(contents, _)| {
            contents.iter().any(|(content, _)| {
                content
                    .diagram
                    .as_ref()
                    .is_some_and(|diagram| !diagram.world.is_empty())
            })
        });
        let things = state
            .space(space)
            .filter(|_| referring)
            .map(|record| {
                let walls = environments.walls(&record.model.environment);
                let projection = record.stage.view(space, &walls, crate::cards::unix_now());
                state.world_things(
                    space,
                    &environments,
                    &projection,
                    &carried,
                    None,
                    Instant::now(),
                )
            })
            .unwrap_or_default();
        let Some(record) = state.space_mut(space) else {
            return;
        };
        let walls = environments.walls(&record.model.environment);
        let now = crate::cards::unix_now();
        let solids = record.panel_solids(&environments);
        let mut notes = Vec::new();
        for (contents, origin) in replies {
            let projection = record.stage.view(space, &walls, now);
            // How many of the reply's panels went to each anchor before, so that each takes a slot of its own there.
            let mut anchored: Vec<String> = Vec::new();
            let mut warned: Vec<crate::stage::Content> = Vec::new();
            let placed = contents
                .into_iter()
                .map(|(mut content, place)| {
                    // A volume's world references are resolved before it is placed, which may stand it near them.
                    content.resolve(&things);
                    // A placement that names someone who has not reported, or a wall or anchor not here, is as none.
                    let said = place.and_then(|place| {
                        let before = match &place {
                            crate::stage::Placement::Anchor(name) => {
                                anchored.push(name.clone());
                                anchored.iter().filter(|went| *went == name).count() - 1
                            }
                            _ => 0,
                        };
                        match (&place, content.diagram.as_deref()) {
                            (crate::stage::Placement::NearReferences, Some(diagram)) => {
                                Some(world::near_references(
                                    diagram,
                                    origin.as_ref(),
                                    &projection,
                                    &solids,
                                ))
                            }
                            _ => record
                                .panel_destination(
                                    &environments,
                                    &place,
                                    before,
                                    anchors::Surroundings {
                                        asker: origin.as_ref(),
                                        people: &people,
                                        projection: &projection,
                                        solids: &solids,
                                    },
                                )
                                .ok(),
                        }
                    });
                    // A block with the id of a panel someone is carrying updates it in their hand, where it stays.
                    let named = content
                        .slug
                        .as_deref()
                        .and_then(|slug| record.stage.named(walls.environment, slug));
                    let held = named.as_ref().is_some_and(|id| carried.contains(id));
                    let placed = said.is_some() && !held;
                    let to = said.unwrap_or_else(|| {
                        crate::presence::by_default(origin.as_ref(), &projection, &solids)
                    });
                    // A volume is laid out as it is projected, from where the one it changes stood, if it changes one,
                    // as it will stand.
                    if content.diagram.is_some() {
                        let previous = content
                            .slug
                            .as_deref()
                            .and_then(|slug| record.stage.named_diagram(walls.environment, slug));
                        let standing = named.as_deref().and_then(|id| projection.panel(id));
                        let pose = world::volume_pose(&to, placed, standing, &projection);
                        content.lay_out(previous, pose.as_ref());
                    }
                    warned.push(crate::stage::Content {
                        warnings: std::mem::take(&mut content.warnings),
                        ..content.clone()
                    });
                    (content, to, placed)
                })
                .collect();
            for ((id, updated), content) in record
                .stage
                .project_placed(&walls, placed, now)
                .into_iter()
                .zip(&warned)
            {
                // What was wrong with a volume's block, the agent is told before the next words.
                if let Some(note) = content.warned(&id) {
                    notes.push((id.clone(), note));
                }
                if updated {
                    log::info!("Space {space} updated {id} in place.");
                } else {
                    log::info!("Space {space} projected {id}.");
                }
            }
        }
        for (id, note) in notes {
            record.model.note_panel(&id, note);
        }
        // Those put at an anchor stand in its cluster, which may lay it out again.
        record.settle(&environments);
        state.touch_projection(space);
        drop(state);
        self.keep();
    }

    /// Apply `event` as the space's driver does, highlighting and projecting the panels of the replies it completes.
    #[cfg(test)]
    pub(crate) fn drive_event(&self, space: &str, event: Event) {
        if let Some(projecting) = self.space_event(space, event) {
            let highlighting = crate::highlighting::highlight(projecting, self.highlighter());
            self.project(space, futures_lite::future::block_on(highlighting));
        }
    }

    /// What highlights the agent's code panels.
    pub(crate) fn highlighter(&self) -> crate::highlighting::Highlighter {
        self.highlighter
            .read()
            .expect("never left poisoned")
            .clone()
    }

    /// Highlight the agent's code panels with `highlighter`, as a test does to see what happens while it is slow.
    #[cfg(test)]
    pub(crate) fn set_highlighter(&self, highlighter: crate::highlighting::Highlighter) {
        *self.highlighter.write().expect("never left poisoned") = highlighter;
    }

    /// When the earliest deferred snapshot, of a space's state or of what it projects, or an agent's next move, is due,
    /// if any.
    pub fn snapshot_deadline(&self) -> Option<Instant> {
        let state = self.lock();
        state
            .spaces
            .iter()
            .flat_map(|space| [space.deferred.get(), space.projection_deferred.get()])
            .flatten()
            .chain(state.movement_deadline())
            .min()
    }

    /// Wait for a space to defer a snapshot. Check [`snapshot_deadline`](Self::snapshot_deadline) after creating the listener to avoid a lost wakeup.
    pub fn listen_snapshots(&self) -> EventListener {
        self.deferred_snapshot.listen()
    }

    /// Show `board`'s cards in the spaces of its environment. See `design/github.md`.
    pub(crate) fn set_board(&self, board: crate::cards::Board) {
        self.lock().board = Some(board);
    }

    /// Notified when someone starts watching the cards.
    pub(crate) fn listen_cards(&self) -> EventListener {
        self.cards_watched.listen()
    }

    /// When the cards are next due to be read at `now`: only while someone watches them in a space that shows them.
    pub(crate) fn cards_next(&self, now: Instant) -> crate::cards::Next {
        let state = self.lock();
        let watched = state.connections.values().any(|connection| {
            connection.cards
                && connection
                    .participant
                    .as_ref()
                    .and_then(|participant| state.participants.get(participant))
                    .and_then(|participant| participant.space.as_deref())
                    .is_some_and(|space| state.shows_cards(space))
        });
        state
            .board
            .as_ref()
            .map_or(crate::cards::Next::Idle, |board| {
                board.refresh.next(now, watched)
            })
    }

    /// Take what reading the cards at `now` found, telling everyone watching if it changed what they see.
    pub(crate) fn cards_fetched(
        &self,
        result: Result<crate::cards::Fetched, crate::cards::Failure>,
        now: Instant,
    ) {
        let mut state = self.lock();
        if state
            .board
            .as_mut()
            .is_some_and(|board| board.fetched(result, now))
        {
            state.touch_cards(None);
        }
    }

    /// Send the deferred snapshots that are due at `now`, of spaces' states and of what they project, and make the
    /// agents' moves that are due.
    pub fn flush_snapshots(&self, now: Instant) {
        let mut state = self.lock();
        // And the agents' moves that are due. See `design/environments.md`.
        state.movements_due(now);
        self.keep_up(&mut state, now);
        let due: Vec<String> = state
            .spaces
            .iter()
            .filter(|space| space.deferred.get().is_some_and(|due| due <= now))
            .map(|space| space.model.id.clone())
            .collect();
        for space in due {
            state.touch_space(&space);
        }
        // What an environment's side changed of what a space projects within the interval since its connections were
        // last told, told now, as it is now. See [`PROJECTION_INTERVAL`].
        for space in &state.spaces {
            if space
                .projection_deferred
                .get()
                .is_some_and(|due| due <= now)
            {
                space.projection_told.set(Some(now));
                state.tell_projection(space);
            }
        }
    }

    pub(crate) fn space_sent(&self, space: &str, key: &str, result: tomori_backend::Result<()>) {
        let mut state = self.lock();
        let Some(record) = state.space_mut(space) else {
            return;
        };
        record
            .model
            .sent(key, result.map_err(|error| error.message));
        state.pump(space);
        state.touch_space(space);
        self.recount(&mut state);
    }

    pub(crate) fn space_decided(
        &self,
        space: &str,
        request: &str,
        participant: &str,
        result: tomori_backend::Result<()>,
    ) {
        let mut state = self.lock();
        let Err(error) = result else {
            if let Some(record) = state.space_mut(space) {
                record.model.decided(request);
            }
            state.pump(space);
            state.touch_space(space);
            self.recount(&mut state);
            return;
        };
        if let Some(record) = state.space_mut(space) {
            if matches!(error.kind, ErrorKind::NotFound | ErrorKind::Closed) {
                // The request is gone, so nothing is pending any more.
                record.model.decided(request);
            } else {
                record.model.restore_approval(request);
            }
        }
        state.pump(space);
        state.error(
            participant,
            format!("The decision was not accepted: {}", error.message),
        );
        state.touch_space(space);
        self.recount(&mut state);
    }

    pub(crate) fn space_answered(
        &self,
        space: &str,
        question: &str,
        participant: &str,
        result: tomori_backend::Result<()>,
    ) {
        let mut state = self.lock();
        let Err(error) = result else {
            if let Some(record) = state.space_mut(space) {
                record.model.answered(question);
            }
            state.pump(space);
            state.touch_space(space);
            self.recount(&mut state);
            return;
        };
        if let Some(record) = state.space_mut(space) {
            if matches!(error.kind, ErrorKind::NotFound | ErrorKind::Closed) {
                // The request is gone, so nothing is pending any more.
                record.model.answered(question);
            } else {
                record.model.restore_question(question);
            }
        }
        state.pump(space);
        state.error(
            participant,
            format!("The answer was not accepted: {}", error.message),
        );
        state.touch_space(space);
        self.recount(&mut state);
    }

    /// The agent answered a participant's mode switch. A switch it accepted is shared when it confirms the mode.
    pub(crate) fn space_mode_set(
        &self,
        space: &str,
        participant: &str,
        result: tomori_backend::Result<()>,
    ) {
        if let Err(error) = result {
            let state = self.lock();
            if state.space(space).is_some() {
                state.error(
                    participant,
                    format!("The mode was not changed: {}", error.message),
                );
            }
        }
    }

    /// The agent was given the space's instructions, or refused them. The space works without them, so a refusal is shown in the space rather than closing it. See `design/spaces.md`.
    pub(crate) fn space_instructed(
        &self,
        space: &str,
        result: tomori_backend::Result<()>,
        all: bool,
    ) {
        let mut state = self.lock();
        let Some(record) = state.space_mut(space) else {
            return;
        };
        if let Err(error) = &result {
            log::warn!(
                "The agent of space {space} did not take its instructions: {}",
                error.message
            );
        }
        record
            .model
            .instructed(result.map_err(|error| error.message), all);
        state.touch_space(space);
    }

    /// The instruction entries the driver of `space` gives now, for its queued [`SpaceCommand::Instructions`], and whether they are every layer. Empty once the space has closed.
    pub(crate) fn instructions_due(&self, space: &str) -> (Vec<(String, Option<String>)>, bool) {
        let state = self.lock();
        let Some(record) = state.space(space) else {
            return (Vec::new(), false);
        };
        let all = record.instructing.take().unwrap_or(true);
        // The environment's layer is looked up now, in the environments offered now, rather than kept with the space, so that however the server was configured, a space is given the layer of the environment it is in.
        let mut entries = record
            .model
            .instruction_entries(self.environments().instructions(&record.model.environment));
        // Only with the other layers, which a closed space or a backend without instructions is not given.
        if !entries.is_empty() {
            entries.extend(self.tools_entry(&record.model.backend));
        }
        if !all {
            entries.retain(|(key, _)| key == crate::instructions::SPACE_KEY);
        }
        (entries, all)
    }

    /// Whether the backend named `backend` takes instruction entries.
    pub(crate) fn takes_instructions(&self, backend: &str) -> bool {
        self.backends
            .get(backend)
            .is_some_and(|backend| backend.capabilities().instructions)
    }

    /// The driver stopped: the session produces no more events.
    pub(crate) fn space_ended(&self, space: &str, error: Option<String>) {
        let mut state = self.lock();
        let Some(record) = state.space_mut(space) else {
            return;
        };
        if !record.model.is_closed() {
            record.model.apply(Event::Closed { reason: error });
        }
        // A closed space keeps no speech, so that up to [`CLOSED_SPACES`] closed spaces hold no audio, and its pacer stops once what is still queued has been spoken.
        record.speech.close();
        record.waker = None;
        // A closed space no longer runs its environment's server side.
        self.reconcile_server_sides(&mut state);
        // Nor needs Tomori's tools, unless another space in its directory does.
        let model = &state.space(space).expect("found above").model;
        self.tools_due(state.shutting_down, &model.backend, &model.directory);
        // Forgotten before the lobby listing that touching the space produces.
        state.forget_closed();
        state.touch_space(space);
        self.recount(&mut state);
        drop(state);
        self.keep();
    }

    /// Render a deferred outgoing item for one connection.
    pub fn render(&self, connection: ConnectionId, item: Outgoing) -> Option<ServerMessage> {
        match item {
            Outgoing::Message(message) => Some(message),
            Outgoing::Space => {
                let state = self.lock();
                // Nothing for a connection not in a space, or, should one be forgotten while occupied, in one that is not listed.
                let space = state
                    .member(connection)
                    .and_then(|(_, space)| space)
                    .and_then(|space| state.space(&space))?;
                let participants = state.participants(&space.model);
                let cards = state.shows_cards(&space.model.id);
                // Whom the agent walks with, by their number.
                let walking = space
                    .following
                    .walks_with()
                    .and_then(|key| state.participants.get(key))
                    .map(|walker| walker.source);
                let environments = self.environments();
                let mut state = space.model.state(
                    participants,
                    environments.instructions(&space.model.environment),
                );
                // Named before the state is fitted to its budget, which counts them.
                state.environment_version = environments.version(&space.model.environment);
                // Where the environment has participants arrive and puts the agent's home and the workers, and where the
                // agent was summoned to, for everyone in it. See `design/environments.md`.
                state.anchors = space.anchors(&environments);
                // Where each worker stands, so that every client lays every cluster out alike.
                state.clusters = space.seated(&environments).layouts();
                state.summoned = space.summoned;
                // The trigger summons it nowhere: it follows, and is called with the hand menu. See
                // `design/environments.md`.
                state.summons = false;
                state.pinned = space.following.pinned;
                // Participants pin it, and let it follow again, from the hand menu. See `design/hand-menu.md`.
                state.pins = true;
                // And have it walk with them, or walk with them no more.
                state.walks = true;
                state.walking = walking;
                state.facing = space.following.faces();
                state.partner = space.partner;
                state.via = space.following.via;
                state.stream = space.following.stream;
                // The avatar its environment names, or else its backend's, for everyone in it. See `design/avatars.md`.
                state.avatar = environments
                    .avatar(&space.model.environment)
                    .or_else(|| self.backends.avatar(&space.model.backend));
                // Every space projects. See `design/projection.md`.
                state.projection = true;
                // Participants adjust walls, and an owner's removal leaves a wall's panels floating.
                state.walls = true;
                // Participants tick and untick checklist items pointed at.
                state.checks = true;
                // Participants choose the rows of an environment's lists it marks choosable. See `design/panel-actions.md`.
                state.choices = true;
                // Walls someone added snap together into groups, which move together. See `design/projection.md`.
                state.groups = true;
                // Walls are locked and unlocked by owners, an environment's own locked unless unlocked.
                state.wall_locks = true;
                // An owner may delete any space, and its session too where its backend deletes sessions. See `design/spaces.md`.
                state.deletes = true;
                state.deletes_session = self
                    .backends
                    .get(&space.model.backend)
                    .is_some_and(|backend| backend.capabilities().delete);
                limits::fit_state(&mut state);
                state.cards = cards;
                Some(ServerMessage::Space {
                    state: Box::new(state),
                })
            }
            Outgoing::Cards => {
                let state = self.lock();
                // Nothing for a connection that does not watch them, has left their space, is in a space that no longer shows them, or only visits it: leaving a space stops watching, and a visitor cannot start, so the role is checked again in case either is ever missed.
                let watching = state
                    .connections
                    .get(&connection)
                    .is_some_and(|record| record.cards);
                let (space, board) = state
                    .member(connection)
                    .and_then(|(_, space)| space)
                    .filter(|space| {
                        watching
                            && state.role(connection, Some(space)).participates()
                            && state.shows_cards(space)
                    })
                    .and_then(|space| state.space(&space).zip(state.board.as_ref()))?;
                let cards =
                    board.cards(&space.model.id, &space.placements, crate::cards::unix_now());
                Some(ServerMessage::Cards {
                    cards: Box::new(cards),
                })
            }
            Outgoing::Projection => {
                let mut state = self.lock();
                // Nothing for a connection that does not read it, or that has left its space since it was queued.
                let space = state
                    .member(connection)
                    .and_then(|(_, space)| space)
                    .filter(|_| {
                        state
                            .connections
                            .get(&connection)
                            .is_some_and(|record| record.projection)
                    })
                    .and_then(|space| state.space(&space))?;
                let environments = self.environments();
                // Within what this connection takes, whatever the space holds. See `design/projection.md`.
                let limits = state
                    .connections
                    .get(&connection)
                    .map_or(Limits::TODAY, |record| record.limits);
                let revision = &space.projection_revision;
                let projected = state
                    .connections
                    .get(&connection)
                    .filter(|record| record.patches)
                    .and_then(|record| record.projected.as_ref());
                // Each message is a revision of its own, even where nothing was touched since the last, as when it is
                // sent whole again on joining, so that a patch always goes from one revision to a later one, past the
                // one last sent even where the space's count began again meanwhile.
                if let Some(sent) = projected.map(|projected| projected.revision)
                    && sent >= revision.get()
                {
                    revision.set(sent + 1);
                }
                // Versions are found from the projection whole, images and all, whatever this client is sent of it,
                // and what it is sent within its limits has the same; one build serves every connection of its limits.
                let mut projection = space.built(&environments, crate::cards::unix_now(), &limits);
                // A client that shows no images is sent each image panel as a text panel saying what it is.
                let images = state
                    .connections
                    .get(&connection)
                    .is_some_and(|record| record.images);
                if !images && projection.panels.iter().any(|panel| panel.image.is_some()) {
                    let mut plain = (*projection).clone();
                    plain.panels = plain
                        .panels
                        .into_iter()
                        .map(crate::stage::without_image)
                        .collect();
                    projection = Arc::new(plain);
                }
                // And each volume past what it draws, or every one, if it draws none, as its outline.
                let (diagrams, world_edges, limits) = state
                    .connections
                    .get(&connection)
                    .map_or((false, false, Limits::TODAY), |record| {
                        (record.diagrams, record.world_edges, record.limits)
                    });
                if projection
                    .panels
                    .iter()
                    .any(|panel| panel.diagram.is_some())
                {
                    let mut plain = (*projection).clone();
                    plain.panels =
                        crate::stage::within_volumes(plain.panels, diagrams, world_edges, &limits);
                    projection = Arc::new(plain);
                }
                let live = state
                    .connections
                    .get(&connection)
                    .is_some_and(|record| record.live_panels);
                let message =
                    crate::patches::message(projected.map(|sent| &**sent), &projection, live);
                // The connection reads what is projected, as found above, so it is there.
                let record = state
                    .connections
                    .get_mut(&connection)
                    .expect("found above, reading what is projected");
                record.projection_sent = projection.revision;
                if record.patches {
                    record.projected = Some(projection);
                }
                Some(message)
            }
            Outgoing::Spaces => {
                let state = self.lock();
                let now = Instant::now();
                let unix = crate::cards::unix_now();
                let roots = self
                    .roots
                    .read()
                    .expect("workspace roots are never left poisoned");
                let mut spaces: Vec<_> = state
                    .spaces
                    .iter()
                    .map(|space| space.summary(now, unix, &roots))
                    .collect();
                limits::fit_spaces(&mut spaces);
                Some(ServerMessage::Spaces { spaces })
            }
        }
    }

    /// Wait until the server sides cancelled by [`Hub::shutdown`] have stopped, or `expiry` is ready, and so abandon those that have not, which are counted.
    pub(crate) async fn server_sides_stopped(&self, expiry: impl Future<Output = ()>) -> usize {
        self.server_sides.stopped(expiry).await
    }

    /// Ask every space driver to close its session, and end every connection.
    pub fn shutdown(&self) {
        self.jobs.close();
        // Spaces still being created are closed as soon as their session starts; see `register_space`. Set first, so that nothing the spaces closing cause is kept: closing their sessions does not close them.
        {
            let mut state = self.lock();
            state.shutting_down = true;
            // Every server side is cancelled with the spaces, and waited for by [`Hub::server_sides_stopped`].
            self.reconcile_server_sides(&mut state);
        }
        // Requests waiting on an agent, such as for its connection to return, fail at once rather than hold up the
        // spaces closing.
        self.backends.shutdown();
        let state = self.lock();
        for space in &state.spaces {
            let _ = space.commands.try_send(SpaceCommand::Close);
        }
        for connection in state.connections.values() {
            connection.outbox.close();
        }
    }
}

#[cfg(all(test, feature = "fake"))]
#[path = "hub_tests.rs"]
mod rarer;

#[cfg(test)]
mod tests;
