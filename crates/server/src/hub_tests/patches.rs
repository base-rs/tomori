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
        Self::patching_as(hub, access, participant, limits, false)
    }

    /// Connected and greeted as `participant`, by a client that applies patches and their data ops to what is
    /// projected, with what that sent read.
    fn live(hub: &Arc<Hub>, access: Access, participant: &str) -> Self {
        Self::patching_as(hub, access, participant, None, true)
    }

    fn patching_as(
        hub: &Arc<Hub>,
        access: Access,
        participant: &str,
        limits: Option<Limits>,
        live_panels: bool,
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
            diagrams: false,
            world_edges: false,
            projection_patches: true,
            live_panels,
            spawns: false,
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
    /// The errors it was sent, each with the panel and the wall it names.
    errors: Vec<(String, Option<String>, Option<String>)>,
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
                    ServerMessage::Error {
                        error, panel, wall, ..
                    } => self.errors.push((error, panel, wall)),
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
                content_scale: None,
                revision: None,
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
                size: None,
                revision: None,
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
                size: None,
                revision: None,
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
                revision: Some(panel.version),
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
        diagrams: false,
        world_edges: false,
        projection_patches: true,
        live_panels: false,
        spawns: false,
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
        revision: Some(panel.version + 5),
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
            diagrams: false,
            world_edges: false,
            projection_patches: false,
            live_panels: false,
            spawns: false,
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
    let hostile = Limits::TODAY
        .with_frame_bytes(u32::MAX)
        .with_projection(u32::MAX, u32::MAX, u32::MAX, u32::MAX)
        .with_volumes(u32::MAX, u32::MAX);
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
        diagrams: false,
        world_edges: false,
        projection_patches: true,
        live_panels: false,
        spawns: false,
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
    // A typical space: the Tomori wall and eight short text panels.
    for index in 0..8 {
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
        // And one that applies data ops and keeps the versions place ops carry, watching.
        let lea = Client::live(&hub, Access::visitor(), "lea");
        let mut held = Held::default();
        let mut watched = Held::default();
        ada.send(ClientMessage::Join {
            space: space.clone(),
        });
        held.read(&ada);
        lea.send(ClientMessage::Join {
            space: space.clone(),
        });
        watched.read(&lea);
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
                            content_scale: None,
                            revision: None,
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
                            content_scale: None,
                            revision: None,
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
                            size: None,
                            revision: None,
                        });
                        true
                    }),
                    9 => panel.is_some_and(|panel| {
                        ada.send(ClientMessage::PlacePanel {
                            panel: panel.id,
                            wall: Some(wall.clone()),
                            slot: Some(random.below(6) as u32),
                            pose: None,
                            size: None,
                            revision: None,
                        });
                        true
                    }),
                    10 => panel.is_some_and(|panel| {
                        ada.send(ClientMessage::DismissPanel { panel: panel.id });
                        true
                    }),
                    11 => panel.is_some_and(|panel| {
                        // Sometimes from a version the panel no longer has, which is refused and undone.
                        ada.send(ClientMessage::CheckPanelItem {
                            panel: panel.id,
                            line: random.below(2) as u32,
                            checked: random.below(2) == 0,
                            revision: Some(panel.version + (random.below(4) == 0) as u64),
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
                                version: None,
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
                                context: Vec::new(),
                            }],
                        )
                        .is_ok(),
                    15 => {
                        other.is_some_and(|wall| hub.agent_remove_layout("session", &wall).is_ok())
                    }
                    16 => {
                        // The agent called to someone, or sent home, its status wall staying where it stands.
                        if random.below(2) == 0 {
                            ada.send(super::looking([1.0, 1.6, 1.0], [1.0, 1.6, -2.0]));
                            ada.send(ClientMessage::CallAgent);
                        } else {
                            ada.send(ClientMessage::PlaceAgent { at: None });
                        }
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
            watched.read(&lea);
            fresh.read();
            // Built afresh, as a client joining now is sent it, not because anything was touched.
            let mut afresh = without_ages(fresh.projection());
            let shown = held.shown();
            afresh.revision = shown.revision;
            assert_eq!(shown, afresh, "seed {seed}, step {step}");
            let mut watching = watched.shown();
            watching.revision = shown.revision;
            assert_eq!(watching, afresh, "seed {seed}, step {step}");
        }
        assert_eq!(held.resyncs, 0, "seed {seed}: every patch applied");
        assert_eq!(watched.resyncs, 0, "seed {seed}: every patch applied");
        assert!(held.patches.len() > 30, "seed {seed}: sent as patches");
    }
    assert!(
        done.iter().all(|done| *done > 0),
        "every kind of change was made: {done:?}"
    );
}

