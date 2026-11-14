// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Calls to the OpenCode server's HTTP interface.
//!
//! Every helper builds a [`Request`], submits it through the pooled
//! [`Client`], and parses the JSON body. The client and its requests borrow
//! from the scheduler, so these run on the connection thread.
use base_http::{
    Field, Method, Path, Request,
    body::{BodyFrame, BufferedBody, EmptyBody, ReadableBody},
};
use base_http_client::{Client, ResponseBody};
use base_scheduler::{Cancellation, Timers};
use futures_lite::StreamExt;
use futures_util::stream::FuturesUnordered;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::endpoint::Resolved;
use crate::wire::question_from_form;

/// Bound the backend's worker cache. Exceeding it is a visible incomplete
/// snapshot, never an authoritative truncated list.
pub(crate) const MAX_TRACKED_WORKERS: usize = 4096;
/// A single agent response must not grow without bound while it is read.
pub(crate) const MAX_JSON_BYTES: usize = 16 * 1024 * 1024;
/// Bound the aggregate size of records retained for a worker family.
const MAX_WORKER_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
/// The listing of one parent's children, with that parent.
type ChildListRequest<'a> =
    Pin<Box<dyn Future<Output = (String, Result<Vec<Value>, Failure>)> + 'a>>;

/// A failed call, with the HTTP status when the server answered.
#[derive(Debug)]
pub struct Failure {
    pub status: Option<u16>,
    pub message: String,
}

impl From<base_http_client::Error> for Failure {
    fn from(error: base_http_client::Error) -> Self {
        Failure {
            status: None,
            message: error.to_string(),
        }
    }
}

/// Submit a request, appending the endpoint's credentials when it has them.
async fn exchange<'scope, B>(
    client: &Client<'scope>,
    mut request: Request<B>,
    auth: Option<&str>,
    cancellation: &Cancellation,
) -> Result<base_http::Response<ResponseBody>, Failure>
where
    B: ReadableBody + 'scope,
    B::Data: Into<Vec<u8>>,
{
    // Every authorization is `Basic` and base64, and a request has room for these few fields.
    if let Some(authorization) = auth {
        let field = Field::new("authorization", authorization)
            .expect("a Basic authorization is a valid field value");
        request
            .fields
            .append(field)
            .expect("a request has room for its authorization");
    }
    // Opt out of connection reuse: opencode closes idle keep-alive connections
    // after five seconds, and `base-http-client` pools and reuses connections
    // without retrying. A request that reuses a connection opencode already
    // dropped reads EOF and fails with "truncated HTTP/1 message" (base-rs/
    // base-http#36). `Connection: close` keeps the client from reusing: each
    // call gets a fresh connection, which stays valid for the single exchange.
    request
        .fields
        .append(Field::new("connection", "close").expect("a valid field"))
        .expect("a request has room to close its connection");
    Ok(client.call(request, cancellation).await?)
}

/// Read a response body to the end, failing with the status when the server
/// refused the call.
async fn read_json(
    response: base_http::Response<ResponseBody>,
    cancellation: &Cancellation,
) -> Result<Value, Failure> {
    let status = response.status.as_u16();
    let mut body = response.body;
    let mut bytes = Vec::new();
    loop {
        let frame = body.read(cancellation).await.map_err(|error| Failure {
            status: Some(status),
            message: error.to_string(),
        })?;
        let end = frame.is_end();
        if let BodyFrame::Data { data, .. } = frame {
            append_json_bytes(&mut bytes, &data, status)?;
        }
        if end {
            break;
        }
    }
    if !(200..300).contains(&status) {
        let message = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|value| {
                value
                    .get("message")
                    .or_else(|| value.get("data").and_then(|data| data.get("message")))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| format!("the server answered with status {status}"));
        return Err(Failure {
            status: Some(status),
            message,
        });
    }
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|error| Failure {
        status: Some(status),
        message: format!("the server sent an unparseable body: {error}"),
    })
}

fn append_json_bytes(bytes: &mut Vec<u8>, data: &[u8], status: u16) -> Result<(), Failure> {
    if bytes.len().saturating_add(data.len()) > MAX_JSON_BYTES {
        return Err(Failure {
            status: Some(status),
            message: "the server's JSON response exceeds the supported size".into(),
        });
    }
    bytes.extend_from_slice(data);
    Ok(())
}

fn post(path: &str, payload: &Value) -> Request<BufferedBody> {
    with_json(Method::POST, path, payload)
}

/// A `method` request for `path` whose body is `payload`.
fn with_json(method: Method, path: &str, payload: &Value) -> Request<BufferedBody> {
    let mut request = Request::new(
        method,
        Path::from(path.to_owned()),
        BufferedBody::from(payload.to_string().into_bytes()),
    );
    request
        .fields
        .append(Field::new("content-type", "application/json").expect("a valid field"))
        .expect("a new request has room for its content type");
    request
}

fn get(path: &str) -> Request<EmptyBody> {
    Request::get(Path::from(path.to_owned()))
}

/// `POST /api/session`: start a session in `directory`, optionally titled.
pub async fn create_session<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    directory: &str,
    title: Option<&str>,
    cancellation: &Cancellation,
) -> Result<Value, Failure> {
    let mut payload = serde_json::json!({ "location": { "directory": directory } });
    if let Some(title) = title.filter(|title| !title.is_empty()) {
        payload["title"] = Value::String(title.to_owned());
    }
    let response = exchange(
        client,
        post("/api/session", &payload),
        resolved.authorization_header().as_deref(),
        cancellation,
    )
    .await?;
    let value = read_json(response, cancellation).await?;
    // The session record is nested under `data`, as with the list endpoint.
    Ok(value.get("data").cloned().unwrap_or(value))
}

