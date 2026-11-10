// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent's own controls of where it stands, through the hub: `move_agent` to someone's conversation spot, in front
//! of them, by an object its environment gives a position, at the anchor by one, `stay` and `follow`, within the limits
//! on its own moves; what someone points at as their focus; its mode, partner and return in `presence`; and the hand
//! menu's **Tomori: stay here** and **Tomori: follow again**. See `design/mcp.md` and `design/environments.md`.
use super::anchors::{rack, yard};
use super::*;
use crate::hub::following::COMMIT;
use serde_json::json;
use tomori_environment::{Object, Outlet};
use tomori_protocol::{AgentMovement, Movement, Resource};

const ADA: [f32; 3] = [0.0, 1.6, 2.0];

/// The horizontal distance between `a` and `b`.
fn level(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).hypot(a[2] - b[2])
}

/// The angle, in degrees, between the level ways from `from` to `a` and to `b`.
fn apart(from: [f32; 3], a: [f32; 3], b: [f32; 3]) -> f32 {
    let (ax, az) = (a[0] - from[0], a[2] - from[2]);
    let (bx, bz) = (b[0] - from[0], b[2] - from[2]);
    ((ax * bx + az * bz) / (ax.hypot(az) * bx.hypot(bz)))
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees()
}

/// Where the agent of `space` is summoned to.
fn summoned(hub: &Hub, space: &str) -> Option<[f32; 3]> {
    hub.lock()
        .space(space)
        .unwrap()
        .summoned
        .map(|pose| pose.position)
}

/// What `presence` says of the agent.
fn agent(hub: &Hub) -> serde_json::Value {
    hub.tool_presence("session").unwrap()["agent"].clone()
}

/// `client` begins words for the agent, as `utterance`, attending to `attention`.
fn press(client: &Client, utterance: &str, attention: Vec<Resource>) {
    client.send(ClientMessage::UtteranceBegin {
        utterance: utterance.to_owned(),
        answer: None,
        progress: false,
        naming: false,
        titling: false,
        attention,
        seen: None,
    });
}

/// Ada, an owner, has the space's agent follow the speaker, going home after 90 s.
fn follows(ada: &Client) {
    ada.send(ClientMessage::SetAgentMovement {
        movement: AgentMovement::new(Movement::Follow, Some(90)),
    });
}

#[test]
fn near_or_beside_someone_it_stands_at_their_conversation_spot_and_in_front_with_front() {
    let (hub, space, commands, _outlet, ada) = yard(0);
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    // Near her, by name: 1.5 m away, to her left of what she looks at, a little above her eyes, facing her.
    let moved = hub.agent_move_agent("session", "near ada", None).unwrap();
    assert_eq!(moved["moved"], true);
    let at = summoned(&hub, &space).unwrap();
    assert!((level(at, ADA) - 1.5).abs() < 1e-3, "{at:?}");
    assert!(at[0] < -1.0 && at[2] < ADA[2]);
    assert!((at[1] - 1.7).abs() < 1e-4);
    // She is whom it talks with now, as presence says; moving only when asked, it is away.
    let report = &moved["agent"];
    assert_eq!(report["mode"], "away");
    // By her id, which tells her from anyone sharing her name.
    assert!(report["partner"].as_str().unwrap().starts_with("ada#"));
    // Of its own accord again within 20 s, refused; asked by whoever's words it answers, it goes.
    assert!(
        hub.agent_move_agent("session", "front ada", None)
            .unwrap_err()
            .contains("20 seconds")
    );
    said(&hub, &ada, &commands, Vec::new(), None);
    let front = hub
        .agent_move_agent("session", "front ada", Some("ada"))
        .unwrap();
    assert_eq!(front["moved"], true);
    // In front of her, as before, 2.3 m along the way she faces.
    let at = summoned(&hub, &space).unwrap();
    assert!((level(at, ADA) - 2.3).abs() < 1e-3, "{at:?}");
    assert!(at[0].abs() < 1e-3);
    // Beside whoever asked, by their words: at her spot again.
    hub.agent_move_agent("session", "beside", Some("ada"))
        .unwrap();
    let at = summoned(&hub, &space).unwrap();
    assert!((level(at, ADA) - 1.5).abs() < 1e-3, "{at:?}");
    // Someone the words were not from, or nobody here, is refused.
    assert!(
        hub.agent_move_agent("session", "near ada", Some("bob"))
            .is_err()
    );
    assert!(
        hub.agent_move_agent("session", "near bob", Some("ada"))
            .unwrap_err()
            .contains("bob")
    );
    // A form it does not know names those it does.
    let unknown = hub
        .agent_move_agent("session", "the moon", None)
        .unwrap_err();
    assert!(
        unknown.contains("follow") && unknown.contains("object ID"),
        "{unknown}"
    );
}

