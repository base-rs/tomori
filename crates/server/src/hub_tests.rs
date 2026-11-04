// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The hub's rarer paths, driven through [`Hub::handle`] and the calls the supervisor and the space drivers make: refusals, a driver that has stopped, and state that has moved on meanwhile.

use super::*;
use crate::fake::FakeBackend;
use crate::outbox::Popped;
use crate::transcriber::NullTranscriber;

/// A hub with the fake backend, and the jobs it gives the supervisor.
fn hub_with(backend: FakeBackend) -> (Arc<Hub>, async_channel::Receiver<Job>) {
    hub_hearing(backend, Arc::new(NullTranscriber::new()))
}

/// A hub with the fake backend that recognizes speech with `transcriber`.
fn hub_hearing(
    backend: FakeBackend,
    transcriber: Arc<dyn Transcriber>,
) -> (Arc<Hub>, async_channel::Receiver<Job>) {
    let (jobs, receiver) = async_channel::unbounded();
    let hub = Hub::new(
        Backends::new().with(backend),
        transcriber,
        jobs,
        Vec::new(),
        SpaceSlots::new(crate::workspace::DEFAULT_MAX_SPACES),
    );
    (hub, receiver)
}

fn hub() -> (Arc<Hub>, async_channel::Receiver<Job>) {
    hub_with(FakeBackend::new())
}

/// A transcriber that hears nothing until a test gives the transcript.
struct Waiting;

impl Transcriber for Waiting {
    fn begin(&self, _: UtteranceId, _: TranscriptSink) {}
    fn push(&self, _: UtteranceId, _: &AudioFrame) {}
    fn end(&self, _: UtteranceId) {}
    fn cancel(&self, _: UtteranceId) {}
}

/// A hub whose transcripts the test gives.
fn hub_waiting() -> (Arc<Hub>, async_channel::Receiver<Job>) {
    hub_hearing(FakeBackend::new(), Arc::new(Waiting))
}

/// One connection to the hub, read as its client would read it.
struct Client {
    hub: Arc<Hub>,
    connection: ConnectionId,
    outbox: Arc<Outbox>,
}

impl Client {
    fn connect(hub: &Arc<Hub>, access: Access) -> Self {
        let (connection, outbox, ..) = hub.connect(Cancellation::new(), access).unwrap();
        Self {
            hub: hub.clone(),
            connection,
            outbox,
        }
    }

    /// Connected and greeted as `participant`, with what that sent read.
    fn greeted(hub: &Arc<Hub>, access: Access, participant: &str) -> Self {
        let client = Self::connect(hub, access);
        client.hello(participant);
        client.read();
        client
    }

    fn hello(&self, participant: &str) {
        self.send(ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            participant: participant.to_owned(),
            name: participant.to_owned(),
        });
    }

    fn send(&self, message: ClientMessage) {
        self.hub.handle(self.connection, message);
    }

    /// Everything sent since, with the space and the lobby rendered as they are now.
    fn read(&self) -> Vec<ServerMessage> {
        let mut messages = Vec::new();
        loop {
            match self.outbox.pop() {
                Popped::Control(item) => messages.extend(self.hub.render(self.connection, item)),
                Popped::Audio(_) => {}
                Popped::Empty | Popped::Closed => return messages,
            }
        }
    }

    /// The errors sent since.
    fn errors(&self) -> Vec<String> {
        self.read()
            .into_iter()
            .filter_map(|message| match message {
                ServerMessage::Error { error, .. } => Some(error),
                _ => None,
            })
            .collect()
    }

    fn join(&self, space: &str) {
        self.send(ClientMessage::Join {
            space: space.to_owned(),
        });
        self.read();
    }
}

fn request(creator: &str) -> CreateRequest {
    CreateRequest {
        title: "Space".to_owned(),
        backend: "fake".to_owned(),
        directory: "/tmp".to_owned(),
        session: None,
        creator: creator.to_owned(),
        creator_space: None,
        environment: "sky".to_owned(),
        instructions: String::new(),
    }
}

/// Register a space whose driver has stopped, so that nothing can be handed to it.
fn stopped(hub: &Hub, snapshot: Snapshot) -> String {
    let (commands, _) = async_channel::unbounded();
    hub.register_space(&request("nobody"), "session".to_owned(), snapshot, commands)
        .unwrap()
}

/// Register a space with a driver, whose commands are returned.
fn running(
    hub: &Hub,
    request: &CreateRequest,
    snapshot: Snapshot,
) -> (String, async_channel::Receiver<SpaceCommand>) {
    let (commands, receiver) = async_channel::unbounded();
    let id = hub
        .register_space(request, "session".to_owned(), snapshot, commands)
        .unwrap();
    (id, receiver)
}

fn permission() -> tomori_backend::Permission {
    tomori_backend::Permission {
        id: "request-1".to_owned(),
        tool: "Bash".to_owned(),
        input: serde_json::json!({"command": "cargo test"}),
        description: None,
        options: Vec::new(),
    }
}

fn question() -> tomori_backend::Question {
    tomori_backend::Question {
        id: "question-1".to_owned(),
        title: "Which?".to_owned(),
        fields: vec![tomori_backend::Field {
            key: "field-1".to_owned(),
            title: "Name it".to_owned(),
            description: None,
            kind: tomori_backend::FieldKind::Text,
            options: Vec::new(),
            custom: false,
        }],
    }
}

