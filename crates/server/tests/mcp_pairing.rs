// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A space's agent listing, confirming and declining a device pairing through Tomori's tools. See `design/security.md` and `design/mcp.md`.
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tomori_protocol::{ClientMessage, ServerMessage};
use tomori_security::Fingerprint;
use tomori_security::pairing::{Nonce, code, display};
use tomori_server::test_support::{TestClient, TestServer, eventually};
use tomori_server::{PAIRING_ATTEMPTS, PairingEvent};

/// Every log record of this test process.
struct Recorder(Mutex<Vec<String>>);

impl log::Log for Recorder {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        self.0.lock().unwrap().push(record.args().to_string());
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

/// The certificate fingerprint the code is derived from; the test clients speak plain HTTP.
fn fingerprint() -> Fingerprint {
    Fingerprint::of(b"test certificate")
}

/// A server that pairs and serves Tomori's tools, with Ada, an owner, in her space "Studio", whose agent session is `session`.
struct Studio {
    server: TestServer,
    pairing: tomori_server::Pairing,
    /// What the console was told.
    prompts: Arc<Mutex<Vec<PairingEvent>>>,
    /// The MCP endpoint's bearer token.
    token: String,
    space: String,
    session: String,
    ada: TestClient,
    /// Ada's owner token, as the status page's cookie.
    cookie: String,
}

impl Studio {
    fn new() -> Self {
        logged();
        let (sender, receiver) = channel();
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let seen = prompts.clone();
        let server = TestServer::with_tokens_and_tools(true, move |server| {
            let server = server
                .with_pairing(fingerprint())
                .with_tokens_over_plain_http_for_tests()
                .with_pairing_prompt(move |event| seen.lock().unwrap().push(event.clone()));
            sender.send(server.pairing()).unwrap();
            server
        });
        let pairing = receiver.recv().unwrap();
        let (_, owner) = server.issue("Ada", tomori_security::Grant::Owner);
        let mut ada = TestClient::open_with(
            server.address,
            &[("Authorization", &format!("Bearer {owner}"))],
        );
        ada.hello_projected("ada", "Ada");
        let space = ada.create_space("Studio").id;
        let token = eventually(|| server.tools_token());
        let session = tomori_backend::Session::id(&server.backend.started()[0]);
        Self {
            server,
            pairing,
            prompts,
            token,
            space,
            session,
            ada,
            cookie: format!("__Host-tomori-token={owner}"),
        }
    }

    /// Call the tool `name` with `arguments` as the agent of the session `session` names, or of the studio's session.
    fn call_as(&self, session: Option<&str>, name: &str, arguments: Value) -> Value {
        let mut params = json!({ "name": name, "arguments": arguments });
        if let Some(session) = session {
            params["_meta"] = json!({ "ai.opencode/sessionID": session });
        }
        call(&self.server, &self.token, &params)
    }

    fn call(&self, name: &str, arguments: Value) -> Value {
        self.call_as(Some(&self.session.clone()), name, arguments)
    }

    /// The body of the newest panel on the space's walls that says `text`, as Ada is sent it.
    fn notice(&self, text: &str) -> String {
        self.ada.wait_for(|message| match message {
            ServerMessage::Projection { projection } => projection
                .panels
                .iter()
                .find(|panel| panel.body.contains(text))
                .map(|panel| {
                    assert_eq!(panel.title, "Pairing");
                    assert_eq!(panel.meta[0].value, "Tomori");
                    panel.body.clone()
                }),
            _ => None,
        })
    }

    /// The line the operator and owners are told, which names the space and the session, once it is logged.
    fn told(&self, did: &str) -> String {
        let line = format!(
            "The agent of space {} (\"Studio\"), session {}, {did}.",
            self.space, self.session
        );
        // The console is told after the log, so once it is, both are; the console is this server's alone.
        let notice = PairingEvent::Agent {
            notice: line.clone(),
        };
        eventually(|| self.prompts.lock().unwrap().contains(&notice).then_some(()));
        assert!(logged().contains(&line), "{line}");
        line
    }

