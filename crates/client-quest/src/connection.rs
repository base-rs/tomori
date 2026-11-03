// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Carries out the [`Conversation`]'s actions: owns the server connection, the microphone and the speaker, and browses for servers while the server list shows.
use crate::{
    capture::{Capture, Recording},
    controller,
    conversation::{Action, Conversation},
    hands,
    identity::{Identity, Store},
    playback::{Playback, Speaking},
    server_choice::{self, Choice, Source},
};
use client::{
    Client, ConnectionState, Disconnect, Event, Fingerprint, Microphone, Options, Transport,
    discovery,
    playback::{MixerOptions, Player, PlayerControl},
};
use protocol::{ClientMessage, Gate, Pose, ServerMessage};
use std::{
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

/// Permission to receive multicast while browsing.
pub trait Multicast {
    fn set_held(&mut self, held: bool);
}

/// Desktop systems deliver multicast without a lock.
impl Multicast for () {
    fn set_held(&mut self, _held: bool) {}
}

pub struct Connection {
    identity: Identity,
    store: Store,
    multicast: Box<dyn Multicast>,
    client: Option<Client>,
    browser: Option<discovery::Browser>,
    servers: discovery::Servers,
    /// Whether the server list needs rebuilding.
    changed: bool,
    /// Browsing failed; it is tried again on request rather than every frame.
    failed: bool,
    remembered: Option<String>,
    voice: Voice,
    speaker: Speaker,
    /// Receives other participants' presence reports, which the conversation ignores. Set by a spectator that renders them.
    presence: Option<PresenceHook>,
    /// The server connected to, and how, for keeping its certificate and token.
    link: Option<Link>,
}

/// How this device is connected to a server, so that what it learns can be kept for that server. See `design/security.md`.
struct Link {
    address: String,
    /// The server's discovered instance name, or its address, under which its certificate and token are kept.
    key: String,
    transport: Transport,
    token: Option<String>,
    /// Whether the token is the one given at startup, kept for this server once a connection over TLS succeeds.
    pending_token: bool,
    /// Whether the server's certificate is already kept for it.
    pinned: bool,
    /// The server presented a different certificate from the one kept: it is not trusted, so nothing it presents is kept.
    distrusted: bool,
    /// Asked to be admitted only to pair, since the server refuses visitors.
    pairing: bool,
}

/// What to do once the client reported why it disconnected.
enum Reopen {
    /// The server refused the token: connect again without it.
    WithoutToken,
    /// The server's certificate changed: connect again as a visitor, sending no token, and keep nothing.
    Distrusted,
    /// The server refuses visitors: connect again asking to pair.
    ToPair,
}

/// How to connect to `choice`, given what is kept for it: pin the certificate kept, or else the one it advertises; with neither, try TLS and fall back to plain HTTP.
fn transport(kept: Option<&str>, advertised: Option<&str>) -> (Transport, bool) {
    let parse =
        |fingerprint: Option<&str>| fingerprint.and_then(|text| text.parse::<Fingerprint>().ok());
    match (parse(kept), parse(advertised)) {
        (Some(pin), _) => (Transport::Tls { pin: Some(pin) }, true),
        (None, Some(pin)) => (Transport::Tls { pin: Some(pin) }, false),
        (None, None) => (Transport::Automatic { pin: None }, false),
    }
}

/// A hook that renders the other participants' presence reports, which the conversation ignores.
type PresenceHook = Box<dyn FnMut(&ServerMessage) + Send>;

/// Why ending push-to-talk failed: the utterance itself, or only the live
/// microphone reopened after it was sent.
#[derive(Debug, PartialEq)]
enum Finish {
    Utterance(String),
    Live(String),
}

impl From<Finish> for String {
    fn from(finish: Finish) -> Self {
        match finish {
            Finish::Utterance(error) | Finish::Live(error) => error,
        }
    }
}

/// Live and push-to-talk audio through the client's [`Microphone`].
struct Voice {
    capture: Box<dyn Capture>,
    /// Shared with the capture thread.
    shared: Arc<Mutex<Captured>>,
    recording: Option<Recording>,
    live: bool,
    generation: Option<u64>,
}

#[derive(Default)]
struct Captured {
    /// For the current client, at the rate of the last recording.
    microphone: Option<Microphone>,
    /// Why capture stopped early.
    failure: Option<String>,
}

impl Voice {
    fn lock(&self) -> std::sync::MutexGuard<'_, Captured> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn open(&mut self, client: &Client, live: bool, generation: u64) -> Result<(), String> {
        // Never rearm a new connection while an old capture callback could
        // still be in flight. Its recording must be stopped and joined first.
        if self.recording.is_some() && self.generation != Some(generation) {
            return Err("Connection lost; microphone stopped.".into());
        }
        if !client.sender().arm_microphone(generation) {
            return Err("Connection lost; microphone stopped.".into());
        }
        if self.recording.is_some() {
            return Ok(());
        }
        match self.capture.permit()? {
            true => {}
            false => {
                return Err(if live {
                    "Allow the microphone, then squeeze the left grip again."
                } else {
                    "Allow the microphone, then hold X again."
                }
                .into());
            }
        }
        {
            let mut captured = self.lock();
            captured.failure = None;
            // Bind before the device opens, so the first samples are already
            // tied to this connection.
            if let Some(microphone) = captured.microphone.as_mut() {
                microphone.bind_connection(generation);
            }
        }
        let shared = self.shared.clone();
        let samples = Box::new(move |samples: &[f32]| {
            let mut captured = shared.lock().unwrap_or_else(PoisonError::into_inner);
            // A capture only feeds a microphone bound to its own connection;
            // the outbox drops its frames once that connection has ended.
            if let Some(microphone) = captured.microphone.as_mut()
                && microphone.connection() == Some(generation)
            {
                // Only fails once the client is closed, when nothing more can be sent.
                let _ = microphone.push_float(samples);
            }
        });
        let shared = self.shared.clone();
        let failed = Box::new(move |error: String| {
            log::warn!("{error}");
            let mut captured = shared.lock().unwrap_or_else(PoisonError::into_inner);
            captured.failure.get_or_insert(error);
        });
        let sender = client.sender();
        let recording = if live {
            self.capture.open_live(
                samples,
                failed,
                Box::new(move || sender.connection_is_current(generation)),
            )?
        } else {
            self.capture.open(samples, failed)?
        };
        let mut captured = self.lock();
        let mut microphone = match captured.microphone.take() {
            Some(microphone) if microphone.sample_rate() == recording.sample_rate => microphone,
            _ => Microphone::new(client.sender(), recording.sample_rate),
        };
        microphone.bind_connection(generation);
        captured.microphone = Some(microphone);
        drop(captured);
        self.recording = Some(recording);
        self.generation = Some(generation);
        Ok(())
    }

    /// Ask for the microphone ahead of its first use, so that the platform's
    /// request does not interrupt the first recording. Opening the microphone
    /// still checks, and explains how to allow it if it was refused.
    fn request_permission(&mut self) -> Result<bool, String> {
        self.capture.request()
    }

    fn start_live(&mut self, client: &Client, generation: u64) -> Result<(), String> {
        self.open(client, true, generation)?;
        let result = self
            .lock()
            .microphone
            .as_mut()
            .unwrap()
            .set_gate(Gate::Live);
        if let Err(error) = result {
            self.recording = None;
            return Err(format!("Cannot send: {error}"));
        }
        self.live = true;
        Ok(())
    }

    fn stop_live(&mut self) -> Result<(), String> {
        self.live = false;
        let talking = self.talking();
        if let Some(microphone) = self.lock().microphone.as_mut() {
            microphone
                .set_gate(Gate::Off)
                .map_err(|error| format!("Cannot send: {error}"))?;
        }
        if !talking {
            self.recording = None;
        }
        Ok(())
    }

    fn talking(&self) -> bool {
        self.lock()
            .microphone
            .as_ref()
            .is_some_and(|microphone| microphone.utterance().is_some())
    }

    fn begin(&mut self, client: &Client, utterance: String, generation: u64) -> Result<(), String> {
        let resume_live = self.live && self.recording.is_some();
        if resume_live {
            if self.generation != Some(generation) || !client.sender().arm_microphone(generation) {
                return Err("Connection lost; microphone stopped.".into());
            }
            // Finish the in-flight live read before changing its gate: samples
            // captured before X must never be transcribed for the agent.
            self.recording = None;
            if let Some(error) = self.lock().failure.take() {
                return Err(error);
            }
        } else {
            self.open(client, false, generation)?;
        }
        let result = self.lock().microphone.as_mut().unwrap().begin(utterance);
        if let Err(error) = result {
            self.recording = None;
            return Err(format!("Cannot send: {error}"));
        }
        // Agent audio is captured as push-to-talk always is, for recognition;
        // `finish` reopens live capture afterwards.
        if resume_live && let Err(error) = self.open(client, false, generation) {
            if let Some(microphone) = self.lock().microphone.as_mut() {
                let _ = microphone.cancel();
            }
            return Err(error);
        }
        Ok(())
    }

    /// Close the microphone, after which every captured sample has been passed on, then end or cancel the utterance.
    fn finish(&mut self, client: Option<&Client>, send: bool) -> Result<(), Finish> {
        let resume_live = self.live;
        let generation = self.generation;
        // Join the capture thread before changing the gate or checking for
        // failure; no in-flight read may become the next mode's audio.
        self.recording = None;
        let mut captured = self.lock();
        let failure = if send { captured.failure.take() } else { None };
        let Some(microphone) = captured.microphone.as_mut() else {
            return failure.map_or(Ok(()), |error| Err(Finish::Utterance(error)));
        };
        if let Some(error) = failure {
            microphone
                .cancel()
                .map_err(|error| Finish::Utterance(format!("Cannot cancel: {error}")))?;
            return Err(Finish::Utterance(error));
        }
        let result = if send {
            microphone.end()
        } else {
            microphone.cancel()
        };
        result.map_err(|error| Finish::Utterance(format!("Cannot send: {error}")))?;
        drop(captured);
        if resume_live {
            let lost = || Finish::Live("Connection lost; live microphone stopped.".into());
            let client = client.ok_or_else(lost)?;
            let generation = generation.ok_or_else(lost)?;
            self.open(client, true, generation).map_err(Finish::Live)?;
        }
        Ok(())
    }

    /// Close the microphone and forget the utterance the server has already ended.
    fn abandon(&mut self) -> Result<(), String> {
        if !self.live {
            self.recording = None;
        }
        let mut captured = self.lock();
        let Some(microphone) = captured.microphone.as_mut() else {
            return Ok(());
        };
        microphone
            .abandon()
            .map_err(|error| format!("Cannot send: {error}"))
    }

    /// The client is going away: close the microphone and forget it.
    fn reset(&mut self) {
        self.recording = None;
        self.live = false;
        self.generation = None;
        self.lock().microphone = None;
    }

    /// Why the open microphone stopped, if it did.
    fn failure(&mut self) -> Option<String> {
        self.recording.as_ref()?;
        self.lock().failure.take()
    }
}

