// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Mapping between the OpenCode V2 event and resource shapes and the backend's
//! neutral vocabulary.
//!
//! The OpenCode server reports everything on one global event stream,
//! addressed by `sessionID`; these functions turn one event into the backend
//! events a single session should see, `None` when the event is not for it.
//! Resource listings are turned into permissions, questions and workers the
//! same way, so attaching to a session recovers what live events would have
//! reported.
use serde_json::Value;
use tomori_backend::{
    AnswerValue, Event, Field, FieldKind, FieldOption, Grant, GrantScope, Mode, Permission,
    PermissionOption, Question, Role, Worker, WorkerState,
};

/// The session a server event names, if it names one.
pub fn session_id_of(value: &Value) -> Option<String> {
    let data = value.get("data")?;
    data.get("sessionID")
        .or_else(|| data.get("form").and_then(|form| form.get("sessionID")))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Turn one OpenCode event into the backend events to emit for `session_id`,
/// or `None` when the event names another session. `seen_messages` records the
/// messages a session has already reported starting, so each message gets a
/// single [`Event::MessageStarted`].
pub fn event_for(
    value: &Value,
    session_id: &str,
    seen_messages: &mut std::collections::HashSet<String>,
) -> Option<Event> {
    let data = value.get("data")?;
    match value.get("type")?.as_str()? {
        "session.text.started" => first_seen(data, session_id, seen_messages),
        "session.text.delta" => text_delta(data, session_id, "delta"),
        // Reasoning is not spoken: the speech stream reads only the
        // assistant's text, and the space model records reasoning separately
        // from the snapshot.
        "session.reasoning.delta" => None,
        // `session.text.ended` names the message that just finished. It fires
        // once, and only after the `session.text.started` that already
        // recorded this id in `seen_messages`, so it must not be gated on
        // `first_seen` (which would return `None` and swallow the completion).
        // Without `MessageCompleted` the reply's final unclosed sentence is
        // never flushed to speech.
        "session.text.ended" => {
            if !for_session(data, session_id) {
                return None;
            }
            Some(Event::MessageCompleted {
                message: message_id(data)?,
            })
        }
        // A tool can be called after the message's text has started, so its
        // start is reported whether or not the message has been seen. A tool
        // that starts a message stands for its start: the space adds the
        // message it names.
        "session.tool.called" => {
            let started = tool_started(data, session_id)?;
            first_seen(data, session_id, seen_messages);
            Some(started)
        }
        "session.tool.success" => tool_finished(data, session_id, None),
        "session.tool.failed" => tool_finished(data, session_id, Some(data)),
        "permission.asked" => permission_asked(data, session_id),
        "permission.replied" => permission_replied(data, session_id),
        "form.created" => form_created(data, session_id),
        "form.replied" | "form.cancelled" => form_resolved(data, session_id),
        "session.inbox.delivered" => inbox_delivered(data, session_id),
        "session.status" => status(data, session_id),
        "session.idle" => idle(data, session_id),
        "session.execution.started" => busy_like(data, session_id, Event::Busy),
        "session.execution.interrupted" => busy_like(data, session_id, Event::Idle),
        "session.execution.succeeded" => busy_like(data, session_id, Event::Idle),
        "session.execution.failed" => execution_failed(data, session_id),
        // A registered session ending closes its own space. The connection
        // also routes the same deletion to any parent that knows it as a worker.
        "session.deleted" => for_session(data, session_id).then(|| Event::Closed {
            reason: Some("the OpenCode session was deleted".into()),
        }),
        // The session's own title; a worker's is routed to its parent by the connection.
        "session.renamed" => {
            if !for_session(data, session_id) {
                return None;
            }
            Some(Event::TitleChanged {
                title: data.get("title")?.as_str()?.to_owned(),
            })
        }
        // Child creation is routed to its parent by the connection.
        "session.created" | "session.forked" => None,
        _ => None,
    }
}

fn for_session(data: &Value, session_id: &str) -> bool {
    data.get("sessionID").and_then(Value::as_str) == Some(session_id)
}

/// Report the message before anything that names it, at most once.
fn first_seen(
    data: &Value,
    session_id: &str,
    seen_messages: &mut std::collections::HashSet<String>,
) -> Option<Event> {
    if !for_session(data, session_id) {
        return None;
    }
    let message = message_id(data)?;
    if !seen_messages.insert(message.clone()) {
        return None;
    }
    Some(Event::MessageStarted {
        message,
        role: Role::Assistant,
    })
}

fn text_delta(data: &Value, session_id: &str, field: &str) -> Option<Event> {
    if !for_session(data, session_id) {
        return None;
    }
    Some(Event::TextDelta {
        message: message_id(data)?,
        text: data.get(field)?.as_str()?.to_owned(),
    })
}

fn tool_started(data: &Value, session_id: &str) -> Option<Event> {
    if !for_session(data, session_id) {
        return None;
    }
    Some(Event::ToolStarted {
        message: message_id(data)?,
        // The V2 stream does not name the tool on the call event; the history
        // does, and the id is what the completion events are matched by.
        tool: String::new(),
        id: data.get("id")?.as_str()?.to_owned(),
        input: data.get("input").cloned().unwrap_or(Value::Null),
    })
}

fn tool_finished(data: &Value, session_id: &str, failed: Option<&Value>) -> Option<Event> {
    if !for_session(data, session_id) {
        return None;
    }
    let error = failed.and_then(|data| {
        data.get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    });
    Some(Event::ToolFinished {
        id: data.get("id")?.as_str()?.to_owned(),
        error,
    })
}

/// A permission request object (from the list endpoint or the event stream's
/// `data`) as a [`Permission`].
pub fn permission_from_request(request: &Value) -> Option<Permission> {
    Some(Permission {
        id: request.get("id")?.as_str()?.to_owned(),
        tool: request.get("action")?.as_str()?.to_owned(),
        input: Value::Object(
            request
                .get("resources")
                .and_then(Value::as_array)
                .map(|resources| {
                    resources
                        .iter()
                        .enumerate()
                        .map(|(index, resource)| (index.to_string(), resource.clone()))
                        .collect()
                })
                .unwrap_or_default(),
        ),
        description: request
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_owned),
        options: request
            .get("save")
            .and_then(Value::as_array)
            .map(|save| {
                save.iter()
                    .enumerate()
                    .filter_map(|(index, rule)| {
                        let rule = rule.as_str()?;
                        let (tool, content) = rule.split_once(' ')?;
                        Some(PermissionOption {
                            id: index.to_string(),
                            grants: vec![Grant::Rule {
                                tool: tool.to_owned(),
                                content: Some(content.to_owned()),
                                scope: GrantScope::Session,
                            }],
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

fn permission_asked(data: &Value, session_id: &str) -> Option<Event> {
    if !for_session(data, session_id) {
        return None;
    }
    permission_from_request(data).map(Event::PermissionRequested)
}

fn permission_replied(data: &Value, session_id: &str) -> Option<Event> {
    if !for_session(data, session_id) {
        return None;
    }
    Some(Event::PermissionResolved {
        id: data.get("requestID")?.as_str()?.to_owned(),
    })
}

fn form_created(data: &Value, session_id: &str) -> Option<Event> {
    let form = data.get("form")?;
    if form.get("sessionID").and_then(Value::as_str) != Some(session_id) {
        return None;
    }
    Some(Event::QuestionAsked(question_from_form(form)))
}

fn form_resolved(data: &Value, session_id: &str) -> Option<Event> {
    let (id, session) = match data.get("id").and_then(Value::as_str) {
        Some(id) => (
            id.to_owned(),
            data.get("sessionID").and_then(Value::as_str)?.to_owned(),
        ),
        None => {
            let form = data.get("form")?;
            (
                form.get("id")?.as_str()?.to_owned(),
                form.get("sessionID")?.as_str()?.to_owned(),
            )
        }
    };
    if session != session_id {
        return None;
    }
    Some(Event::QuestionResolved { id })
}

fn inbox_delivered(data: &Value, session_id: &str) -> Option<Event> {
    if !for_session(data, session_id) {
        return None;
    }
    // The `inboxID` is the `msg_`-prefixed id we sent in `send_prompt`.
    // Strip the prefix to recover the server's delivery key.
    let inbox_id = data.get("inboxID")?.as_str()?;
    let key = inbox_id.strip_prefix("msg_").unwrap_or(inbox_id);
    Some(Event::Delivered {
        key: key.to_owned(),
    })
}

fn status(data: &Value, session_id: &str) -> Option<Event> {
    if !for_session(data, session_id) {
        return None;
    }
    if data.get("status")?.get("type")?.as_str()? == "busy" {
        Some(Event::Busy)
    } else {
        None
    }
}

fn idle(data: &Value, session_id: &str) -> Option<Event> {
    for_session(data, session_id).then_some(Event::Idle)
}

fn busy_like(data: &Value, session_id: &str, event: Event) -> Option<Event> {
    for_session(data, session_id).then_some(event)
}

fn execution_failed(data: &Value, session_id: &str) -> Option<Event> {
    if !for_session(data, session_id) {
        return None;
    }
    Some(Event::Error {
        message: data
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("the agent execution failed")
            .to_owned(),
    })
}

fn message_id(data: &Value) -> Option<String> {
    data.get("assistantMessageID")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// A form as a structured question.
pub fn question_from_form(form: &Value) -> Question {
    let fields = form
        .get("fields")
        .and_then(Value::as_array)
        .map(|fields| {
            fields
                .iter()
                .filter_map(field_from_form_field)
                .collect::<Vec<Field>>()
        })
        .unwrap_or_default();
    Question {
        id: form
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        title: form
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        fields,
    }
}

fn field_from_form_field(field: &Value) -> Option<Field> {
    let key = field.get("key")?.as_str()?.to_owned();
    let (kind, options) = match field.get("type")?.as_str()? {
        "string" => {
            let options = field
                .get("options")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if options.is_empty() {
                (FieldKind::Text, Vec::new())
            } else {
                (FieldKind::Choice, options)
            }
        }
        "number" | "integer" => (FieldKind::Text, Vec::new()),
        "boolean" => (
            FieldKind::Choice,
            vec![Value::String("true".into()), Value::String("false".into())],
        ),
        "multiselect" => (
            FieldKind::MultipleChoice,
            field
                .get("options")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        ),
        // Completed outside Tomori, in the linked tool.
        "external" => (FieldKind::Unsupported, Vec::new()),
        _ => (FieldKind::Unsupported, Vec::new()),
    };
    let options = field_options(&options);
    Some(Field {
        key: key.clone(),
        title: field
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .filter(|title| !title.is_empty())
            .unwrap_or(key),
        description: field
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned),
        kind,
        options,
        custom: field
            .get("custom")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn field_options(values: &[Value]) -> Vec<FieldOption> {
    values
        .iter()
        .map(|option| {
            let label = if let Some(label) = option.as_str() {
                label.to_owned()
            } else {
                option
                    .get("label")
                    .and_then(Value::as_str)
                    .map_or_else(|| option.to_string(), str::to_owned)
            };
            let description = option
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_owned);
            FieldOption { label, description }
        })
        .collect()
}

/// A child session as a delegated worker.
pub fn worker_from_session(info: &Value) -> Option<Worker> {
    let time = info.get("time");
    let busy = time
        .and_then(|time| time.get("busy"))
        .and_then(Value::as_f64);
    let idle = time
        .and_then(|time| time.get("idle"))
        .and_then(Value::as_f64);
    let outcome = match info.get("outcome").and_then(Value::as_str) {
        Some("succeeded") => WorkerState::Completed,
        Some("failed") => WorkerState::Error,
        Some("interrupted") => WorkerState::Stopped,
        _ => WorkerState::Unknown,
    };
    let state = match (busy, idle) {
        (Some(busy), Some(idle)) if busy > idle => WorkerState::Running,
        (Some(busy), Some(idle)) if busy == idle => WorkerState::Unknown,
        (Some(_), None) => WorkerState::Running,
        _ => outcome,
    };
    Some(Worker {
        id: info.get("id")?.as_str()?.to_owned(),
        parent: info
            .get("parentID")
            .and_then(Value::as_str)
            .map(str::to_owned),
        title: info
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("worker")
            .to_owned(),
        state,
    })
}

/// The answers to a question, in the OpenCode reply's shape: an object
/// mapping field keys to strings, arrays of strings, numbers or booleans.
pub fn form_answer(answers: &[(String, AnswerValue)]) -> Value {
    let mut object = serde_json::Map::new();
    for (key, value) in answers {
        let value = match value {
            AnswerValue::Text(text) => Value::String(text.clone()),
            AnswerValue::Choices(choices) => Value::Array(
                choices
                    .iter()
                    .map(|choice| Value::String(choice.clone()))
                    .collect(),
            ),
        };
        object.insert(key.clone(), value);
    }
    Value::Object(object)
}

/// The modes OpenCode's terminal steps through with Tab: its primary agents, in its order, leaving out subagents and
/// the hidden agents it runs itself (compaction, titles and summaries).
pub fn modes_from_agents(agents: &[Value]) -> Vec<Mode> {
    agents
        .iter()
        .filter(|agent| agent.get("mode").and_then(Value::as_str) != Some("subagent"))
        .filter(|agent| agent.get("hidden").and_then(Value::as_bool) != Some(true))
        .filter_map(|agent| {
            let id = agent.get("id").and_then(Value::as_str)?.to_owned();
            Some(Mode {
                // The agent listing has no separate display name; the terminal shows the id.
                name: id.clone(),
                id,
                description: agent
                    .get("description")
                    .and_then(Value::as_str)
                    .filter(|description| !description.is_empty())
                    .map(str::to_owned),
            })
        })
        .collect()
}

/// The agent a session's prompts go to: the one recorded on the session once a participant chose it, otherwise
/// OpenCode's default, the first agent it offers.
pub fn current_mode(info: &Value, offered: &[Mode]) -> Option<String> {
    info.get("agent")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| offered.first().map(|mode| mode.id.clone()))
}

/// The agent an event reports the session switched to, if it reports a switch for `session_id`.
pub fn agent_switched(value: &Value, session_id: &str) -> Option<String> {
    let data = value.get("data")?;
    match value.get("type")?.as_str()? {
        "session.next.agent.switched" | "session.agent.switched" | "session.agent.selected"
            if for_session(data, session_id) =>
        {
            data.get("agent").and_then(Value::as_str).map(str::to_owned)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resumed_child_is_not_mistaken_for_its_previous_outcome() {
        let mut info = serde_json::json!({"id":"child", "outcome":"succeeded", "time":{"busy":101,"idle":100}});
        assert_eq!(
            worker_from_session(&info).unwrap().state,
            WorkerState::Running
        );
        info["time"]["busy"] = serde_json::json!(100);
        assert_eq!(
            worker_from_session(&info).unwrap().state,
            WorkerState::Unknown
        );
        info["time"]["busy"] = serde_json::json!(99);
        assert_eq!(
            worker_from_session(&info).unwrap().state,
            WorkerState::Completed
        );
    }

    #[test]
    fn a_child_without_execution_evidence_is_not_assumed_running() {
        let info = serde_json::json!({"id":"child", "time":{"created":1,"updated":1}});
        assert_eq!(
            worker_from_session(&info).unwrap().state,
            WorkerState::Unknown
        );
    }

    #[test]
    fn renaming_the_registered_session_retitles_it_and_a_rename_without_a_title_is_none() {
        let renamed =
            serde_json::json!({"type":"session.renamed","data":{"sessionID":"s","title":"New"}});
        assert_eq!(
            event_for(&renamed, "s", &mut Default::default()),
            Some(Event::TitleChanged {
                title: "New".into()
            })
        );
        assert!(event_for(&renamed, "other", &mut Default::default()).is_none());
        let untitled = serde_json::json!({"type":"session.renamed","data":{"sessionID":"s"}});
        assert!(event_for(&untitled, "s", &mut Default::default()).is_none());
        let numbered =
            serde_json::json!({"type":"session.renamed","data":{"sessionID":"s","title":1}});
        assert!(event_for(&numbered, "s", &mut Default::default()).is_none());
    }

    #[test]
    fn deleting_the_registered_session_closes_its_space() {
        let event = serde_json::json!({"type":"session.deleted","data":{"sessionID":"parent"}});
        assert_eq!(
            event_for(&event, "parent", &mut Default::default()),
            Some(Event::Closed {
                reason: Some("the OpenCode session was deleted".into())
            })
        );
        assert!(event_for(&event, "other", &mut Default::default()).is_none());
    }

    /// An event of `kind` whose data is `data` with `"sessionID": "s"` added.
    fn for_s(kind: &str, mut data: Value) -> Value {
        data["sessionID"] = "s".into();
        serde_json::json!({"type": kind, "data": data})
    }

    /// Each OpenCode event, as session `s` sees it and as another session does.
    #[test]
    fn each_event_becomes_the_backend_event_for_its_session_only() {
        use serde_json::json;
        let cases = [
            (
                for_s("session.text.started", json!({"assistantMessageID":"m"})),
                Some(Event::MessageStarted {
                    message: "m".into(),
                    role: Role::Assistant,
                }),
            ),
            (
                for_s(
                    "session.tool.called",
                    json!({"assistantMessageID":"m","id":"t1"}),
                ),
                Some(Event::ToolStarted {
                    message: "m".into(),
                    tool: String::new(),
                    id: "t1".into(),
                    input: Value::Null,
                }),
            ),
            (
                for_s(
                    "session.text.delta",
                    json!({"assistantMessageID":"m","delta":"Hi"}),
                ),
                Some(Event::TextDelta {
                    message: "m".into(),
                    text: "Hi".into(),
                }),
            ),
            (
                for_s(
                    "session.reasoning.delta",
                    json!({"assistantMessageID":"m","delta":"Hmm"}),
                ),
                None,
            ),
            (
                for_s("session.text.ended", json!({"assistantMessageID":"m"})),
                Some(Event::MessageCompleted {
                    message: "m".into(),
                }),
            ),
            (
                for_s("session.tool.success", json!({"id":"t1"})),
                Some(Event::ToolFinished {
                    id: "t1".into(),
                    error: None,
                }),
            ),
            (
                for_s(
                    "session.tool.failed",
                    json!({"id":"t1","error":{"message":"denied"}}),
                ),
                Some(Event::ToolFinished {
                    id: "t1".into(),
                    error: Some("denied".into()),
                }),
            ),
            (
                for_s("session.tool.failed", json!({"id":"t1"})),
                Some(Event::ToolFinished {
                    id: "t1".into(),
                    error: None,
                }),
            ),
            (
                for_s(
                    "permission.asked",
                    json!({"id":"p1","action":"bash","resources":["ls"],"message":"List files","save":["bash ls*", "malformed", 3]}),
                ),
                Some(Event::PermissionRequested(Permission {
                    id: "p1".into(),
                    tool: "bash".into(),
                    input: json!({"0": "ls"}),
                    description: Some("List files".into()),
                    options: vec![PermissionOption {
                        id: "0".into(),
                        grants: vec![Grant::Rule {
                            tool: "bash".into(),
                            content: Some("ls*".into()),
                            scope: GrantScope::Session,
                        }],
                    }],
                })),
            ),
            (
                for_s("permission.asked", json!({"id":"p1","action":"edit"})),
                Some(Event::PermissionRequested(Permission {
                    id: "p1".into(),
                    tool: "edit".into(),
                    input: json!({}),
                    description: None,
                    options: Vec::new(),
                })),
            ),
            (
                for_s("permission.replied", json!({"requestID":"p1"})),
                Some(Event::PermissionResolved { id: "p1".into() }),
            ),
            (
                json!({"type":"form.created","data":{"form":{"id":"f1","sessionID":"s","title":"Choose","fields":[]}}}),
                Some(Event::QuestionAsked(Question {
                    id: "f1".into(),
                    title: "Choose".into(),
                    fields: Vec::new(),
                })),
            ),
            (
                for_s("form.replied", json!({"id":"f1"})),
                Some(Event::QuestionResolved { id: "f1".into() }),
            ),
            (
                json!({"type":"form.cancelled","data":{"form":{"id":"f1","sessionID":"s"}}}),
                Some(Event::QuestionResolved { id: "f1".into() }),
            ),
            (
                for_s("session.inbox.delivered", json!({"inboxID":"msg_k1"})),
                Some(Event::Delivered { key: "k1".into() }),
            ),
            (
                for_s("session.inbox.delivered", json!({"inboxID":"k2"})),
                Some(Event::Delivered { key: "k2".into() }),
            ),
            (
                for_s("session.status", json!({"status":{"type":"busy"}})),
                Some(Event::Busy),
            ),
            (
                for_s("session.status", json!({"status":{"type":"retry"}})),
                None,
            ),
            (for_s("session.idle", json!({})), Some(Event::Idle)),
            (
                for_s("session.execution.started", json!({})),
                Some(Event::Busy),
            ),
            (
                for_s("session.execution.interrupted", json!({})),
                Some(Event::Idle),
            ),
            (
                for_s("session.execution.succeeded", json!({})),
                Some(Event::Idle),
            ),
            (
                for_s(
                    "session.execution.failed",
                    json!({"error":{"message":"out of credit"}}),
                ),
                Some(Event::Error {
                    message: "out of credit".into(),
                }),
            ),
            (
                for_s("session.execution.failed", json!({})),
                Some(Event::Error {
                    message: "the agent execution failed".into(),
                }),
            ),
            (for_s("session.created", json!({"id":"child"})), None),
            (for_s("session.forked", json!({"id":"child"})), None),
            (for_s("session.compacted", json!({})), None),
        ];
        for (value, expected) in cases {
            assert_eq!(
                event_for(&value, "s", &mut Default::default()),
                expected,
                "{value}"
            );
            assert_eq!(
                event_for(&value, "other", &mut Default::default()),
                None,
                "{value}"
            );
            assert_eq!(session_id_of(&value).as_deref(), Some("s"), "{value}");
        }
    }

    /// Events missing what they must carry are not reported, rather than reported with something made up.
    #[test]
    fn incomplete_events_are_ignored() {
        use serde_json::json;
        for value in [
            json!({"type":"session.idle"}),
            json!({"data":{"sessionID":"s"}}),
            json!({"type":7,"data":{"sessionID":"s"}}),
            for_s("session.text.started", json!({})),
            for_s("session.text.delta", json!({"delta":"Hi"})),
            for_s("session.text.delta", json!({"assistantMessageID":"m"})),
            for_s(
                "session.text.delta",
                json!({"assistantMessageID":"m","delta":7}),
            ),
            for_s("session.text.ended", json!({})),
            for_s("session.tool.called", json!({"id":"t1"})),
            for_s("session.tool.called", json!({"assistantMessageID":"m"})),
            for_s(
                "session.tool.called",
                json!({"assistantMessageID":"m","id":7}),
            ),
            for_s("session.tool.success", json!({})),
            for_s("session.tool.success", json!({"id":7})),
            for_s("permission.asked", json!({"action":"bash"})),
            for_s("permission.asked", json!({"id":7,"action":"bash"})),
            for_s("permission.asked", json!({"id":"p1"})),
            for_s("permission.asked", json!({"id":"p1","action":7})),
            for_s("permission.replied", json!({})),
            for_s("permission.replied", json!({"requestID":7})),
            json!({"type":"form.created","data":{}}),
            json!({"type":"form.replied","data":{"id":"f1"}}),
            json!({"type":"form.replied","data":{}}),
            json!({"type":"form.replied","data":{"form":{"sessionID":"s"}}}),
            json!({"type":"form.replied","data":{"form":{"id":7,"sessionID":"s"}}}),
            json!({"type":"form.replied","data":{"form":{"id":"f1"}}}),
            json!({"type":"form.replied","data":{"form":{"id":"f1","sessionID":7}}}),
            for_s("session.inbox.delivered", json!({})),
            for_s("session.inbox.delivered", json!({"inboxID":7})),
            for_s("session.status", json!({})),
            for_s("session.status", json!({"status":{}})),
            for_s("session.status", json!({"status":{"type":7}})),
        ] {
            assert_eq!(
                event_for(&value, "s", &mut Default::default()),
                None,
                "{value}"
            );
        }
        assert_eq!(session_id_of(&json!({"type":"session.idle"})), None);
        assert_eq!(
            session_id_of(&json!({"type":"x","data":{"form":{"sessionID":7}}})),
            None
        );
    }

    #[test]
    fn form_fields_become_question_fields() {
        use serde_json::json;
        let question = question_from_form(&json!({
            "id": "f1",
            "title": "Deploy",
            "fields": [
                {"key": "name", "type": "string", "title": "Name", "description": "Who deploys"},
                {"key": "target", "type": "string", "options": ["staging", {"label": "production", "description": "Live"}, {"value": 3}], "custom": true},
                {"key": "count", "type": "number", "title": ""},
                {"key": "steps", "type": "integer"},
                {"key": "confirm", "type": "boolean"},
                {"key": "regions", "type": "multiselect", "options": ["eu", "us"]},
                {"key": "empty", "type": "multiselect"},
                {"key": "login", "type": "external"},
                {"key": "file", "type": "upload"},
                {"type": "string"},
                {"key": 7, "type": "string"},
                {"key": "untyped"},
                {"key": "badly typed", "type": 7}
            ]
        }));
        let field = |key: &str, title: &str, kind: FieldKind, options: Vec<FieldOption>| Field {
            key: key.into(),
            title: title.into(),
            description: None,
            kind,
            options,
            custom: false,
        };
        let option = |label: &str| FieldOption {
            label: label.into(),
            description: None,
        };
        assert_eq!(
            question,
            Question {
                id: "f1".into(),
                title: "Deploy".into(),
                fields: vec![
                    Field {
                        description: Some("Who deploys".into()),
                        ..field("name", "Name", FieldKind::Text, Vec::new())
                    },
                    Field {
                        custom: true,
                        ..field(
                            "target",
                            "target",
                            FieldKind::Choice,
                            vec![
                                option("staging"),
                                FieldOption {
                                    label: "production".into(),
                                    description: Some("Live".into()),
                                },
                                option("{\"value\":3}"),
                            ]
                        )
                    },
                    field("count", "count", FieldKind::Text, Vec::new()),
                    field("steps", "steps", FieldKind::Text, Vec::new()),
                    field(
                        "confirm",
                        "confirm",
                        FieldKind::Choice,
                        vec![option("true"), option("false")]
                    ),
                    field(
                        "regions",
                        "regions",
                        FieldKind::MultipleChoice,
                        vec![option("eu"), option("us")]
                    ),
                    field("empty", "empty", FieldKind::MultipleChoice, Vec::new()),
                    field("login", "login", FieldKind::Unsupported, Vec::new()),
                    field("file", "file", FieldKind::Unsupported, Vec::new()),
                ],
            }
        );
        // A form without an id, title or fields is still a question, which cannot be answered.
        assert_eq!(
            question_from_form(&json!({})),
            Question {
                id: String::new(),
                title: String::new(),
                fields: Vec::new(),
            }
        );
    }

    #[test]
    fn workers_are_child_sessions_with_their_state() {
        use serde_json::json;
        let worker = |info: Value| worker_from_session(&info);
        assert_eq!(
            worker(json!({"id":"c1","parentID":"p","title":"Explore","time":{"busy":5}})),
            Some(Worker {
                id: "c1".into(),
                parent: Some("p".into()),
                title: "Explore".into(),
                state: WorkerState::Running,
            })
        );
        for (outcome, state) in [
            ("failed", WorkerState::Error),
            ("interrupted", WorkerState::Stopped),
            ("unheard of", WorkerState::Unknown),
        ] {
            assert_eq!(
                worker(json!({"id":"c1","outcome":outcome})),
                Some(Worker {
                    id: "c1".into(),
                    parent: None,
                    title: "worker".into(),
                    state,
                })
            );
        }
        assert_eq!(worker(json!({"title":"No id"})), None);
        assert_eq!(worker(json!({"id":7})), None);
    }

    #[test]
    fn answers_take_the_shape_of_an_opencode_reply() {
        assert_eq!(
            form_answer(&[
                ("name".into(), AnswerValue::Text("Ada".into())),
                (
                    "regions".into(),
                    AnswerValue::Choices(vec!["eu".into(), "us".into()])
                ),
            ]),
            serde_json::json!({"name": "Ada", "regions": ["eu", "us"]})
        );
    }

    fn event(
        value: &Value,
        session_id: &str,
        seen: &mut std::collections::HashSet<String>,
    ) -> Option<Event> {
        event_for(value, session_id, seen)
    }

    /// Regression: `session.text.ended` must emit `MessageCompleted` even
    /// though the matching `session.text.started` already recorded the message
    /// in `seen_messages`. Previously the arm was gated on `first_seen`, which
    /// returned `None` for an already-seen id and swallowed the completion — so
    /// the reply's final unclosed sentence was never flushed to speech.
    /// Regression: a tool called after its message's text started was
    /// swallowed, because the arm returned early once the message had been
    /// seen, so the tool was never shown working.
    #[test]
    fn a_tool_called_after_its_messages_text_is_reported() {
        let mut seen = std::collections::HashSet::new();
        let started = serde_json::json!({
            "type": "session.text.started",
            "data": {"sessionID": "s", "assistantMessageID": "m"}
        });
        let called = serde_json::json!({
            "type": "session.tool.called",
            "data": {"sessionID": "s", "assistantMessageID": "m", "id": "t1", "input": {"command": "ls"}}
        });
        let tool = Some(Event::ToolStarted {
            message: "m".into(),
            tool: String::new(),
            id: "t1".into(),
            input: serde_json::json!({"command": "ls"}),
        });
        assert_eq!(
            event(&started, "s", &mut seen),
            Some(Event::MessageStarted {
                message: "m".into(),
                role: Role::Assistant
            })
        );
        assert_eq!(event(&called, "s", &mut seen), tool);

        // A tool that starts its message stands for the message's start, which is not reported again.
        let mut seen = std::collections::HashSet::new();
        assert_eq!(event(&called, "s", &mut seen), tool);
        assert_eq!(event(&started, "s", &mut seen), None);
    }

    #[test]
    fn text_ended_emits_message_completed_after_text_started() {
        let session = "ses_test";
        let mut seen = std::collections::HashSet::new();

        let started = serde_json::json!({
            "type": "session.text.started",
            "data": {
                "sessionID": session,
                "assistantMessageID": "msg_1",
                "ordinal": 0
            }
        });
        assert_eq!(
            event(&started, session, &mut seen),
            Some(Event::MessageStarted {
                message: "msg_1".into(),
                role: Role::Assistant
            })
        );

        let delta = serde_json::json!({
            "type": "session.text.delta",
            "data": {
                "sessionID": session,
                "assistantMessageID": "msg_1",
                "delta": "pong"
            }
        });
        assert_eq!(
            event(&delta, session, &mut seen),
            Some(Event::TextDelta {
                message: "msg_1".into(),
                text: "pong".into()
            })
        );

        let ended = serde_json::json!({
            "type": "session.text.ended",
            "data": {
                "sessionID": session,
                "assistantMessageID": "msg_1",
                "delta": "pong"
            }
        });
        assert_eq!(
            event(&ended, session, &mut seen),
            Some(Event::MessageCompleted {
                message: "msg_1".into()
            }),
            "text.ended must complete the message it names"
        );
    }

    /// The agent listing as OpenCode 1.18 answers it, abridged.
    fn agents() -> Vec<Value> {
        serde_json::from_value(serde_json::json!([
            {"id": "build", "description": "The default agent.", "mode": "primary", "hidden": false},
            {"id": "plan", "description": "Plan mode.", "mode": "primary", "hidden": false},
            {"id": "general", "description": "General-purpose agent.", "mode": "subagent", "hidden": false},
            {"id": "explore", "description": "Fast agent.", "mode": "subagent", "hidden": false},
            {"id": "compaction", "mode": "primary", "hidden": true},
            {"id": "title", "mode": "primary", "hidden": true},
            {"id": "summary", "mode": "primary", "hidden": true},
            {"id": "review", "description": "Reviews changes.", "mode": "all"},
            {"description": "An agent without an id is not offered."}
        ]))
        .unwrap()
    }

    #[test]
    fn modes_are_the_primary_agents_in_order() {
        let modes = modes_from_agents(&agents());
        let ids: Vec<&str> = modes.iter().map(|mode| mode.id.as_str()).collect();
        assert_eq!(ids, ["build", "plan", "review"]);
        assert_eq!(modes[1].name, "plan");
        assert_eq!(modes[1].description.as_deref(), Some("Plan mode."));
    }

    #[test]
    fn the_current_mode_defaults_to_the_first_agent() {
        let modes = modes_from_agents(&agents());
        let fresh = serde_json::json!({"id": "ses_test"});
        assert_eq!(current_mode(&fresh, &modes).as_deref(), Some("build"));
        let switched = serde_json::json!({"id": "ses_test", "agent": "plan"});
        assert_eq!(current_mode(&switched, &modes).as_deref(), Some("plan"));
        assert_eq!(current_mode(&fresh, &[]), None);
    }

    #[test]
    fn reports_agent_switches_for_the_session() {
        let switched = serde_json::json!({
            "type": "session.next.agent.switched",
            "data": {"sessionID": "ses_test", "messageID": "msg_1", "agent": "plan"}
        });
        assert_eq!(
            agent_switched(&switched, "ses_test").as_deref(),
            Some("plan")
        );
        assert_eq!(agent_switched(&switched, "ses_other"), None);
        for incomplete in [
            serde_json::json!({"type": "session.agent.switched"}),
            serde_json::json!({"data": {"sessionID": "ses_test", "agent": "plan"}}),
            serde_json::json!({"type": 7, "data": {"sessionID": "ses_test", "agent": "plan"}}),
        ] {
            assert_eq!(
                agent_switched(&incomplete, "ses_test"),
                None,
                "{incomplete}"
            );
        }
        let mut seen = std::collections::HashSet::new();
        assert_eq!(event(&switched, "ses_test", &mut seen), None);
    }
}