    /// The status page, as Ada sees it.
    fn status_page(&self) -> String {
        let mut stream = TcpStream::connect(self.server.address).unwrap();
        write!(
            stream,
            "GET / HTTP/1.1\r\nHost: tomori\r\nConnection: close\r\nCookie: {}\r\n\r\n",
            self.cookie
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }
}

/// Call a tool with `params` at `server`'s MCP endpoint, with `token`, and return its result.
fn call(server: &TestServer, token: &str, params: &Value) -> Value {
    let body =
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params }).to_string();
    let mut stream = TcpStream::connect(server.tools.unwrap()).unwrap();
    write!(
        stream,
        "POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (_, body) = response.split_once("\r\n\r\n").unwrap();
    serde_json::from_str::<Value>(body).unwrap()["result"].clone()
}

/// The text of a tool's failure.
fn failure(result: &Value) -> String {
    assert_eq!(result["isError"], true, "{result}");
    result["content"][0]["text"].as_str().unwrap().to_owned()
}

/// Begin and reveal, as a device does, returning the code it derives and shows.
fn pair(client: &mut TestClient) -> String {
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
    code(fingerprint(), device, server)
}

fn failed(client: &TestClient) -> String {
    client.wait_for(|message| match message {
        ServerMessage::PairFailed { error } => Some(error.clone()),
        _ => None,
    })
}

/// Another six digits than `code`.
fn wrong(code: &str) -> &'static str {
    if code == "000000" { "000001" } else { "000000" }
}

#[test]
fn the_agent_lists_the_pairing_waiting_but_never_its_code() {
    let studio = Studio::new();
    let listed = studio.call("pairing", json!({}));
    assert_eq!(
        listed["structuredContent"],
        json!({"pending": null, "paused": false})
    );
    // With nothing waiting, there is nothing to confirm or decline.
    assert!(
        failure(&studio.call("confirm_pairing", json!({"code": "123 456"})))
            .starts_with("No pairing is waiting.")
    );
    assert!(
        failure(&studio.call("decline_pairing", json!({}))).starts_with("No pairing is waiting.")
    );

    // Before the device reveals its value, it shows no code, and is withdrawn sooner.
    let mut quest = studio.server.connect("quest", "Quest 3");
    let device = Nonce::generate();
    quest.send(&ClientMessage::PairBegin {
        commitment: device.commitment(),
    });
    let nonce = quest.wait_for(|message| match message {
        ServerMessage::PairNonce { nonce } => Some(nonce.clone()),
        _ => None,
    });
    let unrevealed = studio.call("pairing", json!({}))["structuredContent"].clone();
    assert_eq!(unrevealed["pending"]["device"], "Quest 3");
    assert_eq!(unrevealed["pending"]["code_shown"], false);
    assert!(
        unrevealed["pending"]["expires_in_seconds"]
            .as_u64()
            .unwrap()
            <= 10
    );
    // Nothing can be confirmed before the device shows its code, and trying does not end the pairing.
    assert!(
        failure(&studio.call("confirm_pairing", json!({"code": "123 456"})))
            .starts_with("The device has not shown its code yet.")
    );
    assert!(studio.pairing.pending().is_some());

    quest.send(&ClientMessage::PairReveal {
        nonce: device.to_hex(),
    });
    quest.wait_for(|message| matches!(message, ServerMessage::PairWaiting).then_some(()));
    let shown = code(fingerprint(), device, Nonce::parse(&nonce).unwrap());
    let listed = studio.call("pairing", json!({}));
    let pending = &listed["structuredContent"]["pending"];
    assert_eq!(pending["device"], "Quest 3");
    assert_eq!(pending["code_shown"], true);
    assert!(pending["began_seconds_ago"].as_u64().unwrap() < 10);
    let expires = pending["expires_in_seconds"].as_u64().unwrap();
    assert!((100..=120).contains(&expires), "{expires}");
    assert_eq!(
        pending.as_object().unwrap().keys().collect::<Vec<_>>(),
        [
            "began_seconds_ago",
            "code_shown",
            "device",
            "expires_in_seconds"
        ]
    );
    // Neither the code, in any form, nor the server's value it is derived from, is in what the agent reads.
    let text = listed.to_string();
    for secret in [shown.clone(), display(&shown), nonce] {
        assert!(!text.contains(&secret), "{text}");
    }
    // Listing tells nobody, since it changes nothing.
    let told = studio.prompts.lock().unwrap().len();
    studio.call("pairing", json!({}));
    assert_eq!(studio.prompts.lock().unwrap().len(), told);
}

