// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Diagrams' world references through the hub: resolved against what the space has, objects, anchors, walls, panels,
//! workers, its participants and the agent, the agent told what each named, and of each that names nothing here, in the
//! same turn for `show_diagram` and before the next words for a block; never a person in another space, nor one whose
//! presence is stale; sent to clients that draw world edges, and listed in the outline for those that do not; placed
//! near what they point at; and a world edge pointed at named by both its ends. See `design/diagrams.md`.
use super::anchors::{worker, yard_with};
use super::*;
use serde_json::{Value, json};
use tomori_protocol::world::State;
use tomori_protocol::{Limits, Panel};

/// A client called `participant` that draws volumes, and their world edges where `world_edges` says.
fn seeing(hub: &Arc<Hub>, participant: &str, world_edges: bool) -> Client {
    let client = Client::connect(hub, Access::open());
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
        world_edges,
        projection_patches: false,
        live_panels: false,
        limits: Some(Limits::TODAY.with_volumes(16, 1024)),
    });
    client.read();
    client
}

/// Call the tool `name` with `arguments` as the agent of the session `session` would, over MCP, for its result.
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
    .unwrap()["result"]["structuredContent"]
        .clone()
}

/// What the yard's side describes: a hen standing two metres ahead, a fence post with no place, and a stone the agent
/// cannot name, having no reference.
fn objects() -> Vec<tomori_environment::Object> {
    vec![
        tomori_environment::Object::new("chicken-1", "Pip, a hen, pecking")
            .with_reference("hen/pip")
            .at([1.0, 0.4, -2.0])
            .with_extent(0.3),
        tomori_environment::Object::new("post-1", "a fence post").with_reference("post/1"),
        tomori_environment::Object::new("stone-1", "a stone"),
    ]
}

/// The volumes `client` is sent.
fn volumes(client: &Client) -> Vec<Panel> {
    client
        .projection()
        .panels
        .into_iter()
        .filter(|panel| panel.diagram.is_some())
        .collect()
}

const SOURCE: &str = concat!(
    "pip := object yard:hen/pip [Pip]\n",
    "post := object post/1\n",
    "aisle := anchor aisle-3\n",
    "status := wall status wall\n",
    "builder := worker ses_00\n",
    "sam := person sam\n",
    "me := agent\n",
    "bob := person bob\n",
    "typo := object yard:hen/pi\n",
    "feed[1. Feed Pip] --> pip : here\n",
    "feed --> post & aisle & status & builder & sam & me & bob & typo\n",
);

