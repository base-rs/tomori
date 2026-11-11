// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What each worker was last seen doing, from the events of the one event stream the connection already reads: when
//! it last made progress, a message it has not answered, the tool it is running, how its last step ended, why it
//! failed and the requests it waits on. Only those facts are kept, never the events. After the stream is lost they are
//! unknown until each worker's records are read again. The server judges from them whether a worker has failed,
//! stalled or left a message unanswered. See `design/events.md`.
use std::collections::{BTreeSet, HashMap};

use serde_json::Value;
use tomori_backend::{RunningTool, WorkerActivity};

/// How far a busy worker's progress moves on before it is reported again, in milliseconds, when nothing else about it
/// changed: often enough for thresholds of minutes, seldom enough that a worker streaming text is not reported with
/// every word.
pub(crate) const PROGRESS_STEP: u64 = 30_000;

/// The most tool calls, queued messages and pending requests kept for one worker at once; beyond it the oldest are
/// forgotten.
const MOST: usize = 16;

/// The most workers whose records are read again after the stream is lost, those last active first; the rest are
/// forgotten until they are active again.
pub(crate) const MOST_SYNCED: usize = 64;

/// The most messages of a worker read again after the stream is lost, the latest.
pub(crate) const SYNCED_MESSAGES: usize = 20;

/// The most events held for a worker while its records are read, after which only its progress is kept of the
/// rest.
const HELD: usize = 256;

/// The most characters kept of why a worker failed, and of a tool's name or a step's finish.
const ERROR_CHARS: usize = 300;
const NAME_CHARS: usize = 64;

/// Whether an event of `kind` can say anything about a worker's activity.
pub(crate) fn relevant(kind: &str) -> bool {
    progress(kind)
        || matches!(
            kind,
            "session.created"
                | "session.deleted"
                | "session.execution.started"
                | "session.execution.succeeded"
                | "session.execution.failed"
                | "session.execution.interrupted"
                | "session.step.failed"
                | "session.inbox.enqueued"
                | "session.inbox.delivered"
                | "session.inbox.cancelled"
                | "permission.asked"
                | "permission.replied"
                | "form.created"
                | "form.replied"
                | "form.cancelled"
        )
}

/// Whether an event of `kind` is the worker making progress.
fn progress(kind: &str) -> bool {
    kind.starts_with("session.step.") && kind != "session.step.failed"
        || kind.starts_with("session.text.")
        || kind.starts_with("session.reasoning.")
        || kind.starts_with("session.tool.")
        || kind.starts_with("session.compaction.")
        || kind.starts_with("session.shell.")
        || kind == "session.retry.scheduled"
}

/// The milliseconds since the Unix epoch now.
pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// `text` on one line of at most `limit` characters, its whitespace runs made single spaces.
fn bounded(text: &str, limit: usize) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() <= limit {
        return joined;
    }
    let mut cut: String = joined.chars().take(limit.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// Why a worker failed, from an OpenCode error's type and message, such as `BadRequestError: insufficient credit`.
fn describe_error(error: Option<&Value>) -> Option<String> {
    let error = error?;
    let kind = error.get("type").and_then(Value::as_str);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str());
    let text = match (kind, message) {
        (Some(kind), Some(message)) if !message.starts_with(kind) => format!("{kind}: {message}"),
        (_, Some(message)) => message.to_owned(),
        (Some(kind), None) => kind.to_owned(),
        (None, None) => return None,
    };
    Some(bounded(&text, ERROR_CHARS))
}

/// The number at `pointer` in `value`, as milliseconds.
fn millis(value: &Value, pointer: &str) -> Option<u64> {
    value
        .pointer(pointer)
        .and_then(Value::as_f64)
        .map(|at| at as u64)
}

/// One tool call a worker made.
#[derive(Debug)]
struct Call {
    id: String,
    name: String,
    /// When it began running, once it did.
    running: Option<u64>,
}

/// A message queued for a worker and not yet delivered to it.
#[derive(Debug)]
struct Queued {
    id: String,
    sent: u64,
    /// Since when it has waited on the worker, or `None` while it waits behind a run still going.
    since: Option<u64>,
}

/// A message delivered to a worker and not yet answered.
#[derive(Debug)]
struct Delivered {
    sent: u64,
    at: u64,
    /// It was delivered while a step was going, which is not an answer to it, however it streams on.
    behind_step: bool,
}

