// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tomori's MCP endpoint and its registration with backends. See `design/mcp.md`.
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tomori_backend::{Event, Snapshot, ToolServer, Worker, WorkerState};
use tomori_protocol::{ClientMessage, QueueState, Renamer, ServerMessage, SpeechSetting};
use tomori_server::Server;
use tomori_server::fake::{FAKE_BACKEND_NAME, FakeBackend, FakeSession};
use tomori_server::kept::SpaceStore;
use tomori_server::test_support::{TIMEOUT, TestClient, TestServer, TextTranscriber, eventually};

/// Every log record of this test process.
struct Recorder(std::sync::Mutex<Vec<String>>);

impl log::Log for Recorder {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        self.0.lock().unwrap().push(record.args().to_string());
    }

    fn flush(&self) {}
}

static LOG: Recorder = Recorder(std::sync::Mutex::new(Vec::new()));

/// Everything logged so far, recording at every level from the first call on.
fn logged() -> Vec<String> {
    static RECORDING: std::sync::Once = std::sync::Once::new();
    RECORDING.call_once(|| {
        log::set_logger(&LOG).expect("only this recorder logs here");
        log::set_max_level(log::LevelFilter::Trace);
    });
    log::Log::flush(&LOG);
    assert!(log::Log::enabled(&LOG, &log::Metadata::builder().build()));
    LOG.0.lock().unwrap().clone()
}

/// A server over `backend` serving Tomori's tools.
fn start(backend: &FakeBackend) -> TestServer {
    logged();
    TestServer::with_tools(backend.clone(), |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default()))
    })
}

/// Create a space titled `title` in `/tmp`, and wait for it.
fn create(client: &mut TestClient, title: &str) -> String {
    client.send(&ClientMessage::CreateSpace {
        title: title.to_owned(),
        backend: FAKE_BACKEND_NAME.to_owned(),
        directory: "/tmp".to_owned(),
        session: None,
        environment: None,
        instructions: None,
    });
    client.space(|state| state.title == title).id
}

/// The registrations `backend` has been asked for, once there are `count`.
fn registrations(backend: &FakeBackend, count: usize) -> Vec<(String, Option<ToolServer>)> {
    eventually(|| {
        let tools = backend.tools();
        (tools.len() >= count).then_some(tools)
    })
}

/// The bearer token the server registered itself with.
fn token(server: &ToolServer) -> String {
    let (name, value) = &server.headers[0];
    assert_eq!(name, "Authorization");
    value.strip_prefix("Bearer ").unwrap().to_owned()
}

/// The identifier of `session`, as OpenCode names it in a tool call's `_meta`.
fn id(session: &FakeSession) -> String {
    tomori_backend::Session::id(session)
}

/// A response's status, head and body.
struct Answer {
    status: u16,
    head: String,
    body: String,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap()
    }
}

/// Make one request of the MCP endpoint at `server`, with `fields` and `body`.
fn request(
    server: &TestServer,
    method: &str,
    path: &str,
    fields: &[(&str, &str)],
    body: &str,
) -> Answer {
    let mut stream = TcpStream::connect(server.tools.unwrap()).unwrap();
    let fields: String = fields
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{fields}\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    Answer {
        status: head[9..12].parse().unwrap(),
        head: head.to_owned(),
        body: body.to_owned(),
    }
}

/// Post one JSON-RPC message with `token`.
fn post(server: &TestServer, token: &str, message: &Value) -> Answer {
    request(
        server,
        "POST",
        "/mcp",
        &[("Authorization", &format!("Bearer {token}"))],
        &message.to_string(),
    )
}

/// Call the tool `name` with `arguments`, and return its result, as OpenCode does for the agent of `session`, which it names in `_meta`, or with no `_meta` at all.
fn call(
    server: &TestServer,
    token: &str,
    session: Option<&str>,
    name: &str,
    arguments: Value,
) -> Value {
    let mut params = json!({ "name": name, "arguments": arguments });
    if let Some(session) = session {
        params["_meta"] = json!({ "ai.opencode/sessionID": session, "progressToken": 7 });
    }
    let answer = post(
        server,
        token,
        &json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": params }),
    );
    assert_eq!(answer.status, 200, "{}", answer.body);
    answer.json()["result"].clone()
}

/// The text of a tool's failure.
fn failure(result: &Value) -> &str {
    assert_eq!(result["isError"], true, "{result}");
    result["content"][0]["text"].as_str().unwrap()
}

/// A state directory of its own, removed when dropped.
struct State(PathBuf);