#[test]
fn the_agent_confirms_a_pairing_with_the_code_read_from_the_device() {
    let studio = Studio::new();
    let mut quest = studio.server.connect("quest", "Quest 3");
    let shown = pair(&mut quest);
    // Said as the device shows it, in two groups.
    let confirmed = studio.call("confirm_pairing", json!({"code": display(&shown)}));
    assert_eq!(
        confirmed["structuredContent"],
        json!({
            "device": "Quest 3",
            "confirmed": true,
            "next": "Tell everyone in the space that \"Quest 3\" is paired, as an owner.",
        })
    );
    let token = quest.wait_for(|message| match message {
        ServerMessage::Paired { token } => Some(token.clone()),
        _ => None,
    });
    assert_eq!(studio.pairing.pending(), None);
    let entries = studio.server.tokens.as_ref().unwrap().entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].name, "Quest 3");
    assert_eq!(entries[1].grant, tomori_security::Grant::Owner);
    let quest = studio.server.connect_with_token("quest", "Quest 3", &token);
    assert_eq!(quest.role, tomori_protocol::ConnectionRole::Owner);

    // Everyone in the space sees it on the Tomori wall, the operator on the console and in the log, with the space and session, and owners on the status page.
    let did = "confirmed pairing for \"Quest 3\" while Ada was in the space; it is now an owner";
    assert_eq!(studio.notice(did), format!("The agent {did}."));
    let line = studio.told(did);
    let page = studio.status_page();
    assert!(page.contains("<h3>By agents</h3>"), "{page}");
    let escaped = line.replace('"', "&quot;");
    assert!(page.contains(&format!("<li>{escaped}</li>")), "{page}");
    // Neither the code nor the token is logged with it, or shown on the console.
    let console: Vec<String> = studio
        .prompts
        .lock()
        .unwrap()
        .iter()
        .map(|event| format!("{event} {event:?}"))
        .collect();
    for line in logged().iter().chain(&console) {
        for secret in [&token, &shown, &display(&shown)] {
            assert!(!line.contains(secret.as_str()), "{line}");
        }
    }
}

#[test]
fn a_wrong_code_declines_the_pairing_and_one_that_is_not_a_code_changes_nothing() {
    let studio = Studio::new();
    let mut quest = studio.server.connect("quest", "Quest 3");
    let shown = pair(&mut quest);

    // What is not six digits is refused, leaving the pairing waiting, so that a mishearing costs nothing.
    for code in ["12345", "123 4567", "one two three", ""] {
        let refused = failure(&studio.call("confirm_pairing", json!({ "code": code })));
        assert!(
            refused.starts_with("A pairing code is the six digits the device shows"),
            "{refused}"
        );
        assert!(refused.ends_with("tell everyone there too."), "{refused}");
    }
    assert_eq!(
        failure(&studio.call("confirm_pairing", json!({"code": "1".repeat(33)}))),
        "A pairing code is the six digits the device shows."
    );
    assert_eq!(
        failure(&studio.call("confirm_pairing", json!({}))),
        "This tool needs code, as text."
    );
    assert!(studio.pairing.pending().is_some());
    studio.notice("asked to confirm a pairing and was refused: A pairing code is the six digits");

    // Six digits that are not the device's code decline the pairing at once: a code from anywhere but the device gets one try.
    let refused = failure(&studio.call("confirm_pairing", json!({"code": wrong(&shown)})));
    assert_eq!(
        refused,
        "That code does not match, so the pairing was declined. Tomori has said so on the space's wall; tell everyone there too."
    );
    assert_eq!(
        failed(&quest),
        "The code given did not match, so the pairing was declined."
    );
    assert_eq!(studio.pairing.pending(), None);
    assert_eq!(studio.server.tokens.as_ref().unwrap().entries().len(), 1);
    let did =
        "gave a code that did not match the one \"Quest 3\" shows, so its pairing was declined";
    studio.notice(did);
    studio.told(did);
    // The right code is no use once it is declined.
    assert!(
        failure(&studio.call("confirm_pairing", json!({ "code": shown })))
            .starts_with("No pairing is waiting.")
    );
}

