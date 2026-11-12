// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Every object a side describes is reached, however many it describes and in whatever order: pointed at, it is
//! described to the agent, and `inspect` and `act` take its id; while `presence` lists only the few that matter most
//! to whoever spoke, the same throughout a turn. See `design/environments.md`.
use super::actions::{Answering, Yard, act, call, described, refused, said, yard};
use super::*;
use crate::ambient::AMBIENT_LIMIT;

/// The yard with `posts` fence posts described before its hens, each with an id, the first a metre ahead of home and
/// each half a metre beyond the one before.
fn crowded(posts: usize) -> Yard {
    let yard = yard(Answering(Arc::new(Mutex::new(Vec::new()))));
    let mut objects: Vec<tomori_environment::Object> = (0..posts)
        .map(|index| {
            tomori_environment::Object::new(format!("stake-{index}"), format!("fence post {index}"))
                .with_reference(format!("stake/{index}"))
                .at([0.0, 1.6, -1.0 - index as f32 / 2.0])
        })
        .collect();
    objects.extend(described());
    assert_eq!(yard.hub.channel_describe(&yard.outlet, objects), Ok(()));
    yard
}

/// The descriptions `presence` lists of the yard's objects, in order, and how many it describes in all.
fn listed(yard: &Yard) -> (Vec<String>, u64) {
    let presence = yard.hub.tool_presence("session").unwrap();
    let listed = presence["objects"]["listed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|object| object["description"].as_str().unwrap().to_owned())
        .collect();
    (listed, presence["objects"]["total"].as_u64().unwrap())
}

/// Ada says something pointing at `objects`, and the agent takes it and ends its turn.
fn pointing(yard: &Yard, objects: &[&str]) -> String {
    let words = super::said(
        &yard.hub,
        &yard.ada,
        &yard.commands,
        objects
            .iter()
            .map(|object| tomori_protocol::Resource::object(*object))
            .collect(),
        None,
    );
    yard.hub
        .drive_event(&yard.space, Event::Delivered { key: words.key });
    yard.hub.drive_event(&yard.space, Event::Idle);
    words.text
}

#[test]
fn an_object_past_the_ambient_list_is_described_inspected_and_acted_on() {
    let yard = crowded(40);
    let hub = &yard.hub;
    // Listed, the first of the side's are, Pip, described after 40 posts, not.
    let (before, total) = listed(&yard);
    assert_eq!(total, 44);
    assert_eq!(before.len(), AMBIENT_LIMIT);
    assert!(!before.contains(&"Pip, a hen".to_owned()));
    // Pointed at, she is described to the agent all the same, with her id and what she offers.
    assert_eq!(
        pointing(&yard, &["chicken-1"]),
        "ada: [Pointing at Pip, a hen (id yard:hen/pip; offers peck, go, say)] what's this?"
    );
    let pip = said(&call(
        hub,
        "inspect",
        serde_json::json!({ "id": "yard:hen/pip" }),
    ));
    assert_eq!(pip["description"], "Pip, a hen");
    let done = said(&act(
        hub,
        "yard:hen/pip",
        "peck",
        serde_json::json!({ "pecks": 2 }),
    ));
    assert_eq!(done["outcome"], "done");
    // So is the last of all, Slow, pointed at by nobody.
    let slow = said(&call(
        hub,
        "inspect",
        serde_json::json!({ "id": "yard:hen/slow" }),
    ));
    assert_eq!(slow["description"], "Slow, a hen");
    // An id nothing has is still refused, without asking the side.
    assert!(
        refused(&call(
            hub,
            "inspect",
            serde_json::json!({ "id": "yard:hen/nobody" })
        ))
        .starts_with("Nothing in this space has the id yard:hen/nobody now")
    );
    assert!(
        refused(&act(
            hub,
            "yard:stake/40",
            "peck",
            serde_json::json!({ "pecks": 1 })
        ))
        .starts_with("Nothing in this space has the id yard:stake/40 now that takes actions")
    );
    // Now Pip, whom Ada pointed at, comes first, and the list notes that the rest are reached by their ids.
    let presence = hub.tool_presence("session").unwrap();
    assert_eq!(
        presence["objects"]["listed"][0]["description"],
        "Pip, a hen"
    );
    assert_eq!(presence["objects"]["listed"][0]["id"], "yard:hen/pip");
    // Then Slow, inspected since, and the rest in the side's order.
    assert_eq!(
        presence["objects"]["listed"][1]["description"],
        "Slow, a hen"
    );
    assert_eq!(
        presence["objects"]["listed"][2]["description"],
        "fence post 0"
    );
    assert_eq!(
        presence["objects"]["listed"][2]["position"],
        serde_json::json!([0.0, 1.6, -1.0])
    );
    assert!(
        presence["objects"]["note"]
            .as_str()
            .unwrap()
            .starts_with("The 32 of the 44 objects")
    );
}

#[test]
fn the_list_ranks_what_was_pointed_at_and_near_and_stays_through_a_turn() {
    let yard = crowded(40);
    let hub = &yard.hub;
    // Ada stands by the farthest posts, looking along them, and points at Rosie as she speaks.
    yard.ada.send(looking([0.0, 1.6, -19.0], [0.0, 1.6, -25.0]));
    pointing(&yard, &["chicken-2"]);
    let (first, _) = listed(&yard);
    assert_eq!(
        first[..4],
        [
            "Rosie, a hen",
            "fence post 37",
            "fence post 36",
            "fence post 35"
        ]
    );
    // Slow is inspected and Pip acted on.
    // Rosie, then the post she looks at, then those around her, the nearest first.
    said(&call(
        hub,
        "inspect",
        serde_json::json!({ "id": "yard:hen/slow" }),
    ));
    said(&act(
        hub,
        "yard:hen/pip",
        "peck",
        serde_json::json!({ "pecks": 1 }),
    ));
    // In the same turn, she walks back home and the side describes anew: the list is the same objects, by identity,
    // as they are described now, without those gone.
    yard.ada.send(standing([0.0, 1.6, 0.0]));
    let mut again = described();
    again.reverse();
    again.push(tomori_environment::Object::new("stake-1", "a fallen stake"));
    assert_eq!(hub.channel_describe(&yard.outlet, again), Ok(()));
    let (same, total) = listed(&yard);
    assert_eq!(total, 5);
    assert!(first.contains(&"fence post 1".to_owned()));
    assert_eq!(same, ["Rosie, a hen", "a fallen stake"]);
    // The next turn ranks them again, for where she is now: what was pointed at, then acted on and inspected.
    hub.drive_event(&yard.space, Event::Idle);
    let (next, _) = listed(&yard);
    assert_eq!(
        next,
        [
            "Rosie, a hen",
            "Pip, a hen",
            "Slow, a hen",
            "a fence post",
            "a fallen stake"
        ]
    );
}

/// However many objects a side describes, and however long, `presence` lists at most [`AMBIENT_LIMIT`], within 32 KiB
/// as JSON, while every one is kept to be reached.
#[test]
fn the_list_is_bounded_however_much_is_described() {
    use tomori_environment::{DESCRIPTION_LIMIT, IDENTITY_LIMIT, OBJECT_LIMIT, REFERENCE_BYTES};
    let yard = yard(Answering(Arc::new(Mutex::new(Vec::new()))));
    let objects: Vec<tomori_environment::Object> = (0..OBJECT_LIMIT + 1)
        .map(|index| {
            let id = format!("{index:0>width$}", width = IDENTITY_LIMIT);
            let reference = format!("{index:0>width$}", width = REFERENCE_BYTES);
            tomori_environment::Object::new(id, "😀".repeat(DESCRIPTION_LIMIT * 2))
                .with_reference(reference)
                .at([-19.99, 1.6, -19.99])
        })
        .collect();
    assert_eq!(yard.hub.channel_describe(&yard.outlet, objects), Ok(()));
    let presence = yard.hub.tool_presence("session").unwrap();
    assert_eq!(presence["objects"]["total"], OBJECT_LIMIT);
    let listed = presence["objects"]["listed"].as_array().unwrap();
    assert_eq!(listed.len(), AMBIENT_LIMIT);
    assert!(serde_json::to_string(&presence["objects"]).unwrap().len() <= 32 * 1024);
    // The last kept, though listed nowhere, is inspected by its id; the one past the limit is not kept.
    let last = format!(
        "yard:{:0>width$}",
        OBJECT_LIMIT - 1,
        width = REFERENCE_BYTES
    );
    assert!(yard.hub.agent_inspect("session", &last, None).is_ok());
    let past = format!("yard:{OBJECT_LIMIT:0>width$}", width = REFERENCE_BYTES);
    assert!(yard.hub.agent_inspect("session", &past, None).is_err());
}

/// `client` begins words pointing at `objects`, and they are transcribed and queued, without waiting for them to be
/// handed to the agent.
fn speaking(hub: &Hub, client: &Client, objects: &[&str]) {
    let utterance = format!("u{}", hub.lock().next_utterance);
    client.send(ClientMessage::UtteranceBegin {
        utterance: utterance.clone(),
        answer: None,
        progress: false,
        naming: false,
        titling: false,
        attention: objects
            .iter()
            .map(|object| tomori_protocol::Resource::object(*object))
            .collect(),
        seen: None,
    });
    client.send(ClientMessage::UtteranceEnd {
        utterance,
        pointed: None,
    });
    let utterance = *hub.lock().utterances.keys().next().unwrap();
    hub.transcript_finished(utterance, "and this?");
}

/// `client` begins words pointing at `objects`, and cancels them.
fn cancelling(hub: &Hub, client: &Client, objects: &[&str]) {
    let utterance = format!("u{}", hub.lock().next_utterance);
    client.send(ClientMessage::UtteranceBegin {
        utterance: utterance.clone(),
        answer: None,
        progress: false,
        naming: false,
        titling: false,
        attention: objects
            .iter()
            .map(|object| tomori_protocol::Resource::object(*object))
            .collect(),
        seen: None,
    });
    client.send(ClientMessage::UtteranceCancel { utterance });
}

/// A turn's list is ranked by what its own speaker pointed at and where their head is, never by someone who spoke
/// meanwhile, and words begun and cancelled change nothing.
#[test]
fn a_turn_s_list_is_its_own_speaker_s_and_cancelled_words_change_nothing() {
    let yard = crowded(40);
    let hub = &yard.hub;
    let bob = Client::greeted(hub, Access::open(), "bob");
    bob.join(&yard.space);
    yard.ada.send(looking([0.0, 1.6, -19.0], [0.0, 1.6, -25.0]));
    bob.send(looking([0.0, 1.6, 0.0], [0.0, 1.6, -10.0]));
    // Ada's words, pointing at Rosie, are the agent's; it is still answering them as Bob speaks, pointing at Slow,
    // and begins and cancels words pointing at Pip.
    let words = super::said(
        hub,
        &yard.ada,
        &yard.commands,
        vec![tomori_protocol::Resource::object("chicken-2")],
        None,
    );
    hub.drive_event(&yard.space, Event::Delivered { key: words.key });
    speaking(hub, &bob, &["chicken-3"]);
    cancelling(hub, &bob, &["chicken-1"]);
    // Her turn ranks what she pointed at, then what is around her, not him.
    let (hers, _) = listed(&yard);
    assert_eq!(hers[..2], ["Rosie, a hen", "fence post 37"]);
    // His words are the agent's next: his turn ranks what he pointed at, and the post he looks at by home, his
    // cancelled words changing nothing.
    hub.drive_event(&yard.space, Event::Idle);
    let Ok(SpaceCommand::Send(words)) = yard.commands.try_recv() else {
        unreachable!("his words are handed to the agent next");
    };
    assert!(words.text.contains("Slow, a hen"), "{}", words.text);
    hub.drive_event(&yard.space, Event::Delivered { key: words.key });
    let (his, _) = listed(&yard);
    assert_eq!(his[..2], ["Slow, a hen", "fence post 0"]);
    // Begun and cancelled again in his next turn, his pointing still ranks first.
    hub.drive_event(&yard.space, Event::Idle);
    cancelling(hub, &bob, &["chicken-1"]);
    let (again, _) = listed(&yard);
    assert_eq!(again[0], "Slow, a hen");
}
