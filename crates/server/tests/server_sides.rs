// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The server sides of environments: when they run, what they are given, and how their failures are contained. See `design/environments.md`.
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tomori_backend::Event;
use tomori_environment::{ServerSide, Space};
use tomori_protocol::{ClientMessage, SpaceState};
use tomori_server::fake::FakeBackend;
use tomori_server::kept::SpaceStore;
use tomori_server::test_support::{TIMEOUT, TestClient, TestServer, TextTranscriber, eventually};
use tomori_server::{Environment, Server};

/// What a recording side saw.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Seen {
    Started {
        id: String,
        title: String,
        directory: String,
    },
    Stopped {
        id: String,
    },
}

/// How a recording side ends.
#[derive(Clone, Copy)]
enum Ending {
    /// When it is cancelled, returning `Ok`.
    Cancelled,
    /// At once, with an error.
    Error,
    /// At once, by panicking.
    Panic,
    /// Never, ignoring its cancellation.
    Ignoring,
}

#[derive(Clone)]
struct Recorder {
    seen: Arc<Mutex<Vec<Seen>>>,
    ending: Ending,
}

impl Recorder {
    fn new(ending: Ending) -> Self {
        Self {
            seen: Arc::default(),
            ending,
        }
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    /// Wait until `ready` holds of what was seen.
    fn until(&self, what: &str, ready: impl Fn(&[Seen]) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        while !ready(&self.seen()) {
            assert!(Instant::now() < deadline, "{what}: {:?}", self.seen());
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Wait until it was started `count` times.
    fn started(&self, count: usize) {
        self.until(&format!("not started {count} times"), |seen| {
            seen.iter()
                .filter(|seen| matches!(seen, Seen::Started { .. }))
                .count()
                == count
        });
    }

    /// Wait until it was stopped `count` times.
    fn stopped(&self, count: usize) {
        self.until(&format!("not stopped {count} times"), |seen| {
            seen.iter()
                .filter(|seen| matches!(seen, Seen::Stopped { .. }))
                .count()
                == count
        });
    }
}

impl ServerSide for Recorder {
    fn run(&self, space: &Space) -> tomori_environment::Result<()> {
        self.seen.lock().unwrap().push(Seen::Started {
            id: space.id().to_owned(),
            title: space.title().to_owned(),
            directory: space.directory().to_owned(),
        });
        match self.ending {
            Ending::Error => return Err("the fleet is unreachable".into()),
            Ending::Panic => panic!("the fleet is unreachable"),
            Ending::Ignoring => loop {
                std::thread::sleep(Duration::from_secs(3600));
            },
            Ending::Cancelled => {}
        }
        while !space.cancellation().is_cancelled() {
            std::thread::sleep(Duration::from_millis(5));
        }
        self.seen.lock().unwrap().push(Seen::Stopped {
            id: space.id().to_owned(),
        });
        Ok(())
    }
}

/// The environments Tomori offers, and Observe, running `side`.
fn offered(side: &Recorder) -> Vec<Environment> {
    vec![
        Environment::new("sky", "Sky"),
        Environment::new("minimal", "Grid"),
        Environment::new("plain", "Plain").with_instructions("The space shows a plain thing."),
        Environment::new("observe", "Observe")
            .with_instructions("The space shows the fleet.")
            .with_server_side(side.clone()),
    ]
}

fn server(side: &Recorder) -> TestServer {
    let environments = offered(side);
    TestServer::with_server(FakeBackend::new(), move |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default()))
            .with_environments(environments)
            .with_drain_timeout(Duration::from_millis(300))
    })
}

/// The environment's layer of the agent's instructions, as the space shows it.
fn layer(space: &SpaceState) -> &str {
    &space
        .instructions
        .as_ref()
        .expect("the layers are shown")
        .environment
}

/// The canonical form of `/tmp`, which spaces run in.
fn tmp() -> String {
    std::fs::canonicalize("/tmp")
        .expect("the temporary directory exists")
        .to_string_lossy()
        .into_owned()
}

fn create(client: &mut TestClient, title: &str, environment: &str) -> SpaceState {
    client.send(&ClientMessage::CreateSpace {
        title: title.to_owned(),
        backend: "fake".to_owned(),
        directory: "/tmp".to_owned(),
        session: None,
        environment: Some(environment.to_owned()),
        instructions: None,
    });
    let title = title.to_owned();
    client.space(|state| state.title == title)
}

fn change(client: &mut TestClient, environment: &str) {
    client.send(&ClientMessage::SetEnvironment {
        environment: environment.to_owned(),
    });
    let environment = environment.to_owned();
    client.space(|state| state.environment == environment);
}

#[test]
fn a_server_side_runs_for_a_space_bound_to_its_environment_with_that_space() {
    let side = Recorder::new(Ending::Cancelled);
    let server = server(&side);
    let mut ada = server.connect("ada", "Ada");

    // A space in Sky or the grid, which have no server side, starts nothing.
    create(&mut ada, "Sky", "sky");
    create(&mut ada, "Grid", "minimal");
    let plain = create(&mut ada, "Plain", "plain");
    assert_eq!(layer(&plain), "The space shows a plain thing.");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(side.seen(), []);

    // A space created in its environment starts it once, with the space's identity.
    let space = create(&mut ada, "Fleet", "observe");
    side.started(1);
    // The environment's instructions are the same, with a side or without one.
    assert_eq!(layer(&space), "The space shows the fleet.");
    assert_eq!(
        side.seen(),
        [Seen::Started {
            id: space.id.clone(),
            title: "Fleet".to_owned(),
            directory: tmp(),
        }]
    );

    // Events that change nothing about the binding do not start it again.
    ada.say("u1", "hello");
    ada.space(|state| {
        state
            .messages
            .iter()
            .any(|message| message.text.contains("hello"))
    });
    assert_eq!(side.seen().len(), 1);

    // Leaving the environment cancels it, once.
    change(&mut ada, "minimal");
    side.stopped(1);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(side.seen().len(), 2);
    // And coming back starts it again, for the new binding.
    change(&mut ada, "observe");
    side.started(2);
    assert_eq!(side.seen().len(), 3);
}

