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
            "inspect",
            "panels",
            "update_panel",
            "move_panel",
            "set_wall",
            "add_wall",
            "remove_wall",
            "create_layout",
            "remove_layout",
            "move_agent",
            "set_speech",
            "set_environment",
            "rename_space",
            "pairing",
            "confirm_pairing",
            "decline_pairing",
            "workers",
            "remove_workers",
            "show_image"
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
        "move_agent",
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
        json!([{"name": "Ada", "engagement": null, "recording": false}])
    );
    assert_eq!(state["visitors"], 0);
    assert_eq!(state["modes"], json!(["default", "plan"]));
    let other = call(&server, &token, Some(&two), "space", json!({}));
    assert_eq!(other["structuredContent"]["id"], second);
    assert_eq!(
        other["structuredContent"]["participants"],
        json!([{"name": "Bob", "engagement": null, "recording": false}])
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
        "move_agent",
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
    // Each with their handle: a token holder's tag is their token's, and a visitor's one no token or participant has.
    let ids: Vec<&str> = people
        .iter()
        .map(|person| person["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["Ada#10", "Bob#11", "Vic#12"]);
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
    // Ada's report reaches the hub over her own connection, apart from the agent's calls, so wait until the agent is told where she looks before asking it to move anything there.
    eventually(|| {
        let seen = call(&server, &token, Some(&session), "presence", json!({}));
        (seen["structuredContent"]["people"][0]["looking_at"]["slot"] == 4).then_some(())
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

#[test]
fn the_agent_shows_an_image_file_as_a_panel_whose_image_everyone_who_shows_images_fetches() {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = TestClient::open(server.address);
    ada.hello_showing_images("ada", "Ada");
    let space = create(&mut ada, "Images");
    let token = token(registrations(&backend, 1)[0].1.as_ref().unwrap());
    let caller = id(&backend.started()[0]);
    let caller = Some(caller.as_str());
    let folder = std::env::temp_dir().join(format!("tomori-mcp-images-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let picture = folder.join("render.png");
    image::RgbImage::from_fn(300, 200, |x, y| image::Rgb([x as u8, y as u8, 128]))
        .save(&picture)
        .unwrap();
    let path = picture.display().to_string();

    // Only a space's own session may show one, and a file that is missing or not an image is refused, saying why.
    let refused = call(&server, &token, None, "show_image", json!({"path": path}));
    assert!(failure(&refused).contains("names no session"));
    let stranger = call(
        &server,
        &token,
        Some("ses_stranger"),
        "show_image",
        json!({"path": path}),
    );
    assert!(failure(&stranger).contains("not bound to a Tomori space"));
    let missing = call(
        &server,
        &token,
        caller,
        "show_image",
        json!({"path": folder.join("missing.png").display().to_string()}),
    );
    assert_eq!(failure(&missing), "There is no file at that path.");
    std::fs::write(folder.join("notes.txt"), "Not a picture.").unwrap();
    let text = call(
        &server,
        &token,
        caller,
        "show_image",
        json!({"path": folder.join("notes.txt").display().to_string()}),
    );
    assert_eq!(
        failure(&text),
        "That file is not a PNG, JPEG, WebP or GIF image."
    );
    let untitled = call(
        &server,
        &token,
        caller,
        "show_image",
        json!({"path": path, "title": 7}),
    );
    assert_eq!(failure(&untitled), "The title is text.");
    let uncaptioned = call(
        &server,
        &token,
        caller,
        "show_image",
        json!({"path": path, "caption": []}),
    );
    assert_eq!(failure(&uncaptioned), "The caption is text.");
    let unplaced = call(
        &server,
        &token,
        caller,
        "show_image",
        json!({"path": path, "place": 1}),
    );
    assert_eq!(failure(&unplaced), "The place is text.");
    let long = call(
        &server,
        &token,
        caller,
        "show_image",
        json!({"path": path, "title": "t".repeat(5_000)}),
    );
    assert_eq!(failure(&long), "The title is too long.");
    let pathless = call(
        &server,
        &token,
        caller,
        "show_image",
        json!({"title": "Render"}),
    );
    assert_eq!(failure(&pathless), "This tool needs path, as text.");
    let nowhere = call(
        &server,
        &token,
        caller,
        "show_image",
        json!({"path": path, "place": "behind Ada"}),
    );
    assert!(failure(&nowhere).starts_with("Tomori does not know where \"behind Ada\" is"));
    let nobody = call(
        &server,
        &token,
        caller,
        "show_image",
        json!({"path": path, "place": "front Zed"}),
    );
    assert_eq!(
        failure(&nobody),
        "Nobody called \"Zed\" in this space has reported where they are."
    );

    // Shown, it is a panel titled with the file's name, unless given a title, for everyone in the space.
    let shown = call(
        &server,
        &token,
        caller,
        "show_image",
        json!({"path": path, "title": "Render", "caption": "A gradient, red across and green down.", "place": "wall tomori"}),
    );
    let result = &shown["structuredContent"];
    assert_eq!(result["space"], space);
    assert_eq!(
        (result["width"].as_u64(), result["height"].as_u64()),
        (Some(300), Some(200))
    );
    assert!(
        result["next"]
            .as_str()
            .unwrap()
            .contains("Say in a sentence what it shows")
    );
    let panel = ada.wait_for(|message| match message {
        ServerMessage::Projection { projection } => projection
            .panels
            .iter()
            .find(|panel| panel.image.is_some())
            .cloned(),
        _ => None,
    });
    assert_eq!(panel.id, result["panel"]);
    assert_eq!(panel.title, "Render");
    assert_eq!(panel.body, "A gradient, red across and green down.");
    assert!(matches!(&panel.place, tomori_protocol::Place::Wall { wall, .. } if wall == "tomori"));
    let image = panel.image.unwrap();

    // Its image arrives in chunks, in order, and is what its hash says.
    ada.send(&ClientMessage::FetchImage {
        image: image.hash.clone(),
    });
    let mut png = Vec::new();
    while png.len() < image.bytes as usize {
        let chunk = ada.next_image_chunk(TIMEOUT).expect("the image arrives");
        assert_eq!(tomori_protocol::hash_text(&chunk.hash), image.hash);
        assert_eq!(chunk.offset as usize, png.len());
        png.extend(chunk.data);
    }
    let hash: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(&png).into();
    assert_eq!(tomori_protocol::hash_text(&hash), image.hash);
    let decoded = image::load_from_memory(&png).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (300, 200));
    // Without a title, it is titled with the file's name.
    let untitled = call(&server, &token, caller, "show_image", json!({"path": path}));
    let panel = untitled["structuredContent"]["panel"]
        .as_str()
        .unwrap()
        .to_owned();
    let titled = ada.wait_for(|message| match message {
        ServerMessage::Projection { projection } => {
            projection.panel(&panel).map(|panel| panel.title.clone())
        }
        _ => None,
    });
    assert_eq!(titled, "render.png");
    let _ = std::fs::remove_dir_all(folder);
}

/// How far `at` stands aside from the way someone at `eyes` looks, along −Z, in whole degrees clockwise as seen from
/// above, so that their right is positive.
fn aside(eyes: [f32; 3], at: &Value) -> f64 {
    let x = at[0].as_f64().unwrap() - f64::from(eyes[0]);
    let z = at[2].as_f64().unwrap() - f64::from(eyes[2]);
    x.atan2(-z).to_degrees().round()
}

#[test]
fn the_agent_floats_panels_and_images_beside_someone_by_their_name_or_handle() {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = TestClient::open(server.address);
    ada.hello_showing_images("ada", "Ada");
    create(&mut ada, "Beside");
    let token = token(registrations(&backend, 1)[0].1.as_ref().unwrap());
    let sessions = backend.started();
    let session = id(&sessions[0]);
    let caller = Some(session.as_str());
    // Ada stands across the space from every wall, looking along −Z.
    let eyes = [6.0, 1.6, 6.0];
    ada.send(&ClientMessage::Presence {
        head: looking(eyes, [6.0, 1.6, 0.0]),
        left: None,
        right: None,
        hands: None,
        carrying: None,
    });
    let presence =
        || call(&server, &token, caller, "presence", json!({}))["structuredContent"].clone();
    // Presence names her and gives her handle, which place takes as it takes her name.
    let handle = eventually(|| {
        let seen = presence();
        let person = &seen["people"][0];
        person["head"]
            .is_object()
            .then(|| person["id"].as_str().unwrap().to_owned())
    });
    assert!(handle.starts_with("Ada#"), "{handle}");
    assert_eq!(presence()["people"][0]["name"], "Ada");
    let floating = |panel: &str| {
        eventually(|| {
            presence()["floating"]
                .as_array()?
                .iter()
                .find(|floating| floating["panel"] == panel)
                .map(|floating| floating["position"].clone())
        })
    };

    // A panel nobody asked for goes onto the Tomori wall; moved beside Ada, it floats to her right.
    agent_replies(
        &sessions[0],
        "m1",
        "A list.\n\n```panel\nid: todo\ntitle: Todo\n---\n- [ ] One\n```\n",
    );
    let todo = eventually(|| {
        presence()["walls"][0]["used"][0]["panel"]
            .as_str()
            .map(str::to_owned)
    });
    let moved = call(
        &server,
        &token,
        caller,
        "move_panel",
        json!({"panel": "todo", "place": "beside Ada"}),
    );
    assert_eq!(moved["structuredContent"]["moved"], true, "{moved}");
    assert_eq!(moved["structuredContent"]["place"]["kind"], "free");
    assert_eq!(aside(eyes, &floating(&todo)), 30.0);
    // An image beside her handle fans to her left, clear of the list.
    let folder = std::env::temp_dir().join(format!("tomori-mcp-beside-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let picture = folder.join("render.png");
    image::RgbImage::from_fn(30, 20, |x, y| image::Rgb([x as u8, y as u8, 128]))
        .save(&picture)
        .unwrap();
    let shown = call(
        &server,
        &token,
        caller,
        "show_image",
        json!({"path": picture.display().to_string(), "place": format!("beside {}", handle.to_lowercase())}),
    );
    assert_ne!(shown["isError"], true, "{shown}");
    let image = shown["structuredContent"]["panel"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(aside(eyes, &floating(&image)), -30.0);
    // Updated in place with beside, the list is in its own way no more than before: it stays where it is.
    let updated = call(
        &server,
        &token,
        caller,
        "update_panel",
        json!({"panel": "todo", "check": [1], "place": "beside"}),
    );
    assert!(
        failure(&updated).starts_with("Nobody asked for this as they spoke"),
        "{updated}"
    );
    let updated = call(
        &server,
        &token,
        caller,
        "update_panel",
        json!({"panel": "todo", "check": [1], "place": format!("beside {handle}")}),
    );
    assert_ne!(updated["isError"], true, "{updated}");
    assert_eq!(aside(eyes, &floating(&todo)), 30.0);
    // A handle nobody has, and a form that names no wall where a wall is meant, are said.
    assert_eq!(
        failure(&call(
            &server,
            &token,
            caller,
            "move_panel",
            json!({"panel": "todo", "place": "beside Ada#9999"}),
        )),
        "Nobody called \"Ada#9999\" in this space has reported where they are."
    );
    assert_eq!(
        failure(&call(
            &server,
            &token,
            caller,
            "set_wall",
            json!({"wall": "beside Ada", "columns": 2}),
        )),
        "beside names no wall: say which wall, or looking or near someone."
    );
    let _ = std::fs::remove_dir_all(folder);
}

#[test]
fn the_agent_adds_changes_and_removes_walls_for_everyone_naming_this_wall_by_where_someone_looks() {
    let backend = FakeBackend::new();
    let server = start(&backend);
    // Her client reads what is projected, so that she sees each change.
    let mut ada = tomori_server::test_support::TestClient::open(server.address);
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
    let tool =
        |name: &str, arguments: Value| call(&server, &token, Some(&session), name, arguments);
    let walls = || {
        call(&server, &token, Some(&session), "presence", json!({}))["structuredContent"]["walls"]
            .clone()
    };
    eventually(|| (walls()[0]["name"] == "Tomori wall").then_some(()));
    // Ada's report reaches the hub over her own connection, so wait until the agent is told where she is before placing anything in front of her.
    eventually(|| {
        let presence = call(&server, &token, Some(&session), "presence", json!({}));
        presence["structuredContent"]["people"][0]
            .get("head")
            .is_some()
            .then_some(())
    });
    // A wall four wide and one high, in front of Ada, facing her.
    let added = tool(
        "add_wall",
        json!({"place": "front Ada", "columns": 4, "rows": 1}),
    );
    assert_ne!(added["isError"], true, "{added}");
    let content = &added["structuredContent"];
    assert_eq!(content["space"], space);
    assert_eq!(content["name"], "added wall 1");
    assert_eq!(
        content["now"],
        "4 slots wide and 1 high, 2.6 by 0.5 m, its slots 60 by 36 cm, flat"
    );
    assert!(
        content["next"]
            .as_str()
            .unwrap()
            .starts_with("Tell everyone")
    );
    let wall = content["wall"].as_str().unwrap().to_owned();
    let listed = eventually(|| {
        let walls = walls();
        (walls.as_array()?.len() == 2).then_some(walls)
    });
    assert_eq!(
        (listed[1]["columns"].clone(), listed[1]["rows"].clone()),
        (json!(4), json!(1))
    );
    assert_eq!(listed[1]["free"], json!([0, 1, 2, 3]));
    // The notice of it is on the Tomori wall.
    assert_eq!(listed[0]["used"][0]["title"], "Walls");

    // This wall, the one Ada is looking at, the Tomori wall: four wide, flat.
    let changed = tool(
        "set_wall",
        json!({"wall": "looking Ada", "columns": 4, "radius": 0}),
    );
    assert_ne!(changed["isError"], true, "{changed}");
    let content = &changed["structuredContent"];
    assert_eq!(
        (content["wall"].clone(), content["changed"].clone()),
        (json!("tomori"), json!(true))
    );
    assert!(
        content["now"]
            .as_str()
            .unwrap()
            .ends_with("its slots 42 by 25 cm, flat")
    );
    assert_eq!(
        tool("set_wall", json!({"wall": "Tomori wall", "columns": 4}))["structuredContent"]["changed"],
        false
    );
    // The added wall curved, four metres wide, by name; its slots grow to make it so.
    let wide = tool(
        "set_wall",
        json!({"wall": "the added wall 1", "radius": 3, "width": 4}),
    );
    assert_ne!(wide["isError"], true, "{wide}");
    let now = wide["structuredContent"]["now"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(now.starts_with("4 slots wide and 1 high, 4.0 by"), "{now}");
    assert!(now.ends_with("curved at a radius of 3.0 m"), "{now}");
    let listed = eventually(|| {
        let walls = walls();
        (walls[1]["shape"] == json!({"arc": 3.0})).then_some(walls)
    });
    assert_eq!(listed[1]["size"][0], 4.0);
    // The change reached Ada's client too.
    let projection = ada.wait_for(|message| match message {
        tomori_protocol::ServerMessage::Projection { projection }
            if projection
                .wall(&wall)
                .is_some_and(|shown| shown.radius() == Some(3.0)) =>
        {
            Some(projection.clone())
        }
        _ => None,
    });
    assert_eq!(projection.wall(&wall).unwrap().slots(), 4);

    // What it cannot do, it says, and changes nothing.
    for (arguments, error) in [
        (
            json!({"wall": "Tomori wall", "rows": 2}),
            "The Tomori wall stays one row of at most 6 slots, so that it stays clear of the swarm and the workers.",
        ),
        (
            json!({"wall": "front"}),
            "front names no wall: say which wall, or looking or near someone.",
        ),
        (
            json!({"wall": wall, "scale": 1.0, "width": 3}),
            "Say either scale or width, not both.",
        ),
        (
            json!({"wall": wall, "columns": 9}),
            "A wall has from 1 to 8 columns and from 1 to 4 rows of slots, and at most 24 slots.",
        ),
        (
            json!({"wall": wall, "columns": "four"}),
            "columns is a whole number.",
        ),
        (
            json!({"wall": wall, "place": "near Ada"}),
            "A wall is put in front of someone: say front with their name, or front for whoever asked.",
        ),
        (
            json!({"wall": "Tomori wall", "place": "front Ada"}),
            "The Tomori wall stands at its home, or with the agent while it is summoned, so it cannot be moved.",
        ),
        (
            json!({"wall": "near Zed"}),
            "Nobody called \"Zed\" in this space has reported where they are.",
        ),
        (json!({}), "This tool needs wall, as text."),
    ] {
        assert_eq!(failure(&tool("set_wall", arguments)), error);
    }
    assert!(
        failure(&tool("set_wall", json!({"wall": "attic"})))
            .starts_with("There is no wall called \"attic\" here")
    );
    assert_eq!(
        failure(&tool("add_wall", json!({}))),
        "Nobody asked for this as they spoke, so name whom it is for."
    );
    assert_eq!(
        failure(&tool("remove_wall", json!({"wall": "Tomori wall"}))),
        "Only a wall an owner added can be removed."
    );

    // Moved in front of Ada again, then removed, its panels left floating.
    agent_replies(
        &sessions[0],
        "m1",
        "```panel\ntitle: Plan\nplace: wall added wall 1\n---\nSteps.\n```\n",
    );
    eventually(|| (walls()[1]["used"].as_array()?.len() == 1).then_some(()));
    let moved = tool("set_wall", json!({"wall": wall, "place": "front Ada"}));
    assert_ne!(moved["isError"], true, "{moved}");
    let removed = tool("remove_wall", json!({"wall": "added wall 1"}));
    assert_ne!(removed["isError"], true, "{removed}");
    assert_eq!(removed["structuredContent"]["floating"], 1);
    assert_eq!(
        removed["structuredContent"]["next"],
        "Tell everyone in the space that you removed the added wall 1; the panel on it floats where it was."
    );
    let floating = eventually(|| {
        let presence = call(&server, &token, Some(&session), "presence", json!({}));
        let floating = presence["structuredContent"]["floating"].clone();
        (presence["structuredContent"]["walls"].as_array()?.len() == 1
            && floating.as_array()?.len() == 1)
            .then_some(floating)
    });
    assert_eq!(floating[0]["title"], "Plan");
    // Each change was logged, naming the space.
    let logs = logged();
    assert!(
        logs.iter()
            .any(|line| line.contains(&format!("The agent of space {space} added {wall}")))
    );
    assert!(
        logs.iter()
            .any(|line| line.contains(&format!("The agent of space {space} changed tomori")))
    );
    assert!(
        logs.iter()
            .any(|line| line.contains(&format!("The agent of space {space} removed {wall}")))
    );
}

#[test]
fn the_agent_snaps_a_wall_beside_another_into_a_group_and_takes_it_out_again() {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = tomori_server::test_support::TestClient::open(server.address);
    ada.hello_projected("ada", "Ada");
    create(&mut ada, "Studio");
    let token = token(registrations(&backend, 1)[0].1.as_ref().unwrap());
    let sessions = backend.started();
    let session = id(&sessions[0]);
    ada.send(&ClientMessage::Presence {
        head: looking([0.0, 1.6, 0.0], [0.0, 1.4, -3.0]),
        left: None,
        right: None,
        hands: None,
        carrying: None,
    });
    let tool =
        |name: &str, arguments: Value| call(&server, &token, Some(&session), name, arguments);
    let walls = || {
        call(&server, &token, Some(&session), "presence", json!({}))["structuredContent"]["walls"]
            .clone()
    };
    eventually(|| (walls()[0]["name"] == "Tomori wall").then_some(()));
    let first = tool(
        "add_wall",
        json!({"place": "front Ada", "columns": 2, "rows": 2, "scale": 0.95}),
    );
    assert_ne!(first["isError"], true, "{first}");
    let second = tool(
        "add_wall",
        json!({"place": "front Ada", "columns": 1, "rows": 1, "scale": 1.9}),
    );
    assert_ne!(second["isError"], true, "{second}");
    // Snapped to the right of the first, a big one beside a two by two, they are one group.
    let snapped = tool(
        "set_wall",
        json!({"wall": "added wall 2", "attach": {"to": "added wall 1", "edge": "right"}}),
    );
    assert_ne!(snapped["isError"], true, "{snapped}");
    let content = &snapped["structuredContent"];
    assert_eq!(content["changed"], true);
    let now = content["now"].as_str().unwrap();
    assert!(
        now.ends_with("flat, joined to added wall 1 on its left"),
        "{now}"
    );
    let listed = eventually(|| {
        let walls = walls();
        walls[2].get("group").is_some().then_some(walls)
    });
    assert_eq!(listed[1]["group"], listed[2]["group"]);
    assert_eq!(listed[1]["joined"][0]["edge"], "right");
    assert_eq!(listed[1]["joined"][0]["name"], "added wall 2");
    assert_eq!(listed[2]["joined"][0]["edge"], "left");
    // Ada's client is sent the group, each wall's edge joined.
    let projection = ada.wait_for(|message| match message {
        tomori_protocol::ServerMessage::Projection { projection }
            if projection
                .walls
                .iter()
                .filter(|wall| wall.group.is_some())
                .count()
                == 2 =>
        {
            Some(projection.clone())
        }
        _ => None,
    });
    assert_eq!(projection.walls[1].joined.len(), 1);
    // What it cannot snap, it says.
    for (arguments, error) in [
        (
            json!({"wall": "added wall 2", "attach": {"to": "Tomori wall", "edge": "left"}}),
            "Only walls someone added snap together: the Tomori wall and an environment's walls stand where they are put.",
        ),
        (
            json!({"wall": "added wall 2", "attach": {"to": "added wall 1", "edge": "behind"}}),
            "attach's edge is left, right, top or bottom.",
        ),
        (
            json!({"wall": "added wall 2", "attach": {"to": "added wall 1", "edge": "left", "align": "spread"}}),
            "attach's align is start, middle or end.",
        ),
        (
            json!({"wall": "added wall 2", "attach": {"to": "added wall 1", "edge": "left"}, "detach": true}),
            "Say either attach or detach, not both.",
        ),
        (
            json!({"wall": "added wall 2", "attach": {"to": "added wall 1", "edge": "left"}, "place": "front"}),
            "Say either place or attach, not both: an attached wall stands beside the other.",
        ),
        (
            json!({"wall": "added wall 2", "attach": {"edge": "left"}}),
            "This tool needs to, as text.",
        ),
        (
            json!({"wall": "added wall 2", "attach": {"to": "attic", "edge": "left"}}),
            "There is no wall called \"attic\" here: name one as presence does, or say looking or near, with a name or for whoever asked.",
        ),
        (
            json!({"wall": "added wall 2", "attach": {"to": "x".repeat(1025), "edge": "left"}}),
            "The to is too long; the limit is 1024 bytes.",
        ),
    ] {
        assert_eq!(failure(&tool("set_wall", arguments)), error);
    }
    // Made smaller and snapped below, at once, it moves; if it cannot be attached after it changed, it says so.
    let below = tool(
        "set_wall",
        json!({"wall": "added wall 2", "scale": 1.0, "attach": {"to": "added wall 1", "edge": "bottom", "align": "middle"}}),
    );
    assert_ne!(below["isError"], true, "{below}");
    let refused = tool(
        "set_wall",
        json!({"wall": "added wall 2", "scale": 1.2, "attach": {"to": "Tomori wall", "edge": "bottom"}}),
    );
    assert!(
        failure(&refused)
            .starts_with("The wall was changed, but not attached: Only walls someone added")
    );
    // Taken out again, it stands alone.
    let detached = tool("set_wall", json!({"wall": "added wall 2", "detach": true}));
    assert_ne!(detached["isError"], true, "{detached}");
    eventually(|| walls()[2].get("group").is_none().then_some(()));
    assert_eq!(
        tool("set_wall", json!({"wall": "added wall 2", "detach": true}))["structuredContent"]["changed"],
        false
    );
}

#[test]
fn the_agent_builds_a_layout_of_walls_in_one_step_fills_its_slots_and_takes_it_down() {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = TestClient::open(server.address);
    ada.hello_showing_images("ada", "Ada");
    create(&mut ada, "Dashboard");
    let token = token(registrations(&backend, 1)[0].1.as_ref().unwrap());
    let sessions = backend.started();
    let session = id(&sessions[0]);
    ada.send(&ClientMessage::Presence {
        head: looking([0.0, 1.6, 0.0], [0.0, 1.6, -3.0]),
        left: None,
        right: None,
        hands: None,
        carrying: None,
    });
    let tool =
        |name: &str, arguments: Value| call(&server, &token, Some(&session), name, arguments);
    let walls = || {
        call(&server, &token, Some(&session), "presence", json!({}))["structuredContent"]["walls"]
            .clone()
    };
    eventually(|| (walls()[0]["name"] == "Tomori wall").then_some(()));
    // A two by two and a big one beside it, as tall, where Ada is looking, in one step.
    let built = tool(
        "create_layout",
        json!({"place": "looking Ada", "walls": [
            {"id": "Summary", "columns": 2, "rows": 2, "scale": 0.9},
            {"id": "chart", "columns": 1, "rows": 1, "attach": {"to": "summary", "edge": "right"}},
        ]}),
    );
    assert_ne!(built["isError"], true, "{built}");
    let content = &built["structuredContent"];
    let layout = content["walls"].as_array().unwrap();
    assert_eq!(layout.len(), 2);
    assert_eq!(
        (
            layout[0]["id"].clone(),
            layout[0]["name"].clone(),
            layout[0]["slots"].clone()
        ),
        (json!("summary"), json!("added wall 1"), json!([0, 1, 2, 3]))
    );
    assert_eq!(
        (layout[1]["name"].clone(), layout[1]["slots"].clone()),
        (json!("added wall 2"), json!([0]))
    );
    assert_eq!(content["group"], layout[0]["wall"]);
    assert!(
        content["next"]
            .as_str()
            .unwrap()
            .contains("place: wall NAME slot N")
    );
    let listed = eventually(|| {
        let walls = walls();
        walls[2].get("group").is_some().then_some(walls)
    });
    assert_eq!(listed[1]["joined"][0]["name"], "added wall 2");
    // A notice on the Tomori wall says what it built.
    assert_eq!(listed[0]["used"][0]["title"], "Walls");

    // Filled precisely: a text panel, a code panel and an image, each in the slot it names.
    agent_replies(
        &sessions[0],
        "m1",
        "```panel\ntitle: Open issues\nplace: wall added wall 1 slot 3\n---\n- one\n- two\n```\n\n```rust place=\"wall added wall 2 slot 0\"\nfn main() {\n    println!(\"hi\");\n}\n```\n",
    );
    let folder = std::env::temp_dir().join(format!("tomori-mcp-layouts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let picture = folder.join("chart.png");
    image::RgbImage::from_fn(64, 48, |x, y| image::Rgb([x as u8, y as u8, 200]))
        .save(&picture)
        .unwrap();
    let shown = tool(
        "show_image",
        json!({"path": picture.display().to_string(), "place": "wall added wall 1 slot 1"}),
    );
    assert_ne!(shown["isError"], true, "{shown}");
    let listed = eventually(|| {
        let walls = walls();
        (walls[1]["used"].as_array()?.len() == 2 && walls[2]["used"].as_array()?.len() == 1)
            .then_some(walls)
    });
    let used = |wall: usize| -> Vec<(Value, Value)> {
        listed[wall]["used"]
            .as_array()
            .unwrap()
            .iter()
            .map(|used| (used["slot"].clone(), used["title"].clone()))
            .collect()
    };
    assert_eq!(
        used(1),
        [
            (json!(1), json!("chart.png")),
            (json!(3), json!("Open issues"))
        ]
    );
    assert_eq!(used(2), [(json!(0), json!("rust"))]);
    // A slot it does not have is refused, saying how its slots are numbered.
    assert_eq!(
        failure(&tool(
            "move_panel",
            json!({"panel": "Open issues", "place": "wall added wall 2 slot 1"})
        )),
        "The added wall 2 has no slot 1: its slots are numbered from 0 to 0."
    );
    let _ = std::fs::remove_dir_all(&folder);

    // What it cannot build, it says, and builds nothing.
    for (arguments, error) in [
        (
            json!({"walls": []}),
            "This tool needs walls, a list of from 1 to 6 walls.",
        ),
        (
            json!({"place": 5, "walls": [{"id": "a"}]}),
            "place is text.",
        ),
        (
            json!({"walls": [{"id": "a", "width": "wide"}]}),
            "width is a number.",
        ),
        (
            json!({"walls": [{"id": "a b"}]}),
            "A wall's id is from 1 to 32 letters, digits, - and _, not \"a b\".",
        ),
        (
            json!({"walls": [{"columns": 2}]}),
            "This tool needs id, as text.",
        ),
        (
            json!({"walls": [{"id": "a"}, {"id": "b", "attach": {"to": "a", "edge": "behind"}}]}),
            "attach's edge is left, right, top or bottom.",
        ),
        (
            json!({"walls": [{"id": "a", "scale": 1.0, "width": 2.0}]}),
            "Say either scale or width, not both.",
        ),
        (
            json!({"place": "wall Tomori wall", "walls": [{"id": "a"}]}),
            "A layout stands in front of someone or where they are looking: say front, near or looking, with a name or for whoever asked.",
        ),
        (
            json!({"place": "behind", "walls": [{"id": "a"}]}),
            "Tomori does not know where \"behind\" is: say near NAME, looking NAME, front NAME, beside NAME, wall WALL or wall WALL slot N, or near, looking, front or beside for whoever asked.",
        ),
        (
            json!({"walls": [{"id": "a"}]}),
            "Nobody asked for this as they spoke, so name whom it is for.",
        ),
        (
            json!({"place": "front Ada", "walls": [{"id": "a"}, {"id": "b", "radius": 3, "attach": {"to": "a", "edge": "top"}}]}),
            "A layout's walls are all flat, or all curved at the same radius.",
        ),
    ] {
        assert_eq!(failure(&tool("create_layout", arguments)), error);
    }
    // A curved layout, its later walls curved as its first.
    let curved = tool(
        "create_layout",
        json!({"place": "front Ada", "walls": [
            {"id": "a", "columns": 2, "rows": 1, "radius": 2.4},
            {"id": "b", "columns": 2, "rows": 1, "scale": 0.8, "attach": {"to": "a", "edge": "left", "align": "middle"}},
        ]}),
    );
    assert_ne!(curved["isError"], true, "{curved}");
    let names: Vec<Value> = curved["structuredContent"]["walls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|wall| wall["name"].clone())
        .collect();
    assert_eq!(names, [json!("added wall 3"), json!("added wall 4")]);
    eventually(|| (walls().as_array()?.len() == 5).then_some(()));
    assert!(walls()[3]["shape"]["arc"].is_number() && walls()[4]["shape"]["arc"].is_number());

    // In front of someone near it, and taken down with nothing on it.
    let near = tool(
        "create_layout",
        json!({"place": "near Ada", "walls": [{"id": "lone", "columns": 1, "rows": 1}]}),
    );
    assert_ne!(near["isError"], true, "{near}");
    let lone = near["structuredContent"]["walls"][0]["name"]
        .as_str()
        .unwrap()
        .to_owned();
    eventually(|| (walls().as_array()?.len() == 6).then_some(()));
    let removed = tool("remove_layout", json!({"group": lone}));
    assert_eq!(removed["structuredContent"]["floating"], 0, "{removed}");
    assert!(
        removed["structuredContent"]["next"]
            .as_str()
            .unwrap()
            .ends_with(&format!("you removed {lone}."))
    );
    eventually(|| (walls().as_array()?.len() == 5).then_some(()));
    // Taken down by any of its walls, its panels float; the Tomori wall is not a layout.
    assert_eq!(
        failure(&tool("remove_layout", json!({"group": "Tomori wall"}))),
        "Only walls someone added can be removed: the Tomori wall and an environment's walls stay."
    );
    assert_eq!(
        failure(&tool("remove_layout", json!({}))),
        "This tool needs group, as text."
    );
    let removed = tool("remove_layout", json!({"group": "added wall 2"}));
    assert_ne!(removed["isError"], true, "{removed}");
    assert_eq!(removed["structuredContent"]["floating"], 3);
    assert_eq!(
        removed["structuredContent"]["next"],
        "Tell everyone in the space that you removed added wall 1, added wall 2; the 3 panels on them float where they were."
    );
    eventually(|| (walls().as_array()?.len() == 3).then_some(()));
    agent_replies(
        &sessions[0],
        "m2",
        "```panel\ntitle: Left\nplace: wall added wall 1 slot 0\n---\nOne panel.\n```\n",
    );
    eventually(|| (walls()[1]["used"].as_array()?.len() == 1).then_some(()));
    let removed = tool("remove_layout", json!({"group": "added wall 1"}));
    assert_eq!(removed["structuredContent"]["floating"], 1, "{removed}");
    assert!(
        removed["structuredContent"]["next"]
            .as_str()
            .unwrap()
            .ends_with("the panel on them floats where it was.")
    );
    assert!(
        logged()
            .iter()
            .any(|line| line.contains("built a layout of"))
    );
}

#[test]
fn the_agent_gives_a_wall_slots_of_any_shape_by_their_aspect_or_their_size_in_metres() {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = tomori_server::test_support::TestClient::open(server.address);
    ada.hello_projected("ada", "Ada");
    create(&mut ada, "Studio");
    let token = token(registrations(&backend, 1)[0].1.as_ref().unwrap());
    let sessions = backend.started();
    let session = id(&sessions[0]);
    let home = [0.0, 1.6, 0.0];
    ada.send(&ClientMessage::Presence {
        head: looking(home, [0.0, 9.0, -0.01]),
        left: None,
        right: None,
        hands: None,
        carrying: None,
    });
    let tool =
        |name: &str, arguments: Value| call(&server, &token, Some(&session), name, arguments);
    let walls = || {
        call(&server, &token, Some(&session), "presence", json!({}))["structuredContent"]["walls"]
            .clone()
    };
    eventually(|| {
        let presence = call(&server, &token, Some(&session), "presence", json!({}));
        presence["structuredContent"]["people"][0]
            .get("head")
            .is_some()
            .then_some(())
    });
    let now = |result: &Value| {
        assert_ne!(result["isError"], true, "{result}");
        result["structuredContent"]["now"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    // Three tall slots across, 9:16: as wide as a standard slot, their text its size, and taller.
    let added = tool(
        "add_wall",
        json!({"place": "front Ada", "columns": 3, "rows": 1, "aspect": "9:16"}),
    );
    assert_eq!(
        now(&added),
        "3 slots wide and 1 high, 2.0 by 1.2 m, its slots 60 by 107 cm, flat"
    );
    let wall = added["structuredContent"]["wall"]
        .as_str()
        .unwrap()
        .to_owned();
    let listed = eventually(|| {
        let walls = walls();
        (walls.as_array()?.len() == 2).then_some(walls)
    });
    assert_eq!(listed[1]["slot_size"], json!([0.6, 1.07]));
    // Her client is sent the slots' shape.
    let shown = ada.wait_for(|message| match message {
        tomori_protocol::ServerMessage::Projection { projection } => {
            projection.wall(&wall).cloned()
        }
        _ => None,
    });
    assert_eq!(shown.aspect, Some(0.5625));
    // Square slots half a metre across, by their size in metres: their text as large as the narrower allows.
    let square = tool(
        "set_wall",
        json!({"wall": "added wall 1", "slot_width": 0.5, "slot_height": 0.5}),
    );
    assert!(
        now(&square).ends_with("its slots 50 by 50 cm, flat"),
        "{square}"
    );
    // Only taller, the width kept.
    let taller = tool("set_wall", json!({"wall": wall, "slot_height": 0.8}));
    assert!(
        now(&taller).ends_with("its slots 50 by 80 cm, flat"),
        "{taller}"
    );
    // Back to a standard slot's shape, the text kept its size; and wide, as a number.
    let standard = tool("set_wall", json!({"wall": wall, "aspect": "standard"}));
    assert!(
        now(&standard).ends_with("its slots 50 by 30 cm, flat"),
        "{standard}"
    );
    let wide = tool("set_wall", json!({"wall": wall, "aspect": 3}));
    assert!(
        now(&wide).ends_with("its slots 90 by 30 cm, flat"),
        "{wide}"
    );
    // What cannot be, or is said twice over, is refused, saying why.
    for (name, arguments, error) in [
        (
            "set_wall",
            json!({"wall": wall, "aspect": "tall"}),
            "aspect is the slots' width over their height: a number, or a ratio such as 9:16 or 16:9, or standard.",
        ),
        (
            "set_wall",
            json!({"wall": wall, "aspect": "0:4"}),
            "aspect is the slots' width over their height: a number, or a ratio such as 9:16 or 16:9, or standard.",
        ),
        (
            "set_wall",
            json!({"wall": wall, "aspect": 0.1}),
            "A wall's slots are from four times as tall as they are wide to four times as wide.",
        ),
        (
            "set_wall",
            json!({"wall": wall, "slot_width": 0.5, "scale": 1.0}),
            "Say the slots' size either as slot_width and slot_height, or as scale, aspect or width, not both.",
        ),
        (
            "set_wall",
            json!({"wall": wall, "slot_height": 0}),
            "A slot's width and height are numbers of metres.",
        ),
        (
            "set_wall",
            json!({"wall": "Tomori wall", "aspect": "1:1"}),
            "The Tomori wall's slots stay their size, so that it stays clear of the swarm and the workers.",
        ),
        (
            "add_wall",
            json!({"place": "front Ada", "width": 3}),
            "Give a new wall its slots' size, not its width.",
        ),
        (
            "add_wall",
            json!({"place": "front Ada", "slot_width": 0.5, "aspect": 1}),
            "Say the slots' size either as slot_width and slot_height, or as scale and aspect, not both.",
        ),
        (
            "add_wall",
            json!({"place": "front Ada", "slot_height": 0.5, "scale": 1}),
            "Say the slots' size either as slot_width and slot_height, or as scale and aspect, not both.",
        ),
        (
            "add_wall",
            json!({"place": "front Ada", "slot_width": -1}),
            "A slot's width and height are numbers of metres.",
        ),
        (
            "add_wall",
            json!({"place": "front Ada", "aspect": "wide"}),
            "aspect is the slots' width over their height: a number, or a ratio such as 9:16 or 16:9, or standard.",
        ),
        (
            "add_wall",
            json!({"place": "front Ada", "slot_height": "tall"}),
            "slot_height is a number.",
        ),
        (
            "set_wall",
            json!({"wall": wall, "slot_width": "wide"}),
            "slot_width is a number.",
        ),
        (
            "set_wall",
            json!({"wall": wall, "aspect": "x".repeat(2_000)}),
            "The aspect is too long; the limit is 1024 bytes.",
        ),
        (
            "set_wall",
            json!({"wall": wall, "aspect": "a:16"}),
            "aspect is the slots' width over their height: a number, or a ratio such as 9:16 or 16:9, or standard.",
        ),
        (
            "set_wall",
            json!({"wall": wall, "aspect": "9:b"}),
            "aspect is the slots' width over their height: a number, or a ratio such as 9:16 or 16:9, or standard.",
        ),
        (
            "set_wall",
            json!({"wall": wall, "aspect": true}),
            "aspect is the slots' width over their height: a number, or a ratio such as 9:16 or 16:9, or standard.",
        ),
        (
            "set_wall",
            json!({"wall": wall, "aspect": -1}),
            "aspect is the slots' width over their height: a number, or a ratio such as 9:16 or 16:9, or standard.",
        ),
        (
            "set_wall",
            json!({"wall": wall, "aspect": 1e300}),
            "aspect is the slots' width over their height: a number, or a ratio such as 9:16 or 16:9, or standard.",
        ),
    ] {
        assert_eq!(failure(&tool(name, arguments)), error, "{name}");
    }
    // A new wall can be given its slots' size in metres too.
    let sized = tool(
        "add_wall",
        json!({"place": "front Ada", "columns": 2, "rows": 1, "slot_width": 0.4, "slot_height": 0.9}),
    );
    assert!(
        now(&sized).ends_with("its slots 40 by 90 cm, flat"),
        "{sized}"
    );
}

#[test]
fn the_wall_tools_refuse_arguments_they_cannot_take_and_sessions_bound_to_no_space_and_say_how_many_panels_float()
 {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = tomori_server::test_support::TestClient::open(server.address);
    ada.hello_projected("ada", "Ada");
    create(&mut ada, "Studio");
    let token = token(registrations(&backend, 1)[0].1.as_ref().unwrap());
    let sessions = backend.started();
    let session = id(&sessions[0]);
    let home = [0.0, 1.6, 0.0];
    // Ada looks straight up, at no wall.
    ada.send(&ClientMessage::Presence {
        head: looking(home, [0.0, 9.0, -0.01]),
        left: None,
        right: None,
        hands: None,
        carrying: None,
    });
    let tool =
        |name: &str, arguments: Value| call(&server, &token, Some(&session), name, arguments);
    let walls = || {
        call(&server, &token, Some(&session), "presence", json!({}))["structuredContent"]["walls"]
            .clone()
    };
    eventually(|| (walls()[0]["name"] == "Tomori wall").then_some(()));
    eventually(|| {
        let presence = call(&server, &token, Some(&session), "presence", json!({}));
        presence["structuredContent"]["people"][0]
            .get("head")
            .is_some()
            .then_some(())
    });
    // A session bound to no space changes no wall.
    for name in [
        "set_wall",
        "add_wall",
        "remove_wall",
        "create_layout",
        "remove_layout",
    ] {
        let result = call(
            &server,
            &token,
            Some("ses_nobody"),
            name,
            json!({"wall": "Tomori wall", "group": "Tomori wall", "walls": [{"id": "a"}]}),
        );
        assert_eq!(result["isError"], true, "{name}: {result}");
    }
    // Arguments of the wrong kind or size are refused, saying which.
    let long = "x".repeat(2_000);
    for (name, arguments, error) in [
        (
            "set_wall",
            json!({"wall": long}),
            "The wall is too long; the limit is 1024 bytes.",
        ),
        (
            "set_wall",
            json!({"wall": "Tomori wall", "place": 5}),
            "place is text.",
        ),
        (
            "set_wall",
            json!({"wall": "Tomori wall", "place": long}),
            "The place is too long; the limit is 1024 bytes.",
        ),
        (
            "set_wall",
            json!({"wall": "Tomori wall", "width": "wide"}),
            "width is a number.",
        ),
        (
            "set_wall",
            json!({"wall": "Tomori wall", "radius": "round"}),
            "radius is a number.",
        ),
        (
            "set_wall",
            json!({"wall": "Tomori wall", "scale": "big"}),
            "scale is a number.",
        ),
        (
            "set_wall",
            json!({"wall": "Tomori wall", "rows": -1}),
            "rows is a whole number.",
        ),
        (
            "set_wall",
            json!({"wall": "Tomori wall", "width": 0}),
            "A width is a number of metres.",
        ),
        ("add_wall", json!({"place": 5}), "place is text."),
        (
            "add_wall",
            json!({"place": "front Ada", "columns": "four"}),
            "columns is a whole number.",
        ),
        (
            "add_wall",
            json!({"place": "front Ada", "columns": 8, "rows": 4}),
            "A wall has from 1 to 8 columns and from 1 to 4 rows of slots, and at most 24 slots.",
        ),
        (
            "add_wall",
            json!({"place": "front Zed"}),
            "Nobody called \"Zed\" in this space has reported where they are.",
        ),
        ("remove_wall", json!({}), "This tool needs wall, as text."),
        (
            "remove_layout",
            json!({"group": long}),
            "The group is too long; the limit is 1024 bytes.",
        ),
        (
            "remove_layout",
            json!({"group": "attic"}),
            "There is no wall called \"attic\" here: name one as presence does, or say looking or near, with a name or for whoever asked.",
        ),
        (
            "remove_wall",
            json!({"wall": long}),
            "The wall is too long; the limit is 1024 bytes.",
        ),
        (
            "remove_wall",
            json!({"wall": "wall attic"}),
            "There is no wall called \"attic\" here.",
        ),
    ] {
        assert_eq!(failure(&tool(name, arguments)), error, "{name}");
    }
    // Two walls in front of her, laid out as asked, one curved; looking at none, the one nearest her is hers.
    let first = tool(
        "add_wall",
        json!({"place": "front Ada", "columns": 1, "rows": 1, "scale": 1.5, "radius": 0}),
    );
    assert_ne!(first["isError"], true, "{first}");
    assert_eq!(
        first["structuredContent"]["now"],
        "1 slot wide and 1 high, 1.0 by 0.7 m, its slots 91 by 54 cm, flat"
    );
    let near = tool("set_wall", json!({"wall": "near Ada", "columns": 2}));
    assert_eq!(near["structuredContent"]["name"], "added wall 1", "{near}");
    let looking = tool("set_wall", json!({"wall": "looking Ada", "rows": 2}));
    assert_eq!(
        looking["structuredContent"]["name"], "added wall 1",
        "{looking}"
    );
    let second = tool(
        "add_wall",
        json!({"place": "front Ada", "radius": 3, "columns": 2, "rows": 1}),
    );
    assert_ne!(second["isError"], true, "{second}");
    // Removed, each says how many of its panels float: none, or two.
    agent_replies(
        &sessions[0],
        "m1",
        "```panel\ntitle: One\nplace: wall added wall 1\n---\nA.\n```\n\n```panel\ntitle: Two\nplace: wall added wall 1\n---\nB.\n```\n",
    );
    eventually(|| (walls()[1]["used"].as_array()?.len() == 2).then_some(()));
    let removed = tool("remove_wall", json!({"wall": "added wall 2"}));
    assert_eq!(removed["structuredContent"]["floating"], 0);
    assert_eq!(
        removed["structuredContent"]["next"],
        "Tell everyone in the space that you removed the added wall 2."
    );
    let removed = tool("remove_wall", json!({"wall": "added wall 1"}));
    assert_eq!(
        removed["structuredContent"]["next"],
        "Tell everyone in the space that you removed the added wall 1; the 2 panels on it float where they were."
    );
}

#[test]
fn the_agent_tilts_a_wall_someone_looks_at_and_makes_a_big_wall_stand_as_far_as_it_reads() {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = tomori_server::test_support::TestClient::open(server.address);
    ada.hello_projected("ada", "Ada");
    create(&mut ada, "Studio");
    let token = token(registrations(&backend, 1)[0].1.as_ref().unwrap());
    let sessions = backend.started();
    let session = id(&sessions[0]);
    let home = [0.0, 1.6, 0.0];
    ada.send(&ClientMessage::Presence {
        head: looking(home, [0.0, 1.6, -1.0]),
        left: None,
        right: None,
        hands: None,
        carrying: None,
    });
    let tool =
        |name: &str, arguments: Value| call(&server, &token, Some(&session), name, arguments);
    let walls = || {
        call(&server, &token, Some(&session), "presence", json!({}))["structuredContent"]["walls"]
            .clone()
    };
    eventually(|| {
        let presence = call(&server, &token, Some(&session), "presence", json!({}));
        presence["structuredContent"]["people"][0]
            .get("head")
            .is_some()
            .then_some(())
    });
    let now = |result: &Value| {
        assert_ne!(result["isError"], true, "{result}");
        result["structuredContent"]["now"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let added = tool("add_wall", json!({"place": "front Ada"}));
    let wall = added["structuredContent"]["wall"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(walls()[1]["tilt"], 0.0);
    // "Tilt this wall down a bit": the wall she is looking at, its top towards her.
    let down = tool("set_wall", json!({"wall": "looking Ada", "tilt": 10}));
    assert_eq!(down["structuredContent"]["name"], "added wall 1", "{down}");
    assert!(now(&down).ends_with("flat, tilted 10° down"), "{down}");
    assert_eq!(walls()[1]["tilt"], 10.0);
    // Her client is sent its tilt.
    let shown = ada.wait_for(|message| match message {
        tomori_protocol::ServerMessage::Projection { projection } => projection
            .wall(&wall)
            .filter(|wall| wall.tilt.is_some())
            .cloned(),
        _ => None,
    });
    assert_eq!(shown.tilt, Some(10.0));
    // Back, facing up, and upright again.
    let up = tool("set_wall", json!({"wall": wall, "tilt": -20}));
    assert!(now(&up).ends_with("flat, tilted 20° up"), "{up}");
    let upright = tool("set_wall", json!({"wall": wall, "tilt": 0}));
    assert!(now(&upright).ends_with("flat"), "{upright}");
    assert_eq!(walls()[1]["tilt"], 0.0);
    // Beyond 45°, or not a number, it is refused, saying why.
    assert_eq!(
        failure(&tool("set_wall", json!({"wall": wall, "tilt": 60}))),
        "A wall tilts at most 45° from upright, either way."
    );
    assert_eq!(
        failure(&tool("set_wall", json!({"wall": wall, "tilt": "down"}))),
        "tilt is a number."
    );
    assert_eq!(
        failure(&tool("set_wall", json!({"wall": wall, "tilt": 1e300}))),
        "tilt is a number of degrees, from -45 to 45."
    );
    assert_eq!(
        failure(&tool(
            "add_wall",
            json!({"place": "front Ada", "tilt": "down"})
        )),
        "tilt is a number."
    );
    // A new wall may be tilted as it is added.
    let tilted = tool(
        "add_wall",
        json!({"place": "front Ada", "columns": 1, "rows": 1, "tilt": -15}),
    );
    assert!(now(&tilted).ends_with("flat, tilted 15° up"), "{tilted}");
    // A big wall, five times a standard slot, for a dashboard far away, stands two and a half times as far as a
    // standard one, its text as large to her as a wall of twice a standard slot's at 1.8 m.
    let big = tool(
        "add_wall",
        json!({"place": "front Ada", "columns": 2, "rows": 1, "scale": 5}),
    );
    assert!(
        now(&big).starts_with("2 slots wide and 1 high, 6.7 by 2.3 m, its slots 302 by 180 cm"),
        "{big}"
    );
    let big = big["structuredContent"]["wall"]
        .as_str()
        .unwrap()
        .to_owned();
    let listed = walls();
    let position = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|listed| listed["id"] == big.as_str())
        .unwrap()["position"]
        .clone();
    assert!(
        (position[2].as_f64().unwrap() + 4.5).abs() < 0.05,
        "{position}"
    );
    // Beyond five times, refused.
    let result = tool("set_wall", json!({"wall": big, "scale": 6}));
    assert_eq!(
        failure(&result),
        "A wall's slots are from half to five times a standard slot's size."
    );
}
