// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Shared server state: participants, connections, spaces and open utterances, behind one lock.
//!
//! Every client action and backend event is applied synchronously under the lock. Work that may call out of the server — transcriber calls and backend operations — is deferred until the lock is released, or sent to the space's driver task.
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, Weak};
use std::time::{Duration, Instant};

use base_scheduler::Cancellation;
use event_listener::{Event as Notify, EventListener};
use tomori_backend::{AnswerValue, Backend, ErrorKind, Event, Prompt, Snapshot};
use tomori_discovery::{AttentionCounts, Counts};
use tomori_protocol::{
    Attention, AudioFrame, ClientMessage, ConnectionRole, Engagement, EnvironmentVersion, Frame,
    FrameKind, Gate, Keepalive, PROTOCOL_VERSION, Participant, ServerMessage, SpeechSetting,
    WorkspaceRoot,
};

use crate::access::{self, Access, Grant};
use crate::backends::Backends;
use crate::environments::Environments;
use crate::limits;
use crate::outbox::{Outbox, Outgoing};
use crate::retention::{Delivery, Item, Retention};
use crate::space::{AnswerOutcome, SpaceModel};
use crate::transcriber::{Transcriber, TranscriptSink, UtteranceId};
use crate::workspace::SpaceSlots;

pub(crate) mod tools;

pub type ConnectionId = u64;

/// A registered connection: its identifier, its outbox, the flag it sets when data arrives, and where it reports its writes. See [`Hub::connect`].
pub type Connected = (ConnectionId, Arc<Outbox>, Arc<AtomicBool>, Arc<Output>);

/// The shortest time between snapshots of one space caused by agent events, so that a streaming reply sends about ten full snapshots a second however fast its text arrives. See `design/server.md`.
pub const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(100);

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

/// A pairing as the console and status page show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingView {
    /// Names this pairing, so that the status page confirms the one it showed.
    pub serial: u64,
    pub name: String,
    /// The code, written as the device shows it, once the device has revealed its value.
    pub code: Option<String>,
}

/// How a pairing is confirmed: by typing the code the device shows, on the console, or by the pairing the status page showed, whose code an owner compared.
#[derive(Clone, Copy, Debug)]
pub enum Confirmation<'a> {
    Typed(&'a str),
    Shown(u64),
}

/// A device pairing: its commitment, the server's value, and the code once the device has revealed its own value.
struct PendingPairing {
    serial: u64,
    connection: ConnectionId,
    /// The pairing connection's outbox, told how the pairing ended. Told after the connection goes, it hears nothing.
    outbox: Arc<Outbox>,
    /// What pairs the device, which the server has, or it would not begin the pairing.
    setup: PairingSetup,
    /// The device's display name, which names the token it receives.
    name: String,
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

/// Something about pairing to show on the server's console. See `design/security.md`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairingEvent {
    /// A device began pairing.
    Began { name: String },
    /// A pairing waits to be confirmed, with the code written as the device shows it.
    Waiting { name: String, code: String },
    /// Pairings are paused, since too many began recently.
    Paused,
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

impl tomori_environment::Outlet for SpaceOutlet {
    fn share(&self, state: String) -> tomori_environment::Result<()> {
        self.hub
            .upgrade()
            .map_or(Ok(()), |hub| hub.channel_update(self, &state, None))
    }

    fn announce(&self, event: String, state: String) -> tomori_environment::Result<()> {
        self.hub
            .upgrade()
            .map_or(Ok(()), |hub| hub.channel_update(self, &state, Some(&event)))
    }

