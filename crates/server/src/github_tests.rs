// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;

const TOKEN: &str = "ghp_TESTTOKEN0123456789";

fn repository() -> Repository {
    "ioquatix/tomori".parse().unwrap()
}

/// A service on its own port that answers each request with what `answer` returns for the request's head and body,
/// as raw bytes, and sends each request it read to the returned receiver.
fn service(answer: fn(&str, &str) -> Vec<u8>) -> (u16, mpsc::Receiver<(String, String)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sender, requests) = mpsc::channel();
    std::thread::spawn(move || {
        for mut socket in listener.incoming().flatten() {
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut head = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
                head.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; length];
            let _ = reader.read_exact(&mut body);
            let body = String::from_utf8_lossy(&body).into_owned();
            let response = answer(&head, &body);
            let _ = sender.send((head, body));
            let _ = socket.write_all(&response);
        }
    });
    (port, requests)
}

/// An HTTP response with `status` and JSON `body`, and `fields` besides.
fn respond(status: &str, fields: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{fields}\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn github(port: u16) -> GitHub {
    GitHub::with_api(
        repository(),
        Token::new(TOKEN).unwrap(),
        &format!("http://127.0.0.1:{port}/graphql"),
    )
    .unwrap()
}

/// GitHub's answer with these issues and pull requests.
fn data(issues: Value, pulls: Value) -> String {
    serde_json::json!({"data": {
        "rateLimit": {"remaining": 4_990, "resetAt": "2026-11-05T10:00:00Z"},
        "repository": {"issues": {"nodes": issues}, "pullRequests": {"nodes": pulls}},
    }})
    .to_string()
}

fn answer(status: u16, body: &str) -> Answer {
    Answer {
        status,
        retry_after: None,
        reset: None,
        body: body.as_bytes().to_vec(),
    }
}

/// 2026-11-05T09:00:00Z.
const NOW: u64 = 1_793_869_200;

#[test]
fn repositories_are_owner_and_name() {
    let repository = repository();
    assert_eq!(repository.to_string(), "ioquatix/tomori");
    assert!("a-b_c.d/e.f".parse::<Repository>().is_ok());
    for refused in [
        "tomori",
        "/tomori",
        "ioquatix/",
        "a/b/c",
        "a b/c",
        "ioquatix/tömori",
    ] {
        let error = refused.parse::<Repository>().unwrap_err();
        assert!(error.contains("OWNER/NAME"), "{refused}: {error}");
    }
    assert!(
        format!("{}/x", "a".repeat(101))
            .parse::<Repository>()
            .is_err()
    );
}

#[test]
fn a_token_is_found_without_ever_being_shown() {
    let token = Token::new(&format!("  {TOKEN}\n")).unwrap();
    assert_eq!(format!("{token:?}"), "Token(hidden)");
    assert_eq!(Token::new(" \n"), None);
    // Only visible ASCII is a token, so that it is always a valid field value.
    assert_eq!(Token::new("one\ntwo"), None);
    assert_eq!(Token::new("tök"), None);
    // The variable wins, and the command is not run. Running commands is tested in `tests/github_token.rs`, a process
    // of its own, since a child started here would hold the other tests' locked state directories open.
    let mut never = Command::new("/nonexistent/gh");
    assert_eq!(Token::find(Some(TOKEN.into()), &mut never).unwrap(), token);
    // The source's description names the repository and where it reads from, never the token.
    let github = GitHub::new(repository(), token);
    let described = format!("{github:?}");
    assert!(
        described.contains("ioquatix") && !described.contains(TOKEN),
        "{described}"
    );
    assert_eq!(github.source(), "ioquatix/tomori");
}

#[test]
fn endpoints_are_http_or_https_urls_with_a_host() {
    assert_eq!(
        Api::parse(API),
        Some(Api {
            https: true,
            host: "api.github.com".into(),
            port: 443,
            path: "/graphql".into(),
        })
    );
    assert_eq!(
        Api::parse("http://localhost"),
        Some(Api {
            https: false,
            host: "localhost".into(),
            port: 80,
            path: "/".into(),
        })
    );
    assert_eq!(Api::parse("http://127.0.0.1:8080/x").unwrap().port, 8080);
    for refused in [
        "ftp://host/",
        "api.github.com",
        "https://:443/",
        "http://host:0/",
        "http://host:x/",
    ] {
        assert_eq!(Api::parse(refused), None, "{refused}");
    }
    assert!(GitHub::with_api(repository(), Token::new(TOKEN).unwrap(), "nowhere").is_none());
}

#[test]
fn times_are_read_as_github_writes_them() {
    assert_eq!(timestamp("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(timestamp("2026-11-05T09:00:00Z"), Some(NOW));
    assert_eq!(timestamp("2000-02-29T12:34:56Z"), Some(951_827_696));
    for refused in [
        "2026-11-05T09:00:00",
        "2026-11-05T09:00:00+00:00",
        "2026-11-05 09:00:00Z",
        "20x6-11-05T09:00:00Z",
        "2026-11-05T09:0x:00Z",
        "2026-13-05T09:00:00Z",
        "2026-00-05T09:00:00Z",
        "2026-11-32T09:00:00Z",
        "2026-11-00T09:00:00Z",
        "1969-12-31T23:59:59Z",
    ] {
        assert_eq!(timestamp(refused), None, "{refused}");
    }
}

#[test]
fn issues_and_pull_requests_become_cards_most_recently_updated_first() {
    let issues = serde_json::json!([
        {"number": 97, "title": "Physics\nfor cards", "url": "u", "createdAt": "2026-11-02T09:00:00Z",
         "updatedAt": "2026-11-04T09:00:00Z", "author": {"login": "ioquatix"},
         "labels": {"nodes": [{"name": "enhancement", "color": "a2eeef"}, {"name": "odd", "color": "nope"},
                              {"color": "ffffff"}]}},
        // No times, nor author: opened now, and by a ghost.
        {"number": 5, "title": "Old", "author": null},
        // Neither a number nor a title is no card.
        {"title": "No number"},
        {"number": 6},
    ]);
    let pulls = serde_json::json!([
        {"number": 164, "title": "Publish", "isDraft": false, "createdAt": "2026-11-05T08:00:00Z",
         "updatedAt": "2026-11-05T08:30:00Z", "author": {"login": "ada"},
         "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": "SUCCESS"}}}]}},
        {"number": 165, "title": "Draft", "isDraft": true, "createdAt": "2026-11-01T09:00:00Z",
         "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": "FAILURE"}}}]}},
        {"number": 166, "title": "Errored", "createdAt": "2026-11-01T09:00:00Z",
         "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": "ERROR"}}}]}},
        {"number": 167, "title": "Pending", "createdAt": "2026-11-01T09:00:00Z",
         "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": "PENDING"}}}]}},
        {"number": 168, "title": "Expected", "createdAt": "2026-11-01T09:00:00Z",
         "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": "EXPECTED"}}}]}},
        {"number": 169, "title": "Newer", "createdAt": "2026-11-01T09:00:00Z",
         "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": "SKIPPED"}}}]}},
        {"number": 170, "title": "No checks", "createdAt": "2026-11-01T09:00:00Z",
         "commits": {"nodes": [{"commit": {"statusCheckRollup": null}}]}},
    ]);
    let fetched = interpret(&repository(), answer(200, &data(issues, pulls)), NOW).unwrap();
    let order: Vec<u64> = fetched.cards.iter().map(|card| card.card.number).collect();
    // Issue 5 was "updated" now, then the pull request this morning, issue 97 yesterday, and the rest, by number.
    assert_eq!(order, [5, 164, 97, 170, 169, 168, 167, 166, 165]);
    let card = |number: u64| {
        fetched
            .cards
            .iter()
            .find(|card| card.card.number == number)
            .unwrap()
    };
    let issue = card(97);
    assert_eq!(issue.card.id, "github://ioquatix/tomori/issues/97");
    assert_eq!(issue.url, "https://github.com/ioquatix/tomori/issues/97");
    assert_eq!(issue.card.kind, CardKind::Issue);
    assert_eq!(issue.card.title, "Physics for cards");
    assert_eq!(issue.card.author, "ioquatix");
    assert_eq!(issue.card.checks, Checks::None);
    assert_eq!(issue.opened, NOW - 3 * 86_400);
    let labels: Vec<(&str, &str)> = issue
        .card
        .labels
        .iter()
        .map(|label| (label.name.as_str(), label.color.as_str()))
        .collect();
    assert_eq!(labels, [("enhancement", "a2eeef"), ("odd", "808080")]);
    assert_eq!(
        (card(5).card.author.as_str(), card(5).opened),
        ("ghost", NOW)
    );
    let pull = card(164);
    assert_eq!(pull.card.id, "github://ioquatix/tomori/pull/164");
    assert_eq!(pull.url, "https://github.com/ioquatix/tomori/pull/164");
    assert_eq!(
        (pull.card.kind, pull.card.checks),
        (CardKind::PullRequest, Checks::Passing)
    );
    assert_eq!(
        (card(165).card.kind, card(165).card.checks),
        (CardKind::Draft, Checks::Failing)
    );
    assert_eq!(card(166).card.checks, Checks::Failing);
    assert_eq!(card(167).card.checks, Checks::Pending);
    assert_eq!(card(168).card.checks, Checks::Pending);
    assert_eq!(card(169).card.checks, Checks::Unknown);
    assert_eq!(card(170).card.checks, Checks::None);
    assert_eq!(
        fetched.rate,
        Some(Rate {
            remaining: 4_990,
            reset: Duration::from_secs(3_600),
        })
    );
}