#[test]
fn a_frame_that_does_not_decode_is_refused_and_nothing_else_happens() {
    let (hub, _jobs) = hub();
    let client = Client::greeted(&hub, Access::open(), "ada");
    for (kind, said) in [
        (FrameKind::Control, "Invalid message: "),
        (FrameKind::Audio, "Invalid audio: "),
    ] {
        hub.receive(
            client.connection,
            Frame {
                kind,
                payload: b"\xff".to_vec(),
            },
        );
        let errors = client.errors();
        assert!(
            errors.len() == 1 && errors[0].starts_with(said),
            "{errors:?}"
        );
    }
    // Nor to a connection that has gone.
    hub.disconnect(client.connection);
    hub.receive(
        client.connection,
        Frame {
            kind: FrameKind::Control,
            payload: b"\xff".to_vec(),
        },
    );
}

#[test]
fn hello_is_said_once_in_this_protocol_with_an_identity() {
    let (hub, _jobs) = hub();
    let client = Client::greeted(&hub, Access::open(), "ada");
    client.hello("ada");
    assert_eq!(client.errors(), ["This connection already said hello."]);

    let client = Client::connect(&hub, Access::open());
    client.send(ClientMessage::Hello {
        protocol: PROTOCOL_VERSION + 1,
        participant: "bob".to_owned(),
        name: "Bob".to_owned(),
    });
    let errors = client.errors();
    assert!(errors[0].starts_with("Unsupported protocol"), "{errors:?}");
    assert!(matches!(client.outbox.pop(), Popped::Closed));

    let client = Client::connect(&hub, Access::open());
    client.hello("");
    assert_eq!(client.errors(), ["A participant identity is required."]);

    // A hello on a connection that has gone is ignored.
    let client = Client::connect(&hub, Access::open());
    hub.disconnect(client.connection);
    client.hello("cy");
    assert!(!hub.status().lobby.iter().any(|(name, _)| name == "cy"));
}

#[test]
fn creating_or_listing_is_refused_by_a_backend_that_cannot_attach_or_a_server_shutting_down() {
    let backend = FakeBackend::new();
    backend.attach_nothing();
    let (hub, jobs) = hub_with(backend);
    let client = Client::greeted(&hub, Access::open(), "ada");
    let create = |session: Option<&str>| ClientMessage::CreateSpace {
        title: "Space".to_owned(),
        backend: "fake".to_owned(),
        directory: "/tmp".to_owned(),
        session: session.map(str::to_owned),
        environment: None,
        instructions: None,
    };
    let list = |backend: String| ClientMessage::ListSessions {
        backend,
        directory: "/tmp".to_owned(),
    };
    client.send(create(Some("session")));
    assert_eq!(
        client.errors(),
        ["The fake backend cannot bind an existing session."]
    );
    client.send(list("fake".to_owned()));
    assert_eq!(
        client.errors(),
        ["The fake backend cannot list existing sessions."]
    );
    client.send(list("x".repeat(limits::NAME_BYTES + 1)));
    assert_eq!(client.errors().len(), 1);

    let (other, jobs_listed) = super::rarer::hub();
    let lister = Client::greeted(&other, Access::open(), "ada");
    drop(jobs_listed);
    lister.send(list("fake".to_owned()));
    assert_eq!(lister.errors(), ["The server is shutting down."]);

    drop(jobs);
    client.send(create(None));
    assert_eq!(client.errors(), ["The server is shutting down."]);
}

#[test]
fn a_new_utterance_fails_the_one_still_open() {
    let (hub, _jobs) = hub();
    let id = stopped(&hub, Snapshot::default());
    let client = Client::greeted(&hub, Access::open(), "ada");
    client.join(&id);
    let begin = |utterance: &str| ClientMessage::UtteranceBegin {
        utterance: utterance.to_owned(),
        answer: None,
    };
    client.send(begin("one"));
    client.send(begin("two"));
    let failed: Vec<_> = client
        .read()
        .into_iter()
        .filter_map(|message| match message {
            ServerMessage::TranscriptFailed { utterance, error } => Some((utterance, error)),
            _ => None,
        })
        .collect();
    assert_eq!(
        failed,
        [(
            "one".to_owned(),
            "A new utterance started before this one ended.".to_owned()
        )]
    );
}

#[test]
fn a_decision_or_answer_for_a_driver_that_has_stopped_is_refused_and_kept() {
    let (hub, _jobs) = hub();
    let id = stopped(
        &hub,
        Snapshot {
            permissions: vec![permission()],
            questions: vec![question()],
            ..Snapshot::default()
        },
    );
    let client = Client::greeted(&hub, Access::open(), "ada");
    client.join(&id);
    client.send(ClientMessage::Decide {
        request: "request-1".to_owned(),
        decision: tomori_protocol::Decision::AllowOnce,
        option: None,
    });
    assert_eq!(client.errors(), ["The agent session has stopped."]);
    assert!(
        hub.lock()
            .space(&id)
            .unwrap()
            .model
            .state(Vec::new(), "")
            .approval
            .is_some()
    );
}

