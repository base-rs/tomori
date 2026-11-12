// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Listing sessions, and registrations' event channels: how worker and state events reach a space before and after its snapshot.

use super::*;

#[test]
fn subagent_sessions_are_not_offered_for_attachment() {
    let sessions = [
        json!({"id":"root","title":"Support","location":{"directory":"/tmp"}}),
        json!({"id":"explicit","parentID":null,"title":"Explicit root"}),
        json!({"id":"child","parentID":"root","title":"Review commit"}),
        json!({"id":"archived","title":"Old","time":{"created":1,"archived":2}}),
        json!({"title":"No identifier"}),
    ];
    let summaries = session_summaries(&sessions, None);
    let ids: Vec<_> = summaries
        .iter()
        .map(|session| session.id.as_str())
        .collect();
    assert_eq!(ids, ["root", "explicit"]);
    assert_eq!(summaries[0].title, "Support");
    assert_eq!(summaries[0].directory, "/tmp");
}

/// Sessions are listed most recently active first, keeping OpenCode's order among those active at once, and a
/// busy session is marked busy rather than left out: by OpenCode's list of the sessions it runs, or by its record
/// when that list could not be read.
#[test]
fn sessions_are_listed_newest_first_with_busy_ones_marked() {
    let sessions = [
        json!({"id":"old","time":{"updated":1,"busy":5,"idle":3}}),
        json!({"id":"new","time":{"updated":30,"busy":20,"idle":25}}),
        json!({"id":"timeless"}),
        json!({"id":"middle","time":{"updated":10}}),
        json!({"id":"also-middle","time":{"updated":10}}),
    ];
    let listed = |active: Option<&std::collections::HashSet<String>>| {
        session_summaries(&sessions, active)
            .into_iter()
            .map(|session| (session.id, session.busy))
            .collect::<Vec<_>>()
    };
    let by_record = listed(None);
    assert_eq!(
        by_record,
        [
            ("new".to_owned(), false),
            ("middle".to_owned(), false),
            ("also-middle".to_owned(), false),
            ("old".to_owned(), true),
            ("timeless".to_owned(), false),
        ]
    );
    let running = std::collections::HashSet::from(["middle".to_owned()]);
    let by_process: Vec<_> = listed(Some(&running))
        .into_iter()
        .filter(|(_, busy)| *busy)
        .map(|(id, _)| id)
        .collect();
    assert_eq!(by_process, ["middle"]);
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

/// What a known worker does reaches its session as its activity, after the list that names it, never while the
/// session is opening or being read again, and is forgotten with the worker. See `design/events.md`.
#[test]
fn a_worker_s_activity_is_reported_from_the_events_it_already_reads() {
    let mut registrations = HashMap::new();
    let events = insert_registration(
        &mut registrations,
        "parent".into(),
        "/tmp",
        Modes::default(),
    );
    let activities =
        |events: &RegistrationEvents| -> Vec<(String, tomori_backend::WorkerActivity)> {
            std::iter::from_fn(|| events.try_recv().ok())
                .filter_map(QueuedEvent::into_current)
                .filter_map(|event| match event {
                    Event::WorkerActivity { worker, activity } => Some((worker, activity)),
                    _ => None,
                })
                .collect()
        };
    let send = |registrations: &mut HashMap<String, Registration>, event: Value| {
        dispatch_events(&event.to_string(), registrations);
    };
    // Before the session's workers are reported, the activity waits.
    let registration = registrations.get_mut("parent").unwrap();
    registration.children.insert(
        "child".into(),
        Worker {
            id: "child".into(),
            parent: Some("parent".into()),
            title: "Worker".into(),
            state: WorkerState::Running,
        },
    );
    registration.children.insert(
        "later".into(),
        Worker {
            id: "later".into(),
            parent: Some("parent".into()),
            title: "Worker".into(),
            state: WorkerState::Running,
        },
    );
    send(
        &mut registrations,
        json!({"type":"session.created","data":{"sessionID":"child","parentID":"parent"}}),
    );
    // A worker first seen other than by its creation, as after a restart, has its records read, once.
    for _ in 0..2 {
        send(
            &mut registrations,
            json!({"type":"session.execution.started","data":{"sessionID":"later"}}),
        );
    }
    assert_eq!(registrations["parent"].activity_due, ["later"]);
    assert!(activities(&events).is_empty());
    let registration = registrations.get_mut("parent").unwrap();
    registration.workers_ready = true;
    registration
        .events
        .snapshot_reply(Snapshot::default())
        .accept()
        .unwrap();
    // The session's own events are its own, not a worker's; the activity held while it opened follows them.
    send(
        &mut registrations,
        json!({"type":"session.execution.started","data":{"sessionID":"parent"}}),
    );
    let held = activities(&events);
    assert_eq!(held.len(), 1);
    assert_eq!((held[0].0.as_str(), &held[0].1.tool), ("child", &None));
    // While the session is read again, a worker's activity waits too, and is reported once it has been.
    let registration = registrations.get_mut("parent").unwrap();
    registration.resync = Some(Resync::default());
    send(
        &mut registrations,
        json!({"type":"session.tool.input.started","data":{"sessionID":"child","id":"t1","name":"shell"}}),
    );
    send(
        &mut registrations,
        json!({"type":"session.tool.called","data":{"sessionID":"child","id":"t1"}}),
    );
    assert!(activities(&events).is_empty());
    registrations.get_mut("parent").unwrap().resync = None;
    retry_worker_delivery(&mut registrations);
    let reported = activities(&events);
    assert_eq!(reported.len(), 1);
    assert_eq!(reported[0].0, "child");
    assert_eq!(reported[0].1.tool.as_ref().unwrap().name, "shell");
    // An event about a session it does not know, or one that says nothing of activity, changes nothing.
    send(
        &mut registrations,
        json!({"type":"session.tool.success","data":{"sessionID":"stranger","id":"t1"}}),
    );
    send(
        &mut registrations,
        json!({"type":"session.usage.updated","data":{"sessionID":"child"}}),
    );
    send(&mut registrations, json!({"type":"session.tool.success"}));
    send(
        &mut registrations,
        json!({"type":"permission.asked","data":{}}),
    );
    // A read of the workers' records that was abandoned, as when the stream was lost again, changes nothing.
    finish_activity_sync(
        &mut registrations,
        ActivityResult {
            id: "parent".into(),
            registration: events.identity.clone(),
            read: vec![("child".into(), None)],
            at: 1,
        },
    );
    assert!(activities(&events).is_empty());
    send(
        &mut registrations,
        json!({"type":"session.tool.success","data":{"sessionID":"child","id":"t1"}}),
    );
    assert_eq!(activities(&events)[0].1.tool, None);
    // Deleted, it is forgotten.
    send(
        &mut registrations,
        json!({"type":"permission.asked","data":{"sessionID":"child","id":"p1","action":"bash"}}),
    );
    registrations.get_mut("parent").unwrap().resync = Some(Resync::default());
    send(
        &mut registrations,
        json!({"type":"permission.asked","data":{"sessionID":"child","id":"p2","action":"bash"}}),
    );
    send(
        &mut registrations,
        json!({"type":"session.deleted","data":{"sessionID":"child"}}),
    );
    assert!(!registrations["parent"].activity.is_dirty());
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
    let Some(Event::WorkersUnavailable { .. }) = parent_events.try_recv().unwrap().into_current()
    else {
        unreachable!("expected Some(Event::WorkersUnavailable {{ .. }})");
    };
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
    let created =
        json!({"type": "session.created", "data": {"sessionID": "child", "parentID": "parent"}});
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
    let received: Vec<Event> = std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(QueuedEvent::into_current)
        .collect();
    let busy = received
        .iter()
        .filter(|event| **event == Event::Busy)
        .count();
    assert_eq!(busy, 1024);
    assert_eq!(received.len(), 1025);
    assert!(received.contains(&Event::WorkersChanged {
        workers: Vec::new()
    }));
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
    let Err(async_channel::TryRecvError::Closed) = receiver.try_recv() else {
        unreachable!("expected Err(async_channel::TryRecvError::Closed)");
    };
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
    let RunningState::Running = registration.running else {
        unreachable!("expected RunningState::Running");
    };
    while events.try_recv().is_ok() {}
    registration.publish_modes();
    registration.reconcile_running(RunningState::Idle);
    assert!(!registration.modes_dirty);
    let RunningState::Idle = registration.running else {
        unreachable!("expected RunningState::Idle");
    };
    assert!(
        matches!(events.try_recv().unwrap().into_current(), Some(Event::ModesChanged(modes)) if modes.current.as_deref() == Some("plan"))
    );
    let Some(Event::Idle) = events.try_recv().unwrap().into_current() else {
        unreachable!("expected Some(Event::Idle)");
    };
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
        let Err(async_channel::TryRecvError::Closed) = events.try_recv() else {
            unreachable!("expected Err(async_channel::TryRecvError::Closed)");
        };
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
    let Err(async_channel::TryRecvError::Closed) = events.try_recv() else {
        unreachable!("expected Err(async_channel::TryRecvError::Closed)");
    };
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
    let Some(Event::Busy) = events.try_recv().unwrap().into_current() else {
        unreachable!("expected Some(Event::Busy)");
    };
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
    assert!(events.try_deliver_workers(Vec::new(), None));
    events.deliver(Event::Busy);
    events.advance_workers();
    events.snapshot_reply(Snapshot::default()).accept().unwrap();
    let Ok(Event::Busy) = block_on(session.next_event()) else {
        unreachable!("expected Ok(Event::Busy)");
    };
}

#[test]
fn a_closed_session_does_not_deliver_later_events() {
    use tomori_backend::Session;
    let (events, rx) = Events::new();
    let (commands, _receiver) = Commands::new();
    let session = crate::session::OpenCodeSession::new("parent".into(), rx, commands);
    assert!(events.deliver(Event::Closed { reason: None }));
    assert!(events.deliver(Event::Busy));
    let Ok(Event::Closed { .. }) = block_on(session.next_event()) else {
        unreachable!("expected Ok(Event::Closed {{ .. }})");
    };
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
        let Command::Close { id, identity } = receiver.rx.try_recv().unwrap() else {
            unreachable!("expected close");
        };
        assert_eq!(id, "parent");
        assert!(Arc::ptr_eq(&identity, &expected));
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
    let Some(Event::Busy) = events.try_recv().unwrap().into_current() else {
        unreachable!("expected Some(Event::Busy)");
    };
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
fn a_snapshot_keeps_the_order_its_listing_gave() {
    let (mut registration, _events) = Registration::new();
    let worker = |id: &str| Worker {
        id: id.into(),
        parent: Some("parent".into()),
        title: id.into(),
        state: WorkerState::Running,
    };
    // A child created while the family was read follows those listed.
    registration.initial_creations.insert(
        "ses_0".into(),
        Worker {
            parent: Some("ses_z".into()),
            ..worker("ses_0")
        },
    );
    let mut listed = vec![worker("ses_z"), worker("ses_a")];
    registration.install_workers(&mut listed).unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|worker| worker.id.as_str())
            .collect::<Vec<_>>(),
        ["ses_z", "ses_a", "ses_0"]
    );
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
        let Some(Event::WorkersUnavailable { .. }) = events.try_recv().unwrap().into_current()
        else {
            unreachable!("{kind}");
        };
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
    let first = insert_registration(&mut registrations, "first".into(), "/tmp", Modes::default());
    let second = insert_registration(
        &mut registrations,
        "second".into(),
        "/tmp",
        Modes::default(),
    );
    close_registration(&mut registrations, "first", &first.identity);
    let Some(Event::Closed { .. }) = first.try_recv().unwrap().into_current() else {
        unreachable!("expected Some(Event::Closed {{ .. }})");
    };
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
    let Some(Event::Closed { .. }) = second.try_recv().unwrap().into_current() else {
        unreachable!("expected Some(Event::Closed {{ .. }})");
    };
}