#[test]
fn near_someone_who_has_not_reported_it_stands_beside_where_they_were_as_they_spoke() {
    let (hub, space, commands, _outlet, ada) = yard(0);
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    said(&hub, &ada, &commands, Vec::new(), None);
    // Their presence forgotten, as after their headset stopped reporting: beside where they stood as they spoke,
    // looking the way they looked.
    hub.lock()
        .participants
        .values_mut()
        .for_each(|participant| participant.seen = None);
    hub.agent_move_agent("session", "near", None).unwrap();
    let at = summoned(&hub, &space).unwrap();
    assert!((level(at, ADA) - 1.5).abs() < 1e-3, "{at:?}");
    assert!(at[0] < -1.0);
}

#[test]
fn by_an_object_it_stands_off_the_line_of_sight_to_it_where_it_stands_now_or_at_the_anchor_by_it() {
    let (hub, space, commands, outlet, ada) = yard(0);
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    let rack = [3.0, 1.0, -6.0];
    assert_eq!(
        outlet.describe(vec![
            Object::new("rack-7", "rack 7, its fans loud")
                .with_reference("rack/7")
                .at(rack),
            Object::new("post-1", "a fence post").with_reference("post/1"),
            Object::new("far-1", "beyond the fence").at([0.0, 1.0, -400.0]),
        ]),
        Ok(())
    );
    said(&hub, &ada, &commands, Vec::new(), None);
    // Far from her, it stands beside it as from where she would stand to look at it, 2 m before it: 1.5 m from there,
    // off that line, facing there.
    let moved = hub
        .agent_move_agent("session", "object yard:rack/7", Some("ada"))
        .unwrap();
    assert_eq!(moved["moved"], true);
    let at = summoned(&hub, &space).unwrap();
    let share = 2.0 / level(ADA, rack);
    let viewpoint = [
        rack[0] + (ADA[0] - rack[0]) * share,
        1.6,
        rack[2] + (ADA[2] - rack[2]) * share,
    ];
    assert!((level(at, viewpoint) - 1.5).abs() < 1e-3, "{at:?}");
    assert!((apart(viewpoint, at, rack) - 45.0).abs() < 1.0);
    assert!(level(at, rack) < 2.0);
    // Its side describes it again where it has moved to, and the agent goes to it there; by its reference alone too.
    let moved_to = [-3.0, 1.0, -6.0];
    assert_eq!(
        outlet.describe(vec![
            Object::new("rack-7", "rack 7, wheeled away")
                .with_reference("rack/7")
                .at(moved_to),
        ]),
        Ok(())
    );
    hub.agent_move_agent("session", "object: rack/7", Some("ada"))
        .unwrap();
    assert!(level(summoned(&hub, &space).unwrap(), moved_to) < 2.0);
    // Near her, it stands at her conversation spot for it.
    assert_eq!(
        outlet.describe(vec![
            Object::new("rack-7", "rack 7, beside her")
                .with_reference("rack/7")
                .at([1.0, 1.0, 0.5]),
            Object::new("post-1", "a fence post").with_reference("post/1"),
        ]),
        Ok(())
    );
    hub.agent_move_agent("session", "object rack-7", Some("ada"))
        .unwrap();
    assert!((level(summoned(&hub, &space).unwrap(), ADA) - 1.5).abs() < 1e-3);
    // One without a position, one out of bounds, so left without one, and one no longer described are refused.
    assert!(
        hub.agent_move_agent("session", "object yard:post/1", Some("ada"))
            .unwrap_err()
            .contains("does not say where")
    );
    assert_eq!(
        outlet.describe(vec![Object::new("far-1", "beyond").at([0.0, 1.0, -400.0])]),
        Ok(())
    );
    assert!(
        hub.agent_move_agent("session", "object far-1", Some("ada"))
            .unwrap_err()
            .contains("does not say where")
    );
    assert!(
        hub.agent_move_agent("session", "object yard:rack/7", Some("ada"))
            .unwrap_err()
            .contains("Nothing in this space has the id")
    );
    // An anchor standing by it, though the side gives no position, is where it goes; there already, it stays.
    assert_eq!(
        outlet.place_anchor(rack_anchor("rack/a12", "rack/a12")),
        Ok(())
    );
    let anchored = hub
        .agent_move_agent("session", "object yard:rack/a12", Some("ada"))
        .unwrap();
    assert_eq!(anchored["agent"]["anchor"], "rack/a12");
    let again = hub
        .agent_move_agent("session", "object rack/a12", Some("ada"))
        .unwrap();
    assert_eq!(again["moved"], false);
    // Of its own accord, within 20 s of its last, refused, whether by the object or at the anchor by one; the same
    // again, going nowhere, is no move.
    assert_eq!(
        outlet.describe(vec![
            Object::new("rack-7", "rack 7")
                .with_reference("rack/7")
                .at(rack)
        ]),
        Ok(())
    );
    assert_eq!(
        hub.agent_move_agent("session", "object rack/7", None)
            .unwrap()["moved"],
        true
    );
    assert_eq!(
        hub.agent_move_agent("session", "object rack/7", None)
            .unwrap()["moved"],
        false
    );
    assert!(
        hub.agent_move_agent("session", "object rack/a12", None)
            .unwrap_err()
            .contains("20 seconds")
    );
    hub.lock().space_mut(&space).unwrap().rates = Default::default();
    assert!(
        hub.agent_move_agent("session", "object rack/a12", None)
            .is_ok()
    );
    assert!(
        hub.agent_move_agent("session", "object rack/7", None)
            .unwrap_err()
            .contains("20 seconds")
    );
}

