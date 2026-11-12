// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Deleting a space, and perhaps its agent session, from inside it. See `design/spaces.md`.
use std::time::Duration;

use tomori_backend::{Error, ErrorKind, Event, Session};
use tomori_protocol::{Activity, ClientMessage, QueueState, ServerMessage, SpaceDeletion};
use tomori_security::Grant;
use tomori_server::fake::FakeBackend;
use tomori_server::test_support::{TestClient, TestServer, eventually};

/// The deletion the next `Left` says, if any.
fn left(client: &TestClient) -> Option<SpaceDeletion> {
    client.wait_for(|message| match message {
        ServerMessage::Left { deleted } => Some(deleted.clone()),
        _ => None,
    })
}

/// What the server says became of the session `client` asked to delete.
fn session_deleted(client: &TestClient) -> Option<String> {
    client.wait_for(|message| match message {
        ServerMessage::SessionDeleted { error } => Some(error.clone()),
        _ => None,
    })
}

/// Whether the next listing `client` is sent lists `space`.
fn lists(client: &mut TestClient, space: &str) -> bool {
    client.send(&ClientMessage::ListSpaces);
    client.wait_for(|message| match message {
        ServerMessage::Spaces { spaces } => Some(spaces.iter().any(|listed| listed.id == space)),
        _ => None,
    })
}

fn delete(client: &mut TestClient, space: &str, session: bool) {
    client.send(&ClientMessage::DeleteSpace {
        space: space.to_owned(),
        session,
    });
}

#[test]
fn only_an_owner_deletes_the_space_they_are_in_and_everyone_in_it_is_returned_to_the_lobby() {
    let server = TestServer::with_tokens(true, |server| server);
    let (_, token) = server.issue("Ada", Grant::Owner);
    let mut ada = server.connect_with_token("ada", "Ada", &token);
    let other = ada.create_space("Other").id;
    let studio = ada.create_space("Studio");
    // The server says it deletes spaces, and this one's session too, as the fake backend can.
    assert!(studio.deletes && studio.deletes_session);
    let (_, token) = server.issue(
        "Bob",
        Grant::Collaborator {
            space: studio.id.clone(),
        },
    );
    let mut bob = server.connect_with_token("bob", "Bob", &token);
    bob.send(&ClientMessage::Join {
        space: studio.id.clone(),
    });
    let mut vic = server.connect("vic", "Vic");
    vic.send(&ClientMessage::Join {
        space: studio.id.clone(),
    });
    let joined = ada.space(|state| state.participants.len() == 2 && state.visitors == 1);
    assert_eq!(joined.id, studio.id);

    // A collaborator and a visitor are refused, with or without the session, and nothing is deleted.
    for session in [false, true] {
        delete(&mut bob, &studio.id, session);
        assert_eq!(bob.error(), "Only owners can do that.");
        delete(&mut vic, &studio.id, session);
        assert_eq!(vic.error(), "Visitors can watch, but not act in a space.");
    }
    // An owner deletes only the space they are in, named as the one they saw, within the bound on identifiers.
    delete(&mut ada, &"x".repeat(1025), false);
    assert!(ada.error().contains("space identifier"));
    delete(&mut ada, &other, false);
    assert_eq!(ada.error(), "Only the space you are in can be deleted.");
    assert!(lists(&mut ada, &studio.id) && lists(&mut ada, &other));
    assert!(server.backend.deleted().is_empty());

    // Deleting it returns everyone in it to the lobby, saying who deleted it, and lists it no more.
    delete(&mut ada, &studio.id, false);
    let deleted = Some(SpaceDeletion {
        by: "Ada".into(),
        session: false,
    });
    for client in [&ada, &bob, &vic] {
        assert_eq!(left(client), deleted);
    }
    // Bob is a visitor again in the lobby.
    bob.wait_for(|message| match message {
        ServerMessage::Role {
            role: tomori_protocol::ConnectionRole::Visitor,
        } => Some(()),
        _ => None,
    });
    for client in [&mut ada, &mut bob, &mut vic] {
        assert!(!lists(client, &studio.id));
        assert!(lists(client, &other));
        client.send(&ClientMessage::Join {
            space: studio.id.clone(),
        });
        assert_eq!(client.error(), "That space does not exist.");
    }
    // Its session is kept, closed, without Tomori's instructions, and can be bound to a space again.
    let session = server.backend.started()[1].clone();
    assert!(server.backend.deleted().is_empty());
    eventually(|| session.log().closed.then_some(()));
    eventually(|| {
        server
            .backend
            .forgotten()
            .iter()
            .any(|(forgotten, _)| *forgotten == session.id())
            .then_some(())
    });
    assert!(session.instructions().is_empty());
    ada.send(&ClientMessage::CreateSpace {
        title: "Again".into(),
        backend: "fake".into(),
        directory: "/tmp".into(),
        session: Some(session.id()),
        environment: None,
        instructions: None,
    });
    // Bound to the kept session, the new space takes the session's own title, which it follows.
    let again = ada.space(|state| state.title == "Studio");
    assert_ne!(again.id, studio.id);

    // Outside a space there is nothing to delete.
    ada.send(&ClientMessage::Leave);
    left(&ada);
    delete(&mut ada, &again.id, false);
    assert_eq!(ada.error(), "Join a space first.");
}

