// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Pairing is open while a server has no owner, and otherwise only in a short window that an owner or the operator opens, which closes once a device pairs, when its time is up, or when someone closes it. Visitors, collaborators and agents never open it. See `design/security.md`.
use std::sync::mpsc::channel;
use std::time::Duration;

use tomori_protocol::{ClientMessage, ConnectionRole, PAIRING_CLOSED, ServerMessage};
use tomori_security::Fingerprint;
use tomori_security::pairing::{Nonce, code};
use tomori_server::test_support::{TestClient, TestServer};
use tomori_server::{Opener, PAIRING_OPEN_FOR, PairingState};

fn fingerprint() -> Fingerprint {
    Fingerprint::of(b"test certificate")
}

/// A server that admits by tokens, with visitors if `visitors`, and pairs, with its pairing handle.
fn pairing_server(
    visitors: bool,
    configure: impl FnOnce(tomori_server::Server) -> tomori_server::Server + Send + 'static,
) -> (TestServer, tomori_server::Pairing) {
    let (sender, receiver) = channel();
    let server = TestServer::with_tokens(visitors, move |server| {
        let server = configure(
            server
                .with_pairing(fingerprint())
                .with_tokens_over_plain_http_for_tests(),
        );
        sender.send(server.pairing()).unwrap();
        server
    });
    (server, receiver.recv().unwrap())
}

/// Begin and reveal, as a device does, returning the code it derives.
fn pair(client: &mut TestClient) -> String {
    let device = Nonce::generate();
    client.send(&ClientMessage::PairBegin {
        commitment: device.commitment(),
    });
    let server = client.wait_for(|message| match message {
        ServerMessage::PairNonce { nonce } => Nonce::parse(nonce),
        ServerMessage::PairFailed { error } => panic!("pairing was refused: {error}"),
        _ => None,
    });
    client.send(&ClientMessage::PairReveal {
        nonce: device.to_hex(),
    });
    client.wait_for(|message| matches!(message, ServerMessage::PairWaiting).then_some(()));
    code(fingerprint(), device, server)
}

/// Ask to pair, expecting to be refused, and return why.
fn refused(client: &mut TestClient) -> String {
    client.send(&ClientMessage::PairBegin {
        commitment: Nonce::generate().commitment(),
    });
    failed(client)
}

fn failed(client: &TestClient) -> String {
    client.wait_for(|message| match message {
        ServerMessage::PairFailed { error } => Some(error.clone()),
        _ => None,
    })
}

fn paired(client: &TestClient) -> String {
    client.wait_for(|message| match message {
        ServerMessage::Paired { token } => Some(token.clone()),
        _ => None,
    })
}

/// The next word of whether pairing is open, as an owner's client is told: the seconds left, or `None` once closed.
fn window(client: &TestClient) -> Option<u32> {
    client.wait_for(|message| match message {
        ServerMessage::PairingWindow { closes_in } => Some(*closes_in),
        _ => None,
    })
}

/// Connect presenting `token`, as a client that reads whether pairing is open.
fn owner(server: &TestServer, participant: &str, name: &str, token: &str) -> TestClient {
    let mut client = TestClient::open_with(
        server.address,
        &[("Authorization", &format!("Bearer {token}"))],
    );
    client.hello_reading_pairing(participant, name);
    client
}

/// Post the form `body` to `path` with `cookie`, and an `Origin` if given, returning the status.
fn post(server: &TestServer, path: &str, cookie: &str, origin: Option<&str>) -> u16 {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(server.address).unwrap();
    let origin = origin
        .map(|origin| format!("Origin: {origin}\r\n"))
        .unwrap_or_default();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: tomori\r\nConnection: close\r\nCookie: {cookie}\r\n{origin}Content-Type: application/x-www-form-urlencoded\r\nContent-Length: 0\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response[9..12].parse().unwrap()
}

