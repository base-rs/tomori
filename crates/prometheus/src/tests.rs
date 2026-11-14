// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The client against a fake Prometheus on a loopback port, which answers each request with the reply a test gives it, so that nothing depends on timing but the time-out itself.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

use super::*;
use base_scheduler::Cancellation;

mod cache;

/// A request the fake received.
struct Seen {
    /// Which of the fake's connections it came on, counted from 0.
    connection: usize,
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }
}

/// How the fake answers a request.
enum Reply {
    Http {
        status: u16,
        headers: &'static str,
        body: Vec<u8>,
    },
    /// Promise a longer body than is sent, then hang up.
    Truncated,
}

struct Fake {
    url: String,
    seen: Receiver<Seen>,
    replies: Sender<Reply>,
}

impl Fake {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (saw, seen) = channel();
        let (replies, waiting) = channel();
        let waiting = Arc::new(Mutex::new(waiting));
        thread::spawn(move || {
            for (connection, stream) in listener.incoming().enumerate() {
                let (saw, waiting) = (saw.clone(), waiting.clone());
                thread::spawn(move || serve(connection, stream.unwrap(), saw, waiting));
            }
        });
        Self { url, seen, replies }
    }

    fn reply(&self, status: u16, headers: &'static str, body: &str) {
        self.replies
            .send(Reply::Http {
                status,
                headers,
                body: body.as_bytes().to_vec(),
            })
            .unwrap();
    }

    fn success(&self, data: Value) {
        self.reply(
            200,
            "",
            &json!({"status": "success", "data": data}).to_string(),
        );
    }

    fn next(&self) -> Seen {
        self.seen.recv().unwrap()
    }

    fn client(&self) -> Builder {
        Client::builder(&self.url)
    }
}

fn serve(
    connection: usize,
    stream: TcpStream,
    saw: Sender<Seen>,
    waiting: Arc<Mutex<Receiver<Reply>>>,
) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut stream = stream;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let mut parts = line.split_whitespace();
        let (method, target) = (
            parts.next().unwrap().to_owned(),
            parts.next().unwrap().to_owned(),
        );
        let mut headers = Vec::new();
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).unwrap();
            let Some((name, value)) = header.trim_end().split_once(':') else {
                break;
            };
            headers.push((name.to_ascii_lowercase(), value.trim().to_owned()));
        }
        let length = headers
            .iter()
            .find(|(name, _)| name == "content-length")
            .map_or(0, |(_, value)| value.parse().unwrap());
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let body = String::from_utf8(body).unwrap();
        saw.send(Seen {
            connection,
            method,
            target,
            headers,
            body,
        })
        .unwrap();
        let reply = waiting.lock().unwrap().recv().unwrap();
        let (status, headers, length, body) = match reply {
            Reply::Http {
                status,
                headers,
                body,
            } => (status, headers, body.len(), body),
            Reply::Truncated => (200, "", 100, b"{}".to_vec()),
        };
        let head = format!("HTTP/1.1 {status} Fake\r\ncontent-length: {length}\r\n{headers}\r\n");
        // The client may have given up on the request and closed the connection already.
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(&body);
        if length != body.len() || headers.contains("connection: close") {
            return;
        }
    }
}

fn vector() -> Value {
    json!({"resultType": "vector", "result": [
        {"metric": {"__name__": "up", "pod": "web-1", "url": "https://x/?key=k"}, "value": [1700000000.25, "1"]},
        {"metric": {"pod": "web-2"}, "value": [1700000000.25, "NaN"]},
    ]})
}

fn at() -> Timestamp {
    Timestamp::from_millis(1_700_000_000_250)
}

/// The fields of a form-encoded body or query.
fn form(text: &str) -> Vec<(String, String)> {
    form_urlencoded::parse(text.as_bytes())
        .into_owned()
        .collect()
}

/// A loopback address that refuses connections: the discard port, where nothing listens, below the range a listener bound to port 0 is given a port from, so that no other test's fake can be given it, as one bound and closed at once could be while tests run together.
fn refusing() -> std::net::SocketAddr {
    std::net::SocketAddr::from(([127, 0, 0, 1], 9))
}

/// A client is shared across a server side's threads.
const _: fn() = || {
    fn shared<T: Send + Sync>() {}
    shared::<Client>();
};

