// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Spaces kept across server restarts. See `design/spaces.md`.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tomori_backend::{Event, Session};
use tomori_protocol::{ClientMessage, ConnectionRole, Keepalive, ServerMessage, SpaceSummary};
use tomori_security::Grant;
use tomori_server::fake::FakeBackend;
use tomori_server::kept::{KeptSpace, SPACES_FILE, SpaceStore};
use tomori_server::test_support::{TestServer, TextTranscriber, eventually};
use tomori_server::{RESTORING, Server};

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
    tokens: Option<Arc<tomori_security::TokenStore>>,
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
    let deadline = std::time::Instant::now() + tomori_server::test_support::TIMEOUT;
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

fn listed(client: &tomori_server::test_support::TestClient, id: &str) -> SpaceSummary {
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
        ada.space(|space| space.id == ended && space.activity == tomori_protocol::Activity::Closed);
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
    // Listed again with no attention: the closed space closed before the restart, and history read again is not news.
    ada.send(&ClientMessage::ListSpaces);
    ada.wait_for(|message| match message {
        ServerMessage::Spaces { spaces } => spaces
            .iter()
            .all(|space| space.attention.is_none())
            .then_some(()),
        _ => None,
    });
    ada.send(&ClientMessage::Join {
        space: studio.id.clone(),
    });
    // Bound again to the same session, with its settings, and taking input.
    let restored = ada.space(|space| space.id == studio.id && space.error.is_none());
    assert_eq!(restored.title, studio.title);
    assert_eq!(restored.directory, studio.directory);
    assert_eq!(restored.environment, studio.environment);
    assert!(!restored.attribution);
    assert_ne!(restored.activity, tomori_protocol::Activity::Closed);
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
    assert_eq!(summary.activity, tomori_protocol::Activity::Closed);
    // New spaces never reuse a kept identifier.
    let fresh = ada.create_space("Fresh").id;
    assert!(fresh != studio.id && fresh != ended);
}