#[test]
fn deleting_the_session_too_deletes_only_the_session_bound_to_the_space() {
    let server = TestServer::start();
    let mut ada = server.connect("ada", "Ada");
    let kept = ada.create_space("Kept");
    let studio = ada.create_space("Studio");
    let mut bob = server.connect("bob", "Bob");
    bob.send(&ClientMessage::Join {
        space: studio.id.clone(),
    });
    ada.space(|state| state.participants.len() == 2);
    let kept_session = server.backend.started()[0].id();
    let session = server.backend.started()[1].id();

    delete(&mut ada, &studio.id, true);
    let deleted = Some(SpaceDeletion {
        by: "Ada".into(),
        session: true,
    });
    assert_eq!(left(&ada), deleted);
    assert_eq!(left(&bob), deleted);
    // Only the owner who asked is told what became of it.
    assert_eq!(session_deleted(&ada), None);
    assert!(
        bob.next_message(Duration::from_millis(300))
            .into_iter()
            .all(|message| !matches!(message, ServerMessage::SessionDeleted { .. }))
    );
    assert_eq!(server.backend.deleted(), std::slice::from_ref(&session));
    // The other space's session is untouched.
    assert!(
        server
            .backend
            .started()
            .iter()
            .any(|started| started.id() == kept_session)
    );
    assert!(lists(&mut ada, &kept.id));
    ada.send(&ClientMessage::Join {
        space: kept.id.clone(),
    });
    ada.space(|state| state.id == kept.id && state.error.is_none());
    // The deleted session cannot be bound again.
    ada.send(&ClientMessage::CreateSpace {
        title: "Again".into(),
        backend: "fake".into(),
        directory: "/tmp".into(),
        session: Some(session),
        environment: None,
        instructions: None,
    });
    assert!(ada.error().contains("no fake session"));
}

#[test]
fn a_session_the_backend_cannot_delete_is_kept_and_the_owner_told_why() {
    let server = TestServer::start();
    let mut ada = server.connect("ada", "Ada");
    let studio = ada.create_space("Studio");
    let session = server.backend.started()[0].clone();
    server.backend.fail_deleting(Some(Error::new(
        ErrorKind::Unavailable,
        "the agent is unreachable",
    )));
    delete(&mut ada, &studio.id, true);
    // The space is deleted all the same.
    assert_eq!(
        left(&ada),
        Some(SpaceDeletion {
            by: "Ada".into(),
            session: true,
        })
    );
    assert_eq!(
        session_deleted(&ada).as_deref(),
        Some("the agent is unreachable")
    );
    assert!(!lists(&mut ada, &studio.id));
    assert!(server.backend.deleted().is_empty());
    // Kept, it keeps none of Tomori's instructions.
    eventually(|| {
        server
            .backend
            .forgotten()
            .iter()
            .any(|(forgotten, _)| *forgotten == session.id())
            .then_some(())
    });

    // A session the agent no longer has is gone already, which is what was asked.
    server
        .backend
        .fail_deleting(Some(Error::new(ErrorKind::NotFound, "no such session")));
    let gone = ada.create_space("Gone");
    delete(&mut ada, &gone.id, true);
    left(&ada);
    assert_eq!(session_deleted(&ada), None);
}

#[test]
fn a_backend_that_does_not_delete_sessions_refuses_deleting_one_and_deletes_the_space_alone() {
    let backend = FakeBackend::new();
    backend.delete_nothing();
    let server = TestServer::with_backend(backend);
    let mut ada = server.connect("ada", "Ada");
    let studio = ada.create_space("Studio");
    assert!(studio.deletes && !studio.deletes_session);
    delete(&mut ada, &studio.id, true);
    assert_eq!(
        ada.error(),
        "The fake backend cannot delete its sessions, so nothing was deleted."
    );
    assert!(lists(&mut ada, &studio.id));
    // The owner is still in it, and can delete it alone.
    delete(&mut ada, &studio.id, false);
    assert_eq!(
        left(&ada),
        Some(SpaceDeletion {
            by: "Ada".into(),
            session: false,
        })
    );
    assert!(!lists(&mut ada, &studio.id));
}