impl State {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("tomori-test-mcp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for State {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn tools_are_registered_where_spaces_are_open_and_removed_when_none_are() {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = server.connect("ada", "Ada");
    create(&mut ada, "First");
    let registered = registrations(&backend, 1);
    let (directory, Some(tools)) = &registered[0] else {
        unreachable!("the first space registers the tools");
    };
    assert_eq!(directory, "/tmp");
    assert_eq!(tools.name, "tomori");
    assert_eq!(tools.url, format!("http://{}/mcp", server.tools.unwrap()));
    // The token is long, random and made for this run.
    assert!(token(tools).len() > 64);

    // A second space in the directory registers them again, as does reading a session again, since the agent may have restarted and forgotten them.
    create(&mut ada, "Second");
    registrations(&backend, 2);
    let sessions = backend.started();
    sessions[0].emit(Event::Resynchronized {
        snapshot: Snapshot::default(),
    });
    let again = registrations(&backend, 3);
    assert!(
        again
            .iter()
            .all(|(_, server)| server.as_ref() == Some(tools))
    );

    // While a space is open there, a closed one leaves them; once none is, they are removed. A failure is said, and changes nothing else.
    sessions[0].emit(Event::Closed { reason: None });
    let kept = registrations(&backend, 4);
    assert_eq!(kept[3], ("/tmp".to_owned(), Some(tools.clone())));
    backend.fail_tools(true);
    sessions[1].emit(Event::Closed { reason: None });
    let removed = registrations(&backend, 5);
    assert_eq!(removed[4], ("/tmp".to_owned(), None));
    eventually(|| {
        logged()
            .iter()
            .any(|line| {
                line == "Tomori's tools could not be removed for fake in /tmp: the tools went away"
            })
            .then_some(())
    });
    assert!(
        logged()
            .iter()
            .any(|line| line == "Tomori's tools are registered for fake in /tmp.")
    );

    // A refused registration is said without the token, even when the agent repeats it.
    create(&mut ada, "Third");
    let refused = registrations(&backend, 6);
    assert_eq!(refused[5].1.as_ref(), Some(tools));
    let secret = token(tools);
    let warning = eventually(|| {
        logged()
            .into_iter()
            .find(|line| line.starts_with("Tomori's tools could not be registered"))
    });
    assert_eq!(
        warning,
        "Tomori's tools could not be registered for fake in /tmp: the tools went away, refusing Authorization: (secret)"
    );
    assert!(logged().iter().all(|line| !line.contains(&secret)));
}

#[test]
fn tools_are_removed_once_the_last_space_open_in_a_directory_is_deleted() {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = server.connect("ada", "Ada");
    let first = create(&mut ada, "First");
    let second = create(&mut ada, "Second");
    let registered = registrations(&backend, 2);
    let tools = registered[0]
        .1
        .clone()
        .expect("the first space registers the tools");
    // While another space is open there, a deleted one leaves them; once none is, they are removed, as when the last
    // closes.
    for (space, count) in [(&second, 3), (&first, 4)] {
        ada.send(&ClientMessage::Join {
            space: space.clone(),
        });
        ada.space(|state| &state.id == space);
        ada.send(&ClientMessage::DeleteSpace {
            space: space.clone(),
            session: false,
        });
        ada.wait_for(|message| {
            matches!(
                message,
                tomori_protocol::ServerMessage::Left { deleted: Some(_) }
            )
            .then_some(())
        });
        registrations(&backend, count);
    }
    let tools_now = backend.tools();
    assert_eq!(tools_now[2], ("/tmp".to_owned(), Some(tools)));
    assert_eq!(tools_now[3], ("/tmp".to_owned(), None));
}

#[test]
fn a_backend_without_tools_registers_none_and_its_agent_is_not_told_of_them() {
    let backend = FakeBackend::new();
    backend.take_no_tools();
    let server = start(&backend);
    let mut ada = server.connect("ada", "Ada");
    create(&mut ada, "Plain");
    let session = backend.started()[0].clone();
    eventually(|| session.instructions().contains_key("tomori").then_some(()));
    assert!(!session.instructions().contains_key("tomori-tools"));
    assert!(backend.tools().is_empty());
}

#[test]
fn the_endpoint_answers_only_its_token_on_its_path_and_method() {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = server.connect("ada", "Ada");
    create(&mut ada, "Doors");
    let registered = registrations(&backend, 1);
    let token = token(registered[0].1.as_ref().unwrap());
    let ping = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string();
    let bearer = format!("Bearer {token}");

    // Without the token, or with another, nothing is answered.
    for fields in [
        &[][..],
        &[("Authorization", "Bearer tomori_wrong")],
        &[("Authorization", token.as_str())],
    ] {
        let refused = request(&server, "POST", "/mcp", fields, &ping);
        assert_eq!(refused.status, 401, "{fields:?}");
        assert!(
            refused.head.contains("WWW-Authenticate: Bearer"),
            "{}",
            refused.head
        );
    }
    // Not from web pages, not elsewhere, only by POST, only in a revision it serves, and only small.
    let authorized = [("Authorization", bearer.as_str())];
    let from_page = [authorized[0], ("Origin", "http://example.com")];
    assert_eq!(
        request(&server, "POST", "/mcp", &from_page, &ping).status,
        403
    );
    assert_eq!(
        request(&server, "POST", "/connect", &authorized, &ping).status,
        404
    );
    let streamed = request(&server, "GET", "/mcp", &authorized, "");
    assert_eq!(streamed.status, 405);
    assert!(streamed.head.contains("Allow: POST"), "{}", streamed.head);
    assert_eq!(
        request(&server, "DELETE", "/mcp", &authorized, "").status,
        405
    );
    let old = [authorized[0], ("MCP-Protocol-Version", "2024-11-05")];
    let refused = request(&server, "POST", "/mcp", &old, &ping);
    assert_eq!(refused.status, 400);
    assert_eq!(
        refused.body,
        "MCP protocol version 2024-11-05 is not served."
    );
    let current = [authorized[0], ("MCP-Protocol-Version", "2025-11-25")];
    assert_eq!(
        request(&server, "POST", "/mcp?x=1", &current, &ping).status,
        200
    );
    let large = " ".repeat(65 * 1024);
    assert_eq!(
        request(&server, "POST", "/mcp", &authorized, &large).status,
        413
    );
}

#[test]
fn the_protocol_initialises_lists_tools_and_refuses_what_it_does_not_answer() {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = server.connect("ada", "Ada");
    create(&mut ada, "Protocol");
    let token = token(registrations(&backend, 1)[0].1.as_ref().unwrap());
    let rpc = |message: Value| post(&server, &token, &message).json();

    // The revision asked for is served if it can be, and otherwise the newest.
    for (asked, served) in [
        ("2025-06-18", "2025-06-18"),
        ("2025-11-25", "2025-11-25"),
        ("1999-01-01", "2025-11-25"),
    ] {
        let initialised = rpc(json!({
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {
                "protocolVersion": asked,
                "capabilities": {},
                "clientInfo": {"name": "opencode", "version": "2.2.5"},
            },
        }));
        assert_eq!(initialised["id"], 0);
        assert_eq!(initialised["result"]["protocolVersion"], served);
        assert_eq!(initialised["result"]["serverInfo"]["name"], "tomori");
        assert_eq!(
            initialised["result"]["capabilities"],
            json!({"tools": {"listChanged": false}})
        );
    }
    // Notifications and responses are accepted without an answer.
    let notified = post(
        &server,
        &token,
        &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    );
    assert_eq!((notified.status, notified.body.as_str()), (202, ""));
    for response in [
        json!({"jsonrpc": "2.0", "id": 9, "result": {}}),
        json!({"jsonrpc": "2.0", "id": 9, "error": {"code": -1, "message": "no"}}),
    ] {
        let responded = post(&server, &token, &response);
        assert_eq!((responded.status, responded.body.as_str()), (202, ""));
    }
    assert_eq!(
        rpc(json!({"jsonrpc": "2.0", "id": "p", "method": "ping"}))["result"],
        json!({})
    );

    // None opens or closes pairing, which only an owner or the operator does; see `design/security.md`.
    let listed = rpc(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "server",
            "spaces",
            "space",
            "presence",
            "panels",
            "update_panel",
            "move_panel",
            "set_speech",
            "set_environment",
            "rename_space",
            "pairing",
            "confirm_pairing",
            "decline_pairing",
            "workers",
            "remove_workers"
        ]
    );
    for tool in listed["result"]["tools"].as_array().unwrap() {
        assert_eq!(tool["inputSchema"]["type"], "object");
    }

