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

use backend::{AnswerValue, Backend, ErrorKind, Event, Prompt, Snapshot};
use base_scheduler::Cancellation;
use discovery::{AttentionCounts, Counts};
use event_listener::{Event as Notify, EventListener};
use protocol::{
    Attention, AudioFrame, ClientMessage, ConnectionRole, EnvironmentInfo, Frame, FrameKind, Gate,
    Keepalive, PROTOCOL_VERSION, Participant, ServerMessage, WorkspaceRoot,
};

use crate::access::{self, Access, Grant};
use crate::backends::Backends;
use crate::environments;
use crate::limits;
use crate::outbox::{Outbox, Outgoing};
use crate::space::{AnswerOutcome, SpaceModel};
use crate::transcriber::{Transcriber, TranscriptSink, UtteranceId};
use crate::workspace::SpaceSlots;

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
    pub activity: protocol::Activity,
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
#[derive(Clone, Debug)]
struct KnockRecord {
    id: String,
    space: String,
    /// The visitor, by hub key.
    participant: String,
    connection: ConnectionId,
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
    /// The device's display name, which names the token it receives.
    name: String,
    commitment: String,
    server: security::pairing::Nonce,
    code: Option<String>,
    started: Instant,
}

/// What pairing needs from the server: its certificate's fingerprint, which the code is derived from, and the token store, which issues the token. Without them, as in `--auth open` mode, the server does not pair.
#[derive(Clone)]
pub struct PairingSetup {
    pub fingerprint: security::Fingerprint,
    pub tokens: Arc<security::TokenStore>,
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
        decision: backend::Decision,
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
    environments: Vec<EnvironmentInfo>,
    /// Notified when a space defers a snapshot, so that the server's snapshot timer can wake for it.
    deferred_snapshot: Notify,
    /// Notified when [`Hub::counts`] changes, so that the server can advertise the new counts.
    counts_changed: Notify,
    /// The most visitors connected at once; see `design/security.md`.
    max_visitors: AtomicUsize,
    /// Issues collaborator tokens to admitted visitors; see [`Hub::set_tokens`].
    tokens: RwLock<Option<Arc<security::TokenStore>>>,
    /// Where the spaces are kept across restarts; see [`Hub::restore`].
    kept: RwLock<Option<Arc<crate::kept::SpaceStore>>>,
    /// Whether the server serves only TLS, which pairing needs: the token it issues must not cross the network in the clear.
    encrypted: AtomicBool,
    pairing: RwLock<Option<PairingSetup>>,
    pairing_prompt: RwLock<Option<PairingPrompt>>,
    pairing_timeout: RwLock<Duration>,
    knock_timeout: RwLock<Duration>,
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
    /// Set when a space was forgotten, so that the kept spaces are written once the lock is released.
    unkept: bool,
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
}

struct ConnectionRecord {
    outbox: Arc<Outbox>,
    /// What the connection was admitted as at the door.
    access: Access,
    /// The role the client was last told, so that it is told again only when it changes.
    role: Option<ConnectionRole>,
    participant: Option<String>,
    utterance: Option<OpenUtterance>,
    /// The client's id for the utterance the server last closed by failing it, so that an end or cancel already on its way is ignored rather than refused.
    failed: Option<String>,
    /// Push-to-talk frames that arrived while no utterance was open, which are relayed but not transcribed.
    stray_frames: u64,
    /// Whether the last push-to-talk frame was stray, so that each run of them is logged once.
    straying: bool,
    liveness: Liveness,
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
    name: String,
    source: u32,
    connection: Option<ConnectionId>,
    space: Option<String>,
    /// The space to rejoin when this participant reconnects.
    last_space: Option<String>,
    /// When the participant's last connection ended, while it has none.
    departed: Option<Instant>,
}

