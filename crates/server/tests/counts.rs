// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use discovery::Counts;
use protocol::ClientMessage;
use server::Server;
use server::fake::FakeBackend;
use server::test_support::{TIMEOUT, TestServer, TextTranscriber};

/// Long enough that a few local connections fit within it.
const INTERVAL: Duration = Duration::from_millis(500);

/// A server that reports its counts to the returned receiver, at most once per [`INTERVAL`].
fn counting() -> (TestServer, Receiver<Counts>) {
    let (sender, counts) = channel();
    let server = TestServer::with_server(FakeBackend::new(), move |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default()))
            .with_counts(move |counts| {
                let _ = sender.send(counts);
            })
            .with_counts_interval(INTERVAL)
    });
    (server, counts)
}

fn counts(spaces: u32, participants: u32) -> Counts {
    Counts {
        spaces,
        participants,
    }
}

#[test]
fn counts_are_published_as_spaces_and_participants_come_and_go() {
    let (server, published) = counting();
    let next = || {
        published
            .recv_timeout(TIMEOUT)
            .expect("the counts are published")
    };
    let quiet = || published.recv_timeout(INTERVAL * 3).ok();

    let mut ada = server.connect("ada", "Ada");
    assert_eq!(next(), counts(0, 1));
    let space = ada.create_space("Counted").id;
    assert_eq!(next(), counts(1, 1));

    let mut bob = server.connect("bob", "Bob");
    assert_eq!(next(), counts(1, 2));

    // Joining and leaving a space changes neither count, so nothing is published.
    bob.send(&ClientMessage::Join { space });
    bob.space(|state| state.participants.len() == 2);
    bob.send(&ClientMessage::Leave);
    assert_eq!(quiet(), None);

    bob.finish();
    assert_eq!(next(), counts(1, 1));
    ada.finish();
    assert_eq!(next(), counts(1, 0));
}

#[test]
fn a_burst_of_changes_is_published_once() {
    let (server, published) = counting();
    let clients: Vec<_> = ["ada", "bob", "carol"]
        .into_iter()
        .map(|participant| server.connect(participant, participant))
        .collect();
    assert_eq!(
        published
            .recv_timeout(TIMEOUT)
            .expect("the counts are published"),
        counts(0, 3)
    );

    // Within the interval after that, a change that is undone is not published at all.
    let mut dave = server.connect("dave", "Dave");
    dave.finish();
    assert!(dave.is_closed(TIMEOUT));
    assert_eq!(published.recv_timeout(INTERVAL * 3).ok(), None);
    drop(clients);
}