fn get(server: &TestServer, cookie: &str) -> String {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(server.address).unwrap();
    write!(
        stream,
        "GET / HTTP/1.1\r\nHost: tomori\r\nConnection: close\r\nCookie: {cookie}\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

/// The body of the panel titled "Pairing" that `client`, in a space and reading what is projected there, is next shown.
fn notice(client: &TestClient) -> String {
    client.wait_for(|message| match message {
        ServerMessage::Projection { projection } => projection
            .panels
            .iter()
            .rev()
            .find(|panel| panel.title == "Pairing")
            .map(|panel| panel.body.clone()),
        _ => None,
    })
}

#[test]
fn pairing_is_closed_by_default_even_without_an_owner_and_one_pairing_closes_it() {
    let (server, pairing) = pairing_server(true, |server| server);
    // Closed even at first-time setup, with no owner: a device asking to pair is refused at once, without a pairing or an attempt counted.
    let closed = Some(PairingState::Closed {
        opens_for: PAIRING_OPEN_FOR,
    });
    assert_eq!(pairing.state(), closed);
    let mut quest = server.connect("quest", "Quest 3");
    for _ in 0..=tomori_server::PAIRING_ATTEMPTS {
        assert_eq!(refused(&mut quest), PAIRING_CLOSED);
    }
    assert_eq!(pairing.pending(), None);
    assert!(!pairing.paused());

    // The operator opens it deliberately, as with "pair" on the console, and the first device pairs.
    pairing.open().unwrap();
    let code = pair(&mut quest);
    pairing.confirm(&code).unwrap();
    paired(&quest);

    // One successful pairing closes it again, so the next device is refused.
    assert_eq!(pairing.state(), closed);
    let mut tablet = server.connect("tablet", "Tablet");
    assert_eq!(refused(&mut tablet), PAIRING_CLOSED);
    assert_eq!(pairing.pending(), None);

    // A minted owner token changes nothing: it is closed either way.
    let (other, _) = pairing_server(true, |server| server);
    other.issue("Ada", tomori_security::Grant::Owner);
    let mut device = other.connect("device", "Device");
    assert_eq!(refused(&mut device), PAIRING_CLOSED);
}

#[test]
fn an_owner_opens_pairing_from_a_paired_device_and_every_space_is_told() {
    let (server, pairing) = pairing_server(true, |server| server);
    let (_, token) = server.issue("Ada", tomori_security::Grant::Owner);
    let mut ada = owner(&server, "ada", "Ada", &token);
    // Told on connecting that pairing is closed.
    assert_eq!(window(&ada), None);
    let studio = ada.create_space("Studio").id;
    ada.create_space("Workshop");

    // Someone watching the first space, as a visitor, sees what is projected there.
    let mut visitor = TestClient::open(server.address);
    visitor.hello_projected("visitor", "Vic");
    visitor.send(&ClientMessage::Join {
        space: studio.clone(),
    });

    ada.send(&ClientMessage::OpenPairing);
    assert_eq!(window(&ada), Some(120));
    assert!(matches!(
        pairing.state(),
        Some(PairingState::Open { by, .. }) if by == Opener::owner("Ada")
    ));
    // Every space is told, so that a window nobody expected is seen.
    let told = notice(&visitor);
    assert_eq!(
        told,
        "Ada opened pairing from a paired device for 2 minutes, so that a new device can ask to pair with this server. If nobody expected this, an owner can close it in People & security or on the status page."
    );

    // One device pairs, which closes the window: a second is refused.
    let mut quest = server.connect("quest", "Quest 3");
    let code = pair(&mut quest);
    pairing.confirm(&code).unwrap();
    paired(&quest);
    assert_eq!(window(&ada), None);
    let mut tablet = server.connect("tablet", "Tablet");
    assert_eq!(refused(&mut tablet), PAIRING_CLOSED);

    // Opened again, and again while open, it is open afresh for its whole time.
    ada.send(&ClientMessage::OpenPairing);
    assert_eq!(window(&ada), Some(120));
    ada.send(&ClientMessage::OpenPairing);
    assert_eq!(window(&ada), Some(120));
    // A pairing that is declined does not use the window.
    pair(&mut tablet);
    assert!(pairing.decline());
    failed(&tablet);
    assert!(matches!(pairing.state(), Some(PairingState::Open { .. })));

    // Closed on demand, which withdraws the pairing waiting.
    pair(&mut tablet);
    ada.send(&ClientMessage::ClosePairing);
    assert_eq!(window(&ada), None);
    assert_eq!(
        failed(&tablet),
        "Pairing was closed, so the pairing was withdrawn."
    );
    assert_eq!(pairing.pending(), None);
    ada.send(&ClientMessage::ClosePairing);
    assert_eq!(ada.error(), "Pairing is not open.");

    // Another owner's device is told on connecting while it is open, as its countdown.
    ada.send(&ClientMessage::OpenPairing);
    assert_eq!(window(&ada), Some(120));
    let mut again = owner(&server, "ada-desktop", "Ada", &token);
    let left = window(&again);
    assert!(
        left.is_some_and(|left| left > 100 && left <= 120),
        "{left:?}"
    );
    // The operator closing it tells every owner's device.
    assert!(pairing.close());
    assert_eq!(window(&ada), None);
    assert_eq!(window(&again), None);
    assert!(!pairing.close());
    again.finish();
}

#[test]
fn a_window_closes_when_its_time_is_up_but_a_pairing_begun_in_it_may_finish() {
    let (server, pairing) = pairing_server(true, |server| {
        server.with_pairing_open_for(Duration::from_secs(1))
    });
    let (_, token) = server.issue("Ada", tomori_security::Grant::Owner);
    let ada = owner(&server, "ada", "Ada", &token);
    assert_eq!(window(&ada), None);
    assert_eq!(pairing.open(), Ok(Duration::from_secs(1)));
    assert_eq!(window(&ada), Some(1));
    assert_eq!(window(&ada), None);
    assert!(matches!(pairing.state(), Some(PairingState::Closed { .. })));
    let mut quest = server.connect("quest", "Quest 3");
    assert_eq!(refused(&mut quest), PAIRING_CLOSED);

    // A pairing begun within the window holds it until it ends, and may be confirmed after the window's time.
    pairing.open().unwrap();
    assert_eq!(window(&ada), Some(1));
    let code = pair(&mut quest);
    std::thread::sleep(Duration::from_millis(1500));
    assert!(ada.next_message(Duration::from_millis(500)).is_none());
    assert!(matches!(pairing.state(), Some(PairingState::Open { .. })));
    // Nobody else may begin meanwhile.
    let mut tablet = server.connect("tablet", "Tablet");
    assert_eq!(refused(&mut tablet), PAIRING_CLOSED);
    pairing.confirm(&code).unwrap();
    paired(&quest);
    assert_eq!(window(&ada), None);

    // Once the pairing begun in it ends otherwise, the window, its time up, closes too.
    pairing.open().unwrap();
    assert_eq!(window(&ada), Some(1));
    pair(&mut tablet);
    std::thread::sleep(Duration::from_millis(1500));
    assert!(pairing.decline());
    assert_eq!(window(&ada), None);
}

#[test]
fn an_owner_opens_and_closes_pairing_on_the_status_page() {
    let (server, pairing) = pairing_server(true, |server| server);
    let (_, token) = server.issue("Ada", tomori_security::Grant::Owner);
    let mut ada = owner(&server, "ada", "Ada", &token);
    assert_eq!(window(&ada), None);
    let studio = ada.create_space("Studio").id;
    let (_, bob) = server.issue(
        "Bob",
        tomori_security::Grant::Collaborator { space: studio },
    );
    let cookie = format!("__Host-tomori-token={token}");
    let bob = format!("__Host-tomori-token={bob}");

    let page = get(&server, &cookie);
    assert!(
        page.contains("Pairing is closed: a device that asks to pair is refused.")
            && page
                .contains("action=\"/pairing/open\"><button>Open pairing for 2 minutes</button>"),
        "{page}"
    );
    // Only an owner, and only from the server's own pages.
    assert_eq!(post(&server, "/pairing/open", &bob, None), 403);
    assert_eq!(post(&server, "/pairing/open", "", None), 403);
    assert_eq!(
        post(
            &server,
            "/pairing/open",
            &cookie,
            Some("https://elsewhere.example")
        ),
        403
    );
    assert!(matches!(pairing.state(), Some(PairingState::Closed { .. })));

    assert_eq!(
        post(&server, "/pairing/open", &cookie, Some("https://tomori")),
        303
    );
    assert_eq!(window(&ada), Some(120));
    assert!(matches!(
        pairing.state(),
        Some(PairingState::Open { by, .. }) if by == Opener::status_page(Some("Ada"))
    ));
    let page = get(&server, &cookie);
    assert!(
        page.contains("as Ada opened it on the status page: a device may ask to pair")
            && page.contains("action=\"/pairing/close\"><button>Close pairing</button>"),
        "{page}"
    );

    assert_eq!(post(&server, "/pairing/close", &bob, None), 403);
    assert!(matches!(pairing.state(), Some(PairingState::Open { .. })));
    assert_eq!(post(&server, "/pairing/close", &cookie, None), 303);
    assert_eq!(window(&ada), None);
    assert!(get(&server, &cookie).contains("Pairing is closed"));
    // Closing it again changes nothing.
    assert_eq!(post(&server, "/pairing/close", &cookie, None), 303);
    // Only by POST.
    assert_eq!(server.get("/pairing/open").0, 405);
    assert_eq!(server.get("/pairing/close").0, 405);
}

#[test]
fn visitors_collaborators_and_devices_admitted_only_to_pair_cannot_open_pairing_or_create_spaces() {
    let (server, pairing) = pairing_server(true, |server| server);
    let (_, token) = server.issue("Ada", tomori_security::Grant::Owner);
    let mut ada = server.connect_with_token("ada", "Ada", &token);
    let studio = ada.create_space("Studio").id;
    let (_, bob) = server.issue(
        "Bob",
        tomori_security::Grant::Collaborator {
            space: studio.clone(),
        },
    );

    // A collaborator, in the space they were admitted to and in the lobby.
    let mut bob = server.connect_with_token("bob", "Bob", &bob);
    bob.send(&ClientMessage::Join {
        space: studio.clone(),
    });
    bob.wait_for(|message| {
        matches!(
            message,
            ServerMessage::Role {
                role: ConnectionRole::Collaborator
            }
        )
        .then_some(())
    });
    // A visitor watching it.
    let mut vic = server.connect("vic", "Vic");
    vic.send(&ClientMessage::Join {
        space: studio.clone(),
    });
    vic.wait_for(|message| matches!(message, ServerMessage::Space { .. }).then_some(()));
    let create = ClientMessage::CreateSpace {
        title: "Mine now".into(),
        backend: "fake".into(),
        directory: "/tmp".into(),
        session: None,
        environment: None,
        instructions: None,
    };
    for (client, refusal) in [
        (&mut bob, "Only owners can do that."),
        (&mut vic, "Visitors can watch, but not act in a space."),
    ] {
        for message in [
            ClientMessage::OpenPairing,
            ClientMessage::ClosePairing,
            create.clone(),
        ] {
            client.send(&message);
            assert_eq!(client.error(), refusal, "{message:?}");
        }
    }
    bob.send(&ClientMessage::Leave);
    bob.wait_for(|message| {
        matches!(
            message,
            ServerMessage::Role {
                role: ConnectionRole::Visitor
            }
        )
        .then_some(())
    });
    for message in [ClientMessage::OpenPairing, create.clone()] {
        bob.send(&message);
        assert_eq!(
            bob.error(),
            "Visitors can watch, but not act in a space.",
            "{message:?}"
        );
    }
    assert!(matches!(pairing.state(), Some(PairingState::Closed { .. })));
    // No space was created but Ada's, and nobody else became an owner.
    assert_eq!(server.backend.started().len(), 1);
    assert_eq!(
        server
            .tokens
            .as_ref()
            .unwrap()
            .entries()
            .iter()
            .filter(|entry| entry.grant == tomori_security::Grant::Owner)
            .count(),
        1
    );

    // A device admitted only to pair, on a server that refuses visitors, can only pair, and pairing is closed.
    let (strict, strict_pairing) = pairing_server(false, |server| server);
    strict.issue("Ada", tomori_security::Grant::Owner);
    let mut device = TestClient::open_with(strict.address, &[("Tomori-Pairing", "1")]);
    device.hello("device", "Device");
    assert_eq!(device.role, ConnectionRole::Pairing);
    for message in [ClientMessage::OpenPairing, create] {
        device.send(&message);
        assert_eq!(
            device.error(),
            "This server admits only paired devices: pair to join.",
            "{message:?}"
        );
    }
    assert_eq!(refused(&mut device), PAIRING_CLOSED);
    assert!(matches!(
        strict_pairing.state(),
        Some(PairingState::Closed { .. })
    ));
}

#[test]
fn only_an_owner_s_client_that_reads_it_is_told_whether_pairing_is_open() {
    let (server, pairing) = pairing_server(true, |server| server);
    let (_, token) = server.issue("Ada", tomori_security::Grant::Owner);
    // An older client, which would refuse the message, is never sent it.
    let older = server.connect_with_token("ada", "Ada", &token);
    // Nor is a visitor, though its client reads it.
    let mut visitor = TestClient::open(server.address);
    visitor.hello_reading_pairing("vic", "Vic");
    pairing.open().unwrap();
    for client in [&older, &visitor] {
        let deadline = std::time::Instant::now() + Duration::from_millis(500);
        while let Some(message) =
            client.next_message(deadline.saturating_duration_since(std::time::Instant::now()))
        {
            assert!(
                !matches!(message, ServerMessage::PairingWindow { .. }),
                "{message:?}"
            );
        }
    }
}

#[test]
fn an_open_server_has_no_pairing_to_open() {
    let server = TestServer::start();
    let mut ada = TestClient::open(server.address);
    ada.hello_reading_pairing("ada", "Ada");
    assert_eq!(ada.role, ConnectionRole::Owner);
    ada.send(&ClientMessage::OpenPairing);
    assert_eq!(ada.error(), "This server does not pair devices.");
    ada.send(&ClientMessage::ClosePairing);
    assert_eq!(ada.error(), "Pairing is not open.");
    // Everyone is an owner on its status page, which has no pairing to open or close either.
    assert_eq!(post(&server, "/pairing/open", "", None), 303);
    assert_eq!(post(&server, "/pairing/close", "", None), 303);
    assert!(
        !ada.received()
            .iter()
            .any(|message| message.contains("pairing_window"))
    );
}