#[test]
fn a_size_given_with_both_hands_is_refused_where_the_panel_changed_since_they_took_hold_of_it() {
    let (hub, _jobs) = hub_waiting();
    let (space, _commands) = idle(&hub);
    let ada = Client::live(&hub, Access::open(), "ada");
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
    // Stretched to half as large again against the version seen, it floats at that size, sent whole as it changed.
    ada.send(ClientMessage::PlacePanel {
        panel: panel.id.clone(),
        wall: None,
        slot: None,
        pose: Some(upright([1.0, 1.5, -1.0])),
        size: Some(1.5),
        revision: Some(panel.version),
    });
    held.read(&ada);
    let stretched = held.shown().panels[0].clone();
    assert_eq!(stretched.size, Some(1.5));
    let tomori_protocol::Place::Free { .. } = stretched.place else {
        unreachable!("it floats, as it was left")
    };
    assert!(stretched.version > panel.version);
    // Again against the version it had before, it is refused, naming the panel, and stays as it was.
    ada.send(ClientMessage::PlacePanel {
        panel: panel.id.clone(),
        wall: None,
        slot: None,
        pose: Some(upright([2.0, 1.5, -1.0])),
        size: Some(3.0),
        revision: Some(panel.version),
    });
    held.read(&ada);
    assert!(
        held.errors
            .iter()
            .any(|(_, named, _)| named.as_deref() == Some(panel.id.as_str())),
        "{:?}",
        held.errors
    );
    let shown = held.shown().panels[0].clone();
    assert_eq!(shown.size, Some(1.5));
    assert_eq!(shown.place, stretched.place);
    // Moved with one hand, without a size, it keeps the one it had.
    ada.send(ClientMessage::PlacePanel {
        panel: panel.id.clone(),
        wall: None,
        slot: None,
        pose: Some(upright([2.0, 1.5, -1.0])),
        size: None,
        revision: None,
    });
    held.read(&ada);
    assert_eq!(held.shown().panels[0].size, Some(1.5));
    // A size with a wall means nothing: on a wall, its slot sizes it.
    ada.send(ClientMessage::PlacePanel {
        panel: panel.id.clone(),
        wall: Some(tomori_protocol::TOMORI_WALL.into()),
        slot: None,
        pose: None,
        size: Some(2.0),
        revision: Some(held.shown().panels[0].version),
    });
    held.read(&ada);
    assert_eq!(held.shown().panels[0].size, None);
}

