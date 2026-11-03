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
//!
//! Registrations outlive any one connection. When the stream ends or fails,
//! as when the service restarts, the thread resolves the service again and
//! reconnects with a bounded backoff; commands meanwhile wait for the next
//! attempt. The stream does not replay what was missed, so each session whose
//! state had been read is read again once the stream reopens and presented
//! with [`Event::Resynchronized`]; one that cannot be is closed with the reason.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
use base_scheduler_native::{NativeOptions, NativeScheduler};
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
/// Bound worker identities without changing them, and bound display titles
/// with a visible truncation marker.
const MAX_WORKER_ID_BYTES: usize = 1024;
const MAX_WORKER_TITLE_BYTES: usize = 1024;

fn bounded_worker_title(title: &str) -> String {
    if title.len() <= MAX_WORKER_TITLE_BYTES {
        return title.to_owned();
    }
    let mut end = MAX_WORKER_TITLE_BYTES - '…'.len_utf8();
    while !title.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &title[..end])
}

/// Refuse listed workers whose identities exceed the supported size, and bound
/// their titles.
fn validate_workers(workers: &mut [Worker]) -> backend::Result<()> {
    for worker in workers.iter_mut() {
        if worker.id.is_empty()
            || worker.id.len() > MAX_WORKER_ID_BYTES
            || worker
                .parent
                .as_ref()
                .is_some_and(|parent| parent.is_empty() || parent.len() > MAX_WORKER_ID_BYTES)
        {
            return Err(backend::Error::new(
                backend::ErrorKind::Unavailable,
                "A worker identity exceeds the supported size.",
            ));
        }
        worker.title = bounded_worker_title(&worker.title);
    }
    Ok(())
}

/// How many reconciliations list a session's agents again while OpenCode offers none.
const LISTINGS: u8 = 30;

/// The delay before reconnecting after the event stream is lost or an attempt
/// fails. It doubles after each failed attempt, up to [`RECONNECT_MAX`], and
/// resets once a connection opens.
const RECONNECT_INITIAL: Duration = Duration::from_millis(250);
const RECONNECT_MAX: Duration = Duration::from_secs(10);
/// Resolving the service's address and opening its event stream are each
/// bounded, so an attempt against a service that accepts but never answers
/// still ends.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Commands waiting for a connection while the service is unreachable.
const MAX_HELD: usize = 64;
/// Sessions read again at once after reconnecting.
const MAX_RESYNCS: usize = 16;
/// Attempts to read a session again after reconnecting before its space is
/// closed.
const RESYNC_ATTEMPTS: u8 = 3;
/// Events held for a session while it is read again; more close it, as an
/// overflowing event channel does.
const MAX_RESYNC_EVENTS: usize = 1024;
/// What a session is told when the event stream is lost.
const DISCONNECTED: &str = "Lost the connection to OpenCode; reconnecting.";

/// A worker replacement carries the snapshot epoch under which it was queued.
/// Other events retain their original ordering and are never invalidated.
pub(crate) enum QueuedEvent {
    Ordinary(Event),
    Workers {
        pending: Arc<Mutex<PendingWorkers>>,
        watermark: Arc<AtomicU64>,
    },
}

impl QueuedEvent {
    pub(crate) fn into_current(self) -> Option<Event> {
        match self {
            Self::Ordinary(event) => Some(event),
            Self::Workers { pending, watermark } => {
                let mut pending = pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                pending.latest.take().and_then(|update| match update {
                    PendingWorkerUpdate::Workers { epoch, workers } => (epoch
                        >= watermark.load(Ordering::Acquire))
                    .then_some(Event::WorkersChanged { workers }),
                    PendingWorkerUpdate::Unavailable(message) => {
                        Some(Event::WorkersUnavailable { message })
                    }
                })
            }
        }
    }
}

/// At most one worker replacement is queued per registration. Later changes
/// update the pending value instead of cloning whole families into the channel.
#[derive(Default)]
pub(crate) struct PendingWorkers {
    latest: Option<PendingWorkerUpdate>,
}

enum PendingWorkerUpdate {
    Workers { epoch: u64, workers: Vec<Worker> },
    Unavailable(String),
}

/// The sender side of one session's event channel.
#[derive(Clone)]
pub struct Events {
    tx: async_channel::Sender<QueuedEvent>,
    worker_tx: async_channel::Sender<()>,
    worker_epoch: Arc<AtomicU64>,
    worker_watermark: Arc<AtomicU64>,
    pending_workers: Arc<Mutex<PendingWorkers>>,
    worker_incomplete: Arc<AtomicBool>,
    ordinary_incomplete: Arc<AtomicBool>,
    consumer_started: Arc<AtomicBool>,
}

/// A session's event receiver and the identity of its specific registration.
#[derive(Clone)]
pub(crate) struct RegistrationEvents {
    receiver: async_channel::Receiver<QueuedEvent>,
    worker_receiver: async_channel::Receiver<()>,
    pending: Arc<Mutex<PendingWorkers>>,
    watermark: Arc<AtomicU64>,
    prefer_workers: Arc<AtomicBool>,
    ordinary_incomplete: Arc<AtomicBool>,
    consumer_started: Arc<AtomicBool>,
    pub(crate) identity: Arc<AtomicU64>,
}

impl RegistrationEvents {
    fn worker_event(&self) -> QueuedEvent {
        QueuedEvent::Workers {
            pending: self.pending.clone(),
            watermark: self.watermark.clone(),
        }
    }

    fn try_recv(&self) -> std::result::Result<QueuedEvent, async_channel::TryRecvError> {
        if self.ordinary_incomplete.load(Ordering::Acquire) {
            return Err(async_channel::TryRecvError::Closed);
        }
        let prefer_worker = self.prefer_workers.fetch_xor(true, Ordering::Relaxed);
        if prefer_worker && self.worker_receiver.try_recv().is_ok() {
            return Ok(self.worker_event());
        }
        match self.receiver.try_recv() {
            Ok(event) => return Ok(event),
            Err(async_channel::TryRecvError::Closed) => {
                return Err(async_channel::TryRecvError::Closed);
            }
            Err(async_channel::TryRecvError::Empty) => {}
        }
        if !prefer_worker && self.worker_receiver.try_recv().is_ok() {
            return Ok(self.worker_event());
        }
        Err(async_channel::TryRecvError::Empty)
    }

    pub(crate) async fn recv(&self) -> std::result::Result<QueuedEvent, async_channel::RecvError> {
        self.consumer_started.store(true, Ordering::Release);
        match self.try_recv() {
            Ok(event) => return Ok(event),
            Err(async_channel::TryRecvError::Closed) => return Err(async_channel::RecvError),
            Err(async_channel::TryRecvError::Empty) => {}
        }
        let result = futures_lite::future::race(self.receiver.recv(), async {
            match self.worker_receiver.recv().await {
                Ok(()) => Ok(self.worker_event()),
                // Both channels close together when the registration ends.
                // `race` polls in random order, so leave the end to the
                // ordinary channel, which still holds the reason it closed.
                Err(_) => std::future::pending().await,
            }
        })
        .await;
        if self.ordinary_incomplete.load(Ordering::Acquire) {
            Err(async_channel::RecvError)
        } else {
            result
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.receiver.len() + self.worker_receiver.len()
    }
}

impl Events {
    pub fn new() -> (Self, RegistrationEvents) {
        let (tx, rx) = async_channel::bounded(1024);
        let (worker_tx, worker_rx) = async_channel::bounded(1);
        let identity = Arc::new(AtomicU64::new(0));
        let watermark = Arc::new(AtomicU64::new(0));
        let pending = Arc::new(Mutex::new(PendingWorkers::default()));
        let ordinary_incomplete = Arc::new(AtomicBool::new(false));
        let consumer_started = Arc::new(AtomicBool::new(false));
        (
            Self {
                tx,
                worker_tx,
                worker_epoch: identity.clone(),
                worker_watermark: watermark.clone(),
                pending_workers: pending.clone(),
                worker_incomplete: Arc::new(AtomicBool::new(false)),
                ordinary_incomplete: ordinary_incomplete.clone(),
                consumer_started: consumer_started.clone(),
            },
            RegistrationEvents {
                receiver: rx,
                worker_receiver: worker_rx,
                pending,
                watermark,
                prefer_workers: Arc::new(AtomicBool::new(true)),
                ordinary_incomplete,
                consumer_started,
                identity,
            },
        )
    }

    /// Attempt delivery without blocking the shared dispatcher.
    pub fn deliver(&self, event: Event) -> bool {
        match self.tx.try_send(QueuedEvent::Ordinary(event)) {
            Ok(()) => true,
            Err(async_channel::TrySendError::Full(_)) => {
                // Ordinary events cannot be reconstructed reliably. Closing
                // the channel makes the loss visible instead of silently
                // presenting an incomplete session as current.
                self.mark_ordinary_incomplete();
                false
            }
            Err(async_channel::TrySendError::Closed(_)) => false,
        }
    }

    /// Retryable state events are allowed to wait for ordinary channel space.
    fn try_deliver_state(&self, event: Event) -> bool {
        self.tx.try_send(QueuedEvent::Ordinary(event)).is_ok()
    }

    fn mark_ordinary_incomplete(&self) {
        self.ordinary_incomplete.store(true, Ordering::Release);
        self.tx.close();
    }

    fn publish_worker_update(&self, update: PendingWorkerUpdate) -> bool {
        if self.worker_tx.is_closed() || self.tx.is_closed() {
            return false;
        }
        let mut pending = self
            .pending_workers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.latest = Some(update);
        matches!(
            self.worker_tx.try_send(()),
            Ok(()) | Err(async_channel::TrySendError::Full(()))
        )
    }

    /// Worker notifications use a separate one-element channel so a burst
    /// cannot fill the ordinary event channel or duplicate family snapshots.
    fn try_deliver_workers(&self, workers: Vec<Worker>) -> bool {
        self.publish_worker_update(PendingWorkerUpdate::Workers {
            epoch: self.worker_epoch.load(Ordering::Acquire),
            workers,
        })
    }

    fn try_deliver_worker_error(&self, message: &str) -> bool {
        self.publish_worker_update(PendingWorkerUpdate::Unavailable(message.to_owned()))
    }

    /// Begin a new producer epoch. Old events are superseded only if the
    /// receiver actually accepts the corresponding snapshot.
    fn advance_workers(&self) {
        self.worker_epoch.fetch_add(1, Ordering::AcqRel);
    }

    fn snapshot_reply(&self, snapshot: Snapshot) -> SnapshotReply {
        SnapshotReply {
            snapshot,
            epoch: self.worker_epoch.load(Ordering::Acquire),
            watermark: self.worker_watermark.clone(),
            worker_incomplete: self.worker_incomplete.clone(),
            ordinary_incomplete: self.ordinary_incomplete.clone(),
        }
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
    /// Deletions observed while a snapshot is in flight; its listing may
    /// predate them, so they must not be resurrected when it is installed.
    snapshot_deletions: HashSet<String>,
    snapshot_deletions_overflow: bool,
    /// State and title events for children not yet discovered by the initial
    /// listing. Their parent is not included in execution events.
    initial_changes: HashMap<String, (Option<WorkerState>, Option<String>)>,
    /// Creations whose parent has not yet been discovered by the snapshot.
    initial_creations: BTreeMap<String, Worker>,
    /// Worker events are not published until a snapshot has been installed.
    workers_ready: bool,
    /// The first offered bootstrap epoch; its journals remain until accepted.
    bootstrap_epoch: Option<u64>,
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
    /// Set from a lost event stream until the session has been read again.
    resync: Option<Resync>,
}

/// A session waiting to be read again after the event stream was lost.
#[derive(Default)]
struct Resync {
    /// Ordinary events since the connection was restored, delivered after the
    /// state read again, in order, as when a space first opens.
    events: Vec<Event>,
    /// Failed attempts on connections since the loss.
    failures: u8,
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
    pub fn new() -> (Self, RegistrationEvents) {
        let (events, rx) = Events::new();
        (
            Self {
                events,
                seen: Mutex::new(HashSet::new()),
                running: RunningState::Unknown,
                children: BTreeMap::new(),
                initial_deletions: HashSet::new(),
                snapshot_deletions: HashSet::new(),
                snapshot_deletions_overflow: false,
                initial_changes: HashMap::new(),
                initial_creations: BTreeMap::new(),
                workers_ready: false,
                bootstrap_epoch: None,
                workers_dirty: false,
                workers_incomplete: false,
                snapshot_in_flight: None,
                reconcile_in_flight: None,
                event_revision: 0,
                modes: Modes::default(),
                modes_dirty: false,
                directory: String::new(),
                listings: LISTINGS,
                resync: None,
            },
            rx,
        )
    }

    /// Merge the workers listed when the session is read again after a lost
    /// event stream. A listed worker is added, or its state and title are
    /// updated; a worker deleted while it was read is not restored. A known
    /// worker the listing omits is kept, because an omission is never evidence
    /// of deletion, but it is no longer reported as running: the evidence it
    /// was running predates the gap. Exceeding the tracking limit makes worker
    /// observation visibly incomplete rather than failing the read.
    fn merge_workers(&mut self, mut workers: Vec<Worker>) -> backend::Result<()> {
        if self.workers_incomplete {
            return Ok(());
        }
        validate_workers(&mut workers)?;
        let listed: HashSet<&str> = workers.iter().map(|worker| worker.id.as_str()).collect();
        for (id, known) in &mut self.children {
            if !listed.contains(id.as_str()) && known.state == WorkerState::Running {
                known.state = WorkerState::Unknown;
            }
        }
        for worker in workers {
            if self.snapshot_deletions.contains(&worker.id) {
                continue;
            }
            if let Some(known) = self.children.get_mut(&worker.id) {
                known.state = worker.state;
                known.title = worker.title;
            } else if self.children.len() >= api::MAX_TRACKED_WORKERS {
                self.mark_workers_incomplete();
                return Ok(());
            } else {
                self.children.insert(worker.id.clone(), worker);
            }
        }
        Ok(())
    }

    fn publish_workers(&mut self) {
        if self.workers_ready {
            self.workers_dirty = if self.workers_incomplete {
                !self.events.try_deliver_worker_error(
                    "Worker observation exceeds the supported limit; reattach after reducing it.",
                )
            } else {
                !self
                    .events
                    .try_deliver_workers(self.children.values().cloned().collect())
            };
        }
    }

