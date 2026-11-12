// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Who comes near the space's anchors through the hub: a participant coming within an anchor's radius is told to its
//! agent once, as Tomori's note, between its turns, or with participants' words; visitors and anchors without a radius
//! tell nothing; and `presence` says each radius and who is within it. See `design/environments.md`.
use super::following::{arrived, follows, level, summoned};
use super::timers::{handed, take};
use super::*;
use serde_json::{Value, json};
use tomori_protocol::Pose;
use tomori_protocol::cluster::Anchor;

const NOTE: &str = "ada came within 2 m of station-1 \"Station 1: 10 squats\" (kind station)";

/// A trail whose server side runs, with a station that tells the agent within 2 m, and a bench that does not; a space
/// on it, whose agent's tools are served; Ada, joined; and an outlet for its side.
fn trail() -> (
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
    let station = Anchor::new("station-1", Pose::at([0.0, 0.0, -6.0]))
        .with_kind("station")
        .with_label("Station 1: 10 squats")
        .notify_within(2.0);
    let bench = Anchor::new("bench", Pose::at([6.0, 0.0, -6.0])).with_kind("bench");
    hub.set_environments(Environments::new([crate::environments::Environment::new(
        "trail", "Trail",
    )
    .with_anchor(station)
    .with_anchor(bench)
    .with_server_side(Idle)]));
    hub.serving();
    let mut bound = request("ada");
    bound.environment = "trail".to_owned();
    let (id, commands) = running(
        &hub,
        &bound,
        Snapshot {
            busy: Some(false),
            ..Snapshot::default()
        },
    );
    let outlet = SpaceOutlet {
        hub: Arc::downgrade(&hub),
        space: id.clone(),
        binding: hub.server_sides.binding(&id).unwrap(),
        environment: "trail".to_owned(),
        version: 1,
    };
    let ada = Client::greeted(&hub, Access::open(), "ada");
    ada.send(ClientMessage::Join { space: id.clone() });
    (hub, id, commands, outlet, ada)
}

