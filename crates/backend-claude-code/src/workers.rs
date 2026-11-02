// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Delegated workers as Claude Code reports them, on the stream and in a session's records.
//!
//! A worker is named by the id of the `Agent` tool use that started it. Claude Code names the subagent itself by its
//! own agent id (a task id, such as `a28dae036c1fdfad9`), and every later run of it, such as one started by
//! `SendMessage`, by the id of the tool use that resumed it. Its task events and notifications name both, so a later
//! run is tied back to the worker that first started it. As Claude Code 2.1.311 reports them:
//!
//! - `task_started` names the agent id, the tool use that started this run and a run id that is new for each run. A
//!   resumed subagent is always registered in the background.
//! - `task_updated` names the agent id and a patch of its changed status.
//! - `task_notification` names the agent id and the tool use of the run that ended, with `completed`, `failed` or
//!   `stopped`. Claude Code reports a subagent that was running when its previous process ended as `stopped`.
//! - The `Agent` tool result names the agent id, and a `SendMessage` tool result that resumed a finished subagent
//!   names it as `resumedAgentId`.

use std::collections::{HashMap, HashSet};

use backend::{Worker, WorkerState};
use serde_json::Value;

/// Tools that delegate to a subagent. Claude Code 2.1 names it `Agent`; earlier versions name it `Task`.
pub(crate) const WORKER_TOOLS: [&str; 2] = ["Agent", "Task"];

/// The title of a worker found only by a later run, until its own description arrives.
const UNTITLED: &str = "Agent";

#[derive(Debug)]
struct Agent {
    /// The id of its worker.
    worker: String,
    /// The latest run, so that a late event from an earlier run changes nothing. Claude Code's run ids sort in the
    /// order the runs opened.
    run: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct Workers {
    list: Vec<Worker>,
    /// Subagents by Claude Code's agent id.
    agents: HashMap<String, Agent>,
    /// Tool uses that run a worker other than the one that started it, such as `SendMessage` resuming it.
    aliases: HashMap<String, String>,
    /// Tool uses that run in the background, so their tool result does not mean the run ended.
    background: HashSet<String>,
}

/// A worker's title from its tool input.
pub(crate) fn title<'a>(input: &'a Value, tool: &'a str) -> &'a str {
    str_field(input, "description")
        .or_else(|| str_field(input, "subagent_type"))
        .unwrap_or(tool)
}

impl Workers {
    pub(crate) fn list(&self) -> &[Worker] {
        &self.list
    }

    /// The worker that a tool use runs: the one it started, or the one it resumed.
    fn resolve(&self, tool: &str) -> Option<String> {
        if self.list.iter().any(|worker| worker.id == tool) {
            return Some(tool.to_owned());
        }
        self.aliases.get(tool).cloned()
    }

    fn push(&mut self, id: &str, parent: Option<&str>, title: &str) {
        let parent = parent.map(|parent| self.resolve(parent).unwrap_or_else(|| parent.to_owned()));
        self.list.push(Worker {
            id: id.into(),
            parent,
            title: title.into(),
            state: WorkerState::Running,
        });
    }

    /// A worker tool use started a subagent. `parent` is the tool use whose subagent made the call, if any.
    pub(crate) fn start(&mut self, id: &str, parent: Option<&str>, title: &str) -> bool {
        if self.resolve(id).is_some() {
            return false;
        }
        self.push(id, parent, title);
        true
    }

    fn set(&mut self, id: &str, state: WorkerState) -> bool {
        match self.list.iter_mut().find(|worker| worker.id == id) {
            Some(worker) if worker.state != state => {
                worker.state = state;
                true
            }
            _ => false,
        }
    }

    /// Whether `run` of the agent opened before the latest run seen.
    fn stale(&self, agent: &str, run: Option<&str>) -> bool {
        match (
            self.agents
                .get(agent)
                .and_then(|agent| agent.run.as_deref()),
            run,
        ) {
            (Some(latest), Some(run)) => run < latest,
            _ => false,
        }
    }

