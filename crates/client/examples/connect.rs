// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Connect to a Tomori server and print what it sends.
//!
//! ```text
//! cargo run -p tomori-client --example connect -- 192.168.1.10:8765 [name]
//! cargo run -p tomori-client --example connect -- - [name]
//! cargo run -p tomori-client --example connect
//! ```
//!
//! Without an address, or with `-`, it browses for servers advertised with Bonjour, lists them and asks which one to join.
//!
//! It connects over TLS, accepting the certificate the server presents, or plain HTTP to a server in `--auth open` mode. Without a token it is a visitor, who can only watch; set `TOMORI_TOKEN` to a token from `tomori-server auth mint --owner --name NAME` to create spaces and act in them. See `design/security.md`.
//!
//! Type commands on standard input: `list`, `create <backend> <directory> [title]`, `join <space>`, `leave`,
//! `gate off|live|agent`, `allow <request>`, `deny <request>`, `grant <request> <option>`, `quit`. A pending approval is
//! printed with its request id and the ids of the options it offers.
//!
//! `create` starts a new agent session, such as `create opencode /home/sam/project`, which the server accepts only
//! beneath one of its workspace roots; the directory cannot contain spaces. The title defaults to the directory's last
//! component. The space is bound to the first environment the server offers, which the connected state lists.
use std::collections::BTreeMap;
use std::io::BufRead;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use tomori_client::discovery::{self, Servers};
use tomori_client::{Client, Event, Options, Sender};
use tomori_protocol::{ClientMessage, Decision, Gate, QueueState, ServerMessage};

const COMMANDS: &str = "commands: list, create <backend> <directory> [title], join <space>, leave, gate off|live|agent, allow <request>, deny <request>, grant <request> <option>, quit";

fn decide(request: &str, decision: Decision, option: Option<&str>) -> ClientMessage {
    ClientMessage::Decide {
        request: request.into(),
        decision,
        option: option.map(Into::into),
    }
}

fn command(sender: &Sender, line: &str) -> bool {
    let mut words = line.split_whitespace();
    let message = match (words.next(), words.next()) {
        (Some("quit"), _) => return false,
        (Some("list"), _) => ClientMessage::ListSpaces,
        (Some("create"), Some(backend)) => match words.next() {
            Some(directory) => {
                let title = words.collect::<Vec<_>>().join(" ");
                let title = if title.is_empty() {
                    directory
                        .trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .filter(|name| !name.is_empty())
                        .unwrap_or(directory)
                        .to_owned()
                } else {
                    title
                };
                ClientMessage::CreateSpace {
                    title,
                    backend: backend.into(),
                    directory: directory.into(),
                    session: None,
                    environment: None,
                    instructions: None,
                }
            }
            None => {
                eprintln!("{COMMANDS}");
                return true;
            }
        },
        (Some("join"), Some(space)) => ClientMessage::Join {
            space: space.into(),
        },
        (Some("leave"), _) => ClientMessage::Leave,
        (Some("gate"), Some(gate)) => ClientMessage::Gate {
            gate: match gate {
                "live" => Gate::Live,
                "agent" => Gate::Agent,
                _ => Gate::Off,
            },
        },
        (Some("allow"), Some(request)) => decide(request, Decision::AllowOnce, None),
        (Some("deny"), Some(request)) => decide(request, Decision::Deny, None),
        (Some("grant"), Some(request)) => match words.next() {
            Some(option) => decide(request, Decision::Grant, Some(option)),
            None => {
                eprintln!("{COMMANDS}");
                return true;
            }
        },
        (None, _) => return true,
        _ => {
            eprintln!("{COMMANDS}");
            return true;
        }
    };
    if let Err(error) = sender.send(message) {
        eprintln!("cannot send: {error}");
    }
    true
}

