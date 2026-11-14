// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent's state in a space, one explicit state machine driven by the turn's events: the backend's session
//! events, its speech still to come, its participants' recording and its backend's failures, each an [`Input`]. The
//! state is the most pressing of the facts they leave, by a fixed precedence, so that every client shows the same
//! state, and the agent is never idle while a turn is in progress. Pure: the time is given. Nothing is kept across a
//! restart. See `design/agent-state.md`.
use std::time::{Duration, Instant};

use tomori_protocol::{AGENT_DETAIL_CHARS, AgentState, AgentStatus};

/// How long a turn may go without any event of it, with no tool running and nothing waiting, before it is said to
/// have stalled. It is still in progress, so it stays thinking.
pub(crate) const STALL: Duration = Duration::from_secs(120);

/// How long a recoverable failure the backend reports shows, unless the turn makes progress first.
pub(crate) const FAILURE_HOLD: Duration = Duration::from_secs(10);

/// The most tool calls kept running at once; any beyond them are counted as running no more, and a turn's end clears
/// them all.
const TOOLS: usize = 64;

/// The tools that delegate to a sub-agent, whose call waits on the space's workers.
const DELEGATIONS: &[&str] = &["task"];

/// What a stalled turn says.
const STALLED: &str = "stalled";

/// Something that happened to the agent's turn. Each is applied in every state; see the table in
/// `design/agent-state.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Input<'a> {
    /// Input was sent to the agent, beginning a turn before its backend says it is busy.
    Prompted,
    /// The backend says the agent started working on a turn.
    Busy,
    /// The agent streamed text or began a message.
    Text,
    /// A tool call started.
    ToolStarted { id: &'a str, name: &'a str },
    /// A tool call finished, failed or not.
    ToolFinished { id: &'a str },
    /// The backend says the agent finished its turn.
    Idle,
    /// Whether an approval or a question is pending.
    Waiting(bool),
    /// How many of the space's workers are running.
    Workers(usize),
    /// Whether the agent's speech is still to come.
    Speech(bool),
    /// Whether a participant is speaking to the agent, or their words are being transcribed.
    Listening(bool),
    /// The backend reported a recoverable failure.
    Failed(&'a str),
    /// The backend lost its connection to the agent.
    Disconnected(&'a str),
    /// The backend read the session again after losing it.
    Recovered,
    /// The session ended.
    Closed(&'a str),
    /// Time passed, for the timeouts.
    Tick,
}

/// A failure shown as the error state, and until when, if it passes of itself.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Failure {
    reason: String,
    until: Option<Instant>,
}

/// The agent's state in one space, and the facts it is found from.
#[derive(Clone, Debug)]
pub(crate) struct Machine {
    /// Whether a turn is in progress: input sent to the agent, or its backend busy, until its turn ends.
    turn: bool,
    /// The tool calls running, by id, with their names, oldest first.
    tools: Vec<(String, String)>,
    /// How many of the space's workers are running.
    workers: usize,
    waiting: bool,
    speaking: bool,
    listening: bool,
    /// The failure shown, while it is.
    failure: Option<Failure>,
    /// Whether the session has ended, which nothing recovers.
    closed: bool,
    /// When the turn last made progress, or began thinking again, from which it stalls.
    heard: Option<Instant>,
    /// Whether it has thought for the [`STALL`] timeout without any word of its turn.
    stalled: bool,
    status: AgentStatus,
}

impl Default for Machine {
    /// Idle, as a space is before anything happens in it, at sequence 0.
    fn default() -> Self {
        Self {
            turn: false,
            tools: Vec::new(),
            workers: 0,
            waiting: false,
            speaking: false,
            listening: false,
            failure: None,
            closed: false,
            heard: None,
            stalled: false,
            status: AgentStatus {
                state: AgentState::Idle,
                detail: None,
                sequence: 0,
            },
        }
    }
}

impl Machine {
    /// The agent's state as it was last found, with its sequence number.
    pub(crate) fn status(&self) -> &AgentStatus {
        &self.status
    }

    /// Apply `input` at `now`: returns whether the state or its detail changed, raising its sequence number.
    pub(crate) fn apply(&mut self, input: Input, now: Instant) -> bool {
        match input {
            Input::Prompted => {
                self.turn = true;
                self.progressed(now);
            }
            Input::Busy | Input::Text => {
                self.turn = true;
                self.progressed(now);
                self.recovered();
            }
            Input::ToolStarted { id, name } => {
                if !self.tools.iter().any(|(known, _)| known == id) && self.tools.len() < TOOLS {
                    self.tools.push((id.to_owned(), name.to_owned()));
                }
                self.turn = true;
                self.progressed(now);
                self.recovered();
            }
            Input::ToolFinished { id } => {
                self.tools.retain(|(known, _)| known != id);
                self.progressed(now);
            }
            Input::Idle => {
                self.turn = false;
                self.tools.clear();
                self.heard = None;
                self.stalled = false;
            }
            Input::Waiting(waiting) => {
                if self.waiting != waiting {
                    self.progressed(now);
                }
                self.waiting = waiting;
            }
            Input::Workers(workers) => self.workers = workers,
            Input::Speech(speaking) => self.speaking = speaking,
            Input::Listening(listening) => self.listening = listening,
            Input::Failed(reason) => self.fail(reason, Some(now + FAILURE_HOLD)),
            Input::Disconnected(reason) => self.fail(reason, None),
            Input::Recovered => {
                // What ran while the backend could not see is unknown: the session it read again says whether a turn
                // goes on, which comes as its own input.
                self.tools.clear();
                self.recovered();
            }
            Input::Closed(reason) => {
                self.closed = true;
                self.turn = false;
                self.tools.clear();
                self.heard = None;
                self.stalled = false;
                self.waiting = false;
                self.failure = Some(Failure {
                    reason: reason.to_owned(),
                    until: None,
                });
            }
            Input::Tick => {}
        }
        if self
            .failure
            .as_ref()
            .and_then(|failure| failure.until)
            .is_some_and(|until| until <= now)
        {
            self.failure = None;
        }
        // Thinking stalls from when it last made progress, or began thinking again after something else, at the latest.
        if self.thinking() {
            if self.status.state != AgentState::Thinking {
                self.heard = Some(now);
            }
            self.stalled |= self.heard.is_some_and(|heard| now >= heard + STALL);
        }
        let (state, detail) = self.derive();
        let changed = self.status.state != state || self.status.detail != detail;
        if changed {
            self.status = AgentStatus {
                state,
                detail,
                sequence: self.status.sequence + 1,
            };
        }
        changed
    }

    /// When the state next changes of itself, as a failure passes or a turn stalls, if it would.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        let failure = self.failure.as_ref().and_then(|failure| failure.until);
        // Only thinking stalls: a tool runs as long as it takes, and the others wait on someone.
        let stall = self
            .heard
            .map(|heard| heard + STALL)
            .filter(|_| self.thinking() && !self.stalled);
        failure.into_iter().chain(stall).min()
    }

    fn progressed(&mut self, now: Instant) {
        self.heard = Some(now);
        self.stalled = false;
    }

    /// Whether the facts leave it thinking: in a turn, with nothing more pressing.
    fn thinking(&self) -> bool {
        self.derive().0 == AgentState::Thinking
    }

    /// A failure that passes with progress no longer shows; one of a closed session always does.
    fn recovered(&mut self) {
        if !self.closed {
            self.failure = None;
        }
    }

    fn fail(&mut self, reason: &str, until: Option<Instant>) {
        if !self.closed {
            self.failure = Some(Failure {
                reason: reason.to_owned(),
                until,
            });
        }
    }

    /// The most pressing state the facts leave: an error, then listening, waiting, speaking, working, thinking and idle.
    fn derive(&self) -> (AgentState, Option<String>) {
        if let Some(failure) = &self.failure {
            (AgentState::Error, detail(&failure.reason))
        } else if self.listening {
            (AgentState::Listening, None)
        } else if self.waiting {
            (AgentState::Waiting, None)
        } else if self.speaking {
            (AgentState::Speaking, None)
        } else if !self.tools.is_empty() {
            (AgentState::Working, self.working())
        } else if self.turn {
            (
                AgentState::Thinking,
                self.stalled.then(|| STALLED.to_owned()),
            )
        } else {
            // Its workers running in the background leave it idle, free to be spoken to, saying so.
            (AgentState::Idle, running(self.workers))
        }
    }

    /// What the agent is working on: the workers its sub-agent calls wait on, the tool it runs, or how many.
    fn working(&self) -> Option<String> {
        let delegating = self
            .tools
            .iter()
            .all(|(_, name)| DELEGATIONS.contains(&name.as_str()));
        match self.tools.as_slice() {
            _ if delegating && self.workers == 1 => Some("waiting on 1 worker".to_owned()),
            _ if delegating && self.workers > 1 => {
                Some(format!("waiting on {} workers", self.workers))
            }
            [(_, name)] => detail(&format!("running {name}")),
            tools => Some(format!("running {} tools", tools.len())),
        }
    }
}

/// How many workers run in the background, if any.
fn running(workers: usize) -> Option<String> {
    match workers {
        0 => None,
        1 => Some("1 worker running".to_owned()),
        workers => Some(format!("{workers} workers running")),
    }
}

/// `text` as a detail clients are sent: on one line, at most [`AGENT_DETAIL_CHARS`] characters, or none if empty.
fn detail(text: &str) -> Option<String> {
    let line = tomori_protocol::text::one_line(text.trim(), AGENT_DETAIL_CHARS - 1);
    Some(line).filter(|line| !line.trim().is_empty())
}

#[cfg(test)]
mod tests;
