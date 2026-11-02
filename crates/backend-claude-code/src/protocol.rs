// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Translation between Claude Code's stream-json protocol and backend [`Event`]s.
//!
//! [`State`] is a pure state machine: it consumes output lines and produces events and lines to write back. It has no
//! knowledge of the process, so it can be tested directly against recorded output.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use backend::{
    AnswerValue, Decision, Error, ErrorKind, Event, Field, FieldKind, FieldOption, Grant,
    GrantScope, Mode, Modes, Part, Permission, PermissionOption, Prompt, Question, Result, Role,
    Snapshot, SnapshotMessage,
};
use serde_json::{Map, Value, json};

use crate::workers::{self, WORKER_TOOLS, Workers};

/// The tool Claude Code uses to ask the user structured questions.
const QUESTION_TOOL: &str = "AskUserQuestion";

/// The model Claude Code names on messages it generates itself, such as API errors.
const SYNTHETIC_MODEL: &str = "<synthetic>";

/// The agent-facing reason given when a participant denies a request.
const DENIED: &str = "The user denied this request.";

/// The id of the one standing option a Claude Code permission can offer: the terminal dialog's standing row, such as
/// "Yes, and don't ask again for …", which applies the updates that row displays.
pub(crate) const SUGGESTIONS_OPTION: &str = "suggestions";

/// Tools whose terminal dialog offers to accept edits for the session rather than to add rules.
const EDIT_TOOLS: [&str; 4] = ["Edit", "MultiEdit", "Write", "NotebookEdit"];

/// Claude Code's permission modes, in the order its terminal steps through them with Shift-Tab, with the titles it shows
/// for them. As in Claude Code 2.1.311, Shift-Tab goes from default to accept edits to plan, then to bypass permissions
/// when that mode is available, then to auto when that mode is available, and back to default; any other mode, such as
/// `dontAsk`, steps to default.
const PERMISSION_MODES: [(&str, &str, &str); 5] = [
    (
        "default",
        "Manual",
        "Asks before tool uses that need permission.",
    ),
    ("acceptEdits", "Accept edits", "Edits files without asking."),
    ("plan", "Plan", "Explores and plans without making changes."),
    (
        BYPASS_MODE,
        "Bypass Permissions",
        "Runs every tool without asking.",
    ),
    (
        AUTO_MODE,
        "Auto",
        "Decides itself which tool uses need permission.",
    ),
];

/// The permission mode Claude Code offers only to a process launched with bypassing allowed.
pub(crate) const BYPASS_MODE: &str = "bypassPermissions";

/// The permission mode Claude Code offers only where the user's account and model allow it.
pub(crate) const AUTO_MODE: &str = "auto";

/// What handling one line produced.
#[derive(Debug, Default)]
pub(crate) struct Effects {
    pub events: Vec<Event>,
    /// Lines to write to the agent, in order.
    pub writes: Vec<Value>,
}

#[derive(Debug)]
enum Pending {
    Permission {
        permission: Permission,
        /// The permission updates that [`SUGGESTIONS_OPTION`] applies, as the terminal dialog's row would.
        updates: Vec<Value>,
    },
    Question {
        question: Question,
        input: Value,
    },
}

#[derive(Debug)]
pub(crate) struct State {
    session: String,
    busy: bool,
    messages: Vec<SnapshotMessage>,
    /// The index of the assistant message being streamed.
    current: Option<usize>,
    /// Content block indexes of the current message, mapped to part indexes.
    blocks: HashMap<u64, usize>,
    /// Text and reasoning parts of the current message whose complete content arrived in an `assistant` record.
    settled: HashSet<usize>,
    tools: HashSet<String>,
    /// Unresolved `can_use_tool` requests by request id, oldest first.
    pending: Vec<(String, Pending)>,
    workers: Workers,
    /// Prompt keys that were sent, so a key is delivered at most once.
    sent: HashSet<String>,
    /// Sent prompts awaiting acknowledgement, by message uuid.
    prompts: HashMap<String, Prompt>,
    /// Control requests sent to the agent and awaiting a response, by request id.
    requests: HashMap<String, &'static str>,
    /// The permission mode the agent last reported.
    mode: Option<String>,
    /// The mode the process was started in, until the agent first reports its mode.
    start: Option<String>,
    /// The agent offers bypass permissions mode, because the process was launched with bypassing allowed.
    bypass: bool,
    /// The agent offers auto mode. Claude Code does not say whether it does until it refuses it.
    auto: bool,
    /// `set_permission_mode` requests awaiting a response, by request id, with the mode they ask for.
    mode_requests: HashMap<String, String>,
}

impl State {
    #[cfg(test)]
    pub(crate) fn new(session: String) -> Self {
        Self::attached(session, Workers::default())
    }

    /// A session that already has `workers`, such as those found in its records when attaching.
    pub(crate) fn attached(session: String, workers: Workers) -> Self {
        Self {
            session,
            busy: false,
            messages: Vec::new(),
            current: None,
            blocks: HashMap::new(),
            settled: HashSet::new(),
            tools: HashSet::new(),
            pending: Vec::new(),
            workers,
            sent: HashSet::new(),
            prompts: HashMap::new(),
            requests: HashMap::new(),
            mode: None,
            start: None,
            bypass: false,
            auto: true,
            mode_requests: HashMap::new(),
        }
    }

    /// The state of a process started with `--permission-mode start`. `bypass` says whether it was launched with
    /// bypassing allowed.
    pub(crate) fn starting(session: String, workers: Workers, start: &str, bypass: bool) -> Self {
        Self {
            start: Some(start.into()),
            bypass,
            ..Self::attached(session, workers)
        }
    }

    /// The modes the terminal's Shift-Tab steps through for this process, in its order, and the current mode.
    pub(crate) fn modes(&self) -> Modes {
        Modes {
            offered: PERMISSION_MODES
                .iter()
                .filter(|(id, ..)| match *id {
                    BYPASS_MODE => self.bypass,
                    AUTO_MODE => self.auto,
                    _ => true,
                })
                .map(|(id, name, description)| Mode {
                    id: (*id).into(),
                    name: (*name).into(),
                    description: Some((*description).into()),
                })
                .collect(),
            current: self.mode.clone(),
        }
    }

    /// The control request that asks the agent to switch to the offered permission mode `mode`, as the terminal does.
    pub(crate) fn set_mode(&mut self, mode: &str) -> Result<Value> {
        if !self
            .modes()
            .offered
            .iter()
            .any(|offered| offered.id == mode)
        {
            return Err(Error::new(
                ErrorKind::Invalid,
                format!("the session offers no mode {mode:?}"),
            ));
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.mode_requests.insert(id.clone(), mode.into());
        Ok(json!({
            "type": "control_request",
            "request_id": id,
            "request": {"subtype": "set_permission_mode", "mode": mode},
        }))
    }

    /// The agent reported its permission mode. The first report after starting says whether it took the starting mode.
    fn reported_mode(&mut self, mode: &str, effects: &mut Effects) {
        if let Some(start) = self.start.take()
            && start != mode
        {
            // Claude Code starts in its default mode instead of one it cannot take.
            self.unavailable(&start);
            effects.events.push(Event::ModeRefused {
                mode: start,
                reason: None,
            });
        } else if self.mode.as_deref() == Some(mode) {
            return;
        }
        self.mode = Some(mode.into());
        effects.events.push(Event::ModesChanged(self.modes()));
    }

    /// Stop offering a mode the agent refused because it is unavailable to this process.
    fn unavailable(&mut self, mode: &str) {
        match mode {
            AUTO_MODE => self.auto = false,
            BYPASS_MODE => self.bypass = false,
            _ => {}
        }
    }

    pub(crate) fn session(&self) -> &str {
        &self.session
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            busy: Some(self.busy),
            messages: self.messages.clone(),
            permission: self.pending.iter().find_map(|(_, pending)| match pending {
                Pending::Permission { permission, .. } => Some(permission.clone()),
                Pending::Question { .. } => None,
            }),
            question: self.pending.iter().find_map(|(_, pending)| match pending {
                Pending::Question { question, .. } => Some(question.clone()),
                Pending::Permission { .. } => None,
            }),
            workers: self.workers.list().to_vec(),
            modes: self.modes(),
        }
    }

    /// The agent process ended: report that none of its workers is running any more.
    pub(crate) fn stop_workers(&mut self) -> Vec<Event> {
        let mut effects = Effects::default();
        let changed = self.workers.stop();
        self.workers_changed(changed, &mut effects);
        effects.events
    }

    fn workers_changed(&self, changed: bool, effects: &mut Effects) {
        if changed {
            effects.events.push(Event::WorkersChanged {
                workers: self.workers.list().to_vec(),
            });
        }
    }

    /// The user message for `prompt`, or `None` if its key was already sent.
    pub(crate) fn prompt(&mut self, prompt: Prompt) -> Option<Value> {
        if !self.sent.insert(prompt.key.clone()) {
            return None;
        }
        let uuid = uuid::Uuid::new_v4().to_string();
        let line = json!({
            "type": "user",
            "message": {"role": "user", "content": message_text(&prompt.text)},
            "parent_tool_use_id": null,
            "session_id": self.session,
            "uuid": uuid,
        });
        self.prompts.insert(uuid, prompt);
        Some(line)
    }

    /// A control request to the agent, such as `initialize` or `interrupt`.
    pub(crate) fn control_request(&mut self, subtype: &'static str) -> Value {
        let id = uuid::Uuid::new_v4().to_string();
        self.requests.insert(id.clone(), subtype);
        json!({"type": "control_request", "request_id": id, "request": {"subtype": subtype}})
    }