/// How long to wait before opening the speaker again after it failed.
const SPEAKER_RETRY: Duration = Duration::from_secs(5);

/// Other participants and the agent's speech, played through the platform speaker by the client's [`Player`]. The speaker opens once the server welcomes the client and stays open until the client goes away.
struct Speaker {
    playback: Arc<dyn Playback>,
    /// The player of the current client, shared with the output callback, and its control.
    player: Option<(crate::playback::Shared, PlayerControl)>,
    /// The speaker thread, while it is opening or playing.
    output: Option<Speaking>,
    /// Why the output failed to open or stopped, set from the speaker thread.
    failure: Arc<Mutex<Option<String>>>,
    /// When to try opening the speaker again.
    retry_at: Option<Instant>,
    /// Whether a failure has been shown since the speaker last opened, so that retries do not repeat it.
    reported: bool,
}

impl Speaker {
    fn new(playback: Box<dyn Playback>) -> Self {
        Self {
            playback: Arc::from(playback),
            player: None,
            output: None,
            failure: Arc::default(),
            retry_at: None,
            reported: false,
        }
    }

    /// Take the new client's received audio; nothing plays until [`Self::open`].
    fn attach(&mut self, client: &mut Client) {
        self.detach();
        if let Some(audio) = client.take_audio() {
            let (player, control) = Player::new(audio, MixerOptions::default());
            self.player = Some((Arc::new(Mutex::new(player)), control));
        }
    }

    /// The client is going away: stop playing and forget its audio.
    fn detach(&mut self) {
        self.output = None;
        if let Some((_, control)) = self.player.take() {
            let statistics = control.statistics();
            if statistics != Default::default() {
                log::info!("Playback: {statistics:?}");
            }
        }
        self.retry_at = None;
        self.reported = false;
    }

    fn control(&self) -> Option<&PlayerControl> {
        self.player.as_ref().map(|(_, control)| control)
    }

