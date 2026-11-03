# Proposal: Showing What Needs Attention Across Servers and Spaces

Status: **stage 1 agreed and implemented**, November 2026. The user's decisions are recorded under [Decisions](#decisions). Stage 1 is now a requirement in `design/spaces.md`, `design/server.md` and `design/arrival.md`, which take precedence over this document. Stages 2 to 4 remain proposals for discussion; they are listed as open questions in `design/arrival.md`.

## The request

Spoken from the headset, November 2026:

> As a user of multiple computers, my work laptop and my personal desktop, I would love it if the server browsing interface would show which servers and spaces require my attention and give me the option to jump to them quickly.

## Recommendation in brief

- **What counts.** Each space gets one *attention* level, worked out by the server from facts it already has. There are three levels: **input** (an approval or question is waiting), **problem** (something failed while nobody was watching) and **reply** (the agent finished a turn while nobody was in the space). Running, idle, queued and closed spaces are *not* attention; they keep today's activity display.
- **MVP: arrival only, "for anyone".**
  - The server lists each space's attention in `SpaceSummary`.
  - It advertises three small counts in its Bonjour TXT record, so the server row shows a badge before connecting.
  - The headset marks servers and spaces, and offers one "jump" object per server that connects and joins the most urgent space in a single trigger pull.
  - No backend changes. No protocol version change.
- **Later.** In order:
  1. keep browsing while in the lobby, so other servers' attention shows there;
  2. a lightweight *watch* connection, so you hear about the other computer while inside a space;
  3. per-participant "for me" attention.

## What the system knows today

Everything below is from `origin/main` at `4601a94`.

**Activity and state**

- **Activity.** `Activity` (`crates/protocol/src/space.rs`) is `idle`, `running`, `waiting`, `closed` or `unknown`.
  - The server derives it in `SpaceModel::activity` (`crates/server/src/space.rs:217`): closed wins, then waiting, then the backend's busy flag.
  - Waiting covers pending approvals and questions, and those whose decision or answers are still being delivered (`SpaceModel::is_waiting`).
- **What the lobby listing carries.** `SpaceSummary` holds an id, title, backend, directory, environment, activity and participant count. It has no error, no queue and no "finished while away" fact.
- **Space errors are sticky.** `SpaceState::error` is set by backend errors, refused modes, failed delivery and closing (`space.rs:396–409`, `:507`), and nothing clears it. A raw "has an error" flag would therefore stay lit forever.
- **Backend facts already reported** (`crates/backend/src/lib.rs`, `Event`):
  - `Busy` and `Idle`;
  - `MessageStarted`, `TextDelta` and `MessageCompleted`;
  - `PermissionRequested` and `QuestionAsked`, with their resolutions;
  - `Error` and `Closed`.

  Every attention level below can be derived from these. No backend needs to change.
- **Queued input is not a demand.** The queue releases only when the space is idle (`design/spaces.md`), so queued entries mean "busy with a backlog". A *failed* entry (`QueueState::Failed`) is a visible failure.
- **Being in a space means you heard the reply.** Replies are spoken to everyone in the space (`design/speech.md`).

**The lobby listing**

- **Who gets it.** `HubState::touch_space` (`crates/server/src/hub.rs:288`) calls `touch_lobby` (`:359`) on every space change. That pushes a fresh listing only to participants **not in a space** (`record.space.is_none()`).
- **How often.** Streaming replies touch the space up to ten times a second (`touch_space_paced`), so lobby clients already receive frequent listings, coalesced per connection (`Outbox::spaces`, `crates/server/src/outbox.rs:181`).
- **Participants inside a space receive no listing at all.** Today they cannot see other spaces even on the same server.
- **The listing can be personalised.** It is rendered per connection when sent (`hub.rs`, `Outgoing::Spaces`), so a per-participant listing is possible without new machinery.

**Discovery**

- **TXT keys.** Discovery advertises `protocol`, `name`, `spaces` and `participants` (`crates/discovery/src/lib.rs`).
- **Update rate.** Changes are coalesced to one announcement per 2 seconds (`COUNTS_INTERVAL`, `crates/server/src/server.rs:71`), driven by `Hub::counts` and its `counts_changed` notification.
- **Missing keys.** A missing or malformed key reads as unknown, so new keys are backward compatible.
- **The headset browses only while the server list shows.** See `Conversation::is_browsing` (`crates/client-quest/src/conversation.rs:242`, `mode == Servers`) and `Connection::browse` (`crates/client-quest/src/connection.rs:679`).
  - Browsing holds an Android `MulticastLock`, which the discovery documentation warns costs battery.
  - In the lobby or in a space, the headset knows nothing about other servers.

