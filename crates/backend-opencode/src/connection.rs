// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The connection to the OpenCode server, on its own thread.
//!
//! [`base_http_client::Client`] and its [`Driver`] borrow from a scheduler and
//! the driver must be polled alongside every call, so the whole connection
//! lives on one dedicated thread that owns its scheduler. The [`Backend`] and
//! its sessions are `'static` handles that talk to this thread over channels:
//! every session owns an event channel the thread feeds, and commands (prompts,
//! replies, snapshots) travel one way and are answered on a per-command
//! channel. The thread reads the server's one global event stream and fans its
//! events out to the sessions they name.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use backend::{Event, Modes, SessionSpec, SessionSummary, Snapshot, Worker, WorkerState};
use base_http::body::ReadableBody;
use base_http_client::{Client, Endpoint};
use base_scheduler::{
    AddressHints, AddressInfoCollection, AddressInfoView, AddressResolution, AddressService,
    Cancellation, SocketClose, SocketConnect, SocketOpen, SocketSetOption, SocketStreamReading,
    SocketStreamWriting, TcpNoDelay, Timers,
};
use base_scheduler_direct::{DirectOptions, DirectScheduler};
use futures_lite::StreamExt;
use futures_lite::future::block_on;
use futures_util::stream::FuturesUnordered;
use serde_json::Value;

use crate::api::{self, Failure};
use crate::endpoint::Resolved;
use crate::wire;

/// How often the running state of each session is re-checked against the
/// server's record. The fast path is the event stream's idle events; this is
/// the backstop that clears a busy state when one of those is missed, so a
/// queue gated on the agent being idle cannot be left stuck. It is a no-op
/// while a turn is genuinely running, which is why the cadence can be loose:
/// it only matters once a turn has ended and the stream has not reported it.
const RECONCILE_INTERVAL: Duration = Duration::from_secs(1);
/// A snapshot is isolated from the event dispatcher and bounded in time and
/// concurrency, including every response body and page.
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_SNAPSHOTS: usize = 16;
const MAX_RECONCILIATIONS: usize = 16;

/// How many reconciliations list a session's agents again while OpenCode offers none.
const LISTINGS: u8 = 30;

/// A worker replacement carries the snapshot epoch under which it was queued.
/// Other events retain their original ordering and are never invalidated.
pub(crate) enum QueuedEvent {
    Ordinary(Event),
    Workers {
        event: Event,
        epoch: u64,
        current: Arc<AtomicU64>,
    },
}

impl QueuedEvent {
    pub(crate) fn into_current(self) -> Option<Event> {
        match self {
            Self::Ordinary(event) => Some(event),
            Self::Workers {
                event,
                epoch,
                current,
            } => (epoch == current.load(Ordering::Acquire)).then_some(event),
        }
    }
}

/// The sender side of one session's event channel.
#[derive(Clone)]
pub struct Events {
    tx: async_channel::Sender<QueuedEvent>,
    worker_epoch: Arc<AtomicU64>,
}

impl Events {
    pub fn new() -> (Self, async_channel::Receiver<QueuedEvent>) {
        let (tx, rx) = async_channel::bounded(1024);
        (
            Self {
                tx,
                worker_epoch: Arc::new(AtomicU64::new(0)),
            },
            rx,
        )
    }

    /// Attempt delivery without blocking the shared dispatcher.
    pub fn deliver(&self, event: Event) -> bool {
        self.tx.try_send(QueuedEvent::Ordinary(event)).is_ok()
    }

    /// Worker snapshots must be retried if the consumer is temporarily behind.
    fn try_deliver_workers(&self, workers: Vec<Worker>) -> bool {
        self.tx
            .try_send(QueuedEvent::Workers {
                event: Event::WorkersChanged { workers },
                epoch: self.worker_epoch.load(Ordering::Acquire),
                current: self.worker_epoch.clone(),
            })
            .is_ok()
    }

    fn try_deliver_error(&self, message: &str) -> bool {
        self.tx
            .try_send(QueuedEvent::Ordinary(Event::Error {
                message: message.to_owned(),
            }))
            .is_ok()
    }

    /// An installed snapshot supersedes older queued worker replacements.
    fn advance_workers(&self) {
        self.worker_epoch.fetch_add(1, Ordering::AcqRel);
    }
}

/// A registered session: what the thread needs to serve it. Its id is the map
/// key.
pub struct Registration {
    pub events: Events,
    /// The messages this session has reported as started.
    pub seen: Mutex<HashSet<String>>,
    /// The session's running state as last reconciled against the server's
    /// record, so the reconciler reports a change only once.
    running: RunningState,
    /// Observed workers, keyed by child ID. Only a deletion event removes one.
    children: BTreeMap<String, Worker>,
    /// Child deletions observed before the initial snapshot was installed.
    initial_deletions: HashSet<String>,
    /// State and title events for children not yet discovered by the initial
    /// listing. Their parent is not included in execution events.
    initial_changes: HashMap<String, (Option<WorkerState>, Option<String>)>,
    /// Creations whose parent has not yet been discovered by the snapshot.
    initial_creations: BTreeMap<String, Worker>,
    /// Worker events are not published until a snapshot has been installed.
    workers_ready: bool,
    /// A worker event could not enter the bounded event channel and needs retry.
    workers_dirty: bool,
    /// Once the cache overflows, omissions cannot establish deletion.
    workers_incomplete: bool,
    snapshot_in_flight: Option<Cancellation>,
    reconcile_in_flight: Option<Cancellation>,
    event_revision: u64,
    /// The agents the session offers and the one its prompts go to, as last confirmed by the server.
    modes: Modes,
    /// Latest modes must be retried if the bounded channel was full.
    modes_dirty: bool,
    /// The session's directory, whose agents are its modes.
    directory: String,
    /// How many more times reconciliation lists the agents while none are offered: OpenCode answers an empty listing
    /// while it loads a directory.
    listings: u8,
}

/// The running state of a session as the server's record reports it, derived
/// from `time.busy` after `time.idle`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RunningState {
    /// Not yet reconciled, so no change has been observed.
    Unknown,
    Running,
    Idle,
}

impl RunningState {
    fn event(self) -> Option<Event> {
        match self {
            RunningState::Unknown => None,
            RunningState::Running => Some(Event::Busy),
            RunningState::Idle => Some(Event::Idle),
        }
    }
}

impl Registration {
    pub fn new() -> (Self, async_channel::Receiver<QueuedEvent>) {
        let (events, rx) = Events::new();
        (
            Self {
                events,
                seen: Mutex::new(HashSet::new()),
                running: RunningState::Unknown,
                children: BTreeMap::new(),
                initial_deletions: HashSet::new(),
                initial_changes: HashMap::new(),
                initial_creations: BTreeMap::new(),
                workers_ready: false,
                workers_dirty: false,
                workers_incomplete: false,
                snapshot_in_flight: None,
                reconcile_in_flight: None,
                event_revision: 0,
                modes: Modes::default(),
                modes_dirty: false,
                directory: String::new(),
                listings: LISTINGS,
            },
            rx,
        )
    }

    fn publish_workers(&mut self) {
        if self.workers_ready {
            self.workers_dirty = if self.workers_incomplete {
                !self.events.try_deliver_error(
                    "Worker observation exceeds the supported limit; reattach after reducing it.",
                )
            } else {
                !self
                    .events
                    .try_deliver_workers(self.children.values().cloned().collect())
            };
        }
    }

    fn publish_modes(&mut self) {
        self.modes_dirty = !self.events.deliver(Event::ModesChanged(self.modes.clone()));
    }

    fn reconcile_running(&mut self, running: RunningState) {
        if self.running != running
            && let Some(event) = running.event()
            && self.events.deliver(event)
        {
            self.running = running;
        }
    }