/// Sessions asked for on each page of a session listing.
const SESSION_PAGE: usize = 200;
/// The most sessions a listing reads, newest first, across every directory, before it stops with those it found.
pub(crate) const MAX_SCANNED_SESSIONS: usize = 4096;
/// The most sessions a listing returns, newest first.
pub(crate) const MAX_LISTED_SESSIONS: usize = 1000;
/// The most pages a listing reads, so that a server answering empty pages with a cursor cannot keep it reading.
const MAX_SESSION_PAGES: usize = 64;

/// One page of the listing of sessions a participant can attach to: top-level,
/// unarchived sessions, newest first. Subagents' child sessions are workers of
/// their parent, not conversations to join. OpenCode's `directory` filter
/// matches one directory exactly, so the sessions of every directory are read
/// and those beneath the one asked for kept.
fn session_list_path(cursor: Option<&str>) -> String {
    let mut path = format!("/api/session?parentID=null&archived=false&limit={SESSION_PAGE}");
    if let Some(cursor) = cursor {
        path.push_str("&cursor=");
        path.push_str(&urlencoding(cursor));
    }
    path
}

/// Whether a session in `directory`, as OpenCode records it, is in `root` or beneath it, compared component by
/// component, so that `/home/sam2` is not beneath `/home/sam`, and a trailing slash changes nothing. A directory that
/// is relative or goes up with `..` is not beneath anything, since it could name a directory outside `root`.
pub(crate) fn beneath(directory: &str, root: &str) -> bool {
    let path = std::path::Path::new(directory);
    path.is_absolute()
        && !path
            .components()
            .any(|component| component == std::path::Component::ParentDir)
        && path.starts_with(root)
}

/// `GET /api/session?parentID=null&archived=false`, page by page: the existing top-level, unarchived sessions whose
/// `location.directory` is `directory` or beneath it, newest first. It reads at most [`MAX_SCANNED_SESSIONS`]
/// sessions and returns at most [`MAX_LISTED_SESSIONS`], stopping with those it found at either bound, at the last
/// page, or at a cursor it has already followed.
pub async fn list_sessions<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    directory: &str,
    cancellation: &Cancellation,
) -> Result<Vec<Value>, Failure> {
    let mut sessions = Vec::new();
    let mut ids = std::collections::HashSet::new();
    let mut cursors = std::collections::HashSet::new();
    let mut scanned = 0usize;
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_SESSION_PAGES {
        let response = exchange(
            client,
            get(&session_list_path(cursor.as_deref())),
            resolved.authorization_header().as_deref(),
            cancellation,
        )
        .await?;
        let value = read_json(response, cancellation).await?;
        let page = value
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| Failure {
                status: None,
                message: "the server did not return a session list".to_owned(),
            })?;
        for session in page {
            scanned += 1;
            let id = session.get("id").and_then(Value::as_str);
            let within = session
                .pointer("/location/directory")
                .and_then(Value::as_str)
                .is_some_and(|found| beneath(found, directory));
            if within && id.is_some_and(|id| ids.insert(id.to_owned())) {
                sessions.push(session.clone());
            }
            if sessions.len() == MAX_LISTED_SESSIONS || scanned == MAX_SCANNED_SESSIONS {
                return Ok(sessions);
            }
        }
        match value.pointer("/cursor/next").and_then(Value::as_str) {
            Some(next) if !next.is_empty() && cursors.insert(next.to_owned()) => {
                cursor = Some(next.to_owned())
            }
            _ => break,
        }
    }
    Ok(sessions)
}

/// `GET /api/session/:id`: one session's record.
pub async fn session_info<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    session_id: &str,
    cancellation: &Cancellation,
) -> Result<Value, Failure> {
    let path = format!("/api/session/{}", urlencoding(session_id));
    let response = exchange(
        client,
        get(&path),
        resolved.authorization_header().as_deref(),
        cancellation,
    )
    .await?;
    let value = read_json(response, cancellation).await?;
    // The session record is nested under `data`, as with the list endpoint.
    Ok(value.get("data").cloned().unwrap_or(value))
}

/// The directory a session's record says it is in, if it says, as OpenCode names it: a session moved with OpenCode's
/// session move names the directory it was moved to.
pub fn session_directory(info: &Value) -> Option<&str> {
    info.pointer("/location/directory")
        .and_then(Value::as_str)
        .filter(|directory| !directory.is_empty())
}

/// `duration` as a message says it: in seconds when it is a whole number of them, and otherwise in milliseconds.
pub(crate) fn said(duration: Duration) -> String {
    if duration.subsec_nanos() == 0 {
        format!("{} seconds", duration.as_secs())
    } else {
        format!("{} milliseconds", duration.as_millis())
    }
}

/// How long the optional active-session lookup may take alongside the
/// session record during reconciliation.
pub(crate) const ACTIVE_LOOKUP_TIMEOUT: Duration = Duration::from_secs(2);

/// Process-owned active session drains. An ID in this map is positive,
/// point-in-time evidence that the OpenCode process is running it.
pub async fn active_sessions<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    cancellation: &Cancellation,
) -> Result<std::collections::HashSet<String>, Failure> {
    let response = exchange(
        client,
        get("/api/session/active"),
        resolved.authorization_header().as_deref(),
        cancellation,
    )
    .await?;
    parse_active_sessions(&read_json(response, cancellation).await?)
}