    fn describe(&self, objects: Vec<tomori_environment::Object>) -> tomori_environment::Result<()> {
        self.hub
            .upgrade()
            .map_or(Ok(()), |hub| hub.channel_describe(self, objects))
    }
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
    server_sides: crate::server_sides::ServerSides,
    /// Notified when a space defers a snapshot, so that the server's snapshot timer can wake for it.
    deferred_snapshot: Notify,
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
    /// Whether the server serves only TLS, which pairing needs: the token it issues must not cross the network in the clear.
    encrypted: AtomicBool,
    pairing: RwLock<Option<PairingSetup>>,
    pairing_prompt: RwLock<Option<PairingPrompt>>,
    pairing_timeout: RwLock<Duration>,
    knock_timeout: RwLock<Duration>,
    /// Tomori's MCP server and its registrations with backends; see `design/mcp.md`.
    tools: tools::Tools,
    this: Weak<Hub>,
}

#[derive(Default)]
struct HubState {
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
    /// Knocks waiting for an answer, oldest first.
    knocks: Vec<KnockRecord>,
    next_knock: u64,
    /// When a visitor, by hub key, may next knock on a space, after a knock there was declined or expired.
    knock_cooldowns: HashMap<(String, String), Instant>,
    /// The counts when they were last compared, to notice a change.
    counts: Counts,
    /// The cards the server shows in their environment's spaces, if it shows any. See `design/github.md`.
    board: Option<crate::cards::Board>,
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

/// Whether the HTTP server is writing a connection's output, shared by the connection body and the keepalive sweep.
#[derive(Default)]
pub struct Output {
    /// True from when the body hands out a chunk until the HTTP server asks for the next one, which it does once the chunk is written.
    pending: AtomicBool,
    /// Chunks written so far.
    written: AtomicU64,
}

impl Output {
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
    /// Where their head last was in the space they are in, from their presence, which decides the wall the agent projects
    /// onto in answer to them. See `design/projection.md`.
    head: Option<[f32; 3]>,
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
    /// The space's projection walls and the panels on them. See `design/projection.md`.
    stage: crate::stage::Stage,
}

/// How many spoken answers each space remembers, to acknowledge a repeated finalization. A client repeats only its latest utterance, after reconnecting.
const ANSWERED: usize = 16;

/// When participants are told of a change to a space caused by an agent event; see [`SNAPSHOT_INTERVAL`].
enum Pace {
    Now,
    /// At the end of the interval, as no snapshot was deferred yet.
    Deferred,
    /// At the end of the interval, with the snapshot already deferred.
    AlreadyDeferred,
}

impl SpaceRecord {
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
    fn summary(&self, now: Instant) -> tomori_protocol::SpaceSummary {
        let mut summary = self.model.summary();
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
    /// What the participant attended to as the utterance began, as the space knew it then. Their client names only
    /// identities. See `design/spaces.md`.
    attention: Vec<crate::space::Attended>,
    /// Where the participant's head was as the utterance began, so that what the agent projects in answer goes onto the
    /// wall nearest them. See `design/projection.md`.
    origin: Option<[f32; 3]>,
    /// The connection whose client asked for [`ServerMessage::TranscriptProgress`] as it began the utterance, if it did. Progress is sent only there, as the participant may reconnect with an older client, which would refuse it.
    progress: Option<ConnectionId>,
    /// When its end arrived, and with it the last of its audio, from which the time it takes to transcribe is logged.
    ended: Option<Instant>,
    /// Since when it has waited behind another utterance, if it waits now, and how long it waited before.
    waiting: Option<Instant>,
    waited: Duration,
}

impl UtteranceRecord {
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
}

/// A token holder's new name, to keep with their token. See [`Hub::keep_name`].
struct Rename {
    tokens: Arc<tomori_security::TokenStore>,
    principal: String,
    name: String,
    /// The participant who asked, by hub key, and the utterance that said the name, if they spoke it, to answer once it is kept. A name kept on their behalf, as when their token had none, answers nobody.
    asked: Option<(String, Option<String>)>,
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
fn error_message(error: impl Into<String>) -> ServerMessage {
    ServerMessage::Error {
        error: limits::truncate(&error.into(), limits::ERROR_BYTES),
        space: None,
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
    fn touch_projection(&self, space: &str) {
        for member in self
            .space(space)
            .into_iter()
            .flat_map(|record| record.model.members())
        {
            if let Some(connection) = self
                .participants
                .get(member)
                .and_then(|participant| participant.connection)
                .and_then(|connection| self.connections.get(&connection))
                .filter(|connection| connection.projection)
            {
                connection.outbox.projection();
            }
        }
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
                    backend: model.backend.clone(),
                    directory: model.directory.clone(),
                    session: model.session.clone(),
                    environment: model.environment.clone(),
                    attribution: model.attribution(),
                    brief: model.speech() == SpeechSetting::Brief,
                    instructions: model.space_instructions().to_owned(),
                    closed: model
                        .closed_reason()
                        .filter(|_| !space.kept_open)
                        .map(str::to_owned),
                    closed_order: space.closed.filter(|_| !space.kept_open),
                    placements: space.placements.clone(),
                    stage: space.stage.clone(),
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
        let Some(space) = self.space_mut(id) else {
            return;
        };
        if let Some(prompt) = space.model.next_prompt() {
            let key = prompt.key.clone();
            if space.commands.try_send(SpaceCommand::Send(prompt)).is_err() {
                space
                    .model
                    .sent(&key, Err("the agent session has stopped".to_owned()));
            }
        }
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
        if participates && record.speech.joined(participant, engagement) {
            record.wake();
        }
        let Some(record) = self.participants.get_mut(participant) else {
            unreachable!("found above, and departing forgets no participant");
        };
        record.space = Some(space.to_owned());
        record.last_space = Some(space.to_owned());
        // Where they stood in another space means nothing in this one.
        record.head = None;
        outbox.clear_audio();
        self.update_role(participant);
        if participates {
            let knocks = self.knocks_for(space);
            self.send(participant, ServerMessage::Knocks { knocks });
        }
        self.forget_closed();
        self.touch_space(space);
        // After the space's state, what is projected in it, for a client that reads it.
        if self
            .participants
            .get(participant)
            .and_then(|record| record.connection)
            .and_then(|connection| self.connections.get(&connection))
            .is_some_and(|connection| connection.projection)
        {
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

    /// Whether `name` is taken from `participant`, as a name they cannot choose: whoever else is in the space they are in has it, or another token than theirs does, compared as [`tomori_protocol::text::name_key`] compares names. Their own devices, holding their token, do not count. See `design/security.md`.
    fn name_taken(
        &self,
        participant: &str,
        name: &str,
        tokens: Option<&tomori_security::TokenStore>,
    ) -> bool {
        let key = tomori_protocol::text::name_key(name);
        let (principal, space) = self
            .participants
            .get(participant)
            .map(|record| (record.principal.as_deref(), record.space.as_ref()))
            .unwrap_or_default();
        let present = space.is_some_and(|space| {
            self.participants.iter().any(|(other, there)| {
                other != participant
                    && (principal.is_none() || there.principal.as_deref() != principal)
                    && there.space.as_ref() == Some(space)
                    && tomori_protocol::text::name_key(&there.name) == key
            })
        });
        present
            || tokens.is_some_and(|tokens| {
                tokens.entries().iter().any(|entry| {
                    Some(entry.principal().id.as_str()) != principal
                        && tomori_protocol::text::name_key(&entry.name) == key
                })
            })
    }

    /// Rename `participant`, as they asked, from a message or an utterance's transcript, `utterance`, unless the name is taken (see [`Self::name_taken`]). A token holder's name is kept with their token, which the server writes outside the lock, and everyone holding it is renamed once it is written; anyone else's is kept only while the server keeps them. Renaming to the name they have already changes nothing. See `design/security.md`.
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
        if !unchanged && self.name_taken(participant, name, tokens.as_deref()) {
            let error = "That name is taken.";
            match utterance {
                Some(utterance) => self.send(
                    participant,
                    ServerMessage::TranscriptFailed {
                        utterance,
                        error: error.to_owned(),
                    },
                ),
                None => self.error(participant, format!("Name not changed: {error}")),
            }
            return;
        }
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
    fn admit(&mut self, knock: &KnockRecord, principal: String, token: String) -> bool {
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
            self.utterances.remove(&open.id);
            deferred.push(Deferred::Cancel(open.id));
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
        let hub = Arc::new_cyclic(|this| Self {
            state: Mutex::new(HubState {
                run: run_token(),
                ..HubState::default()
            }),
            backends,
            transcriber,
            jobs,
            slots,
            roots: RwLock::new(roots),
            environments: RwLock::new(Environments::default()),
            server_sides: crate::server_sides::ServerSides::default(),
            deferred_snapshot: Notify::new(),
            counts_changed: Notify::new(),
            cards_watched: Notify::new(),
            max_visitors: AtomicUsize::new(usize::MAX),
            tokens: RwLock::new(None),
            kept: RwLock::new(None),
            encrypted: AtomicBool::new(false),
            pairing: RwLock::new(None),
            pairing_prompt: RwLock::new(None),
            pairing_timeout: RwLock::new(PAIRING_TIMEOUT),
            knock_timeout: RwLock::new(KNOCK_TIMEOUT),
            tools: tools::Tools::default(),
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

    /// What the side of `outlet`'s binding keeps and sends: a new `state`, kept for whoever joins and sent to everyone in the space now unless an `event` goes with it, which is sent to them instead, except as the binding's first update, which is always sent as its state. A side of a binding that has ended, or of a space that has gone, is ignored. See `design/environments.md`.
    fn channel_update(
        &self,
        outlet: &SpaceOutlet,
        state_text: &str,
        event_text: Option<&str>,
    ) -> tomori_environment::Result<()> {
        let kept = crate::channel::state(state_text)?;
        let event = event_text.map(crate::channel::event).transpose()?;
        let mut state = self.lock();
        let Some(record) = self.current_channel(&mut state, outlet) else {
            return Ok(());
        };
        // Clients ignore an event until they have a state, so the first update of a binding is sent as its state, which already includes any event that goes with it.
        let first = record.state.replace(kept.clone()).is_none();
        let message = match event.filter(|_| !first) {
            Some(event) => ServerMessage::EnvironmentEvent {
                environment: outlet.environment.clone(),
                version: outlet.version,
                event,
            },
            None => ServerMessage::EnvironmentState {
                environment: outlet.environment.clone(),
                version: outlet.version,
                state: kept,
            },
        };
        let record = state.space(&outlet.space).expect("found above");
        let channel = record.channel.as_ref().expect("kept above");
        for outbox in state.channel_outboxes(record, channel) {
            outbox.message(message.clone());
        }
        Ok(())
    }

    /// The objects the side of `outlet`'s binding describes, for the agent; see [`Self::channel_update`].
    fn channel_describe(
        &self,
        outlet: &SpaceOutlet,
        objects: Vec<tomori_environment::Object>,
    ) -> tomori_environment::Result<()> {
        let objects = crate::channel::objects(objects);
        let mut state = self.lock();
        if let Some(record) = self.current_channel(&mut state, outlet) {
            record.objects = objects;
        }
        Ok(())
    }

    /// What the space of `outlet` keeps for its binding, begun anew for a new binding, or `None` if the binding has ended or the space has gone.
    fn current_channel<'a>(
        &self,
        state: &'a mut HubState,
        outlet: &SpaceOutlet,
    ) -> Option<&'a mut crate::channel::Channel> {
        let current = self.server_sides.binding(&outlet.space) == Some(outlet.binding);
        let record = state
            .space_mut(&outlet.space)
            .filter(|record| current && record.model.environment == outlet.environment)?;
        let channel = record.channel.get_or_insert_with(|| {
            crate::channel::Channel::new(outlet.binding, &outlet.environment, outlet.version)
        });
        if channel.binding != outlet.binding {
            *channel =
                crate::channel::Channel::new(outlet.binding, &outlet.environment, outlet.version);
        }
        Some(channel)
    }

    /// Start and stop the server sides of the environments to match the spaces that are open and the environments they are bound to. Called whenever a space is bound, closes or changes environment, the environments offered change, or the hub shuts down, so that each is run for as it should be whichever came first. See `design/environments.md`.
    fn reconcile_server_sides(&self, state: &mut HubState) {
        let wanted: Vec<crate::server_sides::Wanted> = if state.shutting_down || !state.serving {
            Vec::new()
        } else {
            state
                .spaces
                .iter()
                .filter(|record| !record.model.is_closed() && !record.restoring)
                .map(|record| crate::server_sides::Wanted {
                    id: record.model.id.clone(),
                    title: record.model.title.clone(),
                    directory: record.model.directory.clone(),
                    environment: record.model.environment.clone(),
                })
                .collect()
        };
        let failures = self.server_sides.reconcile(&self.environments(), &wanted);
        for failure in failures {
            self.show_side_failure(state, failure);
        }
        // What a side kept is forgotten with its binding, so that nobody joining is sent another environment's state, or an earlier run's.
        for record in &mut state.spaces {
            if record.channel.as_ref().is_some_and(|channel| {
                self.server_sides.binding(&record.model.id) != Some(channel.binding)
            }) {
                record.channel = None;
            }
        }
    }

    /// Move the space `space` to `environment`, one the server offers, for everyone in it, as an owner or its agent asked: whether it changed, or why it cannot, as for a closed space. Keeping the change is the caller's. See `design/environments.md` and `design/mcp.md`.
    fn move_space(
        &self,
        state: &mut HubState,
        space: &str,
        environment: String,
    ) -> Result<bool, &'static str> {
        let record = state.space_mut(space).expect("the caller found it");
        let left = record.model.environment.clone();
        if !record.model.set_environment(environment)? {
            return Ok(false);
        }
        // What its stage keeps for the environment left is the last forgotten. See `design/projection.md`.
        record.stage.leave(&left);
        // The environment's layer of the agent's instructions changes with it, so every layer is given again, as on binding.
        record.instruct(true);
        // Its server side, if it has one, runs for the space, and the previous environment's stops.
        self.reconcile_server_sides(state);
        // Everyone in the space, and the lobby's listing, see the change.
        state.touch_space(space);
        // The Tomori wall moves to where the new environment puts it, and that environment's walls show.
        state.touch_projection(space);
        Ok(true)
    }

    /// The server has begun serving: run the server sides of the spaces that were bound while it was being configured, and of those bound from now on. A kept space listed again waits until it is bound to its session again; see [`Hub::bound`].
    pub(crate) fn serving(&self) {
        let mut state = self.lock();
        state.serving = true;
        self.reconcile_server_sides(&mut state);
    }

    /// A server side failed after it started: show it in its space, if the space is still bound to the environment it ran for.
    fn side_failed(&self, failure: crate::server_sides::Failure) {
        let mut state = self.lock();
        self.show_side_failure(&mut state, failure);
    }

    fn show_side_failure(&self, state: &mut HubState, failure: crate::server_sides::Failure) {
        if !self
            .server_sides
            .is_current(&failure.space, failure.binding)
        {
            return;
        }
        let Some(record) = state.space_mut(&failure.space) else {
            return;
        };
        record.model.fail_environment(failure.message);
        state.touch_space(&failure.space);
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

    /// The environments offered for spaces.
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
                    });
                }
                return;
            }
        };
        let admitted = {
            let mut state = self.lock();
            let admitted = state.admit(&knock, entry.principal().id, token);
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
            name: pairing.name.clone(),
            code: pairing
                .code
                .as_deref()
                .map(tomori_security::pairing::display),
        })
    }

