// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Carries out the [`Conversation`]'s actions: owns the server connection, the microphone and the speaker, and browses for servers while the server list shows.
use crate::{
    capture::{Capture, Recording},
    controller,
    conversation::{Action, Conversation},
    feedback::{Event as Feedback, Hand},
    hands,
    identity::{Identity, Store},
    playback::{Playback, Route, Speaking},
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
    /// What the participant hears and feels as things change; see `design/interaction.md`.
    feedback: crate::feedback::Feedback,
    /// This participant's utterances recorded and not yet transcribed, so that their outcome is cued.
    utterances: Vec<String>,
    /// The utterance being recorded now, until X is released or it ends otherwise.
    recording: Option<String>,
    /// Speech played again to this participant, so that a catch-up nobody asked for is cued once.
    replay: Replay,
    /// How close to the server's limit the recording in progress has been cued: 0, 1 at [`crate::recording::WARNING`], 2 at [`crate::recording::CRITICAL`].
    limit_cued: u8,
    /// Where each voice comes from: the other participants' latest heads, from relayed presence. See `design/spatial-audio.md`.
    heads: std::collections::HashMap<u32, Pose>,
    /// This client's own participant number on the current connection, which is never placed.
    local_source: Option<u32>,
    /// The participant whose head the spectator hears from, who is the listener rather than a source.
    listening_as: Option<u32>,
    /// The other participants in the space, kept between frames to avoid allocating.
    others: Vec<u32>,
    /// Measures how much of the speaker the live microphone hears, when started with it.
    echo_test: Option<EchoTest>,
}

/// Speech played again to this participant: a replay they asked for, or a catch-up on joining or returning.
#[derive(Debug, Default)]
struct Replay {
    /// When a replay was asked for, until it ends.
    asked: Option<Instant>,
    /// Speech played again is under way, as of its last segment.
    since: Option<Instant>,
}

/// How long after the last segment played again, with the player quiet, a replay counts as over.
const REPLAY_OVER: Duration = Duration::from_secs(3);
/// How long a replay asked for may take to start, as the server synthesizes it again, before it counts as never coming.
const REPLAY_START: Duration = Duration::from_secs(15);

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
    /// What the microphone heard since last taken, for the echo test.
    heard: Heard,
}

/// The energy of captured samples, from −1 to 1, and how many there were.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Heard {
    energy: f64,
    samples: u64,
}

impl Heard {
    fn add(&mut self, other: Heard) {
        self.energy += other.energy;
        self.samples += other.samples;
    }

    /// The level heard, in dBFS RMS, if anything was.
    fn level(&self) -> Option<f64> {
        (self.samples > 0).then(|| 10.0 * (self.energy / self.samples as f64).max(1e-12).log10())
    }
}

/// When the echo test listens, from when the live microphone opens: first the room, then the probe the speaker plays. See `design/spatial-audio.md`.
const ECHO_FLOOR: std::ops::Range<Duration> =
    Duration::from_millis(300)..Duration::from_millis(1_000);
const ECHO_PROBE: Duration = Duration::from_millis(1_000);
/// How long the echo test waits for the speaker to play as it needs before giving up, saying so.
const ECHO_WAIT: Duration = Duration::from_secs(5);
/// The part of the probe measured, after it starts: its steady middle, allowing for the speaker's and microphone's latency.
const ECHO_HEARD: std::ops::Range<Duration> =
    Duration::from_millis(300)..Duration::from_millis(1_300);

/// The echo test, for the live microphone opened at `opened`: what it heard of the room and of the probe. See `design/spatial-audio.md`.
#[derive(Debug)]
struct EchoTest {
    /// How the live microphone captures, for the log: with the platform's echo canceller or without.
    label: &'static str,
    /// Whether the speaker plays as voice communication in the microphone's session for this test, so that it waits for that.
    routed: bool,
    /// When the live microphone opened, while the test waits for the speaker to play as it should.
    armed: Option<Instant>,
    /// The audio session the speaker played in as voice communication during the test, or `None` as media, for the log.
    speaker: Option<i32>,
    opened: Option<Instant>,
    probed: Option<Instant>,
    floor: Heard,
    echo: Heard,
}

impl EchoTest {
    /// Attribute what the microphone `heard`, up to `now`, to the room or the probe. Returns the result to log once both are measured, and whether to play the probe now.
    fn update(&mut self, heard: Heard, now: Instant) -> (Option<String>, bool) {
        let Some(opened) = self.opened else {
            return (None, false);
        };
        let since = now.saturating_duration_since(opened);
        let mut probe = false;
        match self.probed {
            None if since >= ECHO_PROBE => {
                self.probed = Some(now);
                probe = true;
            }
            None if ECHO_FLOOR.contains(&since) => self.floor.add(heard),
            None => {}
            Some(probed) => {
                let since = now.saturating_duration_since(probed);
                if ECHO_HEARD.contains(&since) {
                    self.echo.add(heard);
                } else if since >= ECHO_HEARD.end {
                    self.opened = None;
                    let level = |heard: Heard| {
                        heard
                            .level()
                            .map_or_else(|| "nothing".into(), |level| format!("{level:.1} dBFS"))
                    };
                    let probe = 20.0 * f64::from(client::playback::PROBE_LEVEL).log10();
                    let below = self.echo.level().map_or_else(String::new, |echo| {
                        format!(", {:.1} dB below the probe", probe - echo)
                    });
                    // Within a few dB of the room, the echo is lost in it, and the measurement says only that it is at most this.
                    let room = match (self.echo.level(), self.floor.level()) {
                        (Some(echo), Some(floor)) if echo - floor < 3.0 => {
                            format!(
                                ", within {:.1} dB of the room, so at most this",
                                echo - floor
                            )
                        }
                        (Some(echo), Some(floor)) => {
                            format!(", {:.1} dB above the room", echo - floor)
                        }
                        _ => String::new(),
                    };
                    return (
                        Some(format!(
                            "Echo test ({}, the speaker {}): the speaker played a probe at {probe:.1} dBFS; the microphone heard {} during it{below}{room}; the room was {}",
                            self.label,
                            self.speaker.map_or_else(
                                || "as media".into(),
                                |session| format!("as voice communication in session {session}")
                            ),
                            level(self.echo),
                            level(self.floor),
                        )),
                        false,
                    );
                }
            }
        }
        (None, probe)
    }
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
            captured.heard.add(Heard {
                energy: samples
                    .iter()
                    .map(|sample| f64::from(*sample).powi(2))
                    .sum(),
                samples: samples.len() as u64,
            });
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