    /// The server welcomed the client: a speaker that failed is tried again now, and shows its failure again if it fails, because the welcome clears what the conversation shows.
    fn connected(&mut self) {
        if self.output.is_none() {
            self.retry_at = None;
            self.reported = false;
        }
    }

    /// Start opening the speaker on its own thread if it is not open and is due. Failing to open is reported later by [`Self::poll`]. Returns a failure to show, once until the speaker opens again.
    fn open(&mut self) -> Option<String> {
        let (player, control) = self.player.as_ref()?;
        if self.output.is_some() || self.retry_at.is_some_and(|at| Instant::now() < at) {
            return None;
        }
        // Audio queued while nothing played is stale.
        control.clear();
        *self.failure.lock().unwrap_or_else(PoisonError::into_inner) = None;
        let failure = self.failure.clone();
        let failed = Box::new(move |error: String| {
            failure
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get_or_insert(error);
        });
        match crate::playback::start(self.playback.clone(), player.clone(), failed) {
            Ok(output) => {
                self.output = Some(output);
                self.retry_at = None;
                None
            }
            Err(error) => self.failed(error),
        }
    }

    /// Close an output that failed to open or stopped, so that it is opened again after a while. Returns a failure to show.
    fn poll(&mut self) -> Option<String> {
        let output = self.output.as_ref()?;
        // Opened since it last failed: a later failure is worth showing again.
        if output.is_open() {
            self.reported = false;
        }
        let error = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()?;
        self.output = None;
        self.failed(error)
    }

    fn failed(&mut self, error: String) -> Option<String> {
        self.retry_at = Some(Instant::now() + SPEAKER_RETRY);
        if std::mem::replace(&mut self.reported, true) {
            log::debug!("Speaker: {error}");
            return None;
        }
        log::warn!("{error}");
        Some(format!("{error} Replies cannot be heard."))
    }
}

impl Connection {
    /// Connects straight away to a server given at startup; otherwise the conversation starts at the server list.
    pub fn new(
        identity: Identity,
        store: Store,
        multicast: Box<dyn Multicast>,
        capture: Box<dyn Capture>,
        playback: Box<dyn Playback>,
        conversation: &mut Conversation,
    ) -> Self {
        // The participant identity is a secret of this device, so it is not logged.
        log::info!("Participant {:?}", identity.name);
        if let Some(address) = &identity.server {
            conversation.connect(Choice::new(address, address, Source::Override));
        } else if identity.token.is_some() {
            log::info!(
                "The token given is presented only to a server given at startup, or over USB."
            );
        }
        let remembered = store.server();
        Self {
            identity,
            store,
            multicast,
            client: None,
            browser: None,
            servers: discovery::Servers::new(),
            changed: true,
            failed: false,
            remembered,
            voice: Voice {
                capture,
                shared: Arc::default(),
                recording: None,
                live: false,
                generation: None,
            },
            speaker: Speaker::new(playback),
            presence: None,
            link: None,
        }
    }

    /// Ask for microphone access once the headset can show the request.
    pub fn request_microphone(&mut self) {
        match self.voice.request_permission() {
            Ok(true) => log::info!("Microphone access is allowed."),
            Ok(false) => log::info!("Asked for microphone access."),
            Err(error) => log::warn!("{error}"),
        }
    }

    /// Report this participant's head, controllers and hands to the server, so the others in the space can render them. The poses are in the space's scene coordinates. The client coalesces these to the latest, and the server relays them to the other participants.
    pub fn send_presence(
        &mut self,
        head: controller::Pose,
        left: Option<controller::Pose>,
        right: Option<controller::Pose>,
        hands: [Option<hands::Hand>; 2],
    ) {
        if self.client.is_none() {
            return;
        }
        let message = ClientMessage::Presence {
            head: Pose {
                position: head.position,
                orientation: head.orientation,
            },
            left: left.map(|pose| Pose {
                position: pose.position,
                orientation: pose.orientation,
            }),
            right: right.map(|pose| Pose {
                position: pose.position,
                orientation: pose.orientation,
            }),
            hands: Some(Box::new(hands)),
        };
        if let Some(client) = &self.client
            && let Err(error) = client.send(message)
        {
            log::debug!("Cannot send presence: {error}");
        }
    }

    /// Receive the other participants' presence reports, which the conversation ignores. The hook is called for every [`ServerMessage::Presence`], from the render loop's thread.
    #[cfg(not(target_os = "android"))]
    pub fn watch_presence(&mut self, hook: impl FnMut(&ServerMessage) + Send + 'static) {
        self.presence = Some(Box::new(hook));
    }

    /// Run once per frame. Browsing, and the multicast lock it needs, stop while unfocused.
    pub fn poll(&mut self, focused: bool, conversation: &mut Conversation) {
        if !focused {
            conversation.stop_live();
            conversation.stop_talking();
            // Stop the capture thread before processing any queued end or
            // cancellation; focus loss must not deliver another live buffer.
            self.voice.recording = None;
        }
        if let Some(error) = self.voice.failure() {
            conversation.microphone_failed(error);
        }
        self.browse(focused && conversation.is_browsing(), conversation);
        conversation.expire_pairing(Instant::now());
        let mut reopen = None;
        while let Some(event) = self.client.as_ref().and_then(Client::try_event) {
            match &event {
                Event::State(ConnectionState::Connected { certificate, .. }) => {
                    self.keep(*certificate)
                }
                Event::State(ConnectionState::Disconnected { reason, .. }) => {
                    reopen = self.refused(reason, conversation).or(reopen);
                }
                _ => {}
            }
            let connected = matches!(event, Event::State(ConnectionState::Connected { .. }));
            if connected && let Some(client) = self.client.as_ref() {
                conversation.set_capture_generation(client.sender().connection_generation());
            }
            if connected
                && let Some(instance) = conversation.server().and_then(|c| c.instance.as_deref())
            {
                self.store.remember_server(instance);
                self.remembered = Some(instance.to_owned());
            }
            if let Some(control) = self.speaker.control() {
                match &event {
                    // Audio from before reconnecting, or from a space that was left, is stale. Moving to another space leaves the current one first.
                    Event::State(ConnectionState::Connected { source, .. }) => {
                        control.set_local_source(Some(*source));
                        control.clear();
                    }
                    Event::Message(ServerMessage::Left) => control.clear(),
                    _ => {}
                }
            }
            // The spectator renders presence, which the conversation ignores.
            if let Event::Message(message @ ServerMessage::Presence { .. }) = &event
                && let Some(presence) = self.presence.as_deref_mut()
            {
                presence(message);
            }
            conversation.apply(event);
            // After the conversation applies the welcome, which clears earlier errors, so that a failure is shown again.
            if connected {
                self.speaker.connected();
                if let Some(error) = self.speaker.open() {
                    conversation.fail(error);
                }
            }
        }
        if let Some(error) = self.voice.failure() {
            conversation.microphone_failed(error);
        }
        if let Some(reopen) = reopen {
            self.reopen(reopen, conversation);
        }
        self.perform(conversation);
        if self
            .client
            .as_ref()
            .is_some_and(|client| client.state().is_connected())
            && let Some(error) = self.speaker.poll().or_else(|| self.speaker.open())
        {
            conversation.fail(error);
        }
        // Silence only during push-to-talk, not while talking to the space.
        if let Some(control) = self.speaker.control() {
            control.set_silenced(self.voice.talking());
        }
    }

