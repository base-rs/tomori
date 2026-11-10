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
fn hello_limits_are_kept_within_today_s_and_welcome_states_today_s() {
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
    assert_eq!(welcome(&client), Some(Limits::TODAY));
    let kept = |client: &Client| {
        let state = hub.lock();
        let record = &state.connections[&client.connection];
        (record.limits, record.patches)
    };
    // A client that states none, or more, is kept within today's; one that states less, within that.
    assert_eq!(kept(&client), (Limits::TODAY, false));
    let more = Limits::TODAY
        .with_frame_bytes(8 * 1024 * 1024)
        .with_projection(40, 500, 60, 560);
    let larger = Client::patching_within(&hub, Access::open(), "larger", Some(more));
    assert_eq!(kept(&larger), (Limits::TODAY, true));
    let less = Limits::TODAY
        .with_frame_bytes(256 * 1024)
        .with_projection(4, 24, 4, 28);
    let smaller = Client::patching_within(&hub, Access::open(), "smaller", Some(less));
    assert_eq!(kept(&smaller), (less, true));
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
fn speech_queued_behind_a_patch_waits_for_far_less_than_behind_the_projection_whole() {
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
    // A change, with agent speech queued after it: the projection goes first, as ever, then the speech, so nothing is
    // reordered; what goes before the speech is the patch, far smaller than the projection whole an older client is
    // sent.
    let ahead = |client: &Client| {
        client.outbox.agent_audio(vec![0; 64]);
        let mut bytes = 0;
        loop {
            let popped = client.outbox.pop();
            if matches!(popped, Popped::Audio(_)) {
                return bytes;
            }
            let Popped::Control(item) = popped else {
                unreachable!("the speech is sent, not {popped:?}");
            };
            let message = hub
                .render(client.connection, item)
                .expect("only the projection is sent before the speech");
            bytes += Frame::server(&message).encode().len();
        }
    };
    ada.send(ClientMessage::AddWall {
        pose: upright([0.0, 1.5, -3.0]),
    });
    let (patched, whole) = (ahead(&ada), ahead(&old));
    assert!(patched > 0);
    assert!(patched * 4 < whole, "{patched} bytes ahead against {whole}");
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
