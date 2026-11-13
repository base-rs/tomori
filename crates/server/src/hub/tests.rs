// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The hub's commoner paths, driven through its calls: a request that fails holding the state, the welcome, instructions, counts and attention in the lobby listing, visitors remembered, and a connection whose output is not accepted.

use super::*;
use crate::transcriber::NullTranscriber;

fn hub() -> Arc<Hub> {
    // Every log line's arguments are evaluated, so that what each would say is exercised; nothing is written, as no logger is installed.
    log::set_max_level(log::LevelFilter::Trace);
    let (jobs, _) = async_channel::unbounded();
    Hub::new(
        Backends::new(),
        Arc::new(NullTranscriber::new()),
        jobs,
        Vec::new(),
        SpaceSlots::new(crate::workspace::DEFAULT_MAX_SPACES),
    )
}

#[test]
fn a_request_that_fails_holding_the_state_fails_alone() {
    let hub = hub();
    let failing = hub.clone();
    let failed = std::thread::spawn(move || {
        let _state = failing.lock();
        panic!("a request fails holding the hub's state");
    })
    .join();
    assert!(failed.is_err());
    assert!(hub.state.is_poisoned());
    // Later requests are served, from the state as it was left.
    assert_eq!(hub.counts().spaces, 0);
    assert!(!hub.state.is_poisoned());
}

#[cfg(feature = "fake")]
#[test]
fn the_welcome_says_how_long_an_utterance_may_be() {
    use crate::test_support::{TestClient, TestServer};
    let welcome = |server: &TestServer| {
        let mut client = TestClient::open(server.address);
        client.send(&ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            participant: "ada".into(),
            name: "Ada".into(),
            names: false,
            environments: None,
            environment_state: false,
            projection: false,
            pairing_window: false,
            speech_held: false,
            images: false,
            diagrams: false,
            world_edges: false,
            projection_patches: false,
            live_panels: false,
            spawns: false,
            limits: None,
        });
        // The welcome is the first reply to hello.
        client.wait_for(|message| {
            let ServerMessage::Welcome {
                utterance_limit, ..
            } = message
            else {
                unreachable!("the welcome comes first, not {message:?}");
            };
            Some(*utterance_limit)
        })
    };
    // Recognition without a limit gives none.
    let server = TestServer::start();
    assert_eq!(welcome(&server), None);
    let server = TestServer::with_server(crate::fake::FakeBackend::new(), |backends| {
        crate::Server::new(
            backends,
            Arc::new(crate::test_support::TextTranscriber::limited(
                std::time::Duration::from_secs(60),
            )),
        )
    });
    assert_eq!(welcome(&server), Some(60));
}

#[cfg(feature = "fake")]
#[test]
fn instructions_queue_once_and_are_read_when_given() {
    let (jobs, _) = async_channel::unbounded();
    let hub = Hub::new(
        Backends::new().with(crate::fake::FakeBackend::new()),
        Arc::new(NullTranscriber::new()),
        jobs,
        Vec::new(),
        SpaceSlots::new(crate::workspace::DEFAULT_MAX_SPACES),
    );
    let (commands, receiver) = async_channel::unbounded();
    let request = CreateRequest {
        title: "Space".to_owned(),
        backend: "fake".to_owned(),
        directory: "/tmp".to_owned(),
        session: None,
        creator: "ada".to_owned(),
        creator_space: None,
        environment: "sky".to_owned(),
        instructions: "Mind the docs.".to_owned(),
    };
    let id = hub
        .register_space(
            &request,
            "session".to_owned(),
            Snapshot::default(),
            commands,
        )
        .unwrap();
    // Changes made while the command given on creation waits join it.
    for _ in 0..3 {
        hub.lock().space(&id).unwrap().instruct(false);
    }
    assert_eq!(receiver.len(), 1);
    let (entries, all) = hub.instructions_due(&id);
    assert!(all);
    assert_eq!(entries.len(), 3);
    // Once it is performed, a change of the space's own layer gives that alone, as it is when performed.
    hub.lock().space(&id).unwrap().instruct(false);
    hub.lock()
        .space_mut(&id)
        .unwrap()
        .model
        .set_space_instructions("Mind the tests.")
        .unwrap();
    assert_eq!(receiver.len(), 2);
    assert_eq!(
        hub.instructions_due(&id),
        (
            vec![(
                crate::instructions::SPACE_KEY.to_owned(),
                Some("Mind the tests.".to_owned())
            )],
            false
        )
    );
}

