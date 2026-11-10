// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Placing what stands at a space's cluster anchors, through the hub: the agent's workers, with `place_worker`, at an
//! anchor, by the object one stands by, beside a wall or a person, or home, within its limits, a participant's placement
//! winning; its panels, with `place: anchor NAME`; and itself, with `move_agent anchor NAME`; what an anchor's going
//! does to them; and placements kept by anchor name. See `design/environments.md` and `design/mcp.md`.
use super::anchors::{rack, state, worker, yard, yard_with};
use super::*;
use serde_json::json;
use tomori_environment::Outlet;
use tomori_protocol::cluster::{HOME, Layout};

/// Where the space `space` seats the worker `session`: the anchor it stands at and its slot there.
fn seat(state: &tomori_protocol::SpaceState, session: &str) -> Option<(String, u32)> {
    state.clusters.iter().find_map(|layout: &Layout| {
        layout
            .slot(session)
            .map(|slot| (layout.anchor.clone(), slot))
    })
}

/// The workers `presence` lists at the anchor `name`, by session.
fn listed(hub: &Hub, name: &str) -> Vec<String> {
    let presence = hub.tool_presence("session").unwrap();
    presence["anchors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|anchor| anchor["name"] == name)
        .map(|anchor| {
            anchor["workers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|worker| worker["session"].as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn the_agent_places_a_worker_at_an_anchor_by_its_object_beside_a_wall_or_a_person_or_home() {
    let mut child = worker("ses_child");
    child.parent = Some("ses_00".to_owned());
    let (hub, space, _commands, outlet, ada) = yard_with(vec![
        worker("ses_00"),
        worker("ses_01"),
        worker("ses_02"),
        worker("ses_03"),
        child,
    ]);
    let alerting = rack("rack/a12", 4.0)
        .with_kind("rack")
        .with_object("rack/a12");
    assert_eq!(outlet.place_anchor(alerting), Ok(()));
    // At an anchor, by session: it stands there, its own workers with it, everyone sent where.
    let placed = hub
        .agent_place_worker("session", "ses_00", "anchor rack/a12", None)
        .unwrap();
    assert_eq!(
        (&placed["at"], &placed["worker"], &placed["title"]),
        (
            &json!("rack/a12"),
            &json!("ses_00"),
            &json!("Look at ses_00")
        )
    );
    assert!(placed["position"].is_array());
    let seen = state(&ada).unwrap();
    assert_eq!(seat(&seen, "ses_00"), Some(("rack/a12".to_owned(), 0)));
    assert_eq!(seat(&seen, "ses_child"), Some(("rack/a12".to_owned(), 1)));
    assert_eq!(listed(&hub, "rack/a12"), ["ses_00", "ses_child"]);
    // By its title, by the object the anchor stands by.
    hub.agent_place_worker("session", "Look at ses_01", "object yard:rack/a12", None)
        .unwrap();
    assert_eq!(listed(&hub, "rack/a12"), ["ses_00", "ses_child", "ses_01"]);
    // `workers` says where each stands.
    let workers = hub.tool_workers("session").unwrap();
    let at: Vec<&str> = workers["workers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|worker| worker["at"].as_str().unwrap())
        .collect();
    assert_eq!(at, ["rack/a12", "rack/a12", HOME, HOME, "rack/a12"]);
    // Beside a wall, just beyond its end, in a small cluster of the server's own, sent to everyone and listed.
    hub.agent_place_worker("session", "ses_02", "wall status wall", None)
        .unwrap();
    let seen = state(&ada).unwrap();
    let (by_wall, _) = seat(&seen, "ses_02").unwrap();
    assert!(by_wall.starts_with("tomori/wall/"), "{by_wall}");
    let own = seen.anchors.cluster(&by_wall).unwrap();
    assert_eq!(own.kind.as_deref(), Some("wall"));
    assert!(
        own.label
            .as_deref()
            .unwrap()
            .starts_with("Beside status wall")
    );
    // Beside someone, to their right, placed from where they stand once.
    ada.send(standing([2.0, 1.6, 1.0]));
    hub.agent_place_worker("session", "ses_03", "beside ada", None)
        .unwrap();
    let seen = state(&ada).unwrap();
    let (beside, _) = seat(&seen, "ses_03").unwrap();
    let own = seen.anchors.cluster(&beside).unwrap();
    assert!((own.pose.position[0] - 3.5).abs() < 1e-3, "{:?}", own.pose);
    assert_eq!(own.label.as_deref(), Some("Beside ada"));
    // Home again, it forgets where it was placed.
    hub.agent_place_worker("session", "ses_03", "home", None)
        .unwrap();
    assert_eq!(seat(&state(&ada).unwrap(), "ses_03").unwrap().0, HOME);
    let kept = hub.lock().space(&space).unwrap().places.clone();
    assert_eq!(kept["yard"].len(), 3);
}

#[test]
fn a_placement_is_refused_for_a_worker_or_a_place_not_here_and_beyond_the_agent_s_limits() {
    let mut done = worker("ses_done");
    done.state = tomori_backend::WorkerState::Completed;
    let mut twin = worker("ses_twin");
    twin.title = "Look at ses_00".to_owned();
    let (hub, space, _commands, outlet, ada) =
        yard_with(vec![worker("ses_00"), worker("ses_01"), done, twin]);
    assert_eq!(outlet.place_anchor(rack("rack/a12", 4.0)), Ok(()));
    assert_eq!(outlet.place_anchor(rack("rack/a13", 4.0)), Ok(()));
    assert_eq!(outlet.place_anchor(rack("aisle-rack", 4.0)), Ok(()));
    let refused = |worker: &str, place: &str| {
        hub.agent_place_worker("session", worker, place, None)
            .unwrap_err()
    };
    assert!(refused("ses_nobody", "home").contains("no worker"));
    assert!(refused("Look at ses_00", "home").contains("More than one"));
    assert!(refused("ses_done", "home").contains("finished"));
    assert!(refused("ses_00", "somewhere").contains("does not know"));
    assert!(refused("ses_00", "anchor").contains("does not know"));
    // An anchor not here names those nearest in name.
    let near = refused("ses_00", "anchor rack/a1");
    assert!(near.contains("Nearest in name: rack/a12, rack/a13."));
    // A session not bound to a space is refused.
    assert!(
        hub.agent_place_worker("nobody", "ses_00", "home", None)
            .unwrap_err()
            .contains("not bound")
    );
    assert!(!refused("ses_00", "anchor zzz").contains("Nearest"));
    assert!(refused("ses_00", "object yard:hen/marigold").contains("No anchor"));
    assert!(refused("ses_00", "wall the moon").contains("no wall"));
    assert!(refused("ses_00", "beside bob").contains("bob"));
    // Placed again of its own accord within a minute, refused; asked for, by whoever's words it answers, not.
    hub.agent_place_worker("session", "ses_00", "anchor rack/a12", None)
        .unwrap();
    hub.agent_place_worker("session", "ses_00", "home", None)
        .unwrap();
    assert!(refused("ses_00", "anchor rack/a12").contains("last minute"));
    give(&hub, &space, "ada", Duration::ZERO);
    hub.agent_place_worker("session", "ses_00", "anchor rack/a12", Some("ada"))
        .unwrap();
    // Someone it may not answer for is refused.
    assert!(
        hub.agent_place_worker("session", "ses_01", "home", Some("bob"))
            .is_err()
    );
    drop(ada);
}

#[test]
fn a_participant_s_placement_wins_over_the_agent_s_unless_they_ask() {
    let (hub, space, _commands, outlet, ada) = yard(2);
    let lea = Client::projecting(&hub, Access::open(), "lea");
    lea.join(&space);
    assert_eq!(outlet.place_anchor(rack("rack/a12", 4.0)), Ok(()));
    // Ada put it by the rack, as carrying it will.
    hub.lock()
        .space_mut(&space)
        .unwrap()
        .placements_mut()
        .push(crate::places::Placement {
            session: "ses_00".to_owned(),
            at: "rack/a12".to_owned(),
            pose: None,
            beside: None,
            by: Some("ada".to_owned()),
        });
    let presence = hub.tool_presence("session").unwrap();
    let rack = presence["anchors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|anchor| anchor["name"] == "rack/a12")
        .unwrap()
        .clone();
    assert_eq!(rack["workers"][0]["placed_by"], "ada");
    let refused = hub
        .agent_place_worker("session", "ses_00", "home", None)
        .unwrap_err();
    assert!(refused.contains("ada placed that worker"), "{refused}");
    // Asked, it moves, staying hers, and a notice says what moved.
    give(&hub, &space, "ada", Duration::ZERO);
    hub.agent_place_worker("session", "ses_00", "wall status wall", Some("ada"))
        .unwrap();
    let kept = hub.lock().space(&space).unwrap().placed_by("ses_00");
    assert_eq!(kept.as_deref(), Some("ada"));
    let notices: Vec<String> = lea
        .projection()
        .panels
        .iter()
        .filter(|panel| panel.title == "Moved")
        .map(|panel| panel.body.clone())
        .collect();
    assert_eq!(notices.len(), 1);
    assert!(
        notices[0]
            .starts_with("Tomori moved worker 'Look at ses_00', which ada placed, to tomori/wall/")
    );
    // Sent home as asked, the notice says so.
    give(&hub, &space, "ada", Duration::ZERO);
    hub.agent_place_worker("session", "ses_00", "home", Some("ada"))
        .unwrap();
    assert!(
        lea.projection()
            .panels
            .iter()
            .any(|panel| panel.body.ends_with("which ada placed, to home."))
    );
    drop(ada);
}

#[test]
fn panels_at_an_anchor_float_in_its_cluster_move_with_it_and_are_left_floating_as_it_goes() {
    let (hub, space, _commands, outlet, ada) = yard(1);
    let lea = Client::projecting(&hub, Access::open(), "lea");
    lea.join(&space);
    let start = Instant::now();
    assert_eq!(
        hub.channel_place_anchor_at(&outlet, rack("rack/a12", 4.0), start),
        Ok(())
    );
    hub.agent_place_worker("session", "ses_00", "anchor rack/a12", None)
        .unwrap();
    // Two panels of one reply at the anchor each take a slot of their own beside the worker; one at an anchor not here
    // goes where it would.
    reply(
        &hub,
        &space,
        "m1",
        "```panel\ntitle: Load\nplace: anchor rack/a12\n---\nHigh.\n```\n```panel\ntitle: Fans\nplace: anchor rack/a12\n---\nLoud.\n```\n```panel\ntitle: Elsewhere\nplace: anchor rack/b7\n---\nx\n```\n",
    );
    let floating = |projection: &tomori_protocol::Projection, title: &str| {
        projection
            .panels
            .iter()
            .find(|panel| panel.title == title)
            .map(|panel| panel.place.clone())
            .unwrap()
    };
    let shown = lea.projection();
    let cluster = tomori_protocol::cluster::Cluster::new(
        &hub.lock()
            .space(&space)
            .unwrap()
            .clusters(&hub.environments())
            .into_iter()
            .find(|anchor| anchor.name == "rack/a12")
            .unwrap(),
    );
    let laid = cluster.arrange(3);
    for (title, slot) in [("Load", 1), ("Fans", 2)] {
        let tomori_protocol::Place::Free { pose } = floating(&shown, title) else {
            unreachable!("{title} floats")
        };
        assert_eq!(pose.position, cluster.slot(&laid, slot).position);
    }
    let tomori_protocol::Place::Wall { .. } = floating(&shown, "Elsewhere") else {
        unreachable!("it goes where it would, onto a wall")
    };
    let presence = hub.tool_presence("session").unwrap();
    let at = presence["anchors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|anchor| anchor["name"] == "rack/a12")
        .unwrap()
        .clone();
    assert_eq!(at["panels"].as_array().unwrap().len(), 2);
    // `move_panel` puts one there too.
    let elsewhere = shown
        .panels
        .iter()
        .find(|panel| panel.title == "Elsewhere")
        .unwrap()
        .id
        .clone();
    hub.agent_move_panel("session", &elsewhere, "anchor rack/a12")
        .unwrap();
    assert!(
        hub.agent_move_panel("session", &elsewhere, "anchor rack/b7")
            .unwrap_err()
            .contains("no anchor")
    );
    // The environment moves the anchor: its panels move with it.
    let before = floating(&lea.projection(), "Load");
    assert_eq!(
        hub.channel_place_anchor_at(
            &outlet,
            rack("rack/a12", -4.0),
            start + Duration::from_secs(1)
        ),
        Ok(())
    );
    let after = floating(&lea.projection(), "Load");
    assert_ne!(before, after);
    // It goes: its worker goes home, forgetting where it was placed, its panels stay where they float, and a notice
    // says so.
    state(&ada);
    assert_eq!(
        hub.channel_remove_anchor_at(&outlet, "rack/a12", start + Duration::from_secs(2)),
        Ok(())
    );
    let projection = lea.projection();
    assert_eq!(floating(&projection, "Load"), after);
    let notice = projection
        .panels
        .iter()
        .find(|panel| panel.title == "Places")
        .unwrap();
    assert_eq!(
        notice.body,
        "The environment closed rack/a12: 1 worker went home, 3 panels were left floating."
    );
    assert_eq!(seat(&state(&ada).unwrap(), "ses_00").unwrap().0, HOME);
    // Come back, nothing returns to it.
    assert_eq!(
        hub.channel_place_anchor_at(
            &outlet,
            rack("rack/a12", 4.0),
            start + Duration::from_secs(3)
        ),
        Ok(())
    );
    assert_eq!(seat(&state(&ada).unwrap(), "ses_00").unwrap().0, HOME);
    assert_eq!(floating(&lea.projection(), "Load"), after);
    // One that goes with nothing at it says nothing.
    assert_eq!(
        hub.channel_place_anchor_at(
            &outlet,
            rack("rack/b7", 4.0),
            start + Duration::from_secs(4)
        ),
        Ok(())
    );
    hub.flush_anchors(start + Duration::from_secs(4));
    let notices = |projection: &tomori_protocol::Projection| {
        projection
            .panels
            .iter()
            .filter(|panel| panel.title == "Places")
            .count()
    };
    let counted = notices(&lea.projection());
    assert_eq!(
        hub.channel_remove_anchor_at(&outlet, "rack/b7", start + Duration::from_secs(4)),
        Ok(())
    );
    hub.flush_anchors(start + Duration::from_secs(6));
    assert_eq!(notices(&lea.projection()), counted);
    // A panel pulled off by a participant leaves its cluster.
    assert_eq!(
        hub.channel_place_anchor_at(
            &outlet,
            rack("rack/c3", 2.0),
            start + Duration::from_secs(7)
        ),
        Ok(())
    );
    hub.agent_move_panel("session", &elsewhere, "anchor rack/c3")
        .unwrap();
    let anchored = hub.lock().space(&space).unwrap().stage.anchored("yard");
    assert_eq!(anchored.len(), 1);
    lea.send(ClientMessage::PlacePanel {
        panel: elsewhere.clone(),
        wall: None,
        slot: None,
        pose: Some(upright([0.5, 1.4, -1.0])),
    });
    assert!(
        hub.lock()
            .space(&space)
            .unwrap()
            .stage
            .anchored("yard")
            .is_empty()
    );
}

#[test]
fn the_agent_goes_to_an_anchor_facing_whoever_watches_it_within_its_limits() {
    let (hub, space, _commands, outlet, ada) = yard(0);
    let start = Instant::now();
    assert_eq!(
        hub.channel_place_anchor_at(&outlet, rack("rack/a12", 4.0), start),
        Ok(())
    );
    let moved = hub
        .agent_move_agent("session", "anchor rack/a12", None)
        .unwrap();
    assert_eq!(
        (&moved["moved"], &moved["agent"]["anchor"]),
        (&json!(true), &json!("rack/a12"))
    );
    // At the anchor's point, the first to arrive there, and listed there.
    let summoned = hub.lock().space(&space).unwrap().summoned.unwrap();
    assert_eq!(summoned.position, [4.0, 1.4, -8.0]);
    let presence = hub.tool_presence("session").unwrap();
    assert_eq!(presence["agent"]["anchor"], "rack/a12");
    let at = presence["anchors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|anchor| anchor["name"] == "rack/a12")
        .unwrap()
        .clone();
    assert_eq!(at["agent"], true);
    // Already there, it does not move; of its own accord again within 20 s, it is refused; asked, it goes.
    let again = hub
        .agent_move_agent("session", "anchor rack/a12", None)
        .unwrap();
    assert_eq!(again["moved"], false);
    assert!(
        hub.agent_move_agent("session", "anchor aisle-3", None)
            .unwrap_err()
            .contains("20 seconds")
    );
    give(&hub, &space, "ada", Duration::ZERO);
    let asked = hub
        .agent_move_agent("session", "anchor aisle-3", Some("ada"))
        .unwrap();
    assert_eq!(asked["agent"]["anchor"], "aisle-3");
    assert!(
        hub.agent_move_agent("session", "anchor rack/b7", Some("ada"))
            .unwrap_err()
            .contains("no anchor")
    );
    assert!(
        hub.agent_move_agent("session", "anchor rack/a12", Some("bob"))
            .is_err()
    );
    // Sent home, it is no longer there.
    hub.agent_move_agent("session", "home", None).unwrap();
    assert!(
        hub.tool_presence("session").unwrap()["agent"]
            .get("anchor")
            .is_none()
    );
    // Where the space keeps it home, refused.
    ada.send(ClientMessage::SetAgentMovement {
        movement: tomori_protocol::AgentMovement::new(tomori_protocol::Movement::Home, None),
    });
    assert!(
        hub.agent_move_agent("session", "anchor aisle-3", None)
            .unwrap_err()
            .contains("keeps you at home")
    );
}

#[test]
fn placements_are_kept_by_anchor_name_and_wait_for_the_side_to_place_its_anchors_again() {
    let (hub, space, _commands, outlet, ada) = yard(2);
    let start = Instant::now();
    assert_eq!(
        hub.channel_place_anchor_at(&outlet, rack("rack/a12", 4.0), start),
        Ok(())
    );
    hub.agent_place_worker("session", "ses_00", "anchor rack/a12", None)
        .unwrap();
    hub.agent_place_worker("session", "ses_01", "anchor aisle-3", None)
        .unwrap();
    // Kept with the space, for its environment.
    let kept = hub.lock().kept();
    assert_eq!(kept[0].places["yard"].len(), 2);
    // In another environment, at home; back, where its anchors are: the aisle at once, the rack once its side places
    // it again, as after a restart.
    ada.send(ClientMessage::SetEnvironment {
        environment: "sky".to_owned(),
    });
    assert_eq!(seat(&state(&ada).unwrap(), "ses_01").unwrap().0, HOME);
    ada.send(ClientMessage::SetEnvironment {
        environment: "yard".to_owned(),
    });
    let back = state(&ada).unwrap();
    assert_eq!(seat(&back, "ses_01").unwrap().0, "aisle-3");
    assert_eq!(seat(&back, "ses_00").unwrap().0, HOME);
    let binding = hub.server_sides.binding(&space).unwrap();
    let current = SpaceOutlet { binding, ..outlet };
    // Within the grace, a sweep keeps it waiting; the rack placed again, it goes there.
    let anchored = hub.lock().space(&space).unwrap().anchored;
    hub.sweep_places(anchored + Duration::from_secs(30));
    assert_eq!(hub.lock().space(&space).unwrap().placements().len(), 2);
    assert_eq!(current.place_anchor(rack("rack/a12", 4.0)), Ok(()));
    assert_eq!(seat(&state(&ada).unwrap(), "ses_00").unwrap().0, "rack/a12");
    // Removed again by the side going, and not back within 60 s, it is forgotten, silently.
    ada.send(ClientMessage::SetEnvironment {
        environment: "sky".to_owned(),
    });
    ada.send(ClientMessage::SetEnvironment {
        environment: "yard".to_owned(),
    });
    let anchored = hub.lock().space(&space).unwrap().anchored;
    hub.sweep_places(anchored + crate::places::GRACE);
    let placements = hub.lock().space(&space).unwrap().placements().to_vec();
    assert_eq!(placements.len(), 1);
    assert_eq!(placements[0].at, "aisle-3");
    // A worker removed is forgotten too, once the space's list of workers is complete.
    hub.drive_event(
        &space,
        Event::WorkersChanged {
            workers: vec![worker("ses_00")],
        },
    );
    hub.sweep_places(Instant::now());
    assert!(hub.lock().space(&space).unwrap().placements().is_empty());
}

#[test]
fn the_agent_at_an_anchor_that_goes_stays_where_it_is_and_the_home_anchor_is_home() {
    let (hub, space, _commands, outlet, _ada) = yard(0);
    let start = Instant::now();
    assert_eq!(
        hub.channel_place_anchor_at(&outlet, rack("rack/a12", 4.0), start),
        Ok(())
    );
    hub.agent_move_agent("session", "anchor rack/a12", None)
        .unwrap();
    let there = hub.lock().space(&space).unwrap().summoned;
    assert_eq!(
        hub.channel_remove_anchor_at(&outlet, "rack/a12", start + Duration::from_secs(1)),
        Ok(())
    );
    let record = |hub: &Hub| {
        let state = hub.lock();
        let record = state.space(&space).unwrap();
        (record.summoned, record.agent_at.clone())
    };
    assert_eq!(record(&hub), (there, None));
    // The home anchor's point is the agent's home.
    let home = hub
        .agent_move_agent("session", "anchor:home", None)
        .unwrap();
    assert_eq!(home["agent"]["at_home"], true);
}

#[test]
fn a_panel_at_an_anchor_not_here_floats_where_it_stood_and_joins_its_cluster_as_it_comes_back() {
    let (hub, space, _commands, outlet, ada) = yard(1);
    let lea = Client::projecting(&hub, Access::open(), "lea");
    lea.join(&space);
    assert_eq!(outlet.place_anchor(rack("rack/a12", 4.0)), Ok(()));
    reply(
        &hub,
        &space,
        "m1",
        "```panel\ntitle: Load\nplace: anchor rack/a12\n---\nHigh.\n```\n",
    );
    let place = |lea: &Client| {
        lea.projection()
            .panels
            .iter()
            .find(|panel| panel.title == "Load")
            .map(|panel| panel.place.clone())
            .unwrap()
    };
    let stood = place(&lea);
    // A new binding forgets the rack until its side places it again: the panel floats where it stood meanwhile.
    ada.send(ClientMessage::SetEnvironment {
        environment: "sky".to_owned(),
    });
    ada.send(ClientMessage::SetEnvironment {
        environment: "yard".to_owned(),
    });
    hub.sweep_places(Instant::now());
    assert_eq!(place(&lea), stood);
    // Placed again elsewhere, the panel joins it there.
    let binding = hub.server_sides.binding(&space).unwrap();
    let current = SpaceOutlet { binding, ..outlet };
    assert_eq!(current.place_anchor(rack("rack/a12", -4.0)), Ok(()));
    assert_ne!(place(&lea), stood);
}

#[test]
fn panels_at_an_anchor_glide_as_its_cluster_is_laid_out_again_for_workers_arriving_there() {
    let (hub, space, _commands, _outlet, ada) = yard(1);
    let lea = Client::projecting(&hub, Access::open(), "lea");
    lea.join(&space);
    // A worker at the aisle, then a panel beside it.
    hub.agent_place_worker("session", "ses_00", "anchor aisle-3", None)
        .unwrap();
    reply(
        &hub,
        &space,
        "m1",
        "```panel\ntitle: Notes\nplace: anchor aisle-3\n---\nx\n```\n",
    );
    let place = |lea: &Client| {
        lea.projection()
            .panels
            .iter()
            .find(|panel| panel.title == "Notes")
            .map(|panel| panel.place.clone())
            .unwrap()
    };
    let before = place(&lea);
    // The worker's own workers join it, as many as fill the aisle past a step that moves the panel's slot: as the hub
    // sweeps, the panel moves to its slot there.
    let aisle = hub
        .lock()
        .space(&space)
        .unwrap()
        .clusters(&hub.environments())
        .into_iter()
        .find(|anchor| anchor.name == "aisle-3")
        .unwrap();
    let cluster = tomori_protocol::cluster::Cluster::new(&aisle);
    let at = |span: usize| cluster.slot(&cluster.arrange(span), 1).position;
    let span = (3..32).find(|span| at(*span) != at(2)).unwrap();
    let children: Vec<tomori_backend::Worker> = std::iter::once(worker("ses_00"))
        .chain((0..span - 2).map(|index| {
            let mut child = worker(&format!("ses_child_{index}"));
            child.parent = Some("ses_00".to_owned());
            child
        }))
        .collect();
    hub.drive_event(&space, Event::WorkersChanged { workers: children });
    state(&ada);
    hub.sweep_places(Instant::now());
    let tomori_protocol::Place::Free { pose } = place(&lea) else {
        unreachable!("it floats")
    };
    assert_ne!(place(&lea), before);
    assert_eq!(pose.position, at(span));
    // Where it stands, it is left: settling again moves nothing.
    let environments = hub.environments();
    assert!(!hub.lock().space_mut(&space).unwrap().settle(&environments));
}

#[test]
fn a_more_sign_gathering_panels_names_only_its_workers_to_the_agent() {
    let (hub, space, _commands, outlet, _ada) = yard(1);
    assert_eq!(outlet.place_anchor(rack("rack/a12", 4.0)), Ok(()));
    let blocks: String = (0..14)
        .map(|index| format!("```panel\ntitle: P{index}\nplace: anchor rack/a12\n---\nx\n```\n"))
        .collect();
    reply(&hub, &space, "m1", &blocks);
    hub.agent_place_worker("session", "ses_00", "anchor rack/a12", None)
        .unwrap();
    let state = hub.lock();
    let record = state.space(&space).unwrap();
    let environments = hub.environments();
    let anchor = record
        .clusters(&environments)
        .into_iter()
        .find(|anchor| anchor.name == "rack/a12")
        .unwrap();
    assert!(record.seated(&environments).gathered(&anchor).len() > 1);
    assert_eq!(
        record.gathered(&environments, "rack/a12"),
        Some(vec!["ses_00".to_owned()])
    );
}

#[test]
fn a_worker_placed_beside_whoever_asked_stands_to_their_right_as_they_stood_as_they_spoke() {
    let (hub, space, commands, _outlet, ada) = yard(1);
    ada.send(standing([0.0, 1.6, 1.0]));
    said(&hub, &ada, &commands, Vec::new(), None);
    hub.agent_place_worker("session", "ses_00", "beside", None)
        .unwrap();
    let seen = state(&ada).unwrap();
    let (beside, _) = seat(&seen, "ses_00").unwrap();
    let own = seen.anchors.cluster(&beside).unwrap();
    assert_eq!(own.label.as_deref(), Some("Beside whoever asked"));
    assert!((own.pose.position[0] - 1.5).abs() < 1e-3);
    drop(space);
}