#[test]
fn at_most_the_card_limit_is_kept_and_a_missing_list_is_empty() {
    let issues: Vec<Value> = (1..=150)
        .map(|number| serde_json::json!({"number": number, "title": "t"}))
        .collect();
    let pulls = issues.clone();
    let fetched = interpret(
        &repository(),
        answer(200, &data(Value::Array(issues), Value::Array(pulls))),
        NOW,
    )
    .unwrap();
    assert_eq!(fetched.cards.len(), MAX_CARDS);
    // A repository without lists, and a rate limit without a reset time, are read as empty and resetting now.
    let bare = serde_json::json!({"data": {"rateLimit": {"remaining": 1, "resetAt": "soon"}, "repository": {}}});
    let fetched = interpret(&repository(), answer(200, &bare.to_string()), NOW).unwrap();
    assert!(fetched.cards.is_empty());
    assert_eq!(
        fetched.rate,
        Some(Rate {
            remaining: 1,
            reset: Duration::ZERO,
        })
    );
    // Without a rate limit, or one without what remains of it, there is none.
    for unlimited in [
        serde_json::json!({"data": {"repository": {}}}),
        serde_json::json!({"data": {"rateLimit": {"resetAt": "2026-11-05T10:00:00Z"}, "repository": {}}}),
    ] {
        assert_eq!(
            interpret(&repository(), answer(200, &unlimited.to_string()), NOW)
                .unwrap()
                .rate,
            None
        );
    }
}

