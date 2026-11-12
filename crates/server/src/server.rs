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
    SocketStreamReading, SocketStreamWriting, SocketType, TcpNoDelay, Timers,
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
    pub(crate) hub: Arc<Hub>,
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
    /// Where the cards come from, for the spaces that show them; see [`Self::with_cards`].
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

    /// Confirm the pairing waiting, if `typed` is the code its device shows, with or without spaces. Returns the device, as pairing shows it. Six other digits decline the pairing; text that is not six digits changes nothing. See `design/security.md`.
    pub fn confirm(
        &self,
        typed: &str,
    ) -> Result<tomori_security::pairing::ChosenName, &'static str> {
        let hub = self.0.upgrade().ok_or("The server has stopped.")?;
        let code = tomori_security::pairing::typed(typed)
            .ok_or("Type the 6-digit code shown on the device you are pairing, or \"decline\".")?;
        hub.confirm_pairing(crate::hub::Confirmation::Typed(&code))
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

    /// Open pairing, as the operator on the server's console, so that a new device may ask to pair for [`crate::PAIRING_OPEN_FOR`]; this is how an owner who has lost every paired device pairs again. Returns how long it is open for. See `design/security.md`.
    pub fn open(&self) -> Result<Duration, &'static str> {
        self.open_by(crate::hub::Opener::console())
    }

    /// Open pairing, as `by`.
    pub(crate) fn open_by(&self, by: crate::hub::Opener) -> Result<Duration, &'static str> {
        self.0
            .upgrade()
            .ok_or("The server has stopped.")?
            .open_pairing(by)
    }

    /// Close pairing, as the operator, withdrawing a pairing waiting. Returns whether it was open.
    pub fn close(&self) -> bool {
        self.0.upgrade().is_some_and(|hub| {
            hub.close_pairing("the operator closed it on the server's console.".to_owned())
        })
    }

    /// Whether a device may ask to pair; `None` once the server has stopped, or on one that does not pair.
    pub fn state(&self) -> Option<crate::PairingState> {
        self.0.upgrade()?.pairing_state()
    }
}

/// How long a disconnected participant keeps their identity, audio source and space, so that a headset that slept or walked out of range for a while comes back where it was. See `design/spaces.md`.
pub const DEFAULT_PARTICIPANT_RETENTION: Duration = Duration::from_secs(12 * 60 * 60);

