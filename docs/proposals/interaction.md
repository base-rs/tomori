# Proposal: A Better Interaction Model Inside a Space

## Status

**Stage 1 is mostly built; stages 2 to 4 are mostly open.** November 2026. What was built is a requirement in `design/speech.md`, `design/spaces.md`, `design/interaction.md`, `design/live-audio.md` and others, which take precedence over this document. The rest of this document is the proposal from [PR #65](https://github.com/ioquatix/tomori/pull/65), kept for its reasoning; it was written before stage 1, and its audit describes `main` at that time.

**Built:**

- **Hearing it again** ([#67](https://github.com/ioquatix/tomori/pull/67)): the server keeps the latest two replies, a participant who was away or absent hears the latest from its start on returning, the right stick click replays and the left stick click mutes, the headset reports engagement from its session state and `XR_EXT_user_presence`, and the brief speech setting.
- **Seeing what went wrong** ([#68](https://github.com/ioquatix/tomori/pull/68)): B never denies, cancels or leaves without a hold; problems on a line of their own; the queue strip with cancel and undo; modes and speech chosen from a list with descriptions; questions answered by voice; and a world-anchored panel with lazy follow.
- **Attention across spaces** ([#66](https://github.com/ioquatix/tomori/pull/66)), from the companion proposal (`attention.md`, PR #62) that this one aligns with.
- **From the later stages:**
  - The controller-anchored interface (`controller-ui.md`, PR #89; [#104](https://github.com/ioquatix/tomori/pull/104)–[#106](https://github.com/ioquatix/tomori/pull/106)) replaced the panel with a status pill, and the conversation surface with a transcript on the right wrist with turns, Markdown-lite and request cards with a button per choice. Its status lists the later fixes.
  - Haptics and sound cues ([#107](https://github.com/ioquatix/tomori/pull/107), landed through #109).
  - Pointing inside a space: at a worker as a prototype ([#98](https://github.com/ioquatix/tomori/pull/98)), and shared attention stage 1, where what a participant points at as their words begin reaches the agent before them ([#175](https://github.com/ioquatix/tomori/pull/175), under [#81](https://github.com/ioquatix/tomori/issues/81)).
  - Unicode text from a glyph atlas ([#176](https://github.com/ioquatix/tomori/pull/176)).

**Decided since the proposal:**

- **Catch-up** (question 1): a returning participant hears the latest reply from its start and stays behind live speech until a pause, rather than jumping to live with a notice (`design/speech.md`).
- **Brief speech** (question 2) was made the default at first, then the user asked for replies in full by default ([#94](https://github.com/ioquatix/tomori/pull/94)); brief stays a space's choice. The agent's own instructions ask it to reply briefly instead ([#93](https://github.com/ioquatix/tomori/pull/93)).
- **Visitors** (questions 12 and 13) always hear live speech, never catch up, and may not ask for replays (`design/security.md`).
- **Engagement and attention.** The proposal suggested that the attention proposal's reply level should mean no participant was *engaged*. Instead, engagement decides only who hears speech live and who catches up: a participant who is away is still in the space, so what happens meanwhile raises no attention for them (`design/spaces.md`).
- **The panel** became the status pill and the wrist transcript rather than a world-anchored conversation surface (question 10), as the controller-anchored interface decided.

**Open:**

- **Stage 2:** structured speakers and tool calls, the timeline, per-message replay and a "while you were away" summary ([#190](https://github.com/ioquatix/tomori/issues/190)); avatars from presence ([#191](https://github.com/ioquatix/tomori/issues/191)); hold to confirm for persistent grants; capacitive labels on the controllers. No issue yet for the last two.
- **Stage 3:** reordering, merging and batching queued entries (question 3), push-to-talk for Tomori's own voice commands (question 5), hand-tracking pointing and poking, and per-participant "heard up to" cursors over the retained history. No issue yet.
- **Stage 4:** incremental history instead of full snapshots, paging beyond the 50 kept messages, and agent summaries on request. No issue yet.
- **Deferred from stage 1:** the stillness heuristic for presence, Android pause and resume, measuring how quickly the Quest reports removal, and speaking a list as its first items. No issue yet.
- **Questions not yet put to a decision:** the live microphone's control (4), approvals by voice (6), notifications off the headset (7), push-to-talk from the right controller (8) and a per-space speech rate (9).

This proposal covers being **inside** a space.

- **Attention.** Knowing which spaces and servers need you, and jumping to them, is covered by the attention proposal (PR #62, `docs/proposals/attention.md`). The two are aligned below rather than duplicated.
- **Security.** Who may connect and what they may do is covered by the `security` branch (PR #58, `design/security.md`): roles, pairing, knocking, and naming participants by public handle. This proposal adds no identity, authentication or permission UX of its own. Where presence, participants or joining meet roles, it follows that branch; see [Roles and security](#roles-and-security).

## The request

Spoken from the headset, November 2026:

> Review the current interaction UI. Right now the HUD is barely workable, the queued messages are extremely difficult to use, and replay of past messages is honestly not great. Propose something significantly better, including presence detection and so on. The 2D UX is honestly not great and we can do so much better.

Observations from the same day's use:

- **Coming back mid-reply.** The user came back to a space mid-reply and couldn't follow the speech without what came before. The intended fix: while a space is empty, keep its speech, and play it from the start of the latest reply when someone joins.
- **Truncation.** Panel lines are cut off at a fixed column width.
- **Confusing selection.** A once chose the ring's selected item while the user pointed at another object. That is now fixed so A chooses the pointed object, but it shows that two selection models were competing.
- **Hidden errors.** Errors are hidden behind "Sending…" status lines.
- **Mode changes.** The user changes modes from the headset.
- **Listening.** The user relies on hearing replies, and would like them to be briefer.

## Recommendation in brief

- **The problem in one sentence.** Everything in a space goes through one head-locked panel of 40 × 12 ASCII characters, driven by buttons whose meaning depends on hidden state. The panel shows one message, one page at a time. Spoken replies can't be heard again. Two of the bound controls do nothing. Errors, the queue and the conversation compete for one status line.
- **MVP: "hear it again, and see what went wrong".** This is mostly client work, plus a few bounded server additions:
  1. **Keep and replay the latest reply.** The server keeps its audio. Pressing the right stick replays it. A participant who wasn't listening, because they were absent or had taken the headset off, hears the reply from its start when they return. This builds on the plan you already have.
  2. **Basic presence.** The headset reports when it is worn and focused. "Nobody listening" then counts removed headsets as away, and the live microphone stops when the headset is taken off.
  3. **Briefer speech.** An opt-in *brief* speech setting per space speaks the first part of a reply, then says there is more to read.
  4. **Errors that stay visible.** Errors get their own line instead of losing to "Sending…". The queue gets its own strip, and B stops meaning "leave the space".
  5. **Answer questions by voice.** The server already supports spoken answers; the headset tells you to use the PC instead.
  6. **A panel that stays put.** The panel stays in the world and follows you lazily, instead of being fixed to your head. Its text wraps instead of being cut off. Modes are chosen from a visible list with their descriptions, instead of being cycled blind.
- **Later stages.**
  - **Stage 2:** a world-anchored conversation surface with scrollable turns, per-message replay, a timeline of events, spatial approvals, a mode dial, a "while you were away" card, and other participants shown as avatars.
  - **Stage 3:** queued input as tangible chips that can be reordered, merged and undone, a separate push-to-talk for Tomori voice commands ("repeat that", "what did I miss"), and hand tracking.
  - **Stage 4:** incremental history instead of full snapshots, longer history, summaries written by the agent on request, and notifications off the headset.

## Audit: what the headset does today

Everything below is from `origin/main` at `5d1e7c6`.

### The panel and its layout

- **Size and font.** The panel is 40 columns by 12 rows (`COLUMNS`, `ROWS`, `crates/client-quest/src/caption.rs:10`). It uses the 8×8 `font8x8` bitmap font at 3.4 mm per font pixel (`PIXEL`).
  - **ASCII only.** Every non-ASCII character becomes `?` (`caption.rs:384`), apart from curly quotes and dashes. Emoji, accents, arrows and box drawing, all common in agent replies, show as `?`.
- **Head-locked.** The panel is drawn at `head * translate(0, -0.32, -1.7)` (`caption.rs:171`), with no depth test, over everything else.
  - It moves with every head movement. You can't look away from it, look past it at the swarm or a worker station, or bring it closer to read.
  - Head-locked text is the classic VR comfort problem: it swims against the world, and it is drawn on top of world objects even when they are nearer, which makes the eyes converge at the wrong depth.
  - `design/spectator.md` already treats it as an obstruction to keep out of the spectator view.
- **Fixed rows** (`Conversation::space_text`, `crates/client-quest/src/conversation.rs:1668`). Of the 12 rows, only 8 show content (`BODY_ROWS = ROWS - 4`, `conversation.rs:24`):

  ```
  row 0   [activity | mode] title           <- cut to 40 columns
  row 1   status                            <- one line, many competitors
  row 2   p1/3 ASSISTANT 7/12               <- page and item counters
  rows 3-10  8 lines of one message
  row 11  footer hint                       <- one line of button hints
  ```

  - So a page holds about 320 characters. A typical 1,500-character reply is five pages, each a separate stick flick down. Moving to the next message is a flick sideways, which resets to page 1 (`navigate`, `conversation.rs:767`).
  - The 50 messages the server keeps (`MESSAGE_LIMIT`, `crates/server/src/space.rs:14`) are reached one at a time, with no overview, timestamps or speaker names. User messages show the attribution prefix baked into the delivered text (`attributed`, `space.rs:135`).
- **Truncation everywhere.** Each header, status and footer line goes through `short()` (`conversation.rs:1859`), which joins lines and cuts the result to 40 characters with `...`. For example, `Queued: <your words>` and `ERROR: <reason>` keep only their first 30 or so characters, exactly where the useful part usually is.
- **Raw Markdown.** Messages are the agent's Markdown as streamed (`MessageModel::display`, `space.rs:90`), so `**`, `#`, backticks and code fences show as literal characters.
  - This is despite the server already parsing the same text into document-level blocks for speech (`design/speech.md`).
  - Tool activity appears only as a placeholder such as `[Bash: working]` until the message has text.
- **Following a streaming reply.** While it follows new messages, the panel jumps to the *last* page of the newest message (`conversation.rs:547`). The beginning of a long streaming reply scrolls out of sight as it is written.
- **Not shown in arrival.** The panel is hidden there (`xr.rs:485`), because arrival uses world-anchored labels. So the app already has two text systems with different rules.
- **A third renderer goes unused.** A world-space card renderer with depth-sorted, occludable labels already exists for worker stations and result cards (`crates/client-quest/src/workspace_graphics.rs`, 44 × 18 cards). But result cards are never populated: the server always sends `presentations: Vec::new()` (`space.rs:634`), `Workspace::from_space` does the same (`delegation.rs:235`), and right-stick card navigation is driven by `Workspace::default()` (`xr.rs:332`).

### Status lines

The status row has a strict priority (`conversation.rs:1718–1757`):

1. the push-to-talk state (`MIC AGENT: …`, `Sending: …`);
2. a client error, then the connection state, then a notice;
3. the space's error;
4. a missing environment;
5. your newest queued entry;
6. the microphone, queue and participant counts.

`MIC LIVE |` is then prepended to whatever wins.

- **Errors hidden behind "Sending…".** While an utterance is `Talk::Sending`, the line shows `Sending: …` and any `self.error` is not shown (`let status = talking.or_else(|| self.status())`, `conversation.rs:1730`).
  - For example, an earlier utterance's `TranscriptFailed` or a refused decision arrives and is stored, but stays invisible until the transcript returns.
  - If the transcript never returns, as when the connection drops while sending, the notice replaces it instead.
- **The space error is sticky and wins over the queue.**
  - `SpaceModel::error` is set by backend errors, refused modes, failed delivery and closing (`space.rs:394–409`, `:507`), and nothing clears it, as the attention proposal also found.
  - Once set, `ERROR: …` takes the status row permanently and hides `Queued: …` and the participant count. A refused mode change an hour ago still owns the line.
- **One line for everything.** Recording, sending, connection, problem, queue, live microphone and presence share 40 characters. Nothing shows *how long ago* something happened.

### The input queue

- **Hidden.** The queue isn't on screen. It is a pseudo-message "one past the last message" (`items`, `conversation.rs:1819`; `body`, `:1649`), reached by flicking right past every message. It renders as `speaker [state]: text` lines inside the 8-row body.
- **Your newest entry only.** The status row shows only your newest queued entry, cut to 40 columns (`:1739`).
- **B is overloaded** (`Conversation::back`, `conversation.rs:926`). In order, B:
  1. denies a pending approval;
  2. waits, while a decision is being sent;
  3. cancels **your newest** queued entry;
  4. otherwise **leaves the space**.

  Consequences:
  - You can't cancel an older entry, or anyone else's, although `design/spaces.md` lets any participant cancel any entry.
  - Pressing B once too often, or a moment after the entry is sent, leaves the space.
  - An approval that arrives just before a B meant for the queue is **denied**. A press intended as "never mind" becomes a decision on a tool call you haven't read.
- **No reordering, editing, merging or "send next".** The server queue is strictly first in, first out (`InputQueue::begin_next`, `crates/server/src/queue.rs:89`), and the protocol has only `CancelQueued`.
  - Each queued utterance becomes **its own agent turn**. "Also, check the tests" spoken while the agent works becomes a separate turn after the first finishes, rather than an amendment.
  - Finished entries linger: up to 8 are kept (`FINISHED_LIMIT`, `queue.rs:9`). Failed ones show only as `[failed]`, with the reason in the sticky space error.

### Message history and replay

- **No replay.** The right-stick click is bound to "Replay spoken reply" and the left-stick click to "Toggle spoken replies" (`xr.rs:964–965`, `:1013–1017`), but both are discarded: `let _ = (controls.sound_toggle, controls.replay);` (`xr.rs:329`, commented "Spoken replies return with server playback").
- **Speech isn't kept.**
  - The server synthesizes each reply once. It paces the audio "to whoever is in the space at that moment" (`Hub::agent_speech`, `Hub::agent_audio`, `crates/server/src/hub.rs:1671–1699`), and keeps none of it.
  - A space nobody is in speaks to no one.
  - A participant who joins mid-segment hears it from that point (`design/live-audio.md`), and "existing history is not read aloud when joining" (`design/speech.md`). That is exactly the mid-reply confusion you hit.
- **Spoken text is thrown away.** `ServerMessage::Speech` carries the segment's message id and text, but the headset only logs it (`conversation.rs:500`). So the panel can't show which sentence is being spoken, or which message a sound belongs to.
- **Local mute exists but isn't wired.** `Mixer::set_agent_muted` and `Mixer::source_level` exist (`crates/client/src/playback/mixer.rs:109`, `:140`), but `PlayerControl` doesn't expose them (`crates/client/src/playback/player.rs`). The headset can't mute the agent locally, as `design/speech.md` allows, or show who is speaking, as `design/live-audio.md` describes.
- **The swarm doesn't show speech.**
  - The swarm's signals have `Speaking`, `Listening` and `Transcribing` modes (`crates/client-quest/src/interaction.rs`).
  - But the headset feeds them only session activity (`xr.rs:364–374`, `..Default::default()`).
  - So it never shows that the agent is speaking or that you are being heard.
- **Missing message facts.** Messages carry no timestamp, no structured speaker (only the baked-in `Ada:` prefix), and no record of whether they were spoken. There are no non-message events either: who allowed what, mode changes, workers finishing, people joining.
- **A closed enum.** `Role` is a closed enum without `#[serde(other)]` (`crates/protocol/src/space.rs:138`). Adding an "event" role would break older clients, so a timeline needs its own field.

### Approvals, questions and standing options

- **Approvals.**
  - The approval replaces the conversation in the body. You can't page back to see what led to it: horizontal navigation is disabled while a request is presented (`conversation.rs:787`).
  - The right stick cycles `1/n: Allow once` and the standing options (`choose`, `:805`), A confirms, and B denies.
  - Deny isn't among the visible choices, and B is the same button that otherwise cancels and leaves.
  - The server also writes "A: allow once. B: deny." into the approval text (`crates/server/src/projection.rs:35`). That is button text authored on the server, which assumes the headset's mapping.
- **Standing options** are shown in full, which matters because the label is the whole of what is granted (`design/agent-backends.md`). But they are squeezed into the 8 body rows, and a long one is cut to "… more" (`conversation.rs:1686–1691`).
  - Nothing distinguishes a one-off allow from a persistent grant written to project or user settings.
  - Both are one A press away.
- **Questions can't be answered on the headset.**
  - The headset appends "Answer this question on the PC." (`conversation.rs:1636`).
  - Yet the server's projection says "Hold X and say your answer or its number." (`projection.rs:208`), and the protocol and server fully support `ClientMessage::Answer` (`hub.rs:1132`).
  - Worse, an utterance spoken while a question is pending is **queued as a new prompt** (`Hub::transcript_finished` always enqueues, `hub.rs:1322–1401`). It then waits behind the very question it was meant to answer.
- **No sense of who.** Nothing says who decided a request, or that another participant is about to.

### Mode switching

- **Blind cycling.** Y steps to the next mode in the agent's order (`refresh`, `conversation.rs:974`; `SpaceState::next_mode`).
  - Going from *default* to *auto* in Claude Code is three presses, each a round trip, with no preview of where the next press lands.
  - Mode descriptions are sent (`protocol::Mode::description`) and never shown.
- **Little feedback.** The current mode appears only inside the cut-down title row. A refused mode becomes the sticky space error. Nobody is told who changed the mode.
- **Y has three meanings.** In a space whose agent offers no modes, Y re-joins the space (`:992–996`). In the lobby it lists again, and on the server list it rescans. The readme still says Y "fetches the space again" in a space (`readme.md:125`).

### Workers

- **What works.** Worker stations are real 3D objects (`crates/client-quest/src/delegation.rs`) with world-anchored, occludable labels. This is the most spatial part of the space today, and it works.
- **What doesn't.**
  - You can't point at a station: the ray works only in arrival (`xr.rs:449`).
  - A worker has only a title and a state (`protocol::Worker`), so there is no "what is it doing" or "what did it produce".
  - A worker's approvals still have to be answered in OpenCode (`readme.md`, workers section).
  - The readme still says the headset doesn't show workers.

### Control mapping and discoverability

| Control | Lobby | In a space |
|---|---|---|
| Right trigger | point and choose | attract the swarm only (no pointing) |
| A | choose highlighted / accept | confirm approval option; **otherwise reset the swarm** (`xr.rs:318`) |
| B | dismiss / back / to servers | deny / cancel newest own entry / **leave** |
| X (hold) | "Join a space to talk" | push-to-talk |
| Y | list again / rescan | **cycle mode**, or re-join |
| Left stick | select | page (up/down), message (left/right) |
| Right stick | (none) | approval option, else (empty) result cards |
| Left grip | (none) | toggle live microphone |
| Left-stick click | (none) | bound to "toggle spoken replies", **does nothing** |
| Right-stick click | (none) | bound to "replay", **does nothing** |

- **Meanings hidden in state.** The same press does different things depending on state you can't see at a glance: whether an approval is pending or a decision is in flight, whether you have an entry queued, which ring is open.
- **Hard to discover.** The only guide is the single footer line, cut to 40 characters, plus the readme.
- **Unused hardware.** No haptics are used anywhere (no haptic output action in `xr.rs`), and there are no audio cues (earcons) for state changes. The controllers' capacitive touch sensors, which could reveal a button's meaning when the thumb rests on it, are unused.
- **Grip as a toggle.** Toggling the live microphone with the grip is easy to trigger by accident, because people squeeze the grip to hold the controller. The only feedback is `MIC LIVE |` prepended to a cut-down status line.
- **Two selection models.** The arrival bug you found came from the stick selection and the ray highlight competing. `press_a` now prefers the highlight (`conversation.rs:815`), but inside a space there is no pointing at all, so the stick model is the only one there.

### Error surfacing

- **Client-side errors** (`self.error`) are one string, replaced by the next error and cleared by any send (`Conversation::send`, `conversation.rs:584`). An error is therefore wiped by the very next action, often before it was read.
- **Space errors** are sticky and never cleared (above).
- **Speech failures** are sent as per-participant `Error` messages (`Hub::speech_failed`, `hub.rs:1703`). They enter the same `self.error` slot as a refused join.
- **No severity, time or source.** There is no audio or haptic cue for an error, which matters for someone listening rather than reading.

### Presence

- **Poses are sent, not shown.**
  - The headset sends its head, controller and hand poses every frame (`connection.send_presence`, `xr.rs:469`), and the server relays them to the space (`hub.rs:1165–1189`).
  - The headset **ignores** other participants' presence ("Presence comes with the presence work", `conversation.rs:504`). Only the spectator renders it (`Connection::watch_presence`).
  - Inside a space you see no one else; the only sign of them is the count `N here`.
- **Session state.**
  - The headset tracks OpenXR session state (`xr.rs:201–235`). Losing focus stops live audio and cancels a held recording, as `design/live-audio.md` requires.
  - Nothing tells the server. A headset on the desk, or one showing the system menu, counts as a present participant and receives the reply into the void.
  - The attention proposal lists this as a risk: "being present is not paying attention".
- **Android lifecycle.** Of the Android lifecycle events, only `Destroy` is handled (`crates/client-quest/src/lib.rs:195`).
- **No presence extension.** `XR_EXT_user_presence` is not enabled, although openxr 0.22, which the headset uses, exposes it (`ExtensionSet::ext_user_presence`, `Event::UserPresenceChangedEXT`).

### What is good, and kept

The arrival scene, pointing with visible feedback, the swarm's activity states, the worker stations, push-to-talk, the shared server-side state and the "first decision wins" rule are sound. The proposal builds on them. In particular, the arrival code's rings of pointable, labelled objects (`arrival.rs`, `arrival_motion.rs`, `pointing.rs`) are reusable for choices inside a space.

## 1. Presence detection

### What the headset can know

| Signal | Source | Meaning | Reliability |
|---|---|---|---|
| Session `FOCUSED` | OpenXR session state, already observed (`xr.rs:201`) | worn, and Tomori has input | good |
| Session `VISIBLE` | OpenXR | worn, but a system menu or overlay has input | good |
| Session `SYNCHRONIZED`, `IDLE` or `STOPPING` | OpenXR; the spec says a runtime with an engagement sensor should leave the running states when the user disengages | removed, or the headset sleeping | depends on the runtime, and the delay is unknown |
| `XrEventDataUserPresenceChangedEXT` | `XR_EXT_user_presence`, from the proximity sensor | worn or removed, directly | Meta's runtime advertises it. A Meta developer feedback report says the "removed" event sometimes doesn't arrive on the Quest 3S, so treat it as a hint. |
| Head pose unchanged for N seconds, with no input | heuristic over `locate_views` | probably set down | weak; a still listener looks the same |
| Head direction towards the conversation surface | head pose | looking at it | good as a cone; this is *attention*, not reading |
| Eye gaze | `XR_EXT_eye_gaze_interaction` | where the eyes point | **not available on the Quest 3**, which has no eye tracking. Quest Pro only. |
| Android `onPause` / `onStop` | `android-activity` `MainEvent`s, not handled today | the app is backgrounded | good, coarse |
| Connection | already known to the server | connected or not | good |

**Recommendation.**

- **One state machine, in the client core.** The headset derives one *engagement* state from these signals, in a small, render-free state machine with tests, as `design/clients.md` wants for the shared model:

  ```
                 user present + FOCUSED
      +--------------------------------------------+
      v                                            |
  [engaged] --VISIBLE / menu--> [distracted] --+   |
      |                              |         |   |
      | removed / IDLE / STOPPING /  |         |   |
      | onPause / still >120 s       v         |   |
      +-------------------------> [away] <-----+   |
                                     |             |
                                     +-------------+
  ```

  - **engaged:** worn, focused and moving.
  - **distracted:** worn, but a system overlay has focus. It is brief, so it doesn't start a catch-up.
  - **away:** removed, asleep or backgrounded. The stillness heuristic only *demotes* to away after a long quiet, two minutes say, and any input or movement restores engaged at once.
- **Measure first.** The actual Quest 3 behaviour should be measured before relying on it, by logging the transitions at `info` (they are lifecycle events under `design/client-logging.md`) across removing, putting on, the system menu and sleep. Record which signal arrives, and after how long.
- **The desktop client** reports engaged while its window has focus, and away otherwise.
- **The spectator** reports nothing. It follows live speech, and under `design/security.md` it is a visitor.

### Telling the server

- **Message.** A new `ClientMessage::Engagement { state }`, with `engaged | distracted | away` and `#[serde(other)] unknown`. It is sent on change only; it is not part of the per-frame pose message.
- **Shown to everyone.** The server records it per participant and shows it in `SpaceState.participants` (a new `engagement` field), so everyone sees who is away.
- **Older clients** never send it and count as engaged, which is today's behaviour.
- **Alignment with the `security` branch.**
  - Participants are named by their public handle (`source`), as that branch requires, never by their Hello identity. The new field sits on `Participant { name, source }` as that branch reshapes it.
  - Visitors send no presence there, so they send no engagement either, and the server ignores one from a visitor's connection under that branch's single role check.
  - A collaborator reports engagement only in the space they were admitted to, because elsewhere they are a visitor.

### How presence drives behaviour

- **Speech is kept for the people who missed it.**
  - This generalises your "keep speech while the space is empty" plan to "keep speech for any participant who wasn't engaged while it played".
  - The server keeps synthesizing, as today, and *retains* the latest reply's audio.
  - Whoever is listening live hears it live as now: engaged participants, visitors (under the `security` branch they hear speech but are not participants) and the spectator feed.
  - A participant who joins, or becomes engaged again, hears the latest reply they didn't hear **from its start**, then continues live.
  - See [Hearing it again](#hearing-it-again-catch-up-and-replay) for how each participant gets their own position in the retained audio.
- **The live microphone.** It turns off when you go away (removed or asleep), not only on focus loss, and stays off until toggled again, as today.
- **Push-to-talk.** A held recording is cancelled on away, as on focus loss today.
- **Catching up on return.** When you come back, the conversation surface shows a short **"while you were away"** card, then the latest reply plays from its start. The card is built from facts the server already has, with no model involved:
  - "2 replies" and "the agent finished";
  - "Approval waiting: Bash(npm test)";
  - "Ada joined" and "Ada switched to plan mode";
  - "worker *Refactor parser* completed";
  - "1 problem: could not deliver input".
- **Others see it.** Avatars of away participants are dimmed, and the count reads "2 here, 1 away".
- **Attention (PR #62).**
  - The "reply" level should mean "finished while no participant was **engaged**", not merely "while nobody was in the space". This closes the risk that proposal lists.
  - Later, the per-participant "heard up to" cursor from stage 3 provides its "for me" stage.
- **Notifying.** A removed headset can't show anything. The options, in order of cost:
  1. the attention badges of PR #62 on other clients;
  2. a chime on the headset if it is still awake;
  3. a notification on the PC, such as a desktop notification from the server, or a push service.

  The third option is outward-facing, so it is an open question rather than a recommendation.
- **Focus on the surface (stage 3).**
  - A dwell of about a second with the head pointed at a message marks it seen. This only adjusts the catch-up ("you saw this, but didn't hear it").
  - It is never used to decide anything, and never sent to the agent.

### Showing other participants (stage 2)

- **Avatars.** Render the relayed `Presence` as a simple head shape and hands or controllers, in a participant colour distinct from the swarm. `design/arrival.md` already uses distinct cubes for people.
- **Name and state.** A world-anchored nameplate shows the name and engagement.
- **Who is speaking.** A ring that pulses with the level of their audio, taken from `Mixer::source_level` as `design/live-audio.md` describes: measured from what actually plays, so a muted agent still shows as speaking.
- **Push-to-talk marker.** A small "to the agent" marker while someone holds push-to-talk.
- **The spectator** already renders presence. The headset should share that code through the shared scene model rather than duplicate it.
- **Only participants.** Avatars are only for participants. Visitors send no presence (`design/security.md`), so they have no avatar.
  - That branch's long-term "people and access" interface includes showing how many visitors are watching.
  - The status line is a natural place for that count, and avatars are a natural place to attach that interface later. Neither is designed here.

### Roles and security

This proposal follows the `security` branch rather than adding its own rules. Where they meet:

- **Hiding actions by role.** The headset hides what the role doesn't permit, as that branch requires, using the role it is told on Hello and on every change. For a **visitor**:
  - the conversation surface, timeline, avatars, status and catch-up card are shown read-only;
  - the queue chips, prompt cards, mode ring, push-to-talk, the live microphone and voice commands are hidden;
  - in their place is that branch's knock object.
- **Knocks are prompt cards.** A knock is shown to participants as a prompt card, the same kind of spatial prompt as an approval: admitted or declined by pointing and the trigger. This matches that branch's "an object in the space, admitted or declined with the trigger".
  - As with approvals, B never declines a knock.
  - The timeline records who admitted or declined it.
- **Authentication notices stay with the server.** Pairing codes, "no longer admitted" and "the server's identity has changed" stay on the server's object in arrival, as that branch places them. They never appear on the conversation surface's problem channel.
  - Inside a space, a change of role, such as being admitted, is a timeline entry and a status change, with no new interface.
- **Names.** Timeline actors, queue chips and avatars name people by their public handle and display name, as that branch requires.
- **Replay and catch-up** are listening, not acting.
  - Visitors already receive the live stream.
  - Whether visitors may also *request* a replay, which costs the server a stream per request, is a question for that work. See the open questions.

## 2. The interaction model

The aim is a space you can **look around** to understand, and **point at** to act on. Text is kept for what must be read, and anchored in the world. Nothing important depends on a button whose meaning is hidden.

### Layout

Seen from above, with you at the bottom:

```
                     worker stations (existing arc)
            .   [w1]      [w2]      [w3]      [w4]   .

                          ( swarm )            <- agent; status halo
                         [ status ]            <- glanceable status, above swarm

  +---------------------------------+     +-------------------+
  | CONVERSATION SURFACE            |     | PROMPT            |
  |  10:02 You: run the tests     ok|     |  Bash             |
  |  10:02 Agent: All 214 pass. [>] |     |  npm test         |
  |  10:05 Ada: and lint it       ok|     | [Allow once]      |
  |  10:05 Agent: Lint found 3 ... >|     | [Always: npm *]   |
  |   ^ speaking now (highlighted)  |     | [Deny]            |
  +---------------------------------+     +-------------------+
          ~1.5 m, 20-30 deg left               ~1.2 m, right

           [ queue tray: chips on a shelf, within reach ]
             (You: also check CI)  (Ada: then deploy)

                                you
                     wrist: mic | mode | link
```

- **The conversation surface.** A world-anchored panel about 1.5 m away, slightly to one side, so the swarm stays visible ahead.
  - **Lazy follow.** It stays where it is while you look around. It glides back into view only when your body has turned more than about 45° for more than a second, easing over half a second, so it never swims with your head.
  - **Your own copy.** Each participant has their own instance. History is shared state, while scrolling is a local choice, which `design/spaces.md` allows ("local presentation choices such as mute or paging").
- **The prompt card.** It appears only while an approval or question waits, beside the surface, and doesn't replace it, so the context stays readable.
- **The queue tray.** A low shelf of chips within arm's reach, which also suits hand tracking.
- **Glanceable status.**
  - The swarm's halo keeps showing activity.
  - A small status line sits above the swarm: mode, "speaking", "listening", queue count.
  - A wrist readout on the left controller shows your microphone state (off, live or to the agent), the mode and the connection.
  - Only an alert pip at the edge of your view is head-locked, and only when something needs you while you're looking elsewhere, such as an approval while you face the workers.

### The conversation surface

- **Turns, not pages.** Each message is a row with its time, speaker and a one-line excerpt, newest at the bottom. Pointing at a row expands it in place, with the full text wrapped.
  - The left stick scrolls smoothly by velocity, with no page jumps.
  - A new message arrives at the bottom with a "new below" marker. It doesn't yank the view if you have scrolled up.
- **Formatted text.** Render Markdown structure: headings, lists, emphasis, and code in a monospace block with a different tint. Unicode comes from a signed-distance-field (SDF) font atlas, so emoji, accents and box drawing are no longer `?`.
  - The server already splits replies into document-level blocks for speech, so in stage 4 it can send them as blocks, and the client wouldn't need its own Markdown parser.
- **Speaking now.** Using `ServerMessage::Speech { message, text }`, the row being spoken is highlighted, along with the block within it. These are captions for free: hard of hearing, or in a noisy room, you can read along.
- **Per-message replay.** Each spoken reply has a replay control. While it is playing, the same control stops it.
- **The timeline.** Events appear between messages in a quieter style:
  - "Ada allowed Bash(npm test) once";
  - "Mode → plan (you)";
  - "worker *Lint* failed";
  - "could not deliver input: …" (with a problem marker).
- **Tool activity** shows as a compact chip on the agent's row ("Bash ✓, Edit …"), not as the row's whole text.

### Hearing it again: catch-up and replay

- **Retained audio.**
  - The server keeps each space's synthesized audio per assistant message, in a bounded ring.
  - **MVP:** only the latest reply.
  - **Stage 2:** the last few replies, within a byte budget per space.
  - Kokoro produces 24 kHz mono 16-bit audio, about 48 KB/s, so 10 minutes is about 29 MB. Encoding retained audio with Opus later cuts that roughly tenfold.
  - Older replies are re-synthesized from their text on demand, at a lower priority than live speech.
- **Replay is per participant.**
  - `ClientMessage::Replay { message }` streams that message's retained audio to **you only**, on a separate source, `REPLAY_SOURCE`. Your mixer can tell it from live speech.
  - `ClientMessage::StopReplay` stops it.
  - Other participants are unaffected. "Replay for everyone" is a separate, explicit choice.
- **Live speech wins.** If new live speech starts while you replay, the replay fades out and the row is marked "replay interrupted". Agent speech is never interrupted by participant input (`design/speech.md`); a replay is local listening, not input.
- **Catch-up on join or return.**
  - Each participant has a *speech cursor* into the space's retained audio. Engaged participants follow the live edge, as do visitors and the spectator feed.
  - A participant who is away keeps the cursor from when they left.
  - Someone joining or returning starts at the beginning of the latest unheard reply, and is paced from their own cursor until they reach the end of the retained audio. They then follow live speech.
  - Nobody hears the same reply twice at once: frames from the live edge aren't also sent to a participant who is catching up.
  - While a reply is still streaming, a late listener stays a little behind live, and can "jump to live".

  This replaces today's "join mid-segment, hear from that point".
- **MVP scope.** In the MVP, cursors cover only the latest reply. Stage 3 extends them to retained history.
- **Brief speech.**
  - A per-space shared setting, `speech: full | brief | off`.
  - With **brief**, the server speaks the first block or blocks, up to about 20 seconds of audio, then a short "More on the panel." The rest is retained and can be played with "continue".
  - A list is spoken as its first three items, then "and four more".
  - This is a speech-policy choice made on the server, and needs no change to the agent.
  - An opt-in *prompt hint* ("your reply will be spoken; be concise") is possible too. It changes the text delivered to the agent, which `design/spaces.md` specifies exactly today, so it is an open question rather than a recommendation.

### Queued input as tangible objects (stage 3; MVP is a simple strip)

- **One chip per entry.** Each queued entry is a chip on the tray, showing the speaker's colour, a one-line excerpt, its state and its age.
  - **Your own.** Your chips sit nearest you.
  - **Failed.** Failed chips are red, with their reason on the chip.
  - **Sent.** Sent chips fly to the swarm and fade, which is a clear "it went".
- **Actions**, by pointing and trigger, or grabbing with the grip or a pinch:
  - **Cancel.** Flick the chip away, or point and press a cancel button. An undo is offered for three seconds; it is client-side, sending `CancelQueued` only when the time is up. If the entry starts sending meanwhile, the undo turns into "already sent".
  - **Reorder.** Drag a chip before another: `ClientMessage::MoveQueued { entry, before }`.
  - **Merge.** Drop one chip on another to send them as one turn, with attribution per line.
    - Optionally, a per-space *batch* setting delivers everything queued as one prompt when the agent goes idle.
    - Batching fixes "each afterthought is its own turn".
  - **Redo.** Hold X on your chip to record a replacement for it. This is the headset's edit, since there is no keyboard.
- **B never cancels or leaves.** It closes or backs out of whatever is focused, and nothing else.

### Approvals and questions as spatial prompts

- **The prompt card.**
  - It slides in beside the surface with a soft chime and a haptic pulse on both controllers. It never replaces the conversation.
  - It shows the tool and its full input in monospace, and "1 of 3 waiting" when several are pending.
- **One button per choice.** *Allow once*, each standing option with its full label, and *Deny*.
  - **Choosing.** Each is chosen by pointing and pulling the trigger, or with the stick plus A when the card is focused.
  - **Persistent grants look different.** A standing option that writes settings has a distinct border and needs **hold to confirm**: hold the trigger for about 0.6 seconds while a ring fills.
  - **No deny by accident.** B does nothing to the request.
- **Who is deciding.**
  - While another participant points at a choice, the card shows "Ada is choosing". The first decision still wins (`design/spaces.md`).
  - Afterwards, the timeline says who decided what.
- **Questions.**
  - Each option is a pointable button.
  - Holding X while the question card is focused **answers it**: the transcript goes to `Answer`, not the queue. The card shows "Answering: …" with the words heard, then the server's acceptance or its error, such as "That isn't one of the options".
  - Multi-field questions step through their fields on the card.
  - A field Tomori can't support says so, with "Answer on the PC".

### Mode switching

- **Choosing a mode.** The mode is shown at the top of the status line and on the wrist.
  - Pointing at it, or pressing its control, opens a small ring of every offered mode, each with its description, reusing the arrival ring code. You choose the one you want, sending `SetMode` with that mode directly; the server already accepts any offered mode.
  - Blind cycling is gone.
- **Feedback.** A change made by anyone is shown on the timeline, with who made it, and spoken as a short earcon.
- **Risky modes.** A mode that removes approvals, such as Claude Code's bypass mode where it is offered, needs hold to confirm.

### Workers

- **Pointable stations.** Pointing at a station shows its full title and state, how long it has been in that state, and in stage 3, its last message excerpt where the backend reports one.
- **Waiting workers.** A worker waiting on a permission shows the same prompt card, anchored at its station, once worker approvals reach Tomori.
- **Result cards.** The unused card renderer can show presentation cards once the server populates them. That is a separate decision (`design/scene-modules.md`).

### Voice commands (stage 3)

- **A separate channel.** Commands to *Tomori* go on their own channel, so they never reach the agent and no prompt is ever eaten by a keyword guess.
  - **Hold Y to command** (once Y no longer cycles modes), and hold X to talk to the agent.
  - The server recognises a small fixed grammar on that utterance's final transcript, and refuses anything else with "I didn't catch a command".
- **Commands:**
  - "repeat that" or "replay"; "what did I miss" (plays the catch-up card as speech, in a distinct system voice); "stop" or "quiet"; "continue";
  - "jump to live";
  - "cancel my last" or "cancel all mine";
  - "send it now" (merge the queue);
  - "switch to plan mode";
  - "who's here".
- **Approvals by voice** are possible ("allow once"), but deserve care. They come last, and need an explicit spoken confirmation. That is an open question.
- **Why not a wake word.** A wake word ("Tomori, repeat that") in the agent channel was considered and **not recommended**:
  - a false positive silently removes your words from the agent;
  - a false negative sends "repeat that" to the agent as a prompt.

### Controls

Recommended mapping in a space. Arrival is unchanged.

| Control | Meaning in a space |
|---|---|
| Right trigger | point and choose: surface rows, chips, prompt buttons, mode, stations, avatars; attract the swarm when pointing at nothing |
| A | confirm the focused item (the highlight wins, as in arrival) |
| B | close or back out of what is open; **never** deny, cancel or leave |
| X (hold) | push-to-talk to the agent; answers a focused question |
| Y (hold) | voice command to Tomori (stage 3; in the MVP, Y opens the mode ring) |
| Left stick | scroll the surface (up/down), previous/next turn (left/right) |
| Right stick | unused in a space (snap turn, if wanted) |
| Right-stick click | replay or stop the latest reply |
| Left-stick click | mute or unmute agent speech locally |
| Left grip | live microphone (kept; see the open questions), with a haptic tick and an earcon |
| Left menu button | help: labels on every button for 5 seconds |

- **Leaving the space.**
  - Point at the space's exit object, which is near the status line, or hold B for a second while a ring fills.
  - Either plays the existing leave transition.
- **Discoverability.**
  - **Labels on touch.** Resting the thumb on a button, using the controllers' capacitive `…/touch` inputs, shows a small label anchored *at that button* on the rendered controller model, saying what it does **now**. The labels fade after a moment.
  - The footer line goes away.
- **Haptics.** A short pulse when:
  - push-to-talk starts and stops;
  - the live microphone toggles;
  - an approval arrives;
  - a choice is made.
- **Earcons.** Short, distinct tones for:
  - a transcript queued;
  - a transcript failed;
  - a reply starting;
  - an error;
  - a mode change.

  They are generated locally, so they work even when the connection is the problem.

### Errors

Errors are split into three channels, so none can hide another:

1. **Blocking prompts:** approvals and questions, on the prompt card.
2. **Problems:** errors, as timeline entries with a red marker. The newest problem also appears on the status line until dismissed or superseded.
   - Each has a time, a source (your words, delivery, speech, the agent, the connection) and the full text.
   - Problems are never overwritten by transient status. A problem with your own words also plays the error earcon.
3. **Transient status:** "Sending…", "Connecting (attempt 3)…". These are small, on their own line, and never take a problem's place.

On the server, the sticky `SpaceState::error` keeps only *ongoing conditions*: closed, or the backend connection lost. It is cleared when the condition ends. One-off failures, such as a refused mode or failed delivery, become timeline entries instead. This also fixes the noise noted in the attention proposal.

### Hand tracking (stage 3)

`design/arrival.md` defers gestures. The model above is built so that they can arrive without redesign:

- **Point and select.** A ray from the hand's aim, either `XR_FB_hand_tracking_aim` or derived from the joints, with a pinch to choose.
- **Near objects.** The surface rows, queue chips and prompt buttons are big enough to **poke** when within reach. The tray is placed within reach for that reason.
- **Push-to-talk without a controller.** Pinch and hold a microphone button on the wrist.
- **Haptics.** Hands get no haptics, so they rely more on earcons and visual confirmation.
- **Fallbacks.** Every action has a pointing path and a controller path. No action needs a gesture alone.

### Legibility, comfort and accessibility

- **Text size.** Size body text by visual angle, not by pixels: roughly 1° to 1.5° of cap height at the reading distance, to be tested on the headset. The 8×8 font at 3.4 mm per pixel, at 1.7 m, is about 0.9° tall.
  - A comfortable size setting (S, M, L) scales the surface, rather than adding columns.
  - Use SDF glyphs with antialiasing, and a dark, slightly translucent backing with high contrast.
- **Line length.** Keep lines to about 50–60 characters, wrapped at words, never cut. Long code lines scroll horizontally inside their block.
- **No head-locked text** apart from the small alert pip at the edge of your view. The surface moves only by lazy follow, with eased motion, and never while you are reading it. Nothing moves the camera.
- **Colour is never the only signal.** Every state has a shape or icon too:
  - queued is hollow, sending is half-filled, sent is a tick, failed is a cross;
  - the halo states keep their shapes.
- **Seated, one-handed and reach.**
  - Everything can be reached within ±35° of straight ahead while seated.
  - Push-to-talk can also be held on the right controller, for one-handed use; open question.
  - The tray height adjusts to your head height.
- **Audio-first use.** Captions follow speech, brief speech is available, catch-up is spoken on request, and errors have earcons. Someone who isn't looking still knows what happened.
- **Speech rate.** A per-space speech rate, from 0.9× to 1.3×, applied in synthesis on the server. The stream is shared, so the rate is shared, which is an open question.

### Several participants

- **Shared and local.** Shared state stays shared: history, timeline, queue, prompts and mode. Scroll position, expansion, local mute and replay are local.
- **Queue chips** carry the speaker's colour and name. Cancelling someone else's chip is allowed, as `design/spaces.md` requires, but it shows "cancelled by you" to them on the timeline.
- **Prompts** show who is choosing and who decided.
- **Catch-up and replay** are personal, using cursors, so one person's "repeat that" doesn't replay to the room.
- **Avatars.** Other people's avatars and speaking rings make it clear who is talking to whom. A push-to-talk marker distinguishes "talking to the agent" from "talking to us".
- **Placing the surface.** In a shared scene, each participant's surface is placed relative to them, not at a shared position. Pointing at a shared surface would need a shared anchor, which `design/environments.md` leaves for later.
- **Visitors** (`design/security.md`) see the same surface read-only, and follow live speech. See [Roles and security](#roles-and-security).

## 3. Changes, staged

### Stage 1: MVP (hear it again, and see what went wrong)

Highest value for the least change. Most of it is client work. On the server, it adds speech retention for the latest reply, engagement, voice answers and brief speech.

- **Speech: retain, replay and catch up.**
  - Server:
    - `Voice` keeps the latest assistant reply's audio per space. It is bounded, for example to 5 minutes, about 14 MB, and the oldest audio is dropped first.
    - The pacer keeps a cursor into that reply for each participant. Engaged participants, visitors and the spectator feed get frames at the live edge, as today.
    - A participant who is away or absent keeps the position where they stopped hearing.
    - On join, or on becoming engaged, a participant who hasn't heard the latest reply to its end is paced from its start, in order and at its playing rate, and live speech follows.
    - This is the largest MVP item, because it changes the pacer's broadcast loop in `voice.rs`.
  - Protocol:
    - `ClientMessage::Replay { message: Option<String> }` (`None` means the latest) and `ClientMessage::StopReplay`;
    - `ServerMessage::Replay { message, sample_rate }`, announcing the frames;
    - `REPLAY_SOURCE`.
  - Client:
    - the right-stick click replays or stops;
    - the left-stick click mutes locally, exposing `set_agent_muted` on `PlayerControl`;
    - the panel shows "Replaying…".
- **Engagement.**
  - Protocol: `ClientMessage::Engagement { state }`, and `Participant.engagement`, which is optional.
  - Client: a state machine from session state, plus `XR_EXT_user_presence` where supported. The live microphone stops on away. Transitions are logged at `info`.
  - Server: record it; "listening" means engaged.
- **Brief speech.**
  - `SpaceState.speech` is `full | brief`, with brief speaking the first blocks up to about 20 seconds, then "More on the panel."
  - `ClientMessage::SetSpeech`.
  - A control in the mode ring.
  - Server work: the cut-off in `Voice`.
- **Answering questions by voice.**
  - Protocol: `UtteranceBegin.answer: Option<{ question, field }>`, which defaults to absent.
  - Server: the final transcript goes to `answer()` instead of `enqueue`. The result comes back as `Transcript` (final) or `TranscriptFailed` with the projection's error.
  - Client: holding X while a question is shown answers it. "Answer this question on the PC." is removed for supported questions.
- **Errors and status** (client only):
  - a separate problem line, which neither "Sending…" nor any other status can displace;
  - each problem keeps its time and stays until dismissed or replaced by a newer problem, instead of being cleared by the next send;
  - status lines wrap to two rows rather than being cut;
  - the server's sticky error is shown as a condition, and can be dismissed locally.
- **Queue strip and B** (client only):
  - the queue is always visible as its own strip of entries (speaker, state, excerpt), and is no longer an item after the last message;
  - the stick plus A, or pointing, selects an entry, and A cancels it with a three-second undo;
  - B no longer denies or leaves: holding B leaves, and deny is an explicit option in the approval choices.
- **Panel placement and text** (client only):
  - the panel becomes world-anchored with lazy follow, instead of head-locked: same mesh, a smoothed transform;
  - body text wraps to more rows, and a reply is shown as a continuous scroll rather than separate pages;
  - Markdown syntax is stripped for display;
  - the panel follows the start of a streaming reply, not its tail.
- **Modes** (client only): Y opens a ring of the offered modes with descriptions, using the arrival ring code, and choosing one sends `SetMode` with that mode.
- **Roles** (client): hide the queue, decision, mode, push-to-talk and live-microphone controls from a visitor, as `design/security.md` requires. This depends on that branch's role message, so it lands with or after it.
- **Also cheap:**
  - feed `speech.playing`, `recording` and `voice_busy` into the swarm's signals, so it shows Speaking, Listening and Transcribing;
  - remove "reset the swarm" from A inside a space;
  - fix the readme's stale control descriptions.
- **Tests:**
  - the engagement state machine;
  - a participant who was away or absent hears the latest reply from its start on return, without the live frames overlapping, while engaged participants and visitors stay live;
  - replay to one participant only, with live speech preempting it;
  - brief speech cut-off at block boundaries;
  - a voice answer routed to `Answer`, with a choice field validated;
  - status priority: an error is visible while sending;
  - B never leaves without a hold;
  - undoing a cancel within the window.

**Risks:**

- **Quest presence signals.** How quickly the Quest runtime reports removal is unknown. If it is slow, speech plays to a removed headset for the first few seconds, and catch-up must start a little *before* the point of removal, which needs a "heard up to" cursor (stage 3). The stillness heuristic must not hold speech from a still, attentive listener; that is why it only demotes after a long quiet.
- **Memory.** Retaining one reply per space is bounded, but 8 spaces × 14 MB is about 112 MB at worst. The bound may need to be per server.
- **Changing two rules.** Catch-up changes "join mid-segment" and "history not read on join". With two listeners, they can hear the same reply at different times, which is confusing over speakers in one room. "Jump to live" mitigates it.
- **Brief speech.** It may cut a reply exactly where it gets important. The cut is at block boundaries, and "continue" is one click away.

### Stage 2: the conversation surface and seeing people

- **Protocol (additive, `serde(default)`):**
  - `Message.created_at` (Unix milliseconds) and `Message.speaker: Option<{ source, name }>` for user messages, with the prefix kept out of the displayed text;
  - `Message.spoken: Option<{ duration_ms, retained }>`;
  - `SpaceState.timeline: Vec<TimelineEntry>` (bounded, for example the last 100), each with an id, time, kind and actor:
    - `decided { request, title, decision, option_label }`;
    - `mode_changed { mode }`;
    - `worker_changed { worker, state }`;
    - `joined` and `left`;
    - `problem { source, text }`;
    - `cancelled { entry }`.

    A separate field is needed, since `Role` can't grow without breaking older clients.
  - `Replay { message }` for any retained or re-synthesized message.
- **Server:**
  - stamp messages;
  - record the actor of decisions, mode changes and cancellations;
  - narrow `error` to ongoing conditions, with one-off failures becoming problem entries;
  - retain audio for the last N replies within a per-space byte budget, and re-synthesize older replies on demand at a lower priority than live speech;
  - the snapshot budget (`design/server.md`) drops the oldest timeline entries alongside the oldest messages.
- **Client:**
  - the world-anchored surface: turns, expansion, SDF Unicode text, Markdown rendering, speaking-now highlight, per-message replay, timeline;
  - prompt cards with pointing and hold to confirm;
  - the mode ring;
  - pointing inside a space;
  - capacitive labels on the controllers;
  - haptics and earcons;
  - avatars from `Presence`, with nameplates, speaking rings and engagement;
  - the "while you were away" card.
- **Shared scene model:** the surface's model (rows, expansion and scroll, without rendering) lives in the render-free scene layer, so the desktop and the spectator can present it (`design/clients.md`).

**Risks:**

- **Snapshot size.** Every streaming delta re-sends the whole `SpaceState` about ten times a second (`design/server.md`), including the timeline. Bounded, but heavier, which stage 4 addresses.
- **New font code.** An SDF font adds a font asset and an atlas, where `caption.rs` deliberately had none.
- **Placement and environments.** Placing a surface well in every environment is unsolved, because environments differ (`design/environments.md`).

### Stage 3: tangible queue, voice commands, hands and personal cursors

- **Protocol:**
  - `MoveQueued { entry, before }` and `MergeQueued { entries }`;
  - a per-space `batch` setting;
  - `UtteranceBegin.purpose: prompt | answer | command`, with commands answered by a `ServerMessage::Command { recognised, outcome }`;
  - a per-participant "heard up to" cursor.
- **Server:**
  - queue reordering and merging, keeping the "first in, first out unless moved" rule in `design/spaces.md`;
  - the command grammar, applied to final transcripts of command utterances only;
  - speech cursors extended from the latest reply to the retained history;
  - "heard by" tracking, which feeds PR #62's "for me" stage;
  - a system voice for spoken catch-up and confirmations: a Kokoro speaker distinct from the agent's.
- **Client:**
  - queue chips with grab, drag, merge, flick and redo;
  - Y-hold commands;
  - hand-tracking pointing, pinch and poke;
  - wrist push-to-talk;
  - focus dwell marking messages seen;
  - worker detail on pointing.

**Risks:**

- **Cursors over history.** Extending cursors to history makes each catch-up a separate stream. It must keep `design/live-audio.md`'s rule that participant audio never displaces agent speech, and bound how many catch-up streams run at once.
- **The command grammar** has to stay small to stay reliable.
- **Merged prompts** change what the agent sees.

### Stage 4: scale and reach

- **Incremental history:** `ServerMessage::MessageDelta` and `TimelineAppended`, instead of full snapshots during streaming, plus `ClientMessage::History { before, limit }` to page beyond the 50 kept messages.
- **Blocks for clients.** Send the server's document-level blocks to clients, so the surface renders tables and code exactly as speech segmented them.
- **Agent summaries.** "Summarise what I missed" on request. It costs an agent turn and enters the session's context, so it is explicit, never automatic.
- **Notifications off the headset**, if wanted.

### How this fits the attention proposal (PR #62)

- **Different jobs.** PR #62 says *which* space needs you and gets you there. This proposal makes the space understandable once you arrive. The "while you were away" card is the natural landing for #62's jump.
- **Reply level.** #62's **reply** level should use engagement, not membership: "a turn ended while no participant was engaged".
- **Stage 3 alert.** #62's stage 3 "HUD line inside a space" becomes the alert pip and a status line entry here, not a new head-locked line.
- **"For me".** #62's stage 4 "for me" uses this stage 3's "heard up to" cursor.

## Open questions for you

1. **Catch-up and live speech.** When you return mid-reply, should you hear the reply from its start, and stay behind live until you catch up (recommended), or jump straight to live after a spoken "you missed the start"?
2. **Brief speech.** Should brief be the default? Is about 20 seconds, or the first block, the right cut-off? Would you also like a concise-reply hint sent to the agent, even though it changes the delivered prompt?
3. **Several queued entries.** Should they be delivered as one turn by default (batching), or kept separate, with merging only when you ask?
4. **The live microphone.** Should it stay on the left grip, or move to a less accidental control, such as a long press of the left-stick click?
5. **Y.** Is "hold Y for Tomori commands" acceptable, with modes moving to pointing at the mode control? Or would you rather keep Y as modes and use another control for commands?
6. **Approvals by voice.** Do you want "allow once" by voice at all? If so, should standing grants ever be voice-only?
7. **Being away.** Should Tomori notify you anywhere other than the headset, such as on the PC, when an approval waits? And how long should the headset be still before it counts as away?
8. **One-handed use.** Should push-to-talk also work from the right controller?
9. **Speech rate.** Is a shared, per-space speech rate useful, given that everyone in the space hears the same stream?
10. **Panel placement.** Should the conversation surface sit to the left of the swarm, the right, or below it? And should it follow you lazily, or stay where you left it?

### Coordination with the security work (`origin/security`, PR #58)

11. **Order of landing.** This proposal's protocol additions assume that branch's shape: participants named by `source`, `QueueEntry.source`, and protocol version 2. I recommend landing it first and building the MVP on top. Is that the order you want, or should the two be developed together?
12. **Visitors and replay.** May visitors request a replay of the latest reply, or only hear the live stream? Each replay is a stream the server paces, so it counts against what visitors can cause, which that branch bounds.
13. **Who counts as listening.** I propose that only participants (owners and collaborators) have catch-up cursors and count as "listening" for attention, while visitors hear live speech but never hold or clear anything. Does that match the intent of the visitor role?
14. **Knocks.** Should knocks use the same prompt card and timeline as approvals, as proposed here? Or does the people-and-access interface that branch defers want its own place in the space?
15. **Engagement visibility.** Is it acceptable that other participants, and visitors, can see when someone is away? It reveals a little about a person's state, much as the participant list already does.

## Design changes this would make

If agreed, I would update these binding documents with your decisions, through `base-design-author`:

- **`design/speech.md`:**
  - retain synthesized audio (the latest reply in the MVP; later, a bounded set per space);
  - give each participant a speech cursor, and play the latest reply they haven't heard from its start on join or return, which replaces "existing history is not read aloud when joining";
  - visitors and the spectator feed stay at the live edge;
  - per-participant replay, with live speech preempting it;
  - the `brief` speech setting;
  - later, a system voice and speech rate.
- **`design/live-audio.md`:**
  - replace "a participant who joins mid-segment hears it from that point" with catch-up from the reply's start;
  - add `REPLAY_SOURCE`, mixed like agent speech and silenced during push-to-talk;
  - stop the live microphone when the headset is removed, as well as on focus loss;
  - frames from the live edge are not sent to a participant who is catching up;
  - later, cursors over the retained history.
- **`design/spaces.md`:**
  - add engagement to per-participant state, as presence;
  - route spoken answers to the presented question;
  - add the timeline, with actors recorded for decisions, mode changes and cancellations;
  - narrow the space error to ongoing conditions;
  - later, queue reordering, merging and batching, and voice commands that never reach the agent.
- **`design/clients.md`:**
  - the engagement state machine and the conversation surface's model belong in the render-free shared layers;
  - the protocol carries engagement as a device-neutral fact.
- **`design/arrival.md`:**
  - pointing extends into the space;
  - the selection rule "the highlight wins over the stick selection" applies everywhere;
  - hand tracking (stage 3) follows the pointing model.
- **`design/client-logging.md`:** log engagement transitions and the signals behind them at `info`.
- **`design/spectator.md`:**
  - the spectator view shows the world-anchored surface or doesn't, for privacy; a decision is needed;
  - it still shows no alert pip, captions bar or recording indicator.
- **`design/agent-backends.md`:** no change, unless you want the concise-reply prompt hint, which would be stated there and in `spaces.md`.
- **`design/security.md`** (PR #58, not on `main` yet):
  - add engagement to what visitors don't send, and to the role check;
  - name timeline actors by public handle;
  - possibly, whether visitors may request replays;
  - nothing about authentication, pairing or roles changes.
- **New `design/interaction.md`:** the control mapping, the three error channels, the rule that no button has a hidden destructive meaning (B never denies, cancels or leaves without a hold), hold to confirm for persistent grants and risky modes, and the legibility and comfort rules: no head-locked text, sizing by visual angle, and colour never the only signal.