**Connections and identity**

- **One connection at a time.**
  - `Conversation::connect` resets all lobby and space state.
  - `leave_server` (`conversation.rs:735`) disconnects **without sending `Leave`**.
  - The server keeps the participant for 12 hours (`DEFAULT_PARTICIPANT_RETENTION`, `server.rs:59`) and rejoins their last space on the next Hello (`hub.rs:808`).

  So returning to a server you left by switching puts you back in your old space.
- **A second Hello with the same participant identity supersedes the first connection** (`hub.rs:810`). A background poll or watch connection must not say Hello as the participant, or it would kick the headset out of its own session and drag it into its old space.
- **Identity.**
  - The headset keeps one random participant identity in one file (`identity.rs`, `Store::participant`) and sends it to every server. "The same person" is therefore already recognisable across servers.
  - Desktop runs keep no identity (`desktop_connection`, `crates/client-quest/src/lib.rs`, `Store::new(None)`).
  - Only one server is remembered (`Store::remember_server`, `identity.rs:97`), and it is connected automatically (`server_choice::automatic`).

**Visuals**

- **Spaces** (`crates/client-quest/src/arrival_graphics.rs`):
  - a waiting space already shows a pulsing amber cube above its diamond (`:146`);
  - a closed one is desaturated;
  - a running one spins faster.
- **Servers.** Constellations show one orbiting point per space and one cube per participant (`design/arrival.md`).

## 1. What needs attention

### Levels

The server computes one level per space. When several apply, the highest priority wins.

| Priority | Level | Defined as | Cleared when |
|---|---|---|---|
| 1 | **input** | Activity is `waiting`: an approval or question is pending, or its decision is being delivered. | It is resolved by anyone (already shared state). |
| 2 | **problem** | While **nobody was in the space**, a queue entry failed, the backend reported an error, or the session closed. | Someone joins the space. |
| 3 | **reply** | A turn ended (busy → idle) that completed at least one assistant message, while **nobody was in the space**. | Someone joins the space. |
| none | busy | Running, including with queued entries. | |
| none | idle | Idle, with nothing unattended. | |
| none | closed | Closed while someone was present, or a problem since cleared by a join. | |

### Why it is defined this way

- **"While nobody was in the space" turns sticky facts into events.** The error string never clears, and a closed space stays listed (up to 32), so flagging every space with an error or closure would be permanent noise. What matters is that something happened *that no one saw*. The server already knows who is in each space (`SpaceModel::participants`).
- **Reply means "unheard", not "unread".** Replies are spoken aloud, so having been present is a good, cheap proxy. It needs one flag per space, set on the idle transition and cleared on join.
- **Queued input is excluded.** The queue drains by itself. A *failed* entry is excluded too unless nobody was present to see it fail, in which case it counts as a problem.
- **Busy is excluded.** "The agent is working" is not "the agent needs me". Busy keeps its current look: a faster spin.
- **Within a level, oldest first.** The server sends how long the space has been in that level, so the longest-waiting approval comes first, on one server or across several.

### For anyone, or for me

**For anyone (the MVP).**

- A space is shared. The first participant to resolve an approval resolves it for everyone (`design/spaces.md`), and anyone joining has seen the reply.
- This fits the user's case, one person with two computers, and works in a public TXT record.

**For me (later).**

- Because the headset sends the same identity to every server, a server can personalise its listing:
  - **reply** only if *you* weren't present when the turn ended;
  - an extra **yours** marker on input when the pending request follows a prompt you spoke (`QueueEntry::participant` of the last delivered entry).
- This needs a small, bounded "seen by" set per space that outlives the 12-hour participant retention.
- Desktop clients would need to store an identity first.

### Mentions

There is no structured fact for "the agent addressed Ada". Speaker attribution (`Ada: …`) goes *to* the agent, and replies are free text. Detecting mentions would mean parsing agent text, which `design/agent-backends.md` assigns to the server rather than backends. That is possible, but fuzzy and noisy. I recommend leaving mentions out until there is a concrete need (see the open questions).

## 2. How a client learns about servers it isn't connected to