    // What it does not answer is refused as JSON-RPC asks.
    let code = |answer: &Value| answer["error"]["code"].as_i64().unwrap();
    assert_eq!(
        code(&rpc(
            json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"})
        )),
        -32601
    );
    let unknown = rpc(json!({
        "jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": {"name": "delete_everything", "arguments": {}},
    }));
    assert_eq!(code(&unknown), -32602);
    assert_eq!(
        unknown["error"]["message"],
        "Tomori has no tool named \"delete_everything\"."
    );
    assert_eq!(code(&rpc(json!({"jsonrpc": "2.0", "id": 5}))), -32600);
    assert_eq!(
        code(&rpc(json!([{"jsonrpc": "2.0", "id": 6, "method": "ping"}]))),
        -32600
    );
    let garbled = request(
        &server,
        "POST",
        "/mcp",
        &[("Authorization", &format!("Bearer {token}"))],
        "{not json",
    );
    assert_eq!(garbled.status, 400);
    assert_eq!(code(&garbled.json()), -32700);
}

#[test]
fn space_tools_act_on_the_space_bound_to_the_calling_session() {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = server.connect("ada", "Ada");
    let first = create(&mut ada, "First");
    let token = token(registrations(&backend, 1)[0].1.as_ref().unwrap());
    let mut bob = server.connect("bob", "Bob");
    let second = create(&mut bob, "Second");
    let sessions = backend.started();
    let (one, two) = (id(&sessions[0]), id(&sessions[1]));
    // Its agent is told what the tools are and what they act on, and given no secret.
    let entry = eventually(|| sessions[0].instructions().get("tomori-tools").cloned());
    assert!(entry.contains("MCP server named tomori"), "{entry}");
    assert!(
        entry.contains("act on the space this session is bound to"),
        "{entry}"
    );
    assert!(!entry.contains("tomori_"), "{entry}");
    assert!(
        entry.contains("presence says where the people in this space are"),
        "{entry}"
    );

    // The global tools need no session, and say nothing secret.
    let described = call(&server, &token, None, "server", json!({}));
    let content = &described["structuredContent"];
    assert_eq!(content["name"], "tomori");
    assert_eq!(content["environments"][0]["id"], "sky");
    assert_eq!(described["content"][0]["text"], content.to_string());
    let listed = call(&server, &token, None, "spaces", json!({}))["structuredContent"].clone();
    assert_eq!(
        listed["spaces"][0],
        json!({"id": first, "title": "First", "environment": "sky", "activity": "idle", "participants": 1})
    );
    assert_eq!(listed["spaces"].as_array().unwrap().len(), 2);

    // A space tool acts for the session OpenCode names, and only for a session bound to a space: not for a sub-agent of one.
    sessions[0].emit(Event::WorkersChanged {
        workers: vec![Worker {
            id: "ses_child".into(),
            parent: Some(one.clone()),
            title: "Child".into(),
            state: WorkerState::Running,
        }],
    });
    ada.space(|state| state.workers.len() == 1);
    for tool in [
        "space",
        "presence",
        "panels",
        "update_panel",
        "move_panel",
        "set_speech",
        "set_environment",
        "rename_space",
    ] {
        let arguments = json!({"speech": "brief", "environment": "minimal", "panel": "panel-1", "place": "nearest", "title": "Elsewhere"});
        assert_eq!(
            failure(&call(&server, &token, None, tool, arguments.clone())),
            "Tomori's space tools act on the space of the agent session that calls them, and this call names no session.",
            "{tool}"
        );
        assert_eq!(
            failure(&call(&server, &token, Some(""), tool, arguments.clone())),
            "Tomori's space tools act on the space of the agent session that calls them, and this call names no session.",
            "{tool}"
        );
        assert!(
            failure(&call(
                &server,
                &token,
                Some("ses_other"),
                tool,
                arguments.clone()
            ))
            .starts_with("This session is not bound to a Tomori space"),
            "{tool}"
        );
        assert!(
            failure(&call(
                &server,
                &token,
                Some("ses_child"),
                tool,
                arguments.clone()
            ))
            .starts_with("This session is not bound to a Tomori space"),
            "{tool}"
        );
        assert_eq!(
            failure(&call(
                &server,
                &token,
                Some(&"s".repeat(257)),
                tool,
                arguments
            )),
            "This call's session identifier is too long.",
            "{tool}"
        );
    }
    // A session ID that is not text names no session.
    let numbered = post(
        &server,
        &token,
        &json!({"jsonrpc": "2.0", "id": 8, "method": "tools/call", "params": {
            "name": "space", "arguments": {}, "_meta": {"ai.opencode/sessionID": 1},
        }}),
    );
    assert!(failure(&numbered.json()["result"]).ends_with("and this call names no session."));

    // Each of the directory's sessions reads its own space, and nothing else.
    let read = call(&server, &token, Some(&one), "space", json!({}));
    let state = &read["structuredContent"];
    assert_eq!(state["id"], first);
    assert_eq!(state["title"], "First");
    assert_eq!(state["environment"], json!({"id": "sky", "label": "Sky"}));
    assert_eq!(state["speech"], "full");
    assert_eq!(
        state["participants"],
        json!([{"name": "Ada", "engagement": null}])
    );
    assert_eq!(state["visitors"], 0);
    assert_eq!(state["modes"], json!(["default", "plan"]));
    let other = call(&server, &token, Some(&two), "space", json!({}));
    assert_eq!(other["structuredContent"]["id"], second);
    assert_eq!(
        other["structuredContent"]["participants"],
        json!([{"name": "Bob", "engagement": null}])
    );

    // A session the agent names anew is followed; one bound to two open spaces is refused rather than guessed at.
    sessions[1].rename(&one);
    sessions[1].emit(Event::Busy);
    eventually(|| {
        call(&server, &token, Some(&two), "space", json!({}))["isError"]
            .as_bool()
            .filter(|refused| *refused)
    });
    for tool in [
        "space",
        "presence",
        "panels",
        "update_panel",
        "move_panel",
        "set_speech",
        "set_environment",
        "rename_space",
    ] {
        assert_eq!(
            failure(&call(
                &server,
                &token,
                Some(&one),
                tool,
                json!({"speech": "brief", "environment": "minimal", "panel": "panel-1", "place": "nearest", "title": "Elsewhere"})
            )),
            "This session is bound to more than one open Tomori space, so Tomori cannot tell which one to act on.",
            "{tool}"
        );
    }
}

