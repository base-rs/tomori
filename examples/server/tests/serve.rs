// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The `tomori-example-server` executable, run as `tomori-server` would be, on an ephemeral loopback port without advertising.
use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tomori_protocol::{ClientMessage, PROTOCOL_VERSION, ServerMessage};
use tomori_server::test_support::TestClient;

const SERVER: &str = env!("CARGO_BIN_EXE_tomori-example-server");

/// An empty directory of its own for the test named `name`.
fn directory(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "tomori-example-server-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
fn the_help_lists_the_placeholder_backend_beside_tomoris() {
    let output = Command::new(SERVER).arg("--help").output().unwrap();
    assert!(!output.status.success());
    let usage = String::from_utf8(output.stderr).unwrap();
    assert!(usage.starts_with("Usage: tomori-example-server"), "{usage}");
    assert!(
        usage.contains("Available: fake, opencode, placeholder."),
        "{usage}"
    );
    // It lists the environments the example registers, in the order it offers them.
    assert!(
        usage.ends_with(
            "  sky                  Sky, version 1.\n  minimal              Grid, version 1.\n  placeholder          Placeholder, version 1.\n"
        ),
        "{usage}"
    );
    assert!(!usage.contains("tomori-server"), "{usage}");
    let output = Command::new(SERVER).arg("auth").output().unwrap();
    let usage = String::from_utf8(output.stderr).unwrap();
    assert!(
        usage.starts_with("Usage: tomori-example-server auth mint"),
        "{usage}"
    );
}

#[test]
fn a_server_without_an_owner_token_says_how_to_mint_one_with_its_own_name() {
    let directory = directory("tokens");
    let state = directory.to_str().unwrap();
    let mut server = Command::new(SERVER)
        .args([
            "--bind",
            "127.0.0.1:0",
            "--no-advertise",
            "--no-mcp",
            "--state-directory",
            state,
            "--workspace-root",
            state,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Read on a thread of its own, so that a server that never says it fails the test at the deadline rather than hanging it.
    let stderr = server.stderr.take().unwrap();
    let (lines, received) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let said = loop {
        match received.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(line) if line.starts_with("No owner token is issued yet") => break Some(line),
            Ok(_) => {}
            Err(_) => break None,
        }
    };
    server.kill().unwrap();
    server.wait().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert_eq!(
        said.as_deref(),
        Some(
            "No owner token is issued yet, and pairing is closed. To pair this server's first device, type \"pair\" here, which opens pairing for 2 minutes, or start the server with --open-pairing; then choose the pairing object on the device and type the code it shows. Or run `tomori-example-server auth mint --owner --name NAME`."
        )
    );
}

#[test]
fn a_space_is_created_with_the_placeholder_backend_in_the_placeholder_environment() {
    let directory = directory("serve");
    let state = directory.to_str().unwrap();
    let mut server = Command::new(SERVER)
        .args([
            "--bind",
            "127.0.0.1:0",
            "--no-advertise",
            "--auth",
            "open",
            "--backend",
            "placeholder",
            "--state-directory",
            state,
            "--workspace-root",
            state,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(server.stderr.take().unwrap()).lines();
    let listening = lines
        .by_ref()
        .map(Result::unwrap)
        .find(|line| line.starts_with("Tomori server listening on http://127.0.0.1:"))
        .expect("the server says where it listens");
    assert!(
        listening.ends_with("(backends: placeholder)."),
        "{listening}"
    );
    let address: SocketAddr = listening
        .trim_start_matches("Tomori server listening on http://")
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();

    // The server offers the placeholder backend, and the placeholder environment after Tomori's own.
    let mut client = TestClient::open(address);
    client.send(&ClientMessage::Hello {
        protocol: PROTOCOL_VERSION,
        participant: "quest".into(),
        name: "Ada".into(),
        names: false,
        environments: None,
        environment_state: false,
        projection: false,
        pairing_window: false,
    });
    let (backends, environments) = client.wait_for(|message| match message {
        ServerMessage::Welcome {
            backends,
            environments,
            ..
        } => Some((backends.clone(), environments.clone())),
        _ => None,
    });
    let backends: Vec<&str> = backends
        .iter()
        .map(|backend| backend.name.as_str())
        .collect();
    assert_eq!(backends, ["placeholder"]);
    let environments: Vec<(&str, &str)> = environments
        .iter()
        .map(|environment| (environment.id.as_str(), environment.label.as_str()))
        .collect();
    assert_eq!(
        environments,
        [
            ("sky", "Sky"),
            ("minimal", "Grid"),
            ("placeholder", "Placeholder")
        ]
    );

    // A space bound to it gives the agent the environment's layer of its instructions.
    client.send(&ClientMessage::CreateSpace {
        title: "Placeholder".into(),
        backend: "placeholder".into(),
        directory: state.into(),
        session: None,
        environment: Some(tomori_example_server::ENVIRONMENT.into()),
        instructions: None,
    });
    let space = client.space(|space| space.title == "Placeholder");
    assert_eq!(space.environment, tomori_example_server::ENVIRONMENT);
    assert_eq!(
        space.instructions.unwrap().environment,
        tomori_example_server::INSTRUCTIONS
    );
    drop(client);

    server.kill().unwrap();
    server.wait().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