    /// Install a snapshot without losing lifecycle events already observed
    /// before the initial consumer started.
    fn install_workers(&mut self, workers: &mut Vec<Worker>) -> backend::Result<()> {
        if self.workers_incomplete {
            return Err(backend::Error::new(
                backend::ErrorKind::Unavailable,
                "Worker observation exceeds the supported limit.",
            ));
        }
        if !self.workers_ready {
            let additional = workers
                .iter()
                .filter(|worker| {
                    !self.initial_deletions.contains(&worker.id)
                        && !self.children.contains_key(&worker.id)
                })
                .count();
            if self.children.len().saturating_add(additional) > api::MAX_TRACKED_WORKERS {
                return Err(backend::Error::new(
                    backend::ErrorKind::Unavailable,
                    "Worker observation exceeds the supported limit.",
                ));
            }
        }
        self.events.advance_workers();
        if !self.workers_ready {
            for mut worker in workers.drain(..) {
                if !self.initial_deletions.contains(&worker.id) {
                    if !self.children.contains_key(&worker.id)
                        && let Some((state, title)) = self.initial_changes.get(&worker.id)
                    {
                        if let Some(state) = state {
                            worker.state = *state;
                        }
                        if let Some(title) = title {
                            worker.title.clone_from(title);
                        }
                    }
                    self.children.entry(worker.id.clone()).or_insert(worker);
                }
            }
            // A nested child can be created while its preexisting parent is
            // still being discovered. Attach pending descendants only after
            // their parent is known, preserving the actual hierarchy.
            loop {
                let ready: Vec<_> = self
                    .initial_creations
                    .iter()
                    .filter(|(_, worker)| {
                        worker
                            .parent
                            .as_ref()
                            .is_some_and(|parent| self.children.contains_key(parent))
                    })
                    .map(|(id, _)| id.clone())
                    .collect();
                if ready.is_empty() {
                    break;
                }
                for id in ready {
                    let mut worker = self
                        .initial_creations
                        .remove(&id)
                        .expect("pending creation");
                    if self.children.contains_key(&id) || self.initial_deletions.contains(&id) {
                        continue;
                    }
                    if self.children.len() == api::MAX_TRACKED_WORKERS {
                        return Err(backend::Error::new(
                            backend::ErrorKind::Unavailable,
                            "Worker observation exceeds the supported limit.",
                        ));
                    }
                    if let Some((state, title)) = self.initial_changes.get(&id) {
                        if let Some(state) = state {
                            worker.state = *state;
                        }
                        if let Some(title) = title {
                            worker.title.clone_from(title);
                        }
                    }
                    self.children.insert(id, worker);
                }
            }
        }
        *workers = self.children.values().cloned().collect();
        self.initial_deletions.clear();
        self.initial_changes.clear();
        self.initial_creations.clear();
        self.workers_ready = true;
        self.workers_dirty = false;
        Ok(())
    }
}

/// One command for the thread, carrying the channel to answer it on.
///
/// Every reply channel carries a `backend::Error`, not an `api::Failure`: the
/// thread maps failures to the neutral error before answering.
pub enum Command {
    Sessions {
        directory: String,
        reply: async_channel::Sender<backend::Result<Vec<SessionSummary>>>,
    },
    Create {
        spec: SessionSpec,
        reply:
            async_channel::Sender<backend::Result<(String, async_channel::Receiver<QueuedEvent>)>>,
    },
    Attach {
        id: String,
        directory: String,
        reply: async_channel::Sender<backend::Result<async_channel::Receiver<QueuedEvent>>>,
    },
    Snapshot {
        id: String,
        reply: async_channel::Sender<backend::Result<Snapshot>>,
    },
    Prompt {
        id: String,
        key: String,
        text: String,
        reply: async_channel::Sender<backend::Result<()>>,
    },
    Interrupt {
        id: String,
        reply: async_channel::Sender<backend::Result<()>>,
    },
    Decide {
        id: String,
        request: String,
        decision: String,
        reply: async_channel::Sender<backend::Result<()>>,
    },
    Answer {
        id: String,
        form: String,
        answer: Value,
        reply: async_channel::Sender<backend::Result<()>>,
    },
    /// Choose the agent for `id`'s later prompts.
    SetMode {
        id: String,
        mode: String,
        reply: async_channel::Sender<backend::Result<()>>,
    },
    /// Close `id`'s registration without deleting its OpenCode session.
    Close { id: String },
}

struct SnapshotResult {
    id: String,
    registration: Arc<AtomicU64>,
    reply: async_channel::Sender<backend::Result<Snapshot>>,
    result: backend::Result<Snapshot>,
}

struct ReconcileResult {
    id: String,
    registration: Arc<AtomicU64>,
    revision: u64,
    modes: Option<Modes>,
    running: Option<bool>,
}

/// The commands channel, shared by the backend and every session handle.
#[derive(Clone)]
pub struct Commands {
    tx: async_channel::Sender<Command>,
}

impl Commands {
    pub fn new() -> (Self, async_channel::Receiver<Command>) {
        let (tx, rx) = async_channel::bounded(256);
        (Self { tx }, rx)
    }

    /// The raw sender, for fire-and-forget commands like close.
    pub fn tx(&self) -> &async_channel::Sender<Command> {
        &self.tx
    }

    /// Send a command built against a fresh reply channel, and wait for it.
    /// The reply channel carries the neutral `Result`, so this returns the
    /// command's own outcome, not a double wrap.
    pub async fn reply_with<T: Send + 'static>(
        &self,
        build: impl FnOnce(async_channel::Sender<std::result::Result<T, backend::Error>>) -> Command,
    ) -> std::result::Result<T, backend::Error> {
        let (reply, waiting) = async_channel::bounded(1);
        let command = build(reply);
        self.tx.send(command).await.map_err(|_| {
            backend::Error::new(backend::ErrorKind::Unavailable, "the connection has closed")
        })?;
        waiting.recv().await.map_err(|_| {
            backend::Error::new(backend::ErrorKind::Unavailable, "the connection has closed")
        })?
    }
}

fn map_failure(failure: Failure) -> backend::Error {
    let kind = match failure.status {
        Some(404) => backend::ErrorKind::NotFound,
        Some(status) if (400..500).contains(&status) => backend::ErrorKind::Invalid,
        _ => backend::ErrorKind::Unavailable,
    };
    backend::Error::new(kind, failure.message)
}

/// Run the connection until its command channel or the server stream closes.
/// Blocking: called on a dedicated thread.
pub fn run_thread(resolved: Resolved, commands: async_channel::Receiver<Command>) {
    if let Err(message) = block_on(drive(resolved, commands)) {
        eprintln!("the opencode connection ended: {message}");
    }
}

async fn drive(
    resolved: Resolved,
    commands: async_channel::Receiver<Command>,
) -> std::result::Result<(), String> {
    let scheduler = DirectScheduler::new(DirectOptions {
        threads: 2,
        capacity: 256,
    })
    .map_err(|error| format!("could not start the connection scheduler: {error}"))?;
    let result = run(&scheduler, resolved, commands).await;
    block_on(scheduler.shutdown());
    result
}

async fn run<'scope, S: Scheduler<'scope> + Send + Sync + 'static>(
    scheduler: &S,
    resolved: Resolved,
    commands: async_channel::Receiver<Command>,
) -> std::result::Result<(), String>
where
    <S as Scheduler<'scope>>::Socket: 'static,
{
    let cancellation = Cancellation::new();
    let address = scheduler
        .address_resolve(
            Some(resolved.host.trim_matches(['[', ']'])),
            AddressService::Port(resolved.port),
            AddressHints::default(),
            &cancellation,
        )
        .await
        .map_err(|error| format!("resolving {}: {error}", resolved.authority()))?
        .entries()
        .next()
        .map(|entry| entry.address())
        .ok_or_else(|| "the server's address did not resolve to any endpoint".to_owned())?;

    let endpoint = Endpoint::new(address);
    let (client, driver) = Client::new(scheduler, endpoint, base_http_client::Options::default())
        .map_err(|error| format!("creating the HTTP client: {error}"))?;

    // The driver must be polled for as long as the client is used, so it runs
    // alongside the connection loop in the same future; finishing either ends
    // both. The cancellation is shared so closing the connection unwinds both.
    let (driver_result, _) = futures_lite::future::zip(
        async {
            let result = driver.run(&cancellation).await;
            cancellation.cancel();
            result
        },
        async {
            service(&client, resolved, commands, &cancellation, scheduler).await;
            cancellation.cancel();
        },
    )
    .await;
    driver_result.map_err(|error| error.to_string())
}

/// The scheduler capabilities the connection needs.
trait Scheduler<'scope>:
    AddressResolution
    + SocketOpen
    + SocketConnect
    + SocketClose
    + SocketSetOption<TcpNoDelay>
    + SocketStreamReading
    + SocketStreamWriting
    + Timers
    + 'scope
{
    type Socket: 'static;
}

impl<'scope, S> Scheduler<'scope> for S
where
    S: AddressResolution
        + SocketOpen
        + SocketConnect
        + SocketClose
        + SocketSetOption<TcpNoDelay>
        + SocketStreamReading
        + SocketStreamWriting
        + Timers
        + 'scope,
    S::Socket: 'static,
{
    type Socket = S::Socket;
}

/// Wraps a future in a `FusedFuture`, pinning its inner future, since
/// `select!` requires every branch to implement it and the channel and body
/// futures do not.
struct Fused<F: Future> {
    inner: Pin<Box<F>>,
    done: bool,
}

impl<F: Future> Fused<F> {
    fn new(inner: F) -> Self {
        Self {
            inner: Box::pin(inner),
            done: false,
        }
    }
}

impl<F: Future> Future for Fused<F> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.done {
            unreachable!("a fused future was polled after finishing");
        }
        let output = self.inner.as_mut().poll(cx);
        if output.is_ready() {
            self.done = true;
        }
        output
    }
}

