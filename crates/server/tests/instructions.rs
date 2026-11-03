// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The instructions Tomori gives a space's agent, in layers. See `design/spaces.md`.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use backend::{Event, Session, Snapshot};
use protocol::{ClientMessage, ServerMessage, SpaceState};
use security::Grant;
use server::fake::{FAKE_BACKEND_NAME, FakeBackend, FakeSession};
use server::test_support::{TIMEOUT, TestClient, TestServer};

const KEYS: [&str; 3] = ["tomori", "tomori-environment", "tomori-space"];

/// Create a space whose own instructions are `instructions`, and wait for it.
fn create(client: &mut TestClient, title: &str, instructions: Option<&str>) -> SpaceState {
    client.send(&ClientMessage::CreateSpace {
        title: title.to_owned(),
        backend: FAKE_BACKEND_NAME.to_owned(),
        directory: "/tmp".to_owned(),
        session: None,
        environment: None,
        instructions: instructions.map(str::to_owned),
    });
    client.space(|state| state.title == title)
}

/// Wait until `ready` holds of `session`.
fn until(session: &FakeSession, what: &str, ready: impl Fn(&FakeSession) -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !ready(session) {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The entries a space whose own instructions are `space` gives its agent in Sky, whose layer is empty.
fn entries(space: Option<&str>) -> BTreeMap<String, String> {
    let mut entries = BTreeMap::from([("tomori".to_owned(), tomori().to_owned())]);
    if let Some(space) = space {
        entries.insert("tomori-space".to_owned(), space.to_owned());
    }
    entries
}

fn tomori() -> &'static str {
    "You are Tomori, the coordinator of a shared spatial workspace. People talk to you aloud from headsets, and your replies are spoken. Reply briefly, in one to three short paragraphs of plain speech, without Markdown, tables or code unless someone asks to see them. Each message begins with the speaker's name. Delegate any non-trivial work, such as reading code, running commands or investigating, to sub-agents; say briefly what you have started, and report the findings when they return."
}

#[test]
fn the_layers_are_given_on_creation_shown_to_everyone_and_changed_by_an_owner() {
    let server = TestServer::start();
    let mut ada = server.connect("ada", "Ada");
    let state = create(&mut ada, "Docs", Some("  Mind the docs.  "));
    let session = server.backend.started()[0].clone();
    until(&session, "the layers were never given", |session| {
        session.instructions() == entries(Some("Mind the docs."))
    });
    // Sky contributes nothing yet, so its entry is removed rather than set empty.
    assert!(
        session
            .log()
            .instructions
            .contains(&("tomori-environment".to_owned(), None))
    );
    let shown = state.instructions.expect("the layers are shown");
    assert_eq!(shown.tomori, tomori());
    assert_eq!(shown.environment, "");
    assert_eq!(shown.space, "Mind the docs.");
    assert_eq!(shown.error, None);
    // Given before the agent's first turn.
    ada.say("u1", "hello");
    until(&session, "the prompt was never delivered", |session| {
        !session.log().prompts.is_empty()
    });
    assert_eq!(session.log().instructed_at_prompts, [KEYS.len()]);

    // Everyone in the space sees them, and an owner's change reaches the agent and them.
    let mut bob = server.connect("bob", "Bob");
    bob.send(&ClientMessage::Join {
        space: state.id.clone(),
    });
    bob.space(|state| {
        state
            .instructions
            .as_ref()
            .is_some_and(|layers| layers.space == "Mind the docs.")
    });
    ada.send(&ClientMessage::SetInstructions {
        text: "Mind the tests.".to_owned(),
    });
    bob.space(|state| {
        state
            .instructions
            .as_ref()
            .is_some_and(|layers| layers.space == "Mind the tests.")
    });
    until(&session, "the change never reached the agent", |session| {
        session.instructions() == entries(Some("Mind the tests."))
    });
    // Only the space's own layer is given again.
    let given = session.log().instructions.len();
    ada.send(&ClientMessage::SetInstructions {
        text: " ".to_owned(),
    });
    until(&session, "the space's layer was never removed", |session| {
        session.instructions() == entries(None)
    });
    assert_eq!(
        session.log().instructions[given..],
        [("tomori-space".to_owned(), None)]
    );
    bob.space(|state| {
        state
            .instructions
            .as_ref()
            .is_some_and(|layers| layers.space.is_empty())
    });

    // Too long is refused without changing anything, when changed or when creating a space.
    ada.send(&ClientMessage::SetInstructions {
        text: "x".repeat(4097),
    });
    assert!(ada.error().contains("instructions is too long"));
    ada.send(&ClientMessage::CreateSpace {
        title: "Long".to_owned(),
        backend: FAKE_BACKEND_NAME.to_owned(),
        directory: "/tmp".to_owned(),
        session: None,
        environment: None,
        instructions: Some("x".repeat(4097)),
    });
    assert!(ada.error().contains("instructions is too long"));
    assert_eq!(server.backend.started().len(), 1);
}

#[test]
fn the_layers_are_given_again_after_the_agent_is_read_again() {
    let server = TestServer::start();
    let mut ada = server.connect("ada", "Ada");
    create(&mut ada, "Gap", Some("Mind the docs."));
    let session = server.backend.started()[0].clone();
    until(&session, "the layers were never given", |session| {
        session.instructions() == entries(Some("Mind the docs."))
    });
    let given = session.log().instructions.len();
    session.emit(Event::Reconnecting {
        reason: "Lost the connection to OpenCode; reconnecting.".to_owned(),
    });
    session.emit(Event::Resynchronized {
        snapshot: Snapshot {
            busy: Some(false),
            ..Snapshot::default()
        },
    });
    until(&session, "the layers were never given again", |session| {
        session.log().instructions.len() == given + KEYS.len()
    });
    assert_eq!(session.instructions(), entries(Some("Mind the docs.")));
}

#[test]
fn a_refusal_is_shown_and_the_space_works_without_them() {
    let backend = FakeBackend::new();
    backend.fail_instructions(true);
    let server = TestServer::with_backend(backend.clone());
    let mut ada = server.connect("ada", "Ada");
    ada.send(&ClientMessage::CreateSpace {
        title: "Refused".to_owned(),
        backend: FAKE_BACKEND_NAME.to_owned(),
        directory: "/tmp".to_owned(),
        session: None,
        environment: None,
        instructions: Some("Mind the docs.".to_owned()),
    });
    // Perhaps already in the first state the creator receives.
    let state = ada.space(|state| {
        state
            .instructions
            .as_ref()
            .is_some_and(|layers| layers.error.is_some())
    });
    let error = state.instructions.unwrap().error.unwrap();
    assert!(error.contains("refused the instructions"), "{error}");
    // Not a problem of the space itself: input still reaches the agent.
    assert_eq!(state.error, None);
    let session = backend.started()[0].clone();
    ada.say("u1", "hello anyway");
    until(&session, "the prompt was never delivered", |session| {
        !session.log().prompts.is_empty()
    });

    // Removing the space's layer alone cannot show that the agent took the rest, so while the warning stands an
    // owner's change gives every layer, starting with Tomori's, which is refused again.
    let tried = session.log().instructions.len();
    ada.send(&ClientMessage::SetInstructions {
        text: String::new(),
    });
    ada.space(|state| {
        state
            .instructions
            .as_ref()
            .is_some_and(|layers| layers.space.is_empty() && layers.error.is_some())
    });
    // The refusal of every layer follows.
    assert!(ada.space(|_| true).instructions.unwrap().error.is_some());
    assert_eq!(
        session.log().instructions[tried..],
        [("tomori".to_owned(), Some(tomori().to_owned()))]
    );

    // Once the agent takes them, after it is read again, the warning clears.
    backend.fail_instructions(false);
    session.emit(Event::Reconnecting {
        reason: "Lost the connection.".to_owned(),
    });
    session.emit(Event::Resynchronized {
        snapshot: Snapshot {
            busy: Some(false),
            ..Snapshot::default()
        },
    });
    ada.space(|state| {
        state
            .instructions
            .as_ref()
            .is_some_and(|layers| layers.error.is_none())
    });
    assert_eq!(session.instructions(), entries(None));
}

#[test]
fn a_backend_without_instructions_is_given_none() {
    let backend = FakeBackend::new();
    backend.take_no_instructions();
    let server = TestServer::with_backend(backend.clone());
    let mut ada = server.connect("ada", "Ada");
    let state = create(&mut ada, "Plain", Some("Mind the docs."));
    assert_eq!(state.instructions, None);
    let session = backend.started()[0].clone();
    ada.send(&ClientMessage::SetInstructions {
        text: "Mind the tests.".to_owned(),
    });
    assert_eq!(ada.error(), "This agent takes no instructions from Tomori.");
    session.emit(Event::Resynchronized {
        snapshot: Snapshot::default(),
    });
    ada.say("u1", "hello");
    until(&session, "the prompt was never delivered", |session| {
        !session.log().prompts.is_empty()
    });
    assert!(session.log().instructions.is_empty());
    assert!(session.instructions().is_empty());
}

#[test]
fn only_owners_change_a_space_s_instructions() {
    let server = TestServer::with_tokens(true, |server| server);
    let (_, token) = server.issue("Ada", Grant::Owner);
    let mut ada = server.connect_with_token("ada", "Ada", &token);
    let state = create(&mut ada, "Owned", Some("Mind the docs."));
    let (_, token) = server.issue(
        "Cal",
        Grant::Collaborator {
            space: state.id.clone(),
        },
    );
    let mut cal = server.connect_with_token("cal", "Cal", &token);
    let mut vic = TestClient::open(server.address);
    vic.hello("vic", "Vic");
    for (client, refusal) in [
        (&mut cal, "Only owners can do that."),
        (&mut vic, "Visitors can watch, but not act in a space."),
    ] {
        client.send(&ClientMessage::Join {
            space: state.id.clone(),
        });
        // Everyone in the space sees the layers.
        client.space(|state| {
            state
                .instructions
                .as_ref()
                .is_some_and(|layers| layers.space == "Mind the docs.")
        });
        client.send(&ClientMessage::SetInstructions {
            text: "Ignore the docs.".to_owned(),
        });
        assert_eq!(client.error(), refusal);
    }
    let session = server.backend.started()[0].clone();
    assert_eq!(session.instructions(), entries(Some("Mind the docs.")));
}

#[test]
fn a_forgotten_space_s_entries_are_removed_unless_another_space_is_bound_to_its_session() {
    let server = TestServer::start();
    let mut ada = server.connect("ada", "Ada");
    let close = |ada: &TestClient, session: &FakeSession, id: &str| {
        session.emit(Event::Closed { reason: None });
        ada.space(|space| space.id == id && space.activity == protocol::Activity::Closed);
    };
    let first = create(&mut ada, "First", Some("Mind the docs.")).id;
    let first_session = server.backend.started()[0].clone();
    until(&first_session, "the layers were never given", |session| {
        session.instructions() == entries(Some("Mind the docs."))
    });
    close(&ada, &first_session, &first);
    // A second closed space is bound again to its session by a third, which is open.
    let second = create(&mut ada, "Second", None).id;
    let second_session = server.backend.started()[1].clone();
    close(&ada, &second_session, &second);
    ada.send(&ClientMessage::CreateSpace {
        title: "Third".to_owned(),
        backend: FAKE_BACKEND_NAME.to_owned(),
        directory: "/tmp".to_owned(),
        session: Some(second_session.id()),
        environment: None,
        instructions: Some("Mind the tests.".to_owned()),
    });
    ada.space(|state| state.title == "Third");
    until(
        &second_session,
        "the third space's layers were never given",
        |session| session.instructions() == entries(Some("Mind the tests.")),
    );
    // Closing more spaces forgets the least recently closed, the first and second.
    for index in 0..server::CLOSED_SPACES {
        let id = create(&mut ada, &format!("Filler {index}"), None).id;
        let session = server.backend.started().last().unwrap().clone();
        close(&ada, &session, &id);
    }
    ada.send(&ClientMessage::Leave);
    ada.wait_for(|message| match message {
        ServerMessage::Spaces { spaces } => spaces
            .iter()
            .all(|space| space.id != first && space.id != second)
            .then_some(()),
        _ => None,
    });
    until(
        &first_session,
        "the first space's entries were never removed",
        |session| session.instructions().is_empty(),
    );
    let forgotten = server.backend.forgotten();
    assert!(forgotten.contains(&(first_session.id(), KEYS.map(str::to_owned).to_vec())));
    // The second's session is the open third space's, which keeps its entries.
    assert!(
        forgotten
            .iter()
            .all(|(session, _)| *session != second_session.id())
    );
    assert_eq!(
        second_session.instructions(),
        entries(Some("Mind the tests."))
    );
}
