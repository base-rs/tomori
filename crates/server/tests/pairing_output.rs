// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What the server writes about pairing where others on its machine may read it, its log and its console: that a pairing began, waits, was confirmed, declined, withdrawn or paused, but never its code, in any form, nor the values it is derived from. See `design/security.md`.
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tomori_protocol::{ClientMessage, ServerMessage};
use tomori_security::Fingerprint;
use tomori_security::pairing::{ChosenName, Nonce, code, display};
use tomori_server::test_support::{TestClient, TestServer, eventually};
use tomori_server::{PairingEvent, PairingView};

/// Every log record of this test process, at every level, with its level and target.
struct Recorder(Mutex<Vec<String>>);

impl log::Log for Recorder {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        self.0.lock().unwrap().push(format!(
            "{} {}: {}",
            record.level(),
            record.target(),
            record.args()
        ));
    }

    fn flush(&self) {}
}

static LOG: Recorder = Recorder(Mutex::new(Vec::new()));

/// Everything logged so far, recording at every level from the first call on.
fn logged() -> Vec<String> {
    static RECORDING: std::sync::Once = std::sync::Once::new();
    RECORDING.call_once(|| {
        log::set_logger(&LOG).expect("only this recorder logs here");
        log::set_max_level(log::LevelFilter::Trace);
    });
    log::Log::flush(&LOG);
    assert!(log::Log::enabled(&LOG, &log::Metadata::builder().build()));
    LOG.0.lock().unwrap().clone()
}

fn fingerprint() -> Fingerprint {
    Fingerprint::of(b"test certificate")
}

/// A server that pairs, configured by `configure`, with its pairing handle and what its console is told.
fn pairing_server(
    configure: impl FnOnce(tomori_server::Server) -> tomori_server::Server + Send + 'static,
) -> (
    TestServer,
    tomori_server::Pairing,
    Arc<Mutex<Vec<PairingEvent>>>,
) {
    logged();
    let (sender, receiver) = channel();
    let console = Arc::new(Mutex::new(Vec::new()));
    let told = console.clone();
    let server = TestServer::with_tokens(true, move |server| {
        let server = configure(
            server
                .with_pairing(fingerprint())
                .with_tokens_over_plain_http_for_tests()
                .with_pairing_prompt(move |event| told.lock().unwrap().push(event.clone())),
        );
        sender.send(server.pairing()).unwrap();
        server
    });
    (server, receiver.recv().unwrap(), console)
}

/// What a pairing's device knows: its code and both sides' values.
struct Secrets {
    code: String,
    device: String,
    server: String,
}

/// Begin and reveal, as a device does, returning what it knows.
fn pair(client: &mut TestClient) -> Secrets {
    let device = Nonce::generate();
    client.send(&ClientMessage::PairBegin {
        commitment: device.commitment(),
    });
    let server = client.wait_for(|message| match message {
        ServerMessage::PairNonce { nonce } => Nonce::parse(nonce),
        _ => None,
    });
    client.send(&ClientMessage::PairReveal {
        nonce: device.to_hex(),
    });
    client.wait_for(|message| matches!(message, ServerMessage::PairWaiting).then_some(()));
    Secrets {
        code: code(fingerprint(), device, server),
        device: device.to_hex(),
        server: server.to_hex(),
    }
}

fn failed(client: &TestClient) -> String {
    client.wait_for(|message| match message {
        ServerMessage::PairFailed { error } => Some(error.clone()),
        _ => None,
    })
}

/// Everything the console was told, as it shows each event and as each is debugged.
fn shown(console: &Mutex<Vec<PairingEvent>>) -> Vec<String> {
    console
        .lock()
        .unwrap()
        .iter()
        .map(|event| format!("{event}\n{event:?}"))
        .collect()
}

