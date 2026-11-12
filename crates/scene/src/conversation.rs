// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The panel's conversation with a Tomori server: choosing a server, the lobby, then the joined space.
//!
//! This is pure state. It applies [`tomori_client::Event`]s and controller input, and queues [`Action`]s for a [`tomori_client::connection::Connection`] to carry out, as its [`Session`], so nothing here touches the network.
use crate::recording::Naming;
use crate::{
    arrival,
    caption::{COLUMNS, ROWS, wrapped_lines},
    environment::Environment,
    interaction::{SessionActivity, SpeechActivity},
    locomotion::Locomotion,
};
#[cfg(any(not(target_os = "android"), test))]
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tomori_client::{
    ConnectionState, Event, UtteranceFailure,
    connection::{Action, Report, Session, Utterance},
    feedback,
    server_choice::{self, Choice},
};
use tomori_protocol::{
    Activity, BackendInfo, ClientMessage, ConnectionRole, Decision, Engagement, EnvironmentInfo,
    QueueState, Role, ServerMessage, SessionSummary, SpaceState, SpaceSummary, SpeechSetting,
    WorkspaceRoot,
};

mod agent;
mod panel_text;

const BODY_ROWS: usize = ROWS - 4;

/// What the headset states in each `Hello` that it reads and draws at most, as a conversation does unless
/// [`Conversation::set_limits`] says otherwise: frames of up to [`tomori_protocol::MOST_FRAME_BYTES`], so that its code
/// panels are sent whole, but today's panels, [`tomori_protocol::Limits::TODAY`]'s twelve floating and ninety in all,
/// chosen while the frame step laid out every panel's face each frame, about 0.7 ms for a full panel on a desktop
/// machine, so that 90 full panels took longer than a headset's frame. Faces are now kept between frames; more panels
/// wait until what they cost is measured on the headset. Its walls are the Tomori wall, the sixteen an environment may
/// declare and today's six owners add, and its slots six for each, so that an environment declaring sixteen standard
/// walls is drawn whole: a wall costs a board, and a slot no more than a board of its own, a few quads each, built and
/// compared every frame and meshed again as they change or change place in the draw order, beside the panels they
/// hold, which stay as many. It draws sixteen volumes of 1,024 nodes together, until what they cost is measured on the
/// headset. See `design/clients.md` and `design/diagrams.md`.
pub const HEADSET_LIMITS: tomori_protocol::Limits = tomori_protocol::Limits::TODAY
    .with_frame_bytes(tomori_protocol::MOST_FRAME_BYTES as u32)
    .with_projection(
        1 + 16 + 6,
        tomori_protocol::WALL_SLOTS * (1 + 16 + 6),
        tomori_protocol::PROJECTION_FLOATING,
        tomori_protocol::PROJECTION_PANELS,
    )
    .with_volumes(16, 1024);

/// What the desktop window states: frames of up to [`tomori_protocol::MOST_FRAME_BYTES`], twice today's walls owners
/// add, with the Tomori wall and the sixteen an environment may declare, six slots for each, and twice today's floating
/// panels and panels, since a desktop machine lays them out about three times as fast as a headset and a slower frame
/// there is not felt as it is in a headset. See [`HEADSET_LIMITS`] and `design/clients.md`.
pub const DESKTOP_LIMITS: tomori_protocol::Limits = tomori_protocol::Limits::TODAY
    .with_frame_bytes(tomori_protocol::MOST_FRAME_BYTES as u32)
    .with_projection(
        1 + 16 + 12,
        tomori_protocol::WALL_SLOTS * (1 + 16 + 12),
        2 * tomori_protocol::PROJECTION_FLOATING,
        2 * tomori_protocol::PROJECTION_PANELS,
    )
    .with_volumes(16, 1024);

/// The longest instructions an owner gives a space, as the server limits them.
const INSTRUCTIONS_BYTES: usize = 4096;

/// Said when a visitor tries to talk or act in the space they are watching. See `design/security.md`.
const VISITING: &str = "Visitors can watch and listen, but not talk or act here.";

/// What A on a choosable row says while a request waits, which a press is more likely meant for. See
/// `design/panel-actions.md`.
const ANSWER_FIRST: &str = "Answer the request before choosing.";
/// Shown while the platform's request for the microphone is unanswered. See `design/arrival.md`.
pub const MICROPHONE_REQUEST: &str = "Waiting for microphone permission...";

/// How long B must be held to leave a space, so that a press meant for something else never leaves. See `design/interaction.md`.
pub const LEAVE_HOLD: Duration = Duration::from_secs(1);

/// Whose a press of B is, declared in B's order, which their ordering follows: each rung takes it only if none before it
/// wants it, so that B never reaches what is further down, the environment's client side above all, while something of
/// Tomori's own is carried, open or pointed at. The frame step decides the first three, which the conversation does
/// not hold, and [`Conversation::back_or_environment`] the rest, in this order, each rung acting as it takes B; holding
/// B to leave starts with any press but one on a panel carried. A new use of B is a new rung, declared where it
/// belongs: a wall carried, which a quick press removes, and a wall taken hold of, which a press lets go of, are to
/// follow [`Back::Carried`] in that order, before [`Back::Pinned`]. See `design/interaction.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Back {
    /// A projected panel carried with the grip, which a quick press throws away.
    Carried,
    /// A wall carried with the grip, which a quick press removes, for an owner, asking again when it holds panels.
    CarriedWall,
    /// A wall taken hold of but not carried, whose handles show, which it lets go of.
    HeldWall,
    /// The hand menu pinned with Y, which it lets go of.
    Pinned,
    /// The right wrist's transcript, while it shows with something to back out of, such as a turn in focus or a problem.
    Wrist,
    /// The list of the agent's modes, which it closes.
    Modes,
    /// A selected queue entry, which it lets go of.
    Queued,
    /// Waiting for a new name, which it stops.
    Naming,
    /// The card saying no speech was heard, which it takes away.
    Unheard,
    /// This participant's own problem, which it dismisses.
    Problem,
    /// The space's ongoing problem, which it dismisses.
    SpaceProblem,
    /// A projected panel pointed at, which the notice says how to throw away.
    Panel,
    /// The environment's client side, its selectable object pointed at.
    Environment,
    /// Nothing: the notice says that holding B leaves.
    Nothing,
    /// Outside a space, where it dismisses what the lobby shows or goes back.
    Lobby,
}

/// How briefly B must be pressed, and let go, while a panel is carried to throw it away, so that a hold never does. See
/// `design/projection.md`.
pub const DISMISS_PRESS: Duration = Duration::from_millis(400);

/// What B pressed while pointing at a panel, not carrying it, says. See `design/projection.md`.
pub const THROW_HOW: &str = "Grip it and press B to throw away.";

/// How long a queued entry's cancellation can be undone before it is sent.
pub const CANCEL_UNDO: Duration = Duration::from_secs(3);

/// How long A does nothing after an approval or question appears, so that a press meant for what was shown before it, such as undoing a cancellation, never decides it.
pub const REQUEST_GRACE: Duration = Duration::from_millis(600);

/// How long the label on the ray says a row was chosen, after A or the trigger chose it. See `design/panel-actions.md`.
pub const CHOSEN_SHOWN: Duration = Duration::from_millis(800);

/// The most panel rows the problem line and the queue strip each take.
const PROBLEM_ROWS: usize = 2;
/// What names the avatar a space's backend names that this build lacks, on a row of its own, before its identifier.
/// The existing sessions the session ring shows at once, so that their labels stay apart; "More sessions" shows the next
/// ones. See `design/arrival.md`.
pub const SESSION_PAGE: usize = 8;
const QUEUE_ROWS: usize = 2;

const LOST: &str = "Connection lost; your words were not sent.";

/// How long after the remembered server is found the headset waits before connecting to it automatically, so that other servers browsing finds at about the same time are known, and one that needs attention can hold the connection back. See `design/arrival.md`.
pub const AUTOMATIC_SETTLE: Duration = Duration::from_millis(1500);

/// Pairing this device as an owner, from choosing the pairing object until the server confirms or refuses it. See `design/security.md`.
#[derive(Clone, PartialEq)]
enum Pairing {
    /// Committed to this value, waiting for the server's.
    Committed(tomori_security::pairing::Nonce, Instant),
    /// Both sides have the code, written for reading, which waits to be confirmed on the server.
    Waiting(String, Instant),
}

/// Written without the code, which the device shows only on its pairing object, and without the value, which is a secret until revealed.
impl std::fmt::Debug for Pairing {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Committed(_, began) => write!(formatter, "Committed([redacted], {began:?})"),
            Self::Waiting(_, began) => write!(formatter, "Waiting([redacted], {began:?})"),
        }
    }
}

/// The problem shown when words that said a new name did not change it, for `reason`.
fn name_not_changed(naming: Naming, reason: &str) -> String {
    format!("{}{reason}", naming.not_changed())
}

/// What the hand menu says while an owner's request to open pairing, or to close it, is on its way.
const OPENING_PAIRING: &str = "Opening pairing for a new device...";
const CLOSING_PAIRING: &str = "Closing pairing...";

/// How long the device waits for a pairing to be confirmed before it gives up itself: the server's timeout and a margin. A pairing the device gives up on is cancelled, so one choice of the pairing object allows the server one try. See `design/security.md`.
const PAIRING_PATIENCE: Duration = Duration::from_secs(135);

/// Push-to-talk, from pressing X until the server's transcript or failure. See `design/speech.md`.
#[derive(Clone, Debug, PartialEq)]
enum Talk {
    /// X is held and the microphone is open, sending `agent` audio.
    Recording {
        utterance: String,
        started: Instant,
        /// The partial transcript so far.
        heard: String,
        /// Whether the words answer the presented question rather than being queued.
        answering: bool,
        /// What the words name rather than being queued, if anything: the participant or the space.
        naming: Option<Naming>,
        /// The worker or card pointed at as X was pressed, which the utterance began attending to, unless it answers
        /// the presented question. See `design/interaction.md`.
        pointed: Option<tomori_protocol::Resource>,
    },
    /// X was released; waiting for the server's final transcript.
    Sending {
        utterance: String,
        heard: String,
        answering: bool,
        /// What the server last said it is doing with the words, once it has them all; `None` until then, and
        /// throughout from a server that does not say. See `design/speech.md`.
        received: Option<crate::recording::Received>,
        naming: Option<Naming>,
    },
}

/// What the transcript was laid out from, besides the space's revision.
#[derive(Clone, Debug, PartialEq)]
struct TranscriptKey {
    revision: u64,
    wrist: crate::transcript::State,
    role: ConnectionRole,
    source: Option<u32>,
    deciding: bool,
    /// The entry being removed, and the undo time left in sixtieths.
    cancelling: Option<(String, u32)>,
    problem: Option<String>,
    answerable: bool,
    hands_only: bool,
    grace: bool,
    columns: usize,
}

/// What became of the last words sent, which the recording indicator shows for a moment, or until dismissed for a failure. See `design/hand-menu.md`.
#[derive(Clone, Debug, PartialEq)]
enum Outcome {
    /// Transcribed as `text`, and queued, or applied as an answer.
    Sent { answered: bool, text: String },
    /// Taken as the participant's new display name, or the space's new title.
    Named(Naming, String),
    /// Not sent, with why, which is also the problem shown.
    Failed(String),
    /// Not sent, as no speech was heard in it: nothing was lost, so the recording card shows it briefly, and it is no problem. See `design/speech.md`.
    Unheard,
    /// Words that said a new name, or title, did not change it, with why; the problem shown says so, and X says it again until B cancels. See `design/hand-menu.md`.
    NameFailed(Naming, String),
}

impl Talk {
    fn utterance(&self) -> &str {
        match self {
            Talk::Recording { utterance, .. } | Talk::Sending { utterance, .. } => utterance,
        }
    }

    fn answering(&self) -> bool {
        match self {
            Talk::Recording { answering, .. } | Talk::Sending { answering, .. } => *answering,
        }
    }

    fn naming(&self) -> Option<Naming> {
        match self {
            Talk::Recording { naming, .. } | Talk::Sending { naming, .. } => *naming,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Servers,
    Lobby,
    Space,
}

/// The lobby: the spaces and the create object, then the creation rings opened from it, one at a time. See `design/arrival.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LobbyStep {
    /// The spaces, followed by the create object.
    Spaces,
    /// Creating a space: choose a backend.
    Backend,
    /// Creating a space: choose a directory.
    Directory,
    /// Creating a space with a backend that can attach: a new session, or one of the backend's existing sessions in the directory.
    Session,
    /// Creating a space: choose the environment it is bound to. Only when the server offers this client more than one.
    Environment,
}

impl LobbyStep {
    fn ring(self) -> Option<arrival::Step> {
        match self {
            LobbyStep::Spaces => None,
            LobbyStep::Backend => Some(arrival::Step::Backend),
            LobbyStep::Directory => Some(arrival::Step::Directory),
            LobbyStep::Session => Some(arrival::Step::Session),
            LobbyStep::Environment => Some(arrival::Step::Environment),
        }
    }
}

/// How near a participant's head, in metres, the agent pinned there counts as called to them already, so that the hand
/// menu no longer offers to call it: within the distance at which it already serves them unmoved.
pub const CALLED_NEAR: f32 = 3.0;

pub struct Conversation {
    /// This participant's number on the connected server, from its Welcome, which the server records on their queued input. Kept while the connection is down, since a reconnect keeps it; see `design/security.md`.
    source: Option<u32>,
    /// The others' right hands in the space, by their numbers, as their presence last said, for what an environment's
    /// client side draws in their hands. See `design/environments.md`.
    right_hands: std::collections::BTreeMap<u32, tomori_protocol::Pose>,
    /// The others' left hands, likewise.
    left_hands: std::collections::BTreeMap<u32, tomori_protocol::Pose>,
    /// How many times the others' hands have been forgotten, on connecting and on leaving or joining a space, after
    /// which their numbers may be someone else's.
    hands_forgotten: u64,
    /// What this connection may do where it is now, from its Welcome and the role messages that follow. See `design/security.md`.
    role: ConnectionRole,
    /// The fingerprint of the server's certificate on this connection, over TLS, which the pairing code is derived from.
    certificate: Option<tomori_client::Fingerprint>,
    pairing: Option<Pairing>,
    /// Whether the server refused to begin pairing because pairing is closed, so that the pairing object says so until it is chosen again, rather than trying again by itself. See `design/arrival.md`.
    pairing_closed: bool,
    /// Whether pairing is open on the server, for an owner whose server says: `None` until it says, `Some(None)` while closed, and `Some(Some(when))` while open, until about `when`. See `design/security.md`.
    pairing_window: Option<Option<Instant>>,
    /// The knocks waiting on the space, while this participant is in it.
    knocks: Vec<tomori_protocol::KnockRequest>,
    /// What became of this visitor's knock on the space, if they knocked.
    knock: Option<tomori_protocol::KnockState>,
    /// The display name this device suggests to servers.
    name: String,
    /// The display name the connected server knows this participant by, from its Welcome and the names that follow, or `None` from a server that does not rename, which takes the suggested one. See `design/security.md`.
    server_name: Option<String>,
    /// Whether the next press of X says a new display name, as chosen in the hand menu.
    naming: bool,
    /// Whether the next press of X says the space's new title, as an owner chose in the hand menu. See
    /// `design/hand-menu.md`.
    titling: bool,
    mode: Mode,
    servers: Vec<Choice>,
    server: usize,
    /// Whether the stick moved the server selection; until then the best choice, listed first, stays selected as servers appear.
    chosen: bool,
    /// The address of the chosen server while it is missing from the list, as while browsing restarts, so that it is selected again when it reappears.
    missing: Option<String>,
    /// Whether the remembered server may still be connected to when it appears: only once, before the participant moves the selection or connects.
    automatic: bool,
    /// When the remembered server, found, is due to be connected automatically; see [`AUTOMATIC_SETTLE`].
    automatic_at: Option<Instant>,
    /// The server chosen in the server list, while in the lobby or a space.
    connected: Option<Choice>,
    state: Option<ConnectionState>,
    spaces: Vec<SpaceSummary>,
    listed: bool,
    /// The selected space; one past the last is the create object.
    selected: usize,
    joining: Option<String>,
    /// A jump is under way: once the server lists its spaces, join the one that most needs attention. See `design/arrival.md`.
    jumping: bool,
    /// Space updates sent before the server handled our leave are ignored until it confirms.
    leaving: bool,
    space: Option<SpaceState>,
    /// The selected message; one past the last is the queue, when it has entries.
    message: usize,
    page: usize,
    follow: bool,
    /// The approval or question being shown, so paging and the chosen option reset when it changes.
    request: Option<String>,
    /// The chosen approval option: 0 allows once, `n` grants the `n`th standing option.
    option: usize,
    /// The approval a decision was sent for, so another press cannot send a second one.
    decided: Option<String>,
    notice: Option<String>,
    /// Whether the platform's request for the microphone is unanswered, which the server list says, since it shows no other notice. See `design/arrival.md`.
    microphone_waiting: bool,
    /// The space's own instructions to the agent that this owner gave for the next space they create, if any. See `design/spaces.md`.
    next_instructions: Option<String>,
    /// Shown in the lobby after a space you were in or joining has gone, until dismissed with B or another choice.
    ended: Option<String>,
    /// The title of the space this owner asked to delete, and whether its session too, until the server says what became of them. See `design/spaces.md`.
    deleting: Option<(String, bool)>,
    /// Which list the lobby shows: the spaces, or one of the creation rings.
    step: LobbyStep,
    /// The selection within the open creation ring.
    menu: usize,
    /// The server's backends, so the session ring is offered only for those that attach.
    backends: Vec<BackendInfo>,
    /// The directories a space may run in, offered so the client can choose without typing a path.
    roots: Vec<WorkspaceRoot>,
    /// The answer to [`ClientMessage::ListSessions`], most recently active first, shown on the session ring after "New
    /// session", [`SESSION_PAGE`] at a time.
    sessions: Vec<SessionSummary>,
    /// The first of [`Self::sessions`] the session ring shows.
    session_page: usize,
    /// The environments a space may be bound to that the server offers this client, offered on the environment ring when there are several.
    environments: Vec<EnvironmentInfo>,
    /// The environments this build draws; a space bound to another shows the grid, and says so.
    drawable: Vec<Environment>,
    /// The environments this build presents over the grid, without a scenery of their own, such as the GitHub prototype, whose cards the scene presents.
    over_grid: Vec<Environment>,
    /// The locomotion profiles this build declares, by environment; see `design/locomotion.md`.
    locomotion: Vec<(Environment, Locomotion)>,
    /// The client sides this build registers, by environment. See `design/environments.md`.
    client_sides: Vec<(Environment, crate::client_side::Factory)>,
    /// The ambiences this build registers, by environment. See `design/environments.md`.
    ambiences: Vec<(Environment, std::sync::Arc<crate::ambience::Ambience>)>,
    /// The worker placements this build registers, by environment. See `design/environments.md`.
    worker_placements: Vec<(Environment, crate::anchors::Placement)>,
    /// The avatars this build registers, by identifier. See `design/avatars.md`.
    avatars: Vec<(String, crate::avatar::Factory)>,
    /// The avatars whose scenery failed in this build's renderer. See `design/avatars.md`.
    avatar_failures: crate::avatar::Failures,
    /// The backend chosen on the backend ring, kept until the space is created.
    menu_backend: Option<String>,
    /// The directory chosen on the directory ring, kept until the space is created.
    menu_directory: Option<String>,
    /// The space waiting at the environment step for its environment.
    menu_create: Option<Creation>,
    error: Option<String>,
    actions: Vec<Action>,
    /// The wall whose change the server last refused since the frame step last took it, as made against a version since
    /// changed, so that it draws it as the server has it at once rather than as sent.
    refused_wall: Option<String>,
    /// The panel a tick, a choice or a change of size was refused on, as made against a version since changed, so that
    /// the frame step draws it as the server has it at once.
    refused_panel: Option<String>,
    /// The last body paginated, since the console's panel is drawn every frame.
    #[cfg(any(not(target_os = "android"), test))]
    paged: Option<Paged>,
    talk: Option<Talk>,
    /// What the participant points at now, a worker or a card, as the frame loop or console last reported: with the
    /// right hand's ray, and with the left's, and whether the left's moved onto what it is on more recently, so that
    /// words begun name it first. See `design/interaction.md`.
    pointing_at: Option<tomori_protocol::Resource>,
    pointing_left: Option<tomori_protocol::Resource>,
    left_first: bool,
    /// The checklist item the participant points at now, by its panel and the line of the body it begins on, which A
    /// ticks or unticks, as the frame step last reported. See `design/projection.md`.
    pointed_item: Option<(String, u32)>,
    /// Whether the participant points at a teleport pad now, which A chooses, as the frame step last reported, and
    /// whether A chose it since the frame step last asked. See `design/locomotion.md`.
    pointed_pad: bool,
    pad_chosen: bool,
    /// The row of an environment's list this participant last chose, by its panel and the row's id, and when, which the
    /// label on the ray says for [`CHOSEN_SHOWN`]. See `design/panel-actions.md`.
    chosen_row: Option<(String, String, Instant)>,
    /// What each object of an environment's that the participant pointed at is called, by its identity, to name it as
    /// a worker or card is named. See `design/environments.md`.
    object_labels: std::collections::HashMap<String, String>,
    /// The cards of the space, in a space whose environment shows them. See `design/github.md`.
    cards: Option<tomori_protocol::Cards>,
    /// What is projected in the joined space, as the server last sent it whole and patched it since. See
    /// `design/projection.md`.
    projection: crate::patched::Held,
    /// The images of image panels fetched and held, and those asked for. See `design/projection.md`.
    images: crate::images::Images,
    /// Where the participant's head is in the space's scene, as the frame step last said: where a wall is added, and
    /// which wall is nearest.
    head: Option<tomori_protocol::Pose>,
    /// The state and events of the joined space's environment, as its server side sent them, until its client side takes them. See `design/environments.md`.
    channel: crate::channel::Replica,
    /// The space whose cards this connection asked to watch, so that it asks once for each space and connection.
    watching: Option<String>,
    live: bool,
    /// Whether the participant is wearing the device and attending. See `design/spaces.md`.
    engagement: Engagement,
    /// The engagement last reported on this connection, so that each change is reported once.
    reported: Option<Engagement>,
    /// Whether this client reports engagement at all; the spectator does not, so it follows live speech.
    reports_engagement: bool,
    /// Whether agent speech is muted locally.
    muted: bool,
    /// Whether a replay or catch-up was playing at the last frame.
    replaying: bool,
    /// Agent speech playing on this device at the last frame, live or played again, which the swarm speaks with.
    speech: SpeechActivity,
    /// Whether the agent is still speaking here, whether it is the participant's turn, and what they spoke over. See `design/speech.md`.
    turn: crate::turn::Turn,
    /// This device's Feedback setting, which the hand menu and console change.
    feedback: feedback::Settings,
    /// Whether the last [`Self::accept`] refused, saying why, rather than acting or having nothing to do.
    refused: bool,
    /// The problem shown and when it first appeared, so that its age can be shown.
    problem_seen: Option<(String, Instant)>,
    /// The space's ongoing problem, as dismissed on this headset; it shows again if it changes.
    dismissed: Option<String>,
    /// The problem saying that this participant's words were not sent, or their name not changed, which what becomes of their next words replaces. See `design/interaction.md`.
    unsent: Option<String>,
    /// The problem last seen: shown on the right wrist's transcript, or on the recording card for the participant's own words, so that it no longer cues the right wrist. See `design/wrist-transcript.md`.
    problem_noticed: Option<String>,
    /// The open list of the space's modes and speech setting, and the selected item.
    space_menu: Option<usize>,
    /// The queued entry selected with the right stick, by id.
    queue_selected: Option<String>,
    /// A queued entry whose cancellation can still be undone, and when it is sent.
    cancelling: Option<(String, Instant)>,
    /// How long B has been held in a space, while it is, and whether that hold has already left.
    leave_hold: Option<(Duration, bool)>,
    /// When the approval or question shown appeared, until [`REQUEST_GRACE`] has passed.
    request_at: Option<Instant>,
    capture_generation: u64,
    /// The longest utterance the server's recognition takes, from its Welcome, when it has a limit.
    utterance_limit: Option<Duration>,
    /// What became of the last words sent, and when.
    outcome: Option<(Outcome, Instant)>,
    /// What the participant has done on the right wrist's transcript. See `design/wrist-transcript.md`.
    wrist: crate::transcript::State,
    /// The names seen in the space with their handles, for speakers' colours.
    known: crate::speakers::Known,
    /// Counts changes to the joined space, so that the transcript is laid out again only when something it shows changes.
    revision: u64,
    /// The transcript last laid out, and what it was laid out from.
    laid_out: Option<(TranscriptKey, Vec<crate::ui_text::Row>)>,
    /// Whether the status pill shows, a setting of this headset's. See `design/interaction.md`.
    pill: bool,
    /// What each connection states in its `Hello` that this client reads and draws at most: [`HEADSET_LIMITS`] unless
    /// [`Conversation::set_limits`] says otherwise.
    limits: tomori_protocol::Limits,
    /// How much of a long frame has arrived, `received` of `length` bytes, while one is arriving, as the projection whole
    /// on joining a space, which the pill shows. See `design/clients.md`.
    receiving: Option<(usize, usize)>,
    /// Whether the participant travels where an environment allows it, a setting of this headset's. See `design/locomotion.md`.
    travel: bool,
    /// Approvals are decided and queued entries removed on the right wrist, so A elsewhere does neither, as on the headset and desktop window; the console decides with its own commands.
    decide_on_wrist: bool,
    /// Makes utterance identifiers unique across launches, since the server keys queued text by participant and utterance, and the participant persists.
    launch: String,
    utterances: u64,
}

/// A body's text, the rows it was paginated into, and its pages.
#[cfg(any(not(target_os = "android"), test))]
struct Paged {
    text: String,
    rows: usize,
    pages: Arc<[Vec<String>]>,
}

impl Conversation {
    /// `name` is this participant's display name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            source: None,
            right_hands: std::collections::BTreeMap::new(),
            left_hands: std::collections::BTreeMap::new(),
            hands_forgotten: 0,
            role: ConnectionRole::Unknown,
            certificate: None,
            pairing: None,
            pairing_closed: false,
            pairing_window: None,
            knocks: Vec::new(),
            knock: None,

