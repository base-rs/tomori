// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::collections::BTreeMap;
use std::future::{Future, pending};
use std::io;
use std::net::SocketAddr;
use std::pin::{Pin, pin};
use std::sync::Arc;
use std::time::Duration;

use base_endpoint::tls::TlsAcceptor;
use base_endpoint::tls::rustls::RustlsAcceptor;
use base_http_server::{Error, Statistics};
use base_scheduler::{
    Cancellation, OperationError, OperationResult, SocketAccept, SocketAddress, SocketBind,
    SocketClose, SocketListen, SocketOpen, SocketReuseAddress, SocketSetOption,
    SocketStreamReading, SocketStreamWriting, SocketType, Timers,
};
use futures_lite::future::zip;
use futures_util::future::{Either, select};
use futures_util::stream::{FuturesUnordered, StreamExt};
use tomori_backend::{Backend, Event, Session, SessionSpec};
use tomori_discovery::Counts;
use tomori_protocol::Keepalive;

use crate::admission::{Admission, Admitted, DEFAULT_REQUEST_TIMEOUT, Watch};
use crate::application::Application;
use crate::backends::{AnySession, Backends};
use crate::door::Door;
use crate::environments::{Environment, Environments};
use crate::hub::{CreateRequest, Hub, Job, Restore, SpaceCommand};
use crate::transcriber::Transcriber;
use crate::voice::{Voice, Voices};
use crate::workspace::{
    DEFAULT_MAX_SPACES, DIRECTORY_UNAVAILABLE, RESOLVE_TIMEOUT, Resolving, SpaceSlots,
    WorkspaceRoots,
};

/// A Tomori server: the HTTP application plus the supervisor that owns every space's backend session.
///
/// [`run`](Self::run) serves until cancelled or serving fails. All work is owned by that one future: connections, space drivers and the keepalive sweep are joined by it. No detached tasks are created. Speech recognition and synthesis run on the speech crate's own threads, which outlive a space only to finish speech already queued.
pub struct Server {
    hub: Arc<Hub>,
    jobs: async_channel::Receiver<Job>,
    application: Application,
    voices: Voices,
    keepalive: Keepalive,
    admission: Admission,
    request_timeout: Duration,
    retention: Duration,
    /// Where spaces may run; `None` until configured, which means the current directory when the server runs.
    roots: Option<WorkspaceRoots>,
    slots: Arc<SpaceSlots>,
    /// How long cancelled connections may take to finish once serving ends; see [`DEFAULT_DRAIN_TIMEOUT`].
    drain_timeout: Duration,
    /// The most connections served at once; see [`MAX_CONNECTIONS`].
    max_connections: usize,
    /// Told the counts to advertise as they change; see [`Self::with_counts`].
    counts: Option<Box<dyn Fn(Counts) + Send + Sync>>,
    counts_interval: Duration,
    /// Serve TLS with this configuration, rather than plain HTTP; see [`Self::with_tls`].
    tls: Option<Arc<rustls::ServerConfig>>,
    /// Spaces listed again from the record of kept spaces, for the supervisor to bind to their sessions once it runs; see [`Self::with_kept_spaces`].
    restoring: std::sync::Mutex<Vec<Restore>>,
    /// Whether the counts include how many spaces need attention; see [`Self::with_attention_advertised`].
    advertise_attention: bool,
    /// Where the cards come from, for the spaces that show them; see [`Self::with_github`].
    cards: Option<Arc<dyn crate::cards::CardSource>>,
}

/// Confirms or declines the pairing waiting, from outside the server, such as its console. See `design/security.md`.
///
/// It does not keep the server: once the server is dropped, its state, and the lock on its state directory, go with it, though a console thread still holds this, and the handle then finds no pairing.
#[derive(Clone)]
pub struct Pairing(std::sync::Weak<Hub>);

impl Pairing {
    /// The pairing pending, revealed or not, if any.
    pub fn pending(&self) -> Option<crate::hub::PairingView> {
        self.0.upgrade()?.pending_pairing()
    }

    /// Confirm the pairing waiting, if `typed` is the code its device shows. Returns the device's name.
    pub fn confirm(&self, typed: &str) -> Result<String, &'static str> {
        self.0
            .upgrade()
            .ok_or("The server has stopped.")?
            .confirm_pairing(crate::hub::Confirmation::Typed(typed))
    }

    /// Decline the pairing pending, revealed or not. Returns whether one was pending.
    pub fn decline(&self) -> bool {
        self.0
            .upgrade()
            .is_some_and(|hub| hub.decline_pairing(None))
    }

    /// Whether pairings are paused, since too many began recently.
    pub fn paused(&self) -> bool {
        self.0.upgrade().is_some_and(|hub| hub.pairing_paused())
    }

    /// Allow pairings again after they were paused.
    pub fn allow(&self) {
        if let Some(hub) = self.0.upgrade() {
            hub.allow_pairing();
        }
    }
}

/// How long a disconnected participant keeps their identity, audio source and space, so that a headset that slept or walked out of range for a while comes back where it was. See `design/spaces.md`.
pub const DEFAULT_PARTICIPANT_RETENTION: Duration = Duration::from_secs(12 * 60 * 60);

/// The most visitors connected at once, so that visitors cannot take every connection slot from participants. See `design/security.md`.
pub const DEFAULT_MAX_VISITORS: usize = 64;

/// The most connections served at once; further peers wait in the listen backlog.
const MAX_CONNECTIONS: usize = 256;

/// How long accepting pauses after the process or system has run out of descriptors, buffers or memory, so that the loop does not spin while connections finish and free them.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// How long cancelled connections may take to finish once serving ends, before they are abandoned so that shutdown completes. See `design/server.md`.
pub const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// The shortest time between two advertised counts, and between starting to serve and the first. Changes in between are coalesced, so a burst of connections or spaces is advertised once, with the counts at its end. Each update is a multicast announcement, and browsers only replace a TXT record they have held for a second (see [`tomori_discovery::Advertisement::update`]), so updates must be at least that far apart. See `design/server.md`.
pub const COUNTS_INTERVAL: Duration = Duration::from_secs(2);

/// Open a listening TCP socket.
pub async fn bind<S>(
    scheduler: &S,
    address: SocketAddr,
    cancellation: &Cancellation,
) -> OperationResult<S::Socket>
where
    S: SocketOpen + SocketBind + SocketListen + SocketSetOption<SocketReuseAddress>,
{
    let socket = scheduler
        .socket_open(address.into(), SocketType::Stream, None, cancellation)
        .await?;
    scheduler
        .socket_set_option(&socket, SocketReuseAddress, true, cancellation)
        .await?;
    scheduler
        .socket_bind(&socket, address, cancellation)
        .await?;
    scheduler.socket_listen(&socket, 128, cancellation).await?;
    Ok(socket)
}

/// Read the cards from `source` on a thread of its own, or `None` once `cancellation` is cancelled first. A thread that cannot start, or ends without an answer, is a failed read.
async fn read_cards(
    source: Arc<dyn crate::cards::CardSource>,
    cancellation: &Cancellation,
) -> Option<Result<crate::cards::Fetched, crate::cards::Failure>> {
    use crate::cards::Failure;
    let (sender, receiver) = async_channel::bounded(1);
    let started = crate::system::spawn("tomori-cards", move || {
        let _ = sender.send_blocking(source.fetch());
    });
    if let Err(error) = started {
        return Some(Err(Failure::new(format!(
            "could not start reading: {error}"
        ))));
    }
    match select(pin!(receiver.recv()), pin!(cancellation.cancelled())).await {
        Either::Left((Ok(result), _)) => Some(result),
        Either::Left((Err(_), _)) => Some(Err(Failure::new("reading ended without an answer"))),
        Either::Right(_) => None,
    }
}