impl<F: Future> futures_core::FusedFuture for Fused<F> {
    fn is_terminated(&self) -> bool {
        self.done
    }
}

/// Read the server's global event stream and service commands, alternating
/// reads over the stream, command channel reads and a periodic reconciliation
/// of each session's running state, until the backend closes or the stream ends.
async fn service<S: Timers>(
    client: &Client<'_>,
    resolved: Resolved,
    commands: async_channel::Receiver<Command>,
    cancellation: &Cancellation,
    timers: &S,
) {
    let mut request = base_http::Request::get(base_http::Path::from("/api/event".to_owned()));
    if let Some(authorization) = resolved.authorization_header()
        && let Ok(field) = base_http::Field::new("authorization", authorization)
    {
        let _ = request.fields.append(field);
    }
    let mut stream = match client.call(request, cancellation).await {
        Ok(response) => response.body,
        Err(failure) => {
            eprintln!("opening the event stream: {failure}");
            return;
        }
    };

    let mut registrations = HashMap::<String, Registration>::new();
    let mut pending = String::new();
    let mut timer = timers.timer();
    let mut next_reconcile = timers.now() + RECONCILE_INTERVAL;
    let mut reconcile_offset = 0usize;
    let mut snapshots: FuturesUnordered<Pin<Box<dyn Future<Output = SnapshotResult> + '_>>> =
        FuturesUnordered::new();
    let mut reconciliations: FuturesUnordered<Pin<Box<dyn Future<Output = ReconcileResult> + '_>>> =
        FuturesUnordered::new();

    loop {
        futures_util::select! {
            command = Fused::new(commands.recv()) => {
                let Ok(command) = command else {
                    break;
                };
                match command {
                    Command::Snapshot { id, reply } => {
                        if let Some(registration) = registrations.get_mut(&id) {
                            if registration.snapshot_in_flight.is_some() || snapshots.len() >= MAX_SNAPSHOTS {
                                let _ = reply.try_send(Err(backend::Error::new(backend::ErrorKind::Unavailable, "A session snapshot is already running or the snapshot limit was reached; please retry.")));
                            } else {
                                let local_cancel = cancellation.child();
                                registration.snapshot_in_flight = Some(local_cancel.clone());
                                let registration = registration.events.worker_epoch.clone();
                                let modes = registrations[&id].modes.clone();
                                let resolved = &resolved;
                                snapshots.push(Box::pin(async move {
                                    let result = futures_lite::future::race(
                                        async { snapshot(client, resolved, &id, modes, &local_cancel).await.map_err(map_failure) },
                                        async {
                                            let timer = timers.sleep(SNAPSHOT_TIMEOUT, cancellation).await;
                                            Err(backend::Error::new(backend::ErrorKind::Unavailable, format!("Snapshot timed out or was cancelled: {timer:?}")))
                                        },
                                    ).await;
                                    local_cancel.cancel();
                                    SnapshotResult { id, registration, reply, result }
                                }));
                            }
                        } else {
                            let _ = reply.try_send(Err(backend::Error::new(backend::ErrorKind::Closed, "The session is not registered.")));
                        }
                    }
                    command => {
                        if !handle_command(command, client, &resolved, &mut registrations, cancellation).await {
                            break;
                        }
                    }
                }
                retry_worker_delivery(&mut registrations);
            }
            completed = Fused::new(async {
                if snapshots.is_empty() {
                    std::future::pending::<Option<SnapshotResult>>().await
                } else {
                    snapshots.next().await
                }
            }) => {
                if let Some(mut completed) = completed {
                    match registrations.get_mut(&completed.id) {
                        Some(registration) if Arc::ptr_eq(&registration.events.worker_epoch, &completed.registration) => {
                            registration.snapshot_in_flight = None;
                            if let Ok(snapshot) = &mut completed.result
                                && let Err(error) = registration.install_workers(&mut snapshot.workers)
                            {
                                completed.result = Err(error);
                            }
                        }
                        _ => completed.result = Err(backend::Error::new(backend::ErrorKind::Closed, "The session was closed during its snapshot.")),
                    }
                    let _ = completed.reply.try_send(completed.result);
                }
            }
            completed = Fused::new(async {
                if reconciliations.is_empty() {
                    std::future::pending::<Option<ReconcileResult>>().await
                } else {
                    reconciliations.next().await
                }
            }) => {
                if let Some(completed) = completed
                    && let Some(registration) = registrations.get_mut(&completed.id)
                    && Arc::ptr_eq(&registration.events.worker_epoch, &completed.registration)
                {
                    registration.reconcile_in_flight = None;
                    if registration.event_revision == completed.revision {
                        if let Some(modes) = completed.modes
                            && !modes.offered.is_empty()
                            && registration.modes.offered.is_empty()
                        {
                            registration.modes = modes;
                            registration.publish_modes();
                        }
                        if let Some(running) = completed.running {
                            let running = if running { RunningState::Running } else { RunningState::Idle };
                            registration.reconcile_running(running);
                        }
                    }
                }
            }
            result = Fused::new(stream.read(cancellation)) => {
                let frame = match result {
                    Ok(frame) => frame,
                    Err(error) => {
                        eprintln!("the opencode event stream failed: {error}; closing the connection");
                        break;
                    },
                };
                let end = frame.is_end();
                if let base_http::body::BodyFrame::Data { data, .. } = frame {
                    pending.push_str(&String::from_utf8_lossy(&data));
                    while let Some(index) = pending.find("\n\n") {
                        let group = pending[..index + 2].to_owned();
                        pending = pending[index + 2..].to_owned();
                        if let Some(payload) = sse_payload(&group) {
                            dispatch_events(&payload, &mut registrations);
                        }
                    }
                }
                if end {
                    eprintln!("the opencode event stream ended; closing the connection");
                    break;
                }
            }
            // Re-check each session's running state against the server's record,
            // so a missed idle event cannot leave a busy state — and the queue it
            // gates — stuck. A no-op while no session is registered.
            due = Fused::new(timers.timer_wait_until(&mut timer, next_reconcile, cancellation)) => {
                if let Err(error) = due {
                    if !cancellation.is_cancelled() {
                        eprintln!("the state reconciliation timer failed: {error}");
                    }
                    break;
                }
                let mut ids: Vec<_> = registrations.keys().cloned().collect();
                ids.sort();
                if !ids.is_empty() {
                    let start = reconcile_offset % ids.len();
                    ids.rotate_left(start);
                    reconcile_offset = (start + MAX_RECONCILIATIONS) % ids.len();
                }
                for id in ids {
                    let registration = registrations.get_mut(&id).expect("registered at the start of this tick");
                    if registration.reconcile_in_flight.is_some() || reconciliations.len() >= MAX_RECONCILIATIONS {
                        continue;
                    }
                    let local_cancel = cancellation.child();
                    registration.reconcile_in_flight = Some(local_cancel.clone());
                    let identity = registration.events.worker_epoch.clone();
                    let revision = registration.event_revision;
                    let directory = registration.directory.clone();
                    let current = registration.modes.current.clone();
                    let list = registration.modes.offered.is_empty() && registration.listings > 0;
                    if list { registration.listings -= 1; }
                    let resolved = &resolved;
                    reconciliations.push(Box::pin(async move {
                        let (modes, running) = futures_lite::future::race(
                            async {
                                let modes = if list {
                                    let info = current.map_or(Value::Null, |agent| serde_json::json!({"agent":agent}));
                                    Some(modes(client, resolved, &directory, &info, &local_cancel).await)
                                } else { None };
                                let running = api::session_running(client, resolved, &id, &local_cancel).await.ok().flatten();
                                (modes, running)
                            },
                            async {
                                let _ = timers.sleep(SNAPSHOT_TIMEOUT, cancellation).await;
                                (None, None)
                            },
                        ).await;
                        local_cancel.cancel();
                        ReconcileResult { id, registration: identity, revision, modes, running }
                    }));
                }
                retry_worker_delivery(&mut registrations);
                next_reconcile = timers.now() + RECONCILE_INTERVAL;
            }
        }
    }

    // Deliver Closed to every session still bound, so no session waits forever
    // for its event channel to fill.
    for registration in registrations.values() {
        registration.events.deliver(Event::Closed {
            reason: Some("the connection ended".to_owned()),
        });
    }
    cancellation.cancel();
}

/// The JSON payload of one SSE group, `None` for keep-alive and comment groups.
fn sse_payload(group: &str) -> Option<String> {
    let mut data = Vec::new();
    for line in group.lines() {
        if let Some(payload) = line.strip_prefix("data:") {
            data.push(payload.trim_start());
        }
    }
    if data.is_empty() {
        None
    } else {
        Some(data.join("\n"))
    }
}

