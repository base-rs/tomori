// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The GitHub environment's prototype against a fake GitHub: cards reach participants who watch them, are read again
//! only while someone does, are placed for everyone in the space and kept with it, and pointing at one tells the agent
//! which; the token never reaches a client, the agent, the record of spaces or the log. See `design/github.md`.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use tomori_environment_github_server::{GitHub, Token};
use tomori_protocol::{Card, Cards, ClientMessage, Pose, Resource, ServerMessage, SpaceState};
use tomori_security::Grant;
use tomori_server::fake::FAKE_BACKEND_NAME;
use tomori_server::kept::SpaceStore;
use tomori_server::test_support::{TestClient, TestServer, eventually};

const TOKEN: &str = "ghp_FAKETOKENFORTESTS0123456789";

/// Every log record, so that the test can check the token is never logged.
struct Recorder(Mutex<Vec<String>>);

impl log::Log for Recorder {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        self.0
            .lock()
            .unwrap()
            .push(format!("{} {}", record.level(), record.args()));
    }

    fn flush(&self) {}
}

static LOG: Recorder = Recorder(Mutex::new(Vec::new()));

/// Start recording log records, once for the process.
fn record() {
    static RECORDING: Once = Once::new();
    RECORDING.call_once(|| {
        log::set_logger(&LOG).unwrap();
        log::set_max_level(log::LevelFilter::Trace);
    });
}

/// A fake GitHub: answers every request with `answer`, counting them.
struct GitHubService {
    port: u16,
    requests: Arc<AtomicUsize>,
    repository: &'static str,
}

impl GitHubService {
    fn start(answer: fn() -> (u16, String)) -> Self {
        Self::start_with_fields(move || {
            let (status, body) = answer();
            (status, String::new(), body)
        })
    }

    /// A fake GitHub answering with a status, fields, each ending in CRLF, and a body.
    fn start_with_fields(answer: impl Fn() -> (u16, String, String) + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(AtomicUsize::new(0));
        let counted = requests.clone();
        std::thread::spawn(move || {
            for mut socket in listener.incoming().flatten() {
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let _ = reader.read_exact(&mut vec![0; length]);
                counted.fetch_add(1, Ordering::SeqCst);
                let (status, fields, body) = answer();
                let _ = write!(
                    socket,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{fields}\r\n{body}",
                    body.len()
                );
            }
        });
        Self {
            port,
            requests,
            repository: "o/r",
        }
    }

    fn github(&self) -> GitHub {
        GitHub::with_api(
            self.repository.parse().unwrap(),
            Token::new(TOKEN).unwrap(),
            &format!("http://127.0.0.1:{}/graphql", self.port),
        )
        .unwrap()
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

/// Two open issues and a pull request whose checks fail.
fn repository() -> (u16, String) {
    let body = serde_json::json!({"data": {
        "rateLimit": {"remaining": 4_999, "resetAt": "2026-11-05T10:00:00Z"},
        "repository": {
            "issues": {"nodes": [
                {"number": 97, "title": "Physics for cards", "createdAt": "2026-11-01T09:00:00Z",
                 "updatedAt": "2026-11-04T09:00:00Z", "author": {"login": "ioquatix"},
                 "labels": {"nodes": [{"name": "enhancement", "color": "a2eeef"}]}},
                {"number": 146, "title": "Better text", "createdAt": "2026-11-01T09:00:00Z",
                 "updatedAt": "2026-11-03T09:00:00Z", "author": {"login": "ioquatix"}},
            ]},
            "pullRequests": {"nodes": [
                {"number": 164, "title": "Publish the clients", "isDraft": false,
                 "createdAt": "2026-11-04T09:00:00Z", "updatedAt": "2026-11-05T09:00:00Z",
                 "author": {"login": "ada"},
                 "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": "FAILURE"}}}]}},
            ]},
        },
    }});
    (200, body.to_string())
}

const ISSUE: &str = "github://o/r/issues/97";

/// A state directory of its own, removed when dropped.
struct State(PathBuf);