impl Server {
    pub fn new(backends: Backends, transcriber: Arc<dyn Transcriber>) -> Self {
        let (sender, jobs) = async_channel::unbounded();
        let slots = SpaceSlots::new(DEFAULT_MAX_SPACES);
        let hub = Hub::new(backends, transcriber, sender, Vec::new(), slots.clone());
        hub.set_max_visitors(DEFAULT_MAX_VISITORS);
        Self {
            application: Application::new(hub.clone()),
            hub,
            jobs,
            voices: Voices::default(),
            keepalive: Keepalive::default(),
            admission: Admission::default(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            retention: DEFAULT_PARTICIPANT_RETENTION,
            roots: None,
            slots,
            drain_timeout: DEFAULT_DRAIN_TIMEOUT,
            max_connections: MAX_CONNECTIONS,
            counts: None,
            counts_interval: COUNTS_INTERVAL,
            tls: None,
            restoring: std::sync::Mutex::new(Vec::new()),
            advertise_attention: true,
            cards: None,
        }
    }

    /// Serve only TLS, with `config`, such as from [`tomori_security::ServerCertificate::server_config`]. A handshake that has not completed within the request timeout is closed, as a request head would be. See `design/security.md`.
    pub fn with_tls(mut self, config: Arc<rustls::ServerConfig>) -> Self {
        self.tls = Some(config);
        self.hub.set_encrypted(true);
        self
    }

    /// Keep spaces across restarts in `store`, and list again those it kept, which are bound to their sessions once the server runs. They are listed at once, so that clients restoring them and collaborators admitted to them find them. See `design/spaces.md`.
    pub fn with_kept_spaces(self, store: Arc<crate::kept::SpaceStore>) -> Self {
        let restores = self.hub.restore(store);
        self.restoring
            .lock()
            .expect("never left poisoned")
            .extend(restores);
        self
    }

    /// How many spaces are listed, participants connected and, unless turned off, spaces need attention now, for advertising before [`run`](Self::run) starts.
    pub fn counts(&self) -> Counts {
        let mut counts = self.hub.counts();
        if !self.advertise_attention {
            counts.attention = None;
        }
        counts
    }

    /// Whether the counts advertised include how many spaces need attention, which is the default. Turned off, they are left out of the counts, so they are never published and changes to them publish nothing. See `design/server.md`.
    pub fn with_attention_advertised(mut self, advertise: bool) -> Self {
        self.advertise_attention = advertise;
        self
    }

    /// While running, call `publish` with the counts whenever they change, at most once per [`COUNTS_INTERVAL`], so that they can be advertised. The first call reports a change from the counts when [`run`](Self::run) started.
    pub fn with_counts(mut self, publish: impl Fn(Counts) + Send + Sync + 'static) -> Self {
        self.counts = Some(Box::new(publish));
        self
    }

    /// Change the shortest time between published counts; see [`COUNTS_INTERVAL`].
    pub fn with_counts_interval(mut self, interval: Duration) -> Self {
        self.counts_interval = interval;
        self
    }

    /// Run spaces only in these directories or beneath them. Without this, the only root is the current directory when the server runs. See `design/workspaces.md`.
    pub fn with_workspace_roots(mut self, roots: WorkspaceRoots) -> Self {
        self.hub.set_roots(roots.to_protocol());
        self.roots = Some(roots);
        self
    }

    /// Offer `environments` for spaces, in order, in place of Tomori's own, Sky and the Minimal grid: the first is the one a space whose creator names none is bound to. A space kept across a restart in an environment no longer offered stays in it, with no instructions from it. It may be called before or after [`Self::with_kept_spaces`]: a space's layer of the instructions is looked up in the environments offered whenever it is shown or given to the agent, so a restored space is given the same layer as a new one. See `design/environments.md`.
    pub fn with_environments(self, environments: impl IntoIterator<Item = Environment>) -> Self {
        self.hub.set_environments(Environments::new(environments));
        self
    }

    /// Offer the GitHub environment, a prototype, whose spaces show `github`'s open issues and pull requests as cards
    /// that participants lay out. The server reads them, read-only, with the token it holds, about once a minute while
    /// someone watches them, and never sends the token to clients, the agent or the logs. Call it after
    /// [`Self::with_environments`], which replaces the environments offered. See `design/github.md`.
    pub fn with_github(self, github: crate::github::GitHub) -> Self {
        self.with_cards(crate::github::ENVIRONMENT, "GitHub", Arc::new(github))
    }

    /// Offer the environment `id`, labelled `label`, whose spaces show the cards `source` reads.
    pub(crate) fn with_cards(
        mut self,
        id: &str,
        label: &str,
        source: Arc<dyn crate::cards::CardSource>,
    ) -> Self {
        let name = source.source();
        self.hub.offer_environment(
            Environment::new(id, label).with_instructions(crate::github::instructions(&name)),
        );
        self.hub.set_board(crate::cards::Board::new(id, name));
        self.cards = Some(source);
        self
    }

    /// The environments offered.
    #[cfg(test)]
    pub(crate) fn environments(&self) -> Vec<tomori_protocol::EnvironmentInfo> {
        self.hub.environment_info()
    }

    /// Use `roots` unless roots were already given.
    #[cfg(feature = "fake")]
    pub(crate) fn with_default_workspace_roots(mut self, roots: WorkspaceRoots) -> Self {
        if self.roots.is_none() {
            // Keep the hub's list, shown to a client, in step with the ones used to resolve.
            self.hub.set_roots(roots.to_protocol());
        }
        self.roots.get_or_insert(roots);
        self
    }

    /// Admit holders of the tokens in `store` as what each grants, and anyone without credentials as a visitor if `visitors`, or not at all. Without this, everyone is an owner, as with `tomori-server --auth open`. See `design/security.md`.
    pub fn with_tokens(mut self, store: Arc<tomori_security::TokenStore>, visitors: bool) -> Self {
        self.hub.set_tokens(store.clone());
        self.application.set_door(Door::tokens(store, visitors));
        self
    }

    /// Pair devices, deriving the code from `fingerprint`, the certificate served with [`Self::with_tls`], and issuing tokens from the store given to [`Self::with_tokens`], which must come first. See `design/security.md`.
    ///
    /// # Panics
    ///
    /// Without a token store from [`Self::with_tokens`], since pairing issues tokens.
    pub fn with_pairing(self, fingerprint: tomori_security::Fingerprint) -> Self {
        let tokens = self
            .application
            .door()
            .store()
            .expect("pairing issues tokens, so `with_tokens` comes first")
            .clone();
        self.hub.set_pairing(crate::hub::PairingSetup {
            fingerprint,
            tokens,
        });
        self
    }

    /// Issue tokens, by pairing and knocking, over plain HTTP, as tests' raw clients need. Both otherwise need [`Self::with_tls`], so that a token never crosses the network in the clear.
    #[cfg(feature = "fake")]
    pub fn with_tokens_over_plain_http_for_tests(self) -> Self {
        self.hub.set_encrypted(true);
        self
    }

    /// Call `prompt` with each [`crate::PairingEvent`], such as a pairing that waits to be confirmed, so that the server shows it on its console.
    pub fn with_pairing_prompt(
        self,
        prompt: impl Fn(&crate::PairingEvent) + Send + Sync + 'static,
    ) -> Self {
        self.hub.set_pairing_prompt(Box::new(prompt));
        self
    }

    /// Change how long a pairing waits to be confirmed; see [`crate::PAIRING_TIMEOUT`].
    pub fn with_pairing_timeout(self, timeout: Duration) -> Self {
        self.hub.set_pairing_timeout(timeout);
        self
    }

    /// Change how long a knock waits for an answer; see [`crate::KNOCK_TIMEOUT`].
    pub fn with_knock_timeout(self, timeout: Duration) -> Self {
        self.hub.set_knock_timeout(timeout);
        self
    }

    /// A handle to confirm or decline pairings, such as from the console, while the server runs.
    pub fn pairing(&self) -> Pairing {
        Pairing(Arc::downgrade(&self.hub))
    }

    /// Change the most visitors connected at once; see [`DEFAULT_MAX_VISITORS`].
    pub fn with_max_visitors(self, limit: usize) -> Self {
        self.hub.set_max_visitors(limit);
        self
    }

    /// Change how many spaces may have a session starting or running at once; see [`DEFAULT_MAX_SPACES`].
    pub fn with_max_spaces(self, limit: usize) -> Self {
        self.slots.set_limit(limit);
        self
    }

    /// Change how long a disconnected participant is remembered; see [`DEFAULT_PARTICIPANT_RETENTION`].
    pub fn with_participant_retention(mut self, retention: Duration) -> Self {
        self.retention = retention;
        self
    }

    /// Change how long a connection may go without a complete request head, or a `/connect` exchange without Hello, before it is closed. The default is 10 seconds. See `design/server.md`.
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Change when quiet connections are pinged and when silent ones are released. See `design/connection-liveness.md`.
    pub fn with_keepalive(mut self, keepalive: Keepalive) -> Self {
        self.keepalive = keepalive;
        self
    }

    /// Speak each space's assistant replies with `synthesizer`, shared by every space. Without one, spaces are silent.
    #[cfg(feature = "speech")]
    pub fn with_synthesizer(
        mut self,
        synthesizer: impl tomori_speech::Synthesizer + 'static,
    ) -> Self {
        self.voices = Voices::new(synthesizer);
        self
    }

    /// How each space's closed blocks are split into speech segments: one per block (the default), or one per sentence.
    #[cfg(feature = "speech")]
    pub fn with_voice_mode(mut self, mode: crate::voice::SpeechMode) -> Self {
        self.voices = self.voices.with_mode(mode);
        self
    }

    /// Serve `listener` until `cancellation` is cancelled, then close every session and connection.
    ///
    /// A failure to serve, such as the listener breaking, also ends the spaces and returns the error, so that the process exits instead of living on without accepting connections. Transient accept errors are not failures; see [`Self::serve`].
    pub async fn run<S>(
        &self,
        scheduler: &S,
        listener: S::Socket,
        cancellation: &Cancellation,
    ) -> Result<Statistics, Error>
    where
        S: SocketAccept + SocketStreamReading + SocketStreamWriting + SocketClose + Timers,
        S::Socket: 'static,
    {
        // The counts when serving starts, taken before any connection is accepted: the publisher is polled after serving begins, and a client served before then would otherwise be counted as the starting point, so its arrival would never be published.
        let counts = self.counts();
        // Ends the spaces, the sweep and the pacing once serving has ended for any reason.
        let services = cancellation.child();
        let serving = async {
            let served = self.serve(scheduler, listener, cancellation).await;
            services.cancel();
            served
        };
        let (served, ((), ((), ((), ((), ()))))) = zip(
            serving,
            zip(
                self.supervise(scheduler, &services),
                zip(
                    self.sweep(scheduler, &services),
                    zip(
                        self.pace(scheduler, &services),
                        zip(
                            self.publish(scheduler, counts, &services),
                            self.refresh_cards(scheduler, &services),
                        ),
                    ),
                ),
            ),
        )
        .await;
        served
    }

    /// Report changes from `published`, the counts when serving started, to the [`with_counts`](Self::with_counts) callback, at most once per interval. A change waits until the interval since the last report, or since starting, has passed, and then the latest counts are reported, unless they have changed back.
    async fn publish<S: Timers>(
        &self,
        scheduler: &S,
        mut published: Counts,
        cancellation: &Cancellation,
    ) {
        let Some(publish) = &self.counts else {
            return;
        };
        let mut timer = scheduler.timer();
        let mut due = scheduler.now() + self.counts_interval;
        loop {
            let listener = self.hub.listen_counts();
            if self.counts() == published {
                if let Either::Right(_) =
                    select(pin!(listener), pin!(cancellation.cancelled())).await
                {
                    return;
                }
                continue;
            }
            if let Err(error) = scheduler
                .timer_wait_until(&mut timer, due, cancellation)
                .await
            {
                if !cancellation.is_cancelled() {
                    log::error!("The counts timer failed: {error}");
                }
                return;
            }
            let counts = self.counts();
            if counts != published {
                publish(counts);
                published = counts;
                due = scheduler.now() + self.counts_interval;
            }
        }
    }

    /// Read the cards whenever they are due while someone watches them, until cancelled; see [`crate::cards::Refresh`]. Each read runs on a thread of its own, as reading blocks; one still running when serving ends finishes within its own timeout, and is ignored.
    async fn refresh_cards<S: Timers>(&self, scheduler: &S, cancellation: &Cancellation) {
        let Some(source) = &self.cards else {
            return;
        };
        let mut timer = scheduler.timer();
        while !cancellation.is_cancelled() {
            let listener = self.hub.listen_cards();
            match self.hub.cards_next(scheduler.now()) {
                crate::cards::Next::Now => {
                    if let Some(result) = read_cards(source.clone(), cancellation).await {
                        self.hub.cards_fetched(result, scheduler.now());
                    }
                }
                crate::cards::Next::At(due) => {
                    if let Err(error) = scheduler
                        .timer_wait_until(&mut timer, due, cancellation)
                        .await
                        && !cancellation.is_cancelled()
                    {
                        log::error!("The cards timer failed: {error}");
                        return;
                    }
                }
                crate::cards::Next::Idle => {
                    select(pin!(listener), pin!(cancellation.cancelled())).await;
                }
            }
        }
    }

    /// Send space snapshots that were deferred to keep a streaming reply within [`crate::hub::SNAPSHOT_INTERVAL`], when they are due. Sleeps until a space defers one, and wakes when another space defers one, which may be due sooner.
    async fn pace<S: Timers>(&self, scheduler: &S, cancellation: &Cancellation) {
        let mut timer = scheduler.timer();
        loop {
            let listener = self.hub.listen_snapshots();
            match self.hub.snapshot_deadline() {
                Some(deadline) => {
                    let due = scheduler.timer_wait_until(&mut timer, deadline, cancellation);
                    match select(pin!(due), pin!(listener)).await {
                        Either::Left((Ok(()), _)) => self.hub.flush_snapshots(scheduler.now()),
                        Either::Left((Err(error), _)) => {
                            if !cancellation.is_cancelled() {
                                log::error!("The snapshot timer failed: {error}");
                            }
                            return;
                        }
                        // A later deferral: compute the earliest deadline again.
                        Either::Right(_) => {}
                    }
                }
                None => {
                    if let Either::Right(_) =
                        select(pin!(listener), pin!(cancellation.cancelled())).await
                    {
                        return;
                    }
                }
            }
        }
    }

    /// Accept and serve HTTP connections until cancelled or accepting fails.
    ///
    /// Transient accept errors, such as a peer that reset before it was accepted or running out of descriptors, are logged and accepting continues, pausing for [`ACCEPT_BACKOFF`] when out of resources. Only cancellation or another accept error, such as a closed scheduler or a broken listener, ends serving. Either way, the listener is closed first, so that new peers are refused rather than left queued with nobody accepting, and then the connections are cancelled and given the drain timeout to finish before they are abandoned.
    ///
    /// TODO(upstream): this follows `base_http_server::serve`, which passes every handler one cancellation shared by all connections. The keepalive sweep must end one silent connection, including a response write blocked on a peer that is gone, so each connection here gets its own child cancellation. `serve` also ends on any accept error, and joins its connections without a time limit before closing its listener. Use `serve` again once it offers per-connection cancellation, keeps accepting after transient accept errors, and closes its listener before a bounded drain.
    async fn serve<S>(
        &self,
        scheduler: &S,
        listener: S::Socket,
        cancellation: &Cancellation,
    ) -> Result<Statistics, Error>
    where
        S: SocketAccept + SocketStreamReading + SocketStreamWriting + SocketClose + Timers,
        S::Socket: 'static,
    {
        let server_options = base_http_server::Options::default();
        let options = server_options.http1.clone();
        let acceptor = self.tls.as_ref().map(|config| {
            let mut config = (**config).clone();
            config.alpn_protocols = server_options.protocols.alpn_protocols();
            RustlsAcceptor::new(Arc::new(config))
        });
        let connections_cancellation = cancellation.child();
        let mut connections = FuturesUnordered::new();
        let mut tracker = Tracker::default();
        let mut timer = scheduler.timer();
        // Failed accept attempts since a connection was last accepted, of which `retries` were retried immediately since the last pause, and whether accepting has paused meanwhile.
        let mut failures = 0_u64;
        let mut retries = 0_u32;
        let mut pausing = false;
        let result: Result<(), Error> = async {
            loop {
                if connections.len() == self.max_connections {
                    let finished = {
                        let next = pin!(connections.next());
                        // Cancellation is checked first: it also ends the connections, so one that finishes with it is usually left to the drain to record. One can still finish first, since cancelling marks every signal before it wakes any waiter; the next accept is then cancelled instead.
                        match select(pin!(cancellation.cancelled()), next).await {
                            Either::Left(_) => return Err(Error::Cancelled),
                            Either::Right((finished, _)) => finished,
                        }
                    };
                    tracker.record(finished.expect("connections are waiting"));
                    continue;
                }
                // Keep this accept alive while existing connections complete, so an accepted peer is never discarded.
                let accepted = completing(
                    scheduler.socket_accept(&listener, cancellation),
                    &mut connections,
                    &mut tracker,
                )
                .await;
                let (socket, peer) = match accepted {
                    Ok(accepted) => accepted,
                    Err(OperationError::Cancelled(_)) => return Err(Error::Cancelled),
                    Err(OperationError::Io(error)) => {
                        let pause = match accept_failure(&error) {
                            // Logged only at debug, since any peer on the network can cause one per connection.
                            AcceptFailure::Retry => {
                                log::debug!("Could not accept a connection, so accepting the next: {error}.");
                                retries += 1;
                                retries > ACCEPT_RETRIES
                            }
                            AcceptFailure::Backoff => true,
                            // The caller reports the error.
                            AcceptFailure::Fatal => return Err(Error::Io(error)),
                        };
                        failures += 1;
                        if pause {
                            if !pausing {
                                log::warn!(
                                    "Could not accept a connection: {error}. Trying again every {ACCEPT_BACKOFF:?} while connections finish."
                                );
                                pausing = true;
                            }
                            retries = 0;
                            let deadline = scheduler.now() + ACCEPT_BACKOFF;
                            completing(
                                scheduler.timer_wait_until(&mut timer, deadline, cancellation),
                                &mut connections,
                                &mut tracker,
                            )
                            .await
                            .map_err(serving_error)?;
                        }
                        continue;
                    }
                };
                if pausing {
                    log::info!("Accepting connections again after {failures} failed attempts.");
                    pausing = false;
                }
                failures = 0;
                retries = 0;
                tracker.statistics.connections += 1;
                let serial = tracker.statistics.connections;
                let connection = connections_cancellation.child();
                let options = &options;
                let server_options = &server_options;
                let acceptor = acceptor.as_ref();
                // Each connection is watched so that one which never completes a request is closed, freeing its slot.
                let watch = self.admission.watch(connection.clone());
                tracker.open.insert(
                    serial,
                    Open {
                        peer,
                        watch: watch.clone(),
                    },
                );
                log::debug!("Accepted connection {serial} from {peer}.");
                let handler = Admitted {
                    application: &self.application,
                    watch,
                };
                connections.push(async move {
                    let outcome = match acceptor {
                        None => {
                            base_http_server::serve_connection(
                                scheduler,
                                socket,
                                &handler,
                                options,
                                &connection,
                            )
                            .await
                        }
                        // The handshake is cancelled with the connection, so the admission sweep closes one that stalls.
                        Some(acceptor) => match acceptor.accept(scheduler, socket, &connection).await {
                            Ok(tls) => {
                                base_http_server::serve_tls_connection(
                                    scheduler,
                                    tls,
                                    &handler,
                                    server_options,
                                    &connection,
                                )
                                .await
                            }
                            Err(error) => Err(error.into()),
                        },
                    };
                    (serial, outcome)
                });
            }
        }
        .await;
        // Refuse new peers at once: a listener left open while connections drain queues them with nobody accepting.
        if let Err(error) = scheduler.socket_close(listener).await {
            log::warn!("Closing the listener failed: {error}.");
        }
        connections_cancellation.cancel();
        self.drain(scheduler, &mut connections, &mut tracker).await;
        // Every way the loop ends because of cancellation yields `Error::Cancelled`, so a failure is still reported when shutdown begins during the drain. Failing to close the listener during a shutdown is only logged, above: it no longer accepts either way.
        match result {
            Err(Error::Cancelled) | Ok(()) => Ok(tracker.statistics),
            Err(error) => Err(error),
        }
    }

    /// Wait at most the drain timeout for cancelled connections to finish, then abandon the rest, whose sockets close as they are dropped.
    async fn drain<S, C>(
        &self,
        scheduler: &S,
        connections: &mut FuturesUnordered<C>,
        tracker: &mut Tracker,
    ) where
        S: Timers,
        C: Future<Output = Finished>,
    {
        if connections.is_empty() {
            return;
        }
        log::debug!("Waiting for {} connections to finish.", connections.len());
        // The server's cancellation has usually been cancelled already, and must not end this wait.
        let never = Cancellation::never();
        let mut timer = scheduler.timer();
        let deadline = scheduler.now() + self.drain_timeout;
        let mut expiry = pin!(scheduler.timer_wait_until(&mut timer, deadline, &never));
        while !connections.is_empty() {
            match select(expiry.as_mut(), connections.next()).await {
                Either::Left((result, _)) => {
                    if let Err(error) = result {
                        log::warn!("The connection drain timer failed: {error}.");
                    }
                    log::warn!(
                        "Stopped waiting for {} connections that had not finished {:?} after being cancelled.",
                        connections.len(),
                        self.drain_timeout
                    );
                    for (serial, open) in &tracker.open {
                        log::warn!(
                            "Abandoned connection {serial} from {}, {}.",
                            open.peer,
                            open.watch.state()
                        );
                    }
                    return;
                }
                Either::Right((finished, _)) => {
                    tracker.record(finished.expect("connections are waiting"))
                }
            }
        }
    }

    /// Run the keepalive and request deadline sweeps until cancelled, often enough that a connection is released within a quarter of the shortest interval after its deadline.
    async fn sweep<S: Timers>(&self, scheduler: &S, cancellation: &Cancellation) {
        let period = (self
            .keepalive
            .interval
            .min(self.keepalive.timeout)
            .min(self.request_timeout)
            .min(self.retention)
            / 4)
        .max(Duration::from_millis(1));
        let mut timer = scheduler.timer();
        loop {
            let deadline = scheduler.now() + period;
            match scheduler
                .timer_wait_until(&mut timer, deadline, cancellation)
                .await
            {
                Ok(()) => {
                    let now = scheduler.now();
                    self.admission.sweep(now, self.request_timeout);
                    self.hub.sweep(now, &self.keepalive, self.request_timeout);
                    self.hub.forget_departed(now, self.retention);
                    self.revoke_withdrawn();
                }
                Err(error) => {
                    if !cancellation.is_cancelled() {
                        log::error!("The keepalive timer failed: {error}");
                    }
                    return;
                }
            }
        }
    }

    /// End the connections whose tokens are no longer issued, such as after `tomori-server auth revoke` in another process, and call those still issued by the names kept with them. Every connection is checked each sweep, not only after the store changes, so that one admitted just before a revocation is ended too. See `design/security.md`.
    fn revoke_withdrawn(&self) {
        let Some(store) = self.application.door().store() else {
            return;
        };
        if let Err(error) = store.reload() {
            log::warn!("Could not read the token store: {error}.");
        }
        // A collaborator token outlives no space: one whose space is gone is revoked.
        for entry in store.entries() {
            if let tomori_security::Grant::Collaborator { space } = &entry.grant
                && !self.hub.has_space(space)
            {
                log::info!("Revoking token {}, whose space {space} is gone.", entry.id);
                crate::door::revoke(store, &entry.id);
            }
        }
        self.hub
            .revoke(|principal| store.entry(principal).is_some());
        // A name changed elsewhere, such as by `tomori-server auth rename`, is used from now on.
        self.hub
            .refresh_names(|principal| store.entry(principal).map(|entry| entry.name));
    }

    /// Start requested spaces and drive their sessions until cancelled.
    async fn supervise<S: Timers>(&self, scheduler: &S, cancellation: &Cancellation) {
        let roots = match &self.roots {
            Some(roots) => roots.clone(),
            None => available(WorkspaceRoots::current()),
        };
        for root in roots.roots() {
            log::info!("Spaces may run in {} or beneath it.", root.display());
        }
        let resolving = Arc::new(Resolving::default());
        let mut spaces = FuturesUnordered::new();
        let restores = std::mem::take(&mut *self.restoring.lock().expect("never left poisoned"));
        for restore in restores {
            let places = Places {
                roots: &roots,
                resolving: &resolving,
                slots: &self.slots,
            };
            spaces.push(Box::pin(run_restored(
                scheduler,
                self.hub.clone(),
                restore,
                self.voices.clone(),
                places,
                cancellation,
            )) as Pin<Box<dyn Future<Output = ()>>>);
        }
        let mut cancelled = pin!(cancellation.cancelled());
        loop {
            let job = {
                let job = pin!(self.jobs.recv());
                let finished = pin!(async {
                    if spaces.is_empty() {
                        pending::<()>().await;
                    }
                    spaces.next().await;
                });
                match select(cancelled.as_mut(), select(job, finished)).await {
                    Either::Left(_) => break,
                    Either::Right((Either::Left((job, _)), _)) => job,
                    Either::Right((Either::Right(_), _)) => continue,
                }
            };
            match job {
                Ok(Job::Create(request)) => {
                    let places = Places {
                        roots: &roots,
                        resolving: &resolving,
                        slots: &self.slots,
                    };
                    spaces.push(Box::pin(run_space(
                        scheduler,
                        self.hub.clone(),
                        request,
                        self.voices.clone(),
                        places,
                        cancellation,
                    )) as Pin<Box<dyn Future<Output = ()>>>);
                }
                Ok(Job::ListSessions {
                    participant,
                    backend,
                    directory,
                }) => {
                    let places = Places {
                        roots: &roots,
                        resolving: &resolving,
                        slots: &self.slots,
                    };
                    spaces.push(Box::pin(list_sessions(
                        scheduler,
                        self.hub.clone(),
                        participant,
                        backend,
                        directory,
                        places,
                        cancellation,
                    )) as Pin<Box<dyn Future<Output = ()>>>);
                }
                Ok(Job::ForgetInstructions { backend, session }) => {
                    spaces.push(
                        Box::pin(forget_instructions(self.hub.clone(), backend, session))
                            as Pin<Box<dyn Future<Output = ()>>>,
                    );
                }
                Err(_) => break,
            }
        }
        self.hub.shutdown();
        while spaces.next().await.is_some() {}
    }
}

/// The workspace roots of the current directory, or none if it cannot be read, so that every directory is refused.
fn available(current: Result<WorkspaceRoots, String>) -> WorkspaceRoots {
    current.unwrap_or_else(|error| {
        log::error!("No workspace roots, so every directory is refused: {error}.");
        WorkspaceRoots::none()
    })
}

/// Remove Tomori's instruction entries from `session` of `backend`, a backend this server may no longer enable.
async fn forget_instructions(hub: Arc<Hub>, backend: String, session: String) {
    let Some(found) = hub.backends.get(&backend) else {
        return;
    };
    let keys = crate::instructions::KEYS.map(str::to_owned).to_vec();
    if let Err(error) = found.forget_instructions(&session, keys).await {
        log::warn!(
            "Could not remove Tomori's instructions from {backend} session {session}: {}",
            error.message
        );
    }
}

/// A connection's serial number and how it ended.
type Finished = (u64, Result<u64, Error>);

/// The connections being served, and what has been served.
#[derive(Default)]
struct Tracker {
    statistics: Statistics,
    /// Connections that have not finished, by serial number.
    open: BTreeMap<u64, Open>,
}

/// A connection that has not finished.
struct Open {
    peer: SocketAddress,
    watch: Arc<Watch>,
}

impl Tracker {
    /// Count one finished connection.
    fn record(&mut self, (serial, outcome): Finished) {
        let peer = match self.open.remove(&serial) {
            Some(open) => open.peer.to_string(),
            None => "an unknown peer".to_owned(),
        };
        match outcome {
            Ok(requests) => {
                self.statistics.requests += requests;
                log::debug!("Connection {serial} from {peer} finished after {requests} requests.");
            }
            Err(Error::Cancelled) => log::debug!("Connection {serial} from {peer} was cancelled."),
            Err(error) => {
                // Logged only at debug, since any peer on the network can make its connection fail.
                log::debug!(
                    "Connection {serial} from {peer} failed: {}.",
                    describe(&error)
                );
                self.statistics.failed_connections += 1;
            }
        }
    }
}

/// Wait for `future` while recording connections that finish meanwhile, so that they free their descriptors and slots.
async fn completing<F, C>(
    future: F,
    connections: &mut FuturesUnordered<C>,
    tracker: &mut Tracker,
) -> F::Output
where
    F: Future,
    C: Future<Output = Finished>,
{
    let mut future = pin!(future);
    loop {
        let completed = pin!(async {
            if connections.is_empty() {
                pending().await
            } else {
                connections.next().await.expect("connections are waiting")
            }
        });
        match select(future.as_mut(), completed).await {
            Either::Left((output, _)) => return output,
            Either::Right((finished, _)) => tracker.record(finished),
        }
    }
}

/// Consecutive immediate accept retries before accepting pauses anyway, so that a per-peer error which persists, rather than belonging to one peer, cannot spin.
const ACCEPT_RETRIES: u32 = 16;

/// How serving responds to a failed accept. This matches `base_http_server`'s classification.
#[derive(Debug, PartialEq)]
enum AcceptFailure {
    /// The failure belongs to one pending peer or interrupted the call, such as a peer that reset before it was accepted; the next accept can succeed at once.
    Retry,
    /// The process or system is out of descriptors, buffers or memory, which finishing connections may free; pause before accepting again.
    Backoff,
    /// The listener or the scheduler failed; serving ends.
    Fatal,
}

fn accept_failure(error: &io::Error) -> AcceptFailure {
    use io::ErrorKind::*;
    match error.kind() {
        ConnectionAborted | ConnectionReset | ConnectionRefused | Interrupted | WouldBlock
        | NetworkDown | NetworkUnreachable | HostUnreachable => return AcceptFailure::Retry,
        OutOfMemory => return AcceptFailure::Backoff,
        _ => {}
    }
    match error.raw_os_error() {
        Some(code) if os::RETRY.contains(&code) => AcceptFailure::Retry,
        Some(code) if os::BACKOFF.contains(&code) => AcceptFailure::Backoff,
        _ => AcceptFailure::Fatal,
    }
}

/// Error codes which `io::ErrorKind` leaves uncategorized.
#[cfg(unix)]
mod os {
    /// Linux reports pending network errors of the new connection from `accept`, and documents that they should be retried, and reports a firewall rejection of the pending peer as `EPERM`. `EACCES`, which shares its `io::ErrorKind`, stays fatal: a security module denying every accept would otherwise loop silently. `EOPNOTSUPP` stays fatal too, since it means the listener cannot accept at all.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    pub const RETRY: &[i32] = &[
        libc::EPERM,
        libc::EPROTO,
        libc::ENOPROTOOPT,
        libc::EHOSTDOWN,
        libc::ENONET,
    ];
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    pub const RETRY: &[i32] = &[libc::EPROTO];

