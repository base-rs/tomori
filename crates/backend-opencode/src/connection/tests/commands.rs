// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Commands against a simulated service: a stalled one holds up only its own session, within the limits on what waits.

use super::*;

/// [`BRISK`], with an active-session lookup long enough for the simulated
/// service to answer one it delays well within it.
const PATIENT: Timing = Timing {
    snapshot: Duration::from_secs(2),
    active_lookup: Duration::from_secs(1),
    ..BRISK
};

/// How long the simulated service delays an active-session lookup that
/// still answers in time.
const ACTIVE_DELAY: Duration = Duration::from_millis(500);

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
                        let _ = reader.read_line(&mut line);
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
                                    // A stream nobody reads any more is
                                    // written to until the service stops.
                                    Ok(event) => {
                                        let data = format!("data: {event}\n\n");
                                        let _ = write!(socket, "{:x}\r\n{}\r\n", data.len(), data)
                                            .and_then(|_| socket.flush());
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
                                    std::thread::sleep(ACTIVE_DELAY);
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
                                json!({"data":messages,"cursor":{"next":"older-page"}}).to_string()
                            } else if path
                                == "/api/session/fast/message?limit=100&cursor=next%2Bpage"
                            {
                                r#"{"data":[{"id":"first","type":"user","text":"First"}],"cursor":{"next":null}}"#.to_owned()
                            } else if path.starts_with("/api/session/duplicate/message?") {
                                r#"{"data":[{"id":"same","type":"user","text":"a"},{"id":"same","type":"user","text":"b"}],"cursor":{"next":null}}"#.to_owned()
                            } else if path.starts_with("/api/session/bad-message/message?") {
                                r#"{"data":[{"id":"message"}],"cursor":{"next":null}}"#.to_owned()
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
                                unreachable!("unexpected path: {path}");
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
                Err(error) => {
                    assert_eq!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock,
                        "accept: {error}"
                    );
                    std::thread::sleep(Duration::from_millis(5))
                }
            }
        }
    });
    let (commands, receiver) = Commands::new();
    let resolved = crate::endpoint::resolve(Some(&format!("http://{address}")));
    let connection = std::thread::spawn(move || {
        run_thread(Box::new(move || Ok(resolved.clone())), receiver, PATIENT)
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
    // deadline; serial leaf traversal would take six seconds.
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
    std::iter::from_fn(|| stale_events.try_recv().ok()).for_each(drop);
    delay_active.store(true, Ordering::Release);
    delayed_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("the slow active lookup did not start");
    let false_busy_wait = first_of(&stale_events, "busy");
    let Err(mpsc::RecvTimeoutError::Timeout) =
        false_busy_wait.recv_timeout(ACTIVE_DELAY + Duration::from_millis(700))
    else {
        unreachable!("expected Err(mpsc::RecvTimeoutError::Timeout)");
    };
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
    let busy_wait = first_of(&fast_events, "busy");
    event_tx
        .send(json!({"type":"session.execution.started","data":{"sessionID":"fast"}}).to_string())
        .unwrap();
    busy_wait
        .recv_timeout(Duration::from_secs(2))
        .expect("the busy event was not delivered");
    let idle_wait = first_of(&fast_events, "idle");
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
    let received = first_of(&fast_events, "workers_changed")
        .recv_timeout(Duration::from_secs(2))
        .expect("the other space was blocked");
    let Event::WorkersChanged { workers } = received else {
        unreachable!("expected Event::WorkersChanged {{ .. }}");
    };
    assert!(workers.iter().any(|worker| worker.id == "child"));
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
    let closed_wait = first_of(&slow_events, "closed");
    closed_wait
        .recv_timeout(Duration::from_secs(2))
        .expect("the closed registration was not reported");
    stall_reconcile.store(false, Ordering::Release);
    event_tx
        .send(json!({"type":"session.deleted","data":{"sessionID":"child"}}).to_string())
        .unwrap();
    let deleted = first_of(&fast_events, "workers_changed")
        .recv_timeout(Duration::from_secs(2))
        .expect("the deletion was blocked");
    assert_eq!(
        deleted,
        Event::WorkersChanged {
            workers: Vec::new()
        }
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
        assert!(matches!(result, Err(error) if error.kind == tomori_backend::ErrorKind::Closed));
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
            .recv_timeout(PATIENT.snapshot + Duration::from_secs(2))
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

/// The answer on `answer`, failing the test rather than waiting forever.
fn answered<T>(answer: &async_channel::Receiver<T>) -> T {
    // Waits at least once, however soon the answer comes, so that how the
    // race went changes no coverage.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut value = None;
    while value.is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "no answer within 5 seconds"
        );
        std::thread::sleep(Duration::from_millis(1));
        value = answer.try_recv().ok();
    }
    value.unwrap()
}

/// Send a prompt to `id` on another thread.
fn prompt_in_thread(
    commands: &Commands,
    id: &str,
    events: &RegistrationEvents,
    key: &str,
) -> std::sync::mpsc::Receiver<(tomori_backend::Result<()>, std::time::Instant)> {
    let commands = commands.clone();
    let id = id.to_owned();
    let identity = events.identity.clone();
    let key = key.to_owned();
    in_thread(move || {
        block_on(commands.reply_with(|reply| Command::Prompt {
            id,
            identity,
            key,
            text: "hello".into(),
            reply,
        }))
    })
}

/// A command OpenCode is slow to answer holds up only the later commands
/// of its own session, which follow it in order; other sessions' events,
/// reads and commands, and new sessions, go on meanwhile, and closing the
/// session fails its waiting commands at once.
#[test]
fn a_stalled_command_holds_up_only_its_own_session() {
    let shared = Arc::new(FakeShared::default());
    let service = FakeService::start(shared.clone(), "secret");
    let (commands, connection) = connect_to(&Arc::new(Mutex::new(service.address)));
    let (stalled, stalled_events) = open_session(&commands);
    let (other, other_events) = open_session(&commands);
    let other_watch = watch(&other_events);
    shared.update(|state| {
        state.stalled_prompts.insert(stalled.clone());
    });
    let first = prompt_in_thread(&commands, &stalled, &stalled_events, "first");
    wait_for(&shared, "the first prompt was never sent", |state| {
        state.prompts.len() == 1
    });
    let second = prompt_in_thread(&commands, &stalled, &stalled_events, "second");

    service.send(json!({"type":"session.execution.started","data":{"sessionID":other}}));
    until(&other_watch, "the other session's event", |event| {
        *event == Event::Busy
    });
    let reading = {
        let commands = commands.clone();
        let other = other.clone();
        let events = other_events.clone();
        in_thread(move || read_snapshot(&commands, &other, &events))
    };
    let (snapshot, _) = reading
        .recv_timeout(Duration::from_secs(2))
        .expect("the other session's read was held up");
    snapshot.unwrap();
    let interrupting = {
        let commands = commands.clone();
        let other = other.clone();
        let identity = other_events.identity.clone();
        in_thread(move || {
            block_on(commands.reply_with(|reply| Command::Interrupt {
                id: other,
                identity,
                reply,
            }))
        })
    };
    let (interrupted, _) = interrupting
        .recv_timeout(Duration::from_secs(2))
        .expect("the other session's command was held up");
    interrupted.unwrap();
    let creating = {
        let commands = commands.clone();
        in_thread(move || open_session(&commands).0)
    };
    creating
        .recv_timeout(Duration::from_secs(2))
        .expect("creating a session was held up");
    // The stalled session's second prompt waits for its first.
    assert_eq!(shared.state.lock().unwrap().prompts.len(), 1);

    shared.update(|state| state.stalled_prompts.clear());
    let (result, _) = first.recv_timeout(Duration::from_secs(3)).unwrap();
    result.unwrap();
    let (result, _) = second.recv_timeout(Duration::from_secs(3)).unwrap();
    result.unwrap();
    assert_eq!(
        shared.state.lock().unwrap().prompts,
        [
            (stalled.clone(), "msg_first".to_owned()),
            (stalled.clone(), "msg_second".to_owned())
        ]
    );

    // Closing a session whose command is stalled fails its waiting ones
    // at once, not when the stalled one is answered.
    shared.update(|state| {
        state.stalled_prompts.insert(stalled.clone());
    });
    let third = prompt_in_thread(&commands, &stalled, &stalled_events, "third");
    wait_for(&shared, "the third prompt was never sent", |state| {
        state.prompts.len() == 3
    });
    let fourth = prompt_in_thread(&commands, &stalled, &stalled_events, "fourth");
    std::thread::sleep(Duration::from_millis(100));
    commands
        .tx()
        .try_send(Command::Close {
            id: stalled.clone(),
            identity: stalled_events.identity.clone(),
        })
        .unwrap();
    let (result, _) = fourth
        .recv_timeout(Duration::from_secs(2))
        .expect("a closed session's waiting command was held up");
    assert_eq!(result.unwrap_err().kind, tomori_backend::ErrorKind::Closed);
    assert_eq!(shared.state.lock().unwrap().prompts.len(), 3);
    shared.update(|state| state.stalled_prompts.clear());
    let (result, _) = third.recv_timeout(Duration::from_secs(3)).unwrap();
    result.unwrap();

    drop(commands);
    join_within(connection, Duration::from_secs(5));
    shared.update(|state| state.stopping = true);
    drop(service);
}

/// At most [`MAX_REQUESTS`] requests are in flight, so further commands,
/// of any session, wait; and at most [`MAX_WAITING`] wait, so further
/// ones are refused at once.
#[test]
fn commands_beyond_the_limits_wait_or_are_refused() {
    let shared = Arc::new(FakeShared::default());
    let service = FakeService::start(shared.clone(), "secret");
    let (commands, connection) = connect_to(&Arc::new(Mutex::new(service.address)));
    let sessions: Vec<_> = (0..=MAX_REQUESTS)
        .map(|_| open_session(&commands))
        .collect();
    shared.update(|state| {
        for (id, _) in &sessions {
            state.stalled_prompts.insert(id.clone());
        }
    });
    let prompts: Vec<_> = sessions
        .iter()
        .map(|(id, events)| prompt_in_thread(&commands, id, events, "stalled"))
        .collect();
    wait_for(&shared, "the prompts were never sent", |state| {
        state.prompts.len() == MAX_REQUESTS
    });
    let listing = {
        let commands = commands.clone();
        in_thread(move || {
            block_on(commands.reply_with(|reply| Command::Sessions {
                directory: "/tmp".into(),
                reply,
            }))
        })
    };
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(shared.state.lock().unwrap().prompts.len(), MAX_REQUESTS);
    assert!(listing.try_recv().is_err(), "the listing did not wait");

    // One prompt and the listing wait; fill the rest of the places with
    // the first session's commands, then one more is refused.
    let (first, first_events) = &sessions[0];
    let mut queued = Vec::new();
    for place in 2..MAX_WAITING {
        let (reply, answer) = async_channel::bounded(1);
        let id = first.clone();
        let identity = first_events.identity.clone();
        let command = match place % 5 {
            0 => Command::Interrupt {
                id,
                identity,
                reply,
            },
            1 => Command::Decide {
                id,
                identity,
                request: "p".into(),
                decision: "once".into(),
                reply,
            },
            2 => Command::Answer {
                id,
                identity,
                form: "f".into(),
                answer: json!({}),
                reply,
            },
            3 => Command::SetMode {
                id,
                identity,
                mode: "plan".into(),
                reply,
            },
            _ => Command::Instruction {
                id,
                identity,
                key: "k".into(),
                value: None,
                reply,
            },
        };
        block_on(commands.tx().send(command)).unwrap();
        queued.push(answer);
    }
    let (reply, refused) = async_channel::bounded(1);
    block_on(commands.tx().send(Command::Sessions {
        directory: "/tmp".into(),
        reply,
    }))
    .unwrap();
    let refusal = answered(&refused).unwrap_err();
    assert_eq!(refusal.message, TOO_MANY_WAITING);
    // Closing the first session fails its waiting commands at once.
    commands
        .tx()
        .try_send(Command::Close {
            id: first.clone(),
            identity: first_events.identity.clone(),
        })
        .unwrap();
    for answer in queued {
        let error = answered(&answer).unwrap_err();
        assert_eq!(error.kind, tomori_backend::ErrorKind::Closed);
    }

    // Shutting down waits neither for the stalled requests nor for the
    // commands behind them, which fail.
    commands.shutdown();
    let (result, _) = listing.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(result.unwrap_err().message, SHUTTING_DOWN);
    for prompt in prompts {
        let (result, _) = prompt.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(result.is_err());
    }
    // The waiting listing was refused as the connection stopped, so a
    // command sent now, as a session may send one until it is dropped, is
    // answered at once there, whichever way the connection noticed the
    // shutdown.
    let (reply, late) = async_channel::bounded(1);
    block_on(commands.tx().send(Command::Sessions {
        directory: "/tmp".into(),
        reply,
    }))
    .unwrap();
    assert_eq!(answered(&late).unwrap_err().message, SHUTTING_DOWN);
    drop(commands);
    join_within(connection, Duration::from_secs(5));
    shared.update(|state| state.stopping = true);
    drop(service);
}

/// Two attachments of one session made at once: the second waits for the
/// first, and is refused, rather than replacing it.
#[test]
fn attaching_one_session_twice_at_once_binds_it_once() {
    let shared = Arc::new(FakeShared::default());
    shared.update(|state| {
        state.sessions.insert("twice".into(), Vec::new());
        state.stalled_records.insert("twice".into());
    });
    let service = FakeService::start(shared.clone(), "secret");
    let (commands, connection) = connect_to(&Arc::new(Mutex::new(service.address)));
    let attach = || {
        let commands = commands.clone();
        in_thread(move || {
            block_on(commands.reply_with(|reply| Command::Attach {
                id: "twice".into(),
                directory: "/tmp".into(),
                reply,
            }))
        })
    };
    let first = attach();
    wait_for(&shared, "the first attachment never started", |state| {
        state.record_requests == 1
    });
    let second = attach();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(shared.state.lock().unwrap().record_requests, 1);
    shared.update(|state| {
        state.stalled_records.clear();
    });
    // The first attachment's events are kept until the second is
    // answered: a registration whose events are dropped is abandoned,
    // and another attachment may then replace it.
    let (first, _) = first.recv_timeout(Duration::from_secs(3)).unwrap();
    let (second, _) = second.recv_timeout(Duration::from_secs(3)).unwrap();
    let events = first.unwrap();
    assert_eq!(second.map(|_| ()).unwrap_err().message, ALREADY_ATTACHED);
    drop(events);
    drop(commands);
    join_within(connection, Duration::from_secs(5));
    shared.update(|state| state.stopping = true);
    drop(service);
}

/// Once the event stream is lost, a request in flight is still answered;
/// commands that arrive meanwhile wait for the next connection, except a
/// close, which is taken at once.
#[test]
fn commands_after_the_stream_is_lost_wait_for_the_next_connection() {
    let shared = Arc::new(FakeShared::default());
    let mut first = FakeService::start(shared.clone(), "secret");
    let address = Arc::new(Mutex::new(first.address));
    let (commands, connection) = connect_to(&address);
    let (id, events) = open_session(&commands);
    let watched = watch(&events);
    let (closing, closing_events) = open_session(&commands);
    let closing_watch = watch(&closing_events);
    shared.update(|state| {
        state.stalled_prompts.insert(id.clone());
    });
    let in_flight = prompt_in_thread(&commands, &id, &events, "in-flight");
    wait_for(&shared, "the prompt was never sent", |state| {
        state.prompts.len() == 1
    });
    // Waiting for the first, so not yet started when the stream ends.
    let waiting = prompt_in_thread(&commands, &id, &events, "waiting");
    std::thread::sleep(Duration::from_millis(100));
    let requests = shared.state.lock().unwrap().event_requests;
    shared.update(|state| state.hold_events = true);
    first.end_streams();
    first.stop();
    // Each session is told at once, while the prompt is still in flight.
    until(&watched, "the disconnection notice", |event| {
        kind(event) == "reconnecting"
    });
    // Sent in order from here, so that the connection takes each before
    // the next, however loaded the machine is.
    let listing = |commands: &Commands| {
        let (reply, answer) = async_channel::bounded(1);
        block_on(commands.tx().send(Command::Sessions {
            directory: "/tmp".into(),
            reply,
        }))
        .unwrap();
        answer
    };
    let held_listing = listing(&commands);
    let (reply, closing_prompt) = async_channel::bounded(1);
    block_on(commands.tx().send(Command::Prompt {
        id: closing.clone(),
        identity: closing_events.identity.clone(),
        key: "closed".into(),
        text: "hello".into(),
        reply,
    }))
    .unwrap();
    commands
        .tx()
        .try_send(Command::Close {
            id: closing.clone(),
            identity: closing_events.identity.clone(),
        })
        .unwrap();
    until(&closing_watch, "the closure", |event| {
        *event == Event::Closed { reason: None }
    });
    // Its command waiting for the next connection fails at once.
    let result = answered(&closing_prompt);
    assert_eq!(result.unwrap_err().kind, tomori_backend::ErrorKind::Closed);
    assert!(held_listing.is_empty(), "the listing did not wait");
    // As many wait as while no connection is open, the listing and the
    // waiting prompt among them; more are refused. The refusal comes once
    // those before it are held.
    let held: Vec<_> = (2..MAX_HELD).map(|_| listing(&commands)).collect();
    let refused = answered(&listing(&commands));
    assert_eq!(refused.unwrap_err().message, TOO_MANY_HELD);
    assert!(held.iter().all(async_channel::Receiver::is_empty));
    // OpenCode is back before the prompt is answered, so the next attempt,
    // which the held listing waits for, reaches it.
    let second = FakeService::start(shared.clone(), "secret");
    *address.lock().unwrap() = second.address;
    shared.update(|state| state.stalled_prompts.clear());
    let (result, _) = in_flight.recv_timeout(Duration::from_secs(3)).unwrap();
    result.unwrap();
    wait_for(
        &shared,
        "the event stream was never requested again",
        |state| state.event_requests > requests,
    );
    shared.update(|state| state.hold_events = false);
    answered(&held_listing).unwrap();
    let (result, _) = waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    result.unwrap();
    for answer in held {
        answered(&answer).unwrap();
    }
    assert_eq!(
        shared.state.lock().unwrap().prompts,
        [
            (id.clone(), "msg_in-flight".to_owned()),
            (id.clone(), "msg_waiting".to_owned())
        ]
    );
    drop(commands);
    join_within(connection, Duration::from_secs(5));
    shared.update(|state| state.stopping = true);
    drop(second);
}

/// While no connection is open, closing a session fails its commands
/// waiting for one at once.
#[test]
fn closing_a_session_while_disconnected_fails_its_held_commands() {
    let shared = Arc::new(FakeShared::default());
    let service = FakeService::start(shared.clone(), "secret");
    let (commands, connection) = connect_to(&Arc::new(Mutex::new(service.address)));
    let (id, events) = open_session(&commands);
    let watched = watch(&events);
    // The stream ends, and the next one is slow to open, so commands wait.
    let requests = shared.state.lock().unwrap().event_requests;
    shared.update(|state| state.hold_events = true);
    service.end_streams();
    until(&watched, "the disconnection notice", |event| {
        kind(event) == "reconnecting"
    });
    wait_for(
        &shared,
        "the event stream was never requested again",
        |state| state.event_requests > requests,
    );
    let held = prompt_in_thread(&commands, &id, &events, "held");
    std::thread::sleep(Duration::from_millis(100));
    commands
        .tx()
        .try_send(Command::Close {
            id: id.clone(),
            identity: events.identity.clone(),
        })
        .unwrap();
    let (result, _) = held
        .recv_timeout(Duration::from_secs(1))
        .expect("the closed session's held command was not refused");
    assert_eq!(result.unwrap_err().kind, tomori_backend::ErrorKind::Closed);
    until(&watched, "the closure", |event| {
        *event == Event::Closed { reason: None }
    });
    shared.update(|state| state.hold_events = false);
    drop(commands);
    join_within(connection, Duration::from_secs(5));
    shared.update(|state| state.stopping = true);
    drop(service);
}

/// A request OpenCode never answers after the event stream is lost is
/// abandoned once the grace period has passed, so the backend reconnects
/// and the session is read again.
#[test]
fn a_request_unanswered_after_the_stream_is_lost_is_abandoned() {
    let shared = Arc::new(FakeShared::default());
    let mut first = FakeService::start(shared.clone(), "secret");
    let address = Arc::new(Mutex::new(first.address));
    let (commands, connection) = connect_to(&address);
    let (id, events) = open_session(&commands);
    let watched = watch(&events);
    shared.update(|state| {
        state.stalled_prompts.insert(id.clone());
    });
    let stalled = prompt_in_thread(&commands, &id, &events, "stalled");
    wait_for(&shared, "the prompt was never sent", |state| {
        state.prompts.len() == 1
    });
    let lost = std::time::Instant::now();
    first.end_streams();
    first.stop();
    let second = FakeService::start(shared.clone(), "secret");
    *address.lock().unwrap() = second.address;
    until(&watched, "the disconnection notice", |event| {
        kind(event) == "reconnecting"
    });
    let (result, answered_at) = stalled
        .recv_timeout(BRISK.in_flight_grace + Duration::from_secs(2))
        .expect("the stalled prompt was never abandoned");
    assert!(result.is_err());
    assert!(answered_at.duration_since(lost) >= BRISK.in_flight_grace - Duration::from_millis(500));
    until(&watched, "the resynchronization", |event| {
        kind(event) == "resynchronized"
    });
    shared.update(|state| state.stalled_prompts.clear());
    let (result, _) = prompt_in_thread(&commands, &id, &events, "after")
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    result.unwrap();
    drop(commands);
    join_within(connection, Duration::from_secs(5));
    shared.update(|state| state.stopping = true);
    drop(second);
}