#[test]
fn an_instant_query_is_posted_with_its_time_and_token_over_one_kept_connection() {
    let fake = Fake::start();
    let client = fake
        .client()
        .token_source(Token::new("s3cret").unwrap())
        .build()
        .unwrap();
    fake.success(vector());
    let response = client.query("up == 1", at()).unwrap();
    let seen = fake.next();
    assert_eq!(seen.method, "POST");
    assert_eq!(seen.target, "/api/v1/query");
    assert_eq!(seen.header("authorization"), Some("Bearer s3cret"));
    assert_eq!(
        seen.header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(seen.header("accept"), Some("application/json"));
    assert_eq!(
        seen.header("user-agent"),
        Some(concat!("tomori-prometheus/", env!("CARGO_PKG_VERSION")))
    );
    assert_eq!(
        form(&seen.body),
        [
            ("query".to_owned(), "up == 1".to_owned()),
            ("time".into(), "1700000000.250".into()),
            ("timeout".into(), "30.000".into()),
        ]
    );
    let samples = response.data.into_vector().unwrap();
    assert_eq!(samples.len(), 2);
    assert_eq!(samples[0].metric["url"], "https://x/?key=k");
    assert_eq!(samples[0].point.at, at());
    assert!(samples[1].point.value.is_nan());

    // A clone shares the connection.
    let clone = client.clone();
    fake.success(json!({"resultType": "scalar", "result": [1, "2"]}));
    assert!(matches!(
        clone.query("2", at()).unwrap().data,
        Data::Scalar(_)
    ));
    assert_eq!(fake.next().connection, seen.connection);

    let debug = format!("{client:?}");
    assert!(
        debug.starts_with(
            "Client { authenticated: true, timeout: 30s, concurrency: 4, method: Post"
        )
    );
    assert!(!debug.contains("s3cret"));
}

#[test]
fn a_range_query_can_be_sent_in_the_url_beneath_another_prefix() {
    let fake = Fake::start();
    let client = Client::builder(&format!("{}/gateway/", fake.url))
        .api_prefix("/prometheus/api/v1/")
        .method(QueryMethod::Get)
        .user_agent("observe-environment/1")
        .timeout(Duration::from_millis(15_500))
        .build()
        .unwrap();
    assert_eq!(QueryMethod::default(), QueryMethod::Post);
    fake.success(json!({"resultType": "matrix", "result": [
        {"metric": {"pod": "web-1"}, "values": [[1700000000, "1"], [1700000060, "+Inf"]]},
    ]}));
    let range = Range::new(
        at() - Duration::from_secs(60),
        at(),
        Duration::from_secs(60),
    );
    let response = client.query_range("rate(x[1m])", range).unwrap();
    let seen = fake.next();
    assert_eq!(seen.method, "GET");
    assert_eq!(seen.header("authorization"), None);
    assert_eq!(seen.header("content-type"), None);
    assert_eq!(seen.header("user-agent"), Some("observe-environment/1"));
    let (path, query) = seen.target.split_once('?').unwrap();
    assert_eq!(path, "/gateway/prometheus/api/v1/query_range");
    assert_eq!(
        form(query),
        [
            ("query".to_owned(), "rate(x[1m])".to_owned()),
            ("start".into(), "1699999940.250".into()),
            ("end".into(), "1700000000.250".into()),
            ("step".into(), "60.000".into()),
            ("timeout".into(), "15.500".into()),
        ]
    );
    assert_eq!(response.data[0].points[1].value, f64::INFINITY);
    assert!(response.warnings.is_empty());

    fake.success(vector());
    assert_eq!(
        client.query_range("up", range).unwrap_err(),
        Error::Parse("a range query's answer is not a matrix".into())
    );
    fake.next();
    fake.reply(503, "", "unavailable");
    assert!(matches!(
        client.query_range("up", range),
        Err(Error::Status { status: 503, .. })
    ));
    fake.next();
}

#[test]
fn a_large_answer_within_the_limit_is_read_in_full() {
    let fake = Fake::start();
    let client = fake.client().build().unwrap();
    let samples: Vec<Value> = (0..5000)
        .map(|index| json!({"metric": {"pod": format!("web-{index}")}, "value": [1, "1"]}))
        .collect();
    fake.success(json!({"resultType": "vector", "result": samples}));
    let samples = client
        .query("up", at())
        .unwrap()
        .data
        .into_vector()
        .unwrap();
    assert_eq!(samples.len(), 5000);
    assert_eq!(samples[4999].metric["pod"], "web-4999");
    fake.next();
}

#[test]
fn bounds_refuse_a_query_before_it_is_sent() {
    let fake = Fake::start();
    let client = fake.client().max_query_bytes(8).build().unwrap();
    assert_eq!(
        client.query("123456789", at()).unwrap_err(),
        Error::Limit("a query of 9 bytes is more than 8".into())
    );
    let second = Duration::from_secs(1);
    assert!(matches!(
        client.query_range("up", Range::new(at(), at() - second, second)),
        Err(Error::Invalid(_))
    ));
    assert_eq!(
        client
            .query_range("up", Range::new(at() - second * 11_000, at(), second))
            .unwrap_err(),
        Error::Limit("a range of 11001 points is more than 11000".into())
    );
    assert!(fake.seen.try_recv().is_err());
}

#[test]
fn a_refused_token_is_renewed_once_and_then_kept() {
    let fake = Fake::start();
    let asked = Arc::new(AtomicUsize::new(0));
    let counter = asked.clone();
    let client = fake
        .client()
        .token_source(move || {
            let count = counter.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(Token::new(&format!("token-{count}")).unwrap())
        })
        .build()
        .unwrap();
    fake.reply(401, "", "expired");
    fake.success(vector());
    client.query("up", at()).unwrap();
    assert_eq!(fake.next().header("authorization"), Some("Bearer token-1"));
    assert_eq!(fake.next().header("authorization"), Some("Bearer token-2"));
    fake.success(vector());
    client.query("up", at()).unwrap();
    assert_eq!(fake.next().header("authorization"), Some("Bearer token-2"));
    assert_eq!(asked.load(Ordering::SeqCst), 2);
}

#[test]
fn a_token_refused_twice_fails_without_being_quoted() {
    let fake = Fake::start();
    let client = fake
        .client()
        .token_source(Token::new("s3cret").unwrap())
        .build()
        .unwrap();
    fake.reply(401, "", "bad token s3cret");
    fake.reply(401, "", "bad token s3cret");
    let error = client.query("up", at()).unwrap_err();
    assert_eq!(
        error,
        Error::Auth("Prometheus refused the request with HTTP 401".into())
    );
    fake.next();
    fake.next();
    assert!(fake.seen.try_recv().is_err());

    // A 403 is not retried.
    fake.reply(403, "", "forbidden");
    assert_eq!(
        client.query("up", at()).unwrap_err(),
        Error::Auth("Prometheus refused the request with HTTP 403".into())
    );
    fake.next();
    assert!(fake.seen.try_recv().is_err());
}

#[test]
fn without_a_token_source_a_401_says_so() {
    let fake = Fake::start();
    let client = fake.client().build().unwrap();
    fake.reply(401, "", "");
    assert_eq!(
        client.query("up", at()).unwrap_err().to_string(),
        "Prometheus authentication failed: Prometheus refused the request with HTTP 401, and the client has no token source"
    );
    fake.next();
}

#[test]
fn a_token_is_asked_for_again_once_its_lifetime_has_passed() {
    let fake = Fake::start();
    let asked = Arc::new(AtomicUsize::new(0));
    let counter = asked.clone();
    let client = fake
        .client()
        .token_lifetime(Duration::ZERO)
        .token_source(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(Token::new("t").unwrap())
        })
        .build()
        .unwrap();
    for _ in 0..2 {
        fake.success(vector());
        client.query("up", at()).unwrap();
        fake.next();
    }
    assert_eq!(asked.load(Ordering::SeqCst), 2);

    let failing = fake
        .client()
        .token_source(|| Err("run `gcloud auth login`".to_owned()))
        .build()
        .unwrap();
    assert_eq!(
        failing.query("up", at()).unwrap_err(),
        Error::Auth("run `gcloud auth login`".into())
    );
    assert!(fake.seen.try_recv().is_err());
}

#[test]
fn busy_and_failing_sources_are_asked_once() {
    let fake = Fake::start();
    let client = fake
        .client()
        .token_source(Token::new("s3cret").unwrap())
        .build()
        .unwrap();
    fake.reply(429, "retry-after: 7\r\n", "slow down");
    assert_eq!(
        client.query("up", at()).unwrap_err(),
        Error::Status {
            status: 429,
            retry_after: Some(Duration::from_secs(7)),
            detail: "slow down".into(),
        }
    );
    fake.next();
    fake.reply(
        503,
        "retry-after: soon\r\n",
        "<html>unavailable for s3cret</html>",
    );
    assert_eq!(
        client.query("up", at()).unwrap_err(),
        Error::Status {
            status: 503,
            retry_after: None,
            detail: "<html>unavailable for [redacted]</html>".into(),
        }
    );
    fake.next();
    assert!(fake.seen.try_recv().is_err());
}

