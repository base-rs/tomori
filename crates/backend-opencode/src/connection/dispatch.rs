// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The event stream: decoding its events whole and bounded, and dispatching each to the registrations it concerns.

use super::*;

/// The largest event the stream may send, as for any response.
pub(super) const MAX_EVENT_BYTES: usize = api::MAX_JSON_BYTES;

/// Splits the event stream's bytes into its event groups, each ending with a
/// blank line. A group is decoded once it is whole, so a character split
/// between two reads is kept, and at most [`MAX_EVENT_BYTES`] of an
/// unfinished one are held.
#[derive(Default)]
pub(super) struct EventGroups {
    pub(super) pending: Vec<u8>,
    /// How much of `pending` has been searched for a group's end.
    searched: usize,
}

impl EventGroups {
    /// Add `data`, returning the groups it completes.
    pub(super) fn push(&mut self, data: &[u8]) -> Vec<String> {
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
    pub(super) fn oversized(&self) -> bool {
        self.pending.len() > MAX_EVENT_BYTES
    }
}

/// The JSON payload of one SSE group, `None` for keep-alive and comment groups.
pub(super) fn sse_payload(group: &str) -> Option<String> {
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

/// Feed one complete SSE event to its session and to any parent that owns its
/// worker. Child events are authoritative; no paginated scan infers removal.
#[cfg(test)]
pub(super) fn dispatch_events(event: &str, registrations: &mut HashMap<String, Registration>) {
    dispatch_events_at(event, registrations, 0);
}

/// [`dispatch_events`] for the event stream's `sequence`th event, which a
/// session being read again holds with that number.
pub(super) fn dispatch_events_at(
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
pub(super) fn apply_worker_event(value: &Value, registrations: &mut HashMap<String, Registration>) {
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
    // A worker cancelled deliberately stopped, rather than failed.
    let kind = crate::wire::as_cancelled(kind, data);
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

pub(super) fn retry_worker_delivery(registrations: &mut HashMap<String, Registration>) {
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