    /// Resolve a pending permission, or decline a pending question.
    pub(crate) fn decide(&mut self, id: &str, decision: Decision) -> Result<(Value, Event)> {
        let index = self.pending_index(id)?;
        let (response, event) = match (&self.pending[index].1, decision) {
            (Pending::Permission { permission, .. }, Decision::AllowOnce) => (
                json!({"behavior": "allow", "updatedInput": permission.input}),
                Event::PermissionResolved { id: id.into() },
            ),
            // The terminal answers its "don't ask again" row with the same updates, which Claude Code then applies and
            // saves to their destinations itself.
            (
                Pending::Permission {
                    permission,
                    updates,
                },
                Decision::Grant { option },
            ) if option == SUGGESTIONS_OPTION && !updates.is_empty() => (
                json!({"behavior": "allow", "updatedInput": permission.input, "updatedPermissions": updates}),
                Event::PermissionResolved { id: id.into() },
            ),
            (Pending::Permission { .. }, Decision::Grant { option }) => {
                return Err(Error::new(
                    ErrorKind::Invalid,
                    format!("this request offers no option {option:?}"),
                ));
            }
            (Pending::Question { .. }, Decision::Grant { .. }) => {
                return Err(Error::new(
                    ErrorKind::Invalid,
                    "a question is answered, not allowed",
                ));
            }
            (Pending::Permission { .. }, Decision::Deny) => (
                json!({"behavior": "deny", "message": DENIED}),
                Event::PermissionResolved { id: id.into() },
            ),
            (Pending::Question { .. }, Decision::Deny) => (
                json!({"behavior": "deny", "message": DENIED}),
                Event::QuestionResolved { id: id.into() },
            ),
            (Pending::Question { .. }, Decision::AllowOnce) => {
                return Err(Error::new(
                    ErrorKind::Invalid,
                    "a question is answered, not allowed",
                ));
            }
        };
        self.pending.remove(index);
        Ok((control_response(id, response), event))
    }

    /// Answer a pending question. Claude Code receives the answers as the question tool's updated input.
    pub(crate) fn answer(
        &mut self,
        id: &str,
        answers: Vec<(String, AnswerValue)>,
    ) -> Result<(Value, Event)> {
        let index = self.pending_index(id)?;
        let Pending::Question { question, input } = &self.pending[index].1 else {
            return Err(Error::new(
                ErrorKind::Invalid,
                "a permission is decided, not answered",
            ));
        };
        if answers.is_empty() {
            return Err(Error::new(ErrorKind::Invalid, "no answers were given"));
        }

        let mut values = Map::new();
        for (key, value) in answers {
            let Some(field) = question.fields.iter().find(|field| field.key == key) else {
                return Err(Error::new(ErrorKind::Invalid, format!("no field {key:?}")));
            };
            let value = match (field.kind, value) {
                (FieldKind::Unsupported, _) => {
                    return Err(Error::new(
                        ErrorKind::Invalid,
                        format!("field {key:?} cannot be answered"),
                    ));
                }
                (_, AnswerValue::Text(text)) => text,
                // Claude Code joins the labels of a multiple selection with a comma.
                (_, AnswerValue::Choices(choices)) => choices.join(", "),
            };
            values.insert(key, Value::String(value));
        }

        let mut input = input.clone();
        if let Some(object) = input.as_object_mut() {
            object.insert("answers".into(), Value::Object(values));
        }
        let response = json!({"behavior": "allow", "updatedInput": input});
        self.pending.remove(index);
        Ok((
            control_response(id, response),
            Event::QuestionResolved { id: id.into() },
        ))
    }

