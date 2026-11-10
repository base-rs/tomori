// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where the agent stands: called by a participant, sent home or pinned by the agent through the `move_agent` tool
//! when asked, kept home by an owner, told by `presence`, and back at home when the space changes environment, and
//! never moved otherwise in a space whose agent moves only when asked. See
//! `design/environments.md` and `design/mcp.md`.
use std::sync::Arc;

use serde_json::{Value, json};
use tomori_protocol::{ClientMessage, Pose, SpaceState};
use tomori_server::Server;
use tomori_server::fake::{FAKE_BACKEND_NAME, FakeBackend};
use tomori_server::test_support::{TestClient, TestServer, TextTranscriber, eventually};

/// A server over `backend` serving Tomori's tools.
fn start(backend: &FakeBackend) -> TestServer {
    TestServer::with_tools(backend.clone(), |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default()))
    })
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

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3)
}

#[test]
fn a_participant_calls_the_agent_and_the_agent_goes_home_when_asked() {
    let backend = FakeBackend::new();
    let server = start(&backend);
    let mut ada = TestClient::open(server.address);
    ada.hello_projected("ada", "Ada");
    ada.send(&ClientMessage::CreateSpace {
        title: "Yard".to_owned(),
        backend: FAKE_BACKEND_NAME.to_owned(),
        directory: "/tmp".to_owned(),
        session: None,
        environment: None,
        instructions: None,
    });
    let state = ada.space(|state| state.title == "Yard");
    // The space opens with the agent at home, moving only when asked, and the server keeps no summons.
    assert!(!state.summons);
    assert_eq!(
        state.movement,
        Some(tomori_protocol::AgentMovement::default())
    );
    assert_eq!(state.summoned, None);
    let token = eventually(|| server.tools_token());
    let session = tomori_backend::Session::id(&backend.started()[0]);
    let presence = content(&call(&server, &token, &session, "presence", json!({}))).clone();
    assert_eq!(presence["agent"]["at_home"], true);
    assert_eq!(presence["agent"]["position"], json!([0.0, 1.4, -2.3]));
    assert_eq!(presence["agent"]["mode"], "home");

    // Ada calls it to her, to look at something together: everyone in the space sees it there, pinned. She stays where
    // she is, since a client reports where it is only so often.
    ada.send(&ClientMessage::Presence {
        head: Pose::at([1.0, 1.6, 0.0]),
        left: None,
        right: None,
        hands: None,
        carrying: None,
    });
    // Once the server has heard where she is, it comes; before, it cannot.
    let called = loop {
        ada.send(&ClientMessage::CallAgent);
        let answer = ada.wait_for(|message| match message {
            tomori_protocol::ServerMessage::Space { state } if state.summoned.is_some() => {
                Some(Some(state.clone()))
            }
            tomori_protocol::ServerMessage::Error { .. } => Some(None),
            _ => None,
        });
        if let Some(state) = answer {
            break state;
        }
    };
    assert!(called.pinned);
    assert!(close(called.summoned.unwrap().position, [1.0, 1.9, -2.3]));
    let presence = content(&call(&server, &token, &session, "presence", json!({}))).clone();
    assert_eq!(presence["agent"]["at_home"], false);
    assert_eq!(presence["agent"]["mode"], "pinned");
    assert_eq!(presence["agent"]["position"], json!([1.0, 1.9, -2.3]));
    assert_eq!(presence["agent"]["home"], json!([0.0, 1.4, -2.3]));

    // Nothing moves it of its own accord: it is still there a while later.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let presence = content(&call(&server, &token, &session, "presence", json!({}))).clone();
    assert_eq!(presence["agent"]["at_home"], false);

    // Asked by voice to go back to its home spot, the agent goes home with the tool.
    let home = content(&call(
        &server,
        &token,
        &session,
        "move_agent",
        json!({"place": "home"}),
    ))
    .clone();
    assert_eq!(home["moved"], true);
    assert_eq!(home["agent"]["at_home"], true);
    ada.space(|state: &SpaceState| state.summoned.is_none());
    // Home again changes nothing.
    let again = content(&call(
        &server,
        &token,
        &session,
        "move_agent",
        json!({"place": "Home"}),
    ))
    .clone();
    assert_eq!(again["moved"], false);

    // To someone who has reported where they are: in front of them, facing them.
    ada.send(&ClientMessage::Presence {
        // A head looks along its −Z.
        head: Pose::at([1.0, 1.6, 0.0]),
        left: None,
        right: None,
        hands: None,
        carrying: None,
    });
    let to_ada = eventually(|| {
        let result = call(
            &server,
            &token,
            &session,
            "move_agent",
            json!({"place": "front Ada"}),
        );
        (result["isError"] != true).then_some(result)
    });
    assert_eq!(content(&to_ada)["moved"], true);
    let state = ada.space(|state: &SpaceState| state.summoned.is_some());
    let at = state.summoned.unwrap();
    // 2.3 m in front of her, a little above her eyes, with room for its wall beneath it.
    assert!(close(at.position, [1.0, 1.9, -2.3]), "{at:?}");
    // It faces her.
    assert!(at.yaw().abs() < 1e-3, "{}", at.yaw());
    // Its entourage gathers about her: its Tomori wall goes beneath it, before her, as its home stands beneath the
    // agent's home, for everyone.
    assert_eq!(state.summoner, Some([1.0, 1.6, 0.0]));
    let wall = ada.wait_for(|message| match message {
        tomori_protocol::ServerMessage::Projection { projection }
            if projection.walls[0].pose.position != [0.0, 0.52, -2.3] =>
        {
            Some(projection.walls[0].pose)
        }
        _ => None,
    });
    assert!(close(wall.position, [1.0, 1.9 - 0.88, -2.3]), "{wall:?}");

    // Somewhere it cannot go, or someone not here, is refused, saying why.
    assert!(
        failure(&call(
            &server,
            &token,
            &session,
            "move_agent",
            json!({"place": "wall Tomori"})
        ))
        .contains("home, stay, near NAME or front NAME")
    );
    assert!(
        failure(&call(
            &server,
            &token,
            &session,
            "move_agent",
            json!({"place": "near Bob"})
        ))
        .contains("Nobody called \"Bob\"")
    );
    // Where someone is looking, it goes in front of them, as for front.
    let looking = content(&call(
        &server,
        &token,
        &session,
        "move_agent",
        json!({"place": "looking Ada"}),
    ))
    .clone();
    assert_eq!(looking["moved"], false);
    assert_eq!(looking["agent"]["position"], json!([1.0, 1.9, -2.3]));
    // Nobody asked as they spoke, so a form that names nobody is refused; and a place too long, or none.
    assert!(
        failure(&call(
            &server,
            &token,
            &session,
            "move_agent",
            json!({"place": "front"})
        ))
        .contains("Nobody asked")
    );
    let long = "x".repeat(2000);
    assert_eq!(
        call(
            &server,
            &token,
            &session,
            "move_agent",
            json!({ "place": long })
        )["isError"],
        true
    );
    assert_eq!(
        call(&server, &token, &session, "move_agent", json!({}))["isError"],
        true
    );

    // Asked to stay, it is pinned where it stands, and says so.
    let stay = content(&call(
        &server,
        &token,
        &session,
        "move_agent",
        json!({"place": "stay here"}),
    ))
    .clone();
    assert_eq!(
        (stay["moved"].as_bool(), stay["agent"]["mode"].as_str()),
        (Some(false), Some("pinned"))
    );
    ada.space(|state: &SpaceState| state.pinned);

    // An owner keeps it home: it goes home, and neither the agent nor a participant moves it from there.
    ada.send(&ClientMessage::SetAgentMovement {
        movement: tomori_protocol::AgentMovement::new(tomori_protocol::Movement::Home, Some(90)),
    });
    let kept = ada.space(|state: &SpaceState| {
        state
            .movement
            .is_some_and(|movement| movement.mode == tomori_protocol::Movement::Home)
    });
    assert_eq!((kept.summoned, kept.pinned), (None, false));
    assert!(
        failure(&call(
            &server,
            &token,
            &session,
            "move_agent",
            json!({"place": "front Ada"})
        ))
        .contains("keeps you at home")
    );
    ada.send(&ClientMessage::CallAgent);
    ada.wait_for(|message| match message {
        tomori_protocol::ServerMessage::Error { error, .. } if error.contains("stays home") => {
            Some(())
        }
        _ => None,
    });
    let home = content(&call(
        &server,
        &token,
        &session,
        "move_agent",
        json!({"place": "home"}),
    ))
    .clone();
    assert_eq!(home["agent"]["movement"], "stays home");
    ada.send(&ClientMessage::SetAgentMovement {
        movement: tomori_protocol::AgentMovement::default(),
    });
    ada.space(|state: &SpaceState| {
        state.movement == Some(tomori_protocol::AgentMovement::default())
    });
    content(&call(
        &server,
        &token,
        &session,
        "move_agent",
        json!({"place": "front Ada"}),
    ));

    // A change of environment brings it home, to the new environment's home for it.
    ada.send(&ClientMessage::SetEnvironment {
        environment: "minimal".to_owned(),
    });
    let moved = ada.space(|state: &SpaceState| state.environment == "minimal");
    assert_eq!(moved.summoned, None);
}