    fn perform(&mut self, conversation: &mut Conversation) {
        while let Some(action) = conversation.next_action() {
            match action {
                Action::Connect(choice) => {
                    let mut key = choice
                        .instance
                        .clone()
                        .unwrap_or_else(|| choice.address.clone());
                    let mut kept = self.store.credential(&key);
                    // A server reached another way, such as over USB and then over Wi-Fi, is recognized by its certificate: the one kept for this name, or else the one it advertises. That certificate is then pinned, so only the server holding its key receives the token kept for it.
                    if kept.token.is_none()
                        && let Some((server, credential)) = kept
                            .certificate
                            .as_deref()
                            .or(choice.fingerprint.as_deref())
                            .and_then(|fingerprint| {
                                self.store.credential_for_certificate(fingerprint)
                            })
                    {
                        key = server;
                        kept = credential;
                    }
                    let (transport, pinned) =
                        transport(kept.certificate.as_deref(), choice.fingerprint.as_deref());
                    // A token given at startup goes only to the server given at startup, or over USB to this computer: never to a discovered server, whose advertisement anyone on the network could make.
                    let startup = self
                        .identity
                        .token
                        .clone()
                        .filter(|_| matches!(choice.source, Source::Override | Source::Usb));
                    let pending_token = kept.token.is_none() && startup.is_some();
                    let link = Link {
                        address: choice.address,
                        key,
                        transport,
                        token: kept.token.or(startup),
                        pending_token,
                        pinned,
                        distrusted: false,
                        pairing: false,
                    };
                    self.open(link, conversation);
                }
                Action::Paired { token, certificate } => {
                    self.paired(token, certificate, conversation)
                }
                Action::Admitted { token, certificate } => self.admitted(token, certificate),
                Action::Disconnect => self.disconnect(),
                Action::Rescan => {
                    self.browser = None;
                    self.servers = discovery::Servers::new();
                    self.changed = true;
                    self.failed = false;
                }
                Action::Send(message) => match &self.client {
                    Some(client) => {
                        if let Err(error) = client.send(message) {
                            conversation.fail(format!("Cannot send: {error}"));
                        }
                    }
                    None => conversation.fail("Not connected."),
                },
                Action::BeginUtterance(utterance, generation) => {
                    // Silence the speaker before the microphone opens.
                    if let Some(control) = self.speaker.control() {
                        control.set_silenced(true);
                    }
                    let begun = match &self.client {
                        Some(client) => self.voice.begin(client, utterance, generation),
                        None => Err("Not connected.".into()),
                    };
                    if let Err(error) = begun {
                        log::warn!("Cannot record: {error}");
                        self.voice.recording = None;
                        conversation.microphone_failed(error);
                    }
                }
                Action::EndUtterance => match self.voice.finish(self.client.as_ref(), true) {
                    Ok(()) => {}
                    Err(Finish::Utterance(error)) => conversation.utterance_not_sent(error),
                    Err(Finish::Live(error)) => conversation.microphone_failed(error),
                },
                Action::CancelUtterance => {
                    if let Err(error) = self.voice.finish(self.client.as_ref(), false) {
                        conversation.fail(String::from(error));
                    }
                }
                Action::AbandonUtterance => {
                    if let Err(error) = self.voice.abandon() {
                        conversation.fail(error);
                    }
                }
                Action::StartLive(generation) => {
                    let result = match &self.client {
                        Some(client) => self.voice.start_live(client, generation),
                        None => Err("Not connected.".into()),
                    };
                    if let Err(error) = result {
                        self.voice.recording = None;
                        conversation.microphone_failed(error);
                    }
                }
                Action::StopLive => {
                    if let Err(error) = self.voice.stop_live() {
                        self.voice.recording = None;
                        conversation.microphone_failed(error);
                    }
                }
            }
        }
    }

    /// Close the connection without stalling the frame: closing waits briefly for the server.
    /// Connect as `link` says.
    fn open(&mut self, link: Link, conversation: &mut Conversation) {
        self.disconnect();
        let mut options = Options::new(
            link.address.clone(),
            &self.identity.participant,
            &self.identity.name,
        );
        options.microphone_rearm_on_disconnect = true;
        options.transport = link.transport;
        options.token = link.token.clone();
        options.pairing = link.pairing;
        match Client::connect(options) {
            Ok(mut client) => {
                self.speaker.attach(&mut client);
                self.client = Some(client);
                self.link = Some(link);
            }
            Err(error) => conversation.fail(format!("Cannot start a connection: {error}")),
        }
    }

    /// Keep what a connection over TLS established: the server's certificate, pinned from then on, and a token given at startup.
    fn keep(&mut self, certificate: Option<Fingerprint>) {
        let Some(link) = self.link.as_mut() else {
            return;
        };
        let Some(certificate) = certificate.filter(|_| !link.distrusted) else {
            return;
        };
        if !link.pinned {
            self.store
                .remember_certificate(&link.key, &certificate.to_string());
            link.pinned = true;
            // Pinned from now on, including when the client reconnects.
            link.transport = Transport::Tls {
                pin: Some(certificate),
            };
        }
        if link.pending_token
            && let Some(token) = &link.token
        {
            self.store.remember_token(&link.key, token);
            link.pending_token = false;
            self.identity.token = None;
        }
    }

    /// Respond to a refused connection: forget a refused token, or distrust a server whose certificate changed.
    fn refused(&mut self, reason: &Disconnect, conversation: &mut Conversation) -> Option<Reopen> {
        let link = self.link.as_mut()?;
        match reason {
            Disconnect::Status(401) if link.token.is_some() => {
                if link.pending_token {
                    self.identity.token = None;
                } else {
                    self.store.forget_token(&link.key);
                }
                conversation.security_notice(
                    "This server no longer admits this device, so it watches as a visitor.",
                );
                Some(Reopen::WithoutToken)
            }
            // The server refuses visitors: ask to pair instead, which it admits.
            Disconnect::Status(401) if !link.pairing => Some(Reopen::ToPair),
            Disconnect::Status(401) => {
                conversation.security_notice("This server admits only devices with a token.");
                None
            }
            Disconnect::Certificate(_) if !link.distrusted => {
                conversation.security_notice(
                    "This server's identity has changed, so this device watches as a visitor and sends it no token.",
                );
                Some(Reopen::Distrusted)
            }
            _ => None,
        }
    }