#[test]
fn the_space_tool_counts_only_input_not_yet_sent_to_the_agent() {
    let backend = FakeBackend::manual();
    let server = start(&backend);
    let mut ada = server.connect("ada", "Ada");
    create(&mut ada, "Queue");
    let token = token(registrations(&backend, 1)[0].1.as_ref().unwrap());
    let session = backend.started().remove(0);
    let one = id(&session);
    let queued = || {
        call(&server, &token, Some(&one), "space", json!({}))["structuredContent"]["queued"].clone()
    };
    let has = |state: &tomori_protocol::SpaceState, text: &str, wanted: QueueState| {
        state
            .queue
            .iter()
            .any(|entry| entry.text == text && entry.state == wanted)
    };
    assert_eq!(queued(), 0);

    // Input delivered to the agent, whose turn is still running, is no longer queued, though the space still shows it.
    ada.say("u1", "first");
    ada.space(|state| has(state, "first", QueueState::Delivered));
    assert_eq!(queued(), 0);

    // Input spoken while the turn runs waits for it, and is counted until it is sent.
    ada.say("u2", "second");
    ada.space(|state| has(state, "second", QueueState::Queued));
    assert_eq!(queued(), 1);
    session.reply("Done.");
    let state = ada.space(|state| has(state, "second", QueueState::Delivered));
    assert_eq!(state.queue.len(), 2);
    assert_eq!(queued(), 0);
}

/// Creating and attaching sessions held, until this is dropped.
struct Held(FakeBackend);

impl Drop for Held {
    fn drop(&mut self) {
        self.0.release_creation();
    }
}

#[test]
fn a_kept_space_s_session_reaches_it_again_once_bound_with_the_new_run_s_token() {
    let state = State::new("rebound");
    let backend = FakeBackend::new();
    logged();
    let kept = |state: &State| {
        let store = Arc::new(SpaceStore::open(&state.0).expect("the spaces open"));
        move |backends| {
            Server::new(backends, Arc::new(TextTranscriber::default())).with_kept_spaces(store)
        }
    };
    let first = TestServer::with_tools(backend.clone(), kept(&state));
    let mut ada = first.connect("ada", "Ada");
    create(&mut ada, "Kept");
    let before = token(registrations(&backend, 1)[0].1.as_ref().unwrap());
    drop(ada);
    drop(first);

    // Listed again by the next run but not yet bound: the previous run's registration, whose token no longer works, is removed, and its session reaches no space.
    let session = id(&backend.started()[0]);
    let earlier = backend.tools().len();
    backend.hold_creation();
    let second = TestServer::with_tools(backend.clone(), kept(&state));
    // Released however the test ends, before the server is dropped, so that a failure does not leave it waiting.
    let held = Held(backend.clone());
    let removed = registrations(&backend, earlier + 1);
    assert_eq!(removed[earlier], ("/tmp".to_owned(), None));
    let waiting = eventually(|| second.tools_token());
    assert!(
        failure(&call(&second, &waiting, Some(&session), "space", json!({})))
            .starts_with("This session is not bound to a Tomori space")
    );

    // Once it is bound, its directory is registered with the new run's endpoint and token.
    drop(held);
    let registered = eventually(|| {
        let tools = backend.tools();
        tools
            .last()
            .and_then(|(_, server)| server.clone())
            .filter(|server| server.url == format!("http://{}/mcp", second.tools.unwrap()))
    });
    let token = token(&registered);
    assert_eq!(token, waiting);
    assert_ne!(token, before);
    // The previous run's token is refused.
    let ping = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"});
    assert_eq!(post(&second, &before, &ping).status, 401);
    let read = eventually(|| {
        let read = call(&second, &token, Some(&session), "space", json!({}));
        (read["isError"] != true).then_some(read)
    });
    assert_eq!(read["structuredContent"]["title"], "Kept");
}