#[test]
fn world_references_resolve_to_what_the_space_has_and_the_agent_is_told_at_once() {
    let (hub, space, commands, outlet, _ada) = yard_with(vec![worker("ses_00")]);
    assert_eq!(hub.channel_describe(&outlet, objects()), Ok(()));
    let sam = seeing(&hub, "sam", true);
    let old = seeing(&hub, "old", false);
    for client in [&sam, &old] {
        client.join(&space);
    }
    sam.send(standing([0.0, 1.6, 0.0]));
    // Sam asks, so that distances are told from him and the volume stands near what it points at for him.
    let prompt = said(&hub, &sam, &commands, Vec::new(), None);
    hub.drive_event(&space, Event::Delivered { key: prompt.key });
    let result = call(
        &hub,
        "show_diagram",
        json!({"title": "Feeding", "id": "feed", "place": "near references", "source": SOURCE}),
    );
    let world: Vec<&str> = result["world"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line.as_str().unwrap())
        .collect();
    assert_eq!(world.len(), 7, "{world:?}");
    assert_eq!(
        world[0],
        "pip → Pip, a hen, pecking (yard:hen/pip), 2.5 m from sam"
    );
    assert_eq!(
        world[1],
        "post → a fence post (yard:post/1), where the environment does not say, so nothing is drawn to it now"
    );
    assert!(
        world[2].starts_with("aisle → anchor aisle-3 \"Aisle 3\" (area) (aisle-3), "),
        "{world:?}"
    );
    assert!(
        world[3].starts_with("status → the status wall ("),
        "{world:?}"
    );
    assert!(
        world[4].starts_with("builder → worker \"Look at ses_00\" (ses_00) (ses_00), "),
        "{world:?}"
    );
    assert!(world[5].starts_with("sam → sam ("), "{world:?}");
    assert!(
        world[6].starts_with("me → you, the agent (agent), "),
        "{world:?}"
    );
    let warnings: Vec<&str> = result["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line.as_str().unwrap())
        .collect();
    assert_eq!(warnings.len(), 2);
    assert!(
        warnings[0].starts_with("bob: no person \"bob\" in this space (there are ada#"),
        "{warnings:?}"
    );
    assert_eq!(
        warnings[1],
        "typo: no object \"yard:hen/pi\" in this space (did you mean yard:hen/pip?); nothing is drawn to it"
    );
    // A client that draws world edges is sent what each resolved to; one that does not, the outline listing them.
    let shown = &volumes(&sam)[0];
    let diagram = shown.diagram.as_ref().unwrap();
    let states: Vec<State> = diagram.world.iter().map(|target| target.state).collect();
    assert_eq!(
        states,
        [
            State::Here,
            State::Absent,
            State::Here,
            State::Here,
            State::Here,
            State::Here,
            State::Here,
            State::Unknown,
            State::Unknown,
        ]
    );
    assert_eq!(diagram.world[0].at, Some([1.0, 0.4, -2.0]));
    assert_eq!(diagram.world[0].radius, 0.3);
    assert!(diagram.world[3].frame.is_some());
    let tomori_protocol::Place::Free { .. } = shown.place else {
        unreachable!("floating near what it points at, not {:?}", shown.place)
    };
    let plain = &volumes(&old)[0];
    assert!(plain.diagram.as_ref().unwrap().world.is_empty());
    assert!(plain.diagram.as_ref().unwrap().world_edges.is_empty());
    assert!(
        plain
            .body
            .contains("- 1. Feed Pip → Pip (yard:hen/pip): here"),
        "{}",
        plain.body
    );
    assert!(plain.body.contains("- 1. Feed Pip → bob"), "{}", plain.body);
    // Pointed at, a world edge is named by both its ends, and a node by what it points at.
    hub.drive_event(&space, Event::Idle);
    let prompt = said(
        &hub,
        &sam,
        &commands,
        vec![
            tomori_protocol::Resource::world_edge_at(&shown.id, 0),
            tomori_protocol::Resource::volume_at(&shown.id, Some("feed"), None),
        ],
        None,
    );
    let text = &prompt.text;
    assert!(
        text.contains("[Pointing at world edge \"1. Feed Pip → Pip: here\" (world edge 0) in diagram \"Feeding\" (id feed, "),
        "{text}"
    );
    assert!(
        text.contains("; from node feed to object \"yard:hen/pip\")"),
        "{text}"
    );
    assert!(
        text.contains("; box; edges: to object \"yard:hen/pip\", to object \"post/1\""),
        "{text}"
    );
}

#[test]
fn only_the_participants_in_the_space_are_named_and_never_where_they_were_last_seen() {
    let (hub, space, _commands, _outlet, _ada) = yard_with(Vec::new());
    let sam = seeing(&hub, "sam", true);
    sam.join(&space);
    sam.send(standing([0.0, 1.6, 0.0]));
    // Bob, in a space of his own, bound to another session, reporting where he is.
    let (bob_commands, _receiver) = async_channel::unbounded();
    let elsewhere = hub
        .register_space(
            &request("bob"),
            "other".to_owned(),
            Snapshot {
                busy: Some(false),
                ..Snapshot::default()
            },
            bob_commands,
        )
        .unwrap();
    let bob = seeing(&hub, "bob", true);
    bob.join(&elsewhere);
    bob.send(standing([3.0, 1.6, 0.0]));
    let result = call(
        &hub,
        "show_diagram",
        json!({"source": "b := person bob\ns := person sam\nn --> b & s\n"}),
    );
    let warning = result["warnings"][0].as_str().unwrap();
    assert!(
        warning.starts_with("b: no person \"bob\" in this space (there are ada#")
            && !warning.contains("bob#"),
        "{warning}"
    );
    assert_eq!(result["warnings"].as_array().unwrap().len(), 1);
    // Five seconds on, without a report, Sam is away: absent, not drawn where he stood.
    let state = hub.lock();
    let environments = hub.environments();
    let projection = {
        let record = state.space(&space).unwrap();
        let walls = environments.walls(&record.model.environment);
        record.stage.view(&space, &walls, crate::cards::unix_now())
    };
    let later = Instant::now() + crate::presence::STALE;
    let things = state.world_things(&space, &environments, &projection, &[], None, later);
    let names: Vec<&str> = things
        .people
        .iter()
        .map(|person| person.label.as_str())
        .collect();
    assert_eq!(names, ["ada", "sam"]);
    assert_eq!(things.people[1].at, None);
    assert_eq!(things.people[1].absent.as_deref(), Some("away"));
    // A space gone has nothing.
    assert!(
        state
            .world_things("nowhere", &environments, &projection, &[], None, later)
            .objects
            .is_empty()
    );
}

#[test]
fn a_block_s_references_are_resolved_as_it_is_projected_and_what_is_wrong_told_before_the_next_words()
 {
    let (hub, space, _commands, outlet, _ada) = yard_with(Vec::new());
    assert_eq!(hub.channel_describe(&outlet, objects()), Ok(()));
    let sam = seeing(&hub, "sam", true);
    sam.join(&space);
    reply(
        &hub,
        &space,
        "m1",
        "```diagram\ntitle: Pip\nid: pip\nplace: near references\n---\npip := object yard:hen/pip\nnope := object yard:hen/nope\na --> pip & nope\n```\n```panel\ntitle: Notes\nplace: near references\n---\nA panel goes beside whoever asked.\n```\n",
    );
    let shown = &volumes(&sam)[0];
    let diagram = shown.diagram.as_ref().unwrap();
    assert_eq!(diagram.world[0].state, State::Here);
    let notes: Vec<String> = hub
        .lock()
        .space(&space)
        .unwrap()
        .model
        .notes()
        .iter()
        .map(|note| note.to_string())
        .collect();
    assert_eq!(notes.len(), 1);
    assert!(
        notes[0].contains("nope: no object 'yard:hen/nope' in this space"),
        "{notes:?}"
    );
    // Shown again by its id, it stays where it stood, laid out from it.
    reply(
        &hub,
        &space,
        "m2",
        "```diagram\ntitle: Pip\nid: pip\n---\npip := object yard:hen/pip\na --> pip\n```\n",
    );
    let again = &volumes(&sam)[0];
    assert_eq!((&again.id, &again.place), (&shown.id, &shown.place));
}

#[test]
fn a_volume_is_laid_out_as_it_will_stand_wherever_it_goes() {
    use crate::stage::Destination;
    let (hub, space, _commands, _outlet, _ada) = yard_with(Vec::new());
    let sam = seeing(&hub, "sam", true);
    sam.join(&space);
    let projection = sam.projection();
    let wall = projection.walls[0].clone();
    let pose = |x: f32| tomori_protocol::Pose::upright_facing([x, 1.5, -2.0], [0.0, 1.5, 0.0]);
    let pose_of = |to: &Destination| super::super::world::volume_pose(to, true, None, &projection);
    assert_eq!(pose_of(&Destination::Free(pose(1.0))), Some(pose(1.0)));
    assert_eq!(
        pose_of(&Destination::Anchor {
            anchor: "aisle-3".into(),
            pose: pose(2.0)
        }),
        Some(pose(2.0))
    );
    let in_slot = pose_of(&Destination::Wall {
        wall: wall.id.clone(),
        slot: Some(0),
    });
    assert_eq!(
        in_slot,
        crate::presence::slot_frame(&wall, 0).map(|(pose, _)| pose)
    );
    assert_eq!(
        pose_of(&Destination::Wall {
            wall: wall.id.clone(),
            slot: Some(99),
        }),
        Some(crate::presence::wall_frame(&wall).0)
    );
    assert_eq!(
        pose_of(&Destination::Wall {
            wall: "nowhere".into(),
            slot: None,
        }),
        None
    );
    assert!(pose_of(&Destination::Nearest([0.0, 1.6, 0.0])).is_some());
    let head = tomori_protocol::Pose::at([0.0, 1.6, 0.0]);
    assert_eq!(
        pose_of(&Destination::Beside {
            head,
            solids: Vec::new()
        }),
        Some(crate::presence::front_of(&head))
    );
    assert!(pose_of(&Destination::Tomori).is_some());
    // Updated where it stands, it stands where it does, unless it is placed afresh.
    let mut standing = Panel::new(
        "panel-1",
        "Volume",
        "",
        tomori_protocol::Place::Free { pose: pose(3.0) },
    );
    let kept = |panel: &Panel| {
        super::super::world::volume_pose(&Destination::Tomori, false, Some(panel), &projection)
    };
    assert_eq!(kept(&standing), Some(pose(3.0)));
    standing.place = tomori_protocol::Place::Wall {
        wall: wall.id.clone(),
        slot: 0,
    };
    assert_eq!(kept(&standing), in_slot);
    standing.place = tomori_protocol::Place::Unknown;
    assert_eq!(kept(&standing), None);
    // Near references, with nobody asking, as panels go by default; asked for by someone with nothing here, beside them.
    let (diagram, _) = tomori_protocol::diagram::parse("a --> b\n");
    assert_eq!(
        super::super::world::near_references(&diagram, None, &projection, &[]),
        crate::presence::by_default(None, &projection, &[])
    );
    let Destination::Beside { .. } =
        super::super::world::near_references(&diagram, Some(&head), &projection, &[])
    else {
        unreachable!("beside whoever asked")
    };
    // Any other panel placed near references goes beside whoever asked; and the form names no wall and no layout.
    let place = crate::stage::Placement::NearReferences;
    let Ok(Destination::Beside { .. }) =
        crate::presence::destination(&place, Some(&head), &[], &projection, &[])
    else {
        unreachable!("beside whoever asked")
    };
    assert!(crate::presence::destination(&place, None, &[], &projection, &[]).is_err());
    assert_eq!(
        crate::presence::which_wall("near references", Some(&head), &[], &projection),
        Err(
            "near references names no wall: say which wall, or looking or near someone.".to_owned()
        )
    );
    let refused = futures_lite::future::block_on(crate::mcp::handle(
        &hub,
        &json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {
                "name": "create_layout",
                "arguments": {"place": "near references", "walls": [{"id": "plan"}]},
                "_meta": {crate::mcp::SESSION_META: "session"},
            },
        }),
    ))
    .unwrap();
    assert!(
        refused
            .to_string()
            .contains("A layout stands in front of someone"),
        "{refused}"
    );
    // A flat wall's frame is its board.
    sam.send(ClientMessage::AddWall {
        pose: tomori_protocol::Pose::upright_facing([0.0, 1.5, -1.8], [0.0, 1.5, 0.0]),
    });
    let added = sam.projection().walls.last().unwrap().clone();
    let (frame, size) = crate::presence::wall_frame(&added);
    let board = added.board().unwrap();
    assert_eq!((frame, size), (added.frame(), [board.0, board.1]));
    drop(space);
}