            name: name.into(),
            server_name: None,
            naming: false,
            titling: false,
            mode: Mode::Servers,
            servers: Vec::new(),
            server: 0,
            chosen: false,
            missing: None,
            automatic: true,
            automatic_at: None,
            connected: None,
            state: None,
            spaces: Vec::new(),
            listed: false,
            selected: 0,
            joining: None,
            jumping: false,
            leaving: false,
            space: None,
            message: 0,
            page: 0,
            follow: true,
            request: None,
            option: 0,
            decided: None,
            notice: None,
            microphone_waiting: false,
            next_instructions: None,
            ended: None,
            deleting: None,
            step: LobbyStep::Spaces,
            menu: 0,
            backends: Vec::new(),
            roots: Vec::new(),
            environments: Vec::new(),
            drawable: Vec::new(),
            over_grid: Vec::new(),
            locomotion: Vec::new(),
            client_sides: Vec::new(),
            ambiences: Vec::new(),
            worker_placements: Vec::new(),
            avatars: Vec::new(),
            avatar_failures: crate::avatar::Failures::default(),
            sessions: Vec::new(),
            session_page: 0,
            menu_backend: None,
            menu_directory: None,
            menu_create: None,
            error: None,
            actions: Vec::new(),
            refused_wall: None,
            refused_panel: None,
            #[cfg(any(not(target_os = "android"), test))]
            paged: None,
            talk: None,
            pointing_at: None,
            pointing_left: None,
            left_first: false,
            pointed_item: None,
            pointed_pad: false,
            pad_chosen: false,
            chosen_row: None,
            object_labels: std::collections::HashMap::new(),
            cards: None,
            projection: crate::patched::Held::default(),
            images: crate::images::Images::default(),
            head: None,
            channel: crate::channel::Replica::default(),
            watching: None,
            live: false,
            engagement: Engagement::Engaged,
            reported: None,
            reports_engagement: true,
            muted: false,
            replaying: false,
            speech: SpeechActivity::default(),
            turn: crate::turn::Turn::default(),
            feedback: feedback::Settings::default(),
            refused: false,
            problem_seen: None,
            dismissed: None,
            unsent: None,
            problem_noticed: None,
            space_menu: None,
            queue_selected: None,
            cancelling: None,
            leave_hold: None,
            request_at: None,
            capture_generation: 0,
            utterance_limit: None,
            outcome: None,
            wrist: Default::default(),
            known: Default::default(),
            revision: 0,
            laid_out: None,
            decide_on_wrist: false,
            pill: true,
            limits: HEADSET_LIMITS,
            receiving: None,
            travel: true,
            launch: format!(
                "{:x}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ),
            utterances: 0,
        }
    }

    /// The current panel: the server list, the lobby, or a space.
    #[cfg(any(not(target_os = "android"), test))]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Whether the server list is showing, which is the only time it is worth browsing.
    pub fn is_browsing(&self) -> bool {
        self.mode == Mode::Servers
    }

    /// The server chosen in the server list, while connected or connecting to it.
    pub fn server(&self) -> Option<&Choice> {
        self.connected.as_ref()
    }

    /// Connection generation at which new microphone actions are requested.
    pub fn set_capture_generation(&mut self, generation: u64) {
        self.capture_generation = generation;
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    /// The wall whose change the server refused since this was last asked, as made against a version since changed, so
    /// that the frame step draws it as the server has it at once, rather than as it was sent: the latest, since a
    /// participant changes one wall at a time. See `design/projection.md`.
    pub fn take_refused_wall(&mut self) -> Option<String> {
        self.refused_wall.take()
    }

    /// The panel the server last refused a tick, a choice or a change of size on, as made against a version since
    /// changed, once.
    pub fn take_refused_panel(&mut self) -> Option<String> {
        self.refused_panel.take()
    }

    /// Take one action so a failure can cancel later queued microphone starts.
    pub fn next_action(&mut self) -> Option<Action> {
        if self.actions.is_empty() {
            None
        } else {
            Some(self.actions.remove(0))
        }
    }

    /// Replace the server list, keeping the selection on the same server.
    pub fn set_servers(&mut self, servers: Vec<Choice>) {
        let selected = self.missing.take().or_else(|| {
            self.servers
                .get(self.server)
                .filter(|_| self.chosen)
                .map(|choice| choice.address.clone())
        });
        match selected {
            Some(address) => match servers.iter().position(|choice| choice.address == address) {
                Some(index) => self.server = index,
                None => {
                    self.server = 0;
                    self.missing = Some(address);
                }
            },
            None => self.server = 0,
        }
        self.servers = servers;
    }

    /// Connect to the remembered server once it has been listed for [`AUTOMATIC_SETTLE`] at `now`, unless the participant chose first. If another server then needs attention and it doesn't, give up connecting automatically, so that the participant can see the other on the server row and nothing connects later while they reach for it. Call every frame.
    pub fn tick(&mut self, now: Instant) {
        let remembered = self.servers.iter().any(|choice| {
            choice.source == server_choice::Source::Remembered && choice.is_available()
        });
        if !self.automatic || self.chosen || self.mode != Mode::Servers || !remembered {
            self.automatic_at = None;
            return;
        }
        if now < *self.automatic_at.get_or_insert(now + AUTOMATIC_SETTLE) {
            return;
        }
        match server_choice::automatic(&self.servers).cloned() {
            Some(choice) => self.connect(choice),
            None => {
                log::info!("Not connecting automatically while another server needs attention.");
                self.automatic = false;
                self.automatic_at = None;
            }
        }
    }

    /// Connect to a server without choosing it from the list, as for an address given at startup.
    pub fn connect(&mut self, choice: Choice) {
        log::info!("Connecting to {} at {}", choice.label, choice.address);
        self.automatic = false;
        // The server being connected is the one to select on return, not one that went missing earlier.
        self.missing = None;
        self.actions.push(Action::Connect(choice.clone()));
        self.connected = Some(choice);
        self.state = None;
        self.source = None;
        self.role = ConnectionRole::Unknown;
        self.certificate = None;
        self.pairing = None;
        self.pairing_closed = false;
        self.pairing_window = None;
        self.knocks.clear();
        self.knock = None;
        self.jumping = false;
        // Each server keeps its own name for this participant.
        self.server_name = None;
        self.naming = false;
        self.titling = false;

        self.spaces.clear();
        self.listed = false;
        self.selected = 0;
        self.close_space();
        // A transcript still awaited cannot arrive on the new connection.
        self.talk = None;
        // The menu state belongs to the previous server; it will be re-offered by the new Welcome.
        self.close_menu();
        self.backends.clear();
        self.roots.clear();
        self.environments.clear();
        self.mode = Mode::Lobby;
        self.error = None;
        self.notice = None;
        self.ended = None;
    }

    /// Join `space` by id, as the lobby's accept does, for a client that takes the space as an argument rather than choosing it in the list.
    #[cfg(not(target_os = "android"))]
    pub fn join_space(&mut self, space: impl Into<String>) {
        let space = space.into();
        self.joining = Some(space.clone());
        self.leaving = false;
        self.error = None;
        self.ended = None;
        self.send(ClientMessage::Join { space });
    }

    /// Show, while `waiting`, that the platform's request for the microphone is unanswered: on the server list, and in the notice unless something else is already noted; and stop showing it once answered. See `design/arrival.md`.
    pub fn microphone_request(&mut self, waiting: bool) {
        self.microphone_waiting = waiting;
        if waiting {
            self.notice.get_or_insert_with(|| MICROPHONE_REQUEST.into());
        } else if self.notice.as_deref() == Some(MICROPHONE_REQUEST) {
            self.notice = None;
        }
    }

    /// Report a problem the connection found, such as a client that could not start.
    /// Tell the participant something about the connection's security, such as that the server's identity changed. See `design/security.md`.
    pub fn security_notice(&mut self, notice: impl Into<String>) {
        let notice = notice.into();
        log::warn!("{notice}");
        self.notice = Some(notice);
    }

    /// What this connection may do where it is now.
    pub fn role(&self) -> ConnectionRole {
        self.role
    }

    /// Whether this connection may create spaces, so that the create object is offered. Only owners create spaces; see `design/security.md`.
    fn creates(&self) -> bool {
        self.role == ConnectionRole::Owner
    }

    pub fn fail(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
    }

    pub fn apply(&mut self, event: Event) {
        match event {
            Event::State(state) => self.connection_changed(state),
            Event::Message(message) => self.receive(message),
            // An image panel's image, already checked against its hash.
            Event::Image { hash, png } => {
                let shown = self.shown_images();
                self.images
                    .arrived(hash, png, &shown.iter().collect::<Vec<_>>());
            }
            // Shown on the pill until all of it has arrived.
            Event::Receiving { received, length } => {
                self.receiving = (received < length).then_some((received, length));
            }
            // Push-to-talk audio for the words being recorded or sent that could not be delivered. Anything else a later client observes is not this conversation's to apply.
            event => {
                if let Event::UtteranceFailed(failed) = event
                    && self.talk.as_ref().map(Talk::utterance) == Some(failed.utterance.as_str())
                {
                    let naming = self.talk.as_ref().and_then(Talk::naming);
                    self.stop_talking();
                    self.talk = None;
                    let error: String = match failed.failure {
                        UtteranceFailure::Disconnected => LOST.into(),
                        UtteranceFailure::Congested => {
                            "Network too slow; your words were not sent.".into()
                        }
                    };
                    self.failed(error, naming);
                }
            }
        }
    }

    fn connection_changed(&mut self, state: ConnectionState) {
        if !matches!(state, ConnectionState::Connected { .. }) {
            // What was arriving will not, on this connection.
            self.receiving = None;
            // What was asked for will not arrive on this connection.
            self.images.disconnected();
            // A pairing belongs to its connection; the server withdraws it. Whether pairing is open is told again on connecting.
            self.pairing = None;
            self.forget_turn();
            self.pairing_window = None;
            self.stop_live();
            let naming = self.talk.as_ref().and_then(Talk::naming);
            match &self.talk {
                Some(Talk::Recording { .. }) => {
                    self.stop_talking();
                    self.failed(LOST.into(), naming);
                }
                // The server may still queue it, but its transcript cannot reach us. A name may still be taken, and says so if it is; until then, X says it again.
                Some(Talk::Sending { .. }) => {
                    self.talk = None;
                    self.resume_naming(naming);
                    self.notice = Some("Connection lost while sending your words.".into());
                }
                None => {}
            }
        }
        if let ConnectionState::Connected {
            server,
            source,
            role,
            certificate,
            backends,
            roots,
            environments,
            utterance_limit,
            name,
        } = &state
        {
            log::info!("Connected to {server} as {role:?}");
            self.server_name = name.clone();
            self.source = Some(*source);
            self.forget_hands();
            self.utterance_limit = *utterance_limit;
            self.role = *role;
            self.certificate = *certificate;
            // In a space a problem, such as words lost as the connection dropped, stays until it is dismissed.
            if self.mode != Mode::Space {
                self.error = None;
            }
            // A new connection starts with nothing reported, and watching nothing.
            self.reported = None;
            self.watching = None;
            self.backends = backends.clone();
            self.roots = roots.clone();
            self.environments = environments.clone();
            match self.mode {
                // A space joined meanwhile is restored by the client, and the server sends its state.
                Mode::Lobby => self.list(),
                // The client rejoins the space; the server may have lost it while away.
                Mode::Space => self.joining = self.space.as_ref().map(|space| space.id.clone()),
                Mode::Servers => {}
            }
        }
        self.state = Some(state);
    }

    fn receive(&mut self, message: ServerMessage) {
        match message {
            // A connection admitted only to pair is not shown spaces, even if a server sends them.
            ServerMessage::Spaces { .. } if self.role == ConnectionRole::Pairing => {}
            ServerMessage::PairNonce { nonce } => self.pair_with(&nonce),
            ServerMessage::PairWaiting => {}
            ServerMessage::Knocks { knocks } => self.knocks = knocks,
            ServerMessage::KnockState { state } => {
                self.notice = Some(
                    match state {
                        tomori_protocol::KnockState::Pending => {
                            "You knocked; waiting for someone to let you in."
                        }
                        tomori_protocol::KnockState::Declined => "Your knock was declined.",
                        tomori_protocol::KnockState::Expired => {
                            "Nobody answered your knock in time."
                        }
                        tomori_protocol::KnockState::Withdrawn
                        | tomori_protocol::KnockState::Unknown => "Your knock was withdrawn.",
                    }
                    .into(),
                );
                self.knock = Some(state);
            }
            // Only an answer to this device's own knock, so that a server cannot hand it a token unasked.
            ServerMessage::Admitted { token }
                if self.knock == Some(tomori_protocol::KnockState::Pending) =>
            {
                self.knock = None;
                // Only over TLS; the connection keeps the token with the certificate it presented.
                if self.certificate.is_some() {
                    self.actions.push(Action::Admitted { token });
                }
                self.notice = Some("You were let in: you can now take part in this space.".into());
            }
            ServerMessage::Admitted { .. } => {
                log::warn!("Ignored an admission this device did not ask for.")
            }
            ServerMessage::Paired { token } => {
                self.pairing = None;
                if self.certificate.is_some() {
                    self.actions.push(Action::Paired { token });
                    self.notice =
                        Some("Paired: this device is now an owner of this server.".into());
                }
            }
            ServerMessage::PairFailed { error } => {
                self.pairing = None;
                // Shown on the pairing object until it is chosen again, which asks again; nothing asks by itself. See `design/arrival.md`.
                self.pairing_closed = error == tomori_protocol::PAIRING_CLOSED;
                self.notice = Some(error);
            }
            ServerMessage::PairingWindow { closes_in } => {
                self.pairing_window = Some(
                    closes_in
                        .map(|seconds| Instant::now() + Duration::from_secs(u64::from(seconds))),
                );
                // What this owner asked for is answered; a change someone else made shows in the menu's row.
                if matches!(
                    self.notice.as_deref(),
                    Some(OPENING_PAIRING | CLOSING_PAIRING)
                ) {
                    self.notice = Some(match closes_in {
                        Some(_) => {
                            "Pairing is open: choose the new device's pairing object now.".into()
                        }
                        None => "Pairing is closed.".into(),
                    });
                }
            }
            ServerMessage::Role { role } => {
                log::info!("Now {role:?}");
                self.role = role;
                // A visitor sends no audio, so the microphone closes as the role changes.
                if !role.participates() {
                    self.stop_live();
                    self.stop_talking();
                    // The server ignores a visitor's engagement, so it is reported again on taking part.
                    self.reported = None;
                }
                // Admitted to act here, the participant's engagement now counts, and the space's cards can be watched.
                self.report();
                self.watch_cards();
            }
            ServerMessage::Spaces { spaces } => {
                let selected = self.spaces.get(self.selected).map(|space| &space.id);
                let urgent = arrival::most_urgent(&spaces).filter(|_| !self.listed);
                self.selected = if self.listed && self.selected == self.spaces.len() {
                    // The create object stays selected, after however many spaces there now are.
                    spaces.len()
                } else if let Some(index) = urgent {
                    // A fresh listing starts on the space that most needs attention, so A goes there.
                    index
                } else {
                    selected
                        .and_then(|id| spaces.iter().position(|space| &space.id == id))
                        .unwrap_or(0)
                };
                self.spaces = spaces;
                self.listed = true;
                if std::mem::take(&mut self.jumping) {
                    self.land();
                }
            }
            ServerMessage::Space { state } => self.update_space(*state),
            ServerMessage::Name { name } => {
                log::info!("Now called {name:?}");
                self.server_name = Some(name);
            }
            // For the joined space's environment only, as it is now: a late one from a space left, or from before the
            // space changed environment, is ignored.
            ServerMessage::EnvironmentState {
                environment,
                version,
                state,
            } => {
                self.receive_environment(environment, version, crate::channel::Update::State(state))
            }
            ServerMessage::EnvironmentEvent {
                environment,
                version,
                event,
            } => {
                self.receive_environment(environment, version, crate::channel::Update::Event(event))
            }
            // Only the joined space's projection is kept, as the cards are.
            ServerMessage::Projection { projection } => {
                if self
                    .space
                    .as_ref()
                    .is_some_and(|space| space.id == projection.space)
                {
                    self.projection.whole(*projection, Instant::now());
                }
            }
            // A change to it, from the revision held; one that does not apply asks for it whole, once. A late one from a
            // space left is ignored. See `design/projection.md`.
            ServerMessage::ProjectionPatch { patch } => {
                if self
                    .space
                    .as_ref()
                    .is_some_and(|space| space.id == patch.space)
                    && let Some(revision) = self.projection.patch(&patch, Instant::now())
                {
                    self.send(ClientMessage::ResyncProjection { revision });
                }
            }
            // Only the joined space's cards are kept; a late update from a space left is ignored.
            ServerMessage::Cards { cards } => {
                if self
                    .space
                    .as_ref()
                    .is_some_and(|space| space.id == cards.space)
                {
                    self.cards = Some(*cards);
                }
            }
            // The answer to a session list, shown on the session ring after "New session".
            ServerMessage::Sessions { sessions } => {
                if self.step == LobbyStep::Session {
                    self.sessions = sessions;
                    self.session_page = 0;
                    self.menu = self
                        .menu
                        .min(self.step_list(LobbyStep::Session).saturating_sub(1));
                } else {
                    self.error = Some("Too late to show those sessions.".into());
                }
            }
            ServerMessage::Left { deleted } => {
                let requested = std::mem::take(&mut self.leaving);
                if self.mode != Mode::Servers {
                    // The lobby says why, for a space deleted while this participant was in it.
                    if let Some(deleted) = &deleted {
                        self.ended = Some(self.deleted(deleted));
                    }
                    self.close_space();
                    self.mode = Mode::Lobby;
                    // Leaving on request, or a deletion, is followed by a listing; otherwise ask for one.
                    if !requested && deleted.is_none() {
                        self.list();
                    }
                }
            }
            // What became of the session this owner asked to delete with its space.
            ServerMessage::SessionDeleted { error } => {
                if let Some((title, true)) = self.deleting.take() {
                    self.ended = Some(match error {
                        None => format!("You deleted {title} and its agent session."),
                        Some(error) => {
                            format!("You deleted {title}, but its agent session was kept: {error}")
                        }
                    });
                }
            }
            // A tick refused, as when the panel changed since the participant saw it, is cued by the connection, and the
            // projection sent again shows the item as it is, so that they look and tick again; it is no problem to show.
            // A choice of a row refused, as of a row that has gone, is cued the same way, and the label says no more
            // that it was chosen.
            ServerMessage::Error {
                error,
                panel: Some(panel),
                wall: None,
                ..
            } => {
                if self
                    .chosen_row
                    .take_if(|(chosen, _, _)| *chosen == panel)
                    .is_some()
                {
                    log::info!("The server refused a choice of a row of {panel}: {error}");
                } else {
                    log::info!("The server refused a change to {panel}: {error}");
                }
                self.refused_panel = Some(panel);
            }
            // So is a change to a wall refused as the wall changed since it was taken hold of, as when another
            // participant changed it meanwhile: the projection sent again shows the wall as it is, at once, to change
            // again.
            ServerMessage::Error {
                error,
                wall: Some(wall),
                ..
            } => {
                log::info!("The server refused a change to {wall}: {error}");
                self.refused_wall = Some(wall);
            }
            ServerMessage::Error { error, space, .. } => {
                log::warn!("Server error: {error}");
                if let Some(gone) = space.filter(|space| Some(space) == self.joining.as_ref()) {
                    // Our join was refused because the space does not exist, as after the server restarts. The client
                    // sends the membership it last recorded again after every reconnect, which would be this join, so
                    // record a leave in its place.
                    let title = self
                        .space
                        .as_ref()
                        .filter(|space| space.id == gone)
                        .map(|space| space.title.clone())
                        .or_else(|| {
                            self.spaces
                                .iter()
                                .find(|summary| summary.id == gone)
                                .map(|summary| summary.title.clone())
                        })
                        .unwrap_or_else(|| "The space".into());
                    self.ended = Some(format!("{title} has ended."));
                    self.spaces.retain(|summary| summary.id != gone);
                    self.selected = self.selected.min(self.spaces.len().saturating_sub(1));
                    self.listed = false;
                    self.close_space();
                    self.mode = Mode::Lobby;
                    self.leaving = true;
                    self.send(ClientMessage::Leave);
                } else {
                    if self.mode == Mode::Lobby {
                        // A join refused without naming the space is not taken as the space being gone, but it is no longer awaited.
                        self.joining = None;
                    }
                    // A deletion refused leaves the space as it was, so no later one is taken for this owner's.
                    if self.mode == Mode::Space {
                        self.deleting = None;
                    }
                    self.error = Some(error);
                }
                // A refused decision can be tried again.
                self.decided = None;
            }
            ServerMessage::Transcript {
                utterance,
                text,
                finished,
            } => {
                let ours = self.talk.as_ref().map(Talk::utterance) == Some(utterance.as_str());
                if ours && finished {
                    // The queue entry now shows the words, and replaces an earlier utterance's failure.
                    let talk = self.talk.take();
                    let outcome = if let Some(naming) = talk.as_ref().and_then(Talk::naming) {
                        // The words were the name the server now uses, or the space's title, which its state shows.
                        if naming == Naming::Name {
                            self.server_name = Some(text.clone());
                        }
                        Outcome::Named(naming, text)
                    } else {
                        let answered = talk.is_some_and(|talk| talk.answering());
                        Outcome::Sent { answered, text }
                    };
                    self.outcome = Some((outcome, Instant::now()));
                    self.replace_unsent();
                } else if ours
                    && let Some(Talk::Recording { heard, .. } | Talk::Sending { heard, .. }) =
                        self.talk.as_mut()
                {
                    *heard = text;
                }
            }
            ServerMessage::TranscriptProgress {
                utterance,
                stage,
                part,
            } => {
                // Only words on their way are being transcribed; an earlier utterance's progress is no longer shown.
                if let Some(Talk::Sending {
                    utterance: ours,
                    received,
                    ..
                }) = self.talk.as_mut()
                    && *ours == utterance
                {
                    *received = Some(crate::recording::Received { stage, part });
                }
            }
            ServerMessage::TranscriptFailed { utterance, error } => {
                log::warn!("Utterance {utterance} failed: {error}");
                let mut naming = None;
                if self.talk.as_ref().map(Talk::utterance) == Some(utterance.as_str()) {
                    let talk = self.talk.take();
                    naming = talk.as_ref().and_then(Talk::naming);
                    // Cancelling would be refused, since the server has already ended it.
                    if matches!(talk, Some(Talk::Recording { .. })) {
                        self.actions.push(Action::AbandonUtterance);
                    }
                }
                if error == tomori_protocol::NO_SPEECH && naming.is_none() {
                    // Nothing said was lost: the card says so for a moment, and the problem line, the pill and the right wrist are not troubled.
                    self.outcome = Some((Outcome::Unheard, Instant::now()));
                } else {
                    // Also shown for an earlier utterance, whose words were not sent either.
                    self.failed(error, naming);
                }
            }
            // Agent speech is heard rather than shown: its audio plays through the speaker.
            ServerMessage::Speech {
                segment,
                text,
                message,
                ..
            } => {
                log::info!("Agent speech {segment}: {text}");
                // Something other than what the participant spoke over is heard again.
                if self.turn.announced(&message) {
                    self.actions.push(Action::SetAgentSkipped(false));
                }
            }
            // A reply held for this participant while they record, and in the pause after they let go of X.
            ServerMessage::SpeechHeld { held } => self.turn.held(held),
            // The others' hands, for what an environment's client side draws in them; the rest of presence is the
            // scene's and the player's.
            ServerMessage::Presence {
                source,
                left,
                right,
                ..
            } => {
                for (hands, pose) in [(&mut self.right_hands, right), (&mut self.left_hands, left)]
                {
                    match pose {
                        Some(pose) if self.space.is_some() => {
                            hands.insert(source, pose);
                        }
                        _ => {
                            hands.remove(&source);
                        }
                    }
                }
            }
            ServerMessage::Welcome { .. }
            | ServerMessage::Ping { .. }
            | ServerMessage::Pong { .. } => {}
        }
    }

    /// Show a space's state. The server sends it after a join, on every change, and unasked when it rejoins this participant after a reconnect.
    fn update_space(&mut self, state: SpaceState) {
        if self.mode == Mode::Servers
            || (self.leaving && self.joining.as_deref() != Some(state.id.as_str()))
        {
            return;
        }
        let entered = self.mode != Mode::Space
            || self.space.as_ref().is_none_or(|space| space.id != state.id);
        // The server put this participant in a space they did not ask for, as their last space when they connect: join it
        // as their own choice, which clears what it needed while nobody was there, now that they see it. A jump moves on
        // from it instead.
        let unasked = entered && self.joining.is_none() && !self.jumping;
        if entered {
            log::info!("Joined space {} ({})", state.title, state.id);
            self.close_space();
            self.mode = Mode::Space;
        } else if self
            .space
            .as_ref()
            .is_some_and(|space| space.title != state.title)
        {
            // Renamed while this participant is in it: say so, and by whom, unless it followed its session. See
            // `design/spaces.md`.
            log::info!("Space {} is now titled {:?}", state.id, state.title);
            self.notice = Some(retitled(&state));
        }
        // A late update from a space being left does not end a join of another.
        if entered || self.joining.as_deref() == Some(state.id.as_str()) {
            self.joining = None;
        }
        self.leaving = false;
        self.known.learn(&state);
        self.revision += 1;
        let request = state
            .approval
            .as_ref()
            .map(|approval| approval.id.clone())
            .or_else(|| state.question.as_ref().map(|question| question.id.clone()));
        let arrived = request.is_some() && request != self.request;
        if request != self.request {
            self.page = 0;
            self.option = 0;
            self.decided = None;
            self.request = request;
        }
        if let Some(approval) = &state.approval {
            // Allow once, each standing option, then deny.
            self.option = self.option.min(approval.options.len() + 1);
        }
        if unasked {
            self.join(state.id.clone());
        }
        let previous = self
            .space
            .as_ref()
            .and_then(|space| space.messages.get(self.message))
            .map(|message| message.id.clone());
        if self.follow {
            self.message = state.messages.len().saturating_sub(1);
        } else if let Some(index) =
            previous.and_then(|id| state.messages.iter().position(|message| message.id == id))
        {
            self.message = index;
        } else {
            self.message = self.message.min(items(&state).saturating_sub(1));
        }
        // What an environment's side sent belongs to the binding it ran for: once the space moves to another environment or version, it is forgotten, so that coming back waits for the new binding's state rather than presenting the old one.
        if self.space.as_ref().is_some_and(|space| {
            (space.environment.as_str(), space.environment_version)
                != (state.environment.as_str(), state.environment_version)
        }) {
            self.channel.clear();
        }
        self.space = Some(state);
        self.watch_cards();
        if arrived {
            self.request_at = Some(Instant::now());
            // The transcript's focus moves to the request's title, so that an A meant for something else there decides nothing.
            self.wrist.focus_card(
                self.space
                    .as_ref()
                    .and_then(|space| space.messages.last())
                    .map(|message| message.id.as_str()),
            );
            // A request takes A: the list closes, a selected entry is let go, and a cancellation still in its undo time is sent now, so that an A meant for them cannot decide it.
            self.space_menu = None;
            self.queue_selected = None;
            if let Some((entry, _)) = self.cancelling.take() {
                self.cancel_now(entry);
            }
        }
        if let Some(selected) = self.space_menu {
            let count = self.menu_items().len();
            self.space_menu = (count > 0).then(|| selected.min(count - 1));
        }
        self.report();
    }

    /// Send the cancellation of `entry` if it is still waiting, or say that it was not cancelled.
    fn cancel_now(&mut self, entry: String) {
        if self.waiting().iter().any(|waiting| waiting.id == entry) {
            self.send(ClientMessage::CancelQueued { entry });
        } else {
            self.notice = Some("Not cancelled: already sent or gone.".into());
        }
    }

    /// Report the engagement to a server that accepts it, once per change on each connection. Sent without clearing a shown error, as it is not something the participant did.
    fn report(&mut self) {
        // A visitor sends no presence, so no engagement either; see `design/security.md`.
        let supported = self.reports_engagement
            && self.role.participates()
            && self.mode == Mode::Space
            && matches!(self.state, Some(ConnectionState::Connected { .. }))
            && self
                .space
                .as_ref()
                .is_some_and(|space| space.speech != SpeechSetting::Unknown);
        if supported && self.reported != Some(self.engagement) {
            self.reported = Some(self.engagement);
            self.actions.push(Action::Send(ClientMessage::Engagement {
                state: self.engagement,
            }));
        }
    }

    /// Never report engagement, so that the server keeps this client at the live edge of speech rather than playing it what it missed, as for the spectator. See `design/spectator.md`.
    #[cfg(not(target_os = "android"))]
    pub fn follow_live_speech_only(&mut self) {
        self.reports_engagement = false;
    }

    /// The participant put the device on or took it off, or something else took or returned its input. Taking it off stops the live microphone and any recording, as losing focus does; see `design/live-audio.md`.
    pub fn set_engagement(&mut self, engagement: Engagement) {
        self.engagement = engagement;
        if !engagement.is_listening() {
            self.stop_live();
            self.stop_talking();
        }
        self.report();
    }

    /// The hand menu's Spoken replies row, or the desktop's V: mute or unmute spoken replies, for this participant only. Other participants still play.
    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        log::info!(
            "Spoken replies {}",
            if self.muted { "muted" } else { "unmuted" }
        );
        self.actions.push(Action::SetAgentMuted(self.muted));
    }

