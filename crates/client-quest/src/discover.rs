// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Find Tomori servers on the network, and the spaces on a chosen one.
//!
//! `tomori discover` browses for servers the way the headset's server list does
//! ([`discovery::browse`]), and, when given a server, lists the spaces on it the
//! way the lobby does (connect and [`tomori_protocol::ClientMessage::ListSpaces`]). The
//! address and space ids it prints are what `--server` and the spectator take.
use std::time::{Duration, Instant};
use tomori_client::{Client, Event, Options, discovery};
use tomori_protocol::{ClientMessage, ServerMessage, SpaceSummary};

/// How long to listen for servers before printing the list.
const BROWSE_FOR: Duration = Duration::from_secs(4);
/// How long to wait for a server to answer with its spaces.
const SPACES_TIMEOUT: Duration = Duration::from_secs(6);

/// Browse for servers, then list the spaces on `server` if one is given.
pub fn run(server: Option<String>) -> anyhow::Result<()> {
    let servers = browse_for(BROWSE_FOR);
    print_servers(&servers);
    match server {
        Some(address) => list_spaces(&address)?,
        None => {
            if servers.compatible().is_empty() {
                eprintln!(
                    "No servers found. Start a server, or pass one: tomori discover --server ADDRESS"
                );
            } else {
                eprintln!(
                    "To list the spaces on one of these, pass it: tomori discover --server ADDRESS"
                );
            }
        }
    }
    Ok(())
}

/// Browse for servers, folding the events into a list for `duration`.
fn browse_for(duration: Duration) -> discovery::Servers {
    let mut servers = discovery::Servers::new();
    let browser = match discovery::browse() {
        Ok(browser) => browser,
        Err(error) => {
            eprintln!("Cannot browse for servers: {error}");
            return servers;
        }
    };
    let deadline = Instant::now() + duration;
    loop {
        let timeout = deadline.saturating_duration_since(Instant::now());
        if timeout == Duration::ZERO {
            break;
        }
        match browser.recv_timeout(timeout) {
            Ok(event) => servers.apply(event),
            Err(_) => break, // The deadline passed.
        };
    }
    servers
}

fn print_servers(servers: &discovery::Servers) {
    if servers.compatible().is_empty() && servers.incompatible().is_empty() {
        println!("No servers found on the network.");
        return;
    }
    println!("Servers:");
    for server in servers.compatible() {
        let address = server
            .address()
            .unwrap_or_else(|| format!("{}:{}", server.host, server.port));
        println!("  {name:<24} {address}", name = server.name);
    }
    for server in servers.incompatible() {
        println!(
            "  {name:<24} (protocol {:?}, this build is {})",
            server.protocol,
            tomori_protocol::PROTOCOL_VERSION,
            name = server.name
        );
    }
}

/// Connect to `address` and print its spaces, with the id to join by.
fn list_spaces(address: &str) -> anyhow::Result<()> {
    let mut options = Options::new(address, "discover", "discover");
    // Listing needs no token: it connects as a visitor, over TLS to a server that admits by token, or plain HTTP to one in `--auth open` mode.
    options.transport = tomori_client::Transport::Automatic { pin: None };
    let client = Client::connect(options)?;
    // The connection thread sends Hello and reconnects; only the space list is wanted.
    if let Err(error) = client.send(ClientMessage::ListSpaces) {
        client.close();
        anyhow::bail!("Cannot send to {address}: {error}");
    }
    let deadline = Instant::now() + SPACES_TIMEOUT;
    let mut connected = false;
    loop {
        // Drain every event that is waiting, which stops the search.
        while let Some(event) = client.try_event() {
            match event {
                Event::State(state) => connected = state.is_connected(),
                Event::Message(ServerMessage::Spaces { spaces }) => {
                    print_spaces(&spaces);
                    client.close();
                    return Ok(());
                }
                Event::Message(ServerMessage::Error { error, .. }) => {
                    client.close();
                    anyhow::bail!("The server at {address} refused: {error}");
                }
                _ => {}
            }
        }
        if Instant::now() >= deadline {
            client.close();
            if connected {
                anyhow::bail!("The server at {address} sent no space list");
            }
            anyhow::bail!("The server at {address} did not answer in time");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn print_spaces(spaces: &[SpaceSummary]) {
    if spaces.is_empty() {
        println!("The server has no spaces yet.");
        return;
    }
    println!("Spaces:");
    for space in spaces {
        println!(
            "  {:<12} {:<24} {}{} participant(s)",
            space.id, space.title, space.backend, space.participants
        );
    }
    eprintln!("Join one with: tomori --spectator --server ADDRESS --space SPACE_ID");
}