#[test]
fn what_a_driver_reports_for_a_space_that_has_gone_is_dropped() {
    let (hub, _jobs) = hub();
    let error = || tomori_backend::Error::new(ErrorKind::Unavailable, "down");
    hub.session_changed("gone", "session".to_owned());
    hub.space_event("gone", Event::Idle);
    hub.space_sent("gone", "key", Ok(()));
    hub.space_decided("gone", "request", "ada", Ok(()));
    hub.space_decided("gone", "request", "ada", Err(error()));
    hub.space_answered("gone", "question", "ada", Ok(()));
    hub.space_answered("gone", "question", "ada", Err(error()));
    hub.space_mode_set("gone", "ada", Err(error()));
    hub.space_instructed("gone", Ok(()), true);
    assert_eq!(hub.instructions_due("gone"), (Vec::new(), false));
    hub.space_ended("gone", None);
    hub.restore_failed("gone", "lost".to_owned(), true);
    assert!(!hub.bound("gone", "session".to_owned(), Snapshot::default()));
    hub.agent_gap("gone", 1);
    assert_eq!(hub.pace_speech("gone", Instant::now()), None);
    hub.speech_failed("gone", "silent".to_owned());
    assert!(hub.status().spaces.is_empty());
}

#[test]
fn a_session_that_changes_its_identifier_keeps_its_space() {
    let (hub, _jobs) = hub();
    let id = stopped(&hub, Snapshot::default());
    hub.session_changed(&id, "renamed".to_owned());
    assert_eq!(hub.lock().space(&id).unwrap().model.session, "renamed");
}

#[test]
fn a_decision_or_answer_the_agent_refused_is_kept_unless_it_is_gone() {
    let (hub, _jobs) = hub();
    let snapshot = || Snapshot {
        permissions: vec![permission()],
        questions: vec![question()],
        ..Snapshot::default()
    };
    let (id, commands) = running(&hub, &request("ada"), snapshot());
    let client = Client::greeted(&hub, Access::open(), "ada");
    client.join(&id);
    let state = |hub: &Hub| hub.lock().space(&id).unwrap().model.state(Vec::new(), "");
    for (kind, kept) in [(ErrorKind::Invalid, true), (ErrorKind::NotFound, false)] {
        client.send(ClientMessage::Decide {
            request: "request-1".to_owned(),
            decision: tomori_protocol::Decision::AllowOnce,
            option: None,
        });
        client.read();
        hub.space_decided(
            &id,
            "request-1",
            "/ada",
            Err(tomori_backend::Error::new(kind, "no")),
        );
        assert_eq!(client.errors(), ["The decision was not accepted: no"]);
        assert_eq!(state(&hub).approval.is_some(), kept);
    }
    // The question shows once no approval is in front of it.
    for (kind, kept) in [(ErrorKind::Invalid, true), (ErrorKind::Closed, false)] {
        client.send(ClientMessage::Answer {
            question: "question-1".to_owned(),
            field: "field-1".to_owned(),
            text: "Ada".to_owned(),
        });
        client.read();
        hub.space_answered(
            &id,
            "question-1",
            "/ada",
            Err(tomori_backend::Error::new(kind, "no")),
        );
        assert_eq!(client.errors(), ["The answer was not accepted: no"]);
        assert_eq!(state(&hub).question.is_some(), kept);
    }
    // A mode the agent would not switch to is said.
    hub.space_mode_set(
        &id,
        "/ada",
        Err(tomori_backend::Error::new(ErrorKind::Invalid, "no")),
    );
    assert_eq!(client.errors(), ["The mode was not changed: no"]);
    drop(commands);
}

#[test]
fn an_answer_completing_a_question_for_a_driver_that_has_stopped_is_refused_and_kept() {
    let (hub, _jobs) = hub();
    let id = stopped(
        &hub,
        Snapshot {
            questions: vec![question()],
            ..Snapshot::default()
        },
    );
    let client = Client::greeted(&hub, Access::open(), "ada");
    client.join(&id);
    client.send(ClientMessage::Answer {
        question: "question-1".to_owned(),
        field: "field-1".to_owned(),
        text: "Ada".to_owned(),
    });
    assert_eq!(client.errors(), ["The agent session has stopped."]);
    assert!(
        hub.lock()
            .space(&id)
            .unwrap()
            .model
            .state(Vec::new(), "")
            .question
            .is_some()
    );
}

#[test]
fn modes_are_switched_only_while_the_agent_can_take_it() {
    let (hub, _jobs) = hub();
    let modes = tomori_backend::Modes {
        offered: vec![tomori_backend::Mode {
            id: "plan".to_owned(),
            name: "Plan".to_owned(),
            description: None,
        }],
        current: None,
    };
    let id = stopped(
        &hub,
        Snapshot {
            modes,
            ..Snapshot::default()
        },
    );
    let client = Client::greeted(&hub, Access::open(), "ada");
    client.join(&id);
    let plan = || ClientMessage::SetMode {
        mode: "plan".to_owned(),
    };
    // The driver has stopped.
    client.send(plan());
    assert_eq!(client.errors(), ["The agent session has stopped."]);
    hub.space_event(
        &id,
        Event::Reconnecting {
            reason: "lost".to_owned(),
        },
    );
    client.send(plan());
    assert_eq!(client.errors(), [crate::space::RECONNECTING]);
    hub.space_ended(&id, None);
    client.send(plan());
    assert_eq!(client.errors(), ["The agent session has stopped."]);
}