/// Active-state lookup is optional evidence and must not hold up the record
/// fallback or the initial snapshot if the endpoint stalls.
pub async fn active_sessions_with_timeout<S: Timers>(
    client: &Client<'_>,
    resolved: &Resolved,
    cancellation: &Cancellation,
    timers: &S,
    timeout: Duration,
) -> Option<std::collections::HashSet<String>> {
    let local = cancellation.child();
    let result = futures_lite::future::race(
        async { active_sessions(client, resolved, &local).await.ok() },
        async {
            let _ = timers.sleep(timeout, &local).await;
            None
        },
    )
    .await;
    local.cancel();
    result
}

fn parse_active_sessions(value: &Value) -> Result<std::collections::HashSet<String>, Failure> {
    let sessions = value
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| Failure {
            status: None,
            message: "the server did not return active sessions".into(),
        })?;
    if sessions.iter().any(|(id, state)| {
        id.is_empty() || state.get("type").and_then(Value::as_str) != Some("running")
    }) {
        return Err(Failure {
            status: None,
            message: "the server returned an invalid active session".into(),
        });
    }
    Ok(sessions.keys().cloned().collect())
}

/// `GET /api/session?parentID=`: the children of a session.
pub async fn child_sessions<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    parent_id: &str,
    cancellation: &Cancellation,
) -> Result<Vec<Value>, Failure> {
    const PAGE_SIZE: usize = 100;
    const MAX_PAGES: usize = 128;
    let mut children = Vec::new();
    let mut ids = std::collections::HashSet::new();
    let mut cursors = std::collections::HashSet::new();
    let mut child_bytes = 0usize;
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut path = format!(
            "/api/session?parentID={}&limit={PAGE_SIZE}",
            urlencoding(parent_id)
        );
        if let Some(cursor) = &cursor {
            path.push_str("&cursor=");
            path.push_str(&urlencoding(cursor));
        }
        let response = exchange(
            client,
            get(&path),
            resolved.authorization_header().as_deref(),
            cancellation,
        )
        .await?;
        let value = read_json(response, cancellation).await?;
        let page = value
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| Failure {
                status: None,
                message: "the server did not return a child session list".to_owned(),
            })?;
        for child in page {
            if children.len() == MAX_TRACKED_WORKERS {
                return Err(Failure {
                    status: None,
                    message: "the child session list exceeds the worker limit".to_owned(),
                });
            }
            let worker = crate::wire::worker_from_session(child).ok_or_else(|| Failure {
                status: None,
                message: "the server returned a child session without an identifier".to_owned(),
            })?;
            if worker.id.is_empty()
                || worker.parent.as_deref() != Some(parent_id)
                || !valid_session_record(child)
            {
                return Err(Failure {
                    status: None,
                    message:
                        "the server returned a child session with an invalid parent or identifier"
                            .to_owned(),
                });
            }
            if !ids.insert(worker.id) {
                return Err(Failure {
                    status: None,
                    message: "the server repeated a child session across pages".to_owned(),
                });
            }
            add_worker_record_size(&mut child_bytes, child)?;
            children.push(child.clone());
        }
        let next = value
            .get("cursor")
            .and_then(Value::as_object)
            .ok_or_else(|| Failure {
                status: None,
                message: "the server did not return a child session cursor".to_owned(),
            })?
            .get("next");
        match next {
            None | Some(Value::Null) => return Ok(children),
            Some(Value::String(next)) if !next.is_empty() && cursors.insert(next.clone()) => {
                cursor = Some(next.clone())
            }
            _ => {
                return Err(Failure {
                    status: None,
                    message: "the server returned an invalid child session cursor".to_owned(),
                });
            }
        }
    }
    Err(Failure {
        status: None,
        message: "the child session list exceeds the page limit".to_owned(),
    })
}

/// A walk of a session's descendants through their direct-child pages, which
/// can stop and later go on where it stopped: a family too large to read in
/// time is read in part, and the rest later. A repeated identity or a limit
/// failure fails the walk rather than reporting what it found as the family.
pub(crate) struct FamilyWalk {
    /// Sessions whose children have not been listed yet, oldest discovered first.
    pending: std::collections::VecDeque<String>,
    /// Sessions whose children were being listed when the walk last stopped.
    listing: Vec<String>,
    /// Every session the walk has found, including the root.
    seen: std::collections::HashSet<String>,
    /// The records found since [`FamilyWalk::take_found`] last took them.
    found: Vec<Value>,
    /// How many descendants, and how many bytes of their records, the walk has
    /// found in all.
    count: usize,
    bytes: usize,
}

impl FamilyWalk {
    /// Child pages listed at once.
    const MAX_IN_FLIGHT: usize = 8;
    /// How long one parent's children may take to list before the walk fails,
    /// so that a walk OpenCode stops answering ends.
    pub(crate) const LISTING_TIMEOUT: Duration = Duration::from_secs(5);

    pub(crate) fn new(session_id: &str) -> Self {
        Self {
            pending: std::collections::VecDeque::from([session_id.to_owned()]),
            listing: Vec::new(),
            seen: std::collections::HashSet::from([session_id.to_owned()]),
            found: Vec::new(),
            count: 0,
            bytes: 0,
        }
    }

