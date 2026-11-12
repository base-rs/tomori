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

pub mod mcp_support;
use mcp_support::{call, failure};

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
            json!({"name": "later", "note": "Later", "once_at": "23:59"}),
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
            json!({"note": "x", "every_seconds": 30}),
            "at most every minute",
        ),
        (
            json!({"note": "x", "in_seconds": -1}),
            "in_seconds is a whole number",
        ),
        (
            json!({"note": "x", "every_seconds": 604_801}),
            "every_seconds is a whole number, at most 604800.",
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
        let refused = failure(&result);
        assert!(refused.contains(said), "{arguments}: {refused}");
    }

    assert_eq!(
        failure(&call("cancel_timer", json!({}))),
        "This tool needs name, as text."
    );
    assert!(
        failure(&call("cancel_timer", json!({"name": "x".repeat(161)})))
            .contains("The name is too long")
    );
    let cancelled = call("cancel_timer", json!({"name": "tea"}));
    assert_eq!(cancelled["structuredContent"]["cancelled"], "tea");
    ada.space(|state| state.timers.len() == 4);
    // Another session has no space, and so no timers.
    let other = call_other(&server, &token);
    assert!(failure(&other).contains("not bound to a Tomori space"));
}

/// `timers` called by a session bound to no space.
fn call_other(server: &TestServer, token: &str) -> Value {
    call(server, token, "ses_nobody", "timers", json!({}))
}

#[test]
fn a_timer_set_seconds_ahead_fires_at_its_second_through_the_server() {
    let backend = FakeBackend::new();
    let server = TestServer::with_tools(backend.clone(), |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default()))
    });
    let mut ada = server.connect("ada", "Ada");
    ada.send(&ClientMessage::CreateSpace {
        title: "Gym".to_owned(),
        backend: FAKE_BACKEND_NAME.to_owned(),
        directory: "/tmp".to_owned(),
        session: None,
        environment: None,
        instructions: None,
    });
    ada.space(|state| state.title == "Gym");
    let token = eventually(|| server.tools_token());
    let session = tomori_backend::Session::id(&backend.started()[0]);
    let call = |name: &str, arguments: Value| call(&server, &token, &session, name, arguments);

    // Every so many seconds, and once at a time to the second, are set and said so.
    let set = call(
        "set_timer",
        json!({"name": "intervals", "note": "Next exercise", "every_seconds": 90}),
    );
    assert_eq!(set["structuredContent"]["timer"]["schedule"], "every 90 s");
    let set = call(
        "set_timer",
        json!({"name": "stretch", "note": "Stretch", "once_at": "23:59:59"}),
    );
    assert_eq!(
        set["structuredContent"]["timer"]["schedule"],
        "once at 23:59:59"
    );
    call("cancel_timer", json!({"name": "intervals"}));
    call("cancel_timer", json!({"name": "stretch"}));

    // Set two seconds ahead, it fires, and is gone from everyone's view, within half a second of its time, give or take
    // how long the call and the space's state take to arrive.
    let started = std::time::Instant::now();
    let set = call(
        "set_timer",
        json!({"name": "plank", "note": "Plank done", "in_seconds": 2, "condition": "always"}),
    );
    let took = started.elapsed();
    let schedule = set["structuredContent"]["timer"]["schedule"]
        .as_str()
        .unwrap();
    assert!(schedule.starts_with("once at "), "{schedule}");
    // To the second, unless it falls on a minute, as one time in sixty.
    assert!(
        ["once at 09:17:30", "once at 09:17"]
            .iter()
            .any(|said| said.len() == schedule.len()),
        "{schedule}"
    );
    assert_eq!(
        set["structuredContent"]["timer"]["next_in"]
            .as_str()
            .unwrap()
            .chars()
            .last(),
        Some('s')
    );
    ada.space(|state| state.timers.iter().any(|timer| timer.name == "plank"));
    ada.space(|state| state.timers.is_empty());
    let fired = started.elapsed();
    assert!(
        fired >= std::time::Duration::from_millis(1400),
        "fired after {fired:?}"
    );
    assert!(
        fired <= took + std::time::Duration::from_millis(2900),
        "fired after {fired:?}, the call taking {took:?}"
    );
}

#[test]
fn a_timer_named_with_an_argument_set_timer_does_not_take_is_refused_and_nothing_is_set() {
    let backend = FakeBackend::new();
    let server = TestServer::with_tools(backend.clone(), |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default()))
    });
    let mut ada = server.connect("ada", "Ada");
    ada.send(&ClientMessage::CreateSpace {
        title: "Night".to_owned(),
        backend: FAKE_BACKEND_NAME.to_owned(),
        directory: "/tmp".to_owned(),
        session: None,
        environment: None,
        instructions: None,
    });
    ada.space(|state| state.title == "Night");
    let token = eventually(|| server.tools_token());
    let session = tomori_backend::Session::id(&backend.started()[0]);
    let call = |name: &str, arguments: Value| call(&server, &token, &session, name, arguments);

    // As the agent once asked, meaning a timer that fires overnight: refused, rather than one that fires only while
    // someone is present.
    let refused = call(
        "set_timer",
        json!({"name": "overnight", "note": "Check the build", "every_minutes": 60, "when_present": false}),
    );
    assert_eq!(
        failure(&refused),
        "when_present is not an argument of set_timer; it takes: condition, count, cron, daily_at, days, every_minutes, every_seconds, in_minutes, in_seconds, name, note, once_at, until, zone."
    );
    let refused = call("cancel_timer", json!({"name": "overnight", "force": true}));
    assert_eq!(
        failure(&refused),
        "force is not an argument of cancel_timer; it takes: name."
    );
    let refused = call("timers", json!({"all": true}));
    assert_eq!(
        failure(&refused),
        "all is not an argument of timers; it takes: none."
    );
    assert_eq!(
        call("timers", json!({}))["structuredContent"]["timers"],
        json!([])
    );

    // As it should have asked, with when_present still a condition it takes.
    let set = call(
        "set_timer",
        json!({"name": "overnight", "note": "Check the build", "every_minutes": 60, "condition": "always"}),
    );
    assert_eq!(
        set["structuredContent"]["timer"]["condition"], "always",
        "{set}"
    );
    let set = call(
        "set_timer",
        json!({"name": "daytime", "note": "Stand up", "every_minutes": 60, "condition": "when_present"}),
    );
    assert_eq!(
        set["structuredContent"]["timer"]["condition"], "present",
        "{set}"
    );
}