    fn reopen(&mut self, reopen: Reopen, conversation: &mut Conversation) {
        let Some(mut link) = self.link.take() else {
            return;
        };
        link.token = None;
        link.pending_token = false;
        match reopen {
            Reopen::Distrusted => {
                link.distrusted = true;
                link.transport = Transport::Tls { pin: None };
            }
            Reopen::ToPair => link.pairing = true,
            Reopen::WithoutToken => {}
        }
        self.open(link, conversation);
    }

    /// A knock admitted this device: keep its collaborator token for the server, with the certificate the connection presented, to present when it connects again. The connection is already a collaborator's, so it is not reopened.
    fn admitted(&mut self, token: String, certificate: Fingerprint) {
        let Some(link) = self.link.as_mut() else {
            return;
        };
        // Never from a server whose identity changed, over a certificate other than the one kept, or in place of a token already kept: an admission may not replace what this device trusts.
        let kept = self.store.credential(&link.key).certificate;
        if link.distrusted
            || link.token.is_some()
            || kept.is_some_and(|kept| kept != certificate.to_string())
        {
            log::warn!(
                "Ignored an admission that would replace what this device keeps for the server."
            );
            return;
        }
        self.store
            .remember_certificate(&link.key, &certificate.to_string());
        self.store.remember_token(&link.key, &token);
        link.token = Some(token);
        link.pending_token = false;
        link.pinned = true;
        link.distrusted = false;
        link.transport = Transport::Tls {
            pin: Some(certificate),
        };
    }

    /// Pairing gave this device an owner token: keep it with the certificate the pairing connection presented, which replaces one it distrusted, and connect with both. See `design/security.md`.
    fn paired(&mut self, token: String, certificate: Fingerprint, conversation: &mut Conversation) {
        let Some(mut link) = self.link.take() else {
            return;
        };
        self.store
            .remember_certificate(&link.key, &certificate.to_string());
        self.store.remember_token(&link.key, &token);
        link.transport = Transport::Tls {
            pin: Some(certificate),
        };
        link.token = Some(token);
        link.pending_token = false;
        link.pinned = true;
        link.distrusted = false;
        link.pairing = false;
        self.open(link, conversation);
    }

    fn disconnect(&mut self) {
        self.link = None;
        self.voice.reset();
        self.speaker.detach();
        if let Some(client) = self.client.take() {
            let closed = std::thread::Builder::new()
                .name("tomori-close".into())
                .spawn(move || client.close());
            if let Err(error) = closed {
                log::warn!("Closing the connection in the background failed: {error}");
            }
        }
    }

    fn browse(&mut self, browsing: bool, conversation: &mut Conversation) {
        if !browsing {
            if self.browser.take().is_some() {
                self.multicast.set_held(false);
                // A new browser reports only the servers it finds, not those that vanished while it was stopped.
                self.servers = discovery::Servers::new();
                self.changed = true;
            }
            return;
        }
        if self.browser.is_none() && !self.failed {
            self.multicast.set_held(true);
            match discovery::browse() {
                Ok(browser) => self.browser = Some(browser),
                Err(error) => {
                    self.multicast.set_held(false);
                    self.failed = true;
                    log::warn!("Cannot browse for servers: {error}");
                    conversation.fail(format!("Cannot search the network: {error}"));
                }
            }
        }
        if let Some(browser) = &self.browser {
            while let Ok(event) = browser.try_recv() {
                self.changed |= self.servers.apply(event);
            }
        }
        self.publish(conversation);
    }

