// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where people arrive as they join a space: the spawn point the server chooses for each, by their role, where the
//! others stand and who arrived where, sent to them alone, as they join, rejoin or the space changes environment. See
//! `design/environments.md`.
use super::*;
use tomori_protocol::Pose;
use tomori_protocol::spawn::{self, Spawn};

impl Client {
    /// Connected and greeted as `participant`, by a client that arrives where the server says, with what that sent
    /// read.
    fn arriving(hub: &Arc<Hub>, access: Access, participant: &str) -> Self {
        let client = Self::connect(hub, access);
        client.send(ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            participant: participant.to_owned(),
            name: participant.to_owned(),
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
            spawns: true,
            limits: None,
        });
        client.read();
        client
    }

    /// Join `space`, saying where it was told it arrives, if anywhere, after the space's state.
    fn arrive(&self, space: &str) -> Option<[f32; 3]> {
        self.send(ClientMessage::Join {
            space: space.to_owned(),
        });
        spawned(&self.read())
    }

    /// Stand with its head at `head`.
    fn stand(&self, head: [f32; 3]) {
        self.send(ClientMessage::Presence {
            head: Pose::at(head),
            left: None,
            right: None,
            hands: None,
            carrying: None,
            carried: Vec::new(),
            rays: None,
            pointing: None,
        });
    }
}

/// Where `messages` say their client arrives, checking that it is told after the space's state names its environment.
fn spawned(messages: &[ServerMessage]) -> Option<[f32; 3]> {
    let state = messages
        .iter()
        .position(|message| matches!(message, ServerMessage::Space { .. }));
    let told: Vec<(usize, [f32; 3])> = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| match message {
            ServerMessage::Spawn {
                at, environment, ..
            } => {
                assert_eq!(environment, "yard");
                Some((index, at.position))
            }
            _ => None,
        })
        .collect();
    assert!(told.len() <= 1, "{messages:?}");
    let (index, at) = *told.first()?;
    assert!(state.is_some_and(|state| state < index), "{messages:?}");
    Some(at)
}

/// A yard whose visitors arrive by the gate, and everyone else at one of two places by the campsite; the grid beside
/// it, declaring none; and a space in the yard.
fn yard() -> (Arc<Hub>, async_channel::Receiver<Job>, String) {
    let (hub, jobs) = hub_waiting();
    hub.set_environments(Environments::new([
        crate::environments::Environment::new("grid", "Grid"),
        crate::environments::Environment::new("yard", "Yard")
            .with_spawn(Spawn::new(Pose::at([0.0, 0.0, 4.0])).with_tag(spawn::VISITOR))
            .with_spawn(Spawn::new(Pose::at([6.0, 0.0, 0.0])))
            .with_spawn(Spawn::new(Pose::at([9.0, 0.0, 0.0]))),
    ]));
    let mut bound = request("ada");
    bound.environment = "yard".to_owned();
    let (space, _commands) = running(&hub, &bound, Snapshot::default());
    (hub, jobs, space)
}

#[test]
fn each_arrives_where_their_role_is_tagged_spread_out_and_never_where_someone_stands() {
    let (hub, _jobs, space) = yard();
    let ada = Client::arriving(&hub, Access::open(), "ada");
    assert_eq!(ada.arrive(&space), Some([6.0, 0.0, 0.0]));
    // Bea joins before Ada has said where she is: she arrives at the other point.
    let bea = Client::arriving(&hub, Access::open(), "bea");
    assert_eq!(bea.arrive(&space), Some([9.0, 0.0, 0.0]));
    // A visitor arrives by the gate.
    let eve = Client::arriving(&hub, Access::visitor(), "eve");
    assert_eq!(eve.arrive(&space), Some([0.0, 0.0, 4.0]));
    // Ada walks off to the gate, so Cy arrives where she arrived; Dan, with both points taken, at the one fewer
    // arrived at, whoever stands there.
    ada.stand([0.5, 1.6, 4.0]);
    let cy = Client::arriving(&hub, Access::open(), "cy");
    assert_eq!(cy.arrive(&space), Some([6.0, 0.0, 0.0]));
    let dan = Client::arriving(&hub, Access::open(), "dan");
    assert_eq!(dan.arrive(&space), Some([9.0, 0.0, 0.0]));
    // Nobody already there is told anything.
    assert_eq!(spawned(&ada.read()), None);
    assert_eq!(spawned(&bea.read()), None);
}

impl Client {
    /// Connected again as `participant`, by a client that arrives where the server says, with what that sent.
    fn reconnecting(hub: &Arc<Hub>, participant: &str) -> (Self, Vec<ServerMessage>) {
        let client = Self::connect(hub, Access::open());
        client.send(ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            participant: participant.to_owned(),
            name: participant.to_owned(),
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
            spawns: true,
            limits: None,
        });
        let read = client.read();
        (client, read)
    }
}

