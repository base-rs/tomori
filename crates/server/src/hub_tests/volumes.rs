// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Volumes through the hub: a diagram block in a reply, and the `show_diagram` tool, project a volume, laid out on the
//! server and updated in place by its id from where it stood; what was wrong with a block is told to the agent before
//! the next words, and the tool's at once; a client that draws volumes is sent its diagram within its limits, and every
//! other client its outline; and pointing at a node or an edge names it to the agent. See `design/diagrams.md`.
use super::*;
use serde_json::{Value, json};
use tomori_protocol::{Limits, Panel};

const BLOCK: &str = "Here is how it fits together.\n\n```diagram\ntitle: How a frame is drawn\nid: renderer\nlayout: layered down\nsource: the code\n---\nxr[client-quest]@{ colour: violet, detail: \"Locates both eyes\" }\nsubgraph scene [tomori-scene]\n  step((Frame step))@{ size: 4, ref: crates/scene/src/scene.rs }\n  faces[projection::faces]\nend\nxr --> step : eyes and input\nstep --> faces : panels changed\nstep --> renderer((Renderer))\nrenderer --> xr\n```\n";

impl Client {
    /// A client that reads what is projected and draws as many volumes, of as many nodes together, as `limits` say.
    fn drawing_volumes(hub: &Arc<Hub>, participant: &str, volumes: u32, nodes: u32) -> Self {
        let client = Self::connect(hub, Access::open());
        client.send(ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            participant: participant.to_owned(),
            name: participant.to_owned(),
            names: false,
            environments: None,
            environment_state: false,
            projection: true,
            pairing_window: false,
            speech_held: false,
            images: false,
            diagrams: true,
            world_edges: false,
            projection_patches: false,
            live_panels: false,
            limits: Some(Limits::TODAY.with_volumes(volumes, nodes)),
        });
        client.read();
        client
    }
}

/// The volumes `client` is sent, and every panel in all.
fn volumes(client: &Client) -> (Vec<Panel>, Vec<Panel>) {
    let panels = client.projection().panels;
    (
        panels
            .iter()
            .filter(|panel| panel.diagram.is_some())
            .cloned()
            .collect(),
        panels,
    )
}

/// Call the tool `name` with `arguments` as the agent of the session `session` would, over MCP.
fn call(hub: &Hub, name: &str, arguments: Value) -> Value {
    futures_lite::future::block_on(crate::mcp::handle(
        hub,
        &json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {
                "name": name,
                "arguments": arguments,
                "_meta": {crate::mcp::SESSION_META: "session"},
            },
        }),
    ))
    .unwrap()["result"]
        .clone()
}

#[test]
fn a_diagram_block_projects_a_volume_laid_out_on_the_server_and_its_outline_for_older_clients() {
    let (hub, _jobs) = hub_waiting();
    let (space, _commands) = idle(&hub);
    let ada = Client::drawing_volumes(&hub, "ada", 16, 1024);
    let old = Client::projecting(&hub, Access::open(), "old");
    let few = Client::drawing_volumes(&hub, "few", 16, 2);
    for client in [&ada, &old, &few] {
        client.join(&space);
    }
    reply(&hub, &space, "m1", BLOCK);
    let (shown, _) = volumes(&ada);
    assert_eq!(shown.len(), 1);
    let volume = &shown[0];
    assert_eq!(volume.title, "How a frame is drawn");
    assert_eq!(volume.slug.as_deref(), Some("renderer"));
    assert_eq!(
        volume.meta,
        [tomori_protocol::Meta::new("source", "the code")]
    );
    let diagram = volume.diagram.as_ref().unwrap();
    assert_eq!(diagram.nodes.len(), 4);
    assert!(diagram.extent.iter().all(|side| *side > 0));
    // Laid out down, the client-quest above the frame step it feeds.
    let at = |id: &str| diagram.nodes[diagram.node(id).unwrap()].at;
    assert!(at("xr")[1] > at("step")[1]);
    // Its body is its outline, which an older client, sent no diagram, shows, as does one that draws too few nodes.
    assert!(volume.body.contains("**tomori-scene**\n- Frame step"));
    assert!(
        volume
            .body
            .contains("- client-quest → Frame step: eyes and input")
    );
    for client in [&old, &few] {
        let (none, panels) = volumes(client);
        assert!(none.is_empty());
        assert_eq!(panels[0].body, volume.body);
    }
    // Nothing was wrong with it, so the agent is told nothing of it.
    assert!(hub.lock().space(&space).unwrap().model.notes().is_empty());
}