    /// Tie a subagent to its worker, without changing the run it is on.
    fn link(&mut self, agent: &str, worker: &str) {
        self.agents.entry(agent.into()).or_insert_with(|| Agent {
            worker: worker.into(),
            run: None,
        });
    }

    /// The worker of `agent`, or a new one for a subagent this session has not reported, started by `tool`.
    fn worker_of(
        &mut self,
        agent: &str,
        tool: Option<&str>,
        parent: Option<&str>,
        title: Option<&str>,
    ) -> (String, bool) {
        if let Some(worker) = tool.and_then(|tool| self.resolve(tool)) {
            return (worker, false);
        }
        if let Some(agent) = self.agents.get(agent) {
            return (agent.worker.clone(), false);
        }
        let id = tool.unwrap_or(agent).to_owned();
        self.push(&id, parent, title.unwrap_or(UNTITLED));
        (id, true)
    }

    /// A `task_started` system message: a subagent started, or started again.
    pub(crate) fn task_started(&mut self, value: &Value) -> bool {
        let tool = str_field(value, "tool_use_id");
        if value["is_backgrounded"].as_bool() == Some(true)
            && let Some(tool) = tool
        {
            self.background.insert(tool.into());
        }
        let Some(agent) = str_field(value, "task_id") else {
            return false;
        };
        // Background shells and other tasks are not workers.
        let subagent = match str_field(value, "task_type") {
            Some(kind) => kind == "local_agent",
            None => value.get("subagent_type").is_some(),
        };
        let run = str_field(value, "run_id");
        if !subagent || self.stale(agent, run) {
            return false;
        }

        let parent = str_field(value, "parent_task_id")
            .and_then(|parent| self.agents.get(parent))
            .map(|parent| parent.worker.clone());
        let description = str_field(value, "description").filter(|text| !text.is_empty());
        let (worker, created) = self.worker_of(agent, tool, parent.as_deref(), description);
        if let Some(tool) = tool
            && tool != worker
        {
            self.aliases.insert(tool.into(), worker.clone());
        }
        self.agents.insert(
            agent.into(),
            Agent {
                worker: worker.clone(),
                run: run.map(Into::into),
            },
        );
        let mut changed = created;
        if let Some(description) = description
            && let Some(entry) = self.list.iter_mut().find(|entry| entry.id == worker)
            && entry.title == UNTITLED
        {
            entry.title = description.into();
            changed = true;
        }
        self.set(&worker, WorkerState::Running) || changed
    }

    /// A `task_updated` system message.
    pub(crate) fn task_updated(&mut self, value: &Value) -> bool {
        let Some(agent) = str_field(value, "task_id") else {
            return false;
        };
        if self.stale(agent, str_field(value, "run_id")) {
            return false;
        }
        let Some(worker) = self.agents.get(agent).map(|agent| agent.worker.clone()) else {
            return false;
        };
        let patch = &value["patch"];
        if patch["is_backgrounded"].as_bool() == Some(true) {
            // A foreground subagent moved to the background: the tool uses running it no longer end with it.
            self.background.insert(worker.clone());
            let aliases: Vec<String> = self
                .aliases
                .iter()
                .filter(|(_, target)| **target == worker)
                .map(|(tool, _)| tool.clone())
                .collect();
            self.background.extend(aliases);
        }
        let state = match str_field(patch, "status") {
            Some("running") => WorkerState::Running,
            Some("completed") => WorkerState::Completed,
            Some("failed") => WorkerState::Error,
            Some("killed") => WorkerState::Stopped,
            Some("paused") => WorkerState::Idle,
            _ => return false,
        };
        self.set(&worker, state)
    }

