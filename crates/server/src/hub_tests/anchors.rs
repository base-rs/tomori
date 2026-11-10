// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A space's cluster anchors through the hub: those its environment declares and those its server side places and
//! removes as it runs, held to their limits and rates; where the server seats the workers, the same for every client;
//! and what the agent is told of them, in `presence` and when someone points at a "+N more" sign. See
//! `design/environments.md`.
use super::*;
use serde_json::json;
use tomori_environment::Outlet;
use tomori_protocol::cluster::{Anchor, HOME, Layout};

/// The room about the default home, which holds two workers, then eight compact.
pub(super) const ROOM: (f32, f32, f32) = (4.2, 1.6, 2.0);

pub(super) fn worker(id: &str) -> tomori_backend::Worker {
    tomori_backend::Worker {
        id: id.to_owned(),
        parent: None,
        title: format!("Look at {id}"),
        state: tomori_backend::WorkerState::Running,
    }
}

/// A yard whose server side runs, with a home of its own room and an aisle, and Sky; a space in it with `workers`,
/// whose agent's tools are served; Ada, joined; and an outlet for its side.
pub(super) fn yard(
    workers: usize,
) -> (
    Arc<Hub>,
    String,
    async_channel::Receiver<SpaceCommand>,
    SpaceOutlet,
    Client,
) {
    yard_with(
        (0..workers)
            .map(|index| worker(&format!("ses_{index:02}")))
            .collect(),
    )
}

/// The yard of [`yard`], its space with `workers`.
pub(super) fn yard_with(
    workers: Vec<tomori_backend::Worker>,
) -> (
    Arc<Hub>,
    String,
    async_channel::Receiver<SpaceCommand>,
    SpaceOutlet,
    Client,
) {
    let (hub, _jobs) = hub_waiting();
    hub.serve_tools(tomori_backend::ToolServer::new(
        "tomori",
        "http://127.0.0.1:1/mcp",
    ));
    let home = tomori_protocol::Pose::at(crate::stage::AGENT).facing_towards([0.0, 1.6, 0.0]);
    let aisle = Anchor::new(
        "aisle-3",
        tomori_protocol::Pose::at([-4.0, 1.4, -6.0]).facing_towards([0.0, 1.4, 0.0]),
    )
    .with_kind("area")
    .with_label("Aisle 3");
    hub.set_environments(Environments::new([
        crate::environments::Environment::new("sky", "Sky"),
        crate::environments::Environment::new("yard", "Yard")
            .with_anchor(Anchor::new(HOME, home).with_extent(ROOM.0, ROOM.1, ROOM.2))
            .with_anchor(aisle)
            .with_server_side(Idle),
    ]));
    hub.serving();
    let mut bound = request("ada");
    bound.environment = "yard".to_owned();
    let (id, commands) = running(
        &hub,
        &bound,
        Snapshot {
            busy: Some(false),
            workers,
            ..Snapshot::default()
        },
    );
    let outlet = SpaceOutlet {
        hub: Arc::downgrade(&hub),
        space: id.clone(),
        binding: hub.server_sides.binding(&id).unwrap(),
        environment: "yard".to_owned(),
        version: 1,
    };
    let ada = Client::greeted(&hub, Access::open(), "ada");
    ada.send(ClientMessage::Join { space: id.clone() });
    (hub, id, commands, outlet, ada)
}

/// The space's state `client` was sent last, if any.
pub(super) fn state(client: &Client) -> Option<tomori_protocol::SpaceState> {
    client
        .read()
        .into_iter()
        .filter_map(|message| match message {
            ServerMessage::Space { state } => Some(*state),
            _ => None,
        })
        .next_back()
}

/// The names of the space's cluster anchors in `state`.
pub(super) fn names(state: &tomori_protocol::SpaceState) -> Vec<&str> {
    state
        .anchors
        .clusters
        .iter()
        .map(|anchor| anchor.name.as_str())
        .collect()
}

pub(super) fn rack(name: &str, x: f32) -> Anchor {
    Anchor::new(
        name,
        tomori_protocol::Pose::at([x, 1.4, -8.0]).facing_towards([0.0, 1.4, 0.0]),
    )
    .with_extent(2.0, 2.5, 1.0)
}