#[test]
fn a_server_side_stops_when_the_space_s_session_closes_and_when_the_server_stops() {
    let side = Recorder::new(Ending::Cancelled);
    let server = server(&side);
    let mut ada = server.connect("ada", "Ada");
    create(&mut ada, "First", "observe");
    side.started(1);
    server.backend.started()[0].emit(Event::Closed {
        reason: Some("done".to_owned()),
    });
    side.stopped(1);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(side.seen().len(), 2);

    // A space bound to its environment when the server stops has its side cancelled, which the server waits for.
    create(&mut ada, "Second", "observe");
    side.started(2);
    drop(ada);
    drop(server);
    // The server waited for it: by the time it stopped, the side had.
    assert_eq!(
        side.seen()
            .iter()
            .filter(|seen| matches!(seen, Seen::Stopped { .. }))
            .count(),
        2
    );
}

#[test]
fn a_server_side_that_fails_leaves_its_space_working_and_is_not_started_again() {
    for (ending, shown) in [
        (
            Ending::Error,
            "The Observe environment failed: the fleet is unreachable",
        ),
        (
            Ending::Panic,
            "The Observe environment stopped unexpectedly.",
        ),
    ] {
        let side = Recorder::new(ending);
        let server = server(&side);
        let mut ada = server.connect("ada", "Ada");
        let other = create(&mut ada, "Other", "sky");
        let space = create(&mut ada, "Fleet", "observe");
        side.started(1);

        // The space shows the error, and keeps its environment and its conversation.
        let failed = if space.error.is_some() {
            space.clone()
        } else {
            ada.space(|state| state.id == space.id && state.error.is_some())
        };
        assert_eq!(failed.error.as_deref(), Some(shown));
        assert_eq!(failed.environment, "observe");
        assert_eq!(layer(&failed), "The space shows the fleet.");
        ada.say("u1", "still there?");
        eventually(|| {
            server
                .backend
                .started()
                .iter()
                .any(|session| !session.log().prompts.is_empty())
                .then_some(())
        });

        // Another space, in Sky, is unaffected, and the side is not started again for this binding.
        assert_eq!(other.error, None);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(side.seen().len(), 1);

        // Leaving the environment ends the error, which belonged to it.
        change(&mut ada, "minimal");
        let recovered = ada.space(|state| state.id == space.id && state.error.is_none());
        assert_eq!(recovered.environment, "minimal");
    }
}

#[test]
fn a_server_side_that_ignores_its_cancellation_does_not_stop_the_server_stopping() {
    let side = Recorder::new(Ending::Ignoring);
    let server = server(&side);
    let mut ada = server.connect("ada", "Ada");
    create(&mut ada, "Fleet", "observe");
    side.started(1);
    drop(ada);
    let stopping = Instant::now();
    drop(server);
    // The server waited for it as long as its drain timeout, 300 ms, and then stopped regardless.
    let waited = stopping.elapsed();
    assert!(waited >= Duration::from_millis(300), "{waited:?}");
    assert!(waited < Duration::from_secs(3), "{waited:?}");
}

/// A state directory of its own, removed when dropped.
struct State(PathBuf);

impl State {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("tomori-test-sides-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for State {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A server that keeps its spaces in `state`, offering the environments before or after listing the kept spaces.
fn keeping(
    side: &Recorder,
    backend: &FakeBackend,
    state: &State,
    environments_first: bool,
) -> TestServer {
    let environments = offered(side);
    let store = Arc::new(SpaceStore::open(&state.0).expect("the spaces open"));
    TestServer::with_server(backend.clone(), move |backends| {
        let server = Server::new(backends, Arc::new(TextTranscriber::default()));
        if environments_first {
            server
                .with_environments(environments)
                .with_kept_spaces(store)
        } else {
            server
                .with_kept_spaces(store)
                .with_environments(environments)
        }
    })
}

#[test]
fn a_restored_space_runs_its_environment_s_server_side_whichever_is_offered_first() {
    for environments_first in [true, false] {
        let state = State::new(if environments_first { "first" } else { "last" });
        let backend = FakeBackend::new();
        let id = {
            let side = Recorder::new(Ending::Cancelled);
            let server = keeping(&side, &backend, &state, environments_first);
            let mut ada = server.connect("ada", "Ada");
            let id = create(&mut ada, "Fleet", "observe").id;
            side.started(1);
            id
        };

        // After a restart the space is restored in its environment, and its side runs for it as for a new space.
        let side = Recorder::new(Ending::Cancelled);
        let server = keeping(&side, &backend, &state, environments_first);
        side.started(1);
        assert_eq!(
            side.seen(),
            [Seen::Started {
                id: id.clone(),
                title: "Fleet".to_owned(),
                directory: tmp(),
            }]
        );
        drop(server);
        side.stopped(1);
    }
}
