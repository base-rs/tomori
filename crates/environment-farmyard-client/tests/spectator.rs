// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The farmyard as a spectator shows it: its client side, registered with its scenery, run as a spectator runs it, so
//! that the chickens it shows are drawn for the spectator's audience as for a participant. See `design/spectator.md`.
use std::time::Instant;

use glam::Vec3;
use tomori_client::server_choice::{Choice, Source};
use tomori_client::{ConnectionState, Event};
use tomori_environment_farmyard::flock::{self, Flock};
use tomori_environment_farmyard_client::client::Drawn;
use tomori_environment_farmyard_client::{ID, VERSION, chicken::Chicken};
use tomori_protocol::{ConnectionRole, Payload, ServerMessage, SpaceState};
use tomori_render::scenery::Sceneries;
use tomori_scene::anchors::Anchor;
use tomori_scene::conversation::Conversation;
use tomori_scene::spectating::Spectated;

const SPACE: &str = "space-1";

#[test]
fn a_spectator_draws_the_chickens_the_farmyard_s_client_side_shows() {
    let mut sceneries = Sceneries::empty();
    tomori_environment_farmyard_client::register(&mut sceneries);
    let farmyard = *sceneries
        .environments()
        .iter()
        .find(|environment| environment.id() == ID)
        .expect("registered");
    // A spectator's conversation, a visitor in a space in the farmyard, as `tomori-client-spectator`'s feed gives it the
    // sceneries.
    let mut conversation = Conversation::new("Spectator");
    conversation.set_drawable(sceneries.environments());
    conversation.set_client_sides(sceneries.client_sides());
    conversation.connect(Choice::new("Studio", "127.0.0.1:9", Source::Override));
    conversation.apply(Event::State(ConnectionState::Connected {
        source: 1,
        server: "studio".into(),
        role: ConnectionRole::Visitor,
        certificate: None,
        backends: vec![],
        roots: vec![],
        environments: vec![],
        utterance_limit: None,
        name: None,
    }));
    conversation.apply(Event::Message(ServerMessage::Space {
        state: Box::new(SpaceState {
            id: SPACE.into(),
            environment: ID.into(),
            environment_version: Some(VERSION),
            ..Default::default()
        }),
    }));
    // The flock as the farmyard's server side shares it.
    let clock = 5_000;
    let flock = Flock::new(flock::seed(SPACE), Chicken::shared().unwrap(), clock);
    conversation.apply(Event::Message(ServerMessage::EnvironmentState {
        environment: ID.into(),
        version: VERSION,
        state: Payload::new(serde_json::to_value(flock.state(clock)).unwrap()),
    }));
    let mut spectated = Spectated::new();
    let drawn = spectated
        .update(
            &mut conversation,
            farmyard,
            Anchor::HOME,
            Some(Vec3::new(0.0, 1.6, 0.0)),
            Instant::now(),
        )
        .expect("the farmyard's client side shows the flock");
    let drawn = drawn.downcast_ref::<Drawn>().expect("the farmyard's own");
    assert_eq!(drawn.chickens.len(), flock.hens().len());
    assert!(!drawn.chickens.is_empty());
}