#[test]
fn a_space_s_things_are_found_where_they_stand_or_said_to_be_away() {
    let (hub, space, _commands, _outlet, _ada) = yard_with(
        (0..30)
            .map(|index| worker(&format!("ses_{index:02}")))
            .collect(),
    );
    let sam = seeing(&hub, "sam", true);
    sam.join(&space);
    sam.send(ClientMessage::AddWall {
        pose: tomori_protocol::Pose::upright_facing([0.0, 1.5, -1.8], [0.0, 1.5, 0.0]),
    });
    let mut projection = sam.projection();
    let added = projection.walls.last().unwrap().id.clone();
    let place = |slot: Option<u32>| match slot {
        Some(slot) => tomori_protocol::Place::Wall {
            wall: added.clone(),
            slot,
        },
        None => tomori_protocol::Place::Unknown,
    };
    projection.panels = vec![
        Panel::new("panel-1", "On a wall", "", place(Some(0))),
        Panel::new("panel-2", "Somewhere new", "", place(None)),
        Panel::new("panel-3", "In a hand", "", place(Some(1))),
        Panel::new(
            "panel-4",
            "Floating",
            "",
            tomori_protocol::Place::Free {
                pose: tomori_protocol::Pose::at([1.0, 1.4, -1.0]),
            },
        ),
        Panel::new(
            "panel-5",
            "Stretched",
            "",
            tomori_protocol::Place::Free {
                pose: tomori_protocol::Pose::at([-1.0, 1.4, -1.0]),
            },
        )
        .with_size(Some(2.0)),
    ];
    let state = hub.lock();
    let things = state.world_things(
        &space,
        &hub.environments(),
        &projection,
        &["panel-3".to_owned()],
        None,
        Instant::now(),
    );
    // A panel in its slot, framed; one placed where no server would, and one carried, away.
    let wall = projection.walls.last().unwrap();
    assert_eq!(
        things.panels[0].at,
        crate::presence::slot_frame(wall, 0).map(|(pose, _)| pose.position)
    );
    assert!(things.panels[0].frame.is_some());
    assert_eq!(things.panels[1].absent.as_deref(), Some("being carried"));
    assert_eq!(things.panels[2].at, None);
    assert_eq!(things.panels[3].at, Some([1.0, 1.4, -1.0]));
    assert_eq!(
        things.panels[3].frame.map(|frame| frame.size),
        Some([
            tomori_protocol::wall::SLOT_SIZE.0,
            tomori_protocol::wall::SLOT_SIZE.1
        ])
    );
    // One stretched by two hands is framed as large as it floats.
    assert_eq!(
        things.panels[4].frame.map(|frame| frame.size),
        Some([
            tomori_protocol::wall::SLOT_SIZE.0 * 2.0,
            tomori_protocol::wall::SLOT_SIZE.1 * 2.0
        ])
    );
    // An added wall is called by its number, never the status wall.
    let named = things
        .walls
        .iter()
        .find(|thing| thing.names[0] == added)
        .unwrap();
    assert!(
        !named.names.contains(&"status wall".to_owned()),
        "{named:?}"
    );
    // Every worker stands somewhere, gathered into its anchor's sign or not.
    assert!(things.workers.iter().all(|worker| worker.at.is_some()));
}
