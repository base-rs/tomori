// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent walking with someone, through the hub with the times its timer would wake at: started by `move_agent
//! with`, gated by `requested_by`, or by the hand menu's **Tomori: walk with me**, it keeps with them across many moves
//! with no exchange, its return home put off; it ends as they ask it to stay, follow or go home, stop from the hand
//! menu, leave, or after half an hour with nothing between them, its return home counting again from then; never for a
//! visitor, nor where it stays home; and `presence` says whom it walks with. See `design/environments.md`.
use super::anchors::yard;
use super::following::{ADA, arrived, level};
use super::*;
use crate::follow::CONVERSATION;
use crate::hub::following::{POLL, SETTLE, WALK_IDLE};
use serde_json::json;
use tomori_protocol::{AgentMovement, Movement};

/// Where the agent of `space` is summoned to.
fn summoned(hub: &Hub, space: &str) -> Option<tomori_protocol::Pose> {
    hub.lock().space(space).unwrap().summoned
}

/// What `presence` says of the agent at `now`.
fn agent(hub: &Hub, space: &str, now: Instant) -> serde_json::Value {
    let state = hub.lock();
    Hub::agent_report(&state, &hub.environments(), space, now)
}

/// Whom the agent of `space` walks with, by their key.
fn walker(hub: &Hub, space: &str) -> Option<String> {
    hub.lock()
        .space(space)
        .unwrap()
        .following
        .walks_with()
        .map(str::to_owned)
}

/// The yard, its agent following the speaker and going home after 90 s quiet, Ada standing 4.3 m from its home, her
/// words given it and its turn over.
fn walk() -> (
    Arc<Hub>,
    String,
    async_channel::Receiver<SpaceCommand>,
    SpaceOutlet,
    Client,
) {
    let (hub, space, commands, outlet, ada) = yard(0);
    ada.send(ClientMessage::SetAgentMovement {
        movement: AgentMovement::new(Movement::Follow, Some(90)),
    });
    ada.send(looking(ADA, [0.0, 1.6, -3.0]));
    // Her words, a tap, move nothing.
    asked(&hub, &ada, &commands, &space);
    assert_eq!(summoned(&hub, &space), None);
    (hub, space, commands, outlet, ada)
}

/// `client`'s words are given to the agent of `space`, which takes them and ends its turn, saying nothing.
fn asked(
    hub: &Hub,
    client: &Client,
    commands: &async_channel::Receiver<SpaceCommand>,
    space: &str,
) {
    let prompt = said(hub, client, commands, Vec::new(), None);
    super::timers::take(hub, space, &prompt);
}

#[test]
fn walking_with_someone_it_keeps_with_them_across_many_moves_with_no_exchange_and_never_goes_home()
{
    let (hub, space, commands, _outlet, ada) = walk();
    // Asked by her words, it walks with her: straight to her, 2 m from her, facing her.
    let walked = hub
        .agent_move_agent("session", "walk with me", Some("ada"))
        .unwrap();
    assert_eq!(walked["moved"], true);
    let mode = walked["agent"]["mode"].as_str().unwrap().to_owned();
    assert!(mode.starts_with("walking with ada#"), "{mode}");
    assert!(walked["agent"].get("returns_home_in_s").is_none());
    let at = summoned(&hub, &space).expect("gone to her");
    assert!(
        (level(at.position, ADA) - CONVERSATION).abs() < 1e-3,
        "{at:?}"
    );
    // Clients are told whom it walks with, by her number.
    let seen = super::anchors::state(&ada).unwrap();
    assert!(seen.walks);
    assert_eq!(seen.walking, Some(hub.lock().participants["/ada"].source));
    // Nothing more is said, and she walks on, step after step, settling more than 3 m from it each time: it comes to
    // her each time, 2 m from her, facing her.
    let start = Instant::now();
    let mut now = start;
    for (step, at) in [
        [4.0, 1.6, 4.0],
        [8.0, 1.6, 2.0],
        [8.0, 1.6, -3.0],
        [3.0, 1.6, -6.0],
    ]
    .into_iter()
    .enumerate()
    {
        now += Duration::from_secs(120);
        ada.send(looking(at, [at[0], 1.6, at[2] - 5.0]));
        hub.flush_snapshots(Instant::now() + SETTLE);
        let to_her = summoned(&hub, &space).expect("followed");
        assert!(
            (level(to_her.position, at) - CONVERSATION).abs() < 1e-3,
            "step {step}: {to_her:?}"
        );
        let facing = crate::follow::apart(
            crate::follow::faces(&to_her),
            crate::follow::towards(to_her.position, at),
        );
        assert!(facing < 1e-3, "step {step}: {facing}");
        // Long past its 90 s return, it is still with her.
        hub.flush_snapshots(now);
        assert_eq!(summoned(&hub, &space), Some(to_her));
    }
    // Arrived, it turns to her as she steps aside within 3 m.
    let aside = [3.8, 1.6, -6.0];
    ada.send(looking(aside, [3.8, 1.6, -11.0]));
    hub.flush_snapshots(Instant::now() + SETTLE);
    let state = hub.lock();
    let record = state.space(&space).unwrap();
    assert_eq!(record.following.faces(), Some(aside));
    drop(state);
    // Its return home is put off throughout.
    assert!(agent(&hub, &space, now).get("returns_home_in_s").is_none());
    // Asked to follow again, by her words, it walks with her no more, and its return counts from then.
    asked(&hub, &ada, &commands, &space);
    let followed = hub
        .agent_move_agent("session", "follow", Some("ada"))
        .unwrap();
    assert_eq!(followed["agent"]["mode"], "following");
    assert_eq!(walker(&hub, &space), None);
    let ended = Instant::now();
    hub.flush_snapshots(ended + POLL);
    assert_eq!(
        agent(&hub, &space, ended + POLL + Duration::from_secs(30))["returns_home_in_s"],
        json!(60)
    );
    hub.flush_snapshots(ended + POLL + Duration::from_secs(91));
    assert_eq!(summoned(&hub, &space), None);
}

