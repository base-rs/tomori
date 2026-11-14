// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

use AgentState::{Error, Idle, Listening, Speaking, Thinking, Waiting, Working};

/// The states, in the order of the table's columns, each reached alone.
const STATES: [AgentState; 7] = [Idle, Listening, Thinking, Speaking, Working, Waiting, Error];

/// The events of the table in `design/agent-state.md`, in its order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    Prompted,
    Busy,
    Text,
    ToolStarted,
    ToolFinished,
    Idle,
    Requested,
    Resolved,
    Workers,
    SpeechToCome,
    SpeechDone,
    ListeningStarted,
    ListeningStopped,
    Failed,
    Disconnected,
    Recovered,
    Closed,
    StallTimeout,
    FailureTimeout,
}

/// The events of the table, by the names `design/agent-state.md` gives them.
const EVENTS: [(&str, Event); 19] = [
    ("Prompted", Event::Prompted),
    ("Busy", Event::Busy),
    ("Text", Event::Text),
    ("Tool started", Event::ToolStarted),
    ("Tool finished", Event::ToolFinished),
    ("Idle", Event::Idle),
    ("Request pending", Event::Requested),
    ("Request resolved", Event::Resolved),
    ("Workers changed", Event::Workers),
    ("Speech to come", Event::SpeechToCome),
    ("Speech done", Event::SpeechDone),
    ("Listening started", Event::ListeningStarted),
    ("Listening stopped", Event::ListeningStopped),
    ("Failed", Event::Failed),
    ("Disconnected", Event::Disconnected),
    ("Recovered", Event::Recovered),
    ("Closed", Event::Closed),
    ("Stall timeout", Event::StallTimeout),
    ("Failure timeout", Event::FailureTimeout),
];

/// A state, and the detail it must have where the table names one, as a cell of the table says it, such as
/// `Thinking, stalled`.
type Cell = (AgentState, Option<String>);

/// The state named `name` in the table.
fn state_named(name: &str) -> AgentState {
    STATES
        .into_iter()
        .find(|state| format!("{state:?}") == name)
        .unwrap_or_else(|| panic!("no state {name:?}"))
}

/// Each event, and the state it leaves each of [`STATES`] in, read from the table in `design/agent-state.md`, so that
/// the design and the machine cannot drift apart.
fn table() -> Vec<(Event, Vec<Cell>)> {
    let design = include_str!("../../../../design/agent-state.md");
    let mut rows = design
        .lines()
        .filter(|line| line.starts_with("| "))
        .map(|line| {
            line.trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect::<Vec<_>>()
        });
    let header = rows.next().expect("a table");
    assert_eq!(header[0], "Event");
    let columns: Vec<AgentState> = header[1..].iter().map(|name| state_named(name)).collect();
    assert_eq!(columns, STATES, "a column for each state, in order");
    let table: Vec<(Event, Vec<Cell>)> = rows
        .filter(|row| !row[0].starts_with("---"))
        .map(|row| {
            let event = EVENTS
                .iter()
                .find(|(name, _)| *name == row[0])
                .unwrap_or_else(|| panic!("no event {:?}", row[0]))
                .1;
            let cells = row[1..]
                .iter()
                .map(|cell| match cell.split_once(", ") {
                    Some((state, detail)) => (state_named(state), Some(detail.to_owned())),
                    None => (state_named(cell), None),
                })
                .collect::<Vec<_>>();
            assert_eq!(cells.len(), STATES.len(), "{event:?}");
            (event, cells)
        })
        .collect();
    let named: Vec<String> = table
        .iter()
        .map(|(event, _)| format!("{event:?}"))
        .collect();
    let expected: Vec<String> = EVENTS
        .iter()
        .map(|(_, event)| format!("{event:?}"))
        .collect();
    assert_eq!(named, expected, "every event tabulated once, in order");
    table
}

/// A machine in `state` alone, as the table's column for it starts, at `now`.
fn reached(state: AgentState, now: Instant) -> Machine {
    let mut machine = Machine::default();
    let inputs: &[Input] = match state {
        Idle => &[],
        Listening => &[Input::Listening(true)],
        Thinking => &[Input::Busy],
        Speaking => &[Input::Speech(true)],
        Working => &[
            Input::Busy,
            Input::ToolStarted {
                id: "t1",
                name: "shell",
            },
        ],
        Waiting => &[Input::Busy, Input::Waiting(true)],
        Error => &[Input::Failed("The agent's server returned an error.")],
        AgentState::Unknown => unreachable!("never found"),
    };
    for input in inputs {
        machine.apply(*input, now);
    }
    assert_eq!(machine.status().state, state, "reached alone");
    machine
}

