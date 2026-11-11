// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Whom the agent's panels go beside, through the hub: while its turn answers someone's words, where they were as
//! those words began, unless they have since walked far or the words began long ago, when it is where they are now; and
//! while it answers nobody's, as after a sub-agent's report, where the participant it talks with, or else whoever spoke
//! last, is now. A name always means where they are now. See `design/projection.md`.
use super::*;

/// Where the panel titled `title` floats, as `client` is sent it.
fn floating(client: &Client, title: &str) -> tomori_protocol::Pose {
    let projection = client.projection();
    let place = projection
        .panels
        .iter()
        .find(|panel| panel.title == title)
        .map(|panel| panel.place.clone());
    let Some(tomori_protocol::Place::Free { pose }) = place else {
        unreachable!("{title} floats: {place:?}");
    };
    pose
}

/// Whether `pose` floats beside someone whose eyes are at `eyes`, looking towards `ahead`, at their reading distance,
/// `bearing` degrees to their right.
fn beside(eyes: [f32; 3], ahead: [f32; 3], pose: &tomori_protocol::Pose, bearing: f32) -> bool {
    let across = (pose.position[0] - eyes[0]).hypot(pose.position[2] - eyes[2]);
    aside(eyes, ahead, pose) == bearing && (across - 1.2).abs() < 1e-3
}

/// The key of the participant called `name`.
fn key_of(hub: &Hub, name: &str) -> String {
    hub.lock()
        .participants
        .iter()
        .find(|(_, record)| record.name == name)
        .map(|(key, _)| key.clone())
        .expect("a participant of that name")
}

/// A sub-agent's report reaches the agent of `space`, not through Tomori, after its turn ended.
fn reported(hub: &Hub, space: &str) {
    hub.drive_event(space, Event::Idle);
    hub.drive_event(space, Event::Busy);
    hub.drive_event(
        space,
        Event::Delivered {
            key: "report".to_owned(),
        },
    );
}

const FINDINGS: &str = "It's done.\n\n```panel\ntitle: Findings\nplace: beside\n---\nAll green.\n```\n\n```panel\ntitle: Details\n---\nNone failed.\n```";

#[test]
fn a_reply_to_a_sub_agent_s_report_puts_its_panels_beside_whoever_spoke_last_where_they_stand_now()
{
    let (hub, _jobs) = hub_waiting();
    let (space, commands) = idle(&hub);
    let ada = Client::projecting(&hub, Access::open(), "ada");
    ada.join(&space);
    let bob = Client::projecting(&hub, Access::open(), "bob");
    bob.join(&space);
    // Ada asks across the space, looking away from every wall, and the agent starts a sub-agent.
    let (asked, away) = ([6.0, 1.6, 6.0], [6.0, 1.6, 0.0]);
    ada.send(looking(asked, away));
    let prompt = said(&hub, &ada, &commands, Vec::new(), None);
    hub.drive_event(&space, Event::Delivered { key: prompt.key });
    reply(&hub, &space, "m1", "I've started a sub-agent on it.");
    // Later she stands a few steps away, turned round, as its report arrives: what the agent projects for it goes
    // beside her there, whether its block says beside or nothing, not where she stood as she asked.
    let (eyes, ahead) = ([4.0, 1.6, 4.5], [4.0, 1.6, 10.5]);
    ada.send(looking(eyes, ahead));
    reported(&hub, &space);
    reply(&hub, &space, "m2", FINDINGS);
    assert!(beside(eyes, ahead, &floating(&ada, "Findings"), 30.0));
    assert!(beside(eyes, ahead, &floating(&ada, "Details"), -30.0));
    // Talking with Bob now, the agent's next report's panels go beside him, where he stands.
    let (his, ahead_of_him) = ([4.0, 1.6, -6.0], [10.0, 1.6, -6.0]);
    bob.send(looking(his, ahead_of_him));
    hub.lock().space_mut(&space).unwrap().following.partner = Some(key_of(&hub, "bob"));
    reported(&hub, &space);
    reply(
        &hub,
        &space,
        "m3",
        "```panel\ntitle: For Bob\nplace: beside\n---\nHis.\n```",
    );
    assert!(beside(his, ahead_of_him, &floating(&bob, "For Bob"), 30.0));
    // Once he has left, beside whoever spoke last, Ada, again.
    bob.send(ClientMessage::Leave);
    reported(&hub, &space);
    reply(
        &hub,
        &space,
        "m4",
        "```panel\ntitle: Last\nplace: beside\n---\nHers.\n```",
    );
    assert!(beside(eyes, ahead, &floating(&ada, "Last"), 70.0));
}

