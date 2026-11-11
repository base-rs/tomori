// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Who comes near the space's anchors through the hub: a participant coming within an anchor's radius is told to its
//! agent once, as Tomori's note, between its turns, or with participants' words; visitors and anchors without a radius
//! tell nothing; and `presence` says each radius and who is within it. See `design/environments.md`.
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
    assert!(hub.lock().space(&space).unwrap().proximity.is_empty());
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
        projection_patches: false,
        live_panels: false,
        limits: None,
    });
    other.join(&space);
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
    for handle in &handles {
        assert!(handle.starts_with("ada#"), "{handle}");
        assert!(
            prompts[0]
                .text
                .contains(&format!("[{handle} came within 2 m of station-1")),
            "{}",
            prompts[0].text
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
