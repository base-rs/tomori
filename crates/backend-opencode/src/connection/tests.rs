// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The connection's tests, by topic, with the simulated OpenCode service and the helpers they share.

use super::*;
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use tomori_backend::WorkerState;

mod commands;
mod driver;
mod events;
mod reconnecting;
mod service;
mod snapshot;
mod workers;

/// The state of a simulated OpenCode service that persists across its
/// restarts, as OpenCode keeps sessions on disk.
#[derive(Default)]
struct FakeState {
    /// Existing sessions and their open permission requests.
    sessions: BTreeMap<String, Vec<Value>>,
    created: usize,
    /// Sessions whose message listing waits until they are removed.
    stalled: HashSet<String>,
    /// How many more message listings fail for each session.
    failing: HashMap<String, usize>,
    /// When each session's messages were listed.
    listed: HashMap<String, Vec<std::time::Instant>>,
    /// While set, an event stream request waits before it is answered.
    hold_events: bool,
    /// Event stream requests received.
    event_requests: usize,
    /// Sessions whose prompts wait until they are removed.
    stalled_prompts: HashSet<String>,
    /// The session and message id of each prompt, as it was received.
    prompts: Vec<(String, String)>,
    /// Sessions whose records wait until they are removed, and how many
    /// such requests arrived.
    stalled_records: HashSet<String>,
    record_requests: usize,
    /// Sessions whose records fail, as a service having trouble answers.
    failing_records: HashSet<String>,
    /// Each session's message listing, if not empty.
    messages: HashMap<String, Value>,
    /// Each session's inbox, if not empty.
    inboxes: HashMap<String, Value>,
    /// Each session's instruction entries.
    instructions: HashMap<String, BTreeMap<String, Value>>,
    /// The instruction requests received, as method, session and key.
    instruction_requests: Vec<(String, String, String)>,
    /// The MCP servers registered at runtime, by their path, naming the server and its directory, with their
    /// configuration.
    mcp: BTreeMap<String, Value>,
    /// The MCP registration requests received, as method and path.
    mcp_requests: Vec<(String, String)>,
    /// While set, MCP registrations fail.
    failing_mcp: bool,
    /// The agents listed for every directory, if not none.
    agents: Option<Value>,
    /// Each reply, interruption and agent switch received, as path and body.
    actions: Vec<(String, Value)>,
    /// Each session's child records, as its child listing answers them.
    children: HashMap<String, Vec<Value>>,
    /// How long each child listing takes to be answered.
    child_latency: Duration,
    /// Sessions whose child listing waits until they are removed.
    stalled_children: HashSet<String>,
    /// Sessions OpenCode lists among those it is running.
    active: HashSet<String>,
    /// Sessions whose child listing fails.
    failing_children: HashSet<String>,
    /// Each session's record, where it is not the plain one.
    records: HashMap<String, Value>,
    /// Sessions whose replies, interruptions and agent switches fail.
    failing_actions: HashSet<String>,
    /// Each session's title, if its record gives one.
    titles: HashMap<String, String>,
    stopping: bool,
}

/// The record of a worker `id` of `parent`, created at `created`.
fn fake_child(id: &str, parent: &str, created: u64) -> Value {
    let mut record = fake_record(id);
    record["parentID"] = json!(parent);
    record["time"]["created"] = json!(created);
    record
}

#[derive(Default)]
struct FakeShared {
    state: Mutex<FakeState>,
    changed: std::sync::Condvar,
}

impl FakeShared {
    fn update(&self, change: impl FnOnce(&mut FakeState)) {
        change(&mut self.state.lock().unwrap());
        self.changed.notify_all();
    }
}

/// One run of the simulated service, on its own port, with its own password.
struct FakeService {
    address: std::net::SocketAddr,
    stopped: Arc<AtomicBool>,
    streams: Arc<Mutex<Vec<std::net::TcpStream>>>,
    /// The authorization header of every request, in order.
    authorizations: Arc<Mutex<Vec<Option<String>>>>,
    accept: Option<std::thread::JoinHandle<()>>,
}