impl State {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("tomori-test-github-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for State {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A server admitting by tokens and visitors, showing `github`'s cards and keeping its spaces in `state`.
fn start(github: GitHub, state: &State) -> TestServer {
    let store = Arc::new(SpaceStore::open(&state.0).expect("the spaces open"));
    TestServer::with_tokens(true, move |server| {
        github.offer(server).with_kept_spaces(store)
    })
}

/// The cards `client` is sent next that satisfy `predicate`.
fn cards(client: &TestClient, mut predicate: impl FnMut(&Cards) -> bool) -> Cards {
    client.wait_for(|message| match message {
        ServerMessage::Cards { cards } if predicate(cards) => Some((**cards).clone()),
        _ => None,
    })
}

fn card<'a>(cards: &'a Cards, id: &str) -> Option<&'a Card> {
    cards.cards.iter().find(|card| card.id == id)
}

fn pose(x: f32) -> Pose {
    Pose {
        position: [x, 1.0, -0.6],
        orientation: [0.0, 0.0, 0.0, 1.0],
    }
}

fn create(client: &mut TestClient, environment: &str) -> SpaceState {
    client.send(&ClientMessage::CreateSpace {
        title: format!("In {environment}"),
        backend: FAKE_BACKEND_NAME.to_owned(),
        directory: "/tmp".to_owned(),
        session: None,
        environment: Some(environment.to_owned()),
        instructions: None,
    });
    client.space(|state| state.environment == environment)
}

