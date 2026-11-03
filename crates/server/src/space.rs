// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::collections::{HashSet, VecDeque};

use backend::{AnswerValue, Event, Modes, Part, Permission, Prompt, Snapshot};
use protocol::{Activity, Participant, QueueState, SpaceState, SpaceSummary};

use crate::limits;
use crate::projection;
use crate::queue::InputQueue;

/// Messages kept in the shared conversation.
const MESSAGE_LIMIT: usize = 50;

/// Longest message text sent to clients.
const MESSAGE_TEXT_LIMIT: usize = 20_000;

/// The shared state of one space, driven by backend events and participant actions. Pure: no I/O.
#[derive(Debug)]
pub struct SpaceModel {
    pub id: String,
    pub title: String,
    pub backend: String,
    pub directory: String,
    /// The environment the space is bound to for its lifetime, the same for every participant.
    pub environment: String,
    /// The bound session's identifier, namespaced by backend.
    pub session: String,
    /// Participant identifiers, in join order.
    pub participants: Vec<String>,
    busy: Option<bool>,
    closed: bool,
    messages: Vec<MessageModel>,
    /// Backend user messages, whose text is already shown from the delivered prompt.
    user_messages: HashSet<String>,
    queue: InputQueue,
    in_flight: Option<Prompt>,
    /// Pending permissions, oldest first. The first is presented; the rest wait their turn.
    approvals: VecDeque<Permission>,
    /// Permissions whose decision has been sent to the backend but not yet confirmed.
    deciding: Vec<Permission>,
    /// Pending questions, oldest first. The first is presented and collects answers.
    questions: VecDeque<PendingQuestion>,
    /// Questions whose answers have been sent to the backend but not yet confirmed.
    submitting: Vec<PendingQuestion>,
    workers: Vec<protocol::Worker>,
    workers_authoritative: bool,
    attribution: bool,
    /// The modes the session offers and its current one, as the agent last reported them.
    modes: Modes,
    error: Option<String>,
    /// Worker observation failures are separate from unrelated space errors.
    worker_error: Option<String>,
    /// The backend reported a lost connection and has not yet read the session again, with the reason it gave. The
    /// space's state may be stale meanwhile, so no input is released.
    reconnecting: Option<String>,
}

#[derive(Debug)]
struct MessageModel {
    id: String,
    role: protocol::Role,
    text: String,
    tools: Vec<ToolModel>,
    completed: bool,
}

#[derive(Debug)]
struct ToolModel {
    id: String,
    name: String,
    state: &'static str,
}

#[derive(Clone, Debug)]
struct PendingQuestion {
    question: backend::Question,
    answers: Vec<(String, AnswerValue)>,
}

/// The outcome of a participant's answer to the current question field.
#[derive(Debug, PartialEq)]
pub enum AnswerOutcome {
    /// The answer was recorded; the question has more fields.
    Recorded,
    /// The question is complete and these answers must be sent to the backend.
    Complete(Vec<(String, AnswerValue)>),
}

impl MessageModel {
    /// Displayable text: the message text, or its tool activity when it has no text yet.
    fn display(&self) -> String {
        let text = if self.text.is_empty() {
            self.tools
                .iter()
                .map(|tool| format!("[{}: {}]", tool.name, tool.state))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            self.text.clone()
        };
        match text.char_indices().nth(MESSAGE_TEXT_LIMIT) {
            Some((index, _)) => text[..index].to_owned(),
            None => text,
        }
    }
}

fn role(role: backend::Role) -> protocol::Role {
    match role {
        backend::Role::User => protocol::Role::User,
        backend::Role::Assistant => protocol::Role::Assistant,
    }
}

/// Project a backend worker. A worker whose parent is the space's own session is reported without a parent, so clients need not know the backend's session identifier.
fn worker(worker: backend::Worker, session: &str) -> protocol::Worker {
    use backend::WorkerState as From;
    use protocol::WorkerState as To;
    protocol::Worker {
        id: worker.id,
        parent: worker.parent.filter(|parent| parent != session),
        title: worker.title,
        state: match worker.state {
            From::Running => To::Running,
            From::Waiting => To::Waiting,
            From::Idle => To::Idle,
            From::Completed => To::Completed,
            From::Error => To::Error,
            From::Stopped => To::Stopped,
            From::Unknown => To::Unknown,
        },
    }
}

/// The text delivered to the agent for a queue entry. With attribution, the speaker's display name prefixes the text as `Name: text`.
pub fn attributed(speaker: &str, text: &str, attribution: bool) -> String {
    if attribution && !speaker.is_empty() {
        format!("{speaker}: {text}")
    } else {
        text.to_owned()
    }
}

