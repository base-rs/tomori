// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Visitors knocking to collaborate in a space. See `design/security.md`.

use std::time::Duration;

use protocol::{ClientMessage, ConnectionRole, KnockRequest, KnockState, ServerMessage};
use security::Grant;
use server::test_support::{TIMEOUT, TestClient, TestServer};

/// A server that admits by tokens, issues them to admitted visitors over its tests' plain connections, and has a space with an owner in it.
fn knocking_server(
    configure: impl FnOnce(server::Server) -> server::Server + Send + 'static,
) -> (TestServer, TestClient, String) {
    let server = TestServer::with_tokens(true, move |server| {
        configure(server.with_tokens_over_plain_http_for_tests())
    });
    let (_, token) = server.issue("Ada", Grant::Owner);
    let mut ada = server.connect_with_token("ada", "Ada", &token);
    let space = ada.create_space("Studio").id;
    (server, ada, space)
}

/// A visitor watching `space`.
fn visitor(server: &TestServer, identity: &str, name: &str, space: &str) -> TestClient {
    let mut client = server.connect(identity, name);
    client.send(&ClientMessage::Join {
        space: space.to_owned(),
    });
    client.space(|state| state.id == space);
    client
}

/// The next knocks message with `count` knocks.
fn knocks_of(client: &TestClient, count: usize) -> Vec<KnockRequest> {
    client.wait_for(|message| match message {
        ServerMessage::Knocks { knocks } if knocks.len() == count => Some(knocks.clone()),
        _ => None,
    })
}

fn knock_state(client: &TestClient) -> KnockState {
    client.wait_for(|message| match message {
        ServerMessage::KnockState { state } => Some(*state),
        _ => None,
    })
}

#[test]
fn an_admitted_visitor_collaborates_in_the_space_on_the_same_connection() {
    let (server, ada, space) = knocking_server(|server| server);
    let mut vic = visitor(&server, "vic", "Vic", &space);
    let source = vic.source;
    // The participants see how many visitors are watching.
    ada.space(|state| state.visitors == 1);

    vic.send(&ClientMessage::Knock);
    assert_eq!(knock_state(&vic), KnockState::Pending);
    let waiting = knocks_of(&ada, 1);
    assert_eq!(waiting[0].name, "Vic");

    let mut ada = ada;
    ada.send(&ClientMessage::DecideKnock {
        knock: waiting[0].id.clone(),
        admit: true,
    });
    let token = vic.wait_for(|message| match message {
        ServerMessage::Admitted { token } => Some(token.clone()),
        _ => None,
    });
    let role = vic.wait_for(|message| match message {
        ServerMessage::Role { role } => Some(*role),
        _ => None,
    });
    assert_eq!(role, ConnectionRole::Collaborator);
    // Now a participant, with the same number, and no longer a visitor.
    let state = ada.space(|state| state.participants.len() == 2);
    assert_eq!(state.visitors, 0);
    assert!(
        state
            .participants
            .iter()
            .any(|participant| participant.source == source)
    );
    assert!(knocks_of(&ada, 0).is_empty());
    vic.say("u1", "hello from vic");
    ada.space(|state| state.queue.iter().any(|entry| entry.source == source));

    // The token admits the collaborator to that space only, when they connect again.
    let entries = server.tokens.as_ref().unwrap().entries();
    assert!(entries.iter().any(|entry| entry.name == "Vic"
        && entry.grant
            == Grant::Collaborator {
                space: space.clone()
            }));
    drop(vic);
    let again = server.connect_with_token("vic", "Vic", &token);
    assert_eq!(again.role, ConnectionRole::Visitor);
    let role = again.wait_for(|message| match message {
        ServerMessage::Role { role } => Some(*role),
        _ => None,
    });
    assert_eq!(role, ConnectionRole::Collaborator);
}

#[test]
fn a_declined_visitor_waits_before_knocking_there_again() {
    let (server, mut ada, space) = knocking_server(|server| server);
    let mut vic = visitor(&server, "vic", "Vic", &space);
    vic.send(&ClientMessage::Knock);
    assert_eq!(knock_state(&vic), KnockState::Pending);
    let waiting = knocks_of(&ada, 1);
    ada.send(&ClientMessage::DecideKnock {
        knock: waiting[0].id.clone(),
        admit: false,
    });
    assert_eq!(knock_state(&vic), KnockState::Declined);
    vic.send(&ClientMessage::Knock);
    assert_eq!(vic.error(), "You knocked recently; try again in a minute.");
    assert!(server.tokens.as_ref().unwrap().entries().len() == 1);
}

#[test]
fn the_first_decision_resolves_a_knock() {
    let (server, mut ada, space) = knocking_server(|server| server);
    let (_, token) = server.issue("Bea", Grant::Owner);
    let mut bea = server.connect_with_token("bea", "Bea", &token);
    bea.send(&ClientMessage::Join {
        space: space.clone(),
    });
    bea.space(|state| state.id == space);
    let mut vic = visitor(&server, "vic", "Vic", &space);
    vic.send(&ClientMessage::Knock);
    assert_eq!(knock_state(&vic), KnockState::Pending);
    let waiting = knocks_of(&ada, 1);
    knocks_of(&bea, 1);
    ada.send(&ClientMessage::DecideKnock {
        knock: waiting[0].id.clone(),
        admit: false,
    });
    knocks_of(&bea, 0);
    bea.send(&ClientMessage::DecideKnock {
        knock: waiting[0].id.clone(),
        admit: true,
    });
    assert_eq!(bea.error(), "That knock was already answered.");
    assert_eq!(knock_state(&vic), KnockState::Declined);
}