#[test]
fn cards_are_watched_placed_kept_and_pointed_at_without_the_token_ever_leaving_the_server() {
    record();
    let github = GitHubService::start(repository);
    let state = State::new("cards");
    let server = start(github.github(), &state);
    let (_, owner) = server.issue("Ada", Grant::Owner);
    let mut ada = server.connect_with_token("ada", "Ada", &owner);
    let space = create(&mut ada, "github");
    assert!(space.cards);
    // The agent is told what the cards are, and nothing of the token.
    let layer = space.instructions.as_ref().unwrap().environment.clone();
    assert!(
        layer.contains("open issues and pull requests of the GitHub repository o/r"),
        "{layer}"
    );
    // Nobody watches yet, so GitHub has not been asked.
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(github.requests(), 0);
    ada.send(&ClientMessage::WatchCards);
    let read = cards(&ada, |cards| cards.cards.len() == 3);
    assert_eq!(
        (
            read.space.as_str(),
            read.source.as_str(),
            read.status.as_ref()
        ),
        (space.id.as_str(), "o/r", None)
    );
    let order: Vec<u64> = read.cards.iter().map(|card| card.number).collect();
    assert_eq!(order, [164, 97, 146]);
    assert!(read.cards.iter().all(|card| card.placement.is_none()));
    assert_eq!(github.requests(), 1);

    // A collaborator watching sees the same cards without GitHub being asked again.
    let (_, collaborator) = server.issue(
        "Bob",
        Grant::Collaborator {
            space: space.id.clone(),
        },
    );
    let mut bob = server.connect_with_token("bob", "Bob", &collaborator);
    bob.send(&ClientMessage::Join {
        space: space.id.clone(),
    });
    assert!(bob.space(|state| state.id == space.id).cards);
    bob.send(&ClientMessage::WatchCards);
    cards(&bob, |cards| cards.cards.len() == 3);
    assert_eq!(github.requests(), 1);

    // A visitor sees that the space shows cards, but cannot watch or place them.
    let mut cal = server.connect("cal", "Cal");
    cal.send(&ClientMessage::Join {
        space: space.id.clone(),
    });
    assert!(cal.space(|state| state.id == space.id).cards);
    cal.send(&ClientMessage::WatchCards);
    assert_eq!(cal.error(), "Visitors can watch, but not act in a space.");
    cal.send(&ClientMessage::PlaceCard {
        card: ISSUE.into(),
        placement: Some(pose(0.0)),
    });
    assert_eq!(cal.error(), "Visitors can watch, but not act in a space.");

    // Placing a card moves it for everyone watching.
    ada.send(&ClientMessage::PlaceCard {
        card: ISSUE.into(),
        placement: Some(pose(0.5)),
    });
    let placed = cards(&bob, |cards| {
        card(cards, ISSUE).is_some_and(|card| card.placement.is_some())
    });
    assert_eq!(card(&placed, ISSUE).unwrap().placement, Some(pose(0.5)));
    cards(&ada, |cards| {
        card(cards, ISSUE).is_some_and(|card| card.placement == Some(pose(0.5)))
    });
    // Somewhere no card can be, a card not on the board, and an oversized identity are refused.
    ada.send(&ClientMessage::PlaceCard {
        card: ISSUE.into(),
        placement: Some(pose(100.0)),
    });
    assert_eq!(ada.error(), "That is not somewhere a card can be put.");
    ada.send(&ClientMessage::PlaceCard {
        card: "github://o/r/issues/1".into(),
        placement: Some(pose(0.0)),
    });
    assert_eq!(ada.error(), "That card is not on the board.");
    ada.send(&ClientMessage::PlaceCard {
        card: "x".repeat(10_000),
        placement: None,
    });
    assert!(ada.error().contains("card"));
    // Putting it in the same place again changes nothing; taking another back to the inbox, which it is in, neither.
    ada.send(&ClientMessage::PlaceCard {
        card: ISSUE.into(),
        placement: Some(pose(0.5)),
    });
    ada.send(&ClientMessage::PlaceCard {
        card: "github://o/r/issues/146".into(),
        placement: None,
    });

    // Pointing at a card as the words begin tells the agent which, as the server has it, before the words.
    ada.say_attending(
        "u1",
        "what's this one about?",
        vec![Resource::card(ISSUE)],
        None,
    );
    let session = server.backend.started()[0].clone();
    let prompt = eventually(|| {
        session
            .log()
            .prompts
            .first()
            .map(|prompt| prompt.text.clone())
    });
    assert!(
        prompt
            .starts_with("Ada: [Pointing at issue #97 (https://github.com/o/r/issues/97, opened "),
        "{prompt}"
    );
    assert!(
        prompt.ends_with(
            "; untrusted repository content: title \"Physics for cards\", author \"ioquatix\", labels \"enhancement\")] \
what's this one about?"
        ),
        "{prompt}"
    );
    // A card the board does not have, and an oversized one, add nothing, and the words are kept; nor does one named
    // as the words end, as clients from before attention did.
    ada.say_attending(
        "u2",
        "and this?",
        vec![Resource::card("github://o/r/issues/1")],
        None,
    );
    ada.say_attending(
        "u3",
        "or this?",
        vec![Resource::card("x".repeat(10_000))],
        Some(Resource::card("x".repeat(10_000))),
    );
    let prompts = eventually(|| {
        let prompts = session.log().prompts;
        (prompts.len() == 3).then_some(prompts)
    });
    assert_eq!(prompts[1].text, "Ada: and this?");
    assert_eq!(prompts[2].text, "Ada: or this?");

    // Elsewhere there are no cards to watch, place or point at.
    let (_, dee_token) = server.issue("Dee", Grant::Owner);
    let mut dee = server.connect_with_token("dee", "Dee", &dee_token);
    let sky = create(&mut dee, "sky");
    assert!(!sky.cards);
    dee.send(&ClientMessage::WatchCards);
    assert_eq!(dee.error(), "This space shows no cards.");
    dee.send(&ClientMessage::PlaceCard {
        card: ISSUE.into(),
        placement: None,
    });
    assert_eq!(dee.error(), "This space shows no cards.");
    dee.say_attending("u4", "this?", vec![Resource::card(ISSUE)], None);
    let other = eventually(|| server.backend.started().get(1).cloned());
    let prompt = eventually(|| {
        other
            .log()
            .prompts
            .first()
            .map(|prompt| prompt.text.clone())
    });
    assert_eq!(prompt, "Dee: this?");
    dee.send(&ClientMessage::Leave);
    dee.send(&ClientMessage::WatchCards);
    assert_eq!(dee.error(), "Join a space first.");

    // Changed to Sky, the space shows no cards, and its watchers are sent none.
    ada.send(&ClientMessage::SetEnvironment {
        environment: "sky".into(),
    });
    assert!(!bob.space(|state| state.environment == "sky").cards);
    ada.send(&ClientMessage::PlaceCard {
        card: ISSUE.into(),
        placement: Some(pose(0.0)),
    });
    assert_eq!(ada.error(), "This space shows no cards.");
    ada.send(&ClientMessage::SetEnvironment {
        environment: "github".into(),
    });
    assert!(bob.space(|state| state.environment == "github").cards);

    // The token reached no client, the agent or the log.
    assert!(
        ada.received()
            .iter()
            .any(|message| message.contains("Physics for cards"))
    );
    for client in [&ada, &bob, &cal, &dee] {
        assert!(
            client
                .received()
                .iter()
                .all(|message| !message.contains(TOKEN))
        );
    }
    for session in server.backend.started() {
        let log = session.log();
        assert!(
            log.prompts
                .iter()
                .all(|prompt| !prompt.text.contains(TOKEN))
        );
        assert!(
            log.instructions
                .iter()
                .all(|(_, text)| !text.as_deref().unwrap_or_default().contains(TOKEN))
        );
    }
    let logged = LOG.0.lock().unwrap().clone();
    assert!(!logged.is_empty());
    assert!(logged.iter().all(|line| !line.contains(TOKEN)));
    drop((ada, bob, cal, dee));
    drop(server);

    // The placement is kept with the space, and never the token.
    let kept = std::fs::read_to_string(state.0.join(tomori_server::kept::SPACES_FILE)).unwrap();
    assert!(kept.contains(ISSUE) && !kept.contains(TOKEN));
    let server = start(github.github(), &state);
    let (_, owner) = server.issue("Ada", Grant::Owner);
    let mut ada = server.connect_with_token("ada", "Ada", &owner);
    ada.send(&ClientMessage::Join {
        space: space.id.clone(),
    });
    assert!(ada.space(|state| state.id == space.id).cards);
    ada.send(&ClientMessage::WatchCards);
    let restored = cards(&ada, |cards| cards.cards.len() == 3);
    assert_eq!(card(&restored, ISSUE).unwrap().placement, Some(pose(0.5)));
    assert!(
        LOG.0
            .lock()
            .unwrap()
            .iter()
            .all(|line| !line.contains(TOKEN))
    );
}

