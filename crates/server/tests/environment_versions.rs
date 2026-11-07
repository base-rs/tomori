// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Agreeing on each environment's version between the server and each client. See `design/environments.md`.
use std::sync::Arc;

use tomori_backend::Event;
use tomori_protocol::{
    Activity, ClientMessage, ENVIRONMENT_LIMIT, EnvironmentVersion, ServerMessage, SpaceState,
};
use tomori_security::Grant;
use tomori_server::fake::{FAKE_BACKEND_NAME, FakeBackend};
use tomori_server::test_support::{TestClient, TestServer, TextTranscriber};
use tomori_server::{Environment, Server};

/// Tomori's own environments, and another project's at version 2.
fn offered() -> [Environment; 3] {
    [
        Environment::new("sky", "Sky"),
        Environment::new("minimal", "Grid"),
        Environment::new("observe", "Observe").with_version(2),
    ]
}

fn server() -> TestServer {
    TestServer::with_server(FakeBackend::new(), |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default())).with_environments(offered())
    })
}

/// A client that presents `presents`, connected as `participant`, and the environments it was offered.
fn present(
    server: &TestServer,
    participant: &str,
    presents: Option<&[(&str, u32)]>,
) -> (TestClient, Vec<(String, u32)>) {
    let mut client = TestClient::open(server.address);
    let offered = client.hello_presenting(participant, participant, listed(presents));
    let offered = offered
        .into_iter()
        .map(|info| (info.id, info.version))
        .collect();
    (client, offered)
}

fn listed(presents: Option<&[(&str, u32)]>) -> Option<Vec<EnvironmentVersion>> {
    presents.map(|presents| {
        presents
            .iter()
            .map(|(id, version)| EnvironmentVersion::new(*id, *version))
            .collect()
    })
}

fn create(client: &mut TestClient, title: &str, environment: Option<&str>) {
    client.send(&ClientMessage::CreateSpace {
        title: title.to_owned(),
        backend: FAKE_BACKEND_NAME.to_owned(),
        directory: "/tmp".to_owned(),
        session: None,
        environment: environment.map(str::to_owned),
        instructions: None,
    });
}

fn join(client: &mut TestClient, id: &str) -> SpaceState {
    client.send(&ClientMessage::Join {
        space: id.to_owned(),
    });
    client.space(|state| state.id == id)
}

fn change(client: &mut TestClient, environment: &str) {
    client.send(&ClientMessage::SetEnvironment {
        environment: environment.to_owned(),
    });
}

/// Check that nothing `client` sent was refused: the server answers a listing after it, with no error before.
fn assert_quiet(client: &mut TestClient) {
    client.send(&ClientMessage::ListSpaces);
    client.wait_for(|message| match message {
        ServerMessage::Error { error, .. } => panic!("refused: {error}"),
        ServerMessage::Spaces { .. } => Some(()),
        _ => None,
    });
}

fn owned(versions: &[(&str, u32)]) -> Vec<(String, u32)> {
    versions
        .iter()
        .map(|(id, version)| ((*id).to_owned(), *version))
        .collect()
}

#[test]
fn each_client_is_offered_only_the_environments_it_presents_at_their_versions() {
    let server = server();
    // A client from before versions lists nothing, and is offered every environment, with its version.
    let (_older, offered) = present(&server, "older", None);
    assert_eq!(
        offered,
        owned(&[("sky", 1), ("minimal", 1), ("observe", 2)])
    );
    // A client is offered those it lists at the same version, in the server's order.
    let (_tomori, offered) = present(
        &server,
        "tomori",
        Some(&[("minimal", 1), ("observe", 1), ("sky", 1), ("floor", 1)]),
    );
    assert_eq!(offered, owned(&[("sky", 1), ("minimal", 1)]));
    let (_observer, offered) = present(&server, "observer", Some(&[("observe", 2)]));
    assert_eq!(offered, owned(&[("observe", 2)]));
    // Only the first entries of a long list are read.
    let mut long: Vec<(&str, u32)> = vec![("floor", 1); ENVIRONMENT_LIMIT];
    long.push(("sky", 1));
    let (_long, offered) = present(&server, "long", Some(&long));
    assert!(offered.is_empty());
    long.truncate(ENVIRONMENT_LIMIT - 1);
    long.push(("sky", 1));
    let (_bounded, offered) = present(&server, "bounded", Some(&long));
    assert_eq!(offered, owned(&[("sky", 1)]));
    // An identifier no environment could have, empty or longer than the server takes, is skipped before the list is
    // bounded, so it takes no place in it.
    let oversized = "o".repeat(1025);
    let mut odd: Vec<(&str, u32)> = vec![("", 1); ENVIRONMENT_LIMIT];
    odd.extend([(oversized.as_str(), 1); ENVIRONMENT_LIMIT]);
    odd.push(("sky", 1));
    let (_odd, offered) = present(&server, "odd", Some(&odd));
    assert_eq!(offered, owned(&[("sky", 1)]));
}