#[test]
fn prometheus_errors_are_typed_and_never_quote_the_token() {
    let fake = Fake::start();
    let client = fake
        .client()
        .token_source(Token::new("s3cret").unwrap())
        .build()
        .unwrap();
    let refused = json!({"status": "error", "errorType": "bad_data", "error": "unknown function near \"s3cret\""});
    fake.reply(400, "", &refused.to_string());
    let error = client.query("s3cret(up)", at()).unwrap_err();
    assert_eq!(
        error,
        Error::Query {
            status: 400,
            retry_after: None,
            error_type: "bad_data".into(),
            message: "unknown function near \"[redacted]\"".into(),
        }
    );
    assert!(!format!("{error} {error:?}").contains("s3cret"));
    fake.next();

    fake.reply(200, "", "<html>a login page</html>");
    assert_eq!(
        client.query("up", at()).unwrap_err(),
        Error::Parse("it is not JSON".into())
    );
    fake.next();
}

#[test]
fn labels_are_filtered_by_the_client() {
    let fake = Fake::start();
    let client = fake
        .client()
        .labels(LabelFilter::only(["pod"]))
        .build()
        .unwrap();
    fake.success(vector());
    let samples = client
        .query("up", at())
        .unwrap()
        .data
        .into_vector()
        .unwrap();
    fake.next();
    assert_eq!(
        samples[0].metric,
        Labels::from([("pod".into(), "web-1".into())])
    );
    assert_eq!(
        samples[1].metric,
        Labels::from([("pod".into(), "web-2".into())])
    );
}

#[test]
fn a_call_s_own_label_filter_replaces_the_client_s() {
    let fake = Fake::start();
    let client = fake
        .client()
        .labels(LabelFilter::only(["pod"]))
        .build()
        .unwrap();
    fake.success(vector());
    let call = Call::new().labels(LabelFilter::only(["url"]));
    let samples = client
        .query_with("up", at(), call.clone())
        .unwrap()
        .data
        .into_vector()
        .unwrap();
    fake.next();
    assert_eq!(
        samples[0].metric,
        Labels::from([("url".into(), "https://x/?key=k".into())])
    );
    assert_eq!(samples[1].metric, Labels::new());
    // A range query keeps the call's labels too, and the client's filter is kept for calls without their own.
    fake.success(json!({"resultType": "matrix", "result": [
        {"metric": {"pod": "web-1", "url": "u"}, "values": [[1700000000.25, "1"]]},
    ]}));
    let range = Range::new(at(), at(), Duration::from_secs(1));
    let series = client.query_range_with("up", range, call).unwrap().data;
    fake.next();
    assert_eq!(series[0].metric, Labels::from([("url".into(), "u".into())]));
    fake.success(vector());
    let samples = client
        .query("up", at())
        .unwrap()
        .data
        .into_vector()
        .unwrap();
    fake.next();
    assert_eq!(
        samples[0].metric,
        Labels::from([("pod".into(), "web-1".into())])
    );
}

#[test]
fn a_large_answer_is_refused_and_the_next_call_succeeds() {
    let fake = Fake::start();
    let client = fake.client().max_response_bytes(100).build().unwrap();
    fake.reply(200, "", &"x".repeat(1000));
    assert_eq!(
        client.query("up", at()).unwrap_err(),
        Error::Limit("the answer is larger than 100 bytes".into())
    );
    fake.next();
    fake.success(json!({"resultType": "vector", "result": []}));
    client.query("up", at()).unwrap();
    fake.next();
}

#[test]
fn a_slow_source_times_out_and_the_next_call_succeeds() {
    let fake = Fake::start();
    let client = fake
        .client()
        .timeout(Duration::from_millis(200))
        .build()
        .unwrap();
    assert_eq!(
        client.query("up", at()).unwrap_err(),
        Error::Timeout(Duration::from_millis(200))
    );
    let first = fake.next().connection;
    // Release the fake's first connection, which the client has given up on.
    fake.reply(200, "", "late");
    fake.success(json!({"resultType": "vector", "result": []}));
    client.query("up", at()).unwrap();
    assert_ne!(fake.next().connection, first);
}

#[test]
fn a_broken_answer_is_a_transport_failure() {
    let fake = Fake::start();
    let client = fake.client().build().unwrap();
    fake.replies.send(Reply::Truncated).unwrap();
    let error = client.query("up", at()).unwrap_err();
    assert!(
        matches!(&error, Error::Transport(message) if message.starts_with("could not read the answer")),
        "{error}"
    );
    fake.next();
}

#[test]
fn unreachable_sources_are_transport_failures() {
    let address = refusing();
    let client = Client::builder(&format!("http://{address}"))
        .build()
        .unwrap();
    let error = client.query("up", at()).unwrap_err();
    assert!(
        matches!(&error, Error::Transport(message) if message.starts_with("could not connect")),
        "{error}"
    );
    assert!(!error.to_string().contains(&address.to_string()));

    let mut builder = Client::builder("https://prometheus.example");
    builder.resolver = Some(Arc::new(Vec::new));
    let client = builder.build().unwrap();
    assert_eq!(
        client.query("up", at()).unwrap_err(),
        Error::Transport("could not resolve the endpoint's host".into())
    );

    // A host that hangs up on the TLS handshake.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || drop(listener.accept()));
    let client = Client::builder(&format!("https://{address}"))
        .build()
        .unwrap();
    assert!(matches!(
        client.query("up", at()).unwrap_err(),
        Error::Transport(_)
    ));
}