#[test]
fn a_panel_s_and_a_wall_s_version_is_the_revision_of_their_last_change() {
    let (hub, _jobs) = hub_waiting();
    let (space, _commands) = idle(&hub);
    let ada = Client::live(&hub, Access::open(), "ada");
    let mut held = Held::default();
    ada.send(ClientMessage::Join {
        space: space.clone(),
    });
    reply(
        &hub,
        &space,
        "m1",
        "```panel\nid: agenda\ntitle: Agenda\n---\n- [ ] One\n- [ ] Two\n```\n",
    );
    held.read(&ada);
    let shown = held.shown();
    let (wall, panel) = (shown.walls[0].clone(), shown.panels[0].clone());
    assert!(wall.version > 0 && panel.version > 0);
    // A tick: the panel is sent whole, at the revision the patch goes to; the wall is as it was.
    ada.send(ClientMessage::CheckPanelItem {
        panel: panel.id.clone(),
        line: 0,
        checked: true,
        revision: Some(panel.version),
    });
    held.read(&ada);
    let patch = held.patches.last().unwrap().clone();
    let ticked = held.shown().panels[0].clone();
    assert_eq!(ticked.version, patch.to);
    assert_eq!(held.shown().walls[0].version, wall.version);
    // A move: a place op, carrying the version it made, to a client that keeps it.
    ada.send(ClientMessage::PlacePanel {
        panel: panel.id.clone(),
        wall: None,
        slot: None,
        pose: Some(upright([1.0, 1.5, -1.0])),
        size: None,
        revision: None,
    });
    held.read(&ada);
    let patch = held.patches.last().unwrap().clone();
    assert!(
        matches!(patch.ops.as_slice(), [tomori_protocol::PatchOp::Place { version, .. }] if *version == patch.to),
        "{:?}",
        patch.ops
    );
    assert_eq!(held.shown().panels[0].version, patch.to);
    // A wall's change: the wall at the revision it was made at, its panels as they were.
    ada.send(ClientMessage::SetWall {
        wall: wall.id.clone(),
        grid: None,
        scale: None,
        aspect: None,
        shape: None,
        pose: None,
        tilt: Some(10.0),
        content_scale: None,
        revision: Some(wall.version),
    });
    held.read(&ada);
    let patch = held.patches.last().unwrap().clone();
    assert_eq!(held.shown().walls[0].version, patch.to);
    assert!(held.shown().panels[0].version < patch.to);
    // Sent again with nothing changed, nothing's version changes.
    let before = held.shown();
    reply(&hub, &space, "m2", "Nothing to show.");
    ada.send(ClientMessage::CheckPanelItem {
        panel: panel.id.clone(),
        line: 0,
        checked: true,
        revision: Some(ticked.version + 99),
    });
    held.read(&ada);
    let mut after = held.shown();
    after.revision = before.revision;
    assert_eq!(after, before);
}

#[test]
fn of_two_participants_changing_a_panel_or_a_wall_from_one_version_one_wins_and_the_other_is_refused()
 {
    let (hub, _jobs) = hub_waiting();
    let (space, _commands) = idle(&hub);
    let ada = Client::live(&hub, Access::open(), "ada");
    let bob = Client::live(&hub, Access::open(), "bob");
    let (mut seen_by_ada, mut seen_by_bob) = (Held::default(), Held::default());
    for (client, held) in [(&ada, &mut seen_by_ada), (&bob, &mut seen_by_bob)] {
        client.send(ClientMessage::Join {
            space: space.clone(),
        });
        held.read(client);
    }
    reply(
        &hub,
        &space,
        "m1",
        "```panel\nid: agenda\ntitle: Agenda\n---\n- [ ] One\n- [ ] Two\n```\n",
    );
    seen_by_ada.read(&ada);
    seen_by_bob.read(&bob);
    let panel = seen_by_ada.shown().panels[0].clone();
    let wall = seen_by_ada.shown().walls[0].clone();
    assert_eq!(seen_by_bob.shown(), seen_by_ada.shown());
    // Both tick an item of the panel as they see it, Ada first: hers lands, and Bob's, made against what changed under
    // it, is refused, naming the panel, rather than tick whatever is on its line now.
    for (client, line) in [(&ada, 0), (&bob, 1)] {
        client.send(ClientMessage::CheckPanelItem {
            panel: panel.id.clone(),
            line,
            checked: true,
            revision: Some(panel.version),
        });
    }
    let refused = |held: &mut Held, client: &Client| {
        held.read(client);
        std::mem::take(&mut held.errors)
    };
    assert!(refused(&mut seen_by_ada, &ada).is_empty());
    assert_eq!(
        refused(&mut seen_by_bob, &bob),
        [(
            crate::stage::STALE_TICK.to_owned(),
            Some(panel.id.clone()),
            None
        )]
    );
    // Both change the Tomori wall from the version they took hold of it at, Bob first: his lands, and Ada's, which
    // would undo it, is refused, naming the wall.
    let change = |columns: u32| ClientMessage::SetWall {
        wall: wall.id.clone(),
        grid: tomori_protocol::Grid::new(columns, 1),
        scale: None,
        aspect: None,
        shape: None,
        pose: None,
        tilt: None,
        content_scale: None,
        revision: Some(wall.version),
    };
    bob.send(change(4));
    ada.send(change(5));
    assert!(refused(&mut seen_by_bob, &bob).is_empty());
    assert_eq!(
        refused(&mut seen_by_ada, &ada),
        [(
            crate::stage::STALE_WALL.to_owned(),
            None,
            Some(wall.id.clone())
        )]
    );
    seen_by_ada.read(&ada);
    seen_by_bob.read(&bob);
    let shown = seen_by_ada.shown();
    assert_eq!(shown.walls[0].grid().columns, 4);
    assert_eq!(shown.panels[0].body, "- [x] One\n- [ ] Two");
    assert_eq!(seen_by_bob.shown(), shown);
}

