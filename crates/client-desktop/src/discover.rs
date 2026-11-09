// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Find Tomori servers on the network, and the spaces on a chosen one.
//!
//! `tomori discover` browses for servers the way the headset's server list does
//! ([`discovery::browse`]), and, when given a server, lists the spaces on it the
//! way the lobby does (connect and [`tomori_protocol::ClientMessage::ListSpaces`]). The
//! address and space ids it prints are what `--server` and the spectator take.
use std::{
    fmt::Write,
    sync::mpsc::RecvTimeoutError,
    time::{Duration, Instant},
};
use tomori_client::{Client, Event, Options, discovery};
use tomori_protocol::{ClientMessage, ServerMessage, SpaceSummary};

/// How long to listen for servers before printing the list.
const BROWSE_FOR: Duration = Duration::from_secs(4);
/// How long to wait for a server to answer with its spaces.
const SPACES_TIMEOUT: Duration = Duration::from_secs(6);

/// Browse for servers, then list the spaces on `server` if one is given.
pub fn run(server: Option<String>) -> anyhow::Result<()> {
    let browser = discovery::browse().map(|browser| move |timeout| browser.recv_timeout(timeout));
    let servers = found(browser, BROWSE_FOR);
    print!("{}", servers_text(&servers));
    let listed = server
        .map(|address| list_spaces(&address, SPACES_TIMEOUT))
        .transpose();
    listed.map(|spaces| report(&servers, spaces.as_deref()))
}

/// Print `spaces`, if they were listed, and say what to do next.
fn report(servers: &discovery::Servers, spaces: Option<&[SpaceSummary]>) {
    if let Some(spaces) = spaces {
        print!("{}", spaces_text(spaces));
    }
    if let Some(hint) = hint(servers, spaces) {
        eprintln!("{hint}");
    }
}

/// What to do next, after finding `servers` and, if a server was given, its `spaces`.
fn hint(servers: &discovery::Servers, spaces: Option<&[SpaceSummary]>) -> Option<&'static str> {
    match spaces {
        Some([]) => None,
        Some(_) => Some("Join one with: tomori-spectator --server ADDRESS --space SPACE_ID"),
        None if servers.compatible().is_empty() => {
            Some("No servers found. Start a server, or pass one: tomori discover --server ADDRESS")
        }
        None => {
            Some("To list the spaces on one of these, pass it: tomori discover --server ADDRESS")
        }
    }
}

/// The servers `browser` finds in `duration`, or none if it could not browse.
fn found<E: std::fmt::Display>(
    browser: Result<impl FnMut(Duration) -> Result<discovery::Event, RecvTimeoutError>, E>,
    duration: Duration,
) -> discovery::Servers {
    match browser {
        Ok(next) => collect(next, duration),
        Err(error) => {
            eprintln!("Cannot browse for servers: {error}");
            discovery::Servers::new()
        }
    }
}

/// Fold the events that `next` gives, waiting at most the time it is given, into a list, for `duration`.
fn collect(
    mut next: impl FnMut(Duration) -> Result<discovery::Event, RecvTimeoutError>,
    duration: Duration,
) -> discovery::Servers {
    let mut servers = discovery::Servers::new();
    let deadline = Instant::now() + duration;
    // Waiting no longer once the deadline has passed.
    while let Ok(event) = next(deadline.saturating_duration_since(Instant::now())) {
        servers.apply(event);
    }
    servers
}

fn servers_text(servers: &discovery::Servers) -> String {
    if servers.compatible().is_empty() && servers.incompatible().is_empty() {
        return "No servers found on the network.\n".to_owned();
    }
    let mut text = "Servers:\n".to_owned();
    for server in servers.compatible() {
        let address = server
            .address()
            .unwrap_or_else(|| format!("{}:{}", server.host, server.port));
        let _ = writeln!(text, "  {name:<24} {address}", name = server.name);
    }
    for server in servers.incompatible() {
        let _ = writeln!(
            text,
            "  {name:<24} (protocol {:?}, this build is {})",
            server.protocol,
            tomori_protocol::PROTOCOL_VERSION,
            name = server.name
        );
    }
    text
}

