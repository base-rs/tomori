// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Pairing a device as an owner. See `design/security.md`.
use std::sync::mpsc::channel;
use std::time::Duration;

use tomori_protocol::{ClientMessage, ConnectionRole, ServerMessage};
use tomori_security::Fingerprint;
use tomori_security::pairing::{ChosenName, Nonce, code, display};
use tomori_server::test_support::{TIMEOUT, TestClient, TestServer};

/// The certificate fingerprint the test servers pair with. The test clients speak plain HTTP, so the fingerprint is only what the code is derived from.
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
    let pending = pairing.pending().unwrap();
    // Shown by what is left of the name it chose, without its digits.
    assert_eq!(pending.device.as_str(), Some("Quest"));
    assert_eq!(pending.code, Some(display(&code)));

    // A wrong code confirms nothing and declines the pairing, telling the device, so that the right code is no use afterwards.
    let wrong = if code == "000000" { "000001" } else { "000000" };
    assert_eq!(
        pairing.confirm(wrong),
        Err("That code does not match, so the pairing was declined.")
    );
    assert_eq!(
        failed(&quest),
        "The code given did not match, so the pairing was declined."
    );
    assert_eq!(pairing.pending(), None);
    assert_eq!(pairing.confirm(&code), Err("No pairing is waiting."));
    assert!(server.tokens.as_ref().unwrap().entries().is_empty());

    let code = pair(&mut quest);
    assert_eq!(pairing.confirm(&code), Ok(ChosenName::new("Quest 3")));
    let token = paired(&quest);
    assert_eq!(pairing.pending(), None);
    let entries = server.tokens.as_ref().unwrap().entries();
    assert_eq!(entries.len(), 1);
    // The token is named as the device named itself, as any token is, which pairing never shows.
    assert_eq!(entries[0].name, "Quest 3");
    assert_eq!(entries[0].grant, tomori_security::Grant::Owner);

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
        "A pairing is already in progress; try again shortly."
    );
    // Nor may the first begin again, which would give it a fresh value from the server to try against the code it has seen.
    first.send(&ClientMessage::PairBegin {
        commitment: Nonce::generate().commitment(),
    });
    assert_eq!(
        failed(&first),
        "A pairing is already in progress; try again shortly."
    );
    assert_eq!(pairing.pending().unwrap().code, Some(display(&code)));
    // The first is unaffected.
    assert_eq!(pairing.confirm(&code), Ok(ChosenName::new("First")));
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
fn a_pairing_is_withdrawn_after_its_timeout() {
    let (server, pairing) = pairing_server(true, |server| {
        server.with_pairing_timeout(Duration::from_secs(1))
    });
    let mut quest = server.connect("quest", "Quest 3");
    pair(&mut quest);
    assert_eq!(failed(&quest), "The pairing was not confirmed in time.");
    assert_eq!(pairing.pending(), None);
}

#[test]
fn a_device_that_does_not_reveal_in_time_is_withdrawn() {
    let (server, pairing) = pairing_server(true, |server| server);
    let mut quest = server.connect("quest", "Quest 3");
    quest.send(&ClientMessage::PairBegin {
        commitment: Nonce::generate().commitment(),
    });
    quest.wait_for(|message| matches!(message, ServerMessage::PairNonce { .. }).then_some(()));
    // Shown, so that it can be declined, though it has no code yet.
    let pending = pairing.pending().unwrap();
    assert_eq!(
        (pending.device.as_str(), pending.code),
        (Some("Quest"), None)
    );
    let deadline = std::time::Instant::now() + tomori_server::REVEAL_TIMEOUT + TIMEOUT;
    let error = loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match quest.next_message(remaining) {
            Some(ServerMessage::PairFailed { error }) => break error,
            Some(_) => {}
            None => panic!("the pairing was not withdrawn"),
        }
    };
    assert_eq!(error, "The pairing was not completed in time.");
    assert_eq!(pairing.pending(), None);
}

