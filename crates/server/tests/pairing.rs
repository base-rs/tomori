// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Pairing a device as an owner. See `design/security.md`.

use std::sync::mpsc::channel;
use std::time::Duration;

use protocol::{ClientMessage, ConnectionRole, ServerMessage};
use security::Fingerprint;
use security::pairing::{Nonce, code, display};
use server::test_support::{TIMEOUT, TestClient, TestServer};

/// The certificate fingerprint the test servers pair with. The test clients speak plain HTTP, so the fingerprint is only what the code is derived from.
fn fingerprint() -> Fingerprint {
    Fingerprint::of(b"test certificate")
}

/// A server that admits by tokens, with visitors if `visitors`, and pairs, with its pairing handle.
fn pairing_server(
    visitors: bool,
    configure: impl FnOnce(server::Server) -> server::Server + Send + 'static,
) -> (TestServer, server::Pairing) {
    let (sender, receiver) = channel();
    let server = TestServer::with_tokens(visitors, move |server| {
        let server = configure(server.with_pairing(fingerprint()));
        sender.send(server.pairing()).unwrap();
        server
    });
    (server, receiver.recv().unwrap())
}

fn failed(client: &TestClient) -> String {
    client.wait_for(|message| match message {
        ServerMessage::PairFailed { error } => Some(error.clone()),
        _ => None,
    })
}

/// Begin and reveal, as a device does, returning the code it derives.
fn pair(client: &mut TestClient) -> String {
    let device = Nonce::generate();
    client.send(&ClientMessage::PairBegin {
        commitment: device.commitment(),
    });
    let server = client.wait_for(|message| match message {
        ServerMessage::PairNonce { nonce } => Nonce::parse(nonce),
        _ => None,
    });
    client.send(&ClientMessage::PairReveal {
        nonce: device.to_hex(),
    });
    client.wait_for(|message| matches!(message, ServerMessage::PairWaiting).then_some(()));
    code(fingerprint(), device, server)
}

fn paired(client: &TestClient) -> String {
    client.wait_for(|message| match message {
        ServerMessage::Paired { token } => Some(token.clone()),
        _ => None,
    })
}

#[test]
fn the_operator_types_the_code_to_pair_a_device_as_an_owner() {
    let (server, pairing) = pairing_server(true, |server| server);
    let mut quest = server.connect("quest", "Quest 3");
    assert_eq!(quest.role, ConnectionRole::Visitor);
    let code = pair(&mut quest);
    assert_eq!(
        pairing.pending(),
        Some(("Quest 3".to_owned(), display(&code)))
    );

    // A wrong code confirms nothing.
    let wrong = if code == "000000" { "000001" } else { "000000" };
    assert_eq!(
        pairing.confirm(wrong),
        Err("That code does not match, so nothing was confirmed.")
    );
    assert!(server.tokens.as_ref().unwrap().entries().is_empty());

    assert_eq!(pairing.confirm(&code), Ok("Quest 3".to_owned()));
    let token = paired(&quest);
    assert_eq!(pairing.pending(), None);
    let entries = server.tokens.as_ref().unwrap().entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "Quest 3");
    assert_eq!(entries[0].grant, security::Grant::Owner);

    // From now on the device connects with its token, as an owner.
    drop(quest);
    let quest = server.connect_with_token("quest", "Quest 3", &token);
    assert_eq!(quest.role, ConnectionRole::Owner);
}

#[test]
fn a_declined_pairing_tells_the_device_and_issues_nothing() {
    let (server, pairing) = pairing_server(true, |server| server);
    let mut quest = server.connect("quest", "Quest 3");
    pair(&mut quest);
    assert!(pairing.decline());
    assert_eq!(failed(&quest), "The pairing was declined.");
    assert!(!pairing.decline());
    assert!(server.tokens.as_ref().unwrap().entries().is_empty());
}

#[test]
fn one_pairing_waits_at_a_time() {
    let (server, pairing) = pairing_server(true, |server| server);
    let mut first = server.connect("first", "First");
    let code = pair(&mut first);
    let mut second = server.connect("second", "Second");
    second.send(&ClientMessage::PairBegin {
        commitment: Nonce::generate().commitment(),
    });
    assert_eq!(
        failed(&second),
        "Another device is pairing; try again shortly."
    );
    // The first is unaffected.
    assert_eq!(pairing.confirm(&code), Ok("First".to_owned()));
    paired(&first);
}

#[test]
fn a_value_that_does_not_match_its_commitment_fails() {
    let (server, pairing) = pairing_server(true, |server| server);
    let mut quest = server.connect("quest", "Quest 3");
    quest.send(&ClientMessage::PairBegin {
        commitment: Nonce::generate().commitment(),
    });
    quest.wait_for(|message| matches!(message, ServerMessage::PairNonce { .. }).then_some(()));
    quest.send(&ClientMessage::PairReveal {
        nonce: Nonce::generate().to_hex(),
    });
    assert_eq!(
        failed(&quest),
        "The pairing could not be checked; try again."
    );
    assert_eq!(pairing.pending(), None);

    // A malformed commitment is refused outright.
    quest.send(&ClientMessage::PairBegin {
        commitment: "not hex".into(),
    });
    assert_eq!(
        quest.error(),
        "A pairing commitment is 64 hexadecimal digits."
    );
}

