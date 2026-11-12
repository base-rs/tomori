// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! An environment's own wall that its environment named, locked as every such wall is: the agent puts panels on it by
//! its name, `place: wall Details slot 4`, in a reply's blocks and with `move_panel`, lists it by that name in
//! `presence`, and still cannot change it. See `design/projection.md` and `design/mcp.md`.

use serde_json::{Value, json};
use tomori_protocol::{Grid, Place, Pose, ServerMessage};
use tomori_server::test_support::{TestClient, TestServer, eventually};
use tomori_server::{Environment, EnvironmentWall};

pub mod mcp_support;
use mcp_support::{agent_replies, call};

#[test]
fn the_agent_puts_panels_on_a_declared_locked_wall_by_the_name_its_environment_gave_it() {
    let server = TestServer::with_tokens_and_tools(false, |server| {
        // Named as Sky, which the test client creates its spaces in: a console of three by three slots.
        server.with_environments([Environment::new("sky", "Yard").with_walls([
            EnvironmentWall::new(Pose::at([-3.0, 1.6, -4.0]).facing_towards([0.0, 1.6, 0.0]))
                .with_grid(Grid::from((3, 3)))
                .with_name("Details"),
            // A second wall, given the same name in another case, is called what any other is.
            EnvironmentWall::new(Pose::at([3.0, 1.6, -4.0]).facing_towards([0.0, 1.6, 0.0]))
                .with_name("details"),
        ])])
    });
    let (_, owner) = server.issue("Ada", tomori_security::Grant::Owner);
    let mut ada = TestClient::open_with(
        server.address,
        &[("Authorization", &format!("Bearer {owner}"))],
    );
    ada.hello_projected("ada", "Ada");
    ada.create_space("Studio");
    let token = eventually(|| server.tools_token());
    let backend = server.backend.started()[0].clone();
    let session = tomori_backend::Session::id(&backend);
    let tool = |name: &str, arguments: Value| call(&server, &token, &session, name, arguments);
    let walls = || tool("presence", json!({}))["structuredContent"]["walls"].clone();
    let names: Vec<String> = eventually(|| {
        let names: Vec<String> = walls()
            .as_array()?
            .iter()
            .map(|wall| wall["name"].as_str().unwrap().to_owned())
            .collect();
        (names.len() == 3).then_some(names)
    });
    assert_eq!(
        names,
        [
            "status wall (the Tomori wall)",
            "Details",
            "environment wall 2"
        ]
    );
    let listed = walls()[1].clone();
    assert_eq!(listed["locked"], true);
    assert_eq!(
        (listed["columns"].clone(), listed["rows"].clone()),
        (json!(3), json!(3))
    );

    // A reply's panel and code blocks go into the named wall's slots, though it is locked.
    agent_replies(
        &backend,
        "m1",
        "Servers under load.\n\n```panel\nid: hot\ntitle: web-2\nplace: wall Details slot 4\n---\nOver capacity.\n```\n\n```text place=\"wall details slot 8\"\nweb-5\nweb-6\n```\n",
    );
    let (wall, shown) = ada.wait_for(|message| match message {
        ServerMessage::Projection { projection } if projection.panels.len() == 2 => Some((
            projection
                .walls
                .iter()
                .find(|wall| wall.name.as_deref() == Some("Details"))
                .map(|wall| wall.id.clone())
                .unwrap(),
            projection.panels.clone(),
        )),
        _ => None,
    });
    let slot = |title: &str| {
        shown
            .iter()
            .find(|panel| panel.title == title)
            .map(|panel| panel.place.clone())
    };
    assert_eq!(
        slot("web-2"),
        Some(Place::Wall {
            wall: wall.clone(),
            slot: 4
        })
    );
    assert_eq!(
        slot("text"),
        Some(Place::Wall {
            wall: wall.clone(),
            slot: 8
        })
    );
    // `move_panel` moves one there too, and a slot the wall lacks is refused, saying how its slots are numbered.
    let moved = tool(
        "move_panel",
        json!({"panel": "hot", "place": "wall Details slot 0"}),
    );
    assert_ne!(moved["isError"], true, "{moved}");
    let refused = tool(
        "move_panel",
        json!({"panel": "hot", "place": "wall Details slot 9"}),
    );
    assert_eq!(refused["isError"], true, "{refused}");
    assert!(
        refused["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("The wall \"Details\" has no slot 9: its slots are numbered from 0 to 8."),
        "{refused}"
    );
    // The wall itself stays as its environment put it: locked, named, so refused by that name.
    let locked = tool("set_wall", json!({"wall": "Details", "columns": 4}));
    assert_eq!(locked["isError"], true, "{locked}");
    assert!(
        locked["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("The wall \"Details\" is locked: ask an owner"),
        "{locked}"
    );
}
