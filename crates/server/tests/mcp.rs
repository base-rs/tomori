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
use tomori_protocol::{ClientMessage, SpeechSetting};
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
            "set_speech",
            "set_environment",
            "pairing",
            "confirm_pairing",
            "decline_pairing"
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
    for tool in ["space", "set_speech", "set_environment"] {
        let arguments = json!({"speech": "brief", "environment": "minimal"});
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
    for tool in ["space", "set_speech", "set_environment"] {
        assert_eq!(
            failure(&call(
                &server,
                &token,
                Some(&one),
                tool,
                json!({"speech": "brief", "environment": "minimal"})
            )),
            "This session is bound to more than one open Tomori space, so Tomori cannot tell which one to act on.",
            "{tool}"
        );
    }
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