    pub const BACKOFF: &[i32] = &[libc::EMFILE, libc::ENFILE, libc::ENOBUFS];
}

/// Winsock codes which `io::ErrorKind` leaves uncategorized.
#[cfg(windows)]
mod os {
    /// `WSAEINTR`.
    pub const RETRY: &[i32] = &[10004];
    /// `WSAEMFILE` and `WSAENOBUFS`.
    pub const BACKOFF: &[i32] = &[10024, 10055];
}

#[cfg(not(any(unix, windows)))]
mod os {
    pub const RETRY: &[i32] = &[];
    pub const BACKOFF: &[i32] = &[];
}

/// A failed wait that ends serving, which the caller reports, or cancellation.
fn serving_error(error: OperationError) -> Error {
    match error {
        OperationError::Cancelled(_) => Error::Cancelled,
        OperationError::Io(error) => Error::Io(error),
    }
}

/// An HTTP server error with its causes, which its own message omits for I/O errors.
fn describe(error: &dyn std::error::Error) -> String {
    let mut description = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        // Some errors repeat their cause in their own message.
        if !description.ends_with(&cause_text) {
            description.push_str(": ");
            description.push_str(&cause_text);
        }
        source = cause.source();
    }
    description
}

/// Where spaces may run, and how many.
struct Places<'a> {
    roots: &'a WorkspaceRoots,
    resolving: &'a Arc<Resolving>,
    slots: &'a Arc<SpaceSlots>,
}

