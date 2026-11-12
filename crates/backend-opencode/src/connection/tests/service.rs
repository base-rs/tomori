// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What each command asks of a simulated service, and what a session's record and history are read as.

use super::*;

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
        // The top-level listing is the children of `null`: one session beneath `/tmp`, busy, one outside it, and
        // a worker that a server returns anyway.
        let mut beneath = fake_record("beneath");
        beneath["title"] = json!("Beneath");
        beneath["location"]["directory"] = json!("/tmp/project");
        let mut outside = fake_record("outside");
        outside["location"]["directory"] = json!("/tmpfiles");
        let mut worker = fake_record("worker");
        worker["parentID"] = json!("beneath");
        state
            .children
            .insert("null".into(), vec![beneath, outside, worker]);
        state.active.insert("beneath".into());
    });
    let service = FakeService::start(shared.clone(), "");
    let backend = crate::OpenCode::new(crate::Config {
        url: Some(format!("http://{}", service.address)),
    });
    assert_eq!(
        block_on(backend.sessions("/tmp")).unwrap(),
        [SessionSummary {
            id: "beneath".into(),
            title: "Beneath".into(),
            directory: "/tmp/project".into(),
            busy: true,
        }]
    );
    // A listing OpenCode refuses fails, with its reason.
    shared.update(|state| {
        state.failing_children.insert("null".into());
    });
    assert_eq!(
        block_on(backend.sessions("/tmp")).unwrap_err().message,
        "lost"
    );
    shared.update(|state| {
        state.failing_children.clear();
        state.children.remove("null");
        state.active.clear();
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
            delete: true,
            tools: true,
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
    block_on(session.answer("f1", vec![("name".into(), AnswerValue::Text("Ada".into()))])).unwrap();
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
    // Where it is now is where its record says.
    assert_eq!(
        block_on(backend.locate("existing")),
        Ok(Some("/tmp".to_owned()))
    );
    let invalid = block_on(backend.forget_instructions("gone", vec!["Tomori".into()])).unwrap_err();
    assert_eq!(invalid.kind, ErrorKind::Invalid);
    block_on(backend.forget_instructions("gone", vec!["tomori".into()])).unwrap();
    assert_eq!(
        shared.state.lock().unwrap().instruction_requests,
        [("DELETE".to_owned(), "gone".to_owned(), "tomori".to_owned())]
    );
    // A session attached is a space's, and is not deleted; once closed, it is stopped and deleted, and deleting it
    // again finds it gone, which is what was asked.
    let refused = block_on(backend.delete_session("existing")).unwrap_err();
    assert_eq!(refused.kind, ErrorKind::Invalid);
    block_on(attached.close()).unwrap();
    let actions = shared.state.lock().unwrap().actions.len();
    block_on(backend.delete_session("existing")).unwrap();
    block_on(backend.delete_session("existing")).unwrap();
    assert_eq!(
        shared.state.lock().unwrap().actions[actions..],
        [
            ("/api/session/existing/interrupt".to_owned(), json!({})),
            ("DELETE /api/session/existing".to_owned(), Value::Null),
            ("DELETE /api/session/existing".to_owned(), Value::Null),
        ]
    );
    assert!(
        !shared
            .state
            .lock()
            .unwrap()
            .sessions
            .contains_key("existing")
    );
    // Tomori's MCP server is registered for a directory, with its headers, without OAuth, and removed again; removing
    // one OpenCode does not have is already done, and a refusal is said. Another server there is never touched.
    let server = tomori_backend::ToolServer::new("tomori", "http://127.0.0.1:9/mcp")
        .with_header("Authorization", "Bearer tomori_secret");
    let other = (
        "/api/experimental/mcp/github?location%5Bdirectory%5D=%2Fwork%2Fa%20b".to_owned(),
        json!({"type": "local", "command": ["github-mcp"]}),
    );
    shared.update(|state| {
        state.mcp.insert(other.0.clone(), other.1.clone());
    });
    block_on(backend.register_tools("/work/a b", &server)).unwrap();
    let path = "/api/experimental/mcp/tomori?location%5Bdirectory%5D=%2Fwork%2Fa%20b";
    assert_eq!(
        shared.state.lock().unwrap().mcp,
        BTreeMap::from([
            other.clone(),
            (
                path.to_owned(),
                json!({
                    "type": "remote",
                    "url": "http://127.0.0.1:9/mcp",
                    "headers": {"Authorization": "Bearer tomori_secret"},
                    "oauth": false,
                })
            )
        ])
    );
    block_on(backend.unregister_tools("/work/a b", "tomori")).unwrap();
    block_on(backend.unregister_tools("/work/a b", "tomori")).unwrap();
    assert_eq!(shared.state.lock().unwrap().mcp, BTreeMap::from([other]));
    shared.update(|state| state.failing_mcp = true);
    let failed = block_on(backend.register_tools("/work/a b", &server)).unwrap_err();
    assert_eq!(failed.message, "lost");
    let failed = block_on(backend.unregister_tools("/work/a b", "tomori")).unwrap_err();
    assert_eq!(failed.message, "lost");
    assert_eq!(
        shared.state.lock().unwrap().mcp_requests,
        [
            ("PUT", path),
            ("DELETE", path),
            ("DELETE", path),
            ("PUT", path),
            ("DELETE", path)
        ]
        .map(|(method, path)| (method.to_owned(), path.to_owned()))
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

/// A worker is deleted only once OpenCode's records say it descends from the
/// session through no other space's session, and, unless forced, that it
/// is not running; it then leaves the session's workers with its own.
#[test]
fn a_worker_is_deleted_only_beneath_its_session_and_takes_its_own_workers() {
    use tomori_backend::{Backend, ErrorKind, Session};

    let shared = Arc::new(FakeShared::default());
    let finished = |id: &str, parent: &str, created: u64| {
        let mut record = fake_child(id, parent, created);
        record["outcome"] = json!("succeeded");
        record
    };
    shared.update(|state| {
        for id in ["root", "w1", "w2", "w3", "w4", "other", "o1", "loose"] {
            state.sessions.insert(id.into(), Vec::new());
        }
        // OpenCode runs w3, so it is observed running; w4 is never observed, and its record looks busy.
        state.active.insert("w3".into());
        let mut running = fake_child("w3", "root", 3);
        running["time"]["busy"] = json!(5);
        let mut unobserved = fake_child("w4", "root", 6);
        unobserved["time"]["busy"] = json!(5);
        state.records.insert("w4".into(), unobserved);
        state.children.insert(
            "root".into(),
            vec![finished("w1", "root", 1), running.clone()],
        );
        state
            .children
            .insert("w1".into(), vec![finished("w2", "w1", 2)]);
        // Another space's session, itself started by root, and its worker.
        for record in [
            finished("w1", "root", 1),
            finished("w2", "w1", 2),
            running,
            finished("other", "root", 4),
            finished("o1", "other", 5),
        ] {
            state
                .records
                .insert(record["id"].as_str().unwrap().to_owned(), record);
        }
    });
    let service = FakeService::start(shared.clone(), "");
    let backend = crate::OpenCode::new(crate::Config {
        url: Some(format!("http://{}", service.address)),
    });
    let root = Arc::new(block_on(backend.attach("root", "/tmp")).unwrap());
    let other = block_on(backend.attach("other", "/tmp")).unwrap();
    let snapshot = block_on(root.snapshot()).unwrap();
    assert_eq!(worker_ids(&snapshot.workers), ["w1", "w2", "w3"]);
    let (tx, events) = std::sync::mpsc::channel();
    let watched = root.clone();
    std::thread::spawn(move || {
        while let Ok(event) = block_on(watched.next_event())
            && tx.send(event).is_ok()
        {}
    });
    let refused = |worker: &str, force: bool| {
        let error = block_on(backend.delete_worker("root", worker, force)).unwrap_err();
        (error.kind, error.message)
    };
    let invalid = |message: &str| (ErrorKind::Invalid, message.to_owned());
    // The space's own session, another space's, one of that space's workers and an unrelated session are
    // refused, and nothing is stopped or deleted.
    assert_eq!(
        refused("root", true),
        invalid("the worker's session is bound to a space")
    );
    assert_eq!(
        refused("other", true),
        invalid("the worker's session is bound to a space")
    );
    assert_eq!(
        refused("o1", true),
        invalid("a session it descends from is bound to another space")
    );
    assert_eq!(
        refused("loose", true),
        invalid("the session is not a worker of this space's session")
    );
    // A running worker is refused unless forced, and so is a worker whose own worker is running.
    assert_eq!(
        refused("w3", false),
        (ErrorKind::Busy, "the worker is running".to_owned())
    );
    assert_eq!(
        refused("w4", false),
        (ErrorKind::Busy, "the worker is running".to_owned())
    );
    // A worker observed as not running is not refused for its record's times, which may be stale.
    shared.update(|state| {
        let mut stale = finished("w2", "w1", 2);
        stale["time"]["busy"] = json!(5);
        state.records.insert("w2".into(), stale);
        state.failing_actions.insert("w2".into());
    });
    let error = block_on(backend.delete_worker("root", "w2", false)).unwrap_err();
    assert_eq!(error.message, "refused", "it went on to stop it");
    shared.update(|state| {
        state.failing_actions.clear();
    });
    assert!(shared.state.lock().unwrap().actions.is_empty());
    // A finished worker is stopped and deleted, and leaves the workers with its own.
    block_on(backend.delete_worker("root", "w1", false)).unwrap();
    let seen = until(&events, "the workers without w1", |event| {
        reported_ids(event).is_some_and(|(ids, _)| ids == ["w3"])
    });
    // Its own worker went with it, never after it.
    assert!(
        seen.iter()
            .filter_map(reported_ids)
            .all(|(ids, _)| ids.contains(&"w1") == ids.contains(&"w2"))
    );
    // Forced, a running worker is stopped first.
    block_on(backend.delete_worker("root", "w3", true)).unwrap();
    until(&events, "no workers", |event| {
        reported_ids(event).is_some_and(|(ids, _)| ids.is_empty())
    });
    // One that no longer exists is already gone.
    block_on(backend.delete_worker("root", "w1", false)).unwrap();
    assert_eq!(
        shared.state.lock().unwrap().actions,
        [
            ("/api/session/w1/interrupt".to_owned(), json!({})),
            ("DELETE /api/session/w1".to_owned(), Value::Null),
            ("/api/session/w3/interrupt".to_owned(), json!({})),
            ("DELETE /api/session/w3".to_owned(), Value::Null),
        ]
    );
    // A worker whose parent is gone cannot be followed to the session.
    shared.update(|state| {
        state.sessions.insert("orphan".into(), Vec::new());
        state
            .records
            .insert("orphan".into(), finished("orphan", "w3", 6));
    });
    assert_eq!(
        refused("orphan", false),
        invalid("a session it descends from no longer exists")
    );
    // A chain too long to follow is refused.
    shared.update(|state| {
        for depth in 0..MAX_WORKER_DEPTH {
            let id = format!("deep{depth}");
            let parent = format!("deep{}", depth + 1);
            state.sessions.insert(id.clone(), Vec::new());
            state.records.insert(id.clone(), finished(&id, &parent, 7));
        }
    });
    assert_eq!(
        refused("deep0", false),
        invalid("its chain of parents is too long to follow to this space's session")
    );
    // A failure reading a record, its own or one it descends from, or stopping it, is said, and nothing is deleted.
    shared.update(|state| {
        state.failing_records.insert("w2".into());
        state.sessions.insert("w9".into(), Vec::new());
        state.records.insert("w9".into(), finished("w9", "w2", 8));
        state.failing_actions.insert("w3".into());
        state.sessions.insert("w3".into(), Vec::new());
    });
    let failed = block_on(backend.delete_worker("root", "w2", false)).unwrap_err();
    assert_eq!(failed.kind, ErrorKind::Unavailable);
    let failed = block_on(backend.delete_worker("root", "w9", false)).unwrap_err();
    assert_eq!(failed.kind, ErrorKind::Unavailable);
    let failed = block_on(backend.delete_worker("root", "w3", true)).unwrap_err();
    assert_eq!(
        (failed.kind, failed.message.as_str()),
        (ErrorKind::Unavailable, "refused")
    );
    assert!(shared.state.lock().unwrap().sessions.contains_key("w3"));
    // A worker of a session no space is attached to is deleted all the same, with nobody to tell.
    shared.update(|state| {
        state.sessions.insert("solo".into(), Vec::new());
        state.sessions.insert("s1".into(), Vec::new());
        state.records.insert("s1".into(), finished("s1", "solo", 9));
    });
    block_on(backend.delete_worker("solo", "s1", false)).unwrap();
    assert!(!shared.state.lock().unwrap().sessions.contains_key("s1"));
    backend.shutdown();
    assert_eq!(
        refused("w2", false).0,
        ErrorKind::Unavailable,
        "a backend shut down deletes nothing"
    );
    drop(other);
}

/// Of a worker's own workers known, one running refuses its deletion
/// unless forced, and with force is stopped first.
#[test]
fn a_worker_whose_own_worker_runs_is_deleted_only_when_forced() {
    use tomori_backend::{Backend, ErrorKind, Session};

    let shared = Arc::new(FakeShared::default());
    shared.update(|state| {
        for id in ["root", "w1", "w2"] {
            state.sessions.insert(id.into(), Vec::new());
        }
        let mut done = fake_child("w1", "root", 1);
        done["outcome"] = json!("succeeded");
        state.children.insert("root".into(), vec![done.clone()]);
        state.records.insert("w1".into(), done);
        let mut running = fake_child("w2", "w1", 2);
        running["time"]["busy"] = json!(5);
        state.children.insert("w1".into(), vec![running]);
        // OpenCode says it is running it, so it is known to run.
        state.active.insert("w2".into());
    });
    let service = FakeService::start(shared.clone(), "");
    let backend = crate::OpenCode::new(crate::Config {
        url: Some(format!("http://{}", service.address)),
    });
    let root = block_on(backend.attach("root", "/tmp")).unwrap();
    let snapshot = block_on(root.snapshot()).unwrap();
    assert_eq!(worker_ids(&snapshot.workers), ["w1", "w2"]);
    let error = block_on(backend.delete_worker("root", "w1", false)).unwrap_err();
    assert_eq!(
        (error.kind, error.message.as_str()),
        (
            ErrorKind::Busy,
            "one of the worker's own workers is running"
        )
    );
    block_on(backend.delete_worker("root", "w1", true)).unwrap();
    assert_eq!(
        shared.state.lock().unwrap().actions,
        [
            ("/api/session/w1/interrupt".to_owned(), json!({})),
            ("/api/session/w2/interrupt".to_owned(), json!({})),
            ("DELETE /api/session/w1".to_owned(), Value::Null),
        ]
    );
    backend.shutdown();
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
    refused(block_on(backend.locate("s1")).map(drop));
    refused(block_on(
        backend.forget_instructions("s1", vec!["tomori".into()]),
    ));
    refused(block_on(backend.delete_session("s1")));
    refused(block_on(backend.delete_worker("s1", "w1", false)));
    let server = tomori_backend::ToolServer::new("tomori", "http://127.0.0.1:9/mcp");
    refused(block_on(backend.register_tools("/tmp", &server)));
    refused(block_on(backend.unregister_tools("/tmp", "tomori")));
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
        kind(event) == "reconnecting"
    });
    let held = {
        let session = crate::OpenCodeSession::new(id.clone(), events.clone(), commands.clone());
        in_thread(move || block_on(session.set_instruction("tomori-environment", Some("The sky."))))
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
        kind(event) == "resynchronized"
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

/// A session's own title is read with it, and a rename of it, but of no
/// other session, is reported as its new title.
#[test]
fn a_session_s_title_is_read_with_it_and_its_renames_are_reported() {
    let shared = Arc::new(FakeShared::default());
    let service = FakeService::start(shared.clone(), "secret");
    let (commands, connection) = connect_to(&Arc::new(Mutex::new(service.address)));
    shared.update(|state| {
        state
            .titles
            .insert("s1".into(), "Tomori opencode support".into());
    });
    let (id, events) = create_session(&commands);
    let snapshot = read_snapshot(&commands, &id, &events).unwrap();
    assert_eq!(snapshot.title.as_deref(), Some("Tomori opencode support"));

    let watched = watch(&events);
    service.send(
        json!({"type":"session.renamed","data":{"sessionID":"other","title":"Not this one"}}),
    );
    service.send(json!({"type":"session.renamed","data":{"sessionID":"s1","title":"Tomori development from the headset"}}));
    let renamed = Event::TitleChanged {
        title: "Tomori development from the headset".into(),
    };
    let seen = until(&watched, "the rename", |event| *event == renamed);
    // A rename of another session reports nothing.
    assert_eq!(seen, [renamed]);

    drop(commands);
    join_within(connection, Duration::from_secs(5));
    shared.update(|state| state.stopping = true);
    drop(service);
}

/// A session's directory is read with it, and where it is now is said by
/// its record when asked; a move of it, but of no other session, is
/// reported as its new directory, whose agents it lists from then on; and
/// reading it again after a move missed says where it is now.
#[test]
fn a_session_s_directory_is_read_with_it_and_its_moves_are_reported() {
    let shared = Arc::new(FakeShared::default());
    let service = FakeService::start(shared.clone(), "secret");
    let (commands, connection) = connect_to(&Arc::new(Mutex::new(service.address)));
    let (id, events) = create_session(&commands);
    let snapshot = read_snapshot(&commands, &id, &events).unwrap();
    assert_eq!(snapshot.directory.as_deref(), Some("/tmp"));
    let locate = |id: &str| {
        let id = id.to_owned();
        block_on(commands.reply_with(|reply| Command::Locate { id, reply }))
    };
    assert_eq!(locate(&id), Ok(Some("/tmp".to_owned())));

    let watched = watch(&events);
    service.send(json!({"type":"session.moved","data":{"sessionID":"other","location":{"directory":"/elsewhere"}}}));
    service.send(json!({"type":"session.moved","data":{"sessionID":"s1","location":{"directory":"/work/moved"},"projectID":"p","subpath":""}}));
    let moved = Event::DirectoryChanged {
        directory: "/work/moved".into(),
    };
    let seen = until(&watched, "the move", |event| *event == moved);
    assert_eq!(seen, [moved]);

    // Its record moves with it, and so does what reading it says.
    shared.update(|state| {
        let mut record = fake_record("s1");
        record["location"]["directory"] = json!("/work/again");
        state.records.insert("s1".into(), record);
    });
    assert_eq!(locate(&id), Ok(Some("/work/again".to_owned())));
    let snapshot = read_snapshot(&commands, &id, &events).unwrap();
    assert_eq!(snapshot.directory.as_deref(), Some("/work/again"));
    // A record without a directory says it cannot tell, and a session that is gone is not found.
    shared.update(|state| {
        let mut record = fake_record("s1");
        record["location"] = json!({});
        state.records.insert("s1".into(), record);
    });
    assert_eq!(locate(&id), Ok(None));
    assert_eq!(
        locate("missing").map_err(|error| error.kind),
        Err(tomori_backend::ErrorKind::NotFound)
    );

    drop(commands);
    join_within(connection, Duration::from_secs(5));
    shared.update(|state| state.stopping = true);
    drop(service);
}

#[test]
fn a_moved_session_lists_the_agents_of_where_it_is_now() {
    let mut registrations = HashMap::new();
    let _events = insert_registration(&mut registrations, "s".into(), "/old", Modes::default());
    let _other = insert_registration(&mut registrations, "other".into(), "/old", Modes::default());
    dispatch_events(
        &json!({"type":"session.moved","data":{"sessionID":"s","location":{"directory":"/new"}}})
            .to_string(),
        &mut registrations,
    );
    assert_eq!(registrations["s"].directory, "/new");
    assert_eq!(registrations["other"].directory, "/old");
    // A move naming no directory leaves it where it was, and a snapshot naming one moves it.
    dispatch_events(
        &json!({"type":"session.moved","data":{"sessionID":"s","location":{}}}).to_string(),
        &mut registrations,
    );
    assert_eq!(registrations["s"].directory, "/new");
    let registration = registrations.get_mut("s").unwrap();
    registration.follow(&Snapshot::default());
    assert_eq!(registration.directory, "/new");
    registration.follow(&Snapshot {
        directory: Some("/newer".into()),
        ..Snapshot::default()
    });
    assert_eq!(registration.directory, "/newer");
}