/// Connect and say hello as `participant`.
fn greet(hub: &Hub, participant: &str) -> ConnectionId {
    let (connection, ..) = hub.connect(Cancellation::new(), Access::open()).unwrap();
    hub.handle(
        connection,
        ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            participant: participant.to_owned(),
            name: participant.to_owned(),
            names: false,
            environments: None,
            environment_state: false,
            projection: false,
            pairing_window: false,
            speech_held: false,
            images: false,
            diagrams: false,
            world_edges: false,
            projection_patches: false,
            live_panels: false,
            spawns: false,
            limits: None,
        },
    );
    connection
}

/// Register a space created by `creator`, as the supervisor does once its session has started.
fn create(hub: &Hub, creator: &str) -> String {
    let (commands, _) = async_channel::unbounded();
    let request = CreateRequest {
        title: "Space".to_owned(),
        backend: "fake".to_owned(),
        directory: "/tmp".to_owned(),
        session: None,
        creator: creator.to_owned(),
        creator_space: None,
        environment: "sky".to_owned(),
        instructions: String::new(),
    };
    hub.register_space(
        &request,
        "session".to_owned(),
        Snapshot::default(),
        commands,
    )
    .expect("the hub is not shutting down")
}

fn counts(spaces: u32, participants: u32) -> Counts {
    Counts {
        spaces,
        participants,
        attention: Some(AttentionCounts::default()),
    }
}

/// `counts` with spaces needing attention.
fn needing(counts: Counts, waiting: u32, problems: u32, replies: u32) -> Counts {
    Counts {
        attention: Some(AttentionCounts {
            waiting,
            problems,
            replies,
        }),
        ..counts
    }
}

/// Whether `action` notified a counts listener.
fn notifies(hub: &Hub, action: impl FnOnce()) -> bool {
    let listener = hub.listen_counts();
    action();
    futures_lite::future::block_on(futures_lite::future::poll_once(listener)).is_some()
}

#[test]
fn visitors_are_not_counted_as_participants() {
    let hub = hub();
    let (visitor, ..) = hub.connect(Cancellation::new(), Access::visitor()).unwrap();
    hub.handle(
        visitor,
        ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            participant: "vic".to_owned(),
            name: "Vic".to_owned(),
            names: false,
            environments: None,
            environment_state: false,
            projection: false,
            pairing_window: false,
            speech_held: false,
            images: false,
            diagrams: false,
            world_edges: false,
            projection_patches: false,
            live_panels: false,
            spawns: false,
            limits: None,
        },
    );
    assert_eq!(hub.counts(), counts(0, 0));
    greet(&hub, "ada");
    assert_eq!(hub.counts(), counts(0, 1));

    // The limit applies to visitors only.
    hub.set_max_visitors(1);
    assert!(
        hub.connect(Cancellation::new(), Access::visitor())
            .is_none()
    );
    assert!(hub.connect(Cancellation::new(), Access::open()).is_some());
    hub.disconnect(visitor);
    assert!(
        hub.connect(Cancellation::new(), Access::visitor())
            .is_some()
    );
}

#[test]
fn only_so_many_disconnected_visitors_are_remembered() {
    let hub = hub();
    let visit = |identity: &str| {
        let (connection, ..) = hub.connect(Cancellation::new(), Access::visitor()).unwrap();
        hub.handle(
            connection,
            ClientMessage::Hello {
                protocol: PROTOCOL_VERSION,
                participant: identity.to_owned(),
                name: identity.to_owned(),
                names: false,
                environments: None,
                environment_state: false,
                projection: false,
                pairing_window: false,
                speech_held: false,
                images: false,
                diagrams: false,
                world_edges: false,
                projection_patches: false,
                live_panels: false,
                spawns: false,
                limits: None,
            },
        );
        connection
    };
    let owner = greet(&hub, "ada");
    hub.disconnect(owner);
    for index in 0..=RETAINED_VISITORS {
        let connection = visit(&format!("visitor-{index}"));
        hub.disconnect(connection);
    }
    let state = hub.lock();
    let visitors = state
        .participants
        .values()
        .filter(|record| record.visitor)
        .count();
    assert_eq!(visitors, RETAINED_VISITORS);
    // The first to leave was forgotten; the owner, not a visitor, is kept for the whole retention.
    assert!(!state.participants.contains_key("/visitor-0"));
    assert!(state.participants.contains_key("/visitor-1"));
    assert!(state.participants.contains_key("/ada"));
}

