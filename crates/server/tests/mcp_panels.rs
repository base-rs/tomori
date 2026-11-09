// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Panels the agent updates in place: a block with the id of a panel shown, and the `panels` and `update_panel` tools,
//! with which the agent reads a list back and ticks its items. See `design/projection.md` and `design/mcp.md`.
use std::sync::Arc;

use serde_json::{Value, json};
use tomori_backend::Event;
use tomori_protocol::{ClientMessage, Panel, Place, ServerMessage};
use tomori_server::Server;
use tomori_server::fake::{FAKE_BACKEND_NAME, FakeBackend, FakeSession};
use tomori_server::test_support::{TestClient, TestServer, TextTranscriber, eventually};

/// A server over `backend` serving Tomori's tools.
fn start(backend: &FakeBackend) -> TestServer {
    TestServer::with_tools(backend.clone(), |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default()))
    })
}

/// A participant `name` of `server`, whose client reads what is projected in its space.
fn projected(server: &TestServer, participant: &str, name: &str) -> TestClient {
    let mut client = TestClient::open(server.address);
    client.hello_projected(participant, name);
    client
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

/// Call the tool `name` with `arguments` for the agent of `session`, as OpenCode does, and return its result.
fn call(server: &TestServer, token: &str, session: &str, name: &str, arguments: Value) -> Value {
    let params = json!({
        "name": name,
        "arguments": arguments,
        "_meta": { "ai.opencode/sessionID": session },
    });
    let body =
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params }).to_string();
    let mut stream = std::net::TcpStream::connect(server.tools.unwrap()).unwrap();
    std::io::Write::write_all(
        &mut stream,
        format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .as_bytes(),
    )
    .unwrap();
    let mut response = String::new();
    std::io::Read::read_to_string(&mut stream, &mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    serde_json::from_str::<Value>(body).unwrap()["result"].clone()
}

/// A tool's structured result, which must not be a failure.
fn content(result: &Value) -> &Value {
    assert_ne!(result["isError"], true, "{result}");
    &result["structuredContent"]
}

/// The text of a tool's failure.
fn failure(result: &Value) -> &str {
    assert_eq!(result["isError"], true, "{result}");
    result["content"][0]["text"].as_str().unwrap()
}

/// The agent of `session` replies `text` as the message `message`.
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

/// The panels of the next projection `client` is sent for which `wanted` holds.
fn panels(client: &TestClient, wanted: impl Fn(&[Panel]) -> bool) -> Vec<Panel> {
    client.wait_for(|message| match message {
        ServerMessage::Projection { projection } if wanted(&projection.panels) => {
            Some(projection.panels.clone())
        }
        _ => None,
    })
}