/// A space in an environment whose server side publishes onto its notice board, with a client that applies data ops,
/// Lea, and one that applies patches without them, Ada, joined, and an outlet to publish with.
fn publishing_space() -> (Arc<Hub>, String, SpaceOutlet, Client, Client) {
    let (hub, _jobs) = hub_with(FakeBackend::new());
    hub.set_environments(Environments::new([
        crate::environments::Environment::new("sky", "Sky"),
        crate::environments::Environment::new("yard", "Yard")
            .with_walls([upright([3.0, 1.4, -4.0])])
            .with_server_side(Idle),
    ]));
    hub.serving();
    let mut bound = request("ada");
    bound.environment = "yard".to_owned();
    let (id, _commands) = running(&hub, &bound, Snapshot::default());
    let outlet = SpaceOutlet {
        hub: Arc::downgrade(&hub),
        space: id.clone(),
        binding: hub.server_sides.binding(&id).unwrap(),
        environment: "yard".to_owned(),
        version: 1,
    };
    let lea = Client::live(&hub, Access::open(), "lea");
    let ada = Client::patching(&hub, Access::open(), "ada");
    for client in [&lea, &ada] {
        client.send(ClientMessage::Join { space: id.clone() });
    }
    (hub, id, outlet, lea, ada)
}

/// A pressure chart of 15 minutes at 5 seconds, `polls` polls in: each poll moves its points along one.
fn pressure(polls: usize) -> tomori_environment::Panel {
    use tomori_environment::chart::{Chart, Colour, Series, SeriesStyle};
    let values = (polls..polls + 181)
        .map(|at| Some(0.5 + (at % 17) as f32 / 20.0))
        .collect();
    tomori_environment::Panel::chart(
        "pressure",
        "Pressure",
        Chart::new(181).with_series(Series::new(
            "greatest",
            Colour::Amber,
            SeriesStyle::Line,
            values,
        )),
    )
}

#[test]
fn a_live_chart_reaches_a_client_that_applies_data_ops_as_its_data_alone_coalesced_and_limited() {
    use tomori_environment::Outlet;
    let (hub, _space, outlet, lea, ada) = publishing_space();
    let (mut live, mut older) = (Held::default(), Held::default());
    assert_eq!(outlet.publish(pressure(0)), Ok(()));
    live.read(&lea);
    older.read(&ada);
    let first = live.shown().panels[0].clone();
    assert!(first.data_seq > 0);
    // Polled again at once, it waits, as the environment's changes are limited, and a later poll takes its place.
    assert_eq!(outlet.publish(pressure(1)), Ok(()));
    assert_eq!(outlet.publish(pressure(2)), Ok(()));
    assert!(lea.read().is_empty());
    // Made at the hub's next sweep, it is one data op, its two new points appended, its version as it was.
    hub.flush_publishes(Instant::now() + Duration::from_secs(1));
    live.read(&lea);
    older.read(&ada);
    let patch = live.patches.last().unwrap().clone();
    let [
        tomori_protocol::PatchOp::ChartAppend {
            values, from, seq, ..
        },
    ] = patch.ops.as_slice()
    else {
        unreachable!("{:?}", patch.ops);
    };
    assert_eq!((values[0].len(), *from), (2, first.data_seq));
    let shown = live.shown().panels[0].clone();
    assert_eq!((shown.version, shown.data_seq), (first.version, *seq));
    // The older client is sent the panel whole, and both hold the same.
    assert!(matches!(
        older.patches.last().unwrap().ops.as_slice(),
        [tomori_protocol::PatchOp::Panel { .. }]
    ));
    assert_eq!(live.shown(), older.shown());
    // A participant's tick of another panel, or anything else made against a version, is never refused for the
    // chart's data having moved on: its version did not change.
    assert_eq!(shown.version, first.version);
}