/// Apply `event` to `machine`, at `now` or as long after it as the event's timeout.
fn happen(machine: &mut Machine, event: Event, now: Instant) {
    let input = match event {
        Event::Prompted => Input::Prompted,
        Event::Busy => Input::Busy,
        Event::Text => Input::Text,
        Event::ToolStarted => Input::ToolStarted {
            id: "t2",
            name: "shell",
        },
        Event::ToolFinished => Input::ToolFinished { id: "t1" },
        Event::Idle => Input::Idle,
        Event::Requested => Input::Waiting(true),
        Event::Resolved => Input::Waiting(false),
        Event::Workers => Input::Workers(2),
        Event::SpeechToCome => Input::Speech(true),
        Event::SpeechDone => Input::Speech(false),
        Event::ListeningStarted => Input::Listening(true),
        Event::ListeningStopped => Input::Listening(false),
        Event::Failed => Input::Failed("The agent's server returned an error."),
        Event::Disconnected => Input::Disconnected("The connection to the agent was lost."),
        Event::Recovered => Input::Recovered,
        Event::Closed => Input::Closed("The agent session ended."),
        Event::StallTimeout => {
            machine.apply(Input::Tick, now + STALL);
            return;
        }
        Event::FailureTimeout => {
            machine.apply(Input::Tick, now + FAILURE_HOLD);
            return;
        }
    };
    machine.apply(input, now);
}

#[test]
fn every_event_in_every_state_leaves_the_state_the_design_tabulates() {
    let now = Instant::now();
    for (event, after) in table() {
        for (state, (expected, detail)) in STATES.into_iter().zip(after) {
            let mut machine = reached(state, now);
            happen(&mut machine, event, now);
            assert_eq!(machine.status().state, expected, "{event:?} in {state:?}");
            if detail.is_some() {
                assert_eq!(machine.status().detail, detail, "{event:?} in {state:?}");
            }
        }
    }
}