impl SpaceModel {
    pub fn new(
        id: String,
        title: String,
        backend: String,
        directory: String,
        environment: String,
        session: String,
        snapshot: Snapshot,
    ) -> Self {
        let workers = snapshot
            .workers
            .into_iter()
            .map(|entry| worker(entry, &session))
            .collect();
        let mut model = Self {
            id,
            title,
            backend,
            directory,
            environment,
            session,
            participants: Vec::new(),
            busy: snapshot.busy,
            closed: false,
            messages: Vec::new(),
            user_messages: HashSet::new(),
            queue: InputQueue::default(),
            in_flight: None,
            approvals: snapshot.permissions.into_iter().collect(),
            deciding: Vec::new(),
            questions: snapshot
                .questions
                .into_iter()
                .map(|question| PendingQuestion {
                    question,
                    answers: Vec::new(),
                })
                .collect(),
            submitting: Vec::new(),
            workers,
            workers_authoritative: true,
            attribution: true,
            modes: snapshot.modes,
            error: None,
            worker_error: None,
            reconnecting: None,
        };
        model.load_messages(snapshot.messages);
        model
    }

    /// Replace the conversation with a snapshot's messages.
    fn load_messages(&mut self, messages: Vec<backend::SnapshotMessage>) {
        self.messages.clear();
        self.user_messages.clear();
        for message in messages {
            let mut text = Vec::new();
            let mut tools = Vec::new();
            for part in message.parts {
                match part {
                    Part::Text { text: part } => text.push(part),
                    Part::Reasoning { .. } => {}
                    Part::Tool {
                        id, tool, finished, ..
                    } => tools.push(ToolModel {
                        id,
                        name: tool,
                        state: if finished { "done" } else { "working" },
                    }),
                }
            }
            self.push_message(MessageModel {
                id: message.id,
                role: role(message.role),
                text: text.join("\n\n"),
                tools,
                completed: message.completed,
            });
        }
    }

    /// Take the session as the backend read it again after losing events. The conversation, running state and modes
    /// are replaced. Pending requests the snapshot no longer lists are withdrawn, including any whose decision or
    /// answers are being delivered, since the agent no longer waits for them; those it lists and the space already
    /// holds keep their place and any partial answers, and new ones are queued after them. Workers are left to their
    /// own events. The error the backend reported for the gap no longer applies, so it is cleared, and input is released
    /// again.
    fn resynchronize(&mut self, snapshot: Snapshot) {
        let Snapshot {
            busy,
            messages,
            permissions,
            questions,
            workers: _,
            modes,
        } = snapshot;
        self.load_messages(messages);
        if busy.is_some() {
            self.busy = busy;
        }
        self.modes = modes;
        let listed: HashSet<&str> = permissions
            .iter()
            .map(|permission| permission.id.as_str())
            .collect();
        self.approvals
            .retain(|pending| listed.contains(pending.id.as_str()));
        self.deciding
            .retain(|pending| listed.contains(pending.id.as_str()));
        for permission in permissions {
            self.apply(Event::PermissionRequested(permission));
        }
        let listed: HashSet<&str> = questions
            .iter()
            .map(|question| question.id.as_str())
            .collect();
        self.questions
            .retain(|pending| listed.contains(pending.question.id.as_str()));
        self.submitting
            .retain(|pending| listed.contains(pending.question.id.as_str()));
        for question in questions {
            self.apply(Event::QuestionAsked(question));
        }
        if let Some(reason) = self.reconnecting.take()
            && self.error.as_deref() == Some(reason.as_str())
        {
            self.error = None;
        }
    }

    pub fn activity(&self) -> Activity {
        if self.closed {
            return Activity::Closed;
        }
        if self.is_waiting() {
            return Activity::Waiting;
        }
        match self.busy {
            Some(true) => Activity::Running,
            Some(false) => Activity::Idle,
            None => Activity::Unknown,
        }
    }

