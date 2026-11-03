// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Roles, visitors and admission by tokens. See `design/security.md`.

use std::time::Duration;

use protocol::{ClientMessage, ConnectionRole, Gate, Pose, ServerMessage, SpaceState};
use security::Grant;
use server::test_support::{TIMEOUT, TestClient, TestServer, speech};

/// The role in the next role message.
fn role(client: &TestClient) -> ConnectionRole {
    client.wait_for(|message| match message {
        ServerMessage::Role { role } => Some(*role),
        _ => None,
    })
}

fn spaces(client: &mut TestClient) -> Vec<protocol::SpaceSummary> {
    client.send(&ClientMessage::ListSpaces);
    client.wait_for(|message| match message {
        ServerMessage::Spaces { spaces } => Some(spaces.clone()),
        _ => None,
    })
}

fn welcome(client: &mut TestClient, participant: &str) -> ServerMessage {
    client.send(&ClientMessage::Hello {
        protocol: protocol::PROTOCOL_VERSION,
        participant: participant.into(),
        name: participant.into(),
    });
    client.wait_for(|message| {
        matches!(message, ServerMessage::Welcome { .. }).then(|| message.clone())
    })
}

#[test]
fn an_open_server_admits_everyone_as_an_owner() {
    let server = TestServer::start();
    let ada = server.connect("ada", "Ada");
    assert_eq!(ada.role, ConnectionRole::Owner);
}

#[test]
fn without_credentials_a_connection_is_a_visitor_who_is_told_nothing_about_creating() {
    let server = TestServer::with_tokens(true, |server| server);
    let (_, token) = server.issue("Ada", Grant::Owner);
    let mut owner = TestClient::open_with(
        server.address,
        &[("Authorization", &format!("Bearer {token}"))],
    );
    let ServerMessage::Welcome {
        role,
        backends,
        roots,
        ..
    } = welcome(&mut owner, "ada")
    else {
        unreachable!()
    };
    assert_eq!(role, ConnectionRole::Owner);
    assert!(!backends.is_empty());
    assert!(!roots.is_empty());

    let mut visitor = TestClient::open(server.address);
    let ServerMessage::Welcome {
        role,
        backends,
        roots,
        environments,
        ..
    } = welcome(&mut visitor, "vic")
    else {
        unreachable!()
    };
    assert_eq!(role, ConnectionRole::Visitor);
    assert!(backends.is_empty());
    assert!(roots.is_empty());
    assert!(!environments.is_empty());

    // A visitor cannot create a space, or list an agent's sessions.
    visitor.send(&ClientMessage::CreateSpace {
        title: "Mine".into(),
        backend: "fake".into(),
        directory: "/tmp".into(),
        session: None,
        environment: None,
    });
    assert_eq!(
        visitor.error(),
        "Visitors can watch, but not act in a space."
    );
    assert!(spaces(&mut visitor).is_empty());
}

#[test]
fn refused_credentials_get_the_same_reply_whatever_the_reason() {
    let server = TestServer::with_tokens(true, |server| server);
    let (entry, revoked) = server.issue("Gone", Grant::Owner);
    server.tokens.as_ref().unwrap().revoke(&entry.id).unwrap();
    let unknown = security::generate_token();
    let replies: Vec<(u16, String)> = [
        format!("Bearer {unknown}"),
        format!("Bearer {revoked}"),
        "Bearer not-a-token".to_owned(),
        "Basic YWRhOnNlY3JldA==".to_owned(),
    ]
    .iter()
    .map(|authorization| server.refused(&[("Authorization", authorization)]))
    .collect();
    for reply in &replies {
        assert_eq!(reply, &(401, "Unauthorized.\n".to_owned()));
    }
    // A cookie is a credential too.
    assert_eq!(
        server
            .refused(&[("Cookie", &format!("tomori-token={unknown}"))])
            .0,
        401
    );
}

#[test]
fn without_visitors_a_connection_needs_credentials() {
    let server = TestServer::with_tokens(false, |server| server);
    assert_eq!(server.refused(&[]).0, 401);
    let (_, token) = server.issue("Ada", Grant::Owner);
    let ada = server.connect_with_token("ada", "Ada", &token);
    assert_eq!(ada.role, ConnectionRole::Owner);
}