#[test]
fn a_pairing_is_withdrawn_when_its_device_leaves() {
    let (server, pairing) = pairing_server(true, |server| server);
    let mut leaving = server.connect("leaving", "Leaving");
    pair(&mut leaving);
    assert!(pairing.pending().is_some());
    // Ending its upload ends the connection.
    leaving.finish();
    let deadline = std::time::Instant::now() + TIMEOUT;
    while pairing.pending().is_some() {
        assert!(std::time::Instant::now() < deadline, "the pairing was kept");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn only_so_many_pairings_begin_within_the_window() {
    let (server, pairing) = pairing_server(true, |server| server);
    let mut quest = server.connect("quest", "Quest 3");
    for _ in 0..tomori_server::PAIRING_ATTEMPTS {
        pair(&mut quest);
        assert!(pairing.decline());
        failed(&quest);
    }
    quest.send(&ClientMessage::PairBegin {
        commitment: Nonce::generate().commitment(),
    });
    assert_eq!(
        failed(&quest),
        "Too many pairings began recently; try again in a few minutes."
    );
    assert!(pairing.paused());

    // An owner who sees it on the status page, or the operator on the console, allows pairing again.
    let (_, owner) = server.issue("Ada", tomori_security::Grant::Owner);
    let owner = format!("__Host-tomori-token={owner}");
    assert!(get(&server, "/", &owner).contains("Pairing is paused"));
    assert_eq!(post(&server, "/pairing/allow", &owner, "").0, 303);
    assert!(!pairing.paused());
    pair(&mut quest);
    pairing.decline();
    failed(&quest);
    for _ in 1..tomori_server::PAIRING_ATTEMPTS {
        pair(&mut quest);
        pairing.decline();
        failed(&quest);
    }
    assert!(pairing.paused());
    pairing.allow();
    pair(&mut quest);
}

#[test]
fn values_are_revealed_only_in_order_and_once() {
    let (server, _) = pairing_server(true, |server| server);
    let mut quest = server.connect("quest", "Quest 3");
    quest.send(&ClientMessage::PairReveal {
        nonce: Nonce::generate().to_hex(),
    });
    assert_eq!(quest.error(), "No pairing is waiting for that.");
    pair(&mut quest);
    quest.send(&ClientMessage::PairReveal {
        nonce: Nonce::generate().to_hex(),
    });
    assert_eq!(quest.error(), "No pairing is waiting for that.");
}

#[test]
fn a_commitment_in_uppercase_is_the_same_commitment() {
    let (server, pairing) = pairing_server(true, |server| server);
    let mut quest = server.connect("quest", "Quest 3");
    let device = Nonce::generate();
    quest.send(&ClientMessage::PairBegin {
        commitment: device.commitment().to_uppercase(),
    });
    quest.wait_for(|message| matches!(message, ServerMessage::PairNonce { .. }).then_some(()));
    quest.send(&ClientMessage::PairReveal {
        nonce: device.to_hex(),
    });
    quest.wait_for(|message| matches!(message, ServerMessage::PairWaiting).then_some(()));
    assert!(pairing.pending().unwrap().code.is_some());
}

#[test]
fn pairing_needs_tls() {
    let (sender, receiver) = channel();
    let server = TestServer::with_tokens(true, move |server| {
        let server = server.with_pairing(fingerprint());
        sender.send(server.pairing()).unwrap();
        server
    });
    let _pairing = receiver.recv().unwrap();
    let mut quest = server.connect("quest", "Quest 3");
    quest.send(&ClientMessage::PairBegin {
        commitment: Nonce::generate().commitment(),
    });
    assert_eq!(failed(&quest), "Pairing needs a connection over TLS.");
}

#[test]
fn without_visitors_a_device_asking_to_pair_may_only_pair() {
    let (server, pairing) = pairing_server(false, |server| server);
    assert_eq!(server.refused(&[]).0, 401);
    let mut quest = TestClient::open_with(server.address, &[("Tomori-Pairing", "1")]);
    quest.hello("quest", "Quest 3");
    assert_eq!(quest.role, ConnectionRole::Pairing);
    // It learns nothing about the spaces, even as they change.
    let (_, owner) = server.issue("Ada", tomori_security::Grant::Owner);
    let mut ada = server.connect_with_token("ada", "Ada", &owner);
    ada.create_space("Secret plans");
    while let Some(message) = quest.next_message(Duration::from_millis(300)) {
        assert!(
            !matches!(
                message,
                ServerMessage::Spaces { .. } | ServerMessage::Space { .. }
            ),
            "{message:?}"
        );
    }
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

/// Post the form `body` to `path` with `cookie`, returning the status and response.
fn post(server: &TestServer, path: &str, cookie: &str, body: &str) -> (u16, String) {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(server.address).unwrap();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: tomori\r\nConnection: close\r\nCookie: {cookie}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
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
    let (server, pairing) = pairing_server(true, |server| server);
    let (_, owner) = server.issue("Ada", tomori_security::Grant::Owner);
    // A collaborator token for a space that exists, since one whose space is gone is revoked.
    let mut ada = server.connect_with_token("ada", "Ada", &owner);
    let studio = ada.create_space("Studio").id;
    let (_, collaborator) = server.issue(
        "Bob",
        tomori_security::Grant::Collaborator { space: studio },
    );
    let owner = format!("__Host-tomori-token={owner}");
    let collaborator = format!("__Host-tomori-token={collaborator}");
    // A device whose name carries digits, in any script, and number words.
    let mut quest = server.connect("quest", "Quest ٤٨٢ 913 four ２");
    let code = pair(&mut quest);

    let page = get(&server, "/", &owner);
    assert!(
        page.contains(&format!(
            "Pairing request from a device calling itself &quot;Quest&quot;, with code <strong>{}</strong>. Confirm only if the device you are pairing shows the same code.",
            display(&code)
        )),
        "{page}"
    );
    // No digit of the name is on the page, but the code's own.
    let (_, request) = page.split_once("Pairing request from").unwrap();
    let (device, _) = request.split_once("with code").unwrap();
    assert!(!device.chars().any(char::is_numeric), "{device}");
    assert!(!device.contains("four"), "{device}");

    let serial = format!("pairing={}", pairing.pending().unwrap().serial);

    // Only an owner can confirm.
    assert_eq!(
        post(&server, "/pairing/confirm", &collaborator, &serial).0,
        403
    );
    assert!(quest.next_message(Duration::from_millis(200)).is_none());

    // A form for another pairing, such as one that has since been withdrawn, confirms nothing.
    assert_eq!(
        post(&server, "/pairing/confirm", &owner, "pairing=999").0,
        303
    );
    assert!(pairing.pending().is_some());

    let (status, response) = post(&server, "/pairing/confirm", &owner, &serial);
    assert_eq!(status, 303, "{response}");
    let token = paired(&quest);
    assert!(token.starts_with(tomori_security::TOKEN_PREFIX));
    assert!(get(&server, "/", &owner).contains("No pairing is pending"));
}

#[test]
fn an_owner_declines_a_pairing_on_the_status_page() {
    let (server, pairing) = pairing_server(true, |server| server);
    let (_, owner) = server.issue("Ada", tomori_security::Grant::Owner);
    let owner = format!("__Host-tomori-token={owner}");
    let mut quest = server.connect("quest", "Quest 3");
    pair(&mut quest);
    let serial = pairing.pending().unwrap().serial;
    assert_eq!(
        post(&server, "/pairing/decline", &owner, "pairing=999").0,
        303
    );
    assert!(pairing.pending().is_some());
    assert_eq!(
        post(
            &server,
            "/pairing/decline",
            &owner,
            &format!("pairing={serial}")
        )
        .0,
        303
    );
    assert_eq!(failed(&quest), "The pairing was declined.");
    assert_eq!(pairing.pending(), None);
}

#[test]
fn a_client_pairs_over_tls_with_the_certificate_it_saw() {
    let certificate = tomori_security::ServerCertificate::generate();
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
        let mut options =
            tomori_client::Options::new(server.address.to_string(), "quest", "Quest 3");
        options.transport = tomori_client::Transport::Tls { pin };
        options.token = token;
        tomori_client::Client::connect(options).unwrap()
    };
    let next = |client: &tomori_client::Client| {
        client
            .events()
            .recv_timeout(TIMEOUT)
            .expect("an event arrives in time")
    };
    let quest = connect(None, None);
    let seen = loop {
        if let tomori_client::Event::State(tomori_client::ConnectionState::Connected {
            role,
            certificate,
            ..
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
        if let tomori_client::Event::Message(ServerMessage::PairNonce { nonce }) = next(&quest) {
            break Nonce::parse(&nonce).unwrap();
        }
    };
    quest
        .send(ClientMessage::PairReveal {
            nonce: device.to_hex(),
        })
        .unwrap();
    loop {
        if let tomori_client::Event::Message(ServerMessage::PairWaiting) = next(&quest) {
            break;
        }
    }
    // The operator types the code the device shows, derived from the certificate it saw.
    pairing.confirm(&code(seen, device, server_nonce)).unwrap();
    let token = loop {
        if let tomori_client::Event::Message(ServerMessage::Paired { token }) = next(&quest) {
            break token;
        }
    };
    quest.close();

    let owner = connect(Some(token), Some(seen));
    loop {
        if let tomori_client::Event::State(tomori_client::ConnectionState::Connected {
            role, ..
        }) = next(&owner)
        {
            assert_eq!(role, ConnectionRole::Owner);
            break;
        }
    }
    owner.close();
}

#[test]
fn a_paired_token_revoked_elsewhere_ends_its_connection() {
    let (server, pairing) = pairing_server(true, |server| server);
    let mut quest = server.connect("quest", "Quest 3");
    let code = pair(&mut quest);
    pairing.confirm(&code).unwrap();
    let token = paired(&quest);
    let owner = server.connect_with_token("quest", "Quest 3", &token);
    assert_eq!(owner.role, ConnectionRole::Owner);
    let id = server.tokens.as_ref().unwrap().entries()[0].id.clone();
    assert!(server.revoke_elsewhere(&id));
    assert!(owner.is_closed(TIMEOUT));
}