    /// Whether a permission or question is pending, including one whose decision or answers are still being delivered.
    fn is_waiting(&self) -> bool {
        !self.approvals.is_empty()
            || !self.deciding.is_empty()
            || !self.questions.is_empty()
            || !self.submitting.is_empty()
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn set_attribution(&mut self, enabled: bool) {
        self.attribution = enabled;
    }

    /// Whether `mode` is one the space shows as offered, so a participant may choose it.
    pub fn offers_mode(&self, mode: &str) -> bool {
        self.shown_modes().iter().any(|offered| offered.id == mode)
    }

    /// The offered modes as shared, bounded so that a misbehaving agent cannot fill the snapshot: a mode whose
    /// identifier no client could send back is left out, and names and descriptions are cut.
    fn shown_modes(&self) -> Vec<protocol::Mode> {
        self.modes
            .offered
            .iter()
            .filter(|mode| mode.id.len() <= limits::IDENTIFIER_BYTES)
            .take(limits::MODES)
            .map(|mode| protocol::Mode {
                id: mode.id.clone(),
                name: limits::truncate(&mode.name, limits::MODE_NAME_BYTES),
                description: mode.description.as_deref().map(|description| {
                    limits::truncate(description, limits::MODE_DESCRIPTION_BYTES)
                }),
            })
            .collect()
    }

    /// The name the space shows for `mode`, falling back to its identifier.
    fn mode_name(&self, mode: &str) -> String {
        let name = self
            .modes
            .offered
            .iter()
            .find(|offered| offered.id == mode)
            .map_or(mode, |offered| offered.name.as_str());
        limits::truncate(name, limits::MODE_NAME_BYTES)
    }

    fn push_message(&mut self, message: MessageModel) {
        self.messages.push(message);
        if self.messages.len() > MESSAGE_LIMIT {
            let excess = self.messages.len() - MESSAGE_LIMIT;
            self.messages.drain(..excess);
        }
    }

    /// The message with this identifier, created as an assistant message if it is new.
    fn message(&mut self, id: &str) -> &mut MessageModel {
        let index = match self.messages.iter().rposition(|message| message.id == id) {
            Some(index) => index,
            None => {
                self.push_message(MessageModel {
                    id: id.to_owned(),
                    role: protocol::Role::Assistant,
                    text: String::new(),
                    tools: Vec::new(),
                    completed: false,
                });
                self.messages.len() - 1
            }
        };
        &mut self.messages[index]
    }

    /// Apply a backend event to the shared state.
    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Busy => self.busy = Some(true),
            Event::Idle => self.busy = Some(false),
            Event::Delivered { key } => self.delivered(&key),
            Event::MessageStarted { message, role } => match role {
                backend::Role::User => {
                    self.user_messages.insert(message);
                }
                backend::Role::Assistant => {
                    self.message(&message);
                }
            },
            Event::TextDelta { message, text } => {
                if !self.user_messages.contains(&message) {
                    self.message(&message).text.push_str(&text);
                }
            }
            Event::MessageCompleted { message } => {
                if !self.user_messages.remove(&message) {
                    self.message(&message).completed = true;
                }
            }
            Event::ToolStarted {
                message, tool, id, ..
            } => {
                // A start the session was re-read with is not added again.
                let tools = &mut self.message(&message).tools;
                if !tools.iter().any(|known| known.id == id) {
                    tools.push(ToolModel {
                        id,
                        name: tool,
                        state: "working",
                    });
                }
            }
            Event::ToolFinished { id, error } => {
                if let Some(tool) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .flat_map(|message| message.tools.iter_mut())
                    .find(|tool| tool.id == id)
                {
                    tool.state = if error.is_some() { "failed" } else { "done" };
                }
            }
            Event::PermissionRequested(permission) => {
                // Parallel tool calls produce several requests; each waits its turn.
                let known = self
                    .approvals
                    .iter()
                    .chain(&self.deciding)
                    .any(|pending| pending.id == permission.id);
                if !known {
                    self.approvals.push_back(permission);
                }
            }
            Event::PermissionResolved { id } => {
                self.approvals.retain(|pending| pending.id != id);
                self.deciding.retain(|pending| pending.id != id);
            }
            Event::QuestionAsked(question) => {
                let known = self
                    .questions
                    .iter()
                    .chain(&self.submitting)
                    .any(|pending| pending.question.id == question.id);
                if !known {
                    self.questions.push_back(PendingQuestion {
                        question,
                        answers: Vec::new(),
                    });
                }
            }
            Event::QuestionResolved { id } => {
                self.questions.retain(|pending| pending.question.id != id);
                self.submitting.retain(|pending| pending.question.id != id);
            }
            Event::WorkersChanged { workers } => {
                self.workers = workers
                    .into_iter()
                    .map(|entry| worker(entry, &self.session))
                    .collect();
                self.workers_authoritative = true;
                self.worker_error = None;
            }
            Event::WorkersUnavailable { message } => {
                self.worker_error = Some(message);
                self.workers_authoritative = false;
            }
            Event::ModesChanged(modes) => self.modes = modes,
            Event::ModeRefused { mode, reason } => {
                let name = self.mode_name(&mode);
                self.error = Some(match reason {
                    Some(reason) => format!("The agent refused {name} mode: {reason}"),
                    None => format!("The agent refused {name} mode."),
                });
            }
            Event::Error { message } => {
                self.error = Some(message);
            }
            Event::Reconnecting { reason } => {
                if !self.closed {
                    self.error = Some(reason.clone());
                    self.reconnecting = Some(reason);
                }
            }
            Event::Resynchronized { snapshot } => {
                if !self.closed {
                    self.resynchronize(snapshot);
                }
            }
            Event::Closed { reason } => {
                self.closed = true;
                self.reconnecting = None;
                self.workers_authoritative = false;
                self.worker_error = None;
                self.busy = None;
                self.error = Some(reason.unwrap_or_else(|| "The agent session ended.".to_owned()));
                if let Some(prompt) = self.in_flight.take() {
                    self.queue.finish(&prompt.key, QueueState::Failed);
                }
                // Nothing waiting can be delivered any more, so say so rather than leave it queued.
                self.queue.fail_waiting();
                // A closed session can no longer receive decisions or answers.
                self.approvals.clear();
                self.deciding.clear();
                self.questions.clear();
                self.submitting.clear();
                // The agent process has ended, so nothing it was running is running any more, whatever the backend
                // last reported.
                for worker in &mut self.workers {
                    if matches!(
                        worker.state,
                        protocol::WorkerState::Running | protocol::WorkerState::Waiting
                    ) {
                        worker.state = protocol::WorkerState::Stopped;
                    }
                }
            }
        }
    }

    /// The text queued under `key`, if it was queued recently; see [`InputQueue::queued_text`].
    pub fn queued_text(&self, key: &str) -> Option<&str> {
        self.queue.queued_text(key)
    }

    /// Queue finalized input from `participant`, shown as spoken by `speaker`. Returns false if this key was already queued.
    pub fn enqueue(
        &mut self,
        key: String,
        participant: String,
        speaker: String,
        text: String,
    ) -> bool {
        self.queue.push(key, participant, speaker, text)
    }

    /// Entries waiting to be delivered.
    pub fn waiting(&self) -> usize {
        self.queue.waiting()
    }

    pub fn cancel(&mut self, key: &str) -> Result<(), &'static str> {
        self.queue.cancel(key)
    }

    /// The next prompt to deliver, if the agent is idle, nothing is pending and input is queued. Marks the entry as sending.
    pub fn next_prompt(&mut self) -> Option<Prompt> {
        if self.closed
            || self.reconnecting.is_some()
            || self.activity() != Activity::Idle
            || self.in_flight.is_some()
        {
            return None;
        }
        let attribution = self.attribution;
        let entry = self.queue.begin_next()?;
        let prompt = Prompt {
            key: entry.id.clone(),
            text: attributed(&entry.speaker, &entry.text, attribution),
        };
        self.in_flight = Some(prompt.clone());
        Some(prompt)
    }

    fn delivered(&mut self, key: &str) {
        if !self
            .in_flight
            .as_ref()
            .is_some_and(|prompt| prompt.key == key)
        {
            return;
        }
        let prompt = self.in_flight.take().expect("checked above");
        self.queue.finish(key, QueueState::Delivered);
        // The agent accepted the prompt, so its turn has started even if `Busy` has not arrived yet.
        self.busy = Some(true);
        self.push_message(MessageModel {
            id: prompt.key,
            role: protocol::Role::User,
            text: prompt.text,
            tools: Vec::new(),
            completed: true,
        });
    }

    /// The backend finished a `send` call.
    pub fn sent(&mut self, key: &str, result: Result<(), String>) {
        match result {
            Ok(()) => self.delivered(key),
            Err(error) => {
                if self
                    .in_flight
                    .as_ref()
                    .is_some_and(|prompt| prompt.key == key)
                {
                    self.in_flight = None;
                    self.queue.finish(key, QueueState::Failed);
                    self.error = Some(format!("Could not deliver input: {error}"));
                }
            }
        }
    }

    /// Claim the presented approval for a decision, which grants `option` if given. Only the first claim succeeds; the next pending approval is presented at once. A claim naming an option the approval does not offer fails and leaves it presented.
    pub fn claim_approval(
        &mut self,
        request: &str,
        option: Option<&str>,
    ) -> Result<(), &'static str> {
        match self.approvals.front() {
            Some(pending)
                if pending.id == request
                    && option.is_some_and(|option| {
                        !projection::options(pending)
                            .iter()
                            .any(|offered| offered.id == option)
                    }) =>
            {
                Err("That option is not offered for this approval.")
            }
            Some(pending) if pending.id == request => {
                let claimed = self.approvals.pop_front().expect("checked above");
                self.deciding.push(claimed);
                Ok(())
            }
            _ => Err("That approval is no longer pending."),
        }
    }

    /// The backend accepted a decision.
    pub fn decided(&mut self, request: &str) {
        self.deciding.retain(|pending| pending.id != request);
    }

    /// The backend rejected a decision for a reason other than the request being gone: present the approval again, ahead of later ones.
    pub fn restore_approval(&mut self, request: &str) {
        if let Some(index) = self
            .deciding
            .iter()
            .position(|pending| pending.id == request)
        {
            let pending = self.deciding.remove(index);
            self.approvals.push_front(pending);
        }
    }

    /// Record an answer to the current field. The first answer to a field wins.
    pub fn answer(
        &mut self,
        question: &str,
        field: &str,
        text: &str,
    ) -> Result<AnswerOutcome, String> {
        let pending = self
            .questions
            .front_mut()
            .filter(|pending| pending.question.id == question)
            .ok_or("That question is no longer pending.")?;
        if !projection::supported(&pending.question) {
            return Err("Complete this question outside Tomori.".to_owned());
        }
        let current = projection::current_field(&pending.question, &pending.answers)
            .filter(|current| projection::field_name(&pending.question, current) == field)
            .ok_or("The question changed; read the current field and answer again.")?;
        let value = projection::answer_value(current, text)?;
        let key = current.key.clone();
        pending.answers.push((key, value));
        if projection::current_field(&pending.question, &pending.answers).is_some() {
            return Ok(AnswerOutcome::Recorded);
        }
        let complete = self.questions.pop_front().expect("checked above");
        let answers = complete.answers.clone();
        self.submitting.push(complete);
        Ok(AnswerOutcome::Complete(answers))
    }

    /// The backend accepted the answers to a question.
    pub fn answered(&mut self, question: &str) {
        self.submitting
            .retain(|pending| pending.question.id != question);
    }

    /// The backend rejected the completed answers for a reason other than the question being gone: present it again from the start, ahead of later questions.
    pub fn restore_question(&mut self, question: &str) {
        if let Some(index) = self
            .submitting
            .iter()
            .position(|pending| pending.question.id == question)
        {
            let mut pending = self.submitting.remove(index);
            pending.answers.clear();
            self.questions.push_front(pending);
        }
    }

    pub fn state(&self, participants: Vec<Participant>) -> SpaceState {
        SpaceState {
            id: self.id.clone(),
            title: self.title.clone(),
            backend: self.backend.clone(),
            directory: self.directory.clone(),
            environment: self.environment.clone(),
            activity: self.activity(),
            messages: self
                .messages
                .iter()
                .filter_map(|message| {
                    let text = message.display();
                    (!text.is_empty()).then(|| protocol::Message {
                        id: message.id.clone(),
                        role: message.role,
                        text,
                        completed: message.completed,
                    })
                })
                .collect(),
            queue: self.queue.entries().to_vec(),
            approval: self.approvals.front().map(projection::approval),
            question: self
                .questions
                .front()
                .map(|pending| projection::question(&pending.question, &pending.answers)),
            workers: self.workers.clone(),
            workers_authoritative: Some(self.workers_authoritative),
            presentations: Vec::new(),
            participants,
            attribution: self.attribution,
            error: match (&self.error, &self.worker_error) {
                (Some(error), Some(worker_error)) => Some(format!("{error} {worker_error}")),
                (Some(error), None) => Some(error.clone()),
                (None, Some(worker_error)) => Some(worker_error.clone()),
                (None, None) => None,
            },
            mode: self
                .modes
                .current
                .as_ref()
                .filter(|mode| mode.len() <= limits::IDENTIFIER_BYTES)
                .cloned(),
            modes: self.shown_modes(),
        }
    }

    pub fn summary(&self) -> SpaceSummary {
        SpaceSummary {
            id: self.id.clone(),
            title: self.title.clone(),
            backend: self.backend.clone(),
            directory: self.directory.clone(),
            environment: self.environment.clone(),
            activity: self.activity(),
            participants: self.participants.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> SpaceModel {
        SpaceModel::new(
            "s1".into(),
            "Title".into(),
            "fake".into(),
            "/tmp".into(),
            "sky".into(),
            "fake-1".into(),
            Snapshot {
                busy: Some(false),
                ..Snapshot::default()
            },
        )
    }

    #[test]
    fn modes_follow_the_agent_and_stay_within_limits() {
        let mut space = model();
        assert_eq!(space.state(Vec::new()).mode, None);
        assert!(!space.offers_mode("plan"));
        let mode = |id: String, name: String| backend::Mode {
            id,
            name,
            description: Some("d".repeat(1000)),
        };
        let mut offered = vec![
            mode("plan".into(), "Plan".into()),
            mode("x".repeat(limits::IDENTIFIER_BYTES + 1), "Long".into()),
            mode("named".into(), "n".repeat(1000)),
        ];
        offered.extend((0..limits::MODES).map(|index| mode(format!("m{index}"), "M".into())));
        space.apply(Event::ModesChanged(Modes {
            offered,
            current: Some("plan".into()),
        }));
        let state = space.state(Vec::new());
        assert_eq!(state.mode.as_deref(), Some("plan"));
        assert_eq!(state.modes.len(), limits::MODES);
        assert_eq!(state.modes[1].id, "named");
        assert!(state.modes[1].name.len() <= limits::MODE_NAME_BYTES);
        assert!(
            state.modes[0].description.as_ref().unwrap().len() <= limits::MODE_DESCRIPTION_BYTES
        );
        assert!(space.offers_mode("plan"));
        assert!(!space.offers_mode(&"x".repeat(limits::IDENTIFIER_BYTES + 1)));

        // A starting mode the agent refused is named in the space error.
        space.apply(Event::ModeRefused {
            mode: "plan".into(),
            reason: None,
        });
        assert_eq!(
            space.state(Vec::new()).error.as_deref(),
            Some("The agent refused Plan mode.")
        );
    }

    #[test]
    fn input_is_delivered_only_while_idle() {
        let mut space = model();
        space.enqueue("a".into(), "ada".into(), "Ada".into(), "hello".into());
        space.enqueue("b".into(), "bob".into(), "Bob".into(), "there".into());
        let prompt = space.next_prompt().unwrap();
        assert_eq!(prompt.text, "Ada: hello");
        assert!(space.next_prompt().is_none());
        space.apply(Event::Delivered { key: "a".into() });
        assert_eq!(space.activity(), Activity::Running);
        assert!(space.next_prompt().is_none());
        space.apply(Event::Idle);
        space.set_attribution(false);
        assert_eq!(space.next_prompt().unwrap().text, "there");
    }

    #[test]
    fn pending_requests_block_delivery_and_resolve_once() {
        let mut space = model();
        space.apply(Event::PermissionRequested(Permission {
            id: "p".into(),
            tool: "Bash".into(),
            input: serde_json::Value::Null,
            description: None,
            options: Vec::new(),
        }));
        space.enqueue("a".into(), "ada".into(), "Ada".into(), "hello".into());
        assert_eq!(space.activity(), Activity::Waiting);
        assert!(space.next_prompt().is_none());
        assert!(space.claim_approval("p", None).is_ok());
        assert!(space.claim_approval("p", None).is_err());
        space.restore_approval("p");
        assert!(space.claim_approval("p", None).is_ok());
        space.apply(Event::PermissionResolved { id: "p".into() });
        assert!(space.next_prompt().is_some());
    }

    fn permission(id: &str) -> Permission {
        Permission {
            id: id.into(),
            tool: "Bash".into(),
            input: serde_json::Value::Null,
            description: None,
            options: Vec::new(),
        }
    }

    #[test]
    fn initial_snapshot_keeps_all_pending_permissions_and_questions() {
        let question = |id: &str| backend::Question {
            id: id.into(),
            title: id.into(),
            fields: Vec::new(),
        };
        let mut space = SpaceModel::new(
            "s1".into(),
            "Title".into(),
            "fake".into(),
            "/tmp".into(),
            "sky".into(),
            "fake-1".into(),
            Snapshot {
                busy: Some(false),
                permissions: vec![permission("p1"), permission("p2")],
                questions: vec![question("q1"), question("q2")],
                ..Snapshot::default()
            },
        );
        space.apply(Event::PermissionRequested(permission("p1")));
        space.apply(Event::QuestionAsked(question("q1")));
        assert_eq!(space.activity(), Activity::Waiting);
        assert_eq!(space.state(Vec::new()).approval.unwrap().id, "p1");
        assert_eq!(space.state(Vec::new()).question.unwrap().id, "q1");
        space.apply(Event::PermissionResolved { id: "p1".into() });
        space.apply(Event::QuestionResolved { id: "q1".into() });
        assert_eq!(space.state(Vec::new()).approval.unwrap().id, "p2");
        assert_eq!(space.state(Vec::new()).question.unwrap().id, "q2");
        space.apply(Event::PermissionResolved { id: "p2".into() });
        space.apply(Event::QuestionResolved { id: "q2".into() });
        assert_eq!(space.activity(), Activity::Idle);
    }

    #[test]
    fn a_resynchronization_replaces_the_conversation_and_reconciles_requests() {
        let question = |id: &str| backend::Question {
            id: id.into(),
            title: id.into(),
            fields: Vec::new(),
        };
        let mut space = model();
        space.apply(Event::MessageStarted {
            message: "partial".into(),
            role: backend::Role::Assistant,
        });
        space.apply(Event::TextDelta {
            message: "partial".into(),
            text: "Half a".into(),
        });
        space.apply(Event::WorkersChanged {
            workers: vec![backend::Worker {
                id: "worker".into(),
                parent: None,
                title: "Worker".into(),
                state: backend::WorkerState::Running,
            }],
        });
        // "kept" waits with a partial answer; "resolved" was answered during
        // the gap; "deciding" was claimed and its decision is being delivered.
        space.apply(Event::PermissionRequested(permission("deciding")));
        space.apply(Event::PermissionRequested(permission("resolved")));
        space.apply(Event::PermissionRequested(permission("kept")));
        assert!(space.claim_approval("deciding", None).is_ok());
        space.apply(Event::QuestionAsked(question("kept")));
        space.apply(Event::QuestionAsked(question("gone")));
        space.questions[0]
            .answers
            .push(("field".into(), AnswerValue::Text("partly".into())));
        space.apply(Event::Reconnecting {
            reason: "Lost the connection to OpenCode; reconnecting.".into(),
        });
        assert_eq!(
            space.state(Vec::new()).error.as_deref(),
            Some("Lost the connection to OpenCode; reconnecting.")
        );

        space.apply(Event::Resynchronized {
            snapshot: Snapshot {
                busy: Some(true),
                messages: vec![backend::SnapshotMessage {
                    id: "partial".into(),
                    role: backend::Role::Assistant,
                    parts: vec![Part::Text {
                        text: "Half a reply, finished during the gap.".into(),
                    }],
                    completed: true,
                }],
                permissions: vec![
                    permission("kept"),
                    permission("raised"),
                    permission("deciding"),
                ],
                questions: vec![question("kept"), question("asked")],
                // Workers are left to their own events.
                workers: Vec::new(),
                modes: Modes::default(),
            },
        });
        let state = space.state(Vec::new());
        assert_eq!(state.messages.len(), 1);
        assert_eq!(
            state.messages[0].text,
            "Half a reply, finished during the gap."
        );
        assert!(state.messages[0].completed);
        assert_eq!(state.error, None);
        assert_eq!(state.workers.len(), 1);
        assert_eq!(state.workers_authoritative, Some(true));
        assert_eq!(space.activity(), Activity::Waiting);
        let approvals: Vec<_> = space.approvals.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(approvals, ["kept", "raised"]);
        let deciding: Vec<_> = space.deciding.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(deciding, ["deciding"]);
        let questions: Vec<_> = space
            .questions
            .iter()
            .map(|pending| (pending.question.id.as_str(), pending.answers.len()))
            .collect();
        assert_eq!(questions, [("kept", 1), ("asked", 0)]);

        // A request resolved during the gap is withdrawn even while its
        // decision is being delivered.
        space.apply(Event::Resynchronized {
            snapshot: Snapshot {
                busy: Some(false),
                ..Snapshot::default()
            },
        });
        assert!(space.approvals.is_empty() && space.deciding.is_empty());
        assert!(space.questions.is_empty());
        assert_eq!(space.activity(), Activity::Idle);
        assert!(space.state(Vec::new()).messages.is_empty());

        // While reconnecting, the state may be stale, so queued input waits
        // for the session to be read again. An error that is not the gap's
        // stays visible.
        assert!(space.enqueue("u1".into(), "ada".into(), "Ada".into(), "hello".into()));
        space.apply(Event::Reconnecting {
            reason: "Lost the connection.".into(),
        });
        space.apply(Event::Error {
            message: "The agent execution failed.".into(),
        });
        assert!(space.next_prompt().is_none());
        space.apply(Event::Resynchronized {
            snapshot: Snapshot {
                busy: Some(false),
                ..Snapshot::default()
            },
        });
        assert_eq!(
            space.state(Vec::new()).error.as_deref(),
            Some("The agent execution failed.")
        );
        assert_eq!(space.next_prompt().unwrap().key, "u1");
        space.sent("u1", Ok(()));

        // A tool start repeated after the state it was read with is not added twice.
        space.apply(Event::Resynchronized {
            snapshot: Snapshot {
                messages: vec![backend::SnapshotMessage {
                    id: "m".into(),
                    role: backend::Role::Assistant,
                    parts: vec![Part::Tool {
                        id: "t".into(),
                        tool: "bash".into(),
                        input: serde_json::Value::Null,
                        finished: false,
                    }],
                    completed: false,
                }],
                ..Snapshot::default()
            },
        });
        space.apply(Event::ToolStarted {
            message: "m".into(),
            tool: "bash".into(),
            id: "t".into(),
            input: serde_json::Value::Null,
        });
        assert_eq!(space.messages[0].tools.len(), 1);
        // A space already closed keeps its closing state.
        space.apply(Event::Closed { reason: None });
        space.apply(Event::Resynchronized {
            snapshot: Snapshot::default(),
        });
        assert_eq!(space.activity(), Activity::Closed);
    }

    #[test]
    fn only_offered_options_can_be_granted() {
        let mut space = model();
        let mut offering = permission("p1");
        offering.options.push(backend::PermissionOption {
            id: "suggestions".into(),
            grants: vec![backend::Grant::Mode {
                mode: "acceptEdits".into(),
                scope: backend::GrantScope::Session,
            }],
        });
        space.apply(Event::PermissionRequested(offering));
        space.apply(Event::PermissionRequested(permission("p2")));

        let approval = space.state(Vec::new()).approval.unwrap();
        assert_eq!(
            approval.options,
            vec![protocol::ApprovalOption {
                id: "suggestions".into(),
                label: "Yes, allow all edits during this session".into(),
            }]
        );
        assert!(space.claim_approval("p1", Some("always")).is_err());
        assert_eq!(space.state(Vec::new()).approval.unwrap().id, "p1");
        assert!(space.claim_approval("p1", Some("suggestions")).is_ok());

        // An approval without options offers nothing to grant.
        assert!(space.state(Vec::new()).approval.unwrap().options.is_empty());
        assert!(space.claim_approval("p2", Some("suggestions")).is_err());
        assert!(space.claim_approval("p2", None).is_ok());
    }

    #[test]
    fn parallel_permissions_are_presented_in_order() {
        let mut space = model();
        space.apply(Event::PermissionRequested(permission("p1")));
        space.apply(Event::PermissionRequested(permission("p2")));
        space.apply(Event::PermissionRequested(permission("p1")));
        space.enqueue("a".into(), "ada".into(), "Ada".into(), "hello".into());
        assert_eq!(space.state(Vec::new()).approval.unwrap().id, "p1");
        assert!(space.claim_approval("p2", None).is_err());
        assert!(space.claim_approval("p1", None).is_ok());
        assert_eq!(space.state(Vec::new()).approval.unwrap().id, "p2");

        // A recoverable failure presents the first request again, ahead of the second.
        space.restore_approval("p1");
        assert_eq!(space.state(Vec::new()).approval.unwrap().id, "p1");
        assert!(space.claim_approval("p1", None).is_ok());
        space.decided("p1");
        assert!(space.claim_approval("p2", None).is_ok());

        // Nothing is delivered until every request is resolved.
        assert_eq!(space.activity(), Activity::Waiting);
        assert!(space.next_prompt().is_none());
        space.apply(Event::PermissionResolved { id: "p2".into() });
        assert_eq!(space.activity(), Activity::Idle);
        assert!(space.next_prompt().is_some());
    }

    #[test]
    fn closing_fails_waiting_input() {
        let mut space = model();
        space.enqueue("a".into(), "ada".into(), "Ada".into(), "first".into());
        space.next_prompt().unwrap();
        space.enqueue("b".into(), "bob".into(), "Bob".into(), "second".into());
        space.apply(Event::PermissionRequested(permission("p")));
        space.apply(Event::Closed { reason: None });
        assert_eq!(space.activity(), Activity::Closed);
        assert_eq!(space.summary().activity, Activity::Closed);
        let state = space.state(Vec::new());
        assert!(
            state
                .queue
                .iter()
                .all(|entry| entry.state == QueueState::Failed)
        );
        assert!(state.approval.is_none());
        assert!(state.error.is_some());
        assert!(space.next_prompt().is_none());
    }

    fn backend_worker(id: &str, state: backend::WorkerState) -> backend::Worker {
        backend::Worker {
            id: id.into(),
            parent: None,
            title: id.into(),
            state,
        }
    }

    #[test]
    fn workers_stop_running_when_the_agent_process_exits() {
        let mut space = model();
        space.apply(Event::WorkersChanged {
            workers: vec![
                backend_worker("running", backend::WorkerState::Running),
                backend_worker("waiting", backend::WorkerState::Waiting),
                backend_worker("completed", backend::WorkerState::Completed),
                backend_worker("failed", backend::WorkerState::Error),
            ],
        });
        space.apply(Event::Closed {
            reason: Some("claude exited with 1".into()),
        });
        let states: Vec<_> = space
            .state(Vec::new())
            .workers
            .iter()
            .map(|worker| worker.state)
            .collect();
        assert_eq!(
            states,
            vec![
                protocol::WorkerState::Stopped,
                protocol::WorkerState::Stopped,
                protocol::WorkerState::Completed,
                protocol::WorkerState::Error,
            ]
        );
    }

    #[test]
    fn a_fresh_worker_update_does_not_clear_an_unrelated_space_error() {
        let mut space = model();
        space.apply(Event::Error {
            message: "unrelated error".into(),
        });
        let current = space.state(Vec::new());
        assert_eq!(current.workers_authoritative, Some(true));
        space.apply(Event::WorkersChanged { workers: vec![] });
        let current = space.state(Vec::new());
        assert_eq!(current.workers_authoritative, Some(true));
        assert_eq!(current.error.as_deref(), Some("unrelated error"));
        space.apply(Event::WorkersUnavailable {
            message: "worker observation interrupted".into(),
        });
        let uncertain = space.state(Vec::new());
        assert_eq!(uncertain.workers_authoritative, Some(false));
        assert_eq!(
            uncertain.error.as_deref(),
            Some("unrelated error worker observation interrupted")
        );
        space.apply(Event::WorkersChanged { workers: vec![] });
        let recovered = space.state(Vec::new());
        assert_eq!(recovered.workers_authoritative, Some(true));
        assert_eq!(recovered.error.as_deref(), Some("unrelated error"));
        space.apply(Event::Closed { reason: None });
        assert_eq!(space.state(Vec::new()).workers_authoritative, Some(false));
    }

    #[test]
    fn text_deltas_stream_into_messages() {
        let mut space = model();
        space.apply(Event::MessageStarted {
            message: "m".into(),
            role: backend::Role::Assistant,
        });
        space.apply(Event::TextDelta {
            message: "m".into(),
            text: "Hel".into(),
        });
        let state = space.state(Vec::new());
        assert_eq!(state.messages[0].text, "Hel");
        assert!(!state.messages[0].completed);
        space.apply(Event::TextDelta {
            message: "m".into(),
            text: "lo".into(),
        });
        space.apply(Event::MessageCompleted {
            message: "m".into(),
        });
        let state = space.state(Vec::new());
        assert_eq!(state.messages[0].text, "Hello");
        assert!(state.messages[0].completed);
    }

    #[test]
    fn user_message_events_do_not_duplicate_delivered_input() {
        let mut space = model();
        space.enqueue("a".into(), "ada".into(), "Ada".into(), "hello".into());
        space.next_prompt().unwrap();
        space.sent("a", Ok(()));
        space.apply(Event::MessageStarted {
            message: "u".into(),
            role: backend::Role::User,
        });
        space.apply(Event::TextDelta {
            message: "u".into(),
            text: "Ada: hello".into(),
        });
        space.apply(Event::MessageCompleted {
            message: "u".into(),
        });
        let state = space.state(Vec::new());
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].role, protocol::Role::User);
        assert_eq!(state.queue[0].state, QueueState::Delivered);
    }

    #[test]
    fn tool_activity_is_shown_until_text_arrives() {
        let mut space = model();
        space.apply(Event::ToolStarted {
            message: "m".into(),
            tool: "Bash".into(),
            id: "t".into(),
            input: serde_json::Value::Null,
        });
        assert_eq!(space.state(Vec::new()).messages[0].text, "[Bash: working]");
        space.apply(Event::ToolFinished {
            id: "t".into(),
            error: None,
        });
        assert_eq!(space.state(Vec::new()).messages[0].text, "[Bash: done]");
    }
}