#[test]
fn refusals_and_errors_say_why_and_how_long_to_wait() {
    let failed = |answer: Answer| interpret(&repository(), answer, NOW).unwrap_err();
    assert_eq!(
        failed(answer(401, "")),
        Failure::new("GitHub refused the server's token")
    );
    let limited = failed(Answer {
        retry_after: Some(120),
        reset: Some(NOW + 600),
        ..answer(403, "")
    });
    assert_eq!(limited.message, "GitHub is limiting requests (403)");
    assert_eq!(limited.retry_after, Some(Duration::from_secs(120)));
    let reset = failed(Answer {
        reset: Some(NOW + 600),
        ..answer(429, "")
    });
    assert_eq!(reset.retry_after, Some(Duration::from_secs(600)));
    assert_eq!(failed(answer(403, "")).retry_after, None);
    assert_eq!(failed(answer(502, "")), Failure::new("GitHub answered 502"));
    assert_eq!(
        failed(answer(200, "<html>")),
        Failure::new("GitHub's answer is not JSON")
    );
    let missing = serde_json::json!({"data": {"repository": null},
        "errors": [{"message": "Could not resolve to a Repository\nwith the name 'ioquatix/gone'."}]});
    assert_eq!(
        failed(answer(200, &missing.to_string())),
        Failure::new("GitHub: Could not resolve to a Repository with the name 'ioquatix/gone'.")
    );
    assert_eq!(
        failed(answer(200, r#"{"data": null}"#)),
        Failure::new("GitHub returned no repository")
    );
}

#[test]
fn the_query_reaches_github_with_the_token_only_in_its_authorization() {
    // Answered in chunks with a trailer, as a proxy might.
    let (port, requests) = service(|_, _| {
        let body = data(
            serde_json::json!([{"number": 1, "title": "One"}]),
            serde_json::json!([]),
        );
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nTrailer: x-done\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\nx-done: 1\r\n\r\n",
            body.len()
        )
        .into_bytes()
    });
    let fetched = github(port).fetch().unwrap();
    assert_eq!(fetched.cards.len(), 1);
    let (head, body) = requests.recv().unwrap();
    assert!(head.starts_with("POST /graphql HTTP/1.1\r\n"), "{head}");
    let head = head.to_ascii_lowercase();
    assert!(head.contains(&format!(
        "authorization: bearer {}",
        TOKEN.to_ascii_lowercase()
    )));
    assert!(head.contains("host: 127.0.0.1\r\n"), "{head}");
    assert!(head.contains("user-agent: tomori-server\r\n"));
    // The body names the repository and asks only to read.
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        body["variables"],
        serde_json::json!({"owner": "ioquatix", "name": "tomori"})
    );
    let query = body["query"].as_str().unwrap();
    assert!(query.starts_with("query(") && !query.contains("mutation"));
    assert!(!body.to_string().contains(TOKEN));
}