struct SpaceRecord {
    model: SpaceModel,
    commands: async_channel::Sender<SpaceCommand>,
    /// When participants were last told that the space changed. Cells, because telling them only needs shared access to the state.
    touched: Cell<Option<Instant>>,
    /// A change caused by agent events that participants have not been told about yet; see [`SNAPSHOT_INTERVAL`].
    deferred: Cell<bool>,
    /// Once the space has closed, when relative to other spaces, so that the least recently closed are forgotten first.
    closed: Option<u64>,
    /// Closed, but kept as open, and not forgotten, so that it is bound again when the server next starts: closed by shutdown, or not bound again after a restart for a reason that may pass. See `design/spaces.md`.
    kept_open: bool,
    /// The attention the space was last listed as needing, and since when, so the listing can say how long it has
    /// waited. Stamped after each agent event and whenever participants are told the space changed.
    attention: Cell<Option<(Attention, Instant)>>,
}

impl SpaceRecord {
    /// Note when the space's attention changed level, at `now`.
    fn stamp_attention(&self, now: Instant) {
        let level = self.model.attention();
        if self.attention.get().map(|(stamped, _)| stamped) != level {
            self.attention.set(level.map(|level| (level, now)));
        }
    }

    /// The space as listed in the lobby at `now`, with how long it has needed its attention.
    fn summary(&self, now: Instant) -> protocol::SpaceSummary {
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
}

/// An utterance whose transcript has not been reported yet.
struct UtteranceRecord {
    participant: String,
    /// The participant's number, their public handle, which keys their queued input.
    source: u32,
    client: String,
    space: String,
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