    /// The records found since this was last called.
    pub(crate) fn take_found(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.found)
    }

    /// List children, at most [`Self::MAX_IN_FLIGHT`] parents at once, until
    /// the family has been read. A parent whose children are not listed
    /// within `timeout`, which is [`Self::LISTING_TIMEOUT`] in production, fails
    /// the walk. The future may be
    /// dropped at any point, or fail: a later call lists again the parents
    /// whose listing it abandoned or that failed.
    pub(crate) async fn run<'scope, S: Timers>(
        &mut self,
        client: &Client<'scope>,
        resolved: &Resolved,
        cancellation: &Cancellation,
        timers: &S,
        timeout: Duration,
    ) -> Result<(), Failure> {
        for parent in self.listing.drain(..).rev() {
            self.pending.push_front(parent);
        }
        let mut requests: FuturesUnordered<ChildListRequest<'_>> = FuturesUnordered::new();
        while !self.pending.is_empty() || !requests.is_empty() {
            while requests.len() < Self::MAX_IN_FLIGHT {
                let Some(parent) = self.pending.pop_front() else {
                    break;
                };
                self.listing.push(parent.clone());
                requests.push(Box::pin(async move {
                    let children = futures_lite::future::or(
                        child_sessions(client, resolved, &parent, cancellation),
                        async {
                            let _ = timers.sleep(timeout, cancellation).await;
                            Err(Failure {
                                status: None,
                                message: format!(
                                    "OpenCode did not list a worker's children within {}",
                                    said(timeout)
                                ),
                            })
                        },
                    )
                    .await;
                    (parent, children)
                }));
            }
            let (parent, children) = requests
                .next()
                .await
                .expect("a request is in flight while the loop runs");
            // A parent whose page fails stays among those being listed, and
            // none of its page is taken, so a later run lists it again whole.
            self.take_page(children?)?;
            self.listing.retain(|listing| *listing != parent);
        }
        Ok(())
    }

    /// Take a parent's page of children, all of it or, when a child repeats
    /// one found before or the page would exceed a limit, none of it.
    fn take_page(&mut self, children: Vec<Value>) -> Result<(), Failure> {
        let id = |child: &Value| -> String {
            child
                .get("id")
                .and_then(Value::as_str)
                .expect("validated child identifier")
                .to_owned()
        };
        // A page's identifiers are distinct, as its listing checks.
        if children.iter().any(|child| self.seen.contains(&id(child))) {
            return Err(Failure {
                status: None,
                message: "the server returned a repeated or cyclic worker identity".into(),
            });
        }
        if self.count + children.len() > MAX_TRACKED_WORKERS {
            return Err(Failure {
                status: None,
                message: "the worker family exceeds the worker limit".into(),
            });
        }
        let mut bytes = self.bytes;
        for child in &children {
            add_worker_record_size(&mut bytes, child)?;
        }
        self.bytes = bytes;
        self.count += children.len();
        for child in children {
            let id = id(&child);
            self.seen.insert(id.clone());
            self.pending.push_back(id);
            self.found.push(child);
        }
        Ok(())
    }
}

/// Workers from child session records, oldest first: in the order their
/// records say they were created, since neither the order they were
/// discovered in nor their identifiers say how old they are.
pub fn workers_from_children(children: &[Value]) -> Vec<tomori_backend::Worker> {
    let created = |child: &Value| {
        child
            .get("time")
            .and_then(|time| time.get("created"))
            .and_then(Value::as_f64)
            .unwrap_or(f64::INFINITY)
    };
    let mut children: Vec<&Value> = children.iter().collect();
    children.sort_by(|a, b| created(a).total_cmp(&created(b)));
    children
        .into_iter()
        .filter_map(crate::wire::worker_from_session)
        .collect()
}

fn add_worker_record_size(total: &mut usize, child: &Value) -> Result<(), Failure> {
    let size = serde_json::to_vec(child)
        .expect("a JSON value serializes")
        .len();
    let next = total.saturating_add(size);
    if next > MAX_WORKER_SNAPSHOT_BYTES {
        return Err(Failure {
            status: None,
            message: "the worker family exceeds the supported snapshot size".into(),
        });
    }
    *total = next;
    Ok(())
}

/// Required fields of OpenCode's Session.Info. Extra fields are accepted for upgrades.
fn valid_session_record(child: &Value) -> bool {
    let tokens = child.get("tokens");
    let cache = tokens.and_then(|tokens| tokens.get("cache"));
    let time = child.get("time");
    child
        .get("projectID")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty())
        && child.get("cost").is_some_and(Value::is_number)
        && ["input", "output", "reasoning"].iter().all(|key| {
            tokens
                .and_then(|tokens| tokens.get(*key))
                .is_some_and(Value::is_number)
        })
        && ["read", "write"].iter().all(|key| {
            cache
                .and_then(|cache| cache.get(*key))
                .is_some_and(Value::is_number)
        })
        && ["created", "updated"].iter().all(|key| {
            time.and_then(|time| time.get(*key))
                .is_some_and(Value::is_number)
        })
        && ["busy", "idle"].iter().all(|key| {
            time.and_then(|time| time.get(*key))
                .is_none_or(Value::is_number)
        })
        && child.get("title").is_none_or(Value::is_string)
        && child.get("parentID").is_none_or(Value::is_string)
        && child.get("outcome").is_none_or(|outcome| {
            matches!(
                outcome.as_str(),
                Some("succeeded" | "failed" | "interrupted")
            )
        })
        && child
            .get("location")
            .and_then(|location| location.get("directory"))
            .and_then(Value::as_str)
            .is_some_and(|directory| !directory.is_empty())
}

