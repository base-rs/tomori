// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Patches of what a space projects, as the hub sends them: to clients that apply them, from the revision each holds,
//! asked for whole again after a gap, and whole to every older client as before. See `design/projection.md`.
use tomori_protocol::{Limits, Projection, ProjectionPatch};

use super::*;

impl Client {
    /// Connected and greeted as `participant`, by a client that applies patches to what is projected and states
    /// `limits`, with what that sent read.
    fn patching_within(
        hub: &Arc<Hub>,
        access: Access,
        participant: &str,
        limits: Option<Limits>,
    ) -> Self {
        let client = Self::connect(hub, access);
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
            projection_patches: true,
            limits,
        });
        client.read();
        client
    }

    /// Connected and greeted as `participant`, by a client that applies patches, with what that sent read.
    fn patching(hub: &Arc<Hub>, access: Access, participant: &str) -> Self {
        Self::patching_within(hub, access, participant, None)
    }
}

/// What a client holds of what is projected, kept as a client keeps it: replaced by each projection sent whole, and
/// patched by each patch, asking for it whole again where a patch does not apply.
#[derive(Default)]
struct Held {
    projection: Option<Projection>,
    wholes: usize,
    patches: Vec<ProjectionPatch>,
    resyncs: usize,
    /// Whether to drop the next patch unapplied, as a client that missed it.
    missing: bool,
}

impl Held {
    /// Read what `client` was sent since, asking again for the projection whole for as long as a patch does not apply.
    fn read(&mut self, client: &Client) {
        loop {
            let mut resync = None;
            for message in client.read() {
                match message {
                    ServerMessage::Projection { projection } => {
                        self.projection = Some(*projection);
                        self.wholes += 1;
                    }
                    ServerMessage::ProjectionPatch { patch } => {
                        self.patches.push((*patch).clone());
                        if std::mem::take(&mut self.missing) {
                            continue;
                        }
                        let applied = self
                            .projection
                            .as_mut()
                            .is_some_and(|projection| patch.apply(projection).is_ok());
                        if !applied {
                            resync = Some(
                                self.projection
                                    .as_ref()
                                    .map_or(0, |projection| projection.revision),
                            );
                        }
                    }
                    _ => {}
                }
            }
            let Some(revision) = resync else {
                return;
            };
            self.resyncs += 1;
            client.send(ClientMessage::ResyncProjection { revision });
        }
    }

    /// What it holds, every panel's age the same, as a client counts each on from when it was sent.
    fn shown(&self) -> Projection {
        without_ages(self.projection.clone().expect("a projection is held"))
    }
}

fn without_ages(mut projection: Projection) -> Projection {
    for panel in &mut projection.panels {
        panel.age = 0;
    }
    projection
}

/// The latest projection `client`, which applies no patches, was sent since, sent whole, as before patches.
fn latest_whole(client: &Client) -> Projection {
    let messages = client.read();
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, ServerMessage::ProjectionPatch { .. })),
        "an older client is sent no patch"
    );
    let Some(ServerMessage::Projection { projection }) = messages.into_iter().last() else {
        unreachable!("the projection is sent whole, last");
    };
    without_ages(*projection)
}

/// A change a client asks for, given what it holds, or `None` for the agent's reply.
type Change = Box<dyn Fn(&Held) -> Option<ClientMessage>>;

fn walls(held: &Held) -> Vec<String> {
    held.shown().walls.into_iter().map(|wall| wall.id).collect()
}

