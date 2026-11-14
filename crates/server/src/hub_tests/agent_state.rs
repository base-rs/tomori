// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent's state driven through the hub by a turn's real events, as everyone in the space is told it: listening as
//! someone speaks and while their words are transcribed, thinking from the moment they are sent, working while a tool
//! runs, speaking while its speech is still to come, waiting on a request, an error with its reason until it passes,
//! and idle only once the turn and its speech are over, each change with a new sequence number. See
//! `design/agent-state.md`.
use super::following::{following, heard, last_state, press, release};
use super::*;
use crate::agent_state::{FAILURE_HOLD, STALL};
use tomori_protocol::AgentState;

/// What `client` was last told the agent is doing, once what is deferred until `at` has been sent, if they were told
/// anything.
fn told_at(hub: &Hub, client: &Client, at: Instant) -> Option<(AgentState, Option<String>, u64)> {
    hub.flush_snapshots(at);
    last_state(client)
        .and_then(|state| state.agent)
        .map(|agent| (agent.state, agent.detail, agent.sequence))
}

/// What `client` was last told the agent is doing, once the paced snapshot is due.
fn told(hub: &Hub, client: &Client) -> (AgentState, Option<String>) {
    let (state, detail, _) =
        told_at(hub, client, Instant::now() + crate::hub::SNAPSHOT_INTERVAL).expect("told");
    (state, detail)
}

/// The input last sent to the agent, of the commands given its driver.
fn sent(commands: &async_channel::Receiver<SpaceCommand>) -> Option<tomori_backend::Prompt> {
    std::iter::from_fn(|| commands.try_recv().ok())
        .filter_map(|command| match command {
            SpaceCommand::Send(prompt) => Some(prompt),
            _ => None,
        })
        .last()
}

#[test]
fn everyone_is_told_the_agent_s_state_through_a_turn_from_its_events() {
    let (hub, _jobs, space, commands, ada) = following(None);
    let first = hub
        .lock()
        .space(&space)
        .unwrap()
        .agent
        .borrow()
        .status()
        .clone();
    assert_eq!(first.state, AgentState::Idle);
    // Ada speaks to it, and her words are transcribed after she lets go.
    press(&ada, "u1");
    assert_eq!(told(&hub, &ada), (AgentState::Listening, None));
    release(&ada, "u1");
    hub.flush_snapshots(Instant::now() + crate::hub::SNAPSHOT_INTERVAL);
    assert!(
        last_state(&ada).is_none(),
        "still listening, nothing new to tell"
    );
    // They are sent: it thinks before its backend says it is busy.
    heard(&hub, "Run the tests.");
    assert!(sent(&commands).is_some());
    assert_eq!(told(&hub, &ada), (AgentState::Thinking, None));
    hub.drive_event(&space, Event::Busy);
    hub.drive_event(
        &space,
        Event::ToolStarted {
            message: "m1".to_owned(),
            tool: "shell".to_owned(),
            id: "t1".to_owned(),
            input: serde_json::json!({}),
        },
    );
    assert_eq!(
        told(&hub, &ada),
        (AgentState::Working, Some("running shell".to_owned()))
    );
    // A sentence it wrote before the tool is still to come: speaking, over the tool.
    hub.speech_queued(&space, 1);
    assert_eq!(told(&hub, &ada), (AgentState::Speaking, None));
    hub.speech_sent(&space);
    assert_eq!(
        told(&hub, &ada).0,
        AgentState::Working,
        "working again once it is sent"
    );
    hub.drive_event(
        &space,
        Event::ToolFinished {
            id: "t1".to_owned(),
            error: None,
        },
    );
    assert_eq!(told(&hub, &ada), (AgentState::Thinking, None));
    hub.drive_event(&space, Event::PermissionRequested(permission()));
    assert_eq!(told(&hub, &ada), (AgentState::Waiting, None));
    hub.drive_event(
        &space,
        Event::PermissionResolved {
            id: "request-1".to_owned(),
        },
    );
    assert_eq!(told(&hub, &ada), (AgentState::Thinking, None));
    // Its turn ends with speech still to come: speaking, and idle only once it has been sent.
    hub.speech_queued(&space, 1);
    hub.drive_event(&space, Event::Idle);
    assert_eq!(told(&hub, &ada), (AgentState::Speaking, None));
    hub.speech_sent(&space);
    let (state, _, sequence) =
        told_at(&hub, &ada, Instant::now() + crate::hub::SNAPSHOT_INTERVAL).unwrap();
    assert_eq!(state, AgentState::Idle);
    assert!(
        sequence > first.sequence + 8,
        "raised with each change: {sequence}"
    );
}

