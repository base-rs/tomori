// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Reading sessions beside the event stream: initial snapshots, reads again after a loss, background discovery of workers, restores of messages still being written, and workers' activity.

use super::*;

pub(super) struct SnapshotResult {
    pub(super) id: String,
    pub(super) registration: Arc<AtomicU64>,
    pub(super) reply: async_channel::Sender<tomori_backend::Result<SnapshotReply>>,
    pub(super) result: tomori_backend::Result<Snapshot>,
    /// The rest of the session's family, when the read ran out of time for it.
    pub(super) rest: Option<api::FamilyWalk>,
}

/// What a background discovery of a session's workers found.
pub(super) struct DiscoveryResult {
    pub(super) id: String,
    pub(super) registration: Arc<AtomicU64>,
    /// Which of the registration's discoveries this was.
    pub(super) number: u64,
    /// The descendants found, and whether the rest of the family was walked
    /// whole, or why not, with the walk, to go on with after a failure.
    pub(super) found: Vec<Value>,
    pub(super) result: std::result::Result<(), Failure>,
    pub(super) walk: api::FamilyWalk,
    /// The sessions OpenCode is running, if it said in time.
    pub(super) active: Option<HashSet<String>>,
}

/// Messages read again once they ended, as OpenCode recorded them.
pub(super) struct RestoreResult {
    pub(super) id: String,
    pub(super) registration: Arc<AtomicU64>,
    /// The wanted messages whose text the record has.
    pub(super) messages: Vec<tomori_backend::SnapshotMessage>,
    /// The wanted messages it has no text for yet, or all of them if the read
    /// failed.
    pub(super) unrecorded: Vec<String>,
}

pub(super) struct ResyncResult {
    pub(super) id: String,
    pub(super) registration: Arc<AtomicU64>,
    pub(super) result: tomori_backend::Result<Snapshot>,
    /// The rest of the session's family, when the read ran out of time for it.
    pub(super) rest: Option<api::FamilyWalk>,
    /// Known workers the read's listing omitted that OpenCode answered do not
    /// exist.
    pub(super) missing: Vec<String>,
    /// The agents offered and the session's current one, if they were listed.
    pub(super) modes: Option<Modes>,
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
pub(super) fn finish_snapshot(
    registrations: &mut HashMap<String, Registration>,
    completed: SnapshotResult,
) {
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
pub(super) fn schedule_snapshot<'a, S: Timers>(
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
pub(super) fn start_resyncs<'a, S: Timers>(
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
pub(super) async fn bounded_modes<S: Timers>(
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
pub(super) fn finish_resync(
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
pub(super) fn due_discoveries(
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
pub(super) fn start_discoveries<'a, S: Timers>(
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
pub(super) fn finish_discovery(
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
pub(super) fn start_restores<'a, S: Timers>(
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
pub(super) struct ActivityResult {
    pub(super) id: String,
    pub(super) registration: Arc<AtomicU64>,
    pub(super) read: Vec<(String, Option<WorkerRecords>)>,
    pub(super) at: u64,
}

/// A worker's latest messages, oldest first, its inbox and its pending permissions and forms.
type WorkerRecords = (Vec<Value>, Vec<Value>, Vec<Value>);

/// Read again, once the session itself has been, the records of the workers whose activity became unknown when the
/// stream was lost, at most [`MAX_RESYNCS`] sessions at once, each worker in turn, each read bounded like a snapshot.
/// See `design/events.md`.
pub(super) fn start_activity_syncs<'a, S: Timers>(
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
pub(super) fn finish_activity_sync(
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
pub(super) fn finish_restore(
    registrations: &mut HashMap<String, Registration>,
    completed: RestoreResult,
) {
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

/// The agents offered in `directory` and the one the session described by `info` uses. A listing that fails leaves
/// the session without offered modes rather than failing it; reconciliation lists them again.
pub(super) async fn modes(
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

/// The session `id` as `parts` describe it, given the sessions OpenCode said
/// it is running, if it did.
pub(super) fn snapshot(
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
pub(super) fn snapshot_messages(value: &Value) -> Vec<tomori_backend::SnapshotMessage> {
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