#[test]
fn a_configuration_that_cannot_be_used_is_refused() {
    let builder = || Client::builder("http://127.0.0.1:9");
    for (builder, why) in [
        (
            builder().timeout(Duration::ZERO),
            "the time-out, concurrency and limits must be more than zero",
        ),
        (
            builder().max_response_bytes(0),
            "the time-out, concurrency and limits must be more than zero",
        ),
        (
            builder().max_query_bytes(0),
            "the time-out, concurrency and limits must be more than zero",
        ),
        (
            builder().concurrency(0),
            "the time-out, concurrency and limits must be more than zero",
        ),
        (
            builder().user_agent(""),
            "the user agent must be printable ASCII",
        ),
        (
            builder().user_agent("a\r\nb"),
            "the user agent must be printable ASCII",
        ),
    ] {
        assert_eq!(builder.build().unwrap_err(), Error::Invalid(why.into()));
    }
    assert!(matches!(
        builder().api_prefix("v1").build(),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        Client::builder("ftp://x").build(),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn a_system_out_of_threads_is_reported() {
    system::refuse("prometheus scheduler");
    let error = Client::builder("http://127.0.0.1:9").build().unwrap_err();
    assert!(
        matches!(&error, Error::Transport(message) if message.starts_with("could not start a scheduler"))
    );
    system::refuse("prometheus thread");
    let error = Client::builder("http://127.0.0.1:9").build().unwrap_err();
    assert!(
        matches!(&error, Error::Transport(message) if message.starts_with("could not start a thread"))
    );
}

#[test]
fn prometheus_errors_keep_their_status_and_retry_after() {
    let fake = Fake::start();
    let client = fake.client().build().unwrap();
    for (status, error_type) in [
        (503, "unavailable"),
        (429, "too_many_requests"),
        (422, "execution"),
    ] {
        let body = json!({"status": "error", "errorType": error_type, "error": "busy"});
        fake.reply(status, "retry-after: 5\r\n", &body.to_string());
        let error = client.query("up", at()).unwrap_err();
        assert_eq!(
            error,
            Error::Query {
                status,
                retry_after: Some(Duration::from_secs(5)),
                error_type: error_type.into(),
                message: "busy".into(),
            }
        );
        assert_eq!(error.status(), Some(status));
        fake.next();
    }
    fake.reply(
        503,
        "retry-after: Thu, 01 Jan 1970 00:00:00 GMT\r\n",
        "down",
    );
    let error = client.query("up", at()).unwrap_err();
    assert_eq!(error.retry_after(), Some(Duration::ZERO));
    fake.next();
}

#[test]
fn redirects_are_not_followed_and_series_stay_apart() {
    let fake = Fake::start();
    let client = fake
        .client()
        .token_source(Token::new("s3cret").unwrap())
        .labels(LabelFilter::only(["job"]))
        .build()
        .unwrap();
    let location: &'static str =
        Box::leak(format!("location: {}/elsewhere\r\n", fake.url).into_boxed_str());
    fake.reply(302, location, "");
    let error = client.query("up", at()).unwrap_err();
    assert_eq!(error.status(), Some(302));
    fake.next();
    assert!(fake.seen.try_recv().is_err());

    fake.success(json!({"resultType": "vector", "result": [
        {"metric": {"job": "web", "pod": "web-1"}, "value": [1, "2"]},
        {"metric": {"job": "web", "pod": "web-2"}, "value": [1, "0"]},
    ]}));
    let samples = client
        .query("up", at())
        .unwrap()
        .data
        .into_vector()
        .unwrap();
    fake.next();
    assert_eq!(samples.len(), 2);
    assert_eq!(samples[0].metric, samples[1].metric);
    assert_eq!((samples[0].point.value, samples[1].point.value), (2.0, 0.0));
}

#[test]
fn a_connection_the_source_closes_is_replaced() {
    let fake = Fake::start();
    let client = fake.client().build().unwrap();
    fake.reply(
        200,
        "connection: close\r\n",
        &json!({"status": "success", "data": {"resultType": "vector", "result": []}}).to_string(),
    );
    client.query("up", at()).unwrap();
    let first = fake.next().connection;
    fake.success(json!({"resultType": "vector", "result": []}));
    client.query("up", at()).unwrap();
    assert_ne!(fake.next().connection, first);
}

#[test]
fn each_address_is_tried_until_one_connects_and_is_kept() {
    let fake = Fake::start();
    let refusing = refusing();
    let listening: std::net::SocketAddr = fake.url.trim_start_matches("http://").parse().unwrap();
    let resolved = Arc::new(AtomicUsize::new(0));
    let counter = resolved.clone();
    let mut builder = Client::builder("http://localhost");
    builder.resolver = Some(Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        vec![refusing, listening]
    }));
    let client = builder.build().unwrap();
    for _ in 0..2 {
        fake.success(json!({"resultType": "vector", "result": []}));
        client.query("up", at()).unwrap();
        assert_eq!(fake.next().connection, 0);
    }
    assert_eq!(resolved.load(Ordering::SeqCst), 1);
}

#[test]
fn requests_whose_callers_gave_up_are_skipped_without_losing_the_connection() {
    let fake = Fake::start();
    let client = fake.client().build().unwrap();
    let abandoned = || {
        let (reply, _) = sync_channel(1);
        let cancellation = Cancellation::new();
        cancellation.cancel();
        let request = Outgoing {
            post: true,
            target: "/abandoned".into(),
            body: Vec::new(),
            authorization: None,
        };
        client
            .shared
            .jobs
            .send_blocking(Job {
                request,
                slot: client.shared.slots.take(false, Duration::ZERO).unwrap(),
                cancellation,
                deadline: std::time::Instant::now(),
                reply,
            })
            .unwrap();
    };
    abandoned();
    for _ in 0..2 {
        fake.success(json!({"resultType": "vector", "result": []}));
        client.query("up", at()).unwrap();
        let seen = fake.next();
        assert_eq!(
            (seen.target.as_str(), seen.connection),
            ("/api/v1/query", 0)
        );
        abandoned();
    }
    assert!(fake.seen.try_recv().is_err());
}

#[test]
fn a_token_refused_but_since_renewed_is_not_renewed_again() {
    let asked = Arc::new(AtomicUsize::new(0));
    let counter = asked.clone();
    let client = Client::builder("http://127.0.0.1:9")
        .token_source(move || {
            let count = counter.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(Token::new(&format!("token-{count}")).unwrap())
        })
        .build()
        .unwrap();
    let (first, generation) = client.token(None).unwrap().unwrap();
    assert_eq!((first.expose(), generation), ("token-1", 1));
    // This caller's request was refused, so it renews.
    let (second, renewed) = client.token(Some(generation)).unwrap().unwrap();
    assert_eq!((second.expose(), renewed), ("token-2", 2));
    // Another caller's request, sent with the first token, was refused too: it takes the renewed token.
    let (third, kept) = client.token(Some(generation)).unwrap().unwrap();
    assert_eq!((third.expose(), kept), ("token-2", 2));
    assert_eq!(asked.load(Ordering::SeqCst), 2);
}

#[test]
fn dropping_a_client_stuck_on_its_resolver_is_bounded() {
    let (entered, stuck) = channel();
    let (release, wait) = channel::<()>();
    let wait = Mutex::new(wait);
    let mut builder = Client::builder("http://localhost:9").timeout(Duration::from_millis(100));
    builder.drop_wait = Duration::from_millis(100);
    builder.resolver = Some(Arc::new(move || {
        entered.send(()).unwrap();
        let _ = wait.lock().unwrap().recv();
        Vec::new()
    }));
    let client = builder.build().unwrap();
    assert_eq!(
        client.query("up", at()).unwrap_err(),
        Error::Timeout(Duration::from_millis(100))
    );
    stuck.recv().unwrap();
    // Without the bound, this would wait for the resolver for ever.
    drop(client);
    drop(release);
}

#[test]
fn a_panic_in_the_clients_thread_fails_only_its_request() {
    let fake = Fake::start();
    let client = fake.client().api_prefix("/panic").build().unwrap();
    assert_eq!(
        client.query("up", at()).unwrap_err(),
        Error::Transport("the client's thread failed while handling the request".into())
    );
    // The thread still serves.
    let (reply, answer) = sync_channel(1);
    let request = Outgoing {
        post: true,
        target: "/api/v1/query".into(),
        body: b"query=up".to_vec(),
        authorization: None,
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let job = Job {
        request,
        slot: client.shared.slots.take(false, Duration::ZERO).unwrap(),
        cancellation: Cancellation::new(),
        deadline,
        reply,
    };
    client.shared.jobs.send_blocking(job).unwrap();
    fake.success(json!({"resultType": "vector", "result": []}));
    assert_eq!(answer.recv().unwrap().ok().unwrap().status, 200);
    fake.next();
}

#[test]
fn a_client_whose_thread_has_stopped_says_so() {
    let client = Client::builder("http://127.0.0.1:9").build().unwrap();
    client.shared.jobs.close();
    assert_eq!(
        client.query("up", at()).unwrap_err(),
        Error::Transport("the client's thread has stopped".into())
    );
}

#[test]
fn an_address_that_connects_but_does_not_answer_is_not_skipped() {
    let silent = Fake::start();
    let other = Fake::start();
    let address = |fake: &Fake| -> std::net::SocketAddr {
        fake.url.trim_start_matches("http://").parse().unwrap()
    };
    let (first, second) = (address(&silent), address(&other));
    let mut builder = Client::builder("http://localhost").timeout(Duration::from_millis(600));
    builder.resolver = Some(Arc::new(move || vec![first, second]));
    let client = builder.build().unwrap();
    assert_eq!(
        client.query("up", at()).unwrap_err(),
        Error::Timeout(Duration::from_millis(600))
    );
    silent.next();
    assert!(other.seen.try_recv().is_err());
    // Release the silent fake's connection.
    silent.reply(200, "", "late");
}

/// A certificate for `localhost`, and the client's roots that trust only it.
fn certificate() -> (Arc<rustls::ServerConfig>, Arc<rustls::ClientConfig>) {
    use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let issued = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let certificate = issued.cert.der().clone();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(issued.key_pair.serialize_der()));
    let server = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![certificate.clone()], key)
        .unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(certificate).unwrap();
    let client = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    (Arc::new(server), Arc::new(client))
}