#[test]
fn a_client_that_misses_a_data_op_is_sent_the_live_panel_s_data_whole_with_the_projection() {
    use tomori_environment::Outlet;
    let (hub, _space, outlet, lea, ada) = publishing_space();
    let (mut live, mut older) = (Held::default(), Held::default());
    assert_eq!(outlet.publish(pressure(0)), Ok(()));
    live.read(&lea);
    older.read(&ada);
    // A patch of its data missed leaves a gap, which the next shows; asked for, the projection is sent whole, the
    // chart's data with it at its seq, and patched from there again.
    live.missing = true;
    let mut soon = Instant::now();
    for polls in 1..=2 {
        soon += Duration::from_secs(1);
        assert_eq!(outlet.publish(pressure(polls)), Ok(()));
        hub.flush_publishes(soon);
        live.read(&lea);
    }
    assert_eq!((live.wholes, live.resyncs), (2, 1));
    older.read(&ada);
    assert_eq!(live.shown(), older.shown());
    let whole = live.shown().panels[0].clone();
    assert_eq!(
        whole.chart.as_ref().unwrap().series[0].values[180],
        tomori_protocol::chart::rounded(0.5 + ((2 + 180) % 17) as f32 / 20.0)
    );
    soon += Duration::from_secs(1);
    assert_eq!(outlet.publish(pressure(3)), Ok(()));
    hub.flush_publishes(soon);
    live.read(&lea);
    let patch = live.patches.last().unwrap().clone();
    assert!(
        matches!(patch.ops.as_slice(), [tomori_protocol::PatchOp::ChartAppend { from, .. }] if *from == whole.data_seq),
        "{:?}",
        patch.ops
    );
    assert_eq!(live.resyncs, 1);
}

#[test]
fn a_client_sent_less_holds_each_wall_at_the_version_every_client_has_and_hears_nothing_of_one_it_was_not_sent()
 {
    let (hub, _jobs) = hub_waiting();
    let (space, _commands) = idle(&hub);
    // The desktop window states the most; a headset that states nothing is sent today's walls.
    let desktop = Client::patching_as(&hub, Access::open(), "desktop", Some(Limits::MOST), true);
    let headset = Client::live(&hub, Access::open(), "headset");
    let (mut all, mut some) = (Held::default(), Held::default());
    for (client, held) in [(&desktop, &mut all), (&headset, &mut some)] {
        client.send(ClientMessage::Join {
            space: space.clone(),
        });
        held.read(client);
    }
    for index in 0..20 {
        let x = (index % 10) as f32 * 3.0 - 13.5;
        let z = 4.0 + (index / 10) as f32 * 3.0;
        desktop.send(ClientMessage::AddWall {
            pose: upright([x, 1.5, z]),
        });
    }
    all.read(&desktop);
    some.read(&headset);
    let (everything, sent) = (all.shown(), some.shown());
    assert_eq!(sent.walls.len(), Limits::TODAY.walls as usize);
    // Every wall the headset holds is at the version the desktop window holds it at.
    for wall in &sent.walls {
        assert_eq!(Some(wall), everything.wall(&wall.id), "{}", wall.id);
    }
    // A wall the headset was never sent changes, from the version the desktop window holds: it lands, and the headset
    // is sent nothing of it.
    let unsent = everything.walls.last().unwrap().clone();
    assert!(sent.wall(&unsent.id).is_none());
    let patches = some.patches.len();
    desktop.send(ClientMessage::SetWall {
        wall: unsent.id.clone(),
        grid: None,
        scale: None,
        aspect: None,
        shape: None,
        pose: None,
        tilt: Some(10.0),
        content_scale: None,
        revision: Some(unsent.version),
    });
    all.read(&desktop);
    some.read(&headset);
    assert!(all.shown().wall(&unsent.id).unwrap().version > unsent.version);
    // The headset is sent the projection again, as everyone in the space is at every change, with nothing in it.
    assert!(some.patches.len() > patches);
    assert!(
        some.patches[patches..]
            .iter()
            .all(|patch| patch.ops.is_empty())
    );
    let mut now = some.shown();
    now.revision = sent.revision;
    assert_eq!(now, sent);
    assert!(all.errors.is_empty() && some.errors.is_empty());
}