/// `POST /api/session/:id/prompt`: deliver one user turn. `key` is the
/// server's delivery key; it is sent to opencode as a `msg_`-prefixed id
/// (which opencode requires) and recovered from the `inboxID` of the
/// `session.inbox.delivered` event by stripping the same prefix.
pub async fn send_prompt<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    session_id: &str,
    key: &str,
    text: &str,
    cancellation: &Cancellation,
) -> Result<(), Failure> {
    let path = format!("/api/session/{}/prompt", urlencoding(session_id));
    // opencode requires the id to start with "msg_"; the server's key does not,
    // so prefix it. The prefix is stripped again in `wire::inbox_delivered`
    // when the `session.inbox.delivered` event's `inboxID` arrives.
    let message_id = if key.starts_with("msg_") {
        key.to_owned()
    } else {
        format!("msg_{key}")
    };
    let payload = serde_json::json!({
        "id": message_id,
        "text": text
    });
    let response = exchange(
        client,
        post(&path, &payload),
        resolved.authorization_header().as_deref(),
        cancellation,
    )
    .await?;
    read_json(response, cancellation).await?;
    Ok(())
}

/// `GET /api/agent?location[directory]=`: the agents configured for a directory, in OpenCode's order. The first call
/// for a directory the server has not loaded yet answers an empty list while it loads, so an empty answer is retried a
/// few times.
pub async fn list_agents<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    directory: &str,
    cancellation: &Cancellation,
) -> Result<Vec<Value>, Failure> {
    let path = format!(
        "/api/agent?location%5Bdirectory%5D={}",
        urlencoding(directory)
    );
    let mut agents = Vec::new();
    for _ in 0..3 {
        let response = exchange(
            client,
            get(&path),
            resolved.authorization_header().as_deref(),
            cancellation,
        )
        .await?;
        let value = read_json(response, cancellation).await?;
        agents = value
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if !agents.is_empty() {
            break;
        }
    }
    Ok(agents)
}

/// `POST /api/session/:id/agent`: choose the agent that answers the session's later prompts. OpenCode confirms with a
/// `session.next.agent.switched` event.
pub async fn switch_agent<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    session_id: &str,
    agent: &str,
    cancellation: &Cancellation,
) -> Result<(), Failure> {
    let path = format!("/api/session/{}/agent", urlencoding(session_id));
    let payload = serde_json::json!({ "agent": agent });
    let response = exchange(
        client,
        post(&path, &payload),
        resolved.authorization_header().as_deref(),
        cancellation,
    )
    .await?;
    read_json(response, cancellation).await?;
    Ok(())
}

/// The path of the session's instruction entry `key`. The key is checked by the caller against OpenCode's pattern,
/// which needs no escaping.
fn instruction_path(session_id: &str, key: &str) -> String {
    format!(
        "/api/experimental/session/{}/instructions/entries/{}",
        urlencoding(session_id),
        urlencoding(key)
    )
}

/// `PUT /api/experimental/session/:id/instructions/entries/:key`: attach or replace one durable instruction entry,
/// which OpenCode announces to the model at its next step boundary. The value is the entry's text.
pub async fn put_instruction<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    session_id: &str,
    key: &str,
    value: &str,
    cancellation: &Cancellation,
) -> Result<(), Failure> {
    let payload = serde_json::json!({ "value": value });
    let request = with_json(Method::PUT, &instruction_path(session_id, key), &payload);
    let response = exchange(
        client,
        request,
        resolved.authorization_header().as_deref(),
        cancellation,
    )
    .await?;
    read_json(response, cancellation).await?;
    Ok(())
}

/// The path of the MCP server `name` registered at runtime for `directory`.
fn mcp_path(directory: &str, name: &str) -> String {
    format!(
        "/api/experimental/mcp/{}?location%5Bdirectory%5D={}",
        urlencoding(name),
        urlencoding(directory)
    )
}

/// `PUT /api/experimental/mcp/:server?location[directory]=`: add the remote MCP server `server` for `directory`, or
/// replace one of its name, connecting it at once. OpenCode keeps it until it restarts. Tomori's own server never needs
/// OAuth, so OpenCode is told not to look for it.
pub async fn put_mcp<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    directory: &str,
    server: &tomori_backend::ToolServer,
    cancellation: &Cancellation,
) -> Result<(), Failure> {
    let headers: serde_json::Map<String, Value> = server
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), Value::String(value.clone())))
        .collect();
    let payload = serde_json::json!({
        "config": {
            "type": "remote",
            "url": server.url,
            "headers": headers,
            "oauth": false,
        }
    });
    let request = with_json(Method::PUT, &mcp_path(directory, &server.name), &payload);
    let response = exchange(
        client,
        request,
        resolved.authorization_header().as_deref(),
        cancellation,
    )
    .await?;
    read_json(response, cancellation).await?;
    Ok(())
}

/// `DELETE /api/experimental/mcp/:server?location[directory]=`: stop the MCP server `name` for `directory` and remove
/// it until OpenCode restarts. OpenCode answers not found for one it does not have.
pub async fn delete_mcp<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    directory: &str,
    name: &str,
    cancellation: &Cancellation,
) -> Result<(), Failure> {
    let request = Request::new(
        Method::DELETE,
        Path::from(mcp_path(directory, name)),
        EmptyBody,
    );
    let response = exchange(
        client,
        request,
        resolved.authorization_header().as_deref(),
        cancellation,
    )
    .await?;
    read_json(response, cancellation).await?;
    Ok(())
}

/// `DELETE /api/experimental/session/:id/instructions/entries/:key`: remove one instruction entry, which OpenCode
/// announces to the model at its next step boundary.
pub async fn delete_instruction<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    session_id: &str,
    key: &str,
    cancellation: &Cancellation,
) -> Result<(), Failure> {
    let request = Request::new(
        Method::DELETE,
        Path::from(instruction_path(session_id, key)),
        EmptyBody,
    );
    let response = exchange(
        client,
        request,
        resolved.authorization_header().as_deref(),
        cancellation,
    )
    .await?;
    read_json(response, cancellation).await?;
    Ok(())
}