#[test]
fn rejoining_arrives_anew_a_reconnect_where_it_arrived_and_everyone_anew_as_the_space_changes_environment()
 {
    let (hub, _jobs, space) = yard();
    let ada = Client::arriving(&hub, Access::open(), "ada");
    let bea = Client::arriving(&hub, Access::open(), "bea");
    assert_eq!(ada.arrive(&space), Some([6.0, 0.0, 0.0]));
    assert_eq!(bea.arrive(&space), Some([9.0, 0.0, 0.0]));
    // Leaving and joining again, Ada arrives where nobody stands.
    ada.send(ClientMessage::Leave);
    ada.read();
    bea.stand([9.0, 1.6, 0.0]);
    assert_eq!(ada.arrive(&space), Some([6.0, 0.0, 0.0]));
    // Reconnecting puts her back in the space where she arrived, though Bea stands there now, and so does her client
    // joining the space again, counting nothing.
    bea.stand([6.1, 1.6, 0.0]);
    let (again, read) = Client::reconnecting(&hub, "ada");
    assert_eq!(spawned(&read), Some([6.0, 0.0, 0.0]));
    assert_eq!(again.arrive(&space), Some([6.0, 0.0, 0.0]));
    // Both walk off: the next goes to the point fewer arrived at, as if Ada had arrived three times, not five.
    bea.stand([0.0, 1.6, -20.0]);
    again.stand([0.0, 1.6, -20.0]);
    let cy = Client::arriving(&hub, Access::open(), "cy");
    assert_eq!(cy.arrive(&space), Some([9.0, 0.0, 0.0]));
    // Into the grid, which declares none, nobody is told; back into the yard, everyone arrives anew, apart, in the
    // order they joined, whatever they stood at or arrived at before.
    again.send(ClientMessage::SetEnvironment {
        environment: "grid".to_owned(),
    });
    assert_eq!(spawned(&again.read()), None);
    assert_eq!(spawned(&bea.read()), None);
    again.send(ClientMessage::SetEnvironment {
        environment: "yard".to_owned(),
    });
    let arrived = [spawned(&bea.read()), spawned(&again.read())];
    assert_eq!(arrived, [Some([6.0, 0.0, 0.0]), Some([9.0, 0.0, 0.0])]);
    // And reconnecting after that arrives where she arrived in it now.
    let (_again, read) = Client::reconnecting(&hub, "ada");
    assert_eq!(spawned(&read), Some([9.0, 0.0, 0.0]));
}

#[test]
fn a_visitor_admitted_stays_where_they_arrived() {
    let (hub, _jobs, space) = yard();
    let store = Store::new("spawn-admit");
    hub.set_tokens(store.tokens.clone());
    hub.set_encrypted(true);
    let ada = Client::arriving(&hub, Access::open(), "ada");
    assert_eq!(ada.arrive(&space), Some([6.0, 0.0, 0.0]));
    let eve = Client::arriving(&hub, Access::visitor(), "eve");
    assert_eq!(eve.arrive(&space), Some([0.0, 0.0, 4.0]));
    eve.send(ClientMessage::Knock);
    eve.read();
    ada.read();
    hub.admit(take_knock(&hub));
    // A collaborator now, Eve is told nothing, and is kept where she arrived, under her new key.
    let told = eve.read();
    assert!(
        told.iter()
            .any(|message| matches!(message, ServerMessage::Admitted { .. }))
    );
    assert_eq!(spawned(&told), None);
    let state = hub.lock();
    let key = state.connections[&eve.connection]
        .participant
        .clone()
        .unwrap();
    assert_ne!(key, "/eve");
    let kept = state.space(&space).unwrap().arrivals.kept(&key);
    assert_eq!(kept.map(|pose| pose.position), Some([0.0, 0.0, 4.0]));
}

#[test]
fn a_client_that_does_not_ask_and_a_space_whose_environment_declares_none_are_told_nothing() {
    let (hub, _jobs, space) = yard();
    // The spectator, and an older client, are never told, and stand nowhere anyone arrives.
    let watching = Client::greeted(&hub, Access::open(), "watching");
    assert_eq!(watching.arrive(&space), None);
    let ada = Client::arriving(&hub, Access::open(), "ada");
    assert_eq!(ada.arrive(&space), Some([6.0, 0.0, 0.0]));
    // A space in an environment that declares none.
    let (plain, _commands) = running(&hub, &request("bea"), Snapshot::default());
    let bea = Client::arriving(&hub, Access::open(), "bea");
    assert_eq!(bea.arrive(&plain), None);
}

#[test]
fn creating_a_space_arrives_its_creator_in_it() {
    let (hub, _jobs, _space) = yard();
    let cy = Client::arriving(&hub, Access::open(), "cy");
    let mut bound = request("/cy");
    bound.environment = "yard".to_owned();
    let (_created, _commands) = running(&hub, &bound, Snapshot::default());
    assert_eq!(spawned(&cy.read()), Some([6.0, 0.0, 0.0]));
}

#[test]
fn someone_no_point_suits_is_told_nothing_and_arrives_at_home() {
    let (hub, _jobs) = hub_waiting();
    hub.set_environments(Environments::new([crate::environments::Environment::new(
        "gate", "Gate",
    )
    .with_spawn(Spawn::new(Pose::at([0.0, 0.0, 4.0])).with_tag(spawn::VISITOR))]));
    let mut bound = request("ada");
    bound.environment = "gate".to_owned();
    let (space, _commands) = running(&hub, &bound, Snapshot::default());
    let ada = Client::arriving(&hub, Access::open(), "ada");
    ada.send(ClientMessage::Join {
        space: space.clone(),
    });
    let told = ada.read();
    assert!(
        !told
            .iter()
            .any(|message| matches!(message, ServerMessage::Spawn { .. })),
        "{told:?}"
    );
}