#[test]
fn knocks_are_bounded() {
    let (server, ada, space) = knocking_server(|server| server);
    let mut vic = visitor(&server, "vic", "Vic", &space);
    vic.send(&ClientMessage::Knock);
    knock_state(&vic);
    vic.send(&ClientMessage::Knock);
    assert_eq!(vic.error(), "Your knock is already waiting.");

    let mut others: Vec<TestClient> = (1..server::KNOCKS_PER_SPACE)
        .map(|index| {
            let mut other = visitor(&server, &format!("other-{index}"), "Other", &space);
            other.send(&ClientMessage::Knock);
            assert_eq!(knock_state(&other), KnockState::Pending);
            other
        })
        .collect();
    knocks_of(&ada, server::KNOCKS_PER_SPACE);
    let mut late = visitor(&server, "late", "Late", &space);
    late.send(&ClientMessage::Knock);
    assert_eq!(
        late.error(),
        "Too many knocks are waiting on this space; try again shortly."
    );
    others.clear();
}

#[test]
fn a_knock_expires() {
    let (server, ada, space) =
        knocking_server(|server| server.with_knock_timeout(Duration::from_millis(500)));
    let mut vic = visitor(&server, "vic", "Vic", &space);
    vic.send(&ClientMessage::Knock);
    knock_state(&vic);
    knocks_of(&ada, 1);
    assert_eq!(knock_state(&vic), KnockState::Expired);
    knocks_of(&ada, 0);
    // An expired knock waits like a declined one.
    vic.send(&ClientMessage::Knock);
    assert_eq!(vic.error(), "You knocked recently; try again in a minute.");
}

#[test]
fn a_knock_is_withdrawn_when_its_visitor_leaves() {
    let (server, ada, space) = knocking_server(|server| server);
    let mut wes = visitor(&server, "wes", "Wes", &space);
    wes.send(&ClientMessage::Knock);
    knock_state(&wes);
    knocks_of(&ada, 1);
    wes.send(&ClientMessage::Leave);
    assert_eq!(knock_state(&wes), KnockState::Withdrawn);
    knocks_of(&ada, 0);
}

#[test]
fn only_a_visitor_knocks_and_only_a_participant_decides() {
    let (server, mut ada, space) = knocking_server(|server| server);
    ada.send(&ClientMessage::Knock);
    assert_eq!(ada.error(), "You already take part in this space.");
    let mut vic = visitor(&server, "vic", "Vic", &space);
    vic.send(&ClientMessage::DecideKnock {
        knock: "knock-1".into(),
        admit: true,
    });
    assert_eq!(vic.error(), "Visitors can watch, but not act in a space.");
    // A visitor in the lobby has no space to knock on.
    let mut lobby = server.connect("lobby", "Lobby");
    lobby.send(&ClientMessage::Knock);
    assert_eq!(lobby.error(), "Join a space first.");
}

#[test]
fn a_collaborator_elsewhere_asks_an_owner_instead_of_knocking() {
    let (server, mut ada, studio) = knocking_server(|server| server);
    let other = ada.create_space("Other").id;
    let (_, token) = server.issue("Bob", Grant::Collaborator { space: studio });
    let mut bob = server.connect_with_token("bob", "Bob", &token);
    bob.send(&ClientMessage::Join {
        space: other.clone(),
    });
    bob.space(|state| state.id == other);
    bob.send(&ClientMessage::Knock);
    assert_eq!(
        bob.error(),
        "You are admitted elsewhere on this server; ask an owner to let you in here."
    );
}

#[test]
fn visitors_are_not_told_about_knocks() {
    let (server, _ada, space) = knocking_server(|server| server);
    let mut vic = visitor(&server, "vic", "Vic", &space);
    let wes = visitor(&server, "wes", "Wes", &space);
    vic.send(&ClientMessage::Knock);
    knock_state(&vic);
    while let Some(message) = wes.next_message(Duration::from_millis(300)) {
        assert!(
            !matches!(message, ServerMessage::Knocks { .. }),
            "{message:?}"
        );
    }
}

#[test]
fn knocking_needs_tls() {
    let server = TestServer::with_tokens(true, |server| server);
    let (_, token) = server.issue("Ada", Grant::Owner);
    let mut ada = server.connect_with_token("ada", "Ada", &token);
    let space = ada.create_space("Studio").id;
    let mut vic = visitor(&server, "vic", "Vic", &space);
    vic.send(&ClientMessage::Knock);
    assert_eq!(vic.error(), "Knocking needs a connection over TLS.");
}

#[test]
fn a_collaborator_token_whose_space_is_gone_is_revoked() {
    let (server, _ada, space) = knocking_server(|server| server);
    let (kept, _) = server.issue("Kept", Grant::Collaborator { space });
    let (gone, token) = server.issue(
        "Gone",
        Grant::Collaborator {
            space: "space-from-before".into(),
        },
    );
    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        let ids: Vec<String> = server
            .tokens
            .as_ref()
            .unwrap()
            .entries()
            .into_iter()
            .map(|entry| entry.id)
            .collect();
        if !ids.contains(&gone.id) {
            assert!(ids.contains(&kept.id));
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the token was kept");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        server
            .refused(&[("Authorization", &format!("Bearer {token}"))])
            .0,
        401
    );
}