#[test]
fn a_walk_ends_as_they_ask_it_to_stay_or_go_home_or_stop_from_the_hand_menu() {
    let (hub, space, commands, _outlet, ada) = walk();
    // Asked to stay, it is pinned where it stands, walking no more.
    ada.send(ClientMessage::WalkAgent { walking: true });
    assert!(walker(&hub, &space).is_some());
    asked(&hub, &ada, &commands, &space);
    hub.agent_move_agent("session", "stay", Some("ada"))
        .unwrap();
    assert_eq!(walker(&hub, &space), None);
    assert_eq!(agent(&hub, &space, Instant::now())["mode"], "pinned");
    // Walking with her again from the hand menu, which ends its pin, then sent home, it walks with nobody.
    ada.send(ClientMessage::WalkAgent { walking: true });
    assert!(!hub.lock().space(&space).unwrap().following.pinned);
    hub.agent_move_agent("session", "home", None).unwrap();
    assert_eq!(walker(&hub, &space), None);
    // From the hand menu, she stops it, and it stays where it is, following again; asking again is refused.
    ada.send(ClientMessage::WalkAgent { walking: true });
    ada.read();
    let at = summoned(&hub, &space);
    ada.send(ClientMessage::WalkAgent { walking: false });
    assert_eq!(walker(&hub, &space), None);
    assert_eq!(summoned(&hub, &space), at);
    assert!(super::anchors::state(&ada).unwrap().walking.is_none());
    ada.send(ClientMessage::WalkAgent { walking: false });
    assert_eq!(ada.errors(), ["Tomori is not walking with you."]);
    // Sent anywhere else, as near someone, it walks with nobody too.
    ada.send(ClientMessage::WalkAgent { walking: true });
    asked(&hub, &ada, &commands, &space);
    hub.agent_move_agent("session", "front ada", Some("ada"))
        .unwrap();
    assert_eq!(walker(&hub, &space), None);
    // Where it moves only when asked, following again ends a walk, leaving it where it stands.
    ada.send(ClientMessage::SetAgentMovement {
        movement: AgentMovement::default(),
    });
    ada.send(ClientMessage::WalkAgent { walking: true });
    let at = summoned(&hub, &space);
    assert!(walker(&hub, &space).is_some());
    let followed = hub.agent_move_agent("session", "follow", None).unwrap();
    assert_eq!(
        (&followed["moved"], &followed["agent"]["mode"]),
        (&json!(false), &json!("away"))
    );
    assert_eq!(summoned(&hub, &space), at);
    // Kept home, it goes home, walking with nobody.
    ada.send(ClientMessage::WalkAgent { walking: true });
    ada.send(ClientMessage::SetAgentMovement {
        movement: AgentMovement::new(Movement::Home, None),
    });
    assert_eq!(walker(&hub, &space), None);
}

#[test]
fn a_walk_ends_as_they_leave_or_after_half_an_hour_with_nothing_between_them() {
    let (hub, space, _commands, _outlet, ada) = walk();
    let bob = Client::greeted(&hub, Access::open(), "bob");
    bob.join(&space);
    bob.send(looking([-4.0, 1.6, 3.0], [-4.0, 1.6, -3.0]));
    // It walks with Bob, who leaves: it stays where it is, its return home counting from then.
    bob.send(ClientMessage::WalkAgent { walking: true });
    assert_eq!(walker(&hub, &space).as_deref(), Some("/bob"));
    let at = summoned(&hub, &space);
    bob.send(ClientMessage::Leave);
    assert_eq!(walker(&hub, &space), None);
    assert_eq!(summoned(&hub, &space), at);
    let left = Instant::now();
    hub.flush_snapshots(left + POLL);
    assert_eq!(
        agent(&hub, &space, left + POLL)["returns_home_in_s"],
        json!(15),
        "its partner gone, sooner"
    );
    // It walks with Ada, who stands still, says nothing, and is told nothing, for half an hour: it walks with her
    // no more, though it stays.
    ada.send(ClientMessage::WalkAgent { walking: true });
    let start = Instant::now();
    hub.flush_snapshots(start + WALK_IDLE - Duration::from_secs(1));
    assert_eq!(walker(&hub, &space).as_deref(), Some("/ada"));
    // Its own speech, or her moving, puts it off.
    hub.lock().space_mut(&space).unwrap().following.spoke(
        start + WALK_IDLE - Duration::from_secs(1),
        Duration::from_secs(1),
    );
    hub.flush_snapshots(start + WALK_IDLE + Duration::from_secs(1));
    assert_eq!(walker(&hub, &space).as_deref(), Some("/ada"));
    hub.flush_snapshots(start + WALK_IDLE * 2);
    assert_eq!(walker(&hub, &space), None);
    assert!(summoned(&hub, &space).is_some());
}