    pub(crate) fn handle_line(&mut self, line: &str) -> Effects {
        let mut effects = Effects::default();
        // Claude Code writes only JSON to standard output in stream-json mode; anything else is not protocol.
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            self.handle(&value, &mut effects);
        }
        effects
    }

    fn handle(&mut self, value: &Value, effects: &mut Effects) {
        match str_field(value, "type") {
            Some("system") => self.system(value, effects),
            Some("stream_event") => {
                if is_top_level(value) {
                    self.stream_event(&value["event"], effects);
                }
            }
            Some("assistant") => self.assistant(value, effects),
            Some("user") => self.user(value, effects),
            Some("command_lifecycle") => {
                if str_field(value, "state") == Some("started")
                    && let Some(uuid) = str_field(value, "command_uuid")
                {
                    self.delivered(uuid, effects);
                }
            }
            Some("result") => {
                self.close_message(effects);
                // An interrupted turn also ends in an error result, with an `aborted_*` terminal reason.
                let aborted = str_field(value, "terminal_reason")
                    .is_some_and(|reason| reason.starts_with("aborted"));
                if value["is_error"].as_bool() == Some(true) && !aborted {
                    let message = str_field(value, "result")
                        .filter(|result| !result.is_empty())
                        .or_else(|| str_field(value, "subtype"))
                        .unwrap_or("the turn failed");
                    effects.events.push(Event::Error {
                        message: message.into(),
                    });
                }
                if self.busy {
                    self.busy = false;
                    effects.events.push(Event::Idle);
                }
            }
            Some("control_request") => self.control(value, effects),
            Some("control_response") => self.control_response(&value["response"], effects),
            Some("control_cancel_request") => {
                if let Some(id) = str_field(value, "request_id")
                    && let Ok(index) = self.pending_index(id)
                {
                    let event = match self.pending.remove(index).1 {
                        Pending::Permission { .. } => Event::PermissionResolved { id: id.into() },
                        Pending::Question { .. } => Event::QuestionResolved { id: id.into() },
                    };
                    effects.events.push(event);
                }
            }
            _ => {}
        }
    }

    fn control_response(&mut self, response: &Value, effects: &mut Effects) {
        let Some(id) = str_field(response, "request_id") else {
            return;
        };
        let failed = str_field(response, "subtype") == Some("error");
        if let Some(mode) = self.mode_requests.remove(id) {
            if failed {
                // Claude Code names why with an error code such as `auto_mode_unavailable` or `bypass_not_launched`.
                if str_field(response, "error_code")
                    .is_some_and(|code| code.starts_with("auto_mode") || code.starts_with("bypass"))
                {
                    self.unavailable(&mode);
                    effects.events.push(Event::ModesChanged(self.modes()));
                }
                effects.events.push(Event::ModeRefused {
                    mode,
                    reason: str_field(response, "error").map(Into::into),
                });
            } else if let Some(mode) = str_field(&response["response"], "mode") {
                self.reported_mode(mode, effects);
            }
            return;
        }
        match self.requests.remove(id) {
            Some(_) if failed => {
                let message = str_field(response, "error").unwrap_or("control request failed");
                effects.events.push(Event::Error {
                    message: message.into(),
                });
            }
            // The agent's mode at connect time, which says whether it took the starting mode.
            Some("initialize") => {
                if let Some(mode) = str_field(&response["response"], "current_permission_mode") {
                    self.reported_mode(mode, effects);
                }
            }
            _ => {}
        }
    }

    fn system(&mut self, value: &Value, effects: &mut Effects) {
        // Claude Code reports its permission mode when a turn starts and whenever it changes, including changes it makes
        // itself, such as leaving plan mode.
        if matches!(str_field(value, "subtype"), Some("init" | "status"))
            && let Some(mode) = str_field(value, "permissionMode")
        {
            self.reported_mode(mode, effects);
        }
        match str_field(value, "subtype") {
            Some("init") => {
                if let Some(session) = str_field(value, "session_id") {
                    self.session = session.into();
                }
            }
            Some("status") if str_field(value, "status") == Some("requesting") => {
                self.begin_turn(effects);
            }
            Some("task_started") => {
                let changed = self.workers.task_started(value);
                self.workers_changed(changed, effects);
            }
            Some("task_updated") => {
                let changed = self.workers.task_updated(value);
                self.workers_changed(changed, effects);
            }
            Some("task_notification") => {
                let changed = self.workers.notified(
                    str_field(value, "task_id"),
                    str_field(value, "tool_use_id"),
                    str_field(value, "status"),
                    str_field(value, "run_id"),
                );
                self.workers_changed(changed, effects);
            }
            _ => {}
        }
    }

    fn stream_event(&mut self, event: &Value, effects: &mut Effects) {
        match str_field(event, "type") {
            Some("message_start") => match str_field(&event["message"], "id") {
                Some(id) => self.start_message(id, effects),
                None => {
                    self.begin_turn(effects);
                    self.close_message(effects);
                }
            },
            Some("content_block_start") => {
                let Some(message) = self.current.map(|index| &mut self.messages[index]) else {
                    return;
                };
                let part = match str_field(&event["content_block"], "type") {
                    Some("text") => Part::Text {
                        text: String::new(),
                    },
                    Some("thinking") => Part::Reasoning {
                        text: String::new(),
                    },
                    // Tool use input streams as partial JSON; the tool is reported once the complete block arrives.
                    _ => return,
                };
                if let Some(index) = event["index"].as_u64() {
                    message.parts.push(part);
                    self.blocks.insert(index, message.parts.len() - 1);
                }
            }
            Some("content_block_delta") => {
                let Some(current) = self.current else { return };
                let Some(part) = event["index"]
                    .as_u64()
                    .and_then(|index| self.blocks.get(&index))
                else {
                    return;
                };
                let message = &mut self.messages[current];
                let delta = &event["delta"];
                match (&mut message.parts[*part], str_field(delta, "type")) {
                    (Part::Text { text }, Some("text_delta")) => {
                        let delta = str_field(delta, "text").unwrap_or_default();
                        text.push_str(delta);
                        effects.events.push(Event::TextDelta {
                            message: message.id.clone(),
                            text: delta.into(),
                        });
                    }
                    (Part::Reasoning { text }, Some("thinking_delta")) => {
                        text.push_str(str_field(delta, "thinking").unwrap_or_default());
                    }
                    _ => {}
                }
            }
            Some("message_stop") => self.close_message(effects),
            _ => {}
        }
    }

    /// One or more complete assistant content blocks. With partial messages, each block streams as stream events first
    /// and the record repeats it; without them, as after Claude Code falls back to a non-streaming request, the record is
    /// the only report of the message.
    fn assistant(&mut self, value: &Value, effects: &mut Effects) {
        let parent = str_field(value, "parent_tool_use_id");
        let message = &value["message"];
        let Some(message_id) = str_field(message, "id") else {
            return;
        };
        let Some(content) = message["content"].as_array() else {
            return;
        };
        // Synthetic messages, such as API errors, are not replies; the turn's result reports the failure.
        if str_field(message, "model") == Some(SYNTHETIC_MODEL)
            || value["isApiErrorMessage"] == true
        {
            return;
        }
        if parent.is_none() && !self.messages.iter().any(|m| m.id == message_id) {
            self.start_message(message_id, effects);
        }
        for block in content {
            match str_field(block, "type") {
                Some("text") if parent.is_none() => {
                    let text = str_field(block, "text").unwrap_or_default();
                    self.settle(message_id, Part::Text { text: text.into() }, effects);
                }
                Some("thinking") if parent.is_none() => {
                    let text = str_field(block, "thinking").unwrap_or_default();
                    self.settle(message_id, Part::Reasoning { text: text.into() }, effects);
                }
                Some("tool_use") => self.tool_use(parent, message_id, block, effects),
                _ => {}
            }
        }
    }

    /// Reconcile a complete text or reasoning block with what streamed for it. The block completes the earliest part of
    /// its kind not yet settled, and only text that did not stream is reported; a block that never streamed is added.
    fn settle(&mut self, message_id: &str, block: Part, effects: &mut Effects) {
        let Some(current) = self
            .current
            .filter(|index| self.messages[*index].id == message_id)
        else {
            // The message already completed, so its blocks have streamed.
            return;
        };
        let message = &mut self.messages[current];
        let reasoning = matches!(block, Part::Reasoning { .. });
        let (Part::Text { text: complete } | Part::Reasoning { text: complete }) = block else {
            unreachable!("only text and reasoning blocks are settled");
        };
        let found = message.parts.iter().enumerate().position(|(index, part)| {
            !self.settled.contains(&index)
                && match part {
                    Part::Text { .. } => !reasoning,
                    Part::Reasoning { .. } => reasoning,
                    Part::Tool { .. } => false,
                }
        });
        let (index, missing) = match found {
            Some(index) => {
                let (Part::Text { text } | Part::Reasoning { text }) = &mut message.parts[index]
                else {
                    unreachable!("the part has the block's kind");
                };
                // Streamed text is never retracted, so a block that disagrees with it adds nothing.
                let missing = complete
                    .strip_prefix(text.as_str())
                    .unwrap_or_default()
                    .to_owned();
                text.push_str(&missing);
                (index, missing)
            }
            // Claude Code omits thinking it does not display, leaving an empty block.
            None if complete.is_empty() => return,
            None => {
                message.parts.push(if reasoning {
                    Part::Reasoning {
                        text: complete.clone(),
                    }
                } else {
                    Part::Text {
                        text: complete.clone(),
                    }
                });
                (message.parts.len() - 1, complete)
            }
        };
        self.settled.insert(index);
        // Later deltas for the block are already included.
        self.blocks.retain(|_, part| *part != index);
        if !reasoning && !missing.is_empty() {
            effects.events.push(Event::TextDelta {
                message: message.id.clone(),
                text: missing,
            });
        }
    }

    fn tool_use(
        &mut self,
        parent: Option<&str>,
        message_id: &str,
        block: &Value,
        effects: &mut Effects,
    ) {
        let (Some(id), Some(tool)) = (str_field(block, "id"), str_field(block, "name")) else {
            return;
        };
        let input = block.get("input").cloned().unwrap_or(Value::Null);

        if WORKER_TOOLS.contains(&tool) {
            let changed = self.workers.start(id, parent, workers::title(&input, tool));
            self.workers_changed(changed, effects);
        }

        // Only the session's own tool activity is reported; subagent activity is summarized by its worker.
        if parent.is_none() && self.tools.insert(id.into()) {
            if let Some(message) = self.messages.iter_mut().rev().find(|m| m.id == message_id) {
                message.parts.push(Part::Tool {
                    id: id.into(),
                    tool: tool.into(),
                    input: input.clone(),
                    finished: false,
                });
            }
            effects.events.push(Event::ToolStarted {
                message: message_id.into(),
                tool: tool.into(),
                id: id.into(),
                input,
            });
        }
    }

    fn user(&mut self, value: &Value, effects: &mut Effects) {
        if value["isReplay"].as_bool() == Some(true) {
            if let Some(uuid) = str_field(value, "uuid") {
                self.delivered(uuid, effects);
            }
            return;
        }

        let top_level = is_top_level(value);
        let parent = str_field(value, "parent_tool_use_id");
        let Some(content) = value["message"]["content"].as_array() else {
            return;
        };
        // The tool's structured output belongs to the message's one tool result.
        let results = content
            .iter()
            .filter(|block| str_field(block, "type") == Some("tool_result"))
            .count();
        let output = value.get("tool_use_result").filter(|_| results == 1);
        for block in content {
            if str_field(block, "type") != Some("tool_result") {
                continue;
            }
            let Some(id) = str_field(block, "tool_use_id") else {
                continue;
            };
            let failed = block["is_error"].as_bool() == Some(true);

            let text = result_text(&block["content"]);
            let changed = self.workers.tool_result(id, failed, output, &text, parent);
            self.workers_changed(changed, effects);

            if top_level && self.tools.contains(id) {
                for message in self.messages.iter_mut().rev() {
                    if let Some(Part::Tool { finished, .. }) = message
                        .parts
                        .iter_mut()
                        .find(|part| matches!(part, Part::Tool { id: tool, .. } if tool == id))
                    {
                        *finished = true;
                        break;
                    }
                }
                effects.events.push(Event::ToolFinished {
                    id: id.into(),
                    error: failed.then_some(text),
                });
            }
        }
    }

    fn control(&mut self, value: &Value, effects: &mut Effects) {
        let Some(id) = str_field(value, "request_id") else {
            return;
        };
        let request = &value["request"];
        if str_field(request, "subtype") != Some("can_use_tool") {
            // Tomori registers no hooks or SDK tools, so no other request is expected; never leave the agent waiting.
            effects.writes.push(json!({
                "type": "control_response",
                "response": {"subtype": "error", "request_id": id, "error": "unsupported control request"},
            }));
            return;
        }

        let tool = str_field(request, "tool_name").unwrap_or_default();
        let input = request.get("input").cloned().unwrap_or(Value::Null);

        if tool == QUESTION_TOOL
            && let Some(question) = question(id, &input)
        {
            effects.events.push(Event::QuestionAsked(question.clone()));
            self.pending
                .push((id.into(), Pending::Question { question, input }));
            return;
        }

        let (updates, grants) = suggestions(tool, request);
        let permission = Permission {
            id: id.into(),
            tool: tool.into(),
            description: str_field(request, "description")
                .or_else(|| str_field(&input, "description"))
                .map(Into::into),
            input,
            options: if grants.is_empty() {
                Vec::new()
            } else {
                vec![PermissionOption {
                    id: SUGGESTIONS_OPTION.into(),
                    grants,
                }]
            },
        };
        effects
            .events
            .push(Event::PermissionRequested(permission.clone()));
        self.pending.push((
            id.into(),
            Pending::Permission {
                permission,
                updates,
            },
        ));
    }

    fn begin_turn(&mut self, effects: &mut Effects) {
        if !self.busy {
            self.busy = true;
            effects.events.push(Event::Busy);
        }
    }

    fn delivered(&mut self, uuid: &str, effects: &mut Effects) {
        let Some(prompt) = self.prompts.remove(uuid) else {
            return;
        };
        effects.events.push(Event::Delivered { key: prompt.key });
        self.messages.push(SnapshotMessage {
            id: uuid.into(),
            role: Role::User,
            parts: vec![Part::Text { text: prompt.text }],
            completed: true,
        });
        self.begin_turn(effects);
    }

    /// Complete the current assistant message and start the next one.
    fn start_message(&mut self, id: &str, effects: &mut Effects) {
        self.begin_turn(effects);
        self.close_message(effects);
        self.messages.push(SnapshotMessage {
            id: id.into(),
            role: Role::Assistant,
            parts: Vec::new(),
            completed: false,
        });
        self.current = Some(self.messages.len() - 1);
        effects.events.push(Event::MessageStarted {
            message: id.into(),
            role: Role::Assistant,
        });
    }

    fn close_message(&mut self, effects: &mut Effects) {
        if let Some(index) = self.current.take() {
            self.blocks.clear();
            self.settled.clear();
            let message = &mut self.messages[index];
            message.completed = true;
            effects.events.push(Event::MessageCompleted {
                message: message.id.clone(),
            });
        }
    }

    fn pending_index(&self, id: &str) -> Result<usize> {
        self.pending
            .iter()
            .position(|(pending, _)| pending == id)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("no pending request {id:?}")))
    }
}

/// Prompt text as sent to Claude Code, which never runs it as a slash command.
///
/// Claude Code 2.1 runs a user message as a slash command only when its raw text begins with `/`; it does not trim
/// the text first. Text that would begin with `/` gains a leading space, a visible change that the model reads past.
/// Text that already begins with whitespace is sent unchanged.
fn message_text(text: &str) -> String {
    if text.starts_with('/') {
        format!(" {text}")
    } else {
        text.into()
    }
}

