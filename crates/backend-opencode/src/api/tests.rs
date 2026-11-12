// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use base_scheduler::{
    AddressHints, AddressInfoCollection, AddressInfoView, AddressResolution, AddressService,
};
use base_scheduler_native::{NativeOptions, NativeScheduler};
use futures_lite::future::block_on;
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};

/// A service on its own port that answers each request with what `route` returns for its method and path, as a
/// status line and a JSON body, until the test ends.
pub(crate) fn service(route: fn(&str, &str) -> (&'static str, String)) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut socket in listener.incoming().flatten() {
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut words = line.split_whitespace();
            let (method, path) = (
                words.next().unwrap().to_owned(),
                words.next().unwrap().to_owned(),
            );
            let mut length = 0;
            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
            }
            reader.read_exact(&mut vec![0; length]).unwrap();
            let (status, body) = route(&method, &path);
            let _ = write!(
                socket,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    port
}

/// A port nothing listens on: the discard port, which tests cannot bind, being privileged, and the system never hands out for another test's ephemeral port, as it would a port bound and released here.
pub(crate) fn unreachable_port() -> u16 {
    9
}

/// Run `calls` with a client of the service on `port`, then stop the client.
pub(crate) fn with_client<T>(
    port: u16,
    calls: impl AsyncFnOnce(&Client<'_>, &Resolved, &Cancellation, &NativeScheduler) -> T,
) -> T {
    block_on(async {
        let scheduler = NativeScheduler::with_options(NativeOptions {
            workers: 1,
            threads: 2,
            capacity: 256,
            ..Default::default()
        })
        .unwrap();
        let cancellation = Cancellation::new();
        let address = scheduler
            .address_resolve(
                Some("127.0.0.1"),
                AddressService::Port(port),
                AddressHints::default(),
                &cancellation,
            )
            .await
            .unwrap()
            .entries()
            .next()
            .unwrap()
            .address();
        let (client, driver) = Client::new(
            &scheduler,
            base_http_client::Endpoint::new(address),
            base_http_client::Options::default(),
        )
        .unwrap();
        let resolved = Resolved {
            host: "127.0.0.1".into(),
            port,
            auth: Some(("opencode".into(), "secret".into())),
        };
        let (_driven, value) = futures_lite::future::zip(driver.run(&cancellation), async {
            let value = calls(&client, &resolved, &cancellation, &scheduler).await;
            cancellation.cancel();
            value
        })
        .await;
        drop(client);
        scheduler.shutdown().await;
        value
    })
}

/// The status and message of a failed call.
fn failed<T: std::fmt::Debug>(result: Result<T, Failure>) -> (Option<u16>, String) {
    let failure = result.unwrap_err();
    (failure.status, failure.message)
}

/// Every call reports a refusal with its status and the service's message, wherever OpenCode puts it.
#[test]
fn a_refused_call_reports_the_status_and_the_services_message() {
    let port = service(|_, path| {
        if path.starts_with("/api/session/s1") {
            ("409 Conflict", r#"{"data":{"message":"busy"}}"#.to_owned())
        } else {
            (
                "503 Service Unavailable",
                r#"{"message":"starting"}"#.to_owned(),
            )
        }
    });
    with_client(port, async |client, resolved, cancellation, scheduler| {
        let starting = (Some(503), "starting".to_owned());
        let busy = (Some(409), "busy".to_owned());
        assert_eq!(
            failed(create_session(client, resolved, "/tmp", Some("t"), cancellation).await),
            starting
        );
        assert_eq!(
            failed(list_sessions(client, resolved, "/tmp", cancellation).await),
            starting
        );
        assert_eq!(
            failed(active_sessions(client, resolved, cancellation).await),
            starting
        );
        assert_eq!(
            failed(child_sessions(client, resolved, "s1", cancellation).await),
            starting
        );
        assert_eq!(
            failed(list_agents(client, resolved, "/tmp", cancellation).await),
            starting
        );
        assert_eq!(
            failed(session_info(client, resolved, "s1", cancellation).await),
            busy
        );
        assert_eq!(
            failed(send_prompt(client, resolved, "s1", "k", "Hi", cancellation).await),
            busy
        );
        assert_eq!(
            failed(switch_agent(client, resolved, "s1", "plan", cancellation).await),
            busy
        );
        assert_eq!(
            failed(interrupt(client, resolved, "s1", cancellation).await),
            busy
        );
        assert_eq!(
            failed(delete_session(client, resolved, "s1", cancellation).await),
            busy
        );
        assert_eq!(
            failed(open_permissions(client, resolved, "s1", cancellation).await),
            busy
        );
        assert_eq!(
            failed(reply_permission(client, resolved, "s1", "p1", "once", cancellation).await),
            busy
        );
        assert_eq!(
            failed(open_forms(client, resolved, "s1", cancellation).await),
            busy
        );
        assert_eq!(
            failed(reply_form(client, resolved, "s1", "f1", &json!({}), cancellation).await),
            busy
        );
        assert_eq!(
            failed(session_messages(client, resolved, "s1", cancellation).await),
            busy
        );
        assert_eq!(
            failed(session_running(client, resolved, "s1", cancellation, scheduler).await),
            busy
        );
        assert_eq!(
            active_sessions_with_timeout(
                client,
                resolved,
                cancellation,
                scheduler,
                Duration::from_secs(5)
            )
            .await,
            None
        );
    });
}

#[test]
fn a_refusal_without_a_message_names_its_status() {
    let port = service(|_, _| ("500 Internal Server Error", "not json".to_owned()));
    with_client(port, async |client, resolved, cancellation, _| {
        assert_eq!(
            failed(interrupt(client, resolved, "s1", cancellation).await),
            (Some(500), "the server answered with status 500".to_owned())
        );
    });
}

#[test]
fn an_unparseable_or_malformed_answer_is_a_failure() {
    let port = service(|_, path| match path {
        "/api/session/active" => ("200 OK", r#"{"data":[]}"#.to_owned()),
        _ if path.ends_with("/permission") || path.ends_with("/form") => {
            ("200 OK", r#"{"data":{}}"#.to_owned())
        }
        _ => ("200 OK", "{not json".to_owned()),
    });
    with_client(port, async |client, resolved, cancellation, _| {
        let (status, message) = failed(session_info(client, resolved, "s1", cancellation).await);
        assert_eq!(status, Some(200));
        assert!(
            message.starts_with("the server sent an unparseable body: "),
            "{message}"
        );
        assert_eq!(
            failed(active_sessions(client, resolved, cancellation).await).0,
            None
        );
        assert_eq!(
            failed(open_permissions(client, resolved, "s1", cancellation).await).0,
            None
        );
        assert_eq!(
            failed(open_forms(client, resolved, "s1", cancellation).await).0,
            None
        );
    });
}

/// A child session record of `parent` that OpenCode would list.
fn child(id: &str, parent: &str) -> Value {
    json!({"id":id,"parentID":parent,"projectID":"p","cost":0,"tokens":{"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}},"location":{"directory":"/tmp"},"time":{"created":1,"updated":1}})
}

/// One page of a listing, as OpenCode answers it.
fn page(data: Vec<Value>, next: Value) -> (&'static str, String) {
    (
        "200 OK",
        json!({"data": data, "cursor": {"next": next}}).to_string(),
    )
}

/// What listing the children of `p` from a service routed by `route` fails with.
fn child_failure(route: fn(&str, &str) -> (&'static str, String)) -> String {
    let mut message = String::new();
    with_client(service(route), async |client, resolved, cancellation, _| {
        message = failed(child_sessions(client, resolved, "p", cancellation).await).1;
    });
    message
}

/// A child listing that cannot be trusted whole fails, rather than reporting part of the family.
#[test]
fn a_child_listing_that_cannot_be_trusted_whole_fails() {
    assert_eq!(
        child_failure(|_, _| page(vec![json!({"parentID":"p"})], Value::Null)),
        "the server returned a child session without an identifier"
    );
    assert_eq!(
        child_failure(|_, _| page(vec![child("c", "q")], Value::Null)),
        "the server returned a child session with an invalid parent or identifier"
    );
    assert_eq!(
        child_failure(|_, path| match path.contains("cursor=") {
            false => page(vec![child("c", "p")], json!("next")),
            true => page(vec![child("c", "p")], Value::Null),
        }),
        "the server repeated a child session across pages"
    );
    assert_eq!(
        child_failure(|_, _| page(Vec::new(), json!(7))),
        "the server returned an invalid child session cursor"
    );
    // A cursor seen before would list the same pages forever.
    assert_eq!(
        child_failure(|_, _| page(Vec::new(), json!("again"))),
        "the server returned an invalid child session cursor"
    );
    assert_eq!(
        child_failure(|_, path| {
            let next = path
                .rsplit_once("cursor=")
                .map_or(0, |(_, n)| n.parse::<u32>().unwrap() + 1);
            page(Vec::new(), json!(next.to_string()))
        }),
        "the child session list exceeds the page limit"
    );
    assert_eq!(
        child_failure(|_, _| page(
            (0..=MAX_TRACKED_WORKERS)
                .map(|n| child(&format!("c{n}"), "p"))
                .collect(),
            Value::Null
        )),
        "the child session list exceeds the worker limit"
    );
    assert_eq!(
        child_failure(|_, _| {
            let mut large = child("c", "p");
            large["title"] = json!("x".repeat(MAX_WORKER_SNAPSHOT_BYTES));
            page(vec![large], Value::Null)
        }),
        "the worker family exceeds the supported snapshot size"
    );
}

/// What reading the worker family of `s` from a service routed by `route` returns.
fn family(route: fn(&str, &str) -> (&'static str, String)) -> Result<Vec<String>, String> {
    let mut result = Ok(Vec::new());
    with_client(
        service(route),
        async |client, resolved, cancellation, timers| {
            let mut walk = FamilyWalk::new("s");
            result = walk
                .run(client, resolved, cancellation, timers)
                .await
                .map(|()| {
                    walk.take_found()
                        .iter()
                        .map(|child| child["id"].as_str().unwrap().to_owned())
                        .collect()
                })
                .map_err(|failure| failure.message);
        },
    );
    result
}

/// A walk stopped while it lists a parent's children lists them again when it goes on, and finds each
/// descendant once.
#[test]
fn a_stopped_family_walk_goes_on_where_it_stopped() {
    with_client(
        service(|_, path| {
            let parent = path
                .split_once("parentID=")
                .unwrap()
                .1
                .split('&')
                .next()
                .unwrap();
            match parent {
                "s" => page(vec![child("c1", "s")], Value::Null),
                "c1" => {
                    // Slow enough that the first run stops while listing it.
                    std::thread::sleep(Duration::from_millis(200));
                    page(vec![child("g1", "c1")], Value::Null)
                }
                _ => page(Vec::new(), Value::Null),
            }
        }),
        async |client, resolved, cancellation, timers| {
            use base_scheduler::Clock as _;
            let mut walk = FamilyWalk::new("s");
            futures_lite::future::or(walk.run(client, resolved, cancellation, timers), async {
                timers
                    .sleep(Duration::from_millis(100), cancellation)
                    .await
                    .unwrap();
                Ok(())
            })
            .await
            .unwrap();
            assert_eq!(walk.take_found(), [child("c1", "s")]);
            walk.run(client, resolved, cancellation, timers)
                .await
                .unwrap();
            assert_eq!(walk.take_found(), [child("g1", "c1")]);
        },
    );
}

/// A walk counts the workers it found before it stopped against the worker limit.
#[test]
fn a_family_walk_is_held_to_the_worker_limit_across_its_runs() {
    with_client(
        service(|_, path| match path.contains("parentID=s&") {
            true => page(vec![child("c1", "s")], Value::Null),
            false => page(Vec::new(), Value::Null),
        }),
        async |client, resolved, cancellation, timers| {
            let mut walk = FamilyWalk::new("s");
            walk.count = MAX_TRACKED_WORKERS;
            assert_eq!(
                failed(walk.run(client, resolved, cancellation, timers).await).1,
                "the worker family exceeds the worker limit"
            );
            // None of the page that would exceed it was taken, and its
            // parent is listed again when the walk goes on.
            assert!(walk.take_found().is_empty());
            walk.count = 0;
            walk.run(client, resolved, cancellation, timers)
                .await
                .unwrap();
            assert_eq!(walk.take_found(), [child("c1", "s")]);

            // Likewise for the size of the records found.
            let mut walk = FamilyWalk::new("s");
            walk.bytes = MAX_WORKER_SNAPSHOT_BYTES;
            assert_eq!(
                failed(walk.run(client, resolved, cancellation, timers).await).1,
                "the worker family exceeds the supported snapshot size"
            );
            assert!(walk.take_found().is_empty());
        },
    );
}

/// A walk whose listing of a parent's children failed lists them again
/// when it goes on, and finds each descendant once.
#[test]
fn a_failed_family_walk_goes_on_with_the_listing_that_failed() {
    static C1_LISTINGS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    with_client(
        service(|_, path| {
            let parent = path
                .split_once("parentID=")
                .unwrap()
                .1
                .split('&')
                .next()
                .unwrap();
            match parent {
                "s" => page(vec![child("c1", "s"), child("c2", "s")], Value::Null),
                "c1" if C1_LISTINGS.fetch_add(1, std::sync::atomic::Ordering::AcqRel) == 0 => (
                    "500 Internal Server Error",
                    r#"{"message":"busy"}"#.to_owned(),
                ),
                "c1" => page(vec![child("g1", "c1")], Value::Null),
                _ => page(Vec::new(), Value::Null),
            }
        }),
        async |client, resolved, cancellation, timers| {
            let mut walk = FamilyWalk::new("s");
            assert_eq!(
                failed(walk.run(client, resolved, cancellation, timers).await).1,
                "busy"
            );
            assert_eq!(walk.take_found(), [child("c1", "s"), child("c2", "s")]);
            walk.run(client, resolved, cancellation, timers)
                .await
                .unwrap();
            assert_eq!(walk.take_found(), [child("g1", "c1")]);
            assert_eq!(C1_LISTINGS.load(std::sync::atomic::Ordering::Acquire), 2);
        },
    );
}

#[test]
fn snapshot_workers_are_listed_oldest_first() {
    let created = |id: &str, time: f64| {
        let mut record = child(id, "s");
        record["time"]["created"] = json!(time);
        record
    };
    let parts = SnapshotParts {
        messages: Value::Null,
        permissions: Vec::new(),
        forms: Vec::new(),
        // As discovered, parent by parent, and as identifiers sort: neither is the order they started.
        children: vec![
            created("ses_a", 30.0),
            created("ses_c", 10.0),
            created("ses_b", 20.5),
            created("ses_d", 20.5),
        ],
        busy: None,
        has_parent: false,
        title: None,
        directory: None,
    };
    assert_eq!(
        parts
            .workers()
            .iter()
            .map(|worker| worker.id.as_str())
            .collect::<Vec<_>>(),
        ["ses_c", "ses_b", "ses_d", "ses_a"]
    );
}

#[test]
fn a_worker_family_is_read_whole_or_not_at_all() {
    // Grandchildren are workers of the session too.
    assert_eq!(
        family(|_, path| {
            match path
                .split_once("parentID=")
                .unwrap()
                .1
                .split('&')
                .next()
                .unwrap()
            {
                "s" => page(vec![child("c1", "s"), child("c2", "s")], Value::Null),
                "c1" => page(vec![child("g1", "c1")], Value::Null),
                _ => page(Vec::new(), Value::Null),
            }
        }),
        Ok(vec!["c1".to_owned(), "c2".to_owned(), "g1".to_owned()])
    );
    // A child listed under its own child would be read forever.
    assert_eq!(
        family(|_, path| {
            match path
                .split_once("parentID=")
                .unwrap()
                .1
                .split('&')
                .next()
                .unwrap()
            {
                "s" => page(vec![child("c1", "s")], Value::Null),
                _ => page(vec![child("c1", "c1")], Value::Null),
            }
        }),
        Err("the server returned a repeated or cyclic worker identity".to_owned())
    );
    // A grandchild listing that fails fails the family.
    assert_eq!(
        family(|_, path| {
            match path
                .split_once("parentID=")
                .unwrap()
                .1
                .split('&')
                .next()
                .unwrap()
            {
                "s" => page(vec![child("c1", "s")], Value::Null),
                _ => (
                    "500 Internal Server Error",
                    r#"{"message":"lost"}"#.to_owned(),
                ),
            }
        }),
        Err("lost".to_owned())
    );
}

/// A message OpenCode would list, with the id `id`.
fn message(id: &str) -> Value {
    json!({"id": id, "type": "user", "text": "Hello."})
}

/// What reading the messages of `s` from a service routed by `route` returns, as message ids.
fn messages(route: fn(&str, &str) -> (&'static str, String)) -> Result<Vec<String>, String> {
    let mut result = Ok(Vec::new());
    with_client(service(route), async |client, resolved, cancellation, _| {
        result = session_messages(client, resolved, "s", cancellation)
            .await
            .map(|messages| {
                messages["data"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|message| message["id"].as_str().unwrap().to_owned())
                    .collect()
            })
            .map_err(|failure| failure.message);
    });
    result
}

#[test]
fn message_history_is_bounded_and_refused_when_malformed() {
    let refused = |message: &str| Err(message.to_owned());
    assert_eq!(
        messages(|_, _| ("200 OK", r#"{"cursor":{"next":null}}"#.to_owned())),
        refused("the server did not return a message list")
    );
    assert_eq!(
        messages(|_, _| ("200 OK", r#"{"data":[]}"#.to_owned())),
        refused("the server did not return a message cursor")
    );
    assert_eq!(
        messages(|_, _| page(Vec::new(), json!(""))),
        refused("the server returned an invalid message cursor")
    );
    assert_eq!(
        messages(|_, _| page(Vec::new(), json!(7))),
        refused("the server returned an invalid message cursor")
    );
    assert_eq!(
        messages(|_, _| page(vec![json!({"type": "user", "text": "No id."})], Value::Null)),
        refused("the server returned a message without a supported identifier")
    );
    assert_eq!(
        messages(|_, _| page(vec![message("m"), message("m")], Value::Null)),
        refused("the server repeated a message across pages")
    );
    assert_eq!(
        messages(|_, _| page(vec![json!({"id": "m", "type": "user"})], Value::Null)),
        refused("the server returned an invalid session message")
    );
    // A cursor seen before would list the same pages forever.
    assert_eq!(
        messages(|_, path| match path.contains("cursor=") {
            false => page(vec![message("m1")], json!("older")),
            true => page(vec![message("m2")], json!("older")),
        }),
        refused("the server returned an invalid message cursor")
    );

    // Only the latest hundred messages are kept, oldest first.
    let kept = messages(|_, _| {
        page(
            (0..150).map(|n| message(&format!("m{n:03}"))).collect(),
            Value::Null,
        )
    })
    .unwrap();
    assert_eq!(kept.len(), 100);
    assert_eq!((kept[0].as_str(), kept[99].as_str()), ("m099", "m000"));
    // Older pages are read until the history ends, and older history beyond the page limit is left out.
    assert_eq!(
        messages(|_, path| {
            let page_number = path
                .rsplit_once("cursor=")
                .map_or(0, |(_, n)| n.parse::<u32>().unwrap() + 1);
            page(Vec::new(), json!(page_number.to_string()))
        }),
        Ok(Vec::new())
    );
    // So are messages beyond the size limit, once one is kept.
    assert_eq!(
        messages(|_, _| {
            let mut large = message("large");
            large["text"] = json!("x".repeat(8 * 1024 * 1024));
            page(vec![message("small"), large], Value::Null)
        }),
        Ok(vec!["small".to_owned()])
    );
}

/// A response larger than any OpenCode sends is refused while it is read, rather than held in memory.
#[test]
fn an_oversized_response_is_refused() {
    let port = service(|_, _| {
        (
            "200 OK",
            format!(r#"{{"data":"{}"}}"#, "x".repeat(MAX_JSON_BYTES)),
        )
    });
    with_client(port, async |client, resolved, cancellation, _| {
        assert_eq!(
            failed(session_info(client, resolved, "s", cancellation).await),
            (
                Some(200),
                "the server's JSON response exceeds the supported size".to_owned()
            )
        );
    });
}

/// Without a service, every call fails without a status, rather than waiting.
#[test]
fn an_unreachable_service_fails_each_call_without_a_status() {
    with_client(
        unreachable_port(),
        async |client, resolved, cancellation, scheduler| {
            assert_eq!(
                failed(create_session(client, resolved, "/tmp", None, cancellation).await).0,
                None
            );
            assert_eq!(
                failed(list_sessions(client, resolved, "/tmp", cancellation).await).0,
                None
            );
            assert_eq!(
                failed(session_info(client, resolved, "s1", cancellation).await).0,
                None
            );
            assert_eq!(
                failed(active_sessions(client, resolved, cancellation).await).0,
                None
            );
            assert_eq!(
                failed(child_sessions(client, resolved, "s1", cancellation).await).0,
                None
            );
            assert_eq!(
                failed(send_prompt(client, resolved, "s1", "k", "Hi", cancellation).await).0,
                None
            );
            assert_eq!(
                failed(list_agents(client, resolved, "/tmp", cancellation).await).0,
                None
            );
            assert_eq!(
                failed(switch_agent(client, resolved, "s1", "plan", cancellation).await).0,
                None
            );
            assert_eq!(
                failed(put_instruction(client, resolved, "s1", "k", "v", cancellation).await).0,
                None
            );
            assert_eq!(
                failed(delete_instruction(client, resolved, "s1", "k", cancellation).await).0,
                None
            );
            let server = tomori_backend::ToolServer::new("tomori", "http://127.0.0.1:1/mcp");
            assert_eq!(
                failed(put_mcp(client, resolved, "/tmp", &server, cancellation).await).0,
                None
            );
            assert_eq!(
                failed(delete_mcp(client, resolved, "/tmp", "tomori", cancellation).await).0,
                None
            );
            assert_eq!(
                failed(interrupt(client, resolved, "s1", cancellation).await).0,
                None
            );
            assert_eq!(
                failed(delete_session(client, resolved, "s1", cancellation).await).0,
                None
            );
            assert_eq!(
                failed(open_permissions(client, resolved, "s1", cancellation).await).0,
                None
            );
            assert_eq!(
                failed(reply_permission(client, resolved, "s1", "p1", "once", cancellation).await)
                    .0,
                None
            );
            assert_eq!(
                failed(open_forms(client, resolved, "s1", cancellation).await).0,
                None
            );
            assert_eq!(
                failed(reply_form(client, resolved, "s1", "f1", &json!({}), cancellation).await).0,
                None
            );
            assert_eq!(
                failed(session_messages(client, resolved, "s1", cancellation).await).0,
                None
            );
            assert_eq!(
                failed(session_running(client, resolved, "s1", cancellation, scheduler).await).0,
                None
            );
        },
    );
}

#[test]
fn session_listing_asks_for_top_level_unarchived_sessions_page_by_page() {
    assert_eq!(
        session_list_path(None),
        "/api/session?parentID=null&archived=false&limit=200"
    );
    assert_eq!(
        session_list_path(Some("a b")),
        "/api/session?parentID=null&archived=false&limit=200&cursor=a%20b"
    );
}

#[test]
fn a_directory_is_beneath_a_root_component_by_component() {
    assert!(beneath("/home/sam", "/home/sam"));
    assert!(beneath("/home/sam/", "/home/sam"));
    assert!(beneath("/home/sam/base/scheduler", "/home/sam"));
    assert!(beneath("/home/sam/base", "/home/sam/"));
    assert!(beneath("/home//sam/./base", "/home/sam"));
    assert!(!beneath("/home/sam2", "/home/sam"));
    assert!(!beneath("/home", "/home/sam"));
    assert!(!beneath("/home/sam/../other", "/home/sam"));
    assert!(!beneath("home/sam", "home"));
    assert!(!beneath("", "/home/sam"));
}

/// A top-level session record in `directory`, as OpenCode lists it.
fn listed(id: &str, directory: &str) -> Value {
    json!({"id":id,"projectID":"p","location":{"directory":directory},"time":{"created":1,"updated":1}})
}

/// The cursor a listing's request follows, if any.
fn cursor_of(path: &str) -> Option<&str> {
    path.split_once("&cursor=").map(|(_, cursor)| cursor)
}

/// Every page is read, and only the sessions beneath the directory, by their `location.directory`, are kept, in the
/// order OpenCode lists them, each once.
#[test]
fn every_page_is_read_and_sessions_beneath_the_directory_kept() {
    let port = service(|_, path| {
        assert!(path.starts_with("/api/session?parentID=null&archived=false&limit=200"));
        match cursor_of(path) {
            None => page(
                vec![
                    listed("root", "/work"),
                    listed("elsewhere", "/home/other"),
                    listed("deep", "/work/base-rs/base-scheduler"),
                    listed("sibling", "/work2/project"),
                    json!({"id":"no-location"}),
                    listed("slashed", "/work/tomori/"),
                ],
                json!("c1"),
            ),
            Some("c1") => page(
                vec![
                    listed("escaping", "/work/../etc"),
                    listed("deep", "/work/base-rs/base-scheduler"),
                    json!({"location":{"directory":"/work"}}),
                    listed("older", "/work/tomori"),
                ],
                json!("c2"),
            ),
            // The last page, after `c2`.
            Some(_) => page(Vec::new(), Value::Null),
        }
    });
    with_client(port, async |client, resolved, cancellation, _| {
        let sessions = list_sessions(client, resolved, "/work", cancellation)
            .await
            .unwrap();
        let ids: Vec<_> = sessions
            .iter()
            .map(|session| session["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["root", "deep", "slashed", "older"]);
    });
}

/// A listing stops at a cursor it has already followed, or at one that is not a string, with what it found.
#[test]
fn a_listing_stops_at_a_repeated_or_invalid_cursor() {
    let repeating = service(|_, path| match cursor_of(path) {
        None => page(vec![listed("a", "/work")], json!("again")),
        Some(_) => page(vec![listed("b", "/work")], json!("again")),
    });
    let invalid = service(|_, _| page(vec![listed("a", "/work")], json!(7)));
    for (port, expected) in [(repeating, vec!["a", "b"]), (invalid, vec!["a"])] {
        with_client(port, async |client, resolved, cancellation, _| {
            let sessions = list_sessions(client, resolved, "/work", cancellation)
                .await
                .unwrap();
            let ids: Vec<_> = sessions
                .iter()
                .map(|session| session["id"].as_str().unwrap())
                .collect();
            assert_eq!(ids, expected);
        });
    }
}

/// A listing that never ends is bounded: by the sessions it returns, by those it reads, and by its pages.
#[test]
fn a_listing_is_bounded() {
    /// The page after `cursor`, numbered from 0, as the next cursor.
    fn next(path: &str) -> u64 {
        cursor_of(path).map_or(0, |cursor| cursor.parse::<u64>().unwrap() + 1)
    }
    /// A page of `SESSION_PAGE` sessions in `directory`, named after `number`.
    fn full(number: u64, directory: &str) -> Vec<Value> {
        (0..SESSION_PAGE)
            .map(|index| listed(&format!("s{number}-{index}"), directory))
            .collect()
    }
    let matching = service(|_, path| {
        let number = next(path);
        page(full(number, "/work"), json!(number.to_string()))
    });
    let elsewhere = service(|_, path| {
        let number = next(path);
        page(full(number, "/other"), json!(number.to_string()))
    });
    let empty = service(|_, path| page(Vec::new(), json!(next(path).to_string())));
    for (port, expected) in [(matching, MAX_LISTED_SESSIONS), (elsewhere, 0), (empty, 0)] {
        with_client(port, async |client, resolved, cancellation, _| {
            let sessions = list_sessions(client, resolved, "/work", cancellation)
                .await
                .unwrap();
            assert_eq!(sessions.len(), expected);
        });
    }
}

/// A page without a list of sessions fails the listing.
#[test]
fn a_page_without_sessions_fails_the_listing() {
    let port = service(|_, _| ("200 OK", r#"{"data":{}}"#.to_owned()));
    with_client(port, async |client, resolved, cancellation, _| {
        assert_eq!(
            failed(list_sessions(client, resolved, "/work", cancellation).await),
            (None, "the server did not return a session list".to_owned())
        );
    });
}

#[test]
fn response_bytes_are_bounded_without_truncating_valid_json() {
    let mut bytes = vec![b'x'; MAX_JSON_BYTES - 1];
    assert!(append_json_bytes(&mut bytes, b"y", 200).is_ok());
    assert_eq!(bytes.len(), MAX_JSON_BYTES);
    assert!(append_json_bytes(&mut bytes, b"z", 200).is_err());
    assert_eq!(bytes.len(), MAX_JSON_BYTES);
}

#[test]
fn aggregate_worker_records_are_bounded_without_partial_success() {
    let child = json!({"id":"child"});
    let size = serde_json::to_vec(&child).unwrap().len();
    let mut total = MAX_WORKER_SNAPSHOT_BYTES - size;
    assert!(add_worker_record_size(&mut total, &child).is_ok());
    assert_eq!(total, MAX_WORKER_SNAPSHOT_BYTES);
    assert!(add_worker_record_size(&mut total, &child).is_err());
    assert_eq!(total, MAX_WORKER_SNAPSHOT_BYTES);
}

#[test]
fn snapshot_message_validation_rejects_missing_required_content() {
    assert!(valid_snapshot_message(
        &json!({"type":"user","text":"hello"})
    ));
    assert!(!valid_snapshot_message(&json!({"id":"m"})));
    assert!(!valid_snapshot_message(&json!({"type":"user"})));
    assert!(valid_snapshot_message(
        &json!({"type":"assistant","content":[{"type":"text","text":"hi"}]})
    ));
    assert!(!valid_snapshot_message(
        &json!({"type":"assistant","content":[{"type":"text"}]})
    ));
    assert!(!valid_snapshot_message(
        &json!({"type":"assistant","content":[{"type":"tool","id":"t"}]})
    ));
    assert!(valid_snapshot_message(&json!({"type":"agent-switched"})));
    assert!(!valid_snapshot_message(&json!({"type":"unrecognized"})));
}

#[test]
fn session_running_state_handles_a_first_turn_without_idle_timestamp() {
    assert_eq!(
        running_from_record(&json!({"time":{"busy":10}})),
        Some(true)
    );
    assert_eq!(
        running_from_record(&json!({"time":{"busy":10.5,"idle":10.25}})),
        Some(true)
    );
    assert_eq!(
        running_from_record(&json!({"time":{"busy":10,"idle":11}})),
        Some(false)
    );
    assert_eq!(
        running_from_record(&json!({"time":{"busy":10,"idle":10}})),
        Some(false)
    );
    assert_eq!(running_from_record(&json!({"time":{}})), Some(false));
    assert_eq!(
        running_from_record(&json!({"time":{"busy":"invalid"}})),
        None
    );
    assert_eq!(running_from_record(&json!({})), None);
    assert_eq!(running_from_record(&json!({"time": 1})), None);
    assert_eq!(
        running_from_record(&json!({"time":{"busy":1,"idle":"invalid"}})),
        None
    );
}

#[test]
fn a_child_may_have_ended_any_way_and_a_message_part_must_be_known() {
    for outcome in ["succeeded", "failed", "interrupted"] {
        let mut record = child("c", "p");
        record["outcome"] = json!(outcome);
        assert!(valid_session_record(&record));
    }
    assert!(!valid_snapshot_message(
        &json!({"type":"assistant","content":[{"type":"image"}]})
    ));
}

/// The service's sessions, each a valid record, with no messages, forms
/// or permissions except one permission request of `s1` that names a
/// resource that is not text.
fn invalid_permission(_method: &str, path: &str) -> (&'static str, String) {
    let path = path.split('?').next().unwrap_or_default();
    match path {
        "/api/session/s1/permission" => (
            "200 OK",
            json!({"data":[{"id":"p","sessionID":"s1","action":"bash","resources":[1]}]})
                .to_string(),
        ),
        "/api/session/s1/message" => page(Vec::new(), Value::Null),
        "/api/session/s1/form" => ("200 OK", r#"{"data":[]}"#.to_owned()),
        _ => ("200 OK", json!({"data": child("s1", "p")}).to_string()),
    }
}

#[test]
fn a_session_is_not_read_with_a_permission_naming_something_other_than_text() {
    with_client(
        service(invalid_permission),
        async |client, resolved, cancellation, _| {
            let parts = SnapshotParts::fetch(client, resolved, "s1", cancellation, || {}).await;
            let Err(failure) = parts else {
                unreachable!("expected the read to fail");
            };
            assert_eq!(
                failure.message,
                "the server returned an invalid permission request"
            );
        },
    );
}

/// A first page of as many messages as are read, after a record that is
/// not a message, and another.
fn many_messages(_method: &str, _path: &str) -> (&'static str, String) {
    let switched = json!({"id": "switched", "type": "agent-switched"});
    let messages = std::iter::once(switched)
        .chain((0..100).map(|index| json!({"id": format!("m{index}"), "type": "user", "text": ""})))
        .collect();
    page(messages, json!("more"))
}

#[test]
fn messages_are_read_no_further_than_the_limit() {
    with_client(
        service(many_messages),
        async |client, resolved, cancellation, _| {
            let messages = session_messages(client, resolved, "s1", cancellation)
                .await
                .unwrap();
            assert_eq!(messages["data"].as_array().unwrap().len(), 100);
        },
    );
}

#[test]
fn a_prompt_keeps_a_key_already_shaped_as_a_message_id_and_an_invalid_record_has_no_running_state()
{
    let port = service(|_, path| match path {
        "/api/session/active" => ("200 OK", r#"{"data":{}}"#.to_owned()),
        "/api/session/s1" => ("200 OK", r#"{"data":{"id":"s1"}}"#.to_owned()),
        _ => ("200 OK", "{}".to_owned()),
    });
    with_client(port, async |client, resolved, cancellation, scheduler| {
        send_prompt(client, resolved, "s1", "msg_k", "Hi", cancellation)
            .await
            .unwrap();
        let running = session_running(client, resolved, "s1", cancellation, scheduler).await;
        assert_eq!(running.unwrap(), None);
    });
}

/// A worker's latest messages come oldest first, at most as many as asked, and its inbox as listed; an answer that
/// is not a list is refused. See `design/events.md`.
#[test]
fn a_worker_s_latest_messages_and_inbox_are_read() {
    let port = service(|_, path| match path {
        "/api/session/w1/message?limit=2&order=desc" => (
            "200 OK",
            r#"{"data":[{"id":"m3"},{"id":"m2"},{"id":"m1"}]}"#.to_owned(),
        ),
        "/api/session/w1/inbox" => ("200 OK", r#"{"data":[{"id":"i1"}]}"#.to_owned()),
        "/api/session/w1/permission" | "/api/session/w4/permission" => {
            ("200 OK", r#"{"data":[{"id":"p1"}]}"#.to_owned())
        }
        "/api/session/w1/form" => ("200 OK", r#"{"data":[{"id":"f1"}]}"#.to_owned()),
        "/api/session/w4/form" => ("200 OK", r#"{"data":{}}"#.to_owned()),
        "/api/session/w3/permission" => (
            "500 Internal Server Error",
            r#"{"message":"busy"}"#.to_owned(),
        ),
        "/api/session/w3/inbox" => (
            "500 Internal Server Error",
            r#"{"message":"busy"}"#.to_owned(),
        ),
        _ => ("200 OK", r#"{"data":{}}"#.to_owned()),
    });
    with_client(port, async |client, resolved, cancellation, _| {
        let messages = recent_messages(client, resolved, "w1", 2, cancellation)
            .await
            .unwrap();
        assert_eq!(messages, [json!({"id":"m2"}), json!({"id":"m3"})]);
        let inbox = session_inbox(client, resolved, "w1", cancellation)
            .await
            .unwrap();
        assert_eq!(inbox, [json!({"id":"i1"})]);
        let requests = session_requests(client, resolved, "w1", cancellation)
            .await
            .unwrap();
        assert_eq!(requests, [json!({"id":"p1"}), json!({"id":"f1"})]);
        assert!(
            session_requests(client, resolved, "w3", cancellation)
                .await
                .is_err()
        );
        assert!(
            session_requests(client, resolved, "w4", cancellation)
                .await
                .is_err()
        );
        let refused = session_inbox(client, resolved, "w2", cancellation)
            .await
            .unwrap_err();
        assert_eq!(refused.message, "the server did not return an inbox");
        assert!(
            session_inbox(client, resolved, "w3", cancellation)
                .await
                .is_err()
        );
    });
}

/// A service that cannot be reached answers no records.
#[test]
fn a_worker_s_records_are_not_read_from_a_service_that_is_gone() {
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    with_client(port, async |client, resolved, cancellation, _| {
        assert!(
            recent_messages(client, resolved, "w1", 2, cancellation)
                .await
                .is_err()
        );
    });
}

#[test]
fn active_session_map_requires_valid_running_records() {
    let active = parse_active_sessions(&json!({"data":{"ses_a":{"type":"running"}}})).unwrap();
    assert!(active.contains("ses_a"));
    assert!(parse_active_sessions(&json!({"data":[]})).is_err());
    assert!(parse_active_sessions(&json!({"data":{"ses_a":{"type":"idle"}}})).is_err());
}