/// The anchor `name` by the object `object`, at a rack 4 m along x.
fn rack_anchor(name: &str, object: &str) -> tomori_protocol::cluster::Anchor {
    rack(name, 4.0).with_object(object)
}

#[test]
fn by_an_object_with_nowhere_clear_beside_it_it_stays_where_it_is() {
    let (hub, space, commands, outlet, ada) = yard(0);
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    // At the corner of what the agent may reach, asked from beyond it, so every spot beside it is out of reach.
    let corner = [19.5, 1.0, 19.5];
    assert_eq!(
        outlet.describe(vec![Object::new("lamp-1", "a lamp").at(corner)]),
        Ok(())
    );
    ada.send(looking([30.0, 1.6, 30.0], corner));
    said(&hub, &ada, &commands, Vec::new(), None);
    let refused = hub
        .agent_move_agent("session", "object lamp-1", Some("ada"))
        .unwrap_err();
    assert!(
        refused.contains("Nowhere beside that is clear"),
        "{refused}"
    );
    assert_eq!(summoned(&hub, &space), None);
}

#[test]
fn without_anyone_s_words_by_an_object_it_stands_as_for_someone_arriving() {
    let (hub, space, _commands, outlet, _ada) = yard(0);
    assert_eq!(
        outlet.describe(vec![Object::new("lamp-1", "a lamp").at([0.0, 1.0, -6.0])]),
        Ok(())
    );
    hub.agent_move_agent("session", "object lamp-1", None)
        .unwrap();
    // From the arrival's eyes, 2 m before the lamp, then 1.5 m off that line.
    let at = summoned(&hub, &space).unwrap();
    assert!((level(at, [0.0, 1.6, -4.0]) - 1.5).abs() < 1e-3, "{at:?}");
}

