// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Driving each space's agent state machine from what the hub sees of its turn: the backend's events, input sent to
//! the agent, its speech still to come, its participants' utterances and the machine's own timeouts. Everyone in the
//! space is told of a change as of any other, paced. See `design/agent-state.md`.
use tomori_protocol::{Activity, WorkerState};

use super::*;
use crate::agent_state::Input;

impl SpaceRecord {
    /// Apply the backend's `event` to the agent's state at `now`, before the space's model applies it.
    pub(super) fn agent_event(&self, event: &Event, now: Instant) {
        let record = self;
        let input = match event {
            Event::Busy => Input::Busy,
            Event::Idle => Input::Idle,
            Event::MessageStarted {
                role: tomori_backend::Role::Assistant,
                ..
            } => Input::Text,
            Event::TextDelta { message, .. } if !record.model.is_user_message(message) => {
                Input::Text
            }
            Event::ToolStarted { id, tool, .. } => Input::ToolStarted { id, name: tool },
            Event::ToolFinished { id, .. } => Input::ToolFinished { id },
            Event::Error { message } => Input::Failed(message),
            Event::Reconnecting { reason } => Input::Disconnected(reason),
            Event::Resynchronized { .. } => Input::Recovered,
            Event::Closed { reason } => {
                Input::Closed(reason.as_deref().unwrap_or("The agent session ended."))
            }
            _ => return,
        };
        record.agent.borrow_mut().apply(input, now);
    }

    /// After the space's session was read again, whether its turn goes on, as the session says.
    pub(super) fn agent_resynchronized(&self, now: Instant) {
        let input = match self.model.activity() {
            Activity::Running | Activity::Waiting => Input::Busy,
            _ => Input::Idle,
        };
        self.agent.borrow_mut().apply(input, now);
    }
}

impl HubState {
    /// Input was sent to `space`'s agent at `now`, beginning a turn: everyone in it is told, paced.
    pub(super) fn agent_prompted(&self, space: &str, now: Instant) {
        if self
            .space(space)
            .is_some_and(|record| record.agent.borrow_mut().apply(Input::Prompted, now))
            && self.touch_space_paced(space, now)
        {
            self.woken.notify(usize::MAX);
        }
    }

    /// Input sent to `space`'s agent was not delivered: unless its backend says it is in a turn, the turn its sending
    /// began has ended.
    pub(super) fn agent_unprompted(&self, space: &str, now: Instant) {
        if let Some(record) = self.space(space)
            && !matches!(
                record.model.activity(),
                Activity::Running | Activity::Waiting
            )
        {
            record.agent.borrow_mut().apply(Input::Idle, now);
        }
    }

    /// Bring `space`'s agent state up to date at `now` with what the space holds: whether someone speaks to the agent
    /// or their words are transcribed, whether a request waits, how many of its workers run, whether its speech is
    /// still to come as its clients were told, and its timeouts. Returns whether it changed.
    pub(super) fn sync_agent(&self, space: &str, now: Instant) -> bool {
        let Some(record) = self.space(space) else {
            return false;
        };
        // Naming someone or titling the space is not speaking to the agent.
        let listening = self
            .utterances
            .values()
            .any(|utterance| utterance.space == space && !utterance.naming && !utterance.titling);
        let model = &record.model;
        let waiting = model.activity() == Activity::Waiting;
        let workers = model
            .workers()
            .iter()
            .filter(|worker| worker.state == WorkerState::Running)
            .count();
        let mut machine = record.agent.borrow_mut();
        [
            Input::Listening(listening),
            Input::Waiting(waiting),
            Input::Workers(workers),
            Input::Speech(record.following.to_come()),
            Input::Tick,
        ]
        .into_iter()
        .fold(false, |changed, input| machine.apply(input, now) | changed)
    }

    /// Bring `space`'s agent state up to date at `now`, as when someone in it starts or stops speaking to the agent,
    /// telling everyone in it, paced, if it changed.
    pub(super) fn agent_changed(&self, space: &str, now: Instant) {
        if self.sync_agent(space, now) && self.touch_space_paced(space, now) {
            self.woken.notify(usize::MAX);
        }
    }

    /// When any space's agent state next changes of itself, as a failure passes or a turn stalls.
    pub(super) fn agent_deadline(&self) -> Option<Instant> {
        self.spaces
            .iter()
            .filter_map(|space| space.agent.borrow().deadline())
            .min()
    }

    /// Bring the agent states due by `now` up to date, telling everyone in a space whose state changed.
    pub(super) fn agents_due(&self, now: Instant) {
        let due: Vec<String> = self
            .spaces
            .iter()
            .filter(|space| {
                space
                    .agent
                    .borrow()
                    .deadline()
                    .is_some_and(|due| due <= now)
            })
            .map(|space| space.model.id.clone())
            .collect();
        // Each is due only when it changes of itself, as it does now.
        for space in due {
            self.sync_agent(&space, now);
            self.touch_space(&space);
        }
    }
}
