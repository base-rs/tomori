// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Panels the agent updates in place: a block with the id of a panel shown, and the `panels` and `update_panel` tools,
//! with which the agent reads a list back and ticks its items. See `design/projection.md` and `design/mcp.md`.
use std::sync::Arc;

use serde_json::{Value, json};
use tomori_backend::Event;
use tomori_protocol::{ClientMessage, Panel, Place, Pose, ServerMessage};
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

    // The agent lists it, briefly, with how much it holds and the start of its text, and reads it back whole by its
    // id, with its body and items.
    let listed = content(&call(&server, &token, &agent, "panels", json!({}))).clone();
    assert_eq!(listed["space"], space);
    let brief = &listed["panels"][0];
    assert_eq!(brief["panel"], agenda.id);
    assert_eq!(brief["id"], "agenda");
    assert_eq!(brief["line_count"], agenda.body.lines().count());
    assert!(brief["item_count"].as_u64().unwrap() > 0);
    assert!(brief.get("items").is_none());
    assert_eq!(
        (&listed["total"], &listed["offset"]),
        (&json!(1), &json!(0))
    );
    assert!(listed.get("next").is_none());
    let excerpt = brief["excerpt"].as_str().unwrap();
    assert!(excerpt.contains("Pairing window") && !excerpt.contains('\n'));
    assert!(brief.get("body").is_none());
    let read = content(&call(
        &server,
        &token,
        &agent,
        "panels",
        json!({"panel": "agenda"}),
    ))
    .clone();
    assert_eq!(read["space"], space);
    assert_eq!(read["panels"].as_array().unwrap().len(), 1);
    let described = &read["panels"][0];
    assert_eq!(described["panel"], agenda.id);
    assert_eq!(described["id"], "agenda");
    assert_eq!(described["kind"], "text");
    assert_eq!(described["wall_name"], "status wall (the Tomori wall)");
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
    assert!(described["line_count"].as_u64().unwrap() > 1);
    // Read by its identity, a code panel says its language, and no body, as listed: its code is in the transcript.
    let read = content(&call(
        &server,
        &token,
        &agent,
        "panels",
        json!({"panel": code.id}),
    ))
    .clone();
    assert_eq!(read["panels"][0]["language"], "rust");
    assert!(read["panels"][0].get("body").is_none());
    assert!(read["panels"][0].get("items").is_none());
    // A panel it names that is not here is refused, saying so.
    let missing = call(
        &server,
        &token,
        &agent,
        "panels",
        json!({"panel": "nothing"}),
    );
    assert_eq!(failure(&missing), "There is no such panel here.");
    let number = call(&server, &token, &agent, "panels", json!({"panel": 5}));
    assert_eq!(failure(&number), "panel is text.");
    // Listed from an offset, as a space of many panels is paged through: past the first, the second; past both, none.
    let paged = content(&call(
        &server,
        &token,
        &agent,
        "panels",
        json!({"offset": 1}),
    ))
    .clone();
    assert_eq!((&paged["total"], &paged["offset"]), (&json!(2), &json!(1)));
    assert_eq!(paged["panels"].as_array().unwrap().len(), 1);
    let past = content(&call(
        &server,
        &token,
        &agent,
        "panels",
        json!({"offset": 7}),
    ))
    .clone();
    assert_eq!((&past["total"], &past["offset"]), (&json!(2), &json!(2)));
    assert!(past["panels"].as_array().unwrap().is_empty());
    let negative = call(&server, &token, &agent, "panels", json!({"offset": -1}));
    assert_eq!(failure(&negative), "offset is a whole number, from 0.");
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
    // Carried by a participant, it stays in their hand, by the tool and by a block with its id, while its text changes.
    let carrying = |panel: Option<&str>| ClientMessage::Presence {
        head: tomori_protocol::Pose::upright_facing([0.0, 1.6, 0.0], [0.0, 1.6, -1.0]),
        left: None,
        right: None,
        hands: None,
        carrying: panel.map(str::to_owned),
        carried: Vec::new(),
        rays: None,
    };
    ada.send(&carrying(Some(&todo.id)));
    let held = eventually(|| {
        let result = update(json!({"panel": "todo", "body": "- [ ] One", "place": "front Ada"}));
        result["structuredContent"]
            .get("stayed")
            .is_some()
            .then(|| result["structuredContent"].clone())
    });
    assert_eq!(
        held["stayed"],
        "Someone is carrying that panel now, so it stays in their hand; its text changed."
    );
    assert_eq!(held["place"]["kind"], "wall");
    agent_replies(
        &session,
        "m1-carried",
        "```panel\nid: todo\nplace: front Ada\n---\n- [x] One, carried\n```\n",
    );
    let shown = panels(&ada, |panels| {
        panels
            .iter()
            .any(|panel| panel.id == todo.id && panel.body == "- [x] One, carried")
    });
    let carried = shown.iter().find(|panel| panel.id == todo.id).unwrap();
    assert!(matches!(carried.place, Place::Wall { .. }));
    ada.send(&carrying(None));
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