#[test]
fn a_side_places_anchors_everyone_sees_that_the_agent_finds_by_kind_and_label() {
    let (hub, _space, _commands, outlet, ada) = yard(0);
    // Those the environment declares are there as the space opens.
    let opened = state(&ada).unwrap();
    assert_eq!(names(&opened), [HOME, "aisle-3"]);
    // One its side places reaches everyone, said to be placed as it runs, its label cleaned.
    let alerting = rack("rack/a12", 4.0)
        .with_kind("rack")
        .with_label("Rack A12,\nalerting: CPU throttled")
        .with_object("rack/a12");
    assert_eq!(outlet.place_anchor(alerting), Ok(()));
    let placed = state(&ada).unwrap();
    assert_eq!(names(&placed), [HOME, "aisle-3", "rack/a12"]);
    let shown = &placed.anchors.clusters[2];
    assert!(shown.placed);
    assert_eq!(
        shown.label.as_deref(),
        Some("Rack A12, alerting: CPU throttled")
    );
    // `presence` lists them, home first, so that the agent finds the rack that is alerting by its kind and label.
    let presence = hub.tool_presence("session").unwrap();
    let anchors = presence["anchors"].as_array().unwrap();
    let listed: Vec<&str> = anchors
        .iter()
        .map(|anchor| anchor["name"].as_str().unwrap())
        .collect();
    assert_eq!(listed, [HOME, "aisle-3", "rack/a12"]);
    assert_eq!(anchors[0]["extent"]["width"], 4.2);
    assert_eq!(anchors[1]["kind"], "area");
    assert_eq!(anchors[1]["extent"]["width"], 1.5);
    assert!(anchors[1].get("placed").is_none());
    let rack = &anchors[2];
    assert_eq!(
        (
            &rack["kind"],
            &rack["label"],
            &rack["placed"],
            &rack["object"]
        ),
        (
            &json!("rack"),
            &json!("Rack A12, alerting: CPU throttled"),
            &json!(true),
            &json!("yard:rack/a12")
        )
    );
    assert_eq!(rack["position"], json!([4.0, 1.4, -8.0]));
    // Facing home, back the way it stands from it.
    assert_eq!(rack["facing_yaw"], json!(-153.0));
    // Removed, once its interval has passed, it goes for everyone.
    let later = Instant::now() + tomori_environment::ANCHOR_INTERVAL;
    assert_eq!(
        hub.channel_remove_anchor_at(&outlet, "rack/a12", later),
        Ok(())
    );
    assert_eq!(names(&state(&ada).unwrap()), [HOME, "aisle-3"]);
    // Removing one that is not there changes nothing, and sends no state, whatever else is said.
    assert_eq!(outlet.remove_anchor("rack/b7"), Ok(()));
    ada.send(ClientMessage::UtteranceEnd {
        utterance: "none".to_owned(),
        pointed: None,
    });
    assert_eq!(state(&ada), None);
}

#[test]
fn a_side_s_anchor_is_refused_for_its_name_its_bounds_and_beyond_the_space_s_limit() {
    let (hub, _space, _commands, outlet, ada) = yard(0);
    state(&ada);
    let refused = |anchor: Anchor| {
        outlet
            .place_anchor(anchor)
            .unwrap_err()
            .message()
            .to_owned()
    };
    // The environment's own places are its own, home among them.
    for (name, why) in [
        ("aisle-3", "declares"),
        (HOME, "declares"),
        ("rack a12", "name"),
        ("tomori/near/1", "Tomori's own"),
    ] {
        assert!(refused(rack(name, 0.0)).contains(why), "{name}");
    }
    assert!(refused(rack("cupboard", 0.0).with_extent(0.1, 1.0, 1.0)).contains("extent"));
    assert!(refused(rack("rack/a1", 0.0).with_kind("a rack")).contains("kind"));
    let far = Anchor::new("hall-9", tomori_protocol::Pose::at([0.0, 1.4, -30.0]));
    assert!(refused(far).contains("20 m"));
    let unturned = Anchor::new(
        "hall-8",
        tomori_protocol::Pose {
            position: [0.0, 1.4, -3.0],
            orientation: [0.0; 4],
        },
    );
    assert!(refused(unturned).contains("orientation"));
    // Two it declares, and so 62 it may place, one at a time as its rate allows, the 63rd refused; placing one already
    // placed again is not a new one.
    let start = Instant::now();
    for index in 0..62 {
        let at = start + Duration::from_secs(index);
        assert_eq!(
            hub.channel_place_anchor_at(&outlet, rack(&format!("rack/{index}"), 0.0), at),
            Ok(()),
            "{index}"
        );
    }
    let later = start + Duration::from_secs(70);
    let over = hub
        .channel_place_anchor_at(&outlet, rack("rack/62", 0.0), later)
        .unwrap_err();
    assert!(over.message().contains("64 anchors"));
    assert_eq!(
        hub.channel_place_anchor_at(&outlet, rack("rack/0", 3.0), later),
        Ok(())
    );
    assert_eq!(state(&ada).unwrap().anchors.clusters.len(), 64);
    // Calls beyond their rate are refused, whatever they change.
    let mut refusal = None;
    for _ in 0..200 {
        if let Err(error) = hub.channel_remove_anchor_at(&outlet, "rack/nothing", later) {
            refusal = Some(error);
            break;
        }
    }
    assert!(refusal.unwrap().message().contains("faster"));
    let placing = hub
        .channel_place_anchor_at(&outlet, rack("rack/0", 5.0), later)
        .unwrap_err();
    assert!(placing.message().contains("faster"));
}