fn fake_record(id: &str) -> Value {
    json!({"id":id,"projectID":"p","cost":0,"tokens":{"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}},"location":{"directory":"/tmp"},"time":{"created":1,"updated":1,"busy":0,"idle":1}})
}

fn basic(password: &str) -> String {
    Resolved {
        host: "127.0.0.1".into(),
        port: 1,
        auth: Some(("opencode".into(), password.into())),
    }
    .authorization_header()
    .unwrap()
}

impl FakeService {
    fn start(shared: Arc<FakeShared>, password: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let streams = Arc::new(Mutex::new(Vec::new()));
        let authorizations = Arc::new(Mutex::new(Vec::new()));
        let expected = (!password.is_empty()).then(|| basic(password));
        let accept = {
            let stopped = stopped.clone();
            let streams = streams.clone();
            let authorizations = authorizations.clone();
            std::thread::spawn(move || {
                while !stopped.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((socket, _)) => {
                            let shared = shared.clone();
                            let streams = streams.clone();
                            let authorizations = authorizations.clone();
                            let expected = expected.clone();
                            std::thread::spawn(move || {
                                fake_exchange(
                                    socket,
                                    &shared,
                                    &streams,
                                    &authorizations,
                                    expected.as_deref(),
                                );
                            });
                        }
                        Err(error) => {
                            assert_eq!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock,
                                "accept: {error}"
                            );
                            std::thread::sleep(Duration::from_millis(5));
                        }
                    }
                }
            })
        };
        Self {
            address,
            stopped,
            streams,
            authorizations,
            accept: Some(accept),
        }
    }

    /// Send one event to every open event stream.
    fn send(&self, event: Value) {
        let data = format!("data: {event}\n\n");
        for stream in self.streams.lock().unwrap().iter_mut() {
            let _ = write!(stream, "{:x}\r\n{}\r\n", data.len(), data);
            let _ = stream.flush();
        }
    }

    /// End every open event stream, as a service does that closes it.
    fn end_streams(&self) {
        for stream in self.streams.lock().unwrap().drain(..) {
            let mut stream = stream;
            let _ = write!(stream, "0\r\n\r\n");
            let _ = stream.flush();
        }
    }

    /// Send `bytes` as one chunk of every open event stream, framed or not.
    fn send_raw(&self, bytes: &[u8]) {
        for stream in self.streams.lock().unwrap().iter_mut() {
            let _ = write!(stream, "{:x}\r\n", bytes.len());
            let _ = stream.write_all(bytes);
            let _ = write!(stream, "\r\n");
            let _ = stream.flush();
        }
    }

    /// Stop the service: its port closes and its event streams end.
    fn stop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(accept) = self.accept.take() {
            accept.join().unwrap();
        }
        for stream in self.streams.lock().unwrap().drain(..) {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

impl Drop for FakeService {
    fn drop(&mut self) {
        self.stop();
    }
}

fn fake_exchange(
    mut socket: std::net::TcpStream,
    shared: &FakeShared,
    streams: &Mutex<Vec<std::net::TcpStream>>,
    authorizations: &Mutex<Vec<Option<String>>>,
    expected: Option<&str>,
) {
    // macOS accepted sockets inherit the listener's nonblocking mode.
    socket.set_nonblocking(false).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut reader = BufReader::new(socket.try_clone().unwrap());
    // A request that cannot be read names no path, and is not found.
    let mut line = String::new();
    let _ = reader.read_line(&mut line);
    let mut words = line.split_whitespace();
    let method = words.next().unwrap_or("").to_owned();
    let path = words.next().unwrap_or("").to_owned();
    let mut authorization = None;
    let mut length = 0usize;
    loop {
        line.clear();
        if reader.read_line(&mut line).is_err() || line == "\r\n" || line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':').unwrap_or_default();
        match name.to_ascii_lowercase().as_str() {
            "authorization" => authorization = Some(value.trim().to_owned()),
            "content-length" => length = value.trim().parse().unwrap_or(0),
            _ => {}
        }
    }
    let mut body = vec![0; length];
    let _ = std::io::Read::read_exact(&mut reader, &mut body);
    authorizations.lock().unwrap().push(authorization.clone());
    let respond = |socket: &mut std::net::TcpStream, status: &str, body: &str| {
        // A response without content has no framing fields.
        if status.starts_with("204") {
            let _ = write!(socket, "HTTP/1.1 {status}\r\nConnection: close\r\n\r\n");
            return;
        }
        let _ = write!(
            socket,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    };
    if authorization.as_deref() != expected {
        respond(
            &mut socket,
            "401 Unauthorized",
            r#"{"message":"unauthorized"}"#,
        );
        return;
    }
    if path == "/api/event" {
        let mut state = shared.state.lock().unwrap();
        state.event_requests += 1;
        shared.changed.notify_all();
        while state.hold_events && !state.stopping {
            state = shared
                .changed
                .wait_timeout(state, Duration::from_millis(50))
                .unwrap()
                .0;
        }
        drop(state);
        let _ = write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n"
        );
        let _ = socket.flush();
        streams.lock().unwrap().push(socket);
        return;
    }
    let session = path
        .strip_prefix("/api/session/")
        .map(|rest| rest.split(['/', '?']).next().unwrap_or("").to_owned());
    let (status, body) = if method == "POST" && path == "/api/session" {
        let mut state = shared.state.lock().unwrap();
        state.created += 1;
        let id = format!("s{}", state.created);
        state.sessions.insert(id.clone(), Vec::new());
        ("200 OK", json!({ "data": fake_record(&id) }).to_string())
    } else if path.starts_with("/api/agent?") {
        let agents = shared.state.lock().unwrap().agents.clone();
        (
            "200 OK",
            json!({ "data": agents.unwrap_or_else(|| json!([])) }).to_string(),
        )
    } else if path == "/api/session/active" {
        let active: serde_json::Map<String, Value> = shared
            .state
            .lock()
            .unwrap()
            .active
            .iter()
            .map(|id| (id.clone(), json!({"type": "running"})))
            .collect();
        ("200 OK", json!({ "data": active }).to_string())
    } else if let Some(query) = path.strip_prefix("/api/session?parentID=") {
        let parent = query.split('&').next().unwrap_or_default().to_owned();
        let latency = shared.state.lock().unwrap().child_latency;
        std::thread::sleep(latency);
        let mut state = shared.state.lock().unwrap();
        while state.stalled_children.contains(&parent) && !state.stopping {
            state = shared
                .changed
                .wait_timeout(state, Duration::from_millis(50))
                .unwrap()
                .0;
        }
        let children = state.children.get(&parent).cloned().unwrap_or_default();
        match state.failing_children.contains(&parent) {
            true => (
                "500 Internal Server Error",
                r#"{"message":"lost"}"#.to_owned(),
            ),
            false => (
                "200 OK",
                json!({"data": children, "cursor": {"next": null}}).to_string(),
            ),
        }
    } else if path.starts_with("/api/experimental/mcp/") {
        let mut state = shared.state.lock().unwrap();
        state.mcp_requests.push((method.clone(), path.clone()));
        if state.failing_mcp {
            (
                "500 Internal Server Error",
                r#"{"message":"lost"}"#.to_owned(),
            )
        } else if method == "PUT" {
            let body: Value = serde_json::from_slice(&body).unwrap();
            state.mcp.insert(path.clone(), body["config"].clone());
            ("204 No Content", String::new())
        } else if state.mcp.remove(&path).is_some() {
            ("204 No Content", String::new())
        } else {
            (
                "404 Not Found",
                r#"{"_tag":"McpServerNotFoundError","message":"MCP server not found: tomori"}"#
                    .to_owned(),
            )
        }
    } else if let Some(rest) = path.strip_prefix("/api/experimental/session/") {
        let mut state = shared.state.lock().unwrap();
        let (id, key) = rest
            .split_once("/instructions/entries")
            .map(|(id, key)| (id.to_owned(), key.trim_start_matches('/').to_owned()))
            .unwrap_or_default();
        state
            .instruction_requests
            .push((method.clone(), id.clone(), key.clone()));
        shared.changed.notify_all();
        if !state.sessions.contains_key(&id) {
            (
                "404 Not Found",
                r#"{"_tag":"SessionNotFoundError","message":"not found"}"#.to_owned(),
            )
        } else {
            let entries = state.instructions.entry(id).or_default();
            // Entries are only put and removed.
            match method.as_str() {
                "PUT" => {
                    let value: Value = serde_json::from_slice(&body).unwrap();
                    entries.insert(key, value["value"].clone());
                }
                _ => {
                    entries.remove(&key);
                }
            }
            ("204 No Content", String::new())
        }
    } else if let Some(id) = session {
        let mut state = shared.state.lock().unwrap();
        if path.ends_with("/prompt") {
            let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let message = body["id"].as_str().unwrap_or_default().to_owned();
            state.prompts.push((id.clone(), message));
            shared.changed.notify_all();
        }
        while state.stalled_prompts.contains(&id) && path.ends_with("/prompt") && !state.stopping {
            state = shared
                .changed
                .wait_timeout(state, Duration::from_millis(50))
                .unwrap()
                .0;
        }
        let record = path == format!("/api/session/{id}");
        if record && state.stalled_records.contains(&id) {
            state.record_requests += 1;
            shared.changed.notify_all();
        }
        while record && state.stalled_records.contains(&id) && !state.stopping {
            state = shared
                .changed
                .wait_timeout(state, Duration::from_millis(50))
                .unwrap()
                .0;
        }
        if state.stopping {
            // Leave without answering, as a service that went away.
            return;
        }
        while state.stalled.contains(&id) && path.contains("/message?") && !state.stopping {
            state = shared
                .changed
                .wait_timeout(state, Duration::from_millis(50))
                .unwrap()
                .0;
        }
        if path.contains("/message?") {
            state
                .listed
                .entry(id.clone())
                .or_default()
                .push(std::time::Instant::now());
        }
        let fail = path.contains("/message?")
            && state
                .failing
                .get(&id)
                .is_some_and(|remaining| *remaining > 0);
        if fail {
            *state.failing.get_mut(&id).unwrap() -= 1;
        }
        // Deleting a session removes it, as OpenCode does with its children, and answers with no content.
        let deleted = (method == "DELETE" && record).then(|| state.sessions.remove(&id).is_some());
        if deleted.is_some() {
            state.actions.push((format!("DELETE {path}"), Value::Null));
            shared.changed.notify_all();
        }
        match state.sessions.get(&id) {
            _ if deleted == Some(true) => ("204 No Content", String::new()),
            _ if record && state.failing_records.contains(&id) => (
                "500 Internal Server Error",
                r#"{"message":"busy"}"#.to_owned(),
            ),
            _ if fail => (
                "500 Internal Server Error",
                r#"{"message":"starting"}"#.to_owned(),
            ),
            None => ("404 Not Found", r#"{"message":"not found"}"#.to_owned()),
            Some(_) if path.contains("/message?") => (
                "200 OK",
                json!({
                    "data": state.messages.get(&id).cloned().unwrap_or_else(|| json!([])),
                    "cursor": {"next": null},
                })
                .to_string(),
            ),
            Some(_) if path.ends_with("/inbox") => (
                "200 OK",
                json!({ "data": state.inboxes.get(&id).cloned().unwrap_or_else(|| json!([])) })
                    .to_string(),
            ),
            Some(permissions) if path.ends_with("/permission") => {
                ("200 OK", json!({ "data": permissions }).to_string())
            }
            Some(_) if path.ends_with("/form") => ("200 OK", r#"{"data":[]}"#.to_owned()),
            Some(_) if path.ends_with("/prompt") => ("200 OK", "{}".to_owned()),
            Some(_) if method == "POST" && state.failing_actions.contains(&id) => (
                "500 Internal Server Error",
                r#"{"message":"refused"}"#.to_owned(),
            ),
            Some(_) if method == "POST" => {
                let body = serde_json::from_slice(&body).unwrap_or(Value::Null);
                state.actions.push((path.clone(), body));
                shared.changed.notify_all();
                ("200 OK", "{}".to_owned())
            }
            Some(_) => {
                let mut record = state
                    .records
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| fake_record(&id));
                if let Some(title) = state.titles.get(&id) {
                    record["title"] = json!(title);
                }
                ("200 OK", json!({ "data": record }).to_string())
            }
        }
    } else {
        ("404 Not Found", r#"{"message":"unexpected"}"#.to_owned())
    };
    respond(&mut socket, status, &body);
}

/// Record a service the way `opencode serve --service` does, replacing the
/// file at once so a concurrent reader never sees half of it.
fn write_service_file(path: &std::path::Path, address: std::net::SocketAddr, password: &str) {
    let partial = path.with_extension("partial");
    std::fs::write(
        &partial,
        json!({"id":"service","version":"2","url":format!("http://{address}"),"pid":1,"password":password}).to_string(),
    )
    .unwrap();
    std::fs::rename(&partial, path).unwrap();
}

/// Forward a session's current events to a channel, ending when it closes.
fn watch(events: &RegistrationEvents) -> std::sync::mpsc::Receiver<Event> {
    let events = events.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    // Until the events end, or nobody watches them.
    std::thread::spawn(move || {
        while let Ok(event) = block_on(events.recv())
            && event
                .into_current()
                .is_none_or(|event| tx.send(event).is_ok())
        {}
    });
    rx
}

/// The events up to and including the first that `wanted` matches.
fn until(
    events: &std::sync::mpsc::Receiver<Event>,
    what: &str,
    wanted: impl Fn(&Event) -> bool,
) -> Vec<Event> {
    until_within(events, Duration::from_secs(5), what, wanted)
}

/// [`until`], waiting at most `limit`.
fn until_within(
    events: &std::sync::mpsc::Receiver<Event>,
    limit: Duration,
    what: &str,
    wanted: impl Fn(&Event) -> bool,
) -> Vec<Event> {
    let deadline = std::time::Instant::now() + limit;
    let mut seen = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let Ok(event) = events.recv_timeout(remaining) else {
            unreachable!("{what} did not arrive; saw {seen:?}");
        };
        let found = wanted(&event);
        seen.push(event);
        if found {
            return seen;
        }
    }
}

fn join_within(thread: std::thread::JoinHandle<()>, limit: Duration) {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(thread.join());
    });
    rx.recv_timeout(limit)
        .expect("the connection thread did not stop in time")
        .unwrap();
}