    /// Confirm the pairing waiting, issuing its device an owner token. Returns the device's name.
    pub fn confirm_pairing(&self, confirmation: Confirmation<'_>) -> Result<String, &'static str> {
        let (pairing, outbox) = {
            let mut state = self.lock();
            let pairing = state.pairing.as_ref().ok_or("No pairing is waiting.")?;
            let code = pairing
                .code
                .as_deref()
                .ok_or("The device has not shown its code yet.")?;
            let confirmed = match confirmation {
                Confirmation::Typed(typed) => typed == code,
                Confirmation::Shown(serial) => serial == pairing.serial,
            };
            if !confirmed {
                return Err(match confirmation {
                    Confirmation::Typed(_) => "That code does not match, so nothing was confirmed.",
                    Confirmation::Shown(_) => {
                        "A different pairing is waiting now, so nothing was confirmed."
                    }
                });
            }
            let pairing = state.pairing.take().expect("checked above");
            let outbox = pairing.outbox.clone();
            (pairing, outbox)
        };
        // The token is issued outside the hub's lock, since issuing writes the token store.
        let issued = pairing
            .setup
            .tokens
            .issue(&pairing.name, tomori_security::Grant::Owner)
            .map_err(|error| {
                log::error!("Could not issue a token for a pairing: {error}.");
                "The token could not be issued."
            });
        match issued {
            Ok((entry, token)) => {
                log::info!("Paired {:?} as owner token {}.", pairing.name, entry.id);
                outbox.message(ServerMessage::Paired { token });
                Ok(pairing.name)
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
        let state = self.lock();
        let now = Instant::now();
        state
            .pairing_attempts
            .iter()
            .filter(|began| now.saturating_duration_since(**began) < PAIRING_WINDOW)
            .count()
            >= PAIRING_ATTEMPTS
    }

    /// Allow pairings again, forgetting those that began recently, as the operator or an owner may when pairing is paused, such as by someone using up the attempts.
    pub fn allow_pairing(&self) {
        self.lock().pairing_attempts.clear();
    }

    /// Decline the pairing waiting, telling its device: the one with `serial` if given, as the status page showed it, or whichever is waiting. Returns whether one was declined.
    pub fn decline_pairing(&self, serial: Option<u64>) -> bool {
        let mut state = self.lock();
        if serial.is_some_and(|serial| {
            state
                .pairing
                .as_ref()
                .is_none_or(|pairing| pairing.serial != serial)
        }) {
            return false;
        }
        state.end_pairing(None, "The pairing was declined.")
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
                Deferred::Keep => self.keep(),
                Deferred::PairingPrompt(event) => {
                    if let Some(prompt) = &*self.pairing_prompt.read().expect("never left poisoned")
                    {
                        prompt(&event);
                    }
                }
            }
        }
    }

    /// Keep a token holder's new name with their token, then call everyone holding it by it, on every device, and answer whoever asked. Called outside the lock, since renaming writes the token store.
    fn keep_name(&self, rename: Rename) {
        let Rename {
            tokens,
            principal,
            name,
            asked,
        } = rename;
        let id = principal.strip_prefix("token:").unwrap_or_default();
        let key = tomori_protocol::text::name_key(&name);
        let kept = match tokens.rename(id, &name, |other| {
            tomori_protocol::text::name_key(other) == key
        }) {
            Ok(tomori_security::Renamed::Renamed) => Ok(()),
            Ok(tomori_security::Renamed::Taken) => Err("That name is taken."),
            Ok(_) => Err("Your token is no longer issued."),
            Err(error) => {
                log::warn!("Could not keep the name of token {id}: {error}.");
                Err("Your name could not be kept. Try again.")
            }
        };
        let mut state = self.lock();
        if kept.is_ok() {
            log::info!("Token {id} is now called {name:?}.");
            state.rename(
                |_, record| record.principal.as_deref() == Some(principal.as_str()),
                &name,
            );
        }
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
        for (id, cancellation) in expired {
            // Release the participant now; cancelling also ends the exchange, even one blocked writing to a peer that is gone.
            self.disconnect(id);
            cancellation.cancel();
        }
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
            } = message
            {
                // Before Hello rejoins the space they were in, so that they are sent its environment's state and what is projected in it.
                if let Some(record) = state.connections.get_mut(&connection) {
                    record.environment_state = environment_state;
                    record.projection = projection;
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
                    None => self.act(
                        &mut state,
                        connection,
                        &participant,
                        space.as_deref(),
                        message,
                        &mut deferred,
                    ),
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
        let participant = participant_key(access.principal.as_deref(), &participant);
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
                    source,
                    connection: None,
                    space: None,
                    last_space: None,
                    departed: None,
                    engagement: None,
                    intents: crate::channel::Intents::new(Instant::now()),
                    head: None,
                },
            );
        }
        let record = state
            .participants
            .get_mut(&participant)
            .expect("inserted above");
        record.name = name;
        record.departed = None;
        // A new connection may be from a client that reports nothing, which stays at the live edge.
        record.engagement = None;
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
        });
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
                outbox.message(ServerMessage::Left);
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
                attention,
            } => {
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
                        });
                if let Err(error) = checked {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state) else {
                    return;
                };
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
                let attention = match state.space(&space) {
                    Some(record) if answer.is_none() && !naming => {
                        let environments = self.environments();
                        let walls = environments.walls(&record.model.environment);
                        let now = crate::cards::unix_now();
                        record.model.resolve_with(
                            &attention,
                            state.cards_of(&space),
                            record.channel.as_ref(),
                            &|panel| record.stage.describe(&walls, panel, now),
                        )
                    }
                    _ => Vec::new(),
                };
                let origin = state.participants[participant].head;
                state.next_utterance += 1;
                let id = UtteranceId(state.next_utterance);
                state.utterances.insert(
                    id,
                    UtteranceRecord {
                        participant: participant.to_owned(),
                        source: state.participants[participant].source,
                        client: utterance.clone(),
                        space,
                        // Words that name the participant answer nothing, since they are never queued.
                        answer,
                        naming,
                        attention,
                        origin,
                        progress: progress.then_some(connection),
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
                            record.attention.is_empty() && record.answer.is_none() && !record.naming
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
                                    &|panel| space.stage.describe(&walls, panel, now),
                                )
                            })
                        })
                });
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
            } => {
                let Some(space) = space.and_then(|space| state.space(space)) else {
                    return;
                };
                let source = state.participants[participant].source;
                let at = head.position;
                let head_at = at.iter().all(|axis| axis.is_finite()).then_some(at);
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
                state
                    .participants
                    .get_mut(participant)
                    .expect("a member is a participant")
                    .head = head_at;
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
                // Refused even to the same connection, which must not get a fresh value from the server once it has seen one; see `design/security.md`.
                if state.pairing.is_some() {
                    outbox.message(failed(
                        "A pairing is already in progress; try again shortly.",
                    ));
                    return;
                }
                let now = Instant::now();
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
                let name = state.participants[participant].name.clone();
                log::info!("{name:?} began pairing.");
                deferred.push(Deferred::PairingPrompt(PairingEvent::Began {
                    name: name.clone(),
                }));
                state.pairing = Some(PendingPairing {
                    serial: state.next_pairing,
                    connection,
                    outbox: outbox.clone(),
                    setup,
                    name,
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
                let name = pairing.name.clone();
                pairing.code = Some(code.clone());
                outbox.message(ServerMessage::PairWaiting);
                let shown = tomori_security::pairing::display(&code);
                log::info!("Pairing request from {name:?}: code {shown}.");
                deferred.push(Deferred::PairingPrompt(PairingEvent::Waiting {
                    name,
                    code: shown,
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
                if let Some(record) = space.and_then(|space| state.space_mut(space)) {
                    record.speech.stop(participant);
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
                if let Some(space) = space
                    && let Some(record) = state.space_mut(space)
                {
                    if record.speech.engagement(participant, engagement) {
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
            ClientMessage::AddWall { pose } => {
                let Some(space) = in_space(state) else {
                    return;
                };
                let record = state.space_mut(&space).expect("checked above");
                match record.stage.add_wall(&record.model.environment, &pose) {
                    Ok(_) => {
                        state.touch_projection(&space);
                        deferred.push(Deferred::Keep);
                    }
                    Err(error) => state.error(participant, error),
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
                let record = state.space_mut(&space).expect("checked above");
                match record.stage.remove_wall(&record.model.environment, &wall) {
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
                let walls = environments.walls(&record.model.environment);
                match record.stage.place(&walls, &panel, target) {
                    Ok(true) => {
                        state.touch_projection(&space);
                        deferred.push(Deferred::Keep);
                    }
                    // Already there, it is sent again, so that a client showing it elsewhere meanwhile puts it back.
                    Ok(false) => state.touch_projection(&space),
                    Err(error) => state.error(participant, error),
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
                match record.stage.dismiss(&record.model.environment, &panel) {
                    Ok(()) => {
                        state.touch_projection(&space);
                        deferred.push(Deferred::Keep);
                    }
                    Err(error) => state.error(participant, error),
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
                    state.utterances.remove(&open.id);
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
        if let Some(origin) = record.origin {
            space.model.originate(&key, origin);
        }
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
    }

    pub(crate) fn transcript_failed(&self, utterance: UtteranceId, error: &str) {
        let mut deferred = Vec::new();
        {
            let mut state = self.lock();
            let Some(record) = state.utterances.remove(&utterance) else {
                return;
            };
            record.log_outcome(utterance, "failed to transcribe");
            // Recognition can fail while the utterance is still open: close it, so that the client may forget it without cancelling and its next utterance does not fail this one again. The transcriber is still told, as every utterance it began ends with an end or a cancel.
            for connection in state.connections.values_mut() {
                if let Some(open) = connection.utterance.take_if(|open| open.id == utterance) {
                    connection.failed = Some(open.client);
                    deferred.push(Deferred::Cancel(utterance));
                }
            }
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
            stage: crate::stage::Stage::default(),
        });
        // Tomori's tools are registered for its directory, if they are not already.
        self.tools_due(state.shutting_down, &request.backend, &request.directory);
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
                model.set_attribution(kept.attribution);
                if kept.brief {
                    model
                        .set_speech(SpeechSetting::Brief)
                        .expect("brief is a known setting");
                }
                let takes = self.takes_instructions(&kept.backend);
                model.set_instructions(kept.instructions, takes);
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
                state.spaces.push(SpaceRecord {
                    model,
                    commands,
                    touched: Cell::new(None),
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
                    stage: kept.stage,
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
        *self.kept.write().expect("never left poisoned") = Some(store);
        restores
    }

    /// Bind the space `id`, listed again after a restart, to its session. Returns false, binding nothing, once shutdown has begun: the caller must close the session.
    pub(crate) fn bound(&self, id: &str, session: String, snapshot: Snapshot) -> bool {
        let mut state = self.lock();
        if state.shutting_down {
            return false;
        }
        let Some(record) = state.space_mut(id) else {
            return false;
        };
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

    /// Write the kept spaces, if there is somewhere to keep them and they changed. Spaces closed once shutdown has begun are kept as open; see `stamp_closed`.
    pub(crate) fn keep(&self) {
        let Some(store) = self.kept.read().expect("never left poisoned").clone() else {
            return;
        };
        let written = store.keep(|| Some(self.lock().kept()));
        if let Err(error) = written {
            log::error!(
                "Could not keep the spaces in {}: {error}.",
                store.path().display()
            );
        }
    }

    pub(crate) fn create_failed(&self, creator: &str, error: String) {
        self.lock().error(creator, error);
    }

    /// Report a backend's session list to `participant`, its failures as an error.
    pub(crate) fn sessions(
        &self,
        participant: &str,
        result: Result<Vec<tomori_backend::SessionSummary>, String>,
    ) {
        let message = match result {
            Ok(sessions) => ServerMessage::Sessions {
                sessions: sessions
                    .into_iter()
                    .map(|session| tomori_protocol::SessionSummary {
                        id: session.id,
                        title: session.title,
                        directory: session.directory,
                    })
                    .collect(),
            },
            Err(error) => ServerMessage::Error {
                error: limits::truncate(&error, limits::ERROR_BYTES),
                space: None,
            },
        };
        self.lock().send(participant, message);
    }

    pub(crate) fn space_event(&self, space: &str, event: Event) {
        let mut state = self.lock();
        let shutting_down = state.shutting_down;
        let Some(record) = state.space_mut(space) else {
            return;
        };
        let resynchronized = matches!(event, Event::Resynchronized { .. });
        let was_open = !record.model.is_closed();
        record.model.apply(event);
        // The panels of a reply it completed go onto the wall nearest whoever asked, or else the Tomori wall, its code
        // panels replacing those of the replies before.
        let replies = record.model.take_projected();
        let projected = !replies.is_empty();
        if projected {
            let environments = self.environments();
            let walls = environments.walls(&record.model.environment);
            let now = crate::cards::unix_now();
            for (contents, origin) in replies {
                for id in record.stage.project_reply(&walls, contents, origin, now) {
                    log::info!("Space {space} projected {id}.");
                }
            }
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
        state.pump(space);
        if state.touch_space_paced(space, Instant::now()) {
            self.deferred_snapshot.notify(usize::MAX);
        }
        // The space's attention may have changed.
        self.recount(&mut state);
        if projected {
            state.touch_projection(space);
            drop(state);
            self.keep();
        }
    }

    /// When the earliest deferred snapshot is due, if any.
    pub fn snapshot_deadline(&self) -> Option<Instant> {
        let state = self.lock();
        state
            .spaces
            .iter()
            .filter_map(|space| space.deferred.get())
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

    /// Offer `environment` too, in place of one with the same identifier, or else after those offered.
    pub(crate) fn offer_environment(&self, environment: crate::Environment) {
        let mut environments = self
            .environments
            .write()
            .expect("environments are never left poisoned");
        *environments = environments.clone().with(environment);
    }

    /// The environments offered, as clients are told them.
    #[cfg(any(test, feature = "fake"))]
    pub(crate) fn environment_info(&self) -> Vec<tomori_protocol::EnvironmentInfo> {
        self.environments().info()
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

    /// Send the deferred snapshots that are due at `now`.
    pub fn flush_snapshots(&self, now: Instant) {
        let state = self.lock();
        let due: Vec<String> = state
            .spaces
            .iter()
            .filter(|space| space.deferred.get().is_some_and(|due| due <= now))
            .map(|space| space.model.id.clone())
            .collect();
        for space in due {
            state.touch_space(&space);
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
    fn takes_instructions(&self, backend: &str) -> bool {
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

    /// Announce an agent speech segment to everyone in the space now, in order with its frames.
    #[cfg_attr(not(feature = "speech"), allow(dead_code))]
    fn live_speech(&self, space: &str, reply: u64, item: Item, mut send: impl FnMut(&Outbox)) {
        let mut state = self.lock();
        let Some(record) = state.space_mut(space) else {
            return;
        };
        record.speech.record(reply, item);
        let state = &*state;
        let record = state.space(space).expect("found above");
        // Visitors have no place in the kept speech, so they always hear it live.
        let participants = record
            .model
            .participants
            .iter()
            .filter(|participant| record.speech.hears_live(participant));
        // Members are connected, since disconnecting leaves the space.
        for outbox in participants
            .chain(&record.model.visitors)
            .filter_map(|member| state.outbox(member))
        {
            send(outbox);
        }
    }

    /// Announce an agent speech segment of reply number `reply` to the space's participants who hear live speech, in order with its frames.
    #[cfg_attr(not(feature = "speech"), allow(dead_code))]
    pub(crate) fn agent_speech(&self, space: &str, reply: u64, message: ServerMessage) {
        self.live_speech(space, reply, Item::Announce(message.clone()), |outbox| {
            outbox.agent_message(message.clone())
        });
    }

    /// Send one agent speech frame of reply number `reply` to the space's participants who hear live speech. A participant who joined mid-segment and reports nothing receives from here on.
    #[cfg_attr(not(feature = "speech"), allow(dead_code))]
    pub(crate) fn agent_audio(&self, space: &str, reply: u64, frame: &AudioFrame) {
        let Ok(encoded) = Frame::audio(frame).try_encode() else {
            log::warn!("An agent speech frame for {space} exceeds the frame limit.");
            return;
        };
        let item = Item::Frame {
            sample_rate: frame.sample_rate,
            samples: frame.samples.as_slice().into(),
        };
        self.live_speech(space, reply, item, |outbox| {
            outbox.agent_audio(encoded.clone())
        });
    }

    /// A breath between two spoken segments of reply number `reply`, kept so that a catch-up pauses there too.
    #[cfg_attr(not(feature = "speech"), allow(dead_code))]
    pub(crate) fn agent_gap(&self, space: &str, reply: u64) {
        self.live_speech(space, reply, Item::Gap, |_| {});
    }

    /// Send each catching-up or replaying participant of `space` what is due by `now`. Returns `None` once the space is gone, and otherwise when the next is due, if anyone is still catching up or replaying.
    #[cfg_attr(not(feature = "speech"), allow(dead_code))]
    pub(crate) fn pace_speech(&self, space: &str, now: Instant) -> Option<Option<Instant>> {
        let mut state = self.lock();
        let record = state.space_mut(space)?;
        let mut deliveries = Vec::new();
        let next = record.speech.pace(now, &mut deliveries);
        // Speech is paced only for members, who are connected, since disconnecting leaves the space.
        for (outbox, delivery) in deliveries
            .into_iter()
            .filter_map(|(participant, delivery)| {
                state.outbox(&participant).map(|outbox| (outbox, delivery))
            })
        {
            match delivery {
                Delivery::Message(message) => outbox.agent_message(message),
                // Kept only once it was encoded live, so it encodes again, as the same frame from another source; one that did not would be skipped.
                Delivery::Frame(frame) => {
                    let _ = Frame::audio(&frame)
                        .try_encode()
                        .map(|encoded| outbox.agent_audio(encoded));
                }
            }
        }
        Some(next)
    }

    /// Let the space's speech pacer be woken when a replay or catch-up starts.
    #[cfg_attr(not(feature = "speech"), allow(dead_code))]
    pub(crate) fn set_speech_waker(&self, space: &str, waker: Box<dyn Fn() + Send>) {
        if let Some(record) = self.lock().space_mut(space) {
            record.waker = Some(waker);
        }
    }

    /// How much of each reply the space speaks; full for a space that is gone.
    #[cfg_attr(not(feature = "speech"), allow(dead_code))]
    pub(crate) fn speech_setting(&self, space: &str) -> SpeechSetting {
        self.lock()
            .space(space)
            .map_or(SpeechSetting::Full, |record| record.model.speech())
    }

    /// Tell everyone in the space that agent speech could not be produced.
    #[cfg_attr(not(feature = "speech"), allow(dead_code))]
    pub(crate) fn speech_failed(&self, space: &str, error: String) {
        let state = self.lock();
        let Some(record) = state.space(space) else {
            return;
        };
        for member in record.model.members() {
            state.error(member, error.clone());
        }
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
                let environments = self.environments();
                let mut state = space.model.state(
                    participants,
                    environments.instructions(&space.model.environment),
                );
                // Named before the state is fitted to its budget, which counts them.
                state.environment_version = environments.version(&space.model.environment);
                // Every space projects. See `design/projection.md`.
                state.projection = true;
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
                let state = self.lock();
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
                let walls = environments.walls(&space.model.environment);
                let projection =
                    space
                        .stage
                        .view(&space.model.id, &walls, crate::cards::unix_now());
                Some(ServerMessage::Projection {
                    projection: Box::new(projection),
                })
            }
            Outgoing::Spaces => {
                let state = self.lock();
                let now = Instant::now();
                let mut spaces: Vec<_> = state
                    .spaces
                    .iter()
                    .map(|space| space.summary(now))
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
mod tests {
    use super::*;
    use crate::transcriber::NullTranscriber;

    fn hub() -> Arc<Hub> {
        let (jobs, _) = async_channel::unbounded();
        Hub::new(
            Backends::new(),
            Arc::new(NullTranscriber::new()),
            jobs,
            Vec::new(),
            SpaceSlots::new(crate::workspace::DEFAULT_MAX_SPACES),
        )
    }

    #[test]
    fn a_request_that_fails_holding_the_state_fails_alone() {
        let hub = hub();
        let failing = hub.clone();
        let failed = std::thread::spawn(move || {
            let _state = failing.lock();
            panic!("a request fails holding the hub's state");
        })
        .join();
        assert!(failed.is_err());
        assert!(hub.state.is_poisoned());
        // Later requests are served, from the state as it was left.
        assert_eq!(hub.counts().spaces, 0);
        assert!(!hub.state.is_poisoned());
    }

    #[cfg(feature = "fake")]
    #[test]
    fn the_welcome_says_how_long_an_utterance_may_be() {
        use crate::test_support::{TestClient, TestServer};
        let welcome = |server: &TestServer| {
            let mut client = TestClient::open(server.address);
            client.send(&ClientMessage::Hello {
                protocol: PROTOCOL_VERSION,
                participant: "ada".into(),
                name: "Ada".into(),
                names: false,
                environments: None,
                environment_state: false,
                projection: false,
            });
            // The welcome is the first reply to hello.
            client.wait_for(|message| {
                let ServerMessage::Welcome {
                    utterance_limit, ..
                } = message
                else {
                    unreachable!("the welcome comes first, not {message:?}");
                };
                Some(*utterance_limit)
            })
        };
        // Recognition without a limit gives none.
        let server = TestServer::start();
        assert_eq!(welcome(&server), None);
        let server = TestServer::with_server(crate::fake::FakeBackend::new(), |backends| {
            crate::Server::new(
                backends,
                Arc::new(crate::test_support::TextTranscriber::limited(
                    std::time::Duration::from_secs(60),
                )),
            )
        });
        assert_eq!(welcome(&server), Some(60));
    }

    #[cfg(feature = "fake")]
    #[test]
    fn instructions_queue_once_and_are_read_when_given() {
        let (jobs, _) = async_channel::unbounded();
        let hub = Hub::new(
            Backends::new().with(crate::fake::FakeBackend::new()),
            Arc::new(NullTranscriber::new()),
            jobs,
            Vec::new(),
            SpaceSlots::new(crate::workspace::DEFAULT_MAX_SPACES),
        );
        let (commands, receiver) = async_channel::unbounded();
        let request = CreateRequest {
            title: "Space".to_owned(),
            backend: "fake".to_owned(),
            directory: "/tmp".to_owned(),
            session: None,
            creator: "ada".to_owned(),
            creator_space: None,
            environment: "sky".to_owned(),
            instructions: "Mind the docs.".to_owned(),
        };
        let id = hub
            .register_space(
                &request,
                "session".to_owned(),
                Snapshot::default(),
                commands,
            )
            .unwrap();
        // Changes made while the command given on creation waits join it.
        for _ in 0..3 {
            hub.lock().space(&id).unwrap().instruct(false);
        }
        assert_eq!(receiver.len(), 1);
        let (entries, all) = hub.instructions_due(&id);
        assert!(all);
        assert_eq!(entries.len(), 3);
        // Once it is performed, a change of the space's own layer gives that alone, as it is when performed.
        hub.lock().space(&id).unwrap().instruct(false);
        hub.lock()
            .space_mut(&id)
            .unwrap()
            .model
            .set_space_instructions("Mind the tests.")
            .unwrap();
        assert_eq!(receiver.len(), 2);
        assert_eq!(
            hub.instructions_due(&id),
            (
                vec![(
                    crate::instructions::SPACE_KEY.to_owned(),
                    Some("Mind the tests.".to_owned())
                )],
                false
            )
        );
    }

    /// Connect and say hello as `participant`.
    fn greet(hub: &Hub, participant: &str) -> ConnectionId {
        let (connection, ..) = hub.connect(Cancellation::new(), Access::open()).unwrap();
        hub.handle(
            connection,
            ClientMessage::Hello {
                protocol: PROTOCOL_VERSION,
                participant: participant.to_owned(),
                name: participant.to_owned(),
                names: false,
                environments: None,
                environment_state: false,
                projection: false,
            },
        );
        connection
    }

    /// Register a space created by `creator`, as the supervisor does once its session has started.
    fn create(hub: &Hub, creator: &str) -> String {
        let (commands, _) = async_channel::unbounded();
        let request = CreateRequest {
            title: "Space".to_owned(),
            backend: "fake".to_owned(),
            directory: "/tmp".to_owned(),
            session: None,
            creator: creator.to_owned(),
            creator_space: None,
            environment: "sky".to_owned(),
            instructions: String::new(),
        };
        hub.register_space(
            &request,
            "session".to_owned(),
            Snapshot::default(),
            commands,
        )
        .expect("the hub is not shutting down")
    }

    fn counts(spaces: u32, participants: u32) -> Counts {
        Counts {
            spaces,
            participants,
            attention: Some(AttentionCounts::default()),
        }
    }

    /// `counts` with spaces needing attention.
    fn needing(counts: Counts, waiting: u32, problems: u32, replies: u32) -> Counts {
        Counts {
            attention: Some(AttentionCounts {
                waiting,
                problems,
                replies,
            }),
            ..counts
        }
    }

    /// Whether `action` notified a counts listener.
    fn notifies(hub: &Hub, action: impl FnOnce()) -> bool {
        let listener = hub.listen_counts();
        action();
        futures_lite::future::block_on(futures_lite::future::poll_once(listener)).is_some()
    }

    #[test]
    fn visitors_are_not_counted_as_participants() {
        let hub = hub();
        let (visitor, ..) = hub.connect(Cancellation::new(), Access::visitor()).unwrap();
        hub.handle(
            visitor,
            ClientMessage::Hello {
                protocol: PROTOCOL_VERSION,
                participant: "vic".to_owned(),
                name: "Vic".to_owned(),
                names: false,
                environments: None,
                environment_state: false,
                projection: false,
            },
        );
        assert_eq!(hub.counts(), counts(0, 0));
        greet(&hub, "ada");
        assert_eq!(hub.counts(), counts(0, 1));

        // The limit applies to visitors only.
        hub.set_max_visitors(1);
        assert!(
            hub.connect(Cancellation::new(), Access::visitor())
                .is_none()
        );
        assert!(hub.connect(Cancellation::new(), Access::open()).is_some());
        hub.disconnect(visitor);
        assert!(
            hub.connect(Cancellation::new(), Access::visitor())
                .is_some()
        );
    }

    #[test]
    fn only_so_many_disconnected_visitors_are_remembered() {
        let hub = hub();
        let visit = |identity: &str| {
            let (connection, ..) = hub.connect(Cancellation::new(), Access::visitor()).unwrap();
            hub.handle(
                connection,
                ClientMessage::Hello {
                    protocol: PROTOCOL_VERSION,
                    participant: identity.to_owned(),
                    name: identity.to_owned(),
                    names: false,
                    environments: None,
                    environment_state: false,
                    projection: false,
                },
            );
            connection
        };
        let owner = greet(&hub, "ada");
        hub.disconnect(owner);
        for index in 0..=RETAINED_VISITORS {
            let connection = visit(&format!("visitor-{index}"));
            hub.disconnect(connection);
        }
        let state = hub.lock();
        let visitors = state
            .participants
            .values()
            .filter(|record| record.visitor)
            .count();
        assert_eq!(visitors, RETAINED_VISITORS);
        // The first to leave was forgotten; the owner, not a visitor, is kept for the whole retention.
        assert!(!state.participants.contains_key("/visitor-0"));
        assert!(state.participants.contains_key("/visitor-1"));
        assert!(state.participants.contains_key("/ada"));
    }

    #[test]
    fn counts_follow_connections_and_listed_spaces() {
        let hub = hub();
        assert_eq!(hub.counts(), counts(0, 0));

        // A connection counts once it has said hello.
        let (silent, ..) = hub.connect(Cancellation::new(), Access::open()).unwrap();
        assert_eq!(hub.counts(), counts(0, 0));
        let mut ada = 0;
        assert!(notifies(&hub, || ada = greet(&hub, "ada")));
        assert_eq!(hub.counts(), counts(0, 1));
        let bob = greet(&hub, "bob");
        assert_eq!(hub.counts(), counts(0, 2));
        hub.disconnect(silent);

        let mut space = String::new();
        assert!(notifies(&hub, || space = create(&hub, "ada")));
        assert_eq!(hub.counts(), counts(1, 2));

        // Moving between a space and the lobby changes neither count.
        assert!(!notifies(&hub, || {
            hub.handle(
                bob,
                ClientMessage::Join {
                    space: space.clone(),
                },
            );
            hub.handle(bob, ClientMessage::Leave);
        }));
        assert_eq!(hub.counts(), counts(1, 2));

        // A reconnect replaces the participant's connection; it does not add one.
        let again = greet(&hub, "ada");
        assert_eq!(hub.counts(), counts(1, 2));
        hub.disconnect(ada);
        assert_eq!(hub.counts(), counts(1, 2));

        assert!(notifies(&hub, || hub.disconnect(again)));
        assert_eq!(hub.counts(), counts(1, 1));

        // A closed space is still listed, so it still counts. Nobody was in it when it closed, so it has a problem.
        assert!(notifies(&hub, || hub.space_ended(&space, None)));
        assert_eq!(hub.counts(), needing(counts(1, 1), 0, 1, 0));
        hub.disconnect(bob);
        assert_eq!(hub.counts(), needing(counts(1, 0), 0, 1, 0));
    }

    /// The lobby listing `connection` would be sent.
    fn listing(hub: &Hub, connection: ConnectionId) -> Vec<tomori_protocol::SpaceSummary> {
        let Some(ServerMessage::Spaces { spaces }) = hub.render(connection, Outgoing::Spaces)
        else {
            unreachable!("the lobby is rendered as a listing");
        };
        spaces
    }

    #[test]
    fn attention_is_listed_and_counted_until_someone_joins() {
        let hub = hub();
        let ada = greet(&hub, "ada");
        let space = create(&hub, "nobody");
        let quiet = listing(&hub, ada);
        assert_eq!((quiet[0].attention, quiet[0].attention_age), (None, None));

        // An approval while nobody is there, then the agent finishing a reply.
        let permission = tomori_backend::Permission {
            id: "p".into(),
            tool: "Bash".into(),
            input: serde_json::Value::Null,
            description: None,
            options: Vec::new(),
        };
        assert!(notifies(&hub, || hub
            .space_event(&space, Event::PermissionRequested(permission))));
        assert_eq!(hub.counts(), needing(counts(1, 1), 1, 0, 0));
        let listed = listing(&hub, ada);
        assert_eq!(listed[0].attention, Some(Attention::Input));
        assert_eq!(listed[0].attention_age, Some(0));

        // Its age counts from when it changed level, as stamped when participants are told.
        let record = |hub: &Hub| {
            let state = hub.lock();
            let since = state.space(&space).unwrap().attention.get().unwrap();
            (
                since.0,
                state
                    .space(&space)
                    .unwrap()
                    .summary(since.1 + Duration::from_secs(90)),
            )
        };
        let (level, later) = record(&hub);
        assert_eq!(level, Attention::Input);
        assert_eq!(later.attention_age, Some(90));

        hub.space_event(&space, Event::PermissionResolved { id: "p".into() });
        for event in [
            Event::Busy,
            Event::MessageStarted {
                message: "m".into(),
                role: tomori_backend::Role::Assistant,
            },
            Event::TextDelta {
                message: "m".into(),
                text: "Done.".into(),
            },
            Event::MessageCompleted {
                message: "m".into(),
            },
            Event::Idle,
        ] {
            hub.space_event(&space, event);
        }
        // A deferred snapshot stamps the new level once it is sent.
        hub.flush_snapshots(Instant::now() + SNAPSHOT_INTERVAL);
        assert_eq!(hub.counts(), needing(counts(1, 1), 0, 0, 1));
        assert_eq!(listing(&hub, ada)[0].attention, Some(Attention::Reply));
        assert_eq!(record(&hub).0, Attention::Reply);

        // Joining clears it for everyone.
        assert!(notifies(&hub, || hub.handle(
            ada,
            ClientMessage::Join {
                space: space.clone()
            }
        )));
        assert_eq!(hub.counts(), counts(1, 1));
        let bob = greet(&hub, "bob");
        assert_eq!(listing(&hub, bob)[0].attention, None);
    }

    #[test]
    fn being_put_back_in_a_space_on_reconnecting_does_not_clear_its_attention() {
        let hub = hub();
        let ada = greet(&hub, "ada");
        let space = create(&hub, &participant_key(None, "ada"));
        hub.disconnect(ada);
        // While Ada is away, the agent replies.
        for event in [
            Event::Busy,
            Event::MessageStarted {
                message: "m".into(),
                role: tomori_backend::Role::Assistant,
            },
            Event::TextDelta {
                message: "m".into(),
                text: "Done.".into(),
            },
            Event::MessageCompleted {
                message: "m".into(),
            },
            Event::Idle,
        ] {
            hub.space_event(&space, event);
        }
        assert_eq!(hub.counts(), needing(counts(1, 0), 0, 0, 1));
        // Reconnecting puts her back in the space, which she has not seen yet; a jump would move on from it.
        let ada = greet(&hub, "ada");
        assert_eq!(
            hub.lock().space(&space).unwrap().model.participants.len(),
            1
        );
        assert_eq!(hub.counts(), needing(counts(1, 1), 0, 0, 1));
        // A visitor watching it does not clear it.
        let (visitor, ..) = hub.connect(Cancellation::new(), Access::visitor()).unwrap();
        hub.handle(
            visitor,
            ClientMessage::Hello {
                protocol: PROTOCOL_VERSION,
                participant: "vic".to_owned(),
                name: "Vic".to_owned(),
                names: false,
                environments: None,
                environment_state: false,
                projection: false,
            },
        );
        hub.handle(
            visitor,
            ClientMessage::Join {
                space: space.clone(),
            },
        );
        assert_eq!(hub.lock().space(&space).unwrap().model.visitors.len(), 1);
        assert_eq!(hub.counts(), needing(counts(1, 1), 0, 0, 1));
        // Choosing to join it, as her client does to restore its membership, clears it.
        hub.handle(
            ada,
            ClientMessage::Join {
                space: space.clone(),
            },
        );
        assert_eq!(hub.counts(), counts(1, 1));
    }

    #[test]
    fn forgotten_closed_spaces_are_no_longer_counted() {
        let hub = hub();
        let spaces: Vec<String> = (0..=CLOSED_SPACES)
            .map(|_| create(&hub, "nobody"))
            .collect();
        let listed = u32::try_from(spaces.len()).unwrap();
        assert_eq!(hub.counts(), counts(listed, 0));
        for space in &spaces[..CLOSED_SPACES] {
            hub.space_ended(space, None);
        }
        // Each closed while nobody was in it.
        let closed = u32::try_from(CLOSED_SPACES).unwrap();
        assert_eq!(hub.counts(), needing(counts(listed, 0), 0, closed, 0));
        // Closing one more than are kept forgets the oldest.
        assert!(notifies(&hub, || hub.space_ended(&spaces[CLOSED_SPACES], None)));
        assert_eq!(hub.counts(), needing(counts(listed - 1, 0), 0, closed, 0));
    }

    #[test]
    fn a_participant_whose_space_was_forgotten_while_away_reconnects_to_the_listing() {
        let hub = hub();
        let ada = greet(&hub, "ada");
        let space = create(&hub, &participant_key(None, "ada"));
        hub.disconnect(ada);
        // Her space closes while she is away, and is forgotten as more spaces close than are kept.
        hub.space_ended(&space, None);
        for _ in 0..CLOSED_SPACES {
            let other = create(&hub, "nobody");
            hub.space_ended(&other, None);
        }
        assert!(!hub.has_space(&space));
        let ada = greet(&hub, "ada");
        let key = participant_key(None, "ada");
        assert_eq!(hub.lock().participants[&key].space, None);
        assert_eq!(hub.render(ada, Outgoing::Space), None);
        assert_eq!(listing(&hub, ada).len(), CLOSED_SPACES);
    }

    #[test]
    fn a_connection_whose_output_is_not_accepted_is_released_even_while_it_sends() {
        let hub = hub();
        let keepalive = Keepalive {
            interval: Duration::from_secs(5),
            timeout: Duration::from_secs(15),
        };
        let hello = Duration::from_secs(3600);
        let blocked = Cancellation::new();
        let (_, _, blocked_received, blocked_output) =
            hub.connect(blocked.clone(), Access::open()).unwrap();
        let flowing = Cancellation::new();
        let (_, _, flowing_received, flowing_output) =
            hub.connect(flowing.clone(), Access::open()).unwrap();

        let start = Instant::now();
        // Both peers keep sending. One write is never accepted; the other connection's writes complete.
        blocked_output.handed_out();
        for second in 0..=20 {
            blocked_received.store(true, Ordering::Release);
            flowing_received.store(true, Ordering::Release);
            flowing_output.asked();
            flowing_output.handed_out();
            hub.sweep(start + Duration::from_secs(second), &keepalive, hello);
            assert_eq!(blocked.is_cancelled(), second >= 15, "after {second} s");
        }
        assert!(!flowing.is_cancelled());
    }
}