#[test]
fn a_side_s_changes_wait_for_their_rate_the_latest_made_at_the_sweep_and_a_removal_cancels_one_waiting()
 {
    let (hub, space, _commands, outlet, ada) = yard(0);
    state(&ada);
    let start = Instant::now();
    let at = |millis: u64| start + Duration::from_millis(millis);
    let pose = |hub: &Hub, name: &str| {
        let state = hub.lock();
        let record = state.space(&space).unwrap();
        record
            .clusters(&hub.environments())
            .into_iter()
            .find(|anchor| anchor.name == name)
            .map(|anchor| anchor.pose.position)
    };
    assert_eq!(
        hub.channel_place_anchor_at(&outlet, rack("rack/a12", 1.0), start),
        Ok(())
    );
    assert_eq!(pose(&hub, "rack/a12"), Some([1.0, 1.4, -8.0]));
    // Moved again at once, it waits, a later move taking its place, and nothing is sent meanwhile.
    state(&ada);
    for x in [2.0, 3.0] {
        assert_eq!(
            hub.channel_place_anchor_at(&outlet, rack("rack/a12", x), at(100)),
            Ok(())
        );
    }
    assert_eq!(pose(&hub, "rack/a12"), Some([1.0, 1.4, -8.0]));
    assert_eq!(state(&ada), None);
    // Another, meanwhile, is placed at once, the move waiting still.
    assert_eq!(
        hub.channel_place_anchor_at(&outlet, rack("rack/b1", 1.0), at(100)),
        Ok(())
    );
    assert_eq!(pose(&hub, "rack/b1"), Some([1.0, 1.4, -8.0]));
    assert_eq!(pose(&hub, "rack/a12"), Some([1.0, 1.4, -8.0]));
    // Not yet at a sweep within its interval; at one after, the latest.
    hub.flush_anchors(at(500));
    assert_eq!(pose(&hub, "rack/a12"), Some([1.0, 1.4, -8.0]));
    hub.flush_anchors(at(1000));
    assert_eq!(pose(&hub, "rack/a12"), Some([3.0, 1.4, -8.0]));
    assert!(state(&ada).is_some());
    // A removal within its interval waits too, and is made at the next sweep after.
    assert_eq!(
        hub.channel_remove_anchor_at(&outlet, "rack/a12", at(1100)),
        Ok(())
    );
    assert!(pose(&hub, "rack/a12").is_some());
    hub.flush_anchors(at(2000));
    assert_eq!(pose(&hub, "rack/a12"), None);
    // With the space's changes spent, a new one waits; removed before it is made, it never is.
    let burst = at(3000);
    for index in 0..8 {
        let name = format!("rack/{index}");
        assert_eq!(
            hub.channel_place_anchor_at(&outlet, rack(&name, 0.0), burst),
            Ok(())
        );
    }
    assert_eq!(
        hub.channel_place_anchor_at(&outlet, rack("rack/b7", 0.0), burst),
        Ok(())
    );
    assert_eq!(pose(&hub, "rack/b7"), None);
    assert_eq!(
        hub.channel_remove_anchor_at(&outlet, "rack/b7", burst),
        Ok(())
    );
    hub.flush_anchors(at(10_000));
    assert_eq!(pose(&hub, "rack/b7"), None);
    // Placed again just as it is, it changes nothing.
    state(&ada);
    assert_eq!(
        hub.channel_place_anchor_at(&outlet, rack("rack/0", 0.0), at(12_000)),
        Ok(())
    );
    assert_eq!(state(&ada), None);
    // A sweep with nothing waiting changes nothing.
    hub.flush_anchors(at(13_000));
    assert_eq!(state(&ada), None);
}