#[test]
fn a_client_that_applies_patches_holds_what_an_older_client_is_sent_whole() {
    let (hub, _jobs) = hub_waiting();
    let (space, _commands) = idle(&hub);
    let ada = Client::patching(&hub, Access::open(), "ada");
    let old = Client::projecting(&hub, Access::open(), "old");
    let mut held = Held::default();
    ada.send(ClientMessage::Join {
        space: space.clone(),
    });
    held.read(&ada);
    old.join(&space);
    // Joining, it is sent the projection whole, at the space's revision.
    assert_eq!((held.wholes, held.patches.len()), (1, 0));
    assert!(held.shown().revision > 0);
    let mut changes: Vec<(&str, Change)> = Vec::new();
    changes.push((
        "a wall added",
        Box::new(|_| {
            Some(ClientMessage::AddWall {
                pose: upright([0.0, 1.5, -3.0]),
            })
        }),
    ));
    changes.push(("the agent's panel", Box::new(|_| None)));
    changes.push((
        "the wall's grid",
        Box::new(|held| {
            Some(ClientMessage::SetWall {
                wall: walls(held)[1].clone(),
                grid: tomori_protocol::Grid::new(4, 1),
                scale: None,
                aspect: None,
                shape: None,
                pose: None,
                tilt: Some(12.0),
            })
        }),
    ));
    changes.push((
        "a panel left floating",
        Box::new(|held| {
            Some(ClientMessage::PlacePanel {
                panel: held.shown().panels[0].id.clone(),
                wall: None,
                slot: None,
                pose: Some(upright([1.0, 1.5, -1.0])),
            })
        }),
    ));
    changes.push((
        "a panel put on the wall",
        Box::new(|held| {
            Some(ClientMessage::PlacePanel {
                panel: held.shown().panels[0].id.clone(),
                wall: Some(walls(held)[1].clone()),
                slot: Some(2),
                pose: None,
            })
        }),
    ));
    changes.push((
        "an item ticked",
        Box::new(|held| {
            let panel = &held.shown().panels[0];
            Some(ClientMessage::CheckPanelItem {
                panel: panel.id.clone(),
                line: 0,
                checked: true,
                revision: Some(panel.revision),
            })
        }),
    ));
    changes.push((
        "the wall locked",
        Box::new(|held| {
            Some(ClientMessage::LockWall {
                wall: walls(held)[1].clone(),
                locked: true,
                reset: false,
            })
        }),
    ));
    changes.push((
        "the wall unlocked",
        Box::new(|held| {
            Some(ClientMessage::LockWall {
                wall: walls(held)[1].clone(),
                locked: false,
                reset: false,
            })
        }),
    ));
    changes.push((
        "the wall removed",
        Box::new(|held| {
            Some(ClientMessage::RemoveWall {
                wall: walls(held)[1].clone(),
            })
        }),
    ));
    changes.push((
        "a panel dismissed",
        Box::new(|held| {
            Some(ClientMessage::DismissPanel {
                panel: held.shown().panels[0].id.clone(),
            })
        }),
    ));
    for (what, change) in changes {
        let before = held.shown();
        match change(&held) {
            Some(message) => ada.send(message),
            None => reply(
                &hub,
                &space,
                "m1",
                "```panel\nid: agenda\ntitle: Agenda\n---\n- [ ] One\n- [ ] Two\n```\n",
            ),
        }
        held.read(&ada);
        let whole = latest_whole(&old);
        // Each change is a patch, from the revision held to a later one, which makes what the older client is sent.
        assert_eq!(held.wholes, 1, "{what}");
        let patch = held.patches.last().unwrap();
        assert_eq!(patch.from, before.revision, "{what}");
        assert!(patch.to > patch.from, "{what}");
        assert!(!patch.ops.is_empty(), "{what}");
        assert_eq!(held.shown(), whole, "{what}");
    }
    assert_eq!(held.resyncs, 0);
}

#[test]
fn a_burst_of_changes_before_a_client_reads_is_one_patch() {
    let (hub, _jobs) = hub_waiting();
    let (space, _commands) = idle(&hub);
    let ada = Client::patching(&hub, Access::open(), "ada");
    let mut held = Held::default();
    ada.send(ClientMessage::Join {
        space: space.clone(),
    });
    for index in 0..4 {
        reply(&hub, &space, &format!("m{index}"), PANEL_REPLY);
    }
    held.read(&ada);
    let patches = held.patches.len();
    for x in [-2.0, 2.0] {
        ada.send(ClientMessage::AddWall {
            pose: upright([x, 1.5, -3.0]),
        });
    }
    reply(&hub, &space, "m5", PANEL_REPLY);
    held.read(&ada);
    assert_eq!(held.patches.len(), patches + 1);
    let patch = held.patches.last().unwrap();
    assert_eq!(patch.ops.len(), 3, "{:?}", patch.ops);
    assert_eq!(held.shown().walls.len(), 3);
    assert_eq!(held.shown().panels.len(), 5);
    // A burst that changes most of what is shown is sent whole instead, as no smaller.
    let wholes = held.wholes;
    for index in 6..12 {
        reply(&hub, &space, &format!("m{index}"), PANEL_REPLY);
    }
    held.read(&ada);
    assert_eq!(held.wholes, wholes + 1);
}