    fn publish(&mut self, conversation: &mut Conversation) {
        if std::mem::take(&mut self.changed) {
            conversation.set_servers(server_choice::choices(
                self.identity.server.as_deref(),
                self.remembered.as_deref(),
                &self.servers,
            ));
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.browser = None;
        self.multicast.set_held(false);
        self.speaker.detach();
        // Close in the foreground, so the server hears the goodbye before the process ends.
        if let Some(client) = self.client.take() {
            client.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{Active, Failed, Samples};

    #[test]
    fn the_kept_certificate_is_pinned_before_the_advertised_one() {
        let kept = Fingerprint::of(b"kept").to_string();
        let advertised = Fingerprint::of(b"advertised").to_string();
        assert_eq!(
            transport(Some(&kept), Some(&advertised)),
            (
                Transport::Tls {
                    pin: kept.parse().ok()
                },
                true
            )
        );
        assert_eq!(
            transport(None, Some(&advertised)),
            (
                Transport::Tls {
                    pin: advertised.parse().ok()
                },
                false
            )
        );
        // Nothing known, or nothing valid: try TLS, then plain HTTP.
        assert_eq!(
            transport(None, None),
            (Transport::Automatic { pin: None }, false)
        );
        assert_eq!(
            transport(Some("not a fingerprint"), None),
            (Transport::Automatic { pin: None }, false)
        );
    }
    use std::{
        sync::atomic::{AtomicBool, AtomicU32, Ordering},
        thread::JoinHandle,
        time::{Duration, Instant},
    };

    /// Captures from its own thread until stopped, joining it when the recording is dropped, as AAudio does.
    struct Fake {
        rate: Arc<AtomicU32>,
        permitted: bool,
        fail: Option<String>,
    }

    struct Worker {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl Drop for Worker {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            self.thread.take().unwrap().join().unwrap();
        }
    }

    impl Capture for Fake {
        fn permit(&mut self) -> Result<bool, String> {
            Ok(self.permitted)
        }

        fn request(&mut self) -> Result<bool, String> {
            Ok(self.permitted)
        }

        fn open(&mut self, mut samples: Samples, failed: Failed) -> Result<Recording, String> {
            let stop = Arc::new(AtomicBool::new(false));
            let fail = self.fail.clone();
            let thread = {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    if let Some(error) = fail {
                        failed(error);
                    }
                    while !stop.load(Ordering::Relaxed) {
                        samples(&[0.1; 160]);
                        std::thread::sleep(Duration::from_millis(1));
                    }
                })
            };
            Ok(Recording::new(
                self.rate.load(Ordering::Relaxed),
                Worker {
                    stop,
                    thread: Some(thread),
                },
            ))
        }

        fn open_live(
            &mut self,
            samples: Samples,
            failed: Failed,
            _active: Active,
        ) -> Result<Recording, String> {
            self.open(samples, failed)
        }
    }

    fn voice(rate: &Arc<AtomicU32>, permitted: bool, fail: Option<&str>) -> Voice {
        Voice {
            capture: Box::new(Fake {
                rate: rate.clone(),
                permitted,
                fail: fail.map(str::to_owned),
            }),
            shared: Arc::default(),
            recording: None,
            live: false,
            generation: None,
        }
    }

    /// A client that never connects, which is enough to queue messages.
    fn client() -> Client {
        Client::connect(Options::new("127.0.0.1:9", "ada", "Ada")).unwrap()
    }

    fn utterance(voice: &Voice) -> Option<String> {
        voice
            .lock()
            .microphone
            .as_ref()
            .and_then(|microphone| microphone.utterance().map(str::to_owned))
    }

    #[test]
    fn closing_a_recording_waits_for_its_thread_without_deadlock() {
        let client = client();
        let rate = Arc::new(AtomicU32::new(16_000));
        let mut voice = voice(&rate, true, None);
        voice.begin(&client, "u1".into(), 0).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(utterance(&voice).as_deref(), Some("u1"));
        voice.finish(Some(&client), true).unwrap();
        assert!(voice.recording.is_none());
        assert_eq!(utterance(&voice), None);

        voice.begin(&client, "u2".into(), 0).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        voice.abandon().unwrap();
        assert_eq!(utterance(&voice), None);

        voice.begin(&client, "u3".into(), 0).unwrap();
        voice.reset();
        assert!(voice.lock().microphone.is_none());
        client.close();
    }

    #[test]
    fn a_new_rate_gets_a_new_microphone() {
        let client = client();
        let rate = Arc::new(AtomicU32::new(16_000));
        let mut voice = voice(&rate, true, None);
        voice.begin(&client, "u1".into(), 0).unwrap();
        voice.finish(Some(&client), false).unwrap();
        rate.store(48_000, Ordering::Relaxed);
        voice.begin(&client, "u2".into(), 0).unwrap();
        let sample_rate = voice.lock().microphone.as_ref().unwrap().sample_rate();
        assert_eq!(sample_rate, 48_000);
        voice.finish(Some(&client), false).unwrap();
        client.close();
    }

    #[test]
    fn nothing_records_without_permission() {
        let client = client();
        let rate = Arc::new(AtomicU32::new(16_000));
        let mut voice = voice(&rate, false, None);
        let error = voice.begin(&client, "u1".into(), 0).unwrap_err();
        assert!(error.contains("Allow the microphone"), "{error}");
        assert!(voice.recording.is_none());
        assert_eq!(utterance(&voice), None);
        client.close();
    }

    #[test]
    fn live_microphone_resumes_after_agent_and_can_be_muted_during_it() {
        let client = client();
        let rate = Arc::new(AtomicU32::new(16_000));
        let mut voice = voice(&rate, true, None);
        voice.start_live(&client, 0).unwrap();
        assert_eq!(voice.lock().microphone.as_ref().unwrap().gate(), Gate::Live);
        voice.begin(&client, "u1".into(), 0).unwrap();
        assert!(voice.talking());
        assert_eq!(
            voice.lock().microphone.as_ref().unwrap().gate(),
            Gate::Agent
        );
        voice.finish(Some(&client), true).unwrap();
        assert!(voice.recording.is_some());
        assert_eq!(voice.lock().microphone.as_ref().unwrap().gate(), Gate::Live);
        voice.begin(&client, "u2".into(), 0).unwrap();
        voice.stop_live().unwrap();
        assert_eq!(
            voice.lock().microphone.as_ref().unwrap().gate(),
            Gate::Agent
        );
        voice.finish(Some(&client), true).unwrap();
        assert!(voice.recording.is_none());
        assert_eq!(voice.lock().microphone.as_ref().unwrap().gate(), Gate::Off);
        client.close();
    }

    #[test]
    fn live_microphone_stops_and_permission_is_required() {
        let client = client();
        let rate = Arc::new(AtomicU32::new(16_000));
        let mut denied = voice(&rate, false, None);
        assert!(
            denied
                .start_live(&client, 0)
                .unwrap_err()
                .contains("left grip")
        );
        assert!(denied.recording.is_none());
        let mut voice = voice(&rate, true, None);
        voice.start_live(&client, 0).unwrap();
        voice.stop_live().unwrap();
        assert!(voice.recording.is_none());
        assert_eq!(voice.lock().microphone.as_ref().unwrap().gate(), Gate::Off);
        client.close();
    }

    #[test]
    fn microphone_access_is_requested_early_without_using_up_the_recording_request() {
        /// Records which permission calls were made, granting only once asked at recording.
        struct Asking(Arc<Mutex<Vec<&'static str>>>);
        impl Capture for Asking {
            fn permit(&mut self) -> Result<bool, String> {
                self.0.lock().unwrap().push("permit");
                Ok(false)
            }
            fn request(&mut self) -> Result<bool, String> {
                self.0.lock().unwrap().push("request");
                Ok(false)
            }
            fn open(&mut self, _samples: Samples, _failed: Failed) -> Result<Recording, String> {
                panic!("opened without permission")
            }
        }
        let client = client();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut voice = Voice {
            capture: Box::new(Asking(calls.clone())),
            shared: Arc::new(Mutex::new(Captured::default())),
            recording: None,
            live: false,
            generation: None,
        };
        assert_eq!(voice.request_permission(), Ok(false));
        // Asking opens nothing.
        assert!(voice.recording.is_none());
        assert!(voice.lock().microphone.is_none());
        // The first recording still makes its own request.
        assert!(
            voice
                .begin(&client, "u1".into(), 0)
                .unwrap_err()
                .contains("hold X again")
        );
        assert_eq!(*calls.lock().unwrap(), ["request", "permit"]);
        client.close();
    }

    #[test]
    fn a_capture_failure_is_reported_only_while_recording() {
        let client = client();
        let rate = Arc::new(AtomicU32::new(16_000));
        let mut voice = voice(&rate, true, Some("The microphone failed."));
        voice.begin(&client, "u1".into(), 0).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let failure = loop {
            if let Some(failure) = voice.failure() {
                break failure;
            }
            assert!(Instant::now() < deadline, "no failure reported");
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(failure, "The microphone failed.");
        voice.finish(Some(&client), false).unwrap();

        // Closed before it is noticed: nothing is reported once the recording has ended.
        voice.begin(&client, "u2".into(), 0).unwrap();
        voice.finish(Some(&client), false).unwrap();
        assert_eq!(voice.failure(), None);
        client.close();
    }

    #[test]
    fn failure_before_end_cancels_the_utterance_and_keeps_the_microphone() {
        let client = client();
        let rate = Arc::new(AtomicU32::new(16_000));
        let mut voice = voice(&rate, true, None);
        voice.start_live(&client, 0).unwrap();
        voice.begin(&client, "u1".into(), 0).unwrap();
        voice.lock().failure = Some("capture failed".into());
        assert_eq!(
            voice.finish(Some(&client), true).unwrap_err(),
            Finish::Utterance("capture failed".into())
        );
        assert_eq!(utterance(&voice), None);
        assert!(voice.lock().microphone.is_some());
        voice.stop_live().unwrap();
        client.close();
    }

    /// A capture that records which kind of capture each step opened.
    struct KindCapture(Arc<Mutex<Vec<&'static str>>>);

    impl Capture for KindCapture {
        fn open(&mut self, _samples: Samples, _failed: Failed) -> Result<Recording, String> {
            self.0.lock().unwrap().push("agent");
            Ok(Recording::new(16_000, ()))
        }
        fn open_live(
            &mut self,
            _samples: Samples,
            _failed: Failed,
            _active: Active,
        ) -> Result<Recording, String> {
            self.0.lock().unwrap().push("live");
            Ok(Recording::new(16_000, ()))
        }
    }

    fn kind_voice() -> (Voice, Arc<Mutex<Vec<&'static str>>>) {
        let opened = Arc::new(Mutex::new(Vec::new()));
        let voice = Voice {
            capture: Box::new(KindCapture(opened.clone())),
            shared: Arc::new(Mutex::new(Captured::default())),
            recording: None,
            live: false,
            generation: None,
        };
        (voice, opened)
    }

    #[test]
    fn push_to_talk_during_live_audio_captures_for_recognition() {
        let client = client();
        let (mut voice, opened) = kind_voice();
        voice.start_live(&client, 0).unwrap();
        voice.begin(&client, "u1".into(), 0).unwrap();
        voice.finish(Some(&client), true).unwrap();
        assert_eq!(*opened.lock().unwrap(), ["live", "agent", "live"]);
        client.close();
    }

    #[test]
    fn a_failed_live_reopen_after_a_sent_utterance_is_reported_separately() {
        struct ReopenFails(u32);
        impl Capture for ReopenFails {
            fn open(&mut self, _samples: Samples, _failed: Failed) -> Result<Recording, String> {
                Ok(Recording::new(16_000, ()))
            }
            fn open_live(
                &mut self,
                _samples: Samples,
                _failed: Failed,
                _active: Active,
            ) -> Result<Recording, String> {
                self.0 += 1;
                if self.0 > 1 {
                    return Err("Cannot open the microphone.".into());
                }
                Ok(Recording::new(16_000, ()))
            }
        }
        let client = client();
        let mut voice = Voice {
            capture: Box::new(ReopenFails(0)),
            shared: Arc::new(Mutex::new(Captured::default())),
            recording: None,
            live: false,
            generation: None,
        };
        voice.start_live(&client, 0).unwrap();
        voice.begin(&client, "u1".into(), 0).unwrap();
        // The utterance was ended; only the live microphone failed afterwards.
        assert_eq!(
            voice.finish(Some(&client), true).unwrap_err(),
            Finish::Live("Cannot open the microphone.".into())
        );
        assert_eq!(utterance(&voice), None);
        client.close();
    }

    #[test]
    fn a_capture_for_one_connection_is_never_reused_for_another() {
        let client = client();
        let (mut voice, opened) = kind_voice();
        voice.start_live(&client, 0).unwrap();
        // Neither live audio nor push-to-talk for a later connection may use
        // the earlier connection's open capture.
        assert!(voice.start_live(&client, 1).is_err());
        assert!(voice.begin(&client, "u1".into(), 1).is_err());
        assert_eq!(*opened.lock().unwrap(), ["live"]);
        // The capture feeds only a microphone bound to its own connection.
        assert_eq!(
            voice.lock().microphone.as_ref().unwrap().connection(),
            Some(0)
        );
        voice.stop_live().unwrap();
        client.close();
    }

    #[test]
    fn live_and_agent_capture_threads_stop_before_the_gate_changes() {
        struct BoundaryCapture {
            shared: Arc<Mutex<Captured>>,
            closed_gates: Arc<Mutex<Vec<Gate>>>,
        }
        impl Capture for BoundaryCapture {
            fn open(&mut self, _samples: Samples, _failed: Failed) -> Result<Recording, String> {
                let stop = Arc::new(AtomicBool::new(false));
                let thread_stop = stop.clone();
                let shared = self.shared.clone();
                let closed_gates = self.closed_gates.clone();
                let thread = std::thread::spawn(move || {
                    while !thread_stop.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    let gate = shared.lock().unwrap().microphone.as_ref().unwrap().gate();
                    closed_gates.lock().unwrap().push(gate);
                });
                Ok(Recording::new(
                    16_000,
                    Worker {
                        stop,
                        thread: Some(thread),
                    },
                ))
            }
            fn open_live(
                &mut self,
                samples: Samples,
                failed: Failed,
                _active: Active,
            ) -> Result<Recording, String> {
                self.open(samples, failed)
            }
        }
        let client = client();
        let shared = Arc::new(Mutex::new(Captured::default()));
        let closed_gates = Arc::new(Mutex::new(Vec::new()));
        let mut voice = Voice {
            capture: Box::new(BoundaryCapture {
                shared: shared.clone(),
                closed_gates: closed_gates.clone(),
            }),
            shared,
            recording: None,
            live: false,
            generation: None,
        };
        voice.start_live(&client, 0).unwrap();
        voice.begin(&client, "u1".into(), 0).unwrap();
        voice.finish(Some(&client), true).unwrap();
        voice.stop_live().unwrap();
        assert_eq!(
            *closed_gates.lock().unwrap(),
            [Gate::Live, Gate::Agent, Gate::Off]
        );
        client.close();
    }

    #[test]
    fn failure_reported_during_capture_shutdown_cancels_the_agent_utterance() {
        struct FailureOnStop(Arc<AtomicU32>);
        impl Capture for FailureOnStop {
            fn open(&mut self, _samples: Samples, failed: Failed) -> Result<Recording, String> {
                let attempt = self.0.fetch_add(1, Ordering::Relaxed);
                let stop = Arc::new(AtomicBool::new(false));
                let thread_stop = stop.clone();
                let thread = std::thread::spawn(move || {
                    while !thread_stop.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    if attempt == 1 {
                        failed("read failed while stopping".into());
                    }
                });
                Ok(Recording::new(
                    16_000,
                    Worker {
                        stop,
                        thread: Some(thread),
                    },
                ))
            }
            fn open_live(
                &mut self,
                samples: Samples,
                failed: Failed,
                _active: Active,
            ) -> Result<Recording, String> {
                self.open(samples, failed)
            }
        }
        let client = client();
        let attempts = Arc::new(AtomicU32::new(0));
        let mut voice = Voice {
            capture: Box::new(FailureOnStop(attempts.clone())),
            shared: Arc::default(),
            recording: None,
            live: false,
            generation: None,
        };
        voice.start_live(&client, 0).unwrap();
        voice.begin(&client, "u1".into(), 0).unwrap();
        assert_eq!(
            voice.finish(Some(&client), true).unwrap_err(),
            Finish::Utterance("read failed while stopping".into())
        );
        assert_eq!(utterance(&voice), None);
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
        voice.stop_live().unwrap();
        client.close();
    }

    /// Opens outputs that render the player once, or fails to open while `refuse` is set. Keeps each open output's failure callback.
    #[derive(Clone, Default)]
    struct FakeSpeaker {
        opened: Arc<AtomicU32>,
        closed: Arc<AtomicU32>,
        refuse: Arc<AtomicBool>,
        failed: Arc<Mutex<Option<crate::playback::Failed>>>,
    }

    /// Closes by counting, so that a test can see the speaker thread close its output.
    struct FakeOutput(Arc<AtomicU32>);

    impl Drop for FakeOutput {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    impl Playback for FakeSpeaker {
        fn open(
            &self,
            player: crate::playback::Shared,
            failed: crate::playback::Failed,
        ) -> Result<crate::playback::Output, String> {
            if self.refuse.load(Ordering::Relaxed) {
                return Err("No speaker is available.".into());
            }
            let mut output = [1; 48];
            player.lock().unwrap().render(48_000, &mut output);
            assert_eq!(output, [0; 48]);
            *self.failed.lock().unwrap() = Some(failed);
            self.opened.fetch_add(1, Ordering::Relaxed);
            Ok(crate::playback::Output::new(FakeOutput(
                self.closed.clone(),
            )))
        }
    }

    /// Wait for the speaker thread.
    fn until(condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Poll the speaker until it reports a failure.
    fn failure(speaker: &mut Speaker) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(error) = speaker.poll() {
                return error;
            }
            assert!(Instant::now() < deadline, "no failure reported");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn the_speaker_opens_on_its_own_thread_and_closes_when_the_client_goes_away() {
        let mut client = client();
        let fake = FakeSpeaker::default();
        let mut speaker = Speaker::new(Box::new(fake.clone()));
        // Nothing to play before a client is attached.
        assert_eq!(speaker.open(), None);
        assert!(speaker.output.is_none());

        speaker.attach(&mut client);
        assert!(client.take_audio().is_none(), "the speaker took the audio");
        assert_eq!(speaker.open(), None);
        assert_eq!(speaker.open(), None);
        until(|| fake.opened.load(Ordering::Relaxed) == 1);
        assert!(speaker.output.as_ref().unwrap().is_open());
        assert_eq!(speaker.poll(), None);

        speaker.detach();
        assert!(speaker.output.is_none() && speaker.control().is_none());
        until(|| fake.closed.load(Ordering::Relaxed) == 1);
        assert_eq!(fake.opened.load(Ordering::Relaxed), 1);
        client.close();
    }

    #[test]
    fn a_failed_speaker_is_reported_once_and_opened_again_later() {
        let mut client = client();
        let fake = FakeSpeaker::default();
        let mut speaker = Speaker::new(Box::new(fake.clone()));
        speaker.attach(&mut client);

        fake.refuse.store(true, Ordering::Relaxed);
        assert_eq!(speaker.open(), None);
        let error = failure(&mut speaker);
        assert!(error.contains("No speaker"), "{error}");
        assert!(speaker.output.is_none());
        // Not tried again until the retry is due, and not shown again when it fails again.
        assert_eq!(speaker.open(), None);
        assert!(speaker.output.is_none());
        speaker.retry_at = Some(Instant::now());
        assert_eq!(speaker.open(), None);
        until(|| speaker.failure.lock().unwrap().is_some());
        assert_eq!(speaker.poll(), None);
        assert!(speaker.output.is_none());

        // A new welcome tries again at once, and shows the failure again.
        speaker.connected();
        assert_eq!(speaker.open(), None);
        let error = failure(&mut speaker);
        assert!(error.contains("No speaker"), "{error}");

        fake.refuse.store(false, Ordering::Relaxed);
        speaker.retry_at = Some(Instant::now());
        assert_eq!(speaker.open(), None);
        until(|| fake.opened.load(Ordering::Relaxed) == 1);
        assert_eq!(speaker.poll(), None);

        // The open output fails from another thread: it is closed, shown, and reopened once due.
        let failed = fake.failed.lock().unwrap().take().unwrap();
        std::thread::spawn(move || failed("The speaker failed: unplugged".into()))
            .join()
            .unwrap();
        let error = speaker.poll().expect("the failure is shown");
        assert!(error.contains("unplugged"), "{error}");
        assert!(speaker.output.is_none());
        until(|| fake.closed.load(Ordering::Relaxed) == 1);
        assert_eq!(speaker.open(), None);
        speaker.retry_at = Some(Instant::now());
        assert_eq!(speaker.open(), None);
        until(|| fake.opened.load(Ordering::Relaxed) == 2);
        client.close();
    }

    #[test]
    fn opening_the_speaker_discards_audio_queued_while_nothing_played() {
        let fake = FakeSpeaker::default();
        let mut speaker = Speaker::new(Box::new(fake.clone()));
        let (sender, audio) = std::sync::mpsc::sync_channel(8);
        let (player, control) = Player::new(audio, MixerOptions::default());
        speaker.player = Some((Arc::new(Mutex::new(player)), control));
        for sequence in 0..8 {
            sender
                .send(protocol::AudioFrame {
                    gate: protocol::Gate::Live,
                    source: 1,
                    sequence,
                    timestamp: 0,
                    sample_rate: 48_000,
                    samples: vec![1000; 960],
                })
                .unwrap();
        }
        // The fake renders as it opens, and asserts that it hears nothing.
        assert_eq!(speaker.open(), None);
        until(|| fake.opened.load(Ordering::Relaxed) == 1);
    }
}