#[test]
fn a_side_s_anchors_belong_to_its_binding_and_a_side_stopped_places_none() {
    let (hub, space, _commands, outlet, ada) = yard(0);
    assert_eq!(outlet.place_anchor(rack("rack/a12", 1.0)), Ok(()));
    // Changing environment forgets them, and the side of the binding that ended changes nothing.
    ada.send(ClientMessage::SetEnvironment {
        environment: "sky".to_owned(),
    });
    ada.send(ClientMessage::SetEnvironment {
        environment: "yard".to_owned(),
    });
    assert_eq!(names(&state(&ada).unwrap()), [HOME, "aisle-3"]);
    assert_eq!(outlet.place_anchor(rack("rack/b7", 1.0)), Ok(()));
    assert_eq!(outlet.remove_anchor("rack/b7"), Ok(()));
    assert_eq!(state(&ada), None);
    // A side that has stopped is refused.
    let binding = hub.server_sides.binding(&space).unwrap();
    let current = SpaceOutlet { binding, ..outlet };
    // The new binding's side, removing before it places anything, removes nothing.
    assert_eq!(current.remove_anchor("rack/a12"), Ok(()));
    assert_eq!(state(&ada), None);
    hub.lock().space_mut(&space).unwrap().stopped = Some(binding);
    for error in [
        current.place_anchor(rack("rack/c1", 1.0)),
        current.remove_anchor("rack/c1"),
    ] {
        assert!(error.unwrap_err().message().contains("stopped"));
    }
}

#[test]
fn every_client_is_sent_the_same_seats_and_the_agent_is_told_whom_a_more_sign_stands_for() {
    // More workers than the home cluster's room holds, even compact.
    let cluster = tomori_protocol::cluster::Cluster::new(
        &Anchor::new(
            HOME,
            tomori_protocol::Pose::at(crate::stage::AGENT).facing_towards([0.0, 1.6, 0.0]),
        )
        .with_extent(ROOM.0, ROOM.1, ROOM.2),
    );
    let more = (1..64)
        .find(|span| cluster.arrange(*span).more().is_some())
        .unwrap();
    let count = more + 3;
    let (hub, space, commands, _outlet, ada) = yard(count);
    let bob = Client::greeted(&hub, Access::open(), "bob");
    bob.send(ClientMessage::Join {
        space: space.clone(),
    });
    let seen = state(&ada).unwrap();
    // Every worker seated once, at home, in a slot of its own, and both sent the same.
    assert_eq!(state(&bob).unwrap().clusters, seen.clusters);
    let [home]: [Layout; 1] = seen.clusters.clone().try_into().unwrap();
    assert_eq!(home.anchor, HOME);
    assert_eq!(home.span as usize, count);
    let mut slots: Vec<u32> = home.seats.iter().map(|seat| seat.slot).collect();
    slots.sort_unstable();
    assert_eq!(slots, (0..count as u32).collect::<Vec<_>>());
    // `presence` says which are gathered into the sign.
    let presence = hub.tool_presence("session").unwrap();
    let gathered: Vec<&str> = presence["anchors"][0]["workers"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|worker| worker["gathered"] == true)
        .map(|worker| worker["session"].as_str().unwrap())
        .collect();
    let first = cluster.arrange(count).more().unwrap();
    assert_eq!(gathered.len(), count - first);
    // Pointed at as someone speaks, the sign names the workers it stands for to the agent, each as the space knows it.
    let prompt = said(
        &hub,
        &ada,
        &commands,
        vec![tomori_protocol::Resource::gathered(HOME)],
        None,
    );
    let told = format!(
        "[Pointing at the \"+{} more\" sign at home, which stands for worker \"Look at {}\" (fake session {}, running)",
        count - first,
        gathered[0],
        gathered[0]
    );
    assert!(prompt.text.contains(&told));
    for session in &gathered {
        assert!(prompt.text.contains(session), "{session}");
    }
    // A sign at an anchor not here, or one that gathers nobody, names nothing.
    let state = hub.lock();
    let record = state.space(&space).unwrap();
    for nowhere in ["rack/a12", "aisle-3"] {
        assert_eq!(
            record.gathered(&hub.environments(), nowhere),
            None,
            "{nowhere}"
        );
    }
}

#[test]
fn an_older_client_pointing_at_a_more_sign_as_its_words_end_names_its_workers_too() {
    let (hub, _space, commands, _outlet, ada) = yard(16);
    let prompt = said(
        &hub,
        &ada,
        &commands,
        Vec::new(),
        Some(tomori_protocol::Resource::gathered(HOME)),
    );
    assert!(prompt.text.contains("[Pointing at the \"+"));
}