/// Start a connection to whichever service `address` names, with the
/// simulated service's password.
fn connect_to(
    address: &Arc<Mutex<std::net::SocketAddr>>,
) -> (Commands, std::thread::JoinHandle<()>) {
    let address = address.clone();
    let (commands, receiver) = Commands::new();
    let connection = std::thread::spawn(move || {
        run_thread(
            Box::new(move || {
                let address = *address.lock().unwrap();
                Ok(Resolved {
                    host: "127.0.0.1".into(),
                    port: address.port(),
                    auth: Some(("opencode".into(), "secret".into())),
                })
            }),
            receiver,
        )
    });
    (commands, connection)
}

fn wait_for(shared: &FakeShared, what: &str, ready: impl Fn(&FakeState) -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut state = shared.state.lock().unwrap();
    while !ready(&state) {
        assert!(std::time::Instant::now() < deadline, "{what}");
        state = shared
            .changed
            .wait_timeout(state, Duration::from_millis(50))
            .unwrap()
            .0;
    }
}

fn in_thread<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> std::sync::mpsc::Receiver<(T, std::time::Instant)> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = work();
        let _ = tx.send((result, std::time::Instant::now()));
    });
    rx
}

/// What kind of event `event` is, as it is serialized.
fn kind(event: &Event) -> String {
    serde_json::to_value(event).unwrap()["type"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// The first of `events` of `kind`, sent once it arrives.
fn first_of(events: &RegistrationEvents, kind: &'static str) -> std::sync::mpsc::Receiver<Event> {
    let events = events.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let found = std::iter::from_fn(|| block_on(events.recv()).ok())
            .filter_map(QueuedEvent::into_current)
            .find(|event| self::kind(event) == kind);
        if let Some(event) = found {
            let _ = tx.send(event);
        }
    });
    rx
}