#[test]
fn what_needs_a_space_is_refused_in_the_lobby() {
    let (hub, _jobs) = hub();
    let client = Client::greeted(&hub, Access::open(), "ada");
    for message in [
        ClientMessage::SetAttribution { enabled: false },
        ClientMessage::DecideKnock {
            knock: "knock-1".to_owned(),
            admit: true,
        },
        ClientMessage::Replay { message: None },
    ] {
        client.send(message);
        assert_eq!(client.errors(), ["Join a space first."]);
    }
    // Presence and stopping a replay outside a space go nowhere, and say nothing.
    client.send(ClientMessage::Presence {
        head: Default::default(),
        left: None,
        right: None,
        hands: None,
    });
    client.send(ClientMessage::StopReplay);
    assert!(client.read().is_empty());
    // Identifiers beyond the limit are refused before anything else.
    let long = "x".repeat(limits::IDENTIFIER_BYTES + 1);
    for message in [
        ClientMessage::DecideKnock {
            knock: long.clone(),
            admit: true,
        },
        ClientMessage::Replay {
            message: Some(long.clone()),
        },
    ] {
        client.send(message);
        assert_eq!(client.errors().len(), 1);
    }
}

#[test]
fn a_visitors_engagement_is_ignored() {
    let (hub, _jobs) = hub();
    let id = stopped(&hub, Snapshot::default());
    let visitor = Client::greeted(&hub, Access::visitor(), "vic");
    visitor.join(&id);
    visitor.send(ClientMessage::Engagement {
        state: Engagement::Away,
    });
    assert!(visitor.read().is_empty());
    assert!(
        hub.lock()
            .participants
            .values()
            .all(|record| record.engagement.is_none())
    );
}

/// A token store in a directory of its own, removed when dropped.
struct Store {
    directory: std::path::PathBuf,
    tokens: Arc<tomori_security::TokenStore>,
}

