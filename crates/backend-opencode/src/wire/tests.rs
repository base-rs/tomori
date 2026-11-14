// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

#[test]
fn a_resumed_child_is_not_mistaken_for_its_previous_outcome() {
    let mut info =
        serde_json::json!({"id":"child", "outcome":"succeeded", "time":{"busy":101,"idle":100}});
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
    let numbered = serde_json::json!({"type":"session.renamed","data":{"sessionID":"s","title":1}});
    assert!(event_for(&numbered, "s", &mut Default::default()).is_none());
}

#[test]
fn moving_the_registered_session_says_where_it_is_now_and_a_move_naming_no_directory_is_none() {
    let moved = serde_json::json!({"type":"session.moved","data":{"sessionID":"s","location":{"directory":"/work/new"},"projectID":"p","subpath":""}});
    assert_eq!(
        event_for(&moved, "s", &mut Default::default()),
        Some(Event::DirectoryChanged {
            directory: "/work/new".into()
        })
    );
    assert!(event_for(&moved, "other", &mut Default::default()).is_none());
    for location in [
        serde_json::json!({}),
        serde_json::json!({"directory": ""}),
        serde_json::json!({"directory": 1}),
    ] {
        let unnamed = serde_json::json!({"type":"session.moved","data":{"sessionID":"s","location":location}});
        assert!(event_for(&unnamed, "s", &mut Default::default()).is_none());
    }
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
        // A deliberate cancel or barge-in that the run noticed as a failure ends the turn, and is no error.
        (
            for_s(
                "session.execution.failed",
                json!({"error":{"type":"aborted","message":"Tool execution interrupted"}}),
            ),
            Some(Event::Idle),
        ),
        (
            for_s(
                "session.execution.failed",
                json!({"error":{"type":"MessageAbortedError","message":"Stopped"}}),
            ),
            Some(Event::Idle),
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
