// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Losing the event stream and connecting again: what is held, read again, resynchronized or closed, against a simulated service.

use super::*;

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
        *event == Event::Busy
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
        *event == Event::Closed { reason: None }
    });

    // While it is down, creating a session fails, and promptly.
    let started = std::time::Instant::now();
    let Err(refused) = create() else {
        unreachable!("a session was created while OpenCode was down");
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
    let requests = shared.state.lock().unwrap().event_requests;
    let second = FakeService::start(shared.clone(), "second");
    assert_ne!(second.address, first.address);
    write_service_file(&path, second.address, "second");

    // A new session can be created once OpenCode is back: once an attempt
    // has found it, rather than the old address, which would refuse it.
    wait_for(&shared, "the reconnection", |state| {
        state.event_requests > requests
    });
    let (created, _created_events) = create().unwrap();
    assert_eq!(created, "s6");

    // s1 is read again, and the permission raised during the gap reaches
    // its space.
    let seen = until(&open[0].2, "s1's resynchronization", |event| {
        kind(event) == "resynchronized"
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
        *event == Event::Busy
    });
    assert!(
        open[1].2.recv_timeout(Duration::from_millis(200)).is_err(),
        "s2 reported an event before it was read again"
    );
    shared.update(|state| {
        state.stalled.remove("s2");
    });
    let seen = until(&open[1].2, "s2's held busy event", |event| {
        *event == Event::Busy
    });
    // Workers travel on their own lane, so only ordinary events are ordered.
    let ordinary: Vec<_> = seen
        .iter()
        .filter(|event| kind(event) != "workers_changed")
        .collect();
    let [Event::Resynchronized { .. }, Event::Busy] = ordinary.as_slice() else {
        unreachable!("{seen:?}");
    };

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

/// A worker's activity is unknown once the stream is lost, and is read again from its records once the session has
/// been: a success missed in the gap leaves no tool running, and a message queued meanwhile is known. See
/// `design/events.md`.
#[test]
fn a_worker_s_activity_is_read_again_after_the_stream_is_lost() {
    let directory =
        std::env::temp_dir().join(format!("tomori-opencode-activity-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("service.json");
    let shared = Arc::new(FakeShared::default());
    shared.update(|state| {
        state.children.insert(
            "s1".into(),
            vec![fake_child("c1", "s1", 1), fake_child("c2", "s1", 2)],
        );
        state.sessions.insert("c1".into(), Vec::new());
        state.sessions.insert("c2".into(), Vec::new());
    });
    let mut first = FakeService::start(shared.clone(), "first");
    write_service_file(&path, first.address, "first");
    let resolve_path = path.clone();
    let mut resolver = crate::endpoint::Resolver::new(None);
    let (commands, receiver) = Commands::new();
    let connection = std::thread::spawn(move || {
        run_thread(
            Box::new(move || resolver.next_from(None, Some(&resolve_path))),
            receiver,
        )
    });
    let (id, events) = block_on(commands.reply_with(|reply| Command::Create {
        spec: SessionSpec {
            directory: "/tmp".into(),
            title: "Activity".into(),
        },
        reply,
    }))
    .unwrap();
    assert_eq!(id, "s1");
    let identity = events.identity.clone();
    let snapshot = block_on(commands.reply_with(|reply| Command::Snapshot {
        id: id.clone(),
        identity,
        reply,
    }))
    .and_then(SnapshotReply::accept)
    .unwrap();
    assert_eq!(snapshot.workers.len(), 2);
    let watched = watch(&events);
    let activity = |event: &Event| match event {
        Event::WorkerActivity { worker, activity } if worker == "c1" => Some(activity.clone()),
        _ => None,
    };
    first.send(json!({"type":"session.execution.started","data":{"sessionID":"c1"}}));
    first.send(json!({"type":"session.tool.input.started","data":{"sessionID":"c1","id":"t1","name":"shell"}}));
    first.send(json!({"type":"session.tool.called","data":{"sessionID":"c1","id":"t1"}}));
    first.send(json!({"type":"session.execution.started","data":{"sessionID":"c2"}}));
    until(&watched, "the worker's running tool", |event| {
        activity(event).is_some_and(|activity| activity.tool.is_some())
    });
    until(
        &watched,
        "the second worker's activity",
        |event| matches!(event, Event::WorkerActivity { worker, .. } if worker == "c2"),
    );

    // The stream is lost; meanwhile the tool and the run succeed, and a follow-up is queued, none of it seen.
    first.stop();
    until(&watched, "the disconnection notice", |event| {
        *event
            == Event::Reconnecting {
                reason: DISCONNECTED.to_owned(),
            }
    });
    shared.update(|state| {
        // Newest first, as OpenCode lists them for `order=desc`: the worker answered the message before it.
        state.messages.insert(
            "c1".into(),
            json!([
                {"type":"assistant","time":{"created":2,"completed":3},"finish":"stop"},
                {"type":"user","time":{"created":1}},
            ]),
        );
        // It now waits on a permission.
        state.sessions.insert(
            "c1".into(),
            vec![json!({"id":"p1","sessionID":"c1","action":"bash","resources":["make"]})],
        );
        state.inboxes.insert(
            "c1".into(),
            json!([{"id":"m1","type":"user","time":{"created":5}}]),
        );
        // The second's records do not answer in time.
        state.stalled.insert("c2".into());
    });
    let second = FakeService::start(shared.clone(), "second");
    write_service_file(&path, second.address, "second");
    // Each is read in turn, the second, whose records do not answer in time, first.
    let mut seen = Vec::new();
    let read = |seen: &[Event], id: &str| {
        seen.iter().rev().find_map(|event| match event {
            Event::WorkerActivity { worker, activity } if worker == id => Some(activity.clone()),
            _ => None,
        })
    };
    while read(&seen, "c1").is_none() || read(&seen, "c2").is_none() {
        seen.extend(until_within(
            &watched,
            Duration::from_secs(15),
            "the workers' activity read again",
            |event| matches!(event, Event::WorkerActivity { .. }),
        ));
    }
    let resynchronized = seen
        .iter()
        .position(|event| matches!(event, Event::Resynchronized { .. }));
    let first = seen
        .iter()
        .position(|event| matches!(event, Event::WorkerActivity { .. }));
    assert!(
        resynchronized.is_some() && resynchronized < first,
        "the session is read again first: {seen:?}"
    );
    let c1 = read(&seen, "c1").unwrap();
    assert_eq!(c1.tool, None);
    assert_eq!(c1.finish.as_deref(), Some("stop"));
    assert_eq!((c1.message, c1.unanswered_since), (Some(5), Some(5)));
    assert_eq!(c1.progressed, 3);
    assert!(c1.waiting);
    // The second starts again from when it was read.
    let c2 = read(&seen, "c2").unwrap();
    assert!(c2.progressed > 1_000_000 && c2.tool.is_none());
    shared.update(|state| {
        state.stalled.remove("c2");
    });

    drop(commands);
    join_within(connection, Duration::from_secs(5));
    shared.update(|state| state.stopping = true);
    drop(second);
    std::fs::remove_dir_all(&directory).unwrap();
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
        kind(event) == "reconnecting"
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
        kind(event) == "reconnecting"
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
        .filter(|event| kind(event) != "workers_changed")
        .collect();
    let Some(Event::Resynchronized { snapshot }) = ordinary.first() else {
        unreachable!("{seen:?}");
    };
    let listed = |id: &str| {
        snapshot
            .messages
            .iter()
            .find(|message| message.id == id)
            .expect("the message was listed")
    };
    assert!(listed("m").completed);
    assert!(
        !ordinary
            .iter()
            .any(|event| serde_json::to_value(event).unwrap()["message"] == "m"),
        "text the listing had was repeated: {seen:?}"
    );
    assert!(
        ordinary
            .iter()
            .any(|event| matches!(event, Event::MessageStarted { message, .. } if message == "n"))
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
        kind(event) == "message_restored"
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
    let later = block_on(
        commands.reply_with::<()>(|_| unreachable!("a request is not built once shutting down")),
    )
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
                    Err(error) => {
                        assert_eq!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock,
                            "accept: {error}"
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
            }
        })
    };
    let (commands, receiver) = Commands::new();
    let resolved = crate::endpoint::resolve_from(Some(&format!("http://{address}")), None, None).0;
    let connection =
        std::thread::spawn(move || run_thread(Box::new(move || Ok(resolved.clone())), receiver));
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
    let Some(Event::Closed { reason: Some(_) }) = unread.try_recv().unwrap().into_current() else {
        unreachable!("expected Some(Event::Closed {{ reason: Some(_) }})");
    };
    assert!(!registrations.contains_key("unread"));
    let busy = json!({"type":"session.execution.started","data":{"sessionID":"read"}}).to_string();
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
    let Some(Event::Reconnecting { .. }) = read.try_recv().unwrap().into_current() else {
        unreachable!("expected Some(Event::Reconnecting {{ .. }})");
    };
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
    let events = insert_registration(&mut registrations, "full".into(), "/tmp", Modes::default());
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
            rest: None,
            missing: Vec::new(),
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
            kind(event) == "reconnecting"
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
        kind(event) == "resynchronized"
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
    let events = insert_registration(&mut registrations, "full".into(), "/tmp", Modes::default());
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
            rest: None,
            missing: Vec::new(),
            modes: Some(Modes {
                offered: vec![mode("build"), mode("plan")],
                current: Some("build".into()),
            }),
        },
        Instant::now(),
    );
    let Some(Event::Resynchronized { snapshot }) = events.try_recv().unwrap().into_current() else {
        unreachable!("the resynchronization was not first");
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
            rest: None,
            missing: Vec::new(),
            modes: None,
        },
        Instant::now(),
    );
    let mut delivered = Vec::new();
    while let Ok(event) = events.try_recv() {
        // Workers travel on their own lane.
        if let Some(event) = event.into_current()
            && kind(&event) != "workers_changed"
        {
            delivered.push(event);
        }
    }
    let Event::Resynchronized { .. } = delivered[0] else {
        unreachable!("{delivered:?}");
    };
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
    let Some(Event::Closed { reason: None }) = events.try_recv().unwrap().into_current() else {
        unreachable!("expected Some(Event::Closed {{ reason: None }})");
    };
    // A later registration of the same session is not touched by the
    // abandoned read's result.
    let replacement = insert_registration(&mut registrations, "s".into(), "/tmp", Modes::default());
    finish_resync(
        &mut registrations,
        ResyncResult {
            id: "s".into(),
            registration: identity,
            result: Ok(Snapshot::default()),
            rest: None,
            missing: Vec::new(),
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
    let other = insert_registration(&mut registrations, "other".into(), "/tmp", Modes::default());
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
            rest: None,
            missing: Vec::new(),
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
