// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Content by digest through the hub: a client that reads content is given a ticket and the assets of the environments
//! offered to it, after its welcome, and one that does not, or is admitted only to pair, is given neither; a ticket
//! reads only for a connection of its own principal, only what that connection is offered, and only so many requests
//! at once; and it ends with its connection. See `design/content.md`.
use super::*;
use crate::content::{OPEN_REQUESTS, Refusal, ServedAsset};
use tomori_assets::{AssetRef, Digest, Embedded, Library};
use tomori_protocol::{ContentKind, EnvironmentVersion, OfferedAsset};

/// A hub offering a garden, which serves a lantern, and a pond, which serves nothing, and the lantern's bytes.
fn garden() -> (Arc<Hub>, &'static [u8]) {
    let lantern = crate::content::tests::content(ContentKind::Model, "the hub's lantern", 48);
    Library::shared().add(Embedded::new().with(lantern));
    let (hub, _jobs) = hub();
    hub.set_environments(Environments::new([
        crate::Environment::new("garden", "Garden").with_asset(ServedAsset::new(
            AssetRef::new("garden", 1, "lantern.glb", Digest::of(lantern)),
            ContentKind::Model,
        )),
        crate::Environment::new("pond", "Pond"),
    ]));
    (hub, lantern)
}

fn holder(principal: &str) -> Access {
    Access {
        principal: Some(principal.to_owned()),
        name: None,
        grant: Grant::Owner,
    }
}

/// A client admitted as `access` that says hello as `participant`, presenting `presents`, reading content if `content`,
/// and what it was sent.
fn greeted(
    hub: &Arc<Hub>,
    access: Access,
    participant: &str,
    presents: Option<Vec<EnvironmentVersion>>,
    content: bool,
) -> (Client, Vec<ServerMessage>) {
    let client = Client::connect(hub, access);
    client.send(ClientMessage::Hello {
        protocol: PROTOCOL_VERSION,
        participant: participant.to_owned(),
        name: participant.to_owned(),
        names: false,
        environments: presents,
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
        content,
    });
    let sent = client.read();
    (client, sent)
}

/// The ticket and assets a client was sent, if it was.
fn offer(sent: &[ServerMessage]) -> Option<(String, Vec<OfferedAsset>)> {
    sent.iter().find_map(|message| match message {
        ServerMessage::Content { session, assets } => Some((session.clone(), assets.clone())),
        _ => None,
    })
}

#[test]
fn a_client_that_reads_content_is_given_a_ticket_and_the_assets_it_may_fetch_after_its_welcome() {
    let (hub, lantern) = garden();
    let (_client, sent) = greeted(&hub, holder("ada"), "ada", None, true);
    assert!(matches!(sent[0], ServerMessage::Welcome { .. }));
    assert!(matches!(sent[1], ServerMessage::Content { .. }));
    let (ticket, assets) = offer(&sent).unwrap();
    assert_eq!(ticket.len(), 64);
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0].digest, Digest::of(lantern).to_string());
    let served = hub
        .content(
            &holder("ada"),
            &ticket,
            &Digest::of(lantern),
            &Cancellation::new(),
        )
        .unwrap();
    assert_eq!((&*served.bytes, served.kind), (lantern, ContentKind::Model));
    // Each connection's ticket is its own.
    let (_other, sent) = greeted(&hub, holder("ada"), "ada-laptop", None, true);
    assert_ne!(offer(&sent).unwrap().0, ticket);
}

#[test]
fn a_client_that_does_not_read_content_or_only_pairs_is_given_no_ticket() {
    let (hub, _lantern) = garden();
    let (_older, sent) = greeted(&hub, holder("ada"), "ada", None, false);
    assert!(offer(&sent).is_none());
    let (_pairing, sent) = greeted(&hub, Access::pairing(), "device", None, true);
    assert!(offer(&sent).is_none());
}

