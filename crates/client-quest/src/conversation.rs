// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The panel's conversation with a Tomori server: choosing a server, the lobby, then the joined space.
//!
//! This is pure state. It applies [`client::Event`]s and controller input, and queues [`Action`]s for [`crate::connection::Connection`] to carry out, so nothing here touches the network.
use crate::{
    arrival,
    caption::{COLUMNS, ROWS, wrapped_lines},
    environment::Environment,
    interaction::SessionActivity,
    server_choice::{self, Choice},
};
use client::{ConnectionState, Event, UtteranceFailure};
use protocol::{
    Activity, BackendInfo, ClientMessage, ConnectionRole, Decision, EnvironmentInfo, QueueState,
    Role, ServerMessage, SessionSummary, SpaceState, SpaceSummary, WorkspaceRoot,
};
use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const BODY_ROWS: usize = ROWS - 4;

/// Said when a visitor tries to talk or act in the space they are watching. See `design/security.md`.
const VISITING: &str = "Visitors can watch and listen, but not talk or act here.";

const LOST: &str = "Connection lost; your words were not sent.";

/// What the conversation needs done with the connection.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Connect to this server, closing any current connection.
    Connect(Choice),
    /// Close the connection.
    Disconnect,
    /// Forget the servers found so far and browse again.
    Rescan,
    Send(ClientMessage),
    /// Open the microphone and send what it hears to the agent as this utterance.
    BeginUtterance(String, u64),
    /// Close the microphone and finish the utterance, for the server to transcribe.
    EndUtterance,
    /// Close the microphone and abandon the utterance.
    CancelUtterance,
    /// Close the microphone and forget the utterance, which the server has already ended.
    AbandonUtterance,
    /// Open or close the microphone for other participants without addressing the agent.
    StartLive(u64),
    StopLive,
    /// Pairing gave this device an owner token for the server, to keep with the certificate the pairing connection presented, and connect with. See `design/security.md`.
    Paired {
        token: String,
        certificate: client::Fingerprint,
    },
}

/// Pairing this device as an owner, from choosing the pairing object until the server confirms or refuses it. See `design/security.md`.
#[derive(Clone, Debug, PartialEq)]
enum Pairing {
    /// Committed to this value, waiting for the server's.
    Committed(security::pairing::Nonce),
    /// Both sides have the code, written for reading, which waits to be confirmed on the server.
    Waiting(String),
}

/// Push-to-talk, from pressing X until the server's transcript or failure. See `design/speech.md`.
#[derive(Clone, Debug, PartialEq)]
enum Talk {
    /// X is held and the microphone is open, sending `agent` audio.
    Recording {
        utterance: String,
        started: Instant,
        /// The partial transcript so far.
        heard: String,
    },
    /// X was released; waiting for the server's final transcript.
    Sending { utterance: String, heard: String },
}