/// Register `id` in the map with its modes, returning its receiver.
fn insert_registration(
    registrations: &mut HashMap<String, Registration>,
    id: String,
    directory: &str,
    modes: Modes,
) -> async_channel::Receiver<QueuedEvent> {
    let (mut registration, rx) = Registration::new();
    registration.modes = modes;
    registration.directory = directory.to_owned();
    registrations.insert(id, registration);
    rx
}

/// The agents offered in `directory` and the one the session described by `info` uses. A listing that fails leaves
/// the session without offered modes rather than failing it; reconciliation lists them again.
async fn modes(
    client: &Client<'_>,
    resolved: &Resolved,
    directory: &str,
    info: &Value,
    cancellation: &Cancellation,
) -> Modes {
    let offered = match api::list_agents(client, resolved, directory, cancellation).await {
        Ok(agents) => wire::modes_from_agents(&agents),
        Err(failure) => {
            eprintln!("listing the agents of {directory}: {}", failure.message);
            Vec::new()
        }
    };
    let current = wire::current_mode(info, &offered);
    Modes { offered, current }
}

/// List the agents of a session that offers none yet, and report them once there are some.
async fn list_modes(
    client: &Client<'_>,
    resolved: &Resolved,
    registration: &mut Registration,
    cancellation: &Cancellation,
) {
    if !registration.modes.offered.is_empty() || registration.listings == 0 {
        return;
    }
    registration.listings -= 1;
    let info = match &registration.modes.current {
        Some(agent) => serde_json::json!({ "agent": agent }),
        None => Value::Null,
    };
    let modes = modes(
        client,
        resolved,
        &registration.directory,
        &info,
        cancellation,
    )
    .await;
    if !modes.offered.is_empty() {
        registration.modes = modes;
        registration.publish_modes();
    }
}

/// Feed one complete SSE event to its session and to any parent that owns its
/// worker. Child events are authoritative; no paginated scan infers removal.
fn dispatch_events(event: &str, registrations: &mut HashMap<String, Registration>) {
    let Ok(value) = serde_json::from_str::<Value>(event) else {
        return;
    };
    apply_worker_event(&value, registrations);
    if let Some(session_id) = wire::session_id_of(&value)
        && let Some(registration) = registrations.get_mut(&session_id)
    {
        registration.event_revision = registration.event_revision.wrapping_add(1);
        if let Some(agent) = wire::agent_switched(&value, &session_id) {
            if registration.modes.current.as_deref() != Some(agent.as_str()) {
                registration.modes.current = Some(agent);
                registration.publish_modes();
            }
        } else if let Some(event) = wire::event_for(
            &value,
            &session_id,
            &mut registration
                .seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        ) {
            let running = match &event {
                Event::Busy => Some(RunningState::Running),
                Event::Idle => Some(RunningState::Idle),
                _ => None,
            };
            if registration.events.deliver(event)
                && let Some(running) = running
            {
                registration.running = running;
            }
        }
    }
    retry_worker_delivery(registrations);
}

/// Apply lifecycle facts to each registered ancestor that already knows the
/// relevant parent or child. A fork's parent is its source, not its parent.
fn apply_worker_event(value: &Value, registrations: &mut HashMap<String, Registration>) {
    let Some(kind) = value.get("type").and_then(Value::as_str) else {
        return;
    };
    if !matches!(
        kind,
        "session.created"
            | "session.deleted"
            | "session.renamed"
            | "session.execution.started"
            | "session.execution.succeeded"
            | "session.execution.failed"
            | "session.execution.interrupted"
    ) {
        return;
    }
    let Some(data) = value.get("data") else {
        return;
    };
    let Some(child) = data
        .get("sessionID")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return;
    };
    for (root, registration) in registrations {
        if !registration.workers_ready && !registration.children.contains_key(child) {
            let state = match kind {
                "session.execution.started" => Some(WorkerState::Running),
                "session.execution.succeeded" => Some(WorkerState::Completed),
                "session.execution.failed" => Some(WorkerState::Error),
                "session.execution.interrupted" => Some(WorkerState::Stopped),
                _ => None,
            };
            let title = (kind == "session.renamed")
                .then(|| data.get("title").and_then(Value::as_str))
                .flatten();
            if state.is_some() || title.is_some() {
                if registration.initial_changes.contains_key(child)
                    || registration.initial_changes.len() < api::MAX_TRACKED_WORKERS
                {
                    let change = registration
                        .initial_changes
                        .entry(child.to_owned())
                        .or_default();
                    if let Some(state) = state {
                        change.0 = Some(state);
                    }
                    if let Some(title) = title {
                        change.1 = Some(title.to_owned());
                    }
                } else {
                    registration.workers_incomplete = true;
                }
            }
        }
        let changed = match kind {
            "session.created" => {
                let parent = data.get("parentID").and_then(Value::as_str);
                if child == root || parent == Some(child) {
                    false
                } else if parent != Some(root.as_str())
                    && !parent.is_some_and(|id| registration.children.contains_key(id))
                {
                    if !registration.workers_ready && parent.is_some() {
                        if registration.initial_creations.contains_key(child)
                            || registration.initial_creations.len() < api::MAX_TRACKED_WORKERS
                        {
                            registration.initial_deletions.remove(child);
                            registration.initial_creations.insert(
                                child.to_owned(),
                                Worker {
                                    id: child.to_owned(),
                                    parent: parent.map(str::to_owned),
                                    title: data
                                        .get("title")
                                        .and_then(Value::as_str)
                                        .unwrap_or("worker")
                                        .to_owned(),
                                    state: WorkerState::Running,
                                },
                            );
                        } else {
                            registration.workers_incomplete = true;
                        }
                    }
                    false
                } else if registration.children.contains_key(child) {
                    false
                } else if registration.children.len() >= api::MAX_TRACKED_WORKERS {
                    registration.workers_incomplete = true;
                    registration.publish_workers();
                    false
                } else {
                    registration.initial_deletions.remove(child);
                    registration.initial_changes.remove(child);
                    registration.children.insert(
                        child.to_owned(),
                        Worker {
                            id: child.to_owned(),
                            parent: parent.map(str::to_owned),
                            title: data
                                .get("title")
                                .and_then(Value::as_str)
                                .unwrap_or("worker")
                                .to_owned(),
                            state: WorkerState::Running,
                        },
                    );
                    true
                }
            }
            "session.deleted" => {
                let removed = registration.children.remove(child).is_some();
                if !registration.workers_ready {
                    // A delete event has no parent ID. Before bootstrap, keep
                    // even an unknown ID: a stale page may later list it.
                    if registration.initial_deletions.len() < api::MAX_TRACKED_WORKERS {
                        registration.initial_deletions.insert(child.to_owned());
                    } else {
                        registration.workers_incomplete = true;
                    }
                }
                registration.initial_changes.remove(child);
                registration.initial_creations.remove(child);
                removed
            }
            "session.renamed" => match (
                registration.children.get_mut(child),
                data.get("title").and_then(Value::as_str),
            ) {
                (Some(worker), Some(title)) if worker.title != title => {
                    worker.title = title.to_owned();
                    true
                }
                _ => false,
            },
            _ => match registration.children.get_mut(child) {
                Some(worker) => {
                    let state = match kind {
                        "session.execution.started" => WorkerState::Running,
                        "session.execution.succeeded" => WorkerState::Completed,
                        "session.execution.failed" => WorkerState::Error,
                        "session.execution.interrupted" => WorkerState::Stopped,
                        _ => unreachable!(),
                    };
                    if worker.state == state {
                        false
                    } else {
                        worker.state = state;
                        true
                    }
                }
                None => false,
            },
        };
        if changed {
            registration.publish_workers();
        }
    }
}

fn retry_worker_delivery(registrations: &mut HashMap<String, Registration>) {
    for registration in registrations.values_mut() {
        if registration.workers_dirty {
            registration.publish_workers();
        }
        if registration.modes_dirty {
            registration.publish_modes();
        }
    }
}

fn close_registration(registrations: &mut HashMap<String, Registration>, id: &str) {
    if let Some(registration) = registrations.remove(id) {
        if let Some(cancellation) = registration.snapshot_in_flight {
            cancellation.cancel();
        }
        if let Some(cancellation) = registration.reconcile_in_flight {
            cancellation.cancel();
        }
        registration.events.deliver(Event::Closed { reason: None });
    }
}

