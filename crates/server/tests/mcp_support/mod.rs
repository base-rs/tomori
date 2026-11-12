// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What the tests of Tomori's MCP tools share: a server serving them, a tool called as OpenCode calls it, and its result read. Each test file declares this module `pub`, so that a helper it does not use is not dead code.
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

use serde_json::{Value, json};
use tomori_backend::Event;
use tomori_protocol::{ClientMessage, QueueState};
use tomori_server::Server;
use tomori_server::fake::{FAKE_BACKEND_NAME, FakeBackend, FakeSession};
use tomori_server::test_support::{TestClient, TestServer, TextTranscriber};

/// A server over `backend` serving Tomori's tools.
pub fn start(backend: &FakeBackend) -> TestServer {
    TestServer::with_tools(backend.clone(), |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default()))
    })
}

/// A participant `name` of `server`, whose client reads what is projected in its space.
pub fn projected(server: &TestServer, participant: &str, name: &str) -> TestClient {
    let mut client = TestClient::open(server.address);
    client.hello_projected(participant, name);
    client
}

/// Call the tool `name` with `arguments` for the agent of `session`, as OpenCode does, and return its result.
pub fn call(
    server: &TestServer,
    token: &str,
    session: &str,
    name: &str,
    arguments: Value,
) -> Value {
    call_from(server, token, Some(session), name, arguments)
}

/// Call the tool `name` with `arguments`, as OpenCode does for the agent of `session`, or without naming a session, and return its result.
pub fn call_from(
    server: &TestServer,
    token: &str,
    session: Option<&str>,
    name: &str,
    arguments: Value,
) -> Value {
    let mut params = json!({ "name": name, "arguments": arguments });
    if let Some(session) = session {
        params["_meta"] = json!({ "ai.opencode/sessionID": session });
    }
    call_with(server, token, &params)
}

/// Call a tool with the `tools/call` parameters `params`, as given, and return its result.
pub fn call_with(server: &TestServer, token: &str, params: &Value) -> Value {
    let body =
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params }).to_string();
    let mut stream = TcpStream::connect(server.tools.unwrap()).unwrap();
    write!(
        stream,
        "POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    serde_json::from_str::<Value>(body).unwrap()["result"].clone()
}

/// The structured content of `result`, which succeeded, and which its text repeats.
pub fn content(result: &Value) -> &Value {
    assert_ne!(result["isError"], true, "{result}");
    assert_eq!(
        result["content"][0]["text"],
        result["structuredContent"].to_string()
    );
    &result["structuredContent"]
}

/// The text of `result`, which failed.
pub fn failure(result: &Value) -> String {
    assert_eq!(result["isError"], true, "{result}");
    result["content"][0]["text"].as_str().unwrap().to_owned()
}

/// Create a space of the fake backend titled `title`, in its default environment; the creator joins it. Returns its identifier.
pub fn create(client: &mut TestClient, title: &str) -> String {
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

/// Say `words` as the utterance `utterance` and wait until they are delivered to the agent, so that they are the words it was given last.
pub fn say(client: &mut TestClient, utterance: &str, words: &str) {
    client.say(utterance, words);
    client.space(|state| {
        state
            .queue
            .iter()
            .any(|entry| entry.text == words && entry.state == QueueState::Delivered)
    });
}

/// The agent of `session` replies `text` as its message `message`, whole.
pub fn agent_replies(session: &FakeSession, message: &str, text: &str) {
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