#[test]
fn a_block_with_the_id_of_a_volume_updates_it_where_it_stands_and_the_agent_is_told_what_was_wrong()
{
    let (hub, _jobs) = hub_waiting();
    let (space, commands) = idle(&hub);
    let ada = Client::drawing_volumes(&hub, "ada", 16, 1024);
    ada.join(&space);
    reply(&hub, &space, "m1", BLOCK);
    let (before, _) = volumes(&ada);
    // A node added, with a misspelt edge and a layout Tomori does not know.
    let grown = BLOCK
        .replace(
            "renderer --> xr\n",
            "renderer --> xr\nfaces --> atlas[(Glyph atlas)]\nrender --> atlas\n",
        )
        .replace("layout: layered down", "layout: spiral");
    reply(&hub, &space, "m2", &grown);
    let (after, panels) = volumes(&ada);
    assert_eq!(panels.len(), 1, "updated, not projected again");
    assert_eq!(after[0].id, before[0].id);
    let (was, now) = (
        before[0].diagram.as_ref().unwrap(),
        after[0].diagram.as_ref().unwrap(),
    );
    assert_eq!(now.nodes.len(), 6);
    // What stood before keeps its order across the rank.
    let x = |diagram: &tomori_protocol::diagram::Diagram, id: &str| {
        diagram.nodes[diagram.node(id).unwrap()].at[0]
    };
    assert_eq!(
        x(was, "xr") <= x(was, "renderer"),
        x(now, "xr") <= x(now, "renderer")
    );
    assert!(
        after[0]
            .meta
            .iter()
            .any(|meta| meta.key == "warnings" && meta.value == "2")
    );
    // The agent is told before the next words, by the block's own lines.
    let notes = hub
        .lock()
        .space(&space)
        .unwrap()
        .model
        .notes()
        .iter()
        .map(|note| note.to_string())
        .collect::<Vec<_>>();
    assert_eq!(notes.len(), 1);
    let note = &notes[0];
    assert!(
        note.starts_with("Tomori showed the diagram \"How a frame is drawn\" (id renderer, panel-"),
        "{note}"
    );
    assert!(
        note.contains(
            "line 16: 'render' is not a node; made a box called render (did you mean renderer?)"
        ),
        "{note}"
    );
    assert!(
        note.contains("layout 'spiral' is not one Tomori knows"),
        "{note}"
    );
    let prompt = said(&hub, &ada, &commands, Vec::new(), None);
    let text = &prompt.text;
    assert!(prompt.text.contains("[Tomori showed the diagram"), "{text}");
}