/// Assert that no line of `lines` holds any of `secrets`: a code's digits in order, together or with spaces or dashes between them, or either side's value.
fn assert_never_written(lines: &[String], secrets: &[Secrets]) {
    assert!(!lines.is_empty());
    for line in lines {
        let digits: String = line
            .chars()
            .filter(|character| !character.is_whitespace() && !matches!(character, '-' | '_'))
            .collect();
        for secrets in secrets {
            assert!(!digits.contains(&secrets.code), "{line}");
            assert!(!line.contains(&display(&secrets.code)), "{line}");
            assert!(!line.contains(&secrets.device), "{line}");
            assert!(!line.contains(&secrets.server), "{line}");
        }
    }
}

/// Another six digits than `code`.
fn wrong(code: &str) -> &'static str {
    if code == "000000" { "000001" } else { "000000" }
}

#[test]
fn pairing_is_logged_and_told_on_the_console_without_its_code() {
    let (server, pairing, console) = pairing_server(|server| server);
    let mut quest = server.connect("quest", "Quest 3");
    let mut secrets = Vec::new();

    // It begins and waits, and the console says how to confirm it, without the code.
    secrets.push(pair(&mut quest));
    let waiting = eventually(|| {
        console
            .lock()
            .unwrap()
            .iter()
            .find(|event| matches!(event, PairingEvent::Waiting { .. }))
            .map(ToString::to_string)
    });
    assert!(
        waiting.starts_with("Pairing request from a device calling itself \"Quest\", begun at "),
        "{waiting}"
    );
    assert!(
        waiting.contains("type here the 6-digit code shown on the device you are pairing"),
        "{waiting}"
    );
    // Only the status page, for owners, shows it; debugged, the pending pairing does not.
    let pending = pairing.pending().unwrap();
    assert_eq!(pending.code, Some(display(&secrets[0].code)));
    let debugged = format!("{pending:?}");
    assert_eq!(
        debugged,
        format!(
            "PairingView {{ serial: {}, device: ChosenName(Some(\"Quest\")), code: Some([redacted]) }}",
            pending.serial
        )
    );

    // A code that does not match declines it.
    assert_eq!(
        pairing.confirm(wrong(&secrets[0].code)),
        Err("That code does not match, so the pairing was declined.")
    );
    failed(&quest);

    // Confirmed with the code the device shows.
    secrets.push(pair(&mut quest));
    assert_eq!(
        pairing.confirm(&display(&secrets[1].code)),
        Ok(ChosenName::new("Quest 3"))
    );
    quest.wait_for(|message| matches!(message, ServerMessage::Paired { .. }).then_some(()));

    // Declined.
    let mut tablet = server.connect("tablet", "Tablet");
    secrets.push(pair(&mut tablet));
    assert!(pairing.decline());
    failed(&tablet);

    // Paused, once too many began.
    tablet.send(&ClientMessage::PairBegin {
        commitment: Nonce::generate().commitment(),
    });
    failed(&tablet);
    eventually(|| {
        console
            .lock()
            .unwrap()
            .contains(&PairingEvent::Paused)
            .then_some(())
    });

    // Withdrawn as its device leaves.
    pairing.allow();
    secrets.push(pair(&mut tablet));
    tablet.finish();
    eventually(|| pairing.pending().is_none().then_some(()));

    let log = logged();
    for expected in [
        "INFO tomori_server::hub: Pairing began from a device calling itself \"Quest\".",
        "INFO tomori_server::hub: Pairing request from a device calling itself \"Quest\": it waits to be confirmed with the code its device shows.",
        "INFO tomori_server::hub: Declined the pairing of a device calling itself \"Quest\", since the code typed did not match the one it shows.",
        "INFO tomori_server::hub: Declined the pairing of a device calling itself \"Tablet\".",
        "INFO tomori_server::hub: A pairing was withdrawn because its device disconnected.",
    ] {
        assert!(log.iter().any(|line| line == expected), "{expected}");
    }
    assert!(
        log.iter()
            .any(|line| line.contains("Paired a device calling itself \"Quest\" as owner token")),
        "{log:?}"
    );
    let mut written = log;
    written.extend(shown(&console));
    written.push(debugged);
    assert_never_written(&written, &secrets);
}