#[test]
fn stay_pins_it_and_follow_lets_it_follow_again_only_where_it_follows_the_speaker() {
    let (hub, space, commands, _outlet, ada) = yard(0);
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    // Moving only when asked, it cannot follow.
    assert!(
        hub.agent_move_agent("session", "follow", None)
            .unwrap_err()
            .contains("only when someone asks")
    );
    follows(&ada);
    // Pinned by its own stay, it follows again on its own word, going home once quiet.
    let stayed = hub.agent_move_agent("session", "stay here", None).unwrap();
    assert_eq!(stayed["agent"]["mode"], "pinned");
    assert!(stayed["agent"].get("pinned_by").is_none());
    let followed = hub
        .agent_move_agent("session", "follow again", None)
        .unwrap();
    assert_eq!(
        (&followed["moved"], &followed["agent"]["mode"]),
        (&json!(false), &json!("home"))
    );
    assert!(!hub.lock().space(&space).unwrap().following.pinned);
    // Called by Ada, it is hers: its own follow is refused until her words ask.
    ada.send(ClientMessage::CallAgent);
    assert_eq!(agent(&hub)["pinned_by"], "ada");
    let refused = hub.agent_move_agent("session", "follow", None).unwrap_err();
    assert!(refused.contains("ada called you here"), "{refused}");
    // Her words, as she holds X far from it, glance only while it is pinned; asked to follow, it goes to her.
    ada.send(looking([6.0, 1.6, 6.0], [6.0, 1.6, 0.0]));
    press(&ada, "u1", Vec::new());
    hub.flush_snapshots(Instant::now() + COMMIT);
    let pinned_at = summoned(&hub, &space).unwrap();
    ada.send(ClientMessage::UtteranceEnd {
        utterance: "u1".to_owned(),
        pointed: None,
    });
    let utterance = *hub.lock().utterances.keys().next().unwrap();
    hub.transcript_finished(utterance, "follow me again");
    while commands.try_recv().is_ok() {}
    assert_eq!(summoned(&hub, &space), Some(pinned_at));
    let followed = hub
        .agent_move_agent("session", "follow me", Some("ada"))
        .unwrap();
    assert_eq!(followed["moved"], true);
    let at = summoned(&hub, &space).unwrap();
    assert!((level(at, [6.0, 1.6, 6.0]) - 1.5).abs() < 1e-3, "{at:?}");
    assert_eq!(agent(&hub)["mode"], "following");
}

#[test]
fn presence_says_its_mode_whom_it_talks_with_and_when_it_goes_home() {
    let (hub, space, _commands, _outlet, ada) = yard(0);
    follows(&ada);
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    let report = agent(&hub);
    assert_eq!(
        (&report["mode"], &report["at_home"]),
        (&json!("home"), &json!(true))
    );
    assert!(report.get("returns_home_in_s").is_none());
    // Following her, on its way to her, it says so, where it has reached, and that it goes home after 90 s quiet.
    hub.agent_move_agent("session", "near ada", None).unwrap();
    let start = Instant::now();
    let report = {
        let state = hub.lock();
        Hub::agent_report(&state, &hub.environments(), &space, start)
    };
    assert_eq!(
        (&report["mode"], &report["moving"]),
        (&json!("following"), &json!(true))
    );
    assert!(report["partner"].as_str().unwrap().starts_with("ada#"));
    assert!(report["now_at"].is_array());
    assert_eq!(report["returns_home_in_s"], 90);
    // Quiet a while, it says how long is left; arrived, it is no longer moving.
    let quiet = start + Duration::from_secs(20);
    hub.flush_snapshots(quiet);
    let later = quiet + Duration::from_secs(30);
    let report = {
        let state = hub.lock();
        Hub::agent_report(&state, &hub.environments(), &space, later)
    };
    assert_eq!(report["returns_home_in_s"], 60);
    assert!(report.get("moving").is_none() && report.get("now_at").is_none());
    // Sent home, it is going home, and then home.
    let sent = hub.agent_move_agent("session", "home", None).unwrap();
    assert_eq!(
        (&sent["moved"], &sent["agent"]["mode"]),
        (&json!(true), &json!("going home"))
    );
    let home = {
        let state = hub.lock();
        Hub::agent_report(
            &state,
            &hub.environments(),
            &space,
            Instant::now() + Duration::from_secs(60),
        )
    };
    assert_eq!(home["mode"], "home");
    // Called back on its way home, it sets out from where it had reached.
    ada.send(ClientMessage::CallAgent);
    assert_eq!(agent(&hub)["mode"], "pinned");
    // Never going home, it says no time.
    ada.send(ClientMessage::SetAgentMovement {
        movement: AgentMovement::new(Movement::Follow, None),
    });
    ada.send(ClientMessage::PinAgent { pinned: false });
    let report = agent(&hub);
    assert_eq!(report["mode"], "following");
    assert!(report.get("returns_home_in_s").is_none());
}

#[test]
fn its_partner_gone_it_says_it_goes_home_sooner() {
    let (hub, space, _commands, _outlet, ada) = yard(0);
    follows(&ada);
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    hub.agent_move_agent("session", "near ada", None).unwrap();
    ada.send(ClientMessage::Engagement {
        state: tomori_protocol::Engagement::Away,
    });
    let report = {
        let state = hub.lock();
        Hub::agent_report(&state, &hub.environments(), &space, Instant::now())
    };
    assert_eq!(report["returns_home_in_s"], 15);
}

