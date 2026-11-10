// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tomori's timer tools through its MCP endpoint: setting a timer in each form, refusing what its rules do not allow,
//! listing and cancelling, only for the space's own session, and everyone in the space seeing them. See
//! `design/timers.md` and `design/mcp.md`.
use std::sync::Arc;

use serde_json::{Value, json};
use tomori_protocol::ClientMessage;
use tomori_server::Server;
use tomori_server::fake::{FAKE_BACKEND_NAME, FakeBackend};
use tomori_server::test_support::{TestServer, TextTranscriber, eventually};

/// Call the tool `name` with `arguments` for the agent of `session`, as OpenCode does, and return its result.
fn call(server: &TestServer, token: &str, session: &str, name: &str, arguments: Value) -> Value {
    let params = json!({
        "name": name,
        "arguments": arguments,
        "_meta": { "ai.opencode/sessionID": session },
    });
    let body =
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params }).to_string();
    let mut stream = std::net::TcpStream::connect(server.tools.unwrap()).unwrap();
    std::io::Write::write_all(
        &mut stream,
        format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .as_bytes(),
    )
    .unwrap();
    let mut response = String::new();
    std::io::Read::read_to_string(&mut stream, &mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    serde_json::from_str::<Value>(body).unwrap()["result"].clone()
}

/// The text of a refused call's result.
fn refusal(result: &Value) -> &str {
    assert_eq!(result["isError"], true, "{result}");
    result["content"][0]["text"].as_str().unwrap()
}

#[test]
fn the_agent_sets_lists_and_cancels_timers_that_everyone_in_its_space_sees() {
    let backend = FakeBackend::new();
    let server = TestServer::with_tools(backend.clone(), |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default()))
    });
    let mut ada = server.connect("ada", "Ada");
    ada.send(&ClientMessage::CreateSpace {
        title: "Studio".to_owned(),
        backend: FAKE_BACKEND_NAME.to_owned(),
        directory: "/tmp".to_owned(),
        session: None,
        environment: None,
        instructions: None,
    });
    ada.space(|state| state.title == "Studio");
    let token = eventually(|| server.tools_token());
    let session = tomori_backend::Session::id(&backend.started()[0]);
    let call = |name: &str, arguments: Value| call(&server, &token, &session, name, arguments);

    // Each form, in its own zone.
    let set = call(
        "set_timer",
        json!({"name": "status", "note": "Give a status update", "every_minutes": 60, "zone": "Europe/London"}),
    );
    let timer = &set["structuredContent"]["timer"];
    assert_eq!(
        (
            &timer["name"],
            &timer["schedule"],
            &timer["condition"],
            &timer["zone"]
        ),
        (
            &json!("status"),
            &json!("every hour"),
            &json!("present"),
            &json!("Europe/London")
        )
    );
    let state = ada.space(|state| !state.timers.is_empty());
    assert_eq!(state.timers[0].schedule, "every hour");
    assert_eq!(state.timers[0].condition, "when present");
    for (arguments, schedule) in [
        (
            json!({"name": "morning", "note": "Summarise overnight", "daily_at": "09:00", "days": ["weekdays"], "condition": "present_or_arrival"}),
            "weekdays at 09:00",
        ),
        (
            json!({"name": "tea", "note": "Tea", "in_minutes": 20, "condition": "always"}),
            "once at",
        ),
        (
            json!({"name": "halves", "note": "Check CI", "cron": "*/30 9-17 * * 1-5", "count": 3}),
            "cron */30 9-17 * * 1-5",
        ),
        (
            json!({"name": "later", "note": "Later", "once_at": "23:59", "name_unknown": true}),
            "once at 23:59",
        ),
    ] {
        let set = call("set_timer", arguments.clone());
        let said = set["structuredContent"]["timer"]["schedule"]
            .as_str()
            .unwrap_or_else(|| panic!("{arguments}: {set}"));
        assert!(said.starts_with(schedule), "{said}");
    }
    let listed = call("timers", json!({}));
    let names: Vec<&str> = listed["structuredContent"]["timers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|timer| timer["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["status", "morning", "tea", "halves", "later"]);

    // What the arguments cannot say, or the rules do not allow, is refused.
    for (arguments, said) in [
        (
            json!({"every_minutes": 60}),
            "This tool needs note, as text.",
        ),
        (
            json!({"note": "x", "every_minutes": "sixty"}),
            "every_minutes is a whole number, at most 10080.",
        ),
        (
            json!({"note": "x", "in_minutes": -1}),
            "in_minutes is a whole number",
        ),
        (
            json!({"note": "x", "count": 1.5, "every_minutes": 60}),
            "count is a whole number",
        ),
        (
            json!({"note": "x", "daily_at": "09:00", "days": "mon"}),
            "days is a list of at most 7 day names",
        ),
        (
            json!({"note": "x", "daily_at": "09:00", "days": [1]}),
            "Each of days is a day's name",
        ),
        (
            json!({"note": "x", "every_minutes": 60, "condition": "sometimes"}),
            "condition is present, present_or_arrival or always.",
        ),
        (
            json!({"note": "x", "every_minutes": 60, "zone": 3}),
            "zone is text.",
        ),
        (
            json!({"note": "x", "every_minutes": 2}),
            "at most every 5 minutes",
        ),
        (json!({"note": "x"}), "Give exactly one schedule"),
        (
            json!({"note": "x", "every_minutes": 60, "zone": "Mars/Base"}),
            "does not know the time zone",
        ),
        (
            json!({"note": "x".repeat(801), "every_minutes": 60}),
            "The note is too long",
        ),
        (
            json!({"note": "x", "every_minutes": 60, "condition": 3}),
            "condition is text.",
        ),
        (
            json!({"note": "x", "every_minutes": 60, "name": 3}),
            "name is text.",
        ),
        (json!({"note": "x", "daily_at": 9}), "daily_at is text."),
        (json!({"note": "x", "once_at": 9}), "once_at is text."),
        (json!({"note": "x", "cron": 9}), "cron is text."),
        (
            json!({"note": "x", "every_minutes": 60, "until": 9}),
            "until is text.",
        ),
    ] {
        let result = call("set_timer", arguments.clone());
        let refused = refusal(&result);
        assert!(refused.contains(said), "{arguments}: {refused}");
    }

    assert_eq!(
        refusal(&call("cancel_timer", json!({}))),
        "This tool needs name, as text."
    );
    assert!(
        refusal(&call("cancel_timer", json!({"name": "x".repeat(161)})))
            .contains("The name is too long")
    );
    let cancelled = call("cancel_timer", json!({"name": "tea"}));
    assert_eq!(cancelled["structuredContent"]["cancelled"], "tea");
    ada.space(|state| state.timers.len() == 4);
    // Another session has no space, and so no timers.
    let other = call_other(&server, &token);
    assert!(refusal(&other).contains("not bound to a Tomori space"));
}

/// `timers` called by a session bound to no space.
fn call_other(server: &TestServer, token: &str) -> Value {
    call(server, token, "ses_nobody", "timers", json!({}))
}