/// The items of a `panels` or `update_panel` result's panel, each as its number, whether it is ticked and its text.
fn items(described: &Value) -> Vec<(u64, Value, String)> {
    described["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["item"].as_u64().unwrap(),
                item["checked"].clone(),
                item["text"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn the_agent_updates_a_checklist_in_place_by_its_id_and_ticks_its_items_through_the_tools() {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = projected(&server, "ada", "Ada");
    let space = create(&mut ada, "Planning");
    let token = eventually(|| server.tools_token());
    let session = backend.started()[0].clone();
    let agent = tomori_backend::Session::id(&session);

    // An agenda, with an id, goes onto the Tomori wall.
    agent_replies(
        &session,
        "m1",
        "Here is the agenda.\n\n```panel\nid: agenda\ntitle: Agenda\n---\n- [ ] Pairing window\n- [ ] Panel ids\n  - [ ] Checklists\n```\n",
    );
    let shown = panels(&ada, |panels| panels.len() == 1);
    let agenda = shown[0].clone();
    assert_eq!(agenda.slug.as_deref(), Some("agenda"));
    assert_eq!(
        agenda.place,
        Place::Wall {
            wall: "tomori".into(),
            slot: 0
        }
    );

    // The agent reads it back, by its id and items.
    let listed = content(&call(&server, &token, &agent, "panels", json!({}))).clone();
    assert_eq!(listed["space"], space);
    let described = &listed["panels"][0];
    assert_eq!(described["panel"], agenda.id);
    assert_eq!(described["id"], "agenda");
    assert_eq!(described["kind"], "text");
    assert_eq!(described["wall_name"], "Tomori wall");
    assert_eq!(described["body"], agenda.body);
    assert_eq!(
        items(described),
        [
            (1, json!(false), "Pairing window".to_owned()),
            (2, json!(false), "Panel ids".to_owned()),
            (3, json!(false), "Checklists".to_owned()),
        ]
    );
    assert!(described.get("updated_seconds_ago").is_none());

    // Ticked by number and by text, in place: the same panel, everyone sees it.
    let update = |arguments: Value| call(&server, &token, &agent, "update_panel", arguments);
    let ticked = content(&update(
        json!({"panel": "agenda", "check": [1, "checklists"]}),
    ))
    .clone();
    assert_eq!(ticked["panel"], agenda.id);
    assert_eq!(ticked["id"], "agenda");
    assert_eq!(ticked["changed"], true);
    assert_eq!(
        items(&ticked)
            .iter()
            .map(|(_, checked, _)| checked.clone())
            .collect::<Vec<_>>(),
        [json!(true), json!(false), json!(true)]
    );
    let shown = panels(&ada, |panels| {
        panels.len() == 1 && panels[0].body.starts_with("- [x]")
    });
    assert_eq!(shown[0].id, agenda.id);
    assert_eq!(shown[0].place, agenda.place);
    assert_eq!(
        shown[0].body,
        "- [x] Pairing window\n- [ ] Panel ids\n  - [x] Checklists"
    );
    // Unticked by its title, and the same again changes nothing.
    let unticked = content(&update(json!({"panel": "Agenda", "uncheck": ["pairing"]}))).clone();
    assert_eq!(unticked["changed"], true);
    assert_eq!(
        content(&update(json!({"panel": agenda.id, "uncheck": [1]})))["changed"],
        false
    );
    // A new title and body, with an item that had no box given one.
    let revised = content(&update(json!({
        "panel": "agenda",
        "title": "Agenda, revised",
        "body": "- [x] Pairing window\n- Panel ids\n- [ ] Checklists",
        "check": ["panel ids"],
    })))
    .clone();
    assert_eq!(
        items(&revised)
            .iter()
            .map(|(_, checked, text)| (checked.clone(), text.as_str()))
            .collect::<Vec<_>>(),
        [
            (json!(true), "Pairing window"),
            (json!(true), "Panel ids"),
            (json!(false), "Checklists")
        ]
    );
    let shown = panels(&ada, |panels| {
        panels.len() == 1 && panels[0].title == "Agenda, revised"
    });
    assert_eq!(shown[0].id, agenda.id);
    let listed = content(&call(&server, &token, &agent, "panels", json!({}))).clone();
    assert!(listed["panels"][0]["updated_seconds_ago"].is_u64());

    // A later block with the same id updates it too, rather than projecting another.
    agent_replies(
        &session,
        "m2",
        "Updated.\n\n```panel\nid: agenda\ntitle: Agenda\n---\n- [x] Pairing window\n- [x] Panel ids\n- [x] Checklists\n```\n",
    );
    let shown = panels(&ada, |panels| {
        panels.len() == 1 && panels[0].body.ends_with("- [x] Checklists")
    });
    assert_eq!(shown[0].id, agenda.id);
    assert_eq!(shown[0].title, "Agenda");

    // What it cannot do, it says, and changes nothing.
    assert_eq!(
        failure(&update(json!({"panel": "agenda"}))),
        "Say what to change: title, body, check, uncheck or place."
    );
    assert_eq!(
        failure(&update(json!({"panel": "agenda", "check": [9]}))),
        "The panel has 3 items, numbered from 1, so it has no item 9."
    );
    assert_eq!(
        failure(&update(
            json!({"panel": "agenda", "check": [1], "uncheck": ["pairing"]})
        )),
        "An item cannot be both checked and unchecked."
    );
    assert_eq!(
        failure(&update(json!({"panel": "agenda", "check": [0]}))),
        "Each of check is an item's number, from 1, or its text."
    );
    assert_eq!(
        failure(&update(json!({"panel": "agenda", "check": "1"}))),
        "check is a list of items."
    );
    assert_eq!(
        failure(&update(json!({"panel": "agenda", "check": vec![1; 31]}))),
        "Tomori ticks and unticks at most 30 items in one call."
    );
    assert_eq!(
        failure(&update(json!({"panel": "agenda", "title": 3}))),
        "title is text."
    );
    assert_eq!(
        failure(&update(json!({"panel": "agenda", "title": "", "body": ""}))),
        "A panel needs a title or a body."
    );
    assert!(
        failure(&update(json!({"panel": "agenda", "place": "behind"})))
            .starts_with("Tomori does not know where \"behind\" is")
    );
    assert_eq!(
        failure(&update(json!({"panel": "nothing", "check": [1]}))),
        "There is no such panel here."
    );
    assert_eq!(
        failure(&update(json!({"panel": "agenda", "uncheck": [99]}))),
        "The panel has 3 items, numbered from 1, so it has no item 99."
    );
    assert_eq!(
        failure(&update(json!({"panel": "agenda", "uncheck": "x"}))),
        "uncheck is a list of items."
    );
    assert_eq!(
        failure(&update(json!({"panel": "agenda", "place": 5}))),
        "place is text."
    );
    assert_eq!(
        failure(&update(json!({"panel": "agenda", "place": "near Zed"}))),
        "Nobody called \"Zed\" in this space has reported where they are."
    );
    assert_eq!(
        failure(&update(json!({"check": [1]}))),
        "This tool needs panel, as text."
    );
    assert_eq!(
        failure(&update(json!({"panel": "x".repeat(2_000), "check": [1]}))),
        "The panel is too long; the limit is 1024 bytes."
    );
    assert_eq!(
        failure(&update(
            json!({"panel": "agenda", "body": "x".repeat(9_000)})
        )),
        "The body is too long; the limit is 8192 bytes."
    );
    // A code panel is not updated in place.
    agent_replies(&session, "m3", "```rust\nfn a() {}\nfn b() {}\n```\n");
    let shown = panels(&ada, |panels| panels.len() == 2);
    let code = shown.iter().find(|panel| panel.code.is_some()).unwrap();
    assert_eq!(
        failure(&update(json!({"panel": code.id, "title": "x"}))),
        "A code panel cannot be updated in place: project a new one."
    );
    let listed = content(&call(&server, &token, &agent, "panels", json!({}))).clone();
    let described = listed["panels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|panel| panel["kind"] == "code")
        .unwrap();
    assert_eq!(described["language"], "rust");
    assert!(described.get("body").is_none());
}

#[test]
fn an_update_by_the_tool_moves_a_panel_only_where_place_says_and_never_one_put_by_hand() {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = projected(&server, "ada", "Ada");
    create(&mut ada, "Planning");
    let token = eventually(|| server.tools_token());
    let session = backend.started()[0].clone();
    let agent = tomori_backend::Session::id(&session);
    agent_replies(
        &session,
        "m1",
        "```panel\nid: todo\n---\n- [ ] One\n```\n\n```panel\ntitle: Other\n---\nText.\n```\n",
    );
    let shown = panels(&ada, |panels| panels.len() == 2);
    let todo = shown
        .iter()
        .find(|panel| panel.slug.is_some())
        .unwrap()
        .clone();
    let update = |arguments: Value| call(&server, &token, &agent, "update_panel", arguments);
    // Onto another slot of the Tomori wall, by the forms move_panel takes.
    let moved = content(&update(
        json!({"panel": "todo", "check": [1], "place": "wall tomori"}),
    ))
    .clone();
    assert_eq!(moved["changed"], true);
    assert!(moved.get("stayed").is_none());
    // A panel without an id is updated by its title, and has none.
    let other = content(&update(json!({"panel": "Other", "body": "- [ ] New"}))).clone();
    assert_eq!(other["changed"], true);
    assert!(other.get("id").is_none());
    // Put somewhere by a participant, it stays there, and says so, while its text changes.
    ada.send(&ClientMessage::PlacePanel {
        panel: todo.id.clone(),
        wall: Some("tomori".into()),
        slot: Some(5),
        pose: None,
    });
    panels(&ada, |panels| {
        panels
            .iter()
            .any(|panel| panel.id == todo.id && panel.pinned)
    });
    let kept = content(&update(
        json!({"panel": "todo", "uncheck": [1], "place": "wall tomori"}),
    ))
    .clone();
    assert_eq!(kept["changed"], true);
    assert_eq!(
        kept["place"],
        json!({"kind": "wall", "wall": "tomori", "slot": 5})
    );
    assert_eq!(
        kept["stayed"],
        "A participant put that panel where it is, so it stays there; its text changed."
    );
    // Left floating, it is listed as floating.
    ada.send(&ClientMessage::PlacePanel {
        panel: todo.id.clone(),
        wall: None,
        slot: None,
        pose: Some(tomori_protocol::Pose::upright_facing(
            [0.5, 1.5, -1.5],
            [0.0, 1.6, 0.0],
        )),
    });
    panels(&ada, |panels| {
        panels
            .iter()
            .any(|panel| panel.id == todo.id && matches!(panel.place, Place::Free { .. }))
    });
    let listed = content(&call(&server, &token, &agent, "panels", json!({}))).clone();
    let floating = listed["panels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|panel| panel["id"] == "todo")
        .unwrap();
    assert_eq!(floating["floating"], true);
    assert!(floating.get("wall").is_none());
    // Dismissed, an update by the tool finds nothing, while a block with its id projects it anew.
    ada.send(&ClientMessage::DismissPanel {
        panel: todo.id.clone(),
    });
    panels(&ada, |panels| panels.len() == 1);
    assert_eq!(
        failure(&update(json!({"panel": "todo", "check": [1]}))),
        "There is no such panel here."
    );
    agent_replies(&session, "m2", "```panel\nid: todo\n---\n- [x] One\n```\n");
    let shown = panels(&ada, |panels| panels.len() == 2);
    let again = shown
        .iter()
        .find(|panel| panel.slug.as_deref() == Some("todo"))
        .unwrap();
    assert_ne!(again.id, todo.id);
}