#[test]
fn only_a_participant_s_words_or_menu_start_a_walk_and_never_where_it_stays_home() {
    let (hub, space, commands, _outlet, ada) = walk();
    // Unasked, or for someone who is not here, the agent cannot start one.
    assert!(
        hub.agent_move_agent("session", "with", None)
            .unwrap_err()
            .contains("requested_by")
    );
    asked(&hub, &ada, &commands, &space);
    assert!(
        hub.agent_move_agent("session", "escort vic", Some("ada"))
            .unwrap_err()
            .contains("vic is not a participant here")
    );
    // A visitor asks from the hand menu, and is refused.
    let vic = Client::greeted(&hub, Access::visitor(), "vic");
    vic.join(&space);
    vic.read();
    vic.send(ClientMessage::WalkAgent { walking: true });
    assert_eq!(vic.errors().len(), 1);
    assert_eq!(walker(&hub, &space), None);
    // Nor for a visitor, by name.
    assert!(
        hub.agent_move_agent("session", "with vic", Some("ada"))
            .is_err()
    );
    // A collaborator walks with it themselves, but only an owner asks it to walk with someone else.
    let cy = super::asking(
        &hub,
        &space,
        "Cy",
        Grant::Collaborator {
            space: space.clone(),
        },
        Duration::ZERO,
    );
    cy.send(looking([-3.0, 1.6, 4.0], [-3.0, 1.6, -1.0]));
    assert!(
        hub.agent_move_agent("session", "with ada", Some("Cy"))
            .unwrap_err()
            .starts_with("Only an owner may ask you to walk with someone else")
    );
    assert_eq!(walker(&hub, &space), None);
    hub.agent_move_agent("session", "with me", Some("Cy"))
        .unwrap();
    let theirs = hub
        .lock()
        .participants
        .iter()
        .find(|(_, participant)| participant.name == "Cy")
        .map(|(key, _)| key.clone());
    assert_eq!(walker(&hub, &space), theirs);
    cy.send(ClientMessage::WalkAgent { walking: false });
    assert_eq!(walker(&hub, &space), None);
    // Someone who has not said where they are is refused too, from the hand menu or by their words.
    let bob = Client::greeted(&hub, Access::open(), "bob");
    bob.join(&space);
    bob.read();
    bob.send(ClientMessage::WalkAgent { walking: true });
    assert_eq!(bob.errors(), ["Tomori does not know where you are yet."]);
    asked(&hub, &bob, &commands, &space);
    assert!(
        hub.agent_move_agent("session", "walk with me", Some("bob"))
            .unwrap_err()
            .contains("Whoever asked is not a participant here")
    );
    asked(&hub, &ada, &commands, &space);
    // Asked by Ada, it walks with Bob, once he has: his words, and Ada's, then only turn it while it walks with him.
    bob.send(looking([-4.0, 1.6, 3.0], [-4.0, 1.6, -3.0]));
    hub.agent_move_agent("session", "with: bob", Some("ada"))
        .unwrap();
    assert_eq!(walker(&hub, &space).as_deref(), Some("/bob"));
    arrived(&hub, &space);
    let for_bob = summoned(&hub, &space);
    ada.send(looking([6.0, 1.6, 6.0], [6.0, 1.6, 0.0]));
    let prompt = said(&hub, &ada, &commands, Vec::new(), None);
    hub.flush_snapshots(Instant::now() + Duration::from_secs(1));
    assert_eq!(summoned(&hub, &space), for_bob);
    super::timers::take(&hub, &space, &prompt);
    // Kept home, nobody starts one.
    ada.send(ClientMessage::SetAgentMovement {
        movement: AgentMovement::new(Movement::Home, None),
    });
    ada.read();
    ada.send(ClientMessage::WalkAgent { walking: true });
    assert_eq!(
        ada.errors(),
        ["Tomori stays home in this space: an owner can let it move in the Space section."]
    );
    asked(&hub, &ada, &commands, &space);
    assert!(
        hub.agent_move_agent("session", "with me", Some("ada"))
            .unwrap_err()
            .contains("keeps you at home")
    );
    // Outside a space, nothing, and it says so.
    let lobby = Client::greeted(&hub, Access::open(), "lou");
    lobby.send(ClientMessage::WalkAgent { walking: true });
    assert_eq!(lobby.errors().len(), 1);
}
