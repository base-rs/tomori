// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

mod raw;
mod server;

use std::net::{SocketAddr, TcpListener};
use std::time::{Duration, Instant};

use server::{ECHO_SOURCE, MiniServer, Mode, Received};
use tomori_client::{Backoff, Client, ConnectionState, Disconnect, Event, Keepalive, Options};
use tomori_protocol::{AudioFrame, ClientMessage, Gate, ServerMessage};

const TIMEOUT: Duration = Duration::from_secs(10);

fn options(address: SocketAddr) -> Options {
    let mut options = Options::new(address.to_string(), "quest-1", "Ada");
    options.backoff = Backoff {
        initial_delay: Duration::from_millis(20),
        delay_limit: Duration::from_millis(100),
        ..Backoff::default()
    };
    options.close_timeout = Duration::from_millis(500);
    options.incoming_audio_limit = 1024;
    options
}

fn localhost() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

/// Wait for the first event matching `predicate`, discarding others.
fn wait_for(client: &Client, mut predicate: impl FnMut(&Event) -> bool) -> Event {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let event = client
            .events()
            .recv_timeout(remaining)
            .expect("timed out waiting for an event");
        if predicate(&event) {
            return event;
        }
    }
}

fn wait_connected(client: &Client) -> u32 {
    match wait_for(client, |event| {
        matches!(event, Event::State(ConnectionState::Connected { .. }))
    }) {
        Event::State(ConnectionState::Connected {
            source,
            server,
            name,
            ..
        }) => {
            assert_eq!(server, "mini");
            // The name the server knows this participant by, rather than the one it suggested.
            assert_eq!(name.as_deref(), Some("Ada Lovelace"));
            source
        }
        _ => unreachable!(),
    }
}

fn wait_disconnected(client: &Client) -> Disconnect {
    match wait_for(client, |event| {
        matches!(event, Event::State(ConnectionState::Disconnected { .. }))
    }) {
        Event::State(ConnectionState::Disconnected { reason, .. }) => reason,
        _ => unreachable!(),
    }
}

fn audio(sequence: u32) -> AudioFrame {
    AudioFrame {
        gate: Gate::Live,
        source: 0,
        sequence,
        timestamp: u64::from(sequence) * 20_000,
        sample_rate: 16_000,
        samples: vec![sequence as i16; 320],
    }
}

fn join(space: &str) -> ClientMessage {
    ClientMessage::Join {
        space: space.into(),
    }
}

fn kick() -> ClientMessage {
    ClientMessage::Answer {
        question: "q".into(),
        field: "f".into(),
        text: "kick".into(),
    }
}

fn wait_for_space(client: &Client, id: &str) {
    wait_for(
        client,
        |event| matches!(event, Event::Message(ServerMessage::Space { state }) if state.id == id),
    );
}