/// Start one space's session, then drive it until it closes or the server shuts down.
async fn run_space<S: Timers>(
    scheduler: &S,
    hub: Arc<Hub>,
    mut request: CreateRequest,
    voices: Voices,
    places: Places<'_>,
    cancellation: &Cancellation,
) {
    let Some(backend) = hub.backends.get(&request.backend) else {
        return;
    };
    // Creating and attaching both run the agent in the directory, so both are confined to the workspace roots. The space shows the canonical directory.
    match resolve(scheduler, &places, &request.directory, cancellation).await {
        Some(directory) => request.directory = directory,
        None => {
            hub.create_failed(&request.creator, DIRECTORY_UNAVAILABLE.to_owned());
            return;
        }
    }
    // Only an accepted directory takes a place, which is held until the session has closed or failed to start. Spaces still starting count, so concurrent requests cannot pass the limit together.
    let Some(slot) = places.slots.acquire() else {
        hub.create_failed(&request.creator, places.slots.full_error());
        return;
    };
    let session = match &request.session {
        Some(id) => backend.attach(id, &request.directory).await,
        None => {
            backend
                .create(SessionSpec {
                    directory: request.directory.clone(),
                    title: request.title.clone(),
                })
                .await
        }
    };
    let session = match session {
        Ok(session) => session,
        Err(error) => {
            hub.create_failed(
                &request.creator,
                format!(
                    "Could not start the {} session: {}",
                    request.backend, error.message
                ),
            );
            return;
        }
    };
    let snapshot = match session.snapshot().await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            // Close the backend handle without deleting an attached session.
            if let Err(error) = session.close().await {
                log::warn!("Closing a session after snapshot failure failed: {error}");
            }
            // Release the slot before reporting the retryable failure.
            drop(slot);
            hub.create_failed(
                &request.creator,
                format!(
                    "Could not read the initial session state: {}. Please retry.",
                    error.message
                ),
            );
            return;
        }
    };
    let (commands, receiver) = async_channel::unbounded();
    let Some(space) = hub.register_space(&request, session.id(), snapshot, commands) else {
        // The server began shutting down while the session started.
        if let Err(error) = session.close().await {
            log::warn!("Closing a session started during shutdown failed: {error}");
        }
        return;
    };
    run_session(&hub, &space, session, receiver, voices, cancellation).await;
    drop(slot);
}

/// Bind a space listed again after a restart to its session, then drive it until it closes or the server shuts down. A space whose session cannot be bound again is listed as closed, with the reason. See `design/spaces.md`.
async fn run_restored<S: Timers>(
    scheduler: &S,
    hub: Arc<Hub>,
    restore: Restore,
    voices: Voices,
    places: Places<'_>,
    cancellation: &Cancellation,
) {
    let Restore {
        id,
        backend,
        directory,
        session,
        commands,
    } = restore;
    let Some(found) = hub.backends.get(&backend) else {
        hub.restore_failed(
            &id,
            format!("The {backend} backend is not enabled on this server."),
            false,
        );
        return;
    };
    let Some(directory) = resolve(scheduler, &places, &directory, cancellation).await else {
        if !cancellation.is_cancelled() {
            hub.restore_failed(&id, DIRECTORY_UNAVAILABLE.to_owned(), false);
        }
        return;
    };
    let Some(slot) = places.slots.acquire() else {
        hub.restore_failed(&id, places.slots.full_error(), false);
        return;
    };
    let session = match found.attach(&session, &directory).await {
        Ok(session) => session,
        Err(error) => {
            // Only the agent saying that the session is gone closes the space for good; it may be unreachable for now.
            let lasting = matches!(
                error.kind,
                tomori_backend::ErrorKind::NotFound | tomori_backend::ErrorKind::Invalid
            );
            hub.restore_failed(
                &id,
                format!("Could not resume the {backend} session: {}", error.message),
                lasting,
            );
            return;
        }
    };
    let snapshot = match session.snapshot().await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            if let Err(error) = session.close().await {
                log::warn!("Closing a session after snapshot failure failed: {error}");
            }
            drop(slot);
            hub.restore_failed(
                &id,
                format!(
                    "Could not read the {backend} session again: {}",
                    error.message
                ),
                false,
            );
            return;
        }
    };
    if !hub.bound(&id, session.id(), snapshot) {
        // The server began shutting down while the session was bound again.
        if let Err(error) = session.close().await {
            log::warn!("Closing a session bound during shutdown failed: {error}");
        }
        return;
    }
    run_session(&hub, &id, session, commands, voices, cancellation).await;
    drop(slot);
}

/// Drive the session of `space`, which is listed, until it closes or the server shuts down, then close it.
async fn run_session(
    hub: &Arc<Hub>,
    space: &str,
    session: AnySession,
    receiver: async_channel::Receiver<SpaceCommand>,
    voices: Voices,
    cancellation: &Cancellation,
) {
    let voice = voices.start(hub, space);
    // A command the session is performing, such as a decision waiting for the agent's connection to return, does not
    // hold up shutdown: the space stops as soon as shutdown begins, and its session is closed below.
    let error = match select(
        pin!(drive(hub, space, &session, receiver, voice)),
        pin!(cancellation.cancelled()),
    )
    .await
    {
        Either::Left((error, _)) => error,
        // The server is stopping, perhaps before the supervisor has said so: say so first, so that the space is kept open.
        Either::Right(_) => {
            hub.stopping();
            None
        }
    };
    hub.space_ended(space, error);
    if let Err(error) = session.close().await {
        log::warn!("Closing the session of {space} failed: {error}");
    }
}

/// List a backend's existing sessions in `directory` for `participant`, resolving the directory first so the backend sees it in canonical form. A directory that is not available, an unknown backend, or a backend failure is reported to `participant` as an error.
async fn list_sessions<S: Timers>(
    scheduler: &S,
    hub: Arc<Hub>,
    participant: String,
    backend: String,
    directory: String,
    places: Places<'_>,
    cancellation: &Cancellation,
) {
    let directory = match resolve(scheduler, &places, &directory, cancellation).await {
        Some(directory) => directory,
        None => {
            hub.sessions(&participant, Err(DIRECTORY_UNAVAILABLE.to_owned()));
            return;
        }
    };
    let Some(found) = hub.backends.get(&backend) else {
        hub.sessions(&participant, Err("Unknown backend.".to_owned()));
        return;
    };
    let sessions = found
        .sessions(&directory)
        .await
        .map_err(|error| error.message);
    hub.sessions(&participant, sessions);
}

/// The canonical form of a client's directory, checked on its own thread for at most [`RESOLVE_TIMEOUT`] so that a slow file system cannot stall the server, or `None` if it is refused.
async fn resolve<S: Timers>(
    scheduler: &S,
    places: &Places<'_>,
    directory: &str,
    cancellation: &Cancellation,
) -> Option<String> {
    let resolved = places
        .roots
        .resolve_in_background(directory, places.resolving)
        .ok()?;
    checked(
        scheduler,
        directory,
        resolved,
        RESOLVE_TIMEOUT,
        cancellation,
    )
    .await
}