/// The longest the server goes without looking at the agents' timers, and judging workers, by the clock.
const TIMER_LOOK: Duration = Duration::from_secs(1);

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

    /// Offer `environments` for spaces, in order, in place of those offered before, which are none until some are: the first is the one a space whose creator names none is bound to. A space kept across a restart in an environment no longer offered stays in it, with no instructions from it. It may be called before or after [`Self::with_kept_spaces`]: a space's layer of the instructions is looked up in the environments offered whenever it is shown or given to the agent, so a restored space is given the same layer as a new one. [`Self::with_environment`] changes one of them. See `design/environments.md`.
    pub fn with_environments(self, environments: impl IntoIterator<Item = Environment>) -> Self {
        self.hub.set_environments(Environments::new(environments));
        self
    }

    /// Offer `environment`, as [`Self::with_environment`] does, in place of the one offered with its identifier or else after those offered before, whose spaces show the cards `source` reads, such as the GitHub environment prototype's, which `tomori-environment-github-server` offers through this. The server reads them, with whatever credential `source` holds, at once when someone first watches them and then about once a minute while someone does, waiting for the source's rate limit when it is nearly used up and backing off after failures, and never sends them to visitors. Each space keeps where its participants put them. Call it after [`Self::with_environments`], which replaces the environments offered. See `design/github.md`.
    pub fn with_cards(
        mut self,
        environment: Environment,
        source: Arc<dyn crate::cards::CardSource>,
    ) -> Self {
        self.hub.set_board(crate::cards::Board::new(
            environment.id(),
            source.source(),
            source.service(),
        ));
        // The same write as `with_environment`'s, so that it replaces one with its identifier in place and reconciles server sides.
        self.hub.set_environment(environment);
        self.cards = Some(source);
        self
    }

    /// Offer `environment` in place of the one offered with its identifier, keeping that one's place in the order, which decides the default and the order clients see, or else after those offered, as [`Builder::environment`](crate::Builder::environment) registers one, so that a [`CommandOption`](crate::CommandOption) can change one environment without knowing the others. Its label, version, instructions, anchors, walls and server side are all the new one's: one registered with a server side and offered again without one has none. It takes effect exactly as [`Self::with_environments`] does, for every space, kept or new, whenever it is next shown or given to the agent; once the server serves, a space bound to it whose server side is not the same one has that side stopped and the new one started. See `design/builders.md`.
    pub fn with_environment(self, environment: Environment) -> Self {
        self.hub.set_environment(environment);
        self
    }

    /// The environments offered, in order, the first the default, so that an option can change one of them, such as its server side, rather than make it again. Unlike [`Builder::environments`](crate::Builder::environments) it returns copies, since the list is shared with the running server; a copy shares its server side, so giving one back unchanged restarts nothing.
    pub fn environments(&self) -> Vec<Environment> {
        self.hub.environments_offered()
    }

    /// The labels of the environments offered, in order.
    #[cfg(test)]
    pub(crate) fn environment_labels(&self) -> Vec<String> {
        self.environments()
            .iter()
            .map(|environment| environment.label().to_owned())
            .collect()
    }

    /// Offer `environments` unless some are offered already, as a test server does.
    #[cfg(feature = "fake")]
    pub(crate) fn with_default_environments(
        self,
        environments: impl IntoIterator<Item = Environment>,
    ) -> Self {
        if self.hub.environments_offered().is_empty() {
            self.hub.set_environments(Environments::new(environments));
        }
        self
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

    /// Change how long pairing stays open once an owner or the operator opens it; see [`crate::PAIRING_OPEN_FOR`].
    pub fn with_pairing_open_for(self, length: Duration) -> Self {
        self.hub.set_pairing_open_for(length);
        self
    }

    /// Change how long a knock waits for an answer; see [`crate::KNOCK_TIMEOUT`].
    pub fn with_knock_timeout(self, timeout: Duration) -> Self {
        self.hub.set_knock_timeout(timeout);
        self
    }

    /// Change how long the agent waits for an environment's server side to answer an inspection; see
    /// [`crate::INSPECT_TIMEOUT`].
    pub fn with_inspect_timeout(self, timeout: Duration) -> Self {
        self.hub.set_inspect_timeout(timeout);
        self
    }

    /// Change how long the agent waits for an environment's server side to answer an action; see
    /// [`crate::ACT_TIMEOUT`].
    pub fn with_act_timeout(self, timeout: Duration) -> Self {
        self.hub.set_act_timeout(timeout);
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

    /// Change how long cancelled connections, and the server sides of environments, may take to finish once serving ends, before they are abandoned, five seconds unless changed.
    pub fn with_drain_timeout(mut self, timeout: Duration) -> Self {
        self.drain_timeout = timeout;
        self
    }

    /// Highlight the agent's code panels with `highlighter`, as a test does to see what happens while it is slow.
    #[cfg(test)]
    pub(crate) fn with_highlighter(self, highlighter: crate::highlighting::Highlighter) -> Self {
        self.hub.set_highlighter(highlighter);
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
        S: SocketAccept
            + SocketSetOption<TcpNoDelay>
            + SocketStreamReading
            + SocketStreamWriting
            + SocketClose
            + Timers,
        S::Socket: 'static,
    {
        self.run_serving(scheduler, listener, None, cancellation)
            .await
    }

    /// Serve `listener`, and Tomori's MCP endpoint on its listener if given, until cancelled; see [`Self::run`].
    async fn run_serving<S>(
        &self,
        scheduler: &S,
        listener: S::Socket,
        tools: Option<(crate::mcp::Endpoint, S::Socket)>,
        cancellation: &Cancellation,
    ) -> Result<Statistics, Error>
    where
        S: SocketAccept
            + SocketSetOption<TcpNoDelay>
            + SocketStreamReading
            + SocketStreamWriting
            + SocketClose
            + Timers,
        S::Socket: 'static,
    {
        // Environments' server sides may run now, for the spaces bound while the server was configured.
        self.hub.serving();
        // The highlighters' grammars are made aside, so that the first code panel does not make them while it holds the
        // hub. See `design/projection.md`.
        #[cfg(feature = "highlight")]
        std::thread::spawn(tomori_highlight::prepare);
        // The counts when serving starts, taken before any connection is accepted: the publisher is polled after serving begins, and a client served before then would otherwise be counted as the starting point, so its arrival would never be published.
        let counts = self.counts();
        // Ends the spaces, the sweep and the pacing once serving has ended for any reason.
        let services = cancellation.child();
        let serving = async {
            let served = self
                .serve(
                    scheduler,
                    &self.application,
                    self.tls.as_ref(),
                    listener,
                    cancellation,
                )
                .await;
            services.cancel();
            served
        };
        let tools = async {
            if let Some((endpoint, listener)) = tools {
                // Serving Tomori's tools ends with the server's serving; failing alone, it is only said, and spaces work on without them.
                if let Err(error) = self
                    .serve(scheduler, &endpoint, None, listener, &services)
                    .await
                {
                    log::error!("Tomori's MCP endpoint stopped: {}.", describe(&error));
                }
            }
        };
        let (served, ((), ((), ((), ((), ((), ((), ((), ())))))))) = zip(
            serving,
            zip(
                tools,
                zip(
                    self.register_tools(&services),
                    zip(
                        self.supervise(scheduler, &services),
                        zip(
                            self.sweep(scheduler, &services),
                            zip(
                                self.pace(scheduler, &services),
                                zip(
                                    self.publish(scheduler, counts, &services),
                                    zip(
                                        self.refresh_cards(scheduler, &services),
                                        self.look_at_timers(scheduler, &services),
                                    ),
                                ),
                            ),
                        ),
                    ),
                ),
            ),
        )
        .await;
        served
    }

    /// Serve `listener`, and Tomori's MCP endpoint on `tools`, as [`Self::run`] serves `listener` alone. The endpoint admits only requests carrying a bearer token made for this run, and it is registered with the backends that take tools, for each directory with a space, with that token, at its address on `tools`, which should be a loopback address, since it serves plain HTTP: headsets, visitors and other machines have no business with it. See `design/mcp.md`.
    pub async fn run_with_tools<S>(
        &self,
        scheduler: &S,
        listener: S::Socket,
        tools: S::Socket,
        cancellation: &Cancellation,
    ) -> Result<Statistics, Error>
    where
        S: SocketAccept
            + SocketSetOption<TcpNoDelay>
            + SocketStreamReading
            + SocketStreamWriting
            + SocketClose
            + Timers
            + base_scheduler::SocketLocalAddress,
        S::Socket: 'static,
    {
        let address = scheduler
            .socket_local_address(&tools)
            .expect("a bound listener has an address");
        let url = tools_url(address);
        let token = tomori_security::generate_token();
        log::info!("Tomori's MCP endpoint is listening on {url}.");
        if !address.ip().is_loopback() {
            log::warn!(
                "Tomori's MCP endpoint listens on {address}, beyond this machine, over plain HTTP: anyone who can reach it and learns its token can use Tomori's tools."
            );
        }
        self.hub.serve_tools(
            tomori_backend::ToolServer::new(crate::mcp::NAME, url)
                .with_header("Authorization", format!("Bearer {token}")),
        );
        let endpoint = crate::mcp::Endpoint::new(self.hub.clone(), &token);
        self.run_serving(scheduler, listener, Some((endpoint, tools)), cancellation)
            .await
    }

    /// Register Tomori's MCP server with the backends that take tools, in each directory where a space is open, and remove it from each where none is any more, one at a time, as spaces there change, until cancelled. A registration that fails is said, and tried again when a space there is next bound or read again. See `design/mcp.md`.
    async fn register_tools(&self, cancellation: &Cancellation) {
        loop {
            let listener = self.hub.listen_tools();
            let Some(registration) = self.hub.next_registration() else {
                if let Either::Right(_) =
                    select(pin!(listener), pin!(cancellation.cancelled())).await
                {
                    return;
                }
                continue;
            };
            let crate::hub::tools::Registration {
                backend,
                directory,
                server,
            } = registration;
            let found = self
                .hub
                .backends
                .get(&backend)
                .expect("only a backend the server has is due");
            let (done, result) = match &server {
                Some(server) => ("registered", found.register_tools(&directory, server).await),
                None => (
                    "removed",
                    found.unregister_tools(&directory, crate::mcp::NAME).await,
                ),
            };
            match result {
                Ok(()) => log::info!("Tomori's tools are {done} for {backend} in {directory}."),
                Err(error) => log::warn!(
                    "Tomori's tools could not be {done} for {backend} in {directory}: {}",
                    redacted(error.message, server.as_ref())
                ),
            }
        }
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

    /// Look at the agents' timers by the clock about every second, [`TIMER_LOOK`], and as the soonest falls due, so that
    /// one set seconds ahead fires within milliseconds of its second, and one due sooner than the server would look
    /// again, as one set a second ahead, within half a second of it. See `design/timers.md`.
    async fn look_at_timers<S: Timers>(&self, scheduler: &S, cancellation: &Cancellation) {
        let mut timer = scheduler.timer();
        loop {
            let wall = jiff::Timestamp::now();
            let wait = self
                .hub
                .next_timer_due(wall)
                .and_then(|due| Duration::try_from(due.duration_since(wall)).ok())
                .map_or(TIMER_LOOK, |until| until.min(TIMER_LOOK));
            match scheduler
                .timer_wait_until(&mut timer, scheduler.now() + wait, cancellation)
                .await
            {
                Ok(()) => self.hub.tick_timers(jiff::Timestamp::now()),
                Err(error) => {
                    if !cancellation.is_cancelled() {
                        log::error!("The agents' timer failed: {error}");
                    }
                    return;
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
    /// Each accepted socket has `TCP_NODELAY` set before it is served, as `base_http_server::serve` does, since `serve_connection` leaves a socket it is given as it is: with Nagle's algorithm on, small writes such as speech frames and patches can wait for the peer's acknowledgement of the last.
    ///
    /// TODO(upstream): this follows `base_http_server::serve`, which passes every handler one cancellation shared by all connections. The keepalive sweep must end one silent connection, including a response write blocked on a peer that is gone, so each connection here gets its own child cancellation. Since base-http 0.3, `serve` keeps accepting after transient accept errors and closes its listener before draining, but it still joins its connections without a time limit. Use `serve` again once it offers per-connection cancellation and a bounded drain, requested in base-rs/base-http#73.
    async fn serve<S, H>(
        &self,
        scheduler: &S,
        handler: &H,
        tls: Option<&Arc<rustls::ServerConfig>>,
        listener: S::Socket,
        cancellation: &Cancellation,
    ) -> Result<Statistics, Error>
    where
        S: SocketAccept
            + SocketSetOption<TcpNoDelay>
            + SocketStreamReading
            + SocketStreamWriting
            + SocketClose
            + Timers,
        S::Socket: 'static,
        H: base_http_server::Handler<Error = std::convert::Infallible>,
    {
        let server_options = base_http_server::Options::default();
        let options = server_options.http1.clone();
        let acceptor = tls.map(|config| {
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
                    application: handler,
                    watch,
                };
                connections.push(async move {
                    // A latency hint only: the connection is served without it.
                    if let Err(error) = scheduler
                        .socket_set_option(&socket, TcpNoDelay, true, &connection)
                        .await
                    {
                        log::debug!("Could not disable Nagle's algorithm for connection {serial}: {error}.");
                    }
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
                Ok(Job::Delete(deleted)) => {
                    spaces.push(Box::pin(finish_deletion(self.hub.clone(), deleted))
                        as Pin<Box<dyn Future<Output = ()>>>);
                }
                Err(_) => break,
            }
        }
        self.hub.shutdown();
        while spaces.next().await.is_some() {}
        // The environments' server sides were cancelled with the spaces: give them the drain timeout to stop, then leave those that have not.
        let never = Cancellation::never();
        let mut timer = scheduler.timer();
        let deadline = scheduler.now() + self.drain_timeout;
        let abandoned = self
            .hub
            .server_sides_stopped(async {
                // Whether the deadline passes or the timer fails, the wait is over.
                let _ = scheduler
                    .timer_wait_until(&mut timer, deadline, &never)
                    .await;
            })
            .await;
        if abandoned > 0 {
            log::warn!(
                "Stopped waiting for {abandoned} environment server sides that had not stopped {:?} after being cancelled.",
                self.drain_timeout
            );
        }
    }
}

/// `message`, from an agent that refused a registration of `server`, without the values of its headers, or any long word of them, such as its bearer token without `Bearer`, which an agent might repeat in saying what it refused.
fn redacted(mut message: String, server: Option<&tomori_backend::ToolServer>) -> String {
    for (_, value) in server.iter().flat_map(|server| &server.headers) {
        message = message.replace(value.as_str(), "(secret)");
        for word in value.split_whitespace().filter(|word| word.len() >= 16) {
            message = message.replace(word, "(secret)");
        }
    }
    message
}

/// The URL of Tomori's MCP endpoint listening on `address`: on the loopback address of its family when it listens on every address, since OpenCode runs on this machine.
fn tools_url(address: SocketAddr) -> String {
    let host = match address.ip() {
        std::net::IpAddr::V4(ip) if ip.is_unspecified() => std::net::Ipv4Addr::LOCALHOST.into(),
        std::net::IpAddr::V6(ip) if ip.is_unspecified() => std::net::Ipv6Addr::LOCALHOST.into(),
        ip => ip,
    };
    format!(
        "http://{}{}",
        SocketAddr::new(host, address.port()),
        crate::mcp::PATH
    )
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

/// Finish deleting a space, whose own handle to its session is closed: delete the session, if its owner asked, and tell them what became of it, or else keep it, removing Tomori's instruction entries as for a forgotten space. A session another space is bound to by now is never deleted, and keeps its entries; and the session is deleted only once the record of spaces no longer lists the space, so that a server that stops meanwhile never lists a space whose session it deleted. See `design/spaces.md`.
async fn finish_deletion(hub: Arc<Hub>, deleted: crate::hub::Deleted) {
    let crate::hub::Deleted {
        backend,
        session,
        asker,
    } = deleted;
    let bound = hub.binds(&backend, &session);
    let Some(asker) = asker else {
        if !bound && hub.takes_instructions(&backend) {
            forget_instructions(hub, backend, session).await;
        }
        return;
    };
    let result = match hub.backends.get(&backend) {
        _ if bound => Err("another space is bound to it now".to_owned()),
        _ if !hub.keep() => Err("the record of spaces could not be written".to_owned()),
        None => Err(format!(
            "the {backend} backend is not enabled on this server"
        )),
        Some(found) => match found.delete_session(&session).await {
            // Already gone, as for a session that had closed because it no longer existed.
            Err(error) if error.kind != tomori_backend::ErrorKind::NotFound => Err(error.message),
            _ => Ok(()),
        },
    };
    match &result {
        Ok(()) => log::info!("Deleted {backend} session {session}, as its space was deleted."),
        Err(error) => {
            log::warn!("Kept {backend} session {session}, whose space was deleted: {error}");
            // Kept, it keeps none of Tomori's entries, as for a space deleted without its session.
            if !bound && hub.takes_instructions(&backend) {
                forget_instructions(hub.clone(), backend, session).await;
            }
        }
    }
    hub.session_deleted(&asker, result);
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
    // The agent may have moved the session while the server was stopped, perhaps from a directory that is gone: it is
    // bound where the agent says it is now, as the agent names it, which the space follows, once that is found within
    // the workspace roots, as the kept directory is otherwise, or if the agent cannot say. See `design/mcp.md`.
    let located = match found.locate(&session).await {
        Ok(Some(now)) if now != directory => {
            log::info!("Space {id}'s session is now in {now}, rather than {directory}.");
            Some(now)
        }
        Ok(_) => None,
        Err(error) => {
            log::info!(
                "Could not tell where space {id}'s session is now, so it is bound in {directory}: {error}"
            );
            None
        }
    };
    let Some(resolved) = resolve(
        scheduler,
        &places,
        located.as_deref().unwrap_or(&directory),
        cancellation,
    )
    .await
    else {
        if !cancellation.is_cancelled() {
            hub.restore_failed(&id, DIRECTORY_UNAVAILABLE.to_owned(), false);
        }
        return;
    };
    let directory = located.unwrap_or(resolved);
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
    if !hub.bound(&id, session.id(), &directory, snapshot) {
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
    // A deleted space's deletion, held until its session is closed below.
    let mut deletion = None;
    // A command the session is performing, such as a decision waiting for the agent's connection to return, does not
    // hold up shutdown: the space stops as soon as shutdown begins, and its session is closed below.
    let error = match select(
        pin!(drive(hub, space, &session, &receiver, voice, &mut deletion)),
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
    // Only now is the session deleted, or its entries removed, so that nothing still holds it: the deletion taken, or
    // one still waiting in the commands, dropped with them.
    drop(deletion);
    drop(receiver);
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
            hub.sessions(
                &participant,
                &backend,
                Err(DIRECTORY_UNAVAILABLE.to_owned()),
            );
            return;
        }
    };
    let Some(found) = hub.backends.get(&backend) else {
        hub.sessions(&participant, &backend, Err("Unknown backend.".to_owned()));
        return;
    };
    let sessions = found
        .sessions(&directory)
        .await
        .map_err(|error| error.message);
    hub.sessions(&participant, &backend, sessions);
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

/// Forward session events to the hub and the space's voice while performing the space's commands. Returns why the session stopped, if it failed. A space deleted meanwhile stops, leaving its deletion in `deletion`.
async fn drive(
    hub: &Hub,
    space: &str,
    session: &AnySession,
    commands: &async_channel::Receiver<SpaceCommand>,
    mut voice: Option<Voice>,
    deletion: &mut Option<crate::hub::Deletion>,
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
                    // The panels of a reply it completed are highlighted outside the hub's lock, and projected before the next event, so that a later reply's are never projected before an earlier one's; if the space stops meanwhile, as the server shuts down, they are not projected. See `crate::highlighting`.
                    if let Some(projecting) = hub.space_event(space, event) {
                        let replies =
                            crate::highlighting::highlight(projecting, hub.highlighter()).await;
                        hub.project(space, replies);
                    }
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
                SpaceCommand::Delete(deleted) => {
                    *deletion = Some(deleted);
                    break;
                }
            }
        }
        None
    };
    match select(pin!(events), pin!(commands)).await {
        Either::Left((error, _)) | Either::Right((error, _)) => error,
    }
}

#[cfg(test)]
mod tests;