| Option | Bandwidth | Battery (headset) | Privacy | Staleness | Verdict |
|---|---|---|---|---|---|
| **A. Counts in the TXT record** | About 30 extra bytes per announcement; at most one announcement per 2 s, and only when the counts change. | None beyond browsing, which already happens in the server list. | Reveals to the LAN *how many* spaces wait for input, have a problem or have a reply. No titles, ids or directories. | A few seconds while browsing. Unknown when not browsing. | **MVP**, for the server row |
| **B. Short-lived poll connection** (connect, `ListSpaces`, close) | Each poll costs a Welcome (backends, roots, environments) plus a listing of up to 256 KiB. | A TCP setup per server per interval. | The same as joining the lobby. | The poll interval. | **Rejected**, see below |
| **C. Watch subscription**: one long-lived, listing-only connection per known server | Pushes a small digest only when attention changes, plus keepalive (a ping at most every 5 s of quiet, `design/connection-liveness.md`). | One idle socket per server; the radio wakes for keepalives. | The same as the lobby today (titles). | Immediate. | **Stage 3**, for awareness inside a space |
| **D. Per-participant attention** | As C. | As C. | Personal state reaches only that participant's connection. | As C. | **Stage 4**, on top of C |

### Why not the TXT record for everything

- A TXT record is public on the LAN and cannot be personalised.
- It is bounded: RFC 6763 suggests keeping the whole record small, so it can't list every space.
- The headset only receives it while browsing with the multicast lock held.

Counts are the right size, and enough to badge a server and colour its orbiting points.

### Why polling is rejected

- **Identity.** A poll saying Hello with the participant's identity supersedes their live connection on that server and rejoins their old space (`hub.rs:808–815`). A poll with a throwaway identity creates a ghost participant, who is counted in the advertised `participants` and kept for 12 hours.
- **Cost and staleness.** It is also the heaviest option in bytes and the stalest.

### The watch subscription (stage 3)

A connection opened with `Hello { watch: true, … }` would get special treatment from the server:

- It does not become a participant: it is not counted, not rejoined, and supersedes no one.
- It receives only `ServerMessage::Attention { spaces }`, with each space's id, title, level and age.
- The digest is sent when any space's level changes, not on every streaming delta.
- The participant identity is still sent, so stage 4 can personalise the digest.

On the client side:

- The same digest goes to participants *inside* a space on the connected server, which today see nothing about their neighbours.
- The client core (`crates/client`) would own watch connections, so they stay platform-neutral (`design/clients.md`).
- Which servers to watch: those you have connected to before. This means remembering a short list of servers (instance name and last addresses) instead of today's single remembered server. Browsing can't be relied on inside a space.

Authentication doesn't exist yet (`design/server.md`), and you said this LAN needs no special protection for now. Watching reveals nothing that joining the lobby doesn't already. When authentication arrives, a watch connection should carry the same credentials as a join.

## 3. Presentation and jump

### The server row (arrival, before connecting)