/// Service one command, returning whether the connection should stay open.
async fn handle_command(
    command: Command,
    client: &Client<'_>,
    resolved: &Resolved,
    registrations: &mut HashMap<String, Registration>,
    cancellation: &Cancellation,
) -> bool {
    match command {
        Command::Close { id } => {
            close_registration(registrations, &id);
            // Keep the shared connection available for other registrations and
            // for a later retry after the last one closes.
            true
        }
        Command::Sessions { directory, reply } => {
            let result = api::list_sessions(client, resolved, &directory, cancellation)
                .await
                .map(|sessions| {
                    sessions
                        .iter()
                        .filter_map(|info| {
                            Some(SessionSummary {
                                id: info.get("id")?.as_str()?.to_owned(),
                                title: info
                                    .get("title")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_owned(),
                                directory: info
                                    .get("location")
                                    .and_then(|location| location.get("directory"))
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_owned(),
                            })
                        })
                        .collect()
                })
                .map_err(map_failure);
            reply.send(result).await.ok();
            true
        }
        Command::Create { spec, reply } => {
            let result = match api::create_session(
                client,
                resolved,
                &spec.directory,
                Some(&spec.title),
                cancellation,
            )
            .await
            {
                Ok(info) => match info.get("id").and_then(Value::as_str) {
                    Some(id) => {
                        let modes =
                            modes(client, resolved, &spec.directory, &info, cancellation).await;
                        let rx = insert_registration(
                            registrations,
                            id.to_owned(),
                            &spec.directory,
                            modes,
                        );
                        Ok((id.to_owned(), rx))
                    }
                    None => Err(Failure {
                        status: None,
                        message: "the server did not return the new session's id".to_owned(),
                    }),
                },
                Err(failure) => Err(failure),
            }
            .map_err(map_failure);
            reply.send(result).await.ok();
            true
        }
        Command::Attach {
            id,
            directory,
            reply,
        } => {
            let result = match api::session_info(client, resolved, &id, cancellation).await {
                Ok(info) => {
                    let session_directory = info
                        .get("location")
                        .and_then(|location| location.get("directory"))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if session_directory == directory {
                        let modes = modes(client, resolved, &directory, &info, cancellation).await;
                        Ok(insert_registration(
                            registrations,
                            id.clone(),
                            &directory,
                            modes,
                        ))
                    } else {
                        Err(Failure {
                            status: Some(404),
                            message: "the session is not in that directory".to_owned(),
                        })
                    }
                }
                Err(failure) => Err(failure),
            }
            .map_err(map_failure);
            reply.send(result).await.ok();
            true
        }
        Command::Snapshot { .. } => unreachable!("snapshots are scheduled by the service loop"),
        Command::Prompt {
            id,
            key,
            text,
            reply,
        } => {
            let result = api::send_prompt(client, resolved, &id, &key, &text, cancellation)
                .await
                .map_err(map_failure);
            reply.send(result).await.ok();
            true
        }
        Command::SetMode { id, mode, reply } => {
            // OpenCode accepts any agent name, so only an offered one is passed on. The mode changes when the server
            // confirms the switch on the event stream.
            if let Some(registration) = registrations.get_mut(&id) {
                list_modes(client, resolved, registration, cancellation).await;
            }
            let offered = registrations.get(&id).is_some_and(|registration| {
                registration
                    .modes
                    .offered
                    .iter()
                    .any(|offered| offered.id == mode)
            });
            let result = if offered {
                api::switch_agent(client, resolved, &id, &mode, cancellation)
                    .await
                    .map_err(map_failure)
            } else {
                Err(backend::Error::new(
                    backend::ErrorKind::Invalid,
                    format!("the session does not offer the {mode} agent"),
                ))
            };
            reply.send(result).await.ok();
            true
        }
        Command::Interrupt { id, reply } => {
            let result = api::interrupt(client, resolved, &id, cancellation)
                .await
                .map_err(map_failure);
            reply.send(result).await.ok();
            true
        }
        Command::Decide {
            id,
            request,
            decision,
            reply,
        } => {
            let result =
                api::reply_permission(client, resolved, &id, &request, &decision, cancellation)
                    .await
                    .map_err(map_failure);
            reply.send(result).await.ok();
            true
        }
        Command::Answer {
            id,
            form,
            answer,
            reply,
        } => {
            let result = api::reply_form(client, resolved, &id, &form, &answer, cancellation)
                .await
                .map_err(map_failure);
            reply.send(result).await.ok();
            true
        }
    }
}

async fn snapshot(
    client: &Client<'_>,
    resolved: &Resolved,
    id: &str,
    modes: Modes,
    cancellation: &Cancellation,
) -> std::result::Result<Snapshot, Failure> {
    let parts = api::SnapshotParts::fetch(client, resolved, id, cancellation).await?;
    let messages = snapshot_messages(&parts.messages);
    Ok(Snapshot {
        modes,
        busy: Some(parts.busy),
        messages,
        permission: parts.permission(),
        question: parts.question(),
        workers: parts.workers(),
    })
}

/// The session's message list as snapshot messages.
fn snapshot_messages(value: &Value) -> Vec<backend::SnapshotMessage> {
    let Some(messages) = value
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| value.as_array())
    else {
        return Vec::new();
    };
    messages.iter().filter_map(snapshot_message).collect()
}