/// The canonical form of `directory`, as the check `resolved` finds it within `limit`, or `None` if it is refused, takes longer, or the server is shutting down.
async fn checked<S: Timers>(
    scheduler: &S,
    directory: &str,
    resolved: async_channel::Receiver<Result<String, crate::workspace::Unavailable>>,
    limit: Duration,
    cancellation: &Cancellation,
) -> Option<String> {
    let mut timer = scheduler.timer();
    let deadline = scheduler.now() + limit;
    let timeout = scheduler.timer_wait_until(&mut timer, deadline, cancellation);
    match select(pin!(resolved.recv()), pin!(timeout)).await {
        Either::Left((result, _)) => result
            .expect("the check answers, or its place is freed only as it does")
            .ok(),
        // The wait fails instead when the server is shutting down.
        Either::Right((Ok(_), _)) => {
            log::warn!("Refused directory {directory:?}: checking it took longer than {limit:?}.");
            None
        }
        Either::Right((Err(_), _)) => None,
    }
}

/// Forward session events to the hub and the space's voice while performing the space's commands. Returns why the session stopped, if it failed.
async fn drive(
    hub: &Hub,
    space: &str,
    session: &AnySession,
    commands: async_channel::Receiver<SpaceCommand>,
    mut voice: Option<Voice>,
) -> Option<String> {
    let events = async {
        let mut bound = session.id();
        loop {
            match session.next_event().await {
                Ok(event) => {
                    // An agent may name its session anew once it starts.
                    let id = session.id();
                    if id != bound {
                        hub.session_changed(space, id.clone());
                        bound = id;
                    }
                    let closed = matches!(event, Event::Closed { .. });
                    // Speech segmentation and queueing never wait for synthesis, and happen outside the hub lock.
                    if let Some(voice) = voice.as_mut() {
                        voice.observe(&event);
                    }
                    hub.space_event(space, event);
                    if closed {
                        return None;
                    }
                }
                Err(error) => return Some(error.message),
            }
        }
    };
    let commands = async {
        while let Ok(command) = commands.recv().await {
            match command {
                SpaceCommand::Send(prompt) => {
                    let key = prompt.key.clone();
                    let result = session.send(prompt).await;
                    hub.space_sent(space, &key, result);
                }
                SpaceCommand::Decide {
                    request,
                    decision,
                    participant,
                } => {
                    let result = session.decide(&request, decision).await;
                    hub.space_decided(space, &request, &participant, result);
                }
                SpaceCommand::Answer {
                    question,
                    answers,
                    participant,
                } => {
                    let result = session.answer(&question, answers).await;
                    hub.space_answered(space, &question, &participant, result);
                }
                SpaceCommand::SetMode { mode, participant } => {
                    let result = session.set_mode(&mode).await;
                    hub.space_mode_set(space, &participant, result);
                }
                SpaceCommand::Instructions => {
                    let (entries, all) = hub.instructions_due(space);
                    if entries.is_empty() {
                        continue;
                    }
                    let mut result = Ok(());
                    for (key, value) in entries {
                        result = session.set_instruction(&key, value.as_deref()).await;
                        // The rest would most likely fail alike; they are all given again after the next reconnection.
                        if result.is_err() {
                            break;
                        }
                    }
                    hub.space_instructed(space, result, all);
                }
                SpaceCommand::Close => break,
            }
        }
        None
    };
    match select(pin!(events), pin!(commands)).await {
        Either::Left((error, _)) | Either::Right((error, _)) => error,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Instant;

    use base_scheduler::{
        BufferPool, BufferPoolOptions, BufferPooling, Clock, IoBuf, SocketLocalAddress,
        SocketOperation, SocketStreamReader, SocketStreamWriter,
    };
    use base_scheduler_native::{NativeOptions, NativeScheduler, NativeSocket};
    use futures_lite::future::block_on;

    use super::*;
    use crate::transcriber::NullTranscriber;

    /// A scheduler whose accepts first fail with queued errors, and whose accepted sockets can be made never to finish closing, as a connection that ignores cancellation would.
    struct Faulty {
        inner: NativeScheduler,
        accept_errors: Mutex<VecDeque<io::Error>>,
        accepts: AtomicUsize,
        stall_connection_closes: bool,
        /// Every timer fails, unless it is cancelled.
        failing_timers: bool,
        /// Closing the listener fails, once it has closed.
        failing_listener_close: bool,
        /// Called each time a timer's wait ends, before its result is returned.
        after_wait: Option<Box<dyn Fn() + Send + Sync>>,
    }

    impl Faulty {
        fn new() -> Self {
            Self {
                inner: NativeScheduler::with_options(NativeOptions {
                    workers: 2,
                    threads: 2,
                    capacity: 256,
                    ..Default::default()
                })
                .unwrap(),
                accept_errors: Mutex::new(VecDeque::new()),
                accepts: AtomicUsize::new(0),
                stall_connection_closes: false,
                failing_timers: false,
                failing_listener_close: false,
                after_wait: None,
            }
        }
    }

    /// A socket, and whether it was accepted rather than listening.
    struct Tagged {
        socket: NativeSocket,
        accepted: bool,
    }

    impl SocketOperation for Faulty {
        type Socket = Tagged;
    }

    impl SocketAccept for Faulty {
        async fn socket_accept(
            &self,
            listener: &Tagged,
            cancellation: &Cancellation,
        ) -> OperationResult<(Tagged, SocketAddress)> {
            self.accepts.fetch_add(1, Ordering::Relaxed);
            let error = self.accept_errors.lock().unwrap().pop_front();
            if let Some(error) = error {
                return Err(error.into());
            }
            let (socket, peer) = self
                .inner
                .socket_accept(&listener.socket, cancellation)
                .await?;
            let socket = Tagged {
                socket,
                accepted: true,
            };
            Ok((socket, peer))
        }
    }

    impl SocketClose for Faulty {
        async fn socket_close(&self, socket: Tagged) -> io::Result<()> {
            if socket.accepted && self.stall_connection_closes {
                let _held = socket;
                return pending().await;
            }
            let listener = !socket.accepted;
            let closed = self.inner.socket_close(socket.socket).await;
            if listener && self.failing_listener_close {
                return Err(io::Error::other("the listener did not close cleanly"));
            }
            closed
        }

        async fn socket_close_submit(&self, socket: Tagged) -> io::Result<()> {
            self.inner.socket_close_submit(socket.socket).await
        }
    }

    impl BufferPooling for Faulty {
        type Pool = <NativeScheduler as BufferPooling>::Pool;

        fn buffer_pool(&self, options: BufferPoolOptions) -> io::Result<Self::Pool> {
            self.inner.buffer_pool(options)
        }
    }

    impl SocketStreamReading for Faulty {
        fn socket_stream_reader<'scheduler>(
            &'scheduler self,
            socket: &Tagged,
        ) -> io::Result<
            impl SocketStreamReader<Buffer = <Self::Pool as BufferPool>::Buffer>
            + 'scheduler
            + use<'scheduler>,
        > {
            self.inner.socket_stream_reader(&socket.socket)
        }

        fn socket_stream_reader_with_pool<'scheduler>(
            &'scheduler self,
            socket: &Tagged,
            pool: Self::Pool,
        ) -> io::Result<
            impl SocketStreamReader<Buffer = <Self::Pool as BufferPool>::Buffer>
            + 'scheduler
            + use<'scheduler>,
        > {
            self.inner
                .socket_stream_reader_with_pool(&socket.socket, pool)
        }
    }

    impl SocketStreamWriting for Faulty {
        fn socket_stream_writer<'scheduler>(
            &'scheduler self,
            socket: &Tagged,
        ) -> io::Result<
            impl SocketStreamWriter<Buffer = <Self::Pool as BufferPool>::Buffer>
            + 'scheduler
            + use<'scheduler>,
        > {
            self.inner.socket_stream_writer(&socket.socket)
        }

        fn socket_stream_writer_with_buffer<'scheduler, B: IoBuf>(
            &'scheduler self,
            socket: &Tagged,
        ) -> io::Result<impl SocketStreamWriter<Buffer = B> + 'scheduler + use<'scheduler, B>>
        {
            self.inner
                .socket_stream_writer_with_buffer::<B>(&socket.socket)
        }
    }

    impl Clock for Faulty {
        fn now(&self) -> Instant {
            self.inner.now()
        }

        async fn sleep_until(
            &self,
            deadline: Instant,
            cancellation: &Cancellation,
        ) -> OperationResult<()> {
            self.inner.sleep_until(deadline, cancellation).await
        }
    }

    impl Timers for Faulty {
        type Timer = <NativeScheduler as Timers>::Timer;

        fn timer(&self) -> Self::Timer {
            self.inner.timer()
        }

        async fn timer_wait_until(
            &self,
            timer: &mut Self::Timer,
            deadline: Instant,
            cancellation: &Cancellation,
        ) -> OperationResult<()> {
            if self.failing_timers && !cancellation.is_cancelled() {
                return Err(io::Error::other("the timer is broken").into());
            }
            let result = self
                .inner
                .timer_wait_until(timer, deadline, cancellation)
                .await;
            if let Some(after_wait) = &self.after_wait {
                after_wait();
            }
            result
        }
    }

    #[test]
    fn counts_that_change_back_before_they_are_due_are_not_published() {
        let published = Arc::new(Mutex::new(Vec::new()));
        let server = {
            let published = published.clone();
            server()
                .with_counts(move |counts| published.lock().unwrap().push(counts))
                .with_counts_interval(Duration::ZERO)
        };
        let listed = server.counts();
        // One more space than is listed.
        let more = Counts {
            spaces: listed.spaces + 1,
            ..listed
        };
        // Serving stops as the wait for each change to be due ends; `listen` is called first.
        let stopping_after = |listen: Box<dyn Fn() + Send + Sync>| {
            let live = Cancellation::new();
            let scheduler = Faulty {
                after_wait: Some(Box::new({
                    let live = live.clone();
                    move || {
                        listen();
                        live.cancel();
                    }
                })),
                ..Faulty::new()
            };
            (scheduler, live)
        };
        // Last published with a space more than is listed, the counts as listed are published once due.
        let (scheduler, live) = stopping_after(Box::new(|| {}));
        block_on(server.publish(&scheduler, more, &live));
        assert_eq!(*published.lock().unwrap(), [listed]);
        // But if a space is listed by then, as there was when they were last published, nothing is.
        let hub = server.hub.clone();
        let (scheduler, live) = stopping_after(Box::new(move || {
            let (commands, _) = async_channel::unbounded();
            let request = crate::hub::CreateRequest {
                title: "Space".to_owned(),
                backend: "fake".to_owned(),
                directory: "/tmp".to_owned(),
                session: None,
                creator: "nobody".to_owned(),
                creator_space: None,
                environment: "sky".to_owned(),
                instructions: String::new(),
            };
            hub.register_space(&request, "session".to_owned(), Default::default(), commands)
                .unwrap();
        }));
        block_on(server.publish(&scheduler, more, &live));
        assert_eq!(server.counts(), more);
        assert_eq!(*published.lock().unwrap(), [listed]);
    }

    /// A server running on its own thread with a [`Faulty`] scheduler.
    struct Running {
        address: SocketAddr,
        cancellation: Cancellation,
        /// Receives what `run` returned, and how many accepts were attempted.
        finished: Receiver<(Result<Statistics, Error>, usize)>,
    }

    fn start(
        accept_errors: Vec<io::Error>,
        stall_connection_closes: bool,
        drain_timeout: Duration,
    ) -> Running {
        start_with(
            Faulty {
                accept_errors: Mutex::new(accept_errors.into()),
                stall_connection_closes,
                ..Faulty::new()
            },
            move |server| server.drain_timeout = drain_timeout,
        )
    }

    /// Serve with `scheduler`, configured by `configure`.
    fn start_with(
        scheduler: Faulty,
        configure: impl FnOnce(&mut Server) + Send + 'static,
    ) -> Running {
        let cancellation = Cancellation::new();
        let (address_sender, address) = channel();
        let (sender, finished) = channel();
        let running = cancellation.clone();
        std::thread::spawn(move || {
            let mut server = Server::new(Backends::new(), Arc::new(NullTranscriber::new()))
                .with_workspace_roots(WorkspaceRoots::none());
            configure(&mut server);
            let result = block_on(async {
                let address = "127.0.0.1:0".parse().unwrap();
                let listener = bind(&scheduler.inner, address, &running).await.unwrap();
                let address = scheduler.inner.socket_local_address(&listener).unwrap();
                address_sender.send(address).unwrap();
                let listener = Tagged {
                    socket: listener,
                    accepted: false,
                };
                server.run(&scheduler, listener, &running).await
            });
            let accepts = scheduler.accepts.load(Ordering::Relaxed);
            block_on(scheduler.inner.shutdown());
            let _ = sender.send((result, accepts));
        });
        Running {
            address: address.recv().unwrap(),
            cancellation,
            finished,
        }
    }

    /// Request `/health` and return the response, reading only as far as the end of its body, so that a socket the server never closes does not matter.
    fn health(address: SocketAddr) -> String {
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(
            stream,
            "GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = Vec::new();
        let mut buffer = [0; 1024];
        while !response.ends_with(b"}\n") {
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0, "the response ended early");
            response.extend_from_slice(&buffer[..count]);
        }
        String::from_utf8(response).unwrap()
    }

    fn server() -> Server {
        Server::new(Backends::new(), Arc::new(NullTranscriber::new()))
            .with_workspace_roots(WorkspaceRoots::none())
    }

    // Small builders called only from other crates are inlined there, where their coverage is not counted as theirs, so each is also called in this crate's tests.
    #[test]
    fn the_participant_retention_given_is_kept() {
        let retention = Duration::from_secs(7);
        assert_eq!(
            server().with_participant_retention(retention).retention,
            retention
        );
    }

    #[test]
    fn the_test_scheduler_passes_through_what_it_does_not_break() {
        let scheduler = Faulty::new();
        let never = Cancellation::never();
        let options = BufferPoolOptions {
            buffers: 1,
            buffer_size: 64,
            alignment: 8,
        };
        let pool = scheduler.buffer_pool(options).unwrap();
        block_on(async {
            scheduler
                .sleep_until(scheduler.now(), &never)
                .await
                .unwrap();
            let address = "127.0.0.1:0".parse().unwrap();
            let socket = Tagged {
                socket: bind(&scheduler.inner, address, &never).await.unwrap(),
                accepted: false,
            };
            assert!(
                scheduler
                    .socket_stream_reader_with_pool(&socket, pool)
                    .is_ok()
            );
            assert!(
                scheduler
                    .socket_stream_writer_with_buffer::<Vec<u8>>(&socket)
                    .is_ok()
            );
            scheduler.socket_close_submit(socket).await.unwrap();
        });
        block_on(scheduler.inner.shutdown());
    }

    /// Cards read from a script, last first, after `delay`, panicking once it is empty.
    struct Scripted {
        delay: Duration,
        results: Mutex<Vec<Result<crate::cards::Fetched, crate::cards::Failure>>>,
    }

    impl crate::cards::CardSource for Scripted {
        fn source(&self) -> String {
            "o/r".to_owned()
        }

        fn fetch(&self) -> Result<crate::cards::Fetched, crate::cards::Failure> {
            std::thread::sleep(self.delay);
            self.results
                .lock()
                .unwrap()
                .pop()
                .expect("the script has another read")
        }
    }

    fn scripted(
        delay: Duration,
        results: Vec<Result<crate::cards::Fetched, crate::cards::Failure>>,
    ) -> Arc<dyn crate::cards::CardSource> {
        Arc::new(Scripted {
            delay,
            results: Mutex::new(results),
        })
    }

    #[test]
    fn cards_are_read_on_a_thread_of_their_own_and_a_lost_read_is_a_failure() {
        use crate::cards::{Failure, Fetched};
        let live = Cancellation::new();
        let read = |source, cancellation: &Cancellation| block_on(read_cards(source, cancellation));
        assert_eq!(
            read(
                scripted(Duration::ZERO, vec![Ok(Fetched::default())]),
                &live
            ),
            Some(Ok(Fetched::default()))
        );
        // A read that panics answers nothing.
        assert_eq!(
            read(scripted(Duration::ZERO, Vec::new()), &live),
            Some(Err(Failure::new("reading ended without an answer")))
        );
        crate::system::refuse("tomori-cards");
        let refused = read(scripted(Duration::ZERO, Vec::new()), &live)
            .unwrap()
            .unwrap_err();
        assert!(
            refused.message.starts_with("could not start reading: "),
            "{refused:?}"
        );
        // Serving ends while it reads: its answer is ignored.
        let cancelled = Cancellation::new();
        cancelled.cancel();
        let slow = scripted(Duration::from_millis(200), vec![Ok(Fetched::default())]);
        assert_eq!(read(slow, &cancelled), None);
    }

    #[test]
    fn cards_wait_until_due_or_watched_and_a_failing_timer_ends_their_reading() {
        let scheduler = Faulty {
            failing_timers: true,
            ..Faulty::new()
        };
        let live = Cancellation::new();
        // A server without cards reads none.
        block_on(server().refresh_cards(&scheduler, &live));
        let server = server().with_cards("github", "GitHub", scripted(Duration::ZERO, Vec::new()));
        assert!(
            server
                .environments()
                .iter()
                .any(|environment| environment.id == "github" && environment.label == "GitHub")
        );
        // Nobody watches, so it waits, until serving ends.
        let later = Cancellation::new();
        block_on(zip(server.refresh_cards(&scheduler, &later), async {
            later.cancel()
        }));
        // Read just now, it waits until they are due, ending as the timer fails.
        server
            .hub
            .cards_fetched(Ok(crate::cards::Fetched::default()), Instant::now());
        block_on(server.refresh_cards(&scheduler, &live));
        // A timer cancelled as serving ends is no failure.
        let later = Cancellation::new();
        block_on(zip(server.refresh_cards(&Faulty::new(), &later), async {
            later.cancel()
        }));
    }

    #[test]
    fn timers_that_fail_end_the_sweep_the_pacing_the_counts_and_the_drain() {
        log::set_max_level(log::LevelFilter::Trace);
        let scheduler = Faulty {
            failing_timers: true,
            ..Faulty::new()
        };
        let live = Cancellation::new();
        let cancelled = Cancellation::new();
        cancelled.cancel();
        // Told nothing, as its timer fails first.
        let server = server().with_counts(drop::<Counts>);
        block_on(server.sweep(&scheduler, &live));
        block_on(server.sweep(&scheduler, &cancelled));
        // A deferred snapshot is waited for.
        let (commands, _receiver) = async_channel::unbounded();
        let request = crate::hub::CreateRequest {
            title: "Space".to_owned(),
            backend: "fake".to_owned(),
            directory: "/tmp".to_owned(),
            session: None,
            creator: "nobody".to_owned(),
            creator_space: None,
            environment: "sky".to_owned(),
            instructions: String::new(),
        };
        let id = server
            .hub
            .register_space(&request, "session".to_owned(), Default::default(), commands)
            .unwrap();
        // The second event within the interval defers its snapshot.
        server.hub.space_event(&id, tomori_backend::Event::Busy);
        server.hub.space_event(&id, tomori_backend::Event::Idle);
        assert!(server.hub.snapshot_deadline().is_some());
        block_on(server.pace(&scheduler, &live));
        block_on(server.pace(&scheduler, &cancelled));
        // Counts that changed are waited for.
        let changed = Counts {
            spaces: 99,
            ..server.counts()
        };
        block_on(server.publish(&scheduler, changed, &live));
        block_on(server.publish(&scheduler, changed, &cancelled));
        // Connections still open are abandoned, and said with what they were doing.
        let mut connections = FuturesUnordered::new();
        connections.push(pending::<Finished>());
        let mut tracker = Tracker::default();
        tracker.open.insert(
            1,
            Open {
                peer: "127.0.0.1:1".parse().unwrap(),
                watch: server.admission.watch(Cancellation::new()),
            },
        );
        block_on(server.drain(&scheduler, &mut connections, &mut tracker));
        assert_eq!(connections.len(), 1);
        block_on(scheduler.inner.shutdown());
    }

    #[test]
    fn a_listener_that_does_not_close_cleanly_is_only_said() {
        let running = start_with(
            Faulty {
                failing_listener_close: true,
                ..Faulty::new()
            },
            |_| {},
        );
        assert!(health(running.address).starts_with("HTTP/1.1 200"));
        running.cancellation.cancel();
        let (result, _) = running
            .finished
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert_eq!(result.unwrap().connections, 1);
    }

    #[test]
    fn a_pause_whose_timer_fails_ends_the_server() {
        let running = start_with(
            Faulty {
                accept_errors: Mutex::new([io::Error::from(io::ErrorKind::OutOfMemory)].into()),
                failing_timers: true,
                ..Faulty::new()
            },
            |_| {},
        );
        let (result, _) = running
            .finished
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        let Err(Error::Io(_)) = result else {
            unreachable!("serving fails with the timer's error, not {result:?}");
        };
    }

    #[test]
    fn a_server_cancelled_while_pausing_or_full_stops_cleanly() {
        // Cancelled while accepting is paused.
        let running = start_with(
            Faulty {
                accept_errors: Mutex::new([io::Error::from(io::ErrorKind::OutOfMemory)].into()),
                ..Faulty::new()
            },
            |server| server.drain_timeout = Duration::ZERO,
        );
        running.cancellation.cancel();
        let (result, _) = running
            .finished
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert_eq!(result.unwrap().connections, 0);
        // A response kept alive shows that the connection was accepted and stays open.
        let healthy = |connection: &mut TcpStream| {
            write!(
                connection,
                "GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n"
            )
            .unwrap();
            let mut response = Vec::new();
            let mut buffer = [0; 1024];
            while !response.ends_with(b"}\n") {
                let count = connection.read(&mut buffer).unwrap();
                response.extend_from_slice(&buffer[..count]);
            }
        };
        // Cancelled while as many connections as it serves are open. The connection is served only while the server waits at the limit, and it can never finish, since its socket never finishes closing, so only the cancellation can end that wait. A connection that could finish might do so first: cancelling marks every signal before it wakes any waiter, so the connection can see its own cancellation and end before the wait is woken.
        let running = start_with(
            Faulty {
                stall_connection_closes: true,
                ..Faulty::new()
            },
            |server| {
                server.max_connections = 1;
                server.drain_timeout = Duration::ZERO;
            },
        );
        let mut idle = TcpStream::connect(running.address).unwrap();
        healthy(&mut idle);
        running.cancellation.cancel();
        let (result, _) = running
            .finished
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert_eq!(result.unwrap().connections, 1);
        drop(idle);
        // At the limit, another is served only once the open connection closes.
        let running = start_with(Faulty::new(), |server| {
            server.max_connections = 1;
            server.drain_timeout = Duration::ZERO;
        });
        let mut idle = TcpStream::connect(running.address).unwrap();
        healthy(&mut idle);
        let mut next = TcpStream::connect(running.address).unwrap();
        drop(idle);
        healthy(&mut next);
        // Whether the cancellation ends the wait at the limit or the next accept, which depends on timing, both are covered elsewhere.
        running.cancellation.cancel();
        let (result, _) = running
            .finished
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert_eq!(result.unwrap().connections, 2);
        drop(next);
    }

    /// A scheduler that refuses the `step`th of opening, configuring, binding and listening, and does nothing else.
    struct Refusing {
        step: usize,
    }

    impl Refusing {
        fn at(&self, step: usize) -> OperationResult<()> {
            if self.step == step {
                Err(io::Error::other("refused").into())
            } else {
                Ok(())
            }
        }
    }

    impl SocketOperation for Refusing {
        type Socket = ();
    }

    impl SocketOpen for Refusing {
        async fn socket_open(
            &self,
            _: base_scheduler::SocketDomain,
            _: SocketType,
            _: Option<base_scheduler::SocketProtocol>,
            _: &Cancellation,
        ) -> OperationResult<()> {
            self.at(0)
        }
    }

    impl SocketSetOption<SocketReuseAddress> for Refusing {
        async fn socket_set_option(
            &self,
            _: &(),
            _: SocketReuseAddress,
            _: bool,
            _: &Cancellation,
        ) -> OperationResult<()> {
            self.at(1)
        }
    }

    impl SocketBind for Refusing {
        async fn socket_bind(
            &self,
            _: &(),
            _: SocketAddress,
            _: &Cancellation,
        ) -> OperationResult<()> {
            self.at(2)
        }
    }

    impl SocketListen for Refusing {
        async fn socket_listen(&self, _: &(), _: i32, _: &Cancellation) -> OperationResult<()> {
            self.at(3)
        }
    }

    #[test]
    fn binding_fails_at_whichever_step_is_refused() {
        let address = "127.0.0.1:0".parse().unwrap();
        let never = Cancellation::never();
        for step in 0..4 {
            assert!(block_on(bind(&Refusing { step }, address, &never)).is_err());
        }
        assert!(block_on(bind(&Refusing { step: 4 }, address, &never)).is_ok());
    }

    /// What a space's driver works with: a hub with `backend`, room for `slots` spaces in `/tmp`, and a scheduler.
    struct Supervision {
        backend: crate::fake::FakeBackend,
        hub: Arc<Hub>,
        roots: WorkspaceRoots,
        resolving: Arc<Resolving>,
        slots: Arc<SpaceSlots>,
        scheduler: Faulty,
    }

    impl Supervision {
        fn new(slots: usize) -> Self {
            let backend = crate::fake::FakeBackend::new();
            let server = Server::new(
                Backends::new().with(backend.clone()),
                Arc::new(NullTranscriber::new()),
            );
            Self {
                backend,
                hub: server.hub.clone(),
                roots: WorkspaceRoots::new(["/tmp"]).unwrap(),
                resolving: Arc::default(),
                slots: SpaceSlots::new(slots),
                scheduler: Faulty::new(),
            }
        }

        fn places(&self) -> Places<'_> {
            Places {
                roots: &self.roots,
                resolving: &self.resolving,
                slots: &self.slots,
            }
        }

        fn request(&self, backend: &str) -> crate::hub::CreateRequest {
            crate::hub::CreateRequest {
                title: "Space".to_owned(),
                backend: backend.to_owned(),
                directory: "/tmp".to_owned(),
                session: None,
                creator: "nobody".to_owned(),
                creator_space: None,
                environment: "sky".to_owned(),
                instructions: String::new(),
            }
        }

        fn create(&self, request: crate::hub::CreateRequest, cancellation: &Cancellation) {
            block_on(run_space(
                &self.scheduler,
                self.hub.clone(),
                request,
                Voices::default(),
                self.places(),
                cancellation,
            ));
        }

        /// A space kept from before a restart, bound to `session` of `backend` in `directory`, listed again.
        fn kept(
            &self,
            name: &str,
            backend: &str,
            directory: &str,
            session: &str,
        ) -> crate::hub::Restore {
            let state = std::env::temp_dir()
                .join(format!("tomori-server-kept-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&state);
            let store = crate::kept::SpaceStore::open(&state).unwrap();
            store
                .keep(|| {
                    Some(vec![crate::kept::KeptSpace {
                        id: "space-kept".to_owned(),
                        title: "Kept".to_owned(),
                        backend: backend.to_owned(),
                        directory: directory.to_owned(),
                        session: session.to_owned(),
                        environment: "sky".to_owned(),
                        attribution: true,
                        brief: false,
                        instructions: String::new(),
                        closed: None,
                        closed_order: None,
                        placements: Default::default(),
                    }])
                })
                .unwrap();
            let restore = self.hub.restore(Arc::new(store)).pop().unwrap();
            let _ = std::fs::remove_dir_all(&state);
            restore
        }

        fn restore(&self, restore: crate::hub::Restore, cancellation: &Cancellation) {
            block_on(run_restored(
                &self.scheduler,
                self.hub.clone(),
                restore,
                Voices::default(),
                self.places(),
                cancellation,
            ));
        }

        /// The kept space's state.
        fn kept_state(&self) -> tomori_protocol::SpaceState {
            self.hub.space_state("space-kept")
        }

        /// A session of the fake backend that something else is bound to.
        fn held(&self) -> String {
            let session = block_on(self.backend.create(SessionSpec {
                directory: "/tmp".to_owned(),
                title: "Held".to_owned(),
            }))
            .unwrap();
            tomori_backend::Session::id(&session)
        }

        /// A session of the fake backend that nothing is bound to, as one left by a server that stopped.
        fn left(&self) -> String {
            let session = block_on(self.backend.create(SessionSpec {
                directory: "/tmp".to_owned(),
                title: "Left".to_owned(),
            }))
            .unwrap();
            block_on(tomori_backend::Session::close(&session)).unwrap();
            tomori_backend::Session::id(&session)
        }
    }

    #[test]
    fn a_space_that_cannot_start_releases_its_place_and_closes_what_it_started() {
        let supervision = Supervision::new(1);
        let live = Cancellation::new();
        // A backend that is not enabled starts nothing.
        supervision.create(supervision.request("gone"), &live);
        // A session whose state cannot be read is closed, even if closing fails.
        supervision.backend.fail_snapshots(true);
        supervision.backend.fail_closing();
        supervision.create(supervision.request("fake"), &live);
        assert!(supervision.backend.started()[0].log().closed);
        supervision.backend.fail_snapshots(false);
        // A session started as the server stops is closed.
        supervision.hub.stopping();
        supervision.create(supervision.request("fake"), &live);
        assert!(supervision.backend.started()[1].log().closed);
        assert!(supervision.hub.status().spaces.is_empty());
        assert!(supervision.slots.acquire().is_some());
        // Closing such a session may succeed too.
        let supervision = Supervision::new(1);
        supervision.hub.stopping();
        supervision.create(supervision.request("fake"), &live);
        assert!(supervision.backend.started()[0].log().closed);
    }

    #[test]
    fn a_space_whose_place_was_taken_after_the_hub_let_it_through_starts_nothing() {
        // The hub refuses a create once every place is taken, but creates made at once may all pass it before any takes a place.
        let supervision = Supervision::new(0);
        supervision.create(supervision.request("fake"), &Cancellation::new());
        assert!(supervision.backend.started().is_empty());
        assert!(supervision.hub.status().spaces.is_empty());
    }

    #[test]
    fn a_kept_space_that_cannot_be_bound_again_says_why() {
        let reason = |supervision: &Supervision| supervision.kept_state().error;
        let live = Cancellation::new();
        let cancelled = Cancellation::new();
        cancelled.cancel();
        // A directory outside the roots, unless the server is stopping.
        let supervision = Supervision::new(1);
        let restore = supervision.kept("outside", "fake", "/nonexistent", "fake-1");
        supervision.restore(restore, &live);
        assert_eq!(reason(&supervision).as_deref(), Some(DIRECTORY_UNAVAILABLE));
        let supervision = Supervision::new(1);
        let restore = supervision.kept("stopping", "fake", "/nonexistent", "fake-1");
        supervision.restore(restore, &cancelled);
        assert_ne!(reason(&supervision).as_deref(), Some(DIRECTORY_UNAVAILABLE));
        // No room for it.
        let supervision = Supervision::new(0);
        let restore = supervision.kept("full", "fake", "/tmp", "fake-1");
        supervision.restore(restore, &live);
        assert_eq!(reason(&supervision), Some(supervision.slots.full_error()));
        // A session the agent no longer has.
        let supervision = Supervision::new(1);
        let restore = supervision.kept("missing", "fake", "/tmp", "fake-99");
        supervision.restore(restore, &live);
        assert!(
            reason(&supervision)
                .unwrap()
                .starts_with("Could not resume the fake session")
        );
        // A session another binding holds, which may be let go.
        let supervision = Supervision::new(1);
        let restore = supervision.kept("held", "fake", "/tmp", &supervision.held());
        supervision.restore(restore, &live);
        assert!(
            reason(&supervision)
                .unwrap()
                .starts_with("Could not resume the fake session")
        );
        assert_eq!(
            supervision.kept_state().activity,
            tomori_protocol::Activity::Closed
        );
        // A session whose state cannot be read, closed even if closing fails.
        let supervision = Supervision::new(1);
        let session = supervision.left();
        supervision.backend.fail_snapshots(true);
        supervision.backend.fail_closing();
        let restore = supervision.kept("unread", "fake", "/tmp", &session);
        supervision.restore(restore, &live);
        assert!(
            reason(&supervision)
                .unwrap()
                .starts_with("Could not read the fake session again")
        );
        // A session bound as the server stops is closed, whether or not closing fails.
        let supervision = Supervision::new(1);
        let session = supervision.left();
        let restore = supervision.kept("closed", "fake", "/tmp", &session);
        supervision.hub.stopping();
        supervision.restore(restore, &live);
        assert!(supervision.backend.started()[0].log().closed);
        let supervision = Supervision::new(1);
        let session = supervision.left();
        supervision.backend.fail_closing();
        let restore = supervision.kept("late", "fake", "/tmp", &session);
        supervision.hub.stopping();
        supervision.restore(restore, &live);
        assert!(supervision.backend.started()[0].log().closed);
    }

    #[test]
    fn a_kept_space_bound_again_is_driven_until_its_session_closes() {
        let supervision = Supervision::new(1);
        let session = supervision.left();
        let restore = supervision.kept("bound", "fake", "/tmp", &session);
        let hub = supervision.hub.clone();
        let backend = supervision.backend.clone();
        let closing = std::thread::spawn(move || {
            crate::test_support::eventually(|| {
                (hub.space_state("space-kept").activity != tomori_protocol::Activity::Unknown)
                    .then_some(())
            });
            backend.started()[0].emit(Event::Closed { reason: None });
        });
        supervision.restore(restore, &Cancellation::new());
        closing.join().unwrap();
        assert_eq!(
            supervision.kept_state().activity,
            tomori_protocol::Activity::Closed
        );
        assert!(supervision.slots.acquire().is_some());
    }

    #[test]
    fn a_session_driven_until_the_server_stops_is_closed_even_if_closing_fails() {
        let supervision = Supervision::new(1);
        supervision.backend.fail_closing();
        let backend = supervision.hub.backends.get("fake").unwrap();
        let session = block_on(backend.create(SessionSpec {
            directory: "/tmp".to_owned(),
            title: "Driven".to_owned(),
        }))
        .unwrap();
        let (commands, receiver) = async_channel::unbounded();
        let space = supervision
            .hub
            .register_space(
                &supervision.request("fake"),
                session.id(),
                tomori_backend::Snapshot::default(),
                commands,
            )
            .unwrap();
        let cancelled = Cancellation::new();
        cancelled.cancel();
        block_on(run_session(
            &supervision.hub,
            &space,
            session,
            receiver,
            Voices::default(),
            &cancelled,
        ));
        assert!(supervision.backend.started()[0].log().closed);
    }

    #[test]
    fn sessions_are_listed_only_for_a_directory_and_backend_it_has() {
        let supervision = Supervision::new(1);
        let live = Cancellation::new();
        for (backend, directory) in [("fake", "/nonexistent"), ("gone", "/tmp")] {
            block_on(list_sessions(
                &supervision.scheduler,
                supervision.hub.clone(),
                "/ada".to_owned(),
                backend.to_owned(),
                directory.to_owned(),
                supervision.places(),
                &live,
            ));
        }
    }

    #[test]
    fn a_check_that_takes_too_long_or_outlasts_the_server_refuses_the_directory() {
        let scheduler = Faulty::new();
        let (_sender, resolved) = async_channel::bounded(1);
        let live = Cancellation::new();
        assert_eq!(
            block_on(checked(
                &scheduler,
                "/slow",
                resolved.clone(),
                Duration::ZERO,
                &live
            )),
            None
        );
        let cancelled = Cancellation::new();
        cancelled.cancel();
        assert_eq!(
            block_on(checked(
                &scheduler,
                "/slow",
                resolved,
                Duration::from_secs(60),
                &cancelled
            )),
            None
        );
        block_on(scheduler.inner.shutdown());
    }

    #[test]
    fn a_driven_session_follows_its_new_name_its_failure_and_instructions_with_nothing_to_give() {
        let supervision = Supervision::new(1);
        let backend = supervision.hub.backends.get("fake").unwrap();
        let start = || {
            let session = block_on(backend.create(SessionSpec {
                directory: "/tmp".to_owned(),
                title: "Driven".to_owned(),
            }))
            .unwrap();
            let (commands, receiver) = async_channel::unbounded();
            let space = supervision
                .hub
                .register_space(
                    &supervision.request("fake"),
                    session.id(),
                    tomori_backend::Snapshot::default(),
                    commands.clone(),
                )
                .unwrap();
            let fake = supervision.backend.started().pop().unwrap();
            (session, space, fake, commands, receiver)
        };
        // Named anew once it has started, then closed.
        let (session, space, fake, _commands, receiver) = start();
        fake.emit(Event::Busy);
        let hub = supervision.hub.clone();
        let renaming = std::thread::spawn(move || {
            crate::test_support::eventually(|| {
                (hub.status().spaces[0].activity == tomori_protocol::Activity::Running)
                    .then_some(())
            });
            fake.rename("renamed");
            fake.emit(Event::Idle);
            fake.emit(Event::Closed { reason: None });
        });
        assert_eq!(
            block_on(drive(&supervision.hub, &space, &session, receiver, None)),
            None
        );
        renaming.join().unwrap();
        assert_eq!(supervision.hub.space_session(&space), "renamed");
        // Its events end in a failure.
        let (session, space, _fake, _commands, receiver) = start();
        block_on(session.close()).unwrap();
        assert_eq!(
            block_on(session.next_event()).unwrap(),
            Event::Closed { reason: None }
        );
        assert!(block_on(drive(&supervision.hub, &space, &session, receiver, None)).is_some());
        // Instructions are skipped once the space has closed, when none are due.
        let (session, space, _fake, commands, receiver) = start();
        supervision.hub.space_ended(&space, None);
        commands.try_send(SpaceCommand::Instructions).unwrap();
        commands.try_send(SpaceCommand::Close).unwrap();
        assert_eq!(
            block_on(drive(&supervision.hub, &space, &session, receiver, None)),
            None
        );
    }

    #[test]
    fn the_supervisor_stops_once_the_hub_shuts_down_and_forgets_only_for_backends_it_has() {
        let supervision = Supervision::new(1);
        block_on(forget_instructions(
            supervision.hub.clone(),
            "gone".to_owned(),
            "fake-1".to_owned(),
        ));
        block_on(forget_instructions(
            supervision.hub.clone(),
            "fake".to_owned(),
            "fake-1".to_owned(),
        ));
        supervision.backend.take_no_instructions();
        block_on(forget_instructions(
            supervision.hub.clone(),
            "fake".to_owned(),
            "fake-1".to_owned(),
        ));
        // Without roots configured, spaces run in the current directory.
        let server = Server::new(Backends::new(), Arc::new(NullTranscriber::new()));
        server.hub.shutdown();
        block_on(server.supervise(&supervision.scheduler, &Cancellation::new()));
        // Roots that cannot be read refuse every directory.
        assert!(available(Err("gone".to_owned())).roots().is_empty());
        // A connection unknown to the tracker is counted.
        let mut tracker = Tracker::default();
        tracker.record((7, Ok(1)));
        assert_eq!(tracker.statistics.requests, 1);
    }

    #[test]
    fn a_token_store_that_cannot_be_read_again_keeps_what_it_read() {
        let state =
            std::env::temp_dir().join(format!("tomori-server-reload-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&state);
        let store = Arc::new(tomori_security::TokenStore::open(&state).unwrap());
        store.issue("Ada", tomori_security::Grant::Owner).unwrap();
        let server = server().with_tokens(store.clone(), false);
        std::fs::write(state.join("tokens.json"), "not json").unwrap();
        server.revoke_withdrawn();
        assert_eq!(store.entries().len(), 1);
        let _ = std::fs::remove_dir_all(&state);
    }

    #[test]
    fn accept_failures_are_classified_by_their_effect_on_the_listener() {
        use io::ErrorKind::*;
        for kind in [
            ConnectionAborted,
            ConnectionReset,
            ConnectionRefused,
            Interrupted,
            WouldBlock,
            NetworkDown,
            NetworkUnreachable,
            HostUnreachable,
        ] {
            assert_eq!(
                accept_failure(&io::Error::from(kind)),
                AcceptFailure::Retry,
                "{kind:?}"
            );
        }
        assert_eq!(
            accept_failure(&io::Error::from(OutOfMemory)),
            AcceptFailure::Backoff
        );
        for kind in [InvalidInput, BrokenPipe, PermissionDenied, Other] {
            assert_eq!(
                accept_failure(&io::Error::from(kind)),
                AcceptFailure::Fatal,
                "{kind:?}"
            );
        }
        // A closed scheduler ends serving.
        let closed = io::Error::new(BrokenPipe, "scheduler is closed");
        assert_eq!(accept_failure(&closed), AcceptFailure::Fatal);
        #[cfg(unix)]
        {
            let classify = |code| accept_failure(&io::Error::from_raw_os_error(code));
            for code in [
                libc::ECONNABORTED,
                libc::ECONNRESET,
                libc::EINTR,
                libc::EAGAIN,
                libc::EPROTO,
                libc::ENETDOWN,
                libc::ENETUNREACH,
                libc::EHOSTUNREACH,
            ] {
                assert_eq!(classify(code), AcceptFailure::Retry, "{code}");
            }
            // Pending network errors of the new connection, and a firewall rejecting the peer, which Linux reports from accept.
            #[cfg(any(target_os = "linux", target_os = "android"))]
            for code in [
                libc::EPERM,
                libc::ENOPROTOOPT,
                libc::EHOSTDOWN,
                libc::ENONET,
            ] {
                assert_eq!(classify(code), AcceptFailure::Retry, "{code}");
            }
            for code in [libc::EMFILE, libc::ENFILE, libc::ENOBUFS, libc::ENOMEM] {
                assert_eq!(classify(code), AcceptFailure::Backoff, "{code}");
            }
            for code in [
                libc::EBADF,
                libc::EINVAL,
                libc::ENOTSOCK,
                libc::EOPNOTSUPP,
                libc::EACCES,
            ] {
                assert_eq!(classify(code), AcceptFailure::Fatal, "{code}");
            }
        }
    }

    #[test]
    fn a_recurring_per_peer_accept_error_pauses_accepting() {
        let resets = (0..=ACCEPT_RETRIES)
            .map(|_| io::Error::from(io::ErrorKind::ConnectionReset))
            .collect::<Vec<_>>();
        let started = Instant::now();
        let running = start(resets, false, DEFAULT_DRAIN_TIMEOUT);
        assert!(health(running.address).starts_with("HTTP/1.1 200"));
        // One more reset than the immediate retries allowed paused accepting before the request was accepted.
        let elapsed = started.elapsed();
        assert!(elapsed >= ACCEPT_BACKOFF, "{elapsed:?}");
        running.cancellation.cancel();
        let (result, _) = running
            .finished
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert_eq!(result.unwrap().connections, 1);
    }

    #[test]
    fn serving_continues_after_transient_accept_errors() {
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut errors = vec![
            io::Error::from(io::ErrorKind::ConnectionAborted),
            io::Error::from(io::ErrorKind::ConnectionReset),
            io::Error::from(io::ErrorKind::NetworkDown),
            io::Error::from(io::ErrorKind::HostUnreachable),
            io::Error::from(io::ErrorKind::OutOfMemory),
        ];
        #[cfg(unix)]
        errors.extend([
            io::Error::from_raw_os_error(libc::EMFILE),
            io::Error::from_raw_os_error(libc::EPROTO),
            io::Error::from_raw_os_error(libc::ENFILE),
        ]);
        let failures = errors.len();
        let running = start(errors, false, DEFAULT_DRAIN_TIMEOUT);
        assert!(health(running.address).starts_with("HTTP/1.1 200"));
        assert!(health(running.address).starts_with("HTTP/1.1 200"));
        running.cancellation.cancel();
        let (result, accepts) = running
            .finished
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        let statistics = result.unwrap();
        assert_eq!(statistics.connections, 2);
        assert_eq!(statistics.requests, 2);
        assert!(accepts > failures + 2, "only {accepts} accepts");
    }

    #[test]
    fn a_failure_to_accept_ends_the_server() {
        let broken = io::Error::other("the listener is broken");
        let running = start(vec![broken], false, DEFAULT_DRAIN_TIMEOUT);
        // Nothing cancels the server: the spaces, sweep and pacing end because serving failed.
        let (result, _) = running
            .finished
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        let Err(Error::Io(_)) = result else {
            unreachable!("serving fails with the accept's error, not {result:?}");
        };
        assert!(!running.cancellation.is_cancelled());
        // The listener is closed. Another socket listening on the port since would only make this fail spuriously, never pass falsely; and tests bind only ephemeral ports, which the system hands out across a wide range before reusing one, so reuse within this window is unlikely.
        assert!(TcpStream::connect(running.address).is_err());
    }

    #[test]
    fn the_listener_closes_before_a_bounded_drain() {
        let drain = Duration::from_secs(1);
        let running = start(Vec::new(), true, drain);
        // This connection is served, then never finishes closing.
        assert!(health(running.address).starts_with("HTTP/1.1 200"));
        let cancelled = Instant::now();
        running.cancellation.cancel();
        // New peers are refused while the connection drains, rather than queued. A peer that connects as the server is cancelled may still be accepted, since an accept that completes wins over a concurrent cancellation, but the accept after it is already cancelled, so at most one is.
        // Probed at least once, whether or not the first probe still connects, so that how the race went changes no coverage.
        let mut probes = 0;
        let mut open = true;
        while open {
            open = TcpStream::connect(running.address).is_ok();
            probes += usize::from(open);
            assert!(
                !open | (cancelled.elapsed() < drain / 2),
                "the listener stayed open"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            running.finished.try_recv().is_err(),
            "the drain ended early"
        );
        let (result, _) = running
            .finished
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        let elapsed = cancelled.elapsed();
        assert!(elapsed >= drain, "the drain ended after {elapsed:?}");
        assert!(elapsed < drain * 4, "the drain took {elapsed:?}");
        let connections = result.unwrap().connections;
        // Every condition is evaluated, so that how the race went changes no coverage.
        let served = (connections == 1) | ((connections == 2) & (probes > 0));
        assert!(
            served,
            "{connections} connections served after {probes} peers connected while cancelling"
        );
    }

    /// An error whose message already ends with its cause's.
    #[derive(Debug)]
    struct Repeating(std::io::Error);

    impl std::fmt::Display for Repeating {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(formatter, "writing failed: {}", self.0)
        }
    }

    impl std::error::Error for Repeating {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn a_cause_an_error_already_says_is_not_said_twice() {
        let error = Repeating(io::Error::other("the peer reset"));
        assert_eq!(describe(&error), "writing failed: the peer reset");
    }
}
