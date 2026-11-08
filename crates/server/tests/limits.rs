// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Every server frame stays within the protocol's frame limit, whatever clients and the agent send. See `design/server.md`.
use std::sync::Arc;
use tomori_protocol::{ClientMessage, MAX_FRAME_BYTES, QueueState, ServerMessage};

use tomori_server::Server;
use tomori_server::fake::FakeBackend;
use tomori_server::test_support::{TIMEOUT, TestClient, TestServer, TextTranscriber, eventually};

/// The snapshot budget documented in `design/server.md`.
const BUDGET: usize = MAX_FRAME_BYTES / 4;

fn json_len<T: serde::Serialize>(value: &T) -> usize {
    serde_json::to_vec(value).unwrap().len()
}

#[test]
fn oversized_client_strings_are_rejected_without_repeating_them() {
    let server = TestServer::start();
    let mut ada = server.connect("ada", "Ada");

    // About a megabyte of JSON once escaped, which an echoing error reply could not carry.
    let space: String = "\u{200b}".repeat(340_000);
    ada.send(&ClientMessage::Join { space });
    let error = ada.error();
    assert!(
        error.contains("space identifier") && error.len() < 100,
        "{error}"
    );

    ada.send(&ClientMessage::CreateSpace {
        title: "A".repeat(600_000),
        backend: "fake".into(),
        directory: "/tmp".into(),
        session: None,
        environment: None,
        instructions: None,
    });
    let error = ada.error();
    assert!(error.contains("title") && error.len() < 100, "{error}");

    ada.send(&ClientMessage::CreateSpace {
        title: "T".into(),
        backend: "B".repeat(600_000),
        directory: "/tmp".into(),
        session: None,
        environment: None,
        instructions: None,
    });
    let error = ada.error();
    assert!(!error.contains("BBB"), "{error}");

    ada.send(&ClientMessage::UtteranceBegin {
        utterance: "u".repeat(10_000),
        answer: None,
        progress: false,
        naming: false,
        attention: Vec::new(),
    });
    let error = ada.error();
    assert!(error.contains("utterance") && error.len() < 100, "{error}");

    ada.send(&ClientMessage::UtteranceEnd {
        utterance: "u".repeat(10_000),
        pointed: None,
    });
    assert_eq!(ada.error(), "That utterance is not open.");

    ada.send(&ClientMessage::Decide {
        request: "r".repeat(10_000),
        decision: tomori_protocol::Decision::Grant,
        option: Some("o".into()),
    });
    let error = ada.error();
    assert!(error.contains("request") && error.len() < 100, "{error}");
    ada.send(&ClientMessage::Decide {
        request: "r".into(),
        decision: tomori_protocol::Decision::Grant,
        option: Some("o".repeat(10_000)),
    });
    let error = ada.error();
    assert!(error.contains("option") && error.len() < 100, "{error}");
    ada.send(&ClientMessage::Answer {
        question: "q".into(),
        field: "f".repeat(10_000),
        text: "yes".into(),
    });
    let error = ada.error();
    assert!(error.contains("field") && error.len() < 100, "{error}");

    // The connection and the server keep working.
    ada.send(&ClientMessage::ListSpaces);
    ada.wait_for(|message| matches!(message, ServerMessage::Spaces { .. }).then_some(()));
    assert_eq!(server.get("/health").0, 200);
}

#[test]
fn an_oversized_hello_is_refused() {
    let server = TestServer::start();
    for (participant, name) in [
        ("p".repeat(1000), "Ada".to_owned()),
        ("ada".to_owned(), "N".repeat(1000)),
        ("ada".to_owned(), String::new()),
        ("ada".to_owned(), " \t ".to_owned()),
        // A name begins each attributed prompt, so it must not read as a slash command.
        ("ada".to_owned(), "/clear".to_owned()),
        ("ada".to_owned(), " /compact".to_owned()),
    ] {
        let mut client = TestClient::open(server.address);
        client.send(&ClientMessage::Hello {
            protocol: tomori_protocol::PROTOCOL_VERSION,
            participant,
            name,
            names: false,
            environments: None,
            environment_state: false,
            projection: false,
        });
        let error = client.error();
        assert!(
            error.len() < 100 && !error.contains("ppp") && !error.contains("NNN"),
            "{error}"
        );
        assert!(client.is_closed(TIMEOUT));
    }
}

