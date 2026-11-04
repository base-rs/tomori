// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Advertise and browse in one process over real multicast DNS.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use tomori_discovery::{Counts, Event, advertise, browse};
use tomori_protocol::PROTOCOL_VERSION;

/// Wait for the first event matching `predicate`, ignoring other servers on the network.
fn wait_for(
    browser: &tomori_discovery::Browser,
    timeout: Duration,
    predicate: impl Fn(&Event) -> bool,
) -> Option<Event> {
    let deadline = Instant::now() + timeout;
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        match browser.recv_timeout(remaining) {
            Ok(event) if predicate(&event) => return Some(event),
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    None
}

#[test]
#[ignore = "needs a network interface with working multicast, which CI containers often lack; run with --ignored"]
fn finds_an_advertised_server_and_loses_it_when_withdrawn() {
    let instance = format!("Round Trip {}", std::process::id());
    let browser = browse().expect("browse");
    let advertisement = advertise(
        &instance,
        48765,
        IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        Counts::default(),
        None,
    )
    .expect("advertise");

    let found = wait_for(
        &browser,
        Duration::from_secs(15),
        |event| matches!(event, Event::Found(server) if server.instance == instance),
    );
    let Some(Event::Found(server)) = found else {
        panic!("the advertised server was not found");
    };
    assert_eq!(server.name, instance);
    assert_eq!(server.port, 48765);
    assert_eq!(server.protocol, Some(PROTOCOL_VERSION));
    assert!(!server.addresses.is_empty());
    assert!(server.addresses.iter().all(|address| address.is_ipv4()));
    assert_eq!((server.spaces, server.participants), (Some(0), Some(0)));

    drop(advertisement);
    let lost = wait_for(
        &browser,
        Duration::from_secs(10),
        |event| matches!(event, Event::Lost { instance: lost } if *lost == instance),
    );
    assert!(lost.is_some(), "the withdrawn server was not lost");
}

#[test]
#[ignore = "needs a network interface with working multicast, which CI containers often lack; run with --ignored"]
fn browsers_see_updated_counts() {
    let instance = format!("Counts {}", std::process::id());
    let browser = browse().expect("browse");
    let advertisement = advertise(
        &instance,
        48766,
        IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        Counts::default(),
        None,
    )
    .expect("advertise");
    let counts = |spaces, participants| {
        let instance = instance.as_str();
        move |event: &Event| {
            matches!(event, Event::Found(server) if server.instance == instance
                && server.spaces == Some(spaces)
                && server.participants == Some(participants))
        }
    };
    assert!(
        wait_for(&browser, Duration::from_secs(15), counts(0, 0)).is_some(),
        "the advertised server was not found"
    );

    // Spaced as the server spaces its updates, so that browsers flush each previous record.
    for (spaces, participants) in [(1, 1), (1, 2), (1, 1)] {
        std::thread::sleep(Duration::from_secs(2));
        advertisement
            .update(Counts {
                spaces,
                participants,
                attention: None,
            })
            .expect("update");
        assert!(
            wait_for(
                &browser,
                Duration::from_secs(10),
                counts(spaces, participants)
            )
            .is_some(),
            "the update to {spaces} spaces and {participants} participants was not seen"
        );
    }
}

/// Advertise on the loopback interface and browse for it in the same process. Nothing is advertised beyond this computer, though browsing still hears the servers on the network, which the tests pass over.
#[test]
fn finds_a_server_advertised_on_loopback_with_its_counts_and_loses_it_when_withdrawn() {
    let instance = format!("Loopback {}", std::process::id());
    let browser = browse().expect("browse");
    let fingerprint = "ab".repeat(32);
    let advertisement = advertise(
        &instance,
        48767,
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        Counts::default(),
        Some(fingerprint.clone()),
    )
    .expect("advertise");
    assert_eq!(advertisement.instance(), instance);
    assert!(format!("{advertisement:?}").starts_with("Advertisement"));

    let found = wait_for(
        &browser,
        Duration::from_secs(15),
        |event| matches!(event, Event::Found(server) if server.instance == instance),
    );
    let Some(Event::Found(server)) = found else {
        unreachable!("wait_for returns only the event it waits for, or None")
    };
    assert_eq!(server.name, instance);
    assert_eq!(
        server.addresses,
        [SocketAddr::from(([127, 0, 0, 1], 48767))]
    );
    assert_eq!(server.protocol, Some(PROTOCOL_VERSION));
    assert_eq!(server.fingerprint, Some(fingerprint));
    assert_eq!((server.spaces, server.participants), (Some(0), Some(0)));

    // Spaced as the server spaces its updates, so that browsers flush the previous record.
    std::thread::sleep(Duration::from_secs(2));
    advertisement
        .update(Counts {
            spaces: 1,
            participants: 2,
            attention: None,
        })
        .expect("update");
    let updated = wait_for(
        &browser,
        Duration::from_secs(10),
        |event| matches!(event, Event::Found(server) if server.instance == instance && server.participants == Some(2)),
    );
    assert!(updated.is_some(), "the update was not seen");

    drop(advertisement);
    let lost = wait_for(
        &browser,
        Duration::from_secs(10),
        |event| matches!(event, Event::Lost { instance: lost } if *lost == instance),
    );
    assert!(lost.is_some(), "the withdrawn server was not lost");
}