#[test]
fn a_client_that_misses_a_patch_asks_and_is_sent_the_projection_whole() {
    let (hub, _jobs) = hub_waiting();
    let (space, _commands) = idle(&hub);
    let ada = Client::patching(&hub, Access::open(), "ada");
    let vic = Client::patching(&hub, Access::visitor(), "vic");
    let old = Client::projecting(&hub, Access::open(), "old");
    let mut held = Held::default();
    let mut watched = Held::default();
    ada.send(ClientMessage::Join {
        space: space.clone(),
    });
    held.read(&ada);
    vic.send(ClientMessage::Join {
        space: space.clone(),
    });
    watched.read(&vic);
    old.join(&space);
    // A patch missed leaves a gap, which the next patch's revision shows.
    held.missing = true;
    reply(&hub, &space, "m1", PANEL_REPLY);
    held.read(&ada);
    watched.read(&vic);
    assert_eq!((held.wholes, held.resyncs), (1, 0));
    ada.send(ClientMessage::AddWall {
        pose: upright([0.0, 1.5, -3.0]),
    });
    held.read(&ada);
    watched.read(&vic);
    // Asked for, it is sent whole, at the space's revision, and patched from there again.
    assert_eq!((held.wholes, held.resyncs), (2, 1));
    assert_eq!(held.shown(), latest_whole(&old));
    assert_eq!(held.shown(), watched.shown());
    let revision = held.shown().revision;
    reply(&hub, &space, "m2", PANEL_REPLY);
    held.read(&ada);
    watched.read(&vic);
    assert_eq!(held.wholes, 2);
    assert_eq!(held.patches.last().unwrap().from, revision);
    assert_eq!(held.shown(), latest_whole(&old));
    // A visitor, who sees what is projected, may ask too.
    vic.send(ClientMessage::ResyncProjection { revision: 0 });
    watched.read(&vic);
    assert_eq!(watched.wholes, 2);
    assert!(vic.errors().is_empty());
    assert_eq!(watched.shown(), held.shown());
    // Outside a space, or from a client that reads no projection, asking sends nothing.
    let outside = Client::patching(&hub, Access::open(), "out");
    outside.send(ClientMessage::ResyncProjection { revision: 3 });
    assert!(outside.read().is_empty());
    let older = Client::greeted(&hub, Access::open(), "older");
    older.join(&space);
    older.send(ClientMessage::ResyncProjection { revision: 3 });
    assert!(older.read().is_empty());
    // Asking again before it is sent is sent it once, and a client that applies no patches, sent it whole at every
    // change, is sent nothing for asking.
    for _ in 0..3 {
        vic.send(ClientMessage::ResyncProjection { revision: 0 });
    }
    let wholes = vic
        .read()
        .into_iter()
        .filter(|message| matches!(message, ServerMessage::Projection { .. }))
        .count();
    assert_eq!(wholes, 1);
    old.read();
    old.send(ClientMessage::ResyncProjection { revision: 0 });
    assert!(old.read().is_empty());
}

#[test]
fn a_client_joining_again_or_reconnecting_is_sent_the_projection_whole() {
    let (hub, _jobs) = hub_waiting();
    let (space, _commands) = idle(&hub);
    let ada = Client::patching(&hub, Access::open(), "ada");
    let mut held = Held::default();
    ada.send(ClientMessage::Join {
        space: space.clone(),
    });
    held.read(&ada);
    reply(&hub, &space, "m1", PANEL_REPLY);
    held.read(&ada);
    assert_eq!((held.wholes, held.patches.len()), (1, 1));
    // Leaving and joining again, it is sent whole, though nothing changed.
    let revision = held.shown().revision;
    ada.send(ClientMessage::Leave);
    ada.send(ClientMessage::Join {
        space: space.clone(),
    });
    held.read(&ada);
    assert_eq!(held.wholes, 2);
    assert_eq!(held.shown().revision, revision, "nothing changed meanwhile");
    // Connecting again, as after a lost connection, it rejoins its space and is sent the projection whole.
    hub.disconnect(ada.connection);
    let again = Client::connect(&hub, Access::open());
    let mut rejoined = Held::default();
    again.send(ClientMessage::Hello {
        protocol: PROTOCOL_VERSION,
        participant: "ada".to_owned(),
        name: "ada".to_owned(),
        names: false,
        environments: None,
        environment_state: false,
        projection: true,
        pairing_window: false,
        speech_held: false,
        images: false,
        projection_patches: true,
        limits: None,
    });
    rejoined.read(&again);
    assert_eq!((rejoined.wholes, rejoined.patches.len()), (1, 0));
    assert_eq!(rejoined.shown().panels, held.shown().panels);
}

