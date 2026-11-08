// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A space's title: following its agent session's, renamed by an owner, typed or said, and kept across a restart. See
//! `design/spaces.md`.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tomori_backend::Event;
use tomori_protocol::{Activity, ClientMessage, Gate, Renamer, ServerMessage, SpaceState};
use tomori_security::Grant;
use tomori_server::Server;
use tomori_server::fake::{FakeBackend, FakeSession};
use tomori_server::kept::{SPACES_FILE, SpaceStore};
use tomori_server::test_support::{TestClient, TestServer, TextTranscriber, eventually, speech};

/// Wait until `client` lists the space `id` titled `title`.
fn listed(client: &TestClient, id: &str, title: &str) {
    client.wait_for(|message| {
        matches!(message, ServerMessage::Spaces { spaces } if spaces
            .iter()
            .any(|space| space.id == id && space.title == title))
        .then_some(())
    });
}

/// Let `session` report something else, and wait for `client` to see it, so that what the session reported before has
/// been applied.
fn settle(client: &TestClient, session: &FakeSession) -> SpaceState {
    session.emit(Event::Busy);
    let state = client.space(|state| state.activity == Activity::Running);
    session.emit(Event::Idle);
    client.space(|state| state.activity == Activity::Idle);
    state
}

fn rename(client: &mut TestClient, title: &str) {
    client.send(&ClientMessage::RenameSpace {
        title: title.to_owned(),
    });
}

/// Say `text` as the space's new title.
fn say_title(client: &mut TestClient, utterance: &str, text: &str) {
    client.send(&ClientMessage::UtteranceBegin {
        utterance: utterance.to_owned(),
        answer: None,
        progress: false,
        naming: false,
        titling: true,
        attention: Vec::new(),
    });
    client.send_audio(Gate::Agent, speech(text));
    client.send(&ClientMessage::UtteranceEnd {
        utterance: utterance.to_owned(),
        pointed: None,
    });
}

#[test]
fn a_space_follows_its_session_s_title_until_an_owner_renames_it_and_again_once_cleared() {
    let server = TestServer::with_tokens(true, |server| server);
    let (_, token) = server.issue("Ada", Grant::Owner);
    let mut ada = server.connect_with_token("ada", "Ada", &token);
    let studio = ada.create_space("Studio");
    let session = server.backend.started()[0].clone();
    assert!(studio.renames);
    assert_eq!(studio.renamed_by, None);
    let (_, token) = server.issue(
        "Bob",
        Grant::Collaborator {
            space: studio.id.clone(),
        },
    );
    let mut bob = server.connect_with_token("bob", "Bob", &token);
    bob.send(&ClientMessage::Join {
        space: studio.id.clone(),
    });
    bob.space(|state| state.id == studio.id);
    let mut vic = server.connect("vic", "Vic");
    vic.send(&ClientMessage::Join {
        space: studio.id.clone(),
    });
    vic.space(|state| state.id == studio.id);
    let lobby = server.connect("cat", "Cat");

    // Renamed in the agent's own interface, the session gives the space its new title, for everyone in it and in the
    // lobby, cleaned onto one line.
    session.retitle("Tomori development\nfrom the headset");
    for client in [&ada, &bob, &vic] {
        let state = client.space(|state| state.title == "Tomori development from the headset");
        assert_eq!(state.renamed_by, None);
    }
    listed(&lobby, &studio.id, "Tomori development from the headset");
    // A title with nothing in it once cleaned is not taken.
    session.retitle(" \u{200b} ");
    assert_eq!(
        settle(&ada, &session).title,
        "Tomori development from the headset"
    );

    // Only an owner renames it; a collaborator and a visitor are refused, and nothing changes.
    rename(&mut bob, "Bob's space");
    assert_eq!(bob.error(), "Only owners can do that.");
    rename(&mut vic, "Vic's space");
    assert_eq!(vic.error(), "Visitors can watch, but not act in a space.");
    // A title too long once cleaned is refused, saying why.
    rename(&mut ada, &format!("  {}  ", "t".repeat(201)));
    assert_eq!(
        ada.error(),
        "Title not changed: That title is too long; the limit is 200 bytes."
    );

    // Renamed by an owner, it says who renamed it, and no longer follows its session.
    rename(&mut ada, "  Release\tplanning ");
    let renamed = Some(Renamer::Participant { name: "Ada".into() });
    for client in [&ada, &bob, &vic] {
        let state = client.space(|state| state.title == "Release planning");
        assert_eq!(state.renamed_by, renamed);
    }
    listed(&lobby, &studio.id, "Release planning");
    session.retitle("Renamed in OpenCode");
    let state = settle(&ada, &session);
    assert_eq!(state.title, "Release planning");
    assert_eq!(state.renamed_by, renamed);

    // Renamed to nothing, it follows its session again, taking the title it has now.
    rename(&mut ada, " ");
    let state = bob.space(|state| state.title == "Renamed in OpenCode");
    assert_eq!(state.renamed_by, None);
    session.retitle("Later");
    bob.space(|state| state.title == "Later");

    // Outside a space there is nothing to rename.
    ada.send(&ClientMessage::Leave);
    ada.wait_for(|message| matches!(message, ServerMessage::Left { .. }).then_some(()));
    rename(&mut ada, "Nowhere");
    assert_eq!(ada.error(), "Join a space first.");
}