/// `DELETE /api/session/:id`: delete a session and its child sessions, which OpenCode answers with no content.
pub async fn delete_session<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    session_id: &str,
    cancellation: &Cancellation,
) -> Result<(), Failure> {
    let request = Request::new(
        Method::DELETE,
        Path::from(format!("/api/session/{}", urlencoding(session_id))),
        EmptyBody,
    );
    let response = exchange(
        client,
        request,
        resolved.authorization_header().as_deref(),
        cancellation,
    )
    .await?;
    read_json(response, cancellation).await?;
    Ok(())
}

/// `POST /api/session/:id/interrupt`: stop the running turn.
pub async fn interrupt<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    session_id: &str,
    cancellation: &Cancellation,
) -> Result<(), Failure> {
    let path = format!("/api/session/{}/interrupt", urlencoding(session_id));
    let response = exchange(
        client,
        post(&path, &Value::Object(serde_json::Map::new())),
        resolved.authorization_header().as_deref(),
        cancellation,
    )
    .await?;
    read_json(response, cancellation).await?;
    Ok(())
}

/// `GET /api/session/:id/permission`: the session's open permission requests.
pub async fn open_permissions<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    session_id: &str,
    cancellation: &Cancellation,
) -> Result<Vec<Value>, Failure> {
    let path = format!("/api/session/{}/permission", urlencoding(session_id));
    let response = exchange(
        client,
        get(&path),
        resolved.authorization_header().as_deref(),
        cancellation,
    )
    .await?;
    let value = read_json(response, cancellation).await?;
    value
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| Failure {
            status: None,
            message: "the server did not return a permission list".into(),
        })
}

/// `POST /api/session/:id/permission/:requestID/reply`: answer a permission.
/// `decision` is `once`, `always` or `reject`.
pub async fn reply_permission<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    session_id: &str,
    request_id: &str,
    decision: &str,
    cancellation: &Cancellation,
) -> Result<(), Failure> {
    let path = format!(
        "/api/session/{}/permission/{}/reply",
        urlencoding(session_id),
        urlencoding(request_id)
    );
    let payload = serde_json::json!({ "decision": decision });
    let response = exchange(
        client,
        post(&path, &payload),
        resolved.authorization_header().as_deref(),
        cancellation,
    )
    .await?;
    read_json(response, cancellation).await?;
    Ok(())
}

/// `GET /api/session/:id/form`: the session's open forms.
pub async fn open_forms<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    session_id: &str,
    cancellation: &Cancellation,
) -> Result<Vec<Value>, Failure> {
    let path = format!("/api/session/{}/form", urlencoding(session_id));
    let response = exchange(
        client,
        get(&path),
        resolved.authorization_header().as_deref(),
        cancellation,
    )
    .await?;
    let value = read_json(response, cancellation).await?;
    value
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| Failure {
            status: None,
            message: "the server did not return a form list".into(),
        })
}

/// `POST /api/session/:id/form/:formID/reply`: answer a form.
pub async fn reply_form<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    session_id: &str,
    form_id: &str,
    answer: &Value,
    cancellation: &Cancellation,
) -> Result<(), Failure> {
    let path = format!(
        "/api/session/{}/form/{}/reply",
        urlencoding(session_id),
        urlencoding(form_id)
    );
    let payload = serde_json::json!({ "answer": answer });
    let response = exchange(
        client,
        post(&path, &payload),
        resolved.authorization_header().as_deref(),
        cancellation,
    )
    .await?;
    read_json(response, cancellation).await?;
    Ok(())
}

/// `GET /api/session/:id/message?limit=N&order=desc`: a session's latest `limit` messages, oldest first, in one page.
/// See `design/events.md`.
pub async fn recent_messages<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    session_id: &str,
    limit: usize,
    cancellation: &Cancellation,
) -> Result<Vec<Value>, Failure> {
    let path = format!(
        "/api/session/{}/message?limit={limit}&order=desc",
        urlencoding(session_id)
    );
    let list = listed(client, resolved, &path, cancellation, "a message list").await?;
    Ok(list.into_iter().take(limit).rev().collect())
}

/// `GET /api/session/:id/inbox`: the messages queued for a session and not yet delivered to it. See `design/events.md`.
pub async fn session_inbox<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    session_id: &str,
    cancellation: &Cancellation,
) -> Result<Vec<Value>, Failure> {
    const MOST: usize = 64;
    let path = format!("/api/session/{}/inbox", urlencoding(session_id));
    let list = listed(client, resolved, &path, cancellation, "an inbox").await?;
    Ok(list.into_iter().take(MOST).collect())
}

/// `GET /api/session/:id/permission` and `GET /api/session/:id/form`: the permissions and forms a session waits on.
/// See `design/events.md`.
pub async fn session_requests<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    session_id: &str,
    cancellation: &Cancellation,
) -> Result<Vec<Value>, Failure> {
    const MOST: usize = 64;
    let session = urlencoding(session_id);
    let mut requests = listed(
        client,
        resolved,
        &format!("/api/session/{session}/permission"),
        cancellation,
        "a permission list",
    )
    .await?;
    requests.extend(
        listed(
            client,
            resolved,
            &format!("/api/session/{session}/form"),
            cancellation,
            "a form list",
        )
        .await?,
    );
    requests.truncate(MOST);
    Ok(requests)
}