/// Connect to `address` and ask for its spaces, waiting at most `timeout` for them.
fn list_spaces(address: &str, timeout: Duration) -> anyhow::Result<Vec<SpaceSummary>> {
    let mut options = Options::new(address, "discover", "discover");
    // Listing needs no token: it connects as a visitor, over TLS to a server that admits by token, or plain HTTP to one in `--auth open` mode.
    options.transport = tomori_client::Transport::Automatic { pin: None };
    Client::connect(options)
        .map_err(anyhow::Error::from)
        .and_then(|client| {
            // The connection thread sends Hello and reconnects; only the space list is wanted. A new client queues it until it connects.
            client
                .send(ClientMessage::ListSpaces)
                .expect("a new client queues messages");
            let spaces = wait_for_spaces(&client, address, timeout);
            client.close();
            spaces
        })
}

/// The spaces `client` is sent, or why there are none.
fn wait_for_spaces(
    client: &Client,
    address: &str,
    timeout: Duration,
) -> anyhow::Result<Vec<SpaceSummary>> {
    let deadline = Instant::now() + timeout;
    let mut connected = false;
    loop {
        // Drain every event that is waiting, which stops the search.
        while let Some(event) = client.try_event() {
            if let Some(answer) = answer(event, &mut connected, address) {
                return answer;
            }
        }
        if Instant::now() >= deadline {
            anyhow::bail!(unanswered(address, connected));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// What `event` says about the spaces on the server at `address`, if anything, noting whether it is `connected`.
fn answer(
    event: Event,
    connected: &mut bool,
    address: &str,
) -> Option<anyhow::Result<Vec<SpaceSummary>>> {
    match event {
        Event::State(state) => {
            *connected = state.is_connected();
            None
        }
        Event::Message(ServerMessage::Spaces { spaces }) => Some(Ok(spaces)),
        Event::Message(ServerMessage::Error { error, .. }) => Some(Err(anyhow::anyhow!(
            "The server at {address} refused: {error}"
        ))),
        _ => None,
    }
}

/// Why no spaces came from the server at `address`, given whether it was `connected`.
fn unanswered(address: &str, connected: bool) -> String {
    if connected {
        format!("The server at {address} sent no space list")
    } else {
        format!("The server at {address} did not answer in time")
    }
}

fn spaces_text(spaces: &[SpaceSummary]) -> String {
    if spaces.is_empty() {
        return "The server has no spaces yet.\n".to_owned();
    }
    let mut text = "Spaces:\n".to_owned();
    for space in spaces {
        let _ = writeln!(
            text,
            "  {:<12} {:<24} {}, {} participant(s)",
            space.id, space.title, space.backend, space.participants
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Server;
    use std::net::SocketAddr;
    use tomori_client::{ConnectionState, Disconnect};

    fn server(name: &str, protocol: u32, addresses: Vec<SocketAddr>) -> discovery::Server {
        discovery::Server {
            instance: name.to_owned(),
            name: name.to_owned(),
            host: format!("{}.local.", name.to_lowercase()),
            addresses,
            port: 8765,
            protocol: Some(protocol),
            spaces: None,
            participants: None,
            waiting: None,
            problems: None,
            replies: None,
            fingerprint: None,
        }
    }

    #[test]
    fn servers_found_while_browsing_are_listed_by_address() {
        let (sender, events) = std::sync::mpsc::channel();
        let current = tomori_protocol::PROTOCOL_VERSION;
        let address = "10.0.0.2:8765".parse().unwrap();
        for event in [
            discovery::Event::Found(server("Studio", current, vec![address])),
            discovery::Event::Found(server("Attic", current, vec![])),
            discovery::Event::Incompatible(server("Old", current + 1, vec![address])),
        ] {
            sender.send(event).unwrap();
        }
        let browser: Result<_, &str> = Ok(move |timeout| events.recv_timeout(timeout));
        let servers = found(browser, Duration::from_millis(50));
        assert_eq!(
            servers_text(&servers),
            format!(
                "Servers:\n  Attic                    attic.local.:8765\n  Studio                   10.0.0.2:8765\n  Old                      (protocol Some({}), this build is {current})\n",
                current + 1
            )
        );
    }

    #[test]
    fn nothing_is_listed_when_browsing_finds_nothing_or_cannot_browse() {
        let none =
            Err::<fn(Duration) -> Result<discovery::Event, RecvTimeoutError>, _>("no multicast");
        let servers = found(none, BROWSE_FOR);
        assert_eq!(servers_text(&servers), "No servers found on the network.\n");
    }

    #[test]
    fn a_servers_spaces_are_listed_with_the_ids_to_join_by() {
        let server = Server::start();
        let bea = server.remote("Bea", Some(server.owner("Bea")));
        assert!(
            list_spaces(&server.address(), SPACES_TIMEOUT)
                .unwrap()
                .is_empty()
        );
        assert_eq!(spaces_text(&[]), "The server has no spaces yet.\n");
        let space = bea.create_space("Studio", || {});
        let spaces = list_spaces(&server.address(), SPACES_TIMEOUT).unwrap();
        assert_eq!(
            spaces_text(&spaces),
            format!(
                "Spaces:\n  {:<12} Studio                   fake, 1 participant(s)\n",
                space.id
            )
        );
    }

    #[test]
    fn a_server_that_does_not_answer_or_refuses_is_reported() {
        let error = list_spaces("127.0.0.1:9", Duration::from_millis(200)).unwrap_err();
        assert_eq!(
            error.to_string(),
            "The server at 127.0.0.1:9 did not answer in time"
        );
        assert_eq!(
            unanswered("host:1", true),
            "The server at host:1 sent no space list"
        );
        let mut connected = false;
        let refused = answer(
            Event::Message(ServerMessage::Error {
                error: "pair to join".into(),
                space: None,
                panel: None,
            }),
            &mut connected,
            "host:1",
        );
        assert_eq!(
            refused.unwrap().unwrap_err().to_string(),
            "The server at host:1 refused: pair to join"
        );
        assert!(
            answer(
                Event::State(ConnectionState::Disconnected {
                    reason: Disconnect::Timeout,
                    retry_in: Duration::from_secs(1),
                }),
                &mut connected,
                "host:1"
            )
            .is_none()
        );
        assert!(!connected);
        assert!(
            answer(
                Event::Message(ServerMessage::LEFT),
                &mut connected,
                "host:1"
            )
            .is_none()
        );
    }

    #[test]
    fn the_next_step_is_suggested() {
        let none = discovery::Servers::new();
        let mut some = discovery::Servers::new();
        some.apply(discovery::Event::Found(server(
            "Studio",
            tomori_protocol::PROTOCOL_VERSION,
            vec![],
        )));
        assert!(hint(&none, None).unwrap().starts_with("No servers found."));
        assert!(hint(&some, None).unwrap().starts_with("To list the spaces"));
        assert_eq!(hint(&some, Some(&[])), None);
        let space = SpaceSummary {
            id: "s1".into(),
            title: "Studio".into(),
            backend: "fake".into(),
            directory: "/tmp".into(),
            environment: String::new(),
            activity: tomori_protocol::Activity::Idle,
            participants: 0,
            attention: None,
            attention_age: None,
            folder: None,
            active_age: None,
            latest: None,
        };
        assert_eq!(
            hint(&some, Some(std::slice::from_ref(&space))),
            Some("Join one with: tomori-spectator --server ADDRESS --space SPACE_ID")
        );
        report(&some, Some(&[space]));
        report(&none, None);
        report(&some, Some(&[]));
    }
}