#[test]
fn a_reply_to_someone_s_words_goes_where_they_began_unless_they_walked_far_or_began_long_ago() {
    let (hub, _jobs) = hub_waiting();
    let (space, commands) = idle(&hub);
    let ada = Client::projecting(&hub, Access::open(), "ada");
    ada.join(&space);
    let (asked, away) = ([6.0, 1.6, 6.0], [6.0, 1.6, 0.0]);
    ada.send(looking(asked, away));
    let prompt = said(&hub, &ada, &commands, Vec::new(), None);
    hub.drive_event(&space, Event::Delivered { key: prompt.key });
    // She steps 2 m aside and turns round as she listens: the reply's panel goes beside where she began, but one
    // naming her beside where she is.
    let (stepped, turned) = ([4.0, 1.6, 6.0], [4.0, 1.6, 12.0]);
    ada.send(looking(stepped, turned));
    let text = [
        "Here.",
        "```panel\ntitle: Answer\nplace: beside\n---\nAs she asked.\n```",
        "```panel\ntitle: Named\nplace: beside ada\n---\nWhere she is.\n```",
    ]
    .join("\n\n");
    reply(&hub, &space, "m1", &text);
    assert!(beside(asked, away, &floating(&ada, "Answer"), 30.0));
    assert!(beside(stepped, turned, &floating(&ada, "Named"), 30.0));
    // Walked more than 3 m from there, beside where she is, in the same turn.
    let (walked, ahead) = ([-4.0, 1.6, 4.0], [-4.0, 1.6, 10.0]);
    ada.send(looking(walked, ahead));
    reply(
        &hub,
        &space,
        "m2",
        "```panel\ntitle: Walked\n---\nWhere she went.\n```",
    );
    assert!(beside(walked, ahead, &floating(&ada, "Walked"), 30.0));
    // Words that began a minute ago, though she has stepped only a metre: beside where she is.
    hub.drive_event(&space, Event::Idle);
    let (again, there) = ([0.0, 1.6, 8.0], [0.0, 1.6, 14.0]);
    ada.send(looking(again, there));
    let prompt = said(&hub, &ada, &commands, Vec::new(), None);
    hub.drive_event(&space, Event::Delivered { key: prompt.key });
    let (step, still) = ([1.0, 1.6, 8.0], [1.0, 1.6, 14.0]);
    ada.send(looking(step, still));
    hub.lock()
        .space_mut(&space)
        .unwrap()
        .model
        .age_origin(crate::presence::ORIGIN_FRESH);
    reply(
        &hub,
        &space,
        "m3",
        "```panel\ntitle: Stale\n---\nWhere she is now.\n```",
    );
    assert!(beside(step, still, &floating(&ada, "Stale"), 30.0));
}

#[test]
fn show_image_and_move_panel_go_beside_whoever_the_agent_acts_for_as_its_panels_do() {
    let folder = pictures("asking");
    let (hub, _jobs) = hub_waiting();
    hub.serve_tools(tomori_backend::ToolServer::new(
        "tomori",
        "http://127.0.0.1:1/mcp",
    ));
    let (space, commands) = idle(&hub);
    let ada = Client::projecting(&hub, Access::open(), "ada");
    ada.join(&space);
    let bob = Client::projecting(&hub, Access::open(), "bob");
    bob.join(&space);
    let (asked, away) = ([6.0, 1.6, 6.0], [6.0, 1.6, 0.0]);
    ada.send(looking(asked, away));
    let (his, ahead_of_him) = ([4.0, 1.6, -6.0], [10.0, 1.6, -6.0]);
    bob.send(looking(his, ahead_of_him));
    let prompt = said(&hub, &ada, &commands, Vec::new(), None);
    hub.drive_event(&space, Event::Delivered { key: prompt.key });
    reply(&hub, &space, "m1", PANEL_REPLY);
    let panel = ada.projection().panels[0].id.clone();
    let prepared = crate::images::prepare(&folder, "small.png").unwrap();
    hub.images.put(&prepared).unwrap();
    let beside_whoever = crate::stage::Placement::parse("beside").unwrap();
    // In reply to her words, a step away from where she began, beside where she began.
    let (stepped, turned) = ([5.0, 1.6, 6.0], [5.0, 1.6, 12.0]);
    ada.send(looking(stepped, turned));
    hub.agent_move_panel("session", &panel, "beside").unwrap();
    assert!(beside(asked, away, &floating(&ada, "Build status"), 30.0));
    // After a sub-agent's report, a few steps from where she began, beside where she stands now, as by name.
    let (eyes, ahead) = ([4.0, 1.6, 4.5], [4.0, 1.6, 10.5]);
    ada.send(looking(eyes, ahead));
    reported(&hub, &space);
    hub.agent_move_panel("session", &panel, "beside").unwrap();
    assert!(beside(eyes, ahead, &floating(&ada, "Build status"), 30.0));
    hub.agent_show_image(
        "session",
        "Picture",
        "",
        Some(&beside_whoever),
        prepared.image.clone(),
    )
    .unwrap();
    assert!(beside(eyes, ahead, &floating(&ada, "Picture"), -30.0));
    // Named, beside him where he stands.
    hub.agent_move_panel("session", &panel, "beside bob")
        .unwrap();
    assert!(beside(
        his,
        ahead_of_him,
        &floating(&ada, "Build status"),
        30.0
    ));
    let _ = std::fs::remove_dir_all(folder);
}