/// A loopback address that accepts TCP connections and then never completes the TLS handshake, as a host whose packets are lost never completes connecting.
fn stalling() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        let held: Vec<TcpStream> = listener.incoming().map(Result::unwrap).collect();
        drop(held);
    });
    address
}

/// An HTTPS fake that answers each connection's request with an empty vector, closing it after; its first answer to a whole request is cut short, failing the request after it connected. A connection that sends no whole request, such as another test's that found this port free when it expected it closed, is not counted.
fn answering(config: Arc<rustls::ServerConfig>) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        let mut answered = 0;
        for stream in listener.incoming() {
            let connection = rustls::ServerConnection::new(config.clone()).unwrap();
            let mut tls = rustls::StreamOwned::new(connection, stream.unwrap());
            let mut head = Vec::new();
            let mut byte = [0];
            while !head.ends_with(b"\r\n\r\n") && tls.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
            }
            let complete = head.ends_with(b"\r\n\r\n");
            let head = String::from_utf8_lossy(&head).to_ascii_lowercase();
            let length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .map_or(0, |value| value.trim().parse().unwrap());
            let mut body = vec![0; length];
            let _ = tls.read_exact(&mut body);
            let answer =
                json!({"status": "success", "data": {"resultType": "vector", "result": []}})
                    .to_string();
            let promised = if complete && answered == 0 {
                1000
            } else {
                answer.len()
            };
            answered += usize::from(complete);
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {promised}\r\nconnection: close\r\n\r\n{answer}"
            );
            let _ = tls.write_all(response.as_bytes());
            let _ = tls.flush();
            tls.conn.send_close_notify();
            let _ = tls.flush();
        }
    });
    address
}

#[test]
fn an_address_that_never_connects_is_skipped_within_the_time_out_and_the_one_that_did_is_tried_first()
 {
    let (server, roots) = certificate();
    let (stall, working) = (stalling(), answering(server));
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let mut builder = Client::builder("https://localhost").timeout(Duration::from_secs(4));
    builder.resolver = Some(Arc::new(move || vec![stall, working]));
    builder.attempts = Some(attempts.clone());
    builder.tls = Some(roots);
    let client = builder.build().unwrap();
    // The stalling address had half the time, and the working one connected within the rest, though its answer was cut short.
    let error = client.query("up", at()).unwrap_err();
    assert!(
        matches!(&error, Error::Transport(message) if message.starts_with("could not read the answer")),
        "{error}"
    );
    assert_eq!(*attempts.lock().unwrap(), [stall, working]);
    // The failure retired only its connection, so the next request opens another to the address that connected, without trying the stalling one again.
    client.query("up", at()).unwrap();
    assert_eq!(*attempts.lock().unwrap(), [stall, working]);
}

#[test]
fn a_time_out_longer_than_a_day_is_taken_as_a_day() {
    let fake = Fake::start();
    for timeout in [Duration::MAX, MAX_TIMEOUT + Duration::from_millis(1)] {
        let client = fake.client().timeout(timeout).build().unwrap();
        assert_eq!(client.shared.timeout, MAX_TIMEOUT);
        fake.success(json!({"resultType": "vector", "result": []}));
        client.query("up", at()).unwrap();
        let seen = fake.next();
        assert!(form(&seen.body).contains(&("timeout".to_owned(), "86400.000".to_owned())));
    }
}

#[test]
fn an_address_still_connecting_when_its_caller_gives_up_is_not_preferred() {
    let (_, roots) = certificate();
    let refusing = refusing();
    let stall = stalling();
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let mut builder = Client::builder("https://localhost").timeout(Duration::from_millis(300));
    builder.resolver = Some(Arc::new(move || vec![refusing, stall]));
    builder.attempts = Some(attempts.clone());
    builder.tls = Some(roots);
    let client = builder.build().unwrap();
    for _ in 0..2 {
        assert_eq!(
            client.query("up", at()).unwrap_err(),
            Error::Timeout(Duration::from_millis(300))
        );
    }
    // Dropping the client waits for its thread, so every attempt has been recorded.
    drop(client);
    assert_eq!(
        *attempts.lock().unwrap(),
        [refusing, stall, refusing, stall]
    );
}

/// How the scripted fake answers a request.
#[derive(Clone, Copy)]
enum Answer {
    /// An empty vector, after a wait.
    Success(Duration),
    /// A status with an empty body, after a wait.
    Status(u16, Duration),
    /// Promise a longer body than is sent, then hang up.
    Truncated,
}

/// A request the scripted fake received: its connection, its expression and its `authorization` field.
#[derive(Clone, Debug)]
struct Heard {
    connection: usize,
    query: String,
    authorization: Option<String>,
}

/// A fake Prometheus that answers every request at once, on each connection's own thread, as a script says, counting the requests in flight together.
struct Scripted {
    url: String,
    heard: Arc<Mutex<Vec<Heard>>>,
    in_flight: Arc<AtomicUsize>,
    most: Arc<AtomicUsize>,
    /// Connections the client has closed.
    closed: Arc<AtomicUsize>,
}