#[test]
fn an_expired_pairing_is_logged_without_its_code() {
    let (server, _pairing, console) =
        pairing_server(|server| server.with_pairing_timeout(Duration::from_secs(1)));
    let mut quest = server.connect("quest", "Quest 3");
    let secrets = pair(&mut quest);
    assert_eq!(failed(&quest), "The pairing was not confirmed in time.");
    let mut written = eventually(|| {
        let log = logged();
        log.iter()
            .any(|line| {
                line.ends_with("A pairing was withdrawn: The pairing was not confirmed in time.")
            })
            .then_some(log)
    });
    written.extend(shown(&console));
    assert_never_written(&written, &[secrets]);
}

#[test]
fn a_pairing_without_its_code_yet_is_debugged_as_such() {
    let view = PairingView {
        serial: 7,
        device: ChosenName::new("Quest"),
        code: None,
    };
    assert_eq!(
        format!("{view:?}"),
        "PairingView { serial: 7, device: ChosenName(Some(\"Quest\")), code: None }"
    );
}

/// The status page, as the owner with `token` sees it.
fn status_page(server: &TestServer, token: &str) -> String {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(server.address).unwrap();
    write!(
        stream,
        "GET / HTTP/1.1\r\nHost: tomori\r\nConnection: close\r\nCookie: __Host-tomori-token={token}\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

/// What is said of the device in `line`, from where the console, the log or the status page names it to the end of its quoted name, if it names one.
fn device_named(line: &str) -> Option<&str> {
    let (_, named) = line.split_once("a device calling itself \"")?;
    Some(named.split_once('"').map_or(named, |(name, _)| name))
}

#[test]
fn a_device_s_chosen_name_is_shown_without_its_digits_or_number_words() {
    let (server, pairing, console) = pairing_server(|server| server);
    let (_, owner) = server.issue("Ada", tomori_security::Grant::Owner);
    for (hostile, kept) in [
        ("Type 482913 to confirm", Some("Type to confirm")),
        ("Quest — code four eight two", Some("Quest code")),
        ("Code ４８２ ٤٨٢ ⑨ NINE", Some("Code")),
        ("f\u{200b}our\u{202e} 8 Quest", Some("Quest")),
        ("482 913", None),
    ] {
        let mut device = server.connect("device", hostile);
        let secrets = pair(&mut device);
        // The status page names it as the console does, as a device calling itself what is left of the name, or as a device.
        let page = status_page(&server, &owner);
        let view = pairing.pending().unwrap();
        assert_eq!(view.device.as_str(), kept, "{hostile}");
        let expected = match kept {
            Some(kept) => format!(
                "Pairing request from a device calling itself &quot;{kept}&quot;, with code"
            ),
            None => "Pairing request from a device, with code".to_owned(),
        };
        assert!(page.contains(&expected), "{page}");
        assert!(pairing.decline());
        failed(&device);
        pairing.allow();
        let mut lines = logged();
        lines.extend(shown(&console));
        assert_never_written(&lines, &[secrets]);
        // Only the status page, for owners, shows the code.
        lines.push(page.replace("&quot;", "\""));
        for line in &lines {
            if let Some(named) = device_named(line) {
                assert!(!named.chars().any(char::is_numeric), "{line}");
                for word in ["four", "eight", "two", "nine", "NINE"] {
                    assert!(!named.contains(word), "{line}");
                }
            }
        }
    }
    // The console says each, marked as the device's own name, and how to confirm.
    let console: Vec<String> = console
        .lock()
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(
        console.contains(
            &"Pairing began from a device calling itself \"Type to confirm\".".to_owned()
        ),
        "{console:?}"
    );
    assert!(
        console.contains(&"Pairing began from a device.".to_owned()),
        "{console:?}"
    );
    assert!(
        console.iter().any(
            |line| line.starts_with("Pairing request from a device, begun at ")
                && line.contains("type here the 6-digit code shown on the device you are pairing")
        ),
        "{console:?}"
    );
}