    /// Right stick click: play the latest reply again from its start, for this participant only, or stop what is playing again and follow live speech. Asking to hear it again unmutes spoken replies.
    pub fn replay(&mut self) {
        let Some(space) = self.space.as_ref().filter(|_| self.mode == Mode::Space) else {
            self.notice = Some("Join a space to hear a reply again.".into());
            return;
        };
        if space.speech == SpeechSetting::Unknown {
            self.notice = Some("This server cannot play replies again.".into());
            return;
        }
        if self.replaying {
            self.notice = None;
            self.send(ClientMessage::StopReplay);
            return;
        }
        if self.muted {
            self.toggle_mute();
        }
        // Asking to hear it again hears what was skipped, too.
        if self.turn.resume() {
            self.actions.push(Action::SetAgentSkipped(false));
        }
        self.notice = None;
        self.send(ClientMessage::Replay { message: None });
    }

    /// This device's Feedback setting, as kept on it.
    pub fn set_feedback(&mut self, settings: feedback::Settings) {
        self.feedback = settings;
    }

    /// Change one half of the Feedback setting, sounds or vibration, and keep it.
    pub fn change_feedback(
        &mut self,
        sounds: Option<feedback::Level>,
        vibration: Option<feedback::Level>,
    ) {
        let settings = feedback::Settings {
            sounds: sounds.unwrap_or(self.feedback.sounds),
            vibration: vibration.unwrap_or(self.feedback.vibration),
            ..self.feedback
        };
        self.keep_feedback(settings);
    }

    /// Play every source in both ears alike, or place each where it is, and keep it. See `design/spatial-audio.md`.
    pub fn set_mono(&mut self, mono: bool) {
        self.keep_feedback(feedback::Settings {
            mono,
            ..self.feedback
        });
    }

    /// Play an environment's ambience at `level` on this device, and keep it. See `design/spatial-audio.md`.
    pub fn set_ambience(&mut self, level: feedback::Ambience) {
        self.keep_feedback(feedback::Settings {
            ambience: level,
            ..self.feedback
        });
    }

    fn keep_feedback(&mut self, settings: feedback::Settings) {
        log::info!(
            "Feedback: sounds {}, vibration {}, {}, ambience {}",
            settings.sounds.label(),
            settings.vibration.label(),
            if settings.mono { "mono" } else { "stereo" },
            settings.ambience.label()
        );
        self.feedback = settings;
        self.actions.push(Action::SetFeedback(settings));
    }

    /// Whether a replay or catch-up is playing, from the player, once per frame.
    pub fn set_replaying(&mut self, replaying: bool) {
        self.replaying = replaying;
    }

    /// Agent speech playing on this device, from the player, once per frame: live or played again, and even while muted here, since the others still hear it. The swarm speaks while it plays; see `design/speech.md`.
    pub fn speech(&self) -> SpeechActivity {
        // A reply held for the participant shows the swarm preparing to speak, rather than idle.
        SpeechActivity {
            pending: self.speech.pending || self.turn.waiting(self.turn_clock()),
            ..self.speech
        }
    }

    /// Agent speech played this frame, `playing` or not: heard if it is neither muted, skipped nor silenced while the participant records; and once it has ended, with the agent free, it is the participant's turn.
    fn hear(&mut self, playing: bool) {
        let recording = matches!(self.talk, Some(Talk::Recording { .. }));
        let audible = !self.muted && !self.turn.is_skipped() && !recording;
        self.turn.played(playing, audible);
        let free = self.turn_free();
        self.turn.settle(self.turn_clock(), free);
    }

    /// The frame's time, as the connection's tick last gave it before the frame's speech, or the clock's without one.
    fn turn_clock(&self) -> Instant {
        self.turn.now().unwrap_or_else(Instant::now)
    }

    /// Whether the agent is free for the participant to speak to: one who takes part, in a space, idle, with no request waiting, nothing queued, and none of their own words being recorded or on their way.
    fn turn_free(&self) -> bool {
        self.talk.is_none()
            && self.role.participates()
            && self
                .space
                .as_ref()
                .filter(|_| self.mode == Mode::Space)
                .is_some_and(|space| {
                    space.activity == tomori_protocol::Activity::Idle
                        && space.approval.is_none()
                        && space.question.is_none()
                        && space.queue.is_empty()
                })
    }

    /// The participant starts speaking: if the agent is speaking here, the rest of what it is saying is skipped, faded out as push-to-talk silences playback and left silent after, and a replay or catch-up is stopped, so that they take the turn rather than talk over it. Its words stay in the transcript, and the right stick plays them again. See `design/speech.md`.
    fn speak_over(&mut self) {
        if !self.turn.speak(self.turn_clock()) {
            return;
        }
        log::info!("Spoke over the agent: skipping the rest of what it is saying");
        self.actions.push(Action::SetAgentSkipped(true));
        if self.replaying {
            self.send(ClientMessage::StopReplay);
        }
    }

    /// Forget what the agent was saying and what was heard of it, as on leaving a space or losing the connection; nothing skipped stays skipped.
    fn forget_turn(&mut self) {
        if self.turn.is_skipped() {
            self.actions.push(Action::SetAgentSkipped(false));
        }
        self.turn.reset();
    }

    /// Finish a jump with the first listing: join the space that most needs attention, even one the server already put this participant in, since joining is what says it has been seen. If someone dealt with it meanwhile, stay in the lobby and say so.
    fn land(&mut self) {
        match arrival::most_urgent(&self.spaces) {
            Some(index) => {
                let space = self.spaces[index].id.clone();
                log::info!("Jumping to space {space}");
                self.selected = index;
                self.join(space);
            }
            None if self.mode == Mode::Lobby => {
                let server = self
                    .connected
                    .as_ref()
                    .map_or("the server", |choice| choice.label.as_str());
                self.notice = Some(format!("Nothing on {server} needs you now."));
            }
            None => {}
        }
    }

    /// Ask to join `space`, from the lobby or from another space.
    fn join(&mut self, space: String) {
        self.joining = Some(space.clone());
        self.leaving = false;
        self.error = None;
        self.ended = None;
        self.send(ClientMessage::Join { space });
    }

    /// What the most urgent space needs, said in a few words for the lobby's status, such as "Deploy needs you".
    fn attention_status(&self) -> Option<String> {
        let space = &self.spaces[arrival::most_urgent(&self.spaces)?];
        let title = &space.title;
        // The most urgent space is one that needs attention.
        space.attention.map(|attention| match attention {
            tomori_protocol::Attention::Input => format!("{title} needs you"),
            tomori_protocol::Attention::Problem => format!("{title} has a problem"),
            tomori_protocol::Attention::Reply => format!("{title} has a new reply"),
            tomori_protocol::Attention::Unknown => format!("{title} needs attention"),
        })
    }

    fn close_space(&mut self) {
        self.stop_live();
        self.stop_talking();
        // A new name, or title, is said in a space; leaving it stops waiting for one.
        self.naming = false;
        self.titling = false;
        // Knocks belong to the space.
        self.knocks.clear();
        self.knock = None;
        self.forget_turn();
        self.space = None;
        self.forget_hands();
        self.cards = None;
        self.projection.clear();
        self.channel.clear();
        self.watching = None;
        self.joining = None;
        self.message = 0;
        self.page = 0;
        self.follow = true;
        self.request = None;
        self.option = 0;
        self.decided = None;
        self.notice = None;
        self.dismissed = None;
        self.space_menu = None;
        self.queue_selected = None;
        self.cancelling = None;
        self.wrist = Default::default();
        self.known.clear();
        self.revision += 1;
    }

    fn list(&mut self) {
        // A connection admitted only to pair sees no spaces.
        if self.role == ConnectionRole::Pairing {
            return;
        }
        self.actions.push(Action::Send(ClientMessage::ListSpaces));
    }

    /// The pairing object: its code while pairing, which is compared with the server's. See `design/arrival.md`.
    fn pairing_item(&self) -> arrival::Item {
        let (label, detail, pending) = match &self.pairing {
            Some(Pairing::Waiting(code, _)) => (code.clone(), "Confirm on the server", true),
            Some(Pairing::Committed(..)) => ("Pairing...".to_owned(), "Choose to cancel", true),
            None if self.pairing_closed => (
                "Pairing is closed".to_owned(),
                "Ask an owner to open it",
                false,
            ),
            None => ("Pair".to_owned(), "Become an owner", false),
        };
        arrival::Item {
            key: arrival::Key::Pair,
            kind: arrival::Kind::Pair { pending },
            label,
            detail: Some(detail.into()),
            selected: false,
        }
    }