/// A worker's latest messages, oldest first, its inbox and its pending permissions and forms, as read from its records.
pub(crate) type Records<'a> = (&'a [Value], &'a [Value], &'a [Value]);

/// What one worker was last seen doing, and what was last reported of it.
#[derive(Debug)]
pub(crate) struct Tracker {
    current: WorkerActivity,
    calls: Vec<Call>,
    queued: Vec<Queued>,
    delivered: Option<Delivered>,
    /// The permissions and forms it waits on, by their ids.
    requests: Vec<String>,
    /// A run is going, as far as its events or records say, or `None` while neither has said.
    running: Option<bool>,
    /// A step is going.
    stepping: bool,
    reported: Option<WorkerActivity>,
    /// Its activity is known: it has not been unknown since the stream was lost, or its records were read again since.
    synced: bool,
    /// The events applied since its activity became unknown, held to be applied again over its records once they are
    /// read, a run of streamed progress kept as one; `None` once more than [`HELD`] arrived.
    held: Option<Vec<(String, Value, u64)>>,
}

impl Default for Tracker {
    fn default() -> Self {
        Self {
            current: WorkerActivity::default(),
            calls: Vec::new(),
            queued: Vec::new(),
            delivered: None,
            requests: Vec::new(),
            running: None,
            stepping: false,
            reported: None,
            synced: true,
            held: Some(Vec::new()),
        }
    }
}

impl Tracker {
    /// Apply an event of `kind`, with its `data`, observed at `at`.
    pub(crate) fn apply(&mut self, kind: &str, data: &Value, at: u64) {
        if !self.synced {
            self.hold(kind, data, at);
        }
        // A worker first seen now has made progress no earlier than now, so it is never judged from nothing.
        if self.current.progressed == 0 {
            self.current.progressed = at;
        }
        let mut calls_changed = false;
        if progress(kind) {
            self.current.progressed = self.current.progressed.max(at);
            self.running = Some(true);
            // A message delivered is answered by progress in a step begun after it, never by the step it arrived in.
            if self
                .delivered
                .as_ref()
                .is_some_and(|delivered| kind == "session.step.started" || !delivered.behind_step)
            {
                self.delivered = None;
            }
        }
        let id = data.get("id").and_then(Value::as_str);
        match kind {
            "session.execution.started" => {
                self.current.progressed = self.current.progressed.max(at);
                self.current.error = None;
                self.running = Some(true);
            }
            "session.execution.succeeded"
            | "session.execution.interrupted"
            | "session.execution.failed" => {
                if kind == "session.execution.failed" {
                    if let Some(error) = describe_error(data.get("error")) {
                        self.current.error = Some(error);
                    }
                    self.current
                        .error
                        .get_or_insert_with(|| "the agent's run failed".to_owned());
                }
                self.running = Some(false);
                self.stepping = false;
                if let Some(delivered) = &mut self.delivered {
                    delivered.behind_step = false;
                }
                // A message queued behind the run is the worker's to answer now that the run has ended.
                for queued in &mut self.queued {
                    queued.since.get_or_insert(at);
                }
                self.requests.clear();
                calls_changed = !self.calls.is_empty();
                self.calls.clear();
            }
            "session.step.started" => self.stepping = true,
            "session.step.ended" => {
                self.stepping = false;
                if let Some(delivered) = &mut self.delivered {
                    delivered.behind_step = false;
                }
                let finish = data.get("finish").and_then(Value::as_str);
                if self.current.finish.as_deref() != finish {
                    self.current.finish = finish.map(|finish| bounded(finish, NAME_CHARS));
                }
            }
            "session.step.failed" => {
                if let Some(error) = describe_error(data.get("error")) {
                    self.current.error = Some(error);
                }
            }
            "session.inbox.enqueued" => {
                let user = data.pointer("/item/type").and_then(Value::as_str) == Some("user");
                if let (true, Some(inbox)) = (user, data.get("inboxID").and_then(Value::as_str)) {
                    // Only a worker known to be idle has it to answer at once.
                    self.queue(inbox, at, (self.running == Some(false)).then_some(at));
                }
            }
            "session.inbox.delivered" => {
                // The message by its id, or else the oldest queued, as the inbox delivers in order.
                let inbox = data.get("inboxID").and_then(Value::as_str);
                let index = inbox
                    .and_then(|inbox| self.queued.iter().position(|queued| queued.id == inbox))
                    .or_else(|| (!self.queued.is_empty()).then_some(0));
                let sent = index.map_or(at, |index| self.queued.remove(index).sent);
                if self.delivered.is_none() {
                    self.delivered = Some(Delivered {
                        sent,
                        at,
                        behind_step: self.stepping,
                    });
                }
            }
            "session.inbox.cancelled" => {
                if let Some(inbox) = data.get("inboxID").and_then(Value::as_str) {
                    self.queued.retain(|queued| queued.id != inbox);
                }
            }
            "permission.asked" | "form.created" => {
                let request = id.or_else(|| data.pointer("/form/id").and_then(Value::as_str));
                if let Some(request) = request
                    && !self.requests.iter().any(|known| known == request)
                {
                    if self.requests.len() == MOST {
                        self.requests.remove(0);
                    }
                    self.requests.push(request.to_owned());
                }
            }
            "permission.replied" | "form.replied" | "form.cancelled" => {
                let request = data
                    .get("requestID")
                    .and_then(Value::as_str)
                    .or(id)
                    .or_else(|| data.pointer("/form/id").and_then(Value::as_str));
                if let Some(request) = request {
                    self.requests.retain(|known| known != request);
                }
            }
            "session.tool.input.started" => {
                if let Some(id) = id {
                    let name = data
                        .get("name")
                        .and_then(Value::as_str)
                        .map_or_else(|| "a tool".to_owned(), |name| bounded(name, NAME_CHARS));
                    self.calls.retain(|call| call.id != id);
                    self.call(id, name, None);
                    calls_changed = true;
                }
            }
            "session.tool.called" => {
                if let Some(id) = id {
                    match self.calls.iter_mut().find(|call| call.id == id) {
                        Some(call) => {
                            calls_changed = call.running.is_none();
                            call.running.get_or_insert(at);
                        }
                        None => {
                            self.call(id, "a tool".to_owned(), Some(at));
                            calls_changed = true;
                        }
                    }
                }
            }
            "session.tool.success" | "session.tool.failed" => {
                if let Some(id) = id {
                    let before = self.calls.len();
                    self.calls.retain(|call| call.id != id);
                    calls_changed = self.calls.len() != before;
                }
            }
            _ => {}
        }
        self.derive(calls_changed);
    }