#[test]
fn a_client_is_offered_only_the_assets_of_the_environments_it_presents() {
    let (hub, lantern) = garden();
    let pond = vec![EnvironmentVersion {
        id: "pond".to_owned(),
        version: 1,
    }];
    let (_client, sent) = greeted(&hub, holder("ada"), "ada", Some(pond), true);
    let (ticket, assets) = offer(&sent).unwrap();
    assert!(assets.is_empty());
    assert_eq!(
        hub.content(
            &holder("ada"),
            &ticket,
            &Digest::of(lantern),
            &Cancellation::new()
        )
        .err(),
        Some(Refusal::Missing)
    );
    // Nor the garden at another version.
    let older = vec![EnvironmentVersion {
        id: "garden".to_owned(),
        version: 2,
    }];
    let (_client, sent) = greeted(&hub, holder("ada"), "ada-2", Some(older), true);
    assert!(offer(&sent).unwrap().1.is_empty());
}

#[test]
fn a_ticket_reads_only_for_its_own_principal_and_only_so_many_at_once_until_its_connection_ends() {
    let (hub, lantern) = garden();
    let digest = Digest::of(lantern);
    let (client, sent) = greeted(&hub, holder("ada"), "ada", None, true);
    let (ticket, _) = offer(&sent).unwrap();
    let refused = |access: &Access, ticket: &str| {
        hub.content(access, ticket, &digest, &Cancellation::new())
            .err()
    };
    assert_eq!(refused(&holder("grace"), &ticket), Some(Refusal::Unbound));
    assert_eq!(refused(&Access::visitor(), &ticket), Some(Refusal::Unbound));
    assert_eq!(
        refused(&holder("ada"), "not a ticket"),
        Some(Refusal::Unbound)
    );
    assert_eq!(
        refused(&holder("ada"), &"0".repeat(64)),
        Some(Refusal::Unbound)
    );
    assert_eq!(
        hub.content(
            &holder("ada"),
            &ticket,
            &Digest::of(b"never kept"),
            &Cancellation::new()
        )
        .err(),
        Some(Refusal::Missing)
    );
    let open: Vec<_> = (0..OPEN_REQUESTS)
        .map(|_| {
            hub.content(&holder("ada"), &ticket, &digest, &Cancellation::new())
                .unwrap()
        })
        .collect();
    assert_eq!(refused(&holder("ada"), &ticket), Some(Refusal::Busy));
    drop(open);
    assert!(
        hub.content(&holder("ada"), &ticket, &digest, &Cancellation::new())
            .is_ok()
    );
    // A visitor's ticket reads for a visitor, as its connection has no principal either.
    let (_visitor, sent) = greeted(&hub, Access::visitor(), "visitor", None, true);
    let (visiting, _) = offer(&sent).unwrap();
    assert!(
        hub.content(&Access::visitor(), &visiting, &digest, &Cancellation::new())
            .is_ok()
    );
    assert_eq!(refused(&holder("ada"), &visiting), Some(Refusal::Unbound));
    // The ticket lasts as long as its connection.
    hub.disconnect(client.connection);
    assert_eq!(refused(&holder("ada"), &ticket), Some(Refusal::Unbound));
}

#[test]
fn a_content_response_whose_client_takes_nothing_for_the_keepalive_timeout_is_ended() {
    let (hub, lantern) = garden();
    let (_client, sent) = greeted(&hub, holder("ada"), "ada", None, true);
    let (ticket, _) = offer(&sent).unwrap();
    let digest = Digest::of(lantern);
    let [stalled, reading, finished] = [(); 3].map(|()| Cancellation::new());
    let mut served: Vec<_> = [&stalled, &reading, &finished]
        .map(|cancellation| {
            hub.content(&holder("ada"), &ticket, &digest, cancellation)
                .unwrap()
        })
        .into();
    // A response that has gone is no longer watched.
    drop(served.pop());
    let keepalive = Keepalive::default();
    let later = Instant::now() + keepalive.timeout;
    hub.sweep(
        later - Duration::from_millis(500),
        &keepalive,
        Duration::from_secs(10),
    );
    assert!(!stalled.is_cancelled() && !reading.is_cancelled());
    std::thread::sleep(Duration::from_millis(1));
    served[1].progress.advanced();
    hub.sweep(later, &keepalive, Duration::from_secs(10));
    assert!(stalled.is_cancelled());
    assert!(!reading.is_cancelled() && !finished.is_cancelled());
}
