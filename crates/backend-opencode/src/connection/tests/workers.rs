// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Workers: lists, their pages and limits, and finding a family in the background, against registrations and a simulated service.

use super::*;

#[test]
fn fork_events_do_not_fabricate_child_workers() {
    let mut registrations = registrations();
    let forked = json!({"type":"session.forked","data":{"sessionID":"fork","parentID":"parent"}});
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
    let Some(Event::WorkersUnavailable { .. }) = events.try_recv().unwrap().into_current() else {
        unreachable!("expected Some(Event::WorkersUnavailable {{ .. }})");
    };
    let deleted = json!({"type":"session.deleted","data":{"sessionID":"worker-1"}});
    apply_worker_event(&deleted, &mut registrations);
    let Some(Event::WorkersUnavailable { .. }) = events.try_recv().unwrap().into_current() else {
        unreachable!("expected Some(Event::WorkersUnavailable {{ .. }})");
    };
}

#[test]
fn child_list_rejects_failed_and_malformed_responses() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    // The client starts once the server has looked for its request and
    // found none, so that the server is seen waiting however loaded the
    // machine is.
    let (looked, looking) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        for attempt in 0..5 {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let (mut socket, _) = loop {
                match listener.accept() {
                    Ok(accepted) => break accepted,
                    Err(error) => {
                        assert_eq!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock,
                            "accept: {error}"
                        );
                        assert!(std::time::Instant::now() < deadline, "no refresh request");
                        let _ = looked.send(());
                        std::thread::sleep(Duration::from_millis(5));
                    }
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
    looking.recv().unwrap();
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
        let (_driver_result, ()) = futures_lite::future::zip(driver.run(&cancellation), async {
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
    // The client starts once the server has looked for its request and
    // found none, so that the server is seen waiting however loaded the
    // machine is.
    let (looked, looking) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        for attempt in 0..2 {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let (mut socket, _) = loop {
                match listener.accept() {
                    Ok(accepted) => break accepted,
                    Err(error) => {
                        assert_eq!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock,
                            "accept: {error}"
                        );
                        assert!(std::time::Instant::now() < deadline, "no page request");
                        let _ = looked.send(());
                        std::thread::sleep(Duration::from_millis(5));
                    }
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
    looking.recv().unwrap();
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
        let (_driver_result, ()) = futures_lite::future::zip(driver.run(&cancellation), async {
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

/// Give the simulated session `root` a chain of workers, each the only
/// child of the one before, created in that order.
fn chain(state: &mut FakeState, root: &str, ids: &[&str]) {
    let mut parent = root.to_owned();
    for (place, id) in ids.iter().enumerate() {
        state.children.insert(
            parent.clone(),
            vec![fake_child(id, &parent, place as u64 + 1)],
        );
        parent = (*id).to_owned();
    }
}

/// A family too deep to walk within the snapshot's bound, a chain of 64
/// workers whose children each take 80 milliseconds to list, opens in time
/// with the workers found so far, marked incomplete. The rest is found in
/// the background, and every worker is then reported, oldest first, as a
/// complete list.
#[test]
fn a_deep_worker_family_opens_in_part_and_is_completed_in_the_background() {
    let shared = Arc::new(FakeShared::default());
    let service = FakeService::start(shared.clone(), "secret");
    let (commands, connection) = connect_to(&Arc::new(Mutex::new(service.address)));
    // Identifiers do not sort in the order the workers were created.
    let ids: Vec<String> = (0..64).map(|level| format!("w{level}")).collect();
    let names: Vec<&str> = ids.iter().map(String::as_str).collect();
    shared.update(|state| {
        state.child_latency = Duration::from_millis(80);
        chain(state, "s1", &names);
    });
    let (id, events) = create_session(&commands);
    assert_eq!(id, "s1");
    let started = std::time::Instant::now();
    let snapshot = read_snapshot(&commands, &id, &events).unwrap();
    let elapsed = started.elapsed();
    assert!(
        elapsed < SNAPSHOT_TIMEOUT + Duration::from_secs(1),
        "the snapshot took {elapsed:?}"
    );
    assert_eq!(snapshot.workers_incomplete.as_deref(), Some(DISCOVERING));
    let found = worker_ids(&snapshot.workers);
    assert!(!found.is_empty() && found.len() < names.len(), "{found:?}");
    assert_eq!(found, names[..found.len()]);

    let watched = watch(&events);
    let seen = until(&watched, "the whole family", |event| {
        reported(event).is_some_and(|(_, notice)| notice.is_none())
    });
    let (workers, _) = reported(seen.last().unwrap()).unwrap();
    assert_eq!(worker_ids(workers), names);
    assert_eq!(workers[0].parent.as_deref(), Some("s1"));
    assert_eq!(workers[63].parent.as_deref(), Some("w62"));

    drop(commands);
    join_within(connection, Duration::from_secs(5));
    shared.update(|state| state.stopping = true);
    drop(service);
}

/// While the rest of a family is discovered in the background, worker
/// changes are still reported, as an incomplete list. Once it has been
/// found, a worker deleted meanwhile is not restored from an older page, a
/// worker created meanwhile under a parent found only then is kept, once,
/// a worker no page lists is not taken as deleted, and a state or title
/// observed meanwhile is kept over the record's.
#[test]
fn workers_discovered_in_the_background_keep_what_happened_meanwhile() {
    let shared = Arc::new(FakeShared::default());
    let service = FakeService::start(shared.clone(), "secret");
    let (commands, connection) = connect_to(&Arc::new(Mutex::new(service.address)));
    shared.update(|state| {
        chain(state, "s1", &["w0", "w1", "w2", "w3", "w4"]);
        // w6 is listed too, though it is created after w3's page is first
        // asked for.
        state
            .children
            .get_mut("w3")
            .unwrap()
            .push(fake_child("w6", "w3", 7));
        state.stalled_children.insert("w2".into());
    });
    let (id, events) = create_session(&commands);
    let snapshot = read_snapshot(&commands, &id, &events).unwrap();
    assert_eq!(worker_ids(&snapshot.workers), ["w0", "w1", "w2"]);
    assert_eq!(snapshot.workers_incomplete.as_deref(), Some(DISCOVERING));

    let watched = watch(&events);
    let partial = |event: &Event, wanted: &[&str]| {
        reported_ids(event) == Some((wanted.to_vec(), Some(DISCOVERING)))
    };
    service.send(json!({"type":"session.execution.started","data":{"sessionID":"w0"}}));
    let seen = until(&watched, "w0 running", |event| {
        partial(event, &["w0", "w1", "w2"])
    });
    assert_eq!(
        reported(seen.last().unwrap()).unwrap().0[0].state,
        WorkerState::Running
    );
    service.send(json!({"type":"session.deleted","data":{"sessionID":"w1"}}));
    until(&watched, "w1's departure", |event| {
        partial(event, &["w0", "w2"])
    });
    // w4 is deleted before it is found, though the simulated service
    // still lists it, as a page read before the deletion would; w5 is
    // created under w3 before w3 is found, and no page lists it; w6 is
    // created there too, and listed; and w3 starts and is renamed.
    service.send(json!({"type":"session.deleted","data":{"sessionID":"w4"}}));
    service.send(
        json!({"type":"session.created","data":{"sessionID":"w5","parentID":"w3","title":"w5"}}),
    );
    service.send(json!({"type":"session.renamed","data":{"sessionID":"w5","title":"Renamed w5"}}));
    service.send(
        json!({"type":"session.created","data":{"sessionID":"w6","parentID":"w3","title":"w6"}}),
    );
    service.send(json!({"type":"session.execution.started","data":{"sessionID":"w3"}}));
    service.send(json!({"type":"session.renamed","data":{"sessionID":"w3","title":"Renamed w3"}}));
    // x is created under the session itself, and no page lists it.
    service.send(
        json!({"type":"session.created","data":{"sessionID":"x","parentID":"s1","title":"x"}}),
    );
    until(&watched, "x's arrival", |event| {
        partial(event, &["w0", "w2", "x"])
    });

    shared.update(|state| {
        state.stalled_children.clear();
    });
    let seen = until(&watched, "the whole family", |event| {
        reported(event).is_some_and(|(_, notice)| notice.is_none())
    });
    let (workers, _) = reported(seen.last().unwrap()).unwrap();
    assert_eq!(worker_ids(workers), ["w0", "w2", "x", "w3", "w6", "w5"]);
    assert_eq!(workers[0].state, WorkerState::Running);
    assert_eq!(workers[3].state, WorkerState::Running);
    assert_eq!(workers[3].title, "Renamed w3");
    assert_eq!(workers[5].parent.as_deref(), Some("w3"));
    assert_eq!(workers[5].title, "Renamed w5");

    drop(commands);
    join_within(connection, Duration::from_secs(5));
    shared.update(|state| state.stopping = true);
    drop(service);
}

/// A background discovery whose listing OpenCode does not answer stops,
/// and the workers are reported as incomplete, with the reason, until it
/// goes on after a while from the listing that failed and finds the rest.
#[test]
fn a_stalled_background_discovery_is_visible_and_tried_again() {
    let shared = Arc::new(FakeShared::default());
    let service = FakeService::start(shared.clone(), "secret");
    let (commands, connection) = connect_to(&Arc::new(Mutex::new(service.address)));
    shared.update(|state| {
        chain(state, "s1", &["w0", "w1"]);
        state.stalled_children.insert("w0".into());
    });
    let (id, events) = create_session(&commands);
    let snapshot = read_snapshot(&commands, &id, &events).unwrap();
    assert_eq!(worker_ids(&snapshot.workers), ["w0"]);
    let watched = watch(&events);
    let seen = until_within(
        &watched,
        SNAPSHOT_TIMEOUT + Duration::from_secs(3),
        "the discovery stopping",
        |event| reported(event).is_some_and(|(_, notice)| notice != Some(DISCOVERING)),
    );
    let (workers, message) = reported(seen.last().unwrap()).unwrap();
    let message = message.unwrap();
    assert_eq!(worker_ids(workers), ["w0"]);
    assert!(
        message.starts_with("Could not find every worker")
            && message.contains("did not list a worker's children within 5 seconds")
            && message.ends_with("Trying again."),
        "{message}"
    );

    // OpenCode answers again, and the walk lists w0's children again.
    shared.update(|state| {
        state.stalled_children.clear();
    });
    until_within(
        &watched,
        DISCOVERY_RETRIES[0] + Duration::from_secs(3),
        "the whole family",
        |event| reported_ids(event) == Some((vec!["w0", "w1"], None)),
    );

    drop(commands);
    join_within(connection, Duration::from_secs(5));
    shared.update(|state| state.stopping = true);
    drop(service);
}

/// A worker found in the background is reported running only on positive
/// evidence, as in a snapshot: one OpenCode lists among the sessions it
/// runs is, while one whose record looks busy but which it does not list
/// is reported as unknown, not running, merely because it was found.
#[test]
fn workers_found_in_the_background_run_only_on_evidence() {
    let shared = Arc::new(FakeShared::default());
    let service = FakeService::start(shared.clone(), "secret");
    let (commands, connection) = connect_to(&Arc::new(Mutex::new(service.address)));
    shared.update(|state| {
        chain(state, "s1", &["w0"]);
        let mut stale = fake_child("stale", "w0", 2);
        stale["time"]["busy"] = json!(3);
        state
            .children
            .insert("w0".into(), vec![stale, fake_child("listed", "w0", 3)]);
        state.active.insert("listed".into());
        state.stalled_children.insert("w0".into());
    });
    let (id, events) = create_session(&commands);
    let snapshot = read_snapshot(&commands, &id, &events).unwrap();
    assert_eq!(worker_ids(&snapshot.workers), ["w0"]);
    assert_eq!(snapshot.workers_incomplete.as_deref(), Some(DISCOVERING));

    let watched = watch(&events);
    shared.update(|state| {
        state.stalled_children.clear();
    });
    let seen = until(&watched, "the whole family", |event| {
        reported(event).is_some_and(|(_, notice)| notice.is_none())
    });
    let (workers, _) = reported(seen.last().unwrap()).unwrap();
    let states: Vec<_> = workers
        .iter()
        .map(|worker| (worker.id.as_str(), worker.state))
        .collect();
    assert_eq!(
        states,
        [
            ("w0", WorkerState::Unknown),
            ("stale", WorkerState::Unknown),
            ("listed", WorkerState::Running),
        ]
    );

    drop(commands);
    join_within(connection, Duration::from_secs(5));
    shared.update(|state| state.stopping = true);
    drop(service);
}

/// A session read again after its event stream was lost, whose family
/// the read runs out of time for, against a simulated service: a known
/// worker the read did not reach is looked up and kept while it exists,
/// one deleted while the stream was down is removed, the workers are
/// reported as incomplete, and the background walk then goes on from
/// where the read stopped and reports every worker as complete.
#[test]
fn a_family_read_again_in_part_is_completed_in_the_background() {
    let shared = Arc::new(FakeShared::default());
    let service = FakeService::start(shared.clone(), "secret");
    let (commands, connection) = connect_to(&Arc::new(Mutex::new(service.address)));
    let names = ["w0", "w1", "w2", "w3"];
    shared.update(|state| {
        chain(state, "s1", &names);
        state
            .children
            .get_mut("s1")
            .unwrap()
            .push(fake_child("gone", "s1", 9));
        // Workers are sessions, which OpenCode answers when looked up.
        for id in names.iter().chain(&["gone"]) {
            state.sessions.insert((*id).into(), Vec::new());
        }
        state.stalled_children.insert("w2".into());
    });
    let (id, events) = create_session(&commands);
    let snapshot = read_snapshot(&commands, &id, &events).unwrap();
    assert_eq!(worker_ids(&snapshot.workers), ["w0", "w1", "w2", "gone"]);
    let watched = watch(&events);
    shared.update(|state| {
        state.stalled_children.clear();
    });
    until(&watched, "the whole family", |event| {
        reported_ids(event) == Some((vec!["w0", "w1", "w2", "gone", "w3"], None))
    });

    // While the stream is down, "gone" is deleted, and the read again
    // cannot list w0's children in time.
    shared.update(|state| {
        state.sessions.remove("gone");
        state.children.get_mut("s1").unwrap().pop();
        state.stalled_children.insert("w0".into());
    });
    service.end_streams();
    let seen = until_within(
        &watched,
        SNAPSHOT_TIMEOUT * 2,
        "the family read again in part",
        |event| reported_ids(event) == Some((names.to_vec(), Some(DISCOVERING))),
    );
    assert!(
        seen.contains(&Event::Reconnecting {
            reason: DISCONNECTED.to_owned()
        }),
        "{seen:?}"
    );
    shared.update(|state| {
        state.stalled_children.clear();
    });
    until(&watched, "the whole family again", |event| {
        reported_ids(event) == Some((names.to_vec(), None))
    });

    drop(commands);
    join_within(connection, Duration::from_secs(5));
    shared.update(|state| state.stopping = true);
    drop(service);
}

/// Only the workers may be read in part: a session whose conversation
/// is not read in time is still refused, whatever its family, and so is
/// one whose family OpenCode fails to list while it is read.
#[test]
fn a_conversation_not_read_in_time_still_refuses_the_snapshot() {
    let shared = Arc::new(FakeShared::default());
    let service = FakeService::start(shared.clone(), "secret");
    let (commands, connection) = connect_to(&Arc::new(Mutex::new(service.address)));
    shared.update(|state| {
        chain(state, "s1", &["w0"]);
        state.stalled.insert("s1".into());
    });
    let (id, events) = create_session(&commands);
    let refused = read_snapshot(&commands, &id, &events).unwrap_err();
    assert_eq!(refused.kind, tomori_backend::ErrorKind::Unavailable);
    assert!(refused.message.contains("timed out"), "{refused:?}");

    shared.update(|state| {
        chain(state, "s2", &["v0"]);
        state.failing_children.insert("v0".into());
    });
    let (id, events) = create_session(&commands);
    assert_eq!(id, "s2");
    let refused = read_snapshot(&commands, &id, &events).unwrap_err();
    assert!(refused.message.contains("lost"), "{refused:?}");

    drop(commands);
    join_within(connection, Duration::from_secs(5));
    shared.update(|state| state.stopping = true);
    drop(service);
}

/// Closing a session while its workers are discovered cancels the
/// discovery, and its result is ignored, as is that of a discovery a
/// later read replaced.
#[test]
fn closing_during_a_discovery_cancels_it_and_ignores_its_result() {
    let mut registrations = HashMap::new();
    let events = insert_registration(&mut registrations, "s".into(), "/tmp", Modes::default());
    let discovering = Cancellation::new();
    let registration = registrations.get_mut("s").unwrap();
    registration.workers_ready = true;
    registration.discovery_in_flight = Some(discovering.clone());
    registration.discoveries = 1;
    let identity = events.identity.clone();
    let found = |number| DiscoveryResult {
        id: "s".into(),
        registration: identity.clone(),
        number,
        found: vec![fake_child("w", "s", 1)],
        result: Ok(()),
        walk: api::FamilyWalk::new("s"),
        active: None,
    };
    close_registration(&mut registrations, "s", &identity);
    assert!(discovering.is_cancelled());
    assert_eq!(
        events.try_recv().unwrap().into_current(),
        Some(Event::Closed { reason: None })
    );
    // A later registration of the same session is not touched by the
    // abandoned discovery's result.
    let replacement = insert_registration(&mut registrations, "s".into(), "/tmp", Modes::default());
    finish_discovery(&mut registrations, found(1), Instant::now());
    assert!(registrations["s"].children.is_empty());
    assert!(replacement.try_recv().is_err());

    // A read that walked the family whole stops the discovery an earlier
    // one started, whose result is then ignored too.
    let identity = replacement.identity.clone();
    let registration = registrations.get_mut("s").unwrap();
    registration.workers_ready = true;
    registration.continue_discovery(Some(api::FamilyWalk::new("s")));
    assert_eq!(registration.workers_partial.as_deref(), Some(DISCOVERING));
    let running = Cancellation::new();
    registration.discovery = None;
    registration.discovery_in_flight = Some(running.clone());
    registration.discoveries = 1;
    registration.continue_discovery(None);
    assert!(running.is_cancelled());
    assert!(registration.workers_partial.is_none());
    finish_discovery(
        &mut registrations,
        DiscoveryResult {
            registration: identity,
            ..found(1)
        },
        Instant::now(),
    );
    assert!(registrations["s"].children.is_empty());
    assert!(replacement.try_recv().is_err());
}

/// Open the session "s" with no workers yet, its family read in part, as
/// when the read ran out of time for it, so that the rest is discovered.
fn opened_in_part() -> (HashMap<String, Registration>, RegistrationEvents) {
    let (mut registrations, events) = opening();
    let (reply, answer) = async_channel::bounded(1);
    finish_snapshot(
        &mut registrations,
        SnapshotResult {
            id: "s".into(),
            registration: events.identity.clone(),
            reply,
            result: Ok(Snapshot::default()),
            rest: Some(api::FamilyWalk::new("s")),
        },
    );
    let snapshot = answer
        .try_recv()
        .unwrap()
        .and_then(SnapshotReply::accept)
        .unwrap();
    assert_eq!(snapshot.workers_incomplete.as_deref(), Some(DISCOVERING));
    (registrations, events)
}

/// Start the discovery waiting for "s", as [`start_discoveries`] does at
/// `at`, and finish it with `found` and `result`.
fn discover(
    registrations: &mut HashMap<String, Registration>,
    events: &RegistrationEvents,
    at: Instant,
    found: Vec<Value>,
    result: std::result::Result<(), Failure>,
) {
    assert_eq!(due_discoveries(registrations, at, MAX_DISCOVERIES), ["s"]);
    let (walk, number) = registrations
        .get_mut("s")
        .unwrap()
        .start_discovery(Cancellation::new());
    finish_discovery(
        registrations,
        DiscoveryResult {
            id: "s".into(),
            registration: events.identity.clone(),
            number,
            found,
            result,
            walk,
            active: None,
        },
        at,
    );
}

/// The latest worker list delivered to `events`, with its notice if it is
/// incomplete.
fn latest_workers(events: &RegistrationEvents) -> Option<(Vec<Worker>, Option<String>)> {
    let mut latest = None;
    while let Ok(event) = events.try_recv() {
        let event = event.into_current();
        latest = event
            .as_ref()
            .and_then(reported)
            .map(|(workers, notice)| (workers.to_vec(), notice.map(str::to_owned)))
            .or(latest);
    }
    latest
}

fn busy() -> std::result::Result<(), Failure> {
    Err(Failure {
        status: Some(500),
        message: "busy".into(),
    })
}

/// A background discovery that fails goes on from where it stopped after
/// 5, 30 and 120 seconds, the workers it found meanwhile reported as
/// incomplete with the reason, and once its last attempt fails it stops,
/// leaving them so. While it waits, lifecycle events are kept over the
/// pages listed later: a worker deleted meanwhile is not restored, and a
/// state observed meanwhile is kept. Time is given, not waited for.
#[test]
fn a_failed_background_discovery_is_tried_again_then_given_up() {
    let (mut registrations, events) = opened_in_part();
    let retrying = "Could not find every worker of this session yet, so some may be missing: busy. Trying again.";
    let start = Instant::now();
    discover(
        &mut registrations,
        &events,
        start,
        vec![fake_child("w1", "s", 1)],
        busy(),
    );
    let (workers, notice) = latest_workers(&events).unwrap();
    assert_eq!(worker_ids(&workers), ["w1"]);
    assert_eq!(notice.as_deref(), Some(retrying));

    // Each retry waits its delay from the failure before it.
    let mut failed = start;
    for (attempt, delay) in DISCOVERY_RETRIES.iter().enumerate() {
        let due = failed + *delay;
        assert!(
            due_discoveries(
                &registrations,
                due - Duration::from_millis(1),
                MAX_DISCOVERIES
            )
            .is_empty(),
            "attempt {attempt} started early"
        );
        if attempt == 0 {
            // w2 starts and w3 is deleted before either is found.
            for event in [
                json!({"type":"session.execution.started","data":{"sessionID":"w2"}}),
                json!({"type":"session.deleted","data":{"sessionID":"w3"}}),
            ] {
                apply_worker_event(&event, &mut registrations);
            }
        }
        let found = match attempt {
            0 => vec![fake_child("w2", "w1", 2)],
            1 => vec![fake_child("w3", "w1", 3)],
            _ => Vec::new(),
        };
        discover(&mut registrations, &events, due, found, busy());
        failed = due;
    }

    // The last attempt failed: the discovery stops, and the workers stay
    // incomplete, with the reason.
    assert!(due_discoveries(&registrations, failed + Duration::from_secs(3600), 1).is_empty());
    let registration = &registrations["s"];
    assert!(registration.discovery.is_none() && registration.discovery_in_flight.is_none());
    assert!(registration.initial_changes.is_empty() && registration.initial_deletions.is_empty());
    let (workers, notice) = latest_workers(&events).unwrap();
    let states: Vec<_> = workers
        .iter()
        .map(|worker| (worker.id.as_str(), worker.state))
        .collect();
    assert_eq!(
        states,
        [("w1", WorkerState::Unknown), ("w2", WorkerState::Running)]
    );
    assert_eq!(
        notice.as_deref(),
        Some("Could not find every worker of this session, so some may be missing: busy")
    );
}

/// A retry a discovery waits for is dropped when the session is read
/// again, which starts the attempts afresh or, having walked the family
/// whole, ends the discovery, and when the connection is lost or the
/// session closes. The result of a discovery stopped so is ignored, so it
/// neither counts as a failure nor overwrites the newer state.
#[test]
fn a_discovery_retry_is_dropped_when_the_session_is_read_again_or_closes() {
    let (mut registrations, events) = opened_in_part();
    let start = Instant::now();
    discover(&mut registrations, &events, start, Vec::new(), busy());
    assert!(due_discoveries(&registrations, start, 1).is_empty());

    // A read again that ran out of time for the family goes on at once,
    // with every attempt still to come.
    let registration = registrations.get_mut("s").unwrap();
    registration.continue_discovery(Some(api::FamilyWalk::new("s")));
    assert_eq!(registration.discovery_failures, 0);
    assert_eq!(registration.workers_partial.as_deref(), Some(DISCOVERING));
    assert_eq!(due_discoveries(&registrations, start, 1), ["s"]);

    // A read again that walked the family whole stops the discovery in
    // flight, whose failure is then ignored.
    let running = Cancellation::new();
    let registration = registrations.get_mut("s").unwrap();
    let (walk, number) = registration.start_discovery(running.clone());
    registration.continue_discovery(None);
    assert!(running.is_cancelled());
    let _ = latest_workers(&events);
    finish_discovery(
        &mut registrations,
        DiscoveryResult {
            id: "s".into(),
            registration: events.identity.clone(),
            number,
            found: vec![fake_child("old", "s", 1)],
            result: busy(),
            walk,
            active: None,
        },
        start,
    );
    let registration = &registrations["s"];
    assert!(registration.discovery.is_none() && registration.discovery_retry_at.is_none());
    assert_eq!(registration.discovery_failures, 0);
    assert!(registration.children.is_empty() && registration.workers_partial.is_none());
    assert!(latest_workers(&events).is_none());

    // A lost connection drops a retry: the session is read again on the
    // next one.
    let registration = registrations.get_mut("s").unwrap();
    registration.continue_discovery(Some(api::FamilyWalk::new("s")));
    discover(&mut registrations, &events, start, Vec::new(), busy());
    let _ = latest_workers(&events);
    connection_lost(&mut registrations);
    // Only the notice of the gap is delivered, no worker list.
    assert_eq!(latest_workers(&events), None);
    let registration = &registrations["s"];
    assert!(registration.discovery.is_none() && registration.discovery_retry_at.is_none());
    assert_eq!(registration.discovery_failures, 0);

    // Closing the session drops a retry with the registration, and the
    // failure of a discovery in flight is not taken by the session's next
    // registration.
    let registration = registrations.get_mut("s").unwrap();
    registration.resync = None;
    registration.continue_discovery(Some(api::FamilyWalk::new("s")));
    let (walk, number) = registration.start_discovery(Cancellation::new());
    close_registration(&mut registrations, "s", &events.identity);
    let replacement = insert_registration(&mut registrations, "s".into(), "/tmp", Modes::default());
    finish_discovery(
        &mut registrations,
        DiscoveryResult {
            id: "s".into(),
            registration: events.identity.clone(),
            number,
            found: Vec::new(),
            result: busy(),
            walk,
            active: None,
        },
        start,
    );
    let registration = &registrations["s"];
    assert!(registration.discovery.is_none() && registration.discovery_retry_at.is_none());
    assert_eq!(registration.discovery_failures, 0);
    assert!(replacement.try_recv().is_err());
}

/// A read again after reconnecting that runs out of time for the family
/// merges the workers it found and discovers the rest in the background,
/// reporting the workers as incomplete meanwhile.
#[test]
fn a_read_again_goes_on_discovering_the_family_it_ran_out_of_time_for() {
    let mut registrations = HashMap::new();
    let events = insert_registration(&mut registrations, "s".into(), "/tmp", Modes::default());
    let registration = registrations.get_mut("s").unwrap();
    registration.workers_ready = true;
    registration.resync = Some(Resync::default());
    let identity = registration.events.worker_epoch.clone();
    finish_resync(
        &mut registrations,
        ResyncResult {
            id: "s".into(),
            registration: identity,
            result: Ok(Snapshot {
                workers: vec![Worker {
                    id: "w".into(),
                    parent: Some("s".into()),
                    title: "w".into(),
                    state: WorkerState::Unknown,
                }],
                ..Snapshot::default()
            }),
            rest: Some(api::FamilyWalk::new("s")),
            missing: Vec::new(),
            modes: None,
        },
        Instant::now(),
    );
    assert!(registrations["s"].discovery.is_some());
    let mut workers = None;
    while let Ok(event) = events.try_recv() {
        if let Some(event @ Event::WorkersPartial { .. }) = event.into_current() {
            workers = Some(event);
        }
    }
    assert!(
        matches!(&workers, Some(Event::WorkersPartial { workers, message })
            if worker_ids(workers) == ["w"] && message == DISCOVERING),
        "{workers:?}"
    );
}

/// Workers found in the background are checked like a snapshot's: an
/// identity beyond the supported size is refused, leaving the workers
/// visibly incomplete, and more workers than can be tracked make
/// observation visibly incomplete, as do pending creations beyond it.
#[test]
fn workers_found_in_the_background_stay_within_the_limits() {
    let worker = |id: String, parent: &str| Worker {
        id,
        parent: Some(parent.into()),
        title: "worker".into(),
        state: WorkerState::Unknown,
    };
    let (mut registration, _events) = Registration::new();
    registration.adopt_discovered(
        vec![worker("x".repeat(MAX_WORKER_ID_BYTES + 1), "s")],
        None,
        Instant::now(),
    );
    assert!(registration.children.is_empty());
    assert!(
        registration
            .workers_partial
            .as_deref()
            .is_some_and(|reason| reason.contains("exceeds the supported size")),
        "{:?}",
        registration.workers_partial
    );

    let (mut registration, _events) = Registration::new();
    registration.adopt_discovered(
        (0..=api::MAX_TRACKED_WORKERS)
            .map(|n| worker(format!("w{n}"), "s"))
            .collect(),
        None,
        Instant::now(),
    );
    assert!(registration.workers_incomplete);
    assert_eq!(registration.children.len(), api::MAX_TRACKED_WORKERS);

    let (mut registration, _events) = Registration::new();
    registration.adopt_discovered(
        (0..api::MAX_TRACKED_WORKERS)
            .map(|n| worker(format!("w{n}"), "s"))
            .collect(),
        None,
        Instant::now(),
    );
    assert!(!registration.workers_incomplete);
    registration
        .initial_creations
        .insert("late".into(), worker("late".into(), "w0"));
    registration.adopt_discovered(Vec::new(), None, Instant::now());
    assert!(registration.workers_incomplete);

    // A snapshot whose workers leave no room for a creation kept while
    // they were read is refused.
    let (mut registration, _events) = Registration::new();
    registration
        .initial_creations
        .insert("late".into(), worker("late".into(), "w0"));
    let mut listed = (0..api::MAX_TRACKED_WORKERS)
        .map(|n| worker(format!("w{n}"), "s"))
        .collect();
    assert!(registration.install_workers(&mut listed).is_err());
}

/// A title observed for a worker before it is found, without a state,
/// is kept over its record's, whether a background discovery or the
/// initial snapshot finds it.
#[test]
fn a_title_observed_before_a_worker_is_found_is_kept() {
    let renamed =
        |id: &str| json!({"type":"session.renamed","data":{"sessionID":id,"title":"Renamed"}});
    let (mut opened, events) = opened_in_part();
    apply_worker_event(&renamed("w"), &mut opened);
    discover(
        &mut opened,
        &events,
        Instant::now(),
        vec![fake_child("w", "s", 1)],
        Ok(()),
    );
    let found = Worker {
        id: "w".into(),
        parent: Some("s".into()),
        title: "Renamed".into(),
        state: WorkerState::Unknown,
    };
    assert_eq!(latest_workers(&events), Some((vec![found], None)));

    let mut registrations = registrations();
    apply_worker_event(&renamed("child"), &mut registrations);
    let mut listed = vec![Worker {
        id: "child".into(),
        parent: Some("parent".into()),
        title: "old".into(),
        state: WorkerState::Completed,
    }];
    registrations
        .get_mut("parent")
        .unwrap()
        .install_workers(&mut listed)
        .unwrap();
    assert_eq!(listed[0].title, "Renamed");
    assert_eq!(listed[0].state, WorkerState::Completed);
}

/// A worker deleted while the event stream was down is found missing
/// when its session is read again: the listing omits it, and OpenCode
/// answers that it does not exist. An omitted worker that still exists, or
/// whose lookup fails, is kept.
#[test]
fn a_worker_deleted_while_disconnected_is_removed_once_verified() {
    let shared = Arc::new(FakeShared::default());
    let mut first = FakeService::start(shared.clone(), "secret");
    let address = Arc::new(Mutex::new(first.address));
    let (commands, connection) = connect_to(&address);
    let (id, events) = block_on(commands.reply_with(|reply| Command::Create {
        spec: SessionSpec {
            directory: "/tmp".into(),
            title: "Workers".into(),
        },
        reply,
    }))
    .unwrap();
    let identity = events.identity.clone();
    block_on(commands.reply_with(|reply| Command::Snapshot {
        id: id.clone(),
        identity,
        reply,
    }))
    .and_then(SnapshotReply::accept)
    .unwrap();
    let watched = watch(&events);
    for child in ["c1", "c2", "c3", "c4"] {
        shared.update(|state| {
            state.sessions.insert(child.into(), Vec::new());
        });
        first.send(json!({"type":"session.created","data":{"sessionID":child,"parentID":id,"title":child}}));
    }
    until(
        &watched,
        "the workers",
        |event| matches!(event, Event::WorkersChanged { workers } if workers.len() == 4),
    );

    // Read the session again after a gap, returning its workers.
    let gap = |service: &mut FakeService, change: &dyn Fn(&mut FakeState)| {
        service.stop();
        until(&watched, "the disconnection notice", |event| {
            kind(event) == "reconnecting"
        });
        shared.update(change);
        let next = FakeService::start(shared.clone(), "secret");
        *address.lock().unwrap() = next.address;
        let mut seen = until_within(
            &watched,
            SNAPSHOT_TIMEOUT * 3,
            "the resynchronization",
            |event| kind(event) == "resynchronized",
        );
        // Workers travel on their own lane, so may come before or after it.
        seen.extend(std::iter::from_fn(|| {
            watched.recv_timeout(Duration::from_millis(300)).ok()
        }));
        let Some(Event::WorkersChanged { workers }) = seen
            .iter()
            .rev()
            .find(|event| kind(event) == "workers_changed")
        else {
            unreachable!("the workers read again did not arrive");
        };
        let workers: Vec<String> = workers.iter().map(|worker| worker.id.clone()).collect();
        (next, workers)
    };
    // While the stream is down, c1 is deleted, and c3's record will fail.
    let (mut second, workers) = gap(&mut first, &|state| {
        state.sessions.remove("c1");
        state.failing_records.insert("c3".into());
    });
    assert_eq!(workers, ["c2", "c3", "c4"]);
    // Then c2 is deleted, and c4's record will not be answered in time:
    // c2, found missing in time, is still removed.
    let (second, workers) = gap(&mut second, &|state| {
        state.sessions.remove("c2");
        state.stalled_records.insert("c4".into());
    });
    assert_eq!(workers, ["c3", "c4"]);
    shared.update(|state| {
        state.stalled_records.clear();
    });
    drop(commands);
    join_within(connection, Duration::from_secs(5));
    shared.update(|state| state.stopping = true);
    drop(second);
}

/// A worker found missing is removed unless a lifecycle event during the
/// read showed it, and none is removed once observation is incomplete.
#[test]
fn only_workers_verified_missing_are_removed() {
    let (mut registration, _events) = Registration::new();
    for id in ["missing", "changed"] {
        registration.children.insert(
            id.into(),
            Worker {
                id: id.into(),
                parent: Some("parent".into()),
                title: id.into(),
                state: WorkerState::Unknown,
            },
        );
    }
    let changes = HashMap::from([("changed".to_owned(), (Some(WorkerState::Completed), None))]);
    let missing = ["missing".to_owned(), "changed".to_owned()];
    assert_eq!(registration.known_workers(), ["changed", "missing"]);
    registration.workers_incomplete = true;
    assert!(registration.known_workers().is_empty());
    registration.remove_missing(&missing, &changes);
    assert_eq!(registration.children.len(), 2);
    registration.workers_incomplete = false;
    registration.remove_missing(&missing, &changes);
    assert_eq!(
        registration.children.keys().collect::<Vec<_>>(),
        ["changed"]
    );
}
