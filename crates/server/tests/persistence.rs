// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Spaces kept across server restarts. See `design/spaces.md`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use backend::{Event, Session};
use protocol::{ClientMessage, ConnectionRole, ServerMessage, SpaceSummary};
use security::Grant;
use server::fake::FakeBackend;
use server::kept::{KeptSpace, SPACES_FILE, SpaceStore};
use server::test_support::{TestServer, TextTranscriber};
use server::{RESTORING, Server};

/// A state directory of its own, removed when dropped.
struct State(PathBuf);

impl State {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("tomori-test-kept-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }

    fn kept(&self) -> Vec<KeptSpace> {
        let text = std::fs::read_to_string(self.0.join(SPACES_FILE)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        serde_json::from_value(value["spaces"].clone()).unwrap()
    }
}

impl Drop for State {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A server over `backend` keeping its spaces in `state`, and admitting by `tokens` if given.
fn start(
    backend: &FakeBackend,
    state: &Path,
    tokens: Option<Arc<security::TokenStore>>,
) -> TestServer {
    let store = Arc::new(SpaceStore::open(state).expect("the spaces open"));
    let issuing = tokens.clone();
    let mut server = TestServer::with_server(backend.clone(), move |backends| {
        let server = Server::new(backends, Arc::new(TextTranscriber::default()));
        let server = match tokens {
            Some(tokens) => server
                .with_tokens(tokens, true)
                .with_tokens_over_plain_http_for_tests(),
            None => server,
        };
        server.with_kept_spaces(store)
    });
    server.tokens = issuing;
    server
}

/// Wait until the latest session was sent `text`.
fn delivered(backend: &FakeBackend, text: &str) {
    let deadline = std::time::Instant::now() + server::test_support::TIMEOUT;
    while !backend
        .started()
        .last()
        .unwrap()
        .log()
        .prompts
        .iter()
        .any(|prompt| prompt.text.contains(text))
    {
        assert!(
            std::time::Instant::now() < deadline,
            "{text:?} was not delivered"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn listed(client: &server::test_support::TestClient, id: &str) -> SpaceSummary {
    client.wait_for(|message| match message {
        ServerMessage::Spaces { spaces } => spaces.iter().find(|space| space.id == id).cloned(),
        _ => None,
    })
}

#[test]
fn spaces_survive_a_restart() {
    let state = State::new("survive");
    let backend = FakeBackend::new();
    let (studio, ended) = {
        let server = start(&backend, &state.0, None);
        let mut ada = server.connect("ada", "Ada");
        let ended = ada.create_space("Ended").id;
        backend.started()[0].emit(Event::Closed {
            reason: Some("The agent finished.".into()),
        });
        ada.space(|space| space.id == ended && space.activity == protocol::Activity::Closed);
        let studio = ada.create_space("Studio");
        ada.send(&ClientMessage::SetAttribution { enabled: false });
        ada.space(|space| space.id == studio.id && !space.attribution);
        (studio, ended)
    };
    // Closing the sessions as the server stopped did not close the space.
    let kept = state.kept();
    assert_eq!(kept.len(), 2);
    let kept_studio = kept.iter().find(|space| space.id == studio.id).unwrap();
    assert_eq!(kept_studio.closed, None);
    assert!(!kept_studio.attribution);
    assert_eq!(
        kept.iter()
            .find(|space| space.id == ended)
            .unwrap()
            .closed
            .as_deref(),
        Some("The agent finished.")
    );

    let server = start(&backend, &state.0, None);
    let mut ada = server.connect("ada", "Ada");
    ada.send(&ClientMessage::ListSpaces);
    assert_eq!(listed(&ada, &studio.id).title, "Studio");
    ada.send(&ClientMessage::Join {
        space: studio.id.clone(),
    });
    // Bound again to the same session, with its settings, and taking input.
    let restored = ada.space(|space| space.id == studio.id && space.error.is_none());
    assert_eq!(restored.title, studio.title);
    assert_eq!(restored.directory, studio.directory);
    assert_eq!(restored.environment, studio.environment);
    assert!(!restored.attribution);
    assert_ne!(restored.activity, protocol::Activity::Closed);
    assert!(restored.queue.is_empty());
    let session = backend.started().last().unwrap().id();
    assert_eq!(
        session, kept_studio.session,
        "the space is bound to its own session again"
    );
    ada.say("u1", "hello again");
    delivered(&backend, "hello again");
    // The closed space is listed closed, with its reason, and is not bound again.
    ada.send(&ClientMessage::Leave);
    let summary = listed(&ada, &ended);
    assert_eq!(summary.activity, protocol::Activity::Closed);
    // New spaces never reuse a kept identifier.
    let fresh = ada.create_space("Fresh").id;
    assert!(fresh != studio.id && fresh != ended);
}

#[test]
fn a_space_whose_session_cannot_be_bound_again_is_listed_closed() {
    let state = State::new("unbound");
    std::fs::create_dir_all(&state.0).unwrap();
    let kept = |id: &str, backend: &str, session: &str| KeptSpace {
        id: id.into(),
        title: "Gone".into(),
        backend: backend.into(),
        directory: std::env::temp_dir().to_string_lossy().into_owned(),
        session: session.into(),
        environment: "sky".into(),
        attribution: true,
        closed: None,
        closed_order: None,
    };
    std::fs::write(
        state.0.join(SPACES_FILE),
        serde_json::to_string(&serde_json::json!({
            "spaces": [
                kept("space-old-1", "fake", "fake-from-before"),
                kept("space-old-2", "missing", "elsewhere"),
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    let backend = FakeBackend::new();
    let server = start(&backend, &state.0, None);
    let mut ada = server.connect("ada", "Ada");
    for (id, reason) in [
        ("space-old-1", "Could not resume the fake session"),
        (
            "space-old-2",
            "The missing backend is not enabled on this server.",
        ),
    ] {
        ada.send(&ClientMessage::Join { space: id.into() });
        let space =
            ada.space(|space| space.id == id && space.activity == protocol::Activity::Closed);
        assert!(
            space.error.as_deref().unwrap().starts_with(reason),
            "{:?}",
            space.error
        );
        ada.send(&ClientMessage::Leave);
    }
    // A session that no longer exists is kept as closed, so that it is not tried again; a backend that is not enabled may be next time.
    drop(server);
    let kept = state.kept();
    assert!(kept[0].closed.is_some());
    assert_eq!(kept[1].closed, None);
}

#[test]
fn a_restored_space_is_listed_while_it_is_bound_again() {
    let state = State::new("waiting");
    let backend = FakeBackend::new();
    let studio = {
        let server = start(&backend, &state.0, None);
        server.connect("ada", "Ada").create_space("Studio").id
    };
    backend.hold_creation();
    let server = start(&backend, &state.0, None);
    let mut ada = server.connect("ada", "Ada");
    ada.send(&ClientMessage::Join {
        space: studio.clone(),
    });
    let waiting = ada.space(|space| space.id == studio);
    assert_eq!(waiting.error.as_deref(), Some(RESTORING));
    // Input waits until the space is bound again.
    ada.say("u1", "are you there");
    ada.space(|space| !space.queue.is_empty());
    backend.release_creation();
    ada.space(|space| space.error.is_none());
    delivered(&backend, "are you there");
}

#[test]
fn a_collaborator_admitted_to_a_kept_space_stays_admitted() {
    let state = State::new("collaborator");
    let tokens = Arc::new(security::TokenStore::open(&state.0).unwrap());
    let backend = FakeBackend::new();
    let (studio, token) = {
        let server = start(&backend, &state.0, Some(tokens.clone()));
        let (_, owner) = server.issue("Ada", Grant::Owner);
        let studio = server
            .connect_with_token("ada", "Ada", &owner)
            .create_space("Studio")
            .id;
        let (_, token) = server.issue(
            "Bob",
            Grant::Collaborator {
                space: studio.clone(),
            },
        );
        (studio, token)
    };
    // While the space is still being bound again, past the first sweeps, which revoke collaborator tokens whose space is gone.
    backend.hold_creation();
    let server = start(&backend, &state.0, Some(tokens.clone()));
    std::thread::sleep(Duration::from_millis(2700));
    backend.release_creation();
    assert!(tokens.entries().iter().any(|entry| entry.grant
        == Grant::Collaborator {
            space: studio.clone()
        }));
    let mut bob = server.connect_with_token("bob", "Bob", &token);
    bob.send(&ClientMessage::Join {
        space: studio.clone(),
    });
    let role = bob.wait_for(|message| match message {
        ServerMessage::Role { role } => Some(*role),
        _ => None,
    });
    assert_eq!(role, ConnectionRole::Collaborator);
}

#[test]
fn a_forgotten_closed_space_is_not_kept() {
    let state = State::new("forgotten");
    let backend = FakeBackend::new();
    let server = start(&backend, &state.0, None);
    let mut ada = server.connect("ada", "Ada");
    let mut ids = Vec::new();
    for index in 0..=server::CLOSED_SPACES {
        let id = ada.create_space(&format!("Space {index}")).id;
        backend.started()[index].emit(Event::Closed { reason: None });
        ada.space(|space| space.id == id && space.activity == protocol::Activity::Closed);
        ids.push(id);
    }
    // Leaving the last lets it be forgotten in its turn; the first, least recently closed, is forgotten now.
    ada.send(&ClientMessage::Leave);
    ada.wait_for(|message| match message {
        ServerMessage::Spaces { spaces } => (spaces.len() == server::CLOSED_SPACES).then_some(()),
        _ => None,
    });
    drop(ada);
    drop(server);
    let kept: Vec<String> = state.kept().into_iter().map(|space| space.id).collect();
    assert_eq!(kept.len(), server::CLOSED_SPACES);
    assert!(!kept.contains(&ids[0]));
    assert!(kept.contains(&ids[server::CLOSED_SPACES]));
}

#[test]
fn a_space_not_bound_again_for_a_passing_reason_is_tried_at_the_next_start() {
    let state = State::new("retried");
    let backend = FakeBackend::new();
    let studio = {
        let server = start(&backend, &state.0, None);
        server.connect("ada", "Ada").create_space("Studio").id
    };
    backend.fail_snapshots(true);
    {
        let server = start(&backend, &state.0, None);
        let mut ada = server.connect("ada", "Ada");
        ada.send(&ClientMessage::Join {
            space: studio.clone(),
        });
        let space = ada.space(|space| space.activity == protocol::Activity::Closed);
        assert!(
            space
                .error
                .unwrap()
                .starts_with("Could not read the fake session again")
        );
    }
    assert_eq!(state.kept()[0].closed, None);
    backend.fail_snapshots(false);
    let server = start(&backend, &state.0, None);
    let mut ada = server.connect("ada", "Ada");
    ada.send(&ClientMessage::Join {
        space: studio.clone(),
    });
    ada.space(|space| space.id == studio && space.error.is_none());
}

#[test]
fn the_least_recently_closed_space_is_still_forgotten_first_after_a_restart() {
    let state = State::new("order");
    let backend = FakeBackend::new();
    let (first, second) = {
        let server = start(&backend, &state.0, None);
        let mut ada = server.connect("ada", "Ada");
        let first = ada.create_space("First").id;
        let second = ada.create_space("Second").id;
        // The second space, which Ada is in, closes first.
        backend.started()[1].emit(Event::Closed { reason: None });
        ada.space(|space| space.id == second && space.activity == protocol::Activity::Closed);
        ada.send(&ClientMessage::Leave);
        listed(&ada, &second);
        backend.started()[0].emit(Event::Closed { reason: None });
        ada.wait_for(|message| match message {
            ServerMessage::Spaces { spaces } => spaces
                .iter()
                .any(|space| space.id == first && space.activity == protocol::Activity::Closed)
                .then_some(()),
            _ => None,
        });
        (first, second)
    };
    let server = start(&backend, &state.0, None);
    let mut ada = server.connect("ada", "Ada");
    for index in 1..server::CLOSED_SPACES {
        let id = ada.create_space(&format!("Space {index}")).id;
        backend
            .started()
            .last()
            .unwrap()
            .emit(Event::Closed { reason: None });
        ada.space(|space| space.id == id && space.activity == protocol::Activity::Closed);
    }
    ada.send(&ClientMessage::Leave);
    let spaces = ada.wait_for(|message| match message {
        ServerMessage::Spaces { spaces } if spaces.len() == server::CLOSED_SPACES => {
            Some(spaces.clone())
        }
        _ => None,
    });
    assert!(spaces.iter().any(|space| space.id == first));
    assert!(!spaces.iter().any(|space| space.id == second));
}
