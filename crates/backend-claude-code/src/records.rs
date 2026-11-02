// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The workers of an existing session, read from Claude Code's own records of it.
//!
//! Claude Code 2.1 keeps a session's transcript in `<session>.jsonl`, and each subagent's transcript and metadata in
//! `<session>/subagents/agent-<agent id>.jsonl` and `.meta.json`. The transcript records the `Agent` tool uses and
//! their results, `SendMessage` results that resumed a subagent, and the task notifications delivered to the session.
//! A subagent's metadata names the tool use that started it and, once its latest run ended, `"finished": true`.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde_json::Value;

use crate::workers::{self, WORKER_TOOLS, Workers};

/// The workers that session `session`'s records in `transcripts` show, with their last known states. The process that
/// ran them has ended, so none is reported as running. Blocks on file I/O.
pub(crate) fn workers(transcripts: &Path, session: &str) -> Workers {
    let mut workers = Workers::default();
    replay(
        &transcripts.join(format!("{session}.jsonl")),
        None,
        &mut workers,
    );

    let mut subagents: Vec<(String, Value)> =
        fs::read_dir(transcripts.join(session).join("subagents"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let agent = name
                    .strip_prefix("agent-")?
                    .strip_suffix(".meta.json")?
                    .to_owned();
                let meta = serde_json::from_slice(&fs::read(entry.path()).ok()?).ok()?;
                Some((agent, meta))
            })
            .collect();
    subagents.sort_by(|a, b| a.0.cmp(&b.0));

    // Workers that subagents started are recorded in their transcripts, and are theirs.
    for (agent, meta) in &subagents {
        if let Some(tool) = meta["toolUseId"].as_str() {
            let path = transcripts
                .join(session)
                .join("subagents")
                .join(format!("agent-{agent}.jsonl"));
            replay(&path, Some(tool), &mut workers);
        }
    }
    // A subagent whose start is not in the transcripts, such as one whose records were compacted, is still shown.
    for (agent, meta) in &subagents {
        let Some(tool) = meta["toolUseId"].as_str() else {
            continue;
        };
        let title = meta["description"]
            .as_str()
            .or_else(|| meta["agentType"].as_str())
            .unwrap_or("Agent");
        workers.recorded(agent, tool, title);
    }
    for (_, meta) in &subagents {
        if meta["finished"].as_bool() == Some(true)
            && let Some(tool) = meta["toolUseId"].as_str()
        {
            workers.finished(tool);
        }
    }

    workers.stop();
    workers
}

/// Apply the worker records of one transcript, in order. `parent` is the worker whose transcript it is, if any.
fn replay(path: &Path, parent: Option<&str>, workers: &mut Workers) {
    let Ok(file) = fs::File::open(path) else {
        return;
    };
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        // Only records with tool activity or notifications matter, so most records are not parsed.
        if !line.contains("tool_use") && !line.contains("<task-notification>") {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        // Earlier versions kept subagent records in the session's transcript; they are not the session's own.
        if parent.is_none() && value["isSidechain"].as_bool() == Some(true) {
            continue;
        }
        match value["type"].as_str() {
            Some("assistant") => {
                for block in value["message"]["content"].as_array().into_iter().flatten() {
                    let (Some("tool_use"), Some(id), Some(tool)) = (
                        block["type"].as_str(),
                        block["id"].as_str(),
                        block["name"].as_str(),
                    ) else {
                        continue;
                    };
                    if WORKER_TOOLS.contains(&tool) {
                        workers.start(id, parent, workers::title(&block["input"], tool));
                    }
                }
            }
            Some("user") => user(&value, parent, workers),
            _ => {}
        }
    }
}

fn user(value: &Value, parent: Option<&str>, workers: &mut Workers) {
    let run = value["origin"]["runId"].as_str();
    match &value["message"]["content"] {
        Value::String(text) => notification(text, run, workers),
        Value::Array(blocks) => {
            let results = blocks
                .iter()
                .filter(|block| block["type"] == "tool_result")
                .count();
            let output = value.get("toolUseResult").filter(|_| results == 1);
            for block in blocks {
                match block["type"].as_str() {
                    Some("tool_result") => {
                        let Some(id) = block["tool_use_id"].as_str() else {
                            continue;
                        };
                        let failed = block["is_error"].as_bool() == Some(true);
                        let text = text(&block["content"]);
                        workers.tool_result(id, failed, output, &text, parent);
                    }
                    Some("text") => {
                        if let Some(text) = block["text"].as_str() {
                            notification(text, run, workers);
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// A `<task-notification>` delivered to the session as a user message.
fn notification(text: &str, run: Option<&str>, workers: &mut Workers) {
    let text = text.trim_start();
    if !text.starts_with("<task-notification>") {
        return;
    }
    workers.notified(
        tag(text, "task-id"),
        tag(text, "tool-use-id"),
        tag(text, "status"),
        run,
    );
}

/// The text of the first `<name>` element of a notification.
fn tag<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&format!("</{name}>"))?;
    Some(text[start..start + end].trim())
}

fn text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backend::WorkerState;

    /// A session whose records are condensed from Claude Code 2.1.311 transcripts: a reviewer that finished, was
    /// resumed with `SendMessage` and finished again; an implementer that was running when the process ended, reported
    /// stopped and resumed, with a nested subagent of its own; a test writer whose metadata records that it finished;
    /// a foreground subagent that failed; and a background shell.
    const SESSION: &str = "5d1c3a52-7f0e-4c39-9a51-2b7f4c0d9e11";

    fn fixture() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/records")
    }

    #[test]
    fn attaching_reports_the_workers_the_records_show() {
        let workers = workers(&fixture(), SESSION);
        let found: Vec<_> = workers
            .list()
            .iter()
            .map(|worker| {
                (
                    worker.id.as_str(),
                    worker.parent.as_deref(),
                    worker.title.as_str(),
                    worker.state,
                )
            })
            .collect();
        assert_eq!(
            found,
            vec![
                (
                    "toolu_review",
                    None,
                    "Review the design",
                    WorkerState::Completed
                ),
                // Running when the process that ran it ended, so not running now.
                (
                    "toolu_implement",
                    None,
                    "Implement the design",
                    WorkerState::Stopped
                ),
                (
                    "toolu_tests",
                    None,
                    "Write the tests",
                    WorkerState::Completed
                ),
                ("toolu_lint", None, "Check the lints", WorkerState::Error),
                (
                    "toolu_nested",
                    Some("toolu_implement"),
                    "Find the call sites",
                    WorkerState::Stopped
                ),
            ]
        );
    }

    #[test]
    fn workers_from_the_records_are_resumed_by_their_agent_id() {
        let mut workers = workers(&fixture(), SESSION);
        let resume = serde_json::json!({"success": true, "message": "Resuming agent a2222bb", "resumedAgentId": "a2222bbbb2222bbbb"});
        assert!(workers.tool_result("toolu_again", false, Some(&resume), "", None));
        assert_eq!(workers.list()[1].state, WorkerState::Running);
        // Its completion names the tool use that resumed it.
        assert!(workers.notified(
            Some("a2222bbbb2222bbbb"),
            Some("toolu_again"),
            Some("completed"),
            None
        ));
        assert_eq!(workers.list()[1].state, WorkerState::Completed);
        assert_eq!(workers.list().len(), 5);
    }

    #[test]
    fn a_session_without_records_has_no_workers() {
        assert!(
            workers(&fixture(), "00000000-0000-4000-8000-000000000000")
                .list()
                .is_empty()
        );
    }
}