    /// Tell a participant's connection its role if it changed, such as on joining or leaving a space.
    fn update_role(&mut self, participant: &str) {
        let Some(record) = self.participants.get(participant) else {
            return;
        };
        let space = record.space.as_deref();
        let Some(connection) = record
            .connection
            .and_then(|id| self.connections.get_mut(&id))
        else {
            return;
        };
        let role = connection.access.role(space);
        if connection.role != Some(role) {
            connection.role = Some(role);
            connection.outbox.message(ServerMessage::Role { role });
        }
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

    fn space(&self, id: &str) -> Option<&SpaceRecord> {
        self.spaces.iter().find(|space| space.model.id == id)
    }

    fn space_mut(&mut self, id: &str) -> Option<&mut SpaceRecord> {
        self.spaces.iter_mut().find(|space| space.model.id == id)
    }

    /// Tell everyone in the space, and the lobby, that the space changed.
    fn touch_space(&self, id: &str) {
        if let Some(space) = self.space(id) {
            let now = Instant::now();
            space.touched.set(Some(now));
            space.deferred.set(false);
            space.stamp_attention(now);
            for member in space.model.members() {
                if let Some(outbox) = self.outbox(member) {
                    outbox.space();
                }
            }
        }
        self.touch_lobby();
    }

    /// Tell the space's participants that it changed because of an agent event, at most once per [`SNAPSHOT_INTERVAL`]. A change within the interval is deferred to its end, when the snapshot rendered then carries the latest state; returns true if so.
    fn touch_space_paced(&self, id: &str, now: Instant) -> bool {
        let Some(space) = self.space(id) else {
            return false;
        };
        if space.deferred.get() {
            return false;
        }
        if space
            .touched
            .get()
            .is_some_and(|touched| now < touched + SNAPSHOT_INTERVAL)
        {
            space.deferred.set(true);
            return true;
        }
        self.touch_space(id);
        false
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
                    closed: model
                        .closed_reason()
                        .filter(|_| !space.kept_open)
                        .map(str::to_owned),
                    closed_order: space.closed.filter(|_| !space.kept_open),
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
            .filter_map(|id| {
                let record = self.participants.get(id)?;
                Some(Participant {
                    name: record.name.clone(),
                    source: record.source,
                })
            })
            .collect()
    }

    /// Take a participant out of their space. A closed space left empty may be forgotten; see [`CLOSED_SPACES`].
    fn leave(&mut self, participant: &str) {
        self.depart(participant);
        self.forget_closed();
    }

    fn depart(&mut self, participant: &str) {
        let Some(record) = self.participants.get_mut(participant) else {
            return;
        };
        let Some(space) = record.space.take() else {
            return;
        };
        if let Some(record) = self.space_mut(&space) {
            record.model.participants.retain(|id| id != participant);
            record.model.visitors.retain(|id| id != participant);
        }
        self.withdraw_knock(participant, &space);
        self.touch_space(&space);
    }

    fn join(&mut self, participant: &str, space: &str) {
        // Not forgotten until the participant is in the space they join, which might be a closed one.
        self.depart(participant);
        let participates = self
            .participants
            .get(participant)
            .and_then(|record| record.connection)
            .is_some_and(|connection| self.role(connection, Some(space)).participates());
        let Some(record) = self.space_mut(space) else {
            return;
        };
        let members = if participates {
            &mut record.model.participants
        } else {
            &mut record.model.visitors
        };
        if !members.iter().any(|id| id == participant) {
            members.push(participant.to_owned());
        }
        if let Some(record) = self.participants.get_mut(participant) {
            record.space = Some(space.to_owned());
            record.last_space = Some(space.to_owned());
        }
        if let Some(outbox) = self.outbox(participant) {
            outbox.clear_audio();
        }
        self.update_role(participant);
        if participates {
            let knocks = self.knocks_for(space);
            self.send(participant, ServerMessage::Knocks { knocks });
        }
        self.forget_closed();
        self.touch_space(space);
    }

    /// The knocks waiting on `space`, as its participants see them.
    fn knocks_for(&self, space: &str) -> Vec<protocol::KnockRequest> {
        self.knocks
            .iter()
            .filter(|knock| knock.space == space)
            .map(|knock| protocol::KnockRequest {
                id: knock.id.clone(),
                name: knock.name.clone(),
            })
            .collect()
    }

    /// Tell the participants of `space` the knocks waiting on it.
    fn announce_knocks(&self, space: &str) {
        let Some(record) = self.space(space) else {
            return;
        };
        let knocks = self.knocks_for(space);
        for participant in &record.model.participants {
            self.send(
                participant,
                ServerMessage::Knocks {
                    knocks: knocks.clone(),
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
                    state: protocol::KnockState::Withdrawn,
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
        let Some(identity) = knock.participant.strip_prefix('/').filter(|_| still_here) else {
            return false;
        };
        let key = participant_key(Some(&principal), identity);
        let Some(mut record) = self.participants.remove(&knock.participant) else {
            return false;
        };
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
        let space = record.space.clone();
        self.participants.insert(key.clone(), record);
        let connection = self
            .connections
            .get_mut(&knock.connection)
            .expect("checked above");
        connection.participant = Some(key.clone());
        connection.access = Access {
            principal: Some(principal),
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
        if let Some(record) = self.connections.get(&pairing.connection) {
            record.outbox.message(ServerMessage::PairFailed {
                error: error.to_owned(),
            });
        }
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

    /// The greeted participant of a connection and their current space.
    fn member(&self, connection: ConnectionId) -> Option<(String, Option<String>)> {
        let participant = self.connections.get(&connection)?.participant.clone()?;
        let space = self.participants.get(&participant)?.space.clone();
        Some((participant, space))
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
        Arc::new_cyclic(|this| Self {
            state: Mutex::new(HubState {
                run: run_token(),
                ..HubState::default()
            }),
            backends,
            transcriber,
            jobs,
            slots,
            roots: RwLock::new(roots),
            environments: environments::offered(),
            deferred_snapshot: Notify::new(),
            counts_changed: Notify::new(),
            max_visitors: AtomicUsize::new(usize::MAX),
            tokens: RwLock::new(None),
            kept: RwLock::new(None),
            encrypted: AtomicBool::new(false),
            pairing: RwLock::new(None),
            pairing_prompt: RwLock::new(None),
            pairing_timeout: RwLock::new(PAIRING_TIMEOUT),
            knock_timeout: RwLock::new(KNOCK_TIMEOUT),
            this: this.clone(),
        })
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
    pub(crate) fn set_tokens(&self, tokens: Arc<security::TokenStore>) {
        *self.tokens.write().expect("never left poisoned") = Some(tokens);
    }

    /// Whether the space `id` is listed, so that a collaborator token for a space that is gone can be revoked.
    pub fn has_space(&self, id: &str) -> bool {
        self.lock().space(id).is_some()
    }

    /// Issue a collaborator token for `knock` and admit its visitor on the same connection, if it is still connected. Called outside the lock, since issuing writes the token store.
    fn admit(&self, knock: KnockRecord) {
        let Some(tokens) = self.tokens.read().expect("never left poisoned").clone() else {
            return;
        };
        let (entry, token) = match tokens.issue(
            &knock.name,
            security::Grant::Collaborator {
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
                    state: protocol::KnockState::Withdrawn,
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
        } else if let Err(error) = tokens.revoke(&entry.id) {
            log::warn!("Could not revoke the token of a visitor who left: {error}.");
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
            code: pairing.code.as_deref().map(security::pairing::display),
        })
    }

    /// Confirm the pairing waiting, issuing its device an owner token. Returns the device's name.
    pub fn confirm_pairing(&self, confirmation: Confirmation<'_>) -> Result<String, &'static str> {
        let pairing = {
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
            state.pairing.take().expect("checked above")
        };
        // The token is issued outside the hub's lock, since issuing writes the token store.
        let issued = self
            .pairing
            .read()
            .expect("never left poisoned")
            .clone()
            .ok_or("This server does not pair devices.")
            .and_then(|setup| {
                setup
                    .tokens
                    .issue(&pairing.name, security::Grant::Owner)
                    .map_err(|error| {
                        log::error!("Could not issue a token for a pairing: {error}.");
                        "The token could not be issued."
                    })
            });
        let state = self.lock();
        let outbox = state
            .connections
            .get(&pairing.connection)
            .map(|record| record.outbox.clone());
        match issued {
            Ok((entry, token)) => {
                log::info!("Paired {:?} as owner token {}.", pairing.name, entry.id);
                if let Some(outbox) = outbox {
                    outbox.message(ServerMessage::Paired { token });
                }
                Ok(pairing.name)
            }
            Err(error) => {
                if let Some(outbox) = outbox {
                    outbox.message(ServerMessage::PairFailed {
                        error: format!("{error} Try again."),
                    });
                }
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

    fn lock(&self) -> MutexGuard<'_, HubState> {
        self.state.lock().expect("hub state is never left poisoned")
    }

    /// How many spaces are listed and participants connected, as advertised; see `design/server.md`.
    pub fn counts(&self) -> Counts {
        self.lock().counts()
    }

    /// Who is where now, for the status page.
    pub fn status(&self) -> Status {
        let state = self.lock();
        let role = |key: &str, space: Option<&str>| {
            let record = state.participants.get(key)?;
            let connection = state.connections.get(&record.connection?)?;
            Some((record.name.clone(), connection.access.role(space)))
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
            .filter_map(|record| {
                let connection = state.connections.get(&record.connection?)?;
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
                    record
                        .outbox
                        .keepalive(ServerMessage::Ping { nonce: *next_ping });
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
                if let Some(record) = state.connections.get(&knock.connection) {
                    record.outbox.message(ServerMessage::KnockState {
                        state: protocol::KnockState::Expired,
                    });
                }
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
                let current = state
                    .participants
                    .get(&participant)
                    .is_some_and(|record| record.connection == Some(connection));
                if current {
                    state.leave(&participant);
                    state.next_departure += 1;
                    let departure = state.next_departure;
                    if let Some(record) = state.participants.get_mut(&participant) {
                        record.connection = None;
                        record.departed = Some(Instant::now());
                        record.departure = departure;
                    }
                    state.forget_excess_visitors();
                }
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
                    record.outbox.keepalive(ServerMessage::Pong { nonce });
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
                ..
            } = message
            {
                self.hello(
                    &mut state,
                    connection,
                    protocol,
                    participant,
                    name,
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
        participant: String,
        name: String,
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
        let invalid = if participant.is_empty() {
            Err("A participant identity is required.".to_owned())
        } else if name.trim().is_empty() {
            Err("A name is required.".to_owned())
        } else if name.trim_start().starts_with('/') {
            // The name begins each attributed prompt, where a leading slash could read as a command to the agent.
            Err("A name must not start with \"/\".".to_owned())
        } else {
            limits::check(
                &participant,
                limits::PARTICIPANT_BYTES,
                "participant identity",
            )
            .and_then(|()| limits::check(&name, limits::NAME_BYTES, "name"))
        };
        if let Err(error) = invalid {
            outbox.message(error_message(error));
            outbox.close();
            return;
        }
        let access = state.connections[&connection].access.clone();
        let participant = participant_key(access.principal.as_deref(), &participant);
        if !state.participants.contains_key(&participant) {
            state.next_source += 1;
            let source = state.next_source;
            state.participants.insert(
                participant.clone(),
                ParticipantRecord {
                    visitor: access.is_visitor(),
                    departure: 0,
                    name: String::new(),
                    source,
                    connection: None,
                    space: None,
                    last_space: None,
                    departed: None,
                },
            );
        }
        let record = state
            .participants
            .get_mut(&participant)
            .expect("inserted above");
        record.name = name;
        record.departed = None;
        let source = record.source;
        let previous = record.connection.replace(connection);
        let rejoin = record.space.clone().or_else(|| record.last_space.clone());
        // A reconnect supersedes the previous connection of the same participant.
        if let Some(previous) = previous.filter(|previous| *previous != connection) {
            state.abandon_utterance(previous, deferred);
            if let Some(old) = state.connections.get_mut(&previous) {
                old.participant = None;
                old.outbox.close();
            }
        }
        let record = state
            .connections
            .get_mut(&connection)
            .expect("checked above");
        record.participant = Some(participant.clone());
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
            environments: self.environments.clone(),
        });
        match rejoin.filter(|space| state.space(space).is_some()) {
            Some(space) => state.join(&participant, &space),
            None if role == ConnectionRole::Pairing => {}
            None => outbox.spaces(),
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
            ClientMessage::CreateSpace {
                title,
                backend,
                directory,
                session,
                environment,
            } => {
                let checked = limits::check(&title, limits::TITLE_BYTES, "title")
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
                let environment =
                    match environments::choose(&self.environments, environment.as_deref()) {
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
                if let Some(record) = state.participants.get_mut(participant) {
                    record.last_space = None;
                }
                state.update_role(participant);
                outbox.spaces();
            }
            ClientMessage::Gate { .. } => {
                // Each audio frame carries its own gate; the announcement needs no state yet.
            }
            ClientMessage::UtteranceBegin { utterance } => {
                if let Err(error) =
                    limits::check(&utterance, limits::UTTERANCE_BYTES, "utterance identifier")
                {
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
                state.next_utterance += 1;
                let id = UtteranceId(state.next_utterance);
                state.utterances.insert(
                    id,
                    UtteranceRecord {
                        participant: participant.to_owned(),
                        source: state.participants[participant].source,
                        client: utterance.clone(),
                        space,
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
            ClientMessage::UtteranceEnd { utterance }
            | ClientMessage::UtteranceCancel { utterance }
                if state.connections[&connection].failed.as_ref() == Some(&utterance) =>
            {
                // Already failed and reported: the client sent this before it heard.
                if let Some(record) = state.connections.get_mut(&connection) {
                    record.failed = None;
                }
            }
            ClientMessage::UtteranceEnd { utterance }
            | ClientMessage::UtteranceCancel { utterance }
                if !state.connections[&connection]
                    .utterance
                    .as_ref()
                    .is_some_and(|open| open.client == utterance) =>
            {
                state.error(participant, "That utterance is not open.");
            }
            ClientMessage::UtteranceEnd { .. } => {
                let open = state
                    .connections
                    .get_mut(&connection)
                    .and_then(|record| record.utterance.take())
                    .expect("checked by the previous arm");
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
                    (protocol::Decision::AllowOnce, None) => backend::Decision::AllowOnce,
                    (protocol::Decision::Deny, None) => backend::Decision::Deny,
                    (protocol::Decision::Grant, Some(option)) => {
                        backend::Decision::Grant { option }
                    }
                    (protocol::Decision::Grant, None) => {
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
                    backend::Decision::Grant { option } => Some(option.as_str()),
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
                let record = state.space_mut(&space).expect("checked above");
                match record.model.answer(&question, &field, &text) {
                    Ok(AnswerOutcome::Recorded) => state.touch_space(&space),
                    Ok(AnswerOutcome::Complete(answers)) => {
                        let sent = record.commands.try_send(SpaceCommand::Answer {
                            question: question.clone(),
                            answers,
                            participant: participant.to_owned(),
                        });
                        if sent.is_err() {
                            record.model.restore_question(&question);
                            state.error(participant, "The agent session has stopped.");
                        }
                        state.touch_space(&space);
                    }
                    Err(error) => state.error(participant, error),
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
                if security::pairing::Nonce::parse(&commitment).is_none() {
                    state.error(
                        participant,
                        "A pairing commitment is 64 hexadecimal digits.",
                    );
                    return;
                }
                let failed = |error: &str| ServerMessage::PairFailed {
                    error: error.to_owned(),
                };
                if self.pairing.read().expect("never left poisoned").is_none() {
                    outbox.message(failed("This server does not pair devices."));
                    return;
                }
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
                let server = security::pairing::Nonce::generate();
                let name = state.participants[participant].name.clone();
                log::info!("{name:?} began pairing.");
                deferred.push(Deferred::PairingPrompt(PairingEvent::Began {
                    name: name.clone(),
                }));
                state.pairing = Some(PendingPairing {
                    serial: state.next_pairing,
                    connection,
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
                let device = security::pairing::Nonce::parse(&nonce)
                    .filter(|device| device.commitment() == pairing.commitment);
                let setup = self.pairing.read().expect("never left poisoned").clone();
                let (Some(device), Some(setup)) = (device, setup) else {
                    state.end_pairing(
                        Some(connection),
                        "The pairing could not be checked; try again.",
                    );
                    return;
                };
                let code = security::pairing::code(setup.fingerprint, device, pairing.server);
                let name = pairing.name.clone();
                pairing.code = Some(code.clone());
                outbox.message(ServerMessage::PairWaiting);
                let shown = security::pairing::display(&code);
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
                if self.tokens.read().expect("never left poisoned").is_none() {
                    state.error(participant, "This server does not admit by knocking.");
                    return;
                }
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
                    name: state.participants[participant].name.clone(),
                    started: now,
                };
                log::info!("{:?} knocked on {space}.", knock.name);
                state.knocks.push(knock);
                outbox.message(ServerMessage::KnockState {
                    state: protocol::KnockState::Pending,
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
                    if let Some(record) = state.connections.get(&knock.connection) {
                        record.outbox.message(ServerMessage::KnockState {
                            state: protocol::KnockState::Declined,
                        });
                    }
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
            // The relayed frame is the size of the frame received, which was within the limit.
            let Ok(encoded) = Frame::audio(&AudioFrame { source, ..frame }).try_encode() else {
                return;
            };
            if let Some(space) = state.space(&space) {
                for other in space.model.members() {
                    if *other != participant
                        && let Some(outbox) = state.outbox(other)
                    {
                        outbox.audio(encoded.clone());
                    }
                }
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

    pub(crate) fn transcript_finished(&self, utterance: UtteranceId, text: &str) {
        let mut state = self.lock();
        let Some(record) = state.utterances.remove(&utterance) else {
            return;
        };
        let text = text.trim();
        // The key makes a repeated finalization of the same client utterance idempotent: it is acknowledged with the text queued the first time, and nothing is queued again. It begins with the participant's number, which contains no `/`, so that no two participants' utterances share a key whatever their utterance identifiers contain. The key is the entry's identifier, which every client sees, so it never contains the participant's identity; see `design/security.md`.
        let key = format!("{}/{}", record.source, record.client);
        if let Some(queued) = state
            .space(&record.space)
            .and_then(|space| space.model.queued_text(&key))
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
            Some("Nothing was heard, so nothing was sent.")
        } else if text.len() > limits::QUEUED_TEXT_BYTES {
            Some("The recording was too long, so nothing was sent. Please say it in shorter parts.")
        } else if state
            .participants
            .get(&record.participant)
            .is_none_or(|participant| participant.space.as_deref() != Some(&record.space))
        {
            Some("You left the space before the transcript was ready, so nothing was sent.")
        } else if state
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
        let speaker = state.participants[&record.participant].name.clone();
        if let Some(space) = state.space_mut(&record.space) {
            let queued = space
                .model
                .enqueue(key, record.source, speaker, text.to_owned());
            debug_assert!(queued, "repeats are acknowledged above");
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
        let model = SpaceModel::new(
            id.clone(),
            request.title.clone(),
            request.backend.clone(),
            request.directory.clone(),
            request.environment.clone(),
            session,
            snapshot,
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
            deferred: Cell::new(false),
            closed: None,
            kept_open: false,
            attention: Cell::new(None),
        });
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
        let mut closed = Vec::new();
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
                let (commands, receiver) = async_channel::unbounded();
                match kept.closed {
                    Some(reason) => {
                        model.apply(Event::Closed {
                            reason: Some(reason),
                        });
                        // It closed before the restart; whether anyone saw it is not known.
                        model.attend();
                        closed.push(kept.id.clone());
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
                    }
                }
                state.spaces.push(SpaceRecord {
                    model,
                    commands,
                    touched: Cell::new(None),
                    deferred: Cell::new(false),
                    closed: None,
                    kept_open: false,
                    attention: Cell::new(None),
                });
            }
            for id in closed {
                let order = state.next_closed;
                state.next_closed += 1;
                if let Some(record) = state.space_mut(&id) {
                    record.closed = Some(order);
                }
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
        result: Result<Vec<backend::SessionSummary>, String>,
    ) {
        let message = match result {
            Ok(sessions) => ServerMessage::Sessions {
                sessions: sessions
                    .into_iter()
                    .map(|session| protocol::SessionSummary {
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
        let Some(record) = state.space_mut(space) else {
            return;
        };
        record.model.apply(event);
        // Stamped now rather than when the snapshot is sent, so a level that ends and starts again within one
        // snapshot interval starts its age again.
        record.stamp_attention(Instant::now());
        state.stamp_closed();
        state.pump(space);
        if state.touch_space_paced(space, Instant::now()) {
            self.deferred_snapshot.notify(usize::MAX);
        }
        // The space's attention may have changed.
        self.recount(&mut state);
    }

    /// When the earliest deferred snapshot is due, if any.
    pub fn snapshot_deadline(&self) -> Option<Instant> {
        let state = self.lock();
        state
            .spaces
            .iter()
            .filter(|space| space.deferred.get())
            .filter_map(|space| Some(space.touched.get()? + SNAPSHOT_INTERVAL))
            .min()
    }

    /// Wait for a space to defer a snapshot. Check [`snapshot_deadline`](Self::snapshot_deadline) after creating the listener to avoid a lost wakeup.
    pub fn listen_snapshots(&self) -> EventListener {
        self.deferred_snapshot.listen()
    }

    /// Send the deferred snapshots that are due at `now`.
    pub fn flush_snapshots(&self, now: Instant) {
        let state = self.lock();
        let due: Vec<String> = state
            .spaces
            .iter()
            .filter(|space| {
                space.deferred.get()
                    && space
                        .touched
                        .get()
                        .is_none_or(|touched| touched + SNAPSHOT_INTERVAL <= now)
            })
            .map(|space| space.model.id.clone())
            .collect();
        for space in due {
            state.touch_space(&space);
        }
    }

    pub(crate) fn space_sent(&self, space: &str, key: &str, result: backend::Result<()>) {
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
        result: backend::Result<()>,
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
        result: backend::Result<()>,
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
        result: backend::Result<()>,
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

    /// The driver stopped: the session produces no more events.
    pub(crate) fn space_ended(&self, space: &str, error: Option<String>) {
        let mut state = self.lock();
        let Some(record) = state.space_mut(space) else {
            return;
        };
        if !record.model.is_closed() {
            record.model.apply(Event::Closed { reason: error });
        }
        // Forgotten before the lobby listing that touching the space produces.
        state.forget_closed();
        state.touch_space(space);
        self.recount(&mut state);
        drop(state);
        self.keep();
    }

    /// Announce an agent speech segment to everyone in the space now, in order with its frames.
    #[cfg_attr(not(feature = "speech"), allow(dead_code))]
    pub(crate) fn agent_speech(&self, space: &str, message: ServerMessage) {
        let state = self.lock();
        let Some(record) = state.space(space) else {
            return;
        };
        for member in record.model.members() {
            if let Some(outbox) = state.outbox(member) {
                outbox.agent_message(message.clone());
            }
        }
    }

    /// Send one agent speech frame to everyone in the space now. Someone who joined mid-segment receives from here on.
    #[cfg_attr(not(feature = "speech"), allow(dead_code))]
    pub(crate) fn agent_audio(&self, space: &str, frame: &AudioFrame) {
        let Ok(encoded) = Frame::audio(frame).try_encode() else {
            log::warn!("An agent speech frame for {space} exceeds the frame limit.");
            return;
        };
        let state = self.lock();
        let Some(record) = state.space(space) else {
            return;
        };
        for member in record.model.members() {
            if let Some(outbox) = state.outbox(member) {
                outbox.agent_audio(encoded.clone());
            }
        }
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
                let (_, space) = state.member(connection)?;
                let space = state.space(&space?)?;
                let participants = state.participants(&space.model);
                let mut state = space.model.state(participants);
                limits::fit_state(&mut state);
                Some(ServerMessage::Space {
                    state: Box::new(state),
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

    /// Ask every space driver to close its session, and end every connection.
    pub fn shutdown(&self) {
        self.jobs.close();
        // Spaces still being created are closed as soon as their session starts; see `register_space`. Set first, so that nothing the spaces closing cause is kept: closing their sessions does not close them.
        self.lock().shutting_down = true;
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

    /// Connect and say hello as `participant`.
    fn greet(hub: &Hub, participant: &str) -> ConnectionId {
        let (connection, ..) = hub.connect(Cancellation::new(), Access::open()).unwrap();
        hub.handle(
            connection,
            ClientMessage::Hello {
                protocol: PROTOCOL_VERSION,
                participant: participant.to_owned(),
                name: participant.to_owned(),
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
    fn listing(hub: &Hub, connection: ConnectionId) -> Vec<protocol::SpaceSummary> {
        match hub.render(connection, Outgoing::Spaces) {
            Some(ServerMessage::Spaces { spaces }) => spaces,
            other => panic!("expected a listing, not {other:?}"),
        }
    }

    #[test]
    fn attention_is_listed_and_counted_until_someone_joins() {
        let hub = hub();
        let ada = greet(&hub, "ada");
        let space = create(&hub, "nobody");
        let quiet = listing(&hub, ada);
        assert_eq!((quiet[0].attention, quiet[0].attention_age), (None, None));

        // An approval while nobody is there, then the agent finishing a reply.
        let permission = backend::Permission {
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
                role: backend::Role::Assistant,
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
                role: backend::Role::Assistant,
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