#[test]
fn an_owner_says_a_space_s_new_title_and_nobody_else_can() {
    let server = TestServer::with_tokens(true, |server| server);
    let (_, token) = server.issue("Ada", Grant::Owner);
    let mut ada = server.connect_with_token("ada", "Ada", &token);
    let studio = ada.create_space("Studio");
    let session = server.backend.started()[0].clone();
    let (_, token) = server.issue(
        "Bob",
        Grant::Collaborator {
            space: studio.id.clone(),
        },
    );
    let mut bob = server.connect_with_token("bob", "Bob", &token);
    bob.send(&ClientMessage::Join {
        space: studio.id.clone(),
    });
    bob.space(|state| state.id == studio.id);

    // The owner's words are the title, without their closing full stop, and are never queued for the agent.
    say_title(&mut ada, "t1", "Release planning.");
    let taken = ada.wait_for(|message| match message {
        ServerMessage::Transcript {
            utterance,
            text,
            finished: true,
        } if utterance == "t1" => Some(text.clone()),
        _ => None,
    });
    assert_eq!(taken, "Release planning");
    let state = bob.space(|state| state.title == "Release planning");
    assert_eq!(
        state.renamed_by,
        Some(Renamer::Participant { name: "Ada".into() })
    );
    assert!(state.queue.is_empty());
    assert!(session.log().prompts.is_empty());

    // Words with nothing but punctuation in them rename nothing.
    say_title(&mut ada, "t2", "...");
    let failed = ada.wait_for(|message| match message {
        ServerMessage::TranscriptFailed { utterance, error } if utterance == "t2" => {
            Some(error.clone())
        }
        _ => None,
    });
    assert_eq!(failed, "No title was heard.");

    // A collaborator cannot say one; their words are refused before they begin.
    say_title(&mut bob, "t3", "Bob's space");
    assert_eq!(bob.error(), "Only owners can do that.");
    let state = settle(&ada, &session);
    assert_eq!(state.title, "Release planning");
    assert!(session.log().prompts.is_empty());
}

/// A state directory of its own, removed when dropped.
struct State(PathBuf);

impl Drop for State {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn start(backend: &FakeBackend, state: &Path) -> TestServer {
    let store = Arc::new(SpaceStore::open(state).expect("the spaces open"));
    TestServer::with_server(backend.clone(), move |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default())).with_kept_spaces(store)
    })
}

#[test]
fn a_renamed_title_is_kept_across_a_restart_while_one_that_follows_takes_its_session_s() {
    let state =
        State(std::env::temp_dir().join(format!("tomori-test-titles-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&state.0);
    let file = state.0.join(SPACES_FILE);
    let backend = FakeBackend::new();
    let (renamed, following) = {
        let server = start(&backend, &state.0);
        let mut ada = server.connect("ada", "Ada");
        let renamed = ada.create_space("Renamed").id;
        rename(&mut ada, "Release planning");
        ada.space(|state| state.title == "Release planning");
        let following = ada.create_space("Following").id;
        (renamed, following)
    };
    // Who renamed it is kept with it; a space that follows its session keeps nothing of that.
    let text = std::fs::read_to_string(&file).unwrap();
    let kept: serde_json::Value = serde_json::from_str(&text).unwrap();
    let spaces = kept["spaces"].as_array().unwrap();
    assert_eq!(
        spaces[0]["renamed_by"],
        serde_json::json!({"kind": "participant", "name": "Ada"})
    );
    assert!(spaces[1].get("renamed_by").is_none());

    // Both sessions are renamed in the agent's own interface while the server is stopped.
    for session in backend.started() {
        session.retitle("Renamed meanwhile");
    }
    let server = start(&backend, &state.0);
    let mut ada = server.connect("ada", "Ada");
    ada.send(&ClientMessage::Join {
        space: following.clone(),
    });
    let shown = ada.space(|shown| shown.id == following && shown.title == "Renamed meanwhile");
    assert_eq!(shown.renamed_by, None);
    ada.send(&ClientMessage::Join {
        space: renamed.clone(),
    });
    let shown = ada.space(|shown| shown.id == renamed && shown.activity == Activity::Idle);
    assert_eq!(shown.title, "Release planning");
    assert_eq!(
        shown.renamed_by,
        Some(Renamer::Participant { name: "Ada".into() })
    );
    // The title the following space took once bound again is kept in turn.
    eventually(|| {
        let text = std::fs::read_to_string(&file).unwrap();
        text.contains("Renamed meanwhile").then_some(())
    });
    drop(server);
}
