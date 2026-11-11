// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The farmyard's hens offer the space's agent an action, as Tomori's own server runs them: pointed at, a hen's id
//! names it in the note before someone's words, `inspect` lists it with its parameter, and `act`, through Tomori's
//! MCP endpoint, has the farmyard's server side make her peck, which it answers, and the space's status wall notes
//! what was done and for whom. See `design/environment-actions.md` and `design/farmyard.md`.
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

use serde_json::{Value, json};
use tomori_protocol::{ClientMessage, QueueState, Resource};
use tomori_server::fake::{FAKE_BACKEND_NAME, FakeBackend};
use tomori_server::test_support::{TestServer, TextTranscriber, eventually};
use tomori_server::{Server, WorkspaceRoots};

/// Call the tool `name` with `arguments` for the agent of `session`.
fn call(server: &TestServer, token: &str, session: &str, name: &str, arguments: Value) -> Value {
    let body = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": arguments, "_meta": { "ai.opencode/sessionID": session } },
    })
    .to_string();
    let mut stream = TcpStream::connect(server.tools.unwrap()).unwrap();
    write!(
        stream,
        "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nAuthorization: Bearer {token}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    serde_json::from_str::<Value>(body).unwrap()["result"].clone()
}

#[test]
fn the_agent_has_a_farmyard_hen_peck_when_someone_points_at_her_and_asks() {
    let environments = tomori_server_app::builder().environments().to_vec();
    let backend = FakeBackend::new();
    let server = TestServer::with_tools(backend.clone(), move |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default()))
            .with_environments(environments)
            .with_workspace_roots(
                WorkspaceRoots::new([env!("CARGO_MANIFEST_DIR")])
                    .expect("the crate's directory exists"),
            )
    });
    let mut ada = server.connect("ada", "Ada");
    ada.send(&ClientMessage::CreateSpace {
        title: "Farm".into(),
        backend: FAKE_BACKEND_NAME.into(),
        directory: env!("CARGO_MANIFEST_DIR").into(),
        session: None,
        environment: Some(tomori_environment_farmyard::ID.into()),
        instructions: None,
    });
    ada.space(|state| state.title == "Farm");
    let session = eventually(|| backend.started().into_iter().next());
    let me = tomori_backend::Session::id(&session);
    let token = eventually(|| server.tools_token());
    // Once the side has described its hens: the flock's names follow the space, so any of them.
    let inspected = |id: &str| call(&server, &token, &me, "inspect", json!({ "id": id }));
    eventually(|| {
        tomori_environment_farmyard::flock::NAMES
            .iter()
            .find_map(|name| {
                inspected(&format!("farmyard:hen/{}", name.to_lowercase()))
                    .get("structuredContent")
                    .cloned()
            })
    });

    // Ada points at the flock as she asks: the note names each hen's id and that she offers to peck.
    let objects: Vec<Resource> = (0..tomori_environment_farmyard::flock::FEWEST)
        .map(|index| Resource::object(format!("chicken-{index}")))
        .collect();
    let words = "can you make her peck";
    ada.say_attending("u1", words, objects, None);
    let mut delivered = false;
    ada.space(|state| {
        delivered |= state
            .queue
            .iter()
            .any(|entry| entry.text == words && entry.state == QueueState::Delivered);
        delivered
    });
    let prompt = eventually(|| {
        session
            .log()
            .prompts
            .into_iter()
            .map(|prompt| prompt.text)
            .find(|text| text.contains("offers peck"))
    });
    let ids: Vec<String> = prompt
        .split("(id ")
        .skip(1)
        .map(|rest| rest.split(';').next().unwrap().to_owned())
        .collect();
    assert_eq!(
        ids.len(),
        tomori_environment_farmyard::flock::FEWEST,
        "{prompt}"
    );
    assert!(
        ids.iter().all(|id| id.starts_with("farmyard:hen/")),
        "{prompt}"
    );

    // Inspected, a hen lists the action with its parameter.
    let hen = inspected(&ids[0])["structuredContent"].clone();
    assert_eq!(hen["actions"][0]["name"], "peck");
    assert_eq!(hen["actions"][0]["consequential"], false);
    assert_eq!(hen["actions"][0]["parameters"][0]["name"], "pecks");
    assert_eq!(hen["actions"][0]["parameters"][0]["most"], 8.0);

    // Taken on Ada's words, without naming her: a hen on the ground pecks, as the farmyard answers.
    let done = ids
        .iter()
        .map(|id| {
            call(
                &server,
                &token,
                &me,
                "act",
                json!({ "id": id, "action": "peck", "parameters": { "pecks": 2 } }),
            )["structuredContent"]
                .clone()
        })
        .inspect(|result| {
            // A hen up in the coop says why she cannot; none goes unanswered.
            assert_ne!(result["outcome"], "no answer", "{result}");
        })
        .find(|result| result["outcome"] == "done")
        .expect("a hen on the ground pecks");
    assert!(
        done["message"]
            .as_str()
            .unwrap()
            .ends_with("pecks at the ground 2 times."),
        "{done}"
    );
    assert_eq!(done["for"], "Ada");
    // The status wall notes what was done, and for whom.
    let notice = call(
        &server,
        &token,
        &me,
        "panels",
        json!({ "panel": "Actions" }),
    );
    let body = notice["structuredContent"]["panels"][0]["body"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        body.starts_with("Tomori took peck on farmyard:hen/"),
        "{body}"
    );
    assert!(body.contains(", as Ada spoke: "), "{body}");
    // A parameter it does not take is refused before the farmyard is asked.
    let refused = call(
        &server,
        &token,
        &me,
        "act",
        json!({ "id": ids[0], "action": "peck", "parameters": { "pecks": 1.5 } }),
    );
    assert_eq!(refused["isError"], true);
    assert_eq!(
        refused["content"][0]["text"],
        "pecks is a number from 1 to 8, in steps of 1."
    );
}