- **Constellation points.** Each server's orbiting points already stand one per space. With the TXT counts, that many points take the attention colour: amber for input, red for problem, a soft white glow for reply. The rest keep the normal colour.
- **Beacon.** A server with any attention gets a slow amber (or red, or white) beacon above it, following the same priority. Its detail line changes from "2 spaces, 1 person" to, for example, "1 needs you · 1 reply".
- **Order stays stable.** Servers don't reorder by urgency. `design/arrival.md` relies on spatial and visual memory, and objects moving as states change would undo that. The beacon is the cue.
- **Jump object.** A server with attention shows a small separate satellite, pointable on its own. Choosing it connects and then joins the top-priority space in one trigger pull, as described under [The jump](#the-jump). Choosing the server itself still goes to its lobby as today.

### The lobby ring (connected)

- **Marks.** Spaces keep their positions. The existing waiting pulse (`arrival_graphics.rs:146`) becomes the input mark, with red for problem and a soft glow for reply.
- **Default selection.** The stick selection starts on the most urgent space, so stick plus A is also quick.
- **Status line.** The heading's status says what is waiting, such as "Deploy is waiting for an approval".
- **Elsewhere shelf (stage 2).** While in the lobby, the headset keeps browsing, so other servers with attention stay visible as small, badged constellations at the edge of the view. Choosing one jumps.
  - This changes `design/arrival.md`, where other servers "shrink and fade away" on choosing one. They would fade except where they need attention.
  - It keeps the multicast lock held during the whole arrival flow, not only in the server list.

### Inside a space (stage 3)

- Arrival takes place in the world, but a space has a head-locked HUD on the headset.
- When another space or server reaches **input**, a short HUD line and optionally a chime: "Desktop: Deploy needs an approval".
- Pressing a jump control goes there. Which control, and whether to speak it, is an open question.
- The spectator view shows no HUD (`design/spectator.md`), so it would not show this.

### The jump

One action, the same whether started from the server row, the elsewhere shelf or a space's HUD:

1. **Finish or stop talking.** Today an in-flight utterance is dropped with "Connection lost; your words were not sent." The jump should wait briefly for a pending transcript, or refuse with "Still sending your words".
2. **Leave the current space explicitly** by sending `Leave` before disconnecting. Then coming back to that server lands in its lobby with attention showing, instead of today's silent rejoin of the old space. This matches "moving to another space leaves the previous one" (`design/spaces.md`), extended across servers. See the open questions.
3. **Switch servers.** Disconnect, connect, then Hello and Welcome, as `Conversation::connect` does. The connecting server shows as chosen in the scene.
4. **Join.** On the first listing, join the target space by id.
   - If the space is gone, the server's join refusal is shown as today.
   - If its attention was resolved meanwhile, it is still joined, with a status saying it was already handled.
5. **Animate.** The current environment folds back into its space object, which returns to the grid. The server change plays as in arrival, and the target space's environment grows out of its object. Every step is interruptible, as `design/arrival.md` requires.

A jump to a space on the **same** server is just a `Join`.

The target is the most urgent space as last known. A watch digest names it by id. With only TXT counts, the client picks the most urgent space from the listing it receives on connecting.

### Desktop and other clients

- **Shared logic.** Attention is computed by the server and carried in shared protocol types. Ordering and colour mapping live in the render-free arrival model (`crates/client-quest/src/arrival.rs`), so any client presents the same thing (`design/clients.md`).
- **Console client.** `console.rs` and `servers_text` would list servers and spaces with their attention first, and could offer a jump key.
- **Desktop window.** It has no arrival scene today, so it would need nothing for the MVP.
- **For me.** "For me" (stage 4) needs desktop runs to store an identity, which they deliberately don't today.
- **Device neutrality.** The protocol stays device-neutral. Nothing depends on hardware.

## 4. Changes, staged

### Stage 1: MVP (attention in arrival, for anyone)

- **Protocol** (`crates/protocol/src/space.rs`):
  - `enum Attention { Input, Problem, Reply, #[serde(other)] Unknown }`;
  - `SpaceSummary.attention: Option<Attention>`, which defaults to none;
  - `SpaceSummary.attention_age: Option<u32>`, in seconds.

  The fields are optional and unknown values read as unknown, so there is no protocol version bump (`design/server.md`).
- **Server:**
  - `SpaceModel` gains an `unattended` level. It is set on the busy → idle transition after a completed assistant message, on a failed entry, on an error and on closing, each only when `participants` is empty. It is cleared in `HubState::join`.
  - `summary()` reports the highest level.
  - `Hub::counts` adds the three counts and notifies `counts_changed` when they change.
- **Discovery:** `Counts` and `Server` gain `waiting`, `problems` and `replies`, advertised as TXT keys of the same names and parsed with the existing unknown-on-malformed rule.
- **Headset:**
  - `Choice` carries the counts;
  - `arrival::Kind::Server` and `Kind::Space` carry attention;
  - new `arrival::Key::Jump(address)`;
  - `Conversation` gains a pending jump: connect, then join the top space on the first listing;
  - the default selection is the most urgent space;
  - graphics show the beacon, the point colours and the space marks.
- **Backends:** none.
- **Tests:**
  - the level transitions, including present versus absent, and clearing on join;
  - priority and age ordering;
  - TXT round-trip and unknown handling;
  - the jump sequence in `Conversation`, including a target that is gone or already resolved.

### Stage 2: other servers while in the lobby

- Keep browsing in `Mode::Lobby`, so `is_browsing` covers all of arrival.
- Add the elsewhere shelf.
- Remember several servers.
- Send `Leave` before a switch.

### Stage 3: watch connections and awareness inside a space

- `Hello.watch` and `ServerMessage::Attention`.
- The server sends the digest to watchers and to in-space participants, only on level changes.
- Watch connections live in the client core.
- Add the HUD line and jump control.
- Decide the keepalive for watchers.

### Stage 4: for me

- Personalised levels from the participant identity, with a bounded "seen by" set per space.
- A "yours" marker.
- Stored desktop identity.
- Mentions, if wanted.

### Risks

- **Reply noise.** In auto mode, an agent working alone may finish many turns. Only the latest unattended reply matters, and one join clears it. If it is still too much, reply could be shown more quietly than input, or not at all (open question 1).
- **Being present is not paying attention.** A headset that is unfocused or taken off but still connected counts as present, so a reply would count as heard. Presence reports could refine this later, if the headset stops reporting presence when unfocused.
- **Races.** Attention can be resolved by someone else between the badge and the jump. The jump must still land somewhere sensible, as described above.
- **Battery.** Stage 2 holds the multicast lock through the whole arrival flow. Stage 3 keeps one socket per remembered server with keepalives. Both should be measured on the Quest.
- **LAN disclosure.** The TXT counts reveal that a machine's agent is waiting. `--no-advertise` already turns advertising off; a narrower switch for just the attention keys may be wanted.
- **Restarts.** Unattended state is in memory, like spaces. After a server restart, space ids change and everything starts clear.

## Decisions

The user answered the questions below in November 2026:

1. A finished reply counts as needing attention.
2. "For anyone" is enough for now: whoever deals with something first clears it for everyone.
3. Publishing the three counts in Bonjour is OK, with an operator switch to turn just them off: `tomori-server --no-advertise-attention`. Turned off, the keys are left out rather than advertised as zero.
4. Jumping away leaves the current space.
5. No notification inside a space in stage 1; that is a later stage.

The remaining questions follow this document's recommendations:

6. The remembered server is connected automatically about one and a half seconds after browsing finds it, rather than at once, so that other servers found at about the same time are known. If another server then advertises attention and the remembered one doesn't, it is not connected automatically for the rest of that visit to the server row, so that the participant sees the other and nothing connects while they reach for it.
7. Mentions are left out until there is a concrete need.
8. A space that closed while nobody was in it counts as a problem.

Stage 1 differs from the sketch above in a few details:

- The beacon above a server is the jump object itself, a pulsing diamond in the attention colour, and the server glows with a ring of points in that colour. Servers are still drawn as single cubes rather than constellations, so there are no orbiting points to colour.
- A reply needs a completed assistant message with text; a turn of tool calls alone is not a reply.
- An agent refusing a mode while nobody is in the space also counts as a problem, as it sets the space's error.
- Only a participant's own `Join` clears a problem or a reply. The server putting a participant back in their last space on Hello does not, since they did not see what happened while away; a reconnecting client restores its membership with a `Join`, which does, and the headset sends a `Join` for a space it was put in unasked once it shows it. A jump that lands in an old space first moves on without clearing that space's attention, or joins it if it is the most urgent.
- A reply whose turn ends during a backend reconnection counts, learned from the resynchronization or a restored message.
- Spaces now survive a server restart, but attention does not: a space listed again starts with no problem or reply, even one already closed or one that cannot be bound again, and only a request still pending in the session shows as input.
- With only the TXT counts, the jump picks the most urgent space from the listing it receives on connecting. If nothing needs attention by then, it stays in the lobby and says so, rather than joining a space that was already handled.
- A jump starts only from the server row, which a participant reaches from a space only by leaving it with `Leave`, so it never needs to leave a space itself yet.

## Open questions for you

1. **Does a finished reply count as needing you,** or only approvals, questions and failures?
2. **Is "for anyone" enough for now?** In that case, the first person to deal with something clears it for everyone, and the later "for me" stage covers more people sharing servers.
3. **Is it OK to publish the three counts in Bonjour on this LAN?** If so, should there be a switch to turn just those off?
4. **When you jump away, should Tomori leave your current space** (recommended), or keep you in it so that returning to that server puts you back?
5. **Do you want to be told while inside a space** (stage 3)? If so, how: a HUD line, a chime, or a spoken line? And which button jumps?
6. **Should the headset still connect automatically to the remembered server** when a *different* server needs you, or stay on the server row?
7. **Mentions:** do you want them, and what should count?
8. **Should a space that closed while you were away count as a problem,** or just show as closed?

## Design changes this would make

If agreed, I would update these binding documents, through `base-design-author`, with the user's decisions:

- **`design/spaces.md`:**
  - define the attention levels, what "unattended" means and when a level clears;
  - add attention to the lobby listing;
  - (stage 3) send the attention digest to participants inside a space;
  - (stage 4) per-participant attention.
- **`design/server.md`:**
  - add the `waiting`, `problems` and `replies` TXT keys under the existing coalescing rule, and say what they reveal;
  - (stage 3) watch connections, which are not participants, are not counted, are never rejoined and supersede no one.
- **`design/arrival.md`:**
  - server beacons and attention-coloured points;
  - the jump object;
  - stable ordering, with the selection starting on the most urgent space;
  - (stage 2) browsing and the elsewhere shelf in the lobby, which changes "the other servers shrink and fade away";
  - the jump transition and leaving the current space.
- **`design/clients.md`:** watch connections belong in the client core, and attention ordering belongs in the shared scene model.
- **`design/connection-liveness.md`:** (stage 3) the keepalive for watch connections, if it differs.
- **`design/agent-backends.md`:** no change, because backends already report every fact used.
