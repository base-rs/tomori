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
use base_scheduler::Cancellation;
use serde_json::Value;

use crate::endpoint::Resolved;
use crate::wire::question_from_form;

/// Bound the backend's worker cache. Exceeding it is a visible incomplete
/// snapshot, never an authoritative truncated list.
pub(crate) const MAX_TRACKED_WORKERS: usize = 4096;
/// A single agent response must not grow without bound while it is read.
const MAX_JSON_BYTES: usize = 16 * 1024 * 1024;

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
    if let Some(authorization) = auth {
        let field = Field::new("authorization", authorization).map_err(|error| Failure {
            status: None,
            message: error.to_string(),
        })?;
        request.fields.append(field).map_err(|error| Failure {
            status: None,
            message: error.to_string(),
        })?;
    }
    // Opt out of connection reuse: opencode closes idle keep-alive connections
    // after five seconds, and `base-http-client` pools and reuses connections
    // without retrying. A request that reuses a connection opencode already
    // dropped reads EOF and fails with "truncated HTTP/1 message" (base-rs/
    // base-http#36). `Connection: close` keeps the client from reusing: each
    // call gets a fresh connection, which stays valid for the single exchange.
    if let Ok(field) = Field::new("connection", "close") {
        request.fields.append(field).map_err(|error| Failure {
            status: None,
            message: error.to_string(),
        })?;
    }
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
    let mut request = Request::new(
        Method::POST,
        Path::from(path.to_owned()),
        BufferedBody::from(payload.to_string().into_bytes()),
    );
    if let Ok(field) = Field::new("content-type", "application/json") {
        let _ = request.fields.append(field);
    }
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

/// `GET /api/session?directory=`: the existing sessions in a directory.
pub async fn list_sessions<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    directory: &str,
    cancellation: &Cancellation,
) -> Result<Vec<Value>, Failure> {
    let path = format!("/api/session?directory={}", urlencoding(directory));
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
                || !valid_child_record(child)
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

/// Discover descendants through direct-child pages. A repeated identity or a
/// limit failure rejects the snapshot rather than returning a partial family.
async fn session_family<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    session_id: &str,
    cancellation: &Cancellation,
) -> Result<Vec<Value>, Failure> {
    let mut pending = std::collections::VecDeque::from([session_id.to_owned()]);
    let mut seen = std::collections::HashSet::from([session_id.to_owned()]);
    let mut family = Vec::new();
    while let Some(parent) = pending.pop_front() {
        for child in child_sessions(client, resolved, &parent, cancellation).await? {
            let id = child
                .get("id")
                .and_then(Value::as_str)
                .expect("validated child identifier");
            if !seen.insert(id.to_owned()) {
                return Err(Failure {
                    status: None,
                    message: "the server returned a repeated or cyclic worker identity".into(),
                });
            }
            if family.len() == MAX_TRACKED_WORKERS {
                return Err(Failure {
                    status: None,
                    message: "the worker family exceeds the worker limit".into(),
                });
            }
            pending.push_back(id.to_owned());
            family.push(child);
        }
    }
    Ok(family)
}