#[test]
fn the_agent_declines_a_pairing() {
    let studio = Studio::new();
    let mut quest = studio.server.connect("quest", "Quest 3");
    pair(&mut quest);
    let declined = studio.call("decline_pairing", json!({}));
    assert_eq!(
        declined["structuredContent"],
        json!({
            "device": "Quest 3",
            "confirmed": false,
            "next": "Tell everyone in the space that you declined pairing for \"Quest 3\".",
        })
    );
    assert_eq!(failed(&quest), "The pairing was declined.");
    assert_eq!(studio.pairing.pending(), None);
    assert_eq!(studio.server.tokens.as_ref().unwrap().entries().len(), 1);
    let did = "declined pairing for \"Quest 3\" while Ada was in the space";
    studio.notice(did);
    studio.told(did);
}

#[test]
fn only_a_session_bound_to_a_space_may_decide_though_nobody_need_be_in_it() {
    let mut studio = Studio::new();
    let mut quest = studio.server.connect("quest", "Quest 3");
    let shown = pair(&mut quest);
    // A call naming no session, or a session bound to no space, such as a sub-agent's, does nothing, and the operator is told.
    for tool in ["pairing", "confirm_pairing", "decline_pairing"] {
        let arguments = json!({ "code": display(&shown) });
        assert!(
            failure(&studio.call_as(None, tool, arguments.clone()))
                .ends_with("this call names no session."),
            "{tool}"
        );
        assert!(
            failure(&studio.call_as(Some("ses_other"), tool, arguments))
                .starts_with("This session is not bound to a Tomori space"),
            "{tool}"
        );
    }
    assert!(studio.pairing.pending().is_some());
    for asked in ["confirm", "decline"] {
        let line = format!(
            "Agent session ses_other, bound to no open space, asked to {asked} a pairing and was refused."
        );
        eventually(|| logged().contains(&line).then_some(()));
    }

    // Bob, another owner, and then nobody: the agent may still decide, as for a maintainer whose only headset lost its pairing, and the notice says who was there.
    let (_, bob) = studio.server.issue("Bob", tomori_security::Grant::Owner);
    let mut bob = studio.server.connect_with_token("bob", "Bob", &bob);
    bob.send(&ClientMessage::Join {
        space: studio.space.clone(),
    });
    bob.space(|state| state.participants.len() == 2);
    studio.call("decline_pairing", json!({}));
    studio.notice("declined pairing for \"Quest 3\" while Ada and Bob were in the space");
    failed(&quest);
    for client in [&mut studio.ada, &mut bob] {
        client.send(&ClientMessage::Leave);
        client.wait_for(|message| matches!(message, ServerMessage::Left { .. }).then_some(()));
    }
    let shown = pair(&mut quest);
    let confirmed = studio.call("confirm_pairing", json!({ "code": shown }));
    assert_eq!(
        confirmed["structuredContent"]["confirmed"], true,
        "{confirmed}"
    );
    studio.told("confirmed pairing for \"Quest 3\" with nobody in the space; it is now an owner");
}