#[test]
fn a_participant_s_tick_reaches_the_agent_through_panels_and_before_the_next_words_without_a_turn()
{
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
        "```panel\nid: agenda\ntitle: Agenda\n---\n- [ ] Pairing window\n- [ ] Panel ids\n```\n",
    );
    let agenda = panels(&ada, |panels| panels.len() == 1)[0].clone();

    // Ada points at the second item and presses A: it is ticked for everyone, in place.
    assert!(agenda.version > 0);
    ada.send(&ClientMessage::CheckPanelItem {
        panel: agenda.id.clone(),
        line: 1,
        checked: true,
        revision: Some(agenda.version),
    });
    let shown = panels(&ada, |panels| {
        panels.len() == 1 && panels[0].body.ends_with("- [x] Panel ids")
    });
    assert_eq!((&shown[0].id, &shown[0].place), (&agenda.id, &agenda.place));

    // The agent sees it at once through `panels`, without a turn of its own.
    let listed = content(&call(
        &server,
        &token,
        &agent,
        "panels",
        json!({"panel": "agenda"}),
    ))
    .clone();
    assert_eq!(
        items(&listed["panels"][0]),
        [
            (1, json!(false), "Pairing window".to_owned()),
            (2, json!(true), "Panel ids".to_owned()),
        ]
    );
    assert!(listed["panels"][0].get("updated_seconds_ago").is_none());
    assert!(session.log().prompts.is_empty());

    // And is told before the next words spoken in the space.
    ada.say("u1", "next topic");
    let prompt = eventually(|| session.log().prompts.first().cloned());
    assert_eq!(
        prompt.text,
        "Ada: [Ada ticked item 2 of 2 of panel \"Agenda\" (id agenda): \"Panel ids\"] next topic"
    );
}