#[test]
fn the_agent_changes_its_space_s_speech_and_environment_for_everyone_and_is_asked_to_say_so() {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = server.connect("ada", "Ada");
    let space = create(&mut ada, "Settings");
    let token = token(registrations(&backend, 1)[0].1.as_ref().unwrap());
    let session = backend.started()[0].clone();
    let caller = id(&session);
    let caller = Some(caller.as_str());
    // A collaborator in the space sees each change as the owner does.
    let mut bob = server.connect("bob", "Bob");
    bob.send(&ClientMessage::Join {
        space: space.clone(),
    });
    bob.space(|state| state.participants.len() == 2);
    eventually(|| {
        session
            .instructions()
            .contains_key("tomori-tools")
            .then_some(())
    });
    let given = session.log().instructions.len();

    // Speech changes for everyone, from the next reply; the agent is asked to tell everyone, and setting it again changes nothing and asks nothing.
    let set = call(
        &server,
        &token,
        caller,
        "set_speech",
        json!({"speech": "brief"}),
    );
    assert_eq!(
        set["structuredContent"],
        json!({
            "space": space,
            "speech": "brief",
            "changed": true,
            "next": "Tell everyone in the space that from your next reply on, only the start of each is spoken, and the rest is left on the panel.",
        })
    );
    ada.space(|state| state.speech == SpeechSetting::Brief);
    bob.space(|state| state.speech == SpeechSetting::Brief);
    let again = call(
        &server,
        &token,
        caller,
        "set_speech",
        json!({"speech": "brief"}),
    );
    assert_eq!(again["structuredContent"]["changed"], false);
    assert!(
        again["structuredContent"]["next"]
            .as_str()
            .unwrap()
            .contains("there is nothing to tell")
    );
    let full = call(
        &server,
        &token,
        caller,
        "set_speech",
        json!({"speech": "full"}),
    );
    assert_eq!(
        full["structuredContent"]["next"],
        "Tell everyone in the space that from your next reply on, each is spoken in full."
    );
    bob.space(|state| state.speech == SpeechSetting::Full);
    assert_eq!(
        failure(&call(
            &server,
            &token,
            caller,
            "set_speech",
            json!({"speech": "loud"})
        )),
        "Speech is full or brief, not \"loud\"."
    );
    assert_eq!(
        failure(&call(&server, &token, caller, "set_speech", json!({}))),
        "This tool needs speech, as text."
    );
    assert_eq!(
        failure(&call(&server, &token, caller, "set_environment", json!({}))),
        "This tool needs environment, as text."
    );

    // The environment changes for everyone, the agent is given every layer again and asked to tell everyone, and one not offered is refused.
    let moved = call(
        &server,
        &token,
        caller,
        "set_environment",
        json!({"environment": "minimal"}),
    );
    assert_eq!(
        moved["structuredContent"],
        json!({
            "space": space,
            "environment": "minimal",
            "changed": true,
            "next": "Tell everyone in the space that you moved the space to the minimal environment.",
        })
    );
    ada.space(|state| state.environment == "minimal");
    bob.space(|state| state.environment == "minimal");
    eventually(|| (session.log().instructions.len() > given).then_some(()));
    let stayed = call(
        &server,
        &token,
        caller,
        "set_environment",
        json!({"environment": "minimal"}),
    );
    assert_eq!(stayed["structuredContent"]["changed"], false);
    assert!(
        failure(&call(
            &server,
            &token,
            caller,
            "set_environment",
            json!({"environment": "moon"})
        ))
        .contains("sky")
    );
    assert!(
        failure(&call(
            &server,
            &token,
            caller,
            "set_environment",
            json!({"environment": "x".repeat(4096)})
        ))
        .contains("environment")
    );
    // Each change is logged, naming the space, and setting what is already so logs nothing.
    let lines = logged();
    let changes = |line: &str| lines.iter().filter(|logged| *logged == line).count();
    assert_eq!(
        changes(&format!(
            "The agent of space {space} changed its environment to minimal."
        )),
        1
    );
    assert_eq!(
        changes(&format!(
            "The agent of space {space} set its speech to Brief."
        )),
        1
    );
    assert_eq!(
        changes(&format!(
            "The agent of space {space} set its speech to Full."
        )),
        1
    );

    // A closed space keeps its settings.
    session.emit(Event::Closed { reason: None });
    ada.space(|state| state.activity == tomori_protocol::Activity::Closed);
    for (tool, arguments) in [
        ("space", json!({})),
        ("set_speech", json!({"speech": "brief"})),
        ("set_environment", json!({"environment": "sky"})),
    ] {
        assert_eq!(
            failure(&call(&server, &token, caller, tool, arguments)),
            "This space's agent session has stopped.",
            "{tool}"
        );
    }
}