#[test]
fn a_client_creates_spaces_only_in_environments_it_presents() {
    let server = server();
    // A space whose creator names none is bound to the first environment offered to it, and its state names the
    // environment's version.
    let (mut observer, _) = present(&server, "observer", Some(&[("observe", 2)]));
    create(&mut observer, "Topology", None);
    let space = observer.space(|state| state.title == "Topology");
    assert_eq!(space.environment, "observe");
    assert_eq!(space.environment_version, Some(2));

    // One it was not offered is refused, naming those it was, and, for one it lists at another version, the version
    // the server offers.
    let (mut tomori, _) = present(
        &server,
        "tomori",
        Some(&[("sky", 1), ("minimal", 1), ("observe", 1)]),
    );
    create(&mut tomori, "Elsewhere", Some("observe"));
    assert_eq!(
        tomori.error(),
        "This client cannot present observe at version 2; available: sky, minimal."
    );
    create(&mut tomori, "Elsewhere", Some("floor"));
    assert_eq!(
        tomori.error(),
        "Unknown environment; available: sky, minimal."
    );
    create(&mut tomori, "Night", None);
    let night = tomori.space(|state| state.title == "Night");
    assert_eq!(
        (night.environment.as_str(), night.environment_version),
        ("sky", Some(1))
    );

    // A client offered nothing cannot create a space, whether or not it names an environment.
    let (mut lacking, offered) = present(&server, "lacking", Some(&[]));
    assert!(offered.is_empty());
    create(&mut lacking, "Nowhere", None);
    assert_eq!(
        lacking.error(),
        "The server offers no environment this client can present."
    );
    create(&mut lacking, "Nowhere", Some("sky"));
    assert_eq!(
        lacking.error(),
        "The server offers no environment this client can present."
    );

    // The lobby lists every space, whatever its environment, to a client that cannot present it too.
    tomori.send(&ClientMessage::ListSpaces);
    tomori.wait_for(|message| match message {
        ServerMessage::Spaces { spaces }
            if spaces
                .iter()
                .any(|listed| listed.id == space.id && listed.environment == "observe")
                && spaces.iter().any(|listed| listed.id == night.id) =>
        {
            Some(())
        }
        _ => None,
    });

    // Anyone may still join a space in an environment they cannot present, and is told its version.
    assert_eq!(join(&mut lacking, &space.id).environment_version, Some(2));

    // Such a client's own request to change the environment is refused as creating one is.
    change(&mut lacking, "sky");
    assert_eq!(
        lacking.error(),
        "The server offers no environment this client can present."
    );
}

#[test]
fn an_owner_s_change_reaches_everyone_whether_or_not_they_can_present_it() {
    let server = TestServer::with_tokens(true, |server| server.with_environments(offered()));
    let (_, token) = server.issue("Ada", Grant::Owner);
    let mut ada = TestClient::open_with(
        server.address,
        &[("Authorization", &format!("Bearer {token}"))],
    );
    let offered_to_ada =
        ada.hello_presenting("ada", "Ada", listed(Some(&[("sky", 1), ("observe", 2)])));
    assert_eq!(offered_to_ada.len(), 2);
    create(&mut ada, "Shared", Some("sky"));
    let space = ada.space(|state| state.title == "Shared");

    // A change to an environment the requester does not present is refused as one the server does not offer it.
    change(&mut ada, "minimal");
    assert_eq!(
        ada.error(),
        "This client cannot present minimal at version 1; available: sky, observe."
    );

    // A collaborator without Observe, a visitor with it at another version, and a client from before versions.
    let (_, bob_token) = server.issue(
        "Bob",
        Grant::Collaborator {
            space: space.id.clone(),
        },
    );
    let bob_fields = [("Authorization", format!("Bearer {bob_token}"))];
    let bob_fields: Vec<(&str, &str)> = bob_fields.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let mut bob = TestClient::open_with(server.address, &bob_fields);
    bob.hello_presenting("bob", "Bob", listed(Some(&[("sky", 1), ("minimal", 1)])));
    join(&mut bob, &space.id);
    let mut vic = TestClient::open(server.address);
    vic.hello_presenting("vic", "Vic", listed(Some(&[("sky", 1), ("observe", 1)])));
    join(&mut vic, &space.id);
    let mut old = TestClient::open(server.address);
    old.hello("old", "Old");
    join(&mut old, &space.id);

    // Nobody else's list matters: the change goes ahead, and everyone is sent the space in the new environment and
    // version, to fall back to the grid if they cannot present it.
    change(&mut ada, "observe");
    for client in [&ada, &bob, &vic, &old] {
        let state = client.space(|state| state.environment == "observe");
        assert_eq!(state.environment_version, Some(2));
    }
    assert_quiet(&mut ada);
    // Choosing the environment the space is in already is not refused either.
    change(&mut ada, "observe");
    assert_quiet(&mut ada);

    // A reconnect's list replaces the earlier one in what it is offered.
    bob.finish();
    let mut bob = TestClient::open_with(server.address, &bob_fields);
    let offered_to_bob =
        bob.hello_presenting("bob", "Bob", listed(Some(&[("sky", 1), ("observe", 2)])));
    assert_eq!(
        offered_to_bob
            .iter()
            .map(|info| info.id.as_str())
            .collect::<Vec<_>>(),
        ["sky", "observe"]
    );
    bob.space(|state| state.id == space.id && state.environment == "observe");

    // A reconnecting owner's new list replaces the earlier one in judging their own requests, not only in what
    // Welcome offers: Observe is no longer offered to Ada, though the space is in it.
    ada.finish();
    let mut ada = TestClient::open_with(
        server.address,
        &[("Authorization", &format!("Bearer {token}"))],
    );
    ada.hello_presenting("ada", "Ada", listed(Some(&[("sky", 1)])));
    join(&mut ada, &space.id);
    change(&mut ada, "observe");
    assert_eq!(
        ada.error(),
        "This client cannot present observe at version 2; available: sky."
    );

    // A closed space keeps its environment, and says so.
    server.backend.started()[0].emit(Event::Closed { reason: None });
    ada.space(|state| state.activity == Activity::Closed);
    change(&mut ada, "sky");
    assert_eq!(ada.error(), "The agent session has stopped.");
}