#[test]
fn a_failed_read_is_shown_to_watchers_and_tried_again_later() {
    let github = GitHubService::start(|| (401, "{}".to_owned()));
    let state = State::new("refused");
    let server = start(github.github(), &state);
    let (_, owner) = server.issue("Ada", Grant::Owner);
    let mut ada = server.connect_with_token("ada", "Ada", &owner);
    create(&mut ada, "github");
    ada.send(&ClientMessage::WatchCards);
    // Once tried, the cards say why they could not be read.
    let refused = cards(&ada, |cards| {
        cards
            .status
            .as_deref()
            .is_some_and(|status| status.starts_with("Could not"))
    });
    assert_eq!(
        refused.status.as_deref(),
        Some("Could not read o/r: GitHub refused the server's token.")
    );
    assert!(refused.cards.is_empty());
    // Not asked again at once, though someone still watches.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(github.requests(), 1);
    assert!(
        ada.received()
            .iter()
            .all(|message| !message.contains(TOKEN))
    );
}

#[test]
fn a_used_up_rate_limit_answered_with_200_is_waited_for_and_said_once() {
    record();
    // GitHub's GraphQL API answers 200 when the token's primary rate limit is used up, saying so in its body and fields.
    let mut github = GitHubService::start_with_fields(|| {
        let reset = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3_600;
        let body = serde_json::json!({"data": null, "errors": [
            {"type": "RATE_LIMITED", "message": "API rate limit already exceeded for user ID 1."}
        ]});
        (
            200,
            format!("X-RateLimit-Remaining: 0\r\nX-RateLimit-Reset: {reset}\r\n"),
            body.to_string(),
        )
    });
    github.repository = "o/limited";
    let state = State::new("limited");
    let server = start(github.github(), &state);
    let (_, owner) = server.issue("Ada", Grant::Owner);
    let mut ada = server.connect_with_token("ada", "Ada", &owner);
    create(&mut ada, "github");
    ada.send(&ClientMessage::WatchCards);
    let limited = cards(&ada, |cards| {
        cards
            .status
            .as_deref()
            .is_some_and(|status| status.starts_with("Could not"))
    });
    assert_eq!(
        limited.status.as_deref(),
        Some("Could not read o/limited: GitHub is limiting requests.")
    );
    assert_eq!(github.requests(), 1);
    // The server says once, at info rather than as a warning, that it waits about an hour, until the limit resets,
    // rather than trying again in 30 seconds.
    let lines: Vec<String> = LOG
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|line| line.contains("o/limited"))
        .cloned()
        .collect();
    assert_eq!(lines.len(), 1, "{lines:?}");
    let seconds: u64 = lines[0]
        .strip_prefix("INFO ")
        .and_then(|line| line.split_once("read again in "))
        .and_then(|(_, rest)| rest.split_once(' '))
        .and_then(|(seconds, _)| seconds.parse().ok())
        .unwrap_or_else(|| panic!("{lines:?}"));
    assert!((3_590..=3_630).contains(&seconds), "{seconds}");
}