#[test]
fn a_tick_or_an_update_from_a_version_since_changed_is_refused_rather_than_land_on_another_item_and_redone()
 {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = projected(&server, "ada", "Ada");
    create(&mut ada, "Planning");
    let token = eventually(|| server.tools_token());
    let session = backend.started()[0].clone();
    let agent = tomori_backend::Session::id(&session);
    let update = |arguments: Value| call(&server, &token, &agent, "update_panel", arguments);
    let version = |value: &Value| value.as_u64().unwrap();
    agent_replies(
        &session,
        "m1",
        "```panel\nid: agenda\ntitle: Agenda\n---\n- [ ] Pairing window\n- [ ] Panel ids\n```\n",
    );
    let agenda = panels(&ada, |panels| panels.len() == 1)[0].clone();
    let first = agenda.version;
    assert!(first > 0);
    // The agent reads the panel's version with the list, the one its participants are sent.
    let listed = content(&call(&server, &token, &agent, "panels", json!({}))).clone();
    assert_eq!(version(&listed["panels"][0]["version"]), first);

    // Ada ticks the first item at the version she saw: it applies, and the panel is at the revision it was made at.
    ada.send(&ClientMessage::CheckPanelItem {
        panel: agenda.id.clone(),
        line: 0,
        checked: true,
        revision: Some(first),
    });
    let shown = panels(&ada, |panels| panels[0].version > first);
    let ticked = shown[0].version;
    assert_eq!(shown[0].body, "- [x] Pairing window\n- [ ] Panel ids");

    // The agent's update by number from the version it read is refused, with the current one, and changes nothing.
    let refused = update(json!({"panel": "agenda", "uncheck": [1], "version": first}));
    let said = failure(&refused);
    assert!(
        said.contains(&format!("changed since version {first}")),
        "{said}"
    );
    assert!(said.contains(&format!("at version {ticked} now")), "{said}");
    assert!(said.contains("Read it again with panels"), "{said}");
    // So is one that names it `revision`, as update_panel did before versions.
    assert!(
        failure(&update(
            json!({"panel": "agenda", "uncheck": [1], "revision": first})
        ))
        .contains(&format!("changed since version {first}"))
    );
    // Read again, it is at that version, and redone from what it shows, it applies, and says its version now.
    let listed = content(&call(
        &server,
        &token,
        &agent,
        "panels",
        json!({"panel": "agenda"}),
    ))
    .clone();
    assert_eq!(version(&listed["panels"][0]["version"]), ticked);
    assert_eq!(listed["panels"][0]["items"][0]["checked"], true);
    let updated = content(&update(json!({
        "panel": "agenda",
        "body": "- [ ] Decide the agenda\n- [x] Pairing window\n- [ ] Panel ids",
        "version": ticked,
    })))
    .clone();
    let rewritten = version(&updated["version"]);
    assert!(rewritten > ticked);
    // Ada is sent it, at that version, before she ticks, so that the projection sent again after her refusal is the one
    // waited for below, rather than this one, which a projection still waiting to be sent would take the place of.
    panels(&ada, |panels| panels[0].version == rewritten);
    assert_eq!(
        failure(&update(
            json!({"panel": "agenda", "check": [1], "version": "two"})
        )),
        "version is the panel's version, a whole number, as you last read it."
    );

    // Ada, still seeing the panel as she ticked it, ticks what was the second item: refused, with the panel named so
    // that her client cues it, and the projection sent again shows the panel as it is, nothing ticked that she did not
    // see.
    ada.send(&ClientMessage::CheckPanelItem {
        panel: agenda.id.clone(),
        line: 1,
        checked: true,
        revision: Some(ticked),
    });
    let (error, panel) = ada.wait_for(|message| match message {
        ServerMessage::Error { error, panel, .. } => Some((error.clone(), panel.clone())),
        _ => None,
    });
    assert_eq!(error, "That panel changed as you ticked it; look again.");
    assert_eq!(panel.as_deref(), Some(agenda.id.as_str()));
    let shown = panels(&ada, |panels| panels[0].version == rewritten);
    assert_eq!(
        shown[0].body,
        "- [ ] Decide the agenda\n- [x] Pairing window\n- [ ] Panel ids"
    );
    // Moved, the panel is at a version of its own, so that what was read before the move is read again.
    ada.send(&ClientMessage::PlacePanel {
        panel: agenda.id.clone(),
        wall: None,
        slot: None,
        pose: Some(Pose::upright_facing([1.0, 1.5, -1.0], [0.0, 1.5, 0.0])),
    });
    let moved = panels(&ada, |panels| panels[0].version > rewritten)[0].version;
    assert!(
        failure(&update(
            json!({"panel": "agenda", "check": [1], "version": rewritten})
        ))
        .contains(&format!("at version {moved} now"))
    );

    // An older client, saying no version, ticks by its line alone, as before.
    ada.send(&ClientMessage::CheckPanelItem {
        panel: agenda.id.clone(),
        line: 2,
        checked: true,
        revision: None,
    });
    let shown = panels(&ada, |panels| panels[0].version > moved);
    assert!(shown[0].body.ends_with("- [x] Panel ids"));
    // The agent without a version updates as before.
    let updated = content(&update(
        json!({"panel": "agenda", "check": ["Decide the agenda"]}),
    ))
    .clone();
    assert!(version(&updated["version"]) > shown[0].version);
    // Nothing refused reaches the agent's notes: only the two ticks that landed.
    ada.say("u1", "next");
    let prompt = eventually(|| session.log().prompts.first().cloned());
    assert_eq!(
        prompt.text,
        "Ada: [Ada ticked item 1 of 2 of panel \"Agenda\" (id agenda): \"Pairing window\"; Ada ticked item 3 of 3 of panel \"Agenda\" (id agenda): \"Panel ids\"] next"
    );
}
