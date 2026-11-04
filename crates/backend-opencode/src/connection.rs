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
/// Why a request fails once the backend is shutting down.
const SHUTTING_DOWN: &str = "the OpenCode backend is shutting down";
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
            Some(reason) => Ok(QueuedEvent::Ordinary(Event::Closed {
                reason: Some(reason),
            })),
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

    /// Finishes once the backend is shutting down, so that a command being
    /// performed inline does not hold up shutdown.
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
    if let Err(message) = block_on(drive(resolve, commands)) {
        eprintln!("the opencode connection ended: {message}");
    }
}

async fn drive(resolve: Resolve, commands: CommandReceiver) -> std::result::Result<(), String> {
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
        Command::Prompt { reply, .. }
        | Command::Interrupt { reply, .. }
        | Command::Decide { reply, .. }
        | Command::Answer { reply, .. }
        | Command::SetMode { reply, .. }
        | Command::Instruction { reply, .. }
        | Command::ForgetInstructions { reply, .. } => {
            let _ = reply.try_send(Err(error));
        }
        // Closing needs no connection; see `hold`.
        Command::Close { .. } | Command::Shutdown => {}
    }
}

/// Whether nobody waits for `command`'s reply any more.
fn abandoned(command: &Command) -> bool {
    match command {
        Command::Sessions { reply, .. } => reply.is_closed(),
        Command::Create { reply, .. } => reply.is_closed(),
        Command::Attach { reply, .. } => reply.is_closed(),
        Command::Snapshot { reply, .. } => reply.is_closed(),
        Command::Prompt { reply, .. }
        | Command::Interrupt { reply, .. }
        | Command::Decide { reply, .. }
        | Command::Answer { reply, .. }
        | Command::SetMode { reply, .. }
        | Command::Instruction { reply, .. }
        | Command::ForgetInstructions { reply, .. } => reply.is_closed(),
        Command::Close { .. } | Command::Shutdown => false,
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
        }
        command if sessions.held.len() < MAX_HELD => sessions.held.push_back(command),
        command => refuse(
            command,
            tomori_backend::Error::new(tomori_backend::ErrorKind::Unavailable, TOO_MANY_HELD),
        ),
    }
    true
}

/// Run `future` for at most `timeout` while no connection is open, taking
/// commands meanwhile as [`hold`] does. `None` once the command channel has
/// closed.
async fn holding<T, S: Timers>(
    future: impl Future<Output = std::result::Result<T, String>>,
    timers: &S,
    commands: &CommandReceiver,
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
                Ok(command) => if !hold(sessions, command) {
                    return None;
                },
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
    let mut unsettled = Vec::new();
    for (id, registration) in registrations.iter_mut() {
        // Their futures were dropped with the connection.
        registration.snapshot_in_flight = None;
        registration.reconcile_in_flight = None;
        // A restore being read is read again on the next connection.
        registration.restore_in_flight = false;
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

/// Open the server's global event stream, then serve it and the commands,
/// alternating reads over the stream, command channel reads and a periodic
/// reconciliation of each session's running state, until the backend closes or
/// the stream ends. Commands held while no connection was open are performed
/// first, in order, and sessions whose events were lost are read again.
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
        sequence,
    } = sessions;

    let mut groups = EventGroups::default();
    let mut timer = timers.timer();
    let mut next_reconcile = timers.now() + RECONCILE_INTERVAL;
    let mut reconcile_offset = 0usize;
    let mut snapshots: SnapshotFutures<'_> = FuturesUnordered::new();
    let mut resyncs: ResyncFutures<'_> = FuturesUnordered::new();
    let mut restores: RestoreFutures<'_> = FuturesUnordered::new();
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

    let outcome = loop {
        // Once shutting down, no further command is started, held or not.
        if commands.is_stopping() {
            break Outcome::Shutdown;
        }
        retry_worker_delivery(registrations);
        // Commands that waited for this connection are performed first, in
        // order.
        let (command, was_held) = match held.pop_front() {
            Some(command) => (command, true),
            None => (
                futures_util::select! {
                    command = Fused::new(commands.recv()) => match command {
                        Ok(Command::Shutdown) | Err(_) => break Outcome::Shutdown,
                        Ok(command) => command,
                    },
                    completed = Fused::new(async {
                        if snapshots.is_empty() {
                            std::future::pending::<Option<SnapshotResult>>().await
                        } else {
                            snapshots.next().await
                        }
                    }) => {
                        if let Some(completed) = completed {
                            finish_snapshot(registrations, completed);
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
                            finish_resync(registrations, completed, timers.now());
                        }
                        // A slot is free: start the next waiting session at once.
                        start_resyncs(registrations, &mut resyncs, sequence, client, resolved, cancellation, timers);
                        start_restores(registrations, &mut restores, client, resolved, cancellation, timers);
                        continue;
                    }
                    completed = Fused::new(async {
                        if restores.is_empty() {
                            std::future::pending::<Option<RestoreResult>>().await
                        } else {
                            restores.next().await
                        }
                    }) => {
                        if let Some(completed) = completed {
                            finish_restore(registrations, completed);
                        }
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
                            let complete = groups.push(&data);
                            for group in &complete {
                                if let Some(payload) = sse_payload(group) {
                                    let number = sequence.fetch_add(1, Ordering::AcqRel) + 1;
                                    dispatch_events_at(&payload, registrations, number);
                                }
                            }
                            start_restores(registrations, &mut restores, client, resolved, cancellation, timers);
                            if groups.oversized() {
                                // Dropping part of it could lose an event, so
                                // the stream is treated as lost, and every
                                // session is read again on the next one.
                                break Outcome::Lost(format!("sent an event larger than {MAX_EVENT_BYTES} bytes"));
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
                    // Retry a session whose read again failed.
                        start_resyncs(registrations, &mut resyncs, sequence, client, resolved, cancellation, timers);
                        retry_worker_delivery(registrations);
                        next_reconcile = timers.now() + RECONCILE_INTERVAL;
                        continue;
                    }
                },
                false,
            ),
        };
        // Commands are performed inline on this loop, one at a time, as they
        // were before reconnection (#35 will isolate them). Shutdown does not
        // wait for one: the backend's shutdown stops it, abandoning its
        // request.
        if let Some(request) = prepare_command(
            command,
            was_held,
            registrations,
            &mut snapshots,
            sequence,
            client,
            resolved,
            cancellation,
            timers,
        ) {
            // The stop is polled first, so a request is not even started once
            // shutting down.
            let stopped = commands.stopped();
            match futures_util::future::select(std::pin::pin!(stopped), request).await {
                futures_util::future::Either::Left(_) => break Outcome::Shutdown,
                futures_util::future::Either::Right((Some(completion), _)) => {
                    complete(completion, registrations);
                }
                futures_util::future::Either::Right((None, _)) => {}
            }
        }
    };
    cancellation.cancel();
    outcome
}

/// Read a session within [`SNAPSHOT_TIMEOUT`], cancelling its requests when the
/// read ends either way. `on_messages` is called just before its messages are
/// requested.
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
) -> tomori_backend::Result<Snapshot> {
    let result = futures_lite::future::race(
        async {
            snapshot(client, resolved, id, modes, local, timers, on_messages)
                .await
                .map_err(map_failure)
        },
        async {
            let timer = timers.sleep(SNAPSHOT_TIMEOUT, cancellation).await;
            Err(tomori_backend::Error::new(
                tomori_backend::ErrorKind::Unavailable,
                format!("Snapshot timed out or was cancelled: {timer:?}"),
            ))
        },
    )
    .await;
    local.cancel();
    result
}