/// The `data` list `path` answers, which is `what`.
async fn listed(
    client: &Client<'_>,
    resolved: &Resolved,
    path: &str,
    cancellation: &Cancellation,
    what: &str,
) -> Result<Vec<Value>, Failure> {
    let response = exchange(
        client,
        get(path),
        resolved.authorization_header().as_deref(),
        cancellation,
    )
    .await?;
    let mut value = read_json(response, cancellation).await?;
    match value.get_mut("data").map(Value::take) {
        Some(Value::Array(list)) => Ok(list),
        _ => Err(Failure {
            status: None,
            message: format!("the server did not return {what}"),
        }),
    }
}

/// `GET /api/session/:id/message`: bounded recent conversation history.
pub async fn session_messages<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    session_id: &str,
    cancellation: &Cancellation,
) -> Result<Value, Failure> {
    const MAX_PAGES: usize = 128;
    const MAX_MESSAGES: usize = 100;
    const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
    const MAX_MESSAGE_ID_BYTES: usize = 1024;
    let mut messages = Vec::new();
    let mut message_bytes = 0usize;
    let mut ids = std::collections::HashSet::new();
    let mut cursors = std::collections::HashSet::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let query = cursor.as_ref().map_or_else(
            || "limit=100&order=desc".to_owned(),
            |cursor| format!("limit=100&cursor={}", urlencoding(cursor)),
        );
        let path = format!("/api/session/{}/message?{query}", urlencoding(session_id));
        let response = exchange(
            client,
            get(&path),
            resolved.authorization_header().as_deref(),
            cancellation,
        )
        .await?;
        let value = read_json(response, cancellation).await?;
        let page = value
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| Failure {
                status: None,
                message: "the server did not return a message list".into(),
            })?;
        let next = value
            .get("cursor")
            .and_then(Value::as_object)
            .ok_or_else(|| Failure {
                status: None,
                message: "the server did not return a message cursor".into(),
            })?
            .get("next");
        let valid_next = match next {
            None | Some(Value::Null) => true,
            Some(Value::String(next)) => !next.is_empty(),
            _ => false,
        };
        if !valid_next {
            return Err(Failure {
                status: None,
                message: "the server returned an invalid message cursor".into(),
            });
        }
        for message in page {
            let id = message
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty() && id.len() <= MAX_MESSAGE_ID_BYTES)
                .ok_or_else(|| Failure {
                    status: None,
                    message: "the server returned a message without a supported identifier".into(),
                })?;
            if !ids.insert(id.to_owned()) {
                return Err(Failure {
                    status: None,
                    message: "the server repeated a message across pages".into(),
                });
            }
            if !valid_snapshot_message(message) {
                return Err(Failure {
                    status: None,
                    message: "the server returned an invalid session message".into(),
                });
            }
            if matches!(
                message.get("type").and_then(Value::as_str),
                Some("user" | "assistant")
            ) {
                let size = serde_json::to_vec(message)
                    .expect("a JSON value serializes")
                    .len();
                if !messages.is_empty() && message_bytes.saturating_add(size) > MAX_MESSAGE_BYTES {
                    return Ok(finish_messages(messages));
                }
                message_bytes = message_bytes.saturating_add(size);
                messages.push(message.clone());
                if messages.len() == MAX_MESSAGES {
                    return Ok(finish_messages(messages));
                }
            }
        }
        match next {
            None | Some(Value::Null) => return Ok(finish_messages(messages)),
            Some(Value::String(next)) if !next.is_empty() && cursors.insert(next.clone()) => {
                cursor = Some(next.clone())
            }
            _ => {
                return Err(Failure {
                    status: None,
                    message: "the server returned an invalid message cursor".into(),
                });
            }
        }
    }
    // Older history is intentionally omitted, as the server retains a
    // bounded recent conversation rather than every historical message.
    Ok(finish_messages(messages))
}

fn finish_messages(mut messages: Vec<Value>) -> Value {
    messages.reverse();
    serde_json::json!({"data": messages})
}

/// Conversation messages must be readable; known non-conversation messages
/// are valid but intentionally not projected as conversation text.
fn valid_snapshot_message(message: &Value) -> bool {
    match message.get("type").and_then(Value::as_str) {
        Some("user") => message.get("text").and_then(Value::as_str).is_some(),
        Some("assistant") => message
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(|parts| {
                parts
                    .iter()
                    .all(|part| match part.get("type").and_then(Value::as_str) {
                        Some("text" | "reasoning") => {
                            part.get("text").and_then(Value::as_str).is_some()
                        }
                        Some("tool") => {
                            part.get("id")
                                .and_then(Value::as_str)
                                .is_some_and(|id| !id.is_empty())
                                && part
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .is_some_and(|name| !name.is_empty())
                                && part.get("state").is_some_and(Value::is_object)
                        }
                        _ => false,
                    })
            }),
        Some(
            "agent-switched" | "model-switched" | "location-switched" | "synthetic" | "system"
            | "skill" | "shell" | "compaction" | "idle",
        ) => true,
        _ => false,
    }
}

/// The pieces of a session needed to reconstruct a [`tomori_backend::Snapshot`]:
/// its record, messages, open permissions and forms, and children.
pub struct SnapshotParts {
    pub messages: Value,
    pub permissions: Vec<Value>,
    pub forms: Vec<Value>,
    /// The descendants found by the session's [`FamilyWalk`], which
    /// [`SnapshotParts::fetch`] leaves to its caller.
    pub children: Vec<Value>,
    /// The record's running state; `None` if its timing is invalid.
    pub busy: Option<bool>,
    /// Child sessions may be background drains, whose absence from the active
    /// endpoint does not by itself establish that they are idle.
    pub has_parent: bool,
    /// The session's own title, as its record gives it, if it gives one.
    pub title: Option<String>,
    /// The directory the session is in now, as its record gives it, if it gives one.
    pub directory: Option<String>,
}

