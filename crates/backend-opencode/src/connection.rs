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
use tomori_backend::{Event, Modes, SessionSpec, SessionSummary, Snapshot, Worker, WorkerState};

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
/// Sessions whose workers are discovered in the background at once, after a
/// read ran out of time for them; each lists at most eight parents' children
/// at once, and gives up on a listing OpenCode does not answer within
/// [`api::FamilyWalk::LISTING_TIMEOUT`].
const MAX_DISCOVERIES: usize = 4;
/// Why a session's workers may be incomplete while they are discovered.
const DISCOVERING: &str = "Still finding this session's workers, so some may be missing.";
/// How long a background discovery that failed, because OpenCode answered a
/// listing with an error or not in time, waits before each attempt to go on
/// from where it stopped. Once they are used up, the workers stay reported as
/// incomplete, with the reason, until the session is read again.
const DISCOVERY_RETRIES: [Duration; 3] = [
    Duration::from_secs(5),
    Duration::from_secs(30),
    Duration::from_secs(120),
];
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
fn validate_workers(workers: &mut [Worker]) -> tomori_backend::Result<()> {
    for worker in workers.iter_mut() {
        if worker.id.is_empty()
            || worker.id.len() > MAX_WORKER_ID_BYTES
            || worker
                .parent
                .as_ref()
                .is_some_and(|parent| parent.is_empty() || parent.len() > MAX_WORKER_ID_BYTES)
        {
            return Err(tomori_backend::Error::new(
                tomori_backend::ErrorKind::Unavailable,
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
/// resets after a connection that stayed open at least that long.
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
/// Connections lost before a session could be read again, after the first
/// loss, before its space is closed.
const RESYNC_INTERRUPTIONS: u8 = 5;
/// Events held for a session while it is read again; more close it, as an
/// overflowing event channel does. Well below the channel's capacity, so the
/// held events and the state they follow can be delivered together.
const MAX_RESYNC_EVENTS: usize = 512;
/// Why a request fails when too many already wait for a connection.
const TOO_MANY_HELD: &str = "OpenCode is not reachable and too many requests are waiting for it";
/// Requests to OpenCode in flight at once on a connection, alongside its
/// event stream.
const MAX_REQUESTS: usize = 16;
/// Commands waiting to start on an open connection, behind a request in
/// flight for their session or the limit on requests; more are refused.
const MAX_WAITING: usize = 256;
/// Why a request fails when too many already wait to start.
const TOO_MANY_WAITING: &str = "too many requests are waiting for OpenCode";
/// How long the requests in flight when the event stream is lost may take to
/// be answered before the backend reconnects without them.
const IN_FLIGHT_GRACE: Duration = CONNECT_TIMEOUT;
/// Why a request fails once the backend is shutting down.
const SHUTTING_DOWN: &str = "the OpenCode backend is shutting down";
/// What a session is told when the event stream is lost.
const DISCONNECTED: &str = "Lost the connection to OpenCode; reconnecting.";

/// A worker replacement carries the snapshot epoch under which it was queued.
/// Other events retain their original ordering and are never invalidated.
/// Each is boxed, since a resynchronization's snapshot is far larger than the
/// rest.
pub(crate) enum QueuedEvent {
    Ordinary(Box<Event>),
    Workers {
        pending: Arc<Mutex<PendingWorkers>>,
        watermark: Arc<AtomicU64>,
    },
}

impl QueuedEvent {
    pub(crate) fn into_current(self) -> Option<Event> {
        match self {
            Self::Ordinary(event) => Some(*event),
            Self::Workers { pending, watermark } => {
                let mut pending = pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                pending.latest.take().and_then(|update| match update {
                    PendingWorkerUpdate::Workers {
                        epoch,
                        workers,
                        partial,
                    } => (epoch >= watermark.load(Ordering::Acquire)).then_some(match partial {
                        Some(message) => Event::WorkersPartial { workers, message },
                        None => Event::WorkersChanged { workers },
                    }),
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
    /// Every worker known, and why they may not be all, if they may not.
    Workers {
        epoch: u64,
        workers: Vec<Worker>,
        partial: Option<String>,
    },
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
    close_reason: Arc<Mutex<Option<String>>>,
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
    /// Why the registration ended, kept beside the channel so that it still
    /// arrives when the channel had no room for the closing event.
    close_reason: Arc<Mutex<Option<String>>>,
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

    /// The end of a registration whose channel has closed: its reason, once,
    /// if it ended with one.
    fn ended(&self) -> std::result::Result<QueuedEvent, async_channel::RecvError> {
        match self
            .close_reason
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            Some(reason) => Ok(QueuedEvent::Ordinary(Box::new(Event::Closed {
                reason: Some(reason),
            }))),
            None => Err(async_channel::RecvError),
        }
    }

    pub(crate) async fn recv(&self) -> std::result::Result<QueuedEvent, async_channel::RecvError> {
        self.consumer_started.store(true, Ordering::Release);
        match self.try_recv() {
            Ok(event) => return Ok(event),
            Err(async_channel::TryRecvError::Closed) => return self.ended(),
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
        if self.ordinary_incomplete.load(Ordering::Acquire) || result.is_err() {
            self.ended()
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
        let close_reason = Arc::new(Mutex::new(None));
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
                close_reason: close_reason.clone(),
            },
            RegistrationEvents {
                receiver: rx,
                worker_receiver: worker_rx,
                pending,
                watermark,
                prefer_workers: Arc::new(AtomicBool::new(true)),
                ordinary_incomplete,
                consumer_started,
                close_reason,
                identity,
            },
        )
    }

    /// Attempt delivery without blocking the shared dispatcher.
    pub fn deliver(&self, event: Event) -> bool {
        match self.tx.try_send(QueuedEvent::Ordinary(Box::new(event))) {
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

    /// End the registration with `reason`. The reason is kept beside the
    /// channel first, so the session receives it even if the channel is full
    /// and the closing event cannot be queued.
    fn close_with(&self, reason: Option<String>) {
        if let Some(reason) = &reason {
            *self
                .close_reason
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reason.clone());
        }
        if self.deliver(Event::Closed { reason }) {
            // Delivered in order; the kept copy is not needed.
            self.close_reason
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
        } else {
            self.tx.close();
        }
    }

    /// How many more ordinary events the channel can take now.
    fn room(&self) -> usize {
        self.tx
            .capacity()
            .unwrap_or(usize::MAX)
            .saturating_sub(self.tx.len())
    }

    /// Retryable state events are allowed to wait for ordinary channel space.
    fn try_deliver_state(&self, event: Event) -> bool {
        self.tx
            .try_send(QueuedEvent::Ordinary(Box::new(event)))
            .is_ok()
    }

    fn mark_ordinary_incomplete(&self) {
        self.ordinary_incomplete.store(true, Ordering::Release);
        self.tx.close();
    }

    fn publish_worker_update(&self, update: PendingWorkerUpdate) -> bool {
        // The worker channel closes only with the ordinary one, when the
        // registration's events are dropped, which holds both receivers.
        if self.tx.is_closed() {
            return false;
        }
        let mut pending = self
            .pending_workers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.latest = Some(update);
        // A full channel already notifies of the latest update; a closed one
        // has nobody left to notify.
        let _ = self.worker_tx.try_send(());
        true
    }

    /// Worker notifications use a separate one-element channel so a burst
    /// cannot fill the ordinary event channel or duplicate family snapshots.
    fn try_deliver_workers(&self, workers: Vec<Worker>, partial: Option<String>) -> bool {
        self.publish_worker_update(PendingWorkerUpdate::Workers {
            epoch: self.worker_epoch.load(Ordering::Acquire),
            workers,
            partial,
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
    /// When each worker was first observed, so that workers first reported
    /// together are reported in the order they were found: oldest first.
    order: HashMap<String, u64>,
    found: u64,
    /// Why the workers known may not be all the session has: while the rest
    /// of its family is discovered in the background, or after that failed.
    workers_partial: Option<String>,
    /// The rest of the family, to be walked once a background discovery can
    /// start, and the cancellation of the one running.
    discovery: Option<api::FamilyWalk>,
    discovery_in_flight: Option<Cancellation>,
    /// The discovery attempts that failed since the session was last read,
    /// and when the walk may next go on after one.
    discovery_failures: usize,
    discovery_retry_at: Option<Instant>,
    /// How many discoveries have started, so that the result of one stopped
    /// since is told from the one running.
    discoveries: u64,
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
    /// Ordinary events held from registration until the initial snapshot is
    /// installed, which they follow, except those it already shows.
    opening: Resync,
    /// Messages a read after reconnecting listed while they were still being
    /// written: text streamed while the stream was down is in no event, so
    /// each is read again once it ends, when OpenCode records its text.
    restoring: HashSet<String>,
    /// Messages that have ended and wait to be read again, those being read,
    /// and whether that read is running.
    restore_due: Vec<String>,
    restore_reading: Vec<String>,
    restore_in_flight: bool,
    /// Messages whose record had no text yet when read, retried at the
    /// reconciliation cadence, with the attempts made for each.
    restore_retry: Vec<String>,
    restore_attempts: HashMap<String, u8>,
    /// What each worker was last seen doing, reported as it changes. See `design/events.md`.
    activity: crate::activity::Workers,
    /// The workers whose records are to be read again, their activity having become unknown when the stream was lost,
    /// and whether that read is running.
    activity_due: Vec<String>,
    activity_in_flight: bool,
}

impl Resync {
    /// Hold `event`, the event stream's `sequence`th. A text delta that
    /// directly follows a held delta of the same message is appended to it,
    /// so a streaming reply takes one place. Returns false when the limit is
    /// reached.
    fn hold(&mut self, sequence: u64, event: Event) -> bool {
        if let Event::TextDelta { message, text } = &event
            && let Some((
                last,
                Event::TextDelta {
                    message: held,
                    text: joined,
                },
            )) = self.events.last_mut()
            && held == message
        {
            joined.push_str(text);
            *last = sequence;
            return true;
        }
        if self.events.len() >= MAX_RESYNC_EVENTS {
            return false;
        }
        self.events.push((sequence, event));
        true
    }
}

/// A session waiting to be read again after the event stream was lost.
#[derive(Default)]
struct Resync {
    /// Ordinary events since the event stream was lost, with their place in
    /// it, delivered after the state read again, in order, as when a space
    /// first opens, except those the read is known to reflect.
    events: Vec<(u64, Event)>,
    /// Failed attempts on connections since the loss.
    failures: u8,
    /// Connections lost again before the session was read.
    interruptions: u8,
    /// How many events had been dispatched when the current read requested the
    /// session's messages: held permission and question events up to it are
    /// reflected in the read's lists.
    mark: Arc<AtomicU64>,
    /// A failed attempt is retried no sooner than this, so that a service
    /// still starting, or about to be lost again, is not read in a burst.
    retry_at: Option<Instant>,
    /// An agent switch was seen during the current read, so its modes are
    /// newer than the read's.
    modes_changed: bool,
    /// Worker lifecycle changes seen during the current read, newer than its
    /// listing: the state and title each event set.
    changes: HashMap<String, (Option<WorkerState>, Option<String>)>,
    /// More changes than can be tracked were seen; the read is retried.
    changes_overflow: bool,
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
                order: HashMap::new(),
                found: 0,
                workers_partial: None,
                discovery: None,
                discovery_in_flight: None,
                discovery_failures: 0,
                discovery_retry_at: None,
                discoveries: 0,
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
                opening: Resync::default(),
                restoring: HashSet::new(),
                restore_due: Vec::new(),
                restore_reading: Vec::new(),
                restore_in_flight: false,
                restore_retry: Vec::new(),
                restore_attempts: HashMap::new(),
                activity: crate::activity::Workers::default(),
                activity_due: Vec::new(),
                activity_in_flight: false,
            },
            rx,
        )
    }

    /// Merge the workers listed when the session is read again after a lost
    /// event stream. A listed worker is added, or its state and title are
    /// updated; a worker deleted while it was read is not restored. A known
    /// worker the listing omits is kept, because an omission is never evidence
    /// of deletion, but it is no longer reported as running: the evidence it
    /// was running predates the gap. A lifecycle event seen during the read is
    /// newer than its listing, so the state or title it set is kept over the
    /// listing's, and a worker it shows running is not downgraded. Exceeding
    /// the tracking limit makes worker observation visibly incomplete rather
    /// than failing the read.
    fn merge_workers(
        &mut self,
        mut workers: Vec<Worker>,
        changes: &HashMap<String, (Option<WorkerState>, Option<String>)>,
    ) -> tomori_backend::Result<()> {
        if self.workers_incomplete {
            return Ok(());
        }
        validate_workers(&mut workers)?;
        let listed: HashSet<&str> = workers.iter().map(|worker| worker.id.as_str()).collect();
        for (id, known) in &mut self.children {
            let changed = changes.get(id).is_some_and(|(state, _)| state.is_some());
            if !listed.contains(id.as_str()) && !changed && known.state == WorkerState::Running {
                known.state = WorkerState::Unknown;
            }
        }
        for mut worker in workers {
            if self.snapshot_deletions.contains(&worker.id) {
                continue;
            }
            let (state, title) = changes.get(&worker.id).cloned().unwrap_or_default();
            if let Some(state) = state {
                worker.state = state;
            }
            if let Some(title) = title {
                worker.title = title;
            }
            if let Some(known) = self.children.get_mut(&worker.id) {
                known.state = worker.state;
                known.title = worker.title;
            } else if self.children.len() >= api::MAX_TRACKED_WORKERS {
                self.mark_workers_incomplete();
                return Ok(());
            } else {
                self.insert_child(worker);
            }
        }
        Ok(())
    }

    /// Observe `worker`, which was not known.
    fn insert_child(&mut self, worker: Worker) {
        self.found += 1;
        self.order.insert(worker.id.clone(), self.found);
        self.children.insert(worker.id.clone(), worker);
    }

    /// Stop observing the worker `id`, returning whether it was known.
    fn remove_child(&mut self, id: &str) -> bool {
        self.order.remove(id);
        self.children.remove(id).is_some()
    }

    /// Go on discovering the session's family with `rest`, the walk a read ran
    /// out of time for, or, when the read walked it whole, stop: a discovery
    /// still running for an earlier read is cancelled either way. While one
    /// runs, the workers are reported as possibly incomplete, and lifecycle
    /// events are kept for the workers it has not found yet. The read starts
    /// the attempts afresh, so a retry an earlier discovery waited for is
    /// dropped.
    fn continue_discovery(&mut self, rest: Option<api::FamilyWalk>) {
        if let Some(cancellation) = self.discovery_in_flight.take() {
            cancellation.cancel();
        }
        self.workers_partial = rest.is_some().then(|| DISCOVERING.to_owned());
        self.discovery = rest;
        self.discovery_failures = 0;
        self.discovery_retry_at = None;
    }

    /// Stop a discovery whose connection was lost, with any retry it waited
    /// for: the session is read again, with its family, on the next one.
    /// Until then the workers known stay as incomplete as they were.
    fn abandon_discovery(&mut self) {
        self.discovery = None;
        self.discovery_in_flight = None;
        self.discovery_failures = 0;
        self.discovery_retry_at = None;
    }

    /// Whether a discovery is waiting to start at `now`: its walk is not
    /// running, and any retry it waits for is due.
    fn discovery_due(&self, now: Instant) -> bool {
        self.discovery.is_some()
            && self
                .discovery_retry_at
                .is_none_or(|retry_at| retry_at <= now)
    }

    /// Start the discovery waiting to start, cancelled with `cancellation`,
    /// returning its walk and which of the registration's discoveries it is.
    fn start_discovery(&mut self, cancellation: Cancellation) -> (api::FamilyWalk, u64) {
        let walk = self.discovery.take().expect("a discovery is waiting");
        self.discovery_retry_at = None;
        self.discovery_in_flight = Some(cancellation);
        self.discoveries += 1;
        (walk, self.discoveries)
    }

    /// Take the workers a background discovery found, as the initial snapshot
    /// takes its listing: a worker already known, or deleted meanwhile, is
    /// left as it is, and the lifecycle events kept for the others are newer
    /// than their records. Creations kept for a parent it found are added
    /// too. `failure` is the rest of the walk and why it stopped before the
    /// family was walked whole: the walk goes on from there after the next of
    /// [`DISCOVERY_RETRIES`] from `now`, or, once they are used up, stops, and
    /// either way the workers stay incomplete.
    fn adopt_discovered(
        &mut self,
        mut workers: Vec<Worker>,
        failure: Option<(api::FamilyWalk, String)>,
        now: Instant,
    ) {
        self.discovery_in_flight = None;
        self.discovery_retry_at = None;
        self.workers_partial = failure.map(|(walk, reason)| {
            match DISCOVERY_RETRIES.get(self.discovery_failures) {
                Some(delay) => {
                    self.discovery_failures += 1;
                    self.discovery_retry_at = Some(now + *delay);
                    self.discovery = Some(walk);
                    format!(
                        "Could not find every worker of this session yet, so some may be missing: {reason}. Trying again."
                    )
                }
                None => format!(
                    "Could not find every worker of this session, so some may be missing: {reason}"
                ),
            }
        });
        if let Err(error) = validate_workers(&mut workers) {
            // The walk has taken these records, so going on would never list
            // them: it stops, and the workers stay incomplete.
            self.discovery = None;
            self.discovery_retry_at = None;
            self.workers_partial = Some(format!(
                "Could not find every worker of this session, so some may be missing: {}",
                error.message
            ));
            workers.clear();
        }
        for mut worker in workers {
            if self.children.contains_key(&worker.id)
                || self.initial_deletions.contains(&worker.id)
                || self.snapshot_deletions.contains(&worker.id)
            {
                continue;
            }
            if self.children.len() >= api::MAX_TRACKED_WORKERS {
                self.mark_workers_incomplete();
                return;
            }
            if let Some((state, title)) = self.initial_changes.get(&worker.id) {
                if let Some(state) = state {
                    worker.state = *state;
                }
                if let Some(title) = title {
                    worker.title.clone_from(title);
                }
            }
            self.insert_child(worker);
        }
        if self.attach_creations().is_err() {
            self.mark_workers_incomplete();
            return;
        }
        if !self.discovering_workers() {
            self.clear_bootstrap_journals();
        }
    }

    /// The workers known before a read again, of which those its listing
    /// omits are looked up, since one may have been deleted while the stream
    /// was down; none once observation is incomplete, as none is removed then.
    fn known_workers(&self) -> Vec<String> {
        if self.workers_incomplete {
            Vec::new()
        } else {
            self.children.keys().cloned().collect()
        }
    }

    /// Remove the known workers a read again found `missing`: its listing
    /// omitted them and OpenCode answered that they do not exist. One a
    /// lifecycle event during the read showed, which is newer evidence than
    /// the lookup, is kept, and none is removed once observation is
    /// incomplete.
    fn remove_missing(
        &mut self,
        missing: &[String],
        changes: &HashMap<String, (Option<WorkerState>, Option<String>)>,
    ) {
        if self.workers_incomplete {
            return;
        }
        for id in missing {
            if !changes.contains_key(id) {
                self.remove_child(id);
            }
        }
    }

    fn publish_workers(&mut self) {
        if self.workers_ready {
            self.workers_dirty = if self.workers_incomplete {
                !self.events.try_deliver_worker_error(
                    "Worker observation exceeds the supported limit; reattach after reducing it.",
                )
            } else {
                // A worker keeps its place in the space, so the order matters
                // for those first reported together.
                let mut workers: Vec<Worker> = self.children.values().cloned().collect();
                workers.sort_by_key(|worker| self.order.get(&worker.id).copied());
                !self
                    .events
                    .try_deliver_workers(workers, self.workers_partial.clone())
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

    /// Stop observing the worker `id`, which OpenCode deleted, remembering the
    /// deletion against any listing still in flight, returning whether it was
    /// known.
    fn forget_deleted(&mut self, id: &str) -> bool {
        let discovering = self.discovering_workers();
        let removed = self.remove_worker(id);
        if discovering {
            // A delete event has no parent ID. Before bootstrap, keep
            // even an unknown ID: a stale page may later list it.
            if self.initial_deletions.contains(id)
                || self.initial_deletions.len() < api::MAX_TRACKED_WORKERS
            {
                self.initial_deletions.insert(id.to_owned());
            } else {
                self.mark_workers_incomplete();
            }
        }
        self.initial_changes.remove(id);
        self.initial_creations.remove(id);
        removed
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
        self.remove_child(id)
    }

    /// Deliver an ordinary event, noting when a message whose text was partly
    /// missed while reconnecting has ended, so that it is read again.
    fn deliver_content(&mut self, event: Event) {
        if let Event::MessageCompleted { message } = &event
            && self.restoring.remove(message)
        {
            self.restore_due.push(message.clone());
        }
        self.events.deliver(event);
    }

    fn publish_modes(&mut self) {
        // While the session is read again, its modes are reported with that
        // state, after the events held for it.
        if self.resync.is_some() {
            self.modes_dirty = false;
            return;
        }
        self.modes_dirty = !self.deliver_state(Event::ModesChanged(self.modes.clone()));
    }

    /// Report what the workers whose activity changed were last seen doing, once the session's workers are reported
    /// and it is not being read again, each after the worker list that names it; those the channel has no room for
    /// wait for the next try. See `design/events.md`.
    fn publish_activity(&mut self) {
        if !self.workers_ready || self.resync.is_some() || !self.activity.is_dirty() {
            return;
        }
        let children = &self.children;
        let events = &self.events;
        self.activity.report(
            |id| children.contains_key(id),
            |id, activity| {
                events.try_deliver_state(Event::WorkerActivity {
                    worker: id.to_owned(),
                    activity,
                })
            },
        );
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

    /// Whether the session's workers are still being discovered: until the
    /// initial snapshot has been accepted, and while the rest of its family is
    /// walked in the background.
    fn discovering_workers(&self) -> bool {
        self.discovery.is_some()
            || self.discovery_in_flight.is_some()
            || !self
                .bootstrap_epoch
                .is_some_and(|epoch| self.events.worker_watermark.load(Ordering::Acquire) >= epoch)
    }

    fn clear_bootstrap_journals(&mut self) {
        self.initial_deletions.clear();
        self.initial_changes.clear();
        self.initial_creations.clear();
    }

    /// Add the creations kept while workers were discovered whose parent is
    /// known now. A nested child can be created while its preexisting parent
    /// is still being discovered, so pending descendants are attached only
    /// after their parent is known, preserving the actual hierarchy.
    fn attach_creations(&mut self) -> tomori_backend::Result<()> {
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
                return Ok(());
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
                    return Err(tomori_backend::Error::new(
                        tomori_backend::ErrorKind::Unavailable,
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
                self.insert_child(worker);
            }
        }
    }

    /// Install a snapshot without losing lifecycle events already observed
    /// before the initial consumer started.
    fn install_workers(&mut self, workers: &mut Vec<Worker>) -> tomori_backend::Result<()> {
        if self.workers_incomplete {
            return Err(tomori_backend::Error::new(
                tomori_backend::ErrorKind::Unavailable,
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
            return Err(tomori_backend::Error::new(
                tomori_backend::ErrorKind::Unavailable,
                "Worker observation exceeds the supported limit.",
            ));
        }
        // The snapshot keeps its listing's order, oldest first, for the workers
        // it lists; others follow.
        let listed: HashMap<String, usize> = workers
            .iter()
            .enumerate()
            .map(|(index, worker)| (worker.id.clone(), index))
            .collect();
        self.events.advance_workers();
        if self.bootstrap_epoch.is_none() {
            self.bootstrap_epoch = Some(self.events.worker_epoch.load(Ordering::Acquire));
        }
        // A worker already known keeps its state.
        for mut worker in workers.drain(..) {
            if !self.initial_deletions.contains(&worker.id)
                && !self.snapshot_deletions.contains(&worker.id)
                && !self.children.contains_key(&worker.id)
            {
                if discovering && let Some((state, title)) = self.initial_changes.get(&worker.id) {
                    if let Some(state) = state {
                        worker.state = *state;
                    }
                    if let Some(title) = title {
                        worker.title.clone_from(title);
                    }
                }
                self.insert_child(worker);
            }
        }
        if discovering {
            self.attach_creations()?;
        }
        *workers = self.children.values().cloned().collect();
        workers.sort_by_key(|worker| listed.get(&worker.id).copied().unwrap_or(usize::MAX));
        if !discovering {
            self.clear_bootstrap_journals();
        }
        self.workers_ready = true;
        Ok(())
    }

    /// Follow the session to the directory `snapshot` says it is in, if it says, so that its agents are listed where it is now.
    fn follow(&mut self, snapshot: &Snapshot) {
        if let Some(directory) = &snapshot.directory {
            directory.clone_into(&mut self.directory);
        }
    }

    /// Install a snapshot read for a session. The first is the initial state
    /// the space opens with: the events held since the session was registered
    /// are delivered after it, as after a reconnection, except those it
    /// already shows (see [`shown`]), and the running state and modes it
    /// reports are the latest known. A channel without room for them refuses
    /// the snapshot, leaving them held for a retry.
    fn install(&mut self, snapshot: &mut Snapshot) -> tomori_backend::Result<()> {
        let opening = !self.workers_ready;
        if opening && self.events.room() <= self.opening.events.len() {
            return Err(tomori_backend::Error::new(
                tomori_backend::ErrorKind::Unavailable,
                "Too many session events arrived while reading the initial state.",
            ));
        }
        self.install_workers(&mut snapshot.workers)?;
        if opening {
            let held = std::mem::take(&mut self.opening.events);
            let recorded =
                recorded_text(snapshot, &held, self.opening.mark.load(Ordering::Acquire));
            let held: Vec<Event> = held
                .into_iter()
                .enumerate()
                .filter(|(index, (_, event))| !recorded.contains(index) && !shown(snapshot, event))
                .map(|(_, (_, event))| event)
                .collect();
            let mut running = match snapshot.busy {
                Some(true) => RunningState::Running,
                Some(false) => RunningState::Idle,
                None => self.running,
            };
            for event in &held {
                match event {
                    Event::Busy => running = RunningState::Running,
                    Event::Idle => running = RunningState::Idle,
                    _ => {}
                }
            }
            self.running = running;
            // An agent switch confirmed during the read is newer than it.
            snapshot.modes = self.modes.clone();
            for event in held {
                self.deliver_content(event);
            }
        }
        Ok(())
    }
}

/// One command for the thread, carrying the channel to answer it on.
///
/// Every reply channel carries a `tomori_backend::Error`, not an `api::Failure`: the
/// thread maps failures to the neutral error before answering.
pub enum Command {
    Sessions {
        directory: String,
        reply: async_channel::Sender<tomori_backend::Result<Vec<SessionSummary>>>,
    },
    Create {
        spec: SessionSpec,
        reply: async_channel::Sender<tomori_backend::Result<(String, RegistrationEvents)>>,
    },
    Attach {
        id: String,
        directory: String,
        reply: async_channel::Sender<tomori_backend::Result<RegistrationEvents>>,
    },
    Snapshot {
        id: String,
        identity: Arc<AtomicU64>,
        reply: async_channel::Sender<tomori_backend::Result<SnapshotReply>>,
    },
    Prompt {
        id: String,
        identity: Arc<AtomicU64>,
        key: String,
        text: String,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    Interrupt {
        id: String,
        identity: Arc<AtomicU64>,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    Decide {
        id: String,
        identity: Arc<AtomicU64>,
        request: String,
        decision: String,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    Answer {
        id: String,
        identity: Arc<AtomicU64>,
        form: String,
        answer: Value,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    /// Choose the agent for `id`'s later prompts.
    SetMode {
        id: String,
        identity: Arc<AtomicU64>,
        mode: String,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    /// Set `id`'s instruction entry `key` to `value`, or remove it when `value` is `None`.
    Instruction {
        id: String,
        identity: Arc<AtomicU64>,
        key: String,
        value: Option<String>,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    /// Remove the instruction entries `keys` from the session `id`, unless it is attached when this is performed:
    /// then a space was bound to it again, and the entries are that space's.
    ForgetInstructions {
        id: String,
        keys: Vec<String>,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    /// Stop any turn the session `id` is running and delete it, with its child sessions, unless it is attached when
    /// this is performed: then a space was bound to it again, and it is that space's.
    DeleteSession {
        id: String,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    /// Delete the worker session `worker` of the session `session`, with its own workers, once OpenCode's records say
    /// that it descends from `session` through no session a space is bound to, and, unless `force`, that neither it nor
    /// any of its own workers known is running; with `force`, those are stopped first.
    DeleteWorker {
        session: String,
        worker: String,
        force: bool,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    /// Register Tomori's MCP server for `directory`, replacing one of its name.
    RegisterTools {
        directory: String,
        server: tomori_backend::ToolServer,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    /// The directory the session `id` is in now, as its record says.
    Locate {
        id: String,
        reply: async_channel::Sender<tomori_backend::Result<Option<String>>>,
    },
    /// Remove the MCP server `name` from `directory`; one OpenCode does not have is already removed.
    UnregisterTools {
        directory: String,
        name: String,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    /// Close `id`'s registration without deleting its OpenCode session.
    Close {
        id: String,
        identity: Arc<AtomicU64>,
    },
    /// The backend is shutting down: fail every waiting and later request,
    /// and close every registration, without waiting for OpenCode.
    Shutdown,
}

struct SnapshotResult {
    id: String,
    registration: Arc<AtomicU64>,
    reply: async_channel::Sender<tomori_backend::Result<SnapshotReply>>,
    result: tomori_backend::Result<Snapshot>,
    /// The rest of the session's family, when the read ran out of time for it.
    rest: Option<api::FamilyWalk>,
}

/// What a background discovery of a session's workers found.
struct DiscoveryResult {
    id: String,
    registration: Arc<AtomicU64>,
    /// Which of the registration's discoveries this was.
    number: u64,
    /// The descendants found, and whether the rest of the family was walked
    /// whole, or why not, with the walk, to go on with after a failure.
    found: Vec<Value>,
    result: std::result::Result<(), Failure>,
    walk: api::FamilyWalk,
    /// The sessions OpenCode is running, if it said in time.
    active: Option<HashSet<String>>,
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
    pub(crate) fn accept(self) -> tomori_backend::Result<Snapshot> {
        if self.ordinary_incomplete.load(Ordering::Acquire) {
            return Err(tomori_backend::Error::new(
                tomori_backend::ErrorKind::Unavailable,
                "Session events were lost while reading the initial state.",
            ));
        }
        if self.worker_incomplete.load(Ordering::Acquire) {
            return Err(tomori_backend::Error::new(
                tomori_backend::ErrorKind::Unavailable,
                "Worker observation exceeds the supported limit.",
            ));
        }
        let previous = self.watermark.fetch_max(self.epoch, Ordering::AcqRel);
        if self.epoch < previous {
            return Err(tomori_backend::Error::new(
                tomori_backend::ErrorKind::Unavailable,
                "A newer session snapshot was already accepted.",
            ));
        }
        Ok(self.snapshot)
    }
}

/// Messages read again once they ended, as OpenCode recorded them.
struct RestoreResult {
    id: String,
    registration: Arc<AtomicU64>,
    /// The wanted messages whose text the record has.
    messages: Vec<tomori_backend::SnapshotMessage>,
    /// The wanted messages it has no text for yet, or all of them if the read
    /// failed.
    unrecorded: Vec<String>,
}

struct ResyncResult {
    id: String,
    registration: Arc<AtomicU64>,
    result: tomori_backend::Result<Snapshot>,
    /// The rest of the session's family, when the read ran out of time for it.
    rest: Option<api::FamilyWalk>,
    /// Known workers the read's listing omitted that OpenCode answered do not
    /// exist.
    missing: Vec<String>,
    /// The agents offered and the session's current one, if they were listed.
    modes: Option<Modes>,
}

struct ReconcileResult {
    id: String,
    registration: Arc<AtomicU64>,
    revision: u64,
    modes: Option<Modes>,
    running: Option<bool>,
}

/// The connection thread's end of the commands channel.
pub struct CommandReceiver {
    rx: async_channel::Receiver<Command>,
    stopping: Arc<AtomicBool>,
    /// Never carries a message; closed when the backend shuts down.
    stop: async_channel::Receiver<()>,
}

impl CommandReceiver {
    /// The next command. Once the backend is shutting down this is
    /// [`Command::Shutdown`], even if a full channel kept that command out.
    async fn recv(&self) -> std::result::Result<Command, async_channel::RecvError> {
        if self.stopping.load(Ordering::Acquire) {
            return Ok(Command::Shutdown);
        }
        self.rx.recv().await
    }

    fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }

    /// Finishes once the backend is shutting down, so that a request in
    /// flight does not hold up shutdown.
    async fn stopped(&self) {
        let _ = self.stop.recv().await;
    }
}

/// The commands channel, shared by the backend and every session handle.
#[derive(Clone)]
pub struct Commands {
    tx: async_channel::Sender<Command>,
    /// Set once the backend is shutting down; later requests fail at once.
    stopping: Arc<AtomicBool>,
    /// Closed once the backend is shutting down.
    stop: async_channel::Sender<()>,
}

impl Commands {
    pub fn new() -> (Self, CommandReceiver) {
        let (tx, rx) = async_channel::bounded(256);
        let stopping = Arc::new(AtomicBool::new(false));
        let (stop, stopped) = async_channel::bounded(1);
        (
            Self {
                tx,
                stopping: stopping.clone(),
                stop,
            },
            CommandReceiver {
                rx,
                stopping,
                stop: stopped,
            },
        )
    }

    /// Stop waiting for OpenCode: requests waiting for a connection, or for
    /// one in flight, fail at once, and so do later ones. Never blocks.
    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
        self.stop.close();
        let _ = self.tx.try_send(Command::Shutdown);
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
        build: impl FnOnce(
            async_channel::Sender<std::result::Result<T, tomori_backend::Error>>,
        ) -> Command,
    ) -> std::result::Result<T, tomori_backend::Error> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(tomori_backend::Error::new(
                tomori_backend::ErrorKind::Unavailable,
                SHUTTING_DOWN,
            ));
        }
        let (reply, waiting) = async_channel::bounded(1);
        let command = build(reply);
        self.tx.send(command).await.map_err(|_| {
            tomori_backend::Error::new(
                tomori_backend::ErrorKind::Unavailable,
                "the connection has closed",
            )
        })?;
        waiting.recv().await.map_err(|_| {
            tomori_backend::Error::new(
                tomori_backend::ErrorKind::Unavailable,
                "the connection has closed",
            )
        })?
    }
}

fn map_failure(failure: Failure) -> tomori_backend::Error {
    let kind = match failure.status {
        Some(404) => tomori_backend::ErrorKind::NotFound,
        Some(status) if (400..500).contains(&status) => tomori_backend::ErrorKind::Invalid,
        _ => tomori_backend::ErrorKind::Unavailable,
    };
    tomori_backend::Error::new(kind, failure.message)
}

/// How the backend finds the OpenCode service. It is called before every
/// connection attempt, so a service that restarts on another port or with
/// another password is found again.
/// It fails when the service's address is not known yet, as when its
/// registration file is missing mid-run, and the attempt is retried later.
pub type Resolve = Box<dyn FnMut() -> std::result::Result<Resolved, String> + Send>;

/// Run the connection until its command channel closes, reconnecting whenever
/// the service cannot be reached or its event stream ends. Blocking: called on
/// a dedicated thread.
pub fn run_thread(resolve: Resolve, commands: CommandReceiver) {
    block_on(drive(resolve, commands));
}

async fn drive(resolve: Resolve, commands: CommandReceiver) {
    // With these options, starting fails only if the system refuses its
    // threads or event queue, as spawning any thread can.
    let scheduler = NativeScheduler::with_options(NativeOptions {
        workers: 1,
        threads: 2,
        capacity: 256,
        ..Default::default()
    })
    .expect("start the connection scheduler");
    run(&scheduler, resolve, commands).await;
    block_on(scheduler.shutdown());
}

/// What outlives one connection to the service.
#[derive(Default)]
struct Sessions {
    registrations: HashMap<String, Registration>,
    /// How many events the event streams have dispatched, across connections.
    sequence: Arc<AtomicU64>,
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
    commands: CommandReceiver,
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
        let outcome = match resolve() {
            Ok(resolved) => {
                let outcome =
                    connect(scheduler, &resolved, &commands, &mut sessions, announce).await;
                match outcome {
                    Outcome::Unreachable(reason) => Outcome::Unreachable(format!(
                        "OpenCode is not reachable at {}: {reason}",
                        resolved.authority()
                    )),
                    outcome => outcome,
                }
            }
            Err(reason) => Outcome::Unreachable(format!("OpenCode is not reachable: {reason}")),
        };
        match outcome {
            Outcome::Shutdown => break,
            Outcome::Unreachable(message) => {
                // Report each distinct failure once, not every retry.
                if reported.as_deref() != Some(message.as_str()) {
                    eprintln!("{message}; retrying");
                    reported = Some(message.clone());
                }
                for command in sessions.held.drain(..) {
                    refuse(
                        command,
                        tomori_backend::Error::new(
                            tomori_backend::ErrorKind::Unavailable,
                            message.clone(),
                        ),
                    );
                }
                attempt_at = started + delay;
                delay = (delay * 2).min(RECONNECT_MAX);
                announce = true;
            }
            Outcome::Lost(reason) => {
                // The sessions were told when the stream was lost.
                eprintln!("the opencode event stream {reason}; reconnecting");
                reported = None;
                let now = scheduler.now();
                let wait = after_loss(delay, now.saturating_duration_since(started));
                attempt_at = now + wait;
                delay = (wait * 2).min(RECONNECT_MAX);
                announce = true;
            }
        }
    }
    // Deliver Closed to every session still bound, so no session waits forever
    // for its event channel to fill, and fail every waiting request.
    for (_, registration) in sessions.registrations.drain() {
        registration
            .events
            .close_with(Some("the connection ended".to_owned()));
    }
    let shutting_down =
        || tomori_backend::Error::new(tomori_backend::ErrorKind::Unavailable, SHUTTING_DOWN);
    for command in sessions.held.drain(..) {
        refuse(command, shutting_down());
    }
    // Sessions may still send commands, such as closing, until they are
    // dropped; answer them at once rather than leave them waiting.
    while let Ok(command) = commands.rx.recv().await {
        refuse(command, shutting_down());
    }
}

/// The delay before reconnecting after an event stream that was open for
/// `lasted` was lost, when the next delay was `delay`. A connection that is
/// lost soon after it opens counts as a failed attempt, so a service that keeps
/// dropping the stream is not reconnected to, and every session read again, in
/// a tight loop; one that lasted resets the backoff.
fn after_loss(delay: Duration, lasted: Duration) -> Duration {
    if lasted >= RECONNECT_MAX {
        RECONNECT_INITIAL
    } else {
        delay
    }
}

/// Answer `command` with `error` without performing it.
fn refuse(command: Command, error: tomori_backend::Error) {
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
        Command::Locate { reply, .. } => {
            let _ = reply.try_send(Err(error));
        }
        Command::Prompt { reply, .. }
        | Command::Interrupt { reply, .. }
        | Command::Decide { reply, .. }
        | Command::Answer { reply, .. }
        | Command::SetMode { reply, .. }
        | Command::Instruction { reply, .. }
        | Command::ForgetInstructions { reply, .. }
        | Command::DeleteSession { reply, .. }
        | Command::DeleteWorker { reply, .. }
        | Command::RegisterTools { reply, .. }
        | Command::UnregisterTools { reply, .. } => {
            let _ = reply.try_send(Err(error));
        }
        // Closing needs no connection; see `hold`.
        Command::Close { .. } | Command::Shutdown => {}
    }
}

/// Whether nobody waits for `command`'s reply any more, for a command that
/// waited to be started.
fn abandoned(command: &Command) -> bool {
    match command {
        Command::Sessions { reply, .. } => reply.is_closed(),
        Command::Create { reply, .. } => reply.is_closed(),
        Command::Attach { reply, .. } => reply.is_closed(),
        Command::Snapshot { reply, .. } => reply.is_closed(),
        Command::Locate { reply, .. } => reply.is_closed(),
        Command::Prompt { reply, .. }
        | Command::Interrupt { reply, .. }
        | Command::Decide { reply, .. }
        | Command::Answer { reply, .. }
        | Command::SetMode { reply, .. }
        | Command::Instruction { reply, .. }
        | Command::ForgetInstructions { reply, .. }
        | Command::DeleteSession { reply, .. }
        | Command::DeleteWorker { reply, .. }
        | Command::RegisterTools { reply, .. }
        | Command::UnregisterTools { reply, .. } => reply.is_closed(),
        Command::Close { .. } | Command::Shutdown => {
            unreachable!("a close or shutdown is never held or waiting")
        }
    }
}

/// Take a command while no connection is open: a close is performed at once,
/// and anything else waits for the next attempt.
/// Returns false when the backend is shutting down.
fn hold(sessions: &mut Sessions, command: Command) -> bool {
    match command {
        Command::Shutdown => return false,
        Command::Close { id, identity } => {
            close_registration(&mut sessions.registrations, &id, &identity);
            withdraw(
                &mut sessions.held,
                &identity,
                |command| command,
                |command| command,
            );
        }
        command if sessions.held.len() < MAX_HELD => sessions.held.push_back(command),
        command => refuse(
            command,
            tomori_backend::Error::new(tomori_backend::ErrorKind::Unavailable, TOO_MANY_HELD),
        ),
    }
    true
}

/// Run `future`, a step of a connection attempt, for at most `timeout` while
/// no connection is open, taking commands meanwhile as [`hold`] does. The
/// attempt ends unreachable if the step fails or takes too long, and shuts
/// down once the command channel has closed.
async fn holding<T, S: Timers>(
    future: impl Future<Output = std::result::Result<T, String>>,
    timers: &S,
    commands: &CommandReceiver,
    sessions: &mut Sessions,
    timeout: Duration,
) -> std::result::Result<T, Outcome> {
    let cancellation = Cancellation::new();
    let deadline = timers.now() + timeout;
    let mut timer = timers.timer();
    let mut future = Fused::new(future);
    loop {
        futures_util::select! {
            result = &mut future => return result.map_err(Outcome::Unreachable),
            command = Fused::new(commands.recv()) => match command {
                Ok(command) => if !hold(sessions, command) {
                    return Err(Outcome::Shutdown);
                },
                Err(_) => return Err(Outcome::Shutdown),
            },
            due = Fused::new(timers.timer_wait_until(&mut timer, deadline, &cancellation)) => {
                return Err(Outcome::Unreachable(match due {
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
    commands: &CommandReceiver,
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
                Ok(command) => if !hold(sessions, command) {
                    return false;
                },
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
    commands: &CommandReceiver,
    sessions: &mut Sessions,
    announce: bool,
) -> Outcome
where
    <S as Scheduler<'scope>>::Socket: 'static,
{
    let cancellation = Cancellation::new();
    let lookup = lookup(scheduler, resolved, &cancellation);
    let address = match holding(lookup, scheduler, commands, sessions, CONNECT_TIMEOUT).await {
        Ok(address) => address,
        Err(outcome) => return outcome,
    };
    // Only options that are not valid are refused, and the defaults are.
    let (client, driver) = Client::new(
        scheduler,
        Endpoint::new(address),
        base_http_client::Options::default(),
    )
    .expect("the default client options are valid");

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
    with_driver(outcome, driver_result)
}

/// The service's first address, resolved by `resolver`.
async fn lookup<R: AddressResolution>(
    resolver: &R,
    resolved: &Resolved,
    cancellation: &Cancellation,
) -> std::result::Result<base_scheduler::SocketAddress, String> {
    resolver
        .address_resolve(
            Some(resolved.host.trim_matches(['[', ']'])),
            AddressService::Port(resolved.port),
            AddressHints::default(),
            cancellation,
        )
        .await
        .map_err(|error| format!("resolving {}: {error}", resolved.authority()))?
        .entries()
        .next()
        .map(|entry| entry.address())
        .ok_or_else(|| "the server's address did not resolve to any endpoint".to_owned())
}

/// How a connection ended, with why its HTTP driver ended if it failed, when
/// that may be why the event stream was lost.
fn with_driver(
    outcome: Outcome,
    driver: std::result::Result<(), base_http_client::Error>,
) -> Outcome {
    match (outcome, driver) {
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
    let mut unsettled = Vec::new();
    for (id, registration) in registrations.iter_mut() {
        // Their futures were dropped with the connection.
        registration.snapshot_in_flight = None;
        registration.reconcile_in_flight = None;
        registration.abandon_discovery();
        // A restore being read is read again on the next connection.
        registration.restore_in_flight = false;
        // What the workers do is unknown until their records are read again on the next connection: what happened
        // meanwhile, such as a run ending, is in no event that will arrive.
        registration.activity_in_flight = false;
        registration.activity_due = registration.activity.unsync();
        let reading = std::mem::take(&mut registration.restore_reading);
        registration.restore_due.extend(reading);
        registration.snapshot_deletions.clear();
        registration.snapshot_deletions_overflow = false;
        if registration.workers_ready {
            // Held events are kept: text deltas are in no record until their
            // message ends. Those the next read reflects are dropped then.
            //
            // Failures are not forgotten, and a connection lost again before
            // the session was read counts too, so a service that keeps
            // dropping its stream cannot leave a space reconnecting forever.
            let again = registration.resync.is_some();
            let resync = registration.resync.get_or_insert_with(Resync::default);
            resync.retry_at = None;
            if again {
                resync.interruptions = resync.interruptions.saturating_add(1);
                if resync.interruptions >= RESYNC_INTERRUPTIONS {
                    unsettled.push(id.clone());
                    continue;
                }
            }
            registration.events.deliver(Event::Reconnecting {
                reason: DISCONNECTED.to_owned(),
            });
        } else {
            unread.push(id.clone());
        }
    }
    for id in unsettled {
        end_registration(
            registrations,
            &id,
            Some(
                "the connection to OpenCode kept dropping before the session could be read again"
                    .to_owned(),
            ),
        );
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
type RestoreFutures<'a> = FuturesUnordered<Pin<Box<dyn Future<Output = RestoreResult> + 'a>>>;
type ActivityFutures<'a> = FuturesUnordered<Pin<Box<dyn Future<Output = ActivityResult> + 'a>>>;
type DiscoveryFutures<'a> = FuturesUnordered<Pin<Box<dyn Future<Output = DiscoveryResult> + 'a>>>;

/// Open the server's global event stream, then serve it and the commands,
/// polling the stream, the command channel, the requests in flight, reads and
/// a periodic reconciliation of each session's running state together, until
/// the backend closes or the stream ends. Commands held while no connection
/// was open are started first, in order, and sessions whose events were lost
/// are read again.
async fn connected<S: Timers>(
    client: &Client<'_>,
    resolved: &Resolved,
    commands: &CommandReceiver,
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
        Ok(stream) => stream,
        Err(outcome) => return outcome,
    };
    if announce {
        eprintln!("connected to OpenCode at {}", resolved.authority());
    }
    let Sessions {
        registrations,
        held,
        sequence,
    } = sessions;

    let mut groups = EventGroups::default();
    let mut timer = timers.timer();
    let mut next_reconcile = timers.now() + RECONCILE_INTERVAL;
    let mut reconcile_offset = 0usize;
    let mut snapshots: SnapshotFutures<'_> = FuturesUnordered::new();
    let mut resyncs: ResyncFutures<'_> = FuturesUnordered::new();
    let mut restores: RestoreFutures<'_> = FuturesUnordered::new();
    let mut activities: ActivityFutures<'_> = FuturesUnordered::new();
    let mut discoveries: DiscoveryFutures<'_> = FuturesUnordered::new();
    let mut reconciliations: FuturesUnordered<Pin<Box<dyn Future<Output = ReconcileResult> + '_>>> =
        FuturesUnordered::new();
    // The stream is open, so whatever happens from now on is observed: read
    // again every session whose events were lost.
    start_resyncs(
        registrations,
        &mut resyncs,
        sequence,
        client,
        resolved,
        cancellation,
        timers,
    );

    // Commands not yet started, in the order they arrived, each with whether
    // it waited for this connection; the requests in flight; and the sessions
    // with one in flight, whose later commands wait for it.
    let mut waiting: VecDeque<(Command, bool)> =
        held.drain(..).map(|command| (command, true)).collect();
    let mut requests: RequestFutures<'_> = FuturesUnordered::new();
    let mut busy: HashSet<String> = HashSet::new();
    // Why the event stream was lost, while the requests in flight are
    // answered, and until when they may be.
    let mut lost: Option<String> = None;
    let mut grace: Option<Instant> = None;
    let mut grace_timer = timers.timer();

    let outcome = loop {
        retry_worker_delivery(registrations);
        // A request in flight is not cut short: the loss is noticed once each
        // has been answered.
        if let Some(reason) = &lost
            && requests.is_empty()
        {
            break Outcome::Lost(reason.clone());
        }
        // Commands that waited for this connection are started first, in
        // order, then the others as they arrive. Once shutting down, none is
        // started, held or not, and the shutdown is seen below as the command
        // channel is.
        if lost.is_none() & !commands.is_stopping() {
            start_commands(
                &mut waiting,
                &mut busy,
                &mut requests,
                registrations,
                &mut snapshots,
                sequence,
                client,
                resolved,
                cancellation,
                timers,
            );
        }
        let reading = lost.is_none();
        let until = grace;
        futures_util::select! {
            () = Fused::new(commands.stopped()) => break Outcome::Shutdown,
            () = Fused::new(async {
                match until {
                    Some(until) => {
                        let _ = timers.timer_wait_until(&mut grace_timer, until, cancellation).await;
                    }
                    None => std::future::pending().await,
                }
            }) => {
                // Those still unanswered are abandoned, so that one OpenCode
                // never answers cannot keep the backend from reconnecting.
                break Outcome::Lost(lost.clone().expect("a grace period follows a loss"));
            }
            command = Fused::new(commands.recv()) => match command {
                Ok(Command::Shutdown) | Err(_) => break Outcome::Shutdown,
                // Once the stream is lost, commands wait for the next
                // connection, as while none is open.
                Ok(Command::Close { id, identity }) if !reading => {
                    close_registration(registrations, &id, &identity);
                    withdraw(held, &identity, |command| command, |command| command);
                }
                Ok(command) if !reading => {
                    if held.len() < MAX_HELD {
                        held.push_back(command);
                    } else {
                        refuse(command, tomori_backend::Error::new(tomori_backend::ErrorKind::Unavailable, TOO_MANY_HELD));
                    }
                }
                Ok(Command::Close { id, identity }) => {
                    close_session(registrations, &mut waiting, &id, &identity);
                }
                Ok(command @ Command::Snapshot { .. }) => {
                    prepare_command(command, false, registrations, &mut snapshots, sequence, client, resolved, cancellation, timers);
                }
                Ok(command) if waiting.len() >= MAX_WAITING => {
                    refuse(command, tomori_backend::Error::new(tomori_backend::ErrorKind::Unavailable, TOO_MANY_WAITING));
                }
                Ok(command) => waiting.push_back((command, false)),
            },
            completed = Fused::new(async {
                if requests.is_empty() {
                    std::future::pending::<Option<(Option<String>, Option<Completion>)>>().await
                } else {
                    requests.next().await
                }
            }) => {
                let (session, completion) = completed.expect("a request was in flight");
                if let Some(session) = session {
                    busy.remove(&session);
                }
                if let Some(completion) = completion {
                    complete(completion, registrations);
                }
            }
            completed = Fused::new(async {
                if snapshots.is_empty() {
                    std::future::pending::<Option<SnapshotResult>>().await
                } else {
                    snapshots.next().await
                }
            }) => {
                finish_snapshot(registrations, completed.expect("a snapshot was in flight"));
                start_discoveries(registrations, &mut discoveries, client, resolved, cancellation, timers);
                continue;
            }
            completed = Fused::new(async {
                if discoveries.is_empty() {
                    std::future::pending::<Option<DiscoveryResult>>().await
                } else {
                    discoveries.next().await
                }
            }) => {
                let completed = completed.expect("a discovery was in flight");
                finish_discovery(registrations, completed, timers.now());
                start_discoveries(registrations, &mut discoveries, client, resolved, cancellation, timers);
                continue;
            }
            completed = Fused::new(async {
                if resyncs.is_empty() {
                    std::future::pending::<Option<ResyncResult>>().await
                } else {
                    resyncs.next().await
                }
            }) => {
                finish_resync(registrations, completed.expect("a read was in flight"), timers.now());
                // A slot is free: start the next waiting session at once.
                start_resyncs(registrations, &mut resyncs, sequence, client, resolved, cancellation, timers);
                start_discoveries(registrations, &mut discoveries, client, resolved, cancellation, timers);
                start_restores(registrations, &mut restores, client, resolved, cancellation, timers);
                start_activity_syncs(registrations, &mut activities, client, resolved, cancellation, timers);
                continue;
            }
            completed = Fused::new(async {
                if activities.is_empty() {
                    std::future::pending::<Option<ActivityResult>>().await
                } else {
                    activities.next().await
                }
            }) => {
                finish_activity_sync(registrations, completed.expect("a read was in flight"));
                start_activity_syncs(registrations, &mut activities, client, resolved, cancellation, timers);
                continue;
            }
            completed = Fused::new(async {
                if restores.is_empty() {
                    std::future::pending::<Option<RestoreResult>>().await
                } else {
                    restores.next().await
                }
            }) => {
                finish_restore(registrations, completed.expect("a restore was in flight"));
                start_restores(registrations, &mut restores, client, resolved, cancellation, timers);
                continue;
            }
            completed = Fused::new(async {
                if reconciliations.is_empty() {
                    std::future::pending::<Option<ReconcileResult>>().await
                } else {
                    reconciliations.next().await
                }
            }) => {
                let completed = completed.expect("a reconciliation was in flight");
                if let Some(registration) = registrations.get_mut(&completed.id)
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
            result = Fused::new(async {
                if reading {
                    stream.read(cancellation).await
                } else {
                    std::future::pending().await
                }
            }) => {
                let loss = match result {
                    Err(error) => Some(format!("failed: {error}")),
                    Ok(frame) => {
                        let end = frame.is_end();
                        let mut oversized = false;
                        if let base_http::body::BodyFrame::Data { data, .. } = frame {
                            let complete = groups.push(&data);
                            for group in &complete {
                                if let Some(payload) = sse_payload(group) {
                                    let number = sequence.fetch_add(1, Ordering::AcqRel) + 1;
                                    dispatch_events_at(&payload, registrations, number);
                                }
                            }
                            start_restores(registrations, &mut restores, client, resolved, cancellation, timers);
                            oversized = groups.oversized();
                        }
                        if oversized {
                            // Dropping part of it could lose an event, so the
                            // stream is treated as lost, and every session is
                            // read again on the next one.
                            Some(format!("sent an event larger than {MAX_EVENT_BYTES} bytes"))
                        } else {
                            end.then(|| "ended".to_owned())
                        }
                    }
                };
                if let Some(reason) = loss {
                    lose(&mut lost, &mut waiting, held, registrations, reason);
                    // The reads in flight are abandoned with the stream; each
                    // session is read again on the next connection.
                    snapshots.clear();
                    resyncs.clear();
                    restores.clear();
                    discoveries.clear();
                    reconciliations.clear();
                    grace = Some(timers.now() + IN_FLIGHT_GRACE);
                }
                continue;
            }
            // Re-check each session's running state against the server's record,
            // so a missed idle event cannot leave a busy state — and the queue it
            // gates — stuck. A no-op while no session is registered.
            due = Fused::new(async {
                if reading {
                    timers.timer_wait_until(&mut timer, next_reconcile, cancellation).await
                } else {
                    std::future::pending().await
                }
            }) => {
                if let Err(error) = due {
                    break Outcome::Lost(format!("stopped: the state reconciliation timer failed: {error}"));
                }
                let mut ids: Vec<_> = registrations.keys().cloned().collect();
                for id in &ids {
                    close_abandoned_registration(registrations, id);
                }
                // A session read again after reconnecting reports its
                // running state from that read.
                // One not yet read reports it with its initial state.
                ids.retain(|id| registrations.get(id).is_some_and(|registration| registration.resync.is_none() && registration.workers_ready));
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
                // Retry a restore whose record had no text yet.
            for registration in registrations.values_mut() {
                let retry = std::mem::take(&mut registration.restore_retry);
                registration.restore_due.extend(retry);
            }
            start_restores(registrations, &mut restores, client, resolved, cancellation, timers);
            start_activity_syncs(registrations, &mut activities, client, resolved, cancellation, timers);
            // Retry a session whose read again failed.
                start_resyncs(registrations, &mut resyncs, sequence, client, resolved, cancellation, timers);
                // Go on with a discovery whose retry is due.
                start_discoveries(registrations, &mut discoveries, client, resolved, cancellation, timers);
                retry_worker_delivery(registrations);
                next_reconcile = timers.now() + RECONCILE_INTERVAL;
                continue;
            }
        }
    };
    // Shutdown abandons the requests in flight rather than wait for OpenCode,
    // and the commands still waiting are refused or held with the others.
    drop(requests);
    for (command, _) in waiting.into_iter().rev() {
        held.push_front(command);
    }
    if matches!(outcome, Outcome::Lost(_)) && lost.is_none() {
        connection_lost(registrations);
    }
    cancellation.cancel();
    outcome
}

/// The event stream was lost: every session is told at once, the requests in
/// flight may still be answered, and the commands not started yet wait for the
/// next connection, ahead of those held later, however many there are.
fn lose(
    lost: &mut Option<String>,
    waiting: &mut VecDeque<(Command, bool)>,
    held: &mut VecDeque<Command>,
    registrations: &mut HashMap<String, Registration>,
    reason: String,
) {
    *lost = Some(reason);
    for (command, _) in waiting.drain(..).rev() {
        held.push_front(command);
    }
    connection_lost(registrations);
}

/// Fail at once the commands in `queue` made through the registration
/// `identity`, which has closed, rather than when they would have started.
fn withdraw<T>(
    queue: &mut VecDeque<T>,
    identity: &Arc<AtomicU64>,
    command: impl Fn(&T) -> &Command,
    into: impl Fn(T) -> Command,
) {
    let mut kept = VecDeque::with_capacity(queue.len());
    for entry in queue.drain(..) {
        if command_registration(command(&entry)).is_some_and(|other| Arc::ptr_eq(other, identity)) {
            refuse(
                into(entry),
                tomori_backend::Error::new(
                    tomori_backend::ErrorKind::Closed,
                    "The session registration has closed.",
                ),
            );
        } else {
            kept.push_back(entry);
        }
    }
    *queue = kept;
}

/// The session a command is performed for, whose commands are performed one
/// at a time, in the order they arrived; `None` for one that may run beside
/// any other.
fn command_session(command: &Command) -> Option<&str> {
    match command {
        Command::Attach { id, .. }
        | Command::Prompt { id, .. }
        | Command::Interrupt { id, .. }
        | Command::Decide { id, .. }
        | Command::Answer { id, .. }
        | Command::SetMode { id, .. }
        | Command::Instruction { id, .. }
        | Command::ForgetInstructions { id, .. }
        | Command::DeleteSession { id, .. }
        | Command::DeleteWorker { worker: id, .. } => Some(id),
        Command::Sessions { .. }
        | Command::Create { .. }
        | Command::Snapshot { .. }
        | Command::Locate { .. }
        | Command::RegisterTools { .. }
        | Command::UnregisterTools { .. }
        | Command::Close { .. }
        | Command::Shutdown => None,
    }
}

/// The registration a command was made through, if it names one.
fn command_registration(command: &Command) -> Option<&Arc<AtomicU64>> {
    match command {
        Command::Prompt { identity, .. }
        | Command::Interrupt { identity, .. }
        | Command::Decide { identity, .. }
        | Command::Answer { identity, .. }
        | Command::SetMode { identity, .. }
        | Command::Instruction { identity, .. } => Some(identity),
        _ => None,
    }
}

/// Close `id`'s registration, failing at once the commands made through it
/// that have not started, rather than when its request in flight is answered.
fn close_session(
    registrations: &mut HashMap<String, Registration>,
    waiting: &mut VecDeque<(Command, bool)>,
    id: &str,
    identity: &Arc<AtomicU64>,
) {
    close_registration(registrations, id, identity);
    withdraw(
        waiting,
        identity,
        |(command, _)| command,
        |(command, _)| command,
    );
}

/// Start the waiting commands that can start, in order: up to
/// [`MAX_REQUESTS`] requests are in flight, and a session's next command waits
/// for the one in flight for it. Each request runs beside the event stream,
/// reads and every other request, so a stalled one holds up only its own
/// session's later commands.
#[allow(clippy::too_many_arguments)]
fn start_commands<'a, S: Timers>(
    waiting: &mut VecDeque<(Command, bool)>,
    busy: &mut HashSet<String>,
    requests: &mut RequestFutures<'a>,
    registrations: &mut HashMap<String, Registration>,
    snapshots: &mut SnapshotFutures<'a>,
    sequence: &Arc<AtomicU64>,
    client: &'a Client<'_>,
    resolved: &'a Resolved,
    cancellation: &'a Cancellation,
    timers: &'a S,
) {
    let mut kept = VecDeque::new();
    while let Some((command, held)) = waiting.pop_front() {
        let session = command_session(&command).map(str::to_owned);
        if requests.len() >= MAX_REQUESTS
            || session
                .as_ref()
                .is_some_and(|session| busy.contains(session))
        {
            kept.push_back((command, held));
            continue;
        }
        if let Some(request) = prepare_command(
            command,
            held,
            registrations,
            snapshots,
            sequence,
            client,
            resolved,
            cancellation,
            timers,
        ) {
            if let Some(session) = &session {
                busy.insert(session.clone());
            }
            requests.push(Box::pin(async move { (session, request.await) }));
        }
    }
    *waiting = kept;
}

/// Read a session within [`SNAPSHOT_TIMEOUT`], cancelling its requests when the
/// read ends either way. `on_messages` is called just before its messages are
/// requested.
///
/// Its conversation, running state and pending requests must be read in
/// time, or the read fails, since a space must not open on any of them
/// incomplete. Its workers are listed for the rest of the time: a family too
/// large to walk in time is returned in part, with the walk to go on with.
#[allow(clippy::too_many_arguments)]
async fn bounded_snapshot<S: Timers>(
    client: &Client<'_>,
    resolved: &Resolved,
    id: &str,
    modes: Modes,
    local: &Cancellation,
    cancellation: &Cancellation,
    timers: &S,
    on_messages: impl FnOnce(),
) -> tomori_backend::Result<(Snapshot, Option<api::FamilyWalk>)> {
    let deadline = timers.now() + SNAPSHOT_TIMEOUT;
    // The connection's cancellation ends the wait early, and so the read.
    let expired = || async {
        let remaining = deadline.saturating_duration_since(timers.now());
        timers.sleep(remaining, cancellation).await
    };
    let read = async {
        let parts = futures_lite::future::or(
            async { Ok(api::SnapshotParts::fetch(client, resolved, id, local, on_messages).await) },
            async {
                let timer = expired().await;
                Err(tomori_backend::Error::new(
                    tomori_backend::ErrorKind::Unavailable,
                    format!("Snapshot timed out or was cancelled: {timer:?}"),
                ))
            },
        )
        .await?
        .map_err(map_failure)?;
        let mut walk = api::FamilyWalk::new(id);
        let walked = futures_lite::future::or(
            async {
                walk.run(client, resolved, local, timers)
                    .await
                    .map(|()| true)
                    .map_err(map_failure)
            },
            async {
                let _ = expired().await;
                Ok(false)
            },
        )
        .await?;
        Ok((parts, walk, walked))
    };
    let (read, active) = futures_lite::future::zip(
        read,
        api::active_sessions_with_timeout(client, resolved, local, timers, Duration::from_secs(1)),
    )
    .await;
    local.cancel();
    let (mut parts, mut walk, walked) = read?;
    parts.children = walk.take_found();
    Ok((
        snapshot(parts, id, modes, active.as_ref()),
        (!walked).then_some(walk),
    ))
}

/// Report a worker as running only on positive evidence: OpenCode listing it
/// among the sessions it runs. Timestamps persist across process restarts, and
/// a background child may be absent from the foreground-drain listing, so
/// without that evidence a worker is kept but not claimed to run.
fn running_evidence(workers: &mut [Worker], active: Option<&HashSet<String>>) {
    for worker in workers {
        if active.is_some_and(|active| active.contains(&worker.id)) {
            worker.state = WorkerState::Running;
        } else if worker.state == WorkerState::Running {
            worker.state = WorkerState::Unknown;
        }
    }
}

/// Answer a snapshot a session asked for, installing it if it was read.
fn finish_snapshot(registrations: &mut HashMap<String, Registration>, completed: SnapshotResult) {
    let result = match registrations.get_mut(&completed.id) {
        Some(registration)
            if Arc::ptr_eq(&registration.events.worker_epoch, &completed.registration) =>
        {
            registration.snapshot_in_flight = None;
            let overflow = std::mem::take(&mut registration.snapshot_deletions_overflow);
            let rest = completed.rest;
            let result = completed.result.and_then(|mut snapshot| {
                if overflow {
                    return Err(tomori_backend::Error::new(
                        tomori_backend::ErrorKind::Unavailable,
                        "Too many sessions were deleted during the snapshot; please retry.",
                    ));
                }
                registration.install(&mut snapshot)?;
                registration.follow(&snapshot);
                // The workers not found in time are discovered in the
                // background; the space opens knowing its list is incomplete.
                registration.continue_discovery(rest);
                snapshot.workers_incomplete = registration.workers_partial.clone();
                Ok(registration.events.snapshot_reply(snapshot))
            });
            registration.snapshot_deletions.clear();
            result
        }
        _ => Err(tomori_backend::Error::new(
            tomori_backend::ErrorKind::Closed,
            "The session was closed during its snapshot.",
        )),
    };
    let _ = completed.reply.try_send(result);
}

/// Start a snapshot a session asked for, isolated from the event dispatcher.
#[allow(clippy::too_many_arguments)]
fn schedule_snapshot<'a, S: Timers>(
    registrations: &mut HashMap<String, Registration>,
    snapshots: &mut SnapshotFutures<'a>,
    sequence: &Arc<AtomicU64>,
    id: String,
    identity: &Arc<AtomicU64>,
    reply: async_channel::Sender<tomori_backend::Result<SnapshotReply>>,
    client: &'a Client<'_>,
    resolved: &'a Resolved,
    cancellation: &'a Cancellation,
    timers: &'a S,
) {
    let Some(registration) = registrations
        .get_mut(&id)
        .filter(|registration| Arc::ptr_eq(&registration.events.worker_epoch, identity))
    else {
        let _ = reply.try_send(Err(tomori_backend::Error::new(
            tomori_backend::ErrorKind::Closed,
            "The session is not registered.",
        )));
        return;
    };
    if registration.resync.is_some() {
        let _ = reply.try_send(Err(tomori_backend::Error::new(
            tomori_backend::ErrorKind::Unavailable,
            "The session is being read again after reconnecting to OpenCode; please retry.",
        )));
        return;
    }
    if registration.snapshot_in_flight.is_some() || snapshots.len() >= MAX_SNAPSHOTS {
        let _ = reply.try_send(Err(tomori_backend::Error::new(
            tomori_backend::ErrorKind::Unavailable,
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
    // Which of the events held for the initial snapshot its listing must
    // already reflect.
    registration.opening.mark = Arc::new(AtomicU64::new(0));
    let mark = registration.opening.mark.clone();
    let sequence = sequence.clone();
    snapshots.push(Box::pin(async move {
        let result = bounded_snapshot(
            client,
            resolved,
            &id,
            modes,
            &local,
            cancellation,
            timers,
            move || mark.store(sequence.load(Ordering::Acquire), Ordering::Release),
        )
        .await;
        let (result, rest) = match result {
            Ok((snapshot, rest)) => (Ok(snapshot), rest),
            Err(error) => (Err(error), None),
        };
        SnapshotResult {
            id,
            registration: identity,
            reply,
            result,
            rest,
        }
    }));
}

/// Start reading again the sessions whose events were lost, as many at once as
/// [`MAX_RESYNCS`] allows, skipping any whose last attempt failed less than
/// [`RECONCILE_INTERVAL`] ago. Each read is isolated and bounded like a
/// snapshot, so a slow session never holds up another or the event stream.
/// The read also lists the directory's agents and the session's current one,
/// which a restarted service may report differently.
#[allow(clippy::too_many_arguments)]
fn start_resyncs<'a, S: Timers>(
    registrations: &mut HashMap<String, Registration>,
    resyncs: &mut ResyncFutures<'a>,
    sequence: &Arc<AtomicU64>,
    client: &'a Client<'_>,
    resolved: &'a Resolved,
    cancellation: &'a Cancellation,
    timers: &'a S,
) {
    let now = timers.now();
    let mut ids: Vec<_> = registrations
        .iter()
        .filter(|(_, registration)| {
            registration.snapshot_in_flight.is_none()
                && registration
                    .resync
                    .as_ref()
                    .is_some_and(|resync| resync.retry_at.is_none_or(|retry_at| retry_at <= now))
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
        let resync = registration.resync.as_mut().expect("listed above");
        resync.modes_changed = false;
        resync.changes.clear();
        resync.changes_overflow = false;
        resync.mark = Arc::new(AtomicU64::new(0));
        let mark = resync.mark.clone();
        let sequence = sequence.clone();
        let identity = registration.events.worker_epoch.clone();
        let current = registration.modes.clone();
        let directory = registration.directory.clone();
        let known = registration.known_workers();
        resyncs.push(Box::pin(async move {
            // The agents are listed alongside the read, and no longer than it.
            let listing = local.child();
            let (result, modes) = futures_lite::future::zip(
                async {
                    let reading = local.child();
                    let result = bounded_snapshot(
                        client,
                        resolved,
                        &id,
                        current,
                        &reading,
                        cancellation,
                        timers,
                        move || mark.store(sequence.load(Ordering::Acquire), Ordering::Release),
                    )
                    .await;
                    listing.cancel();
                    let (result, rest) = match result {
                        Ok((snapshot, rest)) => (Ok(snapshot), rest),
                        Err(error) => (Err(error), None),
                    };
                    let missing = match &result {
                        Ok(snapshot) => {
                            let listed: HashSet<&str> = snapshot
                                .workers
                                .iter()
                                .map(|worker| worker.id.as_str())
                                .collect();
                            let omitted = known
                                .into_iter()
                                .filter(|known| !listed.contains(known.as_str()))
                                .collect();
                            missing_workers(client, resolved, omitted, &local, timers).await
                        }
                        Err(_) => Vec::new(),
                    };
                    (result, rest, missing)
                },
                bounded_modes(client, resolved, &id, &directory, &listing, timers),
            )
            .await;
            local.cancel();
            let (result, rest, missing) = result;
            ResyncResult {
                id,
                registration: identity,
                result,
                rest,
                missing,
                modes,
            }
        }));
    }
}

/// The modes of session `id` in `directory`, read again within [`SNAPSHOT_TIMEOUT`]: the agent its record names, among those listed again. Optional, like the listing at attach: `None` if its record cannot be read in time, or the read is cancelled, which keeps the modes already known.
async fn bounded_modes<S: Timers>(
    client: &Client<'_>,
    resolved: &Resolved,
    id: &str,
    directory: &str,
    cancellation: &Cancellation,
    timers: &S,
) -> Option<Modes> {
    futures_lite::future::race(
        async {
            let info = api::session_info(client, resolved, id, cancellation)
                .await
                .ok()?;
            Some(modes(client, resolved, directory, &info, cancellation).await)
        },
        async {
            let _ = timers.sleep(SNAPSHOT_TIMEOUT, cancellation).await;
            None
        },
    )
    .await
}

/// The workers among `omitted` that OpenCode answers do not exist, looked up
/// by identifier, at most eight at once, within [`SNAPSHOT_TIMEOUT`]. A lookup
/// that fails, finds the worker, or does not finish in time leaves it out, as
/// only a session OpenCode reports not found is evidence of deletion; those
/// found missing in time are kept.
async fn missing_workers<S: Timers>(
    client: &Client<'_>,
    resolved: &Resolved,
    omitted: Vec<String>,
    cancellation: &Cancellation,
    timers: &S,
) -> Vec<String> {
    const MAX_LOOKUPS: usize = 8;
    if omitted.is_empty() {
        return Vec::new();
    }
    let lookups = cancellation.child();
    let looking = &lookups;
    let missing = std::cell::RefCell::new(Vec::new());
    futures_lite::future::race(
        async {
            let mut omitted = omitted.into_iter();
            let mut running = FuturesUnordered::new();
            loop {
                while running.len() < MAX_LOOKUPS
                    && let Some(id) = omitted.next()
                {
                    running.push(async move {
                        let found = api::session_info(client, resolved, &id, looking).await;
                        (id, found)
                    });
                }
                match running.next().await {
                    Some((
                        id,
                        Err(Failure {
                            status: Some(404), ..
                        }),
                    )) => missing.borrow_mut().push(id),
                    Some(_) => {}
                    None => break,
                }
            }
        },
        async {
            let _ = timers.sleep(SNAPSHOT_TIMEOUT, looking).await;
        },
    )
    .await;
    lookups.cancel();
    missing.into_inner()
}

/// Present a session read again after reconnecting: its workers are merged,
/// then [`Event::Resynchronized`] is delivered, followed by the events held
/// since the connection was restored. A failed read is retried after
/// [`RECONCILE_INTERVAL`]; a session that could not be read in
/// [`RESYNC_ATTEMPTS`] attempts is closed with the reason, which says when the
/// session no longer exists. A session whose event channel lacks room for the
/// state and its held events is closed too, rather than losing them.
fn finish_resync(
    registrations: &mut HashMap<String, Registration>,
    completed: ResyncResult,
    now: Instant,
) {
    let Some(registration) = registrations.get_mut(&completed.id).filter(|registration| {
        Arc::ptr_eq(&registration.events.worker_epoch, &completed.registration)
    }) else {
        return;
    };
    registration.snapshot_in_flight = None;
    let overflow = std::mem::take(&mut registration.snapshot_deletions_overflow);
    let mut resync = registration.resync.take().unwrap_or_default();
    let rest = completed.rest;
    let result = completed.result.and_then(|mut snapshot| {
        if overflow || resync.changes_overflow {
            return Err(tomori_backend::Error::new(
                tomori_backend::ErrorKind::Unavailable,
                "too many workers changed while it was read",
            ));
        }
        registration.merge_workers(std::mem::take(&mut snapshot.workers), &resync.changes)?;
        registration.remove_missing(&completed.missing, &resync.changes);
        registration.follow(&snapshot);
        // The workers not found in time are discovered in the background.
        registration.continue_discovery(rest);
        Ok(snapshot)
    });
    registration.snapshot_deletions.clear();
    resync.changes.clear();
    let reason = match result {
        Ok(_) if registration.events.room() <= resync.events.len() => {
            "too many OpenCode events were waiting when the session was read again after reconnecting"
                .to_owned()
        }
        Ok(mut snapshot) => {
            if let Some(modes) = completed.modes
                && !modes.offered.is_empty()
            {
                // An agent switch seen during the read is newer than it.
                let current = if resync.modes_changed {
                    registration.modes.current.take()
                } else {
                    modes.current
                };
                registration.modes = Modes {
                    offered: modes.offered,
                    current,
                };
            }
            snapshot.modes = registration.modes.clone();
            // Held events the read reflects are dropped; every other one is
            // delivered after it, so a fragment may repeat but none is lost.
            let mark = resync.mark.load(Ordering::Acquire);
            let held: Vec<Event> = std::mem::take(&mut resync.events)
                .into_iter()
                .filter(|(sequence, event)| {
                    !reflected(&snapshot, event) && !(*sequence <= mark && requests(event))
                })
                .map(|(_, event)| event)
                .collect();
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
            // Only messages this read lists unfinished still wait to end.
            registration.restoring = HashSet::from_iter(
                snapshot
                    .messages
                    .iter()
                    .filter(|message| {
                        !message.completed && message.role == tomori_backend::Role::Assistant
                    })
                    .map(|message| message.id.clone()),
            );
            registration
                .events
                .deliver(Event::Resynchronized { snapshot });
            for event in held {
                registration.deliver_content(event);
            }
            registration.publish_workers();
            return;
        }
        Err(error) => {
            resync.failures = resync.failures.saturating_add(1);
            if resync.failures < RESYNC_ATTEMPTS {
                // Wait for the next cadence: a service still starting, or
                // about to be lost again, must not use up every attempt at once.
                resync.retry_at = Some(now + RECONCILE_INTERVAL);
                registration.resync = Some(resync);
                return;
            }
            if error.kind == tomori_backend::ErrorKind::NotFound {
                "the OpenCode session no longer exists".to_owned()
            } else {
                format!(
                    "could not read the OpenCode session again after reconnecting: {}",
                    error.message
                )
            }
        }
    };
    end_registration(registrations, &completed.id, Some(reason));
}

/// The sessions whose discoveries may start at `now`, in the order of their
/// identifiers, as many as `free` slots allow: one waiting to be tried again
/// after a failure waits until its retry is due.
fn due_discoveries(
    registrations: &HashMap<String, Registration>,
    now: Instant,
    free: usize,
) -> Vec<String> {
    let mut ids: Vec<_> = registrations
        .iter()
        .filter(|(_, registration)| registration.discovery_due(now))
        .map(|(id, _)| id.clone())
        .collect();
    ids.sort();
    ids.truncate(free);
    ids
}

/// Go on, in the background, walking the families that reads ran out of time
/// for, at most [`MAX_DISCOVERIES`] at once, in the order of their sessions'
/// identifiers. Each is isolated like a read and lists at most eight parents'
/// children at once; one whose listing OpenCode does not answer in time, or
/// answers with an error, stops, and is tried again after a while (see
/// [`DISCOVERY_RETRIES`]).
fn start_discoveries<'a, S: Timers>(
    registrations: &mut HashMap<String, Registration>,
    discoveries: &mut DiscoveryFutures<'a>,
    client: &'a Client<'_>,
    resolved: &'a Resolved,
    cancellation: &'a Cancellation,
    timers: &'a S,
) {
    let free = MAX_DISCOVERIES.saturating_sub(discoveries.len());
    for id in due_discoveries(registrations, timers.now(), free) {
        let registration = registrations.get_mut(&id).expect("listed above");
        let local = cancellation.child();
        let (mut walk, number) = registration.start_discovery(local.clone());
        let identity = registration.events.worker_epoch.clone();
        discoveries.push(Box::pin(async move {
            // A listing OpenCode does not answer in time ends the walk.
            let result = walk.run(client, resolved, &local, timers).await;
            // Like a read, a worker is reported running only on evidence.
            let active = match result {
                Ok(()) => {
                    api::active_sessions_with_timeout(
                        client,
                        resolved,
                        &local,
                        timers,
                        Duration::from_secs(1),
                    )
                    .await
                }
                Err(_) => None,
            };
            local.cancel();
            DiscoveryResult {
                id,
                registration: identity,
                number,
                found: walk.take_found(),
                result,
                walk,
                active,
            }
        }));
    }
}

/// Take what a background discovery found into its session's workers, and
/// report them, as every worker known once the family was walked whole, or
/// as possibly incomplete, with the reason, if the discovery failed, when it
/// is tried again after a while, from `now`, while attempts remain. One for a
/// registration that has since closed, or stopped since, as when the session
/// was read again, is ignored, and so is not a failure.
fn finish_discovery(
    registrations: &mut HashMap<String, Registration>,
    completed: DiscoveryResult,
    now: Instant,
) {
    let Some(registration) = registrations.get_mut(&completed.id).filter(|registration| {
        Arc::ptr_eq(&registration.events.worker_epoch, &completed.registration)
            && registration.discovery_in_flight.is_some()
            && registration.discoveries == completed.number
    }) else {
        return;
    };
    let mut workers = api::workers_from_children(&completed.found);
    running_evidence(&mut workers, completed.active.as_ref());
    let walk = completed.walk;
    registration.adopt_discovered(
        workers,
        completed
            .result
            .err()
            .map(|failure| (walk, failure.message)),
        now,
    );
    registration.publish_workers();
}

/// Read again, once each, the messages that ended after a read listed them
/// still being written, at most [`MAX_RESYNCS`] sessions at once. OpenCode
/// records a message's text when it ends, so this recovers what was streamed
/// while the event stream was down. Each read is isolated and bounded like a
/// snapshot; one that fails leaves the text the space already has.
fn start_restores<'a, S: Timers>(
    registrations: &mut HashMap<String, Registration>,
    restores: &mut RestoreFutures<'a>,
    client: &'a Client<'_>,
    resolved: &'a Resolved,
    cancellation: &'a Cancellation,
    timers: &'a S,
) {
    for (id, registration) in registrations.iter_mut() {
        if restores.len() >= MAX_RESYNCS {
            break;
        }
        if registration.restore_in_flight || registration.restore_due.is_empty() {
            continue;
        }
        registration.restore_in_flight = true;
        let wanted = std::mem::take(&mut registration.restore_due);
        registration.restore_reading.clone_from(&wanted);
        let identity = registration.events.worker_epoch.clone();
        let id = id.clone();
        let local = cancellation.child();
        restores.push(Box::pin(async move {
            let listed = futures_lite::future::race(
                async {
                    api::session_messages(client, resolved, &id, &local)
                        .await
                        .ok()
                        .map(|value| snapshot_messages(&value))
                },
                async {
                    let _ = timers.sleep(SNAPSHOT_TIMEOUT, &local).await;
                    None
                },
            )
            .await;
            local.cancel();
            // The text is recorded when it ends, which may be before the
            // message itself is marked completed, as when tools follow it.
            let messages: Vec<_> = listed
                .unwrap_or_default()
                .into_iter()
                .filter(|message| {
                    wanted.contains(&message.id)
                        && (message.completed
                            || message.parts.iter().any(|part| {
                                matches!(part, tomori_backend::Part::Text { text } if !text.is_empty())
                            }))
                })
                .collect();
            let unrecorded = wanted
                .into_iter()
                .filter(|id| !messages.iter().any(|message| message.id == *id))
                .collect();
            RestoreResult {
                id,
                registration: identity,
                messages,
                unrecorded,
            }
        }));
    }
}

/// The records of a session's workers read again after the stream was lost: each worker's latest messages, oldest
/// first, and its inbox, or `None` where they could not be read, and when.
struct ActivityResult {
    id: String,
    registration: Arc<AtomicU64>,
    read: Vec<(String, Option<WorkerRecords>)>,
    at: u64,
}

/// A worker's latest messages, oldest first, its inbox and its pending permissions and forms.
type WorkerRecords = (Vec<Value>, Vec<Value>, Vec<Value>);

/// Read again, once the session itself has been, the records of the workers whose activity became unknown when the
/// stream was lost, at most [`MAX_RESYNCS`] sessions at once, each worker in turn, each read bounded like a snapshot.
/// See `design/events.md`.
fn start_activity_syncs<'a, S: Timers>(
    registrations: &mut HashMap<String, Registration>,
    activities: &mut ActivityFutures<'a>,
    client: &'a Client<'_>,
    resolved: &'a Resolved,
    cancellation: &'a Cancellation,
    timers: &'a S,
) {
    let room = MAX_RESYNCS.saturating_sub(activities.len());
    let due = registrations.iter_mut().filter(|(_, registration)| {
        !registration.activity_in_flight
            && !registration.activity_due.is_empty()
            && registration.workers_ready
            && registration.resync.is_none()
    });
    for (id, registration) in due.take(room) {
        registration.activity_in_flight = true;
        let workers = std::mem::take(&mut registration.activity_due);
        let identity = registration.events.worker_epoch.clone();
        let id = id.clone();
        let local = cancellation.child();
        activities.push(Box::pin(async move {
            let mut read = Vec::with_capacity(workers.len());
            for worker in workers {
                let records = futures_lite::future::race(
                    async {
                        let messages = api::recent_messages(
                            client,
                            resolved,
                            &worker,
                            crate::activity::SYNCED_MESSAGES,
                            &local,
                        )
                        .await
                        .ok()?;
                        // An inbox or requests that cannot be read hold nothing known.
                        let inbox = api::session_inbox(client, resolved, &worker, &local)
                            .await
                            .unwrap_or_default();
                        let requests = api::session_requests(client, resolved, &worker, &local)
                            .await
                            .unwrap_or_default();
                        Some((messages, inbox, requests))
                    },
                    async {
                        let _ = timers.sleep(SNAPSHOT_TIMEOUT, &local).await;
                        None
                    },
                )
                .await;
                read.push((worker, records));
            }
            local.cancel();
            ActivityResult {
                id,
                registration: identity,
                read,
                at: crate::activity::now(),
            }
        }));
    }
}

/// Install the workers' records read again, and report their activity, unless the stream was lost again meanwhile,
/// when they are read again on the next connection.
fn finish_activity_sync(
    registrations: &mut HashMap<String, Registration>,
    completed: ActivityResult,
) {
    let Some(registration) = registrations.get_mut(&completed.id).filter(|registration| {
        Arc::ptr_eq(&registration.events.worker_epoch, &completed.registration)
            && registration.activity_in_flight
    }) else {
        return;
    };
    registration.activity_in_flight = false;
    for (worker, records) in &completed.read {
        let records = records.as_ref().map(|(messages, inbox, requests)| {
            (messages.as_slice(), inbox.as_slice(), requests.as_slice())
        });
        registration.activity.sync(worker, records, completed.at);
    }
    registration.publish_activity();
}

/// Deliver the messages read again once they ended, after any events already
/// delivered for them, or hold them if the stream has been lost meanwhile.
fn finish_restore(registrations: &mut HashMap<String, Registration>, completed: RestoreResult) {
    let Some(registration) = registrations.get_mut(&completed.id).filter(|registration| {
        Arc::ptr_eq(&registration.events.worker_epoch, &completed.registration)
    }) else {
        return;
    };
    registration.restore_in_flight = false;
    registration.restore_reading.clear();
    for id in completed.unrecorded {
        let attempts = registration.restore_attempts.entry(id.clone()).or_default();
        *attempts += 1;
        if *attempts < RESYNC_ATTEMPTS {
            registration.restore_retry.push(id);
        } else {
            // The record never had its text; the space keeps what it has.
            registration.restore_attempts.remove(&id);
        }
    }
    let mut overflowed = false;
    for message in completed.messages {
        registration.restore_attempts.remove(&message.id);
        let event = Event::MessageRestored { message };
        match registration.resync.as_mut() {
            // Held behind the read after the next connection, which delivers
            // it after the state read again.
            Some(resync) => overflowed |= !resync.hold(u64::MAX, event),
            None => {
                registration.events.deliver(event);
            }
        }
    }
    if overflowed {
        end_registration(
            registrations,
            &completed.id,
            Some(
                "too many OpenCode events arrived before the session could be read again after reconnecting"
                    .to_owned(),
            ),
        );
    }
}

/// Whether the conversation `snapshot` lists already shows `event`. OpenCode
/// keeps a message's text in its records only once the message has ended, so
/// a listed message still being written has none of the text streamed so far:
/// its text is reflected only once the message is listed as completed. A
/// message start, a tool call, a completion and a tool's end are reflected
/// when the listing shows them.
fn reflected(snapshot: &Snapshot, event: &Event) -> bool {
    let listed = |id: &str| snapshot.messages.iter().find(|message| message.id == id);
    let tool = |id: &str| {
        snapshot
            .messages
            .iter()
            .flat_map(|message| &message.parts)
            .find_map(|part| match part {
                tomori_backend::Part::Tool {
                    id: tool, finished, ..
                } if tool == id => Some(*finished),
                _ => None,
            })
    };
    match event {
        Event::MessageStarted { message, .. } => listed(message).is_some(),
        Event::TextDelta { message, .. } | Event::MessageCompleted { message } => {
            listed(message).is_some_and(|message| message.completed)
        }
        Event::ToolStarted { id, .. } => tool(id).is_some(),
        Event::ToolFinished { id, .. } => tool(id) == Some(true),
        _ => false,
    }
}

/// Whether the initial `snapshot` already shows `event`, held since its
/// session was registered: as [`reflected`], except that a message it lists
/// still being written keeps its start. Every event since that start was
/// held, so the space can follow the message, and speak it, from its start.
fn shown(snapshot: &Snapshot, event: &Event) -> bool {
    match event {
        Event::MessageStarted { message, .. } => snapshot
            .messages
            .iter()
            .any(|listed| listed.id == *message && listed.completed),
        event => reflected(snapshot, event),
    }
}

/// The places in `held` of the text deltas whose text `snapshot` already
/// records for a message it lists still being written. OpenCode records a
/// message's text when the text ends, before the message itself completes, as
/// when a tool follows it, while text still streaming is in no record. So a
/// part of a message's text, held up to the end reported for it, is shown if
/// that end was reported before the listing was requested, the `mark`th event,
/// and the parts so ended match the last the snapshot records with text: each
/// exactly, except that the first may have started before the session was
/// registered, unless the message's start was held, so only its end was. Any
/// doubt keeps the text, which may then repeat but is never lost.
fn recorded_text(snapshot: &Snapshot, held: &[(u64, Event)], mark: u64) -> HashSet<usize> {
    let mut recorded = HashSet::new();
    for message in snapshot
        .messages
        .iter()
        .filter(|message| !message.completed)
    {
        let parts: Vec<&str> = message
            .parts
            .iter()
            .filter_map(|part| match part {
                tomori_backend::Part::Text { text } if !text.is_empty() => Some(text.as_str()),
                _ => None,
            })
            .collect();
        let mut started = false;
        // The ended parts' deltas and text, in order.
        let mut ended: Vec<(Vec<usize>, String)> = Vec::new();
        let mut current = (Vec::new(), String::new());
        for (index, (sequence, event)) in held.iter().enumerate() {
            match event {
                Event::MessageStarted { message: id, .. } if *id == message.id => started = true,
                Event::TextDelta { message: id, text } if *id == message.id => {
                    current.0.push(index);
                    current.1.push_str(text);
                }
                Event::MessageCompleted { message: id } if *id == message.id => {
                    if *sequence > mark {
                        break;
                    }
                    ended.push(std::mem::take(&mut current));
                }
                _ => {}
            }
        }
        let Some(first) = parts.len().checked_sub(ended.len()) else {
            continue;
        };
        let shown =
            ended
                .iter()
                .zip(&parts[first..])
                .enumerate()
                .all(|(place, ((_, text), part))| {
                    if place == 0 && !started {
                        part.ends_with(text.as_str())
                    } else {
                        part == text
                    }
                });
        if shown {
            recorded.extend(ended.into_iter().flat_map(|(indices, _)| indices));
        }
    }
    recorded
}

/// Whether `event` changes the pending permissions or questions, which a read
/// lists after it requests the messages: one held from before that request is
/// reflected by the read's lists, and delivering it afterwards could present a
/// request that has since been resolved.
fn requests(event: &Event) -> bool {
    matches!(
        event,
        Event::PermissionRequested(_)
            | Event::PermissionResolved { .. }
            | Event::QuestionAsked(_)
            | Event::QuestionResolved { .. }
    )
}

/// The largest event the stream may send, as for any response.
const MAX_EVENT_BYTES: usize = api::MAX_JSON_BYTES;

/// Splits the event stream's bytes into its event groups, each ending with a
/// blank line. A group is decoded once it is whole, so a character split
/// between two reads is kept, and at most [`MAX_EVENT_BYTES`] of an
/// unfinished one are held.
#[derive(Default)]
struct EventGroups {
    pending: Vec<u8>,
    /// How much of `pending` has been searched for a group's end.
    searched: usize,
}

impl EventGroups {
    /// Add `data`, returning the groups it completes.
    fn push(&mut self, data: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(data);
        let mut groups = Vec::new();
        let mut start = 0;
        // A separator may straddle the previous read.
        let mut from = self.searched.saturating_sub(1);
        while let Some(offset) = self.pending[from..]
            .windows(2)
            .position(|pair| pair == b"\n\n")
        {
            let end = from + offset + 2;
            groups.push(String::from_utf8_lossy(&self.pending[start..end]).into_owned());
            start = end;
            from = end;
        }
        self.pending.drain(..start);
        self.searched = self.pending.len();
        groups
    }

    /// Whether the unfinished group has outgrown the limit.
    fn oversized(&self) -> bool {
        self.pending.len() > MAX_EVENT_BYTES
    }
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

/// Feed one complete SSE event to its session and to any parent that owns its
/// worker. Child events are authoritative; no paginated scan infers removal.
#[cfg(test)]
fn dispatch_events(event: &str, registrations: &mut HashMap<String, Registration>) {
    dispatch_events_at(event, registrations, 0);
}

/// [`dispatch_events`] for the event stream's `sequence`th event, which a
/// session being read again holds with that number.
fn dispatch_events_at(
    event: &str,
    registrations: &mut HashMap<String, Registration>,
    sequence: u64,
) {
    let Ok(value) = serde_json::from_str::<Value>(event) else {
        return;
    };
    apply_worker_event(&value, registrations);
    apply_worker_activity(&value, registrations);
    let mut overflowed = None;
    if let Some(session_id) = wire::session_id_of(&value)
        && let Some(registration) = registrations.get_mut(&session_id)
    {
        registration.event_revision = registration.event_revision.wrapping_add(1);
        // A session moved lists the agents of where it is now.
        if value.get("type").and_then(Value::as_str) == Some("session.moved")
            && let Some(directory) = value
                .get("data")
                .and_then(|data| wire::moved_to(data, &session_id))
        {
            directory.clone_into(&mut registration.directory);
        }
        if let Some(agent) = wire::agent_switched(&value, &session_id) {
            if registration.modes.current.as_deref() != Some(agent.as_str()) {
                registration.modes.current = Some(agent);
                // While the session is read again, its modes are reported
                // with that state.
                match registration.resync.as_mut() {
                    Some(resync) => resync.modes_changed = true,
                    None => registration.publish_modes(),
                }
            }
        } else if registration.resync.is_some() || !registration.workers_ready {
            let opening = registration.resync.is_none();
            let held = registration
                .resync
                .as_mut()
                .unwrap_or(&mut registration.opening);
            if let Some(event) = wire::event_for(
                &value,
                &session_id,
                &mut registration
                    .seen
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            ) && !(opening
                && registration
                    .events
                    .ordinary_incomplete
                    .load(Ordering::Acquire))
            {
                // Held from the loss until the session is read: text deltas
                // are not kept in OpenCode's records until the message ends,
                // so no read covers them, and dropping one would lose it.
                // Likewise before the initial snapshot, which they follow,
                // unless events were lost already and it will be refused.
                if !held.hold(sequence, event) {
                    if opening {
                        // The space has not opened, so its snapshot is
                        // refused rather than presented without them.
                        held.events.clear();
                        registration.events.mark_ordinary_incomplete();
                    } else {
                        overflowed = Some(session_id.clone());
                    }
                }
            }
        } else if let Some(event) = {
            let mut seen = registration
                .seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            wire::event_for(&value, &session_id, &mut seen)
        } {
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
                registration.deliver_content(event);
            }
        }
    }
    if let Some(session_id) = overflowed {
        // Holding more would grow without bound, and dropping any could lose
        // text, so the space closes and says why.
        end_registration(
            registrations,
            &session_id,
            Some(
                "too many OpenCode events arrived before the session could be read again after reconnecting"
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
        // A session being read again keeps what this event says about the
        // worker, which is newer than the listing that read may return.
        if registration.snapshot_in_flight.is_some()
            && let Some(resync) = registration.resync.as_mut()
        {
            let state = match kind {
                "session.execution.started" => Some(WorkerState::Running),
                "session.execution.succeeded" => Some(WorkerState::Completed),
                "session.execution.failed" => Some(WorkerState::Error),
                "session.execution.interrupted" => Some(WorkerState::Stopped),
                _ => None,
            };
            let title = (kind == "session.renamed")
                .then(|| data.get("title").and_then(Value::as_str))
                .flatten()
                .map(bounded_worker_title);
            if state.is_some() || title.is_some() {
                if resync.changes.contains_key(child)
                    || resync.changes.len() < api::MAX_TRACKED_WORKERS
                {
                    let change = resync.changes.entry(child.to_owned()).or_default();
                    if state.is_some() {
                        change.0 = state;
                    }
                    if title.is_some() {
                        change.1 = title;
                    }
                } else {
                    resync.changes_overflow = true;
                }
            }
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
                    registration.insert_child(Worker {
                        id: child.to_owned(),
                        parent: parent.map(str::to_owned),
                        title: data
                            .get("title")
                            .and_then(Value::as_str)
                            .map(bounded_worker_title)
                            .unwrap_or_else(|| "worker".to_owned()),
                        state: WorkerState::Unknown,
                    });
                    true
                }
            }
            "session.deleted" => registration.forget_deleted(child),
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

/// Note what an event says a known worker is doing, for each registered session that knows it. See
/// `design/events.md`.
fn apply_worker_activity(value: &Value, registrations: &mut HashMap<String, Registration>) {
    let Some(kind) = value
        .get("type")
        .and_then(Value::as_str)
        .filter(|kind| crate::activity::relevant(kind))
    else {
        return;
    };
    let Some(data) = value.get("data") else {
        return;
    };
    let Some(worker) = data
        .get("sessionID")
        .or_else(|| data.pointer("/form/sessionID"))
        .and_then(Value::as_str)
    else {
        return;
    };
    let at = crate::activity::now();
    for registration in registrations.values_mut() {
        if kind == "session.deleted" {
            registration.activity.forget(worker);
        } else if registration.children.contains_key(worker)
            && registration.activity.apply(worker, kind, data, at)
            && !registration.activity_due.iter().any(|due| due == worker)
        {
            // First seen other than by its creation, as after a restart: what it was doing is read from its records.
            registration.activity_due.push(worker.to_owned());
        }
    }
}

fn retry_worker_delivery(registrations: &mut HashMap<String, Registration>) {
    for registration in registrations.values_mut() {
        if registration.workers_dirty {
            registration.publish_workers();
        }
        registration.publish_activity();
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
        if let Some(cancellation) = registration.discovery_in_flight {
            cancellation.cancel();
        }
        registration.events.close_with(reason);
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
    reply: &async_channel::Sender<tomori_backend::Result<()>>,
) -> bool {
    if registrations
        .get(id)
        .is_some_and(|registration| Arc::ptr_eq(&registration.events.worker_epoch, identity))
    {
        true
    } else {
        let _ = reply.try_send(Err(tomori_backend::Error::new(
            tomori_backend::ErrorKind::Closed,
            "The session registration has closed.",
        )));
        false
    }
}

/// The sessions a participant can attach to, most recently active first. The
/// listing asks for top-level, unarchived sessions; a child session (a
/// subagent's worker) or an archived one that a server returns anyway is left
/// out. A session is busy when `active`, OpenCode's list of the sessions its
/// process runs, has it, or, when that list could not be read, when its record
/// says so, as for a space's own session (see `design/agent-backends.md`).
fn session_summaries(sessions: &[Value], active: Option<&HashSet<String>>) -> Vec<SessionSummary> {
    let mut listed: Vec<(f64, SessionSummary)> = sessions
        .iter()
        .filter(|info| info.get("parentID").is_none_or(Value::is_null))
        .filter(|info| info.pointer("/time/archived").is_none_or(Value::is_null))
        .filter_map(|info| {
            let id = info.get("id")?.as_str()?.to_owned();
            let busy = match active {
                Some(active) => active.contains(&id),
                None => api::running_from_record(info).unwrap_or(false),
            };
            let updated = info
                .pointer("/time/updated")
                .and_then(Value::as_f64)
                .unwrap_or(f64::NEG_INFINITY);
            Some((
                updated,
                SessionSummary {
                    id,
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
                    busy,
                },
            ))
        })
        .collect();
    // Stable, so sessions OpenCode lists with one time keep its order.
    listed.sort_by(|(left, _), (right, _)| right.total_cmp(left));
    listed.into_iter().map(|(_, summary)| summary).collect()
}

/// The rest of a command whose network part has finished, applied to the
/// registrations by the loop.
enum Completion {
    Created {
        id: String,
        directory: String,
        modes: Modes,
        reply: async_channel::Sender<tomori_backend::Result<(String, RegistrationEvents)>>,
    },
    Attached {
        id: String,
        directory: String,
        modes: Modes,
        reply: async_channel::Sender<tomori_backend::Result<RegistrationEvents>>,
    },
    /// A worker of `session` was deleted, or no longer existed: it and its own workers are no longer observed.
    WorkerDeleted {
        session: String,
        worker: String,
        reply: async_channel::Sender<tomori_backend::Result<()>>,
    },
    /// Agents listed for a session that offered none.
    Listed {
        id: String,
        identity: Arc<AtomicU64>,
        modes: Modes,
    },
}

/// A command's request to OpenCode, performed beside the loop's other work,
/// and what to apply to the registrations once it has finished.
type Request<'a> = Pin<Box<dyn Future<Output = Option<Completion>> + 'a>>;

/// The requests in flight, each with the session it was performed for.
type RequestFutures<'a> =
    FuturesUnordered<Pin<Box<dyn Future<Output = (Option<String>, Option<Completion>)> + 'a>>>;

/// Prepare one command that waited to be started. A snapshot is handled at
/// once; for the others this returns the request for the loop to perform, whose result, if
/// it needs the registrations, the loop applies with [`complete`]. A command
/// that waited for this connection is dropped if its caller has given up.
#[allow(clippy::too_many_arguments)]
fn prepare_command<'a, S: Timers>(
    command: Command,
    held: bool,
    registrations: &mut HashMap<String, Registration>,
    snapshots: &mut SnapshotFutures<'a>,
    sequence: &Arc<AtomicU64>,
    client: &'a Client<'_>,
    resolved: &'a Resolved,
    cancellation: &'a Cancellation,
    timers: &'a S,
) -> Option<Request<'a>> {
    // A caller that gave up while OpenCode was unreachable no longer wants
    // its command performed, such as a session created for nobody.
    if held && abandoned(&command) {
        return None;
    }
    let request: Request<'a> = match command {
        // Neither waits to be started: each is taken as it arrives.
        Command::Shutdown | Command::Close { .. } => {
            unreachable!("a close or shutdown is never held or waiting")
        }
        Command::Snapshot {
            id,
            identity,
            reply,
        } => {
            schedule_snapshot(
                registrations,
                snapshots,
                sequence,
                id,
                &identity,
                reply,
                client,
                resolved,
                cancellation,
                timers,
            );
            return None;
        }
        Command::RegisterTools {
            directory,
            server,
            reply,
        } => Box::pin(async move {
            let result = api::put_mcp(client, resolved, &directory, &server, cancellation)
                .await
                .map_err(map_failure);
            let _ = reply.try_send(result);
            None
        }),
        Command::Locate { id, reply } => Box::pin(async move {
            let result = api::session_info(client, resolved, &id, cancellation)
                .await
                .map(|info| api::session_directory(&info).map(str::to_owned))
                .map_err(map_failure);
            let _ = reply.try_send(result);
            None
        }),
        Command::UnregisterTools {
            directory,
            name,
            reply,
        } => Box::pin(async move {
            let result =
                removed(api::delete_mcp(client, resolved, &directory, &name, cancellation).await)
                    .map_err(map_failure);
            let _ = reply.try_send(result);
            None
        }),
        Command::Sessions { directory, reply } => Box::pin(async move {
            let result = match api::list_sessions(client, resolved, &directory, cancellation).await
            {
                Ok(sessions) => {
                    let active = api::active_sessions_with_timeout(
                        client,
                        resolved,
                        cancellation,
                        timers,
                        api::ACTIVE_LOOKUP_TIMEOUT,
                    )
                    .await;
                    Ok(session_summaries(&sessions, active.as_ref()))
                }
                Err(failure) => Err(map_failure(failure)),
            };
            let _ = reply.try_send(result);
            None
        }),
        Command::Create { spec, reply } => Box::pin(async move {
            let info = match api::create_session(
                client,
                resolved,
                &spec.directory,
                Some(&spec.title),
                cancellation,
            )
            .await
            {
                Ok(info) => info,
                Err(failure) => {
                    let _ = reply.try_send(Err(map_failure(failure)));
                    return None;
                }
            };
            let Some(id) = info.get("id").and_then(Value::as_str).map(str::to_owned) else {
                let _ = reply.try_send(Err(tomori_backend::Error::new(
                    tomori_backend::ErrorKind::Unavailable,
                    "the server did not return the new session's id",
                )));
                return None;
            };
            let modes = modes(client, resolved, &spec.directory, &info, cancellation).await;
            Some(Completion::Created {
                id,
                directory: spec.directory,
                modes,
                reply,
            })
        }),
        Command::Attach {
            id,
            directory,
            reply,
        } => {
            close_abandoned_registration(registrations, &id);
            if registrations.contains_key(&id) {
                let _ = reply.try_send(Err(tomori_backend::Error::new(
                    tomori_backend::ErrorKind::Unavailable,
                    ALREADY_ATTACHED,
                )));
                return None;
            }
            Box::pin(async move {
                let info = match api::session_info(client, resolved, &id, cancellation).await {
                    Ok(info) => info,
                    Err(failure) => {
                        let _ = reply.try_send(Err(map_failure(failure)));
                        return None;
                    }
                };
                let session_directory = api::session_directory(&info).unwrap_or_default();
                if session_directory != directory {
                    let _ = reply.try_send(Err(tomori_backend::Error::new(
                        tomori_backend::ErrorKind::NotFound,
                        "the session is not in that directory",
                    )));
                    return None;
                }
                let modes = modes(client, resolved, &directory, &info, cancellation).await;
                Some(Completion::Attached {
                    id,
                    directory,
                    modes,
                    reply,
                })
            })
        }
        Command::Prompt {
            id,
            identity,
            key,
            text,
            reply,
        } => {
            if !owns_registration(registrations, &id, &identity, &reply) {
                return None;
            }
            Box::pin(async move {
                let result = api::send_prompt(client, resolved, &id, &key, &text, cancellation)
                    .await
                    .map_err(map_failure);
                let _ = reply.try_send(result);
                None
            })
        }
        Command::Interrupt {
            id,
            identity,
            reply,
        } => {
            if !owns_registration(registrations, &id, &identity, &reply) {
                return None;
            }
            Box::pin(async move {
                let result = api::interrupt(client, resolved, &id, cancellation)
                    .await
                    .map_err(map_failure);
                let _ = reply.try_send(result);
                None
            })
        }
        Command::Decide {
            id,
            identity,
            request,
            decision,
            reply,
        } => {
            if !owns_registration(registrations, &id, &identity, &reply) {
                return None;
            }
            Box::pin(async move {
                let result =
                    api::reply_permission(client, resolved, &id, &request, &decision, cancellation)
                        .await
                        .map_err(map_failure);
                let _ = reply.try_send(result);
                None
            })
        }
        Command::Answer {
            id,
            identity,
            form,
            answer,
            reply,
        } => {
            if !owns_registration(registrations, &id, &identity, &reply) {
                return None;
            }
            Box::pin(async move {
                let result = api::reply_form(client, resolved, &id, &form, &answer, cancellation)
                    .await
                    .map_err(map_failure);
                let _ = reply.try_send(result);
                None
            })
        }
        Command::Instruction {
            id,
            identity,
            key,
            value,
            reply,
        } => {
            if !owns_registration(registrations, &id, &identity, &reply) {
                return None;
            }
            Box::pin(async move {
                let result = match value {
                    Some(value) => {
                        api::put_instruction(client, resolved, &id, &key, &value, cancellation)
                            .await
                    }
                    None => removed(
                        api::delete_instruction(client, resolved, &id, &key, cancellation).await,
                    ),
                };
                let _ = reply.try_send(result.map_err(map_failure));
                None
            })
        }
        // A session's commands are performed in order, so an attachment that finished first is seen here, and one that
        // follows sets its own entries afterwards.
        Command::ForgetInstructions { id, reply, .. } if registrations.contains_key(&id) => {
            let _ = reply.try_send(Ok(()));
            return None;
        }
        // A session's commands are performed in order, and its space's handle is closed before it is deleted, so an
        // attachment seen here is a space's that bound it again.
        Command::DeleteSession { id, reply } if registrations.contains_key(&id) => {
            let _ = reply.try_send(Err(tomori_backend::Error::new(
                tomori_backend::ErrorKind::Invalid,
                "the session is bound to a space again",
            )));
            return None;
        }
        // Stopped first, so that no turn goes on in a session being deleted; one that no longer exists is already gone.
        Command::DeleteSession { id, reply } => Box::pin(async move {
            let result = match removed(api::interrupt(client, resolved, &id, cancellation).await) {
                Ok(()) => removed(api::delete_session(client, resolved, &id, cancellation).await),
                failed => failed,
            };
            let _ = reply.try_send(result.map_err(map_failure));
            None
        }),
        // A session attached is a space's own, never a worker to delete.
        Command::DeleteWorker { worker, reply, .. } if registrations.contains_key(&worker) => {
            let _ = reply.try_send(Err(tomori_backend::Error::new(
                tomori_backend::ErrorKind::Invalid,
                "the worker's session is bound to a space",
            )));
            return None;
        }
        Command::DeleteWorker {
            session,
            worker,
            force,
            reply,
        } => {
            let bound: HashSet<String> = registrations
                .keys()
                .filter(|id| **id != session)
                .cloned()
                .collect();
            let known = registrations.get(&session);
            let family = known
                .map(|registration| known_family(&registration.children, &worker))
                .unwrap_or_default();
            let state = known
                .and_then(|registration| registration.children.get(&worker))
                .map(|known| known.state);
            Box::pin(async move {
                let found = WorkerFamily {
                    session: &session,
                    worker: &worker,
                    state,
                    bound: &bound,
                    known: &family,
                };
                match delete_worker(client, resolved, found, force, cancellation).await {
                    Ok(()) => Some(Completion::WorkerDeleted {
                        session,
                        worker,
                        reply,
                    }),
                    Err(error) => {
                        let _ = reply.try_send(Err(error));
                        None
                    }
                }
            })
        }
        Command::ForgetInstructions { id, keys, reply } => Box::pin(async move {
            let mut result = Ok(());
            for key in keys {
                result = removed(
                    api::delete_instruction(client, resolved, &id, &key, cancellation).await,
                );
                if result.is_err() {
                    break;
                }
            }
            let _ = reply.try_send(result.map_err(map_failure));
            None
        }),
        Command::SetMode {
            id,
            identity,
            mode,
            reply,
        } => {
            if !owns_registration(registrations, &id, &identity, &reply) {
                return None;
            }
            let registration = registrations.get_mut(&id).expect("owned above");
            // OpenCode answers an empty listing while it loads a directory,
            // so a session offering none lists its agents again first.
            let list = registration.modes.offered.is_empty() && registration.listings > 0;
            if list {
                registration.listings -= 1;
            }
            let offered = registration.modes.offered.clone();
            let current = registration.modes.current.clone();
            let directory = registration.directory.clone();
            Box::pin(async move {
                let listed = if list {
                    let info =
                        current.map_or(Value::Null, |agent| serde_json::json!({ "agent": agent }));
                    Some(modes(client, resolved, &directory, &info, cancellation).await)
                        .filter(|modes| !modes.offered.is_empty())
                } else {
                    None
                };
                // OpenCode accepts any agent name, so only an offered one is
                // passed on. The mode changes when the server confirms the
                // switch on the event stream.
                let offered = listed.as_ref().map_or(&offered, |modes| &modes.offered);
                let result = if offered.iter().any(|offered| offered.id == mode) {
                    api::switch_agent(client, resolved, &id, &mode, cancellation)
                        .await
                        .map_err(map_failure)
                } else {
                    Err(tomori_backend::Error::new(
                        tomori_backend::ErrorKind::Invalid,
                        format!("the session does not offer the {mode} agent"),
                    ))
                };
                let _ = reply.try_send(result);
                listed.map(|modes| Completion::Listed {
                    id,
                    identity,
                    modes,
                })
            })
        }
    };
    Some(request)
}

/// The outcome of removing an instruction entry or an MCP server: OpenCode answers not found when the session, and so
/// the entry, or the server no longer exists, which is what removing it asked for.
fn removed(result: Result<(), Failure>) -> Result<(), Failure> {
    match result {
        Err(Failure {
            status: Some(404), ..
        }) => Ok(()),
        result => result,
    }
}

/// The most sessions followed up a worker's chain of parents to the session it is to descend from.
const MAX_WORKER_DEPTH: usize = 32;

/// `worker`'s own workers among `children`, a session's known workers, at any depth, each with its state as last
/// observed, not including `worker` itself. Each worker has one parent, so each is found once, unless its parents
/// lead back to `worker`, which is not taken again.
fn known_family(children: &BTreeMap<String, Worker>, worker: &str) -> Vec<(String, WorkerState)> {
    let mut family: Vec<(String, WorkerState)> = Vec::new();
    let mut parents = vec![worker.to_owned()];
    while let Some(parent) = parents.pop() {
        for child in children.values() {
            if child.parent.as_deref() == Some(parent.as_str()) && child.id != worker {
                family.push((child.id.clone(), child.state));
                parents.push(child.id.clone());
            }
        }
    }
    family
}

/// A worker to delete, and what is known around it: the session it is to descend from, its state as observed, if it
/// was, the sessions bound to other spaces, and its own workers known, with their states.
struct WorkerFamily<'a> {
    session: &'a str,
    worker: &'a str,
    state: Option<WorkerState>,
    bound: &'a HashSet<String>,
    known: &'a [(String, WorkerState)],
}

/// The session a record names as its parent.
fn parent_of(record: &Value) -> Option<String> {
    record
        .get("parentID")
        .and_then(Value::as_str)
        .filter(|parent| !parent.is_empty())
        .map(str::to_owned)
}

/// Delete `found.worker`, with its own workers, after checking OpenCode's own records: its chain of parents must reach
/// `found.session` through no session bound to another space, and, unless `force`, neither it nor its own workers known
/// may be running, as observed, or, for a worker never observed, as its record's times say. It is stopped first, as a
/// deleted session is, and with `force`, so are its own workers known to run. One that no longer exists is already
/// gone.
async fn delete_worker(
    client: &Client<'_>,
    resolved: &Resolved,
    found: WorkerFamily<'_>,
    force: bool,
    cancellation: &Cancellation,
) -> tomori_backend::Result<()> {
    let refused =
        |message: &str| tomori_backend::Error::new(tomori_backend::ErrorKind::Invalid, message);
    let record = match api::session_info(client, resolved, found.worker, cancellation).await {
        Err(Failure {
            status: Some(404), ..
        }) => return Ok(()),
        record => record.map_err(map_failure)?,
    };
    let mut parent = parent_of(&record);
    for depth in 0.. {
        match parent {
            None => {
                return Err(refused(
                    "the session is not a worker of this space's session",
                ));
            }
            Some(above) if above == found.session => break,
            Some(above) if found.bound.contains(&above) => {
                return Err(refused(
                    "a session it descends from is bound to another space",
                ));
            }
            Some(_) if depth + 1 >= MAX_WORKER_DEPTH => {
                return Err(refused(
                    "its chain of parents is too long to follow to this space's session",
                ));
            }
            Some(above) => {
                parent = match api::session_info(client, resolved, &above, cancellation).await {
                    Err(Failure {
                        status: Some(404), ..
                    }) => {
                        return Err(refused("a session it descends from no longer exists"));
                    }
                    record => parent_of(&record.map_err(map_failure)?),
                };
            }
        }
    }
    let runs = |state: WorkerState| matches!(state, WorkerState::Running | WorkerState::Waiting);
    let running: Vec<&str> = found
        .known
        .iter()
        .filter(|(_, state)| runs(*state))
        .map(|(id, _)| id.as_str())
        .collect();
    // A record's times outlive the process that wrote them, so they are taken only for a worker never observed.
    let itself = found
        .state
        .map_or_else(|| api::running_from_record(&record) == Some(true), runs);
    if !force {
        if itself {
            return Err(tomori_backend::Error::new(
                tomori_backend::ErrorKind::Busy,
                "the worker is running",
            ));
        }
        if !running.is_empty() {
            return Err(tomori_backend::Error::new(
                tomori_backend::ErrorKind::Busy,
                "one of the worker's own workers is running",
            ));
        }
    }
    // Stopped first, so that no turn goes on in a session being deleted; one that no longer exists is already gone.
    let mut stopping = vec![found.worker];
    if force {
        stopping.extend(running);
    }
    for id in stopping {
        removed(api::interrupt(client, resolved, id, cancellation).await).map_err(map_failure)?;
    }
    removed(api::delete_session(client, resolved, found.worker, cancellation).await)
        .map_err(map_failure)
}

/// Apply a finished request to the registrations.
fn complete(completion: Completion, registrations: &mut HashMap<String, Registration>) {
    match completion {
        Completion::WorkerDeleted {
            session,
            worker,
            reply,
        } => {
            if let Some(registration) = registrations.get_mut(&session) {
                let mut gone: Vec<String> = known_family(&registration.children, &worker)
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect();
                gone.push(worker);
                let mut changed = false;
                for id in gone {
                    changed |= registration.forget_deleted(&id);
                }
                if changed {
                    registration.publish_workers();
                }
            }
            let _ = reply.try_send(Ok(()));
        }
        Completion::Created {
            id,
            directory,
            modes,
            reply,
        } => {
            close_abandoned_registration(registrations, &id);
            if registrations.contains_key(&id) {
                let _ = reply.try_send(Err(tomori_backend::Error::new(
                    tomori_backend::ErrorKind::Unavailable,
                    "The server returned a session that is already attached.",
                )));
            } else {
                let events = insert_registration(registrations, id.clone(), &directory, modes);
                let _ = reply.try_send(Ok((id, events)));
            }
        }
        Completion::Attached {
            id,
            directory,
            modes,
            reply,
        } => {
            // Another attachment of the same session may have finished first.
            close_abandoned_registration(registrations, &id);
            if registrations.contains_key(&id) {
                let _ = reply.try_send(Err(tomori_backend::Error::new(
                    tomori_backend::ErrorKind::Unavailable,
                    ALREADY_ATTACHED,
                )));
            } else {
                let events = insert_registration(registrations, id, &directory, modes);
                let _ = reply.try_send(Ok(events));
            }
        }
        Completion::Listed {
            id,
            identity,
            modes,
        } => {
            if let Some(registration) = registrations.get_mut(&id)
                && Arc::ptr_eq(&registration.events.worker_epoch, &identity)
                && registration.modes.offered.is_empty()
            {
                // A switch confirmed meanwhile is newer than the listing.
                let current = registration.modes.current.take().or(modes.current);
                registration.modes = Modes {
                    offered: modes.offered,
                    current,
                };
                registration.publish_modes();
            }
        }
    }
}

const ALREADY_ATTACHED: &str =
    "That OpenCode session is already attached; join its existing space.";

/// The session `id` as `parts` describe it, given the sessions OpenCode said
/// it is running, if it did.
fn snapshot(
    parts: api::SnapshotParts,
    id: &str,
    modes: Modes,
    active: Option<&HashSet<String>>,
) -> Snapshot {
    let messages = snapshot_messages(&parts.messages);
    let busy = match active {
        Some(active) if active.contains(id) => Some(true),
        Some(_) if !parts.has_parent => Some(false),
        _ => parts.busy,
    };
    let mut workers = parts.workers();
    running_evidence(&mut workers, active);
    Snapshot {
        title: parts.title.clone(),
        directory: parts.directory.clone(),
        modes,
        busy,
        messages,
        permissions: parts.permissions(),
        questions: parts.questions(),
        workers,
        workers_incomplete: None,
    }
}

/// The session's message list, as [`api::session_messages`] answers it, as snapshot messages.
fn snapshot_messages(value: &Value) -> Vec<tomori_backend::SnapshotMessage> {
    value
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(snapshot_message)
        .collect()
}

fn snapshot_message(value: &Value) -> Option<tomori_backend::SnapshotMessage> {
    let id = value.get("id")?.as_str()?.to_owned();
    let role = match value.get("type")?.as_str()? {
        "user" => tomori_backend::Role::User,
        "assistant" => tomori_backend::Role::Assistant,
        _ => return None,
    };
    let parts = match role {
        tomori_backend::Role::User => vec![tomori_backend::Part::Text {
            text: value
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        }],
        tomori_backend::Role::Assistant => value
            .get("content")
            .and_then(Value::as_array)
            .map(|content| {
                content
                    .iter()
                    .filter_map(|part| match part.get("type")?.as_str()? {
                        "text" => Some(tomori_backend::Part::Text {
                            text: part
                                .get("text")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                        }),
                        "reasoning" => Some(tomori_backend::Part::Reasoning {
                            text: part
                                .get("text")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                        }),
                        "tool" => Some(tomori_backend::Part::Tool {
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
                    .collect::<Vec<tomori_backend::Part>>()
            })
            .unwrap_or_default(),
    };
    let completed = match role {
        tomori_backend::Role::User => true,
        tomori_backend::Role::Assistant => value
            .get("time")
            .and_then(|time| time.get("completed"))
            .is_some(),
    };
    Some(tomori_backend::SnapshotMessage {
        id,
        role,
        parts,
        completed,
    })
}

#[cfg(test)]
mod tests;