#[test]
fn the_agent_renames_its_own_space_for_everyone_within_the_limits_and_is_asked_to_say_so() {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = server.connect("ada", "Ada");
    let space = create(&mut ada, "Studio");
    let other = create(&mut ada, "Other");
    let token = token(registrations(&backend, 1)[0].1.as_ref().unwrap());
    let session = backend.started()[0].clone();
    let caller = id(&session);
    let caller = Some(caller.as_str());
    // A collaborator in the space and someone in the lobby see the change.
    let mut bob = server.connect("bob", "Bob");
    bob.send(&ClientMessage::Join {
        space: space.clone(),
    });
    bob.space(|state| state.participants.len() == 1 && state.id == space);
    let lobby = server.connect("cat", "Cat");

    // The title is cleaned onto one line, and everyone, in the space and the lobby, sees that the agent renamed it.
    let renamed = call(
        &server,
        &token,
        caller,
        "rename_space",
        json!({"title": "  Release\nplanning  "}),
    );
    assert_eq!(
        renamed["structuredContent"],
        json!({
            "space": space,
            "title": "Release planning",
            "changed": true,
            "next": "Tell everyone in the space that you renamed the space to Release planning.",
        })
    );
    let state = bob.space(|state| state.title == "Release planning");
    assert_eq!(state.renamed_by, Some(Renamer::Agent));
    assert!(state.renames);
    lobby.wait_for(|message| {
        matches!(message, ServerMessage::Spaces { spaces } if spaces
            .iter()
            .any(|summary| summary.id == space && summary.title == "Release planning"))
        .then_some(())
    });
    eventually(|| {
        logged()
            .iter()
            .any(|line| {
                *line
                    == format!(
                        "Space {space} is now titled \"Release planning\", as its agent asked."
                    )
            })
            .then_some(())
    });
    // Only its own space: the other keeps its title.
    let listed = call(&server, &token, None, "spaces", json!({}))["structuredContent"].clone();
    assert!(
        listed["spaces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|summary| summary["id"] == other && summary["title"] == "Other")
    );
    // Renamed, the space no longer follows its session's title.
    session.retitle("Something else");
    session.emit(Event::Busy);
    let state = bob.space(|state| state.activity == tomori_protocol::Activity::Running);
    assert_eq!(state.title, "Release planning");

    // The same title again changes nothing and asks nothing.
    let again = call(
        &server,
        &token,
        caller,
        "rename_space",
        json!({"title": "Release planning"}),
    );
    assert_eq!(again["structuredContent"]["changed"], false);
    assert!(
        again["structuredContent"]["next"]
            .as_str()
            .unwrap()
            .contains("there is nothing to tell")
    );
    // Too long a title is refused, saying why, and changes nothing; so is a call without one.
    assert_eq!(
        failure(&call(
            &server,
            &token,
            caller,
            "rename_space",
            json!({"title": "t".repeat(201)})
        )),
        "That title is too long; the limit is 200 bytes."
    );
    assert_eq!(
        failure(&call(&server, &token, caller, "rename_space", json!({}))),
        "This tool needs title, as text."
    );
    // An empty title lets it follow its session's title again, which it takes at once.
    let cleared = call(
        &server,
        &token,
        caller,
        "rename_space",
        json!({"title": "  "}),
    );
    assert_eq!(
        cleared["structuredContent"]["next"],
        "Tell everyone in the space that the space follows your session's title again, so it is now called Something else."
    );
    let state = bob.space(|state| state.title == "Something else");
    assert_eq!(state.renamed_by, None);
    // In a closed space, the tool changes nothing.
    session.emit(Event::Closed { reason: None });
    bob.space(|state| state.activity == tomori_protocol::Activity::Closed);
    assert_eq!(
        failure(&call(
            &server,
            &token,
            caller,
            "rename_space",
            json!({"title": "Late"})
        )),
        "This space's agent session has stopped."
    );
}

#[test]
fn no_token_is_logged_while_requests_and_their_sessions_are() {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = server.connect("ada", "Ada");
    create(&mut ada, "Quiet");
    let token = token(registrations(&backend, 1)[0].1.as_ref().unwrap());
    let session = id(&backend.started()[0]);
    call(&server, &token, Some(&session), "space", json!({}));
    call(
        &server,
        &token,
        Some(&session),
        "set_speech",
        json!({"speech": "brief"}),
    );
    request(
        &server,
        "POST",
        "/mcp",
        &[("Authorization", "Bearer tomori_wrong")],
        "{}",
    );

    // A client's description is logged only in part, however long it is.
    let long = "x".repeat(4096);
    post(
        &server,
        &token,
        &json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
            "protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": long},
        }}),
    );
    let initialised = eventually(|| {
        logged()
            .into_iter()
            .find(|line| line.starts_with("MCP initialize") && line.contains("xxxx"))
    });
    assert!(!initialised.contains(&long[..600]), "{initialised}");
    assert!(initialised.contains("xxx…"), "{initialised}");

    // What OpenCode sends is logged, at debug, with the calling session, without the token or the arguments.
    let deadline = Instant::now() + TIMEOUT;
    let needle = format!(r#"{{"ai.opencode/sessionID":"{session}","progressToken":7}}"#);
    let lines = loop {
        let lines = logged();
        if lines.iter().any(|line| line.contains(&needle)) || Instant::now() > deadline {
            break lines;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let call = lines
        .iter()
        .find(|line| line.starts_with("MCP tools/call") && line.contains(&needle))
        .expect("the call's metadata is logged");
    assert!(call.contains("authorization: (secret)"), "{call}");
    assert!(!call.contains("brief"), "{call}");
    for line in &lines {
        assert!(!line.contains(&token), "{line}");
    }
}

/// A pose at `position` whose -Z, the way a head looks or a controller points, faces `target`.
fn looking(position: [f32; 3], target: [f32; 3]) -> tomori_protocol::Pose {
    let direction: Vec<f32> = (0..3).map(|axis| target[axis] - position[axis]).collect();
    let yaw = (-direction[0]).atan2(-direction[2]);
    let pitch = direction[1].atan2(direction[0].hypot(direction[2]));
    let (a, c) = (yaw / 2.0).sin_cos();
    let (e, f) = (pitch / 2.0).sin_cos();
    tomori_protocol::Pose {
        position,
        orientation: [c * e, a * f, -a * e, c * f],
    }
}

/// Where the middle of each of the Tomori wall's slots is, where it stands by default.
fn tomori_slots() -> Vec<[f32; 3]> {
    use tomori_protocol::{Pose, TOMORI_WALL, Wall, WallKind, WallShape};
    let wall = Wall::new(
        TOMORI_WALL,
        WallKind::Tomori,
        Pose::upright_facing([0.0, 0.52, -2.3], [0.0, 0.52, 0.0]),
    )
    .with_shape(WallShape::Arc { radius: 2.3 });
    tomori_server::presence::slot_centres(&wall)
}

#[test]
fn presence_says_where_everyone_in_the_space_is_and_what_they_look_at_to_its_agent_alone() {
    use tomori_protocol::ServerMessage;
    use tomori_security::Grant;
    let server = TestServer::with_tokens_and_tools(true, |server| server);
    let (_, owner) = server.issue("Ada", Grant::Owner);
    let mut ada = server.connect_with_token("ada", "Ada", &owner);
    let other = ada.create_space("Other").id;
    let studio = ada.create_space("Studio").id;
    let (_, collaborator) = server.issue(
        "Bob",
        Grant::Collaborator {
            space: studio.clone(),
        },
    );
    let mut bob = server.connect_with_token("bob", "Bob", &collaborator);
    bob.send(&ClientMessage::Join {
        space: studio.clone(),
    });
    let mut vic = server.connect("vic", "Vic");
    vic.send(&ClientMessage::Join {
        space: studio.clone(),
    });
    ada.space(|state| state.id == studio && state.participants.len() == 2 && state.visitors == 1);
    let token = eventually(|| server.tools_token());
    let sessions = server.backend.started();
    let (elsewhere, session) = (id(&sessions[0]), id(&sessions[1]));
    let presence = |session: &str| {
        let result = call(&server, &token, Some(session), "presence", json!({}));
        assert_ne!(result["isError"], true, "{result}");
        result["structuredContent"].clone()
    };

    // Before anyone reports, each is named with their role, and nothing more is known.
    let before = presence(&session);
    assert_eq!(before["space"], studio);
    assert_eq!(before["environment"], "sky");
    assert!(
        before["coordinates"]
            .as_str()
            .unwrap()
            .contains("in metres")
    );
    let people = before["people"].as_array().unwrap();
    let named: Vec<(&str, &str)> = people
        .iter()
        .map(|person| {
            (
                person["name"].as_str().unwrap(),
                person["role"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        named,
        [
            ("Ada", "owner"),
            ("Bob", "collaborator"),
            ("Vic", "visitor")
        ]
    );
    assert_eq!(
        people[0]["presence"],
        "Their client has not reported where they are."
    );
    assert_eq!(
        people[2]["presence"],
        "Visitors send no presence, so where they are is not known."
    );
    // The Tomori wall, with every slot free.
    assert_eq!(before["walls"][0]["id"], "tomori");
    assert_eq!(before["walls"][0]["free"], json!([0, 1, 2, 3, 4, 5]));
    assert_eq!(before["floating"], json!([]));

    // Ada, on a headset, looks at the Tomori wall's slot 4 and points at its slot 2; Bob, at a desktop, looks at her.
    let slots = tomori_slots();
    let home = [0.0, 1.6, 0.0];
    ada.send(&ClientMessage::Presence {
        head: looking(home, slots[4]),
        left: Some(looking([-0.2, 1.1, -0.3], [-0.2, 1.1, -1.0])),
        right: Some(looking([0.2, 1.1, -0.3], slots[2])),
        hands: None,
        carrying: None,
    });
    let bob_head = [1.5, 1.7, 1.5];
    bob.send(&ClientMessage::Presence {
        head: looking(bob_head, home),
        left: None,
        right: None,
        hands: Some(Box::new([None, None])),
        carrying: None,
    });
    // A visitor's presence is refused, and reaches nobody.
    vic.send(&ClientMessage::Presence {
        head: looking([0.0, 1.6, 1.0], home),
        left: None,
        right: None,
        hands: None,
        carrying: None,
    });
    assert!(!vic.error().is_empty());
    bob.wait_for(|message| matches!(message, ServerMessage::Presence { .. }).then_some(()));
    ada.wait_for(|message| matches!(message, ServerMessage::Presence { .. }).then_some(()));
    let seen = presence(&session);
    let people = seen["people"].as_array().unwrap();
    let (ada_seen, bob_seen, vic_seen) = (&people[0], &people[1], &people[2]);
    assert_eq!(ada_seen["device"], "headset");
    assert_eq!(ada_seen["stale"], false);
    assert!(ada_seen["updated_seconds_ago"].as_f64().unwrap() < 5.0);
    assert_eq!(ada_seen["head"]["position"], json!([0.0, 1.6, 0.0]));
    assert!(ada_seen["head"]["pitch"].as_f64().unwrap() < -15.0);
    assert_eq!(ada_seen["looking_at"]["kind"], "wall");
    assert_eq!(ada_seen["looking_at"]["wall"], "tomori");
    assert_eq!(ada_seen["looking_at"]["slot"], 4);
    assert_eq!(ada_seen["pointing_at"]["slot"], 2);
    assert_eq!(ada_seen["left_hand"]["tracked"], "controller");
    assert_eq!(ada_seen["right_hand"]["position"], json!([0.2, 1.1, -0.3]));
    assert_eq!(ada_seen["nearest_wall"]["wall"], "tomori");
    assert_eq!(ada_seen["walls_in_view"], json!(["tomori"]));
    assert_eq!(bob_seen["device"], "desktop");
    assert_eq!(bob_seen["head"]["position"], json!([1.5, 1.7, 1.5]));
    assert_eq!(bob_seen["head"]["yaw"], -45.0);
    assert_eq!(bob_seen["looking_at"]["kind"], "person");
    assert_eq!(bob_seen["looking_at"]["name"], "Ada");
    assert!(bob_seen.get("pointing_at").is_none());
    assert_eq!(bob_seen["walls_in_view"], json!(["tomori"]));
    assert_eq!(vic_seen["role"], "visitor");
    assert!(vic_seen.get("head").is_none());

    // The agent projects a panel; it takes the Tomori wall's first slot, and Ada, looking at it, is told its title.
    sessions[1].emit(Event::MessageStarted {
        message: "m1".into(),
        role: tomori_backend::Role::Assistant,
    });
    sessions[1].emit(Event::TextDelta {
        message: "m1".into(),
        text: "Here.\n\n```panel\ntitle: Build status\n---\nAll green.\n```\n".into(),
    });
    sessions[1].emit(Event::MessageCompleted {
        message: "m1".into(),
    });
    let projected = eventually(|| {
        let seen = presence(&session);
        (seen["walls"][0]["used"].as_array()?.len() == 1).then_some(seen)
    });
    let used = &projected["walls"][0]["used"][0];
    assert_eq!(used["slot"], 0);
    assert_eq!(used["title"], "Build status");
    assert_eq!(projected["walls"][0]["free"], json!([1, 2, 3, 4, 5]));
    ada.send(&ClientMessage::Presence {
        head: looking(home, slots[0]),
        left: None,
        right: None,
        hands: None,
        carrying: None,
    });
    let looked = eventually(|| {
        let seen = presence(&session);
        (seen["people"][0]["looking_at"]["kind"] == "panel").then_some(seen)
    });
    let at = &looked["people"][0]["looking_at"];
    assert_eq!(at["title"], "Build status");
    assert_eq!(at["wall"], "tomori");
    assert_eq!(at["slot"], 0);
    // The headset still says so, though its controllers are not tracked now, and nothing is pointed at.
    assert_eq!(looked["people"][0]["device"], "headset");
    assert_eq!(looked["people"][0]["right_hand"], Value::Null);
    assert_eq!(looked["people"][0]["pointing_at"], Value::Null);

    // Another space's agent is told only of its own space, where nobody has reported anything; others are refused.
    let theirs = presence(&elsewhere);
    assert_eq!(theirs["space"], other);
    assert_eq!(theirs["people"], json!([]));
    assert!(
        failure(&call(
            &server,
            &token,
            Some("ses_other"),
            "presence",
            json!({})
        ))
        .starts_with("This session is not bound to a Tomori space")
    );
    assert!(
        failure(&call(&server, &token, None, "presence", json!({})))
            .ends_with("and this call names no session.")
    );

    // Joining another space, Ada's presence is forgotten until she reports there.
    ada.send(&ClientMessage::Join {
        space: other.clone(),
    });
    let moved = eventually(|| {
        let theirs = presence(&elsewhere);
        (theirs["people"].as_array()?.len() == 1).then_some(theirs)
    });
    assert_eq!(moved["people"][0]["name"], "Ada");
    assert_eq!(
        moved["people"][0]["presence"],
        "Their client has not reported where they are."
    );
}

/// The agent of `session` completes the reply `message`, of `text`.
fn agent_replies(session: &FakeSession, message: &str, text: &str) {
    session.emit(Event::MessageStarted {
        message: message.into(),
        role: tomori_backend::Role::Assistant,
    });
    session.emit(Event::TextDelta {
        message: message.into(),
        text: text.into(),
    });
    session.emit(Event::MessageCompleted {
        message: message.into(),
    });
}

#[test]
fn the_agent_moves_a_panel_near_someone_or_where_they_look_even_one_put_by_hand_but_not_one_carried()
 {
    let backend = FakeBackend::new();
    let server = start(&backend);
    // Ada's client reads what is projected, so that she sees Tomori's notices.
    let mut ada = TestClient::open(server.address);
    ada.hello_projected("ada", "Ada");
    let space = create(&mut ada, "Studio");
    let token = token(registrations(&backend, 1)[0].1.as_ref().unwrap());
    let sessions = backend.started();
    let session = id(&sessions[0]);
    let slots = tomori_slots();
    let home = [0.0, 1.6, 0.0];
    ada.send(&ClientMessage::Presence {
        head: looking(home, slots[4]),
        left: None,
        right: None,
        hands: None,
        carrying: None,
    });
    // Asked by nobody, two panels go onto the Tomori wall's first slots.
    agent_replies(
        &sessions[0],
        "m1",
        "Two.\n\n```panel\ntitle: Build status\n---\nGreen.\n```\n\n```panel\ntitle: Notes\n---\nSome.\n```\n",
    );
    let walls = |session: &str| {
        call(&server, &token, Some(session), "presence", json!({}))["structuredContent"]["walls"]
            .clone()
    };
    let projected = eventually(|| {
        let walls = walls(&session);
        (walls[0]["used"].as_array()?.len() == 2).then_some(walls)
    });
    let notes = projected[0]["used"][1]["panel"]
        .as_str()
        .unwrap()
        .to_owned();
    let move_panel = |panel: &str, place: &str| {
        call(
            &server,
            &token,
            Some(&session),
            "move_panel",
            json!({ "panel": panel, "place": place }),
        )
    };

    // By its title, onto the empty slot Ada is looking at; again, nothing moves.
    let moved = move_panel("build status", "looking Ada");
    assert_ne!(moved["isError"], true, "{moved}");
    let content = &moved["structuredContent"];
    assert_eq!(content["space"], space);
    assert_eq!(content["moved"], true);
    assert_eq!(
        content["place"],
        json!({"kind": "wall", "wall": "tomori", "slot": 4})
    );
    let build = content["panel"].as_str().unwrap().to_owned();
    assert_eq!(
        move_panel(&build, "looking:Ada")["structuredContent"]["moved"],
        false
    );
    // By its identity, floating in front of her.
    let front = &move_panel(&notes, "front Ada")["structuredContent"];
    assert_eq!(front["moved"], true);
    assert_eq!(front["place"]["kind"], "free");
    assert!(front["place"]["pose"]["position"][2].as_f64().unwrap() < -1.0);
    let after = eventually(|| {
        let walls = walls(&session);
        (walls[0]["used"].as_array()?.len() == 1).then_some(walls)
    });
    assert_eq!(after[0]["used"][0]["slot"], 4);
    assert_eq!(after[0]["free"], json!([0, 1, 2, 3, 5]));

    // What it cannot do, it says, and changes nothing.
    assert!(
        failure(&move_panel(&build, "behind Ada"))
            .starts_with("Tomori does not know where \"behind Ada\" is")
    );
    assert_eq!(
        failure(&move_panel(&build, "near")),
        "Nobody asked for this as they spoke, so name whom it is for."
    );
    assert_eq!(
        failure(&move_panel(&build, "near Zed")),
        "Nobody called \"Zed\" in this space has reported where they are."
    );
    assert_eq!(
        failure(&move_panel(&build, "wall attic")),
        "There is no wall called \"attic\" here."
    );
    assert_eq!(
        failure(&move_panel("panel-99", "nearest")),
        "There is no such panel here."
    );
    assert_eq!(
        failure(&call(
            &server,
            &token,
            Some(&session),
            "move_panel",
            json!({"panel": "x"})
        )),
        "This tool needs place, as text."
    );
    assert_eq!(
        failure(&call(
            &server,
            &token,
            Some(&session),
            "move_panel",
            json!({"place": "nearest"})
        )),
        "This tool needs panel, as text."
    );
    assert_eq!(
        failure(&move_panel(&"x".repeat(2_000), "nearest")),
        "The panel is too long; the limit is 1024 bytes."
    );
    assert_eq!(
        failure(&move_panel(&build, &"x".repeat(2_000))),
        "The place is too long; the limit is 1024 bytes."
    );
    agent_replies(
        &sessions[0],
        "m2",
        "```panel\ntitle: Notes\n---\nMore.\n```\n",
    );
    eventually(|| (walls(&session)[0]["used"].as_array()?.len() == 2).then_some(()));
    assert_eq!(
        failure(&move_panel("Notes", "wall tomori")),
        "More than one panel here has that title, so name it by its id or identity, as panels lists them."
    );

    // Once Ada puts it somewhere herself, the agent still moves it when asked, and it stays hers: pinned where it goes,
    // with a notice on the Tomori wall saying what moved, and where.
    ada.send(&ClientMessage::PlacePanel {
        panel: build.clone(),
        wall: Some("tomori".into()),
        slot: Some(5),
        pose: None,
    });
    eventually(|| (walls(&session)[0]["used"][1]["slot"] == 5).then_some(()));
    let floated = &move_panel(&build, "front Ada")["structuredContent"];
    assert_eq!(floated["moved"], true, "{floated}");
    assert_eq!(floated["place"]["kind"], "free");
    let notice = |wanted: &str| {
        ada.wait_for(|message| match message {
            tomori_protocol::ServerMessage::Projection { projection } => projection
                .panels
                .iter()
                .find(|panel| panel.title == "Moved" && panel.body == wanted)
                .map(|panel| panel.id.clone()),
            _ => None,
        })
    };
    notice("Tomori moved 'Build status' to float in the space.");
    let back = &move_panel(&build, "wall Tomori wall")["structuredContent"];
    assert_eq!(back["moved"], true, "{back}");
    notice("Tomori moved 'Build status' to the Tomori wall.");
    let pinned = eventually(|| {
        walls(&session)[0]["used"]
            .as_array()?
            .iter()
            .find(|used| used["panel"] == build.as_str())
            .map(|used| used["pinned"].clone())
    });
    assert_eq!(pinned, true);
    // One notice replaces the last.
    let titled = |title: &str| {
        walls(&session)[0]["used"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|used| used["title"] == title)
            .count()
    };
    assert_eq!(titled("Moved"), 1);
    // The agent's own panel moves without a notice.
    let notes_now = walls(&session)[0]["used"]
        .as_array()
        .unwrap()
        .iter()
        .find(|used| used["title"] == "Notes")
        .map(|used| used["panel"].as_str().unwrap().to_owned())
        .unwrap();
    assert_eq!(
        move_panel(&notes_now, "front Ada")["structuredContent"]["moved"],
        true
    );
    assert_eq!(titled("Moved"), 1);

    // While Ada carries it, it stays in her hand; once she puts it down, it moves again.
    let carrying = |panel: Option<&str>| ClientMessage::Presence {
        head: looking(home, slots[4]),
        left: None,
        right: None,
        hands: None,
        carrying: panel.map(str::to_owned),
    };
    ada.send(&carrying(Some(&build)));
    let refused = eventually(|| {
        let result = move_panel(&build, "front Ada");
        (result["isError"] == true).then(|| failure(&result).to_owned())
    });
    assert_eq!(
        refused,
        "Someone is carrying that panel now, so it stays in their hand; ask them to put it down first."
    );
    ada.send(&carrying(None));
    eventually(|| {
        (move_panel(&build, "front Ada")["structuredContent"]["moved"] == true).then_some(())
    });
}
