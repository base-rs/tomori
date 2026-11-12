// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What the lobby's listing says of each space, so that spaces are told apart at a glance. See `design/arrival.md`.
use std::path::PathBuf;
use std::sync::Arc;

use tomori_protocol::{ClientMessage, ServerMessage, SpaceSummary};
use tomori_security::Grant;
use tomori_server::fake::FakeBackend;
use tomori_server::kept::{SPACES_FILE, SpaceStore};
use tomori_server::test_support::{TestClient, TestServer, TextTranscriber};
use tomori_server::{Server, WorkspaceRoots};

/// A directory removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("tomori-listing-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    fn directory(&self, relative: &str) -> String {
        let path = self.0.join(relative);
        std::fs::create_dir_all(&path).unwrap();
        path.to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn create(client: &mut TestClient, title: &str, directory: &str) -> String {
    client.send(&ClientMessage::CreateSpace {
        title: title.to_owned(),
        backend: "fake".to_owned(),
        directory: directory.to_owned(),
        session: None,
        environment: Some("sky".to_owned()),
        instructions: None,
    });
    client.space(|state| state.title == title).id
}

/// The first listing `client` receives in which `select` finds a space.
fn listed(client: &TestClient, mut select: impl FnMut(&SpaceSummary) -> bool) -> SpaceSummary {
    client.wait_for(|message| match message {
        ServerMessage::Spaces { spaces } => spaces.iter().find(|space| select(space)).cloned(),
        _ => None,
    })
}

#[test]
fn the_listing_tells_spaces_apart_by_folder_environment_activity_and_newest_reply() {
    let scratch = Scratch::new("apart");
    let root = scratch.directory("Developer");
    let project = scratch.directory("Developer/ioquatix/tomori");
    let roots = WorkspaceRoots::new([&root]).unwrap();
    let server = TestServer::with_tokens(true, move |server| server.with_workspace_roots(roots));
    let (_, token) = server.issue("Ada", Grant::Owner);
    let mut ada = server.connect_with_token("ada", "Ada", &token);
    let tomori = create(&mut ada, "Tomori", &project);
    let session = server.backend.started()[0].clone();
    session.reply("## Done\n\nAll 42 tests pass, and the build is green.\n```sh\ncargo test\n```");
    let at_root = create(&mut ada, "Root", &root);

    // A visitor in the lobby reads each space's folder beneath the workspace root, environment, participants, how long
    // ago it was active and the start of its newest reply.
    let vic = server.connect("vic", "Vic");
    let spaces = vic.wait_for(|message| match message {
        ServerMessage::Spaces { spaces }
            if spaces.len() == 2 && spaces.iter().any(|space| space.latest.is_some()) =>
        {
            Some(spaces.clone())
        }
        _ => None,
    });
    let space = spaces.iter().find(|space| space.id == tomori).unwrap();
    assert_eq!(space.folder.as_deref(), Some("ioquatix/tomori"));
    assert_eq!(space.environment, "sky");
    assert!(space.active_age.is_some_and(|age| age < 120), "{space:?}");
    assert_eq!(
        space.latest.as_deref(),
        Some("Done All 42 tests pass, and the build is green.")
    );
    // A space at the root is named by it, and has no reply yet.
    let space = spaces.iter().find(|space| space.id == at_root).unwrap();
    assert_eq!(space.folder.as_deref(), Some("Developer"));
    assert_eq!((space.participants, space.latest.as_deref()), (1, None));

    // That is no more than the visitor reads by joining the space.
    let mut vic = vic;
    let state = vic.join(&tomori);
    assert!(state.messages.last().is_some_and(|message| {
        message
            .text
            .contains("All 42 tests pass, and the build is green.")
    }));
}

#[test]
fn when_a_space_was_last_active_is_kept_across_a_restart() {
    let scratch = Scratch::new("kept");
    let state = scratch.directory("state");
    let backend = FakeBackend::new();
    let start = |state: &str| {
        let store = Arc::new(SpaceStore::open(std::path::Path::new(state)).unwrap());
        TestServer::with_server(backend.clone(), move |backends| {
            Server::new(backends, Arc::new(TextTranscriber::default())).with_kept_spaces(store)
        })
    };
    let id = {
        let server = start(&state);
        let mut ada = server.connect("ada", "Ada");
        ada.create_space("Studio").id
    };
    let text = std::fs::read_to_string(PathBuf::from(&state).join(SPACES_FILE)).unwrap();
    let kept: serde_json::Value = serde_json::from_str(&text).unwrap();
    let active = kept["spaces"][0]["active"].as_u64().unwrap();
    assert_eq!(active % 60, 0, "kept to the minute");

    let server = start(&state);
    let ada = server.connect("ada", "Ada");
    let space = listed(&ada, |space| space.id == id);
    assert!(space.active_age.is_some_and(|age| age < 180), "{space:?}");
}
