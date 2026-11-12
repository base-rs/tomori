// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A session's initial snapshot: events held while it is read, and how the event stream is decoded.

use super::*;

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
        *event == Event::Busy
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
        .filter(|event| kind(event) != "workers_changed")
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
        kind(event) == "text_delta"
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
        kind(event) == "reconnecting"
    });
    until(&watched, "the session read again", |event| {
        kind(event) == "resynchronized"
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
            rest: None,
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
    let Err(async_channel::TryRecvError::Closed) = events.try_recv() else {
        unreachable!("expected Err(async_channel::TryRecvError::Closed)");
    };
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
    let switched = json!({"type":"session.agent.selected","data":{"sessionID":"s","agent":"plan"}})
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
    let RunningState::Idle = registrations["s"].running else {
        unreachable!("expected RunningState::Idle");
    };
    let delivered: Vec<_> = std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(QueuedEvent::into_current)
        .filter(|event| kind(event) != "workers_changed")
        .collect();
    assert_eq!(delivered, [Event::Busy, Event::Idle]);
    // Once read, events are delivered as they arrive.
    dispatch_events(
        &state_event("session.execution.started"),
        &mut registrations,
    );
    let Some(Event::Busy) = events.try_recv().unwrap().into_current() else {
        unreachable!("expected Some(Event::Busy)");
    };
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