#[test]
fn the_lobby_listing_stays_within_the_budget() {
    const SPACES: usize = 320;
    let server = TestServer::with_server(FakeBackend::new(), |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default())).with_max_spaces(SPACES)
    });
    let lobby = server.connect("lobby", "Lobby");
    let mut creator = server.connect("creator", "Creator");
    // Spaces run in existing directories beneath the workspace roots, and a path is at most 1024 bytes on macOS, so many spaces in a long directory exceed the budget instead.
    let mut directory = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("tomori-lobby-{}", std::process::id()));
    let scratch = directory.clone();
    for part in ['a', 'b', 'c', 'd'] {
        directory.push(part.to_string().repeat(220));
    }
    std::fs::create_dir_all(&directory).unwrap();
    let directory = directory.to_str().unwrap().to_owned();
    assert!(json_len(&directory) * SPACES > BUDGET);
    for index in 0..SPACES {
        creator.send(&ClientMessage::CreateSpace {
            title: format!("Space {index}"),
            backend: "fake".into(),
            directory: directory.clone(),
            session: None,
            environment: None,
            instructions: None,
        });
        let title = format!("Space {index}");
        creator.space(|state| state.title == title);
    }
    // The lobby client decodes every frame it receives, so an oversized one would fail here.
    let spaces = lobby.wait_for(|message| match message {
        ServerMessage::Spaces { spaces }
            if spaces
                .iter()
                .any(|space| space.title == format!("Space {}", SPACES - 1)) =>
        {
            Some(spaces.clone())
        }
        _ => None,
    });
    let _ = std::fs::remove_dir_all(scratch);
    assert!(json_len(&spaces) <= BUDGET);
    assert_eq!(spaces.len(), SPACES);
}

#[test]
fn a_long_conversation_is_trimmed_to_the_budget() {
    let server = TestServer::start();
    let mut ada = server.connect("ada", "Ada");
    ada.create_space("Long");
    let session = server.backend.started()[0].clone();
    // Control characters take six bytes each in JSON, so each message is far larger than its length.
    let text = "\u{1}".repeat(19_000);
    let mut last = String::new();
    for _ in 0..20 {
        last = session.stream(&[&text]);
    }
    let state = ada.space(|state| {
        state
            .messages
            .last()
            .is_some_and(|message| message.id == last)
    });
    assert!(json_len(&state) <= BUDGET);
    assert!(state.messages.len() < 20);
}

#[test]
fn an_enormous_question_is_trimmed_to_the_budget_and_can_still_be_answered() {
    use tomori_backend::{AnswerValue, Event, Field, FieldKind, FieldOption, Question};
    let server = TestServer::start();
    let mut ada = server.connect("ada", "Ada");
    ada.create_space("Question");
    let session = server.backend.started()[0].clone();
    // The field key is the question itself, as agents key answers.
    let asked = "Which? ".repeat(100_000);
    session.emit(Event::QuestionAsked(Question {
        id: "q1".to_owned(),
        title: "T".repeat(300_000),
        fields: vec![Field {
            key: asked.clone(),
            title: asked.clone(),
            description: Some("D".repeat(300_000)),
            kind: FieldKind::Choice,
            options: vec![FieldOption {
                label: "Yes".to_owned(),
                description: None,
            }],
            custom: false,
        }],
    }));
    session.emit(Event::Error {
        message: "E".repeat(300_000),
    });
    let state = ada.space(|state| state.question.is_some() && state.error.is_some());
    assert!(json_len(&state) <= BUDGET, "{}", json_len(&state));
    let question = state.question.unwrap();
    assert_eq!(question.field, "field 1");

    ada.send(&ClientMessage::Answer {
        question: question.id,
        field: question.field,
        text: "1".to_owned(),
    });
    ada.space(|state| state.question.is_none());
    // The question is cleared as soon as it is complete, before the space's driver hands the answers to the session.
    assert_eq!(
        eventually(|| Some(session.log().answers).filter(|answers| !answers.is_empty())),
        [(
            "q1".to_owned(),
            vec![(asked, AnswerValue::Choices(vec!["Yes".to_owned()]))]
        )]
    );
}

