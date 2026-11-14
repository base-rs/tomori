// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A space's agent and its locked walls: `set_wall` and `remove_wall` refuse a locked wall, and `set_wall`'s `locked`
//! and `reset` lock, unlock and put back a wall only when an owner asks. See `design/projection.md` and `design/mcp.md`.

use serde_json::{Value, json};
use tomori_protocol::{ClientMessage, Grid, Pose, ServerMessage};
use tomori_server::test_support::{TestClient, TestServer, eventually};
use tomori_server::{Environment, EnvironmentWall};

pub mod mcp_support;
use mcp_support::{call, failure, say};

/// The yard's notice board, two slots wide and one high, as the environment declares it.
fn board() -> Pose {
    Pose::at([3.0, 1.4, -4.0]).facing_towards([0.0, 1.4, 0.0])
}

#[test]
fn the_agent_changes_no_locked_wall_and_locks_unlocks_and_puts_one_back_only_when_an_owner_asks() {
    let server = TestServer::with_tokens_and_tools(false, |server| {
        // Named as Sky, which the test client creates its spaces in.
        server.with_environments([Environment::new("sky", "Yard")
            .with_walls([EnvironmentWall::new(board()).with_grid(Grid::from((2, 1)))])])
    });
    let (_, owner) = server.issue("Ada", tomori_security::Grant::Owner);
    let mut ada = TestClient::open_with(
        server.address,
        &[("Authorization", &format!("Bearer {owner}"))],
    );
    ada.hello_projected("ada", "Ada");
    let space = ada.create_space("Studio").id;
    let (_, collaborator) = server.issue(
        "Cy",
        tomori_security::Grant::Collaborator {
            space: space.clone(),
        },
    );
    let mut cy = TestClient::open_with(
        server.address,
        &[("Authorization", &format!("Bearer {collaborator}"))],
    );
    cy.hello_projected("cy", "Cy");
    cy.send(&ClientMessage::Join {
        space: space.clone(),
    });
    let token = eventually(|| server.tools_token());
    let session = tomori_backend::Session::id(&server.backend.started()[0]);
    let tool = |name: &str, arguments: Value| call(&server, &token, &session, name, arguments);
    let walls = || tool("presence", json!({}))["structuredContent"]["walls"].clone();
    let notice_board = || eventually(|| walls().as_array()?.get(1).cloned());
    // The environment's own wall is listed locked, two slots wide and one high.
    let listed = notice_board();
    assert_eq!(listed["name"], "environment wall 1");
    assert_eq!(
        (listed["columns"].clone(), listed["rows"].clone()),
        (json!(2), json!(1))
    );
    assert_eq!(listed["locked"], true);
    assert_eq!(walls()[0]["locked"], false);

    // Locked, it is neither changed, moved nor removed, the refusal saying to ask an owner.
    let refused = failure(&tool(
        "set_wall",
        json!({"wall": "environment wall 1", "columns": 3}),
    ));
    assert!(
        refused.starts_with("The environment wall 1 is locked: ask an owner"),
        "{refused}"
    );
    assert_eq!(
        failure(&tool("remove_wall", json!({"wall": "environment wall 1"}))),
        "Only a wall an owner added can be removed."
    );
    // Unlocking it needs who asked, an owner whose words were given last.
    say(&mut ada, "u1", "unlock the notice board");
    assert!(
        failure(&tool(
            "set_wall",
            json!({"wall": "environment wall 1", "locked": false})
        ))
        .starts_with("This tool needs requested_by")
    );
    assert_eq!(
        failure(&tool(
            "set_wall",
            json!({"wall": "environment wall 1", "locked": "no", "requested_by": "Ada"})
        )),
        "locked is true or false."
    );
    assert_eq!(
        failure(&tool(
            "set_wall",
            json!({"wall": "environment wall 1", "reset": "yes", "requested_by": "Ada"})
        )),
        "reset is true or false."
    );
    assert_eq!(
        failure(&tool(
            "set_wall",
            json!({"wall": "environment wall 1", "locked": false, "requested_by": 1})
        )),
        "requested_by is a participant's name, as text."
    );
    let unlocked = tool(
        "set_wall",
        json!({"wall": "environment wall 1", "locked": false, "requested_by": "Ada"}),
    );
    assert_ne!(unlocked["isError"], true, "{unlocked}");
    let content = &unlocked["structuredContent"];
    assert_eq!(
        (content["changed"].clone(), content["locked"].clone()),
        (json!(true), json!(false))
    );
    assert_eq!(notice_board()["locked"], false);
    let again = tool(
        "set_wall",
        json!({"wall": "environment wall 1", "locked": false, "requested_by": "Ada"}),
    );
    assert_eq!(again["structuredContent"]["changed"], false);
    // Unlocked, it changes and moves, for everyone, while it is still never removed.
    let changed = tool(
        "set_wall",
        json!({"wall": "environment wall 1", "columns": 3}),
    );
    assert_ne!(changed["isError"], true, "{changed}");
    assert_eq!(notice_board()["columns"], 3);
    ada.send(&ClientMessage::Presence {
        head: Pose::at([0.0, 1.6, 1.0]),
        left: None,
        right: None,
        hands: None,
        carrying: None,
        carried: Vec::new(),
        rays: None,
        pointing: None,
    });
    let moved = eventually(|| {
        let moved = tool(
            "set_wall",
            json!({"wall": "environment wall 1", "place": "front Ada"}),
        );
        (moved["isError"] != true).then_some(moved)
    });
    assert_eq!(moved["structuredContent"]["changed"], true);
    assert_ne!(notice_board()["position"], json!([3.0, 1.4, -4.0]));

    // A collaborator's words stand behind no lock; an owner's put it back where and as the environment has it, locked.
    say(&mut cy, "u2", "lock it again");
    assert_eq!(
        failure(&tool(
            "set_wall",
            json!({"wall": "environment wall 1", "locked": true, "requested_by": "Cy"})
        )),
        "Cy is a collaborator here, and only an owner may ask for this."
    );
    say(&mut ada, "u3", "put the notice board back");
    assert_eq!(
        failure(&tool(
            "set_wall",
            json!({"wall": "environment wall 1", "reset": true, "columns": 1, "requested_by": "Ada"})
        )),
        "Put a wall back as the environment has it alone, without changing it too."
    );
    assert!(
        failure(&tool(
            "set_wall",
            json!({"wall": "environment wall 1", "reset": true, "locked": false, "requested_by": "Ada"})
        ))
        .starts_with("A wall put back as the environment has it is locked")
    );
    let reset = tool(
        "set_wall",
        json!({"wall": "environment wall 1", "reset": true, "requested_by": "Ada"}),
    );
    assert_ne!(reset["isError"], true, "{reset}");
    let now = reset["structuredContent"]["now"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        now.starts_with("2 slots wide and 1 high") && now.ends_with(", locked"),
        "{now}"
    );
    let listed = notice_board();
    assert_eq!(
        (listed["columns"].clone(), listed["locked"].clone()),
        (json!(2), json!(true))
    );
    // The notice on the Tomori wall says what the agent did.
    ada.wait_for(|message| match message {
        ServerMessage::Projection { projection } => projection
            .panels
            .iter()
            .find(|panel| panel.body.starts_with("The agent put the environment wall 1 back where and as the environment has it: 2 slots wide"))
            .map(|_| ()),
        _ => None,
    });

    // A wall someone added, locked by an owner's word, is removed by nobody until it is unlocked; it is never put back,
    // as only an environment's own wall is.
    let added = tool("add_wall", json!({"place": "front Ada"}));
    assert_ne!(added["isError"], true, "{added}");
    let wall = added["structuredContent"]["wall"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        failure(&tool(
            "set_wall",
            json!({"wall": wall, "reset": true, "requested_by": "Ada"})
        )),
        "Only an environment's own wall is put back as the environment has it."
    );
    // Locked alone, then unlocked and changed in one call, then changed and locked in one: each notice says which.
    let notice = |text: &str| {
        ada.wait_for(|message| match message {
            ServerMessage::Projection { projection } => projection
                .panels
                .iter()
                .find(|panel| panel.body.starts_with(text))
                .map(|_| ()),
            _ => None,
        })
    };
    let alone = tool(
        "set_wall",
        json!({"wall": wall, "locked": true, "requested_by": "Ada"}),
    );
    assert_eq!(alone["structuredContent"]["locked"], true, "{alone}");
    notice("The agent locked the added wall 1");
    let both = tool(
        "set_wall",
        json!({"wall": wall, "locked": false, "rows": 1, "requested_by": "Ada"}),
    );
    assert_eq!(both["structuredContent"]["locked"], false, "{both}");
    notice("The agent unlocked and changed the added wall 1");
    let locked = tool(
        "set_wall",
        json!({"wall": wall, "locked": true, "columns": 2, "requested_by": "Ada"}),
    );
    assert_ne!(locked["isError"], true, "{locked}");
    assert_eq!(locked["structuredContent"]["locked"], true);
    notice("The agent changed and locked the added wall 1");
    // Unlocked to be changed or attached, and refused, it is locked again, as it was.
    let locked_still = || {
        walls()
            .as_array()
            .unwrap()
            .iter()
            .find(|listed| listed["id"] == wall.as_str())
            .unwrap()["locked"]
            .clone()
    };
    let refused = failure(&tool(
        "set_wall",
        json!({"wall": wall, "locked": false, "columns": 8, "rows": 4, "scale": 5, "requested_by": "Ada"}),
    ));
    assert!(!refused.contains("locked"), "{refused}");
    assert_eq!(locked_still(), true);
    let refused = failure(&tool(
        "set_wall",
        json!({"wall": wall, "locked": false, "attach": {"to": "Tomori wall", "edge": "right"}, "requested_by": "Ada"}),
    ));
    assert!(
        refused.starts_with("Only walls someone added snap"),
        "{refused}"
    );
    assert_eq!(locked_still(), true);
    let refused = failure(&tool("remove_wall", json!({"wall": wall})));
    assert!(
        refused.starts_with("The added wall 1 is locked: ask an owner"),
        "{refused}"
    );
    let unlocked = tool(
        "set_wall",
        json!({"wall": wall, "locked": false, "requested_by": "Ada"}),
    );
    assert_ne!(unlocked["isError"], true, "{unlocked}");
    let removed = tool("remove_wall", json!({"wall": wall}));
    assert_eq!(removed["structuredContent"]["removed"], true, "{removed}");
    assert_eq!(walls().as_array().unwrap().len(), 2);

    // A layout's locked walls stay as it is taken down, which says so, and one all locked is refused.
    let built = tool(
        "create_layout",
        json!({"place": "front Ada", "walls": [
            {"id": "a", "columns": 1, "rows": 1},
            {"id": "b", "columns": 1, "rows": 1, "attach": {"to": "a", "edge": "right"}},
            {"id": "c", "columns": 1, "rows": 1, "attach": {"to": "b", "edge": "right"}},
        ]}),
    );
    assert_ne!(built["isError"], true, "{built}");
    let names: Vec<String> = built["structuredContent"]["walls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|wall| wall["name"].as_str().unwrap().to_owned())
        .collect();
    let lock = |name: &str, locked: bool| {
        let result = tool(
            "set_wall",
            json!({"wall": name, "locked": locked, "requested_by": "Ada"}),
        );
        assert_ne!(result["isError"], true, "{result}");
    };
    lock(&names[0], true);
    lock(&names[1], true);
    let removed = tool("remove_layout", json!({"group": names[0]}));
    assert_eq!(
        removed["structuredContent"]["removed"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "{removed}"
    );
    assert!(
        removed["structuredContent"]["next"]
            .as_str()
            .unwrap()
            .ends_with(&format!(
                "you removed {}; {}, {} stay, locked.",
                names[2], names[0], names[1]
            )),
        "{removed}"
    );
    assert_eq!(
        failure(&tool("remove_layout", json!({"group": names[0]}))),
        "Every wall of that layout is locked: ask an owner in the space to unlock them first."
    );
    lock(&names[1], false);
    let removed = tool("remove_layout", json!({"group": names[0]}));
    assert!(
        removed["structuredContent"]["next"]
            .as_str()
            .unwrap()
            .ends_with(&format!(
                "you removed {}; {} stays, locked.",
                names[1], names[0]
            )),
        "{removed}"
    );
}

#[test]
fn a_change_to_a_wall_from_a_version_since_changed_is_refused_and_redone_from_the_version_read_again()
 {
    let server = TestServer::with_tokens_and_tools(false, |server| server);
    let (_, owner) = server.issue("Ada", tomori_security::Grant::Owner);
    let mut ada = TestClient::open_with(
        server.address,
        &[("Authorization", &format!("Bearer {owner}"))],
    );
    ada.hello_projected("ada", "Ada");
    ada.create_space("Studio");
    let token = eventually(|| server.tools_token());
    let session = tomori_backend::Session::id(&server.backend.started()[0]);
    let tool = |name: &str, arguments: Value| call(&server, &token, &session, name, arguments);
    let version = || {
        eventually(|| {
            tool("presence", json!({}))["structuredContent"]["walls"][0]["version"].as_u64()
        })
    };
    // The agent reads the Tomori wall's version, the one its participants are sent.
    let read = version();
    let sent = ada.wait_for(|message| match message {
        ServerMessage::Projection { projection } => {
            projection.wall("tomori").map(|wall| wall.version)
        }
        _ => None,
    });
    assert_eq!(read, sent);
    // Ada tilts it with its hinge, from the version she took hold of it at: it applies.
    let tilt = |tilt: f32, revision: u64| ClientMessage::SetWall {
        wall: "tomori".into(),
        grid: None,
        scale: None,
        aspect: None,
        shape: None,
        pose: None,
        tilt: Some(tilt),
        content_scale: None,
        revision: Some(revision),
    };
    ada.send(&tilt(10.0, read));
    let tilted = ada.wait_for(|message| match message {
        ServerMessage::Projection { projection } => projection
            .wall("tomori")
            .filter(|wall| wall.tilt == Some(10.0))
            .map(|wall| wall.version),
        _ => None,
    });
    assert!(tilted > read);
    // The agent's change from the version it read is refused, saying the version now, and changes nothing.
    let refused = failure(&tool(
        "set_wall",
        json!({"wall": "Tomori wall", "columns": 4, "version": read}),
    ));
    assert!(
        refused.contains(&format!("changed since version {read}")),
        "{refused}"
    );
    assert!(
        refused.contains(&format!("at version {tilted} now")),
        "{refused}"
    );
    assert!(refused.contains("Read it again with presence"), "{refused}");
    assert_eq!(
        failure(&tool(
            "set_wall",
            json!({"wall": "Tomori wall", "columns": 4, "version": -1}),
        )),
        "version is the wall's version, a whole number, as you last read it."
    );
    // Read again, and redone from it, it applies, and says the version it made.
    assert_eq!(version(), tilted);
    let changed = tool(
        "set_wall",
        json!({"wall": "Tomori wall", "columns": 4, "version": tilted}),
    )["structuredContent"]
        .clone();
    assert_eq!(changed["changed"], true);
    let widened = changed["version"].as_u64().unwrap();
    assert!(widened > tilted);
    // Ada is sent it, at that version, before she tilts, so that the projection sent again after her refusal is the one
    // waited for below, rather than this one, which a projection still waiting to be sent would take the place of.
    ada.wait_for(|message| match message {
        ServerMessage::Projection { projection } => projection
            .wall("tomori")
            .filter(|wall| wall.version == widened)
            .map(|_| ()),
        _ => None,
    });
    // Ada, still holding the wall as it was before the agent widened it, tilts it again: refused, naming the wall, so
    // that her client cues it, and the projection sent again shows the wall as it is, the agent's change kept.
    ada.send(&tilt(20.0, tilted));
    let (error, wall) = ada.wait_for(|message| match message {
        ServerMessage::Error { error, wall, .. } => Some((error.clone(), wall.clone())),
        _ => None,
    });
    assert_eq!(error, "That wall changed as you changed it; look again.");
    assert_eq!(wall.as_deref(), Some("tomori"));
    let shown = ada.wait_for(|message| match message {
        ServerMessage::Projection { projection } => projection
            .wall("tomori")
            .filter(|wall| wall.version == widened)
            .cloned(),
        _ => None,
    });
    assert_eq!((shown.grid().columns, shown.tilt), (4, Some(10.0)));
    // A change that says no version, as carrying a wall or an older client's, goes by what it says alone.
    ada.send(&ClientMessage::SetWall {
        wall: "tomori".into(),
        grid: None,
        scale: None,
        aspect: None,
        shape: None,
        pose: None,
        tilt: Some(20.0),
        content_scale: None,
        revision: None,
    });
    ada.wait_for(|message| match message {
        ServerMessage::Projection { projection } => projection
            .wall("tomori")
            .filter(|wall| wall.tilt == Some(20.0))
            .map(|_| ()),
        _ => None,
    });
}

/// The wall `message`, if it is an error, names as refused.
fn refused_wall(message: &ServerMessage) -> Option<String> {
    match message {
        ServerMessage::Error { wall, .. } => wall.clone(),
        _ => None,
    }
}

/// The Tomori wall as `message`, if it is a projection, shows it.
fn tomori_shown(message: &ServerMessage) -> Option<tomori_protocol::Wall> {
    match message {
        ServerMessage::Projection { projection } => projection.wall("tomori").cloned(),
        _ => None,
    }
}

#[test]
fn the_agent_and_a_participant_scale_a_wall_s_content_within_its_slots_from_the_version_they_read()
{
    let server = TestServer::with_tokens_and_tools(false, |server| server);
    let (_, owner) = server.issue("Ada", tomori_security::Grant::Owner);
    let mut ada = TestClient::open_with(
        server.address,
        &[("Authorization", &format!("Bearer {owner}"))],
    );
    ada.hello_projected("ada", "Ada");
    ada.create_space("Studio");
    let token = eventually(|| server.tools_token());
    let session = tomori_backend::Session::id(&server.backend.started()[0]);
    let tool = |name: &str, arguments: Value| call(&server, &token, &session, name, arguments);
    let tomori = || {
        eventually(|| {
            let walls = tool("presence", json!({}))["structuredContent"]["walls"].clone();
            walls[0].get("version").is_some().then(|| walls[0].clone())
        })
    };
    // A message other than a projection shows no wall, and one other than an error refuses none.
    assert!(tomori_shown(&ServerMessage::PairWaiting).is_none());
    assert!(refused_wall(&ServerMessage::PairWaiting).is_none());
    // As large as its slots give it, unless it says otherwise.
    let read = tomori();
    assert_eq!(read["content_scale"], 1.0);
    assert_eq!(read["text_scale"], 0.7);
    let read = read["version"].as_u64().unwrap();
    // "Fit more on this wall": its text drawn smaller, its slots as they were, said in what it now is.
    let changed = tool(
        "set_wall",
        json!({"wall": "Tomori wall", "content_scale": 0.8, "version": read}),
    );
    assert_ne!(changed["isError"], true, "{changed}");
    let now = changed["structuredContent"]["now"].as_str().unwrap();
    assert!(now.ends_with(", its text at 80%"), "{now}");
    let wall = tomori();
    assert_eq!(
        (wall["content_scale"].as_f64(), wall["text_scale"].as_f64()),
        (Some(0.8), Some(0.56))
    );
    assert_eq!(
        (wall["columns"].as_u64(), wall["rows"].as_u64()),
        (Some(6), Some(1))
    );
    let shown =
        ada.wait_for(|message| tomori_shown(message).filter(|wall| wall.content_scale.is_some()));
    assert_eq!((shown.content_scale(), shown.grid().columns), (0.8, 6));
    // Never smaller than half a standard slot's text, nor beyond twice, nor anything but a number.
    for refused in [0.6, 2.5] {
        assert_eq!(
            failure(&tool(
                "set_wall",
                json!({"wall": "Tomori wall", "content_scale": refused})
            )),
            "A wall's text is drawn from half to twice the size its slots give it, and never smaller than half a standard slot's: give it larger slots to make its text smaller still."
        );
    }
    assert_eq!(
        failure(&tool(
            "set_wall",
            json!({"wall": "Tomori wall", "content_scale": "small"})
        )),
        "content_scale is a number."
    );
    assert_eq!(
        failure(&tool(
            "set_wall",
            json!({"wall": "Tomori wall", "content_scale": 1e300})
        )),
        "content_scale is a number, from 0.5 to 2."
    );
    // Ada's slider, let go from the version before the agent's change, is refused, naming the wall, and changes
    // nothing.
    let slider = |content_scale: f32, revision: u64| ClientMessage::SetWall {
        wall: "tomori".into(),
        grid: None,
        scale: None,
        aspect: None,
        shape: None,
        pose: None,
        tilt: None,
        content_scale: Some(content_scale),
        revision: Some(revision),
    };
    ada.send(&slider(1.2, read));
    assert_eq!(ada.wait_for(refused_wall), "tomori");
    // From the version she sees, it applies, for everyone.
    let seen = tomori()["version"].as_u64().unwrap();
    assert_eq!(seen, shown.version);
    ada.send(&slider(1.2, seen));
    ada.wait_for(|message| tomori_shown(message).filter(|wall| wall.content_scale == Some(1.2)));
    assert_eq!(tomori()["content_scale"], 1.2);
    // 1 is as large as its slots give it again, and says nothing of its text.
    let back = tool(
        "set_wall",
        json!({"wall": "Tomori wall", "content_scale": 1}),
    );
    let now = back["structuredContent"]["now"].as_str().unwrap();
    assert!(!now.contains("its text"), "{now}");
    assert_eq!(tomori()["content_scale"], 1.0);
}