#[test]
fn it_is_never_idle_while_a_turn_is_in_progress_whatever_follows() {
    let now = Instant::now();
    for state in STATES {
        for (_, first) in EVENTS {
            for (_, second) in EVENTS {
                let mut machine = reached(state, now);
                happen(&mut machine, first, now);
                happen(&mut machine, second, now);
                if machine.turn {
                    assert_ne!(
                        machine.status().state,
                        Idle,
                        "{first:?} then {second:?} from {state:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_most_pressing_state_shows_and_the_next_shows_as_each_ends() {
    let now = Instant::now();
    let mut machine = Machine::default();
    let shell = Input::ToolStarted {
        id: "t1",
        name: "shell",
    };
    for input in [
        Input::Busy,
        shell,
        Input::Speech(true),
        Input::Waiting(true),
        Input::Listening(true),
        Input::Failed("Overloaded."),
    ] {
        machine.apply(input, now);
    }
    let mut shown = vec![machine.status().state];
    for input in [
        Input::Tick,
        Input::Listening(false),
        Input::Waiting(false),
        Input::Speech(false),
        Input::ToolFinished { id: "t1" },
        Input::Idle,
    ] {
        let at = if input == Input::Tick {
            now + FAILURE_HOLD
        } else {
            now
        };
        machine.apply(input, at);
        shown.push(machine.status().state);
    }
    assert_eq!(
        shown,
        [Error, Listening, Waiting, Speaking, Working, Thinking, Idle]
    );
}

#[test]
fn each_change_raises_the_sequence_once_and_the_same_state_again_does_not() {
    let now = Instant::now();
    let mut machine = Machine::default();
    assert_eq!(machine.status().sequence, 0);
    assert!(machine.apply(Input::Busy, now));
    assert_eq!(machine.status().sequence, 1);
    assert!(!machine.apply(Input::Text, now), "still thinking");
    assert!(!machine.apply(Input::Workers(3), now));
    assert_eq!(machine.status().sequence, 1);
    let shell = Input::ToolStarted {
        id: "t1",
        name: "shell",
    };
    assert!(machine.apply(shell, now));
    assert!(!machine.apply(shell, now), "started once");
    // Another tool changes only the detail, which is a change.
    assert!(machine.apply(
        Input::ToolStarted {
            id: "t2",
            name: "read",
        },
        now
    ));
    assert_eq!(
        machine.status(),
        &AgentStatus {
            state: Working,
            detail: Some("running 2 tools".to_owned()),
            sequence: 3,
        }
    );
}

#[test]
fn working_says_what_runs_and_the_workers_its_sub_agent_calls_wait_on() {
    let now = Instant::now();
    let detail = |inputs: &[Input]| {
        let mut machine = Machine::default();
        for input in inputs {
            machine.apply(*input, now);
        }
        machine.status().detail.clone()
    };
    let task = |id| Input::ToolStarted { id, name: "task" };
    let shell = Input::ToolStarted {
        id: "s",
        name: "shell",
    };
    assert_eq!(detail(&[shell]).as_deref(), Some("running shell"));
    assert_eq!(detail(&[task("a")]).as_deref(), Some("running task"));
    assert_eq!(
        detail(&[task("a"), Input::Workers(1)]).as_deref(),
        Some("waiting on 1 worker")
    );
    assert_eq!(
        detail(&[task("a"), task("b"), Input::Workers(2)]).as_deref(),
        Some("waiting on 2 workers")
    );
    // Workers running beside a tool of its own are not what it waits on.
    assert_eq!(
        detail(&[task("a"), shell, Input::Workers(2)]).as_deref(),
        Some("running 2 tools")
    );
    // A name is bounded, on one line.
    let long = "x".repeat(200);
    let named = detail(&[Input::ToolStarted {
        id: "l",
        name: &format!("a\nb{long}"),
    }])
    .unwrap();
    assert!(named.starts_with("running a b"));
    assert_eq!(named.chars().count(), AGENT_DETAIL_CHARS);
    assert!(named.ends_with('…'));
}

#[test]
fn tool_calls_beyond_the_bound_are_not_kept_and_a_turn_s_end_clears_them() {
    let now = Instant::now();
    let mut machine = Machine::default();
    let ids: Vec<String> = (0..=TOOLS).map(|id| id.to_string()).collect();
    for id in &ids {
        machine.apply(Input::ToolStarted { id, name: "read" }, now);
    }
    assert_eq!(machine.tools.len(), TOOLS);
    assert_eq!(
        machine.status().detail,
        Some(format!("running {TOOLS} tools"))
    );
    machine.apply(Input::Idle, now);
    assert_eq!(machine.status().state, Idle);
    assert!(machine.tools.is_empty());
}

#[test]
fn an_error_says_why_briefly_and_a_closed_session_stays_in_error() {
    let now = Instant::now();
    let mut machine = Machine::default();
    machine.apply(Input::Disconnected("  "), now);
    assert_eq!(
        machine.status(),
        &AgentStatus {
            state: Error,
            detail: None,
            sequence: 1,
        }
    );
    // A disconnection lasts until the session is read again, however long.
    assert_eq!(machine.deadline(), None);
    machine.apply(Input::Tick, now + FAILURE_HOLD * 100);
    assert_eq!(machine.status().state, Error);
    machine.apply(Input::Recovered, now);
    assert_eq!(machine.status().state, Idle);
    machine.apply(Input::Busy, now);
    machine.apply(Input::Closed("The agent process exited."), now);
    assert_eq!(
        machine.status().detail.as_deref(),
        Some("The agent process exited.")
    );
    for input in [
        Input::Recovered,
        Input::Busy,
        Input::Text,
        Input::Failed("Later."),
        Input::Disconnected("Later."),
        Input::Tick,
    ] {
        machine.apply(input, now + FAILURE_HOLD * 100);
        assert_eq!(
            machine.status().detail.as_deref(),
            Some("The agent process exited."),
            "{input:?}"
        );
    }
}

#[test]
fn a_failure_passes_after_its_hold_or_with_progress() {
    let now = Instant::now();
    let mut machine = Machine::default();
    machine.apply(Input::Busy, now);
    machine.apply(Input::Failed("Rate limited."), now);
    assert_eq!(machine.deadline(), Some(now + FAILURE_HOLD));
    machine.apply(Input::Tick, now + FAILURE_HOLD - Duration::from_millis(1));
    assert_eq!(machine.status().state, Error);
    machine.apply(Input::Tick, now + FAILURE_HOLD);
    assert_eq!(machine.status().state, Thinking, "its turn goes on");
    machine.apply(Input::Failed("Rate limited."), now);
    machine.apply(Input::Text, now);
    assert_eq!(machine.status().state, Thinking);
}

#[test]
fn a_turn_with_no_word_for_the_stall_timeout_says_it_stalled_but_still_thinks() {
    let now = Instant::now();
    let mut machine = Machine::default();
    machine.apply(Input::Prompted, now);
    assert_eq!(machine.deadline(), Some(now + STALL));
    assert!(!machine.apply(Input::Tick, now + STALL - Duration::from_millis(1)));
    assert!(machine.apply(Input::Tick, now + STALL));
    assert_eq!(
        machine.status(),
        &AgentStatus {
            state: Thinking,
            detail: Some("stalled".to_owned()),
            sequence: 2,
        }
    );
    assert_eq!(machine.deadline(), None, "told once");
    // Any word of it ends the stall.
    let later = now + STALL * 2;
    assert!(machine.apply(Input::Text, later));
    assert_eq!(machine.status().detail, None);
    assert_eq!(machine.deadline(), Some(later + STALL));
    // A tool runs as long as it takes, and nothing stalls while it does, or while someone is asked.
    machine.apply(
        Input::ToolStarted {
            id: "t",
            name: "shell",
        },
        later,
    );
    assert_eq!(machine.deadline(), None);
    machine.apply(Input::ToolFinished { id: "t" }, later);
    machine.apply(Input::Waiting(true), later);
    assert_eq!(machine.deadline(), None);
    machine.apply(Input::Idle, later);
    machine.apply(Input::Waiting(false), later);
    assert_eq!(machine.deadline(), None, "no turn to stall");
}

#[test]
fn the_earlier_of_a_failure_passing_and_a_stall_is_due() {
    let now = Instant::now();
    let mut machine = Machine::default();
    machine.apply(Input::Busy, now);
    machine.apply(Input::Failed("Overloaded."), now);
    // Its turn is hidden by the failure, so only the failure is due.
    assert_eq!(machine.deadline(), Some(now + FAILURE_HOLD));
    // Thinking again, it stalls from then.
    machine.apply(Input::Tick, now + FAILURE_HOLD);
    assert_eq!(machine.deadline(), Some(now + FAILURE_HOLD + STALL));
}

#[test]
fn thinking_again_after_speaking_stalls_from_then_and_a_stall_holds_whatever_the_time_given() {
    let now = Instant::now();
    let mut machine = Machine::default();
    machine.apply(Input::Busy, now);
    machine.apply(Input::Speech(true), now);
    assert_eq!(machine.deadline(), None);
    let spoken = now + STALL * 2;
    machine.apply(Input::Speech(false), spoken);
    assert_eq!(machine.status().detail, None);
    assert_eq!(machine.deadline(), Some(spoken + STALL));
    machine.apply(Input::Tick, spoken + STALL);
    assert_eq!(machine.status().detail.as_deref(), Some("stalled"));
    // Told again with an earlier time, as when it is brought up to date for another reason, it is still stalled.
    assert!(!machine.apply(Input::Tick, now));
}

#[test]
fn workers_running_outside_a_turn_leave_it_idle_saying_so_and_working_only_in_a_turn() {
    let now = Instant::now();
    let mut machine = Machine::default();
    machine.apply(Input::Workers(1), now);
    assert_eq!(
        machine.status(),
        &AgentStatus {
            state: Idle,
            detail: Some("1 worker running".to_owned()),
            sequence: 1,
        }
    );
    machine.apply(Input::Workers(2), now);
    assert_eq!(
        machine.status().detail.as_deref(),
        Some("2 workers running")
    );
    // In a turn its sub-agent calls wait on them; once it ends, it is idle again, saying so.
    machine.apply(Input::Busy, now);
    machine.apply(
        Input::ToolStarted {
            id: "a",
            name: "task",
        },
        now,
    );
    assert_eq!(
        (machine.status().state, machine.status().detail.as_deref()),
        (Working, Some("waiting on 2 workers"))
    );
    machine.apply(Input::Idle, now);
    assert_eq!(
        (machine.status().state, machine.status().detail.as_deref()),
        (Idle, Some("2 workers running"))
    );
    machine.apply(Input::Workers(0), now);
    assert_eq!(machine.status().detail, None);
}