impl SnapshotParts {
    /// The snapshot's workers, derived from the children, oldest first: in the
    /// order their records say they were created, since neither the order they
    /// were discovered in nor their identifiers say how old they are.
    pub fn workers(&self) -> Vec<tomori_backend::Worker> {
        workers_from_children(&self.children)
    }

    /// All pending permissions, in the order the server returned them.
    pub fn permissions(&self) -> Vec<tomori_backend::Permission> {
        self.permissions
            .iter()
            .filter_map(crate::wire::permission_from_request)
            .collect()
    }

    /// All pending questions, in the order the server returned them.
    pub fn questions(&self) -> Vec<tomori_backend::Question> {
        self.forms.iter().map(question_from_form).collect()
    }

    /// Fetch every piece for `session_id` but its children, which a
    /// [`FamilyWalk`] lists, since they may take longer than the rest.
    /// `on_messages` is called just before the messages are requested, so a
    /// caller can tell which of the events it observed the listing must
    /// already reflect.
    pub async fn fetch<'scope>(
        client: &Client<'scope>,
        resolved: &Resolved,
        session_id: &str,
        cancellation: &Cancellation,
        on_messages: impl FnOnce(),
    ) -> Result<Self, Failure> {
        // Validate the session exists before fetching its parts, and record
        // whether it is currently running a turn (time.busy after time.idle).
        let info = session_info(client, resolved, session_id, cancellation).await?;
        if info.get("id").and_then(Value::as_str) != Some(session_id)
            || !valid_session_record(&info)
        {
            return Err(Failure {
                status: None,
                message: "the server returned an invalid session record".into(),
            });
        }
        let busy = running_from_record(&info);
        on_messages();
        let messages = session_messages(client, resolved, session_id, cancellation).await?;
        let permissions = open_permissions(client, resolved, session_id, cancellation).await?;
        let forms = open_forms(client, resolved, session_id, cancellation).await?;
        let mut permission_ids = std::collections::HashSet::new();
        if permissions.iter().any(|request| {
            request.get("sessionID").and_then(Value::as_str) != Some(session_id)
                || crate::wire::permission_from_request(request).is_none()
                || !request
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty() && permission_ids.insert(id))
                || !request
                    .get("resources")
                    .and_then(Value::as_array)
                    .is_some_and(|resources| resources.iter().all(Value::is_string))
        }) {
            return Err(Failure {
                status: None,
                message: "the server returned an invalid permission request".into(),
            });
        }
        let mut form_ids = std::collections::HashSet::new();
        if forms.iter().any(|form| {
            form.get("sessionID").and_then(Value::as_str) != Some(session_id)
                || !form
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty() && form_ids.insert(id))
                || form.get("title").and_then(Value::as_str).is_none()
                || !form
                    .get("fields")
                    .and_then(Value::as_array)
                    .is_some_and(|fields| {
                        !fields.is_empty()
                            && fields.iter().all(|field| {
                                field.get("key").and_then(Value::as_str).is_some()
                                    && field
                                        .get("type")
                                        .and_then(Value::as_str)
                                        .is_some_and(|kind| !kind.is_empty())
                            })
                    })
        }) {
            return Err(Failure {
                status: None,
                message: "the server returned an invalid form".into(),
            });
        }
        Ok(Self {
            messages,
            permissions,
            forms,
            children: Vec::new(),
            busy,
            has_parent: info.get("parentID").is_some(),
            title: info.get("title").and_then(Value::as_str).map(str::to_owned),
            directory: session_directory(&info).map(str::to_owned),
        })
    }
}

/// Whether the session is running: the process's active-session list when it
/// answers, otherwise the record's `time.busy` / `time.idle` timestamps.
///
/// The optional list is fetched alongside the record under a short bound,
/// `lookup`, [`ACTIVE_LOOKUP_TIMEOUT`] in production, so a stalled list adds
/// at most that bound to the record request.
pub async fn session_running<'scope, S: Timers>(
    client: &Client<'scope>,
    resolved: &Resolved,
    session_id: &str,
    cancellation: &Cancellation,
    timers: &S,
    lookup: Duration,
) -> Result<Option<bool>, Failure> {
    let (info, active) = futures_lite::future::zip(
        session_info(client, resolved, session_id, cancellation),
        active_sessions_with_timeout(client, resolved, cancellation, timers, lookup),
    )
    .await;
    let info = info?;
    if !valid_session_record(&info) {
        return Ok(None);
    }
    let running = match active {
        Some(active) if active.contains(session_id) => Some(true),
        Some(_) if info.get("parentID").is_none() => Some(false),
        _ => running_from_record(&info),
    };
    Ok(running)
}

/// A session's first running turn has `busy` but no earlier `idle` timestamp.
pub(crate) fn running_from_record(info: &Value) -> Option<bool> {
    let time = info.get("time")?.as_object()?;
    let timestamp = |key| match time.get(key) {
        None => Some(None),
        Some(value) => value.as_f64().map(Some),
    };
    match (timestamp("busy")?, timestamp("idle")?) {
        (Some(busy), Some(idle)) => Some(busy > idle),
        (Some(_), None) => Some(true),
        (None, _) => Some(false),
    }
}

/// Percent-encode the segment of a path: ids and directories are opaque.
fn urlencoding(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests;