/// How many projections and patches `client` was sent since.
fn sends(client: &Client) -> usize {
    let sent = client.read().into_iter().filter(|message| {
        matches!(
            message,
            ServerMessage::Projection { .. } | ServerMessage::ProjectionPatch { .. }
        )
    });
    sent.count()
}

/// How many times this thread has built what a connection is sent of a space.
fn views() -> usize {
    crate::stage::VIEWS.with(std::cell::Cell::get)
}

/// What an environment's changes cost the hub's lock, which every space shares, counted rather than timed: each
/// connection is told of them at most four times a second, however often its panels change, the first change at once,
/// and every connection of the same limits told of a change shares one build of the projection.
#[test]
fn an_environment_s_changes_are_told_four_times_a_second_and_built_once_for_connections_alike() {
    let (hub, _jobs) = hub_with(FakeBackend::new());
    hub.set_environments(Environments::new([
        crate::environments::Environment::new("sky", "Sky"),
        crate::environments::Environment::new("yard", "Yard")
            .with_walls(
                [upright([3.0, 1.4, -4.0]), upright([-3.0, 1.4, -4.0])].map(|pose| {
                    crate::EnvironmentWall::new(pose).with_grid(tomori_protocol::Grid::from((8, 2)))
                }),
            )
            .with_server_side(Idle),
    ]));
    hub.serving();
    let mut bound = request("ada");
    bound.environment = "yard".to_owned();
    let (id, _commands) = running(&hub, &bound, Snapshot::default());
    let outlet = SpaceOutlet {
        hub: Arc::downgrade(&hub),
        space: id.clone(),
        binding: hub.server_sides.binding(&id).unwrap(),
        environment: "yard".to_owned(),
        version: 1,
    };
    // Six connections of three kinds: two that state nothing and apply no patches, sent the projection whole within
    // today's limits; two that take the most and apply data ops, as Tomori's own; and two that apply patches within
    // twice today's panels.
    let mut wider = Limits::MOST;
    wider.panels = 2 * Limits::TODAY.panels;
    let clients = [
        Client::projecting(&hub, Access::open(), "old-1"),
        Client::projecting(&hub, Access::open(), "old-2"),
        Client::patching_as(&hub, Access::open(), "live-1", Some(Limits::MOST), true),
        Client::patching_as(&hub, Access::open(), "live-2", Some(Limits::MOST), true),
        Client::patching_within(&hub, Access::open(), "wide-1", Some(wider)),
        Client::patching_within(&hub, Access::open(), "wide-2", Some(wider)),
    ];
    for client in &clients {
        client.send(ClientMessage::Join { space: id.clone() });
        client.read();
    }
    let kinds = 3;
    // As many panels as a side showed before it showed 128, fewer than it puts up at once.
    let panels = 32;
    const { assert!(32 <= crate::channel::PUBLISH_PAGE) };
    let panel = |index: usize, value: usize| {
        tomori_environment::Panel::text(format!("p{index}"), format!("P{index}"), value.to_string())
            .on_wall(index as u32 % 2, None)
    };
    let start = Instant::now();
    let at = |millis: u64| start + Duration::from_millis(millis);
    // The side puts up those panels in one instant: each connection is sent them once, all of them, from one build
    // for each kind of connection, and nothing again once the interval ends, since each has them.
    let built = views();
    for index in 0..panels {
        assert_eq!(
            hub.channel_publish_at(&outlet, panel(index, 0), at(0)),
            Ok(())
        );
    }
    assert_eq!(clients.each_ref().map(sends), [1; 6]);
    // Or one more for each, were a second to end between two connections' sends, since ages are counted in seconds.
    let builds = views() - built;
    assert!((kinds..=2 * kinds).contains(&builds), "{builds}");
    assert_eq!(hub.snapshot_deadline(), Some(at(0) + PROJECTION_INTERVAL));
    hub.flush_snapshots(at(0) + PROJECTION_INTERVAL);
    assert_eq!(clients.each_ref().map(sends), [0; 6]);
    assert_eq!(hub.snapshot_deadline(), None);
    // One change, after a quiet while, is told at once; the next within the interval waits for its end, and is told
    // then, with the newest of whatever changed since.
    assert_eq!(
        hub.channel_publish_at(&outlet, panel(0, 1), at(2000)),
        Ok(())
    );
    assert_eq!(hub.snapshot_deadline(), None);
    assert_eq!(clients.each_ref().map(sends), [1; 6]);
    assert_eq!(
        hub.channel_publish_at(&outlet, panel(1, 1), at(2010)),
        Ok(())
    );
    assert_eq!(
        hub.channel_publish_at(&outlet, panel(2, 1), at(2020)),
        Ok(())
    );
    assert_eq!(clients.each_ref().map(sends), [0; 6]);
    assert_eq!(hub.snapshot_deadline(), Some(at(2250)));
    hub.flush_snapshots(at(2249));
    assert_eq!(clients.each_ref().map(sends), [0; 6]);
    hub.flush_snapshots(at(2250));
    let whole = latest_whole(&clients[0]);
    let bodies = |shown: &Projection| -> Vec<String> {
        let body = |index: usize| {
            let title = format!("P{index}");
            let panel = shown.panels.iter().find(|panel| panel.title == title);
            panel.unwrap().body.clone()
        };
        (0..panels).map(body).collect()
    };
    assert_eq!(bodies(&whole)[..4], ["1", "1", "1", "0"]);
    assert_eq!(clients.each_ref().map(sends)[1..], [1; 5]);
    // The side then changes a panel 16 times a second, as fast as a space's panels could before they were 128, for
    // five seconds, the deferred changes told as each interval ends, as the server's timer tells them. Before each
    // connection was paced it was sent every one, 16 a second, and built the projection for itself each time, twice
    // where it takes less than the most: 80 sends each, and 800 builds for these six.
    let (built, mut told) = (views(), [0; 6]);
    for change in 0..80_u64 {
        let now = at(4000 + change * 1000 / 16);
        while let Some(due) = hub.snapshot_deadline().filter(|due| *due <= now) {
            hub.flush_snapshots(due);
        }
        let update = panel(change as usize % panels, 2 + change as usize / panels);
        assert_eq!(hub.channel_publish_at(&outlet, update, now), Ok(()));
        let second = clients.each_ref().map(sends);
        for (told, sent) in told.iter_mut().zip(second) {
            *told += sent;
        }
        // Never more than one send between two changes, and by the end of each second at most four more.
        assert!(second.iter().all(|sent| *sent <= 1));
        if change % 16 == 15 {
            let most = 4 * (change as usize / 16 + 1);
            assert!(told.iter().all(|told| *told <= most), "{told:?}");
        }
    }
    hub.flush_snapshots(at(9250));
    for (told, sent) in told.iter_mut().zip(clients.each_ref().map(sends)) {
        *told += sent;
    }
    // Four a second each, where there were 16, and one more for what changed last; and one build a send for each
    // kind of connection, with one more wherever a second ended between two connections' sends, since each panel's
    // age is counted in seconds: 63 builds, where there were 800.
    assert_eq!(told, [21; 6]);
    let builds = views() - built;
    assert!((21 * kinds..=23 * kinds).contains(&builds), "{builds}");
    // Each holds the latest of everything.
    let whole = clients[0].projection();
    assert_eq!(bodies(&whole)[..16], [4; 16].map(|value| value.to_string()));
    assert_eq!(bodies(&whole)[16..], [3; 16].map(|value| value.to_string()));
    // A space keeps what it built for only so many kinds of connection: nine more kinds, each stating limits of its
    // own, the last two connections alike, are six kinds past them, and the last kind is built for each of its two.
    let others: Vec<Client> = [0, 1, 2, 3, 4, 5, 6, 7, 8, 8]
        .into_iter()
        .map(|kind| {
            let mut limits = Limits::MOST;
            limits.panels = 100 + kind;
            let name = format!("other-{kind}-{}", views());
            let client = Client::patching_within(&hub, Access::open(), &name, Some(limits));
            client.send(ClientMessage::Join { space: id.clone() });
            client.read();
            client
        })
        .collect();
    let built = views();
    assert_eq!(
        hub.channel_publish_at(&outlet, panel(0, 9), at(20_000)),
        Ok(())
    );
    for client in clients.iter().chain(&others) {
        assert_eq!(sends(client), 1);
    }
    let builds = views() - built;
    assert!(
        (kinds + 10..=2 * (kinds + 10)).contains(&builds),
        "{builds}"
    );
    // A retraction is told as any change its side makes: at once after a quiet while, and the next at the
    // interval's end.
    assert_eq!(hub.channel_retract_at(&outlet, "p5", at(30_000)), Ok(()));
    assert_eq!(clients.each_ref().map(sends), [1; 6]);
    assert_eq!(hub.channel_retract_at(&outlet, "p6", at(30_100)), Ok(()));
    assert_eq!(clients.each_ref().map(sends), [0; 6]);
    assert_eq!(hub.snapshot_deadline(), Some(at(30_250)));
    hub.flush_snapshots(at(30_250));
    assert_eq!(clients.each_ref().map(sends), [1; 6]);
    assert_eq!(clients[0].projection().panels.len(), panels - 2);
}