    fn call(&mut self, id: &str, name: String, running: Option<u64>) {
        if self.calls.len() == MOST {
            self.calls.remove(0);
        }
        self.calls.push(Call {
            id: id.to_owned(),
            name,
            running,
        });
    }

    fn queue(&mut self, id: &str, sent: u64, since: Option<u64>) {
        if self.queued.iter().any(|queued| queued.id == id) {
            return;
        }
        if self.queued.len() == MOST {
            self.queued.remove(0);
        }
        self.queued.push(Queued {
            id: id.to_owned(),
            sent,
            since,
        });
    }

    /// Work out what is reported from what is kept: the message longest unanswered, the tool that has run longest, if
    /// the calls changed, and whether it waits.
    fn derive(&mut self, calls_changed: bool) {
        let delivered = self
            .delivered
            .as_ref()
            .map(|delivered| (delivered.sent, Some(delivered.at)));
        let queued = self.queued.iter().map(|queued| (queued.sent, queued.since));
        // Those already the worker's to answer first, the longest waiting; else the first sent.
        let message = delivered
            .into_iter()
            .chain(queued)
            .min_by_key(|(sent, since)| (since.is_none(), since.unwrap_or(*sent), *sent));
        self.current.message = message.map(|(sent, _)| sent);
        self.current.unanswered_since = message.and_then(|(_, since)| since);
        self.current.waiting = !self.requests.is_empty();
        if calls_changed {
            self.current.tool = self
                .calls
                .iter()
                .filter_map(|call| call.running.map(|since| (call, since)))
                .min_by_key(|(_, since)| *since)
                .map(|(call, since)| RunningTool {
                    name: call.name.clone(),
                    since,
                });
        }
    }

