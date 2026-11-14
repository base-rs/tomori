# Agent State

This describes the agent's state in a space: one explicit state machine, owned by the server and driven by the turn's real events, which every client renders, so that the status pill, the swarm, the spectator and the turn's cue all say the same thing.

## Requirement

**The server owns one state machine per space.** The server finds what the agent in each space is doing with one state machine (`crates/server/src/agent_state.rs`), driven by inputs from the turn's real events, and sends its state to everyone in the space in the space's state, `SpaceState::agent`: the state, a short detail and a sequence number (`tomori_protocol::AgentStatus`). Its states are:

- **Idle:** no turn in progress, and none of the agent's speech playing or still to come. The agent can be spoken to. While the space's workers run in the background its detail says so, such as `2 workers running`, and the pill shows `idle · 2 workers running`. Workers running outside a turn never show as Working.
- **Listening:** a participant is speaking to the agent, from when they press X until their words have been transcribed and queued or have failed. Words that name a participant or title the space are not spoken to the agent.
- **Thinking:** a turn is in progress, from when input is sent to the agent, before its backend says it is busy, until its turn ends, with nothing more pressing.
- **Speaking:** the agent's speech is still to come, as `speech.md` counts it (`speech_to_come`), during its turn or after it.
- **Working:** a tool or sub-agent call is running during a turn. Its detail names it: `running shell` for one tool, `running 3 tools` for several, and `waiting on 2 workers` while only sub-agent calls (OpenCode's `task`) run and the space's workers are running.
- **Waiting:** an approval or question is pending, including one whose decision or answers are being delivered.
- **Error:** the backend reported a failure, lost its connection to the agent, or its session ended, with the reason as its detail.

**Its inputs are the turn's events.** The backend's session events: `Busy`, `Idle`, an assistant message started or its text streamed, `ToolStarted` and `ToolFinished`, approvals and questions asked and resolved, the workers running, `Error`, `Reconnecting`, `Resynchronized`, after which the session read again says whether its turn goes on, and `Closed`; input sent to the agent (`Prompted`), and input that could not be delivered, which ends a turn its backend has not said is busy; whether the agent's speech is still to come, as its clients are told it; whether anyone in the space is recording or being transcribed for the agent; and two timeouts.

**Precedence is explicit.** The state is the most pressing of the facts the inputs leave: Error, then Listening, Waiting, Speaking, Working, Thinking and Idle. Error overrides every other state, so that a failed or closed session is never shown otherwise; a participant's own recording still shows as listening on their own device, a fact of that device (below).

**It is never idle while a turn is in progress.** From input sent, or the backend busy, until its turn ends, the state is never Idle, whatever else happens; Idle requires that no turn is in progress, nothing is pending, nobody is speaking to the agent, and none of its speech is still to come.

**Stalls and errors are explicit transitions, with timeouts.** A turn that has been Thinking for 2 minutes (`agent_state::STALL`) with no event of it, counted from its last event or from when it began thinking again after something more pressing, stays Thinking, with the detail `stalled`, until any event of its turn; a tool runs as long as it takes, and nothing stalls while a tool runs or a request waits. A deliberate cancel is no failure: OpenCode reports one as `session.execution.interrupted`, or as `session.execution.failed` with an error of type `aborted` (or the older `MessageAbortedError`), and its backend reports either as the end of the turn (`Idle`) rather than an `Error`, so that a cancel or barge-in never shows an error. A failure the backend reports (`Error`) shows as Error for 10 seconds (`agent_state::FAILURE_HOLD`), or until the turn makes progress (`Busy`, text, a tool started) or the session is read again; a lost connection shows until the session is read again, however long; a closed session stays in Error, whatever follows. Speech that has made no progress for a minute is no longer to come, as `speech.md` says. Interruptions are transitions too: a participant pressing X shows Listening over everything but Error, cancelling or failing their words ends it, and synthesis stopping ends Speaking. The server wakes for each timeout, rather than waiting for another event.

**Every event in every state is tabulated.** The table gives the state each event leaves each state in, where that state was reached alone, as in the test: Idle from nothing, Listening from a participant recording, Thinking from `Busy`, Speaking from speech to come after the turn, Working from `Busy` and a tool started, Waiting from `Busy` and a request, Error from a reported failure. Where several facts hold, precedence decides. A cell may give the detail the state must have after a comma, such as `Thinking, stalled`. The test reads this table from this document, so the two cannot drift apart.

| Event | Idle | Listening | Thinking | Speaking | Working | Waiting | Error |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Prompted | Thinking | Listening | Thinking | Speaking | Working | Waiting | Error |
| Busy | Thinking | Listening | Thinking | Speaking | Working | Waiting | Thinking |
| Text | Thinking | Listening | Thinking | Speaking | Working | Waiting | Thinking |
| Tool started | Working | Listening | Working | Speaking | Working | Waiting | Working |
| Tool finished | Idle | Listening | Thinking | Speaking | Thinking | Waiting | Error |
| Idle | Idle | Listening | Idle | Speaking | Idle | Waiting | Error |
| Request pending | Waiting | Listening | Waiting | Waiting | Waiting | Waiting | Error |
| Request resolved | Idle | Listening | Thinking | Speaking | Working | Thinking | Error |
| Workers changed | Idle, 2 workers running | Listening | Thinking | Speaking | Working | Waiting | Error |
| Speech to come | Speaking | Listening | Speaking | Speaking | Speaking | Waiting | Error |
| Speech done | Idle | Listening | Thinking | Idle | Working | Waiting | Error |
| Listening started | Listening | Listening | Listening | Listening | Listening | Listening | Error |
| Listening stopped | Idle | Idle | Thinking | Speaking | Working | Waiting | Error |
| Failed | Error | Error | Error | Error | Error | Error | Error |
| Disconnected | Error | Error | Error | Error | Error | Error | Error |
| Recovered | Idle | Listening | Thinking | Speaking | Thinking | Waiting | Idle |
| Closed | Error | Error | Error | Error | Error | Error | Error |
| Stall timeout | Idle | Listening | Thinking, stalled | Speaking | Working | Waiting | Idle |
| Failure timeout | Idle | Listening | Thinking | Speaking | Working | Waiting | Idle |

**Each state carries a sequence number.** `AgentStatus::sequence` changes with every change of the state or its detail in the space, for this run of the server, so that a client tells a change from the same state sent again; only its equality means anything, since it starts again when the server restarts. A detail is at most `AGENT_DETAIL_CHARS` (48) characters on one line.

**The update rate is bounded.** The state is sent in the space's state, paced as the agent's events are (`SNAPSHOT_INTERVAL`): a change within the interval since everyone was last told is sent at its end, as the state is then, so that a state lasting less than the interval may never be sent. Nothing about the state is sent on its own.

**The protocol stays compatible.** `SpaceState::agent` is optional: an older server never sends it, and an older client ignores it and works out what the agent is doing from `SpaceState::activity` and `speech_to_come` as before, which the server still sends unchanged. A newer client prefers the server's state where it is sent; with none, or while it is not connected, or with a state it does not know (`AgentState::Unknown`), it works it out as before.

**Clients render it, layering only this device's facts.** The headset, the desktop window and the spectator show the server's state rather than working it out: the status pill's first row (see `interaction.md`) says `idle`, `listening`, `thinking`, `thinking, stalled`, `speaking`, Working's detail or `working`, `waiting`, or `error` with its reason; the swarm shows listening, waiting, speaking, working for thinking and working, ready for idle, and unavailable for an error; and the participant's turn (see `speech.md`) comes only once the server says the agent is idle and nothing of theirs is queued. What only the device knows is layered on it: its own recording, shown as listening over everything; agent speech playing here, with the swarm's hold and its level, and whether it is muted or skipped here, which shows over thinking, working and idle but not over listening, waiting or an error; a reply held for the participant, which shows as a reply waiting and prepares; and its own transcription and failures. Holds, skips and mute are per device, so they stay on the device.

**The client smooths it.** A client shows a more pressing state at once, by the same precedence, and a less pressing one only once it has lasted 0.4 seconds (`agent_view::BRIEF`), counted from when it was first said, or once the state shown has not been said for 1 second (`agent_view::STALE`), when the newest shows, so that a run of brief states never keeps an old one on screen, so that a state lasting a moment, such as working between two sentences, never flickers through the pill or the swarm; the same state with another detail shows at once. It compares the whole state, not only its sequence number, and starts afresh in each space, since sequence numbers are each space's own, as when the server moves a participant straight into another space. The pill and the swarm follow the same smoothed state, so they never disagree; the turn follows the state as last said, with its own hold.

## Rationale

In November 2026 the headset's pill and swarm showed the agent idle in the middle of a reply, because each worked out the agent's state from several inputs with their own timing: audio playback with its 0.6 s hold, the session's activity, and then the speech still to come (#489). Each fix patched one input. The maintainer agreed to one explicit state machine, owned by the server, which sees every one of the turn's events, driven by them and rendered by every client with light smoothing (#491). An independent review of #489 asked for explicit precedence, explicit stall and error transitions, a sequence number on each state so that clients can smooth changes, and a per-participant layer, since holds, skips and mute are per device, with playback and its level staying on the client, which alone knows what it plays.

## Scope

The state machine in `tomori-server`'s `agent_state.rs`, driven by the hub in `hub/agent_state.rs`; the protocol's `AgentState` and `AgentStatus` in `SpaceState::agent`; and how clients render it, in `tomori-scene`'s `agent_view.rs`, the conversation, the swarm's signals in `interaction.rs` and the status pill, drawn by the spectator through the frame step as a watcher.

## Validation

- Test, in `agent_state/tests.rs`, every event of the table in every state, reached alone, against the table read from this document, and that no two events from any state leave it idle while a turn is in progress; the precedence, as each fact ends; that the sequence changes with each change of state or detail and not otherwise; Working's details, bounded; workers outside a turn leaving it idle with a detail; the bound on tool calls kept; a failure passing after its hold or with progress, a disconnection lasting until recovered, a closed session staying in error; the stall, once, from the last event or thinking again, and none while a tool runs or a request waits; and the deadlines.
- Test, through the hub, that everyone in a space is told the state through a turn from its real events: listening while someone records and is transcribed, thinking once their words are sent, working with the tool's name, speaking while speech is to come, waiting on a request, idle only once the turn and its speech are over, with the sequence raised; an error with its reason until it passes, the server's timer waking for it, a lost connection until the session is read again, and a closed session; the stall when the timer wakes; and that cancelled or failed words stop listening and undelivered input ends its turn.
- Test, in the protocol, that the state is sent only when given, read as none from an older server, and a state from a newer one reads as unknown.
- Test, in `tomori-backend-opencode`, that a deliberate cancel, interrupted or failed as aborted, ends the turn rather than reporting an error, for the space and its workers.
- Test, in `tomori-scene`, the smoothing, a run of brief states capped, and a new space starting afresh; the pill's label for each state and detail, with speech here over the least pressing; the swarm's mode for each state, with this device's speech, recording, transcription, failures and held reply layered on it, and the session's activity for an unknown state; the conversation's pill and turn following the server's state through a turn, smoothed, with no turn while it is busy whatever its session says; and that a watcher's frame, as the spectator draws it through the frame step, shows the server's state.
- On the headset, confirm that the pill and the swarm never show idle part way through a reply, through tool calls and sub-agents' builds, and that they do not flicker between sentences.