fn snapshot_message(value: &Value) -> Option<backend::SnapshotMessage> {
    let id = value.get("id")?.as_str()?.to_owned();
    let role = match value.get("type")?.as_str()? {
        "user" => backend::Role::User,
        "assistant" => backend::Role::Assistant,
        _ => return None,
    };
    let parts = match role {
        backend::Role::User => vec![backend::Part::Text {
            text: value
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        }],
        backend::Role::Assistant => value
            .get("content")
            .and_then(Value::as_array)
            .map(|content| {
                content
                    .iter()
                    .filter_map(|part| match part.get("type")?.as_str()? {
                        "text" => Some(backend::Part::Text {
                            text: part
                                .get("text")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                        }),
                        "reasoning" => Some(backend::Part::Reasoning {
                            text: part
                                .get("text")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                        }),
                        "tool" => Some(backend::Part::Tool {
                            id: part
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            tool: part
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            input: part
                                .get("state")
                                .and_then(|state| state.get("input"))
                                .cloned()
                                .unwrap_or(Value::Null),
                            finished: matches!(
                                part.get("state")
                                    .and_then(|state| state.get("status"))
                                    .and_then(Value::as_str),
                                Some("completed") | Some("error")
                            ),
                        }),
                        _ => None,
                    })
                    .collect::<Vec<backend::Part>>()
            })
            .unwrap_or_default(),
    };
    let completed = match role {
        backend::Role::User => true,
        backend::Role::Assistant => value
            .get("time")
            .and_then(|time| time.get("completed"))
            .is_some(),
    };
    Some(backend::SnapshotMessage {
        id,
        role,
        parts,
        completed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use backend::WorkerState;
    use serde_json::json;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    fn registrations() -> HashMap<String, Registration> {
        let mut registrations = HashMap::new();
        insert_registration(
            &mut registrations,
            "parent".into(),
            "/tmp",
            Modes::default(),
        );
        insert_registration(&mut registrations, "other".into(), "/tmp", Modes::default());
        registrations
    }

    #[test]
    fn child_creation_updates_its_parent_not_other_spaces() {
        let mut registrations = registrations();
        let event = json!({
            "type": "session.created",
            "data": { "sessionID": "child", "parentID": "parent" }
        });
        apply_worker_event(&event, &mut registrations);
        assert!(registrations["parent"].children.contains_key("child"));
        assert!(registrations["other"].children.is_empty());
        assert!(wire::event_for(&event, "parent", &mut HashSet::new()).is_none());
    }

    #[test]
    fn completion_and_resumption_do_not_remove_the_child_but_deletion_does() {
        let mut registrations = registrations();
        registrations.get_mut("parent").unwrap().children.insert(
            "child".into(),
            Worker {
                id: "child".into(),
                parent: Some("parent".into()),
                title: "Worker".into(),
                state: WorkerState::Running,
            },
        );
        for (kind, expected) in [
            ("session.execution.succeeded", WorkerState::Completed),
            ("session.execution.started", WorkerState::Running),
            ("session.execution.failed", WorkerState::Error),
            ("session.execution.interrupted", WorkerState::Stopped),
        ] {
            let event = json!({"type": kind, "data": {"sessionID": "child"}});
            apply_worker_event(&event, &mut registrations);
            assert_eq!(registrations["parent"].children["child"].state, expected);
        }
        let deleted = json!({"type": "session.deleted", "data": {"sessionID": "child"}});
        apply_worker_event(&deleted, &mut registrations);
        assert!(!registrations["parent"].children.contains_key("child"));
        apply_worker_event(&deleted, &mut registrations);
        let unrelated = json!({"type": "session.deleted", "data": {"sessionID": "unrelated"}});
        apply_worker_event(&unrelated, &mut registrations);
        assert!(registrations["other"].children.is_empty());
    }

    #[test]
    fn ordinary_child_messages_do_not_change_workers() {
        let mut registrations = registrations();
        let created = json!({"type": "session.created", "data": {"sessionID": "child", "parentID": "parent"}});
        apply_worker_event(&created, &mut registrations);
        let before = registrations["parent"].children.clone();
        let event = json!({"type": "session.text.delta", "data": {"sessionID": "child"}});
        apply_worker_event(&event, &mut registrations);
        assert_eq!(registrations["parent"].children, before);
    }

    #[test]
    fn a_full_event_channel_retries_the_worker_snapshot() {
        let (mut registration, events) = Registration::new();
        registration.workers_ready = true;
        for _ in 0..1024 {
            registration.events.deliver(Event::Busy);
        }
        registration.publish_workers();
        assert!(registration.workers_dirty);
        while events.try_recv().is_ok() {}
        registration.publish_workers();
        assert!(!registration.workers_dirty);
        assert!(
            matches!(events.try_recv().ok().and_then(QueuedEvent::into_current), Some(Event::WorkersChanged { workers }) if workers.is_empty())
        );
    }

    #[test]
    fn snapshot_invalidates_only_older_worker_events() {
        let (mut registration, events) = Registration::new();
        let worker = Worker {
            id: "child".into(),
            parent: Some("parent".into()),
            title: "Worker".into(),
            state: WorkerState::Completed,
        };
        let mut initial = vec![worker];
        registration.install_workers(&mut initial).unwrap();
        registration.publish_workers();
        registration.events.deliver(Event::Busy);
        registration.children.remove("child");
        let mut later = Vec::new();
        registration.install_workers(&mut later).unwrap();
        assert!(later.is_empty());
        assert!(events.try_recv().unwrap().into_current().is_none());
        assert!(matches!(
            events.try_recv().unwrap().into_current(),
            Some(Event::Busy)
        ));
    }

    #[test]
    fn session_consumer_skips_a_superseded_worker_event() {
        use backend::Session;
        let (events, rx) = Events::new();
        let (commands, _receiver) = Commands::new();
        let session = crate::session::OpenCodeSession::new("parent".into(), rx, commands);
        assert!(events.try_deliver_workers(Vec::new()));
        events.deliver(Event::Busy);
        events.advance_workers();
        assert!(matches!(block_on(session.next_event()), Ok(Event::Busy)));
    }

    #[test]
    fn a_later_snapshot_cannot_reintroduce_a_deleted_worker() {
        let (mut registration, _events) = Registration::new();
        let child = Worker {
            id: "child".into(),
            parent: Some("parent".into()),
            title: "Worker".into(),
            state: WorkerState::Completed,
        };
        let mut initial = vec![child.clone()];
        registration.install_workers(&mut initial).unwrap();
        registration.children.remove("child");
        let mut stale = vec![child];
        registration.install_workers(&mut stale).unwrap();
        assert!(stale.is_empty());
    }

    #[test]
    fn an_absent_snapshot_worker_is_not_inferred_deleted() {
        let (mut registration, _events) = Registration::new();
        let child = Worker {
            id: "child".into(),
            parent: Some("parent".into()),
            title: "Worker".into(),
            state: WorkerState::Running,
        };
        let mut initial = vec![child.clone()];
        registration.install_workers(&mut initial).unwrap();
        let mut incomplete = Vec::new();
        registration.install_workers(&mut incomplete).unwrap();
        assert_eq!(incomplete, [child]);
    }

    #[test]
    fn a_deletion_before_snapshot_prevents_stale_snapshot_resurrection() {
        let mut registrations = registrations();
        let created =
            json!({"type":"session.created","data":{"sessionID":"child","parentID":"parent"}});
        let deleted = json!({"type":"session.deleted","data":{"sessionID":"child"}});
        apply_worker_event(&created, &mut registrations);
        apply_worker_event(&deleted, &mut registrations);
        let mut stale = vec![Worker {
            id: "child".into(),
            parent: Some("parent".into()),
            title: "old".into(),
            state: WorkerState::Completed,
        }];
        registrations
            .get_mut("parent")
            .unwrap()
            .install_workers(&mut stale)
            .unwrap();
        assert!(stale.is_empty());
    }

    #[test]
    fn an_unknown_deletion_before_snapshot_is_not_resurrected() {
        let mut registrations = registrations();
        let deleted = json!({"type":"session.deleted","data":{"sessionID":"child"}});
        apply_worker_event(&deleted, &mut registrations);
        let mut stale = vec![Worker {
            id: "child".into(),
            parent: Some("parent".into()),
            title: "old".into(),
            state: WorkerState::Completed,
        }];
        registrations
            .get_mut("parent")
            .unwrap()
            .install_workers(&mut stale)
            .unwrap();
        assert!(stale.is_empty());
    }

    #[test]
    fn events_for_a_preexisting_child_survive_initial_discovery() {
        let mut registrations = registrations();
        for event in [
            json!({"type":"session.execution.succeeded","data":{"sessionID":"child"}}),
            json!({"type":"session.execution.started","data":{"sessionID":"child"}}),
            json!({"type":"session.renamed","data":{"sessionID":"child","title":"New title"}}),
        ] {
            apply_worker_event(&event, &mut registrations);
        }
        let mut stale = vec![Worker {
            id: "child".into(),
            parent: Some("parent".into()),
            title: "old".into(),
            state: WorkerState::Completed,
        }];
        registrations
            .get_mut("parent")
            .unwrap()
            .install_workers(&mut stale)
            .unwrap();
        assert_eq!(stale[0].state, WorkerState::Running);
        assert_eq!(stale[0].title, "New title");
    }

    #[test]
    fn a_nested_child_created_before_its_parent_is_discovered_is_not_lost() {
        let mut registrations = registrations();
        for event in [
            json!({"type":"session.created","data":{"sessionID":"nested","parentID":"child","title":"Nested"}}),
            json!({"type":"session.execution.succeeded","data":{"sessionID":"nested"}}),
        ] {
            apply_worker_event(&event, &mut registrations);
        }
        let mut snapshot = vec![Worker {
            id: "child".into(),
            parent: Some("parent".into()),
            title: "Child".into(),
            state: WorkerState::Running,
        }];
        registrations
            .get_mut("parent")
            .unwrap()
            .install_workers(&mut snapshot)
            .unwrap();
        let nested = snapshot
            .iter()
            .find(|worker| worker.id == "nested")
            .unwrap();
        assert_eq!(nested.parent.as_deref(), Some("child"));
        assert_eq!(nested.state, WorkerState::Completed);
    }

    #[test]
    fn a_pending_nested_deletion_is_not_recreated_by_bootstrap() {
        let mut registrations = registrations();
        for event in [
            json!({"type":"session.created","data":{"sessionID":"nested","parentID":"child"}}),
            json!({"type":"session.deleted","data":{"sessionID":"nested"}}),
        ] {
            apply_worker_event(&event, &mut registrations);
        }
        let mut snapshot = vec![
            Worker {
                id: "child".into(),
                parent: Some("parent".into()),
                title: "Child".into(),
                state: WorkerState::Running,
            },
            Worker {
                id: "nested".into(),
                parent: Some("child".into()),
                title: "old".into(),
                state: WorkerState::Running,
            },
        ];
        registrations
            .get_mut("parent")
            .unwrap()
            .install_workers(&mut snapshot)
            .unwrap();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].id, "child");
    }

    #[test]
    fn observed_child_deletion_publishes_removal_without_a_network_scan() {
        let mut registrations = HashMap::new();
        let events = insert_registration(
            &mut registrations,
            "parent".into(),
            "/tmp",
            Modes::default(),
        );
        let mut snapshot = vec![Worker {
            id: "child".into(),
            parent: Some("parent".into()),
            title: "Worker".into(),
            state: WorkerState::Completed,
        }];
        registrations
            .get_mut("parent")
            .unwrap()
            .install_workers(&mut snapshot)
            .unwrap();
        let deleted = json!({"type":"session.deleted","data":{"sessionID":"child"}}).to_string();
        dispatch_events(&deleted, &mut registrations);
        assert!(
            matches!(events.try_recv().unwrap().into_current(), Some(Event::WorkersChanged { workers }) if workers.is_empty())
        );
    }

    #[test]
    fn parent_agent_switches_still_reach_the_session() {
        let mut registrations = HashMap::new();
        let events = insert_registration(
            &mut registrations,
            "parent".into(),
            "/tmp",
            Modes::default(),
        );
        let switched =
            json!({"type":"session.agent.selected","data":{"sessionID":"parent","agent":"plan"}})
                .to_string();
        dispatch_events(&switched, &mut registrations);
        assert!(
            matches!(events.try_recv().unwrap().into_current(), Some(Event::ModesChanged(modes)) if modes.current.as_deref() == Some("plan"))
        );
    }

    #[test]
    fn closing_a_registration_leaves_other_sessions_open() {
        let mut registrations = HashMap::new();
        let first =
            insert_registration(&mut registrations, "first".into(), "/tmp", Modes::default());
        let second = insert_registration(
            &mut registrations,
            "second".into(),
            "/tmp",
            Modes::default(),
        );
        close_registration(&mut registrations, "first");
        assert!(matches!(
            first.try_recv().unwrap().into_current(),
            Some(Event::Closed { .. })
        ));
        assert!(second.try_recv().is_err());
        assert!(registrations.contains_key("second"));
        close_registration(&mut registrations, "second");
        assert!(registrations.is_empty());
        assert!(matches!(
            second.try_recv().unwrap().into_current(),
            Some(Event::Closed { .. })
        ));
    }

    #[test]
    fn stalled_snapshot_does_not_block_another_space_or_its_events() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::mpsc;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let server_stopped = stopped.clone();
        let stall_reconcile = Arc::new(AtomicBool::new(false));
        let server_stall_reconcile = stall_reconcile.clone();
        let (event_tx, event_rx) = mpsc::channel::<String>();
        let event_rx = Arc::new(Mutex::new(event_rx));
        let (stalled_tx, stalled_rx) = mpsc::channel();
        let (reconcile_tx, reconcile_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            while !server_stopped.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        let stopped = server_stopped.clone();
                        let stall_reconcile = server_stall_reconcile.clone();
                        let event_rx = event_rx.clone();
                        let stalled = stalled_tx.clone();
                        let reconcile = reconcile_tx.clone();
                        std::thread::spawn(move || {
                            socket
                                .set_read_timeout(Some(Duration::from_secs(3)))
                                .unwrap();
                            let mut reader = BufReader::new(socket.try_clone().unwrap());
                            let mut line = String::new();
                            if reader.read_line(&mut line).is_err() {
                                return;
                            }
                            let path = line.split_whitespace().nth(1).unwrap_or("").to_owned();
                            loop {
                                line.clear();
                                if reader.read_line(&mut line).is_err()
                                    || line == "\r\n"
                                    || line.is_empty()
                                {
                                    break;
                                }
                            }
                            if path == "/api/event" {
                                let _ = write!(
                                    socket,
                                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n"
                                );
                                while !stopped.load(Ordering::Acquire) {
                                    match event_rx
                                        .lock()
                                        .unwrap()
                                        .recv_timeout(Duration::from_millis(50))
                                    {
                                        Ok(event) => {
                                            let data = format!("data: {event}\n\n");
                                            if write!(socket, "{:x}\r\n{}\r\n", data.len(), data)
                                                .and_then(|_| socket.flush())
                                                .is_err()
                                            {
                                                break;
                                            }
                                        }
                                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                                    }
                                }
                            } else if path.starts_with("/api/session/slow/message?") {
                                let _ = write!(
                                    socket,
                                    "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{{"
                                );
                                let _ = socket.flush();
                                let _ = stalled.send(());
                                while !stopped.load(Ordering::Acquire) {
                                    std::thread::sleep(Duration::from_millis(20));
                                }
                            } else if path == "/api/session/slow"
                                && stall_reconcile.load(Ordering::Acquire)
                            {
                                let _ = write!(
                                    socket,
                                    "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{{"
                                );
                                let _ = socket.flush();
                                let _ = reconcile.send(());
                                while !stopped.load(Ordering::Acquire) {
                                    std::thread::sleep(Duration::from_millis(20));
                                }
                            } else {
                                let body = if path.starts_with("/api/agent?") {
                                    r#"{"data":[]}"#.to_owned()
                                } else if path.starts_with("/api/session?parentID=family&")
                                    || path.starts_with("/api/session?parentID=child&")
                                {
                                    let (id, parent) = if path.contains("parentID=family&") {
                                        ("child", "family")
                                    } else {
                                        ("grandchild", "child")
                                    };
                                    json!({"data":[{"id":id,"parentID":parent,"projectID":"p","cost":0,"tokens":{"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}},"time":{"created":1,"updated":1},"location":{"directory":"/tmp"}}],"cursor":{"next":null}}).to_string()
                                } else if path.starts_with("/api/session?parentID=") {
                                    r#"{"data":[],"cursor":{"next":null}}"#.to_owned()
                                } else if path == "/api/session/fast/message?limit=100&order=asc" {
                                    r#"{"data":[{"id":"first","type":"user","text":"First"}],"cursor":{"next":"next+page"}}"#.to_owned()
                                } else if path
                                    == "/api/session/fast/message?limit=100&cursor=next%2Bpage"
                                {
                                    r#"{"data":[{"id":"second","type":"user","text":"Second"}],"cursor":{"next":null}}"#.to_owned()
                                } else if path.starts_with("/api/session/duplicate/message?") {
                                    r#"{"data":[{"id":"same","type":"user"},{"id":"same","type":"user"}],"cursor":{"next":null}}"#.to_owned()
                                } else if path.contains("/message?") {
                                    r#"{"data":[],"cursor":{"next":null}}"#.to_owned()
                                } else if path == "/api/session/bad-permission/permission"
                                    || path == "/api/session/bad-form/form"
                                {
                                    r#"{"invalid":[]}"#.to_owned()
                                } else if path.ends_with("/permission") || path.ends_with("/form") {
                                    r#"{"data":[]}"#.to_owned()
                                } else if path.starts_with("/api/session/") {
                                    let id = path.trim_start_matches("/api/session/");
                                    json!({"data":{"id":id,"location":{"directory":"/tmp"},"time":{"busy":0,"idle":0}}}).to_string()
                                } else {
                                    panic!("unexpected path: {path}");
                                };
                                let _ = write!(
                                    socket,
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                    body.len(),
                                    body
                                );
                            }
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            }
        });
        let (commands, receiver) = Commands::new();
        let resolved = crate::endpoint::resolve(Some(&format!("http://{address}")));
        let connection = std::thread::spawn(move || run_thread(resolved, receiver));
        let attach = |id: &str| {
            let id = id.to_owned();
            block_on(commands.reply_with(|reply| Command::Attach {
                id,
                directory: "/tmp".into(),
                reply,
            }))
            .unwrap()
        };
        let slow_events = attach("slow");
        let fast_events = attach("fast");
        for id in ["bad-permission", "bad-form", "duplicate"] {
            let _events = attach(id);
            let (reply, waiting) = async_channel::bounded(1);
            commands
                .tx()
                .try_send(Command::Snapshot {
                    id: id.into(),
                    reply,
                })
                .unwrap();
            assert!(
                block_on(waiting.recv()).unwrap().is_err(),
                "{id} snapshot should fail closed"
            );
            commands
                .tx()
                .try_send(Command::Close { id: id.into() })
                .unwrap();
        }
        let _family_events = attach("family");
        let (family_reply, family_wait) = async_channel::bounded(1);
        commands
            .tx()
            .try_send(Command::Snapshot {
                id: "family".into(),
                reply: family_reply,
            })
            .unwrap();
        let family = block_on(family_wait.recv()).unwrap().unwrap();
        assert_eq!(family.workers.len(), 2);
        assert_eq!(
            family
                .workers
                .iter()
                .find(|worker| worker.id == "grandchild")
                .unwrap()
                .parent
                .as_deref(),
            Some("child")
        );
        commands
            .tx()
            .try_send(Command::Close {
                id: "family".into(),
            })
            .unwrap();
        let (fast_reply, fast_wait) = async_channel::bounded(1);
        commands
            .tx()
            .try_send(Command::Snapshot {
                id: "fast".into(),
                reply: fast_reply,
            })
            .unwrap();
        let initial_fast = block_on(fast_wait.recv()).unwrap().unwrap();
        assert_eq!(
            initial_fast
                .messages
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        let (slow_reply, slow_wait) = async_channel::bounded(1);
        commands
            .tx()
            .try_send(Command::Snapshot {
                id: "slow".into(),
                reply: slow_reply,
            })
            .unwrap();
        stalled_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("snapshot body did not stall");
        stall_reconcile.store(true, Ordering::Release);
        reconcile_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("the periodic state response did not stall");
        event_tx
            .send(
                json!({"type":"session.created","data":{"sessionID":"child","parentID":"fast"}})
                    .to_string(),
            )
            .unwrap();
        let fast_rx = fast_events.clone();
        let (received_tx, received_rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(event) = block_on(fast_rx.recv()) {
                if let Some(event @ Event::WorkersChanged { .. }) = event.into_current() {
                    let _ = received_tx.send(Some(event));
                    return;
                }
            }
            let _ = received_tx.send(None);
        });
        let received = received_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("the other space was blocked");
        assert!(
            matches!(received, Some(Event::WorkersChanged { workers }) if workers.iter().any(|worker| worker.id == "child"))
        );
        let (fast_reply, fast_wait) = async_channel::bounded(1);
        commands
            .tx()
            .try_send(Command::Snapshot {
                id: "fast".into(),
                reply: fast_reply,
            })
            .unwrap();
        let (fast_tx, fast_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = fast_tx.send(block_on(fast_wait.recv()));
        });
        let fast_snapshot = fast_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("the other snapshot was blocked")
            .unwrap()
            .unwrap();
        assert!(
            fast_snapshot
                .workers
                .iter()
                .any(|worker| worker.id == "child")
        );
        commands
            .tx()
            .try_send(Command::Close { id: "slow".into() })
            .unwrap();
        assert!(block_on(slow_wait.recv()).unwrap().is_err());
        assert!(matches!(
            slow_events.try_recv().unwrap().into_current(),
            Some(Event::Closed { .. })
        ));
        stall_reconcile.store(false, Ordering::Release);
        event_tx
            .send(json!({"type":"session.deleted","data":{"sessionID":"child"}}).to_string())
            .unwrap();
        let (deleted_tx, deleted_rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(event) = block_on(fast_events.recv()) {
                if let Some(Event::WorkersChanged { workers }) = event.into_current() {
                    let _ = deleted_tx.send(workers);
                    return;
                }
            }
        });
        assert!(
            deleted_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("the deletion was blocked")
                .is_empty()
        );
        // A later registration of the same ID gets an independent deadline.
        // A stalled body that is not explicitly closed must still time out.
        let _retried_events = attach("slow");
        let (slow_reply, slow_wait) = async_channel::bounded(1);
        commands
            .tx()
            .try_send(Command::Snapshot {
                id: "slow".into(),
                reply: slow_reply,
            })
            .unwrap();
        stalled_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("retry did not reach the stalled body");
        let (timeout_tx, timeout_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = timeout_tx.send(block_on(slow_wait.recv()));
        });
        assert!(
            timeout_rx
                .recv_timeout(SNAPSHOT_TIMEOUT + Duration::from_secs(2))
                .expect("snapshot timeout did not fire")
                .unwrap()
                .is_err()
        );
        drop(commands);
        connection.join().unwrap();
        stopped.store(true, Ordering::Release);
        drop(event_tx);
        server.join().unwrap();
    }

    #[test]
    fn fork_events_do_not_fabricate_child_workers() {
        let mut registrations = registrations();
        let forked =
            json!({"type":"session.forked","data":{"sessionID":"fork","parentID":"parent"}});
        apply_worker_event(&forked, &mut registrations);
        assert!(registrations["parent"].children.is_empty());
    }

    #[test]
    fn nested_worker_events_keep_their_parent_and_do_not_affect_other_spaces() {
        let mut registrations = registrations();
        for (child, parent) in [("a", "parent"), ("b", "a")] {
            let created =
                json!({"type":"session.created","data":{"sessionID":child,"parentID":parent}});
            apply_worker_event(&created, &mut registrations);
        }
        assert_eq!(
            registrations["parent"].children["b"].parent.as_deref(),
            Some("a")
        );
        let deleted = json!({"type":"session.deleted","data":{"sessionID":"b"}});
        apply_worker_event(&deleted, &mut registrations);
        assert!(registrations["parent"].children.contains_key("a"));
        assert!(!registrations["parent"].children.contains_key("b"));
        assert!(registrations["other"].children.is_empty());
    }

    #[test]
    fn worker_cache_overflow_is_visible_and_never_publishes_a_partial_list_as_complete() {
        let mut registrations = HashMap::new();
        let events = insert_registration(
            &mut registrations,
            "parent".into(),
            "/tmp",
            Modes::default(),
        );
        let registration = registrations.get_mut("parent").unwrap();
        registration.workers_ready = true;
        for index in 0..api::MAX_TRACKED_WORKERS {
            let id = format!("worker-{index}");
            registration.children.insert(
                id.clone(),
                Worker {
                    id,
                    parent: Some("parent".into()),
                    title: "Worker".into(),
                    state: WorkerState::Running,
                },
            );
        }
        let event =
            json!({"type":"session.created","data":{"sessionID":"overflow","parentID":"parent"}});
        apply_worker_event(&event, &mut registrations);
        assert!(registrations["parent"].workers_incomplete);
        assert!(!registrations["parent"].children.contains_key("overflow"));
        assert!(matches!(
            events.try_recv().unwrap().into_current(),
            Some(Event::Error { .. })
        ));
        let deleted = json!({"type":"session.deleted","data":{"sessionID":"worker-1"}});
        apply_worker_event(&deleted, &mut registrations);
        assert!(matches!(
            events.try_recv().unwrap().into_current(),
            Some(Event::Error { .. })
        ));
    }

    #[test]
    fn child_list_rejects_failed_and_malformed_responses() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for attempt in 0..5 {
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                let (mut socket, _) = loop {
                    match listener.accept() {
                        Ok(accepted) => break accepted,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(std::time::Instant::now() < deadline, "no refresh request");
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("accept: {error}"),
                    }
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let expected = "GET /api/session?parentID=parent&limit=100 ";
                assert!(line.starts_with(expected), "{line}");
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                let (status, body) = match attempt {
                    0 => ("503 Service Unavailable", r#"{"error":"temporary"}"#),
                    1 => ("200 OK", r#"{"not_a_child_list":[]}"#),
                    2 => ("200 OK", r#"{"data":[]}"#),
                    3 => ("200 OK", r#"{"data":[{}],"cursor":{"next":null}}"#),
                    _ => ("200 OK", r#"{"data":[],"cursor":{"next":null}}"#),
                };
                write!(socket, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        block_on(async {
            let scheduler = DirectScheduler::new(DirectOptions {
                threads: 2,
                capacity: 256,
            })
            .unwrap();
            let cancellation = Cancellation::new();
            let address = scheduler
                .address_resolve(
                    Some("127.0.0.1"),
                    AddressService::Port(address.port()),
                    AddressHints::default(),
                    &cancellation,
                )
                .await
                .unwrap()
                .entries()
                .next()
                .unwrap()
                .address();
            let (client, driver) = Client::new(
                &scheduler,
                Endpoint::new(address),
                base_http_client::Options::default(),
            )
            .unwrap();
            let resolved = Resolved {
                host: "127.0.0.1".into(),
                port: address.port(),
                auth: None,
            };
            let (_driver_result, ()) =
                futures_lite::future::zip(driver.run(&cancellation), async {
                    for _ in 0..4 {
                        assert!(
                            api::child_sessions(&client, &resolved, "parent", &cancellation)
                                .await
                                .is_err()
                        );
                    }
                    assert!(
                        api::child_sessions(&client, &resolved, "parent", &cancellation)
                            .await
                            .unwrap()
                            .is_empty()
                    );
                    cancellation.cancel();
                })
                .await;
            drop(client);
            scheduler.shutdown().await;
        });
        server.join().unwrap();
    }

    #[test]
    fn child_list_reads_every_page() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for attempt in 0..2 {
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                let (mut socket, _) = loop {
                    match listener.accept() {
                        Ok(accepted) => break accepted,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(std::time::Instant::now() < deadline, "no page request");
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("accept: {error}"),
                    }
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let expected = if attempt == 0 {
                    "GET /api/session?parentID=parent&limit=100 "
                } else {
                    "GET /api/session?parentID=parent&limit=100&cursor=next%2Bpage "
                };
                assert!(line.starts_with(expected), "{line}");
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                let body = if attempt == 0 {
                    r#"{"data":[{"id":"first","parentID":"parent","projectID":"p","cost":0,"tokens":{"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}},"time":{"created":1,"updated":1},"location":{"directory":"/tmp"}}],"cursor":{"next":"next+page"}}"#
                } else {
                    r#"{"data":[{"id":"second","parentID":"parent","projectID":"p","cost":0,"tokens":{"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}},"time":{"created":1,"updated":1},"location":{"directory":"/tmp"}}],"cursor":{"next":null}}"#
                };
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        block_on(async {
            let scheduler = DirectScheduler::new(DirectOptions {
                threads: 2,
                capacity: 256,
            })
            .unwrap();
            let cancellation = Cancellation::new();
            let resolved_address = scheduler
                .address_resolve(
                    Some("127.0.0.1"),
                    AddressService::Port(address.port()),
                    AddressHints::default(),
                    &cancellation,
                )
                .await
                .unwrap()
                .entries()
                .next()
                .unwrap()
                .address();
            let (client, driver) = Client::new(
                &scheduler,
                Endpoint::new(resolved_address),
                base_http_client::Options::default(),
            )
            .unwrap();
            let resolved = Resolved {
                host: "127.0.0.1".into(),
                port: address.port(),
                auth: None,
            };
            let (_driver_result, ()) =
                futures_lite::future::zip(driver.run(&cancellation), async {
                    let children = api::child_sessions(&client, &resolved, "parent", &cancellation)
                        .await
                        .unwrap();
                    assert_eq!(
                        children
                            .iter()
                            .map(|worker| worker["id"].as_str().unwrap())
                            .collect::<Vec<_>>(),
                        ["first", "second"]
                    );
                    cancellation.cancel();
                })
                .await;
            drop(client);
            scheduler.shutdown().await;
        });
        server.join().unwrap();
    }
}