/// Answer a snapshot a session asked for, installing it if it was read.
fn finish_snapshot(registrations: &mut HashMap<String, Registration>, completed: SnapshotResult) {
    let result = match registrations.get_mut(&completed.id) {
        Some(registration)
            if Arc::ptr_eq(&registration.events.worker_epoch, &completed.registration) =>
        {
            registration.snapshot_in_flight = None;
            let overflow = std::mem::take(&mut registration.snapshot_deletions_overflow);
            let result = completed.result.and_then(|mut snapshot| {
                if overflow {
                    return Err(tomori_backend::Error::new(
                        tomori_backend::ErrorKind::Unavailable,
                        "Too many sessions were deleted during the snapshot; please retry.",
                    ));
                }
                registration.install(&mut snapshot)?;
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
        SnapshotResult {
            id,
            registration: identity,
            reply,
            result,
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
        let Some(resync) = registration.resync.as_mut() else {
            continue;
        };
        resync.modes_changed = false;
        resync.changes.clear();
        resync.changes_overflow = false;
        resync.mark = Arc::new(AtomicU64::new(0));
        let mark = resync.mark.clone();
        let sequence = sequence.clone();
        let identity = registration.events.worker_epoch.clone();
        let current = registration.modes.clone();
        let directory = registration.directory.clone();
        resyncs.push(Box::pin(async move {
            let (result, modes) = futures_lite::future::zip(
                bounded_snapshot(
                    client,
                    resolved,
                    &id,
                    current,
                    &local,
                    cancellation,
                    timers,
                    move || mark.store(sequence.load(Ordering::Acquire), Ordering::Release),
                ),
                async {
                    // Optional, like the listing at attach: a failure keeps
                    // the modes already known.
                    let modes_cancel = local.child();
                    futures_lite::future::race(
                        async {
                            let info = api::session_info(client, resolved, &id, &modes_cancel)
                                .await
                                .ok()?;
                            Some(modes(client, resolved, &directory, &info, &modes_cancel).await)
                        },
                        async {
                            let _ = timers.sleep(SNAPSHOT_TIMEOUT, &modes_cancel).await;
                            None
                        },
                    )
                    .await
                },
            )
            .await;
            local.cancel();
            ResyncResult {
                id,
                registration: identity,
                result,
                modes,
            }
        }));
    }
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
    let result = completed.result.and_then(|mut snapshot| {
        if overflow || resync.changes_overflow {
            return Err(tomori_backend::Error::new(
                tomori_backend::ErrorKind::Unavailable,
                "too many workers changed while it was read",
            ));
        }
        registration.merge_workers(std::mem::take(&mut snapshot.workers), &resync.changes)?;
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
    /// Agents listed for a session that offered none.
    Listed {
        id: String,
        identity: Arc<AtomicU64>,
        modes: Modes,
    },
}

/// A command's request to OpenCode, performed inline by the loop, and what to
/// apply to the registrations once it has finished.
type Request<'a> = Pin<Box<dyn Future<Output = Option<Completion>> + 'a>>;

/// Prepare one command. Closing and snapshots are handled at once; for the
/// others this returns the request for the loop to perform, whose result, if
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
        Command::Shutdown => return None,
        Command::Close { id, identity } => {
            close_registration(registrations, &id, &identity);
            return None;
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
        Command::Sessions { directory, reply } => Box::pin(async move {
            let result = api::list_sessions(client, resolved, &directory, cancellation)
                .await
                .map(|sessions| session_summaries(&sessions))
                .map_err(map_failure);
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
                let session_directory = info
                    .get("location")
                    .and_then(|location| location.get("directory"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
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
        // Commands are performed in order, so an attachment that finished first is seen here, and one that follows
        // sets its own entries afterwards.
        Command::ForgetInstructions { id, reply, .. } if registrations.contains_key(&id) => {
            let _ = reply.try_send(Ok(()));
            return None;
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

/// The outcome of removing an instruction entry: OpenCode answers not found when the session, and so the entry, no
/// longer exists, which is what removing it asked for.
fn removed(result: Result<(), Failure>) -> Result<(), Failure> {
    match result {
        Err(Failure {
            status: Some(404), ..
        }) => Ok(()),
        result => result,
    }
}

/// Apply a finished request to the registrations.
fn complete(completion: Completion, registrations: &mut HashMap<String, Registration>) {
    match completion {
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

async fn snapshot<S: Timers>(
    client: &Client<'_>,
    resolved: &Resolved,
    id: &str,
    modes: Modes,
    cancellation: &Cancellation,
    timers: &S,
    on_messages: impl FnOnce(),
) -> std::result::Result<Snapshot, Failure> {
    let (parts, active) = futures_lite::future::zip(
        api::SnapshotParts::fetch(client, resolved, id, cancellation, on_messages),
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
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use tomori_backend::WorkerState;

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
        use tomori_backend::Session;
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
            matches!(block_on(session.next_event()), Err(error) if error.kind == tomori_backend::ErrorKind::Unavailable)
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
        use tomori_backend::Session;
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
        use tomori_backend::Session;
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
            matches!(block_on(session.next_event()), Err(error) if error.kind == tomori_backend::ErrorKind::Closed)
        );
    }

    #[test]
    fn closing_a_session_is_nonblocking_and_identity_scoped() {
        use tomori_backend::Session;
        let (_registration, rx) = Registration::new();
        let expected = rx.identity.clone();
        let (commands, receiver) = Commands::new();
        let session = crate::session::OpenCodeSession::new("parent".into(), rx, commands);
        for _ in 0..2 {
            assert!(block_on(session.close()).is_ok());
            match receiver.rx.try_recv().unwrap() {
                Command::Close { id, identity } => {
                    assert_eq!(id, "parent");
                    assert!(Arc::ptr_eq(&identity, &expected));
                }
                _ => panic!("expected close"),
            }
        }
        assert!(receiver.rx.try_recv().is_err());
    }

    #[test]
    fn closing_a_session_does_not_wait_for_a_full_command_queue() {
        use tomori_backend::Session;
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
            matches!(block_on(session.close()), Err(error) if error.kind == tomori_backend::ErrorKind::Unavailable)
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
                            } else if path == "/api/session/steady"
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
        let connection = std::thread::spawn(move || {
            run_thread(Box::new(move || Ok(resolved.clone())), receiver)
        });
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
        // An open session whose periodic state check will stall: only a
        // session whose initial state has been read is checked.
        let steady_events = attach("steady");
        let (steady_reply, steady_wait) = async_channel::bounded(1);
        commands
            .tx()
            .try_send(Command::Snapshot {
                id: "steady".into(),
                identity: steady_events.identity.clone(),
                reply: steady_reply,
            })
            .unwrap();
        block_on(steady_wait.recv())
            .unwrap()
            .unwrap()
            .accept()
            .unwrap();
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
            assert!(
                matches!(result, Err(error) if error.kind == tomori_backend::ErrorKind::Closed)
            );
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
        /// How many more message listings fail for each session.
        failing: HashMap<String, usize>,
        /// When each session's messages were listed.
        listed: HashMap<String, Vec<std::time::Instant>>,
        /// While set, an event stream request waits before it is answered.
        hold_events: bool,
        /// Event stream requests received.
        event_requests: usize,
        /// Sessions whose prompts wait until they are removed.
        stalled_prompts: HashSet<String>,
        /// Sessions whose records wait until they are removed, and how many
        /// such requests arrived.
        stalled_records: HashSet<String>,
        record_requests: usize,
        /// Each session's message listing, if not empty.
        messages: HashMap<String, Value>,
        /// Each session's instruction entries.
        instructions: HashMap<String, BTreeMap<String, Value>>,
        /// The instruction requests received, as method, session and key.
        instruction_requests: Vec<(String, String, String)>,
        /// The agents listed for every directory, if not none.
        agents: Option<Value>,
        /// Each reply, interruption and agent switch received, as path and body.
        actions: Vec<(String, Value)>,
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
            let expected = (!password.is_empty()).then(|| basic(password));
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
                                        expected.as_deref(),
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

        /// Send `bytes` as one chunk of every open event stream, framed or not.
        fn send_raw(&self, bytes: &[u8]) {
            for stream in self.streams.lock().unwrap().iter_mut() {
                let _ = write!(stream, "{:x}\r\n", bytes.len());
                let _ = stream.write_all(bytes);
                let _ = write!(stream, "\r\n");
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
        expected: Option<&str>,
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
            // A response without content has no framing fields.
            if status.starts_with("204") {
                let _ = write!(socket, "HTTP/1.1 {status}\r\nConnection: close\r\n\r\n");
                return;
            }
            let _ = write!(
                socket,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        };
        if authorization.as_deref() != expected {
            respond(
                &mut socket,
                "401 Unauthorized",
                r#"{"message":"unauthorized"}"#,
            );
            return;
        }
        if path == "/api/event" {
            let mut state = shared.state.lock().unwrap();
            state.event_requests += 1;
            shared.changed.notify_all();
            while state.hold_events && !state.stopping {
                state = shared
                    .changed
                    .wait_timeout(state, Duration::from_millis(50))
                    .unwrap()
                    .0;
            }
            drop(state);
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
            let agents = shared.state.lock().unwrap().agents.clone();
            (
                "200 OK",
                json!({ "data": agents.unwrap_or_else(|| json!([])) }).to_string(),
            )
        } else if path == "/api/session/active" {
            ("200 OK", r#"{"data":{}}"#.to_owned())
        } else if path.starts_with("/api/session?parentID=") {
            ("200 OK", r#"{"data":[],"cursor":{"next":null}}"#.to_owned())
        } else if path.starts_with("/api/session?directory=") {
            ("200 OK", r#"{"data":[]}"#.to_owned())
        } else if let Some(rest) = path.strip_prefix("/api/experimental/session/") {
            let mut state = shared.state.lock().unwrap();
            let (id, key) = rest
                .split_once("/instructions/entries")
                .map(|(id, key)| (id.to_owned(), key.trim_start_matches('/').to_owned()))
                .unwrap_or_default();
            state
                .instruction_requests
                .push((method.clone(), id.clone(), key.clone()));
            shared.changed.notify_all();
            if !state.sessions.contains_key(&id) {
                (
                    "404 Not Found",
                    r#"{"_tag":"SessionNotFoundError","message":"not found"}"#.to_owned(),
                )
            } else {
                let entries = state.instructions.entry(id).or_default();
                match method.as_str() {
                    "PUT" => {
                        let value: Value = serde_json::from_slice(&body).unwrap();
                        entries.insert(key, value["value"].clone());
                        ("204 No Content", String::new())
                    }
                    "DELETE" => {
                        entries.remove(&key);
                        ("204 No Content", String::new())
                    }
                    _ => (
                        "200 OK",
                        json!({ "data": entries.iter().map(|(key, value)| json!({"key": key, "value": value})).collect::<Vec<_>>() })
                            .to_string(),
                    ),
                }
            }
        } else if let Some(id) = session {
            let mut state = shared.state.lock().unwrap();
            while state.stalled_prompts.contains(&id)
                && path.ends_with("/prompt")
                && !state.stopping
            {
                state = shared
                    .changed
                    .wait_timeout(state, Duration::from_millis(50))
                    .unwrap()
                    .0;
            }
            let record = path == format!("/api/session/{id}");
            if record && state.stalled_records.contains(&id) {
                state.record_requests += 1;
                shared.changed.notify_all();
            }
            while record && state.stalled_records.contains(&id) && !state.stopping {
                state = shared
                    .changed
                    .wait_timeout(state, Duration::from_millis(50))
                    .unwrap()
                    .0;
            }
            if state.stopping {
                // Leave without answering, as a service that went away.
                return;
            }
            while state.stalled.contains(&id) && path.contains("/message?") && !state.stopping {
                state = shared
                    .changed
                    .wait_timeout(state, Duration::from_millis(50))
                    .unwrap()
                    .0;
            }
            if path.contains("/message?") {
                state
                    .listed
                    .entry(id.clone())
                    .or_default()
                    .push(std::time::Instant::now());
            }
            let fail = path.contains("/message?")
                && state
                    .failing
                    .get(&id)
                    .is_some_and(|remaining| *remaining > 0);
            if fail {
                *state.failing.get_mut(&id).unwrap() -= 1;
            }
            match state.sessions.get(&id) {
                _ if fail => (
                    "500 Internal Server Error",
                    r#"{"message":"starting"}"#.to_owned(),
                ),
                None => ("404 Not Found", r#"{"message":"not found"}"#.to_owned()),
                Some(_) if path.contains("/message?") => (
                    "200 OK",
                    json!({
                        "data": state.messages.get(&id).cloned().unwrap_or_else(|| json!([])),
                        "cursor": {"next": null},
                    })
                    .to_string(),
                ),
                Some(permissions) if path.ends_with("/permission") => {
                    ("200 OK", json!({ "data": permissions }).to_string())
                }
                Some(_) if path.ends_with("/form") => ("200 OK", r#"{"data":[]}"#.to_owned()),
                Some(_) if path.ends_with("/prompt") => ("200 OK", "{}".to_owned()),
                Some(_) if method == "POST" => {
                    let body = serde_json::from_slice(&body).unwrap_or(Value::Null);
                    state.actions.push((path.clone(), body));
                    shared.changed.notify_all();
                    ("200 OK", "{}".to_owned())
                }
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
        let resolve_path = path.clone();
        // The production resolver, reading this service file. It found the
        // service through the file, so it never falls back to the default
        // address, where a real service may listen.
        let mut resolver = crate::endpoint::Resolver::new(None);
        let (commands, receiver) = Commands::new();
        let connection = std::thread::spawn(move || {
            run_thread(
                Box::new(move || resolver.next_from(None, Some(&resolve_path))),
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

        // Four spaces are open; a fifth session has been created but its
        // space has not read it yet.
        let mut open = Vec::new();
        for _ in 0..4 {
            let (id, events) = create().unwrap();
            read(&id, &events).unwrap();
            let watched = watch(&events);
            open.push((id, events, watched));
        }
        let (unread_id, unread_events) = create().unwrap();
        let unread = watch(&unread_events);
        let ids: Vec<_> = open.iter().map(|(id, ..)| id.clone()).collect();
        assert_eq!(ids, ["s1", "s2", "s3", "s4"]);
        assert_eq!(unread_id, "s5");
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
                |event| matches!(event, Event::Reconnecting { reason } if reason == DISCONNECTED),
            );
        }
        until(
            &unread,
            "the unread session's closure",
            |event| matches!(event, Event::Closed { reason: Some(reason) } if reason.contains("before the session was read")),
        );

        // Closing a space while OpenCode is down does not wait for it.
        assert!(
            commands
                .tx()
                .try_send(Command::Close {
                    id: "s4".into(),
                    identity: open[3].1.identity.clone(),
                })
                .is_ok()
        );
        until(&open[3].2, "s4's closure while disconnected", |event| {
            matches!(event, Event::Closed { reason: None })
        });

        // While it is down, creating a session fails, and promptly.
        let started = std::time::Instant::now();
        let Err(refused) = create() else {
            panic!("a session was created while OpenCode was down");
        };
        assert_eq!(refused.kind, tomori_backend::ErrorKind::Unavailable);
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
                    assert_eq!(error.kind, tomori_backend::ErrorKind::Unavailable);
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(error) => panic!("creating after recovery failed: {error:?}"),
            }
        };
        assert_eq!(created, "s6");

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

    /// Start a connection to whichever service `address` names, with the
    /// simulated service's password.
    fn connect_to(
        address: &Arc<Mutex<std::net::SocketAddr>>,
    ) -> (Commands, std::thread::JoinHandle<()>) {
        let address = address.clone();
        let (commands, receiver) = Commands::new();
        let connection = std::thread::spawn(move || {
            run_thread(
                Box::new(move || {
                    let address = *address.lock().unwrap();
                    Ok(Resolved {
                        host: "127.0.0.1".into(),
                        port: address.port(),
                        auth: Some(("opencode".into(), "secret".into())),
                    })
                }),
                receiver,
            )
        });
        (commands, connection)
    }

    fn wait_for(shared: &FakeShared, what: &str, ready: impl Fn(&FakeState) -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut state = shared.state.lock().unwrap();
        while !ready(&state) {
            assert!(std::time::Instant::now() < deadline, "{what}");
            state = shared
                .changed
                .wait_timeout(state, Duration::from_millis(50))
                .unwrap()
                .0;
        }
    }

    fn in_thread<T: Send + 'static>(
        work: impl FnOnce() -> T + Send + 'static,
    ) -> std::sync::mpsc::Receiver<(T, std::time::Instant)> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = work();
            let _ = tx.send((result, std::time::Instant::now()));
        });
        rx
    }

    #[test]
    fn history_becomes_snapshot_messages_with_their_parts() {
        use tomori_backend::{Part, Role, SnapshotMessage};
        let messages = snapshot_messages(&json!({"data": [
            {"id": "u1", "type": "user", "text": "List the files."},
            {"id": "u2", "type": "user"},
            {"id": "a1", "type": "assistant", "time": {"completed": 2}, "content": [
                {"type": "reasoning", "text": "The user wants a listing."},
                {"type": "text", "text": "Listing them."},
                {"type": "tool", "id": "t1", "name": "bash", "state": {"status": "completed", "input": {"command": "ls"}}},
                {"type": "tool", "id": "t2", "name": "bash", "state": {"status": "error"}},
                {"type": "tool", "id": "t3", "name": "bash", "state": {"status": "running"}},
                {"type": "tool"},
                {"type": "text"},
                {"type": "reasoning"},
                {"type": "image"},
                {"type": 7},
                {}
            ]},
            {"id": "a2", "type": "assistant"},
            {"id": "s1", "type": "agent-switched"},
            {"id": "x", "type": 7},
            {"id": 7, "type": "user"},
            {"type": "user"}
        ]}));
        let tool = |id: &str, input: Value, finished: bool| Part::Tool {
            id: id.into(),
            tool: "bash".into(),
            input,
            finished,
        };
        let text = |text: &str| Part::Text { text: text.into() };
        assert_eq!(
            messages,
            [
                SnapshotMessage {
                    id: "u1".into(),
                    role: Role::User,
                    parts: vec![text("List the files.")],
                    completed: true,
                },
                SnapshotMessage {
                    id: "u2".into(),
                    role: Role::User,
                    parts: vec![text("")],
                    completed: true,
                },
                SnapshotMessage {
                    id: "a1".into(),
                    role: Role::Assistant,
                    parts: vec![
                        Part::Reasoning {
                            text: "The user wants a listing.".into()
                        },
                        text("Listing them."),
                        tool("t1", json!({"command": "ls"}), true),
                        tool("t2", Value::Null, true),
                        tool("t3", Value::Null, false),
                        Part::Tool {
                            id: String::new(),
                            tool: String::new(),
                            input: Value::Null,
                            finished: false,
                        },
                        text(""),
                        Part::Reasoning {
                            text: String::new()
                        },
                    ],
                    completed: true,
                },
                SnapshotMessage {
                    id: "a2".into(),
                    role: Role::Assistant,
                    parts: Vec::new(),
                    completed: false,
                },
            ]
        );
        assert_eq!(snapshot_messages(&json!({})), []);
    }

    /// The backend as the server uses it, reaching a service at an explicit
    /// address without a password: each session command becomes OpenCode's own
    /// request.
    #[test]
    fn the_backend_passes_each_command_to_the_service() {
        use tomori_backend::{AnswerValue, Backend, Decision, ErrorKind, Session};

        let shared = Arc::new(FakeShared::default());
        shared.update(|state| {
            state.agents = Some(json!([
                {"id": "build", "mode": "primary"},
                {"id": "plan", "mode": "primary"}
            ]));
            state.sessions.insert("existing".into(), Vec::new());
        });
        let service = FakeService::start(shared.clone(), "");
        let backend = crate::OpenCode::new(crate::Config {
            url: Some(format!("http://{}", service.address)),
        });
        assert_eq!(backend.name(), crate::NAME);
        assert_eq!(
            backend.capabilities(),
            tomori_backend::Capabilities {
                attach: true,
                interrupt: true,
                questions: true,
                workers: true,
                instructions: true,
            }
        );
        assert_eq!(block_on(backend.sessions("/tmp")).unwrap(), []);

        let session = block_on(backend.create(SessionSpec {
            directory: "/tmp".into(),
            title: "Commands".into(),
        }))
        .unwrap();
        assert_eq!(session.id(), "s1");
        block_on(session.snapshot()).unwrap();
        block_on(session.send(tomori_backend::Prompt {
            key: "k1".into(),
            text: "Hello.".into(),
        }))
        .unwrap();
        block_on(session.decide("p1", Decision::AllowOnce)).unwrap();
        block_on(session.decide("p2", Decision::Grant { option: "0".into() })).unwrap();
        block_on(session.decide("p3", Decision::Deny)).unwrap();
        block_on(session.answer("f1", vec![("name".into(), AnswerValue::Text("Ada".into()))]))
            .unwrap();
        block_on(session.interrupt()).unwrap();
        block_on(session.set_mode("plan")).unwrap();
        // OpenCode accepts any agent name, so one the session does not offer is refused here.
        let refused = block_on(session.set_mode("deploy")).unwrap_err();
        assert_eq!(refused.kind, ErrorKind::Invalid);
        assert_eq!(
            shared.state.lock().unwrap().actions,
            [
                (
                    "/api/session/s1/permission/p1/reply".to_owned(),
                    json!({"decision": "once"})
                ),
                (
                    "/api/session/s1/permission/p2/reply".to_owned(),
                    json!({"decision": "always"})
                ),
                (
                    "/api/session/s1/permission/p3/reply".to_owned(),
                    json!({"decision": "reject"})
                ),
                (
                    "/api/session/s1/form/f1/reply".to_owned(),
                    json!({"answer": {"name": "Ada"}})
                ),
                ("/api/session/s1/interrupt".to_owned(), json!({})),
                ("/api/session/s1/agent".to_owned(), json!({"agent": "plan"})),
            ]
        );

        // An existing session is attached, and instruction entries of a session no space is bound to are forgotten.
        let attached = block_on(backend.attach("existing", "/tmp")).unwrap();
        assert_eq!(attached.id(), "existing");
        let invalid =
            block_on(backend.forget_instructions("gone", vec!["Tomori".into()])).unwrap_err();
        assert_eq!(invalid.kind, ErrorKind::Invalid);
        block_on(backend.forget_instructions("gone", vec!["tomori".into()])).unwrap();
        assert_eq!(
            shared.state.lock().unwrap().instruction_requests,
            [("DELETE".to_owned(), "gone".to_owned(), "tomori".to_owned())]
        );
        // A session that is gone is no longer the service's to act on.
        let unknown = block_on(backend.attach("missing", "/tmp"))
            .map(drop)
            .unwrap_err();
        assert_eq!(unknown.kind, ErrorKind::NotFound);

        // Shutting down stops waiting for the service: later requests fail at once.
        backend.shutdown();
        let closed = block_on(backend.sessions("/tmp")).unwrap_err();
        assert_eq!(closed.kind, ErrorKind::Unavailable);
        assert_eq!(closed.message, SHUTTING_DOWN);
        drop((session, attached));
    }

    /// A backend shut down before its first use never starts its connection.
    #[test]
    fn a_backend_shut_down_before_use_never_connects() {
        use tomori_backend::{Backend, ErrorKind};

        // Nothing listens here, so connecting would be visible as a wait.
        let backend = crate::OpenCode::new(crate::Config {
            url: Some("http://127.0.0.1:1".into()),
        });
        backend.shutdown();
        let refused = |result: tomori_backend::Result<()>| {
            let error = result.unwrap_err();
            assert_eq!(error.kind, ErrorKind::Closed);
            assert_eq!(error.message, crate::SHUT_DOWN);
        };
        refused(block_on(backend.sessions("/tmp")).map(drop));
        refused(
            block_on(backend.create(SessionSpec {
                directory: "/tmp".into(),
                title: "Never".into(),
            }))
            .map(drop),
        );
        refused(block_on(backend.attach("s1", "/tmp")).map(drop));
        refused(block_on(
            backend.forget_instructions("s1", vec!["tomori".into()]),
        ));
        // Shutting down again changes nothing.
        backend.shutdown();
        refused(block_on(backend.sessions("/tmp")).map(drop));
    }

    #[test]
    fn instruction_entries_are_set_and_removed_across_a_reconnect() {
        use tomori_backend::Session;

        let shared = Arc::new(FakeShared::default());
        let mut first = FakeService::start(shared.clone(), "secret");
        let address = Arc::new(Mutex::new(first.address));
        let (commands, connection) = connect_to(&address);
        let (id, events) = block_on(commands.reply_with(|reply| Command::Create {
            spec: SessionSpec {
                directory: "/tmp".into(),
                title: "Instructions".into(),
            },
            reply,
        }))
        .unwrap();
        block_on(commands.reply_with(|reply| Command::Snapshot {
            id: id.clone(),
            identity: events.identity.clone(),
            reply,
        }))
        .and_then(SnapshotReply::accept)
        .unwrap();
        let watched = watch(&events);
        let session = crate::OpenCodeSession::new(id.clone(), events.clone(), commands.clone());
        let entries = |shared: &FakeShared| {
            shared
                .state
                .lock()
                .unwrap()
                .instructions
                .get(&id)
                .cloned()
                .unwrap_or_default()
        };

        // Each entry is put with its text as the value, and replaced or removed by key.
        block_on(session.set_instruction("tomori", Some("Reply briefly."))).unwrap();
        block_on(session.set_instruction("tomori-space", Some("Mind the tests."))).unwrap();
        block_on(session.set_instruction("tomori-space", Some("Mind the docs."))).unwrap();
        assert_eq!(
            entries(&shared),
            BTreeMap::from([
                ("tomori".to_owned(), json!("Reply briefly.")),
                ("tomori-space".to_owned(), json!("Mind the docs.")),
            ])
        );
        block_on(session.set_instruction("tomori-space", None)).unwrap();
        assert_eq!(
            entries(&shared),
            BTreeMap::from([("tomori".to_owned(), json!("Reply briefly."))])
        );
        // A key OpenCode would refuse is refused without asking it.
        let requests = shared.state.lock().unwrap().instruction_requests.len();
        let refused = block_on(session.set_instruction("Tomori/space", Some("x"))).unwrap_err();
        assert_eq!(refused.kind, tomori_backend::ErrorKind::Invalid);
        assert_eq!(
            shared.state.lock().unwrap().instruction_requests.len(),
            requests
        );

        // The service restarts. An entry set while it is down waits for the connection, and is put once it returns,
        // after which the session is read again and entries can be set as before.
        let requests = shared.state.lock().unwrap().event_requests;
        first.stop();
        until(&watched, "the disconnection notice", |event| {
            matches!(event, Event::Reconnecting { .. })
        });
        let held = {
            let session = crate::OpenCodeSession::new(id.clone(), events.clone(), commands.clone());
            in_thread(move || {
                block_on(session.set_instruction("tomori-environment", Some("The sky.")))
            })
        };
        let second = FakeService::start(shared.clone(), "secret");
        *address.lock().unwrap() = second.address;
        wait_for(
            &shared,
            "the event stream was never requested again",
            |state| state.event_requests > requests,
        );
        let (result, _) = held
            .recv_timeout(Duration::from_secs(5))
            .expect("the held entry was never put");
        assert!(result.is_ok(), "{result:?}");
        until(&watched, "the resynchronization", |event| {
            matches!(event, Event::Resynchronized { .. })
        });
        block_on(session.set_instruction("tomori", Some("Reply very briefly."))).unwrap();
        assert_eq!(
            entries(&shared),
            BTreeMap::from([
                ("tomori".to_owned(), json!("Reply very briefly.")),
                ("tomori-environment".to_owned(), json!("The sky.")),
            ])
        );

        // Setting one on a session that no longer exists says so.
        let (gone, gone_events) = block_on(commands.reply_with(|reply| Command::Create {
            spec: SessionSpec {
                directory: "/tmp".into(),
                title: "Gone".into(),
            },
            reply,
        }))
        .unwrap();
        shared.update(|state| {
            state.sessions.remove(&gone);
        });
        let gone_session = crate::OpenCodeSession::new(gone.clone(), gone_events, commands.clone());
        let missing = block_on(gone_session.set_instruction("tomori", Some("x"))).unwrap_err();
        assert_eq!(missing.kind, tomori_backend::ErrorKind::NotFound);
        block_on(gone_session.close()).unwrap();

        // A session attached when forgetting is performed keeps its entries: a space was bound to it again.
        block_on(commands.reply_with(|reply| Command::ForgetInstructions {
            id: id.clone(),
            keys: vec!["tomori".into()],
            reply,
        }))
        .unwrap();
        assert_eq!(entries(&shared).len(), 2);
        // Once it is closed, forgetting removes them, and a session that no longer exists has none.
        block_on(session.close()).unwrap();
        block_on(commands.reply_with(|reply| Command::ForgetInstructions {
            id: id.clone(),
            keys: vec![
                "tomori".into(),
                "tomori-environment".into(),
                "tomori-space".into(),
            ],
            reply,
        }))
        .unwrap();
        assert!(entries(&shared).is_empty());
        shared.update(|state| {
            state.sessions.remove(&id);
        });
        block_on(commands.reply_with(|reply| Command::ForgetInstructions {
            id: id.clone(),
            keys: vec!["tomori".into()],
            reply,
        }))
        .unwrap();
        assert!(
            shared
                .state
                .lock()
                .unwrap()
                .instruction_requests
                .iter()
                .all(|(_, _, key)| tomori_backend::valid_instruction_key(key))
        );

        drop(session);
        drop(gone_session);
        drop(commands);
        join_within(connection, Duration::from_secs(5));
        shared.update(|state| state.stopping = true);
        drop(second);
    }

    #[test]
    fn a_prompt_in_flight_and_held_commands_survive_a_reconnect() {
        let shared = Arc::new(FakeShared::default());
        let mut first = FakeService::start(shared.clone(), "secret");
        let address = Arc::new(Mutex::new(first.address));
        let (commands, connection) = connect_to(&address);
        let (id, events) = block_on(commands.reply_with(|reply| Command::Create {
            spec: SessionSpec {
                directory: "/tmp".into(),
                title: "Commands".into(),
            },
            reply,
        }))
        .unwrap();
        let identity = events.identity.clone();
        block_on(commands.reply_with(|reply| Command::Snapshot {
            id: id.clone(),
            identity: identity.clone(),
            reply,
        }))
        .and_then(SnapshotReply::accept)
        .unwrap();
        let watched = watch(&events);
        let prompt = |commands: &Commands, key: &str| {
            let commands = commands.clone();
            let identity = identity.clone();
            let key = key.to_owned();
            in_thread(move || {
                block_on(commands.reply_with(|reply| Command::Prompt {
                    id: "s1".into(),
                    identity,
                    key,
                    text: "hello".into(),
                    reply,
                }))
            })
        };

        // Only the event stream drops while OpenCode is answering a prompt:
        // the prompt it accepts is reported delivered, not failed, and the
        // loss is noticed once it has been answered.
        shared.update(|state| {
            state.stalled_prompts.insert("s1".into());
        });
        let in_flight = prompt(&commands, "in-flight");
        std::thread::sleep(Duration::from_millis(200));
        let requests = shared.state.lock().unwrap().event_requests;
        shared.update(|state| state.hold_events = true);
        first.stop();
        std::thread::sleep(Duration::from_millis(200));
        shared.update(|state| state.stalled_prompts.clear());
        let (result, _) = in_flight
            .recv_timeout(Duration::from_secs(3))
            .expect("the prompt in flight was never answered");
        assert!(result.is_ok(), "{result:?}");
        until(&watched, "the disconnection notice", |event| {
            matches!(event, Event::Reconnecting { .. })
        });

        // OpenCode returns, and its new event stream is slow to answer, so
        // commands made meanwhile wait for the connection.
        let second = FakeService::start(shared.clone(), "secret");
        *address.lock().unwrap() = second.address;
        wait_for(
            &shared,
            "the event stream was never requested again",
            |state| state.event_requests > requests,
        );
        let held_prompt = prompt(&commands, "held");
        let held_list = {
            let commands = commands.clone();
            in_thread(move || {
                block_on(commands.reply_with(|reply| Command::Sessions {
                    directory: "/tmp".into(),
                    reply,
                }))
            })
        };
        // A caller who gave up: its session must not be created.
        let (abandoned, gave_up) = async_channel::bounded(1);
        commands
            .tx()
            .try_send(Command::Create {
                spec: SessionSpec {
                    directory: "/tmp".into(),
                    title: "Nobody".into(),
                },
                reply: abandoned,
            })
            .unwrap();
        drop(gave_up);
        std::thread::sleep(Duration::from_millis(100));
        let created = shared.state.lock().unwrap().created;
        shared.update(|state| state.hold_events = false);
        let (result, _) = held_prompt
            .recv_timeout(Duration::from_secs(3))
            .expect("the held prompt was never performed");
        assert!(result.is_ok(), "{result:?}");
        let (result, _) = held_list
            .recv_timeout(Duration::from_secs(3))
            .expect("the held listing was never performed");
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(shared.state.lock().unwrap().created, created);

        drop(commands);
        join_within(connection, Duration::from_secs(5));
        drop(second);
    }

    #[test]
    fn shutdown_does_not_wait_for_a_request_in_flight() {
        let shared = Arc::new(FakeShared::default());
        let service = FakeService::start(shared.clone(), "secret");
        let address = Arc::new(Mutex::new(service.address));
        let (commands, connection) = connect_to(&address);
        let (_, events) = block_on(commands.reply_with(|reply| Command::Create {
            spec: SessionSpec {
                directory: "/tmp".into(),
                title: "Stalled".into(),
            },
            reply,
        }))
        .unwrap();
        shared.update(|state| {
            state.stalled_prompts.insert("s1".into());
        });
        let stalled = {
            let commands = commands.clone();
            let identity = events.identity.clone();
            in_thread(move || {
                block_on(commands.reply_with(|reply| Command::Prompt {
                    id: "s1".into(),
                    identity,
                    key: "k".into(),
                    text: "hello".into(),
                    reply,
                }))
            })
        };
        std::thread::sleep(Duration::from_millis(200));
        let started = std::time::Instant::now();
        commands.shutdown();
        // The request is abandoned, which is acceptable while shutting down.
        let (result, answered) = stalled
            .recv_timeout(Duration::from_secs(2))
            .expect("shutdown waited for the request in flight");
        assert!(result.is_err());
        assert!(answered - started < Duration::from_millis(500));
        drop(commands);
        join_within(connection, Duration::from_secs(1));
        shared.update(|state| state.stopping = true);
        drop(service);
    }
    #[test]
    fn a_resynchronization_neither_repeats_nor_loses_streamed_text() {
        let shared = Arc::new(FakeShared::default());
        let mut first = FakeService::start(shared.clone(), "secret");
        let address = Arc::new(Mutex::new(first.address));
        let (commands, connection) = connect_to(&address);
        let (id, events) = block_on(commands.reply_with(|reply| Command::Create {
            spec: SessionSpec {
                directory: "/tmp".into(),
                title: "Text".into(),
            },
            reply,
        }))
        .unwrap();
        block_on(commands.reply_with(|reply| Command::Snapshot {
            id: id.clone(),
            identity: events.identity.clone(),
            reply,
        }))
        .and_then(SnapshotReply::accept)
        .unwrap();
        let watched = watch(&events);
        first.stop();
        until(&watched, "the disconnection notice", |event| {
            matches!(event, Event::Reconnecting { .. })
        });
        // The restarted service holds the session's record, so the read is in
        // flight but has not yet asked for the messages. The listing it will
        // return has "m", which has ended, with its text, and "p", which is
        // still being written: OpenCode keeps no text for it until it ends, so
        // its text part is empty.
        shared.update(|state| {
            state.stalled_records.insert("s1".into());
            state.messages.insert(
                "s1".into(),
                json!([
                    {"id":"m","type":"assistant","content":[{"type":"text","text":"Hello"}],"time":{"completed":2}},
                    {"id":"p","type":"assistant","content":[{"type":"text","text":""}]},
                ]),
            );
        });
        let second = FakeService::start(shared.clone(), "secret");
        *address.lock().unwrap() = second.address;
        wait_for(&shared, "the session was never read again", |state| {
            state.record_requests > 0
        });
        let text = |message: &str, kind: &str, delta: Option<&str>| {
            let mut data = json!({"sessionID":"s1","assistantMessageID":message});
            if let Some(delta) = delta {
                data["delta"] = json!(delta);
            }
            json!({ "type": kind, "data": data })
        };
        // "m" streamed before the messages were requested: it is in the
        // listing. "n" is not.
        second.send(text("m", "session.text.started", None));
        second.send(text("m", "session.text.delta", Some("Hello")));
        second.send(text("m", "session.text.ended", None));
        second.send(text("p", "session.text.started", None));
        second.send(text("p", "session.text.delta", Some("Streamed ")));
        second.send(text("p", "session.text.delta", Some("so far")));
        second.send(text("n", "session.text.started", None));
        second.send(text("n", "session.text.delta", Some("Next")));
        std::thread::sleep(Duration::from_millis(200));
        shared.update(|state| {
            state.stalled_records.clear();
        });
        let seen = until(
            &watched,
            "the text that is not in the listing",
            |event| matches!(event, Event::TextDelta { message, .. } if message == "n"),
        );
        let ordinary: Vec<_> = seen
            .iter()
            .filter(|event| !matches!(event, Event::WorkersChanged { .. }))
            .collect();
        let Some(Event::Resynchronized { snapshot }) = ordinary.first() else {
            panic!("{seen:?}");
        };
        let listed = |id: &str| {
            snapshot
                .messages
                .iter()
                .find(|message| message.id == id)
                .unwrap_or_else(|| panic!("{id} was not listed"))
        };
        assert!(listed("m").completed);
        assert!(
            !ordinary.iter().any(|event| matches!(
                event,
                Event::MessageStarted { message, .. }
                    | Event::TextDelta { message, .. }
                    | Event::MessageCompleted { message }
                    if message == "m"
            )),
            "text the listing had was repeated: {seen:?}"
        );
        assert!(
            ordinary.iter().any(
                |event| matches!(event, Event::MessageStarted { message, .. } if message == "n")
            )
        );
        // The listing has no text for "p", so the text streamed before the
        // read is delivered rather than lost.
        let streamed: String = ordinary
            .iter()
            .filter_map(|event| match event {
                Event::TextDelta { message, text } if message == "p" => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(streamed, "Streamed so far", "{seen:?}");
        assert!(!listed("p").completed);

        // When "p" ends, OpenCode records its whole text, including what was
        // streamed while the event stream was down, and the space gets it.
        let recorded = "Before the gap. During it. Streamed so far.";
        shared.update(|state| {
            state.messages.insert(
                "s1".into(),
                json!([
                    {"id":"m","type":"assistant","content":[{"type":"text","text":"Hello"}],"time":{"completed":2}},
                    {"id":"p","type":"assistant","content":[{"type":"text","text":recorded}],"time":{"completed":3}},
                ]),
            );
        });
        second.send(text("p", "session.text.ended", None));
        let seen = until(&watched, "the restored message", |event| {
            matches!(event, Event::MessageRestored { .. })
        });
        let Some(Event::MessageRestored { message }) = seen.last() else {
            unreachable!()
        };
        assert_eq!(message.id, "p");
        assert!(message.completed);
        assert_eq!(
            message.parts,
            [tomori_backend::Part::Text {
                text: recorded.into()
            }]
        );
        // Only messages the read listed unfinished are read again.
        second.send(text("n", "session.text.ended", None));
        until(
            &watched,
            "the other message's end",
            |event| matches!(event, Event::MessageCompleted { message } if message == "n"),
        );
        let listings = shared.state.lock().unwrap().listed["s1"].len();
        assert!(
            watched.recv_timeout(Duration::from_millis(300)).is_err(),
            "a message the read did not list unfinished was restored"
        );
        assert_eq!(
            shared.state.lock().unwrap().listed["s1"].len(),
            listings,
            "the messages were read again for a message the read did not list unfinished"
        );
        drop(commands);
        join_within(connection, Duration::from_secs(5));
        drop(second);
    }

    #[test]
    fn shutdown_fails_held_commands_at_once() {
        let shared = Arc::new(FakeShared::default());
        shared.update(|state| state.hold_events = true);
        let service = FakeService::start(shared.clone(), "secret");
        let address = Arc::new(Mutex::new(service.address));
        let (commands, connection) = connect_to(&address);
        // The event stream never answers, so the attempt holds commands.
        wait_for(&shared, "the event stream was never requested", |state| {
            state.event_requests > 0
        });
        let held = {
            let commands = commands.clone();
            in_thread(move || {
                block_on(commands.reply_with(|reply| Command::Sessions {
                    directory: "/tmp".into(),
                    reply,
                }))
            })
        };
        std::thread::sleep(Duration::from_millis(100));
        let started = std::time::Instant::now();
        commands.shutdown();
        let (result, answered) = held
            .recv_timeout(Duration::from_secs(2))
            .expect("the held command was not failed");
        assert!(result.is_err());
        assert!(answered - started < Duration::from_millis(500));
        // Later requests fail without waiting.
        let later = block_on(commands.reply_with(|reply| Command::Sessions {
            directory: "/tmp".into(),
            reply,
        }))
        .unwrap_err();
        assert!(later.message.contains("shutting down"), "{later:?}");
        drop(commands);
        join_within(connection, Duration::from_secs(1));
        shared.update(|state| state.stopping = true);
        drop(service);
    }

    #[test]
    fn requests_beyond_the_held_limit_are_refused() {
        let mut sessions = Sessions::default();
        let mut waiting = Vec::new();
        for _ in 0..=MAX_HELD {
            let (reply, receiver) = async_channel::bounded(1);
            assert!(hold(
                &mut sessions,
                Command::Sessions {
                    directory: "/tmp".into(),
                    reply,
                }
            ));
            waiting.push(receiver);
        }
        assert_eq!(sessions.held.len(), MAX_HELD);
        assert!(
            waiting[..MAX_HELD]
                .iter()
                .all(|receiver| receiver.is_empty())
        );
        let refused = waiting[MAX_HELD].try_recv().unwrap().unwrap_err();
        assert_eq!(refused.message, TOO_MANY_HELD);
        assert!(!hold(&mut sessions, Command::Shutdown));
        // Whether a held request's caller has given up.
        let (reply, receiver) = async_channel::bounded::<tomori_backend::Result<()>>(1);
        let command = Command::Interrupt {
            id: "s".into(),
            identity: Arc::new(AtomicU64::new(0)),
            reply,
        };
        assert!(!abandoned(&command));
        drop(receiver);
        assert!(abandoned(&command));
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
            crate::endpoint::resolve_from(Some(&format!("http://{address}")), None, None).0;
        let connection = std::thread::spawn(move || {
            run_thread(Box::new(move || Ok(resolved.clone())), receiver)
        });
        // A command while the service is unreachable fails rather than waits.
        let refused = block_on(commands.reply_with(|reply| Command::Sessions {
            directory: "/tmp".into(),
            reply,
        }))
        .expect_err("listing succeeded against a failing service");
        assert_eq!(refused.kind, tomori_backend::ErrorKind::Unavailable);
        std::thread::sleep(Duration::from_secs(2));
        // 250 ms doubling allows four attempts in two seconds (at 0, 0.25,
        // 0.75 and 1.75 s); a tight loop would make thousands.
        let made = attempts.load(Ordering::Acquire);
        assert!((3..=6).contains(&made), "{made} attempts");
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
    fn connection_losses_before_a_session_is_read_are_bounded() {
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
            Some(Event::Reconnecting { reason }) if reason == DISCONNECTED
        ));
        assert!(matches!(
            unread.try_recv().unwrap().into_current(),
            Some(Event::Closed { reason: Some(_) })
        ));
        assert!(!registrations.contains_key("unread"));
        let busy =
            json!({"type":"session.execution.started","data":{"sessionID":"read"}}).to_string();
        // Events are held whether or not a read is in flight, and a loss
        // before the read finishes keeps them, and the failures counted: a
        // text delta is in no record until its message ends.
        dispatch_events(&busy, &mut registrations);
        let registration = registrations.get_mut("read").unwrap();
        registration.snapshot_in_flight = Some(Cancellation::new());
        registration.resync.as_mut().unwrap().failures = 2;
        dispatch_events(&busy, &mut registrations);
        assert_eq!(
            registrations["read"].resync.as_ref().unwrap().events.len(),
            2
        );
        connection_lost(&mut registrations);
        let resync = registrations["read"].resync.as_ref().unwrap();
        assert_eq!(resync.events.len(), 2);
        assert_eq!((resync.failures, resync.interruptions), (2, 1));
        assert!(matches!(
            read.try_recv().unwrap().into_current(),
            Some(Event::Reconnecting { .. })
        ));
        // A service that keeps dropping the stream before the session can be
        // read closes the space, with the reason, rather than leaving it
        // reconnecting forever.
        for _ in 1..RESYNC_INTERRUPTIONS {
            connection_lost(&mut registrations);
        }
        assert!(!registrations.contains_key("read"));
        let mut last = None;
        while let Ok(event) = read.try_recv() {
            last = event.into_current();
        }
        assert!(
            matches!(&last, Some(Event::Closed { reason: Some(reason) }) if reason.contains("kept dropping")),
            "{last:?}"
        );
    }

    #[test]
    fn lifecycle_events_during_a_read_are_newer_than_its_listing() {
        let mut registrations = HashMap::new();
        let _events = insert_registration(
            &mut registrations,
            "parent".into(),
            "/tmp",
            Modes::default(),
        );
        let registration = registrations.get_mut("parent").unwrap();
        registration.workers_ready = true;
        registration.children.insert(
            "known".into(),
            Worker {
                id: "known".into(),
                parent: Some("parent".into()),
                title: "Known".into(),
                state: WorkerState::Running,
            },
        );
        registration.resync = Some(Resync::default());
        registration.snapshot_in_flight = Some(Cancellation::new());
        // While the session is read: "known" finishes and is renamed, and
        // "late" is created and starts after the listing was taken.
        for event in [
            json!({"type":"session.execution.succeeded","data":{"sessionID":"known"}}),
            json!({"type":"session.renamed","data":{"sessionID":"known","title":"Renamed"}}),
            json!({"type":"session.created","data":{"sessionID":"late","parentID":"parent"}}),
            json!({"type":"session.execution.started","data":{"sessionID":"late"}}),
        ] {
            apply_worker_event(&event, &mut registrations);
        }
        let registration = registrations.get_mut("parent").unwrap();
        let changes = std::mem::take(&mut registration.resync.as_mut().unwrap().changes);
        // The listing predates those events.
        registration
            .merge_workers(
                vec![Worker {
                    id: "known".into(),
                    parent: Some("parent".into()),
                    title: "Known".into(),
                    state: WorkerState::Running,
                }],
                &changes,
            )
            .unwrap();
        assert_eq!(registration.children["known"].state, WorkerState::Completed);
        assert_eq!(registration.children["known"].title, "Renamed");
        assert_eq!(registration.children["late"].state, WorkerState::Running);
    }

    #[test]
    fn a_resync_without_room_for_its_held_events_closes_with_the_reason() {
        let mut registrations = HashMap::new();
        let events =
            insert_registration(&mut registrations, "full".into(), "/tmp", Modes::default());
        let registration = registrations.get_mut("full").unwrap();
        registration.workers_ready = true;
        // The space has not drained its channel; little room is left.
        for _ in 0..1000 {
            assert!(registration.events.deliver(Event::Busy));
        }
        registration.resync = Some(Resync {
            events: vec![(1, Event::Idle); 100],
            ..Resync::default()
        });
        let identity = registration.events.worker_epoch.clone();
        finish_resync(
            &mut registrations,
            ResyncResult {
                id: "full".into(),
                registration: identity,
                result: Ok(Snapshot::default()),
                modes: None,
            },
            Instant::now(),
        );
        assert!(!registrations.contains_key("full"));
        let mut last = None;
        while let Ok(event) = events.try_recv() {
            last = event.into_current();
        }
        assert!(
            matches!(&last, Some(Event::Closed { reason: Some(reason) }) if reason.contains("too many")),
            "{last:?}"
        );
    }

    #[test]
    fn failed_reads_after_reconnecting_are_retried_at_the_cadence_then_close() {
        let shared = Arc::new(FakeShared::default());
        let mut first = FakeService::start(shared.clone(), "secret");
        let address = Arc::new(Mutex::new(first.address));
        let resolve_address = address.clone();
        let (commands, receiver) = Commands::new();
        let connection = std::thread::spawn(move || {
            run_thread(
                Box::new(move || {
                    let address = *resolve_address.lock().unwrap();
                    Ok(Resolved {
                        host: "127.0.0.1".into(),
                        port: address.port(),
                        auth: Some(("opencode".into(), "secret".into())),
                    })
                }),
                receiver,
            )
        });
        let mut open = Vec::new();
        for _ in 0..2 {
            let (id, events) = block_on(commands.reply_with(|reply| Command::Create {
                spec: SessionSpec {
                    directory: "/tmp".into(),
                    title: "Retry".into(),
                },
                reply,
            }))
            .unwrap();
            let identity = events.identity.clone();
            let read_id = id.clone();
            block_on(commands.reply_with(|reply| Command::Snapshot {
                id: read_id,
                identity,
                reply,
            }))
            .and_then(SnapshotReply::accept)
            .unwrap();
            let watched = watch(&events);
            open.push((id, events, watched));
        }
        first.stop();
        for (id, _, events) in &open {
            until(events, &format!("{id}'s disconnection notice"), |event| {
                matches!(event, Event::Reconnecting { .. })
            });
        }
        // The returning service fails s1's first two reads, as one still
        // starting might, and every read of s2.
        shared.update(|state| {
            state.failing.insert("s1".into(), 2);
            state.failing.insert("s2".into(), usize::MAX);
        });
        let second = FakeService::start(shared.clone(), "secret");
        *address.lock().unwrap() = second.address;
        until(&open[0].2, "s1's resynchronization", |event| {
            matches!(event, Event::Resynchronized { .. })
        });
        until(
            &open[1].2,
            "s2's closure",
            |event| matches!(event, Event::Closed { reason: Some(reason) } if reason.contains("could not read") && reason.contains("starting")),
        );
        // Each session was read three times after reconnecting (once each
        // before the restart), a cadence apart rather than in a burst.
        let listed = shared.state.lock().unwrap().listed.clone();
        for id in ["s1", "s2"] {
            let times = &listed[id];
            assert_eq!(times.len(), 4, "{id}: {times:?}");
            for pair in times[1..].windows(2) {
                assert!(
                    pair[1] - pair[0] >= RECONCILE_INTERVAL - Duration::from_millis(50),
                    "{id} was read again too soon: {times:?}"
                );
            }
        }
        drop(commands);
        join_within(connection, Duration::from_secs(5));
        drop(second);
    }

    #[test]
    fn a_close_reason_arrives_even_when_the_channel_is_full() {
        let mut registrations = HashMap::new();
        let events =
            insert_registration(&mut registrations, "full".into(), "/tmp", Modes::default());
        let registration = registrations.get_mut("full").unwrap();
        while registration.events.room() > 0 {
            assert!(registration.events.deliver(Event::Busy));
        }
        end_registration(&mut registrations, "full", Some("the reason".into()));
        // Events already queued were lost with the channel's room, but the
        // session still learns why it ended.
        let ended = block_on(events.recv()).unwrap().into_current();
        assert!(
            matches!(&ended, Some(Event::Closed { reason: Some(reason) }) if reason == "the reason"),
            "{ended:?}"
        );
        assert!(block_on(events.recv()).is_err());
    }

    #[test]
    fn modes_seen_during_a_read_follow_its_resynchronization() {
        let mut registrations = HashMap::new();
        let events = insert_registration(&mut registrations, "s".into(), "/tmp", Modes::default());
        let registration = registrations.get_mut("s").unwrap();
        registration.workers_ready = true;
        registration.resync = Some(Resync::default());
        registration.snapshot_in_flight = Some(Cancellation::new());
        // An agent switch during the read, and agents listed for a mode
        // change, are not reported ahead of the state read again.
        dispatch_events(
            &json!({"type":"session.agent.switched","data":{"sessionID":"s","agent":"plan"}})
                .to_string(),
            &mut registrations,
        );
        let mode = |id: &str| tomori_backend::Mode {
            id: id.into(),
            name: id.into(),
            description: None,
        };
        let identity = registrations["s"].events.worker_epoch.clone();
        complete(
            Completion::Listed {
                id: "s".into(),
                identity: identity.clone(),
                modes: Modes {
                    offered: vec![mode("build")],
                    current: Some("build".into()),
                },
            },
            &mut registrations,
        );
        assert!(
            events.try_recv().is_err(),
            "modes were reported before the read"
        );
        // The read lists the restarted service's agents; the switch is newer.
        finish_resync(
            &mut registrations,
            ResyncResult {
                id: "s".into(),
                registration: identity,
                result: Ok(Snapshot::default()),
                modes: Some(Modes {
                    offered: vec![mode("build"), mode("plan")],
                    current: Some("build".into()),
                }),
            },
            Instant::now(),
        );
        let Some(Event::Resynchronized { snapshot }) = events.try_recv().unwrap().into_current()
        else {
            panic!("the resynchronization was not first");
        };
        assert_eq!(snapshot.modes.current.as_deref(), Some("plan"));
        assert_eq!(snapshot.modes.offered.len(), 2);
    }

    #[test]
    fn held_events_the_read_reflects_are_dropped_and_no_text_is_lost() {
        let mut registrations = HashMap::new();
        let events = insert_registration(&mut registrations, "s".into(), "/tmp", Modes::default());
        let registration = registrations.get_mut("s").unwrap();
        registration.workers_ready = true;
        registration.resync = Some(Resync::default());
        registration.snapshot_in_flight = Some(Cancellation::new());
        let event = |kind: &str, data: Value| {
            let mut data = data;
            data["sessionID"] = json!("s");
            json!({ "type": kind, "data": data }).to_string()
        };
        // Before the messages were requested. "done" has ended, so its text
        // is in the listing. "live" is still being written: OpenCode keeps no
        // text for it yet, so its streamed text must still be delivered. A
        // permission asked and answered is in neither list.
        let before = [
            event("session.text.started", json!({"assistantMessageID":"done"})),
            event(
                "session.text.delta",
                json!({"assistantMessageID":"done","delta":"Finished."}),
            ),
            event("session.text.ended", json!({"assistantMessageID":"done"})),
            event("session.text.started", json!({"assistantMessageID":"live"})),
            event(
                "session.text.delta",
                json!({"assistantMessageID":"live","delta":"Hello "}),
            ),
            event(
                "session.tool.called",
                json!({"assistantMessageID":"live","id":"t1","input":{}}),
            ),
            event(
                "session.tool.success",
                json!({"assistantMessageID":"live","id":"t1"}),
            ),
            event(
                "permission.asked",
                json!({"id":"gone","action":"bash","resources":[]}),
            ),
            event("permission.replied", json!({"requestID":"gone"})),
        ];
        for (number, payload) in before.iter().enumerate() {
            dispatch_events_at(payload, &mut registrations, number as u64 + 1);
        }
        let mark = before.len() as u64;
        registrations["s"]
            .resync
            .as_ref()
            .unwrap()
            .mark
            .store(mark, Ordering::Release);
        // After the request: kept, even if the listing might have them.
        let after = [
            event(
                "session.text.delta",
                json!({"assistantMessageID":"live","delta":"world"}),
            ),
            event(
                "session.text.delta",
                json!({"assistantMessageID":"live","delta":"!"}),
            ),
            event("session.text.ended", json!({"assistantMessageID":"live"})),
            event(
                "permission.asked",
                json!({"id":"new","action":"bash","resources":[]}),
            ),
        ];
        for (number, payload) in after.iter().enumerate() {
            dispatch_events_at(payload, &mut registrations, mark + number as u64 + 1);
        }
        let identity = registrations["s"].events.worker_epoch.clone();
        finish_resync(
            &mut registrations,
            ResyncResult {
                id: "s".into(),
                registration: identity,
                result: Ok(Snapshot {
                    messages: vec![
                        tomori_backend::SnapshotMessage {
                            id: "done".into(),
                            role: tomori_backend::Role::Assistant,
                            parts: vec![tomori_backend::Part::Text {
                                text: "Finished.".into(),
                            }],
                            completed: true,
                        },
                        tomori_backend::SnapshotMessage {
                            id: "live".into(),
                            role: tomori_backend::Role::Assistant,
                            parts: vec![
                                tomori_backend::Part::Text {
                                    text: String::new(),
                                },
                                tomori_backend::Part::Tool {
                                    id: "t1".into(),
                                    tool: "bash".into(),
                                    input: Value::Null,
                                    finished: true,
                                },
                            ],
                            completed: false,
                        },
                    ],
                    ..Snapshot::default()
                }),
                modes: None,
            },
            Instant::now(),
        );
        let mut delivered = Vec::new();
        while let Ok(event) = events.try_recv() {
            // Workers travel on their own lane.
            if let Some(event) = event.into_current()
                && !matches!(event, Event::WorkersChanged { .. })
            {
                delivered.push(event);
            }
        }
        assert!(
            matches!(delivered[0], Event::Resynchronized { .. }),
            "{delivered:?}"
        );
        // Consecutive deltas were held as one.
        assert_eq!(
            delivered[1..4].to_vec(),
            [
                Event::TextDelta {
                    message: "live".into(),
                    text: "Hello ".into(),
                },
                Event::TextDelta {
                    message: "live".into(),
                    text: "world!".into(),
                },
                Event::MessageCompleted {
                    message: "live".into()
                },
            ]
        );
        assert!(
            matches!(&delivered[4], Event::PermissionRequested(permission) if permission.id == "new")
        );
        assert_eq!(delivered.len(), 5, "{delivered:?}");
    }
    #[test]
    fn a_restore_lost_with_the_connection_is_read_again_and_then_held() {
        let mut registrations = HashMap::new();
        let events = insert_registration(&mut registrations, "s".into(), "/tmp", Modes::default());
        let registration = registrations.get_mut("s").unwrap();
        registration.workers_ready = true;
        registration.restoring.insert("p".into());
        // "p" ends live, so it is due to be read again.
        dispatch_events(
            &json!({"type":"session.text.ended","data":{"sessionID":"s","assistantMessageID":"p"}})
                .to_string(),
            &mut registrations,
        );
        assert_eq!(registrations["s"].restore_due, ["p"]);
        assert!(registrations["s"].restoring.is_empty());
        while events.try_recv().is_ok() {}
        // Its read starts, as `start_restores` does, and the stream is lost
        // before it returns: the read is dropped with the connection, and the
        // message is due again rather than forgotten.
        let registration = registrations.get_mut("s").unwrap();
        registration.restore_in_flight = true;
        let wanted = std::mem::take(&mut registration.restore_due);
        registration.restore_reading = wanted;
        connection_lost(&mut registrations);
        while events.try_recv().is_ok() {}
        assert!(!registrations["s"].restore_in_flight);
        assert_eq!(registrations["s"].restore_due, ["p"]);
        assert!(registrations["s"].restore_reading.is_empty());
        // Read again on the next connection before the session itself is: the
        // restore is held behind that read.
        let registration = registrations.get_mut("s").unwrap();
        registration.restore_in_flight = true;
        registration.restore_reading = std::mem::take(&mut registration.restore_due);
        let identity = registrations["s"].events.worker_epoch.clone();
        let restored = tomori_backend::SnapshotMessage {
            id: "p".into(),
            role: tomori_backend::Role::Assistant,
            parts: vec![tomori_backend::Part::Text {
                text: "All of it".into(),
            }],
            completed: false,
        };
        finish_restore(
            &mut registrations,
            RestoreResult {
                id: "s".into(),
                registration: identity.clone(),
                messages: vec![restored.clone()],
                unrecorded: vec!["q".into()],
            },
        );
        assert!(events.try_recv().is_err(), "delivered ahead of the read");
        assert!(!registrations["s"].restore_in_flight);
        assert!(matches!(
            registrations["s"].resync.as_ref().unwrap().events.last(),
            Some((_, Event::MessageRestored { message })) if *message == restored
        ));
        // A message whose record has no text yet is retried, a bounded number
        // of times.
        assert_eq!(registrations["s"].restore_retry, ["q"]);
        for _ in 1..RESYNC_ATTEMPTS {
            let registration = registrations.get_mut("s").unwrap();
            registration.restore_retry.clear();
            finish_restore(
                &mut registrations,
                RestoreResult {
                    id: "s".into(),
                    registration: identity.clone(),
                    messages: Vec::new(),
                    unrecorded: vec!["q".into()],
                },
            );
        }
        assert!(registrations["s"].restore_retry.is_empty());
        assert!(registrations["s"].restore_attempts.is_empty());
    }
    #[test]
    fn closing_during_a_read_cancels_it_and_ignores_its_result() {
        let mut registrations = HashMap::new();
        let events = insert_registration(&mut registrations, "s".into(), "/tmp", Modes::default());
        let read = Cancellation::new();
        let registration = registrations.get_mut("s").unwrap();
        registration.workers_ready = true;
        registration.resync = Some(Resync::default());
        registration.snapshot_in_flight = Some(read.clone());
        let identity = events.identity.clone();
        close_registration(&mut registrations, "s", &identity);
        assert!(read.is_cancelled());
        assert!(matches!(
            events.try_recv().unwrap().into_current(),
            Some(Event::Closed { reason: None })
        ));
        // A later registration of the same session is not touched by the
        // abandoned read's result.
        let replacement =
            insert_registration(&mut registrations, "s".into(), "/tmp", Modes::default());
        finish_resync(
            &mut registrations,
            ResyncResult {
                id: "s".into(),
                registration: identity,
                result: Ok(Snapshot::default()),
                modes: None,
            },
            Instant::now(),
        );
        assert!(replacement.try_recv().is_err());
        assert!(registrations["s"].resync.is_none());
    }

    #[test]
    fn events_are_held_through_retry_gaps_and_beyond_the_limit_close_only_that_session() {
        let mut registrations = HashMap::new();
        let resyncing = insert_registration(
            &mut registrations,
            "parent".into(),
            "/tmp",
            Modes::default(),
        );
        let other =
            insert_registration(&mut registrations, "other".into(), "/tmp", Modes::default());
        let registration = registrations.get_mut("parent").unwrap();
        registration.workers_ready = true;
        // A failed read waits to be retried: no read is in flight.
        registration.resync = Some(Resync {
            failures: 1,
            retry_at: Some(Instant::now() + RECONCILE_INTERVAL),
            ..Resync::default()
        });
        let event = |kind: &str, data: Value| {
            let mut data = data;
            data["sessionID"] = json!("parent");
            json!({ "type": kind, "data": data }).to_string()
        };
        dispatch_events(
            &event("session.text.started", json!({"assistantMessageID":"m"})),
            &mut registrations,
        );
        // A streaming reply takes one place however many deltas it has.
        for _ in 0..MAX_RESYNC_EVENTS * 2 {
            dispatch_events(
                &event(
                    "session.text.delta",
                    json!({"assistantMessageID":"m","delta":"a"}),
                ),
                &mut registrations,
            );
        }
        assert_eq!(
            registrations["parent"]
                .resync
                .as_ref()
                .unwrap()
                .events
                .len(),
            2
        );
        // The retried read finds the message still being written, so the text
        // held through the gap is delivered.
        let identity = registrations["parent"].events.worker_epoch.clone();
        let mut held = registrations["parent"]
            .resync
            .as_ref()
            .unwrap()
            .events
            .clone();
        finish_resync(
            &mut registrations,
            ResyncResult {
                id: "parent".into(),
                registration: identity.clone(),
                result: Ok(Snapshot {
                    messages: vec![tomori_backend::SnapshotMessage {
                        id: "m".into(),
                        role: tomori_backend::Role::Assistant,
                        parts: vec![tomori_backend::Part::Text {
                            text: String::new(),
                        }],
                        completed: false,
                    }],
                    ..Snapshot::default()
                }),
                modes: None,
            },
            Instant::now(),
        );
        let mut text = String::new();
        while let Ok(event) = resyncing.try_recv() {
            if let Some(Event::TextDelta { text: delta, .. }) = event.into_current() {
                text.push_str(&delta);
            }
        }
        assert_eq!(text, "a".repeat(MAX_RESYNC_EVENTS * 2));

        // Beyond the limit of distinct events, the session closes with the
        // reason rather than drop any.
        let registration = registrations.get_mut("parent").unwrap();
        held.clear();
        registration.resync = Some(Resync::default());
        for index in 0..=MAX_RESYNC_EVENTS {
            dispatch_events(
                &event(
                    "permission.asked",
                    json!({"id":format!("p{index}"),"action":"bash","resources":[]}),
                ),
                &mut registrations,
            );
        }
        assert!(!registrations.contains_key("parent"));
        let mut last = None;
        while let Ok(event) = resyncing.try_recv() {
            last = event.into_current();
        }
        assert!(
            matches!(&last, Some(Event::Closed { reason: Some(reason) }) if reason.contains("too many")),
            "{last:?}"
        );
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
            .merge_workers(
                vec![
                    worker("finished", WorkerState::Completed),
                    worker("deleted", WorkerState::Completed),
                    worker("new", WorkerState::Unknown),
                ],
                &HashMap::new(),
            )
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

    /// Read `id`'s initial state, as a space does once it is bound.
    fn read_snapshot(
        commands: &Commands,
        id: &str,
        events: &RegistrationEvents,
    ) -> tomori_backend::Result<Snapshot> {
        let id = id.to_owned();
        let identity = events.identity.clone();
        block_on(commands.reply_with(|reply| Command::Snapshot {
            id,
            identity,
            reply,
        }))
        .and_then(SnapshotReply::accept)
    }

    /// Create a session against a simulated service and open it.
    fn open_session(commands: &Commands) -> (String, RegistrationEvents) {
        let (id, events) = block_on(commands.reply_with(|reply| Command::Create {
            spec: SessionSpec {
                directory: "/tmp".into(),
                title: "Open".into(),
            },
            reply,
        }))
        .unwrap();
        read_snapshot(commands, &id, &events).unwrap();
        (id, events)
    }

    /// Events that arrive before a space's initial state has been read wait
    /// for it, and those it already shows are not delivered again: a message
    /// that ended meanwhile is in the state with its text, while one still
    /// being written has no recorded text yet, so its streamed text follows.
    #[test]
    fn text_streamed_while_the_initial_snapshot_is_read_is_not_repeated() {
        let shared = Arc::new(FakeShared::default());
        shared.update(|state| {
            state.sessions.insert("attached".into(), Vec::new());
            state.messages.insert(
                "attached".into(),
                // Newest first, as OpenCode lists them. "stepped" has
                // recorded its first text, which ended before a tool, but not
                // its second, which is still streaming.
                json!([
                    {"id":"stepped","type":"assistant","time":{"created":4},"content":[
                        {"type":"text","text":"Looking."},
                        {"type":"tool","id":"t1","name":"bash","state":{"status":"running","input":{}}},
                        {"type":"text","text":""},
                    ]},
                    {"id":"live","type":"assistant","time":{"created":3},"content":[{"type":"text","text":""}]},
                    {"id":"done","type":"assistant","time":{"created":1,"completed":2},"content":[{"type":"text","text":"Finished."}]},
                ]),
            );
        });
        let service = FakeService::start(shared.clone(), "secret");
        let (commands, connection) = connect_to(&Arc::new(Mutex::new(service.address)));
        // Another open session shows when the stream has been dispatched.
        let (other, other_events) = open_session(&commands);
        let other_events = watch(&other_events);
        let events = block_on(commands.reply_with(|reply| Command::Attach {
            id: "attached".into(),
            directory: "/tmp".into(),
            reply,
        }))
        .unwrap();
        let watched = watch(&events);
        // The read waits for the session's record, before it asks for the
        // messages, while the events arrive.
        shared.update(|state| {
            state.stalled_records.insert("attached".into());
        });
        let reading = {
            let commands = commands.clone();
            let events = events.clone();
            in_thread(move || read_snapshot(&commands, "attached", &events))
        };
        wait_for(&shared, "the read never started", |state| {
            state.record_requests == 1
        });
        let event = |kind: &str, data: Value| {
            let mut data = data;
            data["sessionID"] = json!("attached");
            json!({ "type": kind, "data": data })
        };
        service.send(event(
            "session.text.started",
            json!({"assistantMessageID":"done"}),
        ));
        service.send(event(
            "session.text.delta",
            json!({"assistantMessageID":"done","delta":"Finished."}),
        ));
        service.send(event(
            "session.text.ended",
            json!({"assistantMessageID":"done"}),
        ));
        service.send(event(
            "session.text.started",
            json!({"assistantMessageID":"live"}),
        ));
        service.send(event(
            "session.text.delta",
            json!({"assistantMessageID":"live","delta":"Still going"}),
        ));
        service.send(event(
            "session.text.started",
            json!({"assistantMessageID":"stepped"}),
        ));
        service.send(event(
            "session.text.delta",
            json!({"assistantMessageID":"stepped","delta":"Look"}),
        ));
        service.send(event(
            "session.text.delta",
            json!({"assistantMessageID":"stepped","delta":"ing."}),
        ));
        service.send(event(
            "session.text.ended",
            json!({"assistantMessageID":"stepped"}),
        ));
        service.send(event(
            "session.tool.called",
            json!({"assistantMessageID":"stepped","id":"t1","input":{}}),
        ));
        service.send(event(
            "session.text.delta",
            json!({"assistantMessageID":"stepped","delta":"Found it."}),
        ));
        service.send(json!({"type":"session.execution.started","data":{"sessionID":other}}));
        until(&other_events, "the other session's event", |event| {
            matches!(event, Event::Busy)
        });
        shared.update(|state| {
            state.stalled_records.remove("attached");
        });
        let (snapshot, _) = reading.recv_timeout(Duration::from_secs(5)).unwrap();
        let snapshot = snapshot.unwrap();
        assert_eq!(
            snapshot
                .messages
                .iter()
                .map(|message| (message.id.as_str(), message.completed))
                .collect::<Vec<_>>(),
            [("done", true), ("live", false), ("stepped", false)]
        );
        let seen = until(
            &watched,
            "the stepped message's new text",
            |event| matches!(event, Event::TextDelta { message, .. } if message == "stepped"),
        );
        // Workers travel on their own lane.
        let ordinary: Vec<_> = seen
            .into_iter()
            .filter(|event| !matches!(event, Event::WorkersChanged { .. }))
            .collect();
        assert_eq!(
            ordinary,
            [
                Event::MessageStarted {
                    message: "live".into(),
                    role: tomori_backend::Role::Assistant,
                },
                Event::TextDelta {
                    message: "live".into(),
                    text: "Still going".into(),
                },
                Event::MessageStarted {
                    message: "stepped".into(),
                    role: tomori_backend::Role::Assistant,
                },
                Event::MessageCompleted {
                    message: "stepped".into()
                },
                Event::TextDelta {
                    message: "stepped".into(),
                    text: "Found it.".into(),
                },
            ],
            "only what the snapshot does not show follows it"
        );
        drop(commands);
        join_within(connection, Duration::from_secs(5));
        shared.update(|state| state.stopping = true);
        drop(service);
    }

    /// The event stream is decoded once each event is whole, so a character
    /// split between two chunks is kept; an unfinished event that outgrows
    /// the limit is not held in memory but ends the stream, after which every
    /// session is read again.
    #[test]
    fn event_groups_are_decoded_whole_and_bounded() {
        let shared = Arc::new(FakeShared::default());
        let service = FakeService::start(shared.clone(), "secret");
        let (commands, connection) = connect_to(&Arc::new(Mutex::new(service.address)));
        let (id, events) = open_session(&commands);
        let watched = watch(&events);
        let payload = format!(
            "data: {}\n\n",
            json!({"type":"session.text.delta","data":{"sessionID":id,"assistantMessageID":"m","delta":"こんにちは"}})
        );
        let split = payload.find('ん').unwrap() + 1;
        service.send_raw(&payload.as_bytes()[..split]);
        std::thread::sleep(Duration::from_millis(50));
        service.send_raw(&payload.as_bytes()[split..]);
        let seen = until(&watched, "the split delta", |event| {
            matches!(event, Event::TextDelta { .. })
        });
        assert_eq!(
            seen.last(),
            Some(&Event::TextDelta {
                message: "m".into(),
                text: "こんにちは".into(),
            })
        );

        // An event without its end, larger than any OpenCode sends.
        service.send_raw(b"data: \"");
        let filler = vec![b'x'; 1024 * 1024];
        for _ in 0..=api::MAX_JSON_BYTES / filler.len() {
            service.send_raw(&filler);
        }
        until(&watched, "the loss of the stream", |event| {
            matches!(event, Event::Reconnecting { .. })
        });
        until(&watched, "the session read again", |event| {
            matches!(event, Event::Resynchronized { .. })
        });
        drop(commands);
        join_within(connection, Duration::from_secs(5));
        shared.update(|state| state.stopping = true);
        drop(service);
    }

    #[test]
    fn event_groups_end_at_a_blank_line_split_between_reads() {
        let mut groups = EventGroups::default();
        assert!(groups.push(b"data: 1\n").is_empty());
        // The blank line ending the group straddles two reads.
        assert_eq!(
            groups.push(b"\ndata: 2\n\ndata: 3\n\n: keep"),
            ["data: 1\n\n", "data: 2\n\n", "data: 3\n\n"]
        );
        assert!(!groups.oversized());
        assert_eq!(groups.push(b"-alive\n\n"), [": keep-alive\n\n"]);
        assert!(groups.pending.is_empty());
        assert!(groups.push(&vec![b'x'; MAX_EVENT_BYTES]).is_empty());
        assert!(!groups.oversized());
        assert!(groups.push(b"x").is_empty());
        assert!(groups.oversized());
    }

    /// A session registered and not yet read, whose snapshot is in flight.
    fn opening() -> (HashMap<String, Registration>, RegistrationEvents) {
        let mut registrations = HashMap::new();
        let events = insert_registration(&mut registrations, "s".into(), "/tmp", Modes::default());
        (registrations, events)
    }

    fn finish_opening(
        registrations: &mut HashMap<String, Registration>,
        snapshot: Snapshot,
    ) -> tomori_backend::Result<Snapshot> {
        let (reply, answer) = async_channel::bounded(1);
        let identity = registrations["s"].events.worker_epoch.clone();
        finish_snapshot(
            registrations,
            SnapshotResult {
                id: "s".into(),
                registration: identity,
                reply,
                result: Ok(snapshot),
            },
        );
        answer.try_recv().unwrap().and_then(SnapshotReply::accept)
    }

    fn state_event(kind: &str) -> String {
        json!({"type": kind, "data": {"sessionID": "s"}}).to_string()
    }

    /// More events than can be held before the initial state is read refuse
    /// it, rather than opening the space without them.
    #[test]
    fn too_many_events_before_the_initial_snapshot_refuse_it() {
        let (mut registrations, events) = opening();
        for turn in 0..=MAX_RESYNC_EVENTS {
            let kind = if turn % 2 == 0 {
                "session.execution.started"
            } else {
                "session.execution.succeeded"
            };
            dispatch_events(&state_event(kind), &mut registrations);
        }
        assert!(registrations["s"].opening.events.is_empty());
        // Nothing more is held for a snapshot that will be refused.
        dispatch_events(
            &state_event("session.execution.started"),
            &mut registrations,
        );
        assert!(registrations["s"].opening.events.is_empty());
        assert!(finish_opening(&mut registrations, Snapshot::default()).is_err());
        assert!(matches!(
            events.try_recv(),
            Err(async_channel::TryRecvError::Closed)
        ));
    }

    /// Without room for the events held for it, the initial state is refused
    /// and they stay held, so a retry delivers them after it with the latest
    /// running state and modes.
    #[test]
    fn events_held_for_the_initial_snapshot_wait_for_room() {
        let (mut registrations, events) = opening();
        dispatch_events(
            &state_event("session.execution.started"),
            &mut registrations,
        );
        dispatch_events(
            &state_event("session.execution.succeeded"),
            &mut registrations,
        );
        let switched =
            json!({"type":"session.agent.selected","data":{"sessionID":"s","agent":"plan"}})
                .to_string();
        dispatch_events(&switched, &mut registrations);
        // The mode is reported at once; the running state waits for the snapshot.
        assert!(
            matches!(events.try_recv().unwrap().into_current(), Some(Event::ModesChanged(modes)) if modes.current.as_deref() == Some("plan"))
        );
        let registration = registrations.get_mut("s").unwrap();
        while registration.events.room() > 2 {
            assert!(registration.events.deliver(Event::Idle));
        }
        let refused = finish_opening(&mut registrations, Snapshot::default()).unwrap_err();
        assert!(refused.message.contains("Too many"), "{refused:?}");
        assert_eq!(registrations["s"].opening.events.len(), 2);
        while events.try_recv().is_ok() {}
        // So is a snapshot that cannot tell which workers were deleted.
        registrations
            .get_mut("s")
            .unwrap()
            .snapshot_deletions_overflow = true;
        let refused = finish_opening(&mut registrations, Snapshot::default()).unwrap_err();
        assert!(refused.message.contains("deleted"), "{refused:?}");
        assert_eq!(registrations["s"].opening.events.len(), 2);
        let snapshot = finish_opening(
            &mut registrations,
            Snapshot {
                busy: Some(true),
                ..Snapshot::default()
            },
        )
        .unwrap();
        assert_eq!(snapshot.modes.current.as_deref(), Some("plan"));
        // The turn the snapshot shows running has ended since.
        assert!(matches!(registrations["s"].running, RunningState::Idle));
        let delivered: Vec<_> = std::iter::from_fn(|| events.try_recv().ok())
            .filter_map(QueuedEvent::into_current)
            .filter(|event| !matches!(event, Event::WorkersChanged { .. }))
            .collect();
        assert_eq!(delivered, [Event::Busy, Event::Idle]);
        // Once read, events are delivered as they arrive.
        dispatch_events(
            &state_event("session.execution.started"),
            &mut registrations,
        );
        assert!(matches!(
            events.try_recv().unwrap().into_current(),
            Some(Event::Busy)
        ));
    }

    /// A session's running state is not reconciled until its initial state
    /// has been read, which reports it, so nothing reaches its space before.
    #[test]
    fn a_session_not_yet_read_is_not_reconciled() {
        let shared = Arc::new(FakeShared::default());
        shared.update(|state| {
            state.sessions.insert("unread".into(), Vec::new());
        });
        let service = FakeService::start(shared.clone(), "secret");
        let (commands, connection) = connect_to(&Arc::new(Mutex::new(service.address)));
        let events = block_on(commands.reply_with(|reply| Command::Attach {
            id: "unread".into(),
            directory: "/tmp".into(),
            reply,
        }))
        .unwrap();
        let watched = watch(&events);
        // Longer than the reconciliation cadence.
        assert!(
            watched
                .recv_timeout(RECONCILE_INTERVAL + Duration::from_millis(500))
                .is_err(),
            "an event reached the session before its initial state"
        );
        let snapshot = read_snapshot(&commands, "unread", &events).unwrap();
        assert_eq!(snapshot.busy, Some(false));
        drop(commands);
        join_within(connection, Duration::from_secs(5));
        shared.update(|state| state.stopping = true);
        drop(service);
    }

    /// Of a message listed still being written, only text whose end was
    /// reported before its listing was requested, and which the listing
    /// records part for part, is taken as shown.
    #[test]
    fn only_text_the_listing_records_is_taken_as_shown() {
        let listed = |parts: &[&str]| Snapshot {
            messages: vec![tomori_backend::SnapshotMessage {
                id: "m".into(),
                role: tomori_backend::Role::Assistant,
                parts: parts
                    .iter()
                    .map(|text| tomori_backend::Part::Text {
                        text: (*text).to_owned(),
                    })
                    .collect(),
                completed: false,
            }],
            ..Snapshot::default()
        };
        let started = |sequence| {
            (
                sequence,
                Event::MessageStarted {
                    message: "m".into(),
                    role: tomori_backend::Role::Assistant,
                },
            )
        };
        let delta = |sequence, text: &str| {
            (
                sequence,
                Event::TextDelta {
                    message: "m".into(),
                    text: text.into(),
                },
            )
        };
        let ended = |sequence| {
            (
                sequence,
                Event::MessageCompleted {
                    message: "m".into(),
                },
            )
        };
        let shown = |snapshot: &Snapshot, held: &[(u64, Event)], mark| {
            let mut places: Vec<_> = recorded_text(snapshot, held, mark).into_iter().collect();
            places.sort();
            places
        };
        // A part that ended after the listing was requested is kept, though
        // an earlier part ends with the same text.
        let held = [delta(5, "."), ended(6)];
        assert!(shown(&listed(&["Done.", ""]), &held, 4).is_empty());
        // Ended before, it is the latest part recorded, of which its text may
        // be the end when the message started before registration.
        assert_eq!(shown(&listed(&["Done."]), &held, 6), [0]);
        // A message whose start was held had all its text held, so its
        // recorded part must be exactly that text.
        let held = [started(1), delta(2, "ing."), ended(3)];
        assert!(shown(&listed(&["Looking."]), &held, 3).is_empty());
        // Several parts are matched in order, and text still streaming is kept.
        let held = [
            started(1),
            delta(2, "A"),
            ended(3),
            delta(4, "B"),
            ended(5),
            delta(6, "C"),
        ];
        assert_eq!(shown(&listed(&["A", "B", ""]), &held, 5), [1, 3]);
        assert!(shown(&listed(&["A", "X", ""]), &held, 5).is_empty());
        // More ended parts than are recorded: the record is not yet complete.
        assert!(shown(&listed(&["B"]), &held, 5).is_empty());
    }
}