#[test]
fn what_someone_points_at_where_the_server_knows_its_place_is_their_focus() {
    let (hub, space, _commands, outlet, ada) = yard(1);
    follows(&ada);
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    // An object to her right, its side giving where it stands: the agent stands to her left of it, not of her gaze.
    let lamp = [4.0, 1.6, 2.0];
    assert_eq!(
        outlet.describe(vec![Object::new("lamp-1", "a lamp").at(lamp)]),
        Ok(())
    );
    press(
        &ada,
        "u1",
        vec![Resource::card("issue-1"), Resource::object("lamp-1")],
    );
    let start = Instant::now();
    hub.flush_snapshots(start + COMMIT);
    let at = summoned(&hub, &space).unwrap();
    assert!((apart(ADA, at, lamp) - 45.0).abs() < 1.0, "{at:?}");
    assert!(at[2] < ADA[2] - 0.5);
    ada.send(ClientMessage::UtteranceCancel {
        utterance: "u1".to_owned(),
    });
    hub.lock()
        .agent_home(&space, "a test sent it", Instant::now());
    // A worker, where its cluster seats it.
    let worker = {
        let state = hub.lock();
        let record = state.space(&space).unwrap();
        let environments = hub.environments();
        record
            .seated(&environments)
            .position(
                &record.clusters(&environments),
                &crate::seating::Member::Worker("ses_00".to_owned()),
            )
            .unwrap()
            .0
    };
    press(&ada, "u2", vec![Resource::worker("ses_00")]);
    let start = Instant::now();
    hub.flush_snapshots(start + COMMIT);
    let at = summoned(&hub, &space).unwrap();
    assert!((apart(ADA, at, worker) - 45.0).abs() < 1.0, "{at:?}");
    ada.send(ClientMessage::UtteranceCancel {
        utterance: "u2".to_owned(),
    });
    hub.lock()
        .agent_home(&space, "a test sent it", Instant::now());
    // An object without a place, or one it does not know, leaves her gaze her focus.
    assert_eq!(
        outlet.describe(vec![Object::new("post-1", "a post")]),
        Ok(())
    );
    for object in ["post-1", "nothing"] {
        press(&ada, "u3", vec![Resource::object(object)]);
        let start = Instant::now();
        hub.flush_snapshots(start + COMMIT);
        let at = summoned(&hub, &space).unwrap();
        assert!(at[0] < -1.0, "{at:?}");
        ada.send(ClientMessage::UtteranceCancel {
            utterance: "u3".to_owned(),
        });
        hub.lock()
            .agent_home(&space, "a test sent it", Instant::now());
    }
}

#[test]
fn an_object_it_points_at_has_no_place_in_a_space_whose_side_is_another_environment_s() {
    let (hub, space, _commands, outlet, ada) = yard(0);
    follows(&ada);
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    assert_eq!(
        outlet.describe(vec![Object::new("lamp-1", "a lamp").at([4.0, 1.6, 2.0])]),
        Ok(())
    );
    // The channel kept is another environment's, as just after a change of environment.
    hub.lock()
        .space_mut(&space)
        .unwrap()
        .channel
        .as_mut()
        .unwrap()
        .environment = "sky".to_owned();
    press(&ada, "u1", vec![Resource::object("lamp-1")]);
    let start = Instant::now();
    hub.flush_snapshots(start + COMMIT);
    assert!(summoned(&hub, &space).unwrap()[0] < -1.0);
    assert!(
        hub.agent_move_agent("session", "object lamp-1", None)
            .unwrap_err()
            .contains("Nothing in this space")
    );
}