    fn begin(
        &mut self,
        client: &Client,
        utterance: String,
        generation: u64,
        answer: Option<protocol::AnswerTarget>,
    ) -> Result<(), String> {
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
        let result = self
            .lock()
            .microphone
            .as_mut()
            .unwrap()
            .begin_answer(utterance, answer);
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
        self.finish_pointing(client, send, None)
    }

    /// Finish as [`Self::finish`] does, sending what the participant pointed at with an utterance that is ended.
    fn finish_pointing(
        &mut self,
        client: Option<&Client>,
        send: bool,
        pointed: Option<protocol::Pointed>,
    ) -> Result<(), Finish> {
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
            microphone.end_pointing(pointed)
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
    /// Whether agent speech is muted locally, kept across connections.
    muted: bool,
    /// How the open output plays, and how the next one should.
    route: Route,
    /// Whether the players made for later connections load the echo probe, for the echo test.
    probe: bool,
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
            muted: false,
            route: Route::default(),
            probe: false,
        }
    }

    /// Play as `route` says: an open output that plays otherwise is closed and opened again at once. See `design/spatial-audio.md`.
    fn reroute(&mut self, route: Route) -> Option<String> {
        if route == self.route {
            return None;
        }
        log::info!(
            "Speaker: playing {}",
            match route.communication {
                Some(session) =>
                    format!("as voice communication in session {session}, for the live microphone"),
                None => "as media".into(),
            }
        );
        self.route = route;
        self.output.take()?;
        self.retry_at = None;
        // What is buffered is still current: rerouting must not cut a reply short.
        self.start(false)
    }

    /// Take the new client's received audio; nothing plays until [`Self::open`].
    fn attach(&mut self, client: &mut Client) {
        self.detach();
        if let Some(audio) = client.take_audio() {
            let (mut player, control) = Player::new(audio, MixerOptions::default());
            if self.probe {
                player.load_probe();
            }
            control.set_agent_muted(self.muted);
            self.player = Some((Arc::new(Mutex::new(player)), control));
        }
    }

    /// Mute or unmute agent speech locally, on this player and those of later connections.
    fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        if let Some(control) = self.control() {
            control.set_agent_muted(muted);
        }
    }

    /// Whether agent speech played again for this participant is playing.
    fn is_replaying(&self) -> bool {
        self.control().is_some_and(PlayerControl::is_replaying)
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
        if self.output.is_some() || self.retry_at.is_some_and(|at| Instant::now() < at) {
            return None;
        }
        // Audio queued while nothing played is stale.
        self.start(true)
    }

    /// Start the speaker thread, first discarding what was queued if `clear`.
    fn start(&mut self, clear: bool) -> Option<String> {
        let (player, control) = self.player.as_ref()?;
        if clear {
            control.clear();
        }
        *self.failure.lock().unwrap_or_else(PoisonError::into_inner) = None;
        let failure = self.failure.clone();
        let failed = Box::new(move |error: String| {
            failure
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get_or_insert(error);
        });
        match crate::playback::start(self.playback.clone(), player.clone(), failed, self.route) {
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
        let feedback = store.feedback();
        conversation.set_feedback(feedback);
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
            feedback: crate::feedback::Feedback::new(feedback),
            utterances: Vec::new(),
            recording: None,
            replay: Replay::default(),
            limit_cued: 0,
            heads: std::collections::HashMap::new(),
            local_source: None,
            listening_as: None,
            others: Vec::new(),
            echo_test: None,
        }
    }

    /// Measure, each time the live microphone opens, how much of a probe the speaker plays it hears, and log it: `label` says how the microphone captures. Needs a quiet room. See `design/spatial-audio.md`.
    #[cfg(any(target_os = "android", test))]
    pub fn set_echo_test(&mut self, label: &'static str, routed: bool) {
        log::info!(
            "Echo test ({label}): each time the live microphone opens, a probe plays a second after the speaker is ready; keep the room quiet and the volume the same between runs"
        );
        if let Some((player, _)) = &self.speaker.player {
            player
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .load_probe();
        }
        self.speaker.probe = true;
        self.echo_test = Some(EchoTest {
            label,
            routed,
            armed: None,
            speaker: None,
            opened: None,
            probed: None,
            floor: Heard::default(),
            echo: Heard::default(),
        });
    }

    /// Route the speaker for the live microphone, and run the echo test.
    fn listen_live(&mut self, conversation: &mut Conversation) {
        // Push-to-talk while live closes the live stream for a while, so the session stays until live audio stops or the microphone opens in another.
        let route = Route {
            communication: self
                .voice
                .live
                .then(|| {
                    self.voice
                        .capture
                        .live_session()
                        .or(self.speaker.route.communication)
                })
                .flatten(),
        };
        if let Some(error) = self.speaker.reroute(route) {
            conversation.fail(error);
        }
        let heard = std::mem::take(&mut self.voice.lock().heard);
        if let Some(test) = &mut self.echo_test {
            if !self.voice.live {
                test.opened = None;
                test.armed = None;
                return;
            }
            // Timed from when the speaker plays as the test needs, after any reopening for the live microphone: as voice communication, not having fallen back to media, when the test needs that.
            let now = Instant::now();
            if let Some(armed) = test.armed {
                let playing = self
                    .speaker
                    .output
                    .as_ref()
                    .filter(|output| output.is_open())
                    .map(Speaking::communication);
                match playing {
                    Some(communication) if !test.routed || communication.is_some() => {
                        test.armed = None;
                        test.speaker = communication;
                        test.opened = Some(now);
                    }
                    _ if now.saturating_duration_since(armed) >= ECHO_WAIT => {
                        test.armed = None;
                        log::warn!(
                            "Echo test ({}): nothing measured, as the speaker did not play {} within {} seconds",
                            test.label,
                            if test.routed {
                                "as voice communication in the live microphone's session"
                            } else {
                                "at all"
                            },
                            ECHO_WAIT.as_secs()
                        );
                    }
                    _ => {}
                }
            }
            let (result, probe) = test.update(heard, Instant::now());
            if probe && let Some(control) = self.speaker.control() {
                control.play_probe();
            }
            if let Some(result) = result {
                log::info!("{result}");
            }
        }
    }

    /// Hear as `participant` does, from their head, as the spectator does for the participant it follows: they are the listener, so their own voice is not placed on a seat.
    #[cfg(not(target_os = "android"))]
    pub fn listen_as(&mut self, participant: Option<u32>) {
        self.listening_as = participant;
    }

    /// Where this participant hears from now, in the space's scene coordinates, and where the agent is, typically the swarm's centre; called once a frame. Every other participant in the space is placed as [`crate::spatial_audio`] says. Without a listener, every source plays centred.
    pub fn listen(
        &mut self,
        listener: Option<controller::Pose>,
        agent: [f32; 3],
        conversation: &Conversation,
    ) {
        let Some(control) = self.speaker.control() else {
            return;
        };
        self.others.clear();
        if let Some((space, _)) = conversation.joined_space() {
            self.others.extend(
                space
                    .participants
                    .iter()
                    .map(|participant| participant.source)
                    .filter(|source| {
                        Some(*source) != self.local_source && Some(*source) != self.listening_as
                    }),
            );
        }
        self.others.sort_unstable();
        self.others.dedup();
        let others = &self.others;
        self.heads.retain(|source, _| others.contains(source));
        let listener = listener.map(|pose| client::playback::spatial::Listener {
            position: pose.position,
            orientation: pose.orientation,
        });
        let mono = self.feedback.settings().mono;
        control.set_scene(|scene| {
            crate::spatial_audio::fill(scene, listener, agent, others, &self.heads, mono)
        });
    }

    /// What the participant hears and feels as things change, for the frame loop to report pointing and choosing to, and to take vibrations from.
    pub fn feedback(&mut self) -> &mut crate::feedback::Feedback {
        &mut self.feedback
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
            self.feedback
                .emit(Feedback::NotSent(Hand::Left), Instant::now());
            conversation.microphone_failed(error);
        }
        let browsing = focused && conversation.is_browsing();
        self.browse(browsing, conversation);
        // Only while the server list is seen and kept current, so nothing connects automatically behind the system menu.
        if browsing {
            conversation.tick(Instant::now());
        }
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
            self.observe(&event);
            match &event {
                Event::Message(ServerMessage::Presence { source, head, .. }) => {
                    self.heads.insert(*source, *head);
                }
                Event::State(ConnectionState::Connected { source, .. }) => {
                    self.local_source = Some(*source);
                    self.heads.clear();
                }
                Event::Message(ServerMessage::Left) => self.heads.clear(),
                _ => {}
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
            self.feedback
                .emit(Feedback::NotSent(Hand::Left), Instant::now());
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
        conversation.set_replaying(self.speaker.is_replaying());
        self.listen_live(conversation);
        self.cue(conversation);
    }

    /// Cue what the server said about this participant's words and speech played again.
    fn observe(&mut self, event: &Event) {
        let now = Instant::now();
        match event {
            Event::Message(ServerMessage::Transcript {
                utterance,
                finished: true,
                ..
            }) => {
                if self.forget_utterance(utterance) {
                    self.feedback.emit(Feedback::Sent, now);
                }
            }
            Event::Message(ServerMessage::TranscriptFailed { utterance, .. }) => {
                if self.forget_utterance(utterance) {
                    self.feedback.emit(Feedback::NotSent(Hand::Left), now);
                }
            }
            Event::Message(ServerMessage::Speech { replay: true, .. }) => {
                if self.replay.since.replace(now).is_none() && self.replay.asked.is_none() {
                    self.feedback.emit(Feedback::CatchingUp, now);
                }
            }
            // Words being recorded when the connection drops are lost. Words already released may still be queued by the server, but their transcript cannot reach this client, so they are forgotten without a cue, as the conversation shows no failure for them.
            Event::State(ConnectionState::Disconnected { .. }) => {
                self.replay = Replay::default();
                let lost = self.end_recording().is_some();
                self.utterances.clear();
                if lost {
                    self.feedback.emit(Feedback::NotSent(Hand::Left), now);
                }
            }
            // Live speech stops a replay.
            Event::Message(ServerMessage::Speech { replay: false, .. } | ServerMessage::Left)
            | Event::State(_) => self.replay = Replay::default(),
            _ => {}
        }
    }

    /// The utterance being recorded ended without being sent: forget it.
    fn end_recording(&mut self) -> Option<String> {
        let utterance = self.recording.take()?;
        self.utterances.retain(|id| *id != utterance);
        Some(utterance)
    }

    fn forget_utterance(&mut self, utterance: &str) -> bool {
        if self.recording.as_deref() == Some(utterance) {
            self.recording = None;
        }
        let known = self.utterances.iter().position(|id| id == utterance);
        known.map(|index| self.utterances.remove(index)).is_some()
    }

    /// Cue the requests and knocks waiting, and the recording nearing the limit, and play the cues due.
    fn cue(&mut self, conversation: &Conversation) {
        let now = Instant::now();
        self.cue_limit(
            conversation.recording(),
            conversation.utterance_limit(),
            now,
        );
        let over = match self.replay.since {
            Some(since) => now.saturating_duration_since(since) >= REPLAY_OVER,
            None => self
                .replay
                .asked
                .is_some_and(|asked| now.saturating_duration_since(asked) >= REPLAY_START),
        };
        if over && !self.speaker.is_replaying() {
            self.replay = Replay::default();
        }
        // Visitors cannot decide requests or let anyone in, so nothing waits for them.
        let space = conversation
            .joined_space()
            .map(|(space, _)| space)
            .filter(|_| conversation.role().participates());
        let requests = space.into_iter().flat_map(|space| {
            let approval = space.approval.as_ref().map(|approval| approval.id.as_str());
            let question = space.question.as_ref().map(|question| question.id.as_str());
            approval.into_iter().chain(question)
        });
        self.feedback.observe_requests(requests, now);
        let knocks = space
            .into_iter()
            .flat_map(|_| conversation.knocks().iter().map(|knock| knock.id.as_str()));
        self.feedback.observe_knocks(knocks, now);
        self.play_cues();
    }

    /// Cue a recording that has been going for `elapsed` reaching 30 and then 10 seconds before `limit`, as the recording card turns amber and then red; only the nearer when both are passed at once. See `design/feedback.md`.
    fn cue_limit(&mut self, elapsed: Option<Duration>, limit: Option<Duration>, now: Instant) {
        let Some(elapsed) = elapsed else {
            self.limit_cued = 0;
            return;
        };
        let Some(left) = limit.map(|limit| limit.saturating_sub(elapsed)) else {
            return;
        };
        let stage = if left <= crate::recording::CRITICAL {
            2
        } else if left <= crate::recording::WARNING {
            1
        } else {
            0
        };
        if stage > self.limit_cued {
            self.limit_cued = stage;
            self.feedback.emit(
                if stage == 2 {
                    Feedback::LimitCritical
                } else {
                    Feedback::LimitWarning
                },
                now,
            );
        }
    }

    /// Play the cues due. Nothing plays without a connection and an open speaker, and a cue played late would mislead, so they are dropped then.
    fn play_cues(&mut self) {
        match self
            .speaker
            .control()
            .filter(|_| self.speaker.output.is_some())
        {
            Some(control) => {
                for (earcon, direction) in self.feedback.take_earcons() {
                    control.play(earcon, direction);
                }
            }
            None => self.feedback.take_earcons().for_each(drop),
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
                Action::Send(message) => {
                    match &message {
                        ClientMessage::Replay { .. } => self.replay.asked = Some(Instant::now()),
                        ClientMessage::StopReplay => self.replay = Replay::default(),
                        _ => {}
                    }
                    match &self.client {
                        Some(client) => {
                            if let Err(error) = client.send(message) {
                                self.feedback
                                    .emit(Feedback::NotSent(Hand::Right), Instant::now());
                                conversation.fail(format!("Cannot send: {error}"));
                            }
                        }
                        None => {
                            self.feedback
                                .emit(Feedback::NotSent(Hand::Right), Instant::now());
                            conversation.fail("Not connected.");
                        }
                    }
                }
                Action::BeginUtterance(utterance, generation, answer) => {
                    // The opening tick plays through the silencing, as X is pressed and before the microphone has opened; see `design/live-audio.md`. A live microphone is already open and would hear it, so then it is only felt.
                    self.feedback.emit_heard(
                        Feedback::RecordingStarts,
                        !self.voice.live,
                        Instant::now(),
                    );
                    self.play_cues();
                    // Silence the speaker before the microphone opens.
                    if let Some(control) = self.speaker.control() {
                        control.set_silenced(true);
                    }
                    self.utterances.retain(|id| *id != utterance);
                    self.utterances.push(utterance.clone());
                    self.recording = Some(utterance.clone());
                    let begun = match &self.client {
                        Some(client) => self.voice.begin(client, utterance, generation, answer),
                        None => Err("Not connected.".into()),
                    };
                    if let Err(error) = begun {
                        log::warn!("Cannot record: {error}");
                        self.end_recording();
                        self.feedback
                            .emit(Feedback::NotSent(Hand::Left), Instant::now());
                        self.voice.recording = None;
                        conversation.microphone_failed(error);
                    }
                }
                Action::EndUtterance(pointed) => {
                    self.feedback.emit(Feedback::RecordingStops, Instant::now());
                    let released = self.recording.take();
                    match self
                        .voice
                        .finish_pointing(self.client.as_ref(), true, pointed)
                    {
                        Ok(()) => {}
                        Err(Finish::Utterance(error)) => {
                            self.utterances.retain(|id| Some(id) != released.as_ref());
                            self.feedback
                                .emit(Feedback::NotSent(Hand::Left), Instant::now());
                            conversation.utterance_not_sent(error)
                        }
                        Err(Finish::Live(error)) => conversation.microphone_failed(error),
                    }
                }
                Action::CancelUtterance => {
                    self.end_recording();
                    if let Err(error) = self.voice.finish(self.client.as_ref(), false) {
                        conversation.fail(String::from(error));
                    }
                }
                Action::AbandonUtterance => {
                    self.end_recording();
                    if let Err(error) = self.voice.abandon() {
                        conversation.fail(error);
                    }
                }
                Action::StartLive(generation) => {
                    let result = match &self.client {
                        Some(client) => self.voice.start_live(client, generation),
                        None => Err("Not connected.".into()),
                    };
                    match result {
                        Ok(()) => {
                            self.feedback.emit(Feedback::LiveOn, Instant::now());
                            if let Some(test) = &mut self.echo_test {
                                *test = EchoTest {
                                    armed: Some(Instant::now()),
                                    speaker: None,
                                    opened: None,
                                    probed: None,
                                    floor: Heard::default(),
                                    echo: Heard::default(),
                                    ..*test
                                };
                            }
                        }
                        Err(error) => {
                            self.voice.recording = None;
                            self.feedback
                                .emit(Feedback::NotSent(Hand::Left), Instant::now());
                            conversation.microphone_failed(error);
                        }
                    }
                }
                Action::StopLive => {
                    // Only a microphone that was live turns off; one that failed to start never turned on.
                    if self.voice.live {
                        self.feedback.emit(Feedback::LiveOff, Instant::now());
                    }
                    if let Err(error) = self.voice.stop_live() {
                        self.voice.recording = None;
                        conversation.microphone_failed(error);
                    }
                }
                Action::SetAgentMuted(muted) => self.speaker.set_muted(muted),
                Action::SetFeedback(settings) => {
                    self.feedback.set_settings(settings);
                    self.store.remember_feedback(settings);
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
        self.utterances.clear();
        self.recording = None;
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
        voice.begin(&client, "u1".into(), 0, None).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(utterance(&voice).as_deref(), Some("u1"));
        voice.finish(Some(&client), true).unwrap();
        assert!(voice.recording.is_none());
        assert_eq!(utterance(&voice), None);

        voice.begin(&client, "u2".into(), 0, None).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        voice.abandon().unwrap();
        assert_eq!(utterance(&voice), None);

        voice.begin(&client, "u3".into(), 0, None).unwrap();
        voice.reset();
        assert!(voice.lock().microphone.is_none());
        client.close();
    }

    #[test]
    fn a_new_rate_gets_a_new_microphone() {
        let client = client();
        let rate = Arc::new(AtomicU32::new(16_000));
        let mut voice = voice(&rate, true, None);
        voice.begin(&client, "u1".into(), 0, None).unwrap();
        voice.finish(Some(&client), false).unwrap();
        rate.store(48_000, Ordering::Relaxed);
        voice.begin(&client, "u2".into(), 0, None).unwrap();
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
        let error = voice.begin(&client, "u1".into(), 0, None).unwrap_err();
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
        voice.begin(&client, "u1".into(), 0, None).unwrap();
        assert!(voice.talking());
        assert_eq!(
            voice.lock().microphone.as_ref().unwrap().gate(),
            Gate::Agent
        );
        voice.finish(Some(&client), true).unwrap();
        assert!(voice.recording.is_some());
        assert_eq!(voice.lock().microphone.as_ref().unwrap().gate(), Gate::Live);
        voice.begin(&client, "u2".into(), 0, None).unwrap();
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
                .begin(&client, "u1".into(), 0, None)
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
        voice.begin(&client, "u1".into(), 0, None).unwrap();
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
        voice.begin(&client, "u2".into(), 0, None).unwrap();
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
        voice.begin(&client, "u1".into(), 0, None).unwrap();
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
        voice.begin(&client, "u1".into(), 0, None).unwrap();
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
        voice.begin(&client, "u1".into(), 0, None).unwrap();
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
        assert!(voice.begin(&client, "u1".into(), 1, None).is_err());
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
        voice.begin(&client, "u1".into(), 0, None).unwrap();
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
        voice.begin(&client, "u1".into(), 0, None).unwrap();
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
            _route: Route,
        ) -> Result<crate::playback::Output, String> {
            if self.refuse.load(Ordering::Relaxed) {
                return Err("No speaker is available.".into());
            }
            let mut output = [1; 48];
            player
                .lock()
                .unwrap()
                .render(48_000, client::playback::Layout::Mono, &mut output);
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

    fn feedback_connection() -> (Connection, Conversation) {
        let mut conversation = Conversation::new("Ada");
        let store = Store::new(None);
        let identity = Identity::new(&store, None, Default::default());
        let (voice, _) = kind_voice();
        let connection = Connection::new(
            identity,
            store,
            Box::new(()),
            voice.capture,
            Box::new(FakeSpeaker::default()),
            &mut conversation,
        );
        (connection, conversation)
    }

    fn cues(connection: &mut Connection) -> Vec<client::playback::Earcon> {
        connection
            .feedback
            .take_earcons()
            .map(|(earcon, _)| earcon)
            .collect()
    }

    fn message(message: ServerMessage) -> Event {
        Event::Message(message)
    }

    #[test]
    fn a_recording_nearing_the_limit_is_felt_at_30_and_10_seconds_left() {
        let (mut connection, _conversation) = feedback_connection();
        connection.feedback.set_haptics(true);
        let now = Instant::now();
        let limit = Some(Duration::from_secs(60));
        let felt = |connection: &mut Connection| {
            connection
                .feedback
                .haptics(now + Duration::from_secs(1))
                .len()
        };
        connection.cue_limit(Some(Duration::from_secs(20)), limit, now);
        assert_eq!(felt(&mut connection), 0);
        connection.cue_limit(Some(Duration::from_secs(30)), limit, now);
        assert_eq!(felt(&mut connection), 1);
        connection.cue_limit(Some(Duration::from_secs(40)), limit, now);
        assert_eq!(felt(&mut connection), 0);
        connection.cue_limit(Some(Duration::from_secs(50)), limit, now);
        assert_eq!(felt(&mut connection), 2);
        connection.cue_limit(Some(Duration::from_secs(55)), limit, now);
        assert_eq!(felt(&mut connection), 0);
        // Felt, never heard; a new recording starts again; no limit, no cue.
        assert!(cues(&mut connection).is_empty());
        connection.cue_limit(None, limit, now);
        connection.cue_limit(Some(Duration::from_secs(55)), limit, now);
        assert_eq!(felt(&mut connection), 2);
        connection.cue_limit(None, None, now);
        connection.cue_limit(Some(Duration::from_secs(55)), None, now);
        assert_eq!(felt(&mut connection), 0);
    }

    #[test]
    fn this_participants_words_are_cued_when_sent_or_failed() {
        use client::playback::Earcon;
        let (mut connection, _conversation) = feedback_connection();
        connection.utterances = vec!["u-1".into(), "u-2".into()];
        let transcript = |utterance: &str, finished| {
            message(ServerMessage::Transcript {
                utterance: utterance.into(),
                text: "hello".into(),
                finished,
            })
        };
        // Partial words, and another participant's, are not cued.
        connection.observe(&transcript("u-1", false));
        connection.observe(&transcript("someone-else", true));
        assert!(cues(&mut connection).is_empty());
        connection.observe(&transcript("u-1", true));
        assert_eq!(cues(&mut connection), [Earcon::Sent]);
        connection.observe(&transcript("u-1", true));
        assert!(cues(&mut connection).is_empty());
        connection.observe(&message(ServerMessage::TranscriptFailed {
            utterance: "u-2".into(),
            error: "Too long.".into(),
        }));
        assert_eq!(cues(&mut connection), [Earcon::Failed]);
        assert!(connection.utterances.is_empty());
    }

    #[test]
    fn a_catch_up_nobody_asked_for_is_cued_once() {
        use client::playback::Earcon;
        let (mut connection, mut conversation) = feedback_connection();
        let speech = |replay| {
            message(ServerMessage::Speech {
                segment: 0,
                message: "m".into(),
                text: "Hello.".into(),
                sample_rate: 24_000,
                replay,
            })
        };
        connection.observe(&speech(true));
        connection.observe(&speech(true));
        assert_eq!(cues(&mut connection), [Earcon::CatchingUp]);
        // Live speech ends it; a replay asked for is not cued.
        connection.observe(&speech(false));
        conversation.push_action(Action::Send(ClientMessage::Replay { message: None }));
        connection.perform(&mut conversation);
        connection.observe(&speech(true));
        assert!(!cues(&mut connection).contains(&Earcon::CatchingUp));
        // Stopping it, or a quiet player some time after its last segment, ends it too.
        conversation.push_action(Action::Send(ClientMessage::StopReplay));
        connection.perform(&mut conversation);
        connection.observe(&speech(true));
        assert!(cues(&mut connection).contains(&Earcon::CatchingUp));
        connection.replay.since = Some(Instant::now() - REPLAY_OVER);
        connection.cue(&conversation);
        assert!(connection.replay.since.is_none());
    }

    #[test]
    fn a_request_that_cannot_be_sent_is_cued_on_the_right_hand_and_the_setting_is_kept() {
        use crate::feedback::{Level, Settings};
        use client::playback::Earcon;
        let (mut connection, mut conversation) = feedback_connection();
        connection.feedback.set_haptics(true);
        conversation.push_action(Action::Send(ClientMessage::Leave));
        connection.perform(&mut conversation);
        assert_eq!(cues(&mut connection), [Earcon::Failed]);
        let felt = connection.feedback.haptics(Instant::now());
        assert_eq!(felt[0].0, Hand::Right);
        let quiet = Settings {
            sounds: Level::Off,
            vibration: Level::Reduced,
            mono: false,
        };
        conversation.push_action(Action::SetFeedback(quiet));
        connection.perform(&mut conversation);
        assert_eq!(connection.feedback.settings(), quiet);
        // Without a connection, cues have nowhere to play and are dropped.
        connection.feedback.set_settings(Settings::default());
        connection.feedback.emit(Feedback::Choose, Instant::now());
        connection.cue(&conversation);
        assert!(cues(&mut connection).is_empty());
    }

    #[test]
    fn the_recording_tick_is_heard_only_while_the_live_microphone_is_off() {
        let (mut connection, mut conversation) = feedback_connection();
        let (_audio, received) = std::sync::mpsc::sync_channel(1);
        let (player, control) = Player::new(received, MixerOptions::default());
        let player = Arc::new(Mutex::new(player));
        connection.speaker.player = Some((player.clone(), control));
        assert_eq!(connection.speaker.open(), None);
        until(|| {
            connection
                .speaker
                .output
                .as_ref()
                .is_some_and(Speaking::is_open)
        });
        let heard = |player: &crate::playback::Shared| {
            let mut output = vec![0; 4_800];
            player
                .lock()
                .unwrap()
                .render(48_000, client::playback::Layout::Mono, &mut output);
            output.iter().any(|sample| *sample != 0)
        };
        // Without a connection recording fails at once; the tick still plays through the silencing, and the failure does not.
        conversation.push_action(Action::BeginUtterance("u-1".into(), 0, None));
        connection.perform(&mut conversation);
        assert!(heard(&player));
        assert!(connection.utterances.is_empty());
        assert!(connection.recording.is_none());
        connection.voice.live = true;
        conversation.push_action(Action::BeginUtterance("u-2".into(), 0, None));
        connection.perform(&mut conversation);
        assert!(!heard(&player));
    }

    #[test]
    fn the_live_microphone_is_cued_only_when_it_turns_on_or_off() {
        use client::playback::Earcon;
        let (mut connection, mut conversation) = feedback_connection();
        // Failing to start is a failure, and stopping what never started is silent.
        conversation.push_action(Action::StartLive(0));
        connection.perform(&mut conversation);
        assert_eq!(cues(&mut connection), [Earcon::Failed]);
        conversation.push_action(Action::StopLive);
        connection.perform(&mut conversation);
        assert!(cues(&mut connection).is_empty());
        connection.voice.live = true;
        conversation.push_action(Action::StopLive);
        connection.perform(&mut conversation);
        assert_eq!(cues(&mut connection), [Earcon::Off]);
    }

    #[test]
    fn words_on_their_way_are_lost_when_the_connection_drops() {
        use client::playback::Earcon;
        let (mut connection, mut conversation) = feedback_connection();
        let dropped = || {
            Event::State(ConnectionState::Disconnected {
                reason: Disconnect::Timeout,
                retry_in: Duration::from_secs(1),
            })
        };
        connection.observe(&dropped());
        assert!(cues(&mut connection).is_empty());
        connection.utterances = vec!["u-1".into()];
        connection.recording = Some("u-1".into());
        connection.observe(&dropped());
        assert_eq!(cues(&mut connection), [Earcon::Failed]);
        assert!(connection.utterances.is_empty());
        // Words already released may still be queued, so only a recording is cued as lost; the others are forgotten.
        connection.utterances = vec!["u-2".into()];
        connection.observe(&dropped());
        assert!(cues(&mut connection).is_empty());
        assert!(connection.utterances.is_empty());
        // Cancelling or abandoning forgets only the utterance being recorded, not one still being transcribed.
        connection.utterances = vec!["u-2".into(), "u-3".into()];
        connection.recording = Some("u-3".into());
        conversation.push_action(Action::CancelUtterance);
        connection.perform(&mut conversation);
        assert_eq!(connection.utterances, ["u-2"]);
        connection.utterances.push("u-4".into());
        connection.recording = Some("u-4".into());
        connection.observe(&message(ServerMessage::TranscriptFailed {
            utterance: "u-4".into(),
            error: "Too long.".into(),
        }));
        conversation.push_action(Action::AbandonUtterance);
        connection.perform(&mut conversation);
        assert_eq!(connection.utterances, ["u-2"]);
    }

    #[test]
    fn a_replay_asked_for_that_never_comes_stops_counting_after_a_while() {
        use client::playback::Earcon;
        let (mut connection, mut conversation) = feedback_connection();
        conversation.push_action(Action::Send(ClientMessage::Replay { message: None }));
        connection.perform(&mut conversation);
        cues(&mut connection);
        assert!(connection.replay.asked.is_some());
        connection.replay.asked = Some(Instant::now() - REPLAY_OVER);
        connection.cue(&conversation);
        assert!(connection.replay.asked.is_some());
        connection.replay.asked = Some(Instant::now() - REPLAY_START);
        connection.cue(&conversation);
        assert!(connection.replay.asked.is_none());
        connection.observe(&message(ServerMessage::Speech {
            segment: 0,
            message: "m".into(),
            text: "Hello.".into(),
            sample_rate: 24_000,
            replay: true,
        }));
        assert_eq!(cues(&mut connection), [Earcon::CatchingUp]);
    }

    #[test]
    fn the_echo_test_hears_the_room_then_the_probe_and_reports_once() {
        let (mut connection, _conversation) = feedback_connection();
        connection.set_echo_test("test", false);
        let test = connection.echo_test.as_mut().unwrap();
        let start = Instant::now();
        let at = |milliseconds: u64| start + Duration::from_millis(milliseconds);
        let heard = |level: f64| Heard {
            energy: level * level * 320.0,
            samples: 320,
        };
        // Before the live microphone opens, nothing.
        assert_eq!(test.update(heard(1.0), at(0)), (None, false));
        test.opened = Some(start);
        // The first 300 ms are ignored, then the room is heard until the probe plays at one second.
        assert_eq!(test.update(heard(1.0), at(100)), (None, false));
        assert_eq!(test.update(heard(0.001), at(500)), (None, false));
        assert_eq!(test.update(heard(0.001), at(900)), (None, false));
        assert_eq!(test.update(Heard::default(), at(1_000)), (None, true));
        assert_eq!(test.update(heard(1.0), at(1_100)), (None, false));
        assert_eq!(test.update(heard(0.01), at(1_400)), (None, false));
        assert_eq!(test.update(heard(0.01), at(2_200)), (None, false));
        let (result, probe) = test.update(Heard::default(), at(2_400));
        assert!(!probe);
        assert_eq!(
            result.as_deref(),
            Some(
                "Echo test (test, the speaker as media): the speaker played a probe at -20.0 dBFS; the microphone heard -40.0 dBFS during it, 20.0 dB below the probe, 20.0 dB above the room; the room was -60.0 dBFS"
            )
        );
        // Once only, until the microphone opens again.
        assert_eq!(test.update(heard(1.0), at(3_000)), (None, false));
        assert_eq!(Heard::default().level(), None);
    }

    #[test]
    fn the_echo_test_waits_for_the_speaker_to_play_as_it_should() {
        let (mut connection, mut conversation) = feedback_connection();
        let (_audio, received) = std::sync::mpsc::sync_channel(1);
        let (player, control) = Player::new(received, MixerOptions::default());
        connection.speaker.player = Some((Arc::new(Mutex::new(player)), control));
        connection.set_echo_test("test", true);
        assert!(connection.speaker.probe);
        // Live, but the speaker is not open yet: still waiting.
        connection.voice.live = true;
        connection.echo_test.as_mut().unwrap().armed = Some(Instant::now());
        connection.listen_live(&mut conversation);
        assert!(connection.echo_test.as_ref().unwrap().opened.is_none());
        assert_eq!(connection.speaker.open(), None);
        until(|| {
            connection
                .speaker
                .output
                .as_ref()
                .is_some_and(Speaking::is_open)
        });
        // Open, but as media rather than in the microphone's session, which this test needs: it waits, and gives up after a while.
        connection.listen_live(&mut conversation);
        assert!(connection.echo_test.as_ref().unwrap().armed.is_some());
        connection.echo_test.as_mut().unwrap().armed = Some(Instant::now() - ECHO_WAIT);
        connection.listen_live(&mut conversation);
        let test = connection.echo_test.as_ref().unwrap();
        assert!(test.armed.is_none() && test.opened.is_none());
        // A test that needs no session starts once the speaker is open, as media.
        let test = connection.echo_test.as_mut().unwrap();
        test.routed = false;
        test.armed = Some(Instant::now());
        connection.listen_live(&mut conversation);
        let test = connection.echo_test.as_ref().unwrap();
        assert!(test.armed.is_none() && test.opened.is_some());
        assert_eq!(test.speaker, None);
        // Live audio stopping ends the test.
        connection.voice.live = false;
        connection.listen_live(&mut conversation);
        assert!(connection.echo_test.as_ref().unwrap().opened.is_none());
    }

    #[test]
    fn rerouting_the_speaker_reopens_it_only_when_the_route_changes() {
        let mut speaker = Speaker::new(Box::new(FakeSpeaker::default()));
        let route = Route {
            communication: Some(42),
        };
        // Nothing open: only remembered for the next open.
        assert_eq!(speaker.reroute(route), None);
        assert_eq!(speaker.route, route);
        assert_eq!(speaker.reroute(route), None);
        assert!(speaker.output.is_none());
    }

    #[test]
    fn rerouting_an_open_speaker_reopens_it_once() {
        let fake = FakeSpeaker::default();
        let opened = fake.opened.clone();
        let mut speaker = Speaker::new(Box::new(fake));
        let (_audio, received) = std::sync::mpsc::sync_channel(1);
        let (player, control) = Player::new(received, MixerOptions::default());
        speaker.player = Some((Arc::new(Mutex::new(player)), control));
        assert_eq!(speaker.open(), None);
        until(|| opened.load(Ordering::Relaxed) == 1);
        let route = Route {
            communication: Some(42),
        };
        assert_eq!(speaker.reroute(route), None);
        until(|| opened.load(Ordering::Relaxed) == 2);
        until(|| speaker.output.as_ref().is_some_and(Speaking::is_open));
        // The fake opens as media, as a speaker that fell back would.
        assert_eq!(speaker.output.as_ref().unwrap().communication(), None);
        assert_eq!(speaker.reroute(route), None);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(opened.load(Ordering::Relaxed), 2);
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