#[test]
fn counts_follow_connections_and_listed_spaces() {
    let hub = hub();
    assert_eq!(hub.counts(), counts(0, 0));

    // A connection counts once it has said hello.
    let (silent, ..) = hub.connect(Cancellation::new(), Access::open()).unwrap();
    assert_eq!(hub.counts(), counts(0, 0));
    let mut ada = 0;
    assert!(notifies(&hub, || ada = greet(&hub, "ada")));
    assert_eq!(hub.counts(), counts(0, 1));
    let bob = greet(&hub, "bob");
    assert_eq!(hub.counts(), counts(0, 2));
    hub.disconnect(silent);

    let mut space = String::new();
    assert!(notifies(&hub, || space = create(&hub, "ada")));
    assert_eq!(hub.counts(), counts(1, 2));

    // Moving between a space and the lobby changes neither count.
    assert!(!notifies(&hub, || {
        hub.handle(
            bob,
            ClientMessage::Join {
                space: space.clone(),
            },
        );
        hub.handle(bob, ClientMessage::Leave);
    }));
    assert_eq!(hub.counts(), counts(1, 2));

    // A reconnect replaces the participant's connection; it does not add one.
    let again = greet(&hub, "ada");
    assert_eq!(hub.counts(), counts(1, 2));
    hub.disconnect(ada);
    assert_eq!(hub.counts(), counts(1, 2));

    assert!(notifies(&hub, || hub.disconnect(again)));
    assert_eq!(hub.counts(), counts(1, 1));

    // A closed space is still listed, so it still counts. Nobody was in it when it closed, so it has a problem.
    assert!(notifies(&hub, || hub.space_ended(&space, None)));
    assert_eq!(hub.counts(), needing(counts(1, 1), 0, 1, 0));
    hub.disconnect(bob);
    assert_eq!(hub.counts(), needing(counts(1, 0), 0, 1, 0));
}

/// The lobby listing `connection` would be sent.
fn listing(hub: &Hub, connection: ConnectionId) -> Vec<tomori_protocol::SpaceSummary> {
    let Some(ServerMessage::Spaces { spaces }) = hub.render(connection, Outgoing::Spaces) else {
        unreachable!("the lobby is rendered as a listing");
    };
    spaces
}

#[test]
fn attention_is_listed_and_counted_until_someone_joins() {
    let hub = hub();
    let ada = greet(&hub, "ada");
    let space = create(&hub, "nobody");
    let quiet = listing(&hub, ada);
    assert_eq!((quiet[0].attention, quiet[0].attention_age), (None, None));

    // An approval while nobody is there, then the agent finishing a reply.
    let permission = tomori_backend::Permission {
        id: "p".into(),
        tool: "Bash".into(),
        input: serde_json::Value::Null,
        description: None,
        options: Vec::new(),
    };
    assert!(notifies(&hub, || hub
        .drive_event(&space, Event::PermissionRequested(permission))));
    assert_eq!(hub.counts(), needing(counts(1, 1), 1, 0, 0));
    let listed = listing(&hub, ada);
    assert_eq!(listed[0].attention, Some(Attention::Input));
    assert_eq!(listed[0].attention_age, Some(0));

    // Its age counts from when it changed level, as stamped when participants are told.
    let record = |hub: &Hub| {
        let state = hub.lock();
        let since = state.space(&space).unwrap().attention.get().unwrap();
        (
            since.0,
            state
                .space(&space)
                .unwrap()
                .summary(since.1 + Duration::from_secs(90), 0, &[]),
        )
    };
    let (level, later) = record(&hub);
    assert_eq!(level, Attention::Input);
    assert_eq!(later.attention_age, Some(90));

    hub.drive_event(&space, Event::PermissionResolved { id: "p".into() });
    for event in [
        Event::Busy,
        Event::MessageStarted {
            message: "m".into(),
            role: tomori_backend::Role::Assistant,
        },
        Event::TextDelta {
            message: "m".into(),
            text: "Done.".into(),
        },
        Event::MessageCompleted {
            message: "m".into(),
        },
        Event::Idle,
    ] {
        hub.drive_event(&space, event);
    }
    // A deferred snapshot stamps the new level once it is sent.
    hub.flush_snapshots(Instant::now() + SNAPSHOT_INTERVAL);
    assert_eq!(hub.counts(), needing(counts(1, 1), 0, 0, 1));
    assert_eq!(listing(&hub, ada)[0].attention, Some(Attention::Reply));
    assert_eq!(record(&hub).0, Attention::Reply);

    // Joining clears it for everyone.
    assert!(notifies(&hub, || hub.handle(
        ada,
        ClientMessage::Join {
            space: space.clone()
        }
    )));
    assert_eq!(hub.counts(), counts(1, 1));
    let bob = greet(&hub, "bob");
    assert_eq!(listing(&hub, bob)[0].attention, None);
}