#[test]
fn a_tick_refused_is_a_patch_that_changes_nothing_so_that_the_client_undoes_it() {
    let (hub, _jobs) = hub_waiting();
    let (space, _commands) = idle(&hub);
    let ada = Client::patching(&hub, Access::open(), "ada");
    let mut held = Held::default();
    ada.send(ClientMessage::Join {
        space: space.clone(),
    });
    reply(
        &hub,
        &space,
        "m1",
        "```panel\nid: agenda\ntitle: Agenda\n---\n- [ ] One\n```\n",
    );
    held.read(&ada);
    let panel = held.shown().panels[0].clone();
    ada.send(ClientMessage::CheckPanelItem {
        panel: panel.id.clone(),
        line: 0,
        checked: true,
        revision: Some(panel.revision + 5),
    });
    let messages = ada.read();
    assert!(
        messages
            .iter()
            .any(|message| matches!(message, ServerMessage::Error { panel: Some(_), .. }))
    );
    let patch = messages
        .into_iter()
        .find_map(|message| match message {
            ServerMessage::ProjectionPatch { patch } => Some(*patch),
            _ => None,
        })
        .expect("sent again, as a patch");
    assert!(patch.ops.is_empty());
    assert_eq!(patch.from, held.shown().revision);
    assert!(patch.to > patch.from);
}

#[test]
fn hello_limits_are_negotiated_within_the_most_and_welcome_states_the_most() {
    let (hub, _jobs) = hub_waiting();
    let welcome = |client: &Client| {
        client.send(ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            participant: "p".to_owned(),
            name: "p".to_owned(),
            names: false,
            environments: None,
            environment_state: false,
            projection: true,
            pairing_window: false,
            speech_held: false,
            images: false,
            projection_patches: false,
            limits: None,
        });
        let Some(ServerMessage::Welcome { limits, .. }) = client.read().into_iter().next() else {
            unreachable!("the welcome comes first");
        };
        limits
    };
    let client = Client::connect(&hub, Access::open());
    assert_eq!(welcome(&client), Some(Limits::MOST));
    let kept = |client: &Client| {
        let state = hub.lock();
        let record = &state.connections[&client.connection];
        assert_eq!(
            record.liveness.output.frame_limit(),
            record.limits.frame_bytes as usize
        );
        (record.limits, record.patches)
    };
    // A client that states none, as an older one, is kept within today's, frames of 1 MiB included.
    assert_eq!(kept(&client), (Limits::TODAY, false));
    // One that states more is sent within what it states, up to the most.
    let more = Limits::TODAY
        .with_frame_bytes(8 * 1024 * 1024)
        .with_projection(40, 500, 60, 560);
    let larger = Client::patching_within(&hub, Access::open(), "larger", Some(more));
    assert_eq!(kept(&larger), (more, true));
    let headset = Limits::TODAY.with_frame_bytes(tomori_protocol::MOST_FRAME_BYTES as u32);
    let quest = Client::patching_within(&hub, Access::open(), "quest", Some(headset));
    assert_eq!(kept(&quest), (headset, true));
    // A hostile one is held to the most.
    let hostile = Limits::TODAY.with_frame_bytes(u32::MAX).with_projection(
        u32::MAX,
        u32::MAX,
        u32::MAX,
        u32::MAX,
    );
    let greedy = Client::patching_within(&hub, Access::open(), "greedy", Some(hostile));
    assert_eq!(kept(&greedy), (Limits::MOST, true));
    // One that states less is sent within that, but frames no shorter than every peer takes.
    let less = Limits::TODAY
        .with_frame_bytes(256 * 1024)
        .with_projection(4, 24, 4, 28);
    let smaller = Client::patching_within(&hub, Access::open(), "smaller", Some(less));
    assert_eq!(
        kept(&smaller),
        (
            less.with_frame_bytes(tomori_protocol::MAX_FRAME_BYTES as u32),
            true
        )
    );
    // Patches are only for a client that reads what is projected.
    let blind = Client::connect(&hub, Access::open());
    blind.send(ClientMessage::Hello {
        protocol: PROTOCOL_VERSION,
        participant: "blind".to_owned(),
        name: "blind".to_owned(),
        names: false,
        environments: None,
        environment_state: false,
        projection: false,
        pairing_window: false,
        speech_held: false,
        images: false,
        projection_patches: true,
        limits: None,
    });
    assert_eq!(kept(&blind), (Limits::TODAY, false));
}