#[test]
fn visitors_beyond_the_limit_are_refused_but_owners_are_not() {
    let server = TestServer::with_tokens(true, |server| server.with_max_visitors(1));
    let first = server.connect("first", "First");
    assert_eq!(first.role, ConnectionRole::Visitor);
    assert_eq!(
        server.refused(&[]),
        (503, "Too many visitors are connected.\n".to_owned())
    );
    let (_, token) = server.issue("Ada", Grant::Owner);
    let ada = server.connect_with_token("ada", "Ada", &token);
    assert_eq!(ada.role, ConnectionRole::Owner);
}

/// An owner's space, with the owner in it.
fn owned_space(server: &TestServer) -> (TestClient, SpaceState) {
    let (_, token) = server.issue("Ada", Grant::Owner);
    let mut ada = server.connect_with_token("ada", "Ada", &token);
    let space = ada.create_space("Studio");
    (ada, space)
}

#[test]
fn a_visitor_watches_and_hears_a_space_without_taking_part() {
    let server = TestServer::with_tokens(true, |server| server);
    let (mut ada, space) = owned_space(&server);
    let mut vic = server.connect("vic", "Vic");
    vic.send(&ClientMessage::Join {
        space: space.id.clone(),
    });
    let state = vic.space(|state| state.id == space.id);
    // A visitor is not a participant, nor counted as one.
    assert_eq!(state.participants.len(), 1);
    assert_eq!(spaces(&mut vic)[0].participants, 1);

    // The visitor sees the owner's presence and hears their voice.
    let head = Pose {
        position: [0.0, 1.6, 0.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    };
    ada.send(&ClientMessage::Presence {
        head,
        left: None,
        right: None,
        hands: None,
    });
    let source = vic.wait_for(|message| match message {
        ServerMessage::Presence { source, .. } => Some(*source),
        _ => None,
    });
    assert_eq!(source, ada.source);
    ada.send_audio(Gate::Live, speech("hello"));
    assert_eq!(vic.next_audio(TIMEOUT).unwrap().source, ada.source);

    // The visitor's presence is refused, and their audio is neither relayed nor transcribed.
    vic.send(&ClientMessage::Presence {
        head,
        left: None,
        right: None,
        hands: None,
    });
    assert_eq!(vic.error(), "Visitors can watch, but not act in a space.");
    vic.send_audio(Gate::Live, speech("not heard"));
    vic.send(&ClientMessage::UtteranceBegin {
        utterance: "u1".into(),
    });
    assert_eq!(vic.error(), "Visitors can watch, but not act in a space.");
    assert!(ada.next_audio(Duration::from_millis(300)).is_none());
}

#[test]
fn a_collaborator_takes_part_only_in_their_space() {
    let server = TestServer::with_tokens(true, |server| server);
    let (mut ada, studio) = owned_space(&server);
    let other = ada.create_space("Other");
    let (_, token) = server.issue(
        "Bob",
        Grant::Collaborator {
            space: studio.id.clone(),
        },
    );
    let mut bob = server.connect_with_token("bob", "Bob", &token);
    assert_eq!(bob.role, ConnectionRole::Visitor);

    // In the space they were admitted to, they are a participant who can talk to the agent.
    bob.send(&ClientMessage::Join {
        space: studio.id.clone(),
    });
    assert_eq!(role(&bob), ConnectionRole::Collaborator);
    // Ada moved to the other space when she created it, so Bob is the studio's only participant.
    let state = bob.space(|state| state.id == studio.id);
    let names: Vec<&str> = state
        .participants
        .iter()
        .map(|participant| participant.name.as_str())
        .collect();
    assert_eq!(names, ["Bob"]);
    bob.say("u1", "run the tests");
    bob.space(|state| state.queue.iter().any(|entry| entry.source == bob.source));

    // But they still cannot create a space.
    bob.send(&ClientMessage::CreateSpace {
        title: "Mine".into(),
        backend: "fake".into(),
        directory: "/tmp".into(),
        session: None,
        environment: None,
    });
    assert_eq!(bob.error(), "Only owners can do that.");

    // Elsewhere they are a visitor.
    bob.send(&ClientMessage::Join {
        space: other.id.clone(),
    });
    assert_eq!(role(&bob), ConnectionRole::Visitor);
    let state = bob.space(|state| state.id == other.id);
    let names: Vec<&str> = state
        .participants
        .iter()
        .map(|participant| participant.name.as_str())
        .collect();
    assert_eq!(names, ["Ada"]);
    bob.send(&ClientMessage::Leave);
    bob.send(&ClientMessage::Join {
        space: studio.id.clone(),
    });
    assert_eq!(role(&bob), ConnectionRole::Collaborator);

    // Reconnecting with the token, they are a collaborator again when they rejoin.
    drop(bob);
    let bob = server.connect_with_token("bob", "Bob", &token);
    assert_eq!(bob.role, ConnectionRole::Visitor);
    assert_eq!(role(&bob), ConnectionRole::Collaborator);
}

#[test]
fn revoking_a_token_ends_its_connections_and_refuses_it() {
    let server = TestServer::with_tokens(true, |server| server);
    let (entry, token) = server.issue("Ada", Grant::Owner);
    let ada = server.connect_with_token("ada", "Ada", &token);
    // Revoked by another process, as `tomori-server auth revoke` does: the server notices within its sweep.
    assert!(server.revoke_elsewhere(&entry.id));
    assert!(ada.is_closed(TIMEOUT));
    assert_eq!(
        server
            .refused(&[("Authorization", &format!("Bearer {token}"))])
            .0,
        401
    );
}

#[test]
fn an_owners_identity_without_their_token_is_someone_else() {
    let server = TestServer::with_tokens(true, |server| server);
    let (mut ada, space) = owned_space(&server);
    // A visitor saying the owner's identity becomes a separate visitor, with another number, and the owner stays connected.
    let impostor = server.connect("ada", "Ada");
    assert_eq!(impostor.role, ConnectionRole::Visitor);
    assert_ne!(impostor.source, ada.source);
    assert!(!ada.is_closed(Duration::from_millis(300)));
    let state = spaces(&mut ada);
    assert_eq!(state[0].id, space.id);
    assert_eq!(state[0].participants, 1);
}

/// A server that admits by tokens and serves only TLS, with a new certificate.
fn tls_server(
    configure: impl FnOnce(server::Server) -> server::Server + Send + 'static,
) -> (TestServer, security::Fingerprint) {
    let certificate = security::ServerCertificate::generate();
    let fingerprint = certificate.fingerprint();
    let config = certificate.server_config().unwrap();
    (
        TestServer::with_tokens(true, move |server| configure(server.with_tls(config))),
        fingerprint,
    )
}

/// `GET /health` over TLS, pinning `expected`: the status line, or the TLS error.
fn health_over_tls(
    address: std::net::SocketAddr,
    expected: security::Fingerprint,
) -> Result<String, std::io::Error> {
    use std::io::{BufRead, BufReader, Write};
    let pinned = security::Pinned::new(Some(expected));
    let connection = rustls::ClientConnection::new(
        security::client_config(pinned),
        rustls::pki_types::ServerName::try_from("tomori").unwrap(),
    )
    .unwrap();
    let mut stream = rustls::StreamOwned::new(connection, std::net::TcpStream::connect(address)?);
    stream.write_all(b"GET /health HTTP/1.1\r\nHost: tomori\r\nConnection: close\r\n\r\n")?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    Ok(line)
}

#[test]
fn a_server_with_tls_serves_only_tls_with_its_certificate() {
    let (server, fingerprint) = tls_server(|server| server);
    assert!(
        health_over_tls(server.address, fingerprint)
            .unwrap()
            .starts_with("HTTP/1.1 200")
    );
    // A client pinning another certificate refuses the handshake.
    let other = security::ServerCertificate::generate().fingerprint();
    assert!(health_over_tls(server.address, other).is_err());
    // Plain HTTP is not served.
    use std::io::{Read, Write};
    let mut plain = std::net::TcpStream::connect(server.address).unwrap();
    plain.set_read_timeout(Some(TIMEOUT)).unwrap();
    plain
        .write_all(b"GET /health HTTP/1.1\r\nHost: tomori\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut response = Vec::new();
    let _ = plain.read_to_end(&mut response);
    assert!(!response.starts_with(b"HTTP/1.1 200"));
}

#[test]
fn a_stalled_handshake_is_closed_after_the_request_timeout() {
    use std::io::Read;
    let (server, _) = tls_server(|server| server.with_request_timeout(Duration::from_millis(200)));
    let mut silent = std::net::TcpStream::connect(server.address).unwrap();
    silent.set_read_timeout(Some(TIMEOUT)).unwrap();
    let started = std::time::Instant::now();
    let mut buffer = [0_u8; 16];
    assert_eq!(silent.read(&mut buffer).unwrap_or(0), 0);
    assert!(started.elapsed() < TIMEOUT);
}