    /// What the hand menu shows now. See `design/hand-menu.md`.
    pub fn menu_model(&self) -> crate::hand_menu::Model {
        let space = self.space.as_ref().filter(|_| self.mode == Mode::Space);
        crate::hand_menu::Model {
            name: self.display_name().to_owned(),
            renames: self.renames(),
            naming: self.naming,
            titling: self.titling,
            role: self.role,
            in_space: space.is_some(),
            participants: space
                .map(|space| {
                    space
                        .participants
                        .iter()
                        .map(|participant| participant.name.clone())
                        .collect()
                })
                .unwrap_or_default(),
            visitors: space.map_or(0, |space| space.visitors),
            knocks: self.knocks().to_vec(),
            can_knock: self.can_knock(),
            knock: self.knock_state(),
            pairing: self.pairable().then(|| self.pairing_item().label),
            pairing_window: self.pairing_window_left(Instant::now()),
            notice: self.notice.clone(),
            instructions: space.and_then(|space| space.instructions.clone()),
            space: space.map(|space| crate::hand_menu::SpaceInfo {
                title: space.title.clone(),
                running: format!(
                    "{} in {}",
                    space.backend,
                    space
                        .directory
                        .trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .unwrap_or_default()
                ),
                // The avatar this build lacks, as the swarm is shown instead, on a row of its own: a diagnostic for
                // owners alone, since everyone else sees the default swarm and can do nothing about it.
                missing_avatar: self
                    .missing_avatar(space)
                    .filter(|_| self.role == ConnectionRole::Owner)
                    .map(str::to_owned),
                activity: activity_label(space.activity).into(),
                modes: space
                    .modes
                    .iter()
                    .map(|mode| {
                        (
                            mode.id.clone(),
                            mode.name.clone(),
                            mode.description.clone(),
                            Some(&mode.id) == space.mode.as_ref(),
                        )
                    })
                    .collect(),
                mode: space
                    .current_mode()
                    .map(|mode| mode.name.clone())
                    .or_else(|| space.mode.clone().filter(|_| !space.modes.is_empty())),
                speech: space.speech,
                environment: (!space.environment.is_empty()).then(|| self.shown_environment(space)),
                environments: self.changeable_environments(space),
                here: here(space),
                walls: self
                    .manages_walls()
                    .then(|| self.nearest_added_wall().is_some()),
                deletion: (self.role == ConnectionRole::Owner && space.deletes)
                    .then_some(space.deletes_session),
                renamed_by: space.renamed_by.as_ref().map(renamer),
                retitles: self.retitles(),
            }),
            people: space
                .map(|space| {
                    space
                        .participants
                        .iter()
                        .map(|participant| {
                            let colour = crate::speakers::colour(participant.source);
                            crate::hand_menu::Person {
                                name: participant.name.clone(),
                                colour: [colour[0], colour[1], colour[2]],
                                you: Some(participant.source) == self.source,
                                away: participant.engagement == Some(Engagement::Away),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
            muted: self.muted,
            pill: self.pill,
            travel: self.declared_locomotion().travels().then_some(self.travel),
            agent_away: self.sends_agent_home()
                && self
                    .joined_space()
                    .is_some_and(|(space, _)| space.summoned.is_some() || space.pinned),
            calls: self.offers_call(),
            pins: self.offers_pin(),
            walks: self.offers_walk(),
            movement: self.agent_movement(),
            pointed: None,
            feedback: self.feedback,
        }
    }

    /// The display name this participant goes by: the one the server knows them by, or else the one this device suggests.
    pub fn display_name(&self) -> &str {
        self.server_name.as_deref().unwrap_or(&self.name)
    }

    /// Whether this participant can say a new name here: the server renames, and they take part in the space they are in, since the words are recorded as push-to-talk. See `design/security.md`.
    fn renames(&self) -> bool {
        self.server_name.is_some()
            && self.mode == Mode::Space
            && self.space.is_some()
            && self.role.participates()
    }

    /// Whether this participant can rename the space they are in: they are an owner, on a server whose space state
    /// says it renames spaces. See `design/hand-menu.md`.
    fn retitles(&self) -> bool {
        self.role == ConnectionRole::Owner
            && self.mode == Mode::Space
            && self.space.as_ref().is_some_and(|space| space.renames)
    }

    /// Give the joined space `title`, as typed on the console, or let it follow its agent session's title again when
    /// `title` is empty. Only owners can, on a server that renames spaces. Says why not in the notice.
    pub fn retitle(&mut self, title: &str) {
        if !self.retitles() {
            self.notice = Some(if self.space.is_none() || self.mode != Mode::Space {
                "Join a space to rename it.".into()
            } else if self.role != ConnectionRole::Owner {
                "Only owners can rename a space.".into()
            } else {
                "This server cannot rename spaces.".into()
            });
            return;
        }
        let title = title.trim();
        self.notice = Some(if title.is_empty() {
            "Following the agent session's title again.".into()
        } else {
            format!("Renaming this space to “{title}”.")
        });
        self.send(ClientMessage::RenameSpace {
            title: title.to_owned(),
        });
    }

    /// Change this participant's display name to `name`, as typed on the console, on a server that renames. The server keeps it with their token, or for a visitor while it keeps them. Says what happened in the notice.
    pub fn rename(&mut self, name: &str) {
        if self.server_name.is_none()
            || !matches!(self.state, Some(ConnectionState::Connected { .. }))
        {
            self.notice = Some("This server does not change names.".into());
            return;
        }
        // Cleaned and checked as the server does, so that what is refused says why at once; the server checks again, and whether someone else has the name.
        match tomori_protocol::text::display_name(name) {
            Ok(name) => {
                self.notice = Some(format!("Changing your name to {name}."));
                self.send(ClientMessage::SetName { name });
            }
            Err(error) => self.notice = Some(format!("Name not changed: {error}")),
        }
    }

    /// The name the server gives the environment `id`, or the identifier itself when it offers none by that name.
    /// What the lobby says of `space` beneath its title, a line each: its project folder and environment; what it
    /// needs, or else what its agent is doing, how many are here and how long ago it was last active; and the start of
    /// its newest reply, quoted. What an older server does not list is left out, its folder then being the directory's
    /// last part. See `design/arrival.md`.
    fn space_detail(&self, space: &SpaceSummary) -> String {
        let folder = space.folder.clone().unwrap_or_else(|| {
            space
                .directory
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_owned()
        });
        let place = [folder, self.environment_name(&space.environment)]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        let status = space
            .attention
            .map_or(activity_label(space.activity), arrival::attention_label);
        let mut now = vec![status.to_owned()];
        if space.participants > 0 {
            now.push(format!("{} here", space.participants));
        }
        now.extend(space.active_age.map(ago));
        let mut lines = vec![place, now.join(" · ")];
        lines.extend(space.latest.as_ref().map(|latest| format!("“{latest}”")));
        lines.retain(|line| !line.is_empty());
        lines.join("\n")
    }

    fn environment_name(&self, id: &str) -> String {
        self.environments
            .iter()
            .find(|offered| offered.id == id)
            .map_or_else(|| id.to_owned(), |offered| offered.label.clone())
    }

    /// The environments this participant can change `space` to, as the hand menu offers them: id, name and whether it is the current one, starting with the current one when the server no longer offers it. Only owners can, and only on a server that changes environments. See `design/environments.md`.
    fn changeable_environments(&self, space: &SpaceState) -> Vec<(String, String, bool)> {
        if self.role != ConnectionRole::Owner || !space.environment_changes {
            return Vec::new();
        }
        let mut offered: Vec<_> = self
            .environments
            .iter()
            .map(|offered| {
                let current = offered.id == space.environment;
                let name = if current {
                    self.shown_environment(space)
                } else {
                    offered.label.clone()
                };
                (offered.id.clone(), name, current)
            })
            .collect();
        if !offered.iter().any(|(_, _, current)| *current) {
            offered.insert(
                0,
                (
                    space.environment.clone(),
                    self.shown_environment(space),
                    true,
                ),
            );
        }
        offered
    }

    /// Change the joined space's environment to `id`, for everyone in it. Only owners can, and only on a server that changes environments; the server refuses one it does not offer. Says what happened in the notice. See `design/environments.md`.
    pub fn change_environment(&mut self, id: &str) {
        if self.role != ConnectionRole::Owner {
            self.notice = Some("Only owners can change the space's environment.".into());
            return;
        }
        let in_space = self.mode == Mode::Space;
        let Some(space) = self.space.as_ref().filter(|_| in_space) else {
            self.notice = Some("Join a space first.".into());
            return;
        };
        if !space.environment_changes {
            self.notice = Some("This server does not change a space's environment.".into());
            return;
        }
        let name = self.environment_name(id);
        // Choosing the environment it is in, as the menu lists it even when this client cannot present it, changes nothing, and the server would refuse it as one not offered to this client.
        if space.environment == id {
            self.notice = Some(format!("The space is already in {name}."));
            return;
        }
        self.send(ClientMessage::SetEnvironment {
            environment: id.to_owned(),
        });
        self.notice = Some(format!("Changing the environment to {name} for everyone."));
    }

    /// Delete the joined space for everyone in it, and its agent session too if `session`, as an owner, once the hand menu has asked. Only on a server that deletes spaces, and the session only where it can be; the server returns everyone to the lobby, saying who deleted it. See `design/spaces.md`.
    /// Returns whether it asked; if not, the notice says why.
    pub fn delete_space(&mut self, session: bool) -> bool {
        if self.role != ConnectionRole::Owner {
            self.notice = Some("Only owners can delete a space.".into());
            return false;
        }
        let in_space = self.mode == Mode::Space;
        let Some(space) = self.space.as_ref().filter(|_| in_space) else {
            self.notice = Some("Join a space first.".into());
            return false;
        };
        if !space.deletes || (session && !space.deletes_session) {
            self.notice = Some(if space.deletes {
                "This space's agent session cannot be deleted from here.".into()
            } else {
                "This server does not delete spaces.".into()
            });
            return false;
        }
        let (id, title) = (space.id.clone(), space.title.clone());
        // The lobby says what became of it, once the server has deleted it.
        self.deleting = Some((title, session));
        self.send(ClientMessage::DeleteSpace { space: id, session })
    }

    /// What the lobby says of the joined space, deleted as `deleted` says: by this owner, as they asked, or by someone else.
    fn deleted(&mut self, deleted: &tomori_protocol::SpaceDeletion) -> String {
        let title = self
            .space
            .as_ref()
            .map_or_else(|| "The space".to_owned(), |space| space.title.clone());
        match self.deleting.take() {
            // The session's fate follows.
            Some((asked, true)) if deleted.session => {
                self.deleting = Some((asked, true));
                format!("You deleted {title}; deleting its agent session.")
            }
            Some(_) => format!("You deleted {title}."),
            None if deleted.session => {
                format!(
                    "{title} was deleted by {}, with its agent session.",
                    deleted.by
                )
            }
            None => format!("{title} was deleted by {}.", deleted.by),
        }
    }

    /// Act on a choice made in the hand menu.
    pub fn choose_menu(&mut self, choice: crate::hand_menu::Choice) {
        match choice {
            crate::hand_menu::Choice::Knock => self.knock(),
            crate::hand_menu::Choice::Admit(id) => self.decide_knock(&id, true),
            crate::hand_menu::Choice::Decline(id) => self.decide_knock(&id, false),
            crate::hand_menu::Choice::Pair => self.pair(),
            crate::hand_menu::Choice::PairingWindow => self.toggle_pairing_window(),
            crate::hand_menu::Choice::ClearInstructions => self.instruct(""),
            crate::hand_menu::Choice::Mode(mode) => {
                self.send(ClientMessage::SetMode { mode });
            }
            crate::hand_menu::Choice::Speech => {
                if let Some(space) = self.space.as_ref() {
                    let speech = match space.speech {
                        SpeechSetting::Brief => SpeechSetting::Full,
                        _ => SpeechSetting::Brief,
                    };
                    self.send(ClientMessage::SetSpeech { speech });
                }
            }
            crate::hand_menu::Choice::Environment(id) => self.change_environment(&id),
            crate::hand_menu::Choice::Mute => self.toggle_mute(),
            crate::hand_menu::Choice::Pill => self.toggle_pill(),
            crate::hand_menu::Choice::Travel => self.toggle_travel(),
            crate::hand_menu::Choice::AgentHome => self.send_agent_home(),
            crate::hand_menu::Choice::CallAgent => self.call_agent(),
            crate::hand_menu::Choice::AgentStay => self.pin_agent(true),
            crate::hand_menu::Choice::AgentFollow => self.pin_agent(false),
            crate::hand_menu::Choice::AgentWalk => self.walk_agent(true),
            crate::hand_menu::Choice::AgentWalkEnd => self.walk_agent(false),
            crate::hand_menu::Choice::AgentMovement => {
                if let Some((movement, _)) = self.agent_movement() {
                    self.set_agent_movement(movement.next_mode());
                }
            }
            crate::hand_menu::Choice::AgentReturn => {
                if let Some((movement, _)) = self.agent_movement() {
                    self.set_agent_movement(movement.next_return());
                }
            }
            crate::hand_menu::Choice::AddWall => self.add_wall(),
            crate::hand_menu::Choice::RemoveWall => self.remove_wall(),
            crate::hand_menu::Choice::Leave if self.mode == Mode::Space => self.leave(),
            crate::hand_menu::Choice::Delete { session } => {
                self.delete_space(session);
            }
            crate::hand_menu::Choice::Leave => {}
            crate::hand_menu::Choice::Sounds => {
                self.change_feedback(Some(self.feedback.sounds.next()), None)
            }
            crate::hand_menu::Choice::Vibration => {
                self.change_feedback(None, Some(self.feedback.vibration.next()))
            }
            crate::hand_menu::Choice::Mono => self.set_mono(!self.feedback.mono),
            crate::hand_menu::Choice::Ambience => self.set_ambience(self.feedback.ambience.next()),
            // The next press of X says the new name; choosing again changes nothing after all.
            crate::hand_menu::Choice::Rename => {
                self.naming = !self.naming && self.renames();
                self.titling &= !self.naming;
            }
            // The next press of X says the space's new title, as for a name.
            crate::hand_menu::Choice::RenameSpace => {
                self.titling = !self.titling && self.retitles();
                self.naming &= !self.titling;
            }
            crate::hand_menu::Choice::FollowSession => self.retitle(""),
        }
    }

    /// Give the joined space's agent `text` as the space's own instructions, or, outside a space, the agent of the next space this owner creates; empty text removes them. Only owners can, and only where the server shows a space's instructions. Says what happened in the notice. See `design/spaces.md`.
    pub fn instruct(&mut self, text: &str) {
        let text = text.trim();
        if self.role != ConnectionRole::Owner {
            self.notice = Some("Only owners can give a space instructions.".into());
            return;
        }
        // The server's limit, checked here so that a space is not refused for them.
        if text.len() > INSTRUCTIONS_BYTES {
            self.notice = Some(format!(
                "Instructions can be at most {INSTRUCTIONS_BYTES} bytes."
            ));
            return;
        }
        let in_space = self.mode == Mode::Space;
        match self.space.as_ref().filter(|_| in_space) {
            Some(space) if space.instructions.is_none() => {
                self.notice = Some("This agent takes no instructions from Tomori.".into());
            }
            Some(_) => {
                // An owner takes part, so this is always sent.
                self.send(ClientMessage::SetInstructions {
                    text: text.to_owned(),
                });
                self.notice = Some(if text.is_empty() {
                    "Removing this space's instructions.".into()
                } else {
                    "Giving the agent this space's instructions.".into()
                });
            }
            None => {
                self.next_instructions = (!text.is_empty()).then(|| text.to_owned());
                self.notice = Some(if text.is_empty() {
                    "The next space you create has no instructions of its own.".into()
                } else {
                    "The next space you create is given these instructions, if its agent takes them."
                        .into()
                });
            }
        }
    }

    /// The knocks waiting on the space, for a participant to admit or decline.
    pub fn knocks(&self) -> &[tomori_protocol::KnockRequest] {
        if self.role.participates() {
            &self.knocks
        } else {
            &[]
        }
    }

    /// What became of this visitor's knock, if they knocked.
    pub fn knock_state(&self) -> Option<tomori_protocol::KnockState> {
        self.knock
    }

    /// Whether this visitor can knock on the space they are watching: watching one, and not already waiting.
    pub fn can_knock(&self) -> bool {
        self.mode == Mode::Space
            && self.space.is_some()
            && self.role == ConnectionRole::Visitor
            && self.knock != Some(tomori_protocol::KnockState::Pending)
    }

    /// Knock on the space this visitor is watching, to ask to take part. See `design/security.md`.
    pub fn knock(&mut self) {
        if self.can_knock() {
            self.actions.push(Action::Send(ClientMessage::Knock));
        }
    }

    /// Admit or decline the knock `id`, as a participant.
    pub fn decide_knock(&mut self, id: &str, admit: bool) {
        self.send(ClientMessage::DecideKnock {
            knock: id.to_owned(),
            admit,
        });
    }

    /// Whether this device can pair with the server: connected over TLS, and not already an owner. See `design/security.md`.
    pub fn pairable(&self) -> bool {
        self.certificate.is_some()
            && self.role != ConnectionRole::Owner
            && matches!(self.state, Some(ConnectionState::Connected { .. }))
    }

    /// Whether pairing is open on the server, as the hand menu offers it to an owner whose server says: `Some(None)` while closed, and `Some(Some(seconds))` left while open at `now`. `None` for anyone else, and from a server that does not say. See `design/hand-menu.md`.
    pub fn pairing_window_left(&self, now: Instant) -> Option<Option<u32>> {
        let window = self
            .pairing_window
            .filter(|_| self.role == ConnectionRole::Owner)?;
        Some(window.map(|until| {
            let left = until.saturating_duration_since(now);
            u32::try_from(left.as_secs() + u64::from(left.subsec_nanos() > 0)).unwrap_or(u32::MAX)
        }))
    }

    /// Open pairing on the server so that a new device can ask to pair, or close it while it is open, as an owner whose server says whether it is open. See `design/security.md`.
    pub fn toggle_pairing_window(&mut self) {
        match self.pairing_window_left(Instant::now()) {
            None if self.role == ConnectionRole::Owner => {
                self.notice = Some("This server does not open pairing for new devices.".into());
            }
            None => self.notice = Some("Only owners can open pairing.".into()),
            Some(None) => {
                self.send(ClientMessage::OpenPairing);
                self.notice = Some(OPENING_PAIRING.into());
            }
            Some(Some(_)) => {
                self.send(ClientMessage::ClosePairing);
                self.notice = Some(CLOSING_PAIRING.into());
            }
        }
    }

    /// Start pairing, committing to a random value, or cancel the pairing in progress.
    pub fn pair(&mut self) {
        if self.pairing.take().is_some() {
            self.actions.push(Action::Send(ClientMessage::PairCancel));
            self.notice = None;
            return;
        }
        if !self.pairable() {
            return;
        }
        self.pairing_closed = false;
        let device = tomori_security::pairing::Nonce::generate();
        self.actions.push(Action::Send(ClientMessage::PairBegin {
            commitment: device.commitment(),
        }));
        self.pairing = Some(Pairing::Committed(device, Instant::now()));
        // In a space a problem stays until it is dismissed or replaced by a newer one, so that it is not wiped before it is read; elsewhere the next attempt replaces it.
        if self.mode != Mode::Space {
            self.error = None;
        }
        self.notice = Some("Pairing...".into());
    }

    /// The server sent its value: reveal ours, and derive the code from both and the certificate this connection presented.
    fn pair_with(&mut self, nonce: &str) {
        let (Some(Pairing::Committed(device, began)), Some(certificate), Some(server)) = (
            self.pairing.clone(),
            self.certificate,
            tomori_security::pairing::Nonce::parse(nonce),
        ) else {
            return;
        };
        self.actions.push(Action::Send(ClientMessage::PairReveal {
            nonce: device.to_hex(),
        }));
        let code = tomori_security::pairing::display(&tomori_security::pairing::code(
            certificate,
            device,
            server,
        ));
        self.notice = Some(format!("Pairing code {code}: confirm it on the server."));
        self.pairing = Some(Pairing::Waiting(code, began));
    }

    /// Give up on a pairing that has waited longer than the device waits for one, the server's timeout and a margin, at `now`, cancelling it on the server. Called every frame.
    pub fn expire_pairing(&mut self, now: Instant) {
        let began = match &self.pairing {
            Some(Pairing::Committed(_, began) | Pairing::Waiting(_, began)) => *began,
            None => return,
        };
        if now.saturating_duration_since(began) >= PAIRING_PATIENCE {
            self.pairing = None;
            self.actions.push(Action::Send(ClientMessage::PairCancel));
            self.notice = Some("The pairing was not confirmed in time.".into());
        }
    }

    /// Send `message`, unless it acts in the space and this connection only watches. Returns whether it was sent.
    fn send(&mut self, message: ClientMessage) -> bool {
        // A visitor only watches: acting in the space, such as deciding an approval, is not sent. See `design/security.md`.
        let acts = matches!(
            message,
            ClientMessage::CancelQueued { .. }
                | ClientMessage::Decide { .. }
                | ClientMessage::Answer { .. }
                | ClientMessage::SetAttribution { .. }
                | ClientMessage::SetMode { .. }
                | ClientMessage::DecideKnock { .. }
                | ClientMessage::SetSpeech { .. }
                | ClientMessage::SetInstructions { .. }
                | ClientMessage::SetEnvironment { .. }
                | ClientMessage::Replay { .. }
                | ClientMessage::StopReplay
                | ClientMessage::AddWall { .. }
                | ClientMessage::RemoveWall { .. }
                | ClientMessage::SetWall { .. }
                | ClientMessage::AttachWall { .. }
                | ClientMessage::LockWall { .. }
                | ClientMessage::PlacePanel { .. }
                | ClientMessage::DismissPanel { .. }
                | ClientMessage::CheckPanelItem { .. }
                | ClientMessage::ChoosePanelRow { .. }
        );
        if acts && !self.role.participates() {
            self.notice = Some(VISITING.into());
            return false;
        }
        // In a space a problem stays until it is dismissed or replaced by a newer one, so that it is not wiped before it is read; elsewhere the next attempt replaces it.
        if self.mode != Mode::Space {
            self.error = None;
        }
        self.actions.push(Action::Send(message));
        true
    }

    /// The number of entries in the list shown at `step`, so the stick stays on one of them.
    fn step_list(&self, step: LobbyStep) -> usize {
        match step {
            // The spaces, then the create object.
            LobbyStep::Spaces => self.spaces.len() + usize::from(self.creates()),
            LobbyStep::Backend => self.backends.len(),
            LobbyStep::Directory => self.roots.len(),
            // "New session", then a page of the existing sessions, and "More sessions" when they do not fit on one.
            LobbyStep::Session => {
                1 + self.paged_sessions().len() + usize::from(self.sessions.len() > SESSION_PAGE)
            }
            LobbyStep::Environment => self.environments.len(),
        }
    }

    /// The existing sessions the session ring shows now.
    fn paged_sessions(&self) -> &[SessionSummary] {
        let start = self.session_page.min(self.sessions.len());
        &self.sessions[start..(start + SESSION_PAGE).min(self.sessions.len())]
    }

    /// Show the next page of existing sessions on the session ring, after the last returning to the first, with the
    /// selection kept on "More sessions".
    fn next_session_page(&mut self) {
        self.session_page += SESSION_PAGE;
        if self.session_page >= self.sessions.len() {
            self.session_page = 0;
        }
        self.menu = self.step_list(LobbyStep::Session) - 1;
    }

    /// Whether the chosen backend can attach to an existing session, so that the session ring is offered.
    fn attaches(&self) -> bool {
        self.backends
            .iter()
            .any(|backend| Some(&backend.name) == self.menu_backend.as_ref() && backend.attach)
    }

    /// Close the creation rings, back to the spaces with the create object selected, forgetting their choices.
    fn close_menu(&mut self) {
        if self.step != LobbyStep::Spaces {
            self.selected = self.spaces.len();
        }
        self.step = LobbyStep::Spaces;
        self.menu = 0;
        self.sessions.clear();
        self.session_page = 0;
        self.menu_backend = None;
        self.menu_directory = None;
        self.menu_create = None;
    }

    /// Send `create`, bound to the only environment the server offers, or ask which one when it offers several. A server that offers none predates environments, and binds its own.
    fn submit(&mut self, create: Creation) {
        self.joining = None;
        if self.environments.len() > 1 {
            self.menu = 0;
            self.step = LobbyStep::Environment;
            self.menu_create = Some(create);
            return;
        }
        let only = self.environments.first().map(|offered| offered.id.clone());
        self.send(create.message(only));
        self.close_menu();
    }

    /// The environments this build has sceneries for, as [`Self::set_drawable`] gave them.
    #[cfg(any(test, feature = "testing"))]
    pub fn drawable(&self) -> &[Environment] {
        &self.drawable
    }

    /// Draw `drawable`, the environments this build has sceneries for, beside the built-in grid, which is all a conversation draws until it is given them.
    pub fn set_drawable(&mut self, drawable: Vec<Environment>) {
        self.drawable = drawable;
    }

    /// Present `over_grid` over the grid, without sceneries of their own, as the GitHub prototype's cards are: listed in `Hello` and shown without a notice. See `tomori-render`'s `Sceneries::present_over_grid`.
    pub fn set_over_grid(&mut self, over_grid: Vec<Environment>) {
        self.over_grid = over_grid;
    }

    /// Present each environment `client_sides` names with its client side, in a space bound to it whose scenery this
    /// build draws. See `design/environments.md`.
    pub fn set_client_sides(
        &mut self,
        client_sides: Vec<(Environment, crate::client_side::Factory)>,
    ) {
        self.client_sides = client_sides;
    }

    /// The client side of the joined space's environment, as this build draws it, if the build registers one: the
    /// environment, at the version drawn, and what makes its client side.
    pub fn client_side(&self) -> Option<(Environment, crate::client_side::Factory)> {
        let environment = self.environment();
        self.joined_space()?;
        self.client_sides
            .iter()
            .find(|(registered, _)| registered.id() == environment.id())
            .map(|(_, factory)| (environment, factory.clone()))
    }

    /// Sound each environment `ambiences` names with its ambience, in a space bound to it whose scenery this build
    /// draws. See `design/environments.md`.
    pub fn set_ambiences(
        &mut self,
        ambiences: Vec<(Environment, std::sync::Arc<crate::ambience::Ambience>)>,
    ) {
        self.ambiences = ambiences;
    }

    /// The ambience of the joined space's environment, as this build draws it, if the build registers one, in the
    /// environment's own coordinates.
    pub fn ambience(&self) -> Option<(Environment, std::sync::Arc<crate::ambience::Ambience>)> {
        let environment = self.environment();
        self.joined_space()?;
        self.ambiences
            .iter()
            .find(|(registered, _)| registered.id() == environment.id())
            .map(|(_, ambience)| (environment, ambience.clone()))
    }

    /// Place the workers of each environment `placements` names by its placement, in a space bound to it whose scenery
    /// this build draws. See `design/environments.md`.
    pub fn set_worker_placements(
        &mut self,
        placements: Vec<(Environment, crate::anchors::Placement)>,
    ) {
        self.worker_placements = placements;
    }

    /// The worker placements this build registers, as [`Self::set_worker_placements`] gave them.
    #[cfg(any(test, feature = "testing"))]
    pub fn worker_placements(&self) -> &[(Environment, crate::anchors::Placement)] {
        &self.worker_placements
    }

    /// Let the participant travel as `locomotion` declares, for each environment it names, in a space bound to it whose scenery this build draws. An environment it does not name has none, as Tomori's own do.
    pub fn set_locomotion(&mut self, locomotion: Vec<(Environment, Locomotion)>) {
        self.locomotion = locomotion;
    }

    /// How the participant may travel: as declared for the joined space's environment, when this build draws it and travel is not turned off on this headset, and otherwise not at all. See `design/locomotion.md`.
    pub fn locomotion(&self) -> Locomotion {
        if self.travel {
            // Declared in the environment's own coordinates, it is carried into the space's.
            self.declared_locomotion()
                .carried(&self.arrangement().carriage)
        } else {
            Locomotion::None
        }
    }

    /// The joined space's environment's declared surfaces, its floors and decks, carried into the space's coordinates,
    /// whether or not travel is turned off here: what hides a locked wall's padlock beneath a deck from an owner on it,
    /// and stops the ray before what is beyond a floor. See `design/projection.md`.
    pub fn surfaces(&self) -> Locomotion {
        self.declared_locomotion()
            .carried(&self.arrangement().carriage)
    }

    /// Turn travel off on this headset, staying room scale wherever an environment allows it, or on again. See `design/locomotion.md`.
    pub fn toggle_travel(&mut self) {
        self.travel = !self.travel;
    }

    /// How the joined space's environment lets the participant travel, as declared, whether or not it is turned off here.
    fn declared_locomotion(&self) -> &Locomotion {
        const NONE: &Locomotion = &Locomotion::None;
        // The environment the space is shown in: one this build cannot draw at the space's version is the grid, which declares none.
        self.joined_space()
            .map(|(space, _)| self.environment_of(space))
            .and_then(|environment| {
                self.locomotion
                    .iter()
                    .find(|(declared, _)| *declared == environment)
            })
            .map_or(NONE, |(_, locomotion)| locomotion)
    }

    /// The environments this build presents, each at its version, as its Hello lists them, each once: the grid first, always at its built-in version, then those it draws, at the versions their sceneries are registered at, and those it presents over the grid, at the versions they are presented at unless a scenery is registered for them. Each environment's version is in its own crate, which its server half registers it with too. See `design/environments.md`.
    pub fn presented(&self) -> Vec<tomori_protocol::EnvironmentVersion> {
        let mut presented: Vec<Environment> = Vec::new();
        for environment in std::iter::once(Environment::MINIMAL)
            .chain(self.drawable.iter().copied())
            .chain(self.over_grid.iter().copied())
        {
            if !presented.contains(&environment) {
                presented.push(environment);
            }
        }
        presented
            .into_iter()
            .map(|environment| {
                tomori_protocol::EnvironmentVersion::new(environment.id(), environment.version())
            })
            .collect()
    }

    /// What this build presents of the environment `id`: the environment it draws, or one it presents over the grid, or `None` when it cannot present it at all.
    fn presents(&self, id: &str) -> Option<Environment> {
        Environment::for_space(id, &self.drawable).or_else(|| {
            self.over_grid
                .iter()
                .copied()
                .find(|environment| environment.id() == id)
        })
    }

    /// The scenery to draw: the Minimal grid outside a space, and inside one the environment it is bound to, or the grid when this build cannot draw that environment, or draws it at another version than the space's. See `design/environments.md`.
    pub fn environment(&self) -> Environment {
        match &self.space {
            Some(space) if self.mode == Mode::Space => self.environment_of(space),
            _ => Environment::MINIMAL,
        }
    }

    /// The scenery to draw for `space`: its environment, when this build draws it at the version the space's state names, or names none, as from an older server; otherwise the grid. See `design/environments.md`.
    pub fn environment_of(&self, space: &SpaceState) -> Environment {
        Environment::for_space(&space.environment, &self.drawable)
            .filter(|environment| {
                space
                    .environment_version
                    .is_none_or(|version| version == environment.version())
            })
            .unwrap_or(Environment::MINIMAL)
    }

    /// What `space`'s environment needs that this build lacks, when it shows the grid instead: the environment, or the version it needs, as the space's state names it. `None` when it presents the environment at that version, or names none to compare. See `design/environments.md`.
    fn environment_notice(&self, space: &SpaceState) -> Option<String> {
        let id = space.environment.as_str();
        if id.is_empty() {
            return Some("The server named no environment.".into());
        }
        match (self.presents(id), space.environment_version) {
            // Says which side has which version. A server that agrees on versions does not offer this client the environment, so it is named by its label only when known, and otherwise by its identifier.
            (Some(presented), Some(version)) if presented.version() != version => Some(format!(
                "{} needs version {version}; this app has {}",
                self.environment_name(id),
                presented.version()
            )),
            (Some(_), _) => None,
            (None, Some(version)) => Some(format!("Unknown environment: {id} version {version}")),
            (None, None) => Some(format!("Unknown environment: {id}")),
        }
    }

    /// How the hand menu names the joined space's environment: what this build lacks of it, or else its name.
    fn shown_environment(&self, space: &SpaceState) -> String {
        self.environment_notice(space)
            .unwrap_or_else(|| self.environment_name(&space.environment))
    }

    /// Open the first creation ring.
    fn open_menu(&mut self) {
        self.close_menu();
        self.step = LobbyStep::Backend;
    }

    /// Step back to the previous creation ring, selecting what was chosen on it, or close the rings from the first.
    fn back_ring(&mut self) {
        match self.step {
            LobbyStep::Spaces => {}
            LobbyStep::Backend => self.close_menu(),
            LobbyStep::Directory => {
                self.step = LobbyStep::Backend;
                self.menu = self
                    .backends
                    .iter()
                    .position(|backend| Some(&backend.name) == self.menu_backend.as_ref())
                    .unwrap_or(0);
                self.menu_backend = None;
            }
            LobbyStep::Session => {
                self.step = LobbyStep::Directory;
                self.menu = self
                    .roots
                    .iter()
                    .position(|root| Some(&root.path) == self.menu_directory.as_ref())
                    .unwrap_or(0);
                self.menu_directory = None;
                self.sessions.clear();
                self.session_page = 0;
            }
            LobbyStep::Environment => {
                self.menu_create = None;
                if self.attaches() {
                    self.step = LobbyStep::Session;
                    self.menu = 0;
                } else {
                    self.step = LobbyStep::Directory;
                    self.menu = self
                        .roots
                        .iter()
                        .position(|root| Some(&root.path) == self.menu_directory.as_ref())
                        .unwrap_or(0);
                }
            }
        }
    }

    /// A title for a space the client creates, from the backend and the chosen root, since the headset cannot type one.
    fn lobby_title(&self) -> String {
        let backend = self.menu_backend.as_deref().unwrap_or("agent");
        let label = self
            .roots
            .iter()
            .find(|root| Some(&root.path) == self.menu_directory.as_ref())
            .map(|root| root.label.as_str())
            .unwrap_or("");
        format!("{backend} in {label}")
    }

    /// Create the space chosen on the rings, bound to `session` if one was chosen, then choose its environment if the server offers this client more than one.
    fn create(&mut self, session: Option<SessionSummary>) {
        let (Some(backend), Some(mut directory)) =
            (self.menu_backend.clone(), self.menu_directory.clone())
        else {
            return;
        };
        // An existing session runs in its own directory, which may be beneath the folder chosen; the server checks it
        // against its roots as it does the folder.
        if let Some(session) = session
            .as_ref()
            .filter(|session| !session.directory.is_empty())
        {
            directory = session.directory.clone();
        }
        let title = match &session {
            Some(session) if !session.title.trim().is_empty() => session.title.clone(),
            _ => self.lobby_title(),
        };
        let create = Creation {
            title,
            backend,
            directory,
            session: session.map(|session| session.id),
            instructions: self.next_instructions.take(),
        };
        self.submit(create);
    }

    /// Return to the server list, closing the connection.
    fn leave_server(&mut self) {
        self.actions.push(Action::Disconnect);
        self.connected = None;
        self.jumping = false;
        self.state = None;
        // What belonged to the connection goes with it, so that a token the server sends as the participant leaves, in the same frame, is not passed on.
        self.certificate = None;
        self.pairing = None;
        self.pairing_closed = false;
        self.pairing_window = None;
        self.knock = None;
        self.spaces.clear();
        self.listed = false;
        self.close_space();
        self.close_menu();
        self.talk = None;
        self.leaving = false;
        self.error = None;
        self.ended = None;
        self.mode = Mode::Servers;
    }

    /// The pending approval that a press would decide, if any.
    fn approval(&self) -> Option<&tomori_protocol::Approval> {
        self.space
            .as_ref()
            .and_then(|space| space.approval.as_ref())
            .filter(|approval| self.decided.as_ref() != Some(&approval.id))
    }

    /// Whether the right stick chooses among an approval's choices, rather than selecting queued entries.
    #[cfg(any(not(target_os = "android"), test))]
    pub fn is_choosing(&self) -> bool {
        self.mode == Mode::Space && self.approval().is_some()
    }

    /// Left stick: choose a server or space, or page through messages.
    pub fn navigate(&mut self, horizontal: i32, vertical: i32) {
        match self.mode {
            Mode::Servers => {
                self.server = shift(self.server, vertical, self.servers.len());
                if vertical != 0 {
                    self.chosen = true;
                    self.missing = None;
                }
            }
            Mode::Lobby => match self.step {
                LobbyStep::Spaces => {
                    self.selected = shift(self.selected, vertical, self.step_list(self.step))
                }
                step => self.menu = shift(self.menu, vertical, self.step_list(step)),
            },
            Mode::Space => {
                let Some(space) = &self.space else {
                    return;
                };
                if let Some(selected) = self.space_menu {
                    self.space_menu = Some(shift(selected, vertical, self.menu_items().len()));
                    return;
                }
                let (count, messages) = (items(space), space.messages.len());
                if self.request.is_none() && horizontal != 0 {
                    self.message = shift(self.message, horizontal, count);
                    self.page = 0;
                }
                if vertical != 0 {
                    self.page = self.page.saturating_add_signed(vertical as isize);
                }
                let last_page = paginate(&self.body().1, self.body_rows()).len() - 1;
                self.page = self.page.min(last_page);
                // Back at the newest page, follow new messages again.
                self.follow = self.request.is_none()
                    && self.message + 1 == messages.max(1)
                    && self.page == last_page;
            }
        }
    }

    /// Right stick left or right: choose among an approval's choices: allow once, each standing option, then deny.
    #[cfg(any(not(target_os = "android"), test))]
    pub fn choose(&mut self, direction: i32) {
        if let Some(count) = self.approval().map(|approval| approval.options.len() + 2) {
            self.option = shift(self.option, direction, count);
        }
    }

    /// The entries still waiting in the queue, oldest first, which any participant may cancel.
    fn waiting(&self) -> Vec<&tomori_protocol::QueueEntry> {
        self.space
            .as_ref()
            .map(|space| {
                space
                    .queue
                    .iter()
                    .filter(|entry| entry.state == QueueState::Queued)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Right stick up or down, while no approval is presented: select a waiting entry in the queue strip, which A then cancels. Moving past either end clears the selection.
    #[cfg(any(not(target_os = "android"), test))]
    pub fn select_queued(&mut self, direction: i32) {
        if self.mode != Mode::Space || self.is_choosing() || self.space_menu.is_some() {
            return;
        }
        if !self.role.participates() {
            self.notice = Some(VISITING.into());
            return;
        }
        let waiting: Vec<String> = self
            .waiting()
            .iter()
            .map(|entry| entry.id.clone())
            .collect();
        let current = self
            .queue_selected
            .as_ref()
            .and_then(|id| waiting.iter().position(|entry| entry == id));
        let next = match current {
            None if direction > 0 => (!waiting.is_empty()).then_some(0),
            None => waiting.len().checked_sub(1),
            Some(index) => index
                .checked_add_signed(direction as isize)
                .filter(|index| *index < waiting.len()),
        };
        self.queue_selected = next.map(|index| waiting[index].clone());
    }

    /// Advance what waits on time in a space at `now`: a queued entry's cancellation whose undo time has run out is sent. Call every frame.
    pub fn tick_space(&mut self, now: Instant) {
        if let Some((entry, due)) = &self.cancelling
            && now >= *due
        {
            let entry = entry.clone();
            self.cancelling = None;
            self.cancel_now(entry);
        }
        // The images of the panels shown that are not held yet, a few at a time. See `design/projection.md`.
        let shown = self.shown_images();
        for hash in self.images.wanted(&shown.iter().collect::<Vec<_>>(), now) {
            self.actions
                .push(Action::Send(ClientMessage::FetchImage { image: hash }));
        }
        // A selected entry that has been sent or cancelled is no longer selected.
        if let Some(selected) = &self.queue_selected
            && !self.waiting().iter().any(|entry| &entry.id == selected)
        {
            self.queue_selected = None;
        }
    }

    /// B held in a space for `held`, or released (`None`). Holding it for [`LEAVE_HOLD`] leaves the space, once per hold. Returns whether this call left.
    pub fn hold_back(&mut self, held: Option<Duration>) -> bool {
        let Some(held) = held.filter(|_| self.mode == Mode::Space) else {
            self.leave_hold = None;
            return false;
        };
        let left = self.leave_hold.is_some_and(|(_, left)| left);
        self.leave_hold = Some((held, left));
        if held >= LEAVE_HOLD && !left {
            self.leave_hold = Some((held, true));
            self.leave();
            return true;
        }
        false
    }

    /// Whether the right stick up and down selects queued entries now, rather than doing anything else.
    #[cfg(test)]
    pub fn selects_queue(&self) -> bool {
        self.mode == Mode::Space
            && self.role.participates()
            && !self.is_choosing()
            && self.space_menu.is_none()
            && !self.waiting().is_empty()
    }

    /// Leave the space for the lobby. A cancellation still in its undo time is sent first, as it was asked for.
    fn leave(&mut self) {
        if let Some((entry, _)) = self.cancelling.take() {
            self.cancel_now(entry);
        }
        self.leaving = true;
        self.close_space();
        self.listed = false;
        self.mode = Mode::Lobby;
        self.send(ClientMessage::Leave);
    }

    /// The entries of the space's list: its modes, then its speech setting where the server has one.
    fn menu_items(&self) -> Vec<MenuItem> {
        let Some(space) = &self.space else {
            return Vec::new();
        };
        let mut items: Vec<MenuItem> = space
            .modes
            .iter()
            .map(|mode| MenuItem::Mode(mode.clone()))
            .collect();
        if space.speech != SpeechSetting::Unknown {
            items.push(MenuItem::Speech(space.speech));
        }
        items
    }

    /// A while the ray highlights an arrival object chooses that object, as
    /// the trigger does, so that A never acts on a different item from the one
    /// pointed at. Otherwise A accepts as [`Self::accept`] does. Returns true
    /// when A has nothing to do here.
    pub fn press_a(&mut self, highlighted: Option<&arrival::Key>) -> bool {
        self.refused = false;
        match highlighted {
            Some(key) if self.arrival().is_some() => {
                self.choose_object(key);
                false
            }
            _ => self.accept(),
        }
    }

    /// Let the request shown have been there for longer than [`REQUEST_GRACE`], as tests of the frame step cannot wait for.
    #[cfg(test)]
    pub(crate) fn settle_request(&mut self) {
        self.request_at = None;
    }

    /// Whether the last press of A was refused, saying why, as just after a request appears, so that it is not felt as a choice. See `design/feedback.md`.
    pub fn a_refused(&self) -> bool {
        self.refused
    }

    /// A: connect, join, confirm the chosen approval choice, choose from the space's list, or cancel the selected queued entry or undo that. Returns true when A has nothing to do here. A never resets the swarm in a space, where it would hide what A does; see `design/interaction.md`.
    pub fn accept(&mut self) -> bool {
        // Choosing anything takes over from a jump still waiting for the listing.
        self.jumping = false;
        self.refused = false;
        match self.mode {
            Mode::Servers => {
                match self.servers.get(self.server).cloned() {
                    Some(choice) if choice.is_available() => self.connect(choice),
                    Some(choice) => {
                        self.refused = true;
                        self.error = Some(format!(
                            "Cannot join {}: {}.",
                            choice.label,
                            choice.unavailable.unwrap_or_default()
                        ));
                    }
                    None => {}
                }
                false
            }
            Mode::Lobby => {
                match self.step {
                    LobbyStep::Spaces => match self.spaces.get(self.selected) {
                        Some(space) => {
                            let space = space.id.clone();
                            self.join(space);
                        }
                        // The create object opens its first ring.
                        None if self.creates() => {
                            self.ended = None;
                            self.open_menu();
                        }
                        None => {}
                    },
                    LobbyStep::Backend => {
                        if let Some(backend) = self.backends.get(self.menu) {
                            self.menu_backend = Some(backend.name.clone());
                            self.menu = 0;
                            self.step = LobbyStep::Directory;
                        }
                    }
                    LobbyStep::Directory => {
                        if let Some(root) = self.roots.get(self.menu) {
                            self.menu_directory = Some(root.path.clone());
                            if self.attaches() {
                                self.menu = 0;
                                self.step = LobbyStep::Session;
                                self.list_sessions();
                            } else {
                                self.create(None);
                            }
                        }
                    }
                    LobbyStep::Session => {
                        let shown = self.paged_sessions();
                        match self.menu.checked_sub(1) {
                            None => self.create(None),
                            Some(index) if index < shown.len() => {
                                let session = shown[index].clone();
                                self.create(Some(session));
                            }
                            Some(_) => self.next_session_page(),
                        }
                    }
                    LobbyStep::Environment => {
                        let environment = self
                            .environments
                            .get(self.menu)
                            .map(|offered| offered.id.clone());
                        if let (Some(environment), Some(create)) =
                            (environment, self.menu_create.take())
                        {
                            self.send(create.message(Some(environment)));
                            self.close_menu();
                        }
                    }
                }
                false
            }
            Mode::Space => {
                if let Some(selected) = self.space_menu {
                    match self.menu_items().get(selected) {
                        Some(MenuItem::Mode(mode)) => {
                            let mode = mode.id.clone();
                            self.send(ClientMessage::SetMode { mode });
                        }
                        Some(MenuItem::Speech(speech)) => {
                            let speech = match speech {
                                SpeechSetting::Brief => SpeechSetting::Full,
                                _ => SpeechSetting::Brief,
                            };
                            self.send(ClientMessage::SetSpeech { speech });
                        }
                        None => {}
                    }
                    self.space_menu = None;
                    return false;
                }
                if self
                    .space
                    .as_ref()
                    .is_some_and(|space| space.approval.is_some() || space.question.is_some())
                    && self
                        .request_at
                        .is_some_and(|at| at.elapsed() < REQUEST_GRACE)
                {
                    self.refused = true;
                    self.notice = Some("Read the request before choosing.".into());
                    return false;
                }
                // Pointing at a checklist item on a text panel, A ticks or unticks it, for everyone, after the right
                // wrist's transcript, which the frame step gives A first, and the list. See `design/interaction.md`.
                if let Some((panel, line)) = self.pointed_item.clone()
                    && let Some(checked) = self.item_checked(&panel, line)
                {
                    if !self.check_item(&panel, line, !checked) {
                        self.refused = true;
                    }
                    return false;
                }
                if self.decide_on_wrist
                    && self
                        .space
                        .as_ref()
                        .is_some_and(|space| space.approval.is_some() || space.question.is_some())
                {
                    self.refused = true;
                    self.notice = Some("Turn your right wrist to decide.".into());
                    return false;
                }
                if self.approval().is_some() {
                    self.decide();
                    return false;
                }
                // Removing queued entries is on the right wrist.
                if self.decide_on_wrist {
                    return !self.last_meanings();
                }
                // A decision on its way still owns the button.
                if self
                    .space
                    .as_ref()
                    .is_some_and(|space| space.approval.is_some())
                {
                    return false;
                }
                // A undoes a cancellation still in its undo time, or cancels the selected entry after it.
                if (self.cancelling.is_some() || self.queue_selected.is_some())
                    && !self.role.participates()
                {
                    self.refused = true;
                    self.notice = Some(VISITING.into());
                    return false;
                }
                if let Some((entry, _)) = self.cancelling.take() {
                    self.queue_selected = Some(entry);
                    self.notice = Some("Not cancelled.".into());
                    false
                } else if let Some(entry) = self.queue_selected.take() {
                    self.cancelling = Some((entry, Instant::now() + CANCEL_UNDO));
                    self.notice = None;
                    false
                } else {
                    !self.last_meanings()
                }
            }
        }
    }

    /// A's last meanings, after everything that decides something, a request, a decision on its way and a queued entry,
    /// so that a press meant for a prompt never acts on the world beyond Tomori: a choosable row of an environment's
    /// list pointed at, which it chooses for that environment, unless a request, an approval or a question, waits, when
    /// it is refused, saying to answer the request first, the press being likely meant for it; then a teleport pad
    /// pointed at. Returns whether it did either, or was refused, as a visitor's choice is. See
    /// `design/panel-actions.md` and `design/locomotion.md`.
    fn last_meanings(&mut self) -> bool {
        if self.pointed_row().is_some() {
            let requested = self
                .space
                .as_ref()
                .is_some_and(|space| space.approval.is_some() || space.question.is_some());
            if requested {
                self.refused = true;
                self.notice = Some(ANSWER_FIRST.into());
            } else if !self.choose_row() {
                self.refused = true;
            }
            return true;
        }
        self.choose_pad()
    }

    /// Pointing at a teleport pad, A chooses it, as the trigger does, which the frame step takes the participant to the
    /// pad paired with: the last of A's meanings, after everything that decides something, such as a request, so that
    /// a prompt is never answered by teleporting. Returns whether it chose one. See `design/locomotion.md`.
    fn choose_pad(&mut self) -> bool {
        self.pad_chosen |= self.pointed_pad;
        self.pointed_pad
    }

    /// Send the decision for the chosen choice of the pending approval.
    fn decide(&mut self) {
        let Some(approval) = self.approval() else {
            return;
        };
        let request = approval.id.clone();
        let (decision, option) = match self.option.checked_sub(1) {
            None => (Decision::AllowOnce, None),
            Some(index) if index < approval.options.len() => {
                (Decision::Grant, Some(approval.options[index].id.clone()))
            }
            Some(_) => (Decision::Deny, None),
        };
        if self.send(ClientMessage::Decide {
            request: request.clone(),
            decision,
            option,
        }) {
            self.decided = Some(request);
        }
    }

    /// Decide approvals and remove queued entries only on the right wrist, as the headset and desktop window do.
    pub fn decide_on_wrist(&mut self) {
        self.decide_on_wrist = true;
    }

    /// The right wrist's transcript, `columns` wide, read-only when shown from hand tracking alone. See `design/wrist-transcript.md`.
    pub fn transcript(&mut self, columns: usize, hands_only: bool) -> Vec<crate::ui_text::Row> {
        let now = Instant::now();
        self.observe_problem(now);
        // The problem line shows it, so it has been seen and no longer cues the wrist.
        if let Some(problem) = self.problem() {
            self.problem_noticed = Some(problem);
        }
        let problem = self.problem_text(now);
        let answerable = self.answerable().is_some();
        let Some(space) = self.space.as_ref().filter(|_| self.mode == Mode::Space) else {
            return Vec::new();
        };
        let cancelling = self.cancelling.as_ref().map(|(id, due)| {
            let left = due.saturating_duration_since(now).as_secs_f32() / CANCEL_UNDO.as_secs_f32();
            (id.as_str(), left)
        });
        let grace = self
            .request_at
            .is_some_and(|at| at.elapsed() < REQUEST_GRACE);
        // A decision is only awaited for the approval shown: another request forgets it.
        let deciding = self.decided.is_some();
        // Laid out again only when something it shows has changed, as it is drawn every frame.
        let key = TranscriptKey {
            revision: self.revision,
            wrist: self.wrist.clone(),
            role: self.role,
            source: self.source,
            deciding,
            cancelling: cancelling.map(|(id, left)| (id.to_owned(), (left * 60.0) as u32)),
            problem: problem.clone(),
            answerable,
            hands_only,
            grace,
            columns,
        };
        if let Some((laid_out, rows)) = &self.laid_out
            && *laid_out == key
        {
            return rows.clone();
        }
        let input = crate::transcript::Input {
            you: self.source,
            participates: self.role.participates(),
            known: Some(&self.known),
            deciding,
            cancelling,
            problem,
            answerable,
            hands_only,
            grace,
        };
        let rows = crate::transcript::layout(space, &input, &mut self.wrist, columns);
        // Laying out settles the focus, which is what the next frame compares.
        self.laid_out = Some((
            TranscriptKey {
                wrist: self.wrist.clone(),
                ..key
            },
            rows.clone(),
        ));
        rows
    }

    /// What the transcript's stick moves through.
    fn wrist_items(&self) -> Vec<crate::transcript::Item> {
        let Some(space) = self.space.as_ref().filter(|_| self.mode == Mode::Space) else {
            return Vec::new();
        };
        let input = crate::transcript::Input {
            you: self.source,
            participates: self.role.participates(),
            known: Some(&self.known),
            deciding: self.decided.is_some(),
            ..Default::default()
        };
        // The width it was last laid out at, on the wrist or detached, so that moving through it matches what is shown.
        let columns = self
            .laid_out
            .as_ref()
            .map_or(crate::transcript::WRIST_COLUMNS, |(key, _)| key.columns);
        crate::transcript::items(space, &input, &self.wrist, columns)
    }

    /// The newest message's id, which the transcript counts newer messages from.
    fn newest(&self) -> Option<String> {
        self.space
            .as_ref()
            .and_then(|space| space.messages.last())
            .map(|message| message.id.clone())
    }

    /// The right wrist's transcript opened: while a request waits, focus its title and give A a moment of grace again, so that a reflex press decides nothing.
    pub fn wrist_opened(&mut self) {
        let pending = self
            .space
            .as_ref()
            .is_some_and(|space| space.approval.is_some() || space.question.is_some());
        if pending && self.mode == Mode::Space {
            let newest = self.newest();
            self.wrist.focus_card(newest.as_deref());
            self.request_at = Some(Instant::now());
        }
    }

    /// The right stick up (`-1`) or down (`1`) on the transcript.
    pub fn wrist_step(&mut self, direction: i32) {
        let items = self.wrist_items();
        let newest = self.newest();
        self.wrist.step(&items, direction, newest.as_deref());
    }

    /// The right stick left (`-1`) or right (`1`) on the transcript: the previous or next turn.
    pub fn wrist_jump(&mut self, direction: i32) {
        let items = self.wrist_items();
        let newest = self.newest();
        self.wrist.jump(&items, direction, newest.as_deref());
    }

    /// The right wrist's focus: a turn, a request's button or a queued entry.
    pub fn wrist_focus(&self) -> Option<&crate::transcript::Focus> {
        self.wrist.focus.as_ref()
    }

    /// A on the transcript: expand or collapse a turn, confirm a choice, or remove a queued entry or undo that. Returns whether it did any of them, rather than nothing or refusing.
    pub fn wrist_accept(&mut self) -> bool {
        use crate::transcript::Focus;
        let items = self.wrist_items();
        self.wrist.settle(&items);
        match self.wrist.focus.clone() {
            None | Some(Focus::Card(0)) => false,
            Some(Focus::Group(..)) => {
                self.wrist.toggle(&items);
                true
            }
            // Settled, the focus is only on what this participant may act on: a visitor is offered no choices or entries to remove.
            Some(Focus::Card(choice)) => {
                if self
                    .request_at
                    .is_some_and(|at| at.elapsed() < REQUEST_GRACE)
                {
                    self.notice = Some("Read the request before choosing.".into());
                    false
                } else {
                    // Settled, a choice is focused only while an approval waits for this participant's decision.
                    self.option = choice - 1;
                    self.decide();
                    true
                }
            }
            Some(Focus::Queue(entry)) => {
                match self.cancelling.take() {
                    // A again within the undo time keeps it.
                    Some((cancelling, _)) if cancelling == entry => {
                        self.notice = Some("Not removed.".into());
                        true
                    }
                    other => {
                        if let Some((other, _)) = other {
                            self.cancel_now(other);
                        }
                        // Settled, the focus is only on an entry still waiting.
                        self.cancelling = Some((entry, Instant::now() + CANCEL_UNDO));
                        self.notice = None;
                        true
                    }
                }
            }
        }
    }

    /// B on the transcript: collapse, stop focusing and follow live, or dismiss the problem. Returns false when there was nothing to back out of, and never decides, cancels or leaves.
    pub fn wrist_back(&mut self) -> bool {
        if self.wrist.back() {
            return true;
        }
        if self.error.take().is_some() {
            return true;
        }
        if let Some(error) = self.space.as_ref().and_then(|space| space.error.clone())
            && self.dismissed.as_ref() != Some(&error)
        {
            self.dismissed = Some(error);
            return true;
        }
        false
    }

    /// What waits for this participant on the right wrist, for its cue at each frame: `Some(false)` for a request, until it is decided or answered, and `Some(true)` for a problem not yet seen. See `design/wrist-transcript.md`.
    pub fn wrist_waiting(&mut self) -> Option<bool> {
        let problem = self.problem();
        // A problem seen stays seen only while it lasts; shown again later, it is news again.
        if self.problem_noticed != problem {
            self.problem_noticed = None;
        }
        let space = self.space.as_ref().filter(|_| self.mode == Mode::Space)?;
        if self.approval().is_some() || space.question.is_some() {
            Some(false)
        } else {
            problem
                .filter(|problem| self.problem_noticed.as_ref() != Some(problem))
                .map(|_| true)
        }
    }

    /// What the status pill shows at `now`, while in a space and it is not turned off. See `design/interaction.md`.
    pub fn pill_status(&mut self, now: Instant) -> Option<crate::pill::Status> {
        use crate::pill::{Need, Status};
        if !self.pill {
            return None;
        }
        self.observe_problem(now);
        let problem = self.problem_text(now);
        let space = self.space.as_ref().filter(|_| self.mode == Mode::Space)?;
        let connection = match &self.state {
            None | Some(ConnectionState::Connecting { attempt: 1 }) => Some("Connecting...".into()),
            Some(ConnectionState::Connecting { attempt }) => {
                Some(format!("Connecting (attempt {attempt})..."))
            }
            Some(ConnectionState::Disconnected { retry_in, .. }) => Some(format!(
                "Offline, retrying in {} s",
                retry_in.as_secs_f32().ceil()
            )),
            Some(ConnectionState::Closed) => Some("Disconnected.".into()),
            // Connected, or in a state a later client knows of, saying only how much has arrived of a long frame
            // arriving, as the projection whole on joining, which is forgotten whenever the connection is not connected.
            Some(_) => self
                .receiving
                .map(|(received, length)| format!("Syncing space... {}%", received * 100 / length)),
        };
        let leaving = self
            .leave_hold
            .is_some_and(|(held, left)| !left && held >= Duration::from_millis(150));
        let need = if self.approval().is_some() {
            Some(Need::Approval)
        } else if space.question.is_some() {
            Some(Need::Question)
        } else if let Some(problem) = problem {
            Some(Need::Problem(problem))
        } else if let Some(connection) = connection {
            Some(Need::Connection(connection))
        } else if leaving {
            Some(Need::Leaving)
        } else {
            self.notice.clone().map(Need::Notice)
        };
        Some(Status {
            activity: space.activity,
            speaking: self.turn.speaking_as(now, self.muted),
            // A space that closes plays nothing more.
            waiting: self.turn.waiting(now) && space.activity != tomori_protocol::Activity::Closed,
            pulse: (self.turn.pulse(now) * 255.0).round() as u8,
            mode: space
                .current_mode()
                .map(|mode| mode.name.clone())
                .or_else(|| space.mode.clone().filter(|_| !space.modes.is_empty())),
            title: space.title.clone(),
            replaying: self.replaying,
            need,
        })
    }

    /// State `limits` in each `Hello` from the next connection on, as the desktop window states [`DESKTOP_LIMITS`].
    pub fn set_limits(&mut self, limits: tomori_protocol::Limits) {
        self.limits = limits;
    }

    /// How much of a long frame has arrived, `received` of `length` bytes, while one is arriving.
    pub fn receiving(&self) -> Option<(usize, usize)> {
        self.receiving
    }

    /// Turn the status pill on or off, on this headset.
    pub fn toggle_pill(&mut self) {
        self.pill = !self.pill;
    }

    /// The problem with how long it has been shown, on one line, for the transcript.
    fn problem_text(&self, now: Instant) -> Option<String> {
        let problem = self.problem()?;
        Some(format!(
            "! {} {}",
            age(self.problem_age(&problem, now)),
            problem.replace('\n', " ")
        ))
    }

    /// How long `problem` has been shown at `now`: no time at all if it was not shown before.
    fn problem_age(&self, problem: &str, now: Instant) -> Duration {
        match &self.problem_seen {
            Some((seen, since)) if seen == problem => now.saturating_duration_since(*since),
            _ => Duration::ZERO,
        }
    }

    /// B: in a space, close what is open, or dismiss the problem shown; it never decides an approval, cancels input or leaves, which takes holding it (see [`Self::hold_back`]). In the lobby, dismiss what is shown or return to the server list.
    pub fn back(&mut self) {
        self.back_or_environment(false);
    }

    /// B, as [`Self::back`], while the environment's client side is in focus, its selectable object pointed at, or not: in a space with nothing of its own to close or dismiss, B is then the client side's, [`Back::Environment`], in place of the notice that holding it leaves. Returns whose it was, of the rungs from [`Back::Modes`] on, in their order. See `design/interaction.md`.
    pub fn back_or_environment(&mut self, environment: bool) -> Back {
        self.jumping = false;
        match self.mode {
            Mode::Servers => Back::Lobby,
            Mode::Lobby if self.dismissable() => {
                self.error = None;
                self.ended = None;
                Back::Lobby
            }
            // At the spaces there is nothing to return from, so go to the server list.
            Mode::Lobby if self.step == LobbyStep::Spaces => {
                self.leave_server();
                Back::Lobby
            }
            Mode::Lobby => {
                self.back_ring();
                Back::Lobby
            }
            Mode::Space => {
                if self.space_menu.take().is_some() {
                    return Back::Modes;
                }
                if self.queue_selected.take().is_some() {
                    return Back::Queued;
                }
                // Stop waiting for a new name or title, and dismiss why the last was not changed.
                if std::mem::take(&mut self.naming) | std::mem::take(&mut self.titling) {
                    if matches!(self.outcome, Some((Outcome::NameFailed(..), _))) {
                        self.error = None;
                    }
                    return Back::Naming;
                }
                // The card saying that no speech was heard, which is no problem.
                if let Some((Outcome::Unheard, at)) = self.outcome
                    && self.unheard_shown(Instant::now(), at)
                {
                    self.outcome = None;
                    return Back::Unheard;
                }
                if self.error.take().is_some() {
                    return Back::Problem;
                }
                if let Some(error) = self.space.as_ref().and_then(|space| space.error.clone())
                    && self.dismissed.as_ref() != Some(&error)
                {
                    self.dismissed = Some(error);
                    return Back::SpaceProblem;
                }
                // B on its own never dismisses a panel, which takes carrying it with the grip; pointed at one, a
                // participant is told how. See `design/projection.md`.
                if let Some(tomori_protocol::Resource::Panel { panel, .. }) =
                    self.pointing_at.as_ref().or(self.pointing_left.as_ref())
                    && self.role.participates()
                    && self
                        .projection()
                        .is_some_and(|projection| projection.panel(panel).is_some())
                {
                    self.notice = Some(THROW_HOW.into());
                    return Back::Panel;
                }
                if environment {
                    return Back::Environment;
                }
                self.notice = Some("Hold B to leave the space.".into());
                Back::Nothing
            }
        }
    }

    /// Ask the chosen backend for its sessions in the chosen directory.
    fn list_sessions(&mut self) {
        if let (Some(backend), Some(directory)) =
            (self.menu_backend.clone(), self.menu_directory.clone())
        {
            self.send(ClientMessage::ListSessions { backend, directory });
        }
    }

    /// Y: browse again, refresh the spaces or the sessions being shown, or in a space open or close the list of the agent's modes, with their descriptions, and the speech setting. A space with neither fetches its state again instead.
    pub fn refresh(&mut self) {
        // In a space a problem stays until it is dismissed; see `design/interaction.md`.
        if self.mode != Mode::Space {
            self.error = None;
        }
        match self.mode {
            Mode::Servers => self.actions.push(Action::Rescan),
            Mode::Lobby => match self.step {
                LobbyStep::Session => self.list_sessions(),
                LobbyStep::Spaces
                | LobbyStep::Backend
                | LobbyStep::Directory
                | LobbyStep::Environment => self.list(),
            },
            Mode::Space => {
                if self.space_menu.take().is_some() {
                    return;
                }
                let Some(space) = &self.space else {
                    return;
                };
                let items = self.menu_items();
                if items.is_empty() {
                    let space = space.id.clone();
                    self.joining = Some(space.clone());
                    self.send(ClientMessage::Join { space });
                    return;
                }
                // The list opens on the current mode, so A alone keeps it.
                let current = items
                    .iter()
                    .position(|item| matches!(item, MenuItem::Mode(mode) if Some(&mode.id) == space.mode.as_ref()))
                    .unwrap_or(0);
                self.space_menu = Some(current);
            }
        }
    }

    /// X pressed or released: push-to-talk. While held, the microphone sends `agent` audio, heard by the other participants and transcribed for the agent; releasing sends it to be queued, or, while a question Tomori can answer is presented, as the answer to its current field.
    pub fn talk(&mut self, pressed: bool) {
        if !pressed {
            if let Some(Talk::Recording {
                utterance,
                heard,
                answering,
                naming,
                ..
            }) = self.talk.take()
            {
                self.actions.push(Action::EndUtterance);
                self.talk = Some(Talk::Sending {
                    utterance,
                    heard,
                    answering,
                    received: None,
                    naming,
                });
            }
            return;
        }
        if matches!(self.talk, Some(Talk::Recording { .. })) {
            return;
        }
        if self.mode != Mode::Space || self.space.is_none() {
            self.notice = Some("Join a space to talk to the agent.".into());
            return;
        }
        if !self.role.participates() {
            self.notice = Some(VISITING.into());
            return;
        }
        if !matches!(self.state, Some(ConnectionState::Connected { .. })) {
            self.error = Some("Offline; cannot record.".into());
            return;
        }
        self.utterances += 1;
        let utterance = format!("{}-{}", self.launch, self.utterances);
        // A new recording replaces what the indicator said of the last.
        self.outcome = None;
        // A problem shown stays until it is dismissed or replaced; see `design/interaction.md`.
        self.notice = None;
        // Words chosen to say a new name, or title, answer nothing; see `design/security.md`.
        let naming = self.take_naming();
        let answer = self
            .answerable()
            .filter(|_| naming.is_none())
            .map(|question| tomori_protocol::AnswerTarget {
                question: question.id.clone(),
                field: question.field.clone(),
            });
        let answering = answer.is_some();
        // What is pointed at as X is pressed is what the words are about, captured now, as the server captures it;
        // pointing elsewhere while talking does not change it. An answer goes to the question presented, and a name to
        // the participant, so either is about nothing pointed at.
        // Both rays' targets, the one moved onto most recently first, and one alone where both are on it.
        let (first, second) = if self.left_first {
            (&self.pointing_left, &self.pointing_at)
        } else {
            (&self.pointing_at, &self.pointing_left)
        };
        let mut attended: Vec<tomori_protocol::Resource> = Vec::new();
        for target in [first, second].into_iter().flatten() {
            if naming.is_none() && self.knows(target) && !attended.contains(target) {
                attended.push(target.clone());
            }
        }
        attended.truncate(tomori_protocol::ATTENTION_LIMIT);
        let pointed = attended.first().cloned();
        let attention = if answering { Vec::new() } else { attended };
        // The words follow the newest of the agent's messages shown, which is what they answer unless it says more
        // before they arrive; the server is told which, so that it can say so then. See `design/interaction.md`.
        let seen = self.newest_said().filter(|_| !answering);
        let begun = Utterance::new(utterance.clone(), self.capture_generation);
        self.speak_over();
        self.actions.push(Action::BeginUtterance(match naming {
            Some(Naming::Name) => begun.naming(),
            Some(Naming::Title) => begun.titling(),
            None => begun.answering(answer).attending(attention).seeing(seen),
        }));
        self.talk = Some(Talk::Recording {
            utterance,
            started: Instant::now(),
            heard: String::new(),
            answering,
            naming,
            pointed,
        });
    }

    /// The newest of the agent's messages in the space that says anything, as shown, by identifier: also the newest
    /// whose speech played, since speech is of text already shown.
    fn newest_said(&self) -> Option<String> {
        self.space.as_ref().and_then(|space| {
            space
                .messages
                .iter()
                .rfind(|message| message.role == Role::Assistant && !message.text.trim().is_empty())
                .map(|message| message.id.clone())
        })
    }

    /// The worker the participant points at now, by identifier, or `None`: from the headset's ray each frame, or a
    /// console command. The worker pointed at as X is pressed is sent as the words begin, and pointing elsewhere
    /// while X is held does not change it. See `design/interaction.md`.
    pub fn point_at(&mut self, worker: Option<&str>) {
        self.point(worker.map(tomori_protocol::Resource::worker));
    }

    /// What the participant points at now, a worker or a card, as [`Self::point_at`] says of workers.
    pub fn point(&mut self, target: Option<tomori_protocol::Resource>) {
        if self.pointing_at != target {
            self.pointing_at = target;
        }
    }

    /// What the participant's left ray points at now, from the headset each frame, as [`Self::point`] says of the
    /// right's, and whether it moved onto it more recently than the right moved onto its own, `first`: words begun
    /// name both, that one first. See `design/interaction.md`.
    pub fn point_left(&mut self, target: Option<tomori_protocol::Resource>, first: bool) {
        if self.pointing_left != target {
            self.pointing_left = target;
        }
        self.left_first = first;
    }

    /// Know `object`, something an environment shows, as `label`, as either ray points at it: attended to as
    /// `Resource::object` while they talk, as a worker or card is, and named `label` while they do.
    pub fn label_object(&mut self, object: &str, label: &str) {
        if self.object_labels.get(object).map(String::as_str) != Some(label) {
            self.object_labels
                .insert(object.to_owned(), label.to_owned());
        }
    }

    /// The checklist item the participant points at now, by its panel and the line of its body the item begins on, or
    /// `None`: from the headset's ray or the desktop's mouse each frame, on a panel pointed at rather than carried. A
    /// ticks or unticks it. See `design/projection.md`.
    pub fn point_item(&mut self, item: Option<(String, u32)>) {
        self.pointed_item = item;
    }

    /// Whether the participant points at a teleport pad now, from the headset's ray or the desktop's mouse each frame,
    /// which A chooses. See `design/locomotion.md`.
    pub fn point_pad(&mut self, pointed: bool) {
        self.pointed_pad = pointed;
    }

    /// Whether A chose the teleport pad pointed at since this was last asked, which the frame step then takes the
    /// participant from.
    pub fn take_pad_choice(&mut self) -> bool {
        std::mem::take(&mut self.pad_chosen)
    }

    /// Whether the checklist item that begins on `line` of the panel `panel`, which the joined space shows, is ticked, as
    /// the item A would tick or untick there: `None` where there is no such item with a box, the panel is not a text
    /// panel, or the server takes no ticking. See `design/projection.md`.
    pub fn item_checked(&self, panel: &str, line: u32) -> Option<bool> {
        if !self.joined_space().is_some_and(|(space, _)| space.checks) {
            return None;
        }
        let shown = self.projection()?.panel(panel)?;
        // An environment's list is chosen, never ticked, whatever its rows' text shows.
        if shown.code.is_some() || shown.image.is_some() || !shown.rows.is_empty() {
            return None;
        }
        tomori_protocol::checklist::items(&shown.body)
            .into_iter()
            .find(|item| u32::try_from(item.line).ok() == Some(line))
            .and_then(|item| item.checked)
    }

    /// The item the participant points at now, as the frame step last reported it, for its tests.
    #[cfg(test)]
    pub(crate) fn pointed_item(&self) -> Option<&(String, u32)> {
        self.pointed_item.as_ref()
    }

    /// The row of an environment's list panel on `line` of the panel `panel`'s body, which the joined space shows, by
    /// the row's id and whether it may be chosen; `None` where the panel is not a list, or has no row there. See
    /// `design/panel-actions.md`.
    pub fn row_at(&self, panel: &str, line: u32) -> Option<&tomori_protocol::ListRow> {
        self.projection()?.panel(panel)?.row_on(line)
    }

    /// The choosable row the participant points at now, by its panel and the row's id, as A or the trigger would choose
    /// it: `None` where they point at no such row, or the server takes no choices. A visitor points at it too, and is
    /// told they only watch.
    pub fn pointed_row(&self) -> Option<(String, String)> {
        self.choosable_row(self.pointed_item.as_ref()?)
    }

    /// The choosable row that begins on `item`'s line of its panel, as [`Self::pointed_row`] says of the item pointed
    /// at, by its panel and the row's id: as either ray points at it, which that hand's trigger chooses.
    pub fn choosable_row(&self, (panel, line): &(String, u32)) -> Option<(String, String)> {
        if !self.joined_space().is_some_and(|(space, _)| space.choices) {
            return None;
        }
        self.row_at(panel, *line)
            .filter(|row| row.choosable)
            .map(|row| (panel.clone(), row.id.clone()))
    }

    /// Choose the row the participant points at, [`Self::pointed_row`], for its environment, as A or the trigger does:
    /// sent by the row's id with the panel's version the server last sent and the revision of the row's key, so that a
    /// choice made as the list changes names the row the participant saw, and its environment is handed that row's key
    /// or none. Says whether it chose, as only a participant does; a visitor is told they
    /// only watch. See `design/panel-actions.md`.
    pub fn choose_row(&mut self) -> bool {
        let Some(row) = self.pointed_row() else {
            return false;
        };
        self.choose_row_at(row)
    }

    /// Choose `row`, the row of a list by its panel and its id, as [`Self::choose_row`] does the row pointed at: as the
    /// trigger of the hand whose ray is on it does. Says whether it chose.
    pub fn choose_row_at(&mut self, (panel, row): (String, String)) -> bool {
        let shown = self
            .projection()
            .and_then(|projection| projection.panel(&panel));
        let revision = shown
            .map(|shown| shown.version)
            .filter(|version| *version > 0);
        // The revision of the row's key as it was sent, so that the server hands its environment the key seen.
        let key = shown
            .and_then(|shown| shown.rows.iter().find(|listed| listed.id == row))
            .and_then(|listed| listed.key);
        if !self.send(ClientMessage::ChoosePanelRow {
            panel: panel.clone(),
            row: row.clone(),
            revision,
            key,
        }) {
            return false;
        }
        self.chosen_row = Some((panel, row, Instant::now()));
        true
    }

    /// Whether this participant chose the row `row` of the panel `panel` within [`CHOSEN_SHOWN`], which the label on the
    /// ray says.
    pub fn just_chose(&self, panel: &str, row: &str) -> bool {
        self.chosen_row()
            .is_some_and(|(chosen, id)| chosen == panel && id == row)
    }

    /// The row this participant chose within [`CHOSEN_SHOWN`], by its panel and its id, which its list draws chosen.
    pub fn chosen_row(&self) -> Option<(&str, &str)> {
        self.chosen_row
            .as_ref()
            .filter(|(.., at)| at.elapsed() < CHOSEN_SHOWN)
            .map(|(panel, row, _)| (panel.as_str(), row.as_str()))
    }

    /// Whether A would tick or untick a checklist item for this participant: one who takes part, as visitors only watch.
    pub fn ticks(&self) -> bool {
        self.role.participates()
    }

    /// Tick, or untick, as `checked` says, the checklist item that begins on `line` of the panel `panel`, which
    /// [`Self::item_checked`] found shown, for everyone in the space, showing it so at once, until the server's projection
    /// says how it is. It says the panel's version the server last sent, so that the server refuses the tick where the
    /// panel has changed since, rather than tick another item. It never counts its own tick in that version: a guess at
    /// the version the tick makes could equal the version of another change, such as the agent's rewrite the tick
    /// raced, and so let a second tick land on another item; a second tick before the projection comes back is refused
    /// instead, and ticked again. Says whether it asked, as only a participant does; a visitor is told they only watch.
    /// See `design/projection.md`.
    fn check_item(&mut self, panel: &str, line: u32, checked: bool) -> bool {
        let revision = self
            .projection()
            .and_then(|projection| projection.panel(panel))
            .map(|shown| shown.version)
            // An older server keeps no versions, and its ticks go by their line alone.
            .filter(|version| *version > 0);
        if !self.send(ClientMessage::CheckPanelItem {
            panel: panel.to_owned(),
            line,
            checked,
            revision,
        }) {
            return false;
        }
        self.projection.tick(
            panel,
            usize::try_from(line).expect("a line of a body shown is a usize"),
            checked,
        );
        true
    }

    /// The notice shown, for the frame step's tests.
    #[cfg(test)]
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// The worker the participant points at now, as the console shows it.
    #[cfg(not(target_os = "android"))]
    pub fn pointing_at(&self) -> Option<&str> {
        match &self.pointing_at {
            Some(tomori_protocol::Resource::Worker { worker, .. }) => Some(worker),
            _ => None,
        }
    }

    /// Whether the joined space has `target`, a worker or a card.
    fn knows(&self, target: &tomori_protocol::Resource) -> bool {
        self.about(target).is_some()
    }

    /// What `target` is called, as the recording indicator and status line name it: a worker's title, or a card's
    /// number and title. `None` when the joined space does not have it, or for a kind this client does not show.
    fn about(&self, target: &tomori_protocol::Resource) -> Option<String> {
        use tomori_protocol::Resource;
        match target {
            Resource::Worker { worker, .. } => self
                .joined_space()
                .and_then(|(space, _)| space.workers.iter().find(|known| &known.id == worker))
                .map(|worker| worker.title.clone()),
            Resource::Card { card, .. } => self
                .cards()
                .and_then(|cards| cards.cards.iter().find(|known| &known.id == card))
                .map(|card| format!("#{} {}", card.number, card.title)),
            Resource::Object { object, .. } => self
                .joined_space()
                .and_then(|_| self.object_labels.get(object).cloned()),
            Resource::Panel { panel, .. } => self
                .projection()
                .and_then(|projection| projection.panel(panel))
                .map(crate::projection::name),
            // A cluster's "+N more" sign, by the anchor it stands at.
            Resource::Gathered { gathered, .. } => self
                .joined_space()
                .map(|_| format!("the workers gathered at {gathered}")),
            // A timer's dial, by its name. See `design/timers.md`.
            Resource::Timer { timer, .. } => self
                .joined_space()
                .filter(|(space, _)| space.timers.iter().any(|known| &known.name == timer))
                .map(|_| format!("timer {timer}")),
            _ => None,
        }
    }

    /// The images the panels projected in the joined space show, each once.
    fn shown_images(&self) -> Vec<tomori_protocol::Image> {
        self.projection()
            .map(|projection| {
                crate::images::shown(projection)
                    .into_iter()
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The images of image panels held, to draw. See `design/projection.md`.
    pub fn images(&self) -> &crate::images::Images {
        &self.images
    }

    /// What is projected in the joined space, while the server projects there. See `design/projection.md`.
    pub fn projection(&self) -> Option<&tomori_protocol::Projection> {
        self.projection.projection().filter(|projection| {
            self.joined_space()
                .is_some_and(|(space, _)| space.id == projection.space && space.projection)
        })
    }

    /// Where the participant's head is now, in the space's scene, as the frame step says each frame.
    pub fn observe_head(&mut self, head: Option<tomori_protocol::Pose>) {
        self.head = head;
    }

    /// Put the panel `panel` onto a wall's layout, or leave it floating, as `target` says, for everyone in the space.
    /// Only a participant can.
    pub fn place_panel(&mut self, panel: String, target: crate::projection::Target) {
        if self.projection().is_none() {
            return;
        }
        let (wall, slot, pose, sized) = match target {
            crate::projection::Target::Wall(wall, slot) => (Some(wall), Some(slot), None, None),
            crate::projection::Target::Free(pose) => (None, None, Some(pose), None),
            crate::projection::Target::Sized(pose, sized) => (None, None, Some(pose), Some(sized)),
        };
        // Stretched or shrunk with both hands, it says how large, and the version it was taken hold of at, so that the
        // server refuses it where the panel has changed since.
        self.send(ClientMessage::PlacePanel {
            panel,
            wall,
            slot,
            pose,
            size: sized.map(|sized| sized.size),
            revision: sized.map(|sized| sized.version),
        });
    }

    /// Dismiss the panel `panel`, which the joined space shows, for everyone in it. Says whether it asked, as only a
    /// participant does.
    pub fn dismiss_panel(&mut self, panel: &str) -> bool {
        if self
            .projection()
            .is_none_or(|projection| projection.panel(panel).is_none())
        {
            return false;
        }
        self.send(ClientMessage::DismissPanel {
            panel: panel.to_owned(),
        })
    }

    /// Whether this participant manages the joined space's walls: an owner, in a space the server projects in.
    fn manages_walls(&self) -> bool {
        self.role == ConnectionRole::Owner && self.projection().is_some()
    }

    /// What this participant may do with the joined space's walls on the headset: adjust them, as a participant in a space
    /// whose server takes wall changes, and remove one someone added, as an owner there too; and, where the server locks
    /// walls, see each one's lock, and lock and unlock them as an owner. See `design/projection.md`.
    pub fn wall_rights(&self) -> crate::projection::editing::Rights {
        let adjust = self.role.participates()
            && self.projection().is_some()
            && self.joined_space().is_some_and(|(space, _)| space.walls);
        let locks = adjust
            && self
                .joined_space()
                .is_some_and(|(space, _)| space.wall_locks);
        crate::projection::editing::Rights {
            adjust,
            remove: adjust && self.manages_walls(),
            snap: adjust && self.joined_space().is_some_and(|(space, _)| space.groups),
            locks,
            lock: locks && self.manages_walls(),
        }
    }

    /// Lock or unlock a wall, or put an environment's own back, as `change` says, for everyone in the space. Says whether
    /// it asked, as only an owner does, in a space whose server locks walls.
    pub fn lock_wall(&mut self, change: crate::projection::editing::LockChange) -> bool {
        if !self.wall_rights().lock {
            return false;
        }
        self.send(ClientMessage::LockWall {
            wall: change.wall,
            locked: change.locked,
            reset: change.reset,
        })
    }

    /// Change a wall as `change` says, for everyone in the space. Says whether it asked, as only a participant does, in a
    /// space whose server takes wall changes.
    pub fn set_wall(&mut self, change: crate::projection::editing::Change) -> bool {
        use crate::projection::editing::Joining;
        let rights = self.wall_rights();
        if !rights.adjust {
            return false;
        }
        // Snapping, or taking a wall out of its group, only where the server snaps walls.
        match change.join {
            Some(_) if !rights.snap => return false,
            Some(Joining::Attach { attach, alone }) => {
                return self.send(ClientMessage::AttachWall {
                    wall: change.wall,
                    attach: Some(attach),
                    alone,
                    pose: None,
                });
            }
            Some(Joining::Leave) => {
                return self.send(ClientMessage::AttachWall {
                    wall: change.wall,
                    attach: None,
                    alone: true,
                    pose: change.pose,
                });
            }
            None => {}
        }
        self.send(ClientMessage::SetWall {
            wall: change.wall,
            grid: change.grid,
            scale: change.scale,
            aspect: change.aspect,
            shape: change.shape,
            pose: change.pose,
            tilt: change.tilt,
            content_scale: change.content_scale,
            // The version of the wall as it was taken hold of, so that a change made against it never undoes another
            // made meanwhile; an older server keeps none.
            revision: change.version.filter(|version| *version > 0),
        })
    }

    /// Remove the wall `wall`, one someone added, for everyone, as an owner; its panels float where they were. Says
    /// whether it asked.
    pub fn remove_wall_by_id(&mut self, wall: String) -> bool {
        if !self.wall_rights().remove {
            return false;
        }
        self.send(ClientMessage::RemoveWall { wall })
    }

    /// The added wall nearest the participant's head, if there is one and they manage the walls.
    fn nearest_added_wall(&self) -> Option<&tomori_protocol::Wall> {
        let head = glam::Vec3::from_array(self.head?.position);
        self.projection()
            .filter(|_| self.manages_walls())?
            .walls
            .iter()
            .filter(|wall| wall.kind == tomori_protocol::WallKind::Added)
            .min_by(|a, b| {
                let distance = |wall: &tomori_protocol::Wall| {
                    glam::Vec3::from_array(wall.pose.position).distance(head)
                };
                distance(a).total_cmp(&distance(b))
            })
    }

    /// Add a wall for everyone where the participant stands, as an owner: ahead of them at eye height, facing them.
    fn add_wall(&mut self) {
        let Some(head) = self.head.filter(|_| self.manages_walls()) else {
            return;
        };
        let pose = crate::projection::added_wall(head);
        self.send(ClientMessage::AddWall { pose });
    }

    /// Remove, for everyone, the added wall nearest the participant, as an owner.
    fn remove_wall(&mut self) {
        if let Some(wall) = self.nearest_added_wall().map(|wall| wall.id.clone()) {
            self.send(ClientMessage::RemoveWall { wall });
        }
    }

    /// The cards of the joined space, while its environment shows them. See `design/github.md`.
    pub fn cards(&self) -> Option<&tomori_protocol::Cards> {
        self.cards.as_ref().filter(|cards| {
            self.joined_space()
                .is_some_and(|(space, _)| space.id == cards.space)
        })
    }

    /// Keep `update` of `environment` at `version`, if that is the joined space's environment as the space says.
    fn receive_environment(
        &mut self,
        environment: String,
        version: u32,
        update: crate::channel::Update,
    ) {
        if let Some(space) = self.space.as_ref().filter(|space| {
            space.environment == environment
                && space
                    .environment_version
                    .is_none_or(|named| named == version)
        }) {
            self.channel
                .receive(&space.id, &environment, version, update);
        }
    }

    /// What the server side of `environment` at `version` sent for the joined space since it was last taken, in order:
    /// its state, which replaces whatever was sent before, and its events. Nothing for another environment or version,
    /// or outside a space. See `design/environments.md`.
    pub fn environment_updates(
        &mut self,
        environment: &str,
        version: u32,
    ) -> Vec<crate::channel::Update> {
        match self.joined_space() {
            Some((space, _)) => {
                let space = space.id.clone();
                self.channel.take(&space, environment, version)
            }
            None => Vec::new(),
        }
    }

    /// Ask the server side of `environment` at `version`, the joined space's, for `intent`. Only a participant asks, of
    /// a server that has sent the environment's state in this space, so that it runs the side and takes intents; says
    /// whether it asked.
    pub fn environment_intent(
        &mut self,
        environment: &str,
        version: u32,
        intent: tomori_protocol::Payload,
    ) -> bool {
        let asking = self.role.participates()
            && self.joined_space().is_some_and(|(space, connected)| {
                connected && self.channel.has(&space.id, environment, version)
            });
        if asking {
            self.actions
                .push(Action::Send(ClientMessage::EnvironmentIntent {
                    environment: environment.to_owned(),
                    version,
                    intent,
                }));
        }
        asking
    }

    /// Put the card `card` at `placement`, or back in the inbox, for everyone in the space. Only a participant can.
    pub fn place_card(&mut self, card: String, placement: Option<tomori_protocol::Pose>) {
        if !self.role.participates() || self.cards().is_none() {
            return;
        }
        self.actions
            .push(Action::Send(ClientMessage::PlaceCard { card, placement }));
    }

    /// Ask to watch the joined space's cards, once for each space and connection, if it shows them and this
    /// participant takes part; forget them if it does not show them.
    fn watch_cards(&mut self) {
        let Some(space) = self.space.as_ref().filter(|_| self.mode == Mode::Space) else {
            return;
        };
        if !space.cards {
            self.cards = None;
            self.watching = None;
            return;
        }
        if self.role.participates() && self.watching.as_ref() != Some(&space.id) {
            self.watching = Some(space.id.clone());
            self.actions.push(Action::Send(ClientMessage::WatchCards));
        }
    }

    /// The presented question, when this participant can answer it by voice: Tomori supports it, and the server says it routes spoken answers. See `design/spaces.md`.
    fn answerable(&self) -> Option<&tomori_protocol::Question> {
        let space = self.space.as_ref().filter(|_| self.role.participates())?;
        space
            .question
            .as_ref()
            .filter(|question| question.supported && space.voice_answers)
    }

    /// Toggle an explicitly enabled microphone for the space, not the agent.
    pub fn toggle_live(&mut self) {
        if self.live {
            self.stop_live();
            return;
        }
        if matches!(self.talk, Some(Talk::Recording { .. })) {
            self.notice = Some("Release X before enabling live audio.".into());
            return;
        }
        if self.mode != Mode::Space || self.space.is_none() {
            self.notice = Some("Join a space to use live audio.".into());
            return;
        }
        if !self.role.participates() {
            self.notice = Some(VISITING.into());
            return;
        }
        if !matches!(self.state, Some(ConnectionState::Connected { .. })) {
            self.error = Some("Offline; cannot use live audio.".into());
            return;
        }
        self.notice = None;
        self.live = true;
        self.actions
            .push(Action::StartLive(self.capture_generation));
    }

    /// Stop live audio on focus loss, departure, failure, or explicit mute.
    pub fn stop_live(&mut self) {
        if self.live {
            self.live = false;
            let pending = self
                .actions
                .iter()
                .any(|action| matches!(action, Action::StartLive(_)));
            self.actions
                .retain(|action| !matches!(action, Action::StartLive(_)));
            if !pending {
                // Mute the live gate before a previously queued X release can
                // restore it, for example when focus is lost just afterward.
                let before_end = self
                    .actions
                    .iter()
                    .position(|action| matches!(action, Action::EndUtterance))
                    .unwrap_or(self.actions.len());
                self.actions.insert(before_end, Action::StopLive);
            }
        }
    }

    /// Abandon a recording without sending it, as when the session loses focus. Words already released are still transcribed.
    pub fn stop_talking(&mut self) {
        if let Some(Talk::Recording {
            utterance, naming, ..
        }) = self.talk.clone()
        {
            // A name not said yet is still to be said: X says it again.
            self.resume_naming(naming);
            let pending = self.actions.iter().any(
                |action| matches!(action, Action::BeginUtterance(begun) if begun.id == *utterance),
            );
            self.actions.retain(
                |action| !matches!(action, Action::BeginUtterance(begun) if begun.id == *utterance),
            );
            if !pending {
                self.actions.push(Action::CancelUtterance);
            }
            self.talk = None;
        }
    }

    /// The microphone could not record, so the recording is abandoned.
    pub fn microphone_failed(&mut self, error: impl Into<String>) {
        let naming = self.talk.as_ref().and_then(Talk::naming);
        // Words being recorded are lost with the capture.
        let recording = matches!(self.talk, Some(Talk::Recording { .. }));
        self.stop_live();
        // X may have been released while capture failed, before the queued end
        // was sent. Cancel instead of finalizing an incomplete utterance.
        // Only an utterance whose end has not been performed yet is lost; a
        // failure after that comes from the live capture reopened after it,
        // and the sent utterance is still transcribed.
        let mut cancelled = false;
        if let Some(Talk::Sending { utterance, .. }) = &self.talk
            && self
                .actions
                .iter()
                .any(|action| matches!(action, Action::EndUtterance))
        {
            cancelled = true;
            let pending = self.actions.iter().any(
                |action| matches!(action, Action::BeginUtterance(begun) if begun.id == *utterance),
            );
            self.actions.retain(|action| {
                !matches!(action, Action::EndUtterance)
                    && !matches!(action, Action::BeginUtterance(begun) if begun.id == *utterance)
            });
            if !pending {
                self.actions.push(Action::CancelUtterance);
            }
        }
        self.stop_talking();
        if cancelled {
            self.talk = None;
        }
        self.failed(error.into(), naming);
        // Words already with the server are still transcribed, and the live microphone failing loses none, so what becomes of the next words does not answer it.
        if !recording && !cancelled {
            self.unsent = None;
        }
    }

    /// Whether the next press of X says a new name, `Naming::Name`, or the space's title, `Naming::Title`, taking it, so
    /// that the press after says words for the agent again; `None` when it says neither, or the participant can no
    /// longer say the one chosen here.
    fn take_naming(&mut self) -> Option<Naming> {
        let naming = std::mem::take(&mut self.naming) && self.renames();
        let titling = std::mem::take(&mut self.titling) && self.retitles();
        if naming {
            Some(Naming::Name)
        } else {
            titling.then_some(Naming::Title)
        }
    }

    /// Words that were to say `naming` were not said after all, so the next press of X says it again, where it still can.
    fn resume_naming(&mut self, naming: Option<Naming>) {
        match naming {
            Some(Naming::Name) => self.naming = self.renames(),
            Some(Naming::Title) => self.titling = self.retitles(),
            None => {}
        }
    }

    /// Show `error` as the problem, and on the recording indicator, as words that were not sent, or, if they were `naming` the participant or the space, as a name not changed, which X then says again. See `design/hand-menu.md`.
    fn failed(&mut self, error: String, naming: Option<Naming>) {
        let problem = if let Some(naming) = naming {
            self.resume_naming(Some(naming));
            let problem = name_not_changed(naming, &error);
            self.outcome = Some((Outcome::NameFailed(naming, error), Instant::now()));
            problem
        } else {
            self.outcome = Some((Outcome::Failed(error.clone()), Instant::now()));
            error
        };
        // Unless later words are on their way, the recording card shows it where the words were spoken, with the not-sent cue, so the right wrist does not cue it too.
        if self.talk.is_none() {
            self.problem_noticed = Some(problem.clone());
        }
        // What becomes of the next words replaces it.
        self.unsent = Some(problem.clone());
        self.error = Some(problem);
    }

    /// What became of this participant's newest words replaces the problem saying that earlier words were not sent, which is no longer news.
    fn replace_unsent(&mut self) {
        if let Some(unsent) = self.unsent.take()
            && self.error.as_ref() == Some(&unsent)
        {
            self.error = None;
        }
    }

    /// What the recording indicator on the left controller says at `now`, if anything. See `design/hand-menu.md`.
    pub fn recording_phase(&self, now: Instant) -> Option<crate::recording::Phase> {
        use crate::recording::Phase;
        match &self.talk {
            Some(Talk::Recording {
                started,
                heard,
                naming: Some(what),
                ..
            }) => {
                return Some(Phase::Naming {
                    what: *what,
                    elapsed: now.saturating_duration_since(*started),
                    heard: heard.clone(),
                });
            }
            Some(Talk::Sending {
                heard,
                naming: Some(what),
                ..
            }) => {
                return Some(Phase::Renaming {
                    what: *what,
                    heard: heard.clone(),
                });
            }
            Some(Talk::Recording {
                started,
                heard,
                answering,
                pointed,
                ..
            }) => {
                // The worker or card the words will be about, which an answer never is.
                let about = pointed
                    .as_ref()
                    .filter(|_| !answering)
                    .and_then(|pointed| self.about(pointed));
                return Some(Phase::Recording {
                    about,
                    elapsed: now.saturating_duration_since(*started),
                    heard: heard.clone(),
                    answering: answering
                        .then(|| {
                            self.space
                                .as_ref()
                                .and_then(|space| space.question.as_ref())
                                .map(|question| question.title.clone())
                        })
                        .flatten(),
                });
            }
            Some(Talk::Sending {
                heard,
                answering,
                received,
                ..
            }) => {
                return Some(Phase::Sending {
                    heard: heard.clone(),
                    answering: *answering,
                    received: *received,
                });
            }
            None => {}
        }
        match &self.outcome {
            Some((Outcome::Sent { answered, text }, at))
                if now.saturating_duration_since(*at) < crate::recording::OUTCOME =>
            {
                // Where the words are in the queue, by this participant's newest entry with them.
                let position = (!answered)
                    .then(|| {
                        let waiting: Vec<_> = self
                            .space
                            .as_ref()?
                            .queue
                            .iter()
                            .filter(|entry| {
                                matches!(entry.state, QueueState::Queued | QueueState::Sending)
                            })
                            .collect();
                        waiting
                            .iter()
                            .rposition(|entry| {
                                Some(entry.source) == self.source
                                    && entry.text.trim() == text.trim()
                            })
                            .map(|index| index + 1)
                    })
                    .flatten();
                Some(Phase::Sent {
                    answered: *answered,
                    position,
                })
            }
            Some((Outcome::Named(what, name), at))
                if now.saturating_duration_since(*at) < crate::recording::OUTCOME =>
            {
                Some(Phase::Named {
                    what: *what,
                    name: name.clone(),
                })
            }
            // For a few seconds, unless B dismisses the problem or another recording starts first.
            Some((Outcome::Failed(reason), at))
                if self.error.as_ref() == Some(reason)
                    && now.saturating_duration_since(*at) < crate::recording::NOT_SENT =>
            {
                Some(Phase::Failed {
                    reason: reason.clone(),
                })
            }
            Some((Outcome::Unheard, at)) if self.unheard_shown(now, *at) => Some(Phase::Failed {
                reason: tomori_protocol::NO_SPEECH.into(),
            }),
            Some((Outcome::NameFailed(what, reason), _))
                if self.error.as_deref() == Some(name_not_changed(*what, reason).as_str()) =>
            {
                Some(Phase::NameFailed {
                    what: *what,
                    reason: reason.clone(),
                })
            }
            _ => None,
        }
    }

    /// Whether the card still says, at `now`, that no speech was heard in words that ended `at`.
    fn unheard_shown(&self, now: Instant, at: Instant) -> bool {
        self.talk.is_none() && now.saturating_duration_since(at) < crate::recording::NOT_SENT
    }

    /// The longest utterance the server's recognition takes, when it says.
    pub fn utterance_limit(&self) -> Option<Duration> {
        self.utterance_limit
    }

    /// Whether the live microphone is on, which the `LIVE` badge on the left controller shows.
    pub fn live(&self) -> bool {
        self.live
    }

    /// Ending the utterance failed, so its words were not sent: stop showing
    /// them as on their way, and report why.
    pub fn utterance_not_sent(&mut self, error: impl Into<String>) {
        // A later utterance may already be queued behind this end: cancel it
        // as any capture failure does, before clearing what is on its way.
        self.microphone_failed(error);
        if matches!(self.talk, Some(Talk::Sending { .. })) {
            self.talk = None;
        }
    }

    /// How long X has been held, while recording.
    pub fn recording(&self) -> Option<Duration> {
        match &self.talk {
            Some(Talk::Recording { started, .. }) => Some(started.elapsed()),
            _ => None,
        }
    }

    /// The joined space, and whether its state is current: not while the connection is down, when the server may have moved on without us.
    /// This participant's number on the connected server, which the others in the space know them by.
    pub fn source(&self) -> Option<u32> {
        self.source
    }

    /// The right hands of the others in the joined space, by their numbers, as their presence last said, in the space's
    /// coordinates: only those the space's state still lists, and never this participant's own.
    pub fn right_hands(&self) -> Vec<(u32, tomori_protocol::Pose)> {
        self.others(&self.right_hands)
    }

    /// The others' left hands, likewise.
    pub fn left_hands(&self) -> Vec<(u32, tomori_protocol::Pose)> {
        self.others(&self.left_hands)
    }

    /// How many times the others' hands have been forgotten, on connecting and on leaving or joining a space: once it
    /// changes, a number may be someone else's, so what was drawn of their hands is forgotten too.
    pub fn hands_forgotten(&self) -> u64 {
        self.hands_forgotten
    }

    fn forget_hands(&mut self) {
        self.right_hands.clear();
        self.left_hands.clear();
        self.hands_forgotten += 1;
    }

    fn others(
        &self,
        hands: &std::collections::BTreeMap<u32, tomori_protocol::Pose>,
    ) -> Vec<(u32, tomori_protocol::Pose)> {
        let Some((space, _)) = self.joined_space() else {
            return Vec::new();
        };
        hands
            .iter()
            .filter(|(source, _)| {
                Some(**source) != self.source
                    && space
                        .participants
                        .iter()
                        .any(|participant| participant.source == **source)
            })
            .map(|(source, pose)| (*source, *pose))
            .collect()
    }

    pub fn joined_space(&self) -> Option<(&SpaceState, bool)> {
        let space = self.space.as_ref().filter(|_| self.mode == Mode::Space)?;
        Some((
            space,
            matches!(self.state, Some(ConnectionState::Connected { .. })),
        ))
    }

    /// What the swarm should show. `Unknown` and a lost connection are never shown as ready.
    pub fn activity(&self) -> SessionActivity {
        let Some(space) = self.space.as_ref().filter(|_| self.mode == Mode::Space) else {
            return SessionActivity::Unselected;
        };
        if !matches!(self.state, Some(ConnectionState::Connected { .. })) {
            return SessionActivity::Unavailable;
        }
        if self.decided.is_some() {
            return SessionActivity::Working;
        }
        if space.approval.is_some() || space.question.is_some() {
            return SessionActivity::Waiting;
        }
        match space.activity {
            Activity::Idle
                if space.queue.iter().any(|entry| {
                    matches!(entry.state, QueueState::Queued | QueueState::Sending)
                }) =>
            {
                SessionActivity::Working
            }
            Activity::Idle => SessionActivity::Ready,
            Activity::Running => SessionActivity::Working,
            Activity::Waiting => SessionActivity::Waiting,
            Activity::Closed => SessionActivity::Unavailable,
            Activity::Unknown => SessionActivity::Unknown,
        }
    }

    /// What the headset shows as objects in the world, outside a space. See `design/arrival.md`.
    pub fn arrival(&self) -> Option<arrival::View> {
        match self.mode {
            Mode::Space => None,
            Mode::Servers => Some(arrival::View {
                heading: "Choose a server".into(),
                status: Some(self.servers_status().into()),
                connected: None,
                items: self
                    .servers
                    .iter()
                    .enumerate()
                    .flat_map(|(index, choice)| {
                        let attention = choice.attention();
                        let server = arrival::Item {
                            key: arrival::Key::Server(choice.address.clone()),
                            kind: arrival::Kind::Server {
                                source: choice.source,
                                available: choice.is_available(),
                                attention,
                            },
                            label: choice.label.clone(),
                            detail: Some(choice.detail()),
                            selected: index == self.server,
                        };
                        // A server whose spaces need attention has a jump object beside it.
                        let jump = attention.map(|attention| arrival::Item {
                            key: arrival::Key::Jump(choice.address.clone()),
                            kind: arrival::Kind::Jump { attention },
                            label: "Jump".into(),
                            detail: Some(format!("join what needs you on {}", choice.label)),
                            selected: false,
                        });
                        std::iter::once(server).chain(jump)
                    })
                    .collect(),
                ring: None,
                joining: None,
            }),
            Mode::Lobby => {
                let server = self
                    .connected
                    .as_ref()
                    .map_or("server", |choice| choice.label.as_str());
                let ring = self.step.ring();
                let spaces = self.step == LobbyStep::Spaces;
                let status = self.status().or_else(|| self.ended.clone()).or_else(|| {
                    if self.joining.is_some() {
                        Some("Joining...".into())
                    } else if self.role == ConnectionRole::Pairing {
                        Some("This server admits only paired devices: pair to join.".into())
                    } else if !self.listed {
                        Some("Listing spaces...".into())
                    } else if !spaces {
                        Some(self.ring_hint(self.step).into())
                    } else if self.spaces.is_empty() && self.creates() {
                        Some("No spaces yet; create one.".into())
                    } else if self.spaces.is_empty() {
                        Some("No spaces yet.".into())
                    } else {
                        self.attention_status()
                    }
                });
                let items = self
                    .spaces
                    .iter()
                    .enumerate()
                    .map(|(index, space)| arrival::Item {
                        key: arrival::Key::Space(space.id.clone()),
                        kind: arrival::Kind::Space {
                            activity: space.activity,
                            participants: space.participants,
                            hue: arrival::hue(&space.id),
                            attention: space.attention,
                        },
                        label: space.title.clone(),
                        detail: Some(self.space_detail(space)),
                        selected: spaces && index == self.selected,
                    })
                    .chain(self.creates().then(|| arrival::Item {
                        key: arrival::Key::Create,
                        kind: arrival::Kind::Create { open: !spaces },
                        label: if spaces { "New space" } else { "Back" }.into(),
                        detail: None,
                        selected: spaces && self.selected >= self.spaces.len(),
                    }))
                    .chain(self.pairable().then(|| self.pairing_item()))
                    .collect();
                Some(arrival::View {
                    heading: match ring {
                        Some(step) => step.title().into(),
                        None => format!("Spaces on {server}"),
                    },
                    status,
                    connected: self.connected.as_ref().map(|choice| arrival::Item {
                        key: arrival::Key::Connected(choice.address.clone()),
                        kind: arrival::Kind::Connected,
                        label: choice.label.clone(),
                        detail: Some("Choose to change".into()),
                        selected: false,
                    }),
                    items,
                    ring: ring.map(|step| arrival::Ring {
                        step,
                        items: self.ring_items(self.step),
                    }),
                    joining: self
                        .joining
                        .as_ref()
                        .map(|space| arrival::Key::Space(space.clone())),
                })
            }
        }
    }

    /// The space joined, as the arrival object it was chosen from, so that its environment grows out of that object.
    pub fn entered(&self) -> Option<arrival::Key> {
        self.space
            .as_ref()
            .filter(|_| self.mode == Mode::Space)
            .map(|space| arrival::Key::Space(space.id.clone()))
    }

    /// Choose an arrival object, as by pointing at it and pulling the trigger.
    pub fn choose_object(&mut self, key: &arrival::Key) {
        match (self.mode, key) {
            // Connect, then join the space that most needs attention once the server lists them.
            (Mode::Servers, arrival::Key::Jump(address)) => {
                if let Some(index) = self
                    .servers
                    .iter()
                    .position(|choice| &choice.address == address && choice.is_available())
                {
                    self.server = index;
                    self.chosen = true;
                    let choice = self.servers[index].clone();
                    let label = choice.label.clone();
                    self.connect(choice);
                    self.jumping = true;
                    self.notice = Some(format!("Jumping to what needs you on {label}..."));
                }
            }
            (Mode::Servers, arrival::Key::Server(address)) => {
                if let Some(index) = self
                    .servers
                    .iter()
                    .position(|choice| &choice.address == address)
                {
                    self.server = index;
                    self.chosen = true;
                    self.missing = None;
                    self.accept();
                }
            }
            (Mode::Lobby, arrival::Key::Connected(_)) => self.leave_server(),
            (Mode::Lobby, arrival::Key::Space(id)) => {
                if let Some(index) = self.spaces.iter().position(|space| &space.id == id) {
                    self.close_menu();
                    self.selected = index;
                    self.accept();
                }
            }
            (Mode::Lobby, arrival::Key::Pair) => self.pair(),
            (Mode::Lobby, arrival::Key::Create) if !self.creates() => {}
            (Mode::Lobby, arrival::Key::Create) => {
                self.error = None;
                if self.step == LobbyStep::Spaces {
                    self.selected = self.spaces.len();
                    self.accept();
                } else {
                    self.back_ring();
                }
            }
            (Mode::Lobby, key) => {
                let position = self
                    .ring_items(self.step)
                    .iter()
                    .position(|item| &item.key == key);
                if let Some(index) = position {
                    self.error = None;
                    self.menu = index;
                    self.accept();
                }
            }
            _ => {}
        }
    }
}

/// What an entry of the space's list is called.
#[cfg(any(not(target_os = "android"), test))]
fn menu_label(item: &MenuItem) -> String {
    match item {
        MenuItem::Mode(mode) => format!("Mode: {}", mode.name),
        MenuItem::Speech(SpeechSetting::Brief) => "Speech: brief".into(),
        MenuItem::Speech(_) => "Speech: full".into(),
    }
}

/// What a participant in a space is told when its title changes to the one `state` has.
fn retitled(state: &SpaceState) -> String {
    match &state.renamed_by {
        Some(by) => format!("{} renamed this space to “{}”.", renamer(by), state.title),
        None => format!("This space is now called “{}”.", state.title),
    }
}

/// Who renamed a space, as the hand menu names them: the agent as Tomori, which is how people in a space speak of it.
fn renamer(renamer: &tomori_protocol::Renamer) -> String {
    match renamer {
        tomori_protocol::Renamer::Agent => "Tomori".into(),
        tomori_protocol::Renamer::Participant { name } => name.clone(),
        tomori_protocol::Renamer::Unknown => "someone".into(),
    }
}

/// How long ago something happened, `seconds` since, as the lobby says it: "just now" within a minute, and then in
/// minutes, hours or days.
fn ago(seconds: u32) -> String {
    match seconds {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", seconds / 60),
        3600..86_400 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86_400),
    }
}

/// How long ago something happened, briefly.
fn age(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m", seconds / 60),
        _ => format!("{}h", seconds / 3600),
    }
}

/// A space chosen on the creation rings, not yet bound to an environment.
#[derive(Debug)]
struct Creation {
    title: String,
    backend: String,
    directory: String,
    session: Option<String>,
    instructions: Option<String>,
}

impl Creation {
    /// The `CreateSpace` for this space in `environment`, or in the server's own when `None`.
    fn message(self, environment: Option<String>) -> ClientMessage {
        ClientMessage::CreateSpace {
            title: self.title,
            backend: self.backend,
            directory: self.directory,
            session: self.session,
            environment,
            instructions: self.instructions,
        }
    }
}

/// An entry of the space's list, opened with Y.
#[derive(Clone, Debug, PartialEq)]
enum MenuItem {
    Mode(tomori_protocol::Mode),
    Speech(SpeechSetting),
}

/// The messages, each of which the stick can select.
fn items(space: &SpaceState) -> usize {
    space.messages.len()
}

/// How many participants are in the space, and how many of them are away.
fn here(space: &SpaceState) -> String {
    let away = space
        .participants
        .iter()
        .filter(|participant| participant.engagement == Some(Engagement::Away))
        .count();
    match away {
        0 => format!("{} here", space.participants.len()),
        away => format!("{} here, {away} away", space.participants.len() - away),
    }
}

fn activity_label(activity: Activity) -> &'static str {
    match activity {
        Activity::Idle => "idle",
        Activity::Running => "running",
        Activity::Waiting => "waiting",
        Activity::Closed => "closed",
        Activity::Unknown => "unknown",
    }
}

fn queue_label(state: QueueState) -> &'static str {
    match state {
        QueueState::Queued => "queued",
        QueueState::Sending => "sending",
        QueueState::Delivered => "sent",
        QueueState::Failed => "failed",
    }
}

/// The first of `rows` visible entries that keeps `selected` in view.
fn window(selected: usize, count: usize, rows: usize) -> usize {
    selected
        .saturating_sub(rows - 1)
        .min(count.saturating_sub(rows))
}

/// The end of `text` within `width` columns, so that words still being heard stay in view.
#[cfg(any(not(target_os = "android"), test))]
fn tail(text: &str, width: usize) -> String {
    let line = crate::ui_text::clip(text, usize::MAX);
    if crate::ui_text::width(&line) <= width {
        return line;
    }
    // As many characters from the end as fit beside the "…", counted in cells as drawn.
    let mut kept = Vec::new();
    let mut used = 1;
    for ch in line.chars().rev() {
        used += crate::ui_text::width(ch.encode_utf8(&mut [0; 4]));
        if used > width {
            break;
        }
        kept.push(ch);
    }
    std::iter::once('…').chain(kept.into_iter().rev()).collect()
}

/// What tells an existing session apart beneath its title on the session ring: whether the agent is busy in it, and
/// its directory relative to the folder chosen, unless it is that folder.
fn session_detail(session: &SessionSummary, folder: &str) -> Option<String> {
    let relative = std::path::Path::new(&session.directory)
        .strip_prefix(folder)
        .ok()
        .map(|relative| relative.to_string_lossy().into_owned())
        .unwrap_or_else(|| session.directory.clone());
    match (session.busy, relative.is_empty()) {
        (true, true) => Some("busy".into()),
        (true, false) => Some(format!("busy, {relative}")),
        (false, true) => None,
        (false, false) => Some(relative),
    }
}

/// `text` on one row of the panel, as [`crate::ui_text::clip`] cuts it.
fn short(text: &str) -> String {
    crate::ui_text::clip(text, COLUMNS)
}

fn shift(index: usize, delta: i32, count: usize) -> usize {
    index
        .saturating_add_signed(delta as isize)
        .min(count.saturating_sub(1))
}

/// `text` paged at the body's full height, for tests.
#[cfg(test)]
fn pages(text: &str) -> Vec<Vec<String>> {
    paginate(text, BODY_ROWS)
}

fn paginate(text: &str, rows: usize) -> Vec<Vec<String>> {
    let lines = wrapped_lines(text);
    if lines.is_empty() {
        vec![vec![String::new()]]
    } else {
        lines.chunks(rows).map(|page| page.to_vec()).collect()
    }
}

/// The conversation is the session its connection carries out: see `design/clients.md`.
impl Session for Conversation {
    fn next_action(&mut self) -> Option<Action> {
        Conversation::next_action(self)
    }

    fn apply(&mut self, event: Event) {
        Conversation::apply(self, event);
    }

    fn environments(&self) -> Option<Vec<tomori_protocol::EnvironmentVersion>> {
        Some(self.presented())
    }

    /// The conversation applies patches to the projection it holds. See `design/projection.md`.
    fn projection_patches(&self) -> bool {
        true
    }

    /// What this client reads and draws at most: [`HEADSET_LIMITS`] unless [`Conversation::set_limits`] says otherwise.
    fn limits(&self) -> tomori_protocol::Limits {
        self.limits
    }

    /// And their data ops, to its live panels, with the shared rule, keeping each panel's version as a place op says it.
    fn live_panels(&self) -> bool {
        true
    }

    fn report(&mut self, report: Report) {
        // Reported every frame, and needing nothing more of the match below.
        if let Report::Replaying(replaying) = report {
            self.set_replaying(replaying);
        }
        if let Report::Tick { now, .. } = report {
            self.turn.tick(now);
        }
        if let Report::AgentSpeech { playing, level } = report {
            self.speech = SpeechActivity {
                playing,
                level,
                ..SpeechActivity::default()
            };
            self.hear(playing);
        }
        match report {
            Report::Connecting(choice) => self.connect(choice),
            Report::Feedback(settings) => self.set_feedback(settings),
            Report::Unfocused => {
                self.stop_live();
                self.stop_talking();
            }
            Report::Servers(servers) => self.set_servers(servers),
            Report::Tick { now, browsing } => {
                if browsing {
                    self.tick(now);
                }
                self.expire_pairing(now);
            }
            Report::Generation(generation) => self.set_capture_generation(generation),
            Report::Failed(error) => self.fail(error),
            Report::MicrophoneFailed(error) => self.microphone_failed(error),
            Report::NotSent(error) => self.utterance_not_sent(error),
            Report::SecurityNotice(notice) => self.security_notice(notice),
            // Replaying and agent speech, applied above, and anything a later connection reports that this conversation does not know, which needs nothing of it.
            _ => {}
        }
    }

    fn is_browsing(&self) -> bool {
        Conversation::is_browsing(self)
    }

    /// The approval and question waiting in the joined space, the knocks on it, and the recording, nearing the server's limit. Visitors cannot decide requests or let anyone in, so nothing waits for them.
    fn observe(&self, feedback: &mut feedback::Feedback, now: Instant) {
        feedback.observe_recording(self.recording(), self.utterance_limit(), now);
        feedback.observe_turn(self.turn.turn(), now);
        let space = self
            .joined_space()
            .map(|(space, _)| space)
            .filter(|_| self.role().participates());
        let requests = space.into_iter().flat_map(|space| {
            let approval = space.approval.as_ref().map(|approval| approval.id.as_str());
            let question = space.question.as_ref().map(|question| question.id.as_str());
            approval.into_iter().chain(question)
        });
        feedback.observe_requests(requests, now);
        let knocks = space
            .into_iter()
            .flat_map(|_| self.knocks().iter().map(|knock| knock.id.as_str()));
        feedback.observe_knocks(knocks, now);
    }
}

#[cfg(test)]
mod tests;