/// A change made at once, a participant's or the agent's, while a change of the environment's waits for its interval
/// to end, tells everyone both together, and nothing is then left to tell at the interval's end.
#[test]
fn a_change_told_at_once_carries_what_the_environment_changed_that_waited() {
    let (hub, id, outlet, lea, ada) = publishing_space();
    let text = |id: &str, body: &str| tomori_environment::Panel::text(id, id, body);
    let start = Instant::now();
    let at = |millis: u64| start + Duration::from_millis(millis);
    let bodies = |held: &Held| -> Vec<(String, String)> {
        let panels = held.shown().panels.into_iter();
        panels.map(|panel| (panel.title, panel.body)).collect()
    };
    // When what the environment changed is due to be told, if any of it waits.
    let deferred = || {
        let state = hub.lock();
        state.space(&id).unwrap().projection_deferred.get()
    };
    let (mut live, mut patched) = (Held::default(), Held::default());
    for panel in ["a", "b", "c"] {
        assert_eq!(
            hub.channel_publish_at(&outlet, text(panel, "1"), at(0)),
            Ok(())
        );
    }
    hub.flush_snapshots(at(250));
    assert_eq!(
        hub.channel_publish_at(&outlet, text("a", "2"), at(1000)),
        Ok(())
    );
    live.read(&lea);
    patched.read(&ada);
    // The environment changes another panel within the interval: it waits, and nobody is told.
    assert_eq!(
        hub.channel_publish_at(&outlet, text("b", "2"), at(1100)),
        Ok(())
    );
    assert_eq!(deferred(), Some(at(1250)));
    assert_eq!((sends(&lea), sends(&ada)), (0, 0));
    // A participant throws the third panel away meanwhile: told at once, as ever, with what waited.
    let thrown = live.shown().panels[2].id.clone();
    lea.send(ClientMessage::DismissPanel { panel: thrown });
    assert_eq!(deferred(), None);
    live.read(&lea);
    patched.read(&ada);
    let expected = vec![
        ("a".to_owned(), "2".to_owned()),
        ("b".to_owned(), "2".to_owned()),
    ];
    assert_eq!(
        (bodies(&live), bodies(&patched)),
        (expected.clone(), expected)
    );
    hub.flush_snapshots(at(1250));
    assert_eq!((sends(&lea), sends(&ada)), (0, 0));
    // So with the agent's own panel: its reply's panel goes up at once, with the environment's change that waited.
    assert_eq!(
        hub.channel_publish_at(&outlet, text("a", "3"), at(2000)),
        Ok(())
    );
    live.read(&lea);
    patched.read(&ada);
    assert_eq!(
        hub.channel_publish_at(&outlet, text("b", "3"), at(2100)),
        Ok(())
    );
    assert_eq!(deferred(), Some(at(2250)));
    reply(&hub, &id, "m1", "```panel\ntitle: Notes\n---\nA note.\n```");
    assert_eq!(deferred(), None);
    live.read(&lea);
    patched.read(&ada);
    for held in [&live, &patched] {
        let shown = bodies(held);
        assert!(
            shown.contains(&("b".to_owned(), "3".to_owned())),
            "{shown:?}"
        );
        assert!(
            shown.contains(&("Notes".to_owned(), "A note.".to_owned())),
            "{shown:?}"
        );
    }
    hub.flush_snapshots(at(2250));
    assert_eq!((sends(&lea), sends(&ada)), (0, 0));
}