fn control_response(id: &str, response: Value) -> Value {
    json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": id, "response": response},
    })
}

/// The permission updates that the terminal dialog's standing row would apply for this request, with the grants they
/// make. Both are empty when the terminal would offer no such row.
///
/// This follows the permission dialogs of Claude Code 2.1.311, which build each row from the request's suggestions
/// and apply only the updates the row displays. Each tool family has its own dialog:
///
/// - Bash: one row for the suggested Bash and `Read` rules and directories ("Yes, and allow access to ... and ...
///   commands"). When the suggestions are Bash rules alone, the terminal instead shows an editable command prefix that
///   saves one Bash rule to local settings; it is offered here only for a single suggested rule, as that prefix.
/// - Edit tools: one row that switches to `acceptEdits` for the session, adds the suggested directories for the
///   session and adds the suggested `Read` rules. A file in a `.claude` folder gets a different row that the terminal
///   builds itself, which is not offered.
/// - `Read`: one row for the suggested `Read` rules ("Yes, allow reading from ... during this session").
/// - `WebFetch`: one row that saves the request's own `domain:` rule to local settings.
/// - Other tools: one row that saves a rule for the whole tool to local settings, offered when Claude Code suggests
///   exactly that rule.
///
/// No row is offered when the request sets `suppress_always_allow_rule` or `requires_user_interaction`, when a safety
/// check requires manual approval, or when an organization ceiling or blocked outside reads raised the prompt.
fn suggestions(tool: &str, request: &Value) -> (Vec<Value>, Vec<Grant>) {
    let updates = standing_row(tool, request).unwrap_or_default();
    let grants = updates.iter().flat_map(update_grants).collect();
    (updates, grants)
}

/// The suggestions one dialog row reads, at most, as in Claude Code.
const ROW_SUGGESTIONS: usize = 64;
/// The rules and directories one row shows, at most, as in Claude Code.
const ROW_ENTRIES: usize = 8;
/// The length of the names a row lists, joined, beyond which Claude Code drops the row.
const ROW_LABEL: usize = 160;
/// The length of a rule, path or prefix beyond which Claude Code does not display it.
const DISPLAYED_TEXT: usize = 256;

fn standing_row(tool: &str, request: &Value) -> Option<Vec<Value>> {
    if request["suppress_always_allow_rule"].as_bool() == Some(true)
        || request["requires_user_interaction"].as_bool() == Some(true)
        || request["classifier_approvable"].as_bool() == Some(false)
        || matches!(
            str_field(request, "decision_reason_code"),
            Some("org_ask_ceiling" | "outside_reads_blocked")
        )
    {
        return None;
    }
    let suggestions = request["permission_suggestions"].as_array()?;
    if suggestions.is_empty() || suggestions.len() > ROW_SUGGESTIONS {
        return None;
    }
    let input = &request["input"];
    match tool {
        "Bash" => bash_row(suggestions, str_field(request, "decision_reason_type")),
        "Read" => read_row(suggestions),
        "WebFetch" => web_fetch_row(suggestions, str_field(input, "url")?),
        tool if EDIT_TOOLS.contains(&tool) => edit_row(
            suggestions,
            str_field(input, "file_path").or_else(|| str_field(input, "notebook_path"))?,
        ),
        // These tools have dialogs of their own, which Tomori does not model.
        QUESTION_TOOL | "ExitPlanMode" | "EnterPlanMode" | "PowerShell" | "Skill" => None,
        tool => tool_row(suggestions, tool),
    }
}

fn bash_row(suggestions: &[Value], reason: Option<&str>) -> Option<Vec<Value>> {
    let row = displayed(
        suggestions,
        &["addRules", "addDirectories"],
        |tool, content| match (tool, content) {
            ("Bash", Some(content)) => !command_prefix(content).is_empty(),
            ("Read", Some(content)) => !read_path(content).is_empty(),
            _ => false,
        },
    );
    let rules: Vec<(&str, &str)> = row_rules(&row).collect();
    let directories: Vec<&str> = row_directories(&row).collect();
    let commands = rules.iter().filter(|(tool, _)| *tool == "Bash").count();

    if commands == rules.len() && directories.is_empty() {
        // The terminal shows its editable command prefix in place of a row of Bash rules alone, seeded from the one
        // suggested rule of a compound command and otherwise from its own reading of the command. Either way it saves
        // exactly one Bash rule to local settings, which is offered when there is one rule to save.
        if let [(_, content)] = rules[..] {
            return Some(vec![command_rule(content)]);
        }
        // A compound command with several suggested rules gets the row itself; any other command gets a prefix that
        // only the terminal computes.
        if rules.is_empty() || reason != Some("subcommandResults") {
            return None;
        }
    }

    let names = rules
        .iter()
        .map(|(tool, content)| {
            if *tool == "Bash" {
                command_prefix(content)
            } else {
                read_path(content)
            }
        })
        .chain(directories.iter().copied());
    fits(rules.len() + directories.len(), names).then_some(row)
}

fn read_row(suggestions: &[Value]) -> Option<Vec<Value>> {
    let row = read_rules(suggestions);
    let rules: Vec<(&str, &str)> = row_rules(&row).collect();
    fits(
        rules.len(),
        rules.iter().map(|(_, content)| read_path(content)),
    )
    .then_some(row)
}

fn edit_row(suggestions: &[Value], file: &str) -> Option<Vec<Value>> {
    // Claude Code builds its own row for files in a project's or the user's `.claude` folder.
    if Path::new(file)
        .components()
        .any(|component| component.as_os_str() == ".claude")
    {
        return None;
    }

    let mut row = Vec::new();
    if suggestions.iter().any(|suggestion| {
        str_field(suggestion, "type") == Some("setMode")
            && str_field(suggestion, "mode") == Some("acceptEdits")
    }) {
        row.push(json!({"type": "setMode", "destination": "session", "mode": "acceptEdits"}));
    }
    // The terminal reads the first directory suggestion only, and adds its directories for the session.
    if let Some(suggestion) = suggestions
        .iter()
        .find(|suggestion| str_field(suggestion, "type") == Some("addDirectories"))
    {
        let mut directories: Vec<&str> = Vec::new();
        for directory in suggestion["directories"].as_array().into_iter().flatten() {
            if let Some(directory) = directory.as_str().filter(|path| displayable_path(path))
                && !directories.contains(&directory)
            {
                directories.push(directory);
            }
        }
        if !directories.is_empty()
            && directories.len() <= ROW_ENTRIES
            && directories.join(", ").chars().count() <= ROW_LABEL
        {
            row.push(
                json!({"type": "addDirectories", "destination": "session", "directories": directories}),
            );
        }
    }
    let rules = read_rules(suggestions);
    let paths: Vec<&str> = row_rules(&rules)
        .map(|(_, content)| read_path(content))
        .collect();
    if fits(paths.len(), paths.iter().copied()) {
        row.extend(rules);
    }
    (!row.is_empty()).then_some(row)
}

fn web_fetch_row(suggestions: &[Value], url: &str) -> Option<Vec<Value>> {
    let rule = format!("domain:{}", url_host(url)?);
    let suggested = suggestions.iter().any(|suggestion| {
        str_field(suggestion, "type") == Some("addRules")
            && str_field(suggestion, "behavior") == Some("allow")
            && str_field(suggestion, "destination") == Some("localSettings")
            && suggestion["rules"].as_array().is_some_and(|rules| {
                rules.iter().any(|candidate| {
                    str_field(candidate, "toolName") == Some("WebFetch")
                        && str_field(candidate, "ruleContent") == Some(&rule)
                })
            })
    });
    suggested.then(|| {
        vec![json!({
            "type": "addRules",
            "rules": [{"toolName": "WebFetch", "ruleContent": rule}],
            "behavior": "allow",
            "destination": "localSettings",
        })]
    })
}

fn tool_row(suggestions: &[Value], tool: &str) -> Option<Vec<Value>> {
    let update = json!({
        "type": "addRules",
        "rules": [{"toolName": tool}],
        "behavior": "allow",
        "destination": "localSettings",
    });
    (displayable(tool) && suggestions.contains(&update)).then(|| vec![update])
}

/// The `addRules` and `addDirectories` updates of `suggestions` that a row displays, as Claude Code's dialogs filter
/// them: rules that allow, kept in local settings or the session, of the types the row shows, whose rules pass `shown`.
fn displayed(
    suggestions: &[Value],
    types: &[&str],
    shown: impl Fn(&str, Option<&str>) -> bool,
) -> Vec<Value> {
    let mut row = Vec::new();
    for suggestion in suggestions {
        let Some(kind) = str_field(suggestion, "type").filter(|kind| types.contains(kind)) else {
            continue;
        };
        let Some(destination) = str_field(suggestion, "destination")
            .filter(|destination| matches!(*destination, "localSettings" | "session"))
        else {
            continue;
        };
        match kind {
            "addRules" if str_field(suggestion, "behavior") == Some("allow") => {
                let rules: Vec<Value> = suggestion["rules"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|rule| {
                        let tool = str_field(rule, "toolName").filter(|tool| displayable(tool))?;
                        let content = match rule.get("ruleContent") {
                            None => None,
                            Some(content) => {
                                Some(content.as_str().filter(|content| displayable(content))?)
                            }
                        };
                        shown(tool, content).then(|| match content {
                            Some(content) => json!({"toolName": tool, "ruleContent": content}),
                            None => json!({"toolName": tool}),
                        })
                    })
                    .collect();
                if !rules.is_empty() {
                    row.push(json!({"type": "addRules", "rules": rules, "behavior": "allow", "destination": destination}));
                }
            }
            "addDirectories" => {
                let directories: Vec<&str> = suggestion["directories"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|directory| {
                        directory.as_str().filter(|path| displayable_path(path))
                    })
                    .collect();
                if !directories.is_empty() {
                    row.push(json!({"type": "addDirectories", "directories": directories, "destination": destination}));
                }
            }
            _ => {}
        }
    }
    row
}

