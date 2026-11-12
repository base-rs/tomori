// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Performing commands as requests to OpenCode, each beside the others, and completing them, including deleting a worker checked against the agent's records.

use super::*;

pub(super) fn map_failure(failure: Failure) -> tomori_backend::Error {
    let kind = match failure.status {
        Some(404) => tomori_backend::ErrorKind::NotFound,
        Some(status) if (400..500).contains(&status) => tomori_backend::ErrorKind::Invalid,
        _ => tomori_backend::ErrorKind::Unavailable,
    };
    tomori_backend::Error::new(kind, failure.message)
}

/// The sessions a participant can attach to, most recently active first. The
/// listing asks for top-level, unarchived sessions; a child session (a
/// subagent's worker) or an archived one that a server returns anyway is left
/// out. A session is busy when `active`, OpenCode's list of the sessions its
/// process runs, has it, or, when that list could not be read, when its record
/// says so, as for a space's own session (see `design/agent-backends.md`).
pub(super) fn session_summaries(
    sessions: &[Value],
    active: Option<&HashSet<String>>,
) -> Vec<SessionSummary> {
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
pub(super) enum Completion {
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
pub(super) type RequestFutures<'a> =
    FuturesUnordered<Pin<Box<dyn Future<Output = (Option<String>, Option<Completion>)> + 'a>>>;

/// Prepare one command that waited to be started. A snapshot is handled at
/// once; for the others this returns the request for the loop to perform, whose result, if
/// it needs the registrations, the loop applies with [`complete`]. A command
/// that waited for this connection is dropped if its caller has given up.
pub(super) fn prepare_command<'a, S: Timers>(
    command: Command,
    held: bool,
    registrations: &mut HashMap<String, Registration>,
    snapshots: &mut SnapshotFutures<'a>,
    sequence: &Arc<AtomicU64>,
    service: Service<'a, '_, S>,
) -> Option<Request<'a>> {
    let Service {
        client,
        resolved,
        cancellation,
        timers,
    } = service;
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
                service,
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
pub(super) const MAX_WORKER_DEPTH: usize = 32;

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
pub(super) fn complete(completion: Completion, registrations: &mut HashMap<String, Registration>) {
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

pub(super) const ALREADY_ATTACHED: &str =
    "That OpenCode session is already attached; join its existing space.";