    fn mark_workers_incomplete(&mut self) {
        if !self.workers_incomplete {
            self.workers_incomplete = true;
            self.events.worker_incomplete.store(true, Ordering::Release);
            self.publish_workers();
        }
    }

    /// Remove an observed deletion, remembering it against any snapshot
    /// whose listing is still in flight.
    fn remove_worker(&mut self, id: &str) -> bool {
        if self.snapshot_in_flight.is_some() {
            if self.snapshot_deletions.contains(id)
                || self.snapshot_deletions.len() < api::MAX_TRACKED_WORKERS
            {
                self.snapshot_deletions.insert(id.to_owned());
            } else {
                self.snapshot_deletions_overflow = true;
            }
        }
        self.children.remove(id).is_some()
    }

    fn publish_modes(&mut self) {
        self.modes_dirty = !self.deliver_state(Event::ModesChanged(self.modes.clone()));
    }

    fn deliver_state(&self, event: Event) -> bool {
        let delivered = self.events.try_deliver_state(event);
        if !delivered && !self.events.consumer_started.load(Ordering::Acquire) {
            // A retry after snapshot acceptance is too late to make the
            // initial busy/mode state safe. Refuse the bootstrap instead.
            self.events.mark_ordinary_incomplete();
        }
        delivered
    }

    fn reconcile_running(&mut self, running: RunningState) {
        if self.running != running
            && let Some(event) = running.event()
            && self.deliver_state(event)
        {
            self.running = running;
        }
    }

    fn discovering_workers(&self) -> bool {
        !self
            .bootstrap_epoch
            .is_some_and(|epoch| self.events.worker_watermark.load(Ordering::Acquire) >= epoch)
    }

