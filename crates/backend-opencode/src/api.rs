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

/// The listing of sessions a participant can attach to: the directory's own
/// top-level, unarchived sessions. Subagents' child sessions are workers of
/// their parent, not conversations to join.
fn session_list_path(directory: &str) -> String {
    format!(
        "/api/session?directory={}&parentID=null&archived=false",
        urlencoding(directory)
    )
}

/// `GET /api/session?directory=&parentID=null&archived=false`: the existing
/// top-level, unarchived sessions in a directory.
pub async fn list_sessions<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    directory: &str,
    cancellation: &Cancellation,
) -> Result<Vec<Value>, Failure> {
    let path = session_list_path(directory);
    let response = exchange(
        client,
        get(&path),
        resolved.authorization_header().as_deref(),
        cancellation,
    )
    .await?;
    let value = read_json(response, cancellation).await?;
    Ok(value
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
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

/// How long the optional active-session lookup may take alongside the
/// session record during reconciliation.
const ACTIVE_LOOKUP_TIMEOUT: Duration = Duration::from_secs(2);

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
    /// within [`Self::LISTING_TIMEOUT`] fails the walk. The future may be
    /// dropped at any point, or fail: a later call lists again the parents
    /// whose listing it abandoned or that failed.
    pub(crate) async fn run<'scope, S: Timers>(
        &mut self,
        client: &Client<'scope>,
        resolved: &Resolved,
        cancellation: &Cancellation,
        timers: &S,
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
                            let _ = timers.sleep(Self::LISTING_TIMEOUT, cancellation).await;
                            Err(Failure {
                                status: None,
                                message: format!(
                                    "OpenCode did not list a worker's children within {} seconds",
                                    Self::LISTING_TIMEOUT.as_secs()
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
        })
    }
}

/// Whether the session is running: the process's active-session list when it
/// answers, otherwise the record's `time.busy` / `time.idle` timestamps.
///
/// The optional list is fetched alongside the record under a short bound, so
/// a stalled list adds at most that bound to the record request.
pub async fn session_running<'scope, S: Timers>(
    client: &Client<'scope>,
    resolved: &Resolved,
    session_id: &str,
    cancellation: &Cancellation,
    timers: &S,
) -> Result<Option<bool>, Failure> {
    let (info, active) = futures_lite::future::zip(
        session_info(client, resolved, session_id, cancellation),
        active_sessions_with_timeout(
            client,
            resolved,
            cancellation,
            timers,
            ACTIVE_LOOKUP_TIMEOUT,
        ),
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
fn running_from_record(info: &Value) -> Option<bool> {
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
pub(crate) mod tests {
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
            let (status, message) =
                failed(session_info(client, resolved, "s1", cancellation).await);
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
                assert_eq!(
                    failed(interrupt(client, resolved, "s1", cancellation).await).0,
                    None
                );
                assert_eq!(
                    failed(open_permissions(client, resolved, "s1", cancellation).await).0,
                    None
                );
                assert_eq!(
                    failed(
                        reply_permission(client, resolved, "s1", "p1", "once", cancellation).await
                    )
                    .0,
                    None
                );
                assert_eq!(
                    failed(open_forms(client, resolved, "s1", cancellation).await).0,
                    None
                );
                assert_eq!(
                    failed(
                        reply_form(client, resolved, "s1", "f1", &json!({}), cancellation).await
                    )
                    .0,
                    None
                );
                assert_eq!(
                    failed(session_messages(client, resolved, "s1", cancellation).await).0,
                    None
                );
                assert_eq!(
                    failed(session_running(client, resolved, "s1", cancellation, scheduler).await)
                        .0,
                    None
                );
            },
        );
    }

    #[test]
    fn session_listing_asks_for_top_level_unarchived_sessions() {
        assert_eq!(
            session_list_path("/home/a b"),
            "/api/session?directory=%2Fhome%2Fa%20b&parentID=null&archived=false"
        );
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
            .chain(
                (0..100)
                    .map(|index| json!({"id": format!("m{index}"), "type": "user", "text": ""})),
            )
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

    #[test]
    fn active_session_map_requires_valid_running_records() {
        let active = parse_active_sessions(&json!({"data":{"ses_a":{"type":"running"}}})).unwrap();
        assert!(active.contains("ses_a"));
        assert!(parse_active_sessions(&json!({"data":[]})).is_err());
        assert!(parse_active_sessions(&json!({"data":{"ses_a":{"type":"idle"}}})).is_err());
    }
}