#[test]
fn speech_queued_goes_before_the_projection_and_a_patch_is_far_smaller_than_it_whole() {
    let (hub, _jobs) = hub_waiting();
    let (space, _commands) = idle(&hub);
    let ada = Client::patching(&hub, Access::open(), "ada");
    let old = Client::projecting(&hub, Access::open(), "old");
    let mut held = Held::default();
    ada.send(ClientMessage::Join {
        space: space.clone(),
    });
    held.read(&ada);
    old.join(&space);
    for index in 0..4 {
        reply(&hub, &space, &format!("m{index}"), PANEL_REPLY);
    }
    held.read(&ada);
    old.read();
    // A change, with agent speech queued after it: the speech goes first, since the projection may be sent whole in
    // one long frame, and nothing is ahead of it; then the projection, which for a client that takes patches is the
    // patch, far smaller than the projection whole an older client is sent.
    let ahead = |client: &Client| {
        client.outbox.agent_audio(vec![0; 64]);
        let Popped::Audio(_) = client.outbox.pop() else {
            unreachable!("the speech first");
        };
        let Popped::Control(item) = client.outbox.pop() else {
            unreachable!("the projection is sent next");
        };
        let message = hub
            .render(client.connection, item)
            .expect("the projection is sent after the speech");
        Frame::server(&message).encode().len()
    };
    ada.send(ClientMessage::AddWall {
        pose: upright([0.0, 1.5, -3.0]),
    });
    let (patched, whole) = (ahead(&ada), ahead(&old));
    assert!(patched > 0);
    assert!(patched * 4 < whole, "{patched} bytes against {whole}");
}

#[test]
fn in_a_space_of_more_walls_than_today_s_each_client_is_sent_what_it_takes() {
    let (hub, _jobs) = hub_waiting();
    let (space, _commands) = idle(&hub);
    // The desktop window states the most, and an older client nothing.
    let desktop = Client::patching_within(&hub, Access::open(), "desktop", Some(Limits::MOST));
    let old = Client::projecting(&hub, Access::open(), "old");
    let mut held = Held::default();
    desktop.send(ClientMessage::Join {
        space: space.clone(),
    });
    held.read(&desktop);
    old.join(&space);
    old.read();
    // Twenty walls added, more than the six of before.
    for index in 0..20 {
        let x = (index % 10) as f32 * 3.0 - 13.5;
        let z = 4.0 + (index / 10) as f32 * 3.0;
        desktop.send(ClientMessage::AddWall {
            pose: upright([x, 1.5, z]),
        });
    }
    held.read(&desktop);
    let all = held.shown();
    let older = without_ages(old.projections().pop().expect("sent the projection"));
    let walls = all.walls.len();
    assert!(walls >= 21, "{walls}");
    // The older client is sent today's walls, the first of them, as before; the desktop window every one.
    assert_eq!(older.walls.len(), Limits::TODAY.walls as usize);
    assert_eq!(older.walls[..], all.walls[..older.walls.len()]);
    // Each connection's frames are within what it negotiated.
    let limit = |client: &Client| {
        hub.lock().connections[&client.connection]
            .liveness
            .output
            .frame_limit()
    };
    assert_eq!(limit(&desktop), tomori_protocol::MOST_FRAME_BYTES);
    assert_eq!(limit(&old), tomori_protocol::MAX_FRAME_BYTES);
}

#[test]
fn a_projection_sent_again_with_nothing_touched_is_a_later_revision() {
    let (hub, _jobs) = hub_waiting();
    let (space, _commands) = idle(&hub);
    let ada = Client::patching(&hub, Access::open(), "ada");
    let mut held = Held::default();
    ada.send(ClientMessage::Join { space });
    held.read(&ada);
    let revision = held.shown().revision;
    // Sent again, as a patch, so that it never goes from one revision to the same one.
    let Some(ServerMessage::ProjectionPatch { patch }) =
        hub.render(ada.connection, Outgoing::Projection)
    else {
        unreachable!("sent again as a patch");
    };
    assert_eq!((patch.from, patch.to), (revision, revision + 1));
    assert!(patch.ops.is_empty());
}