    /// A task notification, from the stream or a session record: the run of `agent` started by `tool` ended.
    pub(crate) fn notified(
        &mut self,
        agent: Option<&str>,
        tool: Option<&str>,
        status: Option<&str>,
        run: Option<&str>,
    ) -> bool {
        if agent.is_some_and(|agent| self.stale(agent, run)) {
            return false;
        }
        let Some(worker) = tool.and_then(|tool| self.resolve(tool)).or_else(|| {
            agent
                .and_then(|agent| self.agents.get(agent))
                .map(|agent| agent.worker.clone())
        }) else {
            return false;
        };
        if let Some(agent) = agent {
            self.link(agent, &worker);
        }
        let state = match status {
            Some("completed") => WorkerState::Completed,
            Some("stopped") => WorkerState::Stopped,
            _ => WorkerState::Error,
        };
        self.set(&worker, state)
    }

    /// The result of tool use `id`. `result` is the tool's structured output, if reported, and `text` its content as
    /// the model reads it. `parent` is the tool use whose subagent made the call, if any.
    pub(crate) fn tool_result(
        &mut self,
        id: &str,
        failed: bool,
        result: Option<&Value>,
        text: &str,
        parent: Option<&str>,
    ) -> bool {
        // `SendMessage` reports its output as the JSON text of its result as well.
        let parsed = match result {
            Some(result) if result.is_object() => None,
            _ => serde_json::from_str::<Value>(text).ok(),
        };
        let structured = result
            .filter(|result| result.is_object())
            .or(parsed.as_ref());

        if !failed
            && let Some(agent) = structured.and_then(|result| str_field(result, "resumedAgentId"))
        {
            // A finished subagent runs again, in the background, until its notification.
            let (worker, created) = self.worker_of(agent, Some(id), parent, None);
            self.link(agent, &worker);
            if id != worker {
                self.aliases.insert(id.into(), worker.clone());
            }
            self.background.insert(id.into());
            return self.set(&worker, WorkerState::Running) || created;
        }

        let Some(worker) = self.resolve(id) else {
            return false;
        };
        let agent = structured
            .and_then(|result| str_field(result, "agentId"))
            .map(str::to_owned)
            .or_else(|| agent_id(text));
        if let Some(agent) = agent {
            self.link(&agent, &worker);
        }
        if structured.and_then(|result| str_field(result, "status")) == Some("async_launched") {
            self.background.insert(id.into());
        }
        if failed {
            self.set(&worker, WorkerState::Error)
        } else if !self.background.contains(id) && !self.background.contains(&worker) {
            self.set(&worker, WorkerState::Completed)
        } else {
            false
        }
    }

    /// The agent process running the workers ended, so none of them is running any more.
    pub(crate) fn stop(&mut self) -> bool {
        let mut changed = false;
        for worker in &mut self.list {
            if matches!(worker.state, WorkerState::Running | WorkerState::Waiting) {
                worker.state = WorkerState::Stopped;
                changed = true;
            }
        }
        changed
    }

    /// A subagent that Claude Code recorded as finished, in its own records, when nothing else said so.
    pub(crate) fn finished(&mut self, worker: &str) -> bool {
        match self.list.iter().find(|entry| entry.id == worker) {
            Some(entry) if entry.state == WorkerState::Running => {
                self.set(worker, WorkerState::Completed)
            }
            _ => false,
        }
    }

    /// Tie a subagent found in the session's records to its worker, adding the worker if no record started it.
    pub(crate) fn recorded(&mut self, agent: &str, worker: &str, title: &str) {
        if self.resolve(worker).is_none() {
            self.push(worker, None, title);
        }
        self.link(agent, worker);
    }
}

/// The agent id that an `Agent` tool result names in its text, as `agentId: <id>`.
fn agent_id(text: &str) -> Option<String> {
    let rest = &text[text.find("agentId: ")? + "agentId: ".len()..];
    let id: String = rest
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || *character == '-')
        .collect();
    (!id.is_empty()).then_some(id)
}

fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}