impl Scripted {
    fn start(script: impl Fn(&Heard) -> Answer + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let fake = Self {
            url,
            heard: Arc::default(),
            in_flight: Arc::default(),
            most: Arc::default(),
            closed: Arc::default(),
        };
        let script = Arc::new(script);
        let (heard, in_flight, most, closed) = (
            fake.heard.clone(),
            fake.in_flight.clone(),
            fake.most.clone(),
            fake.closed.clone(),
        );
        thread::spawn(move || {
            for (connection, stream) in listener.incoming().enumerate() {
                let (script, heard, in_flight, most, closed) = (
                    script.clone(),
                    heard.clone(),
                    in_flight.clone(),
                    most.clone(),
                    closed.clone(),
                );
                thread::spawn(move || {
                    let stream = stream.unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut stream = stream;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 {
                            closed.fetch_add(1, Ordering::SeqCst);
                            return;
                        }
                        let (mut length, mut authorization) = (0, None);
                        loop {
                            let mut header = String::new();
                            reader.read_line(&mut header).unwrap();
                            let Some((name, value)) = header.trim_end().split_once(':') else {
                                break;
                            };
                            match name.to_ascii_lowercase().as_str() {
                                "content-length" => length = value.trim().parse().unwrap(),
                                "authorization" => authorization = Some(value.trim().to_owned()),
                                _ => {}
                            }
                        }
                        let mut body = vec![0; length];
                        reader.read_exact(&mut body).unwrap();
                        let query = form(&String::from_utf8(body).unwrap())
                            .into_iter()
                            .find(|(name, _)| name == "query")
                            .map(|(_, value)| value)
                            .unwrap_or_default();
                        let request = Heard {
                            connection,
                            query,
                            authorization,
                        };
                        heard.lock().unwrap().push(request.clone());
                        let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                        most.fetch_max(now, Ordering::SeqCst);
                        let answer = script(&request);
                        let (status, wait, promised, body) = match answer {
                            Answer::Success(wait) => {
                                let body = json!({"status": "success", "data": {"resultType": "vector", "result": []}}).to_string();
                                (200, wait, body.len(), body)
                            }
                            Answer::Status(status, wait) => (status, wait, 0, String::new()),
                            Answer::Truncated => (200, Duration::ZERO, 100, "{}".into()),
                        };
                        thread::sleep(wait);
                        in_flight.fetch_sub(1, Ordering::SeqCst);
                        let head =
                            format!("HTTP/1.1 {status} Fake\r\ncontent-length: {promised}\r\n\r\n");
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(body.as_bytes());
                        if promised != body.len() {
                            return;
                        }
                    }
                });
            }
        });
        fake
    }

    fn heard(&self) -> Vec<Heard> {
        self.heard.lock().unwrap().clone()
    }

    /// Wait until the fake has heard `count` requests.
    fn wait_for(&self, count: usize) {
        while self.heard.lock().unwrap().len() < count {
            thread::sleep(Duration::from_millis(5));
        }
    }
}

/// `slow` queries are answered after `wait`, any others at once.
fn slow_and_fast(wait: Duration) -> impl Fn(&Heard) -> Answer + Send + Sync + 'static {
    move |heard| {
        if heard.query.starts_with("slow") {
            Answer::Success(wait)
        } else {
            Answer::Success(Duration::ZERO)
        }
    }
}

/// Run `query` on its own thread with a clone of `client`, for how long it took and what it gave.
fn spawn_query(
    client: &Client,
    query: &'static str,
    call: Call,
) -> thread::JoinHandle<(Duration, Result<Response<Data>, Error>)> {
    let client = client.clone();
    thread::spawn(move || {
        let started = std::time::Instant::now();
        let result = client.query_with(query, at(), call);
        (started.elapsed(), result)
    })
}

#[test]
fn two_slow_requests_finish_together_over_connections_of_their_own() {
    let slow = Duration::from_millis(600);
    let fake = Scripted::start(slow_and_fast(slow));
    let client = Client::builder(&fake.url).concurrency(2).build().unwrap();
    let started = std::time::Instant::now();
    let requests = [
        spawn_query(&client, "slow1", Call::new()),
        spawn_query(&client, "slow2", Call::new()),
    ];
    for request in requests {
        assert!(request.join().unwrap().1.is_ok());
    }
    let elapsed = started.elapsed();
    assert!(elapsed >= slow && elapsed < slow * 2, "{elapsed:?}");
    let heard = fake.heard();
    assert_ne!(heard[0].connection, heard[1].connection);
    assert_eq!(fake.most.load(Ordering::SeqCst), 2);
}

#[test]
fn no_more_requests_than_the_concurrency_are_in_flight() {
    let slow = Duration::from_millis(200);
    let fake = Scripted::start(slow_and_fast(slow));
    let client = Client::builder(&fake.url).concurrency(2).build().unwrap();
    let started = std::time::Instant::now();
    let requests: Vec<_> = (0..5)
        .map(|_| spawn_query(&client, "slow", Call::new()))
        .collect();
    for request in requests {
        assert!(request.join().unwrap().1.is_ok());
    }
    // Five requests, two at a time, take three turns.
    assert!(started.elapsed() >= slow * 3);
    assert_eq!(fake.most.load(Ordering::SeqCst), 2);
    // Connections are kept and reused, so no more are opened than were needed at once.
    let mut connections: Vec<_> = fake.heard().iter().map(|heard| heard.connection).collect();
    connections.sort();
    connections.dedup();
    assert_eq!(connections.len(), 2);
}

#[test]
fn a_slow_request_does_not_hold_up_a_fast_one() {
    let fake = Scripted::start(slow_and_fast(Duration::from_secs(3)));
    let client = Client::builder(&fake.url).concurrency(2).build().unwrap();
    let slow = spawn_query(&client, "slow", Call::new());
    fake.wait_for(1);
    let started = std::time::Instant::now();
    client.query("fast", at()).unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(slow.join().unwrap().1.is_ok());
}

#[test]
fn a_token_refused_to_requests_in_flight_together_is_renewed_once() {
    // No refusal is sent until all four requests have arrived with the first token, so that every one is refused while the others are in flight.
    let together = Arc::new(std::sync::Barrier::new(4));
    let fake = Scripted::start(move |heard| match heard.authorization.as_deref() {
        Some("Bearer token-1") => {
            together.wait();
            Answer::Status(401, Duration::ZERO)
        }
        _ => Answer::Success(Duration::ZERO),
    });
    let asked = Arc::new(AtomicUsize::new(0));
    let counter = asked.clone();
    let client = Client::builder(&fake.url)
        .token_source(move || {
            let count = counter.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(Token::new(&format!("token-{count}")).unwrap())
        })
        .build()
        .unwrap();
    client.token(None).unwrap();
    let requests: Vec<_> = (0..4)
        .map(|_| spawn_query(&client, "up", Call::new()))
        .collect();
    for request in requests {
        assert!(request.join().unwrap().1.is_ok());
    }
    assert_eq!(asked.load(Ordering::SeqCst), 2);
    let heard = fake.heard();
    let refused = heard
        .iter()
        .filter(|heard| heard.authorization.as_deref() == Some("Bearer token-1"))
        .count();
    assert_eq!((refused, heard.len()), (4, 8));
}