fn registrations() -> HashMap<String, Registration> {
    let mut registrations = HashMap::new();
    insert_registration(
        &mut registrations,
        "parent".into(),
        "/tmp",
        Modes::default(),
    );
    insert_registration(&mut registrations, "other".into(), "/tmp", Modes::default());
    registrations
}

/// Read `id`'s initial state, as a space does once it is bound.
fn read_snapshot(
    commands: &Commands,
    id: &str,
    events: &RegistrationEvents,
) -> tomori_backend::Result<Snapshot> {
    let id = id.to_owned();
    let identity = events.identity.clone();
    block_on(commands.reply_with(|reply| Command::Snapshot {
        id,
        identity,
        reply,
    }))
    .and_then(SnapshotReply::accept)
}

/// Create a session against a simulated service and read it.
fn open_session(commands: &Commands) -> (String, RegistrationEvents) {
    let (id, events) = block_on(commands.reply_with(|reply| Command::Create {
        spec: SessionSpec {
            directory: "/tmp".into(),
            title: "Open".into(),
        },
        reply,
    }))
    .unwrap();
    read_snapshot(commands, &id, &events).unwrap();
    (id, events)
}

fn worker_ids(workers: &[Worker]) -> Vec<&str> {
    workers.iter().map(|worker| worker.id.as_str()).collect()
}

/// The identifiers of the workers `event` reports, with its notice, as
/// [`reported`].
fn reported_ids(event: &Event) -> Option<(Vec<&str>, Option<&str>)> {
    reported(event).map(|(workers, notice)| (worker_ids(workers), notice))
}

/// The workers `event` reports, with its notice if the list is
/// incomplete; none for an event that reports no worker list.
fn reported(event: &Event) -> Option<(&[Worker], Option<&str>)> {
    match event {
        Event::WorkersChanged { workers } => Some((workers, None)),
        Event::WorkersPartial { workers, message } => Some((workers, Some(message))),
        _ => None,
    }
}

/// Create a session against a simulated service without reading it.
fn create_session(commands: &Commands) -> (String, RegistrationEvents) {
    block_on(commands.reply_with(|reply| Command::Create {
        spec: SessionSpec {
            directory: "/tmp".into(),
            title: "Family".into(),
        },
        reply,
    }))
    .unwrap()
}

/// A session registered and not yet read, whose snapshot is in flight.
fn opening() -> (HashMap<String, Registration>, RegistrationEvents) {
    let mut registrations = HashMap::new();
    let events = insert_registration(&mut registrations, "s".into(), "/tmp", Modes::default());
    (registrations, events)
}