fn describe(message: &ServerMessage) -> String {
    match message {
        ServerMessage::Space { state } => {
            let mut text = format!(
                "space {} ({}) environment={} activity={:?} messages={} queued={} participants={}",
                state.id,
                state.title,
                state.environment,
                state.activity,
                state.messages.len(),
                // Entries not yet delivered, as clients count them; the queue also keeps recently finished ones.
                state
                    .queue
                    .iter()
                    .filter(|entry| matches!(entry.state, QueueState::Queued | QueueState::Sending))
                    .count(),
                state.participants.len()
            );
            if let Some(approval) = &state.approval {
                text.push_str(&format!(
                    "\napproval {} ({}): {}",
                    approval.id, approval.title, approval.text
                ));
                for option in &approval.options {
                    text.push_str(&format!("\n  option {}: {}", option.id, option.label));
                }
            }
            text
        }
        other => format!("{other:?}"),
    }
}

/// How long to browse before listing servers.
const BROWSE_TIME: Duration = Duration::from_secs(3);

/// Browse for servers, list them and read the user's choice; `None` at the end of input.
fn choose() -> std::io::Result<Option<String>> {
    let browser = discovery::browse().map_err(std::io::Error::other)?;
    let mut servers = Servers::new();
    loop {
        eprintln!("Looking for Tomori servers...");
        let deadline = Instant::now() + BROWSE_TIME;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            match browser.recv_timeout(remaining) {
                Ok(event) => {
                    servers.apply(event);
                }
                Err(_) => break,
            }
        }
        let compatible = servers.compatible();
        for (index, server) in compatible.iter().enumerate() {
            let address = server.address().unwrap_or_default();
            eprintln!("  {}. {} ({address})", index + 1, server.name);
        }
        for server in servers.incompatible() {
            let protocol = server
                .protocol
                .map_or_else(|| "unknown".to_owned(), |protocol| protocol.to_string());
            eprintln!(
                "  -  {} (protocol {protocol}; this client speaks {}, so it is unavailable)",
                server.name,
                tomori_protocol::PROTOCOL_VERSION
            );
        }
        if compatible.is_empty() {
            eprintln!(
                "No servers found. Press Enter to look again, or give an address: connect <host[:port]>."
            );
        } else {
            eprint!(
                "Choose a server [1-{}], or press Enter to look again: ",
                compatible.len()
            );
        }
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match line.parse::<usize>() {
            Ok(choice) if (1..=compatible.len()).contains(&choice) => {
                return Ok(compatible[choice - 1].address());
            }
            _ => eprintln!("{line:?} is not in the list."),
        }
    }
}

fn main() -> std::io::Result<()> {
    let mut arguments = std::env::args().skip(1);
    let address = match arguments.next() {
        Some(address) if address != "-" => address,
        _ => match choose()? {
            Some(address) => address,
            None => std::process::exit(1),
        },
    };
    eprintln!("Connecting to {address}.");
    let name = arguments.next().unwrap_or_else(|| "Console".into());
    // A random identity, since the server takes a known one for the participant who used it.
    let participant = {
        use std::hash::{BuildHasher, Hasher};
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u32(std::process::id());
        format!("console-{:016x}", hasher.finish())
    };
    let mut options = Options::new(address, participant, name);
    options.transport = tomori_client::Transport::Automatic { pin: None };
    options.token = std::env::var("TOMORI_TOKEN")
        .ok()
        .filter(|token| !token.is_empty());
    let client = Client::connect(options)?;

    let sender = client.sender();
    let (quit, quitting) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if !command(&sender, &line) {
                break;
            }
        }
        let _ = quit.send(());
    });

    let mut audio: BTreeMap<u32, usize> = BTreeMap::new();
    let mut reported = Instant::now();
    loop {
        match client.events().recv_timeout(Duration::from_millis(100)) {
            Ok(Event::State(state)) => println!("state: {state:?}"),
            Ok(Event::Message(message)) => println!("message: {}", describe(&message)),
            Ok(Event::UtteranceFailed(failed)) => {
                println!("utterance {} failed: {}", failed.utterance, failed.failure)
            }
            // Anything a later client observes that this console does not know.
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        while let Some(frame) = client.try_audio() {
            *audio.entry(frame.source).or_default() += 1;
        }
        if reported.elapsed() >= Duration::from_secs(1) {
            if !audio.is_empty() {
                println!("audio frames per source: {audio:?}");
                audio.clear();
            }
            reported = Instant::now();
        }
        if quitting.try_recv().is_ok() {
            break;
        }
    }
    client.close();
    Ok(())
}