    /// What to report now, if its activity is known and differs from what was last reported in more than progress, or
    /// progress moved on by at least [`PROGRESS_STEP`].
    pub(crate) fn due(&self) -> bool {
        if !self.synced {
            return false;
        }
        let Some(reported) = &self.reported else {
            return true;
        };
        let current = &self.current;
        current.progressed >= reported.progressed.saturating_add(PROGRESS_STEP)
            || current.message != reported.message
            || current.unanswered_since != reported.unanswered_since
            || current.tool != reported.tool
            || current.finish != reported.finish
            || current.error != reported.error
            || current.waiting != reported.waiting
    }

    /// Hold an event applied while its activity is unknown, a run of streamed progress kept as its latest.
    fn hold(&mut self, kind: &str, data: &Value, at: u64) {
        let Some(held) = &mut self.held else {
            return;
        };
        let streamed = kind.ends_with(".delta");
        if streamed
            && let Some((last, _, when)) = held.last_mut()
            && last == kind
        {
            *when = at;
            return;
        }
        if held.len() == HELD {
            self.held = None;
            return;
        }
        let data = if streamed { Value::Null } else { data.clone() };
        held.push((kind.to_owned(), data, at));
    }

    /// The worker's activity became unknown, as when the stream was lost: everything kept is forgotten, and nothing is
    /// reported until its records are read again.
    fn unsync(&mut self) {
        *self = Self {
            synced: false,
            ..Self::default()
        };
    }

    /// Its records were read again at `at`: its latest messages, oldest first, its inbox and its pending permissions
    /// and forms, or `None` if they could not be read. What they say is the worker's state, and the events held since
    /// its activity became unknown, which are newer, are applied over it again; if too many were held, what those
    /// events said is kept, with the records' progress if later. A worker whose records could not be read keeps what
    /// its events said, or starts again from now.
    fn sync(&mut self, records: Option<Records<'_>>, at: u64) {
        let held = self.held.take();
        match (records, held) {
            (Some(records), Some(held)) => {
                *self = Self::from_records(records);
                for (kind, data, when) in &held {
                    self.apply(kind, data, *when);
                }
            }
            (Some(records), None) => {
                let read = Self::from_records(records);
                self.current.progressed = self.current.progressed.max(read.current.progressed);
            }
            (None, _) => {}
        }
        if self.current.progressed == 0 {
            self.current.progressed = at;
        }
        self.held = Some(Vec::new());
        self.synced = true;
        self.reported = None;
    }

    /// What a worker was doing, as its records say: its latest messages, oldest first, its inbox and its pending
    /// requests.
    fn from_records((messages, inbox, requests): Records<'_>) -> Self {
        let mut tracker = Self::default();
        for request in requests {
            if let Some(id) = request.get("id").and_then(Value::as_str)
                && tracker.requests.len() < MOST
            {
                tracker.requests.push(id.to_owned());
            }
        }
        let mut ended = 0;
        for message in messages {
            let created = millis(message, "/time/created").unwrap_or(0);
            match message.get("type").and_then(Value::as_str) {
                Some("user") => {
                    if tracker.delivered.is_none() {
                        tracker.delivered = Some(Delivered {
                            sent: created,
                            at: created,
                            behind_step: false,
                        });
                    }
                }
                Some("assistant") => {
                    let completed = millis(message, "/time/completed");
                    let latest = [millis(message, "/time/streamed"), completed]
                        .into_iter()
                        .flatten()
                        .fold(created, u64::max);
                    tracker.current.progressed = tracker.current.progressed.max(latest);
                    if tracker
                        .delivered
                        .as_ref()
                        .is_some_and(|delivered| created >= delivered.at)
                    {
                        tracker.delivered = None;
                    }
                    tracker.current.finish = message
                        .get("finish")
                        .and_then(Value::as_str)
                        .map(|finish| bounded(finish, NAME_CHARS));
                    if let Some(error) = describe_error(message.get("error")) {
                        tracker.current.error = Some(error);
                    }
                    tracker.running = Some(completed.is_none());
                    ended = ended.max(completed.unwrap_or(0));
                    tracker.calls.clear();
                    let tools = message
                        .get("content")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter(|part| {
                            part.get("type").and_then(Value::as_str) == Some("tool")
                                && part.pointer("/state/status").and_then(Value::as_str)
                                    == Some("running")
                        });
                    for (index, tool) in tools.enumerate() {
                        let name = tool
                            .get("name")
                            .and_then(Value::as_str)
                            .map_or_else(|| "a tool".to_owned(), |name| bounded(name, NAME_CHARS));
                        let since = millis(tool, "/time/ran").unwrap_or(created);
                        tracker.call(&format!("record-{index}"), name, Some(since));
                    }
                }
                Some("idle") => {
                    tracker.running = Some(false);
                    ended = ended.max(created);
                }
                _ => {}
            }
        }
        for (index, item) in inbox
            .iter()
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("user"))
            .enumerate()
        {
            let sent = millis(item, "/time/created").unwrap_or(0);
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .map_or_else(|| format!("record-{index}"), str::to_owned);
            let since = (tracker.running != Some(true)).then(|| sent.max(ended));
            tracker.queue(&id, sent, since);
        }
        tracker.derive(true);
        tracker
    }

    /// `activity` was reported.
    fn reported(&mut self) {
        self.reported = Some(self.current.clone());
    }
}

