// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Changing a space's environment after it is created. See `design/environments.md`.
use std::sync::Arc;

use tomori_backend::Event;
use tomori_protocol::{Activity, ClientMessage, ServerMessage};
use tomori_security::Grant;
use tomori_server::fake::{FAKE_BACKEND_NAME, FakeBackend, FakeSession};
use tomori_server::test_support::{TestClient, TestServer, TextTranscriber, eventually};
use tomori_server::{Environment, Server};

/// Sky's layer, registered in its place as another project's builder may.
const STARS: &str = "The space shows the night sky.";
/// The layer of an environment another project registers.
const TOPOLOGY: &str = "The space shows the service's topology.";

/// Tomori's own environments, with Sky given a layer, and another project's, as a builder offers them.
fn offered() -> [Environment; 3] {
    [
        Environment::new("sky", "Sky").with_instructions(STARS),
        Environment::new("minimal", "Grid"),
        Environment::new("observe", "Observe").with_instructions(TOPOLOGY),
    ]
}

/// The entry giving the agent the environment's `layer`, or removing it when there is none.
fn environment_entry(layer: Option<&str>) -> (String, Option<String>) {
    ("tomori-environment".to_owned(), layer.map(str::to_owned))
}

/// The environment's layer `session` holds now.
fn held(session: &FakeSession) -> Option<String> {
    session.instructions().get("tomori-environment").cloned()
}

#[test]
fn an_owner_changes_the_environment_for_everyone_in_the_space() {
    let server = TestServer::start();
    let mut ada = server.connect("ada", "Ada");
    // Outside a space there is nothing to change.
    ada.set_environment("minimal");
    assert_eq!(ada.error(), "Join a space first.");
    let space = ada.create_space("Shrine");
    assert_eq!(space.environment, "sky");
    assert!(space.environment_changes);
    let mut bob = server.connect("bob", "Bob");
    bob.join(&space.id);
    let mut cal = server.connect("cal", "Cal");
    let session = server.backend.started()[0].clone();
    let given = eventually(|| {
        let given = session.log().instructions.len();
        (given == 3).then_some(given)
    });

    // Something said before the change stays in the conversation.
    ada.say("u1", "hello");
    ada.space(|state| {
        state
            .messages
            .iter()
            .any(|message| message.text.contains("hello"))
    });
    ada.set_environment("minimal");
    // Everyone in the space switches, and the lobby lists the space in its new environment.
    for client in [&ada, &bob] {
        let state = client.space(|state| state.id == space.id && state.environment == "minimal");
        assert!(
            state
                .messages
                .iter()
                .any(|message| message.text.contains("hello"))
        );
    }
    // Cal, in the lobby, is listed the space again, in its new environment; the listing from before still names Sky.
    cal.wait_for(|message| match message {
        ServerMessage::Spaces { spaces }
            if spaces
                .iter()
                .any(|listed| listed.id == space.id && listed.environment == "minimal") =>
        {
            Some(())
        }
        _ => None,
    });
    // Someone joining later arrives in it.
    assert_eq!(cal.join(&space.id).environment, "minimal");
    // The agent is given every layer again, the environment's among them.
    eventually(|| (session.log().instructions.len() == given + 3).then_some(()));
    assert!(session.log().instructions[given..].contains(&("tomori-environment".to_owned(), None)));

    // An environment the server does not offer is refused, naming those it does, and changes nothing.
    ada.set_environment("observe");
    assert_eq!(ada.error(), "Unknown environment; available: sky, minimal.");
    ada.set_environment(&"x".repeat(2048));
    assert!(ada.error().contains("environment is too long"));
    // Choosing the environment the space is in already changes nothing, and is not refused.
    ada.set_environment("minimal");
    ada.set_environment("sky");
    bob.space(|state| state.environment == "sky");
    eventually(|| (session.log().instructions.len() == given + 6).then_some(()));

    // A closed space keeps the environment it had.
    session.emit(Event::Closed { reason: None });
    ada.space(|state| state.activity == Activity::Closed);
    ada.set_environment("minimal");
    assert_eq!(ada.error(), "The agent session has stopped.");
    // Cal is still in the space, with snapshots from before unread, so leaves first: the snapshot after joining again is of the space as it is now.
    cal.send(&ClientMessage::Leave);
    cal.wait_for(|message| matches!(message, ServerMessage::Left { .. }).then_some(()));
    assert_eq!(cal.join(&space.id).environment, "sky");
}

#[test]
fn the_environment_s_layer_reaches_the_agent_when_a_space_is_created_and_when_it_changes() {
    let server = TestServer::with_server(FakeBackend::new(), |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default())).with_environments(offered())
    });
    let mut ada = server.connect("ada", "Ada");
    ada.send(&ClientMessage::CreateSpace {
        title: "Topology".to_owned(),
        backend: FAKE_BACKEND_NAME.to_owned(),
        directory: "/tmp".to_owned(),
        session: None,
        environment: Some("observe".to_owned()),
        instructions: None,
    });
    let state = ada.space(|state| state.title == "Topology");
    assert_eq!(state.instructions.unwrap().environment, TOPOLOGY);
    // Given to the agent, through its backend's instruction entries, before its first turn.
    let session = server.backend.started()[0].clone();
    eventually(|| (session.log().instructions.len() == 3).then_some(()));
    assert!(
        session
            .log()
            .instructions
            .contains(&environment_entry(Some(TOPOLOGY)))
    );
    assert_eq!(held(&session).as_deref(), Some(TOPOLOGY));

    // Each change gives the agent the new environment's layer, removing it for one with none.
    for (environment, layer) in [
        ("minimal", None),
        ("sky", Some(STARS)),
        ("observe", Some(TOPOLOGY)),
    ] {
        let given = session.log().instructions.len();
        ada.set_environment(environment);
        let state = ada.space(|state| state.environment == environment);
        assert_eq!(state.instructions.unwrap().environment, layer.unwrap_or(""));
        eventually(|| (session.log().instructions.len() == given + 3).then_some(()));
        assert!(
            session.log().instructions[given..].contains(&environment_entry(layer)),
            "{environment}"
        );
        assert_eq!(held(&session).as_deref(), layer, "{environment}");
    }
}

#[test]
fn only_owners_change_a_space_s_environment() {
    let server = TestServer::with_tokens(true, |server| server);
    let (_, token) = server.issue("Ada", Grant::Owner);
    let mut ada = server.connect_with_token("ada", "Ada", &token);
    let space = ada.create_space("Owned");
    let (_, token) = server.issue(
        "Cal",
        Grant::Collaborator {
            space: space.id.clone(),
        },
    );
    let mut cal = server.connect_with_token("cal", "Cal", &token);
    let mut vic = TestClient::open(server.address);
    vic.hello("vic", "Vic");
    for (client, refusal) in [
        (&mut cal, "Only owners can do that."),
        (&mut vic, "Visitors can watch, but not act in a space."),
    ] {
        client.join(&space.id);
        client.set_environment("minimal");
        assert_eq!(client.error(), refusal);
    }
    // Nothing changed: the owner's next change is the first anyone sees.
    ada.set_environment("minimal");
    let state = cal.space(|state| state.environment != "sky");
    assert_eq!(state.environment, "minimal");
    vic.space(|state| state.environment == "minimal");
}