#[test]
fn a_deleted_space_s_agent_can_no_longer_decide_and_another_space_s_decides_the_one_pairing() {
    let mut studio = Studio::new();
    let workshop = studio.ada.create_space("Workshop").id;
    let other = tomori_backend::Session::id(&studio.server.backend.started()[1]);
    let mut quest = studio.server.connect("quest", "Quest 3");
    let shown = pair(&mut quest);
    // The studio is deleted while the pairing waits: its session decides nothing, and the pairing is left waiting.
    studio.ada.send(&ClientMessage::Join {
        space: studio.space.clone(),
    });
    studio.ada.space(|state| state.id == studio.space);
    studio.ada.send(&ClientMessage::DeleteSpace {
        space: studio.space.clone(),
        session: false,
    });
    studio.ada.wait_for(|message| {
        matches!(message, ServerMessage::Left { deleted: Some(_) }).then_some(())
    });
    for tool in ["pairing", "confirm_pairing", "decline_pairing"] {
        let result = studio.call(tool, json!({ "code": display(&shown) }));
        assert_eq!(result["isError"], true, "{tool}: {result}");
    }
    assert!(studio.pairing.pending().is_some());
    // The workshop's agent confirms the server's one pairing, and then there is none for either space to decide.
    let confirmed = studio.call_as(Some(&other), "confirm_pairing", json!({ "code": shown }));
    assert_eq!(
        confirmed["structuredContent"]["confirmed"], true,
        "{confirmed}"
    );
    assert!(
        failure(&studio.call_as(Some(&other), "confirm_pairing", json!({ "code": shown })))
            .starts_with("No pairing is waiting.")
    );
    let line = format!(
        "The agent of space {workshop} (\"Workshop\"), session {other}, confirmed pairing for \"Quest 3\" with nobody in the space; it is now an owner."
    );
    eventually(|| logged().contains(&line).then_some(()));
}

#[test]
fn the_rate_limit_holds_and_the_agent_cannot_lift_it() {
    let studio = Studio::new();
    let mut quest = studio.server.connect("quest", "Quest 3");
    for _ in 0..PAIRING_ATTEMPTS {
        let shown = pair(&mut quest);
        studio.call("confirm_pairing", json!({"code": wrong(&shown)}));
        failed(&quest);
    }
    quest.send(&ClientMessage::PairBegin {
        commitment: Nonce::generate().commitment(),
    });
    assert_eq!(
        failed(&quest),
        "Too many pairings began recently; try again in a few minutes."
    );
    assert_eq!(
        studio.call("pairing", json!({}))["structuredContent"],
        json!({"pending": null, "paused": true})
    );
    // There is no tool to allow pairing again: only the operator or an owner on the status page can.
    assert!(studio.call("allow_pairing", json!({})).is_null());
    assert!(studio.pairing.paused());
}

#[test]
fn an_open_server_does_not_pair_and_the_tools_say_so() {
    logged();
    let backend = tomori_server::fake::FakeBackend::new();
    let server = TestServer::with_tools(backend.clone(), |backends| {
        tomori_server::Server::new(
            backends,
            Arc::new(tomori_server::test_support::TextTranscriber::default()),
        )
    });
    let mut ada = server.connect("ada", "Ada");
    let space = ada.create_space("Open").id;
    let token = eventually(|| server.tools_token());
    let session = tomori_backend::Session::id(&backend.started()[0]);
    for tool in ["pairing", "confirm_pairing", "decline_pairing"] {
        let params = json!({
            "name": tool,
            "arguments": {"code": "123 456"},
            "_meta": {"ai.opencode/sessionID": session},
        });
        assert!(
            failure(&call(&server, &token, &params))
                .starts_with("This server does not pair devices: in --auth open mode"),
            "{tool}"
        );
    }
    let line = format!(
        "The agent of space {space} (\"Open\"), session {session}, asked to decline a pairing and was refused: This server does not pair devices: in --auth open mode everyone who connects is already an owner, so there is no pairing to list, confirm or decline."
    );
    eventually(|| logged().contains(&line).then_some(()));
}