#[test]
fn deleting_during_a_reply_with_input_queued_keeps_the_session_and_sends_nothing_more() {
    let server = TestServer::with_backend(FakeBackend::manual());
    let mut ada = server.connect("ada", "Ada");
    let studio = ada.create_space("Studio");
    let session = server.backend.started()[0].clone();
    ada.say("u1", "run the tests");
    eventually(|| session.is_busy().then_some(()));
    session.start_message();
    ada.say("u2", "and then the docs");
    ada.space(|state| {
        state.activity == Activity::Running
            && state
                .queue
                .iter()
                .any(|entry| entry.state == QueueState::Queued)
    });
    delete(&mut ada, &studio.id, false);
    left(&ada);
    eventually(|| session.log().closed.then_some(()));
    // The reply under way is not interrupted, and the input queued is never sent.
    let log = session.log();
    assert_eq!(log.interrupts, 0);
    assert_eq!(log.prompts.len(), 1);
    assert!(server.backend.deleted().is_empty());
}

#[test]
fn deleting_while_the_agent_reconnects_deletes_the_space_at_once_and_the_session_once_it_answers() {
    let server = TestServer::start();
    let mut ada = server.connect("ada", "Ada");
    let studio = ada.create_space("Studio");
    let session = server.backend.started()[0].clone();
    session.emit(Event::Reconnecting {
        reason: "The agent's connection dropped.".into(),
    });
    ada.space(|state| state.error.as_deref() == Some("The agent's connection dropped."));
    server.backend.hold_commands();
    delete(&mut ada, &studio.id, true);
    assert_eq!(
        left(&ada),
        Some(SpaceDeletion {
            by: "Ada".into(),
            session: true,
        })
    );
    assert!(!lists(&mut ada, &studio.id));
    // The session waits for the agent, and is deleted once it answers.
    assert!(
        ada.next_message(Duration::from_millis(300))
            .into_iter()
            .all(|message| !matches!(message, ServerMessage::SessionDeleted { .. }))
    );
    assert!(server.backend.deleted().is_empty());
    server.backend.release_commands();
    assert_eq!(session_deleted(&ada), None);
    assert_eq!(server.backend.deleted(), [session.id()]);

    // An owner who reconnects meanwhile is not told on the new connection, which did not ask.
    let studio = ada.create_space("Studio");
    server.backend.hold_commands();
    delete(&mut ada, &studio.id, true);
    left(&ada);
    drop(ada);
    let ada = server.connect("ada", "Ada");
    server.backend.release_commands();
    eventually(|| (server.backend.deleted().len() == 2).then_some(()));
    assert!(
        std::iter::from_fn(|| ada.next_message(Duration::from_millis(300)))
            .all(|message| !matches!(message, ServerMessage::SessionDeleted { .. }))
    );
}

#[test]
fn a_space_whose_session_has_closed_is_deleted_with_it() {
    let server = TestServer::start();
    let mut ada = server.connect("ada", "Ada");
    let studio = ada.create_space("Studio");
    let session = server.backend.started()[0].clone();
    session.emit(Event::Closed {
        reason: Some("The agent finished.".into()),
    });
    ada.space(|state| state.activity == Activity::Closed);
    eventually(|| session.log().closed.then_some(()));
    delete(&mut ada, &studio.id, true);
    left(&ada);
    assert_eq!(session_deleted(&ada), None);
    assert_eq!(server.backend.deleted(), [session.id()]);
}

#[test]
fn a_session_another_space_is_bound_to_is_never_deleted_with_a_space() {
    let server = TestServer::start();
    let mut ada = server.connect("ada", "Ada");
    let first = ada.create_space("First");
    let session = server.backend.started()[0].clone();
    // Its session ends, and another space is bound to it again.
    session.emit(Event::Closed { reason: None });
    ada.space(|state| state.id == first.id && state.activity == Activity::Closed);
    eventually(|| session.log().closed.then_some(()));
    ada.send(&ClientMessage::CreateSpace {
        title: "Second".into(),
        backend: "fake".into(),
        directory: "/tmp".into(),
        session: Some(session.id()),
        environment: None,
        instructions: None,
    });
    // Bound to the first space's session, the second takes that session's own title.
    let second = ada.space(|state| state.title == "First" && state.id != first.id);
    ada.join(&first.id);
    delete(&mut ada, &first.id, true);
    assert_eq!(
        ada.error(),
        "Its agent session is bound to another space too, so nothing was deleted."
    );
    // Deleting the closed space alone leaves the other space's session, and its instructions, as they are.
    delete(&mut ada, &first.id, false);
    left(&ada);
    assert!(lists(&mut ada, &second.id));
    std::thread::sleep(Duration::from_millis(200));
    assert!(server.backend.deleted().is_empty());
    assert!(
        !server
            .backend
            .forgotten()
            .iter()
            .any(|(forgotten, _)| *forgotten == session.id())
    );
}
