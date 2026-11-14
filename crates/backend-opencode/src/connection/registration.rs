// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A space's registration with the connection: its event channels, its worker lane, what it holds while it is read, and the map of registrations by session.

use super::*;

/// Bound worker identities without changing them, and bound display titles
/// with a visible truncation marker.
pub(super) const MAX_WORKER_ID_BYTES: usize = 1024;

pub(super) const MAX_WORKER_TITLE_BYTES: usize = 1024;

pub(super) fn bounded_worker_title(title: &str) -> String {
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
    pub(super) worker_epoch: Arc<AtomicU64>,
    worker_watermark: Arc<AtomicU64>,
    pending_workers: Arc<Mutex<PendingWorkers>>,
    worker_incomplete: Arc<AtomicBool>,
    pub(super) ordinary_incomplete: Arc<AtomicBool>,
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

    pub(super) fn try_recv(&self) -> std::result::Result<QueuedEvent, async_channel::TryRecvError> {
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
    pub(super) fn len(&self) -> usize {
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
    pub(super) fn close_with(&self, reason: Option<String>) {
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
    pub(super) fn room(&self) -> usize {
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

    pub(super) fn mark_ordinary_incomplete(&self) {
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
    pub(super) fn try_deliver_workers(
        &self,
        workers: Vec<Worker>,
        partial: Option<String>,
    ) -> bool {
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
    pub(super) fn advance_workers(&self) {
        self.worker_epoch.fetch_add(1, Ordering::AcqRel);
    }

    pub(super) fn snapshot_reply(&self, snapshot: Snapshot) -> SnapshotReply {
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
    pub(super) running: RunningState,
    /// Observed workers, keyed by child ID. Only a deletion event removes one.
    pub(super) children: BTreeMap<String, Worker>,
    /// When each worker was first observed, so that workers first reported
    /// together are reported in the order they were found: oldest first.
    order: HashMap<String, u64>,
    found: u64,
    /// Why the workers known may not be all the session has: while the rest
    /// of its family is discovered in the background, or after that failed.
    pub(super) workers_partial: Option<String>,
    /// The rest of the family, to be walked once a background discovery can
    /// start, and the cancellation of the one running.
    pub(super) discovery: Option<api::FamilyWalk>,
    pub(super) discovery_in_flight: Option<Cancellation>,
    /// The discovery attempts that failed since the session was last read,
    /// and when the walk may next go on after one.
    pub(super) discovery_failures: usize,
    pub(super) discovery_retry_at: Option<Instant>,
    /// How many discoveries have started, so that the result of one stopped
    /// since is told from the one running.
    pub(super) discoveries: u64,
    /// Child deletions observed before the initial snapshot was installed.
    pub(super) initial_deletions: HashSet<String>,
    /// Deletions observed while a snapshot is in flight; its listing may
    /// predate them, so they must not be resurrected when it is installed.
    pub(super) snapshot_deletions: HashSet<String>,
    pub(super) snapshot_deletions_overflow: bool,
    /// State and title events for children not yet discovered by the initial
    /// listing. Their parent is not included in execution events.
    pub(super) initial_changes: HashMap<String, (Option<WorkerState>, Option<String>)>,
    /// Creations whose parent has not yet been discovered by the snapshot.
    pub(super) initial_creations: BTreeMap<String, Worker>,
    /// Worker events are not published until a snapshot has been installed.
    pub(super) workers_ready: bool,
    /// The first offered bootstrap epoch; its journals remain until accepted.
    bootstrap_epoch: Option<u64>,
    /// A worker event could not enter the bounded event channel and needs retry.
    pub(super) workers_dirty: bool,
    /// Once the cache overflows, omissions cannot establish deletion.
    pub(super) workers_incomplete: bool,
    pub(super) snapshot_in_flight: Option<Cancellation>,
    pub(super) reconcile_in_flight: Option<Cancellation>,
    pub(super) event_revision: u64,
    /// The agents the session offers and the one its prompts go to, as last confirmed by the server.
    pub(super) modes: Modes,
    /// Latest modes must be retried if the bounded channel was full.
    pub(super) modes_dirty: bool,
    /// The session's directory, whose agents are its modes.
    pub(super) directory: String,
    /// How many more times reconciliation lists the agents while none are offered: OpenCode answers an empty listing
    /// while it loads a directory.
    pub(super) listings: u8,
    /// Set from a lost event stream until the session has been read again.
    pub(super) resync: Option<Resync>,
    /// Ordinary events held from registration until the initial snapshot is
    /// installed, which they follow, except those it already shows.
    pub(super) opening: Resync,
    /// Messages a read after reconnecting listed while they were still being
    /// written: text streamed while the stream was down is in no event, so
    /// each is read again once it ends, when OpenCode records its text.
    pub(super) restoring: HashSet<String>,
    /// Messages that have ended and wait to be read again, those being read,
    /// and whether that read is running.
    pub(super) restore_due: Vec<String>,
    pub(super) restore_reading: Vec<String>,
    pub(super) restore_in_flight: bool,
    /// Messages whose record had no text yet when read, retried at the
    /// reconciliation cadence, with the attempts made for each.
    pub(super) restore_retry: Vec<String>,
    pub(super) restore_attempts: HashMap<String, u8>,
    /// What each worker was last seen doing, reported as it changes. See `design/events.md`.
    pub(super) activity: crate::activity::Workers,
    /// The workers whose records are to be read again, their activity having become unknown when the stream was lost,
    /// and whether that read is running.
    pub(super) activity_due: Vec<String>,
    pub(super) activity_in_flight: bool,
}

impl Resync {
    /// Hold `event`, the event stream's `sequence`th. A text delta that
    /// directly follows a held delta of the same message is appended to it,
    /// so a streaming reply takes one place. Returns false when the limit is
    /// reached.
    pub(super) fn hold(&mut self, sequence: u64, event: Event) -> bool {
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
pub(super) struct Resync {
    /// Ordinary events since the event stream was lost, with their place in
    /// it, delivered after the state read again, in order, as when a space
    /// first opens, except those the read is known to reflect.
    pub(super) events: Vec<(u64, Event)>,
    /// Failed attempts on connections since the loss.
    pub(super) failures: u8,
    /// Connections lost again before the session was read.
    pub(super) interruptions: u8,
    /// How many events had been dispatched when the current read requested the
    /// session's messages: held permission and question events up to it are
    /// reflected in the read's lists.
    pub(super) mark: Arc<AtomicU64>,
    /// A failed attempt is retried no sooner than this, so that a service
    /// still starting, or about to be lost again, is not read in a burst.
    pub(super) retry_at: Option<Instant>,
    /// An agent switch was seen during the current read, so its modes are
    /// newer than the read's.
    pub(super) modes_changed: bool,
    /// Worker lifecycle changes seen during the current read, newer than its
    /// listing: the state and title each event set.
    pub(super) changes: HashMap<String, (Option<WorkerState>, Option<String>)>,
    /// More changes than can be tracked were seen; the read is retried.
    pub(super) changes_overflow: bool,
}

/// The running state of a session as the server's record reports it, derived
/// from `time.busy` after `time.idle`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RunningState {
    /// Not yet reconciled, so no change has been observed.
    Unknown,
    Running,
    Idle,
}

impl RunningState {
    pub(super) fn event(self) -> Option<Event> {
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
    pub(super) fn merge_workers(
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
    pub(super) fn insert_child(&mut self, worker: Worker) {
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
    pub(super) fn continue_discovery(&mut self, rest: Option<api::FamilyWalk>) {
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
    pub(super) fn abandon_discovery(&mut self) {
        self.discovery = None;
        self.discovery_in_flight = None;
        self.discovery_failures = 0;
        self.discovery_retry_at = None;
    }

    /// Whether a discovery is waiting to start at `now`: its walk is not
    /// running, and any retry it waits for is due.
    pub(super) fn discovery_due(&self, now: Instant) -> bool {
        self.discovery.is_some()
            && self
                .discovery_retry_at
                .is_none_or(|retry_at| retry_at <= now)
    }

    /// Start the discovery waiting to start, cancelled with `cancellation`,
    /// returning its walk and which of the registration's discoveries it is.
    pub(super) fn start_discovery(&mut self, cancellation: Cancellation) -> (api::FamilyWalk, u64) {
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
    /// `retries`, [`DISCOVERY_RETRIES`] in production, from `now`, or, once
    /// they are used up, stops, and
    /// either way the workers stay incomplete.
    pub(super) fn adopt_discovered(
        &mut self,
        mut workers: Vec<Worker>,
        failure: Option<(api::FamilyWalk, String)>,
        now: Instant,
        retries: &[Duration],
    ) {
        self.discovery_in_flight = None;
        self.discovery_retry_at = None;
        self.workers_partial = failure.map(|(walk, reason)| {
            match retries.get(self.discovery_failures) {
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
    pub(super) fn known_workers(&self) -> Vec<String> {
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
    pub(super) fn remove_missing(
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

    pub(super) fn publish_workers(&mut self) {
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

    pub(super) fn mark_workers_incomplete(&mut self) {
        if !self.workers_incomplete {
            self.workers_incomplete = true;
            self.events.worker_incomplete.store(true, Ordering::Release);
            self.publish_workers();
        }
    }

    /// Stop observing the worker `id`, which OpenCode deleted, remembering the
    /// deletion against any listing still in flight, returning whether it was
    /// known.
    pub(super) fn forget_deleted(&mut self, id: &str) -> bool {
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
    pub(super) fn remove_worker(&mut self, id: &str) -> bool {
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
    pub(super) fn deliver_content(&mut self, event: Event) {
        if let Event::MessageCompleted { message } = &event
            && self.restoring.remove(message)
        {
            self.restore_due.push(message.clone());
        }
        self.events.deliver(event);
    }

    pub(super) fn publish_modes(&mut self) {
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
    pub(super) fn publish_activity(&mut self) {
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

    pub(super) fn deliver_state(&self, event: Event) -> bool {
        let delivered = self.events.try_deliver_state(event);
        if !delivered && !self.events.consumer_started.load(Ordering::Acquire) {
            // A retry after snapshot acceptance is too late to make the
            // initial busy/mode state safe. Refuse the bootstrap instead.
            self.events.mark_ordinary_incomplete();
        }
        delivered
    }

    pub(super) fn reconcile_running(&mut self, running: RunningState) {
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
    pub(super) fn discovering_workers(&self) -> bool {
        self.discovery.is_some()
            || self.discovery_in_flight.is_some()
            || !self
                .bootstrap_epoch
                .is_some_and(|epoch| self.events.worker_watermark.load(Ordering::Acquire) >= epoch)
    }

    pub(super) fn clear_bootstrap_journals(&mut self) {
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
    pub(super) fn install_workers(
        &mut self,
        workers: &mut Vec<Worker>,
    ) -> tomori_backend::Result<()> {
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
    pub(super) fn follow(&mut self, snapshot: &Snapshot) {
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
    pub(super) fn install(&mut self, snapshot: &mut Snapshot) -> tomori_backend::Result<()> {
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

/// Whether the conversation `snapshot` lists already shows `event`. OpenCode
/// keeps a message's text in its records only once the message has ended, so
/// a listed message still being written has none of the text streamed so far:
/// its text is reflected only once the message is listed as completed. A
/// message start, a tool call, a completion and a tool's end are reflected
/// when the listing shows them.
pub(super) fn reflected(snapshot: &Snapshot, event: &Event) -> bool {
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
pub(super) fn recorded_text(
    snapshot: &Snapshot,
    held: &[(u64, Event)],
    mark: u64,
) -> HashSet<usize> {
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
pub(super) fn requests(event: &Event) -> bool {
    matches!(
        event,
        Event::PermissionRequested(_)
            | Event::PermissionResolved { .. }
            | Event::QuestionAsked(_)
            | Event::QuestionResolved { .. }
    )
}

/// Register `id` in the map with its modes, returning its receiver.
pub(super) fn insert_registration(
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

pub(super) fn close_registration(
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
pub(super) fn end_registration(
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
pub(super) fn close_abandoned_registration(
    registrations: &mut HashMap<String, Registration>,
    id: &str,
) {
    let identity = registrations.get(id).and_then(|registration| {
        (registration.events.tx.receiver_count() == 0)
            .then(|| registration.events.worker_epoch.clone())
    });
    if let Some(identity) = identity {
        close_registration(registrations, id, &identity);
    }
}

pub(super) fn owns_registration(
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