#[test]
fn a_failure_shows_its_reason_until_it_passes_and_a_closed_session_stays_in_error() {
    let (hub, _jobs, space, _commands, ada) = following(None);
    hub.drive_event(
        &space,
        Event::Error {
            message: "The model is overloaded.".to_owned(),
        },
    );
    assert_eq!(
        told(&hub, &ada),
        (
            AgentState::Error,
            Some("The model is overloaded.".to_owned())
        )
    );
    // The server's timer wakes for it to pass.
    let deadline = hub.snapshot_deadline().expect("due");
    assert!(deadline <= Instant::now() + FAILURE_HOLD);
    let (state, ..) = told_at(&hub, &ada, Instant::now() + FAILURE_HOLD).unwrap();
    assert_eq!(state, AgentState::Idle);
    hub.drive_event(
        &space,
        Event::Reconnecting {
            reason: "Lost the agent.".to_owned(),
        },
    );
    assert_eq!(told(&hub, &ada).0, AgentState::Error);
    hub.drive_event(
        &space,
        Event::Resynchronized {
            snapshot: Snapshot {
                busy: Some(true),
                ..Snapshot::default()
            },
        },
    );
    assert_eq!(
        told(&hub, &ada),
        (AgentState::Thinking, None),
        "its turn goes on as the session read again says"
    );
    hub.drive_event(&space, Event::Closed { reason: None });
    assert_eq!(
        told(&hub, &ada),
        (
            AgentState::Error,
            Some("The agent session ended.".to_owned())
        )
    );
}

#[test]
fn a_turn_with_no_word_says_it_stalled_when_the_timer_wakes_and_a_cancelled_utterance_stops_listening()
 {
    let (hub, _jobs, space, _commands, ada) = following(None);
    hub.drive_event(&space, Event::Busy);
    assert_eq!(told(&hub, &ada), (AgentState::Thinking, None));
    let deadline = hub.snapshot_deadline().expect("due");
    assert!(deadline <= Instant::now() + STALL);
    let (state, detail, _) = told_at(&hub, &ada, Instant::now() + STALL).unwrap();
    assert_eq!(
        (state, detail.as_deref()),
        (AgentState::Thinking, Some("stalled"))
    );
    // Someone speaking to it shows over its turn, until they cancel.
    press(&ada, "u1");
    assert_eq!(told(&hub, &ada).0, AgentState::Listening);
    super::following::cancel(&ada, "u1");
    assert_eq!(told(&hub, &ada).0, AgentState::Thinking);
}

#[test]
fn words_that_fail_to_transcribe_stop_listening_and_input_that_is_not_delivered_ends_its_turn() {
    let (hub, _jobs, space, commands, ada) = following(None);
    press(&ada, "u1");
    release(&ada, "u1");
    assert_eq!(told(&hub, &ada).0, AgentState::Listening);
    let utterance = *hub.lock().utterances.keys().next().unwrap();
    hub.transcript_failed(utterance, "lost");
    assert_eq!(told(&hub, &ada).0, AgentState::Idle);
    press(&ada, "u2");
    release(&ada, "u2");
    heard(&hub, "Hello.");
    let prompt = sent(&commands).expect("sent");
    assert_eq!(told(&hub, &ada).0, AgentState::Thinking);
    hub.space_sent(
        &space,
        &prompt.key,
        Err(tomori_backend::Error {
            kind: ErrorKind::Unavailable,
            message: "The agent is not running.".to_owned(),
        }),
    );
    assert_eq!(told(&hub, &ada).0, AgentState::Idle);
    // Input not delivered once its backend says it is busy, as with another's input, leaves its turn going on.
    press(&ada, "u3");
    release(&ada, "u3");
    heard(&hub, "And then?");
    let prompt = sent(&commands).expect("sent");
    hub.drive_event(&space, Event::Busy);
    hub.space_sent(
        &space,
        &prompt.key,
        Err(tomori_backend::Error {
            kind: ErrorKind::Unavailable,
            message: "The agent is busy.".to_owned(),
        }),
    );
    assert_eq!(told(&hub, &ada).0, AgentState::Thinking);
}