fn read_rules(suggestions: &[Value]) -> Vec<Value> {
    displayed(suggestions, &["addRules"], |tool, content| {
        tool == "Read" && content.is_some_and(|content| !read_path(content).is_empty())
    })
}

fn row_rules(row: &[Value]) -> impl Iterator<Item = (&str, &str)> {
    row.iter()
        .flat_map(|update| update["rules"].as_array().into_iter().flatten())
        .filter_map(|rule| {
            Some((
                str_field(rule, "toolName")?,
                str_field(rule, "ruleContent")?,
            ))
        })
}

fn row_directories(row: &[Value]) -> impl Iterator<Item = &str> {
    row.iter()
        .flat_map(|update| update["directories"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
}

/// Whether a row of `count` entries naming `names` is short enough for Claude Code to show it.
fn fits<'a>(count: usize, names: impl Iterator<Item = &'a str>) -> bool {
    let names: Vec<&str> = names.collect();
    count > 0 && count <= ROW_ENTRIES && names.join(" and ").chars().count() <= ROW_LABEL
}

/// The one Bash rule that the terminal's command prefix row saves for `prefix`; `*` allows every command.
fn command_rule(prefix: &str) -> Value {
    let rule = if prefix == "*" {
        json!({"toolName": "Bash"})
    } else {
        json!({"toolName": "Bash", "ruleContent": prefix})
    };
    json!({"type": "addRules", "rules": [rule], "behavior": "allow", "destination": "localSettings"})
}

/// A Bash rule without its trailing wildcard, as Claude Code names it in a row.
fn command_prefix(content: &str) -> &str {
    content
        .strip_suffix(":*")
        .or_else(|| content.strip_suffix(" *"))
        .unwrap_or(content)
}

/// The path a `Read` rule names, as Claude Code shows it.
fn read_path(content: &str) -> &str {
    let path = content.strip_suffix("/**").unwrap_or(content);
    let path = path.strip_prefix("./").unwrap_or(path);
    path.strip_prefix('/')
        .filter(|rest| rest.starts_with('/'))
        .unwrap_or(path)
}

/// Text that Claude Code would display in a row unchanged: not blank, short, on one line, with no control characters.
fn displayable(text: &str) -> bool {
    !text.trim().is_empty()
        && text.trim() == text
        && text.chars().count() <= DISPLAYED_TEXT
        && !text
            .chars()
            .any(|character| character.is_control() || matches!(character, '\u{2028}' | '\u{2029}'))
}

fn displayable_path(path: &str) -> bool {
    !path.trim().is_empty()
        && path.chars().count() <= DISPLAYED_TEXT
        && !path.chars().any(char::is_control)
}

/// The lowercase host of an `http` or `https` URL that names a domain, which Claude Code's `domain:` rules match.
fn url_host(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?;
    let domain = host.contains('.')
        && host
            .chars()
            .any(|character| character.is_ascii_alphabetic())
        && host
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-'));
    domain.then(|| host.to_ascii_lowercase())
}

/// The grants one permission update makes.
fn update_grants(update: &Value) -> Vec<Grant> {
    let Some(scope) = str_field(update, "destination").and_then(grant_scope) else {
        return Vec::new();
    };
    match str_field(update, "type") {
        Some("addRules") => update["rules"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|rule| {
                Some(Grant::Rule {
                    tool: str_field(rule, "toolName")?.into(),
                    content: str_field(rule, "ruleContent").map(Into::into),
                    scope,
                })
            })
            .collect(),
        Some("addDirectories") => row_directories(std::slice::from_ref(update))
            .map(|path| Grant::Directory {
                path: path.into(),
                scope,
            })
            .collect(),
        Some("setMode") => str_field(update, "mode")
            .map(|mode| {
                vec![Grant::Mode {
                    mode: mode.into(),
                    scope,
                }]
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn grant_scope(destination: &str) -> Option<GrantScope> {
    match destination {
        "session" => Some(GrantScope::Session),
        "localSettings" => Some(GrantScope::Local),
        "projectSettings" => Some(GrantScope::Project),
        "userSettings" => Some(GrantScope::User),
        _ => None,
    }
}

/// Map `AskUserQuestion` input to a question. Each field is keyed by its question text, as Claude Code keys answers.
fn question(id: &str, input: &Value) -> Option<Question> {
    let questions = input["questions"].as_array()?;
    let mut fields = Vec::with_capacity(questions.len());
    for question in questions {
        let text = str_field(question, "question")?;
        let options: Vec<FieldOption> = question["options"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|option| {
                Some(FieldOption {
                    label: str_field(option, "label")?.into(),
                    description: str_field(option, "description").map(Into::into),
                })
            })
            .collect();
        let kind = match str_field(question, "kind") {
            None | Some("choice") if question["multiSelect"].as_bool() == Some(true) => {
                FieldKind::MultipleChoice
            }
            None | Some("choice") => FieldKind::Choice,
            // Number answers are returned to Claude Code as text, like every other answer.
            Some("text" | "number") => FieldKind::Text,
            Some(_) => FieldKind::Unsupported,
        };
        fields.push(Field {
            key: text.into(),
            title: text.into(),
            description: str_field(question, "description").map(Into::into),
            kind,
            options,
            // Claude Code always lets the user type an answer other than the listed options.
            custom: matches!(kind, FieldKind::Choice | FieldKind::MultipleChoice),
        });
    }
    let title = str_field(input, "title")
        .or_else(|| questions.first().and_then(|q| str_field(q, "header")))
        .unwrap_or_default();
    Some(Question {
        id: id.into(),
        title: title.into(),
        fields,
    })
}

fn result_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| str_field(block, "text"))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn is_top_level(value: &Value) -> bool {
    value["parent_tool_use_id"].is_null()
}

fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use backend::{Worker, WorkerState};

    fn handle(state: &mut State, value: Value) -> Vec<Event> {
        state.handle_line(&value.to_string()).events
    }

    fn can_use_tool(id: &str, tool: &str, input: Value) -> Value {
        json!({
            "type": "control_request",
            "request_id": id,
            "request": {"subtype": "can_use_tool", "tool_name": tool, "input": input, "tool_use_id": "toolu_1"},
        })
    }

    #[test]
    fn prompts_never_run_slash_commands() {
        let mut state = State::new("session".into());
        let mut content = |text: &str| {
            let prompt = Prompt {
                key: text.into(),
                text: text.into(),
            };
            state.prompt(prompt).unwrap()["message"]["content"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        assert_eq!(content("/model opus"), " /model opus");
        assert_eq!(content("/model: hello"), " /model: hello");
        assert_eq!(content("  /cost"), "  /cost");
        assert_eq!(content("Samuel: /cost"), "Samuel: /cost");
        assert_eq!(content("Read /etc/hosts."), "Read /etc/hosts.");

        // The delivered message is reported as the participant said it.
        let mut state = State::new("session".into());
        let line = state
            .prompt(Prompt {
                key: "one".into(),
                text: "/compact".into(),
            })
            .unwrap();
        let replay = json!({"type": "user", "isReplay": true, "uuid": line["uuid"], "message": line["message"]});
        handle(&mut state, replay);
        assert_eq!(
            state.snapshot().messages[0].parts,
            vec![Part::Text {
                text: "/compact".into()
            }]
        );
    }

    #[test]
    fn cancelled_requests_are_resolved() {
        let mut state = State::new("session".into());
        let input = json!({"command": "rm -rf build", "description": "Remove the build directory"});
        let events = handle(&mut state, can_use_tool("perm-1", "Bash", input));
        let Event::PermissionRequested(permission) = &events[0] else {
            panic!("{events:?}")
        };
        assert_eq!(
            permission.description.as_deref(),
            Some("Remove the build directory")
        );

        let events = handle(
            &mut state,
            json!({"type": "control_cancel_request", "request_id": "perm-1"}),
        );
        assert_eq!(
            events,
            vec![Event::PermissionResolved {
                id: "perm-1".into()
            }]
        );
        assert_eq!(state.snapshot().permission, None);
        assert_eq!(
            state
                .decide("perm-1", Decision::AllowOnce)
                .unwrap_err()
                .kind,
            ErrorKind::NotFound
        );
    }

    #[test]
    fn allowing_returns_the_original_input() {
        let mut state = State::new("session".into());
        let input = json!({"command": "cargo test"});
        handle(&mut state, can_use_tool("perm-1", "Bash", input.clone()));
        let (response, event) = state.decide("perm-1", Decision::AllowOnce).unwrap();
        assert_eq!(
            response["response"]["response"],
            json!({"behavior": "allow", "updatedInput": input})
        );
        assert_eq!(
            event,
            Event::PermissionResolved {
                id: "perm-1".into()
            }
        );
    }

    fn suggested(id: &str, tool: &str, input: Value, suggestions: Value) -> Value {
        let mut request = can_use_tool(id, tool, input);
        request["request"]["permission_suggestions"] = suggestions;
        request
    }

    fn options(state: &mut State, request: Value) -> Vec<PermissionOption> {
        let events = handle(state, request);
        let Event::PermissionRequested(permission) = &events[0] else {
            panic!("{events:?}")
        };
        permission.options.clone()
    }

    /// The updates the standing option of `request` applies.
    fn updates(request: Value) -> Option<Value> {
        let mut state = State::new("session".into());
        let id = request["request_id"].as_str().unwrap().to_owned();
        if options(&mut state, request).is_empty() {
            return None;
        }
        let (response, _) = state
            .decide(
                &id,
                Decision::Grant {
                    option: SUGGESTIONS_OPTION.into(),
                },
            )
            .unwrap();
        Some(response["response"]["response"]["updatedPermissions"].clone())
    }

    fn rules(destination: &str, rules: Value) -> Value {
        json!({"type": "addRules", "rules": rules, "behavior": "allow", "destination": destination})
    }

    // The suggestion shapes in these tests were sent by Claude Code 2.1.311 for the commands, files and URLs shown.

    #[test]
    fn bash_rules_alone_offer_the_command_prefix() {
        let prefix = rules(
            "localSettings",
            json!([{"toolName": "Bash", "ruleContent": "npm run *"}]),
        );
        let request = suggested(
            "p1",
            "Bash",
            json!({"command": "npm run lint"}),
            json!([prefix.clone()]),
        );
        assert_eq!(
            options(&mut State::new("session".into()), request.clone()),
            vec![PermissionOption {
                id: SUGGESTIONS_OPTION.into(),
                grants: vec![Grant::Rule {
                    tool: "Bash".into(),
                    content: Some("npm run *".into()),
                    scope: GrantScope::Local,
                }],
            }]
        );
        assert_eq!(updates(request), Some(json!([prefix])));

        // The prefix row always saves to local settings, and `*` allows every command.
        let request = suggested(
            "p2",
            "Bash",
            json!({"command": "ls"}),
            json!([rules(
                "session",
                json!([{"toolName": "Bash", "ruleContent": "*"}])
            )]),
        );
        assert_eq!(
            updates(request),
            Some(json!([rules(
                "localSettings",
                json!([{"toolName": "Bash"}])
            )]))
        );
    }

    #[test]
    fn compound_bash_commands_offer_their_rules_together() {
        let suggestion = rules(
            "localSettings",
            json!([
                {"toolName": "Bash", "ruleContent": "mkdir -p build"},
                {"toolName": "Bash", "ruleContent": "touch build/x"},
            ]),
        );
        let mut request = suggested(
            "p1",
            "Bash",
            json!({"command": "mkdir -p build && touch build/x"}),
            json!([suggestion.clone()]),
        );
        request["request"]["decision_reason_type"] = json!("subcommandResults");
        assert_eq!(updates(request.clone()), Some(json!([suggestion])));

        // Without the compound reason, the terminal shows a prefix that only it computes.
        request["request"]["decision_reason_type"] = json!("other");
        assert_eq!(updates(request), None);
    }

    #[test]
    fn bash_outside_the_project_offers_the_command_and_the_read_rule() {
        let command = rules(
            "localSettings",
            json!([{"toolName": "Bash", "ruleContent": "grep -rn \"outside\" /tmp/outside"}]),
        );
        let read = rules(
            "session",
            json!([{"toolName": "Read", "ruleContent": "//private/tmp/outside/**"}]),
        );
        let request = suggested(
            "p1",
            "Bash",
            json!({"command": "grep -rn \"outside\" /tmp/outside"}),
            json!([command.clone(), read.clone()]),
        );
        assert_eq!(updates(request), Some(json!([command, read])));
    }

    #[test]
    fn bash_rows_show_only_what_the_terminal_shows() {
        let request = suggested(
            "p1",
            "Bash",
            json!({"command": "make"}),
            json!([
                rules("localSettings", json!([{"toolName": "Bash", "ruleContent": "make"}])),
                // Other destinations, other tools' rules, denials and other update types are not shown.
                rules("userSettings", json!([{"toolName": "Bash", "ruleContent": "make:*"}])),
                rules("projectSettings", json!([{"toolName": "Bash", "ruleContent": "make test"}])),
                rules("localSettings", json!([{"toolName": "WebSearch"}])),
                {"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": "rm"}], "behavior": "deny", "destination": "localSettings"},
                {"type": "setMode", "mode": "acceptEdits", "destination": "session"},
                {"type": "addDirectories", "directories": ["/work"], "destination": "session"},
            ]),
        );
        assert_eq!(
            updates(request),
            Some(json!([
                rules("localSettings", json!([{"toolName": "Bash", "ruleContent": "make"}])),
                {"type": "addDirectories", "directories": ["/work"], "destination": "session"},
            ]))
        );

        // A row of more than eight entries is dropped.
        let many: Vec<Value> = (0..9)
            .map(|index| json!({"toolName": "Bash", "ruleContent": format!("make {index}")}))
            .collect();
        let mut request = suggested(
            "p2",
            "Bash",
            json!({"command": "make"}),
            json!([rules("localSettings", json!(many))]),
        );
        request["request"]["decision_reason_type"] = json!("subcommandResults");
        assert_eq!(updates(request), None);

        // A rule split over lines is never shown.
        let request = suggested(
            "p3",
            "Bash",
            json!({"command": "make"}),
            json!([rules(
                "localSettings",
                json!([{"toolName": "Bash", "ruleContent": "make\nrm"}])
            )]),
        );
        assert_eq!(updates(request), None);
    }

    #[test]
    fn edits_offer_accept_edits_and_directories_for_the_session() {
        let request = suggested(
            "p1",
            "Write",
            json!({"file_path": "/work/notes.txt", "content": "hi\n"}),
            json!([{"type": "setMode", "mode": "acceptEdits", "destination": "session"}]),
        );
        assert_eq!(
            options(&mut State::new("session".into()), request.clone()),
            vec![PermissionOption {
                id: SUGGESTIONS_OPTION.into(),
                grants: vec![Grant::Mode {
                    mode: "acceptEdits".into(),
                    scope: GrantScope::Session,
                }],
            }]
        );

        let request = suggested(
            "p2",
            "Write",
            json!({"file_path": "/tmp/outside/new.txt", "content": "x"}),
            json!([
                {"type": "setMode", "mode": "acceptEdits", "destination": "session"},
                {"type": "addDirectories", "directories": ["/tmp/outside", "/private/tmp/outside"], "destination": "session"},
                rules("localSettings", json!([{"toolName": "Edit"}])),
            ]),
        );
        assert_eq!(
            updates(request),
            Some(json!([
                {"type": "setMode", "destination": "session", "mode": "acceptEdits"},
                {"type": "addDirectories", "destination": "session", "directories": ["/tmp/outside", "/private/tmp/outside"]},
            ]))
        );

        // Claude Code builds its own row for files in a `.claude` folder.
        let request = suggested(
            "p3",
            "Edit",
            json!({"file_path": "/work/.claude/settings.json"}),
            json!([{"type": "setMode", "mode": "acceptEdits", "destination": "session"}]),
        );
        assert_eq!(updates(request), None);

        // A notebook names its file differently.
        let request = suggested(
            "p4",
            "NotebookEdit",
            json!({"notebook_path": "/work/notes.ipynb", "new_source": "x"}),
            json!([{"type": "setMode", "mode": "acceptEdits", "destination": "session"}]),
        );
        assert_eq!(
            updates(request),
            Some(json!([{"type": "setMode", "destination": "session", "mode": "acceptEdits"}]))
        );
    }

    #[test]
    fn reads_outside_the_project_offer_read_rules_for_the_session() {
        let suggestions = json!([
            rules(
                "session",
                json!([{"toolName": "Read", "ruleContent": "//tmp/outside/**"}])
            ),
            rules(
                "session",
                json!([{"toolName": "Read", "ruleContent": "//private/tmp/outside/**"}])
            ),
        ]);
        let request = suggested(
            "p1",
            "Read",
            json!({"file_path": "/tmp/outside/data.txt"}),
            suggestions.clone(),
        );
        assert_eq!(updates(request), Some(suggestions));

        let mut request = suggested(
            "p2",
            "Read",
            json!({"file_path": "/tmp/outside/data.txt"}),
            json!([rules(
                "session",
                json!([{"toolName": "Read", "ruleContent": "//tmp/outside/**"}])
            )]),
        );
        request["request"]["decision_reason_code"] = json!("outside_reads_blocked");
        assert_eq!(updates(request), None);
    }

    #[test]
    fn web_fetch_offers_the_request_domain() {
        let domain = json!({"type": "addRules", "destination": "localSettings", "rules": [{"toolName": "WebFetch", "ruleContent": "domain:example.com"}], "behavior": "allow"});
        let request = suggested(
            "p1",
            "WebFetch",
            json!({"url": "https://Example.com/page?q=1", "prompt": "Title?"}),
            json!([domain]),
        );
        assert_eq!(
            updates(request),
            Some(json!([rules(
                "localSettings",
                json!([{"toolName": "WebFetch", "ruleContent": "domain:example.com"}])
            )]))
        );

        // A rule for another domain is not the row the terminal shows.
        let request = suggested(
            "p2",
            "WebFetch",
            json!({"url": "https://other.example/", "prompt": "Title?"}),
            json!([domain]),
        );
        assert_eq!(updates(request), None);
    }

    #[test]
    fn other_tools_offer_a_rule_for_the_whole_tool() {
        let tool = rules("localSettings", json!([{"toolName": "WebSearch"}]));
        let request = suggested(
            "p1",
            "WebSearch",
            json!({"query": "example domain iana"}),
            json!([tool.clone()]),
        );
        assert_eq!(updates(request), Some(json!([tool])));

        // Narrower or differently kept rules are not the terminal's row.
        let request = suggested(
            "p2",
            "mcp__server__tool",
            json!({}),
            json!([
                rules("session", json!([{"toolName": "mcp__server__tool"}])),
                rules(
                    "localSettings",
                    json!([{"toolName": "mcp__server__tool", "ruleContent": "x"}])
                ),
            ]),
        );
        assert_eq!(updates(request), None);
    }

    #[test]
    fn no_option_when_the_terminal_withholds_the_row() {
        let suggestions = json!([rules(
            "localSettings",
            json!([{"toolName": "Bash", "ruleContent": "make"}])
        )]);
        for (field, value) in [
            ("suppress_always_allow_rule", json!(true)),
            ("requires_user_interaction", json!(true)),
            ("classifier_approvable", json!(false)),
            ("decision_reason_code", json!("org_ask_ceiling")),
        ] {
            let mut request = suggested(
                "p1",
                "Bash",
                json!({"command": "make"}),
                suggestions.clone(),
            );
            request["request"][field] = value;
            assert_eq!(updates(request), None, "{field}");
        }

        let mut state = State::new("session".into());
        assert_eq!(
            options(&mut state, can_use_tool("p2", "Bash", json!({}))),
            Vec::new()
        );
        let error = state
            .decide(
                "p2",
                Decision::Grant {
                    option: SUGGESTIONS_OPTION.into(),
                },
            )
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Invalid);
    }

    #[test]
    fn allowing_once_grants_nothing() {
        let mut state = State::new("session".into());
        handle(
            &mut state,
            suggested(
                "p1",
                "Bash",
                json!({"command": "make"}),
                json!([rules(
                    "localSettings",
                    json!([{"toolName": "Bash", "ruleContent": "make"}])
                )]),
            ),
        );
        let (response, _) = state.decide("p1", Decision::AllowOnce).unwrap();
        assert_eq!(
            response["response"]["response"],
            json!({"behavior": "allow", "updatedInput": {"command": "make"}})
        );
    }

    #[test]
    fn maps_extended_question_kinds() {
        let mut state = State::new("session".into());
        let input = json!({
            "title": "Project setup",
            "questions": [
                {"question": "What should it be called?", "header": "Name", "kind": "text"},
                {"question": "How many workers?", "header": "Workers", "kind": "number", "min": 1, "max": 8},
                {"question": "Pick a colour.", "header": "Colour", "kind": "colour"},
            ],
        });
        let events = handle(
            &mut state,
            can_use_tool("question-1", "AskUserQuestion", input),
        );
        let Event::QuestionAsked(question) = &events[0] else {
            panic!("{events:?}")
        };
        assert_eq!(question.title, "Project setup");
        let kinds: Vec<_> = question.fields.iter().map(|field| field.kind).collect();
        assert_eq!(
            kinds,
            vec![FieldKind::Text, FieldKind::Text, FieldKind::Unsupported]
        );
        assert!(question.fields.iter().all(|field| !field.custom));

        let error = state
            .answer(
                "question-1",
                vec![("Pick a colour.".into(), AnswerValue::Text("red".into()))],
            )
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Invalid);

        let (response, _) = state
            .answer(
                "question-1",
                vec![(
                    "What should it be called?".into(),
                    AnswerValue::Text("Tomori".into()),
                )],
            )
            .unwrap();
        assert_eq!(
            response["response"]["response"]["updatedInput"]["answers"],
            json!({"What should it be called?": "Tomori"})
        );
    }

    #[test]
    fn malformed_questions_are_permissions() {
        let mut state = State::new("session".into());
        let events = handle(
            &mut state,
            can_use_tool("question-1", "AskUserQuestion", json!({"questions": "?"})),
        );
        assert!(
            matches!(&events[0], Event::PermissionRequested(permission) if permission.tool == "AskUserQuestion")
        );
    }

    #[test]
    fn failed_turns_report_an_error() {
        let mut state = State::new("session".into());
        handle(
            &mut state,
            json!({"type": "system", "subtype": "status", "status": "requesting"}),
        );
        let events = handle(
            &mut state,
            json!({"type": "result", "subtype": "success", "is_error": true, "result": "API Error: 529 Overloaded", "terminal_reason": "completed"}),
        );
        assert_eq!(
            events,
            vec![
                Event::Error {
                    message: "API Error: 529 Overloaded".into()
                },
                Event::Idle,
            ]
        );
    }

    fn stream(event: Value) -> Value {
        json!({"type": "stream_event", "event": event, "parent_tool_use_id": null})
    }

    fn assistant(id: &str, content: Value) -> Value {
        json!({
            "type": "assistant",
            "parent_tool_use_id": null,
            "message": {"model": "claude-haiku-5-5", "id": id, "content": content},
        })
    }

    fn text_delta(index: u64, text: &str) -> Value {
        stream(
            json!({"type": "content_block_delta", "index": index, "delta": {"type": "text_delta", "text": text}}),
        )
    }

    fn deltas(events: &[Event]) -> String {
        events
            .iter()
            .filter_map(|event| match event {
                Event::TextDelta { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn complete_records_do_not_repeat_streamed_text() {
        let mut state = State::new("session".into());
        let mut events = handle(
            &mut state,
            stream(json!({"type": "message_start", "message": {"id": "msg_1"}})),
        );
        for event in [
            stream(
                json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            ),
            text_delta(0, "Hel"),
            // The record completes the block, including text that did not stream.
            assistant("msg_1", json!([{"type": "text", "text": "Hello."}])),
            // A late delta is already included.
            text_delta(0, "lo."),
            stream(json!({"type": "content_block_stop", "index": 0})),
            stream(
                json!({"type": "content_block_start", "index": 1, "content_block": {"type": "text", "text": ""}}),
            ),
            text_delta(1, " Bye."),
            // A record that disagrees with streamed text cannot retract it.
            assistant("msg_1", json!([{"type": "text", "text": " Goodbye."}])),
            text_delta(1, " Ignored."),
            // A block that did not stream is reported whole.
            assistant("msg_1", json!([{"type": "text", "text": " Other."}])),
            stream(json!({"type": "message_stop"})),
            // A record for a completed message has streamed.
            assistant("msg_1", json!([{"type": "text", "text": "Again."}])),
        ] {
            events.extend(handle(&mut state, event));
        }
        assert_eq!(deltas(&events), "Hello. Bye. Other.");
        assert_eq!(
            state.snapshot().messages[0].parts,
            vec![
                Part::Text {
                    text: "Hello.".into()
                },
                Part::Text {
                    text: " Bye.".into()
                },
                Part::Text {
                    text: " Other.".into()
                },
            ]
        );
        assert_eq!(
            events.last(),
            Some(&Event::MessageCompleted {
                message: "msg_1".into()
            })
        );
    }

    #[test]
    fn a_fallback_after_a_partial_stream_starts_a_new_message() {
        let mut state = State::new("session".into());
        let mut events = handle(
            &mut state,
            stream(json!({"type": "message_start", "message": {"id": "msg_partial"}})),
        );
        // The stream ended without a complete block, so Claude Code retried without streaming.
        events.extend(handle(
            &mut state,
            assistant(
                "msg_full",
                json!([{"type": "thinking", "thinking": "", "signature": "x"}]),
            ),
        ));
        events.extend(handle(
            &mut state,
            assistant("msg_full", json!([{"type": "text", "text": "Reply."}])),
        ));
        assert_eq!(
            state.snapshot().messages[1].parts,
            vec![Part::Text {
                text: "Reply.".into()
            }]
        );
        // Synthetic messages are not replies.
        events.extend(handle(
            &mut state,
            json!({
                "type": "assistant",
                "parent_tool_use_id": null,
                "message": {"model": "<synthetic>", "id": "msg_error", "content": [{"type": "text", "text": "API Error"}]},
            }),
        ));
        events.extend(handle(
            &mut state,
            json!({"type": "result", "subtype": "success", "is_error": false}),
        ));
        let started = |id: &str| Event::MessageStarted {
            message: id.into(),
            role: Role::Assistant,
        };
        assert_eq!(
            events,
            vec![
                Event::Busy,
                started("msg_partial"),
                Event::MessageCompleted {
                    message: "msg_partial".into()
                },
                started("msg_full"),
                Event::TextDelta {
                    message: "msg_full".into(),
                    text: "Reply.".into()
                },
                Event::MessageCompleted {
                    message: "msg_full".into()
                },
                Event::Idle,
            ]
        );
    }

    #[test]
    fn nested_workers_name_their_parent() {
        let mut state = State::new("session".into());
        let agent = |id: &str, parent: Value| {
            json!({
                "type": "assistant",
                "parent_tool_use_id": parent,
                "message": {"id": "msg_1", "content": [
                    {"type": "tool_use", "id": id, "name": "Agent", "input": {"description": "Explore", "prompt": "..."}},
                ]},
            })
        };
        handle(&mut state, agent("toolu_outer", Value::Null));
        let events = handle(&mut state, agent("toolu_inner", json!("toolu_outer")));
        // The nested tool use is not the session's own activity.
        assert_eq!(events.len(), 1);
        let Event::WorkersChanged { workers } = &events[0] else {
            panic!("{events:?}")
        };
        assert_eq!(workers[0].parent, None);
        assert_eq!(workers[1].parent.as_deref(), Some("toolu_outer"));

        let events = handle(
            &mut state,
            json!({
                "type": "user",
                "parent_tool_use_id": "toolu_outer",
                "message": {"content": [{"type": "tool_result", "tool_use_id": "toolu_inner", "is_error": true, "content": "failed"}]},
            }),
        );
        let Event::WorkersChanged { workers } = &events[0] else {
            panic!("{events:?}")
        };
        assert_eq!(workers[1].state, WorkerState::Error);
    }

    fn tool_use(id: &str, name: &str, input: Value) -> Value {
        json!({
            "type": "assistant",
            "parent_tool_use_id": null,
            "message": {"model": "claude-opus-5-5", "id": format!("msg_{id}"), "content": [
                {"type": "tool_use", "id": id, "name": name, "input": input},
            ]},
        })
    }

    fn tool_result(id: &str, text: &str, output: Value) -> Value {
        json!({
            "type": "user",
            "parent_tool_use_id": null,
            "message": {"role": "user", "content": [{"tool_use_id": id, "type": "tool_result", "content": [{"type": "text", "text": text}]}]},
            "tool_use_result": output,
        })
    }

    fn task(subtype: &str, fields: Value) -> Value {
        let mut value = json!({"type": "system", "subtype": subtype, "session_id": "session"});
        for (key, field) in fields.as_object().unwrap() {
            value[key] = field.clone();
        }
        value
    }

    fn states(state: &State) -> Vec<(String, WorkerState)> {
        state
            .snapshot()
            .workers
            .into_iter()
            .map(|worker| (worker.id, worker.state))
            .collect()
    }

    fn worker_states(events: &[Event]) -> Vec<Vec<WorkerState>> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::WorkersChanged { workers } => {
                    Some(workers.iter().map(|worker| worker.state).collect())
                }
                _ => None,
            })
            .collect()
    }

    // The resume below follows Claude Code 2.1.311: `SendMessage` to a finished subagent starts a new run of it in the
    // background, named by the same agent id, a new run id and the `SendMessage` tool use, and its completion
    // notification names that tool use rather than the `Agent` tool use that first started it.

    #[test]
    fn a_resumed_worker_runs_again_and_then_finishes() {
        let mut state = State::new("session".into());
        let agent = "a28dae036c1fdfad9";
        handle(
            &mut state,
            tool_use(
                "toolu_agent",
                "Agent",
                json!({"description": "Review the design", "subagent_type": "general-purpose", "prompt": "..."}),
            ),
        );
        handle(
            &mut state,
            task(
                "task_started",
                json!({"task_id": agent, "run_id": "0mvyn868n-91ea8097", "tool_use_id": "toolu_agent", "description": "Review the design", "subagent_type": "general-purpose", "is_backgrounded": true, "task_type": "local_agent"}),
            ),
        );
        handle(
            &mut state,
            tool_result(
                "toolu_agent",
                "Async agent launched successfully.\nagentId: a28dae036c1fdfad9",
                json!({"isAsync": true, "status": "async_launched", "agentId": agent}),
            ),
        );
        assert_eq!(
            states(&state),
            vec![("toolu_agent".into(), WorkerState::Running)]
        );
        handle(
            &mut state,
            task(
                "task_notification",
                json!({"task_id": agent, "run_id": "0mvyn868n-91ea8097", "tool_use_id": "toolu_agent", "status": "completed", "summary": "Agent \"Review the design\" finished"}),
            ),
        );
        assert_eq!(
            states(&state),
            vec![("toolu_agent".into(), WorkerState::Completed)]
        );

        // `SendMessage` resumes it.
        handle(
            &mut state,
            tool_use(
                "toolu_send",
                "SendMessage",
                json!({"to": agent, "summary": "Confirm the fixes", "message": "..."}),
            ),
        );
        let mut events = handle(
            &mut state,
            task(
                "task_started",
                json!({"task_id": agent, "run_id": "0mvynahll-947d762a", "tool_use_id": "toolu_send", "description": "Review the design", "subagent_type": "general-purpose", "is_backgrounded": true, "task_type": "local_agent"}),
            ),
        );
        events.extend(handle(
            &mut state,
            tool_result("toolu_send", r#"{"success":true,"message":"Resuming agent a28dae0","resumedAgentId":"a28dae036c1fdfad9"}"#, json!({"success": true, "message": "Resuming agent a28dae0", "resumedAgentId": agent})),
        ));
        assert_eq!(worker_states(&events), vec![vec![WorkerState::Running]]);
        // The resumed run's activity belongs to the same worker.
        handle(
            &mut state,
            json!({
                "type": "assistant",
                "parent_tool_use_id": "toolu_send",
                "message": {"id": "msg_nested", "content": [
                    {"type": "tool_use", "id": "toolu_nested", "name": "Agent", "input": {"description": "Explore", "prompt": "..."}},
                ]},
            }),
        );
        assert_eq!(
            state.snapshot().workers[1].parent.as_deref(),
            Some("toolu_agent")
        );

        // A late notification of the earlier run changes nothing.
        let events = handle(
            &mut state,
            task(
                "task_notification",
                json!({"task_id": agent, "run_id": "0mvyn868n-91ea8097", "tool_use_id": "toolu_agent", "status": "completed", "summary": ""}),
            ),
        );
        assert!(events.is_empty(), "{events:?}");

        let events = handle(
            &mut state,
            task(
                "task_notification",
                json!({"task_id": agent, "run_id": "0mvynahll-947d762a", "tool_use_id": "toolu_send", "status": "completed", "summary": "Agent \"Review the design\" finished"}),
            ),
        );
        assert_eq!(
            worker_states(&events),
            vec![vec![WorkerState::Completed, WorkerState::Running]]
        );
        assert_eq!(state.snapshot().workers.len(), 2);
    }

    #[test]
    fn a_resumed_worker_the_session_did_not_report_is_added() {
        let mut state = State::new("session".into());
        // Only the text of the result is reported.
        let events = handle(
            &mut state,
            tool_result(
                "toolu_send",
                r#"{"success":true,"message":"Resuming agent a2dfc76","resumedAgentId":"a2dfc762df22d6d4b"}"#,
                Value::Null,
            ),
        );
        let Event::WorkersChanged { workers } = &events[0] else {
            panic!("{events:?}")
        };
        assert_eq!(
            workers[..],
            [Worker {
                id: "toolu_send".into(),
                parent: None,
                title: "Agent".into(),
                state: WorkerState::Running,
            }]
        );
        handle(
            &mut state,
            task(
                "task_started",
                json!({"task_id": "a2dfc762df22d6d4b", "run_id": "0mvyq0000-00000000", "tool_use_id": "toolu_send", "description": "Implement arrival transitions", "is_backgrounded": true, "task_type": "local_agent"}),
            ),
        );
        assert_eq!(
            state.snapshot().workers[0].title,
            "Implement arrival transitions"
        );
        handle(
            &mut state,
            task(
                "task_notification",
                json!({"task_id": "a2dfc762df22d6d4b", "tool_use_id": "toolu_send", "status": "failed", "summary": ""}),
            ),
        );
        assert_eq!(
            states(&state),
            vec![("toolu_send".into(), WorkerState::Error)]
        );
    }

    #[test]
    fn a_resume_the_tool_waits_for_ends_with_its_result() {
        let mut state = State::new("session".into());
        handle(
            &mut state,
            tool_use("toolu_agent", "Agent", json!({"description": "Explore"})),
        );
        handle(
            &mut state,
            tool_result(
                "toolu_agent",
                "Found it.",
                json!({"status": "completed", "agentId": "a1"}),
            ),
        );
        handle(
            &mut state,
            tool_use(
                "toolu_send",
                "SendMessage",
                json!({"to": "a1", "message": "More."}),
            ),
        );
        let events = handle(
            &mut state,
            task(
                "task_started",
                json!({"task_id": "a1", "run_id": "2", "tool_use_id": "toolu_send", "description": "Explore", "is_backgrounded": false, "awaited": true, "task_type": "local_agent"}),
            ),
        );
        assert_eq!(worker_states(&events), vec![vec![WorkerState::Running]]);
        let events = handle(
            &mut state,
            tool_result("toolu_send", "Found more.", Value::Null),
        );
        assert_eq!(worker_states(&events), vec![vec![WorkerState::Completed]]);
    }

    #[test]
    fn background_shells_are_not_workers() {
        let mut state = State::new("session".into());
        let events = handle(
            &mut state,
            task(
                "task_started",
                json!({"task_id": "b1", "tool_use_id": "toolu_shell", "description": "sleep", "is_backgrounded": true, "task_type": "local_bash"}),
            ),
        );
        assert!(events.is_empty());
        let events = handle(
            &mut state,
            task(
                "task_notification",
                json!({"task_id": "b1", "tool_use_id": "toolu_shell", "status": "stopped"}),
            ),
        );
        assert!(events.is_empty());
    }

    #[test]
    fn workers_stop_when_the_process_ends() {
        let mut state = State::new("session".into());
        handle(
            &mut state,
            tool_use("toolu_a", "Agent", json!({"description": "A"})),
        );
        handle(
            &mut state,
            tool_use("toolu_b", "Agent", json!({"description": "B"})),
        );
        handle(&mut state, tool_result("toolu_b", "Done.", Value::Null));
        // Claude Code reports a subagent it stopped, too.
        handle(
            &mut state,
            tool_use("toolu_c", "Agent", json!({"description": "C"})),
        );
        handle(
            &mut state,
            task(
                "task_started",
                json!({"task_id": "ac", "tool_use_id": "toolu_c", "description": "C", "is_backgrounded": true, "task_type": "local_agent"}),
            ),
        );
        handle(
            &mut state,
            task(
                "task_updated",
                json!({"task_id": "ac", "patch": {"status": "killed"}}),
            ),
        );
        let events = state.stop_workers();
        assert_eq!(
            worker_states(&events),
            vec![vec![
                WorkerState::Stopped,
                WorkerState::Completed,
                WorkerState::Stopped
            ]]
        );
        assert!(state.stop_workers().is_empty());
    }
}
