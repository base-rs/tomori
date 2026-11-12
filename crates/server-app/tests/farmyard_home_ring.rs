// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The farmyard's home ring, as a participant sees it: from the anchors Tomori's own server sends a farmyard space,
//! through the arrangement the client lays out from them, the ring drawn on the floor about the agent's home, the agent
//! and its workers stand about the middle of the workbench's curve, as the scenery draws the bench there, and the ring
//! is drawn on the yard's earth. See `design/farmyard.md` and `design/environments.md`.
use std::sync::Arc;

use glam::{Vec2, Vec3};
use tomori_client::{ConnectionState, Event};
use tomori_environment_farmyard::layout;
use tomori_protocol::{ClientMessage, ConnectionRole, EnvironmentInfo, ServerMessage};
use tomori_scene::{conversation::Conversation, environment::Environment};
use tomori_server::fake::{FAKE_BACKEND_NAME, FakeBackend};
use tomori_server::test_support::{TestServer, TextTranscriber};
use tomori_server::{Server, WorkspaceRoots};

/// The circle through `a`, `b` and `c` on the ground: its middle.
fn circumcentre(a: Vec2, b: Vec2, c: Vec2) -> Vec2 {
    let (b, c) = (b - a, c - a);
    let d = 2.0 * b.perp_dot(c);
    a + Vec2::new(
        c.y * b.length_squared() - b.y * c.length_squared(),
        b.x * c.length_squared() - c.x * b.length_squared(),
    ) / d
}

#[test]
fn the_farmyards_home_ring_is_drawn_on_the_earth_about_the_middle_of_its_workbenchs_curve() {
    // Tomori's own server, offering its own environments, sends a farmyard space's anchors, its workspace this crate.
    let environments = tomori_server_app::builder().environments().to_vec();
    let server = TestServer::with_server(FakeBackend::new(), move |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default()))
            .with_environments(environments)
            .with_workspace_roots(
                WorkspaceRoots::new([env!("CARGO_MANIFEST_DIR")])
                    .expect("the crate's directory exists"),
            )
    });
    let mut client = server.connect("ada", "Ada");
    client.send(&ClientMessage::CreateSpace {
        title: "Farm".into(),
        backend: FAKE_BACKEND_NAME.into(),
        directory: env!("CARGO_MANIFEST_DIR").into(),
        session: None,
        environment: Some(tomori_environment_farmyard::ID.into()),
        instructions: None,
    });
    let state = client.space(|state| state.title == "Farm");
    assert_eq!(state.environment, tomori_environment_farmyard::ID);
    // A client drawing the farmyard, joined to it, lays out its arrangement from them.
    let farmyard = Environment::new(tomori_environment_farmyard::ID)
        .with_version(tomori_environment_farmyard::VERSION);
    let mut conversation = Conversation::new("Ada");
    conversation.set_drawable(vec![farmyard]);
    conversation.connect(tomori_client::server_choice::Choice::new(
        "Studio",
        "10.0.0.1:8765",
        tomori_client::server_choice::Source::Discovered,
    ));
    conversation.apply(Event::State(ConnectionState::Connected {
        source: 1,
        server: "studio".into(),
        role: ConnectionRole::Owner,
        certificate: None,
        backends: vec![],
        roots: vec![],
        environments: vec![EnvironmentInfo {
            id: farmyard.id().into(),
            label: farmyard.id().into(),
            version: farmyard.version(),
        }],
        utterance_limit: None,
        name: None,
    }));
    conversation.apply(Event::Message(ServerMessage::Space {
        state: Box::new(state.clone()),
    }));
    let arrangement = conversation.arrangement();
    let ring = arrangement.home_ring();
    // The middle of the bench's curve, where the scenery draws it in the space: the circle through its top's front edge,
    // at either end and in the middle.
    let carriage = arrangement.carriage;
    let ground = |at: Vec2| {
        let at = carriage.place(Vec3::new(at.x, 0.0, at.y));
        Vec2::new(at.x, at.z)
    };
    let [front, _] = layout::WORKBENCH_TOP;
    let span = layout::WORKBENCH_HALF_SPAN;
    let bench = circumcentre(
        ground(layout::on_workbench(front, -span)),
        ground(layout::on_workbench(front, 0.0)),
        ground(layout::on_workbench(front, span)),
    );
    // The ring drawn on the floor stands about it, within a millimetre, the workers' ring of 1.4 m, opening out of the
    // corner, the way the bench faces, over all but its 60° behind the agent.
    let middle = Vec2::new(ring.centre.x, ring.centre.z);
    assert!(middle.distance(bench) < 1e-3, "{middle} {bench}");
    assert_eq!(ring.radius, tomori_protocol::cluster::RING);
    let out = ground(layout::on_workbench(1.0, 0.0)) - bench;
    assert!(
        (Vec2::new(ring.towards.x, ring.towards.z).normalize() + out.normalize()).length() < 1e-4,
        "{ring:?}"
    );
    assert!((ring.sweep.to_degrees() - layout::HOME_SWEEP).abs() < 1e-3);
    // The agent's home, above it, and its workers, one to eighteen, each 1.4 m from it about the vertical.
    let home = arrangement.home.position;
    assert!(Vec2::new(home.x, home.z).distance(bench) < 1e-3, "{home}");
    let anchor = state
        .anchors
        .cluster(tomori_protocol::cluster::HOME)
        .expect("the farmyard sends its home anchor");
    let cluster = tomori_protocol::cluster::Cluster::new(anchor)
        .with_audience(arrangement.audience.to_array());
    for count in 1..=18 {
        for slot in cluster.slots(count) {
            let at = Vec2::new(slot.position[0], slot.position[2]);
            assert!(
                (at.distance(bench) - tomori_protocol::cluster::RING).abs() < 1e-3,
                "{count}: {slot:?}"
            );
        }
    }
    // And the yard's earth is the space's floor, which the ring is drawn on however high the agent's home stands.
    let earth = carriage.place(Vec3::new(layout::WORKBENCH.x, 0.0, layout::WORKBENCH.y));
    assert!(
        (earth.y - tomori_protocol::cluster::FLOOR).abs() < 1e-6,
        "{earth}"
    );
    assert!(ring.centre.y > tomori_protocol::cluster::FLOOR + 1.0);
}