#[test]
fn a_pairing_is_withdrawn_after_its_timeout_or_when_its_device_leaves() {
    let (server, pairing) = pairing_server(true, |server| {
        server.with_pairing_timeout(Duration::from_millis(300))
    });
    let mut quest = server.connect("quest", "Quest 3");
    pair(&mut quest);
    assert_eq!(failed(&quest), "The pairing was not confirmed in time.");
    assert_eq!(pairing.pending(), None);

    let mut leaving = server.connect("leaving", "Leaving");
    pair(&mut leaving);
    assert!(pairing.pending().is_some());
    drop(leaving);
    let deadline = std::time::Instant::now() + TIMEOUT;
    while pairing.pending().is_some() {
        assert!(std::time::Instant::now() < deadline, "the pairing was kept");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn without_visitors_a_device_asking_to_pair_may_only_pair() {
    let (server, pairing) = pairing_server(false, |server| server);
    assert_eq!(server.refused(&[]).0, 401);
    let mut quest = TestClient::open_with(server.address, &[("Tomori-Pairing", "1")]);
    quest.hello("quest", "Quest 3");
    assert_eq!(quest.role, ConnectionRole::Pairing);
    quest.send(&ClientMessage::ListSpaces);
    assert_eq!(
        quest.error(),
        "This server admits only paired devices: pair to join."
    );
    let code = pair(&mut quest);
    pairing.confirm(&code).unwrap();
    let token = paired(&quest);
    let quest = server.connect_with_token("quest", "Quest 3", &token);
    assert_eq!(quest.role, ConnectionRole::Owner);
}

#[test]
fn an_open_server_does_not_pair() {
    let server = TestServer::start();
    let mut ada = server.connect("ada", "Ada");
    ada.send(&ClientMessage::PairBegin {
        commitment: Nonce::generate().commitment(),
    });
    assert_eq!(failed(&ada), "This server does not pair devices.");
}

/// Post an empty form to `path` with `cookie`, returning the status and body.
fn post(server: &TestServer, path: &str, cookie: &str) -> (u16, String) {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(server.address).unwrap();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: tomori\r\nConnection: close\r\nCookie: {cookie}\r\nContent-Length: 0\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    (response[9..12].parse().unwrap(), response)
}

fn get(server: &TestServer, path: &str, cookie: &str) -> String {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(server.address).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: tomori\r\nConnection: close\r\nCookie: {cookie}\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

#[test]
fn an_owner_confirms_a_pairing_on_the_status_page() {
    let (server, _) = pairing_server(true, |server| server);
    let (_, owner) = server.issue("Ada", security::Grant::Owner);
    let (_, collaborator) = server.issue(
        "Bob",
        security::Grant::Collaborator {
            space: "studio".into(),
        },
    );
    let owner = format!("__Host-tomori-token={owner}");
    let collaborator = format!("__Host-tomori-token={collaborator}");
    let mut quest = server.connect("quest", "Quest 3");
    let code = pair(&mut quest);

    let page = get(&server, "/", &owner);
    assert!(
        page.contains(&format!(
            "Quest 3 asks to pair with code <strong>{}</strong>",
            display(&code)
        )),
        "{page}"
    );

    // Only an owner can confirm.
    assert_eq!(post(&server, "/pairing/confirm", &collaborator).0, 403);
    assert!(quest.next_message(Duration::from_millis(200)).is_none());

    let (status, response) = post(&server, "/pairing/confirm", &owner);
    assert_eq!(status, 303, "{response}");
    let token = paired(&quest);
    assert!(token.starts_with(security::TOKEN_PREFIX));
    assert!(get(&server, "/", &owner).contains("No pairing is pending"));

    // Declining with nothing waiting changes nothing.
    assert_eq!(post(&server, "/pairing/decline", &owner).0, 303);
}

#[test]
fn a_client_pairs_over_tls_with_the_certificate_it_saw() {
    let certificate = security::ServerCertificate::generate();
    let fingerprint = certificate.fingerprint();
    let config = certificate.server_config().unwrap();
    let (sender, receiver) = channel();
    let server = TestServer::with_tokens(true, move |server| {
        let server = server.with_tls(config).with_pairing(fingerprint);
        sender.send(server.pairing()).unwrap();
        server
    });
    let pairing = receiver.recv().unwrap();

    let connect = |token: Option<String>, pin: Option<Fingerprint>| {
        let mut options = client::Options::new(server.address.to_string(), "quest", "Quest 3");
        options.transport = client::Transport::Tls { pin };
        options.token = token;
        client::Client::connect(options).unwrap()
    };
    let next = |client: &client::Client| {
        client
            .events()
            .recv_timeout(TIMEOUT)
            .expect("an event arrives in time")
    };
    let quest = connect(None, None);
    let seen = loop {
        if let client::Event::State(client::ConnectionState::Connected {
            role, certificate, ..
        }) = next(&quest)
        {
            assert_eq!(role, ConnectionRole::Visitor);
            break certificate.unwrap();
        }
    };
    assert_eq!(seen, fingerprint);

    let device = Nonce::generate();
    quest
        .send(ClientMessage::PairBegin {
            commitment: device.commitment(),
        })
        .unwrap();
    let server_nonce = loop {
        if let client::Event::Message(ServerMessage::PairNonce { nonce }) = next(&quest) {
            break Nonce::parse(&nonce).unwrap();
        }
    };
    quest
        .send(ClientMessage::PairReveal {
            nonce: device.to_hex(),
        })
        .unwrap();
    loop {
        if let client::Event::Message(ServerMessage::PairWaiting) = next(&quest) {
            break;
        }
    }
    // The operator types the code the device shows, derived from the certificate it saw.
    pairing.confirm(&code(seen, device, server_nonce)).unwrap();
    let token = loop {
        if let client::Event::Message(ServerMessage::Paired { token }) = next(&quest) {
            break token;
        }
    };
    quest.close();

    let owner = connect(Some(token), Some(seen));
    loop {
        if let client::Event::State(client::ConnectionState::Connected { role, .. }) = next(&owner)
        {
            assert_eq!(role, ConnectionRole::Owner);
            break;
        }
    }
    owner.close();
}