#[test]
fn a_participant_pins_it_and_lets_it_follow_again_from_the_hand_menu_and_always_wins() {
    let (hub, space, _commands, _outlet, ada) = yard(0);
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    // Every space says it takes pins.
    assert!(super::anchors::state(&ada).unwrap().pins);
    // Moving only when asked, she pins it, but cannot have it follow.
    ada.send(ClientMessage::PinAgent { pinned: true });
    assert!(hub.lock().space(&space).unwrap().following.pinned);
    ada.send(ClientMessage::PinAgent { pinned: false });
    assert_eq!(
        ada.errors(),
        [
            "Tomori moves only when asked in this space, so it stays where it goes: an owner can let it follow in the Space section."
        ]
    );
    // Following, the agent pinned itself, and her follow again ends its pin; her stay pins it as hers.
    follows(&ada);
    hub.agent_move_agent("session", "stay", None).unwrap();
    ada.send(ClientMessage::PinAgent { pinned: false });
    assert_eq!(agent(&hub)["mode"], "home");
    ada.send(ClientMessage::PinAgent { pinned: true });
    assert_eq!(agent(&hub)["pinned_by"], "ada");
    // Kept home, neither.
    ada.send(ClientMessage::SetAgentMovement {
        movement: AgentMovement::new(Movement::Home, Some(90)),
    });
    ada.read();
    ada.send(ClientMessage::PinAgent { pinned: true });
    assert_eq!(
        ada.errors(),
        ["Tomori stays home in this space: an owner can let it move in the Space section."]
    );
    // Visitors only watch.
    let vic = Client::greeted(&hub, Access::visitor(), "vic");
    vic.send(ClientMessage::Join {
        space: space.clone(),
    });
    vic.read();
    vic.send(ClientMessage::PinAgent { pinned: true });
    assert_eq!(vic.errors().len(), 1);
    // Outside a space, nothing, and it says so.
    let lobby = Client::greeted(&hub, Access::open(), "lou");
    lobby.send(ClientMessage::PinAgent { pinned: true });
    assert_eq!(lobby.errors().len(), 1);
}

#[test]
fn a_worker_placed_by_an_object_with_no_anchor_stands_before_where_it_stood_and_never_chases_it() {
    let (hub, space, commands, outlet, ada) = yard(2);
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    said(&hub, &ada, &commands, Vec::new(), None);
    let lamp = [0.0, 1.0, -4.0];
    assert_eq!(
        outlet.describe(vec![
            Object::new("lamp-1", "a lamp")
                .with_reference("lamp/1")
                .at(lamp),
            Object::new("long-1", "a lamp of a long reference",)
                .with_reference("x".repeat(tomori_environment::REFERENCE_BYTES))
                .at([4.0, 1.0, -4.0]),
            Object::new("post-1", "a post"),
        ]),
        Ok(())
    );
    let placed = hub
        .agent_place_worker("session", "ses_00", "object yard:lamp/1", None)
        .unwrap();
    assert_eq!(placed["at"], "tomori/object/lamp/1");
    // In a cluster of the server's own, 1 m before the lamp towards Ada, at her eye height, facing her.
    let seen = super::anchors::state(&ada).unwrap();
    let own = seen.anchors.cluster("tomori/object/lamp/1").unwrap();
    assert_eq!(own.kind.as_deref(), Some("object"));
    assert_eq!(own.label.as_deref(), Some("Beside yard:lamp/1"));
    assert!(
        (own.pose.position[2] - (lamp[2] + 1.0)).abs() < 1e-3,
        "{:?}",
        own.pose
    );
    assert!((own.pose.position[1] - 1.6).abs() < 1e-3);
    // The lamp moving, the worker stays where it was placed.
    assert_eq!(
        outlet.describe(vec![
            Object::new("lamp-1", "a lamp")
                .with_reference("lamp/1")
                .at([6.0, 1.0, -4.0])
        ]),
        Ok(())
    );
    let kept = hub.lock().space(&space).unwrap().places["yard"][0].pose;
    assert_eq!(kept, Some(own.pose));
    // A reference too long for a cluster's name is named by its place among those described.
    assert_eq!(
        outlet.describe(vec![
            Object::new("lamp-1", "a lamp").with_reference("lamp/1"),
            Object::new("long-1", "a lamp")
                .with_reference("x".repeat(tomori_environment::REFERENCE_BYTES))
                .at([4.0, 1.0, -4.0]),
            Object::new("post-1", "a post"),
        ]),
        Ok(())
    );
    let long = hub
        .agent_place_worker("session", "ses_01", "object long-1", None)
        .unwrap();
    assert_eq!(long["at"], "tomori/object/1");
    // One without a position, and one not described, have nowhere.
    for object in ["post-1", "lamp-9"] {
        assert!(
            hub.agent_place_worker(
                "session",
                "ses_01",
                &format!("object {object}"),
                Some("ada")
            )
            .unwrap_err()
            .contains("does not say where it stands")
        );
    }
}