/// The anchor `name` as `presence` lists it.
fn listed(hub: &Hub, name: &str) -> Value {
    let presence = hub.tool_presence("session").unwrap();
    presence["anchors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|anchor| anchor["name"] == name)
        .cloned()
        .unwrap_or(Value::Null)
}

fn has_events(hub: &Hub, space: &str) -> bool {
    hub.lock().space(space).unwrap().model.has_events()
}

/// The agent's turn ended a quiet moment ago, and the hub looks at its clock.
fn quiet(hub: &Hub, space: &str) {
    hub.lock().space_mut(space).unwrap().model.quieten();
    hub.tick_timers(jiff::Timestamp::now());
}

#[test]
fn coming_near_a_station_is_told_once_between_the_agent_s_turns() {
    let (hub, space, commands, _outlet, ada) = trail();
    // Presence says how near, and nobody is within it yet.
    assert_eq!(
        (
            &listed(&hub, "station-1")["notify_within"],
            &listed(&hub, "station-1")["within"]
        ),
        (&json!(2.0), &json!([]))
    );
    // The bench tells nothing, nor says how near.
    ada.send(standing([6.0, 1.6, -6.0]));
    hub.sweep_proximity(Instant::now());
    assert!(!has_events(&hub, &space));
    assert!(listed(&hub, "bench").get("notify_within").is_none());
    // Ada walks to the station while the agent works: the note waits for its turn to end.
    hub.drive_event(&space, Event::Busy);
    ada.send(standing([0.5, 1.6, -4.5]));
    hub.sweep_proximity(Instant::now());
    assert!(handed(&commands).is_empty());
    assert_eq!(listed(&hub, "station-1")["within"], json!(["ada"]));
    hub.drive_event(&space, Event::Idle);
    hub.tick_timers(jiff::Timestamp::now());
    assert!(handed(&commands).is_empty(), "not straight after it spoke");
    quiet(&hub, &space);
    let prompts = handed(&commands);
    assert_eq!(prompts.len(), 1);
    assert!(crate::events::is_prompt(&prompts[0].key));
    assert_eq!(prompts[0].text, format!("[{NOTE}]"));
    take(&hub, &space, &prompts[0]);
    // Staying there tells nothing more.
    ada.send(standing([0.0, 1.6, -6.0]));
    hub.sweep_proximity(Instant::now());
    assert!(!has_events(&hub, &space));
}

#[test]
fn a_note_rides_on_words_and_visitors_tell_nothing() {
    let (hub, space, commands, _outlet, ada) = trail();
    let vic = Client::greeted(&hub, Access::visitor(), "vic");
    vic.join(&space);
    vic.send(standing([0.0, 1.6, -6.0]));
    ada.send(standing([6.0, 1.6, -6.0]));
    // A visitor reports no presence, so where they stand is not known, and nobody is told of them.
    assert_eq!(hub.lock().space(&space).unwrap().model.visitors.len(), 1);
    assert!(
        hub.lock()
            .participants
            .values()
            .all(|participant| !participant.visitor || participant.seen.is_none())
    );
    hub.sweep_proximity(Instant::now());
    assert!(!has_events(&hub, &space));
    assert_eq!(listed(&hub, "station-1")["within"], json!([]));
    // The agent has just spoken as Ada reaches the station, and her words take the note with them.
    hub.drive_event(&space, Event::Busy);
    hub.drive_event(&space, Event::Idle);
    ada.send(standing([0.0, 1.6, -6.0]));
    hub.sweep_proximity(Instant::now());
    assert!(handed(&commands).is_empty());
    let prompt = said(&hub, &ada, &commands, Vec::new(), None);
    assert_eq!(prompt.text, format!("ada: [{NOTE}] what's this?"));
}

#[test]
fn anchors_placed_and_removed_while_someone_is_within_them() {
    let (hub, space, commands, outlet, ada) = trail();
    hub.drive_event(&space, Event::Busy);
    ada.send(standing([3.0, 1.6, -8.0]));
    hub.sweep_proximity(Instant::now());
    assert!(!has_events(&hub, &space));
    // A hen wanders up to her: she is within its radius as it is placed.
    let now = Instant::now();
    let hen = Anchor::new("hen/marigold", Pose::at([3.5, 0.0, -8.0]))
        .with_kind("hen")
        .with_label("Marigold")
        .notify_within(1.0);
    hub.channel_place_anchor_at(&outlet, hen.clone(), now)
        .unwrap();
    hub.sweep_proximity(now);
    assert_eq!(listed(&hub, "hen/marigold")["within"], json!(["ada"]));
    assert!(has_events(&hub, &space));
    // Removed, nobody is within it.
    hub.channel_remove_anchor_at(
        &outlet,
        "hen/marigold",
        now + tomori_environment::ANCHOR_INTERVAL,
    )
    .unwrap();
    hub.sweep_proximity(now);
    assert_eq!(listed(&hub, "hen/marigold"), Value::Null);
    assert!(
        hub.lock()
            .space(&space)
            .unwrap()
            .proximity
            .within("hen/marigold")
            .is_empty()
    );
    // Placed again, within the cooldown, she is within it, but it is not told again.
    hub.channel_place_anchor_at(&outlet, hen, now + tomori_environment::ANCHOR_INTERVAL * 2)
        .unwrap();
    hub.sweep_proximity(now);
    assert_eq!(listed(&hub, "hen/marigold")["within"], json!(["ada"]));
    hub.drive_event(&space, Event::Idle);
    quiet(&hub, &space);
    let prompts = handed(&commands);
    assert_eq!(
        prompts
            .iter()
            .map(|prompt| prompt.text.as_str())
            .collect::<Vec<_>>(),
        ["[ada came within 1 m of hen/marigold \"Marigold\" (kind hen)]"]
    );
}

#[test]
fn participants_who_share_a_name_are_told_apart_and_nothing_is_told_while_the_agent_is_away() {
    let (hub, space, commands, _outlet, ada) = trail();
    let other = Client::connect(&hub, Access::open());
    other.send(ClientMessage::Hello {
        protocol: PROTOCOL_VERSION,
        participant: "ada-2".to_owned(),
        name: "ada".to_owned(),
        names: false,
        environments: None,
        environment_state: false,
        projection: false,
        pairing_window: false,
        speech_held: false,
        images: false,
        diagrams: false,
        world_edges: false,
        projection_patches: false,
        live_panels: false,
        limits: None,
    });
    other.join(&space);
    // Each arrives away from the station, then walks up to it.
    ada.send(standing([6.0, 1.6, 0.0]));
    other.send(standing([6.0, 1.6, 0.0]));
    hub.sweep_proximity(Instant::now());
    ada.send(standing([0.0, 1.6, -6.0]));
    other.send(standing([1.0, 1.6, -6.0]));
    hub.sweep_proximity(Instant::now());
    let prompts = handed(&commands);
    assert_eq!(prompts.len(), 1);
    let within = listed(&hub, "station-1")["within"].clone();
    let handles: Vec<&str> = within
        .as_array()
        .unwrap()
        .iter()
        .map(|handle| handle.as_str().unwrap())
        .collect();
    assert_eq!(handles.len(), 2);
    let text = &prompts[0].text;
    for handle in &handles {
        assert!(handle.starts_with("ada#"), "{handle}");
        assert!(
            text.contains(&format!("[{handle} came within 2 m of station-1")),
            "{text}"
        );
    }
    take(&hub, &space, &prompts[0]);
    // While the agent cannot be told, as while its backend reconnects, nobody coming near is told, then or later.
    hub.drive_event(
        &space,
        Event::Reconnecting {
            reason: "lost".to_owned(),
        },
    );
    ada.send(standing([9.0, 1.6, -6.0]));
    hub.sweep_proximity(Instant::now() + crate::proximity::COOLDOWN);
    ada.send(standing([0.0, 1.6, -6.0]));
    hub.sweep_proximity(Instant::now() + crate::proximity::COOLDOWN);
    assert!(!has_events(&hub, &space));
}

/// The timer `status` fires every hour, by a clock at 09:17 UTC, with Ada present.
fn hourly(hub: &Hub, ada: &Client) {
    hub.set_zone("UTC");
    hub.set_clock("2026-11-11T09:17:00Z".parse().unwrap());
    ada.send(ClientMessage::Engagement {
        state: tomori_protocol::Engagement::Engaged,
    });
    hub.agent_set_timer(
        "session",
        crate::timers::Request {
            name: Some("status".to_owned()),
            note: "Give a status update".to_owned(),
            every_minutes: Some(60),
            ..crate::timers::Request::default()
        },
    )
    .unwrap();
}

#[test]
fn a_reply_to_a_note_about_someone_comes_to_them_and_keeps_with_them_through_it() {
    use crate::hub::following::{POLL, SETTLE};
    let (hub, space, commands, _outlet, ada) = trail();
    follows(&ada);
    ada.send(standing([6.0, 1.6, -6.0]));
    hub.sweep_proximity(Instant::now());
    // She walks up to the station, 5 m from the agent's home: the note is given it at once, its turn being over.
    let near = [0.5, 1.6, -7.5];
    ada.send(standing(near));
    hub.sweep_proximity(Instant::now());
    let prompts = handed(&commands);
    assert_eq!(prompts.len(), 1);
    hub.drive_event(
        &space,
        Event::Delivered {
            key: prompts[0].key.clone(),
        },
    );
    // Thinking, it has said nothing yet, and moves nothing.
    hub.flush_snapshots(Instant::now() + Duration::from_secs(2));
    assert_eq!(summoned(&hub, &space), None);
    // Its reply's speech queued, it comes straight to her, 2 m from her, as though she had spoken to it, and she is
    // whom it talks with.
    hub.speech_queued(&space, 2);
    let at = summoned(&hub, &space).expect("gone to her");
    assert!(
        (level(at, near) - crate::follow::CONVERSATION).abs() < 1e-3,
        "{at:?}"
    );
    let partner = hub.tool_presence("session").unwrap()["agent"]["partner"].clone();
    assert!(partner.as_str().unwrap().starts_with("ada#"), "{partner}");
    // More of it queued changes nothing.
    hub.speech_queued(&space, 1);
    assert_eq!(summoned(&hub, &space), Some(at));
    // Its turn over, its voice still to come, it follows her on as she walks on.
    hub.drive_event(&space, Event::Idle);
    arrived(&hub, &space);
    let on = [5.0, 1.6, -9.0];
    ada.send(standing(on));
    hub.flush_snapshots(Instant::now() + SETTLE);
    let to_her = summoned(&hub, &space).expect("followed");
    assert!(
        (level(to_her, on) - crate::follow::CONVERSATION).abs() < 1e-3,
        "{to_her:?}"
    );
    // Its voice heard, it keeps with her no more.
    for _ in 0..3 {
        hub.speech_sent(&space);
    }
    arrived(&hub, &space);
    hub.flush_snapshots(Instant::now() + POLL);
    ada.send(standing([-5.0, 1.6, -9.0]));
    hub.flush_snapshots(Instant::now() + POLL + SETTLE);
    assert_eq!(summoned(&hub, &space), Some(to_her));
}

#[test]
fn a_note_s_reply_moves_nothing_without_speech_nor_for_a_timer_and_follows_whoever_came_near_last()
{
    let (hub, space, commands, _outlet, ada) = trail();
    follows(&ada);
    hourly(&hub, &ada);
    let bob = Client::greeted(&hub, Access::open(), "bob");
    bob.join(&space);
    ada.send(standing([6.0, 1.6, -6.0]));
    bob.send(standing([-6.0, 1.6, -6.0]));
    hub.sweep_proximity(Instant::now());
    // Ada reaches the station, and its reply says nothing: it moves nothing.
    ada.send(standing([0.5, 1.6, -7.5]));
    hub.sweep_proximity(Instant::now());
    let prompts = handed(&commands);
    assert_eq!(prompts.len(), 1);
    take(&hub, &space, &prompts[0]);
    hub.flush_snapshots(Instant::now() + Duration::from_secs(2));
    assert_eq!(summoned(&hub, &space), None);
    // The timer fires: its reply to that is nobody's exchange, and it talks with nobody yet, so its speech moves
    // nothing.
    hub.lock().space_mut(&space).unwrap().model.lapse();
    hub.tick_timers("2026-11-11T10:00:00Z".parse().unwrap());
    let prompts = handed(&commands);
    let text = &prompts[0].text;
    assert!(text.starts_with("[Timer \"status\""), "{text}");
    hub.drive_event(
        &space,
        Event::Delivered {
            key: prompts[0].key.clone(),
        },
    );
    hub.speech_queued(&space, 1);
    assert_eq!(summoned(&hub, &space), None);
    hub.speech_sent(&space);
    hub.drive_event(&space, Event::Idle);
    // While the agent works, Bob reaches the station, then Ada walks off and back to it, past its cooldown, and the
    // timer fires again: told together, its reply follows whoever came near last, Ada, the timer naming nobody.
    hub.drive_event(&space, Event::Busy);
    let bob_at = [-1.0, 1.6, -5.0];
    bob.send(standing(bob_at));
    hub.sweep_proximity(Instant::now());
    let later = Instant::now() + crate::proximity::COOLDOWN;
    ada.send(standing([6.0, 1.6, -6.0]));
    hub.sweep_proximity(later);
    let ada_at = [0.5, 1.6, -7.5];
    ada.send(standing(ada_at));
    hub.sweep_proximity(later);
    hub.drive_event(&space, Event::Idle);
    hub.lock().space_mut(&space).unwrap().model.lapse();
    hub.tick_timers("2026-11-11T11:00:00Z".parse().unwrap());
    let prompts = handed(&commands);
    assert_eq!(prompts.len(), 1);
    let text = &prompts[0].text;
    assert!(
        text.starts_with("[bob came within") && text.ends_with("note: \"Give a status update\"]"),
        "{text}"
    );
    assert!(
        text.rfind("[ada came").unwrap() > text.rfind("[bob came").unwrap(),
        "{text}"
    );
    hub.drive_event(
        &space,
        Event::Delivered {
            key: prompts[0].key.clone(),
        },
    );
    hub.speech_queued(&space, 1);
    let at = summoned(&hub, &space).expect("gone to Ada");
    assert!(
        (level(at, ada_at) - crate::follow::CONVERSATION).abs() < 1e-3,
        "{at:?}"
    );
}

#[test]
fn a_note_s_reply_follows_nobody_who_has_left_nor_where_it_does_not_follow() {
    let (hub, space, commands, _outlet, ada) = trail();
    let bob = Client::greeted(&hub, Access::open(), "bob");
    bob.join(&space);
    bob.send(standing([6.0, 1.6, -6.0]));
    hub.sweep_proximity(Instant::now());
    // Moving only when asked, a reply to a note about Bob moves nothing.
    bob.send(standing([0.5, 1.6, -7.5]));
    hub.sweep_proximity(Instant::now());
    let prompts = handed(&commands);
    hub.drive_event(
        &space,
        Event::Delivered {
            key: prompts[0].key.clone(),
        },
    );
    hub.speech_queued(&space, 1);
    assert_eq!(summoned(&hub, &space), None);
    // Following, but Bob gone by the time it speaks, nor does it.
    follows(&ada);
    bob.send(ClientMessage::Leave);
    hub.speech_queued(&space, 1);
    assert_eq!(summoned(&hub, &space), None);
    // Nor pinned.
    ada.send(ClientMessage::PinAgent { pinned: true });
    hub.speech_queued(&space, 1);
    assert_eq!(summoned(&hub, &space), None);
}

#[test]
fn a_spoken_reply_to_a_timer_goes_to_whom_it_talks_with() {
    use crate::hub::following::POLL;
    let (hub, space, commands, _outlet, ada) = trail();
    follows(&ada);
    hourly(&hub, &ada);
    ada.send(standing([6.0, 1.6, -6.0]));
    hub.sweep_proximity(Instant::now());
    // Its reply to a note about her coming near the station goes to her, and is over.
    let near = [0.5, 1.6, -7.5];
    ada.send(standing(near));
    hub.sweep_proximity(Instant::now());
    let prompts = handed(&commands);
    hub.drive_event(
        &space,
        Event::Delivered {
            key: prompts[0].key.clone(),
        },
    );
    hub.speech_queued(&space, 1);
    let at = summoned(&hub, &space).expect("gone to her");
    hub.speech_sent(&space);
    hub.drive_event(&space, Event::Idle);
    arrived(&hub, &space);
    hub.flush_snapshots(Instant::now() + POLL);
    // She walks off: it stays.
    let off = [8.0, 1.6, 2.0];
    ada.send(standing(off));
    hub.flush_snapshots(Instant::now() + POLL + crate::hub::following::SETTLE);
    assert_eq!(summoned(&hub, &space), Some(at));
    // The timer fires, naming nobody: its spoken reply goes to her, whom it talks with.
    hub.lock().space_mut(&space).unwrap().model.lapse();
    hub.tick_timers("2026-11-11T10:00:00Z".parse().unwrap());
    let prompts = handed(&commands);
    let text = &prompts[0].text;
    assert!(text.starts_with("[Timer \"status\""), "{text}");
    hub.drive_event(
        &space,
        Event::Delivered {
            key: prompts[0].key.clone(),
        },
    );
    hub.speech_queued(&space, 1);
    let to_her = summoned(&hub, &space).expect("gone to her");
    assert!(
        (level(to_her, off) - crate::follow::CONVERSATION).abs() < 1e-3,
        "{to_her:?}"
    );
}