impl Talk {
    fn utterance(&self) -> &str {
        match self {
            Talk::Recording { utterance, .. } | Talk::Sending { utterance, .. } => utterance,
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
    /// Creating a space: choose the environment it is bound to. Only when the server offers more than one.
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

pub struct Conversation {
    /// This participant's number on the connected server, from its Welcome, which the server records on their queued input. Kept while the connection is down, since a reconnect keeps it; see `design/security.md`.
    source: Option<u32>,
    /// What this connection may do where it is now, from its Welcome and the role messages that follow. See `design/security.md`.
    role: ConnectionRole,
    /// The fingerprint of the server's certificate on this connection, over TLS, which the pairing code is derived from.
    certificate: Option<client::Fingerprint>,
    pairing: Option<Pairing>,
    name: String,
    mode: Mode,
    servers: Vec<Choice>,
    server: usize,
    /// Whether the stick moved the server selection; until then the best choice, listed first, stays selected as servers appear.
    chosen: bool,
    /// The address of the chosen server while it is missing from the list, as while browsing restarts, so that it is selected again when it reappears.
    missing: Option<String>,
    /// Whether the remembered server may still be connected to when it appears: only once, before the participant moves the selection or connects.
    automatic: bool,
    /// The server chosen in the server list, while in the lobby or a space.
    connected: Option<Choice>,
    state: Option<ConnectionState>,
    spaces: Vec<SpaceSummary>,
    listed: bool,
    /// The selected space; one past the last is the create object.
    selected: usize,
    joining: Option<String>,
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
    /// Shown in the lobby after a space you were in or joining has gone, until dismissed with B or another choice.
    ended: Option<String>,
    /// Which list the lobby shows: the spaces, or one of the creation rings.
    step: LobbyStep,
    /// The selection within the open creation ring.
    menu: usize,
    /// The server's backends, so the session ring is offered only for those that attach.
    backends: Vec<BackendInfo>,
    /// The directories a space may run in, offered so the client can choose without typing a path.
    roots: Vec<WorkspaceRoot>,
    /// The answer to [`ClientMessage::ListSessions`], shown on the session ring after "New session".
    sessions: Vec<SessionSummary>,
    /// The environments a space may be bound to, offered on the environment ring when there are several.
    environments: Vec<EnvironmentInfo>,
    /// The backend chosen on the backend ring, kept until the space is created.
    menu_backend: Option<String>,
    /// The directory chosen on the directory ring, kept until the space is created.
    menu_directory: Option<String>,
    /// The `CreateSpace` waiting at the environment step for its environment.
    menu_create: Option<ClientMessage>,
    error: Option<String>,
    actions: Vec<Action>,
    /// The last body paginated, since the panel is drawn every frame.
    paged: Option<Paged>,
    talk: Option<Talk>,
    live: bool,
    capture_generation: u64,
    /// Makes utterance identifiers unique across launches, since the server keys queued text by participant and utterance, and the participant persists.
    launch: String,
    utterances: u64,
}

/// A body's text, the rows it was paginated into, and its pages.
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
            role: ConnectionRole::Unknown,
            certificate: None,
            pairing: None,
            name: name.into(),
            mode: Mode::Servers,
            servers: Vec::new(),
            server: 0,
            chosen: false,
            missing: None,
            automatic: true,
            connected: None,
            state: None,
            spaces: Vec::new(),
            listed: false,
            selected: 0,
            joining: None,
            leaving: false,
            space: None,
            message: 0,
            page: 0,
            follow: true,
            request: None,
            option: 0,
            decided: None,
            notice: None,
            ended: None,
            step: LobbyStep::Spaces,
            menu: 0,
            backends: Vec::new(),
            roots: Vec::new(),
            environments: Vec::new(),
            sessions: Vec::new(),
            menu_backend: None,
            menu_directory: None,
            menu_create: None,
            error: None,
            actions: Vec::new(),
            paged: None,
            talk: None,
            live: false,
            capture_generation: 0,
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

    #[cfg(test)]
    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
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
        if self.automatic
            && !self.chosen
            && self.mode == Mode::Servers
            && let Some(choice) = server_choice::automatic(&self.servers).cloned()
        {
            self.connect(choice);
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
            Event::UtteranceFailed(failed) => {
                if self.talk.as_ref().map(Talk::utterance) == Some(failed.utterance.as_str()) {
                    self.stop_talking();
                    self.talk = None;
                    self.error = Some(match failed.failure {
                        UtteranceFailure::Disconnected => LOST.into(),
                        UtteranceFailure::Congested => {
                            "Network too slow; your words were not sent.".into()
                        }
                    });
                }
            }
        }
    }

    fn connection_changed(&mut self, state: ConnectionState) {
        if !matches!(state, ConnectionState::Connected { .. }) {
            // A pairing belongs to its connection; the server withdraws it.
            self.pairing = None;
            self.stop_live();
            match &self.talk {
                Some(Talk::Recording { .. }) => {
                    self.stop_talking();
                    self.error = Some(LOST.into());
                }
                // The server may still queue it, but its transcript cannot reach us.
                Some(Talk::Sending { .. }) => {
                    self.talk = None;
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
        } = &state
        {
            log::info!("Connected to {server} as {role:?}");
            self.source = Some(*source);
            self.role = *role;
            self.certificate = *certificate;
            self.error = None;
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
            ServerMessage::PairNonce { nonce } => self.pair_with(&nonce),
            ServerMessage::PairWaiting => {}
            ServerMessage::Paired { token } => {
                self.pairing = None;
                if let Some(certificate) = self.certificate {
                    self.actions.push(Action::Paired { token, certificate });
                    self.notice =
                        Some("Paired: this device is now an owner of this server.".into());
                }
            }
            ServerMessage::PairFailed { error } => {
                self.pairing = None;
                self.notice = Some(error);
            }
            ServerMessage::Role { role } => {
                log::info!("Now {role:?}");
                self.role = role;
                // A visitor sends no audio, so the microphone closes as the role changes.
                if !role.participates() {
                    self.stop_live();
                    self.stop_talking();
                }
            }
            ServerMessage::Spaces { spaces } => {
                let selected = self.spaces.get(self.selected).map(|space| &space.id);
                self.selected = if self.listed && self.selected == self.spaces.len() {
                    // The create object stays selected, after however many spaces there now are.
                    spaces.len()
                } else {
                    selected
                        .and_then(|id| spaces.iter().position(|space| &space.id == id))
                        .unwrap_or(0)
                };
                self.spaces = spaces;
                self.listed = true;
            }
            ServerMessage::Space { state } => self.update_space(*state),
            // The answer to a session list, shown on the session ring after "New session".
            ServerMessage::Sessions { sessions } => {
                if self.step == LobbyStep::Session {
                    self.sessions = sessions;
                    self.menu = self.menu.min(self.sessions.len());
                } else {
                    self.error = Some("Too late to show those sessions.".into());
                }
            }
            ServerMessage::Left => {
                let requested = std::mem::take(&mut self.leaving);
                if self.mode != Mode::Servers {
                    self.close_space();
                    self.mode = Mode::Lobby;
                    // Leaving on request is followed by a listing; otherwise ask for one.
                    if !requested {
                        self.list();
                    }
                }
            }
            ServerMessage::Error { error, space } => {
                log::warn!("Server error: {error}");
                if let Some(gone) = space.filter(|space| Some(space) == self.joining.as_ref()) {
                    // Our join was refused because the space does not exist, as after the server restarts. The client
                    // sends the membership it last recorded again after every reconnect, which would be this join, so
                    // record a leave in its place.
                    let title = self
                        .space
                        .as_ref()
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
                if let Some(talk) = self
                    .talk
                    .as_mut()
                    .filter(|talk| talk.utterance() == utterance)
                {
                    if finished {
                        // The queue entry now shows the words. An earlier utterance's failure stays shown.
                        self.talk = None;
                    } else {
                        let (Talk::Recording { heard, .. } | Talk::Sending { heard, .. }) = talk;
                        *heard = text;
                    }
                }
            }
            ServerMessage::TranscriptFailed { utterance, error } => {
                log::warn!("Utterance {utterance} failed: {error}");
                if self.talk.as_ref().map(Talk::utterance) == Some(utterance.as_str()) {
                    // Cancelling would be refused, since the server has already ended it.
                    if matches!(self.talk.take(), Some(Talk::Recording { .. })) {
                        self.actions.push(Action::AbandonUtterance);
                    }
                }
                // Also shown for an earlier utterance, whose words were not sent either.
                self.error = Some(error);
            }
            // Agent speech is heard rather than shown: its audio plays through the speaker.
            ServerMessage::Speech { segment, text, .. } => {
                log::info!("Agent speech {segment}: {text}");
            }
            // Presence comes with the presence work.
            ServerMessage::Welcome { .. }
            | ServerMessage::Presence { .. }
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
        if entered {
            log::info!("Joined space {} ({})", state.title, state.id);
            self.close_space();
            self.mode = Mode::Space;
        }
        self.joining = None;
        self.leaving = false;
        let request = state
            .approval
            .as_ref()
            .map(|approval| approval.id.clone())
            .or_else(|| state.question.as_ref().map(|question| question.id.clone()));
        if request != self.request {
            self.page = 0;
            self.option = 0;
            self.decided = None;
            self.request = request;
        }
        if let Some(approval) = &state.approval {
            self.option = self.option.min(approval.options.len());
        }
        let previous = self
            .space
            .as_ref()
            .and_then(|space| space.messages.get(self.message))
            .map(|message| message.id.clone());
        if self.follow {
            self.message = state.messages.len().saturating_sub(1);
            if self.request.is_none() {
                self.page = state
                    .messages
                    .last()
                    .map(|message| pages(&message.text).len() - 1)
                    .unwrap_or(0);
            }
        } else if let Some(index) =
            previous.and_then(|id| state.messages.iter().position(|message| message.id == id))
        {
            self.message = index;
        } else {
            self.message = self.message.min(items(&state).saturating_sub(1));
        }
        self.space = Some(state);
    }

    fn close_space(&mut self) {
        self.stop_live();
        self.stop_talking();
        self.space = None;
        self.joining = None;
        self.message = 0;
        self.page = 0;
        self.follow = true;
        self.request = None;
        self.option = 0;
        self.decided = None;
        self.notice = None;
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
            Some(Pairing::Waiting(code)) => (code.clone(), "Confirm on the server", true),
            Some(Pairing::Committed(_)) => ("Pairing...".to_owned(), "Choose to cancel", true),
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

    /// Whether this device can pair with the server: connected over TLS, and not already an owner. See `design/security.md`.
    pub fn pairable(&self) -> bool {
        self.certificate.is_some()
            && self.role != ConnectionRole::Owner
            && matches!(self.state, Some(ConnectionState::Connected { .. }))
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
        let device = security::pairing::Nonce::generate();
        self.actions.push(Action::Send(ClientMessage::PairBegin {
            commitment: device.commitment(),
        }));
        self.pairing = Some(Pairing::Committed(device));
        self.error = None;
        self.notice = Some("Pairing...".into());
    }

    /// The server sent its value: reveal ours, and derive the code from both and the certificate this connection presented.
    fn pair_with(&mut self, nonce: &str) {
        let (Some(Pairing::Committed(device)), Some(certificate), Some(server)) = (
            self.pairing.clone(),
            self.certificate,
            security::pairing::Nonce::parse(nonce),
        ) else {
            return;
        };
        self.actions.push(Action::Send(ClientMessage::PairReveal {
            nonce: device.to_hex(),
        }));
        let code =
            security::pairing::display(&security::pairing::code(certificate, device, server));
        self.notice = Some(format!("Pairing code {code}: confirm it on the server."));
        self.pairing = Some(Pairing::Waiting(code));
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
        );
        if acts && !self.role.participates() {
            self.notice = Some(VISITING.into());
            return false;
        }
        self.error = None;
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
            // "New session", then the existing sessions.
            LobbyStep::Session => self.sessions.len() + 1,
            LobbyStep::Environment => self.environments.len(),
        }
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
        self.menu_backend = None;
        self.menu_directory = None;
        self.menu_create = None;
    }

    /// Send `create`, bound to the only environment the server offers, or ask which one when it offers several. A server that offers none predates environments, and binds its own.
    fn submit(&mut self, create: ClientMessage) {
        self.joining = None;
        if self.environments.len() > 1 {
            self.menu = 0;
            self.step = LobbyStep::Environment;
            self.menu_create = Some(create);
            return;
        }
        let only = self.environments.first().map(|offered| offered.id.clone());
        self.send(with_environment(create, only));
        self.close_menu();
    }

    /// The scenery to draw: the Minimal grid outside a space, and inside one the environment it is bound to, or the grid when this build cannot draw that environment. See `design/environments.md`.
    pub fn environment(&self) -> Environment {
        match &self.space {
            Some(space) if self.mode == Mode::Space => {
                Environment::for_space(&space.environment).unwrap_or(Environment::Minimal)
            }
            _ => Environment::Minimal,
        }
    }

    /// The joined space's environment, when this build cannot draw it and shows the grid instead.
    fn missing_environment(&self) -> Option<&str> {
        self.space
            .as_ref()
            .filter(|space| Environment::for_space(&space.environment).is_none())
            .map(|space| space.environment.as_str())
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

    /// Create the space chosen on the rings, bound to `session` if one was chosen, then choose its environment if the server offers more than one.
    fn create(&mut self, session: Option<SessionSummary>) {
        let (Some(backend), Some(directory)) =
            (self.menu_backend.clone(), self.menu_directory.clone())
        else {
            return;
        };
        let title = match &session {
            Some(session) if !session.title.trim().is_empty() => session.title.clone(),
            _ => self.lobby_title(),
        };
        let create = ClientMessage::CreateSpace {
            title,
            backend,
            directory,
            session: session.map(|session| session.id),
            environment: None,
        };
        self.submit(create);
    }

    /// Return to the server list, closing the connection.
    fn leave_server(&mut self) {
        self.actions.push(Action::Disconnect);
        self.connected = None;
        self.state = None;
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
    fn approval(&self) -> Option<&protocol::Approval> {
        self.space
            .as_ref()
            .and_then(|space| space.approval.as_ref())
            .filter(|approval| self.decided.as_ref() != Some(&approval.id))
    }

    /// Whether the right stick chooses approval options rather than presentation cards.
    pub fn is_choosing(&self) -> bool {
        self.mode == Mode::Space
            && self
                .approval()
                .is_some_and(|approval| !approval.options.is_empty())
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
                let (count, messages) = (items(space), space.messages.len());
                if self.request.is_none() && horizontal != 0 {
                    self.message = shift(self.message, horizontal, count);
                    self.page = 0;
                }
                if vertical != 0 {
                    self.page = self.page.saturating_add_signed(vertical as isize);
                }
                let last_page = pages(&self.body().1).len() - 1;
                self.page = self.page.min(last_page);
                // Back at the newest page, follow new messages again.
                self.follow = self.request.is_none()
                    && self.message + 1 == messages.max(1)
                    && self.page == last_page;
            }
        }
    }

    /// Right stick left or right: choose among an approval's options.
    pub fn choose(&mut self, direction: i32) {
        if let Some(count) = self.approval().map(|approval| approval.options.len() + 1) {
            self.option = shift(self.option, direction, count);
        }
    }

    /// A while the ray highlights an arrival object chooses that object, as
    /// the trigger does, so that A never acts on a different item from the one
    /// pointed at. Otherwise A accepts as [`Self::accept`] does. Returns true
    /// when A has nothing to do here, so it can keep its swarm-reset meaning.
    pub fn press_a(&mut self, highlighted: Option<&arrival::Key>) -> bool {
        match highlighted {
            Some(key) if self.arrival().is_some() => {
                self.choose_object(key);
                false
            }
            _ => self.accept(),
        }
    }

    /// A: connect, join, or confirm the chosen approval option. Returns true when A has nothing to do here, so it can keep its swarm-reset meaning.
    pub fn accept(&mut self) -> bool {
        match self.mode {
            Mode::Servers => {
                match self.servers.get(self.server).cloned() {
                    Some(choice) if choice.is_available() => self.connect(choice),
                    Some(choice) => {
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
                            self.joining = Some(space.clone());
                            self.leaving = false;
                            self.error = None;
                            self.ended = None;
                            self.send(ClientMessage::Join { space });
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
                        let session = self
                            .menu
                            .checked_sub(1)
                            .and_then(|index| self.sessions.get(index))
                            .cloned();
                        self.create(session);
                    }
                    LobbyStep::Environment => {
                        let environment = self
                            .environments
                            .get(self.menu)
                            .map(|offered| offered.id.clone());
                        if let (Some(environment), Some(create)) =
                            (environment, self.menu_create.take())
                        {
                            self.send(with_environment(create, Some(environment)));
                            self.close_menu();
                        }
                    }
                }
                false
            }
            Mode::Space => {
                let Some(approval) = self.approval() else {
                    // A decision on its way still owns the button.
                    return self
                        .space
                        .as_ref()
                        .is_none_or(|space| space.approval.is_none());
                };
                let request = approval.id.clone();
                let (decision, option) = match self.option.checked_sub(1) {
                    None => (Decision::AllowOnce, None),
                    Some(index) => (Decision::Grant, Some(approval.options[index].id.clone())),
                };
                if self.send(ClientMessage::Decide {
                    request: request.clone(),
                    decision,
                    option,
                }) {
                    self.decided = Some(request);
                }
                false
            }
        }
    }

    /// B: deny the pending approval, cancel your newest queued input, or leave the space; in the lobby, dismiss what is shown or return to the server list.
    pub fn back(&mut self) {
        match self.mode {
            Mode::Servers => {}
            Mode::Lobby if self.dismissable() => {
                self.error = None;
                self.ended = None;
            }
            // At the spaces there is nothing to return from, so go to the server list.
            Mode::Lobby if self.step == LobbyStep::Spaces => self.leave_server(),
            Mode::Lobby => self.back_ring(),
            Mode::Space => {
                let pending = self
                    .space
                    .as_ref()
                    .is_some_and(|space| space.approval.is_some());
                if let Some(approval) = self.approval() {
                    let request = approval.id.clone();
                    if self.send(ClientMessage::Decide {
                        request: request.clone(),
                        decision: Decision::Deny,
                        option: None,
                    }) {
                        self.decided = Some(request);
                    }
                } else if pending {
                    // Wait for the decision already sent.
                } else if let Some(entry) = self.own_queued().map(|entry| entry.id.clone()) {
                    self.send(ClientMessage::CancelQueued { entry });
                } else {
                    self.leaving = true;
                    self.close_space();
                    self.listed = false;
                    self.mode = Mode::Lobby;
                    self.send(ClientMessage::Leave);
                }
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

    /// Y: browse again, refresh the spaces or the sessions being shown, or in a space step to the agent's next mode, as its terminal key does. A space whose agent offers no modes fetches its state again instead.
    pub fn refresh(&mut self) {
        self.error = None;
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
                let Some(space) = &self.space else {
                    return;
                };
                // Stepped from the mode the agent last confirmed, so a second press before it confirms asks again rather than skipping a mode.
                if let Some(mode) = space.next_mode().map(|mode| mode.id.clone()) {
                    self.send(ClientMessage::SetMode { mode });
                } else {
                    let space = space.id.clone();
                    self.joining = Some(space.clone());
                    self.send(ClientMessage::Join { space });
                }
            }
        }
    }

    /// X pressed or released: push-to-talk. While held, the microphone sends `agent` audio, heard by the other participants and transcribed for the agent; releasing sends it to be queued.
    pub fn talk(&mut self, pressed: bool) {
        if !pressed {
            if let Some(Talk::Recording {
                utterance, heard, ..
            }) = self.talk.take()
            {
                self.actions.push(Action::EndUtterance);
                self.talk = Some(Talk::Sending { utterance, heard });
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
        self.error = None;
        self.notice = None;
        self.actions.push(Action::BeginUtterance(
            utterance.clone(),
            self.capture_generation,
        ));
        self.talk = Some(Talk::Recording {
            utterance,
            started: Instant::now(),
            heard: String::new(),
        });
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
        self.error = None;
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
        if let Some(Talk::Recording { utterance, .. }) = &self.talk {
            let pending = self
                .actions
                .iter()
                .any(|action| matches!(action, Action::BeginUtterance(id, _) if id == utterance));
            self.actions.retain(
                |action| !matches!(action, Action::BeginUtterance(id, _) if id == utterance),
            );
            if !pending {
                self.actions.push(Action::CancelUtterance);
            }
            self.talk = None;
        }
    }

    /// The microphone could not record, so the recording is abandoned.
    pub fn microphone_failed(&mut self, error: impl Into<String>) {
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
            let pending = self
                .actions
                .iter()
                .any(|action| matches!(action, Action::BeginUtterance(id, _) if id == utterance));
            self.actions.retain(|action| {
                !matches!(action, Action::EndUtterance)
                    && !matches!(action, Action::BeginUtterance(id, _) if id == utterance)
            });
            if !pending {
                self.actions.push(Action::CancelUtterance);
            }
        }
        self.stop_talking();
        if cancelled {
            self.talk = None;
        }
        self.error = Some(error.into());
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

    /// Your newest entry still waiting in the queue. Entries are matched by participant number, since display names need not be unique.
    fn own_queued(&self) -> Option<&protocol::QueueEntry> {
        let source = self.source?;
        self.space
            .as_ref()?
            .queue
            .iter()
            .rev()
            .find(|entry| entry.state == QueueState::Queued && entry.source == source)
    }

    /// The joined space, and whether its state is current: not while the connection is down, when the server may have moved on without us.
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

    /// The panel text, at most [`ROWS`] lines of at most [`COLUMNS`] characters.
    pub fn text(&mut self) -> String {
        let lines = match self.mode {
            Mode::Servers => self.servers_text(),
            Mode::Lobby => self.lobby_text(),
            Mode::Space => self.space_text(),
        };
        debug_assert!(lines.len() <= ROWS);
        lines.join("\n")
    }

    /// Whether the lobby shows an error or an ended space, which B dismisses before it returns to the server list.
    fn dismissable(&self) -> bool {
        self.error.is_some() || (self.ended.is_some() && self.status().is_none())
    }

    fn status(&self) -> Option<String> {
        if let Some(error) = &self.error {
            return Some(format!("ERROR: {error}"));
        }
        match &self.state {
            None | Some(ConnectionState::Connecting { attempt: 1 }) => {
                return Some("Connecting...".into());
            }
            Some(ConnectionState::Connecting { attempt }) => {
                return Some(format!("Connecting (attempt {attempt})..."));
            }
            Some(ConnectionState::Disconnected { reason, retry_in }) => {
                return Some(format!(
                    "Offline, retry in {}s: {reason}",
                    retry_in.as_secs_f32().ceil()
                ));
            }
            Some(ConnectionState::Closed) => return Some("Disconnected.".into()),
            Some(ConnectionState::Connected { .. }) => {}
        }
        self.notice.clone()
    }

    fn servers_text(&self) -> Vec<String> {
        let mut lines = vec![
            short(&format!(
                "SERVERS {}/{}",
                (self.server + 1).min(self.servers.len()),
                self.servers.len()
            )),
            "Stick up/down: choose. A: connect.".into(),
        ];
        let rows = ROWS - 4;
        let start = window(self.server, self.servers.len(), rows);
        for (index, choice) in self.servers.iter().enumerate().skip(start).take(rows) {
            let marker = if index == self.server { '>' } else { ' ' };
            lines.push(short(&match &choice.unavailable {
                // A row has no room for both, and the counts say more about a discovered server than its address.
                None => match choice.counts() {
                    Some(counts) => format!("{marker} {} ({counts})", choice.label),
                    None => format!("{marker} {} {}", choice.label, choice.address),
                },
                Some(reason) => format!("{marker} ({} - {reason})", choice.label),
            }));
        }
        while lines.len() < ROWS - 2 {
            lines.push(String::new());
        }
        lines.push(short(
            self.error.as_deref().unwrap_or("Searching the network..."),
        ));
        lines.push("A connect | Y search again".into());
        lines
    }

    fn lobby_text(&self) -> Vec<String> {
        match self.step {
            LobbyStep::Spaces => self.spaces_text(),
            step => self.menu_text(step),
        }
    }

    /// The lobby at its spaces list.
    fn spaces_text(&self) -> Vec<String> {
        let server = self
            .connected
            .as_ref()
            .map_or("server", |choice| choice.label.as_str());
        let mut lines = vec![short(&format!(
            "{server} | SPACES {}/{}",
            (self.selected + 1).min(self.spaces.len()),
            self.spaces.len()
        ))];
        lines.push(short(
            &self
                .status()
                .or_else(|| self.ended.clone())
                .unwrap_or_else(|| {
                    if self.joining.is_some() {
                        "Joining...".into()
                    } else if self.role == ConnectionRole::Pairing {
                        "This server admits only paired devices: pair to join.".into()
                    } else if !self.listed {
                        "Listing spaces...".into()
                    } else if self.spaces.is_empty() && self.creates() {
                        "No spaces yet; create one.".into()
                    } else if self.spaces.is_empty() {
                        "No spaces yet.".into()
                    } else {
                        format!("You are {}.", self.name)
                    }
                }),
        ));
        let rows = ROWS - 3;
        // The spaces, then the create object.
        let entries = self
            .spaces
            .iter()
            .map(|space| {
                format!(
                    "{} [{}{}]",
                    space.title,
                    activity_label(space.activity),
                    match space.participants {
                        0 => String::new(),
                        count => format!(", {count} here"),
                    }
                )
            })
            .chain(self.creates().then(|| "+ New space".to_owned()));
        let start = window(self.selected, self.step_list(LobbyStep::Spaces), rows);
        for (index, entry) in entries.enumerate().skip(start).take(rows) {
            let marker = if index == self.selected { '>' } else { ' ' };
            lines.push(short(&format!("{marker} {entry}")));
        }
        while lines.len() < ROWS - 1 {
            lines.push(String::new());
        }
        lines.push(if self.dismissable() {
            "A choose | B dismiss | Y refresh".into()
        } else {
            "A choose | B servers | Y refresh".into()
        });
        lines
    }

    /// What the open creation ring needs said: what is missing when it is empty, since the headset cannot type, or what it is for.
    fn ring_hint(&self, step: LobbyStep) -> &'static str {
        match step {
            LobbyStep::Backend if self.backends.is_empty() => {
                "The server has no backends; ask on the PC."
            }
            LobbyStep::Directory if self.roots.is_empty() => {
                "The server offers no folders; ask on the PC."
            }
            LobbyStep::Backend => "The agent that runs in the space.",
            LobbyStep::Directory => "The folder the agent works in.",
            LobbyStep::Session if self.sessions.is_empty() => {
                "Start a new session; none listed yet."
            }
            LobbyStep::Session => "A new session, or attach to an existing one.",
            LobbyStep::Environment => "Where the space takes place, for everyone in it.",
            LobbyStep::Spaces => "",
        }
    }

    /// The creation rings, as a list, at one of their steps.
    fn menu_text(&self, step: LobbyStep) -> Vec<String> {
        let label = match step {
            LobbyStep::Backend => "NEW SPACE: BACKEND",
            LobbyStep::Directory => "NEW SPACE: FOLDER",
            LobbyStep::Session => "NEW SPACE: SESSION",
            LobbyStep::Environment => "NEW SPACE: ENVIRONMENT",
            LobbyStep::Spaces => unreachable!(),
        };
        let count = self.step_list(step);
        let mut lines = vec![short(&format!(
            "{label} {}/{}",
            (self.menu + 1).min(count),
            count
        ))];
        lines.push(short(
            &self
                .status()
                .unwrap_or_else(|| self.ring_hint(step).to_owned()),
        ));
        let rows = ROWS - 3;
        let start = window(self.menu, count, rows);
        for item in self.ring_items(step).into_iter().skip(start).take(rows) {
            let marker = if item.selected { '>' } else { ' ' };
            lines.push(short(&format!("{marker} {}", item.label)));
        }
        while lines.len() < ROWS - 1 {
            lines.push(String::new());
        }
        lines.push(if self.dismissable() {
            "A choose | B dismiss | Y refresh".into()
        } else {
            "A choose | B back | Y refresh".into()
        });
        lines
    }

    /// The choices on the ring at `step`, with the stick's selection marked.
    fn ring_items(&self, step: LobbyStep) -> Vec<arrival::Item> {
        let choice = |key, label: &str, detail: Option<&str>| arrival::Item {
            key,
            kind: arrival::Kind::Choice,
            label: label.to_owned(),
            detail: detail.map(str::to_owned),
            selected: false,
        };
        let mut items: Vec<_> = match step {
            LobbyStep::Spaces => Vec::new(),
            LobbyStep::Backend => self
                .backends
                .iter()
                .map(|backend| {
                    choice(
                        arrival::Key::Backend(backend.name.clone()),
                        &backend.name,
                        backend.attach.then_some("can attach"),
                    )
                })
                .collect(),
            LobbyStep::Directory => self
                .roots
                .iter()
                .map(|root| {
                    choice(
                        arrival::Key::Directory(root.path.clone()),
                        &root.label,
                        None,
                    )
                })
                .collect(),
            LobbyStep::Session => {
                std::iter::once(choice(arrival::Key::Session(None), "New session", None))
                    .chain(self.sessions.iter().map(|session| {
                        let title = if session.title.trim().is_empty() {
                            &session.id
                        } else {
                            &session.title
                        };
                        choice(arrival::Key::Session(Some(session.id.clone())), title, None)
                    }))
                    .collect()
            }
            LobbyStep::Environment => self
                .environments
                .iter()
                .map(|offered| {
                    choice(
                        arrival::Key::Environment(offered.id.clone()),
                        &offered.label,
                        None,
                    )
                })
                .collect(),
        };
        if let Some(item) = items.get_mut(self.menu) {
            item.selected = true;
        }
        items
    }

    /// What the headset shows as objects in the world, outside a space. See `design/arrival.md`.
    pub fn arrival(&self) -> Option<arrival::View> {
        match self.mode {
            Mode::Space => None,
            Mode::Servers => Some(arrival::View {
                heading: "Choose a server".into(),
                status: Some(
                    self.error
                        .clone()
                        .unwrap_or_else(|| "Searching the network...".into()),
                ),
                connected: None,
                items: self
                    .servers
                    .iter()
                    .enumerate()
                    .map(|(index, choice)| arrival::Item {
                        key: arrival::Key::Server(choice.address.clone()),
                        kind: arrival::Kind::Server {
                            source: choice.source,
                            available: choice.is_available(),
                        },
                        label: choice.label.clone(),
                        detail: Some(
                            choice
                                .unavailable
                                .clone()
                                .or_else(|| choice.counts())
                                .unwrap_or_else(|| choice.address.clone()),
                        ),
                        selected: index == self.server,
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
                        None
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
                        },
                        label: space.title.clone(),
                        detail: Some(activity_label(space.activity).into()),
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

    /// The label and text of the selected item: the pending request, a message or the queue.
    fn body(&self) -> (String, String) {
        let Some(space) = &self.space else {
            return ("Loading".into(), String::new());
        };
        if let Some(approval) = &space.approval {
            return (
                format!("APPROVAL: {}", approval.title),
                approval.text.clone(),
            );
        }
        if let Some(question) = &space.question {
            let mut text = question.text.clone();
            text.push_str("\n\nAnswer this question on the PC.");
            return (format!("QUESTION: {}", question.title), text);
        }
        if let Some(message) = space.messages.get(self.message) {
            let role = match message.role {
                Role::User => "USER",
                Role::Assistant => "ASSISTANT",
            };
            return (
                format!("{role} {}/{}", self.message + 1, space.messages.len()),
                message.text.clone(),
            );
        }
        if self.message == space.messages.len() && !space.queue.is_empty() {
            let text = space
                .queue
                .iter()
                .map(|entry| {
                    format!(
                        "{} [{}]: {}",
                        entry.speaker,
                        queue_label(entry.state),
                        entry.text
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            return (format!("QUEUE {}", space.queue.len()), text);
        }
        ("No messages yet".into(), String::new())
    }

    fn space_text(&mut self) -> Vec<String> {
        let (label, body) = self.body();
        // The chosen option is shown in full, since a standing option's label is the whole of what it grants.
        let choice = self
            .space
            .as_ref()
            .and_then(|space| space.approval.as_ref())
            .filter(|_| self.decided.is_none())
            .map(|approval| {
                let option = match self.option.checked_sub(1) {
                    None => "Allow once",
                    Some(index) => approval.options[index].label.as_str(),
                };
                let mut lines = wrapped_lines(&format!(
                    "{}/{}: {option}",
                    self.option + 1,
                    approval.options.len() + 1
                ));
                if lines.len() >= BODY_ROWS {
                    lines.truncate(BODY_ROWS - 1);
                    if let Some(last) = lines.last_mut() {
                        *last = short(&format!("{last} more"));
                    }
                }
                lines
            });
        let rows = BODY_ROWS - choice.as_ref().map_or(0, Vec::len);
        let pages = match &self.paged {
            Some(paged) if paged.text == body && paged.rows == rows => paged.pages.clone(),
            _ => {
                let pages: Arc<[Vec<String>]> = paginate(&body, rows).into();
                self.paged = Some(Paged {
                    text: body,
                    rows,
                    pages: pages.clone(),
                });
                pages
            }
        };
        self.page = self.page.min(pages.len() - 1);
        let Some(space) = &self.space else {
            return vec![
                "SPACE".into(),
                "Loading...".into(),
                String::new(),
                "B leave".into(),
            ];
        };
        let choosing = space.approval.as_ref().zip(choice);
        // The open microphone, and words on their way, are always shown.
        let talking = match &self.talk {
            Some(Talk::Recording { heard, .. }) if heard.is_empty() => {
                Some("MIC AGENT: listening...".to_owned())
            }
            Some(Talk::Recording { heard, .. }) => {
                Some(format!("MIC AGENT: {}", tail(heard, COLUMNS - 11)))
            }
            Some(Talk::Sending { heard, .. }) => {
                Some(format!("Sending: {}...", tail(heard, COLUMNS - 12)))
            }
            None => None,
        };
        let status = talking.or_else(|| self.status()).unwrap_or_else(|| {
            if let Some(error) = &space.error {
                format!("ERROR: {error}")
            } else if let Some(missing) = self.missing_environment() {
                // The grid is shown instead, never another environment.
                match missing {
                    "" => "The server named no environment.".into(),
                    missing => format!("Unknown environment: {missing}"),
                }
            } else if let Some(entry) = self.own_queued() {
                format!("Queued: {}", entry.text)
            } else {
                let waiting = space
                    .queue
                    .iter()
                    .filter(|entry| entry.state == QueueState::Queued)
                    .count();
                match (self.live, waiting) {
                    (true, 0) => format!("{} here", space.participants.len()),
                    (true, count) => format!("{count} queued | {} here", space.participants.len()),
                    (false, 0) => format!("Mic off | {} here", space.participants.len()),
                    (false, count) => format!(
                        "Mic off | {count} queued | {} here",
                        space.participants.len()
                    ),
                }
            }
        });
        // The current mode is shown by the name the agent's interface gives it, even when it is not one Tomori can step to.
        let mode = space
            .current_mode()
            .map(|mode| mode.name.as_str())
            .or(space.mode.as_deref().filter(|_| !space.modes.is_empty()));
        let status = if self.live && !matches!(self.talk, Some(Talk::Recording { .. })) {
            format!("MIC LIVE | {status}")
        } else {
            status
        };
        let mut lines = vec![
            short(&match mode {
                Some(mode) => format!(
                    "[{} | {mode}] {}",
                    activity_label(space.activity),
                    space.title
                ),
                None => format!("[{}] {}", activity_label(space.activity), space.title),
            }),
            short(&status),
            short(&format!("p{}/{} {label}", self.page + 1, pages.len())),
        ];
        lines.extend(pages[self.page].iter().cloned());
        let footer = if matches!(self.talk, Some(Talk::Recording { .. })) {
            "Release X to send"
        } else if let Some((approval, choice)) = choosing {
            while lines.len() + choice.len() < ROWS - 1 {
                lines.push(String::new());
            }
            lines.extend(choice);
            if approval.options.is_empty() {
                "A allow once | B deny | stick page"
            } else {
                "R-stick choose | A confirm | B deny"
            }
        } else if space.approval.is_some() {
            "Sending decision..."
        } else if self.own_queued().is_some() {
            "B cancel queued | stick pages"
        } else if space.modes.is_empty() {
            "X talk | B leave | Y refresh"
        } else {
            "X talk | B leave | Y mode"
        };
        while lines.len() < ROWS - 1 {
            lines.push(String::new());
        }
        lines.push(footer.into());
        lines
    }
}

/// `create`, a `CreateSpace`, bound to `chosen`.
fn with_environment(mut create: ClientMessage, chosen: Option<String>) -> ClientMessage {
    if let ClientMessage::CreateSpace { environment, .. } = &mut create {
        *environment = chosen;
    }
    create
}

/// Messages, then the queue when it has entries.
fn items(space: &SpaceState) -> usize {
    space.messages.len() + usize::from(!space.queue.is_empty())
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
fn tail(text: &str, width: usize) -> String {
    let ascii = wrapped_lines(&text.replace('\n', " ")).join(" ");
    if ascii.len() <= width {
        ascii
    } else {
        format!("...{}", &ascii[ascii.len() - (width - 3)..])
    }
}

fn short(text: &str) -> String {
    let ascii = wrapped_lines(&text.replace('\n', " ")).join(" ");
    if ascii.len() <= COLUMNS {
        ascii
    } else {
        format!("{}...", &ascii[..COLUMNS - 3])
    }
}

fn shift(index: usize, delta: i32, count: usize) -> usize {
    index
        .saturating_add_signed(delta as isize)
        .min(count.saturating_sub(1))
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server_choice::{Source, USB};
    use protocol::{Approval, ApprovalOption, Message, Participant, Question, QueueEntry};
    use std::time::Duration;

    const SPACE: &str = "space-1";

    fn connected() -> Event {
        connected_with(vec![], vec![])
    }

    fn connected_with(backends: Vec<BackendInfo>, roots: Vec<WorkspaceRoot>) -> Event {
        Event::State(ConnectionState::Connected {
            source: 1,
            server: "studio".into(),
            role: ConnectionRole::Owner,
            certificate: None,
            backends,
            roots,
            environments: sky(),
        })
    }

    /// The environments today's server offers.
    fn sky() -> Vec<EnvironmentInfo> {
        vec![EnvironmentInfo {
            id: "sky".into(),
            label: "Sky".into(),
        }]
    }

    /// A backend list for the menu tests.
    fn backends() -> Vec<BackendInfo> {
        vec![
            BackendInfo {
                name: "fake".into(),
                attach: true,
            },
            BackendInfo {
                name: "plain".into(),
                attach: false,
            },
        ]
    }

    /// A directory list for the menu tests.
    fn roots() -> Vec<WorkspaceRoot> {
        vec![WorkspaceRoot {
            path: "/work/tomori".into(),
            label: "tomori".into(),
        }]
    }

    /// Connected to a server that offers `backends` and `roots`, in the lobby.
    fn in_lobby(backends: Vec<BackendInfo>, roots: Vec<WorkspaceRoot>) -> Conversation {
        let mut conversation = Conversation::new("Ada");
        conversation.connect(Choice::new("Studio", "10.0.0.1:8765", Source::Discovered));
        conversation.take_actions();
        conversation.apply(connected_with(backends, roots));
        conversation.take_actions();
        conversation
    }

    fn space(change: impl FnOnce(&mut SpaceState)) -> Event {
        let mut state = SpaceState {
            id: SPACE.into(),
            title: "Tomori".into(),
            backend: "claude-code".into(),
            directory: "/work/tomori".into(),
            environment: "sky".into(),
            activity: Activity::Idle,
            participants: vec![Participant {
                name: "Ada".into(),
                source: 1,
            }],
            ..Default::default()
        };
        change(&mut state);
        Event::Message(ServerMessage::Space {
            state: Box::new(state),
        })
    }

    fn message(id: &str, role: Role, text: &str) -> Message {
        Message {
            id: id.into(),
            role,
            text: text.into(),
            completed: true,
        }
    }

    fn approval(options: &[(&str, &str)]) -> Approval {
        Approval {
            id: "request-1".into(),
            title: "Bash".into(),
            text: "cargo test".into(),
            options: options
                .iter()
                .map(|(id, label)| ApprovalOption {
                    id: (*id).into(),
                    label: (*label).into(),
                })
                .collect(),
        }
    }

    /// An entry spoken by `speaker`. Ada is this client, participant 1 as connected in these tests; anyone else is participant 2.
    fn entry(id: &str, speaker: &str, state: QueueState) -> QueueEntry {
        QueueEntry {
            id: id.into(),
            source: if speaker == "Ada" { 1 } else { 2 },
            speaker: speaker.into(),
            text: format!("text of {id}"),
            state,
        }
    }

    fn sent(conversation: &mut Conversation) -> Vec<ClientMessage> {
        conversation
            .take_actions()
            .into_iter()
            .filter_map(|action| match action {
                Action::Send(message) => Some(message),
                _ => None,
            })
            .collect()
    }

    /// The server's refusal of a join of `space`.
    fn refused(space: &str) -> Event {
        Event::Message(ServerMessage::Error {
            error: "That space does not exist.".into(),
            space: Some(space.into()),
        })
    }

    /// A conversation connected to a server and joined to [`SPACE`].
    fn joined(change: impl FnOnce(&mut SpaceState)) -> Conversation {
        let mut conversation = Conversation::new("Ada");
        conversation.connect(Choice::new("Studio", "10.0.0.1:8765", Source::Discovered));
        conversation.apply(connected());
        conversation.apply(space(change));
        conversation.take_actions();
        conversation
    }

    fn assert_fits(conversation: &mut Conversation) {
        let text = conversation.text();
        assert!(text.lines().count() <= ROWS, "{text}");
        assert!(text.lines().all(|line| line.len() <= COLUMNS), "{text}");
    }

    #[test]
    fn connects_lists_and_joins() {
        let mut conversation = Conversation::new("Ada");
        assert!(conversation.is_browsing());
        conversation.set_servers(vec![
            Choice::new("Studio", "10.0.0.1:8765", Source::Discovered),
            Choice::new("USB", USB, Source::Usb),
        ]);
        assert_fits(&mut conversation);
        conversation.navigate(0, 1);
        assert!(!conversation.accept());
        assert_eq!(
            conversation.take_actions(),
            [Action::Connect(Choice::new("USB", USB, Source::Usb))]
        );
        assert_eq!(conversation.mode(), Mode::Lobby);
        assert!(!conversation.is_browsing());

        conversation.apply(connected());
        assert_eq!(sent(&mut conversation), [ClientMessage::ListSpaces]);
        conversation.apply(Event::Message(ServerMessage::Spaces {
            spaces: vec![SpaceSummary {
                id: SPACE.into(),
                title: "Tomori".into(),
                backend: "claude-code".into(),
                directory: "/work/tomori".into(),
                environment: "sky".into(),
                activity: Activity::Running,
                participants: 2,
            }],
        }));
        assert!(conversation.text().contains("> Tomori [running, 2 here]"));
        assert_fits(&mut conversation);
        assert!(!conversation.accept());
        assert_eq!(
            sent(&mut conversation),
            [ClientMessage::Join {
                space: SPACE.into()
            }]
        );
        conversation.apply(space(|_| {}));
        assert_eq!(conversation.mode(), Mode::Space);
    }

    fn listed(conversation: &mut Conversation, spaces: Vec<SpaceSummary>) {
        conversation.apply(Event::Message(ServerMessage::Spaces { spaces }));
    }

    fn summary(id: &str, title: &str) -> SpaceSummary {
        SpaceSummary {
            id: id.into(),
            title: title.into(),
            backend: "fake".into(),
            directory: "/work/tomori".into(),
            activity: Activity::Idle,
            participants: 1,
            environment: "sky".into(),
        }
    }

    fn sessions() -> Event {
        Event::Message(ServerMessage::Sessions {
            sessions: vec![
                protocol::SessionSummary {
                    id: "ses_1".into(),
                    title: "Tomori opencode support".into(),
                    directory: "/work/tomori".into(),
                },
                protocol::SessionSummary {
                    id: "ses_2".into(),
                    title: "Other work".into(),
                    directory: "/work/tomori".into(),
                },
            ],
        })
    }

    fn keys(items: &[arrival::Item]) -> Vec<arrival::Key> {
        items.iter().map(|item| item.key.clone()).collect()
    }

    #[test]
    fn the_create_object_is_shown_directly_when_there_are_no_spaces() {
        let mut conversation = in_lobby(backends(), roots());
        listed(&mut conversation, vec![]);
        let view = conversation.arrival().unwrap();
        assert_eq!(keys(&view.items), [arrival::Key::Create]);
        assert!(view.items[0].selected);
        assert_eq!(view.ring, None);
        assert_eq!(view.status.as_deref(), Some("No spaces yet; create one."));
        assert!(conversation.text().contains("> + New space"));
        assert_fits(&mut conversation);

        // With spaces, it sits after them in the same view.
        listed(&mut conversation, vec![summary("one", "One")]);
        let view = conversation.arrival().unwrap();
        assert_eq!(
            keys(&view.items),
            [arrival::Key::Space("one".into()), arrival::Key::Create]
        );
        // The create object stays selected as the spaces change.
        assert!(view.items[1].selected);
    }

    /// Told the role `role`, as the server does whenever it changes.
    fn role(role: ConnectionRole) -> Event {
        Event::Message(ServerMessage::Role { role })
    }

    /// Connected over TLS with `certificate`, as `role`, in the lobby.
    fn connected_securely(role: ConnectionRole, certificate: client::Fingerprint) -> Conversation {
        let mut conversation = Conversation::new("Ada");
        conversation.connect(Choice::new("Studio", "10.0.0.1:8765", Source::Discovered));
        conversation.take_actions();
        conversation.apply(Event::State(ConnectionState::Connected {
            source: 1,
            server: "studio".into(),
            role,
            certificate: Some(certificate),
            backends: vec![],
            roots: vec![],
            environments: sky(),
        }));
        conversation.take_actions();
        conversation
    }

    fn pairing_label(conversation: &Conversation) -> Option<String> {
        conversation
            .arrival()?
            .items
            .into_iter()
            .find(|item| item.key == arrival::Key::Pair)
            .map(|item| item.label)
    }

    #[test]
    fn a_visitor_pairs_with_a_code_derived_from_the_certificate() {
        use security::pairing::{Nonce, code, display};
        let certificate = client::Fingerprint::of(b"server certificate");
        let mut conversation = connected_securely(ConnectionRole::Visitor, certificate);
        assert_eq!(pairing_label(&conversation).as_deref(), Some("Pair"));

        conversation.choose_object(&arrival::Key::Pair);
        let [ClientMessage::PairBegin { commitment }] = &sent(&mut conversation)[..] else {
            panic!("expected a commitment");
        };
        let commitment = commitment.clone();
        assert_eq!(pairing_label(&conversation).as_deref(), Some("Pairing..."));

        let server = Nonce::generate();
        conversation.apply(Event::Message(ServerMessage::PairNonce {
            nonce: server.to_hex(),
        }));
        let [ClientMessage::PairReveal { nonce }] = &sent(&mut conversation)[..] else {
            panic!("expected the value revealed");
        };
        let device = Nonce::parse(nonce).unwrap();
        // The revealed value is the one committed to, and the code is derived from it, the server's and the certificate.
        assert_eq!(device.commitment(), commitment);
        let expected = display(&code(certificate, device, server));
        assert_eq!(pairing_label(&conversation), Some(expected.clone()));
        assert!(conversation.status().unwrap().contains(&expected));

        conversation.apply(Event::Message(ServerMessage::Paired {
            token: "tomori_paired".into(),
        }));
        assert_eq!(
            conversation.take_actions(),
            [Action::Paired {
                token: "tomori_paired".into(),
                certificate
            }]
        );
        assert!(conversation.status().unwrap().contains("now an owner"));
    }

    #[test]
    fn a_failed_or_cancelled_pairing_starts_over() {
        let certificate = client::Fingerprint::of(b"server certificate");
        let mut conversation = connected_securely(ConnectionRole::Visitor, certificate);
        conversation.choose_object(&arrival::Key::Pair);
        conversation.take_actions();
        conversation.apply(Event::Message(ServerMessage::PairFailed {
            error: "The pairing was declined.".into(),
        }));
        assert_eq!(
            conversation.status().as_deref(),
            Some("The pairing was declined.")
        );
        assert_eq!(pairing_label(&conversation).as_deref(), Some("Pair"));

        // Choosing it again while pairing cancels.
        conversation.choose_object(&arrival::Key::Pair);
        conversation.take_actions();
        conversation.choose_object(&arrival::Key::Pair);
        assert_eq!(sent(&mut conversation), [ClientMessage::PairCancel]);
        // A value from a server that was not asked is ignored.
        conversation.apply(Event::Message(ServerMessage::PairNonce {
            nonce: security::pairing::Nonce::generate().to_hex(),
        }));
        assert!(conversation.take_actions().is_empty());
    }

    #[test]
    fn pairing_is_offered_over_tls_to_anyone_but_an_owner() {
        let certificate = client::Fingerprint::of(b"server certificate");
        assert!(pairing_label(&connected_securely(ConnectionRole::Owner, certificate)).is_none());
        assert!(
            pairing_label(&connected_securely(
                ConnectionRole::Collaborator,
                certificate
            ))
            .is_some()
        );
        // Over plain HTTP there is no certificate to derive a code from.
        let mut plain = in_lobby(vec![], vec![]);
        plain.apply(role(ConnectionRole::Visitor));
        assert!(pairing_label(&plain).is_none());
        plain.pair();
        assert!(plain.take_actions().is_empty());
    }

    #[test]
    fn a_connection_admitted_only_to_pair_lists_no_spaces() {
        let certificate = client::Fingerprint::of(b"server certificate");
        let mut conversation = Conversation::new("Ada");
        conversation.connect(Choice::new("Studio", "10.0.0.1:8765", Source::Discovered));
        conversation.take_actions();
        conversation.apply(Event::State(ConnectionState::Connected {
            source: 1,
            server: "studio".into(),
            role: ConnectionRole::Pairing,
            certificate: Some(certificate),
            backends: vec![],
            roots: vec![],
            environments: sky(),
        }));
        assert!(sent(&mut conversation).is_empty());
        let view = conversation.arrival().unwrap();
        assert_eq!(keys(&view.items), [arrival::Key::Pair]);
        assert_eq!(
            view.status.as_deref(),
            Some("This server admits only paired devices: pair to join.")
        );
    }

    #[test]
    fn only_owners_are_offered_the_create_object() {
        let mut conversation = in_lobby(vec![], vec![]);
        conversation.apply(role(ConnectionRole::Visitor));
        assert_eq!(conversation.role(), ConnectionRole::Visitor);
        listed(&mut conversation, vec![]);
        let view = conversation.arrival().unwrap();
        assert!(view.items.is_empty());
        assert_eq!(view.status.as_deref(), Some("No spaces yet."));
        assert!(!conversation.text().contains("New space"));
        // Choosing it anyway, as a stale pointer might, opens nothing.
        conversation.choose_object(&arrival::Key::Create);
        assert!(!conversation.text().contains("NEW SPACE"));

        listed(&mut conversation, vec![summary("one", "One")]);
        let view = conversation.arrival().unwrap();
        assert_eq!(keys(&view.items), [arrival::Key::Space("one".into())]);
        assert!(!conversation.text().contains("New space"));
    }

    #[test]
    fn a_visitor_watches_but_does_not_talk_or_act() {
        let mut conversation = joined(|state| state.approval = Some(approval(&[])));
        conversation.apply(role(ConnectionRole::Visitor));
        conversation.talk(true);
        assert_eq!(conversation.status().as_deref(), Some(VISITING));
        assert!(conversation.take_actions().is_empty());
        conversation.toggle_live();
        assert!(conversation.take_actions().is_empty());
        conversation.accept();
        assert!(sent(&mut conversation).is_empty());

        // Admitted as a collaborator, they can, until they become a visitor again, which closes the microphone.
        conversation.apply(role(ConnectionRole::Collaborator));
        conversation.toggle_live();
        assert!(conversation.live);
        conversation.apply(role(ConnectionRole::Visitor));
        assert!(!conversation.live);
        conversation.apply(role(ConnectionRole::Collaborator));
        conversation.take_actions();
        conversation.accept();
        assert!(matches!(
            sent(&mut conversation)[..],
            [ClientMessage::Decide { .. }]
        ));
    }

    #[test]
    fn the_stick_creates_a_space_and_a_backend_without_attaching_skips_the_session_ring() {
        let mut conversation = in_lobby(backends(), roots());
        listed(&mut conversation, vec![]);
        // A on the create object opens the backend ring in place.
        assert!(!conversation.accept());
        assert!(conversation.text().contains("NEW SPACE: BACKEND"));
        assert_fits(&mut conversation);
        let view = conversation.arrival().unwrap();
        let ring = view.ring.unwrap();
        assert_eq!(ring.step, arrival::Step::Backend);
        assert_eq!(
            keys(&ring.items),
            [
                arrival::Key::Backend("fake".into()),
                arrival::Key::Backend("plain".into())
            ]
        );
        assert_eq!(view.items, conversation.arrival().unwrap().items);
        assert_eq!(
            view.items.last().unwrap().kind,
            arrival::Kind::Create { open: true }
        );

        conversation.navigate(0, 1);
        conversation.accept();
        assert!(conversation.text().contains("NEW SPACE: FOLDER"));
        conversation.accept();
        assert_eq!(
            sent(&mut conversation),
            [ClientMessage::CreateSpace {
                title: "plain in tomori".into(),
                backend: "plain".into(),
                directory: "/work/tomori".into(),
                session: None,
                environment: Some("sky".into()),
            }]
        );
        // Back at the spaces, with the rings closed.
        let view = conversation.arrival().unwrap();
        assert_eq!(view.ring, None);
        assert!(conversation.text().contains("SPACES"));
    }

    #[test]
    fn pointing_creates_a_space_attached_to_an_existing_session() {
        let mut conversation = in_lobby(backends(), roots());
        listed(&mut conversation, vec![summary("one", "One")]);
        conversation.choose_object(&arrival::Key::Create);
        conversation.choose_object(&arrival::Key::Backend("fake".into()));
        conversation.choose_object(&arrival::Key::Directory("/work/tomori".into()));
        // The backend attaches, so its sessions in the folder are asked for and the session ring opens.
        assert_eq!(
            sent(&mut conversation),
            [ClientMessage::ListSessions {
                backend: "fake".into(),
                directory: "/work/tomori".into(),
            }]
        );
        let ring = conversation.arrival().unwrap().ring.unwrap();
        assert_eq!(ring.step, arrival::Step::Session);
        assert_eq!(keys(&ring.items), [arrival::Key::Session(None)]);
        conversation.apply(sessions());
        let ring = conversation.arrival().unwrap().ring.unwrap();
        assert_eq!(ring.items.len(), 3);
        assert_eq!(ring.items[1].label, "Tomori opencode support");
        assert!(conversation.text().contains("NEW SPACE: SESSION"));
        assert_fits(&mut conversation);

        conversation.choose_object(&arrival::Key::Session(Some("ses_1".into())));
        assert_eq!(
            sent(&mut conversation),
            [ClientMessage::CreateSpace {
                title: "Tomori opencode support".into(),
                backend: "fake".into(),
                directory: "/work/tomori".into(),
                session: Some("ses_1".into()),
                environment: Some("sky".into()),
            }]
        );
        assert_eq!(conversation.arrival().unwrap().ring, None);
    }

    #[test]
    fn a_chooses_the_highlighted_session_not_the_ring_selection() {
        let mut conversation = in_lobby(backends(), roots());
        listed(&mut conversation, vec![summary("one", "One")]);
        conversation.choose_object(&arrival::Key::Create);
        conversation.choose_object(&arrival::Key::Backend("fake".into()));
        conversation.choose_object(&arrival::Key::Directory("/work/tomori".into()));
        sent(&mut conversation);
        conversation.apply(sessions());
        // The ring's own selection is still "New session"; the ray highlights
        // an existing session, and A must choose that one.
        let highlighted = arrival::Key::Session(Some("ses_1".into()));
        assert!(!conversation.press_a(Some(&highlighted)));
        assert!(matches!(
            sent(&mut conversation).as_slice(),
            [ClientMessage::CreateSpace { session: Some(session), .. }] if session == "ses_1"
        ));
    }

    #[test]
    fn a_without_a_highlight_accepts_the_ring_selection() {
        let mut conversation = in_lobby(backends(), roots());
        listed(&mut conversation, vec![summary("one", "One")]);
        conversation.choose_object(&arrival::Key::Create);
        conversation.choose_object(&arrival::Key::Backend("fake".into()));
        conversation.choose_object(&arrival::Key::Directory("/work/tomori".into()));
        sent(&mut conversation);
        conversation.apply(sessions());
        assert!(!conversation.press_a(None));
        assert!(matches!(
            sent(&mut conversation).as_slice(),
            [ClientMessage::CreateSpace { session: None, .. }]
        ));
    }

    #[test]
    fn a_new_session_is_offered_first_on_the_session_ring() {
        let mut conversation = in_lobby(backends(), roots());
        listed(&mut conversation, vec![]);
        conversation.accept();
        conversation.accept();
        conversation.accept();
        conversation.apply(sessions());
        conversation.take_actions();
        assert!(conversation.text().contains("> New session"));
        conversation.accept();
        assert_eq!(
            sent(&mut conversation),
            [ClientMessage::CreateSpace {
                title: "fake in tomori".into(),
                backend: "fake".into(),
                directory: "/work/tomori".into(),
                session: None,
                environment: Some("sky".into()),
            }]
        );
    }

    #[test]
    fn stepping_back_returns_to_the_previous_ring() {
        let mut conversation = in_lobby(backends(), roots());
        listed(&mut conversation, vec![summary("one", "One")]);
        conversation.choose_object(&arrival::Key::Create);
        conversation.choose_object(&arrival::Key::Backend("fake".into()));
        conversation.choose_object(&arrival::Key::Directory("/work/tomori".into()));
        conversation.apply(sessions());
        let step = |conversation: &Conversation| {
            conversation.arrival().unwrap().ring.map(|ring| {
                (
                    ring.step,
                    keys(
                        &ring
                            .items
                            .into_iter()
                            .filter(|item| item.selected)
                            .collect::<Vec<_>>(),
                    ),
                )
            })
        };
        // B steps back, with what was chosen there selected.
        conversation.back();
        assert_eq!(
            step(&conversation),
            Some((
                arrival::Step::Directory,
                vec![arrival::Key::Directory("/work/tomori".into())]
            ))
        );
        // So does choosing the create object while its rings are open.
        conversation.choose_object(&arrival::Key::Create);
        assert_eq!(
            step(&conversation),
            Some((
                arrival::Step::Backend,
                vec![arrival::Key::Backend("fake".into())]
            ))
        );
        conversation.back();
        assert_eq!(step(&conversation), None);
        conversation.take_actions();
        // From the spaces, B returns to the servers.
        conversation.back();
        assert_eq!(conversation.take_actions(), [Action::Disconnect]);
        assert_eq!(conversation.mode(), Mode::Servers);
    }

    #[test]
    fn pointing_chooses_servers_spaces_and_the_connected_server() {
        let mut conversation = Conversation::new("Ada");
        let mut old = Choice::new("Old", "10.0.0.3:8765", Source::Discovered);
        old.unavailable = Some("protocol 1, needs 2".into());
        let studio = Choice::new("Studio", "10.0.0.1:8765", Source::Discovered);
        conversation.set_servers(vec![
            old,
            studio.clone(),
            Choice::new("USB", USB, Source::Usb),
        ]);
        let view = conversation.arrival().unwrap();
        assert_eq!(view.items.len(), 3);
        assert_eq!(
            view.items[0].kind,
            arrival::Kind::Server {
                source: Source::Discovered,
                available: false
            }
        );
        // An incompatible server is shown but can't be chosen.
        conversation.choose_object(&arrival::Key::Server("10.0.0.3:8765".into()));
        assert!(conversation.take_actions().is_empty());
        assert!(
            conversation
                .arrival()
                .unwrap()
                .status
                .unwrap()
                .contains("Cannot join Old")
        );
        conversation.choose_object(&arrival::Key::Server("10.0.0.1:8765".into()));
        assert_eq!(conversation.take_actions(), [Action::Connect(studio)]);
        conversation.apply(connected());
        listed(
            &mut conversation,
            vec![summary("one", "One"), summary("two", "Two")],
        );
        conversation.take_actions();
        let view = conversation.arrival().unwrap();
        assert_eq!(view.connected.unwrap().label, "Studio");
        assert_eq!(view.heading, "Spaces on Studio");

        conversation.choose_object(&arrival::Key::Space("two".into()));
        assert_eq!(
            sent(&mut conversation),
            [ClientMessage::Join {
                space: "two".into()
            }]
        );
        // The connected server returns to the servers.
        conversation.choose_object(&arrival::Key::Connected("10.0.0.1:8765".into()));
        assert_eq!(conversation.take_actions(), [Action::Disconnect]);
        assert!(conversation.is_browsing());
    }

    #[test]
    fn a_ring_key_from_another_ring_is_ignored() {
        let mut conversation = in_lobby(backends(), roots());
        listed(&mut conversation, vec![]);
        conversation.choose_object(&arrival::Key::Directory("/work/tomori".into()));
        assert_eq!(conversation.arrival().unwrap().ring, None);
        conversation.choose_object(&arrival::Key::Create);
        conversation.choose_object(&arrival::Key::Directory("/work/tomori".into()));
        assert_eq!(
            conversation.arrival().unwrap().ring.unwrap().step,
            arrival::Step::Backend
        );
        assert!(sent(&mut conversation).is_empty());
    }

    /// Walk the creation rings from the create object, choosing the first backend, the first folder and a new session.
    fn create_in(conversation: &mut Conversation) {
        listed(conversation, vec![]);
        conversation.accept();
        conversation.accept();
        conversation.accept();
        conversation.apply(sessions());
        conversation.take_actions();
        conversation.accept();
    }

    #[test]
    fn several_environments_are_chosen_after_the_folder() {
        let mut conversation = in_lobby(backends(), roots());
        conversation.apply(Event::State(ConnectionState::Connected {
            source: 1,
            server: "studio".into(),
            role: ConnectionRole::Owner,
            certificate: None,
            backends: backends(),
            roots: roots(),
            environments: vec![
                EnvironmentInfo {
                    id: "sky".into(),
                    label: "Sky".into(),
                },
                EnvironmentInfo {
                    id: "observe".into(),
                    label: "Observe".into(),
                },
            ],
        }));
        conversation.take_actions();
        create_in(&mut conversation);
        assert!(sent(&mut conversation).is_empty());
        let text = conversation.text();
        assert!(text.contains("NEW SPACE: ENVIRONMENT 1/2"), "{text}");
        assert!(text.contains("> Sky") && text.contains("Observe"), "{text}");
        assert_fits(&mut conversation);
        let ring = conversation.arrival().unwrap().ring.unwrap();
        assert_eq!(ring.step, arrival::Step::Environment);

        // B steps back to the session ring without creating anything; choosing again offers the environments again.
        conversation.back();
        assert!(conversation.text().contains("NEW SPACE: SESSION"));
        assert!(sent(&mut conversation).is_empty());
        conversation.accept();
        conversation.choose_object(&arrival::Key::Environment("observe".into()));
        assert_eq!(
            sent(&mut conversation),
            [ClientMessage::CreateSpace {
                title: "fake in tomori".into(),
                backend: "fake".into(),
                directory: "/work/tomori".into(),
                session: None,
                environment: Some("observe".into()),
            }]
        );
        assert!(conversation.text().contains("SPACES"));
    }

    #[test]
    fn a_server_offering_no_environments_binds_its_own() {
        let mut conversation = in_lobby(backends(), roots());
        conversation.apply(Event::State(ConnectionState::Connected {
            source: 1,
            server: "studio".into(),
            role: ConnectionRole::Owner,
            certificate: None,
            backends: backends(),
            roots: roots(),
            environments: vec![],
        }));
        conversation.take_actions();
        create_in(&mut conversation);
        assert!(matches!(
            sent(&mut conversation).as_slice(),
            [ClientMessage::CreateSpace {
                environment: None,
                ..
            }]
        ));
    }

    #[test]
    fn arrival_and_the_lobby_use_the_grid_and_a_space_its_environment() {
        let mut conversation = Conversation::new("Ada");
        assert_eq!(conversation.environment(), Environment::Minimal);
        conversation.connect(Choice::new("Studio", "10.0.0.1:8765", Source::Discovered));
        conversation.apply(connected());
        assert_eq!(conversation.environment(), Environment::Minimal);
        conversation.apply(space(|_| {}));
        assert_eq!(conversation.environment(), Environment::Sky);
        // Leaving returns to the grid at once, before the server confirms.
        conversation.back();
        assert_eq!(conversation.mode(), Mode::Lobby);
        assert_eq!(conversation.environment(), Environment::Minimal);
        conversation.apply(Event::Message(ServerMessage::Left));
        assert_eq!(conversation.environment(), Environment::Minimal);
        // Returning to the servers keeps the grid.
        conversation.apply(space(|_| {}));
        assert_eq!(conversation.environment(), Environment::Sky);
        conversation.apply(Event::Message(ServerMessage::Left));
        conversation.back();
        assert_eq!(conversation.mode(), Mode::Servers);
        assert_eq!(conversation.environment(), Environment::Minimal);
    }

    #[test]
    fn a_space_in_an_unknown_environment_shows_the_grid_and_names_it() {
        let mut conversation = joined(|state| state.environment = "observe".into());
        assert_eq!(conversation.environment(), Environment::Minimal);
        let text = conversation.text();
        assert!(text.contains("Unknown environment: observe"), "{text}");
        assert_fits(&mut conversation);
        // An older server names none.
        conversation.apply(space(|state| state.environment = String::new()));
        assert_eq!(conversation.environment(), Environment::Minimal);
        assert!(
            conversation
                .text()
                .contains("The server named no environment.")
        );
        // A known one is drawn, and the notice goes.
        conversation.apply(space(|_| {}));
        assert_eq!(conversation.environment(), Environment::Sky);
        assert!(!conversation.text().contains("environment"));
    }

    #[test]
    fn the_server_list_shows_advertised_counts() {
        let mut conversation = Conversation::new("Ada");
        let mut studio = Choice::new("Studio", "10.0.0.1:8765", Source::Discovered);
        studio.spaces = Some(2);
        studio.participants = Some(1);
        let usb = Choice::new("USB", "127.0.0.1:8765", Source::Usb);
        conversation.set_servers(vec![studio, usb]);
        let text = conversation.text();
        assert!(text.contains("> Studio (2 spaces, 1 person)\n"), "{text}");
        assert!(text.contains("\nUSB 127.0.0.1:8765\n"), "{text}");
    }

    #[test]
    fn an_incompatible_server_cannot_be_chosen() {
        let mut conversation = Conversation::new("Ada");
        let mut old = Choice::new("Old", "10.0.0.3:8765", Source::Discovered);
        old.unavailable = Some("protocol 1, needs 2".into());
        conversation.set_servers(vec![old]);
        assert!(conversation.text().contains("(Old - protocol 1, needs 2)"));
        conversation.accept();
        assert!(conversation.take_actions().is_empty());
        assert_eq!(conversation.mode(), Mode::Servers);
        assert!(conversation.text().contains("Cannot join Old"));
    }

    #[test]
    fn approval_with_options_sends_grant_with_the_chosen_option() {
        let mut conversation = joined(|state| {
            state.activity = Activity::Waiting;
            state.approval = Some(approval(&[
                ("project", "Always allow in this project"),
                ("session", "Allow for this session"),
            ]));
        });
        assert!(conversation.is_choosing());
        assert!(conversation.text().contains("1/3: Allow once"));
        conversation.choose(1);
        conversation.choose(1);
        conversation.choose(1);
        assert!(conversation.text().contains("3/3: Allow for this session"));
        conversation.choose(-1);
        assert_fits(&mut conversation);
        assert!(!conversation.accept());
        assert_eq!(
            sent(&mut conversation),
            [ClientMessage::Decide {
                request: "request-1".into(),
                decision: Decision::Grant,
                option: Some("project".into()),
            }]
        );
        // Further presses neither decide again nor leave while the decision is on its way.
        assert!(!conversation.accept());
        conversation.back();
        assert!(conversation.take_actions().is_empty());
        assert_eq!(conversation.mode(), Mode::Space);
        assert!(conversation.text().contains("Sending decision..."));
    }

    #[test]
    fn approval_without_a_choice_allows_once_and_b_denies() {
        let mut conversation = joined(|state| state.approval = Some(approval(&[])));
        assert!(!conversation.is_choosing());
        conversation.choose(1);
        conversation.accept();
        assert_eq!(
            sent(&mut conversation),
            [ClientMessage::Decide {
                request: "request-1".into(),
                decision: Decision::AllowOnce,
                option: None,
            }]
        );

        let mut conversation = joined(|state| state.approval = Some(approval(&[("a", "A")])));
        conversation.back();
        assert_eq!(
            sent(&mut conversation),
            [ClientMessage::Decide {
                request: "request-1".into(),
                decision: Decision::Deny,
                option: None,
            }]
        );
        assert_eq!(conversation.mode(), Mode::Space);
    }

    #[test]
    fn no_decision_is_sent_without_a_press() {
        let mut conversation = joined(|_| {});
        for _ in 0..3 {
            conversation.apply(space(|state| {
                state.approval = Some(approval(&[("a", "A")]));
            }));
            conversation.apply(connected());
            conversation.navigate(1, 1);
            conversation.choose(1);
            conversation.refresh();
            assert_fits(&mut conversation);
        }
        assert!(
            sent(&mut conversation)
                .iter()
                .all(|message| !matches!(message, ClientMessage::Decide { .. }))
        );
    }

    #[test]
    fn a_new_approval_resets_the_choice_and_a_refused_decision_can_be_retried() {
        let mut conversation = joined(|state| state.approval = Some(approval(&[("a", "A")])));
        conversation.choose(1);
        conversation.accept();
        conversation.take_actions();
        conversation.apply(Event::Message(ServerMessage::Error {
            error: "That request is no longer pending.".into(),
            space: None,
        }));
        assert!(conversation.is_choosing());
        assert!(conversation.text().contains("ERROR: That request"));

        conversation.apply(space(|state| {
            let mut next = approval(&[("b", "B")]);
            next.id = "request-2".into();
            state.approval = Some(next);
        }));
        assert!(conversation.text().contains("1/2: Allow once"));
        conversation.accept();
        assert_eq!(
            sent(&mut conversation),
            [ClientMessage::Decide {
                request: "request-2".into(),
                decision: Decision::AllowOnce,
                option: None,
            }]
        );
    }

    #[test]
    fn a_standing_option_is_shown_in_full() {
        let label =
            "Yes, and don't ask again for Bash commands starting with cargo test in this project";
        let mut conversation =
            joined(|state| state.approval = Some(approval(&[("always", label)])));
        conversation.choose(1);
        let text = conversation.text();
        assert_eq!(text.lines().count(), ROWS);
        let shown = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(shown.contains(&format!("2/2: {label}")), "{text}");
    }

    #[test]
    fn activity_is_never_ready_unless_the_server_says_idle() {
        let mut conversation = Conversation::new("Ada");
        assert_eq!(conversation.activity(), SessionActivity::Unselected);

        let cases = [
            (Activity::Idle, SessionActivity::Ready),
            (Activity::Running, SessionActivity::Working),
            (Activity::Waiting, SessionActivity::Waiting),
            (Activity::Closed, SessionActivity::Unavailable),
            (Activity::Unknown, SessionActivity::Unknown),
        ];
        for (activity, expected) in cases {
            conversation = joined(|state| state.activity = activity);
            assert_eq!(conversation.activity(), expected, "{activity:?}");
        }

        let conversation = joined(|state| {
            state.queue = vec![entry("1", "Bea", QueueState::Queued)];
        });
        assert_eq!(conversation.activity(), SessionActivity::Working);
        let conversation = joined(|state| state.question = Some(question()));
        assert_eq!(conversation.activity(), SessionActivity::Waiting);

        let mut conversation = joined(|_| {});
        conversation.apply(Event::State(ConnectionState::Disconnected {
            reason: client::Disconnect::Timeout,
            retry_in: Duration::from_secs(2),
        }));
        assert_eq!(conversation.activity(), SessionActivity::Unavailable);
        assert!(conversation.text().contains("Offline, retry in 2s"));
        assert_fits(&mut conversation);
    }

    #[test]
    fn the_joined_space_is_current_only_while_connected_and_gone_after_leaving() {
        assert!(Conversation::new("Ada").joined_space().is_none());
        let mut conversation = joined(|state| {
            state.workers = vec![protocol::Worker {
                id: "w".into(),
                parent: None,
                title: "Explore".into(),
                state: protocol::WorkerState::Running,
            }]
        });
        let (space, current) = conversation.joined_space().unwrap();
        assert_eq!(
            (space.id.as_str(), space.workers.len(), current),
            (SPACE, 1, true)
        );
        conversation.apply(Event::State(ConnectionState::Disconnected {
            reason: client::Disconnect::Timeout,
            retry_in: Duration::from_secs(2),
        }));
        assert!(!conversation.joined_space().unwrap().1);
        conversation.apply(connected());
        assert!(conversation.joined_space().unwrap().1);
        conversation.back();
        assert!(conversation.joined_space().is_none());
    }

    #[test]
    fn an_unknown_activity_from_a_newer_server_is_shown_as_unknown() {
        let state: SpaceState = serde_json::from_str(
            r#"{"id":"s","title":"t","backend":"b","directory":"/","activity":"thinking",
            "messages":[],"queue":[],"approval":null,"question":null,"workers":[],
            "presentations":[],"participants":[],"attribution":true,"error":null}"#,
        )
        .unwrap();
        let mut conversation = joined(|_| {});
        conversation.apply(Event::Message(ServerMessage::Space {
            state: Box::new(state),
        }));
        assert_eq!(conversation.activity(), SessionActivity::Unknown);
        assert!(conversation.text().starts_with("[unknown] t"));
    }

    fn question() -> Question {
        Question {
            id: "question-1".into(),
            title: "Which test?".into(),
            text: "Pick one".into(),
            field: "test".into(),
            supported: true,
        }
    }

    #[test]
    fn redraws_a_message_whose_text_changes_to_the_same_length() {
        let mut conversation = joined(|state| {
            state.messages = vec![message("m1", Role::Assistant, "Hello")];
        });
        assert!(conversation.text().contains("Hello"));
        conversation.apply(space(|state| {
            state.messages = vec![message("m1", Role::Assistant, "Howdy")];
        }));
        let text = conversation.text();
        assert!(text.contains("Howdy") && !text.contains("Hello"));
    }

    #[test]
    fn reconnecting_accepts_the_space_the_server_rejoins() {
        let mut conversation = joined(|state| {
            state.messages = vec![message("m1", Role::Assistant, "Hello")];
        });
        conversation.apply(Event::State(ConnectionState::Disconnected {
            reason: client::Disconnect::Closed,
            retry_in: Duration::from_secs(1),
        }));
        conversation.apply(Event::State(ConnectionState::Connecting { attempt: 2 }));
        assert_eq!(conversation.mode(), Mode::Space);
        conversation.apply(connected());
        // The client restores membership itself; nothing is asked of the server.
        assert!(conversation.take_actions().is_empty());
        conversation.apply(space(|state| {
            state.messages = vec![
                message("m1", Role::Assistant, "Hello"),
                message("m2", Role::Assistant, "Welcome back"),
            ];
        }));
        assert_eq!(conversation.activity(), SessionActivity::Ready);
        assert!(conversation.text().contains("Welcome back"));
    }

    #[test]
    fn a_space_lost_while_reconnecting_returns_to_the_lobby() {
        let mut conversation = joined(|_| {});
        conversation.apply(Event::State(ConnectionState::Disconnected {
            reason: client::Disconnect::Closed,
            retry_in: Duration::from_secs(1),
        }));
        conversation.apply(connected());
        conversation.apply(refused(SPACE));
        assert_eq!(conversation.mode(), Mode::Lobby);
        // Leaving keeps the client from asking for the space again on the next reconnect; the server lists spaces after it.
        assert_eq!(sent(&mut conversation), [ClientMessage::Leave]);
        conversation.apply(Event::Message(ServerMessage::Left));
        assert_eq!(conversation.mode(), Mode::Lobby);
        assert!(conversation.take_actions().is_empty());
        // The lobby says the space has ended, rather than showing the server's error, and B dismisses it before it leaves the server.
        let text = conversation.text();
        assert!(text.contains("Tomori has ended."), "{text}");
        assert!(!text.contains("ERROR"), "{text}");
        assert!(text.contains("B dismiss"), "{text}");
        conversation.back();
        assert_eq!(conversation.mode(), Mode::Lobby);
        assert!(conversation.take_actions().is_empty());
        let text = conversation.text();
        assert!(
            !text.contains("has ended") && text.contains("B servers"),
            "{text}"
        );
        conversation.back();
        assert_eq!(conversation.mode(), Mode::Servers);
    }

    #[test]
    fn an_ended_space_hidden_by_the_connection_status_is_not_offered_for_dismissal() {
        let mut conversation = joined(|_| {});
        conversation.apply(connected());
        conversation.apply(refused(SPACE));
        conversation.apply(Event::Message(ServerMessage::Left));
        assert!(conversation.text().contains("B dismiss"));
        conversation.apply(Event::State(ConnectionState::Connecting { attempt: 2 }));
        let text = conversation.text();
        assert!(
            !text.contains("has ended") && text.contains("B servers"),
            "{text}"
        );
    }

    #[test]
    fn another_error_while_rejoining_stays_in_the_space() {
        let mut conversation = joined(|_| {});
        conversation.apply(Event::State(ConnectionState::Disconnected {
            reason: client::Disconnect::Closed,
            retry_in: Duration::from_secs(1),
        }));
        conversation.apply(connected());
        // Such as a decision sent before the connection dropped, refused once it was delivered.
        conversation.apply(Event::Message(ServerMessage::Error {
            error: "That request is no longer pending.".into(),
            space: None,
        }));
        assert_eq!(conversation.mode(), Mode::Space);
        assert!(conversation.take_actions().is_empty());
        // Nor does a refusal of some other space.
        conversation.apply(refused("space-other"));
        assert_eq!(conversation.mode(), Mode::Space);
        conversation.apply(space(|_| {}));
        assert_eq!(conversation.mode(), Mode::Space);
    }

    #[test]
    fn a_refused_join_from_the_lobby_leaves() {
        let mut conversation = Conversation::new("Ada");
        conversation.connect(Choice::new("Studio", "10.0.0.1:8765", Source::Discovered));
        conversation.apply(connected());
        conversation.apply(Event::Message(ServerMessage::Spaces {
            spaces: vec![SpaceSummary {
                id: SPACE.into(),
                title: "Tomori".into(),
                backend: "claude-code".into(),
                directory: "/work/tomori".into(),
                environment: "sky".into(),
                activity: Activity::Idle,
                participants: 0,
            }],
        }));
        conversation.take_actions();
        conversation.accept();
        conversation.take_actions();
        conversation.apply(refused(SPACE));
        assert_eq!(conversation.mode(), Mode::Lobby);
        assert_eq!(sent(&mut conversation), [ClientMessage::Leave]);
        // The gone space is no longer offered while the listing that follows the leave is awaited.
        let text = conversation.text();
        assert!(!text.contains("Joining...") && !text.contains("> Tomori"));
        assert!(text.contains("SPACES 0/0"));
        assert!(text.contains("Tomori has ended."), "{text}");
    }

    #[test]
    fn a_refused_decision_in_a_space_stays_in_the_space() {
        let mut conversation = joined(|_| {});
        conversation.apply(Event::Message(ServerMessage::Error {
            error: "That approval was already decided.".into(),
            space: None,
        }));
        assert_eq!(conversation.mode(), Mode::Space);
    }

    #[test]
    fn the_server_rejoining_after_a_reconnect_in_the_lobby_enters_the_space() {
        let mut conversation = Conversation::new("Ada");
        conversation.connect(Choice::new("Studio", "10.0.0.1:8765", Source::Override));
        conversation.apply(connected());
        conversation.apply(space(|_| {}));
        assert_eq!(conversation.mode(), Mode::Space);
    }

    #[test]
    fn updates_sent_before_a_leave_was_handled_are_ignored() {
        let mut conversation = joined(|_| {});
        conversation.back();
        assert_eq!(sent(&mut conversation), [ClientMessage::Leave]);
        assert_eq!(conversation.mode(), Mode::Lobby);
        conversation.apply(space(|_| {}));
        assert_eq!(conversation.mode(), Mode::Lobby);
        conversation.apply(Event::Message(ServerMessage::Left));
        // Leaving is followed by a listing, so none is requested.
        assert!(conversation.take_actions().is_empty());
        conversation.apply(space(|_| {}));
        assert_eq!(conversation.mode(), Mode::Space);
    }

    #[test]
    fn being_left_by_the_server_returns_to_the_lobby_and_lists() {
        let mut conversation = joined(|_| {});
        conversation.apply(Event::Message(ServerMessage::Left));
        assert_eq!(conversation.mode(), Mode::Lobby);
        assert_eq!(sent(&mut conversation), [ClientMessage::ListSpaces]);
    }

    #[test]
    fn b_cancels_your_newest_queued_entry_before_leaving() {
        let mut conversation = joined(|state| {
            state.queue = vec![
                entry("1", "Ada", QueueState::Queued),
                entry("2", "Bea", QueueState::Queued),
                entry("3", "Ada", QueueState::Queued),
                entry("4", "Ada", QueueState::Sending),
            ];
        });
        assert!(conversation.text().contains("Queued: text of 3"));
        assert!(conversation.text().contains("B cancel queued"));
        conversation.back();
        assert_eq!(
            sent(&mut conversation),
            [ClientMessage::CancelQueued { entry: "3".into() }]
        );
        conversation.apply(space(|state| {
            state.queue = vec![
                entry("2", "Bea", QueueState::Queued),
                entry("4", "Ada", QueueState::Sending),
            ];
        }));
        // Others' entries and those already sending are not yours to cancel here.
        conversation.back();
        assert_eq!(sent(&mut conversation), [ClientMessage::Leave]);
    }

    #[test]
    fn another_participant_with_your_name_does_not_own_your_entries() {
        let mut conversation = joined(|state| {
            let mut namesake = entry("1", "Ada", QueueState::Queued);
            namesake.source = 2;
            state.queue = vec![namesake];
        });
        assert!(!conversation.text().contains("B cancel queued"));
        conversation.back();
        assert_eq!(sent(&mut conversation), [ClientMessage::Leave]);
    }

    #[test]
    fn your_entries_follow_the_number_the_server_gives_you() {
        let mut conversation = joined(|state| {
            state.queue = vec![entry("1", "Ada", QueueState::Queued)];
        });
        assert!(conversation.text().contains("B cancel queued"));
        // Still yours while the connection is down, since a reconnect keeps the number.
        conversation.apply(Event::State(ConnectionState::Connecting { attempt: 1 }));
        assert!(conversation.text().contains("B cancel queued"));
        // A server that numbers you differently, such as after it restarts, does not see the entry as yours.
        conversation.apply(Event::State(ConnectionState::Connected {
            source: 2,
            server: "studio".into(),
            role: ConnectionRole::Owner,
            certificate: None,
            backends: vec![],
            roots: vec![],
            environments: sky(),
        }));
        assert!(!conversation.text().contains("B cancel queued"));
        // Choosing another server forgets the number until it welcomes you.
        conversation.connect(Choice::new("Other", "10.0.0.2:8765", Source::Discovered));
        assert_eq!(conversation.source, None);
    }

    #[test]
    fn the_queue_is_shown_after_the_messages() {
        let mut conversation = joined(|state| {
            state.messages = vec![message("m1", Role::User, "Hi")];
            state.queue = vec![
                entry("1", "Bea", QueueState::Sending),
                entry("2", "Ada", QueueState::Queued),
            ];
        });
        assert!(conversation.text().contains("USER 1/1"));
        conversation.navigate(1, 0);
        let text = conversation.text();
        assert!(text.contains("QUEUE 2"), "{text}");
        assert!(text.contains("Bea [sending]: text of 1"), "{text}");
        assert!(text.contains("Ada [queued]: text of 2"), "{text}");
    }

    #[test]
    fn pages_through_long_messages_and_follows_new_ones_from_the_last_page() {
        let long = (0..30).map(|i| format!("line {i}\n")).collect::<String>();
        let mut conversation = joined(|state| {
            state.messages = vec![
                message("m1", Role::User, &long),
                message("m2", Role::Assistant, &long),
            ];
        });
        // Following shows the newest message's last page.
        let total = pages(&long).len();
        assert!(total > 1);
        assert!(
            conversation
                .text()
                .contains(&format!("p{total}/{total} ASSISTANT 2/2"))
        );
        conversation.navigate(-1, 0);
        assert!(
            conversation
                .text()
                .contains(&format!("p1/{total} USER 1/2"))
        );
        conversation.navigate(0, 1);
        assert!(
            conversation
                .text()
                .contains(&format!("p2/{total} USER 1/2"))
        );

        // While reading an older message, new ones do not move the view.
        conversation.apply(space(|state| {
            state.messages = vec![
                message("m1", Role::User, &long),
                message("m2", Role::Assistant, &long),
                message("m3", Role::Assistant, "New"),
            ];
        }));
        assert!(conversation.text().contains("USER 1/3"));

        // Returning to the newest page follows again.
        conversation.navigate(1, 0);
        conversation.navigate(1, 0);
        assert!(conversation.text().contains("ASSISTANT 3/3"));
        conversation.apply(space(|state| {
            state.messages = vec![
                message("m1", Role::User, &long),
                message("m2", Role::Assistant, &long),
                message("m3", Role::Assistant, "New"),
                message("m4", Role::Assistant, "Newer"),
            ];
        }));
        assert!(conversation.text().contains("ASSISTANT 4/4"));
        assert_fits(&mut conversation);
    }

    #[test]
    fn everything_fits_the_panel() {
        let long = "Very long reply ".repeat(1000);
        let mut conversation = joined(|state| {
            state.title = "A title far too long for one line of the panel".into();
            state.messages = vec![message("m1", Role::Assistant, &long)];
            state.queue = (0..20)
                .map(|i| entry(&i.to_string(), "Ada", QueueState::Queued))
                .collect();
            state.error = Some(long.clone());
        });
        assert_fits(&mut conversation);
        conversation.navigate(1, 0);
        assert_fits(&mut conversation);
        conversation.apply(space(|state| {
            let mut long_approval = approval(&[("a", &long)]);
            long_approval.text = long.clone();
            state.approval = Some(long_approval);
        }));
        conversation.choose(1);
        assert_fits(&mut conversation);
        conversation.apply(space(|state| state.question = Some(question())));
        assert!(
            conversation
                .text()
                .contains("Answer this question on the PC.")
        );
        assert_fits(&mut conversation);

        let mut conversation = Conversation::new("Ada");
        conversation.set_servers(
            (0..20)
                .map(|i| Choice::new(long.clone(), format!("10.0.0.{i}:8765"), Source::Discovered))
                .collect(),
        );
        conversation.navigate(0, 15);
        assert!(conversation.text().contains("SERVERS 16/20"));
        assert!(conversation.text().contains("> "));
        assert_fits(&mut conversation);
        conversation.accept();
        conversation.fail(long.clone());
        assert_fits(&mut conversation);
        conversation.apply(Event::Message(ServerMessage::Spaces {
            spaces: (0..20)
                .map(|i| SpaceSummary {
                    id: i.to_string(),
                    title: long.clone(),
                    backend: "claude-code".into(),
                    directory: "/".into(),
                    environment: "sky".into(),
                    activity: Activity::Unknown,
                    participants: 0,
                })
                .collect(),
        }));
        conversation.navigate(0, 19);
        assert_fits(&mut conversation);

        let paged = pages(&(0..200).map(|i| format!("line {i}\n")).collect::<String>());
        assert!(paged.last().unwrap().last().unwrap().contains("199"));
        assert!(short(&"🦀".repeat(200)).len() <= COLUMNS);
    }

    fn modes(current: &str) -> impl FnOnce(&mut SpaceState) {
        let current = current.to_owned();
        move |state| {
            state.mode = Some(current);
            state.modes = [
                ("default", "Manual"),
                ("acceptEdits", "Accept edits"),
                ("plan", "Plan"),
                ("auto", "Auto"),
            ]
            .map(|(id, name)| protocol::Mode {
                id: id.into(),
                name: name.into(),
                description: None,
            })
            .into();
        }
    }

    #[test]
    fn y_steps_through_the_agents_modes_in_its_order() {
        let mut conversation = joined(modes("auto"));
        assert!(
            conversation
                .text()
                .lines()
                .next()
                .unwrap()
                .starts_with("[idle | Auto] Tomori")
        );
        assert!(conversation.text().ends_with("Y mode"));
        assert_fits(&mut conversation);
        let mut stepped = Vec::new();
        for confirmed in ["auto", "default", "acceptEdits", "plan"] {
            conversation.apply(space(modes(confirmed)));
            conversation.refresh();
            stepped.extend(sent(&mut conversation));
        }
        let mode = |mode: &str| ClientMessage::SetMode { mode: mode.into() };
        assert_eq!(
            stepped,
            [
                mode("default"),
                mode("acceptEdits"),
                mode("plan"),
                mode("auto")
            ]
        );

        // Until the agent confirms, another press asks for the same mode again.
        conversation.refresh();
        assert_eq!(sent(&mut conversation), [mode("auto")]);

        // A mode the agent is in but Tomori cannot step to is still shown, and steps to the first.
        conversation.apply(space(modes("dontAsk")));
        assert!(conversation.text().starts_with("[idle | dontAsk]"));
        conversation.refresh();
        assert_eq!(sent(&mut conversation), [mode("default")]);
    }

    #[test]
    fn y_fetches_the_space_again_when_the_agent_offers_no_modes() {
        let mut conversation = joined(|_| {});
        assert!(conversation.text().starts_with("[idle] Tomori"));
        assert!(conversation.text().ends_with("Y refresh"));
        conversation.refresh();
        assert_eq!(
            sent(&mut conversation),
            [ClientMessage::Join {
                space: SPACE.into()
            }]
        );
    }

    #[test]
    fn b_in_the_lobby_returns_to_the_server_list() {
        let mut conversation = Conversation::new("Ada");
        conversation.connect(Choice::new("Studio", "10.0.0.1:8765", Source::Discovered));
        conversation.take_actions();
        conversation.back();
        assert_eq!(conversation.take_actions(), [Action::Disconnect]);
        assert!(conversation.is_browsing());
        assert_eq!(conversation.server(), None);
        conversation.refresh();
        assert_eq!(conversation.take_actions(), [Action::Rescan]);
    }

    fn transcript(utterance: &str, text: &str, finished: bool) -> Event {
        Event::Message(ServerMessage::Transcript {
            utterance: utterance.into(),
            text: text.into(),
            finished,
        })
    }

    /// Press X and return the utterance it began.
    fn press(conversation: &mut Conversation) -> String {
        conversation.talk(true);
        match conversation.take_actions().as_slice() {
            [Action::BeginUtterance(utterance, _)] => utterance.clone(),
            actions => panic!("{actions:?}"),
        }
    }

    #[test]
    fn holding_x_records_an_utterance_and_releasing_sends_it() {
        let mut conversation = joined(|_| {});
        assert!(conversation.text().contains("Mic off"));
        assert!(conversation.text().contains("X talk"));
        let utterance = press(&mut conversation);
        assert!(conversation.recording().is_some());
        assert!(conversation.text().contains("MIC AGENT: listening..."));
        assert!(conversation.text().contains("Release X to send"));
        // Held: no second utterance.
        conversation.talk(true);
        assert!(conversation.take_actions().is_empty());

        conversation.apply(transcript(&utterance, "hello the", false));
        assert!(conversation.text().contains("MIC AGENT: hello the"));
        conversation.talk(false);
        assert_eq!(conversation.take_actions(), [Action::EndUtterance]);
        assert_eq!(conversation.recording(), None);
        assert!(conversation.text().contains("Sending: hello the..."));
        conversation.apply(transcript(&utterance, "hello there", false));
        assert!(conversation.text().contains("Sending: hello there..."));
        conversation.apply(transcript(&utterance, "Hello there.", true));
        assert!(conversation.text().contains("Mic off"));
        // Releasing again does nothing.
        conversation.talk(false);
        assert!(conversation.take_actions().is_empty());
    }

    #[test]
    fn live_toggle_stays_visible_and_x_returns_to_live() {
        let mut conversation = joined(|_| {});
        conversation.toggle_live();
        assert_eq!(conversation.take_actions(), [Action::StartLive(0)]);
        assert!(conversation.text().contains("MIC LIVE"));
        let utterance = press(&mut conversation);
        assert!(conversation.text().contains("MIC AGENT"));
        conversation.talk(false);
        assert_eq!(conversation.take_actions(), [Action::EndUtterance]);
        assert!(conversation.text().contains("MIC LIVE"));
        conversation.apply(transcript(&utterance, "hello", true));
        assert!(conversation.text().contains("MIC LIVE"));
        conversation.toggle_live();
        assert_eq!(conversation.take_actions(), [Action::StopLive]);
        assert!(conversation.text().contains("Mic off"));
    }

    #[test]
    fn live_stops_on_focus_loss_disconnect_and_microphone_failure() {
        let mut conversation = joined(|_| {});
        conversation.toggle_live();
        conversation.take_actions();
        conversation.stop_live();
        assert_eq!(conversation.take_actions(), [Action::StopLive]);
        assert!(!conversation.live);
        conversation.toggle_live();
        conversation.take_actions();
        conversation.microphone_failed("no microphone");
        assert_eq!(conversation.take_actions(), [Action::StopLive]);
        assert!(!conversation.live);
        conversation.toggle_live();
        conversation.take_actions();
        conversation.apply(Event::State(ConnectionState::Closed));
        assert!(conversation.take_actions().contains(&Action::StopLive));
        assert!(!conversation.live);
    }

    #[test]
    fn focus_loss_cancels_queued_microphone_starts() {
        let mut conversation = joined(|_| {});
        conversation.toggle_live();
        conversation.talk(true);
        conversation.stop_live();
        conversation.stop_talking();
        assert!(conversation.take_actions().is_empty());
        assert!(!conversation.live);
        assert!(conversation.recording().is_none());
    }

    #[test]
    fn failed_live_start_removes_a_later_queued_agent_start() {
        let mut conversation = joined(|_| {});
        conversation.toggle_live();
        conversation.talk(true);
        assert!(matches!(
            conversation.next_action(),
            Some(Action::StartLive(_))
        ));
        conversation.microphone_failed("capture failed");
        assert_eq!(conversation.next_action(), Some(Action::StopLive));
        assert_eq!(conversation.next_action(), None);
        assert!(conversation.recording().is_none());
    }

    #[test]
    fn live_mute_precedes_a_queued_agent_end() {
        let mut conversation = joined(|_| {});
        conversation.toggle_live();
        conversation.take_actions();
        press(&mut conversation);
        conversation.talk(false);
        conversation.stop_live();
        assert_eq!(
            conversation.take_actions(),
            [Action::StopLive, Action::EndUtterance]
        );
    }

    #[test]
    fn queued_microphone_actions_keep_their_original_connection_generation() {
        let mut conversation = joined(|_| {});
        conversation.set_capture_generation(7);
        conversation.toggle_live();
        conversation.set_capture_generation(8);
        assert_eq!(conversation.take_actions(), [Action::StartLive(7)]);
        conversation.talk(true);
        conversation.set_capture_generation(9);
        assert!(matches!(
            conversation.next_action(),
            Some(Action::BeginUtterance(_, 8))
        ));
    }

    #[test]
    fn capture_failure_before_a_queued_end_cancels_instead() {
        let mut conversation = joined(|_| {});
        conversation.toggle_live();
        conversation.take_actions();
        press(&mut conversation);
        conversation.talk(false);
        conversation.microphone_failed("capture failed");
        assert_eq!(
            conversation.take_actions(),
            [Action::StopLive, Action::CancelUtterance]
        );
        assert!(conversation.text().contains("capture failed"));
        assert!(!conversation.live);
    }

    #[test]
    fn a_live_failure_after_an_utterance_was_ended_keeps_its_pending_transcript() {
        let mut conversation = joined(|_| {});
        conversation.toggle_live();
        conversation.take_actions();
        press(&mut conversation);
        conversation.talk(false);
        // The end has been performed: the utterance is with the server.
        conversation.take_actions();
        assert!(matches!(conversation.talk, Some(Talk::Sending { .. })));
        // The live capture reopened afterwards fails.
        conversation.microphone_failed("capture failed");
        assert!(matches!(conversation.talk, Some(Talk::Sending { .. })));
        assert_eq!(conversation.take_actions(), [Action::StopLive]);
        assert!(!conversation.live);
        // The panel no longer claims a live microphone, and the error is kept
        // to show once the pending transcript is resolved.
        assert!(!conversation.text().contains("MIC LIVE"));
        assert_eq!(conversation.error.as_deref(), Some("capture failed"));
        let utterance = conversation.talk.as_ref().unwrap().utterance().to_owned();
        conversation.apply(transcript(&utterance, "hello", true));
        assert!(conversation.talk.is_none());
        assert!(conversation.text().contains("capture failed"));
    }

    #[test]
    fn an_utterance_whose_end_failed_is_not_left_sending() {
        let mut conversation = joined(|_| {});
        conversation.toggle_live();
        conversation.take_actions();
        press(&mut conversation);
        conversation.talk(false);
        // The end was taken to be performed, and failed while stopping.
        assert_eq!(conversation.take_actions(), [Action::EndUtterance]);
        conversation.utterance_not_sent("Microphone lost audio; not sent.");
        assert!(conversation.talk.is_none());
        assert_eq!(conversation.take_actions(), [Action::StopLive]);
        assert!(
            conversation
                .text()
                .contains("Microphone lost audio; not sent."),
            "{}",
            conversation.text()
        );
    }

    #[test]
    fn a_failed_end_also_withdraws_a_later_utterance_queued_behind_it() {
        let mut conversation = joined(|_| {});
        conversation.toggle_live();
        conversation.take_actions();
        press(&mut conversation);
        conversation.talk(false);
        // A second press and release land before the first end is performed.
        conversation.talk(true);
        conversation.talk(false);
        assert!(matches!(
            conversation.next_action(),
            Some(Action::EndUtterance)
        ));
        conversation.utterance_not_sent("Microphone lost audio; not sent.");
        assert!(conversation.talk.is_none());
        // The second utterance never began, so nothing of it is performed.
        assert_eq!(conversation.take_actions(), [Action::StopLive]);
    }

    #[test]
    fn capture_failure_discards_an_unstarted_utterance_and_its_end() {
        let mut conversation = joined(|_| {});
        conversation.toggle_live();
        conversation.take_actions();
        conversation.talk(true);
        conversation.talk(false);
        conversation.microphone_failed("capture failed");
        assert_eq!(conversation.take_actions(), [Action::StopLive]);
        assert!(conversation.text().contains("capture failed"));
    }

    #[test]
    fn utterances_are_unique_within_and_across_launches() {
        let mut conversation = joined(|_| {});
        let first = press(&mut conversation);
        conversation.talk(false);
        conversation.take_actions();
        let second = press(&mut conversation);
        assert_ne!(first, second);
        std::thread::sleep(Duration::from_millis(1));
        let mut relaunched = joined(|_| {});
        assert_ne!(press(&mut relaunched), first);
    }

    #[test]
    fn a_failed_transcript_is_shown() {
        let mut conversation = joined(|_| {});
        let utterance = press(&mut conversation);
        conversation.talk(false);
        conversation.take_actions();
        conversation.apply(Event::Message(ServerMessage::TranscriptFailed {
            utterance,
            error: "Nothing was heard, so nothing was sent.".into(),
        }));
        assert!(conversation.text().contains("ERROR: Nothing was heard"));
        // The next press clears it.
        press(&mut conversation);
        assert!(conversation.text().contains("MIC AGENT"));
    }

    #[test]
    fn a_failure_while_recording_closes_the_microphone() {
        let mut conversation = joined(|_| {});
        let utterance = press(&mut conversation);
        conversation.apply(Event::Message(ServerMessage::TranscriptFailed {
            utterance: utterance.clone(),
            error: "Some of the recording was lost, so nothing was sent. Please try again.".into(),
        }));
        // The server has already ended it, so it is not cancelled.
        assert_eq!(conversation.take_actions(), [Action::AbandonUtterance]);
        assert_eq!(conversation.recording(), None);
        conversation.talk(false);
        assert!(conversation.take_actions().is_empty());
        assert!(
            conversation
                .text()
                .contains("Some of the recording was lost")
        );

        let utterance = press(&mut conversation);
        conversation.apply(Event::UtteranceFailed(client::FailedUtterance {
            utterance,
            failure: UtteranceFailure::Congested,
        }));
        assert_eq!(conversation.take_actions(), [Action::CancelUtterance]);
        assert!(conversation.text().contains("Network too slow"));

        press(&mut conversation);
        conversation.microphone_failed("No microphone is available.");
        assert_eq!(conversation.take_actions(), [Action::CancelUtterance]);
        assert!(conversation.text().contains("No microphone is available."));
    }

    #[test]
    fn losing_focus_the_connection_or_the_space_cancels_recording() {
        let mut conversation = joined(|_| {});
        press(&mut conversation);
        conversation.stop_talking();
        assert_eq!(conversation.take_actions(), [Action::CancelUtterance]);
        // A released utterance is still transcribed.
        press(&mut conversation);
        conversation.talk(false);
        conversation.take_actions();
        conversation.stop_talking();
        assert!(conversation.take_actions().is_empty());

        press(&mut conversation);
        conversation.apply(Event::State(ConnectionState::Disconnected {
            reason: client::Disconnect::Timeout,
            retry_in: Duration::from_secs(1),
        }));
        assert_eq!(conversation.take_actions(), [Action::CancelUtterance]);
        assert!(conversation.text().contains("Connection lost"));
        // Offline, nothing is recorded.
        conversation.talk(true);
        assert!(conversation.take_actions().is_empty());
        assert!(conversation.text().contains("Offline; cannot record."));

        conversation.apply(connected());
        conversation.take_actions();
        press(&mut conversation);
        conversation.back();
        assert_eq!(
            conversation.take_actions(),
            [Action::CancelUtterance, Action::Send(ClientMessage::Leave)]
        );
    }

    #[test]
    fn x_outside_a_space_explains_itself() {
        let mut conversation = Conversation::new("Ada");
        conversation.connect(Choice::new("Studio", "10.0.0.1:8765", Source::Discovered));
        conversation.apply(connected());
        conversation.take_actions();
        conversation.talk(true);
        assert!(conversation.take_actions().is_empty());
        assert!(
            conversation
                .text()
                .contains("Join a space to talk to the agent.")
        );
    }

    #[test]
    fn the_recording_panel_fits() {
        let long = "word ".repeat(200);
        let mut conversation = joined(|_| {});
        let utterance = press(&mut conversation);
        conversation.apply(transcript(&utterance, &long, false));
        assert_fits(&mut conversation);
        conversation.talk(false);
        assert_fits(&mut conversation);
    }

    #[test]
    fn the_newest_words_heard_stay_in_view() {
        let mut conversation = joined(|_| {});
        let utterance = press(&mut conversation);
        let heard = format!("{} the newest words", "older words ".repeat(10));
        conversation.apply(transcript(&utterance, &heard, false));
        assert!(conversation.text().contains("MIC AGENT: ..."));
        assert!(conversation.text().contains("the newest words"));
        conversation.talk(false);
        assert!(conversation.text().contains("the newest words..."));
    }

    #[test]
    fn a_late_failure_of_an_earlier_utterance_is_shown_after_the_current_one() {
        let mut conversation = joined(|_| {});
        let first = press(&mut conversation);
        conversation.talk(false);
        conversation.take_actions();
        let second = press(&mut conversation);
        conversation.apply(Event::Message(ServerMessage::TranscriptFailed {
            utterance: first,
            error: "Nothing was heard, so nothing was sent.".into(),
        }));
        assert!(conversation.text().contains("MIC AGENT"));
        conversation.talk(false);
        assert!(conversation.text().contains("Sending: "));
        // Once the current words are sent, the earlier failure is shown.
        conversation.apply(transcript(&second, "Run the tests.", true));
        assert!(conversation.text().contains("ERROR: Nothing was heard"));
    }

    #[test]
    fn leaving_the_server_forgets_words_being_sent() {
        let mut conversation = joined(|_| {});
        press(&mut conversation);
        conversation.talk(false);
        conversation.take_actions();
        conversation.connect(Choice::new("Other", "10.0.0.2:8765", Source::Discovered));
        assert_eq!(conversation.talk, None);

        let mut conversation = joined(|_| {});
        press(&mut conversation);
        conversation.talk(false);
        conversation.take_actions();
        // Leave the space, then the server.
        conversation.back();
        conversation.back();
        assert_eq!(conversation.mode, Mode::Servers);
        assert_eq!(conversation.talk, None);
    }

    #[test]
    fn keeps_the_selection_when_lists_change() {
        let mut conversation = Conversation::new("Ada");
        conversation.set_servers(vec![
            Choice::new("A", "a", Source::Discovered),
            Choice::new("B", "b", Source::Discovered),
        ]);
        conversation.navigate(0, 1);
        conversation.set_servers(vec![
            Choice::new("C", "c", Source::Discovered),
            Choice::new("A", "a", Source::Discovered),
            Choice::new("B", "b", Source::Discovered),
        ]);
        conversation.accept();
        assert!(matches!(
            conversation.take_actions().as_slice(),
            [Action::Connect(choice)] if choice.address == "b"
        ));
    }

    #[test]
    fn selects_the_used_server_again_when_browsing_finds_it_after_returning() {
        let mut conversation = Conversation::new("Ada");
        let usb = Choice::new("USB", USB, Source::Usb);
        let servers = vec![
            Choice::new("A", "a", Source::Discovered),
            Choice::new("B", "b", Source::Discovered),
            usb.clone(),
        ];
        conversation.set_servers(servers.clone());
        conversation.navigate(0, 1);
        conversation.accept();
        conversation.back();
        conversation.take_actions();
        // Browsing restarts with only the USB entry, then finds the servers again.
        conversation.set_servers(vec![usb.clone()]);
        conversation.set_servers(servers);
        conversation.accept();
        assert!(matches!(
            conversation.take_actions().as_slice(),
            [Action::Connect(choice)] if choice.address == "b"
        ));
        // Moving the selection while the server is missing forgets it.
        conversation.back();
        conversation.set_servers(vec![usb.clone()]);
        conversation.navigate(0, 1);
        conversation.set_servers(vec![usb.clone(), Choice::new("B", "b", Source::Discovered)]);
        assert!(conversation.text().contains("> USB"));
    }

    #[test]
    fn connecting_to_another_server_while_the_chosen_one_is_missing_forgets_it() {
        let mut conversation = Conversation::new("Ada");
        let usb = Choice::new("USB", USB, Source::Usb);
        let b = Choice::new("B", "b", Source::Discovered);
        conversation.set_servers(vec![usb.clone(), b.clone()]);
        conversation.navigate(0, 1);
        conversation.accept();
        conversation.back();
        // B is missing when browsing restarts, and A connects to USB instead.
        conversation.set_servers(vec![usb.clone()]);
        conversation.accept();
        conversation.back();
        conversation.take_actions();
        conversation.set_servers(vec![usb.clone()]);
        conversation.set_servers(vec![b, usb]);
        let text = conversation.text();
        assert!(text.contains("> USB"), "{text}");
    }

    fn remembered() -> Choice {
        let mut choice = Choice::new("Studio", "10.0.0.2:8765", Source::Remembered);
        choice.instance = Some("studio".into());
        choice
    }

    #[test]
    fn connects_to_the_remembered_server_when_it_appears_but_only_once() {
        let mut conversation = Conversation::new("Ada");
        conversation.set_servers(vec![Choice::new("USB", USB, Source::Usb)]);
        assert!(conversation.take_actions().is_empty());
        conversation.set_servers(vec![remembered(), Choice::new("USB", USB, Source::Usb)]);
        assert!(matches!(
            conversation.take_actions().as_slice(),
            [Action::Connect(choice)] if choice.address == "10.0.0.2:8765"
        ));
        assert_eq!(conversation.mode(), Mode::Lobby);
        // Returning to the list leaves the choice to the participant.
        conversation.back();
        conversation.take_actions();
        conversation.set_servers(vec![remembered(), Choice::new("USB", USB, Source::Usb)]);
        assert!(conversation.take_actions().is_empty());
        assert_eq!(conversation.mode(), Mode::Servers);
    }

    #[test]
    fn moving_the_selection_stops_the_remembered_server_connecting() {
        let mut conversation = Conversation::new("Ada");
        conversation.set_servers(vec![
            Choice::new("Other", "10.0.0.3:8765", Source::Discovered),
            Choice::new("USB", USB, Source::Usb),
        ]);
        conversation.navigate(0, 1);
        conversation.set_servers(vec![
            remembered(),
            Choice::new("Other", "10.0.0.3:8765", Source::Discovered),
            Choice::new("USB", USB, Source::Usb),
        ]);
        assert!(conversation.take_actions().is_empty());
        assert_eq!(conversation.mode(), Mode::Servers);
    }

    #[test]
    fn selects_the_best_server_until_one_is_chosen() {
        let mut conversation = Conversation::new("Ada");
        conversation.set_servers(vec![Choice::new("USB", USB, Source::Usb)]);
        conversation.set_servers(vec![
            Choice::new("Studio", "10.0.0.2:8765", Source::Discovered),
            Choice::new("USB", USB, Source::Usb),
        ]);
        conversation.accept();
        assert!(matches!(
            conversation.take_actions().as_slice(),
            [Action::Connect(choice)] if choice.address == "10.0.0.2:8765"
        ));
    }
}