impl Store {
    fn new(name: &str) -> Self {
        let directory =
            std::env::temp_dir().join(format!("tomori-hub-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let tokens = Arc::new(tomori_security::TokenStore::open(&directory).unwrap());
        Self { directory, tokens }
    }

    /// Make every later write fail, by taking the name of the file each write goes through.
    fn refuse_writes(&self) {
        std::fs::create_dir(self.directory.join("tokens.tmp")).unwrap();
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

/// A hub that admits knocks with `store`'s tokens, a space, its owner in it, and a visitor in it who has knocked.
fn knocked(store: &Store) -> (Arc<Hub>, String, Client, Client) {
    let (hub, _jobs) = hub();
    hub.set_tokens(store.tokens.clone());
    hub.set_encrypted(true);
    let id = stopped(&hub, Snapshot::default());
    let owner = Client::greeted(&hub, Access::open(), "ada");
    owner.join(&id);
    let visitor = Client::greeted(&hub, Access::visitor(), "vic");
    visitor.join(&id);
    visitor.send(ClientMessage::Knock);
    visitor.read();
    owner.read();
    (hub, id, owner, visitor)
}

/// The knock waiting, taken as an answer to it takes it.
fn take_knock(hub: &Hub) -> KnockRecord {
    hub.lock().knocks.remove(0)
}

fn knock_states(client: &Client) -> Vec<tomori_protocol::KnockState> {
    client
        .read()
        .into_iter()
        .filter_map(|message| match message {
            ServerMessage::KnockState { state } => Some(state),
            _ => None,
        })
        .collect()
}

#[test]
fn a_knock_needs_a_server_that_admits_by_knocking() {
    let (hub, _jobs) = hub();
    let id = stopped(&hub, Snapshot::default());
    let visitor = Client::greeted(&hub, Access::visitor(), "vic");
    visitor.join(&id);
    visitor.send(ClientMessage::Knock);
    assert_eq!(
        visitor.errors(),
        ["This server does not admit by knocking."]
    );
}

#[test]
fn a_visitor_who_knocks_again_before_being_admitted_is_admitted_once() {
    let store = Store::new("again");
    let (hub, id, owner, visitor) = knocked(&store);
    // Answered, then knocked again before the admission is made.
    let knock = take_knock(&hub);
    visitor.send(ClientMessage::Knock);
    visitor.read();
    owner.read();
    hub.admit(knock);
    assert!(hub.lock().knocks.is_empty());
    let messages = visitor.read();
    assert!(
        messages
            .iter()
            .any(|message| matches!(message, ServerMessage::Admitted { .. })),
        "{messages:?}"
    );
    assert_eq!(hub.status().spaces[0].participants.len(), 2);
    let _ = id;
}

#[test]
fn a_visitor_admitted_after_moving_on_takes_part_only_where_they_knocked() {
    let store = Store::new("moved");
    let (hub, id, _owner, visitor) = knocked(&store);
    let knock = take_knock(&hub);
    let other = stopped(&hub, Snapshot::default());
    visitor.join(&other);
    hub.admit(knock);
    let state = hub.lock();
    let space = |id: &str| &state.space(id).unwrap().model;
    assert_eq!(space(&other).visitors.len(), 1);
    assert!(space(&other).participants.is_empty());
    assert_eq!(space(&id).participants.len(), 1);
}

#[test]
fn a_visitor_who_connected_again_before_being_admitted_is_not_admitted_and_their_token_is_revoked()
{
    let store = Store::new("reconnected");
    let (hub, _id, _owner, visitor) = knocked(&store);
    let knock = take_knock(&hub);
    let again = Client::greeted(&hub, Access::visitor(), "vic");
    hub.admit(knock);
    // The connection superseded is closed, so the withdrawal it is told goes nowhere.
    assert!(matches!(visitor.outbox.pop(), Popped::Closed));
    assert!(store.tokens.entries().is_empty());
    assert!(again.read().is_empty());
}

#[test]
fn a_token_that_cannot_be_issued_is_said_to_the_visitor_still_here() {
    let store = Store::new("unissued");
    let (hub, _id, _owner, visitor) = knocked(&store);
    store.refuse_writes();
    hub.admit(take_knock(&hub));
    assert_eq!(
        visitor.errors(),
        ["You were admitted, but the token could not be issued. Knock again."]
    );
    // Nobody is told once the visitor has gone.
    visitor.send(ClientMessage::Knock);
    let knock = take_knock(&hub);
    hub.disconnect(visitor.connection);
    hub.admit(knock);
    assert!(store.tokens.entries().is_empty());
}

#[test]
fn a_declined_visitor_waits_out_the_cooldown_kept_until_it_ends() {
    let store = Store::new("declined");
    let (hub, _id, owner, visitor) = knocked(&store);
    let knock = hub.lock().knocks[0].id.clone();
    owner.send(ClientMessage::DecideKnock {
        knock,
        admit: false,
    });
    assert_eq!(
        knock_states(&visitor),
        [tomori_protocol::KnockState::Declined]
    );
    let keepalive = Keepalive {
        interval: Duration::from_secs(5),
        timeout: Duration::from_secs(15),
    };
    let hello = Duration::from_secs(3600);
    hub.sweep(Instant::now(), &keepalive, hello);
    assert_eq!(hub.lock().knock_cooldowns.len(), 1);
    hub.sweep(Instant::now() + KNOCK_COOLDOWN, &keepalive, hello);
    assert!(hub.lock().knock_cooldowns.is_empty());
}

/// A hub that pairs with `store`'s tokens over TLS, with a device connected only to pair.
fn pairing(store: &Store) -> (Arc<Hub>, Client) {
    let (hub, _jobs) = hub();
    hub.set_pairing(PairingSetup {
        fingerprint: tomori_security::Fingerprint::of(b"certificate"),
        tokens: store.tokens.clone(),
    });
    hub.set_encrypted(true);
    let device = Client::greeted(&hub, Access::pairing(), "quest");
    (hub, device)
}

/// Begin pairing and reveal the device's value, as a device does.
fn pair(device: &Client) {
    let nonce = tomori_security::pairing::Nonce::generate();
    device.send(ClientMessage::PairBegin {
        commitment: nonce.commitment(),
    });
    device.read();
    device.send(ClientMessage::PairReveal {
        nonce: nonce.to_hex(),
    });
    device.read();
}

#[test]
fn a_pairing_is_confirmed_only_once_its_code_is_shown() {
    let store = Store::new("unshown");
    let (hub, device) = pairing(&store);
    assert_eq!(
        hub.confirm_pairing(Confirmation::Shown(1)),
        Err("No pairing is waiting.")
    );
    device.send(ClientMessage::PairBegin {
        commitment: tomori_security::pairing::Nonce::generate().commitment(),
    });
    device.read();
    assert_eq!(
        hub.confirm_pairing(Confirmation::Shown(1)),
        Err("The device has not shown its code yet.")
    );
    // The device may cancel it.
    device.send(ClientMessage::PairCancel);
    let failed: Vec<_> = device
        .read()
        .into_iter()
        .filter_map(|message| match message {
            ServerMessage::PairFailed { error } => Some(error),
            _ => None,
        })
        .collect();
    assert_eq!(failed, ["The pairing was cancelled."]);
    assert!(hub.pending_pairing().is_none());
}

#[test]
fn a_pairing_whose_token_cannot_be_issued_fails_and_says_so() {
    let store = Store::new("unpaired");
    let (hub, device) = pairing(&store);
    pair(&device);
    let serial = hub.pending_pairing().unwrap().serial;
    store.refuse_writes();
    assert_eq!(
        hub.confirm_pairing(Confirmation::Shown(serial)),
        Err("The token could not be issued.")
    );
    let messages = device.read();
    assert!(
        matches!(&messages[..], [ServerMessage::PairFailed { error }] if error == "The token could not be issued. Try again."),
        "{messages:?}"
    );
}

#[test]
fn pairings_begun_long_ago_no_longer_count_against_beginning_another() {
    let store = Store::new("window");
    let (hub, device) = pairing(&store);
    {
        let mut state = hub.lock();
        let long_ago = Instant::now() - PAIRING_WINDOW;
        state.pairing_attempts = std::iter::repeat_n(long_ago, PAIRING_ATTEMPTS).collect();
    }
    device.send(ClientMessage::PairBegin {
        commitment: tomori_security::pairing::Nonce::generate().commitment(),
    });
    device.read();
    assert!(hub.pending_pairing().is_some());
    assert_eq!(hub.lock().pairing_attempts.len(), 1);
}

fn speech(segment: u32) -> ServerMessage {
    ServerMessage::Speech {
        segment,
        message: "message-1".to_owned(),
        text: "Hello.".to_owned(),
        sample_rate: 16_000,
        replay: false,
    }
}

fn frame(samples: usize) -> AudioFrame {
    AudioFrame {
        gate: Gate::Agent,
        source: 0,
        sequence: 0,
        timestamp: 0,
        sample_rate: 16_000,
        samples: vec![0; samples],
    }
}

#[test]
fn instructions_given_twice_before_the_agent_takes_them_are_given_once() {
    let (hub, _jobs) = hub();
    let (id, _commands) = running(
        &hub,
        &CreateRequest {
            instructions: "Mind the docs.".to_owned(),
            ..request("nobody")
        },
        Snapshot::default(),
    );
    hub.instructions_due(&id);
    let owner = Client::greeted(&hub, Access::open(), "ada");
    owner.join(&id);
    for text in ["One.", "Two."] {
        owner.send(ClientMessage::SetInstructions {
            text: text.to_owned(),
        });
    }
    let (entries, all) = hub.instructions_due(&id);
    assert!(!all);
    assert_eq!(entries.len(), 1);
}

#[test]
fn a_replay_asked_for_wakes_no_pacer_where_there_is_none() {
    let (hub, _jobs) = hub();
    let id = stopped(&hub, Snapshot::default());
    let client = Client::greeted(&hub, Access::open(), "ada");
    client.join(&id);
    hub.agent_speech(&id, 1, speech(1));
    hub.agent_audio(&id, 1, &frame(320));
    hub.agent_gap(&id, 1);
    client.read();
    client.send(ClientMessage::Replay { message: None });
    assert!(client.errors().is_empty());
    // Waking one set for a space that has gone does nothing.
    hub.set_speech_waker("gone", Box::new(|| {}));
}

#[test]
fn an_agent_frame_too_large_to_send_is_dropped() {
    let (hub, _jobs) = hub();
    let id = stopped(&hub, Snapshot::default());
    let client = Client::greeted(&hub, Access::open(), "ada");
    client.join(&id);
    hub.agent_audio(&id, 1, &frame(tomori_protocol::MAX_FRAME_BYTES));
    assert!(matches!(client.outbox.pop(), Popped::Empty));
}

#[test]
fn what_cannot_be_said_to_someone_who_has_gone_is_not_said() {
    let (hub, _jobs) = hub();
    hub.create_failed("nobody", "failed".to_owned());
    let client = Client::greeted(&hub, Access::open(), "ada");
    hub.disconnect(client.connection);
    hub.sessions("/ada", Ok(Vec::new()));
    hub.handle(client.connection, ClientMessage::Ping { nonce: 1 });
    hub.handle(client.connection, ClientMessage::ListSpaces);
    hub.receive(client.connection, Frame::audio(&frame(1)));
    assert!(client.read().is_empty());
}

#[test]
fn a_session_list_that_failed_is_said_as_an_error() {
    let (hub, _jobs) = hub();
    let client = Client::greeted(&hub, Access::open(), "ada");
    hub.sessions("/ada", Err("unreachable agent".to_owned()));
    assert_eq!(client.errors(), ["unreachable agent"]);
}

#[test]
fn the_status_lists_those_connected_in_the_lobby_by_what_they_were_admitted_as() {
    let (hub, _jobs) = hub();
    let collaborator = Access {
        principal: Some("token:abc".to_owned()),
        grant: Grant::Collaborator {
            space: "elsewhere".to_owned(),
        },
    };
    Client::greeted(&hub, collaborator, "cy");
    let gone = Client::greeted(&hub, Access::open(), "ada");
    hub.disconnect(gone.connection);
    assert_eq!(
        hub.status().lobby,
        [("cy".to_owned(), ConnectionRole::Collaborator)]
    );
}

#[test]
fn utterances_are_ended_or_cancelled_only_while_open() {
    let (hub, _jobs) = hub();
    let client = Client::greeted(&hub, Access::open(), "ada");
    let begin = ClientMessage::UtteranceBegin {
        utterance: "one".to_owned(),
        answer: None,
    };
    client.send(begin.clone());
    assert_eq!(client.errors(), ["Join a space first."]);
    let id = stopped(&hub, Snapshot::default());
    client.join(&id);
    client.send(ClientMessage::UtteranceCancel {
        utterance: "one".to_owned(),
    });
    assert_eq!(client.errors(), ["That utterance is not open."]);
    // One that failed is forgotten without an error when the client cancels it before it heard.
    client.send(begin);
    let utterance = *hub.lock().utterances.keys().next().unwrap();
    hub.transcript_failed(utterance, "lost");
    client.send(ClientMessage::UtteranceCancel {
        utterance: "one".to_owned(),
    });
    let messages = client.read();
    assert!(
        matches!(&messages[..], [ServerMessage::TranscriptFailed { .. }]),
        "{messages:?}"
    );
}

#[test]
fn an_utterance_transcribed_before_it_ended_is_still_ended() {
    let (hub, _jobs) = hub();
    let id = stopped(&hub, Snapshot::default());
    let client = Client::greeted(&hub, Access::open(), "ada");
    client.join(&id);
    client.send(ClientMessage::UtteranceBegin {
        utterance: "one".to_owned(),
        answer: None,
    });
    let utterance = *hub.lock().utterances.keys().next().unwrap();
    hub.transcript_finished(utterance, "Deploy it.");
    client.send(ClientMessage::UtteranceEnd {
        utterance: "one".to_owned(),
        pointed: None,
    });
    client.read();
    assert!(
        hub.lock().connections[&client.connection]
            .utterance
            .is_none()
    );
}

#[test]
fn a_decision_names_an_option_only_to_grant_it() {
    let (hub, _jobs) = hub();
    let id = stopped(
        &hub,
        Snapshot {
            permissions: vec![permission()],
            ..Snapshot::default()
        },
    );
    let client = Client::greeted(&hub, Access::open(), "ada");
    client.join(&id);
    for (decision, option, error) in [
        (
            tomori_protocol::Decision::Grant,
            None,
            "Choose an option to grant.",
        ),
        (
            tomori_protocol::Decision::Deny,
            Some("always"),
            "Only a grant decision takes an option.",
        ),
    ] {
        client.send(ClientMessage::Decide {
            request: "request-1".to_owned(),
            decision,
            option: option.map(str::to_owned),
        });
        assert_eq!(client.errors(), [error]);
    }
}

#[test]
fn an_owner_in_the_lobby_is_refused_what_needs_a_space() {
    let (hub, _jobs) = hub();
    let client = Client::greeted(&hub, Access::open(), "ada");
    client.send(ClientMessage::SetInstructions {
        text: "Mind the docs.".to_owned(),
    });
    assert_eq!(client.errors(), ["Join a space first."]);
    client.send(ClientMessage::SetSpeech {
        speech: SpeechSetting::Brief,
    });
    assert_eq!(client.errors(), ["Join a space first."]);
}

#[test]
fn an_engagement_reported_again_changes_nothing() {
    let (hub, _jobs) = hub();
    let id = stopped(&hub, Snapshot::default());
    let client = Client::greeted(&hub, Access::open(), "ada");
    client.join(&id);
    let away = || ClientMessage::Engagement {
        state: Engagement::Away,
    };
    client.send(away());
    client.read();
    client.send(away());
    assert!(client.read().is_empty());
}

#[test]
fn a_visitors_audio_is_neither_relayed_nor_transcribed() {
    let (hub, _jobs) = hub();
    let id = stopped(&hub, Snapshot::default());
    let owner = Client::greeted(&hub, Access::open(), "ada");
    owner.join(&id);
    let visitor = Client::greeted(&hub, Access::visitor(), "vic");
    visitor.join(&id);
    owner.read();
    hub.receive(visitor.connection, Frame::audio(&frame(320)));
    assert!(matches!(owner.outbox.pop(), Popped::Empty));
}

#[test]
fn a_transcript_comes_to_nothing_for_an_utterance_unknown_unheard_or_left_behind() {
    let (hub, _jobs) = hub_waiting();
    hub.transcript_finished(UtteranceId(99), "Deploy it.");
    let id = stopped(&hub, Snapshot::default());
    let client = Client::greeted(&hub, Access::open(), "ada");
    client.join(&id);
    let spoken = |text: &str| {
        client.send(ClientMessage::UtteranceBegin {
            utterance: "one".to_owned(),
            answer: None,
        });
        client.send(ClientMessage::UtteranceEnd {
            utterance: "one".to_owned(),
            pointed: None,
        });
        let utterance = *hub.lock().utterances.keys().next().unwrap();
        client.read();
        (utterance, text.to_owned())
    };
    let failure = |client: &Client| {
        client.read().into_iter().find_map(|message| match message {
            ServerMessage::TranscriptFailed { error, .. } => Some(error),
            _ => None,
        })
    };
    let (utterance, text) = spoken("  ");
    hub.transcript_finished(utterance, &text);
    assert_eq!(
        failure(&client).as_deref(),
        Some("Nothing was heard, so nothing was sent.")
    );
    let (utterance, text) = spoken("Deploy it.");
    client.send(ClientMessage::Leave);
    client.read();
    hub.transcript_finished(utterance, &text);
    assert_eq!(
        failure(&client).as_deref(),
        Some("You left the space before the transcript was ready, so nothing was sent.")
    );
}

#[test]
fn only_the_latest_spoken_answers_are_remembered() {
    let (hub, _jobs) = hub_waiting();
    let (id, _commands) = running(
        &hub,
        &request("nobody"),
        Snapshot {
            questions: vec![question()],
            ..Snapshot::default()
        },
    );
    let client = Client::greeted(&hub, Access::open(), "ada");
    client.join(&id);
    {
        let mut state = hub.lock();
        let space = state.space_mut(&id).unwrap();
        space.answered = (0..ANSWERED)
            .map(|index| (format!("1/old-{index}"), "old".to_owned()))
            .collect();
    }
    client.send(ClientMessage::UtteranceBegin {
        utterance: "one".to_owned(),
        answer: Some(tomori_protocol::AnswerTarget {
            question: "question-1".to_owned(),
            field: "field-1".to_owned(),
        }),
    });
    client.send(ClientMessage::UtteranceEnd {
        utterance: "one".to_owned(),
        pointed: None,
    });
    let utterance = *hub.lock().utterances.keys().next().unwrap();
    hub.transcript_finished(utterance, "Ada");
    let state = hub.lock();
    let answered = &state.space(&id).unwrap().answered;
    assert_eq!(answered.len(), ANSWERED);
    assert_eq!(answered.back().unwrap().1, "Ada");
    assert_eq!(answered.front().unwrap().0, "1/old-1");
}

#[test]
fn a_space_left_renders_nothing_for_it() {
    let (hub, _jobs) = hub();
    let id = stopped(&hub, Snapshot::default());
    let client = Client::greeted(&hub, Access::open(), "ada");
    client.join(&id);
    client.send(ClientMessage::Leave);
    assert!(hub.render(client.connection, Outgoing::Space).is_none());
}

#[test]
fn nothing_is_bound_once_the_server_is_stopping() {
    let (hub, _jobs) = hub();
    let id = stopped(&hub, Snapshot::default());
    hub.stopping();
    assert!(!hub.bound(&id, "session".to_owned(), Snapshot::default()));
}

#[test]
fn spaces_kept_are_listed_once_and_a_store_that_refuses_them_is_said() {
    let directory = std::env::temp_dir().join(format!("tomori-hub-kept-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let store = Arc::new(crate::kept::SpaceStore::open(&directory).unwrap());
    let (hub, _jobs) = hub();
    assert!(hub.restore(store.clone()).is_empty());
    let id = stopped(&hub, Snapshot::default());
    assert_eq!(store.spaces().len(), 1);
    // Restored again, the space already listed is not listed twice.
    assert!(hub.restore(store.clone()).is_empty());
    assert_eq!(hub.status().spaces.len(), 1);
    // A store that cannot be written keeps what it had, and says why.
    log::set_max_level(log::LevelFilter::Trace);
    std::fs::create_dir(directory.join("spaces.tmp")).unwrap();
    stopped(&hub, Snapshot::default());
    assert_eq!(store.spaces().len(), 1);
    assert_eq!(store.spaces()[0].id, id);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn words_for_a_driver_that_has_stopped_are_marked_unsent() {
    let (hub, _jobs) = hub_waiting();
    let id = stopped(
        &hub,
        Snapshot {
            busy: Some(false),
            ..Snapshot::default()
        },
    );
    let client = Client::greeted(&hub, Access::open(), "ada");
    client.join(&id);
    client.send(ClientMessage::UtteranceBegin {
        utterance: "one".to_owned(),
        answer: None,
    });
    let utterance = *hub.lock().utterances.keys().next().unwrap();
    hub.transcript_partial(UtteranceId(99), "nobody");
    hub.transcript_finished(utterance, "Deploy it.");
    let state = hub.lock().space(&id).unwrap().model.state(Vec::new(), "");
    assert_eq!(state.queue.len(), 1);
    assert_eq!(state.queue[0].state, tomori_protocol::QueueState::Failed);
}

#[test]
fn a_visitor_admitted_after_leaving_for_the_lobby_is_admitted_there() {
    let store = Store::new("lobby");
    let (hub, id, _owner, visitor) = knocked(&store);
    let knock = take_knock(&hub);
    visitor.send(ClientMessage::Leave);
    visitor.read();
    hub.admit(knock);
    assert!(
        visitor
            .read()
            .iter()
            .any(|message| matches!(message, ServerMessage::Admitted { .. }))
    );
    assert!(hub.lock().space(&id).unwrap().model.visitors.is_empty());
}

#[test]
fn a_question_of_several_fields_is_answered_one_field_at_a_time() {
    let (hub, _jobs) = hub();
    let mut asked = question();
    asked.fields.push(tomori_backend::Field {
        key: "field-2".to_owned(),
        ..asked.fields[0].clone()
    });
    let (id, commands) = running(
        &hub,
        &request("nobody"),
        Snapshot {
            questions: vec![asked],
            ..Snapshot::default()
        },
    );
    let client = Client::greeted(&hub, Access::open(), "ada");
    client.join(&id);
    client.send(ClientMessage::Answer {
        question: "question-1".to_owned(),
        field: "field-1".to_owned(),
        text: "Ada".to_owned(),
    });
    assert!(client.errors().is_empty());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok())
            .all(|command| !matches!(command, SpaceCommand::Answer { .. }))
    );
}

#[test]
fn the_console_is_prompted_as_a_pairing_begins() {
    let store = Store::new("prompted");
    let (hub, device) = pairing(&store);
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = events.clone();
    hub.set_pairing_prompt(Box::new(move |event: &PairingEvent| {
        seen.lock().unwrap().push(event.clone())
    }));
    pair(&device);
    assert!(matches!(
        &events.lock().unwrap()[..],
        [PairingEvent::Began { .. }, PairingEvent::Waiting { .. }]
    ));
}

#[test]
fn a_participant_in_the_lobby_is_refused_a_decision_and_their_silence_is_not_relayed() {
    let (hub, _jobs) = hub();
    let client = Client::greeted(&hub, Access::open(), "ada");
    client.send(ClientMessage::Decide {
        request: "request-1".to_owned(),
        decision: tomori_protocol::Decision::AllowOnce,
        option: None,
    });
    assert_eq!(client.errors(), ["Join a space first."]);
    let id = stopped(&hub, Snapshot::default());
    let other = Client::greeted(&hub, Access::open(), "bob");
    other.join(&id);
    client.join(&id);
    other.read();
    hub.receive(
        client.connection,
        Frame::audio(&AudioFrame {
            gate: Gate::Off,
            ..frame(320)
        }),
    );
    assert!(matches!(other.outbox.pop(), Popped::Empty));
}

#[test]
fn a_connection_that_never_said_hello_renders_no_space() {
    let (hub, _jobs) = hub();
    let client = Client::connect(&hub, Access::open());
    assert!(hub.render(client.connection, Outgoing::Space).is_none());
}

#[test]
fn audio_from_the_lobby_goes_nowhere() {
    let (hub, _jobs) = hub();
    let client = Client::greeted(&hub, Access::open(), "ada");
    hub.receive(client.connection, Frame::audio(&frame(320)));
    assert!(client.read().is_empty());
}