/// A small, fast generator of numbers, seeded, so that each run tries the same sequences.
struct Random(u64);

impl Random {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound.max(1) as u64) as usize
    }
}

/// Every way the hub changes what a space projects, at random, by participants, the agent's replies and its tools, the
/// agent summoned and sent home, and the space changing environment: after each burst, what a client holds from the
/// patches it was sent is what a client joining now would be sent whole, built afresh rather than sent because the
/// projection was touched, so that a change that forgets to send it shows here, and no patch is ever refused.
#[test]
fn every_change_the_hub_makes_patched_in_turn_is_the_projection_built_afresh() {
    const KINDS: usize = 18;
    let mut done = [0usize; KINDS];
    for seed in 1..=16u64 {
        let mut random = Random(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let (hub, _jobs) = hub_waiting();
        hub.set_environments(Environments::new([
            crate::Environment::new("sky", "Sky"),
            crate::Environment::new("farmyard", "Farmyard").with_stage(upright([0.0, 1.5, -4.0])),
        ]));
        hub.serve_tools(tomori_backend::ToolServer::new(
            "tomori",
            "http://127.0.0.1:1/mcp",
        ));
        let (space, _commands) = idle(&hub);
        let ada = Client::patching(&hub, Access::open(), "ada");
        let fresh = Client::projecting(&hub, Access::open(), "fresh");
        let mut held = Held::default();
        ada.send(ClientMessage::Join {
            space: space.clone(),
        });
        held.read(&ada);
        ada.send(standing([0.0, 1.6, 0.0]));
        fresh.join(&space);
        let mut serial = 0;
        for step in 0..60 {
            for _ in 0..1 + random.below(3) {
                serial += 1;
                let shown = held.shown();
                let panel = (!shown.panels.is_empty())
                    .then(|| shown.panels[random.below(shown.panels.len())].clone());
                let added: Vec<String> = shown.walls[1..]
                    .iter()
                    .map(|wall| wall.id.clone())
                    .collect();
                let wall = shown.walls[random.below(shown.walls.len())].id.clone();
                let other = (!added.is_empty()).then(|| added[random.below(added.len())].clone());
                let kind = random.below(KINDS);
                let ok = match kind {
                    0 => {
                        reply(
                            &hub,
                            &space,
                            &format!("m{serial}"),
                            &format!(
                                "```panel\nid: list-{serial}\ntitle: List {serial}\n---\n- [ ] One\n- [ ] Two\n```\n"
                            ),
                        );
                        true
                    }
                    1 => {
                        reply(&hub, &space, &format!("m{serial}"), PANEL_REPLY);
                        true
                    }
                    2 => {
                        ada.send(ClientMessage::AddWall {
                            pose: upright([
                                random.below(6) as f32 - 3.0,
                                1.5,
                                -3.0 - random.below(3) as f32,
                            ]),
                        });
                        true
                    }
                    3 => other.is_some_and(|wall| {
                        ada.send(ClientMessage::RemoveWall { wall });
                        true
                    }),
                    4 => {
                        ada.send(ClientMessage::SetWall {
                            wall: wall.clone(),
                            grid: tomori_protocol::Grid::new(
                                1 + random.below(4) as u32,
                                1 + random.below(2) as u32,
                            ),
                            scale: Some([1.0, 1.5, 2.0][random.below(3)]),
                            aspect: Some([0.0, 0.5, 2.0][random.below(3)]),
                            shape: None,
                            pose: None,
                            tilt: Some(random.below(30) as f32),
                        });
                        true
                    }
                    5 => other.is_some_and(|wall| {
                        ada.send(ClientMessage::SetWall {
                            wall,
                            grid: None,
                            scale: None,
                            aspect: None,
                            shape: None,
                            pose: Some(upright([random.below(6) as f32 - 3.0, 1.5, 3.0])),
                            tilt: None,
                        });
                        true
                    }),
                    6 => {
                        ada.send(ClientMessage::LockWall {
                            wall: wall.clone(),
                            locked: random.below(2) == 0,
                            reset: random.below(4) == 0,
                        });
                        true
                    }
                    7 => match (other, added.len() > 1) {
                        (Some(moved), true) => {
                            let to = added.iter().find(|id| **id != moved).unwrap().clone();
                            ada.send(ClientMessage::AttachWall {
                                wall: moved,
                                attach: (random.below(3) != 0).then(|| {
                                    tomori_protocol::Attach::new(
                                        to,
                                        tomori_protocol::Edge::ALL[random.below(4)],
                                        tomori_protocol::Align::ALL[random.below(3)],
                                    )
                                }),
                                alone: random.below(2) == 0,
                                pose: Some(upright([0.0, 1.5, -5.0])),
                            });
                            true
                        }
                        _ => false,
                    },
                    8 => panel.is_some_and(|panel| {
                        ada.send(ClientMessage::PlacePanel {
                            panel: panel.id,
                            wall: None,
                            slot: None,
                            pose: Some(upright([random.below(4) as f32 - 2.0, 1.5, -1.0])),
                        });
                        true
                    }),
                    9 => panel.is_some_and(|panel| {
                        ada.send(ClientMessage::PlacePanel {
                            panel: panel.id,
                            wall: Some(wall.clone()),
                            slot: Some(random.below(6) as u32),
                            pose: None,
                        });
                        true
                    }),
                    10 => panel.is_some_and(|panel| {
                        ada.send(ClientMessage::DismissPanel { panel: panel.id });
                        true
                    }),
                    11 => panel.is_some_and(|panel| {
                        // Sometimes from a revision the panel no longer has, which is refused and undone.
                        ada.send(ClientMessage::CheckPanelItem {
                            panel: panel.id,
                            line: random.below(2) as u32,
                            checked: random.below(2) == 0,
                            revision: Some(panel.revision + (random.below(4) == 0) as u64),
                        });
                        true
                    }),
                    12 => panel.is_some_and(|panel| {
                        let place =
                            ["wall tomori", "front ada", "beside ada", "near ada"][random.below(4)];
                        hub.agent_move_panel("session", &panel.id, place).is_ok()
                    }),
                    13 => panel.is_some_and(|panel| {
                        hub.agent_update_panel(
                            "session",
                            crate::hub::agent_panels::Update {
                                panel: panel.id,
                                title: Some(format!("Updated {serial}")),
                                body: (random.below(2) == 0)
                                    .then(|| format!("- [ ] Item {serial}\n- [x] Done")),
                                check: Vec::new(),
                                uncheck: Vec::new(),
                                place: (random.below(3) == 0).then(|| "beside ada".to_owned()),
                                revision: None,
                            },
                        )
                        .is_ok()
                    }),
                    14 => hub
                        .agent_create_layout(
                            "session",
                            Some("front ada"),
                            vec![crate::stage::Planned {
                                id: format!("board-{serial}"),
                                layout: crate::stage::Layout::FLAT,
                                width: None,
                                sized: false,
                                attach: None,
                            }],
                        )
                        .is_ok(),
                    15 => {
                        other.is_some_and(|wall| hub.agent_remove_layout("session", &wall).is_ok())
                    }
                    16 => {
                        // The agent summoned to someone, or sent home, its Tomori wall travelling with it.
                        let at = (random.below(2) == 0).then(|| upright([1.0, 0.0, 1.0]));
                        ada.send(ClientMessage::PlaceAgent { at });
                        true
                    }
                    _ => {
                        let environment = ["sky", "farmyard"][random.below(2)];
                        hub.agent_set_environment("session", environment).is_ok()
                    }
                };
                if ok {
                    done[kind] += 1;
                }
            }
            held.read(&ada);
            fresh.read();
            // Built afresh, as a client joining now is sent it, not because anything was touched.
            let mut afresh = without_ages(fresh.projection());
            let shown = held.shown();
            afresh.revision = shown.revision;
            assert_eq!(shown, afresh, "seed {seed}, step {step}");
        }
        assert_eq!(held.resyncs, 0, "seed {seed}: every patch applied");
        assert!(held.patches.len() > 30, "seed {seed}: sent as patches");
    }
    assert!(
        done.iter().all(|done| *done > 0),
        "every kind of change was made: {done:?}"
    );
}