#[test]
fn a_failed_connection_is_retired_without_disturbing_the_others() {
    let fake = Scripted::start(|heard| match heard.query.as_str() {
        "slow" => Answer::Success(Duration::from_millis(500)),
        "broken" => Answer::Truncated,
        _ => Answer::Success(Duration::ZERO),
    });
    let client = Client::builder(&fake.url).concurrency(2).build().unwrap();
    let slow = spawn_query(&client, "slow", Call::new());
    fake.wait_for(1);
    let error = client.query("broken", at()).unwrap_err();
    assert!(
        matches!(&error, Error::Transport(message) if message.starts_with("could not read the answer")),
        "{error}"
    );
    assert!(slow.join().unwrap().1.is_ok());
    client.query("up", at()).unwrap();
    let heard = fake.heard();
    assert_ne!(heard[0].connection, heard[1].connection);
    // The next request reuses the slow request's connection, which the failure left alone.
    assert_eq!(heard[2].connection, heard[0].connection);
}

#[test]
fn waiting_for_a_slot_is_told_apart_from_the_source_not_answering() {
    let fake = Scripted::start(slow_and_fast(Duration::from_millis(800)));
    let client = Client::builder(&fake.url).concurrency(1).build().unwrap();
    let slow = spawn_query(&client, "slow", Call::new());
    fake.wait_for(1);
    let short = Duration::from_millis(100);
    assert_eq!(
        client
            .query_with("fast", at(), Call::new().timeout(short))
            .unwrap_err(),
        Error::Queued(short)
    );
    assert!(slow.join().unwrap().1.is_ok());
    // With a slot, the same time-out is the source's to keep.
    assert_eq!(
        client
            .query_with("slow", at(), Call::new().timeout(short))
            .unwrap_err(),
        Error::Timeout(short)
    );
    assert_eq!(fake.heard().len(), 2);
}

#[test]
fn background_requests_leave_a_slot_for_others() {
    let fake = Scripted::start(slow_and_fast(Duration::from_millis(800)));
    let client = Client::builder(&fake.url).concurrency(2).build().unwrap();
    let background = Call::new().background();
    let slow = spawn_query(&client, "slow", background.clone());
    fake.wait_for(1);
    let short = Duration::from_millis(100);
    assert_eq!(
        client
            .query_with("slow", at(), background.timeout(short))
            .unwrap_err(),
        Error::Queued(short)
    );
    let started = std::time::Instant::now();
    client.query("fast", at()).unwrap();
    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(slow.join().unwrap().1.is_ok());
}

#[test]
fn a_call_may_have_a_time_out_of_its_own() {
    let fake = Fake::start();
    let client = fake.client().build().unwrap();
    let call = Call::new().timeout(Duration::from_millis(1500));
    fake.success(json!({"resultType": "matrix", "result": []}));
    let range = Range::new(
        at() - Duration::from_secs(60),
        at(),
        Duration::from_secs(60),
    );
    client.query_range_with("up", range, call).unwrap();
    assert!(form(&fake.next().body).contains(&("timeout".to_owned(), "1.500".to_owned())));
    fake.success(vector());
    let long = Call::new().timeout(Duration::MAX);
    client.query_with("up", at(), long).unwrap();
    assert!(form(&fake.next().body).contains(&("timeout".to_owned(), "86400.000".to_owned())));
    assert_eq!(
        client
            .query_with("up", at(), Call::new().timeout(Duration::ZERO))
            .unwrap_err(),
        Error::Invalid("a time-out must be more than zero".into())
    );
    assert!(fake.seen.try_recv().is_err());
}

#[test]
fn dropping_a_client_with_requests_in_flight_stops_it_promptly() {
    let fake = Scripted::start(slow_and_fast(Duration::from_secs(1)));
    let mut builder = Client::builder(&fake.url).concurrency(3);
    builder.drop_wait = Duration::from_secs(30);
    let client = builder.build().unwrap();
    let short = Call::new().timeout(Duration::from_millis(200));
    let requests: Vec<_> = (0..3)
        .map(|_| spawn_query(&client, "slow", short.clone()))
        .collect();
    for request in requests {
        assert_eq!(
            request.join().unwrap().1.unwrap_err(),
            Error::Timeout(Duration::from_millis(200))
        );
    }
    let started = std::time::Instant::now();
    drop(client);
    // The thread stopped of its own accord, well within the wait.
    assert!(started.elapsed() < Duration::from_secs(2));
    // Every connection was closed, which the fake sees once it has answered.
    while fake.closed.load(Ordering::SeqCst) < 3 {
        assert!(started.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_millis(5));
    }
}

/// An HTTPS fake whose first `answered` connections have their requests answered with an empty vector, a `slow` query after 800 ms, and whose later connections never complete the handshake, as an address that stops connecting. With `keep_alive`, a connection answers request after request; otherwise it closes after its first. It records which connection each request came on.
fn tls_fake(
    config: Arc<rustls::ServerConfig>,
    answered: usize,
    keep_alive: bool,
) -> (std::net::SocketAddr, Arc<Mutex<Vec<usize>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let heard = Arc::new(Mutex::new(Vec::new()));
    let record = heard.clone();
    thread::spawn(move || {
        let mut held = Vec::new();
        for (index, stream) in listener.incoming().enumerate() {
            let stream = stream.unwrap();
            if index >= answered {
                held.push(stream);
                continue;
            }
            let (config, record) = (config.clone(), record.clone());
            thread::spawn(move || {
                let connection = rustls::ServerConnection::new(config).unwrap();
                let mut tls = rustls::StreamOwned::new(connection, stream);
                loop {
                    let mut reader = BufReader::new(&mut tls);
                    let mut length = 0;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        if let Some(value) =
                            line.to_ascii_lowercase().strip_prefix("content-length:")
                        {
                            length = value.trim().parse().unwrap();
                        }
                        if line == "\r\n" {
                            break;
                        }
                    }
                    let mut body = vec![0; length];
                    reader.read_exact(&mut body).unwrap();
                    record.lock().unwrap().push(index);
                    if String::from_utf8_lossy(&body).contains("query=slow") {
                        thread::sleep(Duration::from_millis(800));
                    }
                    let answer = json!({"status": "success", "data": {"resultType": "vector", "result": []}}).to_string();
                    let close = if keep_alive {
                        ""
                    } else {
                        "connection: close\r\n"
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n{close}\r\n{answer}",
                        answer.len()
                    );
                    let _ = tls.write_all(response.as_bytes());
                    if !keep_alive {
                        tls.conn.send_close_notify();
                        let _ = tls.flush();
                        return;
                    }
                    let _ = tls.flush();
                }
            });
        }
    });
    (address, heard)
}