#[test]
fn being_put_back_in_a_space_on_reconnecting_does_not_clear_its_attention() {
    let hub = hub();
    let ada = greet(&hub, "ada");
    let space = create(&hub, &participant_key(None, "ada"));
    hub.disconnect(ada);
    // While Ada is away, the agent replies.
    for event in [
        Event::Busy,
        Event::MessageStarted {
            message: "m".into(),
            role: tomori_backend::Role::Assistant,
        },
        Event::TextDelta {
            message: "m".into(),
            text: "Done.".into(),
        },
        Event::MessageCompleted {
            message: "m".into(),
        },
        Event::Idle,
    ] {
        hub.drive_event(&space, event);
    }
    assert_eq!(hub.counts(), needing(counts(1, 0), 0, 0, 1));
    // Reconnecting puts her back in the space, which she has not seen yet; a jump would move on from it.
    let ada = greet(&hub, "ada");
    assert_eq!(
        hub.lock().space(&space).unwrap().model.participants.len(),
        1
    );
    assert_eq!(hub.counts(), needing(counts(1, 1), 0, 0, 1));
    // A visitor watching it does not clear it.
    let (visitor, ..) = hub.connect(Cancellation::new(), Access::visitor()).unwrap();
    hub.handle(
        visitor,
        ClientMessage::Hello {
            protocol: PROTOCOL_VERSION,
            participant: "vic".to_owned(),
            name: "Vic".to_owned(),
            names: false,
            environments: None,
            environment_state: false,
            projection: false,
            pairing_window: false,
            speech_held: false,
            images: false,
            diagrams: false,
            world_edges: false,
            projection_patches: false,
            live_panels: false,
            spawns: false,
            limits: None,
        },
    );
    hub.handle(
        visitor,
        ClientMessage::Join {
            space: space.clone(),
        },
    );
    assert_eq!(hub.lock().space(&space).unwrap().model.visitors.len(), 1);
    assert_eq!(hub.counts(), needing(counts(1, 1), 0, 0, 1));
    // Choosing to join it, as her client does to restore its membership, clears it.
    hub.handle(
        ada,
        ClientMessage::Join {
            space: space.clone(),
        },
    );
    assert_eq!(hub.counts(), counts(1, 1));
}

#[test]
fn forgotten_closed_spaces_are_no_longer_counted() {
    let hub = hub();
    let spaces: Vec<String> = (0..=CLOSED_SPACES)
        .map(|_| create(&hub, "nobody"))
        .collect();
    let listed = u32::try_from(spaces.len()).unwrap();
    assert_eq!(hub.counts(), counts(listed, 0));
    for space in &spaces[..CLOSED_SPACES] {
        hub.space_ended(space, None);
    }
    // Each closed while nobody was in it.
    let closed = u32::try_from(CLOSED_SPACES).unwrap();
    assert_eq!(hub.counts(), needing(counts(listed, 0), 0, closed, 0));
    // Closing one more than are kept forgets the oldest.
    assert!(notifies(&hub, || hub.space_ended(&spaces[CLOSED_SPACES], None)));
    assert_eq!(hub.counts(), needing(counts(listed - 1, 0), 0, closed, 0));
}

#[test]
fn a_participant_whose_space_was_forgotten_while_away_reconnects_to_the_listing() {
    let hub = hub();
    let ada = greet(&hub, "ada");
    let space = create(&hub, &participant_key(None, "ada"));
    hub.disconnect(ada);
    // Her space closes while she is away, and is forgotten as more spaces close than are kept.
    hub.space_ended(&space, None);
    for _ in 0..CLOSED_SPACES {
        let other = create(&hub, "nobody");
        hub.space_ended(&other, None);
    }
    assert!(!hub.has_space(&space));
    let ada = greet(&hub, "ada");
    let key = participant_key(None, "ada");
    assert_eq!(hub.lock().participants[&key].space, None);
    assert_eq!(hub.render(ada, Outgoing::Space), None);
    assert_eq!(listing(&hub, ada).len(), CLOSED_SPACES);
}

#[test]
fn a_connection_whose_output_is_not_accepted_is_released_even_while_it_sends() {
    let hub = hub();
    let keepalive = Keepalive {
        interval: Duration::from_secs(5),
        timeout: Duration::from_secs(15),
    };
    let hello = Duration::from_secs(3600);
    let blocked = Cancellation::new();
    let (_, _, blocked_received, blocked_output) =
        hub.connect(blocked.clone(), Access::open()).unwrap();
    let flowing = Cancellation::new();
    let (_, _, flowing_received, flowing_output) =
        hub.connect(flowing.clone(), Access::open()).unwrap();

    let start = Instant::now();
    // Both peers keep sending. One write is never accepted; the other connection's writes complete.
    blocked_output.handed_out();
    for second in 0..=20 {
        blocked_received.store(true, Ordering::Release);
        flowing_received.store(true, Ordering::Release);
        flowing_output.asked();
        flowing_output.handed_out();
        hub.sweep(start + Duration::from_secs(second), &keepalive, hello);
        assert_eq!(blocked.is_cancelled(), second >= 15, "after {second} s");
    }
    assert!(!flowing.is_cancelled());
}