/// The trackers of a session's workers, and those whose activity is still to be reported.
#[derive(Debug, Default)]
pub(crate) struct Workers {
    trackers: HashMap<String, Tracker>,
    dirty: BTreeSet<String>,
}

impl Workers {
    /// Apply an event of `kind` about the worker `id`; it is reported next if what is reported of it changed. Returns
    /// whether the worker was first seen now other than by its creation, as after a restart, when what it was doing is
    /// unknown until its records are read.
    pub(crate) fn apply(&mut self, id: &str, kind: &str, data: &Value, at: u64) -> bool {
        let (tracker, first) = match self.trackers.get_mut(id) {
            Some(tracker) => (tracker, false),
            None => {
                let tracker = self.trackers.entry(id.to_owned()).or_default();
                let unknown = kind != "session.created";
                tracker.synced = !unknown;
                (tracker, unknown)
            }
        };
        tracker.apply(kind, data, at);
        if tracker.due() && !self.dirty.contains(id) {
            self.dirty.insert(id.to_owned());
        }
        first
    }

    /// Forget the worker `id`, as when its session was deleted.
    pub(crate) fn forget(&mut self, id: &str) {
        self.trackers.remove(id);
        self.dirty.remove(id);
    }

    pub(crate) fn is_dirty(&self) -> bool {
        !self.dirty.is_empty()
    }

    /// Every worker's activity became unknown, as when the stream was lost. Returns the workers whose records are to be
    /// read again, at most [`MOST_SYNCED`], those last active first; the rest are forgotten.
    pub(crate) fn unsync(&mut self) -> Vec<String> {
        let mut known: Vec<(u64, String)> = self
            .trackers
            .iter()
            .map(|(id, tracker)| (tracker.current.progressed, id.clone()))
            .collect();
        known.sort_unstable_by(|a, b| b.cmp(a));
        for (_, id) in known.iter().skip(MOST_SYNCED) {
            self.trackers.remove(id);
        }
        self.dirty.clear();
        for tracker in self.trackers.values_mut() {
            tracker.unsync();
        }
        known
            .into_iter()
            .take(MOST_SYNCED)
            .map(|(_, id)| id)
            .collect()
    }

    /// The records of the worker `id` were read again at `at`, as [`Tracker::sync`] takes them.
    pub(crate) fn sync(&mut self, id: &str, records: Option<Records<'_>>, at: u64) {
        if let Some(tracker) = self.trackers.get_mut(id) {
            tracker.sync(records, at);
            self.dirty.insert(id.to_owned());
        }
    }

    /// Report each worker due, while `deliver` takes them, keeping those it refuses for later, and forgetting those
    /// `known` no longer knows.
    pub(crate) fn report(
        &mut self,
        known: impl Fn(&str) -> bool,
        mut deliver: impl FnMut(&str, WorkerActivity) -> bool,
    ) {
        let dirty = std::mem::take(&mut self.dirty);
        let mut refused = false;
        for id in dirty {
            if !known(&id) {
                self.trackers.remove(&id);
                continue;
            }
            let Some(tracker) = self.trackers.get_mut(&id) else {
                continue;
            };
            if !tracker.due() {
                continue;
            }
            if !refused && deliver(&id, tracker.current.clone()) {
                tracker.reported();
            } else {
                refused = true;
                self.dirty.insert(id);
            }
        }
    }
}

#[cfg(test)]
mod tests;