/// An HTTPS fake whose first `answered` connections each answer one request and close, and whose later connections stall.
fn answering_then_stalling(
    config: Arc<rustls::ServerConfig>,
    answered: usize,
) -> std::net::SocketAddr {
    tls_fake(config, answered, false).0
}

#[test]
fn a_kept_address_that_stops_connecting_is_passed_over_for_another() {
    let (server, roots) = certificate();
    let (flaky, working) = (
        answering_then_stalling(server.clone(), 1),
        answering_then_stalling(server, usize::MAX),
    );
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let mut builder = Client::builder("https://localhost").timeout(Duration::from_secs(4));
    builder.resolver = Some(Arc::new(move || vec![flaky, working]));
    builder.attempts = Some(attempts.clone());
    builder.tls = Some(roots);
    let client = builder.build().unwrap();
    client.query("up", at()).unwrap();
    assert_eq!(*attempts.lock().unwrap(), [flaky]);
    // A new connection to the kept address stalls, so within its share of the time the request finds the other, trying the stalled one last.
    let started = std::time::Instant::now();
    client.query("up", at()).unwrap();
    assert!(started.elapsed() < Duration::from_secs(4));
    assert_eq!(*attempts.lock().unwrap(), [flaky, working]);
    client.query("up", at()).unwrap();
    assert_eq!(*attempts.lock().unwrap(), [flaky, working]);
}

#[test]
fn a_caller_giving_up_on_a_new_connection_leaves_the_kept_address_and_its_other_connections() {
    let (server, roots) = certificate();
    let (only, heard) = tls_fake(server, 1, true);
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let mut builder = Client::builder("https://localhost").concurrency(2);
    builder.resolver = Some(Arc::new(move || vec![only]));
    builder.attempts = Some(attempts.clone());
    builder.tls = Some(roots);
    let client = builder.build().unwrap();
    let slow = spawn_query(&client, "slow", Call::new());
    while heard.lock().unwrap().is_empty() {
        thread::sleep(Duration::from_millis(5));
    }
    // The first connection is busy, and a second never completes connecting: the host's only address is waited for, as long as the caller waits.
    let short = Duration::from_millis(200);
    assert_eq!(
        client
            .query_with("up", at(), Call::new().timeout(short))
            .unwrap_err(),
        Error::Timeout(short)
    );
    assert!(slow.join().unwrap().1.is_ok());
    // Only the connection being made was given up: the next request reuses the first, over the address kept.
    client.query("up", at()).unwrap();
    assert_eq!(*heard.lock().unwrap(), [0, 0]);
    drop(client);
    assert_eq!(*attempts.lock().unwrap(), [only]);
}

#[test]
fn a_request_waiting_for_another_to_find_an_address_can_be_given_up() {
    let (_, roots) = certificate();
    let stall = stalling();
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let mut builder = Client::builder("https://localhost").timeout(Duration::from_secs(1));
    builder.resolver = Some(Arc::new(move || vec![stall]));
    builder.attempts = Some(attempts.clone());
    builder.tls = Some(roots);
    let client = builder.build().unwrap();
    let finding = spawn_query(&client, "up", Call::new());
    while attempts.lock().unwrap().is_empty() {
        thread::sleep(Duration::from_millis(5));
    }
    let short = Duration::from_millis(100);
    assert_eq!(
        client
            .query_with("up", at(), Call::new().timeout(short))
            .unwrap_err(),
        Error::Timeout(short)
    );
    assert_eq!(
        finding.join().unwrap().1.unwrap_err(),
        Error::Timeout(Duration::from_secs(1))
    );
    drop(client);
    // The request that gave up never tried an address of its own.
    assert_eq!(*attempts.lock().unwrap(), [stall]);
}

#[test]
fn requests_together_on_a_kept_address_that_stops_connecting_forget_it_once() {
    let (server, roots) = certificate();
    let (flaky, refusing) = (answering_then_stalling(server, 1), refusing());
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let mut builder = Client::builder("https://localhost")
        .timeout(Duration::from_secs(2))
        .concurrency(2);
    builder.resolver = Some(Arc::new(move || vec![flaky, refusing]));
    builder.attempts = Some(attempts.clone());
    builder.tls = Some(roots);
    let client = builder.build().unwrap();
    client.query("up", at()).unwrap();
    // Both stall connecting to the kept address and pass it over; the first to find the addresses again finds the other refusing and the kept one still stalling, and the second waits for it.
    let requests = [
        spawn_query(&client, "up", Call::new()),
        spawn_query(&client, "up", Call::new()),
    ];
    for request in requests {
        assert_eq!(
            request.join().unwrap().1.unwrap_err(),
            Error::Timeout(Duration::from_secs(2))
        );
    }
    drop(client);
    let attempts = attempts.lock().unwrap();
    assert_eq!(attempts[..3], [flaky, refusing, flaky]);
}

#[test]
fn a_renewal_holds_up_only_callers_that_need_its_token() {
    let (entered, renewing) = channel();
    let (release, released) = channel::<()>();
    let released = Mutex::new(released);
    let asked = Arc::new(AtomicUsize::new(0));
    let counter = asked.clone();
    let client = Client::builder("http://127.0.0.1:9")
        .token_source(move || {
            let count = counter.fetch_add(1, Ordering::SeqCst) + 1;
            if count == 2 {
                entered.send(()).unwrap();
                released.lock().unwrap().recv().unwrap();
            }
            Ok(Token::new(&format!("token-{count}")).unwrap())
        })
        .build()
        .unwrap();
    assert_eq!(client.token(None).unwrap().unwrap().1, 1);
    // A caller whose request was refused renews, and the source takes its time.
    let renewer = {
        let client = client.clone();
        thread::spawn(move || client.token(Some(1)).unwrap().unwrap())
    };
    renewing.recv().unwrap();
    // A caller whose token is still good is not held up meanwhile.
    let (token, generation) = client.token(None).unwrap().unwrap();
    assert_eq!((token.expose(), generation), ("token-1", 1));
    // Another refused caller waits for the renewal rather than asking again.
    let waiter = {
        let client = client.clone();
        thread::spawn(move || client.token(Some(1)).unwrap().unwrap())
    };
    while client.shared.kept().waiting == 0 {
        thread::sleep(Duration::from_millis(1));
    }
    release.send(()).unwrap();
    for caller in [renewer, waiter] {
        let (token, generation) = caller.join().unwrap();
        assert_eq!((token.expose(), generation), ("token-2", 2));
    }
    assert_eq!(asked.load(Ordering::SeqCst), 2);
}