#[test]
fn streams_control_and_audio_in_both_directions_at_once() {
    let server = MiniServer::start(localhost(), Mode::Normal);
    let client = Client::connect(options(server.address)).unwrap();
    assert_eq!(wait_connected(&client), 1);
    assert!(client.state().is_connected());

    // Each reply arrives while the request body is still open: the exchange is full duplex.
    client.send(join("alpha")).unwrap();
    wait_for_space(&client, "alpha");

    let count = 200;
    for sequence in 0..count {
        client.send_audio(audio(sequence)).unwrap();
        if sequence % 20 == 0 {
            client.send(ClientMessage::ListSpaces).unwrap();
        }
        // Pace like a microphone, more or less, so the congestion limit is not what this test measures.
        std::thread::sleep(Duration::from_millis(1));
    }
    // Congestion drops happen as frames are queued, so the count is final once sending ends.
    let dropped = client.sender().dropped_audio_frames();
    let mut received = Vec::new();
    let deadline = Instant::now() + TIMEOUT;
    while (received.len() as u64 + dropped) < u64::from(count) && Instant::now() < deadline {
        match client.try_audio() {
            Some(frame) => {
                assert_eq!(frame.source, ECHO_SOURCE);
                assert_eq!(frame.samples, vec![frame.sequence as i16; 320]);
                received.push(frame.sequence);
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    assert_eq!(received.len() as u64 + dropped, u64::from(count));
    assert!(received.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(dropped < u64::from(count) / 2, "dropped {dropped} frames");

    let mut spaces = 0;
    while let Some(event) = client.try_event() {
        if matches!(event, Event::Message(ServerMessage::Spaces { .. })) {
            spaces += 1;
        }
    }
    assert_eq!(spaces, 10);

    // Hello always comes first.
    assert!(matches!(
        server.received(1).first(),
        Some(Received::Message(ClientMessage::Hello { participant, name, names, .. })) if participant == "quest-1" && name == "Ada" && *names
    ));
}

#[test]
fn audio_attributed_to_this_client_is_never_delivered() {
    let server = MiniServer::start(localhost(), Mode::EchoToSender);
    let client = Client::connect(options(server.address)).unwrap();
    let source = wait_connected(&client);
    for sequence in 0..5 {
        client.send_audio(audio(sequence)).unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut received = Vec::new();
    let deadline = Instant::now() + TIMEOUT;
    while received.len() < 5 && Instant::now() < deadline {
        match client.try_audio() {
            Some(frame) => received.push((frame.source, frame.sequence)),
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    // Only the copies from another source arrive, and nothing follows them.
    std::thread::sleep(Duration::from_millis(100));
    while let Some(frame) = client.try_audio() {
        received.push((frame.source, frame.sequence));
    }
    assert_ne!(source, ECHO_SOURCE);
    let expected: Vec<_> = (0..5).map(|sequence| (ECHO_SOURCE, sequence)).collect();
    assert_eq!(received, expected);
}

#[test]
fn reconnects_and_restores_the_joined_space_and_gate() {
    let server = MiniServer::start(localhost(), Mode::Normal);
    let client = Client::connect(options(server.address)).unwrap();
    assert_eq!(wait_connected(&client), 1);
    client.send(join("alpha")).unwrap();
    client
        .send(ClientMessage::Gate { gate: Gate::Agent })
        .unwrap();
    wait_for_space(&client, "alpha");

    client.send(kick()).unwrap();
    assert_eq!(wait_disconnected(&client), Disconnect::Closed);
    assert_eq!(wait_connected(&client), 2);
    wait_for_space(&client, "alpha");

    let restored = server.received(2);
    assert!(matches!(
        &restored[..3],
        [
            Received::Message(ClientMessage::Hello { .. }),
            Received::Message(ClientMessage::Join { space }),
            Received::Message(ClientMessage::Gate { gate: Gate::Agent }),
        ] if space == "alpha"
    ));

    assert_eq!(
        client.sender().membership(),
        tomori_client::Membership {
            space: Some("alpha".into()),
            gate: Gate::Agent
        }
    );

    // Leaving, once the server confirms it, clears what is restored next time.
    client.send(ClientMessage::Leave).unwrap();
    wait_for(&client, |event| {
        matches!(event, Event::Message(ServerMessage::Left { .. }))
    });
    client.send(kick()).unwrap();
    assert_eq!(wait_connected(&client), 3);
    client.send(ClientMessage::ListSpaces).unwrap();
    wait_for(&client, |event| {
        matches!(event, Event::Message(ServerMessage::Spaces { .. }))
    });
    let restored = server.received(3);
    assert!(matches!(
        &restored[..3],
        [
            Received::Message(ClientMessage::Hello { .. }),
            Received::Message(ClientMessage::Gate { gate: Gate::Agent }),
            Received::Message(ClientMessage::ListSpaces),
        ]
    ));
}

#[test]
fn a_name_the_server_gives_is_suggested_when_reconnecting() {
    let server = MiniServer::start(localhost(), Mode::Normal);
    let client = Client::connect(options(server.address)).unwrap();
    assert_eq!(wait_connected(&client), 1);
    client
        .send(ClientMessage::SetName { name: "Vic".into() })
        .unwrap();
    wait_for(
        &client,
        |event| matches!(event, Event::Message(ServerMessage::Name { name }) if name == "Vic"),
    );
    client.send(kick()).unwrap();
    assert_eq!(wait_disconnected(&client), Disconnect::Closed);
    assert_eq!(wait_connected(&client), 2);
    // A server that keeps no name for this participant, as for a visitor, hears it again.
    assert!(matches!(
        server.received(2).first(),
        Some(Received::Message(ClientMessage::Hello { name, .. })) if name == "Vic"
    ));
}

#[test]
fn an_image_asked_for_arrives_whole_and_checked_and_one_that_is_not_what_its_hash_says_never_does()
{
    let server = MiniServer::start(localhost(), Mode::Normal);
    let client = Client::connect(options(server.address)).unwrap();
    assert_eq!(wait_connected(&client), 1);
    // The client says it shows image panels.
    assert!(matches!(
        server.received(1).first(),
        Some(Received::Message(ClientMessage::Hello { images: true, .. }))
    ));
    client
        .send(ClientMessage::FetchImage {
            image: server::BROKEN_IMAGE.into(),
        })
        .unwrap();
    let (hash, bytes) = server::image();
    client
        .send(ClientMessage::FetchImage {
            image: hash.clone(),
        })
        .unwrap();
    // The broken one is dropped, so the first image to arrive is the second asked for.
    let Event::Image { hash: arrived, png } =
        wait_for(&client, |event| matches!(event, Event::Image { .. }))
    else {
        unreachable!("an image");
    };
    assert_eq!((arrived, &png[..]), (hash, &bytes[..]));
}

#[test]
fn an_image_not_what_its_hash_says_is_dropped_and_the_connection_kept() {
    let server = MiniServer::start(localhost(), Mode::Normal);
    let client = Client::connect(options(server.address)).unwrap();
    assert_eq!(wait_connected(&client), 1);
    client
        .send(ClientMessage::Answer {
            question: "q".into(),
            field: "f".into(),
            text: "image".into(),
        })
        .unwrap();
    let event = wait_for(&client, |event| {
        matches!(
            event,
            Event::Image { .. }
                | Event::Message(ServerMessage::Name { .. })
                | Event::State(ConnectionState::Disconnected { .. })
        )
    });
    assert!(
        matches!(event, Event::Message(ServerMessage::Name { name }) if name == "After the image")
    );
}

#[test]
fn an_image_that_cannot_be_read_ends_the_connection_as_a_protocol_error() {
    let server = MiniServer::start(localhost(), Mode::Normal);
    let client = Client::connect(options(server.address)).unwrap();
    assert_eq!(wait_connected(&client), 1);
    client
        .send(ClientMessage::Answer {
            question: "q".into(),
            field: "f".into(),
            text: "bad image".into(),
        })
        .unwrap();
    assert!(matches!(
        wait_disconnected(&client),
        Disconnect::Protocol(reason) if reason.starts_with("invalid image frame")
    ));
}

#[test]
fn hello_lists_the_environments_the_client_presents_and_reads_their_state_on_every_connection() {
    let server = MiniServer::start(localhost(), Mode::Normal);
    let mut options = options(server.address);
    let presents = vec![tomori_protocol::EnvironmentVersion::new("observe", 2)];
    options.environments = Some(presents.clone());
    assert!(format!("{options:?}").contains("observe"));
    let client = Client::connect(options).unwrap();
    assert_eq!(wait_connected(&client), 1);
    client.send(kick()).unwrap();
    assert_eq!(wait_disconnected(&client), Disconnect::Closed);
    assert_eq!(wait_connected(&client), 2);
    for connection in [1, 2] {
        assert!(matches!(
            server.received(connection).first(),
            Some(Received::Message(ClientMessage::Hello { environments: Some(listed), environment_state: true, .. })) if *listed == presents
        ));
    }
}

#[test]
fn opted_in_microphone_does_not_resume_while_the_application_is_stalled() {
    let server = MiniServer::start(localhost(), Mode::Normal);
    let mut options = options(server.address);
    options.microphone_rearm_on_disconnect = true;
    let client = Client::connect(options).unwrap();
    assert_eq!(wait_connected(&client), 1);
    client.send(join("alpha")).unwrap();
    wait_for_space(&client, "alpha");
    client
        .send(ClientMessage::Gate { gate: Gate::Live })
        .unwrap();
    let sender = client.sender();
    let generation = sender.connection_generation();
    assert!(sender.connection_is_current(generation));
    assert!(sender.arm_microphone(generation));
    client.send_audio(audio(1)).unwrap();
    client.send(kick()).unwrap();

    // Do not drain connection events: simulate an application/render stall
    // spanning both disconnection and reconnection.
    let deadline = Instant::now() + TIMEOUT;
    while !matches!(client.state(), ConnectionState::Connected { source: 2, .. }) {
        assert!(Instant::now() < deadline, "client did not reconnect");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(client.sender().membership().gate, Gate::Off);
    assert!(!sender.connection_is_current(generation));
    assert!(!sender.arm_microphone(generation));
    client.send_audio(audio(2)).unwrap();
    client.send(ClientMessage::ListSpaces).unwrap();
    wait_for(&client, |event| {
        matches!(event, Event::Message(ServerMessage::Spaces { .. }))
    });
    assert!(!server.received(2).contains(&Received::Audio(2)));

    assert!(sender.arm_microphone(sender.connection_generation()));
    client
        .send(ClientMessage::Gate { gate: Gate::Live })
        .unwrap();
    // A capture still bound to the old connection cannot use the new arming.
    sender.send_audio_for(audio(4), Some(generation)).unwrap();
    client.send_audio(audio(3)).unwrap();
    let deadline = Instant::now() + TIMEOUT;
    while !server.received(2).contains(&Received::Audio(3)) {
        assert!(
            Instant::now() < deadline,
            "explicitly rearmed audio was not sent"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    // Frames are sent in order, so the earlier old-connection frame would
    // have arrived before this one.
    assert!(!server.received(2).contains(&Received::Audio(4)));
}

#[test]
fn retries_until_the_server_is_listening() {
    // Reserve a port, then release it so nothing is listening.
    let address = TcpListener::bind(localhost())
        .unwrap()
        .local_addr()
        .unwrap();
    let client = Client::connect(options(address)).unwrap();
    // Sent while disconnected: delivered once connected.
    client.send(join("beta")).unwrap();
    client.send(ClientMessage::ListSpaces).unwrap();
    let reason = wait_disconnected(&client);
    assert!(
        reason.to_string().starts_with("connection failed: "),
        "{reason}"
    );
    wait_for(&client, |event| {
        matches!(
            event,
            Event::State(ConnectionState::Connecting { attempt }) if *attempt > 1
        )
    });

    let server = MiniServer::start(address, Mode::Normal);
    wait_connected(&client);
    wait_for_space(&client, "beta");
    wait_for(&client, |event| {
        matches!(event, Event::Message(ServerMessage::Spaces { .. }))
    });
    drop(server);
}

#[test]
fn tries_each_resolved_address_in_turn() {
    // `localhost` can resolve to `::1` before `127.0.0.1`, where nothing listens on this port.
    let server = MiniServer::start(localhost(), Mode::Normal);
    let mut options = options(server.address);
    options.address = format!("localhost:{}", server.address.port());
    let client = Client::connect(options).unwrap();
    let first = wait_for(&client, |event| {
        matches!(
            event,
            Event::State(ConnectionState::Connected { .. } | ConnectionState::Disconnected { .. })
        )
    });
    assert!(
        matches!(first, Event::State(ConnectionState::Connected { .. })),
        "{first:?}"
    );
}

#[test]
fn closing_delivers_queued_control_messages() {
    let server = MiniServer::start(localhost(), Mode::Normal);
    let client = Client::connect(options(server.address)).unwrap();
    wait_connected(&client);
    let sender = client.sender();
    sender.send(ClientMessage::ListSpaces).unwrap();
    sender.send(ClientMessage::Leave).unwrap();
    let started = Instant::now();
    client.close();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(sender.state(), ConnectionState::Closed);
    assert_eq!(
        sender.send(ClientMessage::ListSpaces),
        Err(tomori_client::SendError::Closed)
    );
    let received = server.received(1);
    assert!(received.ends_with(&[
        Received::Message(ClientMessage::ListSpaces),
        Received::Message(ClientMessage::Leave),
    ]));
}

#[test]
fn reports_incompatible_servers() {
    let server = MiniServer::start(localhost(), Mode::WrongProtocol);
    let client = Client::connect(options(server.address)).unwrap();
    assert!(matches!(
        wait_disconnected(&client),
        Disconnect::Protocol(error) if error.contains(&format!("protocol {}", tomori_protocol::PROTOCOL_VERSION + 1))
    ));

    let server = MiniServer::start(localhost(), Mode::Unavailable);
    let mut client = Client::connect(options(server.address)).unwrap();
    let audio = client.take_audio().unwrap();
    assert!(client.take_audio().is_none());
    assert!(client.try_audio().is_none());
    assert_eq!(wait_disconnected(&client), Disconnect::Status(503));
    drop(client);
    assert!(audio.recv().is_err());
}

fn raw_disconnect(body: Option<Vec<u8>>, connect_timeout: Duration) -> (Disconnect, bool) {
    let mut options = options(raw::start(body));
    options.connect_timeout = connect_timeout;
    let client = Client::connect(options).unwrap();
    let mut connected = false;
    match wait_for(&client, |event| {
        if matches!(event, Event::State(ConnectionState::Connected { .. })) {
            connected = true;
        }
        matches!(event, Event::State(ConnectionState::Disconnected { .. }))
    }) {
        Event::State(ConnectionState::Disconnected { reason, .. }) => (reason, connected),
        _ => unreachable!(),
    }
}

#[test]
fn times_out_when_the_server_does_not_respond() {
    let (reason, connected) = raw_disconnect(None, Duration::from_millis(200));
    assert_eq!(reason, Disconnect::Timeout);
    assert!(!connected);

    // A response without a Welcome also times out.
    let (reason, connected) = raw_disconnect(Some(Vec::new()), Duration::from_millis(200));
    assert_eq!(reason, Disconnect::Timeout);
    assert!(!connected);
}

#[test]
fn requires_welcome_first_and_only_once() {
    let refusal = raw::message(&ServerMessage::Error {
        error: "full".into(),
        space: None,
        panel: None,
    });
    let (reason, _) = raw_disconnect(Some(refusal), TIMEOUT);
    assert_eq!(
        reason,
        Disconnect::Protocol("server refused the connection: full".into())
    );

    let (reason, _) = raw_disconnect(Some(raw::message(&ServerMessage::LEFT)), TIMEOUT);
    assert_eq!(
        reason,
        Disconnect::Protocol("expected welcome, received Left { deleted: None }".into())
    );

    let (reason, _) = raw_disconnect(Some(raw::chunk(b"not a frame")), TIMEOUT);
    assert_eq!(
        reason,
        Disconnect::Protocol(format!("unknown frame kind {}", b'n'))
    );

    // A first control frame that is not a message.
    let (reason, connected) = raw_disconnect(Some(raw::control(b"{")), TIMEOUT);
    assert!(
        reason
            .to_string()
            .starts_with("protocol error: invalid control message"),
        "{reason}"
    );
    assert!(!connected);

    // A later one.
    let mut body = raw::welcome();
    body.extend(raw::control(b"{"));
    let (reason, connected) = raw_disconnect(Some(body), TIMEOUT);
    assert!(
        reason
            .to_string()
            .starts_with("protocol error: invalid control message"),
        "{reason}"
    );
    assert!(connected);

    let mut twice = raw::welcome();
    twice.extend(raw::welcome());
    let (reason, connected) = raw_disconnect(Some(twice), TIMEOUT);
    assert_eq!(
        reason,
        Disconnect::Protocol("unexpected second welcome".into())
    );
    assert!(connected);
}

#[test]
fn reports_how_the_response_ended() {
    // Trailers end the response cleanly.
    let mut body = raw::welcome();
    body.extend(b"0\r\nx-done: 1\r\n\r\n");
    let (reason, connected) = raw_disconnect(Some(body), TIMEOUT);
    assert_eq!(reason, Disconnect::Closed);
    assert!(connected);

    // So does closing the connection, for a response it delimits.
    let client = Client::connect(options(raw::start_closing(raw::welcome_frame()))).unwrap();
    wait_connected(&client);
    assert_eq!(wait_disconnected(&client), Disconnect::Closed);

    // Ending part way through a frame is a protocol error.
    let mut body = raw::welcome();
    body.extend(raw::chunk(&[0, 9, 0, 0, 0, b'{']));
    body.extend(b"0\r\n\r\n");
    let (reason, _) = raw_disconnect(Some(body), TIMEOUT);
    assert_eq!(
        reason,
        Disconnect::Protocol("connection ended inside a frame".into())
    );

    // So is a malformed audio frame.
    let mut body = raw::welcome();
    body.extend(raw::chunk(&[1, 1, 0, 0, 0, 0]));
    let (reason, _) = raw_disconnect(Some(body), TIMEOUT);
    assert_eq!(
        reason,
        Disconnect::Protocol("invalid audio frame: header is truncated".into())
    );
}

#[test]
fn reports_the_servers_utterance_limit() {
    let address = raw::start(Some(raw::welcome_limiting(Some(300))));
    let client = Client::connect(options(address)).unwrap();
    let connected = wait_for(&client, |event| {
        matches!(event, Event::State(ConnectionState::Connected { .. }))
    });
    let Event::State(ConnectionState::Connected {
        utterance_limit, ..
    }) = connected
    else {
        unreachable!("wait_for returns a connected state")
    };
    assert_eq!(utterance_limit, Some(Duration::from_secs(300)));
}

#[test]
fn asks_to_pair_only_when_told_to() {
    for pairing in [false, true] {
        let (address, heads) = raw::start_recording(Some(raw::welcome()));
        let mut options = options(address);
        options.pairing = pairing;
        let client = Client::connect(options).unwrap();
        wait_connected(&client);
        let head = String::from_utf8(heads.recv_timeout(TIMEOUT).unwrap()).unwrap();
        assert_eq!(
            head.to_ascii_lowercase().contains("tomori-pairing: 1\r\n"),
            pairing,
            "{head}"
        );
    }
}

fn keepalive(interval: u64, timeout: u64) -> Keepalive {
    Keepalive {
        interval: Duration::from_millis(interval),
        timeout: Duration::from_millis(timeout),
    }
}

#[test]
fn reconnects_when_the_server_goes_silent() {
    // The peer welcomes the client and then sends nothing, like a server whose network went away.
    let mut options = options(raw::start(Some(raw::welcome())));
    options.keepalive = keepalive(50, 300);
    let client = Client::connect(options).unwrap();
    wait_connected(&client);
    assert_eq!(wait_disconnected(&client), Disconnect::Timeout);
    wait_connected(&client);
}

#[test]
fn pings_keep_a_quiet_connection_open() {
    let server = MiniServer::start(localhost(), Mode::Normal);
    let mut options = options(server.address);
    options.keepalive = keepalive(30, 300);
    let client = Client::connect(options).unwrap();
    wait_connected(&client);

    // The client answers the server's ping without reporting it.
    client
        .send(ClientMessage::Answer {
            question: "q".into(),
            field: "f".into(),
            text: "ping".into(),
        })
        .unwrap();
    let deadline = Instant::now() + TIMEOUT;
    while !server
        .received(1)
        .contains(&Received::Message(ClientMessage::Pong { nonce: 7 }))
    {
        assert!(Instant::now() < deadline, "the client never answered");
        std::thread::sleep(Duration::from_millis(5));
    }

    // Nothing else is sent, so only the client's pings and the server's pongs keep the connection alive for several timeouts.
    std::thread::sleep(Duration::from_millis(1000));
    assert!(client.state().is_connected());
    while let Some(event) = client.try_event() {
        assert!(
            !matches!(
                event,
                Event::State(ConnectionState::Disconnected { .. })
                    | Event::Message(ServerMessage::Ping { .. } | ServerMessage::Pong { .. })
            ),
            "unexpected {event:?}"
        );
    }
    let pings = server
        .received(1)
        .iter()
        .filter(|received| matches!(received, Received::Message(ClientMessage::Ping { .. })))
        .count();
    assert!(pings >= 3, "only {pings} pings");
}

#[test]
fn reports_unresolvable_addresses() {
    let mut options = options(localhost());
    options.address = "invalid host name:1".into();
    let client = Client::connect(options).unwrap();
    let reason = wait_disconnected(&client);
    assert!(
        reason
            .to_string()
            .starts_with("cannot resolve the server address: "),
        "{reason}"
    );
}

#[test]
fn congestion_drops_outgoing_audio_and_closing_stays_bounded() {
    let server = MiniServer::start(localhost(), Mode::Stall);
    let client = Client::connect(options(server.address)).unwrap();
    wait_connected(&client);
    // Far more than the socket buffers hold, with the server no longer reading.
    let count = 20_000;
    for sequence in 0..count {
        client.send_audio(audio(sequence)).unwrap();
        if sequence % 1000 == 0 {
            client.send(ClientMessage::ListSpaces).unwrap();
        }
    }
    std::thread::sleep(Duration::from_millis(200));
    let dropped = client.sender().dropped_audio_frames();
    assert!(
        dropped > u64::from(count) / 2,
        "dropped only {dropped} frames"
    );
    assert!(client.state().is_connected());

    let started = Instant::now();
    client.close();
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(3), "closing took {elapsed:?}");
}

#[test]
fn a_connection_reset_while_writing_is_a_transport_failure() {
    let (reset, resets) = std::sync::mpsc::channel();
    let client = Client::connect(options(raw::start_resetting(raw::welcome(), resets))).unwrap();
    wait_connected(&client);
    // Far more than the socket buffers hold, with the peer no longer reading.
    let deadline = Instant::now() + TIMEOUT;
    let mut sequence = 0;
    while client.sender().dropped_audio_frames() == 0 {
        assert!(Instant::now() < deadline, "the writes never backed up");
        client.send_audio(audio(sequence)).unwrap();
        sequence += 1;
    }
    reset.send(()).unwrap();
    let reason = wait_disconnected(&client);
    assert!(matches!(reason, Disconnect::Transport(_)), "{reason:?}");
}