    fn clear_bootstrap_journals(&mut self) {
        self.initial_deletions.clear();
        self.initial_changes.clear();
        self.initial_creations.clear();
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
        validate_workers(workers)?;
        let discovering = self.discovering_workers();
        let additional = workers
            .iter()
            .filter(|worker| {
                !self.initial_deletions.contains(&worker.id)
                    && !self.snapshot_deletions.contains(&worker.id)
                    && !self.children.contains_key(&worker.id)
            })
            .count();
        if self.children.len().saturating_add(additional) > api::MAX_TRACKED_WORKERS {
            return Err(backend::Error::new(
                backend::ErrorKind::Unavailable,
                "Worker observation exceeds the supported limit.",
            ));
        }
        self.events.advance_workers();
        if self.bootstrap_epoch.is_none() {
            self.bootstrap_epoch = Some(self.events.worker_epoch.load(Ordering::Acquire));
        }
        for mut worker in workers.drain(..) {
            if !self.initial_deletions.contains(&worker.id)
                && !self.snapshot_deletions.contains(&worker.id)
            {
                if discovering
                    && !self.children.contains_key(&worker.id)
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
        if discovering {
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
        if !discovering {
            self.clear_bootstrap_journals();
        }
        self.workers_ready = true;
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
        reply: async_channel::Sender<backend::Result<(String, RegistrationEvents)>>,
    },
    Attach {
        id: String,
        directory: String,
        reply: async_channel::Sender<backend::Result<RegistrationEvents>>,
    },
    Snapshot {
        id: String,
        identity: Arc<AtomicU64>,
        reply: async_channel::Sender<backend::Result<SnapshotReply>>,
    },
    Prompt {
        id: String,
        identity: Arc<AtomicU64>,
        key: String,
        text: String,
        reply: async_channel::Sender<backend::Result<()>>,
    },
    Interrupt {
        id: String,
        identity: Arc<AtomicU64>,
        reply: async_channel::Sender<backend::Result<()>>,
    },
    Decide {
        id: String,
        identity: Arc<AtomicU64>,
        request: String,
        decision: String,
        reply: async_channel::Sender<backend::Result<()>>,
    },
    Answer {
        id: String,
        identity: Arc<AtomicU64>,
        form: String,
        answer: Value,
        reply: async_channel::Sender<backend::Result<()>>,
    },
    /// Choose the agent for `id`'s later prompts.
    SetMode {
        id: String,
        identity: Arc<AtomicU64>,
        mode: String,
        reply: async_channel::Sender<backend::Result<()>>,
    },
    /// Close `id`'s registration without deleting its OpenCode session.
    Close {
        id: String,
        identity: Arc<AtomicU64>,
    },
}

struct SnapshotResult {
    id: String,
    registration: Arc<AtomicU64>,
    reply: async_channel::Sender<backend::Result<SnapshotReply>>,
    result: backend::Result<Snapshot>,
}

/// A snapshot's worker epoch becomes authoritative only when its recipient
/// receives it; an abandoned reply must not invalidate queued worker events.
pub(crate) struct SnapshotReply {
    snapshot: Snapshot,
    epoch: u64,
    watermark: Arc<AtomicU64>,
    worker_incomplete: Arc<AtomicBool>,
    ordinary_incomplete: Arc<AtomicBool>,
}

impl SnapshotReply {
    pub(crate) fn accept(self) -> backend::Result<Snapshot> {
        if self.ordinary_incomplete.load(Ordering::Acquire) {
            return Err(backend::Error::new(
                backend::ErrorKind::Unavailable,
                "Session events were lost while reading the initial state.",
            ));
        }
        if self.worker_incomplete.load(Ordering::Acquire) {
            return Err(backend::Error::new(
                backend::ErrorKind::Unavailable,
                "Worker observation exceeds the supported limit.",
            ));
        }
        let previous = self.watermark.fetch_max(self.epoch, Ordering::AcqRel);
        if self.epoch < previous {
            return Err(backend::Error::new(
                backend::ErrorKind::Unavailable,
                "A newer session snapshot was already accepted.",
            ));
        }
        Ok(self.snapshot)
    }
}

struct ResyncResult {
    id: String,
    registration: Arc<AtomicU64>,
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

    /// The raw sender, including nonblocking registration cleanup.
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

/// How the backend finds the OpenCode service. It is called before every
/// connection attempt, so a service that restarts on another port or with
/// another password is found again.
pub type Resolve = Box<dyn FnMut() -> Resolved + Send>;

/// Run the connection until its command channel closes, reconnecting whenever
/// the service cannot be reached or its event stream ends. Blocking: called on
/// a dedicated thread.
pub fn run_thread(resolve: Resolve, commands: async_channel::Receiver<Command>) {
    if let Err(message) = block_on(drive(resolve, commands)) {
        eprintln!("the opencode connection ended: {message}");
    }
}

async fn drive(
    resolve: Resolve,
    commands: async_channel::Receiver<Command>,
) -> std::result::Result<(), String> {
    let scheduler = NativeScheduler::with_options(NativeOptions {
        workers: 1,
        threads: 2,
        capacity: 256,
        ..Default::default()
    })
    .map_err(|error| format!("could not start the connection scheduler: {error}"))?;
    run(&scheduler, resolve, commands).await;
    block_on(scheduler.shutdown());
    Ok(())
}

/// What outlives one connection to the service.
#[derive(Default)]
struct Sessions {
    registrations: HashMap<String, Registration>,
    /// Commands received while no connection is open, performed in order once
    /// one opens, or refused if the attempt fails.
    held: VecDeque<Command>,
}

/// Why a connection to the service ended.
enum Outcome {
    /// The command channel closed: the backend and every session were dropped.
    Shutdown,
    /// No connection could be opened.
    Unreachable(String),
    /// The event stream was open and then ended or failed.
    Lost(String),
}

/// Connect, serve until the connection is lost, and connect again, with a
/// bounded backoff, until the command channel closes.
async fn run<'scope, S: Scheduler<'scope> + Send + Sync + 'static>(
    scheduler: &S,
    mut resolve: Resolve,
    commands: async_channel::Receiver<Command>,
) where
    <S as Scheduler<'scope>>::Socket: 'static,
{
    let mut sessions = Sessions::default();
    let mut delay = RECONNECT_INITIAL;
    let mut attempt_at = scheduler.now();
    let mut earliest = attempt_at;
    let mut reported: Option<String> = None;
    let mut announce = false;
    loop {
        if !wait_until_attempt(scheduler, &commands, &mut sessions, attempt_at, earliest).await {
            break;
        }
        let started = scheduler.now();
        // A command waiting for a connection brings the next attempt forward,
        // but never to less than the initial delay after this one.
        earliest = started + RECONNECT_INITIAL;
        let resolved = resolve();
        match connect(scheduler, &resolved, &commands, &mut sessions, announce).await {
            Outcome::Shutdown => break,
            Outcome::Unreachable(reason) => {
                let message = format!(
                    "OpenCode is not reachable at {}: {reason}",
                    resolved.authority()
                );
                // Report each distinct failure once, not every retry.
                if reported.as_deref() != Some(message.as_str()) {
                    eprintln!("{message}; retrying");
                    reported = Some(message.clone());
                }
                for command in sessions.held.drain(..) {
                    refuse(
                        command,
                        backend::Error::new(backend::ErrorKind::Unavailable, message.clone()),
                    );
                }
                attempt_at = started + delay;
                delay = (delay * 2).min(RECONNECT_MAX);
                announce = true;
            }
            Outcome::Lost(reason) => {
                eprintln!("the opencode event stream {reason}; reconnecting");
                reported = None;
                connection_lost(&mut sessions.registrations);
                let now = scheduler.now();
                // A connection that is lost soon after it opens counts as a
                // failed attempt, so a service that keeps dropping the stream
                // is not reconnected to, and every session read again, in a
                // tight loop.
                if now.saturating_duration_since(started) >= RECONNECT_MAX {
                    delay = RECONNECT_INITIAL;
                }
                attempt_at = now + delay;
                delay = (delay * 2).min(RECONNECT_MAX);
                announce = true;
            }
        }
    }
    // Deliver Closed to every session still bound, so no session waits forever
    // for its event channel to fill.
    for registration in sessions.registrations.values() {
        registration.events.deliver(Event::Closed {
            reason: Some("the connection ended".to_owned()),
        });
    }
}

/// Answer `command` with `error` without performing it.
fn refuse(command: Command, error: backend::Error) {
    match command {
        Command::Sessions { reply, .. } => {
            let _ = reply.try_send(Err(error));
        }
        Command::Create { reply, .. } => {
            let _ = reply.try_send(Err(error));
        }
        Command::Attach { reply, .. } => {
            let _ = reply.try_send(Err(error));
        }
        Command::Snapshot { reply, .. } => {
            let _ = reply.try_send(Err(error));
        }
        Command::Prompt { reply, .. }
        | Command::Interrupt { reply, .. }
        | Command::Decide { reply, .. }
        | Command::Answer { reply, .. }
        | Command::SetMode { reply, .. } => {
            let _ = reply.try_send(Err(error));
        }
        // Closing needs no connection; see `hold`.
        Command::Close { .. } => {}
    }
}

/// Take a command while no connection is open: a close is performed at once,
/// and anything else waits for the next attempt.
fn hold(sessions: &mut Sessions, command: Command) {
    match command {
        Command::Close { id, identity } => {
            close_registration(&mut sessions.registrations, &id, &identity);
        }
        command if sessions.held.len() < MAX_HELD => sessions.held.push_back(command),
        command => refuse(
            command,
            backend::Error::new(
                backend::ErrorKind::Unavailable,
                "OpenCode is not reachable and too many requests are waiting for it",
            ),
        ),
    }
}

/// Run `future` for at most `timeout` while no connection is open, taking
/// commands meanwhile as [`hold`] does. `None` once the command channel has
/// closed.
async fn holding<T, S: Timers>(
    future: impl Future<Output = std::result::Result<T, String>>,
    timers: &S,
    commands: &async_channel::Receiver<Command>,
    sessions: &mut Sessions,
    timeout: Duration,
) -> Option<std::result::Result<T, String>> {
    let cancellation = Cancellation::new();
    let deadline = timers.now() + timeout;
    let mut timer = timers.timer();
    let mut future = Fused::new(future);
    loop {
        futures_util::select! {
            result = &mut future => return Some(result),
            command = Fused::new(commands.recv()) => match command {
                Ok(command) => hold(sessions, command),
                Err(_) => return None,
            },
            due = Fused::new(timers.timer_wait_until(&mut timer, deadline, &cancellation)) => {
                return Some(Err(match due {
                    Ok(()) => format!("no answer within {timeout:?}"),
                    Err(error) => format!("the connection timer failed: {error}"),
                }));
            }
        }
    }
}

/// Wait until `deadline`, or `earliest` while a command waits for a
/// connection, taking commands meanwhile as [`hold`] does. Returns false once
/// the command channel has closed.
async fn wait_until_attempt<S: Timers>(
    timers: &S,
    commands: &async_channel::Receiver<Command>,
    sessions: &mut Sessions,
    deadline: Instant,
    earliest: Instant,
) -> bool {
    let cancellation = Cancellation::new();
    let mut timer = timers.timer();
    loop {
        let deadline = if sessions.held.is_empty() {
            deadline
        } else {
            deadline.min(earliest)
        };
        if timers.now() >= deadline {
            return true;
        }
        futures_util::select! {
            command = Fused::new(commands.recv()) => match command {
                Ok(command) => hold(sessions, command),
                Err(_) => return false,
            },
            due = Fused::new(timers.timer_wait_until(&mut timer, deadline, &cancellation)) => {
                if let Err(error) = due {
                    // Without a timer the backoff cannot be bounded; stop
                    // rather than retry in a tight loop.
                    eprintln!("the opencode reconnection timer failed: {error}");
                    return false;
                }
            }
        }
    }
}

/// Resolve the service's address, open its event stream and serve until it
/// ends. Every step before the stream is open is bounded by
/// [`CONNECT_TIMEOUT`] and keeps taking commands.
async fn connect<'scope, S: Scheduler<'scope> + Send + Sync + 'static>(
    scheduler: &S,
    resolved: &Resolved,
    commands: &async_channel::Receiver<Command>,
    sessions: &mut Sessions,
    announce: bool,
) -> Outcome
where
    <S as Scheduler<'scope>>::Socket: 'static,
{
    let cancellation = Cancellation::new();
    let lookup = async {
        scheduler
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
            .ok_or_else(|| "the server's address did not resolve to any endpoint".to_owned())
    };
    let address = match holding(lookup, scheduler, commands, sessions, CONNECT_TIMEOUT).await {
        None => return Outcome::Shutdown,
        Some(Err(reason)) => return Outcome::Unreachable(reason),
        Some(Ok(address)) => address,
    };
    let (client, driver) = match Client::new(
        scheduler,
        Endpoint::new(address),
        base_http_client::Options::default(),
    ) {
        Ok(created) => created,
        Err(error) => return Outcome::Unreachable(format!("creating the HTTP client: {error}")),
    };

    // The driver must be polled for as long as the client is used, so it runs
    // alongside the connection loop in the same future; finishing either ends
    // both. The cancellation is shared so closing the connection unwinds both.
    let (driver_result, outcome) = futures_lite::future::zip(
        async {
            let result = driver.run(&cancellation).await;
            cancellation.cancel();
            result
        },
        async {
            let outcome = connected(
                &client,
                resolved,
                commands,
                sessions,
                &cancellation,
                scheduler,
                announce,
            )
            .await;
            cancellation.cancel();
            outcome
        },
    )
    .await;
    match (outcome, driver_result) {
        (Outcome::Lost(reason), Err(error)) => {
            Outcome::Lost(format!("{reason} (the HTTP driver ended: {error})"))
        }
        (outcome, _) => outcome,
    }
}

/// The event stream was lost, so events are missing until the next connection.
/// Every session whose state has been read is read again then, holding its
/// events meanwhile, and is told about the gap now. A session not yet read is
/// closed: the read it is waiting for was abandoned with the connection, and
/// reading it later would present events from before the gap as current.
fn connection_lost(registrations: &mut HashMap<String, Registration>) {
    let mut unread = Vec::new();
    for (id, registration) in registrations.iter_mut() {
        // Their futures were dropped with the connection.
        registration.snapshot_in_flight = None;
        registration.reconcile_in_flight = None;
        registration.snapshot_deletions.clear();
        registration.snapshot_deletions_overflow = false;
        if registration.workers_ready {
            // Events held from a connection lost before the session was read
            // again precede the next read, which covers them; delivering them
            // after it would present them as newer.
            let resync = registration.resync.get_or_insert_with(Resync::default);
            resync.events.clear();
            resync.failures = 0;
            registration.events.deliver(Event::Error {
                message: DISCONNECTED.to_owned(),
            });
        } else {
            unread.push(id.clone());
        }
    }
    for id in unread {
        end_registration(
            registrations,
            &id,
            Some("the connection to OpenCode was lost before the session was read".to_owned()),
        );
    }
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

type SnapshotFutures<'a> = FuturesUnordered<Pin<Box<dyn Future<Output = SnapshotResult> + 'a>>>;
type ResyncFutures<'a> = FuturesUnordered<Pin<Box<dyn Future<Output = ResyncResult> + 'a>>>;

/// Open the server's global event stream, then serve it and the commands,
/// alternating reads over the stream, command channel reads and a periodic
/// reconciliation of each session's running state, until the backend closes or
/// the stream ends. Commands held while no connection was open are performed
/// first, in order, and sessions whose events were lost are read again.
async fn connected<S: Timers>(
    client: &Client<'_>,
    resolved: &Resolved,
    commands: &async_channel::Receiver<Command>,
    sessions: &mut Sessions,
    cancellation: &Cancellation,
    timers: &S,
    announce: bool,
) -> Outcome {
    let open = async {
        let mut request = base_http::Request::get(base_http::Path::from("/api/event".to_owned()));
        if let Some(authorization) = resolved.authorization_header()
            && let Ok(field) = base_http::Field::new("authorization", authorization)
        {
            let _ = request.fields.append(field);
        }
        let response = client
            .call(request, cancellation)
            .await
            .map_err(|failure| format!("opening the event stream: {failure}"))?;
        let status = response.status.as_u16();
        if !(200..300).contains(&status) {
            return Err(format!("the event stream was refused with status {status}"));
        }
        Ok(response.body)
    };
    let mut stream = match holding(open, timers, commands, sessions, CONNECT_TIMEOUT).await {
        None => return Outcome::Shutdown,
        Some(Err(reason)) => return Outcome::Unreachable(reason),
        Some(Ok(stream)) => stream,
    };
    if announce {
        eprintln!("connected to OpenCode at {}", resolved.authority());
    }
    let Sessions {
        registrations,
        held,
    } = sessions;

    let mut pending = String::new();
    let mut timer = timers.timer();
    let mut next_reconcile = timers.now() + RECONCILE_INTERVAL;
    let mut reconcile_offset = 0usize;
    let mut snapshots: SnapshotFutures<'_> = FuturesUnordered::new();
    let mut resyncs: ResyncFutures<'_> = FuturesUnordered::new();
    let mut reconciliations: FuturesUnordered<Pin<Box<dyn Future<Output = ReconcileResult> + '_>>> =
        FuturesUnordered::new();
    // The stream is open, so whatever happens from now on is observed: read
    // again every session whose events were lost.
    start_resyncs(
        registrations,
        &mut resyncs,
        client,
        resolved,
        cancellation,
        timers,
    );

    let outcome = loop {
        let command = match held.pop_front() {
            Some(command) => command,
            None => futures_util::select! {
                command = Fused::new(commands.recv()) => match command {
                    Ok(command) => command,
                    Err(_) => break Outcome::Shutdown,
                },
                completed = Fused::new(async {
                    if snapshots.is_empty() {
                        std::future::pending::<Option<SnapshotResult>>().await
                    } else {
                        snapshots.next().await
                    }
                }) => {
                    if let Some(completed) = completed {
                        let result = match registrations.get_mut(&completed.id) {
                            Some(registration) if Arc::ptr_eq(&registration.events.worker_epoch, &completed.registration) => {
                                registration.snapshot_in_flight = None;
                                let overflow = std::mem::take(&mut registration.snapshot_deletions_overflow);
                                let result = completed.result.and_then(|mut snapshot| {
                                    if overflow {
                                        return Err(backend::Error::new(backend::ErrorKind::Unavailable, "Too many sessions were deleted during the snapshot; please retry."));
                                    }
                                    registration.install_workers(&mut snapshot.workers)?;
                                    Ok(registration.events.snapshot_reply(snapshot))
                                });
                                registration.snapshot_deletions.clear();
                                result
                            }
                            _ => Err(backend::Error::new(backend::ErrorKind::Closed, "The session was closed during its snapshot.")),
                        };
                        let _ = completed.reply.try_send(result);
                    }
                    continue;
                }
                completed = Fused::new(async {
                    if resyncs.is_empty() {
                        std::future::pending::<Option<ResyncResult>>().await
                    } else {
                        resyncs.next().await
                    }
                }) => {
                    if let Some(completed) = completed {
                        finish_resync(registrations, completed);
                    }
                    // A slot is free: start the next waiting session at once.
                    start_resyncs(registrations, &mut resyncs, client, resolved, cancellation, timers);
                    continue;
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
                        if registration.event_revision == completed.revision && registration.resync.is_none() {
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
                    continue;
                }
                result = Fused::new(stream.read(cancellation)) => {
                    let frame = match result {
                        Ok(frame) => frame,
                        Err(error) => break Outcome::Lost(format!("failed: {error}")),
                    };
                    let end = frame.is_end();
                    if let base_http::body::BodyFrame::Data { data, .. } = frame {
                        pending.push_str(&String::from_utf8_lossy(&data));
                        while let Some(index) = pending.find("\n\n") {
                            let group = pending[..index + 2].to_owned();
                            pending = pending[index + 2..].to_owned();
                            if let Some(payload) = sse_payload(&group) {
                                dispatch_events(&payload, registrations);
                            }
                        }
                    }
                    if end {
                        break Outcome::Lost("ended".to_owned());
                    }
                    continue;
                }
                // Re-check each session's running state against the server's record,
                // so a missed idle event cannot leave a busy state — and the queue it
                // gates — stuck. A no-op while no session is registered.
                due = Fused::new(timers.timer_wait_until(&mut timer, next_reconcile, cancellation)) => {
                    if let Err(error) = due {
                        break Outcome::Lost(format!("stopped: the state reconciliation timer failed: {error}"));
                    }
                    let mut ids: Vec<_> = registrations.keys().cloned().collect();
                    for id in &ids {
                        close_abandoned_registration(registrations, id);
                    }
                    // A session read again after reconnecting reports its
                    // running state from that read.
                    ids.retain(|id| registrations.get(id).is_some_and(|registration| registration.resync.is_none()));
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
                        reconciliations.push(Box::pin(async move {
                            let (modes, running) = futures_lite::future::race(
                                async {
                                    let modes = if list {
                                        let info = current.map_or(Value::Null, |agent| serde_json::json!({"agent":agent}));
                                        Some(modes(client, resolved, &directory, &info, &local_cancel).await)
                                    } else { None };
                                    let running = api::session_running(client, resolved, &id, &local_cancel, timers).await.ok().flatten();
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
                    // Retry a session whose read again failed.
                    start_resyncs(registrations, &mut resyncs, client, resolved, cancellation, timers);
                    retry_worker_delivery(registrations);
                    next_reconcile = timers.now() + RECONCILE_INTERVAL;
                    continue;
                }
            },
        };
        match command {
            Command::Snapshot {
                id,
                identity,
                reply,
            } => schedule_snapshot(
                registrations,
                &mut snapshots,
                id,
                &identity,
                reply,
                client,
                resolved,
                cancellation,
                timers,
            ),
            command => {
                if !handle_command(command, client, resolved, registrations, cancellation).await {
                    break Outcome::Shutdown;
                }
            }
        }
        retry_worker_delivery(registrations);
    };
    cancellation.cancel();
    outcome
}

/// Read a session within [`SNAPSHOT_TIMEOUT`], cancelling its requests when the
/// read ends either way.
async fn bounded_snapshot<S: Timers>(
    client: &Client<'_>,
    resolved: &Resolved,
    id: &str,
    modes: Modes,
    local: &Cancellation,
    cancellation: &Cancellation,
    timers: &S,
) -> backend::Result<Snapshot> {
    let result = futures_lite::future::race(
        async {
            snapshot(client, resolved, id, modes, local, timers)
                .await
                .map_err(map_failure)
        },
        async {
            let timer = timers.sleep(SNAPSHOT_TIMEOUT, cancellation).await;
            Err(backend::Error::new(
                backend::ErrorKind::Unavailable,
                format!("Snapshot timed out or was cancelled: {timer:?}"),
            ))
        },
    )
    .await;
    local.cancel();
    result
}

/// Start a snapshot a session asked for, isolated from the event dispatcher.
#[allow(clippy::too_many_arguments)]
fn schedule_snapshot<'a, S: Timers>(
    registrations: &mut HashMap<String, Registration>,
    snapshots: &mut SnapshotFutures<'a>,
    id: String,
    identity: &Arc<AtomicU64>,
    reply: async_channel::Sender<backend::Result<SnapshotReply>>,
    client: &'a Client<'_>,
    resolved: &'a Resolved,
    cancellation: &'a Cancellation,
    timers: &'a S,
) {
    let Some(registration) = registrations
        .get_mut(&id)
        .filter(|registration| Arc::ptr_eq(&registration.events.worker_epoch, identity))
    else {
        let _ = reply.try_send(Err(backend::Error::new(
            backend::ErrorKind::Closed,
            "The session is not registered.",
        )));
        return;
    };
    if registration.resync.is_some() {
        let _ = reply.try_send(Err(backend::Error::new(
            backend::ErrorKind::Unavailable,
            "The session is being read again after reconnecting to OpenCode; please retry.",
        )));
        return;
    }
    if registration.snapshot_in_flight.is_some() || snapshots.len() >= MAX_SNAPSHOTS {
        let _ = reply.try_send(Err(backend::Error::new(
            backend::ErrorKind::Unavailable,
            "A session snapshot is already running or the snapshot limit was reached; please retry.",
        )));
        return;
    }
    let local = cancellation.child();
    registration.snapshot_in_flight = Some(local.clone());
    registration.snapshot_deletions.clear();
    registration.snapshot_deletions_overflow = false;
    let identity = registration.events.worker_epoch.clone();
    let modes = registration.modes.clone();
    snapshots.push(Box::pin(async move {
        let result =
            bounded_snapshot(client, resolved, &id, modes, &local, cancellation, timers).await;
        SnapshotResult {
            id,
            registration: identity,
            reply,
            result,
        }
    }));
}

/// Start reading again the sessions whose events were lost, as many at once as
/// [`MAX_RESYNCS`] allows. Each read is isolated and bounded like a snapshot,
/// so a slow session never holds up another or the event stream.
fn start_resyncs<'a, S: Timers>(
    registrations: &mut HashMap<String, Registration>,
    resyncs: &mut ResyncFutures<'a>,
    client: &'a Client<'_>,
    resolved: &'a Resolved,
    cancellation: &'a Cancellation,
    timers: &'a S,
) {
    let mut ids: Vec<_> = registrations
        .iter()
        .filter(|(_, registration)| {
            registration.resync.is_some() && registration.snapshot_in_flight.is_none()
        })
        .map(|(id, _)| id.clone())
        .collect();
    ids.sort();
    for id in ids {
        if resyncs.len() >= MAX_RESYNCS {
            break;
        }
        let registration = registrations.get_mut(&id).expect("listed above");
        let local = cancellation.child();
        registration.snapshot_in_flight = Some(local.clone());
        registration.snapshot_deletions.clear();
        registration.snapshot_deletions_overflow = false;
        let identity = registration.events.worker_epoch.clone();
        let modes = registration.modes.clone();
        resyncs.push(Box::pin(async move {
            let result =
                bounded_snapshot(client, resolved, &id, modes, &local, cancellation, timers).await;
            ResyncResult {
                id,
                registration: identity,
                result,
            }
        }));
    }
}

/// Present a session read again after reconnecting: its workers are merged,
/// then [`Event::Resynchronized`] is delivered, followed by the events held
/// since the connection was restored. A session that no longer exists, or that
/// could not be read in [`RESYNC_ATTEMPTS`] attempts, is closed with the reason.
fn finish_resync(registrations: &mut HashMap<String, Registration>, completed: ResyncResult) {
    let Some(registration) = registrations.get_mut(&completed.id).filter(|registration| {
        Arc::ptr_eq(&registration.events.worker_epoch, &completed.registration)
    }) else {
        return;
    };
    registration.snapshot_in_flight = None;
    let overflow = std::mem::take(&mut registration.snapshot_deletions_overflow);
    let result = completed.result.and_then(|mut snapshot| {
        if overflow {
            return Err(backend::Error::new(
                backend::ErrorKind::Unavailable,
                "too many sessions were deleted while it was read",
            ));
        }
        registration.merge_workers(std::mem::take(&mut snapshot.workers))?;
        Ok(snapshot)
    });
    registration.snapshot_deletions.clear();
    let reason = match result {
        Ok(mut snapshot) => {
            // Mode changes seen meanwhile were held back with the events.
            snapshot.modes = registration.modes.clone();
            let held = registration
                .resync
                .take()
                .map(|resync| resync.events)
                .unwrap_or_default();
            let mut running = match snapshot.busy {
                Some(true) => RunningState::Running,
                Some(false) => RunningState::Idle,
                None => registration.running,
            };
            for event in &held {
                match event {
                    Event::Busy => running = RunningState::Running,
                    Event::Idle => running = RunningState::Idle,
                    _ => {}
                }
            }
            registration.running = running;
            registration.event_revision = registration.event_revision.wrapping_add(1);
            registration.modes_dirty = false;
            registration
                .events
                .deliver(Event::Resynchronized { snapshot });
            for event in held {
                registration.events.deliver(event);
            }
            registration.publish_workers();
            return;
        }
        Err(error) if error.kind == backend::ErrorKind::NotFound => {
            "the OpenCode session no longer exists".to_owned()
        }
        Err(error) => {
            let resync = registration.resync.get_or_insert_with(Resync::default);
            resync.failures = resync.failures.saturating_add(1);
            if resync.failures < RESYNC_ATTEMPTS {
                return;
            }
            format!(
                "could not read the OpenCode session again after reconnecting: {}",
                error.message
            )
        }
    };
    end_registration(registrations, &completed.id, Some(reason));
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
) -> RegistrationEvents {
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
    let mut overflowed = None;
    if let Some(session_id) = wire::session_id_of(&value)
        && let Some(registration) = registrations.get_mut(&session_id)
    {
        registration.event_revision = registration.event_revision.wrapping_add(1);
        if let Some(agent) = wire::agent_switched(&value, &session_id) {
            if registration.modes.current.as_deref() != Some(agent.as_str()) {
                registration.modes.current = Some(agent);
                // While the session is read again, its modes are reported
                // with that state.
                if registration.resync.is_none() {
                    registration.publish_modes();
                }
            }
        } else if let Some(resync) = registration.resync.as_mut() {
            if let Some(event) = wire::event_for(
                &value,
                &session_id,
                &mut registration
                    .seen
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            ) {
                if resync.events.len() < MAX_RESYNC_EVENTS {
                    resync.events.push(event);
                } else {
                    overflowed = Some(session_id);
                }
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
            if let Some(running) = running {
                if registration.deliver_state(event) {
                    registration.running = running;
                }
            } else {
                registration.events.deliver(event);
            }
        }
    }
    if let Some(session_id) = overflowed {
        end_registration(
            registrations,
            &session_id,
            Some(
                "too many OpenCode events arrived while the session was read again after reconnecting"
                    .to_owned(),
            ),
        );
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
        let parent = data.get("parentID").and_then(Value::as_str);
        if child.len() > MAX_WORKER_ID_BYTES
            || parent.is_some_and(|parent| parent.len() > MAX_WORKER_ID_BYTES)
        {
            let relevant = parent == Some(root.as_str())
                || parent.is_some_and(|parent| registration.children.contains_key(parent))
                || registration.children.contains_key(child);
            if relevant {
                registration.mark_workers_incomplete();
            }
            continue;
        }
        let discovering = registration.discovering_workers();
        if !discovering {
            registration.clear_bootstrap_journals();
        }
        if discovering && !registration.children.contains_key(child) {
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
                        change.1 = Some(bounded_worker_title(title));
                    }
                } else {
                    registration.mark_workers_incomplete();
                }
            }
        }
        let changed = match kind {
            "session.created" => {
                if child == root || parent == Some(child) {
                    false
                } else if parent != Some(root.as_str())
                    && !parent.is_some_and(|id| registration.children.contains_key(id))
                {
                    if discovering && parent.is_some() {
                        if registration.initial_creations.contains_key(child)
                            || registration.initial_creations.len() < api::MAX_TRACKED_WORKERS
                        {
                            registration.initial_deletions.remove(child);
                            registration.snapshot_deletions.remove(child);
                            registration.initial_creations.insert(
                                child.to_owned(),
                                Worker {
                                    id: child.to_owned(),
                                    parent: parent.map(str::to_owned),
                                    title: data
                                        .get("title")
                                        .and_then(Value::as_str)
                                        .map(bounded_worker_title)
                                        .unwrap_or_else(|| "worker".to_owned()),
                                    state: WorkerState::Unknown,
                                },
                            );
                        } else {
                            registration.mark_workers_incomplete();
                        }
                    }
                    false
                } else if registration.children.contains_key(child) {
                    false
                } else if registration.children.len() >= api::MAX_TRACKED_WORKERS {
                    registration.mark_workers_incomplete();
                    false
                } else {
                    registration.initial_deletions.remove(child);
                    registration.initial_changes.remove(child);
                    registration.snapshot_deletions.remove(child);
                    registration.children.insert(
                        child.to_owned(),
                        Worker {
                            id: child.to_owned(),
                            parent: parent.map(str::to_owned),
                            title: data
                                .get("title")
                                .and_then(Value::as_str)
                                .map(bounded_worker_title)
                                .unwrap_or_else(|| "worker".to_owned()),
                            state: WorkerState::Unknown,
                        },
                    );
                    true
                }
            }
            "session.deleted" => {
                let removed = registration.remove_worker(child);
                if discovering {
                    // A delete event has no parent ID. Before bootstrap, keep
                    // even an unknown ID: a stale page may later list it.
                    if registration.initial_deletions.contains(child)
                        || registration.initial_deletions.len() < api::MAX_TRACKED_WORKERS
                    {
                        registration.initial_deletions.insert(child.to_owned());
                    } else {
                        registration.mark_workers_incomplete();
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
                (Some(worker), Some(title)) => {
                    let title = bounded_worker_title(title);
                    if worker.title == title {
                        false
                    } else {
                        worker.title = title;
                        true
                    }
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

fn close_registration(
    registrations: &mut HashMap<String, Registration>,
    id: &str,
    identity: &Arc<AtomicU64>,
) {
    if registrations
        .get(id)
        .is_some_and(|registration| Arc::ptr_eq(&registration.events.worker_epoch, identity))
    {
        end_registration(registrations, id, None);
    }
}

/// Remove `id`'s registration, cancel its work and tell its session why.
fn end_registration(
    registrations: &mut HashMap<String, Registration>,
    id: &str,
    reason: Option<String>,
) {
    if let Some(registration) = registrations.remove(id) {
        if let Some(cancellation) = registration.snapshot_in_flight {
            cancellation.cancel();
        }
        if let Some(cancellation) = registration.reconcile_in_flight {
            cancellation.cancel();
        }
        registration.events.deliver(Event::Closed { reason });
    }
}

/// A create/attach reply can be abandoned before the caller obtains its
/// handle. Reap only registrations whose event receiver no longer exists.
fn close_abandoned_registration(registrations: &mut HashMap<String, Registration>, id: &str) {
    let identity = registrations.get(id).and_then(|registration| {
        (registration.events.tx.receiver_count() == 0)
            .then(|| registration.events.worker_epoch.clone())
    });
    if let Some(identity) = identity {
        close_registration(registrations, id, &identity);
    }
}

fn owns_registration(
    registrations: &HashMap<String, Registration>,
    id: &str,
    identity: &Arc<AtomicU64>,
    reply: &async_channel::Sender<backend::Result<()>>,
) -> bool {
    if registrations
        .get(id)
        .is_some_and(|registration| Arc::ptr_eq(&registration.events.worker_epoch, identity))
    {
        true
    } else {
        let _ = reply.try_send(Err(backend::Error::new(
            backend::ErrorKind::Closed,
            "The session registration has closed.",
        )));
        false
    }
}

/// The sessions a participant can attach to. The listing asks for top-level,
/// unarchived sessions; a child session (a subagent's worker) or an archived
/// one that a server returns anyway is left out.
fn session_summaries(sessions: &[Value]) -> Vec<SessionSummary> {
    sessions
        .iter()
        .filter(|info| info.get("parentID").is_none_or(Value::is_null))
        .filter(|info| info.pointer("/time/archived").is_none_or(Value::is_null))
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
        Command::Close { id, identity } => {
            close_registration(registrations, &id, &identity);
            // Keep the shared connection available for other registrations and
            // for a later retry after the last one closes.
            true
        }
        Command::Sessions { directory, reply } => {
            let result = api::list_sessions(client, resolved, &directory, cancellation)
                .await
                .map(|sessions| session_summaries(&sessions))
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
                        close_abandoned_registration(registrations, id);
                        if registrations.contains_key(id) {
                            let _ = reply.try_send(Err(backend::Error::new(
                                backend::ErrorKind::Unavailable,
                                "The server returned a session that is already attached.",
                            )));
                            return true;
                        }
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
            close_abandoned_registration(registrations, &id);
            if registrations.contains_key(&id) {
                let _ = reply.try_send(Err(backend::Error::new(
                    backend::ErrorKind::Unavailable,
                    "That OpenCode session is already attached; join its existing space.",
                )));
                return true;
            }
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
            identity,
            key,
            text,
            reply,
        } => {
            if !owns_registration(registrations, &id, &identity, &reply) {
                return true;
            }
            let result = api::send_prompt(client, resolved, &id, &key, &text, cancellation)
                .await
                .map_err(map_failure);
            reply.send(result).await.ok();
            true
        }
        Command::SetMode {
            id,
            identity,
            mode,
            reply,
        } => {
            if !owns_registration(registrations, &id, &identity, &reply) {
                return true;
            }
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
        Command::Interrupt {
            id,
            identity,
            reply,
        } => {
            if !owns_registration(registrations, &id, &identity, &reply) {
                return true;
            }
            let result = api::interrupt(client, resolved, &id, cancellation)
                .await
                .map_err(map_failure);
            reply.send(result).await.ok();
            true
        }
        Command::Decide {
            id,
            identity,
            request,
            decision,
            reply,
        } => {
            if !owns_registration(registrations, &id, &identity, &reply) {
                return true;
            }
            let result =
                api::reply_permission(client, resolved, &id, &request, &decision, cancellation)
                    .await
                    .map_err(map_failure);
            reply.send(result).await.ok();
            true
        }
        Command::Answer {
            id,
            identity,
            form,
            answer,
            reply,
        } => {
            if !owns_registration(registrations, &id, &identity, &reply) {
                return true;
            }
            let result = api::reply_form(client, resolved, &id, &form, &answer, cancellation)
                .await
                .map_err(map_failure);
            reply.send(result).await.ok();
            true
        }
    }
}

async fn snapshot<S: Timers>(
    client: &Client<'_>,
    resolved: &Resolved,
    id: &str,
    modes: Modes,
    cancellation: &Cancellation,
    timers: &S,
) -> std::result::Result<Snapshot, Failure> {
    let (parts, active) = futures_lite::future::zip(
        api::SnapshotParts::fetch(client, resolved, id, cancellation),
        api::active_sessions_with_timeout(
            client,
            resolved,
            cancellation,
            timers,
            Duration::from_secs(1),
        ),
    )
    .await;
    let parts = parts?;
    let messages = snapshot_messages(&parts.messages);
    let busy = match &active {
        Some(active) if active.contains(id) => Some(true),
        Some(_) if !parts.has_parent => Some(false),
        _ => parts.busy,
    };
    let mut workers = parts.workers();
    for worker in &mut workers {
        if active
            .as_ref()
            .is_some_and(|active| active.contains(&worker.id))
        {
            worker.state = WorkerState::Running;
        } else if worker.state == WorkerState::Running {
            // Timestamps persist across process restarts, and a background
            // child may be absent from the foreground-drain listing. Without
            // positive evidence, retain the worker but do not claim it runs.
            worker.state = WorkerState::Unknown;
        }
    }
    Ok(Snapshot {
        modes,
        busy,
        messages,
        permissions: parts.permissions(),
        questions: parts.questions(),
        workers,
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

    #[test]
    fn subagent_sessions_are_not_offered_for_attachment() {
        let sessions = [
            json!({"id":"root","title":"Support","location":{"directory":"/tmp"}}),
            json!({"id":"explicit","parentID":null,"title":"Explicit root"}),
            json!({"id":"child","parentID":"root","title":"Review commit"}),
            json!({"id":"archived","title":"Old","time":{"created":1,"archived":2}}),
            json!({"title":"No identifier"}),
        ];
        let summaries = session_summaries(&sessions);
        let ids: Vec<_> = summaries
            .iter()
            .map(|session| session.id.as_str())
            .collect();
        assert_eq!(ids, ["root", "explicit"]);
        assert_eq!(summaries[0].title, "Support");
        assert_eq!(summaries[0].directory, "/tmp");
    }

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
        assert_eq!(
            registrations["parent"].children["child"].state,
            WorkerState::Unknown
        );
        let started = json!({"type":"session.execution.started","data":{"sessionID":"child"}});
        apply_worker_event(&started, &mut registrations);
        assert_eq!(
            registrations["parent"].children["child"].state,
            WorkerState::Running
        );
        assert!(registrations["other"].children.is_empty());
        assert!(wire::event_for(&event, "parent", &mut HashSet::new()).is_none());
    }

    #[test]
    fn live_worker_titles_are_utf8_bounded_and_oversized_ids_are_not_guessed() {
        let mut registrations = HashMap::new();
        let parent_events = insert_registration(
            &mut registrations,
            "parent".into(),
            "/tmp",
            Modes::default(),
        );
        let _other_events =
            insert_registration(&mut registrations, "other".into(), "/tmp", Modes::default());
        for id in ["parent", "other"] {
            let registration = registrations.get_mut(id).unwrap();
            registration.install_workers(&mut Vec::new()).unwrap();
            registration
                .events
                .snapshot_reply(Snapshot::default())
                .accept()
                .unwrap();
        }
        let title = "é".repeat(MAX_WORKER_TITLE_BYTES);
        apply_worker_event(
            &json!({"type":"session.created","data":{"sessionID":"child","parentID":"parent","title":title}}),
            &mut registrations,
        );
        let stored = &registrations["parent"].children["child"].title;
        assert!(stored.len() <= MAX_WORKER_TITLE_BYTES && stored.ends_with('…'));
        let oversized = "x".repeat(MAX_WORKER_ID_BYTES + 1);
        apply_worker_event(
            &json!({"type":"session.created","data":{"sessionID":oversized,"parentID":"parent"}}),
            &mut registrations,
        );
        assert!(registrations["parent"].workers_incomplete);
        assert!(!registrations["other"].workers_incomplete);
        assert!(matches!(
            parent_events.try_recv().unwrap().into_current(),
            Some(Event::WorkersUnavailable { .. })
        ));
    }

    #[test]
    fn a_snapshot_rejects_an_oversized_worker_identity() {
        let (mut registration, _events) = Registration::new();
        let mut workers = vec![Worker {
            id: "x".repeat(MAX_WORKER_ID_BYTES + 1),
            parent: Some("parent".into()),
            title: "Worker".into(),
            state: WorkerState::Running,
        }];
        assert!(registration.install_workers(&mut workers).is_err());
        assert!(!registration.workers_ready);
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
    fn a_full_ordinary_channel_does_not_block_worker_updates() {
        let (mut registration, events) = Registration::new();
        registration.workers_ready = true;
        for _ in 0..1024 {
            assert!(registration.events.deliver(Event::Busy));
        }
        registration.publish_workers();
        assert!(!registration.workers_dirty);
        assert_eq!(events.len(), 1025);
        let mut workers = 0;
        let mut ordinary = 0;
        while let Ok(event) = events.try_recv() {
            match event.into_current() {
                Some(Event::WorkersChanged { workers: list }) if list.is_empty() => workers += 1,
                Some(Event::Busy) => ordinary += 1,
                _ => panic!("unexpected event"),
            }
        }
        assert_eq!(workers, 1);
        assert_eq!(ordinary, 1024);
    }

    #[test]
    fn a_lost_ordinary_event_closes_the_channel_visibly() {
        let (events, receiver) = Events::new();
        let snapshot = events.snapshot_reply(Snapshot::default());
        for _ in 0..1024 {
            assert!(events.deliver(Event::Busy));
        }
        assert!(!events.deliver(Event::Error {
            message: "could not deliver".into()
        }));
        assert!(snapshot.accept().is_err());
        assert!(matches!(
            receiver.try_recv(),
            Err(async_channel::TryRecvError::Closed)
        ));
    }

    #[test]
    fn ordinary_event_loss_after_snapshot_is_reported_without_draining_stale_events() {
        use backend::Session;
        let (events, receiver) = Events::new();
        events.snapshot_reply(Snapshot::default()).accept().unwrap();
        for _ in 0..1024 {
            assert!(events.deliver(Event::Busy));
        }
        assert!(!events.deliver(Event::Error {
            message: "lost".into()
        }));
        let (commands, _receiver) = Commands::new();
        let session = crate::session::OpenCodeSession::new("parent".into(), receiver, commands);
        assert!(
            matches!(block_on(session.next_event()), Err(error) if error.kind == backend::ErrorKind::Unavailable)
        );
    }

    #[test]
    fn mode_and_running_changes_retry_when_ordinary_events_are_full() {
        let (mut registration, events) = Registration::new();
        let mut initial = Vec::new();
        registration.install_workers(&mut initial).unwrap();
        registration
            .events
            .snapshot_reply(Snapshot::default())
            .accept()
            .unwrap();
        let mut waiting = Box::pin(events.recv());
        assert!(block_on(futures_lite::future::poll_once(waiting.as_mut())).is_none());
        drop(waiting);
        registration.running = RunningState::Running;
        registration.modes.current = Some("plan".into());
        for _ in 0..1024 {
            assert!(registration.events.deliver(Event::Busy));
        }
        registration.publish_modes();
        registration.reconcile_running(RunningState::Idle);
        assert!(registration.modes_dirty);
        assert!(matches!(registration.running, RunningState::Running));
        while events.try_recv().is_ok() {}
        registration.publish_modes();
        registration.reconcile_running(RunningState::Idle);
        assert!(!registration.modes_dirty);
        assert!(matches!(registration.running, RunningState::Idle));
        assert!(
            matches!(events.try_recv().unwrap().into_current(), Some(Event::ModesChanged(modes)) if modes.current.as_deref() == Some("plan"))
        );
        assert!(matches!(
            events.try_recv().unwrap().into_current(),
            Some(Event::Idle)
        ));
    }

    #[test]
    fn a_lost_busy_or_mode_event_refuses_an_unaccepted_snapshot() {
        for kind in ["busy", "mode"] {
            let mut registrations = HashMap::new();
            let events = insert_registration(
                &mut registrations,
                "parent".into(),
                "/tmp",
                Modes::default(),
            );
            let registration = registrations.get_mut("parent").unwrap();
            let mut workers = Vec::new();
            registration.install_workers(&mut workers).unwrap();
            let snapshot = registration.events.snapshot_reply(Snapshot::default());
            for _ in 0..1024 {
                assert!(registration.events.deliver(Event::Busy));
            }
            let event = match kind {
                "busy" => json!({"type":"session.execution.started","data":{"sessionID":"parent"}}),
                "mode" => json!({"type":"session.agent.selected","data":{"sessionID":"parent","agent":"plan"}}),
                _ => unreachable!(),
            }.to_string();
            dispatch_events(&event, &mut registrations);
            assert!(snapshot.accept().is_err(), "{kind}");
            assert!(matches!(
                events.try_recv(),
                Err(async_channel::TryRecvError::Closed)
            ));
        }
    }

    #[test]
    fn a_lost_state_event_after_snapshot_acceptance_closes_before_consumption() {
        let mut registrations = HashMap::new();
        let events = insert_registration(
            &mut registrations,
            "parent".into(),
            "/tmp",
            Modes::default(),
        );
        let registration = registrations.get_mut("parent").unwrap();
        registration.install_workers(&mut Vec::new()).unwrap();
        registration
            .events
            .snapshot_reply(Snapshot::default())
            .accept()
            .unwrap();
        for _ in 0..1024 {
            assert!(registration.events.deliver(Event::Busy));
        }
        let event =
            json!({"type":"session.execution.started","data":{"sessionID":"parent"}}).to_string();
        dispatch_events(&event, &mut registrations);
        assert!(matches!(
            events.try_recv(),
            Err(async_channel::TryRecvError::Closed)
        ));
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
        registration.remove_worker("child");
        let mut later = Vec::new();
        registration.install_workers(&mut later).unwrap();
        registration
            .events
            .snapshot_reply(Snapshot::default())
            .accept()
            .unwrap();
        assert!(later.is_empty());
        assert!(events.try_recv().unwrap().into_current().is_none());
        assert!(matches!(
            events.try_recv().unwrap().into_current(),
            Some(Event::Busy)
        ));
    }

    #[test]
    fn a_new_worker_update_survives_an_accepted_snapshot() {
        let (mut registration, events) = Registration::new();
        let mut initial = vec![Worker {
            id: "child".into(),
            parent: Some("parent".into()),
            title: "Worker".into(),
            state: WorkerState::Running,
        }];
        registration.install_workers(&mut initial).unwrap();
        registration
            .events
            .snapshot_reply(Snapshot::default())
            .accept()
            .unwrap();
        registration.publish_workers();
        let mut snapshot = Vec::new();
        registration.install_workers(&mut snapshot).unwrap();
        registration
            .events
            .snapshot_reply(Snapshot::default())
            .accept()
            .unwrap();
        registration.remove_worker("child");
        registration.publish_workers();
        assert!(
            matches!(events.try_recv().unwrap().into_current(), Some(Event::WorkersChanged { workers }) if workers.is_empty())
        );
    }

    #[test]
    fn an_older_snapshot_reply_is_rejected_after_a_newer_one() {
        let (mut registration, _events) = Registration::new();
        let mut initial = Vec::new();
        registration.install_workers(&mut initial).unwrap();
        let older = registration.events.snapshot_reply(Snapshot::default());
        let mut later = Vec::new();
        registration.install_workers(&mut later).unwrap();
        let newer = registration.events.snapshot_reply(Snapshot::default());
        assert!(newer.accept().is_ok());
        assert!(older.accept().is_err());
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
        events.snapshot_reply(Snapshot::default()).accept().unwrap();
        assert!(matches!(block_on(session.next_event()), Ok(Event::Busy)));
    }

    #[test]
    fn a_closed_session_does_not_deliver_later_events() {
        use backend::Session;
        let (events, rx) = Events::new();
        let (commands, _receiver) = Commands::new();
        let session = crate::session::OpenCodeSession::new("parent".into(), rx, commands);
        assert!(events.deliver(Event::Closed { reason: None }));
        assert!(events.deliver(Event::Busy));
        assert!(matches!(
            block_on(session.next_event()),
            Ok(Event::Closed { .. })
        ));
        assert!(
            matches!(block_on(session.next_event()), Err(error) if error.kind == backend::ErrorKind::Closed)
        );
    }

    #[test]
    fn closing_a_session_is_nonblocking_and_identity_scoped() {
        use backend::Session;
        let (_registration, rx) = Registration::new();
        let expected = rx.identity.clone();
        let (commands, receiver) = Commands::new();
        let session = crate::session::OpenCodeSession::new("parent".into(), rx, commands);
        for _ in 0..2 {
            assert!(block_on(session.close()).is_ok());
            match receiver.try_recv().unwrap() {
                Command::Close { id, identity } => {
                    assert_eq!(id, "parent");
                    assert!(Arc::ptr_eq(&identity, &expected));
                }
                _ => panic!("expected close"),
            }
        }
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn closing_a_session_does_not_wait_for_a_full_command_queue() {
        use backend::Session;
        let (_registration, rx) = Registration::new();
        let identity = rx.identity.clone();
        let (commands, _receiver) = Commands::new();
        for _ in 0..256 {
            assert!(
                commands
                    .tx()
                    .try_send(Command::Close {
                        id: "parent".into(),
                        identity: identity.clone()
                    })
                    .is_ok()
            );
        }
        let session = crate::session::OpenCodeSession::new("parent".into(), rx, commands);
        assert!(
            matches!(block_on(session.close()), Err(error) if error.kind == backend::ErrorKind::Unavailable)
        );
    }

    #[test]
    fn abandoning_a_snapshot_does_not_discard_a_queued_worker_update() {
        let (mut registration, events) = Registration::new();
        let child = Worker {
            id: "child".into(),
            parent: Some("parent".into()),
            title: "Worker".into(),
            state: WorkerState::Completed,
        };
        let mut initial = vec![child];
        registration.install_workers(&mut initial).unwrap();
        registration
            .events
            .snapshot_reply(Snapshot::default())
            .accept()
            .unwrap();
        registration.publish_workers();
        registration.events.deliver(Event::Busy);
        registration.remove_worker("child");
        registration.publish_workers();

        // The reply can be queued successfully yet dropped before its caller
        // consumes it. It must not invalidate the coalesced worker event.
        let mut later = Vec::new();
        registration.install_workers(&mut later).unwrap();
        let (reply, waiting) = async_channel::bounded(1);
        assert!(
            reply
                .try_send(registration.events.snapshot_reply(Snapshot::default()))
                .is_ok()
        );
        drop(waiting);
        assert!(
            matches!(events.try_recv().unwrap().into_current(), Some(Event::WorkersChanged { workers }) if workers.is_empty())
        );
        assert!(matches!(
            events.try_recv().unwrap().into_current(),
            Some(Event::Busy)
        ));
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn a_worker_burst_coalesces_to_one_bounded_latest_update() {
        let (mut registration, events) = Registration::new();
        let mut initial = Vec::new();
        registration.install_workers(&mut initial).unwrap();
        registration
            .events
            .snapshot_reply(Snapshot::default())
            .accept()
            .unwrap();
        for index in 0..1000 {
            let id = format!("worker-{index}");
            registration.children.insert(
                id.clone(),
                Worker {
                    id: id.clone(),
                    parent: Some("parent".into()),
                    title: "Worker".into(),
                    state: WorkerState::Running,
                },
            );
            registration.publish_workers();
            registration.children.remove(&id);
        }
        registration.publish_workers();
        assert_eq!(events.len(), 1);
        assert!(
            matches!(events.try_recv().unwrap().into_current(), Some(Event::WorkersChanged { workers }) if workers.is_empty())
        );
        assert!(!registration.workers_dirty);
    }

    #[test]
    fn an_abandoned_snapshot_preserves_workers_when_ordinary_events_are_full() {
        let (mut registration, events) = Registration::new();
        let mut initial = vec![Worker {
            id: "child".into(),
            parent: Some("parent".into()),
            title: "Worker".into(),
            state: WorkerState::Running,
        }];
        registration.install_workers(&mut initial).unwrap();
        registration
            .events
            .snapshot_reply(Snapshot::default())
            .accept()
            .unwrap();
        for _ in 0..1024 {
            assert!(registration.events.deliver(Event::Busy));
        }
        registration.remove_worker("child");
        registration.publish_workers();
        assert!(!registration.workers_dirty);
        let mut later = Vec::new();
        registration.install_workers(&mut later).unwrap();
        drop(registration.events.snapshot_reply(Snapshot::default()));
        assert!(
            matches!(events.try_recv().unwrap().into_current(), Some(Event::WorkersChanged { workers }) if workers.is_empty())
        );
        assert_eq!(events.len(), 1024);
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
        registration
            .events
            .snapshot_reply(Snapshot::default())
            .accept()
            .unwrap();
        // A later snapshot began listing before the deletion was observed.
        registration.snapshot_in_flight = Some(Cancellation::new());
        registration.remove_worker("child");
        let mut stale = vec![child];
        registration.install_workers(&mut stale).unwrap();
        assert!(stale.is_empty());
    }

    #[test]
    fn a_later_snapshot_discovers_a_missed_worker_but_not_a_deleted_one() {
        let (mut registration, _events) = Registration::new();
        let worker = |id: &str| Worker {
            id: id.into(),
            parent: Some("parent".into()),
            title: id.into(),
            state: WorkerState::Running,
        };
        let mut initial = vec![worker("deleted")];
        registration.install_workers(&mut initial).unwrap();
        registration
            .events
            .snapshot_reply(Snapshot::default())
            .accept()
            .unwrap();
        // Outside a snapshot window, a deletion leaves no retained record.
        let mut known = vec![worker("gone")];
        registration.install_workers(&mut known).unwrap();
        assert!(registration.remove_worker("gone"));
        assert!(registration.snapshot_deletions.is_empty());
        registration.snapshot_in_flight = Some(Cancellation::new());
        assert!(registration.remove_worker("deleted"));
        let mut later = vec![worker("missed"), worker("deleted")];
        registration.install_workers(&mut later).unwrap();
        assert_eq!(
            later
                .iter()
                .map(|worker| worker.id.as_str())
                .collect::<Vec<_>>(),
            ["missed"]
        );
    }

    #[test]
    fn an_uncached_deletion_during_a_later_snapshot_is_not_resurrected() {
        let mut registrations = registrations();
        let parent = registrations.get_mut("parent").unwrap();
        parent.install_workers(&mut Vec::new()).unwrap();
        parent
            .events
            .snapshot_reply(Snapshot::default())
            .accept()
            .unwrap();
        parent.snapshot_in_flight = Some(Cancellation::new());
        let deleted = json!({"type":"session.deleted","data":{"sessionID":"listed"}});
        apply_worker_event(&deleted, &mut registrations);
        let parent = registrations.get_mut("parent").unwrap();
        let mut stale = vec![Worker {
            id: "listed".into(),
            parent: Some("parent".into()),
            title: "Listed".into(),
            state: WorkerState::Running,
        }];
        parent.install_workers(&mut stale).unwrap();
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
    fn retry_after_an_abandoned_bootstrap_discovers_workers_without_resurrecting_deletions() {
        let mut registrations = registrations();
        let parent = registrations.get_mut("parent").unwrap();
        let mut initial = Vec::new();
        parent.install_workers(&mut initial).unwrap();
        drop(parent.events.snapshot_reply(Snapshot::default()));

        let deleted = json!({"type":"session.deleted","data":{"sessionID":"deleted"}});
        apply_worker_event(&deleted, &mut registrations);
        let worker = |id: &str| Worker {
            id: id.into(),
            parent: Some("parent".into()),
            title: id.into(),
            state: WorkerState::Running,
        };
        let parent = registrations.get_mut("parent").unwrap();
        let mut retried = vec![worker("existing"), worker("deleted")];
        parent.install_workers(&mut retried).unwrap();
        assert_eq!(
            retried
                .iter()
                .map(|worker| worker.id.as_str())
                .collect::<Vec<_>>(),
            ["existing"]
        );
        parent
            .events
            .snapshot_reply(Snapshot::default())
            .accept()
            .unwrap();
        let mut stale = vec![worker("deleted")];
        parent.install_workers(&mut stale).unwrap();
        assert_eq!(
            stale
                .iter()
                .map(|worker| worker.id.as_str())
                .collect::<Vec<_>>(),
            ["existing"]
        );
    }

    #[test]
    fn bootstrap_journal_overflow_reports_uncertainty_before_or_after_acceptance() {
        for kind in ["deletion", "change", "creation"] {
            let mut registrations = HashMap::new();
            let events = insert_registration(
                &mut registrations,
                "parent".into(),
                "/tmp",
                Modes::default(),
            );
            let registration = registrations.get_mut("parent").unwrap();
            let mut workers = Vec::new();
            registration.install_workers(&mut workers).unwrap();
            let snapshot = registration.events.snapshot_reply(Snapshot::default());
            for index in 0..api::MAX_TRACKED_WORKERS {
                let id = format!("worker-{index}");
                match kind {
                    "deletion" => {
                        registration.initial_deletions.insert(id);
                    }
                    "change" => {
                        registration
                            .initial_changes
                            .insert(id, (Some(WorkerState::Running), None));
                    }
                    "creation" => {
                        registration.initial_creations.insert(
                            id.clone(),
                            Worker {
                                id,
                                parent: Some("unknown".into()),
                                title: "Worker".into(),
                                state: WorkerState::Unknown,
                            },
                        );
                    }
                    _ => unreachable!(),
                }
            }
            let event = match kind {
                "deletion" => json!({"type":"session.deleted","data":{"sessionID":"overflow"}}),
                "change" => {
                    json!({"type":"session.execution.started","data":{"sessionID":"overflow"}})
                }
                "creation" => {
                    json!({"type":"session.created","data":{"sessionID":"overflow","parentID":"unknown"}})
                }
                _ => unreachable!(),
            };
            apply_worker_event(&event, &mut registrations);
            assert!(snapshot.accept().is_err());
            assert!(
                matches!(
                    events.try_recv().unwrap().into_current(),
                    Some(Event::WorkersUnavailable { .. })
                ),
                "{kind}"
            );
        }
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
        close_registration(&mut registrations, "first", &first.identity);
        assert!(matches!(
            first.try_recv().unwrap().into_current(),
            Some(Event::Closed { .. })
        ));
        assert!(second.try_recv().is_err());
        assert!(registrations.contains_key("second"));
        let replacement =
            insert_registration(&mut registrations, "first".into(), "/tmp", Modes::default());
        // A late close from the old handle cannot remove the replacement.
        close_registration(&mut registrations, "first", &first.identity);
        assert!(registrations.contains_key("first"));
        assert!(replacement.try_recv().is_err());
        close_registration(&mut registrations, "first", &replacement.identity);
        close_registration(&mut registrations, "second", &second.identity);
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
        let stall_active = Arc::new(AtomicBool::new(false));
        let server_stall_active = stall_active.clone();
        let delay_active = Arc::new(AtomicBool::new(false));
        let server_delay_active = delay_active.clone();
        let (event_tx, event_rx) = mpsc::channel::<String>();
        let event_rx = Arc::new(Mutex::new(event_rx));
        let (stalled_tx, stalled_rx) = mpsc::channel();
        let (reconcile_tx, reconcile_rx) = mpsc::channel();
        let (active_tx, active_rx) = mpsc::channel();
        let (delayed_tx, delayed_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            while !server_stopped.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        let stopped = server_stopped.clone();
                        let stall_reconcile = server_stall_reconcile.clone();
                        let stall_active = server_stall_active.clone();
                        let delay_active = server_delay_active.clone();
                        let event_rx = event_rx.clone();
                        let stalled = stalled_tx.clone();
                        let reconcile = reconcile_tx.clone();
                        let active = active_tx.clone();
                        let delayed = delayed_tx.clone();
                        std::thread::spawn(move || {
                            // macOS accepted sockets inherit the listener's nonblocking mode.
                            socket.set_nonblocking(false).unwrap();
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
                            } else if path == "/api/session/active"
                                && stall_active.load(Ordering::Acquire)
                            {
                                let _ = write!(
                                    socket,
                                    "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{{"
                                );
                                let _ = socket.flush();
                                let _ = active.send(());
                                while !stopped.load(Ordering::Acquire) {
                                    std::thread::sleep(Duration::from_millis(20));
                                }
                            } else {
                                let body = if path.starts_with("/api/agent?") {
                                    r#"{"data":[]}"#.to_owned()
                                } else if path == "/api/session/active" {
                                    if delay_active.load(Ordering::Acquire) {
                                        let _ = delayed.send(());
                                        std::thread::sleep(Duration::from_millis(1500));
                                    }
                                    r#"{"data":{"child":{"type":"running"}}}"#.to_owned()
                                } else if path.starts_with("/api/session?parentID=wide&") {
                                    let children: Vec<_> = (0..60).map(|index| json!({
                                        "id": format!("wide-child-{index}"), "parentID":"wide", "projectID":"p", "cost":0,
                                        "tokens":{"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}},
                                        "time":{"created":1,"updated":1}, "location":{"directory":"/tmp"}
                                    })).collect();
                                    json!({"data":children,"cursor":{"next":null}}).to_string()
                                } else if path.starts_with("/api/session?parentID=wide-child-") {
                                    std::thread::sleep(Duration::from_millis(100));
                                    r#"{"data":[],"cursor":{"next":null}}"#.to_owned()
                                } else if path.starts_with("/api/session?parentID=stale&") {
                                    json!({"data":[{"id":"stale-child","parentID":"stale","projectID":"p","cost":0,"tokens":{"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}},"outcome":"succeeded","time":{"created":1,"updated":1,"busy":20,"idle":10},"location":{"directory":"/tmp"}}],"cursor":{"next":null}}).to_string()
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
                                } else if path == "/api/session/fast/message?limit=100&order=desc" {
                                    r#"{"data":[{"id":"second","type":"user","text":"Second"}],"cursor":{"next":"next+page"}}"#.to_owned()
                                } else if path == "/api/session/long/message?limit=100&order=desc" {
                                    let messages: Vec<_> = (0..100).map(|index| json!({
                                        "id":format!("message-{index:03}"), "type":"user", "text":format!("Message {index}")
                                    })).collect();
                                    json!({"data":messages,"cursor":{"next":"older-page"}})
                                        .to_string()
                                } else if path
                                    == "/api/session/fast/message?limit=100&cursor=next%2Bpage"
                                {
                                    r#"{"data":[{"id":"first","type":"user","text":"First"}],"cursor":{"next":null}}"#.to_owned()
                                } else if path.starts_with("/api/session/duplicate/message?") {
                                    r#"{"data":[{"id":"same","type":"user","text":"a"},{"id":"same","type":"user","text":"b"}],"cursor":{"next":null}}"#.to_owned()
                                } else if path.starts_with("/api/session/bad-message/message?") {
                                    r#"{"data":[{"id":"message"}],"cursor":{"next":null}}"#
                                        .to_owned()
                                } else if path.contains("/message?") {
                                    r#"{"data":[],"cursor":{"next":null}}"#.to_owned()
                                } else if path == "/api/session/bad-permission/permission"
                                    || path == "/api/session/bad-form/form"
                                {
                                    r#"{"invalid":[]}"#.to_owned()
                                } else if path == "/api/session/bad-field/form" {
                                    r#"{"data":[{"id":"form","sessionID":"bad-field","title":"Question","fields":[{"key":"good","type":"string"},{"type":"string"}]}]}"#.to_owned()
                                } else if path == "/api/session/multi/permission" {
                                    r#"{"data":[{"id":"p1","sessionID":"multi","action":"read","resources":[]},{"id":"p2","sessionID":"multi","action":"write","resources":[]}]}"#.to_owned()
                                } else if path == "/api/session/multi/form" {
                                    r#"{"data":[{"id":"q1","sessionID":"multi","title":"First","fields":[{"key":"a","type":"string"}]},{"id":"q2","sessionID":"multi","title":"Second","fields":[{"key":"","type":"string"}]}]}"#.to_owned()
                                } else if path.ends_with("/permission") || path.ends_with("/form") {
                                    r#"{"data":[]}"#.to_owned()
                                } else if path == "/api/session/bad-record" {
                                    r#"{"data":{"id":"bad-record","location":{"directory":"/tmp"},"time":{}}}"#.to_owned()
                                } else if path == "/api/session/stale" {
                                    json!({"data":{"id":"stale","projectID":"p","cost":0,"tokens":{"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}},"location":{"directory":"/tmp"},"time":{"created":1,"updated":1,"busy":20,"idle":10}}}).to_string()
                                } else if path.starts_with("/api/session/") {
                                    let id = path.trim_start_matches("/api/session/");
                                    json!({"data":{"id":id,"projectID":"p","cost":0,"tokens":{"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}},"location":{"directory":"/tmp"},"time":{"created":1,"updated":1,"busy":0,"idle":1}}}).to_string()
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
        let connection =
            std::thread::spawn(move || run_thread(Box::new(move || resolved.clone()), receiver));
        let attach = |id: &str| {
            let id = id.to_owned();
            block_on(commands.reply_with(|reply| Command::Attach {
                id,
                directory: "/tmp".into(),
                reply,
            }))
            .unwrap()
        };
        let close = |id: &str, events: &RegistrationEvents| {
            let id = id.to_owned();
            let identity = events.identity.clone();
            assert!(
                commands
                    .tx()
                    .try_send(Command::Close { id, identity })
                    .is_ok()
            );
        };
        // If a caller abandons an attach, a later retry can reclaim the ID
        // both before and after the first reply reaches its channel.
        let (abandoned_reply, abandoned_wait) = async_channel::bounded(1);
        commands
            .tx()
            .try_send(Command::Attach {
                id: "abandoned-before".into(),
                directory: "/tmp".into(),
                reply: abandoned_reply,
            })
            .unwrap();
        drop(abandoned_wait);
        let recovered = attach("abandoned-before");
        close("abandoned-before", &recovered);
        let abandoned = attach("abandoned-after");
        drop(abandoned);
        let recovered = attach("abandoned-after");
        close("abandoned-after", &recovered);
        let slow_events = attach("slow");
        let fast_events = attach("fast");
        let duplicate = block_on(commands.reply_with(|reply| Command::Attach {
            id: "fast".into(),
            directory: "/tmp".into(),
            reply,
        }));
        assert!(
            duplicate.is_err(),
            "a duplicate attachment must not replace the first"
        );
        for id in [
            "bad-permission",
            "bad-form",
            "bad-field",
            "bad-message",
            "bad-record",
            "duplicate",
        ] {
            let events = attach(id);
            let (reply, waiting) = async_channel::bounded(1);
            commands
                .tx()
                .try_send(Command::Snapshot {
                    id: id.into(),
                    identity: events.identity.clone(),
                    reply,
                })
                .unwrap();
            assert!(
                block_on(waiting.recv()).unwrap().is_err(),
                "{id} snapshot should fail closed"
            );
            close(id, &events);
        }
        let multi_events = attach("multi");
        let (multi_reply, multi_wait) = async_channel::bounded(1);
        commands
            .tx()
            .try_send(Command::Snapshot {
                id: "multi".into(),
                identity: multi_events.identity.clone(),
                reply: multi_reply,
            })
            .unwrap();
        let multi = block_on(multi_wait.recv())
            .unwrap()
            .unwrap()
            .accept()
            .unwrap();
        assert_eq!(
            multi
                .permissions
                .iter()
                .map(|permission| permission.id.as_str())
                .collect::<Vec<_>>(),
            ["p1", "p2"]
        );
        assert_eq!(
            multi
                .questions
                .iter()
                .map(|question| question.id.as_str())
                .collect::<Vec<_>>(),
            ["q1", "q2"]
        );
        assert_eq!(multi.questions[1].fields[0].key, "");
        close("multi", &multi_events);
        // A session with additional older history is attachable: only the
        // recent bounded window is needed by the server's conversation model.
        let long_events = attach("long");
        let (long_reply, long_wait) = async_channel::bounded(1);
        commands
            .tx()
            .try_send(Command::Snapshot {
                id: "long".into(),
                identity: long_events.identity.clone(),
                reply: long_reply,
            })
            .unwrap();
        let long = block_on(long_wait.recv())
            .unwrap()
            .unwrap()
            .accept()
            .unwrap();
        assert_eq!(long.messages.len(), 100);
        assert_eq!(long.messages.first().unwrap().id, "message-099");
        assert_eq!(long.messages.last().unwrap().id, "message-000");
        close("long", &long_events);
        // A wide family with modest per-request latency must fit within the
        // deadline; serial leaf traversal would take more than five seconds.
        let wide_events = attach("wide");
        let (wide_reply, wide_wait) = async_channel::bounded(1);
        commands
            .tx()
            .try_send(Command::Snapshot {
                id: "wide".into(),
                identity: wide_events.identity.clone(),
                reply: wide_reply,
            })
            .unwrap();
        let wide = block_on(wide_wait.recv())
            .unwrap()
            .unwrap()
            .accept()
            .unwrap();
        assert_eq!(wide.workers.len(), 60);
        close("wide", &wide_events);
        let family_events = attach("family");
        let (family_reply, family_wait) = async_channel::bounded(1);
        commands
            .tx()
            .try_send(Command::Snapshot {
                id: "family".into(),
                identity: family_events.identity.clone(),
                reply: family_reply,
            })
            .unwrap();
        let family = block_on(family_wait.recv())
            .unwrap()
            .unwrap()
            .accept()
            .unwrap();
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
        assert_eq!(
            family
                .workers
                .iter()
                .find(|worker| worker.id == "child")
                .unwrap()
                .state,
            WorkerState::Running
        );
        close("family", &family_events);
        let stale_events = attach("stale");
        let (stale_reply, stale_wait) = async_channel::bounded(1);
        commands
            .tx()
            .try_send(Command::Snapshot {
                id: "stale".into(),
                identity: stale_events.identity.clone(),
                reply: stale_reply,
            })
            .unwrap();
        let stale = block_on(stale_wait.recv())
            .unwrap()
            .unwrap()
            .accept()
            .unwrap();
        assert_eq!(stale.busy, Some(false));
        assert_eq!(stale.workers[0].state, WorkerState::Unknown);
        // A slow but valid active response must win over the stale busy
        // timestamp rather than falsely making this idle session busy.
        while stale_events.try_recv().is_ok() {}
        delay_active.store(true, Ordering::Release);
        delayed_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("the slow active lookup did not start");
        let stale_watch = stale_events.clone();
        let (false_busy_tx, false_busy_wait) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(event) = block_on(stale_watch.recv()) {
                if matches!(event.into_current(), Some(Event::Busy)) {
                    let _ = false_busy_tx.send(());
                    return;
                }
            }
        });
        assert!(matches!(
            false_busy_wait.recv_timeout(Duration::from_millis(2200)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        delay_active.store(false, Ordering::Release);
        close("stale", &stale_events);
        let (fast_reply, fast_wait) = async_channel::bounded(1);
        commands
            .tx()
            .try_send(Command::Snapshot {
                id: "fast".into(),
                identity: fast_events.identity.clone(),
                reply: fast_reply,
            })
            .unwrap();
        let initial_fast = block_on(fast_wait.recv())
            .unwrap()
            .unwrap()
            .accept()
            .unwrap();
        assert_eq!(
            initial_fast
                .messages
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        // A stalled optional active-session lookup must not discard the
        // valid idle record used to recover a missed idle event.
        stall_active.store(true, Ordering::Release);
        let probe_events = attach("active-probe");
        let (probe_reply, probe_wait) = async_channel::bounded(1);
        commands
            .tx()
            .try_send(Command::Snapshot {
                id: "active-probe".into(),
                identity: probe_events.identity.clone(),
                reply: probe_reply,
            })
            .unwrap();
        active_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("active lookup did not stall");
        let busy_events = fast_events.clone();
        let (busy_tx, busy_wait) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(event) = block_on(busy_events.recv()) {
                if matches!(event.into_current(), Some(Event::Busy)) {
                    let _ = busy_tx.send(());
                    return;
                }
            }
        });
        event_tx
            .send(
                json!({"type":"session.execution.started","data":{"sessionID":"fast"}}).to_string(),
            )
            .unwrap();
        busy_wait
            .recv_timeout(Duration::from_secs(2))
            .expect("the busy event was not delivered");
        let idle_events = fast_events.clone();
        let (idle_tx, idle_wait) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(event) = block_on(idle_events.recv()) {
                if matches!(event.into_current(), Some(Event::Idle)) {
                    let _ = idle_tx.send(());
                    return;
                }
            }
        });
        idle_wait
            .recv_timeout(Duration::from_secs(4))
            .expect("idle reconciliation did not fall back to the session record");
        assert!(
            block_on(probe_wait.recv())
                .unwrap()
                .unwrap()
                .accept()
                .is_ok()
        );
        close("active-probe", &probe_events);
        stall_active.store(false, Ordering::Release);
        let (slow_reply, slow_wait) = async_channel::bounded(1);
        commands
            .tx()
            .try_send(Command::Snapshot {
                id: "slow".into(),
                identity: slow_events.identity.clone(),
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
                identity: fast_events.identity.clone(),
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
            .unwrap()
            .accept()
            .unwrap();
        assert!(
            fast_snapshot
                .workers
                .iter()
                .any(|worker| worker.id == "child")
        );
        close("slow", &slow_events);
        assert!(block_on(slow_wait.recv()).unwrap().is_err());
        let closed_rx = slow_events.clone();
        let (closed_tx, closed_wait) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(event) = block_on(closed_rx.recv()) {
                if matches!(event.into_current(), Some(Event::Closed { .. })) {
                    let _ = closed_tx.send(());
                    return;
                }
            }
        });
        closed_wait
            .recv_timeout(Duration::from_secs(2))
            .expect("the closed registration was not reported");
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
        let retried_events = attach("slow");
        // A delayed close or snapshot from the old handle must not affect
        // the replacement, even though both name the same OpenCode session.
        close("slow", &slow_events);
        let duplicate = block_on(commands.reply_with(|reply| Command::Attach {
            id: "slow".into(),
            directory: "/tmp".into(),
            reply,
        }));
        assert!(duplicate.is_err());
        let (stale_reply, stale_wait) = async_channel::bounded(1);
        commands
            .tx()
            .try_send(Command::Snapshot {
                id: "slow".into(),
                identity: slow_events.identity.clone(),
                reply: stale_reply,
            })
            .unwrap();
        assert!(block_on(stale_wait.recv()).unwrap().is_err());
        for operation in 0..5 {
            let identity = slow_events.identity.clone();
            let result = block_on(commands.reply_with(|reply| match operation {
                0 => Command::Prompt {
                    id: "slow".into(),
                    identity,
                    key: "old".into(),
                    text: "old".into(),
                    reply,
                },
                1 => Command::Interrupt {
                    id: "slow".into(),
                    identity,
                    reply,
                },
                2 => Command::Decide {
                    id: "slow".into(),
                    identity,
                    request: "old".into(),
                    decision: "reject".into(),
                    reply,
                },
                3 => Command::Answer {
                    id: "slow".into(),
                    identity,
                    form: "old".into(),
                    answer: Value::Null,
                    reply,
                },
                _ => Command::SetMode {
                    id: "slow".into(),
                    identity,
                    mode: "old".into(),
                    reply,
                },
            }));
            assert!(matches!(result, Err(error) if error.kind == backend::ErrorKind::Closed));
        }
        let (slow_reply, slow_wait) = async_channel::bounded(1);
        commands
            .tx()
            .try_send(Command::Snapshot {
                id: "slow".into(),
                identity: retried_events.identity.clone(),
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
            Some(Event::WorkersUnavailable { .. })
        ));
        let deleted = json!({"type":"session.deleted","data":{"sessionID":"worker-1"}});
        apply_worker_event(&deleted, &mut registrations);
        assert!(matches!(
            events.try_recv().unwrap().into_current(),
            Some(Event::WorkersUnavailable { .. })
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
                // macOS accepted sockets inherit the listener's nonblocking mode.
                socket.set_nonblocking(false).unwrap();
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
            let scheduler = NativeScheduler::with_options(NativeOptions {
                workers: 1,
                threads: 2,
                capacity: 256,
                ..Default::default()
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
                // macOS accepted sockets inherit the listener's nonblocking mode.
                socket.set_nonblocking(false).unwrap();
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
            let scheduler = NativeScheduler::with_options(NativeOptions {
                workers: 1,
                threads: 2,
                capacity: 256,
                ..Default::default()
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

    /// The state of a simulated OpenCode service that persists across its
    /// restarts, as OpenCode keeps sessions on disk.
    #[derive(Default)]
    struct FakeState {
        /// Existing sessions and their open permission requests.
        sessions: BTreeMap<String, Vec<Value>>,
        created: usize,
        /// Sessions whose message listing waits until they are removed.
        stalled: HashSet<String>,
        stopping: bool,
    }

    #[derive(Default)]
    struct FakeShared {
        state: Mutex<FakeState>,
        changed: std::sync::Condvar,
    }

    impl FakeShared {
        fn update(&self, change: impl FnOnce(&mut FakeState)) {
            change(&mut self.state.lock().unwrap());
            self.changed.notify_all();
        }
    }

    /// One run of the simulated service, on its own port, with its own password.
    struct FakeService {
        address: std::net::SocketAddr,
        stopped: Arc<AtomicBool>,
        streams: Arc<Mutex<Vec<std::net::TcpStream>>>,
        /// The authorization header of every request, in order.
        authorizations: Arc<Mutex<Vec<Option<String>>>>,
        accept: Option<std::thread::JoinHandle<()>>,
    }

    fn fake_record(id: &str) -> Value {
        json!({"id":id,"projectID":"p","cost":0,"tokens":{"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}},"location":{"directory":"/tmp"},"time":{"created":1,"updated":1,"busy":0,"idle":1}})
    }

    fn basic(password: &str) -> String {
        Resolved {
            host: "127.0.0.1".into(),
            port: 1,
            auth: Some(("opencode".into(), password.into())),
        }
        .authorization_header()
        .unwrap()
    }

    impl FakeService {
        fn start(shared: Arc<FakeShared>, password: &str) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let stopped = Arc::new(AtomicBool::new(false));
            let streams = Arc::new(Mutex::new(Vec::new()));
            let authorizations = Arc::new(Mutex::new(Vec::new()));
            let expected = basic(password);
            let accept = {
                let stopped = stopped.clone();
                let streams = streams.clone();
                let authorizations = authorizations.clone();
                std::thread::spawn(move || {
                    while !stopped.load(Ordering::Acquire) {
                        match listener.accept() {
                            Ok((socket, _)) => {
                                let shared = shared.clone();
                                let streams = streams.clone();
                                let authorizations = authorizations.clone();
                                let expected = expected.clone();
                                std::thread::spawn(move || {
                                    fake_exchange(
                                        socket,
                                        &shared,
                                        &streams,
                                        &authorizations,
                                        &expected,
                                    );
                                });
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                std::thread::sleep(Duration::from_millis(5));
                            }
                            Err(error) => panic!("accept: {error}"),
                        }
                    }
                })
            };
            Self {
                address,
                stopped,
                streams,
                authorizations,
                accept: Some(accept),
            }
        }

        /// Send one event to every open event stream.
        fn send(&self, event: Value) {
            let data = format!("data: {event}\n\n");
            for stream in self.streams.lock().unwrap().iter_mut() {
                let _ = write!(stream, "{:x}\r\n{}\r\n", data.len(), data);
                let _ = stream.flush();
            }
        }

        /// Stop the service: its port closes and its event streams end.
        fn stop(&mut self) {
            self.stopped.store(true, Ordering::Release);
            if let Some(accept) = self.accept.take() {
                accept.join().unwrap();
            }
            for stream in self.streams.lock().unwrap().drain(..) {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
        }
    }

    impl Drop for FakeService {
        fn drop(&mut self) {
            self.stop();
        }
    }

    fn fake_exchange(
        mut socket: std::net::TcpStream,
        shared: &FakeShared,
        streams: &Mutex<Vec<std::net::TcpStream>>,
        authorizations: &Mutex<Vec<Option<String>>>,
        expected: &str,
    ) {
        // macOS accepted sockets inherit the listener's nonblocking mode.
        socket.set_nonblocking(false).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(socket.try_clone().unwrap());
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            return;
        }
        let mut words = line.split_whitespace();
        let method = words.next().unwrap_or("").to_owned();
        let path = words.next().unwrap_or("").to_owned();
        let mut authorization = None;
        let mut length = 0usize;
        loop {
            line.clear();
            if reader.read_line(&mut line).is_err() || line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                match name.to_ascii_lowercase().as_str() {
                    "authorization" => authorization = Some(value.trim().to_owned()),
                    "content-length" => length = value.trim().parse().unwrap_or(0),
                    _ => {}
                }
            }
        }
        let mut body = vec![0; length];
        let _ = std::io::Read::read_exact(&mut reader, &mut body);
        authorizations.lock().unwrap().push(authorization.clone());
        let respond = |socket: &mut std::net::TcpStream, status: &str, body: &str| {
            let _ = write!(
                socket,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        };
        if authorization.as_deref() != Some(expected) {
            respond(
                &mut socket,
                "401 Unauthorized",
                r#"{"message":"unauthorized"}"#,
            );
            return;
        }
        if path == "/api/event" {
            let _ = write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n"
            );
            let _ = socket.flush();
            streams.lock().unwrap().push(socket);
            return;
        }
        let session = path
            .strip_prefix("/api/session/")
            .map(|rest| rest.split(['/', '?']).next().unwrap_or("").to_owned());
        let (status, body) = if method == "POST" && path == "/api/session" {
            let mut state = shared.state.lock().unwrap();
            state.created += 1;
            let id = format!("s{}", state.created);
            state.sessions.insert(id.clone(), Vec::new());
            ("200 OK", json!({ "data": fake_record(&id) }).to_string())
        } else if path.starts_with("/api/agent?") {
            ("200 OK", r#"{"data":[]}"#.to_owned())
        } else if path == "/api/session/active" {
            ("200 OK", r#"{"data":{}}"#.to_owned())
        } else if path.starts_with("/api/session?parentID=") {
            ("200 OK", r#"{"data":[],"cursor":{"next":null}}"#.to_owned())
        } else if let Some(id) = session {
            let mut state = shared.state.lock().unwrap();
            while state.stalled.contains(&id) && path.contains("/message?") && !state.stopping {
                state = shared
                    .changed
                    .wait_timeout(state, Duration::from_millis(50))
                    .unwrap()
                    .0;
            }
            match state.sessions.get(&id) {
                None => ("404 Not Found", r#"{"message":"not found"}"#.to_owned()),
                Some(_) if path.contains("/message?") => {
                    ("200 OK", r#"{"data":[],"cursor":{"next":null}}"#.to_owned())
                }
                Some(permissions) if path.ends_with("/permission") => {
                    ("200 OK", json!({ "data": permissions }).to_string())
                }
                Some(_) if path.ends_with("/form") => ("200 OK", r#"{"data":[]}"#.to_owned()),
                Some(_) => ("200 OK", json!({ "data": fake_record(&id) }).to_string()),
            }
        } else {
            ("404 Not Found", r#"{"message":"unexpected"}"#.to_owned())
        };
        respond(&mut socket, status, &body);
    }

    /// Record a service the way `opencode serve --service` does, replacing the
    /// file at once so a concurrent reader never sees half of it.
    fn write_service_file(path: &std::path::Path, address: std::net::SocketAddr, password: &str) {
        let partial = path.with_extension("partial");
        std::fs::write(
            &partial,
            json!({"id":"service","version":"2","url":format!("http://{address}"),"pid":1,"password":password}).to_string(),
        )
        .unwrap();
        std::fs::rename(&partial, path).unwrap();
    }

    /// Forward a session's current events to a channel, ending when it closes.
    fn watch(events: &RegistrationEvents) -> std::sync::mpsc::Receiver<Event> {
        let events = events.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(event) = block_on(events.recv()) {
                if let Some(event) = event.into_current()
                    && tx.send(event).is_err()
                {
                    return;
                }
            }
        });
        rx
    }

    /// The events up to and including the first that `wanted` matches.
    fn until(
        events: &std::sync::mpsc::Receiver<Event>,
        what: &str,
        wanted: impl Fn(&Event) -> bool,
    ) -> Vec<Event> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut seen = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            match events.recv_timeout(remaining) {
                Ok(event) => {
                    let found = wanted(&event);
                    seen.push(event);
                    if found {
                        return seen;
                    }
                }
                Err(_) => panic!("{what} did not arrive; saw {seen:?}"),
            }
        }
    }

    /// An address nothing listens on, so a test never reaches a real service.
    fn closed_address() -> std::net::SocketAddr {
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
    }

    fn join_within(thread: std::thread::JoinHandle<()>, limit: Duration) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(thread.join());
        });
        rx.recv_timeout(limit)
            .expect("the connection thread did not stop in time")
            .unwrap();
    }

    #[test]
    fn a_restarted_service_is_found_again_and_its_sessions_are_read_again() {
        let directory =
            std::env::temp_dir().join(format!("tomori-opencode-reconnect-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("service.json");
        let shared = Arc::new(FakeShared::default());
        let mut first = FakeService::start(shared.clone(), "first");
        write_service_file(&path, first.address, "first");
        let fallback = closed_address();
        let resolve_path = path.clone();
        let (commands, receiver) = Commands::new();
        let connection = std::thread::spawn(move || {
            run_thread(
                Box::new(move || {
                    crate::endpoint::read_service_file_at(&resolve_path).unwrap_or(Resolved {
                        host: "127.0.0.1".into(),
                        port: fallback.port(),
                        auth: None,
                    })
                }),
                receiver,
            )
        });
        let create = || {
            block_on(commands.reply_with(|reply| Command::Create {
                spec: SessionSpec {
                    directory: "/tmp".into(),
                    title: "Reconnect".into(),
                },
                reply,
            }))
        };
        let read = |id: &str, events: &RegistrationEvents| {
            let id = id.to_owned();
            let identity = events.identity.clone();
            block_on(commands.reply_with(|reply| Command::Snapshot {
                id,
                identity,
                reply,
            }))
            .and_then(SnapshotReply::accept)
        };

        // Three spaces are open; a fourth session has been created but its
        // space has not read it yet.
        let mut open = Vec::new();
        for _ in 0..3 {
            let (id, events) = create().unwrap();
            read(&id, &events).unwrap();
            let watched = watch(&events);
            open.push((id, events, watched));
        }
        let (unread_id, unread_events) = create().unwrap();
        let unread = watch(&unread_events);
        let ids: Vec<_> = open.iter().map(|(id, ..)| id.clone()).collect();
        assert_eq!(ids, ["s1", "s2", "s3"]);
        assert_eq!(unread_id, "s4");
        first.send(json!({"type":"session.execution.started","data":{"sessionID":"s1"}}));
        until(&open[0].2, "the first busy event", |event| {
            matches!(event, Event::Busy)
        });

        // The service restarts. Every open space is told about the gap; the
        // unread session is closed, since its first read would present events
        // from before the gap.
        first.stop();
        for (id, _, events) in &open {
            until(
                events,
                &format!("{id}'s disconnection notice"),
                |event| matches!(event, Event::Error { message } if message == DISCONNECTED),
            );
        }
        until(
            &unread,
            "the unread session's closure",
            |event| matches!(event, Event::Closed { reason: Some(reason) } if reason.contains("before the session was read")),
        );

        // While it is down, creating a session fails, and promptly.
        let started = std::time::Instant::now();
        let Err(refused) = create() else {
            panic!("a session was created while OpenCode was down");
        };
        assert_eq!(refused.kind, backend::ErrorKind::Unavailable);
        assert!(started.elapsed() < Duration::from_secs(3), "{refused:?}");

        // During the gap s1 raised a permission, s3 was deleted, and reading
        // s2 will stall. The service returns on another port with another
        // password, recorded in its service file.
        shared.update(|state| {
            state.sessions.insert(
                "s1".into(),
                vec![json!({"id":"p1","sessionID":"s1","action":"bash","resources":["make"]})],
            );
            state.sessions.remove("s3");
            state.stalled.insert("s2".into());
        });
        let second = FakeService::start(shared.clone(), "second");
        assert_ne!(second.address, first.address);
        write_service_file(&path, second.address, "second");

        // A new session can be created once OpenCode is back. An attempt still
        // using the old address may refuse the first request.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let (created, _created_events) = loop {
            match create() {
                Ok(created) => break created,
                Err(error) if std::time::Instant::now() < deadline => {
                    assert_eq!(error.kind, backend::ErrorKind::Unavailable);
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(error) => panic!("creating after recovery failed: {error:?}"),
            }
        };
        assert_eq!(created, "s5");

        // s1 is read again, and the permission raised during the gap reaches
        // its space.
        let seen = until(&open[0].2, "s1's resynchronization", |event| {
            matches!(event, Event::Resynchronized { .. })
        });
        let Some(Event::Resynchronized { snapshot }) = seen.last() else {
            unreachable!()
        };
        assert_eq!(
            snapshot
                .permissions
                .iter()
                .map(|permission| permission.id.as_str())
                .collect::<Vec<_>>(),
            ["p1"]
        );
        assert_eq!(snapshot.busy, Some(false));
        assert!(snapshot.workers.is_empty());

        // s3 no longer exists, so only its space closes, with the reason.
        until(
            &open[2].2,
            "s3's closure",
            |event| matches!(event, Event::Closed { reason: Some(reason) } if reason.contains("no longer exists")),
        );

        // While s2 is still being read, s1's live events are not held up, and
        // s2's own events wait for its state.
        second.send(json!({"type":"session.execution.started","data":{"sessionID":"s2"}}));
        second.send(json!({"type":"session.execution.started","data":{"sessionID":"s1"}}));
        until(&open[0].2, "s1's live busy event", |event| {
            matches!(event, Event::Busy)
        });
        assert!(
            open[1].2.recv_timeout(Duration::from_millis(200)).is_err(),
            "s2 reported an event before it was read again"
        );
        shared.update(|state| {
            state.stalled.remove("s2");
        });
        let seen = until(&open[1].2, "s2's held busy event", |event| {
            matches!(event, Event::Busy)
        });
        // Workers travel on their own lane, so only ordinary events are ordered.
        let ordinary: Vec<_> = seen
            .iter()
            .filter(|event| !matches!(event, Event::WorkersChanged { .. }))
            .collect();
        assert!(
            matches!(
                ordinary.as_slice(),
                [Event::Resynchronized { .. }, Event::Busy]
            ),
            "{seen:?}"
        );

        // Each run of the service only ever saw its own credentials.
        assert!(
            first
                .authorizations
                .lock()
                .unwrap()
                .iter()
                .all(|authorization| authorization.as_deref() == Some(basic("first").as_str()))
        );
        assert!(
            second
                .authorizations
                .lock()
                .unwrap()
                .iter()
                .all(|authorization| authorization.as_deref() == Some(basic("second").as_str()))
        );

        drop(commands);
        join_within(connection, Duration::from_secs(5));
        shared.update(|state| state.stopping = true);
        drop(second);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn reconnecting_backs_off_and_shutdown_stays_prompt() {
        // A service that accepts connections and drops them at once, so every
        // attempt fails after reaching it.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let stopped = Arc::new(AtomicBool::new(false));
        let server = {
            let attempts = attempts.clone();
            let stopped = stopped.clone();
            std::thread::spawn(move || {
                while !stopped.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((socket, _)) => {
                            attempts.fetch_add(1, Ordering::AcqRel);
                            drop(socket);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("accept: {error}"),
                    }
                }
            })
        };
        let (commands, receiver) = Commands::new();
        let resolved =
            crate::endpoint::resolve_from(Some(&format!("http://{address}")), None, None);
        let connection =
            std::thread::spawn(move || run_thread(Box::new(move || resolved.clone()), receiver));
        // A command while the service is unreachable fails rather than waits.
        let refused = block_on(commands.reply_with(|reply| Command::Sessions {
            directory: "/tmp".into(),
            reply,
        }))
        .expect_err("listing succeeded against a failing service");
        assert_eq!(refused.kind, backend::ErrorKind::Unavailable);
        std::thread::sleep(Duration::from_secs(2));
        // 250 ms doubling allows about five attempts in two seconds; a tight
        // loop would make thousands.
        let made = attempts.load(Ordering::Acquire);
        assert!((2..=7).contains(&made), "{made} attempts");
        // Dropping the backend stops the thread even while it waits to retry.
        drop(commands);
        join_within(connection, Duration::from_secs(1));
        stopped.store(true, Ordering::Release);
        server.join().unwrap();
    }

    #[test]
    fn the_reason_a_registration_ended_is_always_received() {
        // Ending a registration queues Closed and drops both senders at once;
        // a waiting receiver must see Closed, not the worker channel's end.
        for _ in 0..200 {
            let mut registrations = HashMap::new();
            let events =
                insert_registration(&mut registrations, "gone".into(), "/tmp", Modes::default());
            let waiting = events.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(block_on(waiting.recv()).map(QueuedEvent::into_current));
            });
            // Let the receiver block on both channels first.
            std::thread::sleep(Duration::from_millis(2));
            end_registration(&mut registrations, "gone", Some("the reason".into()));
            assert!(matches!(
                rx.recv_timeout(Duration::from_secs(5)).unwrap(),
                Ok(Some(Event::Closed { reason: Some(reason) })) if reason == "the reason"
            ));
        }
    }

    #[test]
    fn a_second_loss_discards_events_held_from_the_connection_between() {
        let mut registrations = HashMap::new();
        let read = insert_registration(&mut registrations, "read".into(), "/tmp", Modes::default());
        let unread = insert_registration(
            &mut registrations,
            "unread".into(),
            "/tmp",
            Modes::default(),
        );
        registrations.get_mut("read").unwrap().workers_ready = true;
        connection_lost(&mut registrations);
        assert!(matches!(
            read.try_recv().unwrap().into_current(),
            Some(Event::Error { message }) if message == DISCONNECTED
        ));
        assert!(matches!(
            unread.try_recv().unwrap().into_current(),
            Some(Event::Closed { reason: Some(_) })
        ));
        assert!(!registrations.contains_key("unread"));
        // The stream reopens, an event is held, and the stream is lost again
        // before the session is read: the next read covers that event.
        let busy =
            json!({"type":"session.execution.started","data":{"sessionID":"read"}}).to_string();
        dispatch_events(&busy, &mut registrations);
        assert_eq!(
            registrations["read"].resync.as_ref().unwrap().events.len(),
            1
        );
        connection_lost(&mut registrations);
        assert!(
            registrations["read"]
                .resync
                .as_ref()
                .unwrap()
                .events
                .is_empty()
        );
        assert!(matches!(
            read.try_recv().unwrap().into_current(),
            Some(Event::Error { .. })
        ));
        assert!(read.try_recv().is_err());
    }

    #[test]
    fn held_events_beyond_the_limit_close_only_that_session() {
        let mut registrations = HashMap::new();
        let resyncing = insert_registration(
            &mut registrations,
            "parent".into(),
            "/tmp",
            Modes::default(),
        );
        let other =
            insert_registration(&mut registrations, "other".into(), "/tmp", Modes::default());
        registrations.get_mut("parent").unwrap().resync = Some(Resync::default());
        let busy =
            json!({"type":"session.execution.started","data":{"sessionID":"parent"}}).to_string();
        for _ in 0..MAX_RESYNC_EVENTS {
            dispatch_events(&busy, &mut registrations);
        }
        assert!(resyncing.try_recv().is_err(), "held events were delivered");
        dispatch_events(&busy, &mut registrations);
        assert!(!registrations.contains_key("parent"));
        assert!(matches!(
            resyncing.try_recv().unwrap().into_current(),
            Some(Event::Closed { reason: Some(reason) }) if reason.contains("too many")
        ));
        assert!(registrations.contains_key("other"));
        assert!(other.try_recv().is_err());
    }

    #[test]
    fn workers_read_again_are_merged_without_inferring_deletion() {
        let (mut registration, _events) = Registration::new();
        let worker = |id: &str, state| Worker {
            id: id.into(),
            parent: Some("parent".into()),
            title: id.into(),
            state,
        };
        for known in [
            worker("omitted", WorkerState::Running),
            worker("finished", WorkerState::Running),
            worker("deleted", WorkerState::Completed),
        ] {
            registration.children.insert(known.id.clone(), known);
        }
        // "deleted" was deleted while the session was read; the listing still
        // shows it, and shows "finished" completing during the gap.
        registration.snapshot_deletions.insert("deleted".into());
        registration.children.remove("deleted");
        registration
            .merge_workers(vec![
                worker("finished", WorkerState::Completed),
                worker("deleted", WorkerState::Completed),
                worker("new", WorkerState::Unknown),
            ])
            .unwrap();
        let states: Vec<_> = registration
            .children
            .values()
            .map(|worker| (worker.id.as_str(), worker.state))
            .collect();
        assert_eq!(
            states,
            [
                ("finished", WorkerState::Completed),
                ("new", WorkerState::Unknown),
                // Kept, but no longer claimed to run without fresh evidence.
                ("omitted", WorkerState::Unknown),
            ]
        );
    }
}