#[test]
fn a_space_s_changed_environment_is_kept_when_it_changes_and_across_a_restart() {
    let state = State::new("environment");
    let backend = FakeBackend::new();
    let id = {
        let store = Arc::new(SpaceStore::open(&state.0).expect("the spaces open"));
        let kept = store.clone();
        // The periodic sweep and leaving keep the spaces too, so no sweep runs here and Ada stays connected: only the change itself can keep the new environment.
        let never = Duration::from_secs(3600);
        let server = TestServer::with_server(backend.clone(), move |backends| {
            Server::new(backends, Arc::new(TextTranscriber::default()))
                .with_keepalive(Keepalive {
                    interval: never,
                    timeout: never,
                })
                .with_request_timeout(never)
                .with_participant_retention(never)
                .with_kept_spaces(store)
        });
        let mut ada = server.connect("ada", "Ada");
        let created = ada.create_space("Shrine");
        assert_eq!(created.environment, "sky");
        let id = created.id;
        ada.send(&ClientMessage::SetEnvironment {
            environment: "minimal".into(),
        });
        ada.space(|space| space.environment == "minimal");
        let deadline = std::time::Instant::now() + tomori_server::test_support::TIMEOUT;
        while kept.spaces()[0].environment != "minimal" {
            assert!(
                std::time::Instant::now() < deadline,
                "the change was not kept when it was made"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        id
    };
    assert_eq!(state.kept()[0].environment, "minimal");

    let server = start(&backend, &state.0, None);
    let mut ada = server.connect("ada", "Ada");
    ada.send(&ClientMessage::ListSpaces);
    assert_eq!(listed(&ada, &id).environment, "minimal");
    ada.send(&ClientMessage::Join { space: id.clone() });
    let restored = ada.space(|space| space.id == id && space.error.is_none());
    assert_eq!(restored.environment, "minimal");
}

#[test]
fn a_space_s_instructions_are_kept_and_given_again_after_a_restart() {
    let state = State::new("instructions");
    let backend = FakeBackend::new();
    let id = {
        let server = start(&backend, &state.0, None);
        let mut ada = server.connect("ada", "Ada");
        ada.send(&ClientMessage::CreateSpace {
            title: "Docs".into(),
            backend: "fake".into(),
            directory: std::env::temp_dir().to_string_lossy().into_owned(),
            session: None,
            environment: None,
            instructions: Some("Mind the docs.".into()),
        });
        let id = ada.space(|space| space.title == "Docs").id;
        let deadline = std::time::Instant::now() + tomori_server::test_support::TIMEOUT;
        while backend.started()[0].log().instructions.len() < 3 {
            assert!(std::time::Instant::now() < deadline, "never given");
            std::thread::sleep(Duration::from_millis(10));
        }
        id
    };
    let kept = state.kept();
    assert_eq!(kept[0].instructions, "Mind the docs.");
    let session = backend.started()[0].clone();
    let given = session.log().instructions.len();
    assert_eq!(given, 3);

    let server = start(&backend, &state.0, None);
    let mut ada = server.connect("ada", "Ada");
    ada.send(&ClientMessage::Join { space: id.clone() });
    let restored = ada.space(|space| space.id == id && space.error.is_none());
    assert_eq!(
        restored.instructions.map(|layers| layers.space),
        Some("Mind the docs.".to_owned())
    );
    // Bound again, the session is given every layer again.
    let deadline = std::time::Instant::now() + tomori_server::test_support::TIMEOUT;
    while session.log().instructions.len() < given * 2 {
        assert!(std::time::Instant::now() < deadline, "not given again");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        session
            .instructions()
            .get("tomori-space")
            .map(String::as_str),
        Some("Mind the docs.")
    );
}

#[test]
fn a_restored_space_is_given_its_environment_s_layer_whenever_the_environments_are_offered() {
    use tomori_server::Environment;
    const STARS: &str = "The space shows the night sky.";
    const TOPOLOGY: &str = "The space shows the service's topology.";
    let state = State::new("environment-layer");
    let backend = FakeBackend::new();
    // Offered before the kept spaces are listed again, as `Builder::environment` offers them, or after, as a builder's `configure` hook or an embedding server may: Sky with a layer registered in its place, and another project's environment.
    let start = |after: bool| {
        let store = Arc::new(SpaceStore::open(&state.0).expect("the spaces open"));
        TestServer::with_server(backend.clone(), move |backends| {
            let offered = [
                Environment::new("sky", "Sky").with_instructions(STARS),
                Environment::new("minimal", "Grid"),
                Environment::new("observe", "Observe").with_instructions(TOPOLOGY),
            ];
            let server = Server::new(backends, Arc::new(TextTranscriber::default()));
            if after {
                server.with_kept_spaces(store).with_environments(offered)
            } else {
                server.with_environments(offered).with_kept_spaces(store)
            }
        })
    };
    let entry = |layer: &str| ("tomori-environment".to_owned(), Some(layer.to_owned()));
    let spaces = {
        let server = start(false);
        let mut ada = server.connect("ada", "Ada");
        [("Topology", "observe", TOPOLOGY), ("Night", "sky", STARS)]
            .into_iter()
            .enumerate()
            .map(|(index, (title, environment, layer))| {
                ada.send(&ClientMessage::CreateSpace {
                    title: title.into(),
                    backend: "fake".into(),
                    directory: std::env::temp_dir().to_string_lossy().into_owned(),
                    session: None,
                    environment: Some(environment.into()),
                    instructions: None,
                });
                let id = ada.space(|space| space.title == title).id;
                let session = backend.started()[index].clone();
                eventually(|| {
                    session
                        .log()
                        .instructions
                        .contains(&entry(layer))
                        .then_some(())
                });
                (id, session, layer)
            })
            .collect::<Vec<_>>()
    };
    for after in [true, false] {
        let given: Vec<usize> = spaces
            .iter()
            .map(|(_, session, _)| session.log().instructions.len())
            .collect();
        let server = start(after);
        let mut ada = server.connect("ada", "Ada");
        for ((id, session, layer), given) in spaces.iter().zip(given) {
            // Bound again, the agent is given the environment's layer through its instruction entries, as a new space's is.
            eventually(|| (session.log().instructions.len() >= given + 3).then_some(()));
            assert!(
                session.log().instructions[given..].contains(&entry(layer)),
                "{id}, offered after: {after}"
            );
            assert_eq!(
                session
                    .instructions()
                    .get("tomori-environment")
                    .map(String::as_str),
                Some(*layer)
            );
            ada.send(&ClientMessage::Join { space: id.clone() });
            let restored = ada.space(|space| space.id == *id);
            assert_eq!(
                restored.instructions.map(|layers| layers.environment),
                Some((*layer).to_owned())
            );
        }
    }
}

#[test]
fn a_space_whose_session_cannot_be_bound_again_is_listed_closed() {
    let state = State::new("unbound");
    std::fs::create_dir_all(&state.0).unwrap();
    let kept = |id: &str, backend: &str, session: &str| KeptSpace {
        id: id.into(),
        title: "Gone".into(),
        renamed_by: None,
        backend: backend.into(),
        directory: std::env::temp_dir().to_string_lossy().into_owned(),
        session: session.into(),
        environment: "sky".into(),
        attribution: true,
        brief: false,
        movement: None,
        instructions: "Mind the docs.".into(),
        closed: None,
        closed_order: None,
        placements: Default::default(),
        stage: Default::default(),
        places: Default::default(),
        active: None,
        timers: Default::default(),
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
    // Not being bound again after a restart is listed closed, but needs no attention.
    ada.wait_for(|message| match message {
        ServerMessage::Spaces { spaces } => (spaces.len() == 2
            && spaces
                .iter()
                .all(|space| space.activity == tomori_protocol::Activity::Closed))
        .then(|| assert!(spaces.iter().all(|space| space.attention.is_none()))),
        _ => None,
    });
    for (id, reason) in [
        ("space-old-1", "Could not resume the fake session"),
        (
            "space-old-2",
            "The missing backend is not enabled on this server.",
        ),
    ] {
        ada.send(&ClientMessage::Join { space: id.into() });
        let space = ada
            .space(|space| space.id == id && space.activity == tomori_protocol::Activity::Closed);
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
    // Its own instructions are kept for when its backend is enabled again.
    assert_eq!(kept[1].instructions, "Mind the docs.");
}

#[test]
fn only_a_space_chosen_brief_is_brief_after_a_restart() {
    let state = State::new("speech");
    let backend = FakeBackend::new();
    let (brief, full) = {
        let server = start(&backend, &state.0, None);
        let mut ada = server.connect("ada", "Ada");
        let brief = ada.create_space("Brief").id;
        ada.send(&ClientMessage::SetSpeech {
            speech: tomori_protocol::SpeechSetting::Brief,
        });
        ada.space(|space| {
            space.id == brief && space.speech == tomori_protocol::SpeechSetting::Brief
        });
        let full = ada.create_space("Full").id;
        (brief, full)
    };
    let kept = state.kept();
    assert!(kept.iter().find(|space| space.id == brief).unwrap().brief);
    assert!(!kept.iter().find(|space| space.id == full).unwrap().brief);
    // A space speaking in full is kept without the setting, as every space was before it was stored.
    let text = std::fs::read_to_string(state.0.join(SPACES_FILE)).unwrap();
    assert_eq!(text.matches("\"brief\"").count(), 1, "{text}");

    let server = start(&backend, &state.0, None);
    let mut ada = server.connect("ada", "Ada");
    for (id, speech) in [
        (&brief, tomori_protocol::SpeechSetting::Brief),
        (&full, tomori_protocol::SpeechSetting::Full),
    ] {
        ada.send(&ClientMessage::Join { space: id.clone() });
        let restored = ada.space(|space| &space.id == id && space.error.is_none());
        assert_eq!(restored.speech, speech);
        ada.send(&ClientMessage::Leave);
    }
}

#[test]
fn a_space_kept_before_its_speech_was_stored_speaks_in_full() {
    let state = State::new("unkept-speech");
    std::fs::create_dir_all(&state.0).unwrap();
    // A record from before the speech setting was stored, when every space began brief.
    std::fs::write(
        state.0.join(SPACES_FILE),
        serde_json::json!({
            "spaces": [{
                "id": "space-old-1",
                "title": "Old",
                "backend": "fake",
                "directory": std::env::temp_dir().to_string_lossy(),
                "session": "fake-from-before",
                "environment": "sky",
                "attribution": true,
            }]
        })
        .to_string(),
    )
    .unwrap();
    assert!(!SpaceStore::open(&state.0).unwrap().spaces()[0].brief);
    let backend = FakeBackend::new();
    let server = start(&backend, &state.0, None);
    let mut ada = server.connect("ada", "Ada");
    ada.send(&ClientMessage::Join {
        space: "space-old-1".into(),
    });
    let space = ada.space(|space| space.id == "space-old-1");
    assert_eq!(space.speech, tomori_protocol::SpeechSetting::Full);
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
    let tokens = Arc::new(tomori_security::TokenStore::open(&state.0).unwrap());
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
    for index in 0..=tomori_server::CLOSED_SPACES {
        let id = ada.create_space(&format!("Space {index}")).id;
        backend.started()[index].emit(Event::Closed { reason: None });
        ada.space(|space| space.id == id && space.activity == tomori_protocol::Activity::Closed);
        ids.push(id);
    }
    // Leaving the last lets it be forgotten in its turn; the first, least recently closed, is forgotten now.
    ada.send(&ClientMessage::Leave);
    ada.wait_for(|message| match message {
        ServerMessage::Spaces { spaces } => {
            (spaces.len() == tomori_server::CLOSED_SPACES).then_some(())
        }
        _ => None,
    });
    drop(ada);
    drop(server);
    let kept: Vec<String> = state.kept().into_iter().map(|space| space.id).collect();
    assert_eq!(kept.len(), tomori_server::CLOSED_SPACES);
    assert!(!kept.contains(&ids[0]));
    assert!(kept.contains(&ids[tomori_server::CLOSED_SPACES]));
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
        let space = ada.space(|space| space.activity == tomori_protocol::Activity::Closed);
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
        ada.space(|space| {
            space.id == second && space.activity == tomori_protocol::Activity::Closed
        });
        ada.send(&ClientMessage::Leave);
        listed(&ada, &second);
        backend.started()[0].emit(Event::Closed { reason: None });
        ada.wait_for(|message| match message {
            ServerMessage::Spaces { spaces } => spaces
                .iter()
                .any(|space| {
                    space.id == first && space.activity == tomori_protocol::Activity::Closed
                })
                .then_some(()),
            _ => None,
        });
        (first, second)
    };
    let server = start(&backend, &state.0, None);
    let mut ada = server.connect("ada", "Ada");
    for index in 1..tomori_server::CLOSED_SPACES {
        let id = ada.create_space(&format!("Space {index}")).id;
        backend
            .started()
            .last()
            .unwrap()
            .emit(Event::Closed { reason: None });
        ada.space(|space| space.id == id && space.activity == tomori_protocol::Activity::Closed);
    }
    ada.send(&ClientMessage::Leave);
    let spaces = ada.wait_for(|message| match message {
        ServerMessage::Spaces { spaces } if spaces.len() == tomori_server::CLOSED_SPACES => {
            Some(spaces.clone())
        }
        _ => None,
    });
    assert!(spaces.iter().any(|space| space.id == first));
    assert!(!spaces.iter().any(|space| space.id == second));
}

#[test]
fn a_deleted_space_is_kept_no_more_and_stays_deleted_after_a_restart() {
    let state = State::new("deleted");
    let backend = FakeBackend::new();
    let (kept, deleted, gone) = {
        let server = start(&backend, &state.0, None);
        let mut ada = server.connect("ada", "Ada");
        let kept = ada.create_space("Kept").id;
        let deleted = ada.create_space("Deleted").id;
        ada.send(&ClientMessage::DeleteSpace {
            space: deleted.clone(),
            session: false,
        });
        ada.wait_for(|message| matches!(message, ServerMessage::Left { .. }).then_some(()));
        // Kept no more once everyone in it has been told.
        eventually(|| (state.kept().len() == 1).then_some(()));
        // A space deleted with its session is not kept once the session is deleted.
        let gone = ada.create_space("Gone").id;
        ada.send(&ClientMessage::DeleteSpace {
            space: gone.clone(),
            session: true,
        });
        ada.wait_for(|message| {
            matches!(message, ServerMessage::SessionDeleted { error: None }).then_some(())
        });
        assert_eq!(backend.deleted().len(), 1);
        (kept, deleted, gone)
    };
    assert_eq!(
        state
            .kept()
            .iter()
            .map(|space| space.id.as_str())
            .collect::<Vec<_>>(),
        [kept.as_str()]
    );
    let server = start(&backend, &state.0, None);
    let mut ada = server.connect("ada", "Ada");
    ada.send(&ClientMessage::ListSpaces);
    assert_eq!(listed(&ada, &kept).id, kept);
    for id in [deleted, gone] {
        ada.send(&ClientMessage::Join { space: id });
        assert_eq!(ada.error(), "That space does not exist.");
    }
}