#[test]
fn show_diagram_projects_a_volume_and_answers_with_its_warnings_at_once() {
    let (hub, _jobs) = hub_waiting();
    hub.serve_tools(tomori_backend::ToolServer::new(
        "tomori",
        "http://127.0.0.1:1/mcp",
    ));
    let (space, _commands) = idle(&hub);
    let ada = Client::drawing_volumes(&hub, "ada", 16, 1024);
    ada.join(&space);
    let answer = call(
        &hub,
        "show_diagram",
        json!({
            "title": "From a question to an answer",
            "id": "flow",
            "layout": "force",
            "nodes": [
                {"id": "hub", "label": "Hub", "shape": "sphere", "size": 5, "colour": "teal", "detail": "Spaces under one lock"},
                {"id": "mcp", "label": "MCP endpoint"},
            ],
            "edges": [
                {"from": "mcp", "to": "hub", "label": "acts on the space"},
                {"from": "hub", "to": "opencod"},
            ],
        }),
    );
    let result = &answer["structuredContent"];
    assert_eq!(result["id"], "flow");
    assert_eq!(result["updated"], false);
    assert_eq!(
        result["warnings"],
        json!(["\"opencod\" is not a node; made a box called opencod"])
    );
    assert!(result["size"][0].as_f64().unwrap() > 0.1);
    let (shown, _) = volumes(&ada);
    assert_eq!(
        shown[0].diagram.as_ref().unwrap().layout,
        tomori_protocol::diagram::Layout::Force
    );
    // Shown again by its id, from a block's body, it is updated where it is.
    let answer = call(
        &hub,
        "show_diagram",
        json!({"id": "flow", "title": "Again", "source": "hub((Hub)) --> mcp\nmcp --> opencode", "layout": "spiral"}),
    );
    let result = &answer["structuredContent"];
    assert_eq!(result["updated"], true);
    assert_eq!(result["panel"], shown[0].id);
    assert_eq!(
        result["warnings"],
        json!(["\"spiral\" is not a layout; laid out layered down"])
    );
    assert!(result["next"].as_str().unwrap().contains("fix them"));
    let answer = call(
        &hub,
        "show_diagram",
        json!({"source": "a --> b", "layout": "LR", "place": "wall status wall"}),
    );
    assert_eq!(answer["structuredContent"]["warnings"], json!([]));
    // Many warnings are answered with the first and how many more, as a block's are told.
    let source = format!("a --> b\n{}", "?? nonsense\n".repeat(20));
    let answer = call(&hub, "show_diagram", json!({ "source": source }));
    let warnings = answer["structuredContent"]["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), tomori_protocol::diagram::WARNINGS + 1);
    assert_eq!(warnings.last().unwrap(), "and 12 more");
    // Refused: a diagram of nothing, a bad id, a place Tomori does not know, one too large, or an argument it does not
    // take.
    for (arguments, error) in [
        (
            json!({"source": "%% nothing"}),
            "A diagram needs at least one node.",
        ),
        (
            json!({"id": "not an id!", "source": "a"}),
            "An id is 1 to 32 letters, digits, - and _, such as renderer.",
        ),
        (json!({"nodes": {"id": "a"}}), "nodes must be a list."),
    ] {
        let answer = call(&hub, "show_diagram", arguments);
        assert_eq!(answer["isError"], true);
        assert_eq!(answer["content"][0]["text"], error);
    }
    for arguments in [
        json!({"source": "a", "place": "behind the moon"}),
        json!({"source": "a", "place": "near Nobody"}),
        json!({"source": "a", "title": 3}),
        json!({"source": "a", "id": 3}),
        json!({"source": "a", "place": 3}),
        json!({"source": 3}),
    ] {
        assert_eq!(call(&hub, "show_diagram", arguments)["isError"], true);
    }
    // A volume is read back by panels, its outline its body, and is not updated by update_panel.
    let answer = call(&hub, "panels", json!({"panel": shown[0].id}));
    let read = &answer["structuredContent"]["panels"][0];
    assert_eq!(read["kind"], "diagram");
    assert_eq!(
        (read["node_count"].as_u64(), read["edge_count"].as_u64()),
        (Some(3), Some(2))
    );
    let answer = call(
        &hub,
        "update_panel",
        json!({"panel": shown[0].id, "title": "Renamed"}),
    );
    assert_eq!(
        answer["content"][0]["text"],
        "A diagram is updated with show_diagram, or a diagram block, of its id."
    );
    let large: Vec<Value> = (0..300)
        .map(|index| json!({"id": format!("n{index}"), "detail": "x".repeat(100)}))
        .collect();
    let answer = call(&hub, "show_diagram", json!({"nodes": large}));
    assert!(
        answer["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("A diagram is at most 16 KiB")
    );
    let answer = call(
        &hub,
        "show_diagram",
        json!({"nodes": [{"id": "a", "fill": "#fff"}]}),
    );
    assert_eq!(answer["isError"], true);
}

#[test]
fn show_diagram_is_refused_without_a_space_bound_to_its_session() {
    let (hub, _jobs) = hub_waiting();
    hub.serve_tools(tomori_backend::ToolServer::new(
        "tomori",
        "http://127.0.0.1:1/mcp",
    ));
    let answer = call(&hub, "show_diagram", json!({"source": "a"}));
    assert_eq!(answer["isError"], true);
}

#[test]
fn pointing_at_a_node_or_an_edge_names_it_to_the_agent_with_each_hand() {
    let (hub, _jobs) = hub_waiting();
    let (space, commands) = idle(&hub);
    let ada = Client::drawing_volumes(&hub, "ada", 16, 1024);
    ada.join(&space);
    reply(&hub, &space, "m1", BLOCK);
    let (shown, _) = volumes(&ada);
    let panel = shown[0].id.clone();
    let prompt = said(
        &hub,
        &ada,
        &commands,
        vec![
            tomori_protocol::Resource::volume_at(&panel, Some("step"), None),
            tomori_protocol::Resource::volume_at(&panel, None, Some(1)),
        ],
        None,
    );
    let text = &prompt.text;
    assert!(
        text.contains("[Pointing at node \"Frame step\" (id step) in diagram \"How a frame is drawn\" (id renderer, "),
        "{text}"
    );
    assert!(prompt.text.contains("; sphere, in group \"tomori-scene\"; edges: from client-quest \"eyes and input\", to projection::faces \"panels changed\", to Renderer; ref: \"crates/scene/src/scene.rs\""), "{text}");
    assert!(
        prompt.text.contains(
            "edge \"Frame step → projection::faces: panels changed\" (edge 1) in diagram"
        ),
        "{text}"
    );
}

#[test]
fn a_volume_goes_onto_the_wall_whoever_asked_was_looking_at_or_else_floats_beside_them() {
    let (hub, _jobs) = hub_waiting();
    let (space, commands) = idle(&hub);
    let ada = Client::drawing_volumes(&hub, "ada", 16, 1024);
    ada.join(&space);
    // A wall ahead of her, 1.8 m away, which she looks at as she asks.
    ada.send(ClientMessage::AddWall {
        pose: tomori_protocol::Pose::upright_facing([0.0, 1.5, -1.8], [0.0, 1.5, 0.0]),
    });
    let wall = ada.projection().walls.last().unwrap().id.clone();
    ada.send(looking([0.0, 1.6, 0.0], [0.0, 1.5, -1.8]));
    let prompt = said(&hub, &ada, &commands, Vec::new(), None);
    hub.drive_event(&space, Event::Delivered { key: prompt.key });
    reply(&hub, &space, "m1", BLOCK);
    let (shown, _) = volumes(&ada);
    let tomori_protocol::Place::Wall { wall: on, .. } = &shown[0].place else {
        unreachable!("on the wall she looked at, not {:?}", shown[0].place)
    };
    assert_eq!(*on, wall);
    // Asked again looking away from every wall, its next goes beside her.
    ada.send(looking([0.0, 1.6, 0.0], [0.0, 1.5, 6.0]));
    hub.drive_event(&space, Event::Idle);
    let prompt = said(&hub, &ada, &commands, Vec::new(), None);
    hub.drive_event(&space, Event::Delivered { key: prompt.key });
    reply(
        &hub,
        &space,
        "m2",
        &BLOCK.replace("id: renderer", "id: other"),
    );
    let (shown, _) = volumes(&ada);
    let tomori_protocol::Place::Free { .. } = shown[1].place else {
        unreachable!("floating beside her, not {:?}", shown[1].place)
    };
}