#[test]
fn requests_whose_identifiers_no_reply_could_name_are_refused_visibly_within_the_budget() {
    use tomori_backend::{
        Decision, Event, Field, FieldKind, Grant, GrantScope, Permission, PermissionOption,
        Question,
    };
    let server = TestServer::start();
    let mut ada = server.connect("ada", "Ada");
    ada.create_space("Identifiers");
    let session = server.backend.started()[0].clone();
    // Each identifier alone is larger than the snapshot budget, and longer than any identifier a client may name.
    let oversized = "i".repeat(BUDGET + 1);
    session.emit(Event::PermissionRequested(Permission {
        id: oversized.clone(),
        tool: "Bash".to_owned(),
        input: serde_json::json!({"command": "ls"}),
        description: None,
        options: Vec::new(),
    }));
    let refused = |state: &tomori_protocol::SpaceState, what: &str| {
        state
            .error
            .as_deref()
            .is_some_and(|error| error.contains(what) && error.contains("agent's own interface"))
    };
    let state = ada.space(|state| refused(state, "an approval"));
    assert!(state.approval.is_none());
    assert!(json_len(&state) <= BUDGET, "{}", json_len(&state));
    session.emit(Event::QuestionAsked(Question {
        id: oversized.clone(),
        title: "Which?".to_owned(),
        fields: vec![Field {
            key: "which".to_owned(),
            title: "Which?".to_owned(),
            description: None,
            kind: FieldKind::Text,
            options: Vec::new(),
            custom: false,
        }],
    }));
    let grant = |id: String| PermissionOption {
        id,
        grants: vec![Grant::Rule {
            tool: "Bash".to_owned(),
            content: None,
            scope: GrantScope::Session,
        }],
    };
    session.emit(Event::PermissionRequested(Permission {
        id: "p1".to_owned(),
        tool: "Bash".to_owned(),
        input: serde_json::json!({"command": "ls"}),
        description: None,
        options: vec![grant(oversized.clone()), grant("session".to_owned())],
    }));
    let state = ada.space(|state| state.approval.as_ref().is_some_and(|a| a.id == "p1"));
    assert!(json_len(&state) <= BUDGET, "{}", json_len(&state));
    assert!(state.question.is_none());
    assert!(refused(&state, "a question"));
    // Only the option a reply could name is offered, and the request can still be answered with it.
    let approval = state.approval.unwrap();
    assert_eq!(
        approval
            .options
            .iter()
            .map(|option| option.id.as_str())
            .collect::<Vec<_>>(),
        ["session"]
    );
    ada.send(&ClientMessage::Decide {
        request: approval.id,
        decision: tomori_protocol::Decision::Grant,
        option: Some("session".to_owned()),
    });
    ada.space(|state| state.approval.is_none());
    assert_eq!(
        eventually(|| Some(session.log().decisions).filter(|decisions| !decisions.is_empty())),
        [(
            "p1".to_owned(),
            Decision::Grant {
                option: "session".to_owned()
            }
        )]
    );
}

/// The error reported for `utterance`.
fn transcript_failure(client: &TestClient, utterance: &str) -> String {
    client.wait_for(|message| match message {
        ServerMessage::TranscriptFailed {
            utterance: failed,
            error,
        } if failed == utterance => Some(error.clone()),
        _ => None,
    })
}

#[test]
fn queued_input_is_bounded() {
    let server = TestServer::with_backend(tomori_server::fake::FakeBackend::manual());
    let mut ada = server.connect("ada", "Ada");
    ada.create_space("Busy");

    // A transcript longer than the limit is refused rather than queued.
    ada.say("long", &"a".repeat(5000));
    let error = transcript_failure(&ada, "long");
    assert!(error.contains("too long"), "{error}");

    // The first entry is delivered and keeps the agent busy; the rest wait, up to the limit.
    ada.say("u0", "entry 0");
    ada.space(|state| {
        state
            .queue
            .iter()
            .any(|entry| entry.text == "entry 0" && entry.state == QueueState::Delivered)
    });
    for index in 1..=17 {
        ada.say(&format!("u{index}"), &format!("entry {index}"));
    }
    // Snapshots are sent when entries are queued, before the failure of the one that did not fit.
    let mut latest = None;
    let error = ada.wait_for(|message| match message {
        ServerMessage::Space { state } => {
            latest = Some(state.clone());
            None
        }
        ServerMessage::TranscriptFailed { utterance, error } if utterance == "u17" => {
            Some(error.clone())
        }
        _ => None,
    });
    assert!(error.contains("queue is full"), "{error}");
    let state = latest.expect("a snapshot was sent");
    let texts: Vec<&str> = state
        .queue
        .iter()
        .map(|entry| entry.text.as_str())
        .collect();
    assert_eq!(texts.len(), 17, "{texts:?}");
    assert_eq!(texts.last(), Some(&"entry 16"));
}