#[test]
fn limits_and_failures_of_the_exchange_are_reported() {
    // Rate limit fields are read from the response.
    let (port, _) = service(|_, _| respond("429 Too Many Requests", "Retry-After: 61\r\n", "{}"));
    let failure = github(port).fetch().unwrap_err();
    assert_eq!(failure.retry_after, Some(Duration::from_secs(61)));
    let (port, _) = service(|_, _| {
        respond(
            "403 Forbidden",
            &format!("X-RateLimit-Reset: {}\r\n", unix_now() + 3_600),
            "{}",
        )
    });
    let wait = github(port).fetch().unwrap_err().retry_after.unwrap();
    assert!(wait > Duration::from_secs(3_500), "{wait:?}");
    // A body cut short, or too large, is not read.
    let (port, _) = service(|_, _| {
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{\"data\"".to_vec()
    });
    let failure = github(port).fetch().unwrap_err();
    assert!(
        failure
            .message
            .starts_with("could not read GitHub's answer"),
        "{failure:?}"
    );
    let (port, _) = service(|_, _| respond("200 OK", "", &" ".repeat(MAX_RESPONSE_BYTES + 1)));
    assert_eq!(
        github(port).fetch().unwrap_err(),
        Failure::new("GitHub's answer is too large")
    );
    // Nothing listening.
    let closed = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let failure = github(closed).fetch().unwrap_err();
    assert!(
        failure.message.starts_with("could not reach GitHub"),
        "{failure:?}"
    );
    // A host that cannot be resolved.
    let unresolvable = GitHub::with_api(
        repository(),
        Token::new(TOKEN).unwrap(),
        "http://bad\0host/graphql",
    )
    .unwrap();
    assert_eq!(
        unresolvable.fetch().unwrap_err(),
        Failure::new("could not resolve bad\0host")
    );
    // HTTPS is checked against the trusted roots: a local service is not GitHub.
    let (port, _) = service(|_, _| b"not tls".to_vec());
    let encrypted = GitHub::with_api(
        repository(),
        Token::new(TOKEN).unwrap(),
        &format!("https://localhost:{port}/graphql"),
    )
    .unwrap();
    let failure = encrypted.fetch().unwrap_err();
    assert!(
        failure.message.starts_with("could not reach GitHub"),
        "{failure:?}"
    );
}

#[test]
fn a_silent_service_times_out_and_a_refused_scheduler_is_reported() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut silent = github(port);
    silent.timeout = Duration::from_millis(100);
    let failure = silent.fetch().unwrap_err();
    assert_eq!(
        failure,
        Failure::new("GitHub did not answer within 0 seconds")
    );
    drop(listener);
    crate::system::refuse("github scheduler");
    let failure = silent.fetch().unwrap_err();
    assert!(
        failure.message.starts_with("could not start a scheduler"),
        "{failure:?}"
    );
}

#[test]
fn the_agent_is_told_what_the_cards_are_and_to_change_nothing_unasked() {
    let text = instructions("ioquatix/tomori");
    assert!(
        text.contains("open issues and pull requests of the GitHub repository ioquatix/tomori")
    );
    assert!(text.contains("[Pointing at the card for issue ioquatix/tomori#97"));
    assert!(text.contains("gh issue view 97 --repo ioquatix/tomori"));
    assert!(text.contains("nothing is changed on GitHub"));
}