/// Required fields of OpenCode's Session.Info, checked before a record may
/// participate in a worker snapshot. Extra fields are accepted for upgrades.
fn valid_child_record(child: &Value) -> bool {
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

/// `GET /api/session/:id/message`: the session's messages, for a snapshot.
pub async fn session_messages<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    session_id: &str,
    cancellation: &Cancellation,
) -> Result<Value, Failure> {
    const MAX_PAGES: usize = 128;
    let mut messages = Vec::new();
    let mut ids = std::collections::HashSet::new();
    let mut cursors = std::collections::HashSet::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let query = cursor.as_ref().map_or_else(
            || "limit=100&order=asc".to_owned(),
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
        for message in page {
            let id = message
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| Failure {
                    status: None,
                    message: "the server returned a message without an identifier".into(),
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
            messages.push(message.clone());
        }
        let next = value
            .get("cursor")
            .and_then(Value::as_object)
            .ok_or_else(|| Failure {
                status: None,
                message: "the server did not return a message cursor".into(),
            })?
            .get("next");
        match next {
            None | Some(Value::Null) => return Ok(serde_json::json!({"data": messages})),
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
    Err(Failure {
        status: None,
        message: "the message list exceeds the page limit".into(),
    })
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

/// The pieces of a session needed to reconstruct a [`backend::Snapshot`]:
/// its record, messages, open permissions and forms, and children.
pub struct SnapshotParts {
    pub messages: Value,
    pub permissions: Vec<Value>,
    pub forms: Vec<Value>,
    pub children: Vec<Value>,
    /// Whether the session is currently running a turn, derived from the
    /// session record's `time.busy` / `time.idle` timestamps.
    pub busy: bool,
}

impl SnapshotParts {
    /// The snapshot's workers, derived from the children.
    pub fn workers(&self) -> Vec<backend::Worker> {
        self.children
            .iter()
            .filter_map(crate::wire::worker_from_session)
            .collect()
    }

    /// All pending permissions, in the order the server returned them.
    pub fn permissions(&self) -> Vec<backend::Permission> {
        self.permissions
            .iter()
            .filter_map(crate::wire::permission_from_request)
            .collect()
    }

    /// All pending questions, in the order the server returned them.
    pub fn questions(&self) -> Vec<backend::Question> {
        self.forms.iter().map(question_from_form).collect()
    }

    /// Fetch every piece for `session_id`.
    pub async fn fetch<'scope>(
        client: &Client<'scope>,
        resolved: &Resolved,
        session_id: &str,
        cancellation: &Cancellation,
    ) -> Result<Self, Failure> {
        // Validate the session exists before fetching its parts, and record
        // whether it is currently running a turn (time.busy after time.idle).
        let info = session_info(client, resolved, session_id, cancellation).await?;
        if info.get("id").and_then(Value::as_str) != Some(session_id) {
            return Err(Failure {
                status: None,
                message: "the server returned an invalid session record".into(),
            });
        }
        let busy = running_from_record(&info).ok_or_else(|| Failure {
            status: None,
            message: "the server returned invalid session timing".into(),
        })?;
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
                                field
                                    .get("key")
                                    .and_then(Value::as_str)
                                    .is_some_and(|key| !key.is_empty())
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
        let children = session_family(client, resolved, session_id, cancellation).await?;
        Ok(Self {
            messages,
            permissions,
            forms,
            children,
            busy,
        })
    }
}

/// The session record's running state, from its `time.busy` / `time.idle`
/// timestamps: running when the last turn started after its last end.
pub async fn session_running<'scope>(
    client: &Client<'scope>,
    resolved: &Resolved,
    session_id: &str,
    cancellation: &Cancellation,
) -> Result<Option<bool>, Failure> {
    let info = session_info(client, resolved, session_id, cancellation).await?;
    let running = running_from_record(&info);
    Ok(running)
}

/// A session's first running turn has `busy` but no earlier `idle` timestamp.
fn running_from_record(info: &Value) -> Option<bool> {
    let time = info.get("time")?.as_object()?;
    let timestamp = |key| match time.get(key) {
        None => Some(None),
        Some(value) => value.as_u64().map(Some),
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
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn response_bytes_are_bounded_without_truncating_valid_json() {
        let mut bytes = vec![b'x'; MAX_JSON_BYTES - 1];
        assert!(append_json_bytes(&mut bytes, b"y", 200).is_ok());
        assert_eq!(bytes.len(), MAX_JSON_BYTES);
        assert!(append_json_bytes(&mut bytes, b"z", 200).is_err());
        assert_eq!(bytes.len(), MAX_JSON_BYTES);
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
            running_from_record(&json!({"time":{"busy":10,"idle":11}})),
            Some(false)
        );
        assert_eq!(running_from_record(&json!({"time":{}})), Some(false));
        assert_eq!(
            running_from_record(&json!({"time":{"busy":"invalid"}})),
            None
        );
        assert_eq!(running_from_record(&json!({})), None);
    }
}
