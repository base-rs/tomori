# Proposal: A Controller-Anchored, Structured Interface for the Headset

## Status

**Stage 1 is built; stages 2 and 3 are open.** November 2026. The binding requirements are in `design/wrist-transcript.md`, `design/hand-menu.md`, `design/interaction.md` and `design/feedback.md`, which take precedence over this document. The rest of this document is the proposal as it was put to the user, from [PR #89](https://github.com/ioquatix/tomori/pull/89), kept for its reasoning; it was written before stage 1, and its references to the code describe `main` at that time.

**Built:**

- **The recording card** on the left controller, with the server's utterance limit in `Welcome`, and the local-space panel renderer with depth and translucency ([#104](https://github.com/ioquatix/tomori/pull/104)).
- **The right wrist's transcript:** speaker colours, grouped turns, Markdown-lite, tool chips, the request card with a button per choice, the queue with removal and undo, and detaching with the right grip ([#105](https://github.com/ioquatix/tomori/pull/105)).
- **The status pill** in place of the floating panel, and the hand menu's header, Space section, participant swatches and Y pin ([#106](https://github.com/ioquatix/tomori/pull/106)). #104 and #105 landed on `main` through #106, which was stacked on them.
- **Later fixes:** the recording card saying when the server is transcribing ([#162](https://github.com/ioquatix/tomori/pull/162)); one palm-up gesture for both hands ([#140](https://github.com/ioquatix/tomori/pull/140)); the hand menu's text ([#169](https://github.com/ioquatix/tomori/pull/169)); pointing at its rows by their rectangles ([#171](https://github.com/ioquatix/tomori/pull/171)); and keeping them still while a mode is pointed at ([#173](https://github.com/ioquatix/tomori/pull/173)).
- **From the later stages:** haptics for recording, the 30 s and 10 s limit, requests and choices came with the audio and haptics proposal's stage 1 ([#107](https://github.com/ioquatix/tomori/pull/107), landed through [#109](https://github.com/ioquatix/tomori/pull/109)); antialiased Unicode text from a glyph atlas came with [#176](https://github.com/ioquatix/tomori/pull/176), under [#146](https://github.com/ioquatix/tomori/issues/146).

**Decided since the proposal.** The user asked to "just let it make decisions", so stage 1 took the recommended answer to each [open question](#open-questions-for-you), as #104 records, with these changes:

1. The floating panel became the two-row status pill, which can be turned off; the transcript detaches for long reading.
2. The right gesture mirrored the left at first. Since #140, both hands use one gesture: the palm within 45° of world up for 0.2 s, hiding beyond 55°, rather than the palm facing the head.
3. Y pins the left menu open at its Space section until Y or B.
4. The right grip detaches and reattaches the transcript.
5. A person's colour is the same on every headset, from their public handle.
6. "More on the panel." was left alone. Replies are now spoken in full by default ([#94](https://github.com/ioquatix/tomori/pull/94)), so it is said only in a space chosen brief.
7. Hand tracking is read-only at first.

**Open:**

- **Structured facts:** `Message.speaker` and `Message.tools`, a "speaking now" highlight, replaying a chosen reply, and a timeline of events: [#190](https://github.com/ioquatix/tomori/issues/190).
- **Avatars** in participants' colours: [#191](https://github.com/ioquatix/tomori/issues/191).
- **Hands:** poking, pinch-scrolling and pinch-to-talk with hand tracking. No issue yet; [#78](https://github.com/ioquatix/tomori/issues/78) covers input routing in environments.
- **The queue:** reordering and merging entries (`MoveQueued`), shared with the interaction proposal's stage 3. No issue yet.
- **Smaller deferrals from stage 1:** several pending requests as "1 of 3", smooth scrolling, the `LIVE` badge following the microphone's level, and tuning sizes and angles on the headset. No issue yet.

## The request

Spoken from the headset, November 2026:

- **The panel.** "I still don't really like the panel, but I get why it exists." It is "just a wall of text", with little styling or organisation, so it is hard to follow and process.
- **Recording.** The indicator that says "Release X to send" should pop up from the **left controller**, where X is, while recording. Today it floats in the middle of space.
- **The right controller.** Rotating the right wrist should show a structured interface, like the existing left-hand menu, holding:
  - the transcript and interaction: the conversation, approvals and questions;
  - managing the queue, such as removing entries;
  - colour coding that tells the agent apart from the humans, and each human from the others.
- **The left-hand menu** gains space-level information and controls, such as the chat title and the current mode (build or plan).

## Recommendation in brief

- **The problem in one sentence.** Every kind of information in a space is set as 40 × 12 ASCII rows in one font and two colours, on one panel in front of you. Separation comes only from position, and the rows a message gets shrink as other things compete for them.
- **The shape of the answer.** Each part of the interface goes to the hand that acts on it:
  - **Left controller: your voice and the space.**
    - **Recording indicator.** While X is held, a small card rises from the left controller with the timer, the words heard so far, the release hint and a warning near the limit. The live microphone shows a badge there too.
    - **Hand menu.** It gains a **Space** section (title, mode, speech, leaving) and the **Agent** section from issue #72, next to the existing **People & security**.
  - **Right wrist: the conversation.** Turning the right palm towards your face shows a styled transcript:
    - a colour bar and name per speaker, with turns grouped;
    - tool use as compact chips;
    - approvals and questions as distinct cards with one button per choice;
    - the queue, with a remove action and undo on each entry.

    It is driven entirely by the right thumb (stick, A, B), so no pointing is needed. Squeezing the **right grip** detaches it into the world, to read it at length without holding your arm up.
  - **Floating panel: shrunk to a status pill.** Two short lines, low in the view and still world-anchored with the existing lazy follow. The first says what the agent is doing; the second appears only when something needs you, such as "Approval waiting: turn your right wrist". The pill can be turned off.
- **MVP.** This is almost entirely client work. The one addition to the protocol is the utterance limit in `Welcome`.
  1. The recording indicator on the left controller.
  2. The right wrist transcript, with speaker colours, grouping, Markdown-lite styling, request cards and queue removal, plus detaching with the right grip.
  3. The status pill in place of the panel.
  4. The Space section in the left menu, coordinated with #72's Agent section.
  5. A generalised local-space panel renderer: per-span colour, translucent backgrounds, depth testing, and the wider `font8x8` character tables.
- **Later stages.**
  - **Stage 2:** structured speakers and tool calls in the protocol, a "speaking now" highlight, replaying a chosen reply, haptics, and participant colours shared with avatars.
  - **Stage 3:** hand-tracking interaction, antialiased Unicode text, a timeline of events, and reordering the queue.

## 1. Diagnosis: why the panel is hard to read

Everything below is from `origin/main` at `ed29dd0`.

### One font, two colours, no structure

- **The renderer is a grid of lit pixels.** `Caption` (`crates/client-quest/src/caption.rs`) draws `font8x8` glyphs as one quad per lit font pixel.
  - The grid is fixed at 40 columns × 12 rows (`COLUMNS`, `ROWS`, `caption.rs:10`), at 3.4 mm per font pixel.
  - One dark quad covers the whole panel (`mesh`, `caption.rs:217`).
  - Row 0 is cyan and every other row the same white (`caption.rs:228`). There is no other colour, weight, size, background, indent or divider. Nothing distinguishes a header from a problem, a message, a choice or a hint, except which row it falls on.
- **ASCII only.** `wrapped_lines` maps every other character to `?` (`caption.rs:384`), except curly quotes and dashes.
  - This rules out accents, arrows, box drawing, bullets and check marks.
  - The `font8x8` crate already ships Latin-1, box drawing, block elements and Greek tables. Only `BASIC_FONTS` is used.
- **Drawn over everything.** The panel has its own render pass with no depth buffer (`caption.rs:100`, drawn after the scene at `graphics.rs:475`), so it covers nearer objects.
  - It also drops by 0.4 m whenever the swarm leaves its neutral state (`caption_offset`, `graphics.rs:274`), so its position depends on what the agent is doing.

### The rows are rationed

`Conversation::space_text` (`crates/client-quest/src/conversation.rs:2713`) builds the panel as one list of strings:

```
row 0      [running | build] Fix the parser...   <- cut to 40 columns
row 1      MIC LIVE | Brief | 2 here             <- one status line
row 2-3    ! 2m Could not deliver input...       <- problem, 0-2 rows
row n      p2/5 ASSISTANT 7/12                   <- page and message counters
rows ...   the page of one message               <- what is left
           (blank padding rows)
           1/3: Allow once                       <- approval choice, if any
rows       > You queued: also check the CI...    <- queue strip, 0-2 rows
row 11     X talk | Y modes | hold B leave       <- footer of button hints
```

- **Little room for content.** The body has `BODY_ROWS = ROWS - 4` (`conversation.rs:25`), which is 8 rows, less whatever the problem, queue strip and approval choice take (`body_rows`, `conversation.rs:2675`). With a problem, two queued entries and an approval, a message gets about 3 rows: roughly 120 characters a page.
- **One message at a time.** `body()` (`conversation.rs:2450`) shows a single message, paged by `paginate` (`conversation.rs:2919`).
  - The left stick moves between messages and pages.
  - So there is no overview of the conversation, and no sense of where you are in it beyond `7/12`.
- **No speakers.**
  - A message is labelled only `USER` or `ASSISTANT` (`conversation.rs:2472`), because `protocol::Message` carries only a role, its text and whether it has finished.
  - Who spoke appears only as the attribution prefix baked into the delivered text (`Ada: run the tests`, `attributed` in `crates/server/src/space.rs`).
  - The agent and every human look identical apart from that label.
- **Raw Markdown.** Replies show `**`, `#`, backticks and fences as typed.
- **Tool activity** is shown only for a message with no text, flattened into lines such as `[Bash: working]` (`MessageModel::display`, `crates/server/src/space.rs:111`). Once the agent writes text, the tools disappear.
- **Truncation and filler.**
  - The header, status and queue rows go through `short()` (`conversation.rs:2898`), which cuts them at 40 columns.
  - Blank rows pad the layout so that the strip and footer land at the bottom (`conversation.rs:2782`, `:2802`). A short message therefore floats in empty space, with the queue far below it.
- **Hints as text.** The footer spends a row on button hints that change with hidden state (`conversation.rs:2776–2800`), on a panel already short of rows.

### The recording indicator floats with the panel

- **Fixed to the panel.** `recording_mesh` (`caption.rs:331`) draws the timer, the bar and "Release X when finished" at a fixed offset above the panel, in the panel's own coordinates. The panel is placed in front of you by `PanelAnchor` (`panel_anchor.rs`), so the indicator hangs about 1.7 m away in mid-air, not where your thumb is.
- **Duplicated.** The footer says "Release X to send" at the same time (`conversation.rs:2777`).
- **No limit known.** The timer is always shown unbounded. `xr.rs:424` passes `Duration::ZERO` as the maximum, because the client doesn't know the server's 5-minute limit (`speech::stt::MAX_UTTERANCE`). So the existing amber and red warnings in `RecordingProgress::color` never fire on the headset.

### What is good, and kept

- **World anchoring.** The lazily following anchor (`PanelAnchor`) is the right behaviour for anything left in the world, and is reused for the status pill and the detached transcript.
- **The hand menu's pattern.** Palm-facing visibility with hysteresis (`hand_menu::Visibility`), rows built from client state, sections and wrist cues are all reused for the right wrist.
- **The safety rules** of `design/interaction.md`:
  - B never decides, cancels or leaves;
  - deny is an explicit choice;
  - cancelling has a 3-second undo;
  - A is ignored for 0.6 seconds after a request appears.

  All of them carry over unchanged.
- **The conversation model's state.** Selection, cancelling, the request grace and following live are already modelled in `Conversation`. The new interface should be a new **view** of that model, not a second model.

## 2. Left controller

The left hand is **your voice and the space**: X to talk and the left grip for the live microphone are already there, so what you say, and the settings of the space you say it in, live on that hand.

### The recording indicator

**Placement.**

- **Anchor.** The indicator is anchored to the left **grip pose** (`left_grip_space`, `xr.rs:1166`). Its centre is offset in the grip frame by about 6 cm along +Y (up out of the top of the controller, above the face with X and Y) and about 3 cm along −Z (forward).
  - It rises out of the controller like a speech bubble, above the X button, where the thumb is.
- **Facing.** It turns to face the head, tilting fully as hand-menu labels do (`Label::tilt`), so it is readable however the controller is held.
- **Smoothing.** Its position follows the controller through a short critically damped filter, about 60 ms, so hand tremor does not shake the text. Its orientation is recomputed each frame from the head.
- **Size.** About 11 × 5 cm, with a font pixel of 1.0 mm. At a typical 45 cm from the eyes, a glyph is about 1.0° tall, the size of the hand menu's text.
- **Appearing.** It grows out of the controller over 120 ms when X is pressed, and shrinks back after the outcome below.

```
            .---------------------------.
            | (o) REC  1:42 / 5:00       |   <- dot pulses; red while recording
            | [##########..............] |   <- progress against the limit
            | "...and then check the CI" |   <- words heard so far, 2 rows
            | Release X to send          |
            '-------------.-------------'
                           \
                       [ X ] [ Y ]           <- left controller
```

**What it shows.**

| State | Shows |
|---|---|
| Recording to the agent | `REC` with elapsed time, and `/ 5:00` when the limit is known; a progress bar against the limit; the last two rows of the partial transcript; "Release X to send" |
| Recording an answer | `ANSWER` in amber with the question's field ("Answering: *which branch?*"), and "Release X to answer" |
| Offline recognizer | No partial words are available (`design/speech.md`: the offline backend shows none), so the transcript rows say "Transcribed when you let go" rather than staying blank |
| Last 30 seconds | The bar and time turn amber with "0:30 left"; in the last 10 seconds red, with a short haptic pulse at 30 s and 10 s (stage 2; see haptics) |
| Released | "Sending…", then "Queued, 2nd" or "Answered" for about 1.5 s, then it shrinks away |
| Failed or too long | Red "Not sent: longer than 5 minutes", or the server's reason. It stays until X is pressed again or B dismisses it, and the problem also goes to the problem line (§3) |
| Live microphone on | A small persistent `LIVE` badge on the controller, with a dot that follows your level. It replaces the `MIC LIVE |` prefix of today's status line (`conversation.rs:2638`) |

**The limit.**

- **Advertised by the server.** The server tells the client its utterance limit in `Welcome`, as `utterance_limit: Option<u32>`, in seconds. It is absent from older servers, and for backends without one.
- **Without it,** the indicator shows elapsed time only, as today.
- **With it,** the existing `RecordingProgress` colours apply, at 30 and 10 seconds from the end.
- **Why not a client constant.** A fixed 5 minutes would be wrong for the streaming backend and for any future one.

**Without controllers.**

- **No X without controllers.** Push-to-talk needs X, so a recording only starts with a controller in hand. The indicator therefore follows whatever is tracking the left hand while it lasts, in this order:
  1. the left **grip pose**;
  2. the left hand's **wrist joint** from hand tracking (`Palm::from_left_hand` already reads it), with the same offset along the back of the hand. This covers a controller that loses tracking mid-recording, and any future pinch-to-talk;
  3. neither tracked: a spot low in the view, placed by a `PanelAnchor` of its own, so that a recording in progress is never invisible.
- **Pinch-to-talk.** The stage 3 gesture (pinch and hold the left thumb and index finger) uses the same indicator at the wrist joint, so it needs no new design.

### The left-hand menu

`design/hand-menu.md` already makes the hand menu the place for settings that move off the panel. This proposal adds two sections and a header, keeping **People & security** as it is. Choosing stays as today: the right ray with the trigger, or A on the highlighted item.

```
  .-------------------------------------.
  | Fix the parser            (o) running|   <- header: space title, activity
  | build  *  Brief speech  *  2 here    |   <- mode, speech, people
  |-------------------------------------|
  | > Space                              |
  | > Agent                              |   <- issue #72
  | > People & security          (!)     |   <- flashes on a knock, as today
  '-------------------------------------'
              [left wrist]
```

- **The header** isn't a button. It is the space's title, wrapped to two rows rather than cut, with the activity and its symbol, the mode, the speech setting and the count of who is here. It is there whenever the menu opens in a space, so a glance at the left palm answers "where am I and what is it doing".
- **Space.**
  - **Title** in full, with the backend and the directory's short name.
  - **Mode:** one row per mode the agent offers, the current one marked. Pointing at a row shows its description beneath it (the menu's equivalent of the description in today's Y list, `menu_body`, `conversation.rs:2394`), and choosing it sends `SetMode` directly.
  - **Speech:** brief or full, for everyone in the space, as today.
  - **On this headset:** mute spoken replies, the local equivalent of the left stick click, and turn the status pill on or off (§4).
  - **Leave space:** a button that asks "Leave Fix the parser?" with *Leave* and *Stay*, so a stray trigger pull doesn't leave. Holding B for a second still leaves, as `design/interaction.md` requires.
- **Agent** (issue #72, being implemented in parallel). This proposal only fixes where the section sits and how it looks, and takes its contents from #72:
  - one row per instruction layer in effect: *Tomori* (built in), *Environment* (such as "Sky: none") and *Space*, the owner's text;
  - choosing a layer shows its full text, wrapped, read-only;
  - only an owner sees a way to change the space layer, in whatever form #72 settles on (it has no keyboard; dictating with X is the natural fit).

  If #72 lands first, its section is kept and the Space section is added beside it.
- **People & security** keeps its two parts, and gains the participant colours of §3: a swatch beside each name, "You" marked, and "away" for a participant whose engagement is `Away`. This is the colour key for the transcript.

**Y.**

- **Today.** Y opens the mode list on the panel.
- **The difficulty.** With the panel gone, Y needs a new meaning. The menu is shown by pose, so a button cannot simply "open" it: it would hide again at once.
- **Recommended: pin.** Y **pins** the left menu open at its Space section, standing above the left controller whatever its angle, until Y or B is pressed. This is an alternative to the gesture for anyone who finds it awkward, and keeps Y meaning "modes and speech". It is an open question.

## 3. The right wrist: the conversation

### Showing it

- **The gesture.** It mirrors the left hand menu, so there is one gesture to learn:
  - **Shown** once the right palm has faced the head, within about **35°**, for about **0.2 s**;
  - **Hidden** once it has turned more than **55°** away;
  - **Lost hand:** hidden when the hand or controller is not tracked.
- **Implementation.** `hand_menu::Visibility` is reused as is. The palm comes from:
  - **Hand tracking:** the right hand's wrist and knuckles. The cross product is mirrored, since for a right hand the index finger crossed with the little finger points out of the palm.
  - **Controller:** the right **grip pose's −X axis**. OpenXR's grip +X points into the right palm and away from the left palm, which is why the left uses +X (`Palm::from_left_controller`).
- **Conflicts with the left menu.** The right hand points at the left menu, and the two gestures can't both be meant at once. So:
  - Whichever shows first wins, and the other waits until it hides.
  - While the right trigger is held (attracting or choosing), the right wrist doesn't show.
  - While the right ray highlights a left-menu item, the right wrist doesn't show.
- **In a space only.** In arrival there is no conversation, and the right hand is pointing.
- **Thresholds.** They start equal to the left's, to be tuned on the headset.
  - The right palm passes through "towards the head" when pointing across the body. The 0.2 s hold and the trigger rule are there for that. If it still shows by accident, the hold rises to 0.3 s for the right.

### Placement, size and legibility

- **Where.** The panel stands just above the right wrist, offset towards the head by about 5 cm along the palm's normal, as the hand menu is (`HandMenu::objects`). It lies on a plane facing the head, refined once from its centre, exactly as the hand menu does.
- **Size.** About **27 × 19 cm**, with a font pixel of **1.0 mm** (the hand menu uses 1.1 mm).
  - That is **28 columns × 14 rows** of content inside a header and footer.
  - At a reading distance of 35–45 cm, a glyph is 1.0–1.3° tall: larger than today's panel (about 0.9°) and in line with the usual 1–1.5° guideline.
  - On the Quest 3 that is about 3.5 display pixels per font pixel, so the bitmap font stays crisp without antialiasing.
  - Bringing the wrist closer enlarges it naturally.
- **Text.** Body text is near-white on a dark backing at about 85% opacity. Colour goes on bars, names, borders and chips, never on body text, so contrast stays high whatever the speaker.
- **Comfort.** Holding the forearm up is tiring beyond half a minute or so, so:
  - The panel is for **glancing and acting**: what happened, what is waiting, deciding.
  - **Squeezing the right grip** while it shows **detaches** it into the world. It stays where it is, upright, and then follows lazily with the existing `PanelAnchor` rules (45°, 0.5 m, 1 s dwell, 0.5 s glide). You can lower your arm and read.
  - Detached, it can be placed about 0.6 m away at the same angular size, and widens to 40 columns.
  - Squeezing the grip again, or B with nothing else to back out of, returns it to the wrist.
  - The right grip is unbound today (`xr.rs:1107–1162`). Squeezes count only while the panel shows, on a firm squeeze (> 0.75) and its release, like the left grip's `LiveGrip`.

### Layout

From top to bottom:

1. a header;
2. the problem line, when there is one;
3. the scrolling transcript;
4. the request card, pinned while an approval or question waits;
5. the queue, pinned;
6. a one-row footer of hints for the focused item.

The header, request card and queue never scroll away.

```
 .------------------------------------------.
 | Fix the parser        (o) running  build |  header: title, activity, mode
 | ! 2m Could not deliver input             |  problem line (red), B dismisses
 |------------------------------------------|
 | |Ada                                     |  violet bar and name
 | |run the tests and fix what fails        |
 | |                                        |
 | |Agent                                   |  cyan bar and name
 | |  [Bash ok] [Read ok] [Edit ...]        |  tool chips
 | |All 214 pass after fixing two parser    |
 | |tests. The cause was an off-by-one in   |
 | |`span_end`.                    [+6 rows]|  collapsed; A expands
 | |                                        |
 | |You                                     |  your own colour, named "You"
 | |  and lint it                           |
 | |              v 1 new below             |  shown when scrolled up
 |=== APPROVAL =============================|  amber card, pinned
 | Bash: npm publish --tag next             |  tinted, as code
 |   [ Allow once                        ]  |
 |   [ Yes, and don't ask again for      ]  |  standing option, full label
 |   [ Bash(npm publish:*) here          ]  |
 |   [ Deny                              ]  |
 |--- Queue (2) ----------------------------|  pinned
 | |You   o  also check the CI          [x] |  hollow: queued
 | |Bob   o  then deploy it             [x] |
 |------------------------------------------|
 | stick: move   A: allow once   B: back    |  footer: what A and B do now
 '------------------------------------------'
```

**Speakers and colours.**

- **The agent** is cyan, the colour the panel already uses for its header row, and close to the swarm's.
- **Humans** each get a colour from a palette of six: violet, green, pink, peach, periwinkle and lime, to be tuned on the headset. The palette avoids cyan, the attention amber (`attention_colour`, `arrival.rs:617`) and the problem red, which keep their meanings.
- **Assignment.**
  - The colour comes from the participant's **public handle** (`Participant::source`, the same number queue entries carry). So it is **the same for everyone** in the space, and the same on every visit while the server keeps the participant, as space hues are (`arrival::hue`).
  - Two people can share a colour in a large space. The name is always shown with it, since colour is never the only signal.
- **You** are shown in your own colour, as the others see you, named "You".
- **Someone typing in OpenCode's own terminal**, which gives no attribution prefix, is a neutral grey, "Someone".
- **Visitors** never appear as speakers; they cannot speak to the agent.
- **Elsewhere.** The same colours are used for queue entries, the swatches in People & security, and, once the headset renders presence, avatars.

**Who said what (MVP).**

- **The prefix.** With attribution on, a user message's text begins with the speaker's display name, a colon and a space (`design/spaces.md`). The client strips that prefix for display, and matches the name against the space's participants and the speakers of queue entries it has seen, which carry handles.
- **Match found:** that speaker's colour.
- **No match**, or attribution off: the name as given, in grey, or "Someone".
- **Stage 2** replaces this guess with a structured speaker on each message (§5).

**Grouping.**

- **One header per run.** Consecutive messages by the same speaker share one header and one bar. The agent's turn, from the first assistant message after a user message to the next user message, is one group. Its tool-only messages fold into its chip row.
- **Space between groups.** A blank half-row separates groups, and nothing pads the layout.

**Messages.**

- **Collapsed.** A group shows at most **6 rows**, ending in `[+N rows]`. The newest agent group, while it streams, shows its **first** rows and grows downwards: it does not follow the tail, which `design/interaction.md` requires today.
- **Expanded.** A on a focused group expands it in place, and the stick then moves through its rows. B collapses it.
- **Brief speech.** The rest of a brief reply, which the server says is "on the panel", is read here by expanding it.

**Styling (Markdown-lite).** The client parses messages with `base-markdown`, which the speech crate already uses to split replies into blocks (`crates/speech/src/speakable.rs`):

- **Headings:** bright, and bold by double-striking, the glyph drawn twice one font pixel apart.
- **Lists:** `•` bullets with a hanging indent.
- **Emphasis:** bold double-struck; italic shown plainly.
- **Inline code and code blocks:** a tinted background, since everything is already monospace. Code blocks wrap with a `↵` continuation mark rather than being cut.
- **Links:** their text only.
- **Tables:** rows of cells separated by `│`, cut per cell.
- **Markers:** none of `**`, `#` or backticks shows.

**Tool activity.**

- **Chips.** A row of small chips under the agent's name, such as `[Bash ok] [Read ok] [Edit ...]`, oldest first. A chip shows a filled tick when done and a pulsing dot while working; its shape, not only its colour, gives the state.
- **Many tools.** More than fit become `+5`.
- **Source.** In the MVP the chips come from the `[Name: state]` lines the server writes for a tool-only message (`space.rs:115`). In stage 2 they come from a structured field, and also show for messages that have text.

**Requests.**

- **Pinned.** A pending approval or question is a card pinned above the queue, in amber, with a border and a title bar ("APPROVAL" or "QUESTION").
  - It never replaces the transcript, so what led to it stays readable above.
  - Opening the wrist UI while one is pending focuses the card.
- **Approvals: one button per choice.**
  - The choices are, in order, *Allow once*, each standing option with its **full** label wrapped over as many rows as it needs, and *Deny*.
  - The stick moves between them and A confirms. Deny is a button like the others, and nothing is decided without A.
  - The card's body shows the approval text, tinted as code where it is a command.
- **Questions** show their title, text and current field, with "Hold X to answer", since X is on the left hand and its indicator shows the answer being heard. A question Tomori can't answer says "Answer on the PC".
- **No reflex decisions.**
  - The 0.6-second request grace (`REQUEST_GRACE`) applies when a request appears, as today, **and** when the wrist UI opens with one pending.
  - Focus lands on the card's title, not on a choice, so reaching *Allow once* takes one deliberate move down.
- **Several pending** (as for workers later) show "1 of 3".
- **Afterwards,** the decided card collapses into a one-row note in the transcript ("You allowed Bash once"). This uses what the client knows locally; who else decided needs the stage 3 timeline.

**The queue.**

- **Rows.** Waiting and sending entries, oldest first, one row each:
  - the speaker's bar and name ("You" for your own);
  - a state shape: hollow for queued, half-filled for sending;
  - the start of the words.
- **Size.** At most four rows; a longer queue scrolls within its strip and says how many there are.
- **Removing.**
  - A on a focused entry starts the existing cancellation. The row shows "Removing… A undoes" with a bar draining over the 3-second undo time (`CANCEL_UNDO`).
  - An entry that starts sending meanwhile says "Already sent".
  - Any participant may remove any waiting entry, as `design/spaces.md` allows.
- **Visitors** see the queue without the remove action.

**The problem line** is the existing one (`problem_lines`, `conversation.rs:2494`): the newest problem with its age, red, on up to two rows, under the header, dismissed by B.

**Following live.**

- **At the bottom,** the transcript follows new messages.
- **Scrolled up,** following stops, and new messages show "v N new below" rather than moving the view.
- **Back to live:** moving to the bottom, or B with nothing open, resumes following.

**The wrist cue.**

- **What it is.** While the right wrist UI is closed and something waits for you (a request, or a problem you haven't seen), a small amber or red glow shows on the right wrist, as the left wrist shows one for knocks (`hand_menu::cue`).
- **What it tells you.** Together with the status pill (§4), it shows that something is waiting, and that you turn the right wrist to see it.

### Controls while the right wrist UI shows

The right thumb is on the stick, A and B whichever way the wrist is turned, so the panel needs no pointing. Moving focus replaces pointing:

| Control | While the right wrist UI shows | Otherwise in a space |
|---|---|---|
| Right stick up and down | Move focus by item: a group, a row within an expanded group, a card choice or a queue entry. Holding repeats after 0.4 s | Pages result cards only, as today (`xr.rs:416`); queue selection moves to the wrist UI |
| Right stick left and right | Jump to the previous or next group | Pages result cards only, as today; approval choices move to the wrist UI |
| A | Act on the focused item: expand or collapse a group, confirm a choice, remove a queue entry, undo a removal | As today: confirm in the hand menu's highlight; otherwise nothing |
| B | Back out: collapse, deselect, dismiss the problem, return to following live, then reattach if detached. **Never** decides, cancels or leaves | As today |
| Hold B for 1 s | Leaves, as today | As today |
| Right trigger | Nothing: the ray is hidden, and the swarm isn't attracted | Point and attract, as today |
| Right grip | Detach or reattach | Nothing |
| Right stick click | Replay the latest reply, or stop it, as today. Stage 2: the focused reply, if it is still kept | As today |
| X, Y, left stick, left stick click, left grip | Unchanged: they belong to the left hand | Unchanged |

**Changes from `design/interaction.md`.**

- **Queue selection** moves from the right stick on the panel to the wrist UI.
- **Approval choices** move from the right stick's left and right to buttons on the card.
- **The left stick** no longer pages a panel, and is left free in a space.

**While the left menu shows,** A chooses only in it, as today (`xr.rs:394`), and the two never show together (above).

**Visitors** (`design/security.md`).

- **What they see.** The whole transcript, the request cards and the queue, read-only.
- **What is hidden.** The choice buttons are replaced by "Waiting for a participant", and the remove action is gone.
- **What still works.** A expands and collapses groups, since reading is not acting. The footer says "You're watching" where it would offer an action.
- **The client sends nothing** a visitor may not send, as `Conversation::send` already refuses (`conversation.rs:1121`).

### With hand tracking

- **Showing.** The panel shows from the right hand's joints exactly as with a controller. Hand tracking has no stick, A or B.
- **MVP: read-only.** It follows live and shows the request card with "Decide with a controller or on the PC".
- **Stage 3: poke and pinch.** The left index fingertip **pokes** rows and buttons, which are at least 2 cm tall for that reason, and a pinch-drag scrolls. This is the same near-touch model the left menu would gain.

## 4. What happens to the floating panel

**Recommendation: shrink it to a glanceable status pill, and let the wrist UI detach for long reading.** Remove the wall of text, and keep the panel's one job no wrist can do: being visible without a gesture.

```
              .----------------------------------.
              | (o) running  build  Fix the parser |
              | ! Approval waiting: turn right wrist|   <- only when something needs you
              '----------------------------------'
                    (low in view, about 20 degrees below eye level)
```

- **Why not remove it entirely.** The wrist UI is out of sight by design. An approval or a problem arriving while you look at the swarm or a worker would be missed, and spoken replies are heard but approvals are silent. The pill, the wrist cue and later haptics together make "something needs you" impossible to miss without bringing back the wall of text.
- **Why not keep the full panel as an option.**
  - Two full presentations of the same conversation would double the layout and the tests.
  - The detached wrist UI already *is* a world-anchored conversation panel, on demand and with the same styling.
- **Placement.** The existing `PanelAnchor`, placed lower: about 20° below eye level, 1.6 m away.
- **Size.** At most 2 rows of 34 columns, at today's 3.4 mm font pixel, about 1 m wide.
- **Visibility.** It hides while either wrist UI shows, as the panel hides for the hand menu today (`xr.rs:622`). The left menu's Space section can turn it off.
- **Row 1** is always shown:
  - the activity, with its symbol;
  - the mode;
  - the title, cut to fit, since the full title is in the left menu;
  - `Replaying` while a replay plays.
- **Row 2** appears only when something needs you, in order:
  1. a pending approval or question;
  2. the newest unseen problem;
  3. the connection ("Offline, retrying in 4 s");
  4. "Leaving: keep holding B";
  5. a notice.

  Each says where to look.

**Where today's panel rows go.**

| Today on the panel | Moves to |
|---|---|
| Title, activity, mode (row 0) | Pill row 1; wrist UI header; left menu header |
| `MIC AGENT`, `Sending…`, partial words | Left controller recording indicator |
| `MIC LIVE` | `LIVE` badge on the left controller |
| Connection, notices, "Leaving: keep holding B" | Pill row 2 |
| Counts, `Muted`, `Brief` | Left menu header; Space section |
| Problem line | Wrist UI problem line; pill row 2; wrist cue |
| Message pages | Wrist UI transcript |
| Approval choice, question | Wrist UI request card; pill row 2; wrist cue |
| Queue strip | Wrist UI queue |
| Y list of modes and speech | Left menu Space section |
| Footer hints | Wrist UI footer for the focused item; the recording indicator's own hint |
| Recording timer above the panel | Left controller recording indicator |

**The console and desktop.**

- **The console** prints `Conversation::text()` (`console.rs:99`), and many tests assert on it. The new interface is built as a **view model**, rows of styled spans, and a plain-text rendering of that view model is what the console prints. The console keeps working and the tests keep a text form to assert on.
- **New console commands,** following the existing `m` command for People & security:
  - `t` prints the transcript view;
  - `s` the Space section;
  - `a` the Agent section.

  `design/hand-menu.md` requires equivalents without hands.

## 5. Rendering and technical feasibility

### What the renderer has now

- **Two text paths, both `font8x8` bitmaps drawn as quads.** There is no font file, glyph atlas or text texture.
  - **`Caption`** (`caption.rs`):
    - vertices are 2D positions in the panel's local space, with RGB colours;
    - one transform uniform per eye;
    - no depth and no alpha;
    - the mesh is rebuilt only when the text changes, and moving the panel costs one uniform write per frame.
  - **Arrival labels** (`arrival_graphics.rs:368`):
    - vertices are in **world** space, RGBA with alpha blending (`arrival_graphics.rs:547`), drawn in the main pass with depth;
    - lit pixels in a row are merged into one quad;
    - the mesh is rebuilt whenever anything moves. The hand menu's rows are rebuilt every frame while it follows the hand.
- **Monospace only.** Glyphs are 8 × 8 on a 9 × 11 cell. There is no proportional font, antialiasing or emoji, and Unicode is limited to the `font8x8` tables. Of those, only `BASIC_FONTS` is used, while `LATIN`, `BOX`, `BLOCK`, `GREEK`, `HIRAGANA` and `MISC` are in the same crate.

### What is feasible without new dependencies

| Wanted | How | Effort |
|---|---|---|
| A colour per speaker, bars, chips, borders, card backgrounds | Per-vertex colour is already there; these are quads | Small |
| Translucent backing | RGBA vertices and alpha blending, as arrival labels have | Small |
| Headings | Double-struck bold, or a 1.25× font pixel for that row | Small |
| Code | A tinted background behind monospace text that is already monospace | Small |
| Accents, `•`, `→`, box drawing, block bars | Look up `LATIN_FONTS`, `BOX_FONTS`, `BLOCK_FONTS` and `MISC_FONTS` after `BASIC_FONTS`, falling back to `?` | Small |
| Icons: tick, dot, hollow and half-filled circles, `x` | Drawn as quads, as `status_symbol` does for worker stations (`workspace_graphics.rs:1073`) | Small |
| Scrolling | By whole rows, so nothing needs clipping. Smooth scrolling later needs a clip rectangle, a discard in the fragment shader against local bounds | Small, then small |
| Occlusion by the hand and controller | Draw in the main pass with the depth buffer, rather than after it without one | Small |
| Markdown structure | `base-markdown`, already a dependency of the speech crate | Moderate |

### What has to be added

1. **A local-space panel renderer** (`ui_panel.rs`), generalised from `Caption`:
   - **Vertices:** 2D local positions with RGBA colours, alpha-blended and depth-tested in the main pass.
   - **Drawing:** one model matrix per frame; the mesh is rebuilt only when the content or focus changes.
   - **Instances:** the wrist UI, the recording indicator and the pill each use one. `Caption` is retired with the panel.
   - **Cost:** about 28 × 14 glyphs at roughly 20 merged runs each is about 8,000 quads, or 48,000 vertices of 24 bytes, about 1.2 MB. That is rebuilt only on change, unlike the hand menu's world-space rebuild every frame.
2. **A layout module** (`ui_layout.rs`), free of rendering code:
   - **Input:** styled spans (text, colour, weight, background), with a width in columns.
   - **Output:** wrapped rows with hanging indents, and continuation marks for code.
   - **Fit:** it reports the rows used, so the view can enforce its budgets.

   It replaces `wrapped_lines`, `short` and `paginate`.
3. **The view models**, also free of rendering code:
   - `transcript::View::of(&SpaceState, &local) -> Vec<Row>`, holding the groups, cards, queue and focus;
   - `recording::View`;
   - `pill::View`;
   - the new hand-menu sections.

   They are built from `Conversation`'s existing state. They also feed the console's text.
4. **The right palm and the right wrist's visibility** (`hand_menu::Palm::from_right_*`), reusing `Visibility`.
5. **The right grip action**, and in stage 2 a **haptic output action** on both controllers (`/user/hand/*/output/haptic`); no haptics exist today.
6. **Protocol, MVP:** `ServerMessage::Welcome.utterance_limit: Option<u32>`, with `#[serde(default)]`.
7. **Protocol, stage 2**, all additive with `#[serde(default)]` so that older peers ignore them:
   - **`Message.speaker`:** `Option<{ source: Option<u32>, name: String }>`. The server sets it for user messages it delivered from a queue entry, matching the delivered, attributed text, and leaves it absent otherwise.
   - **`Message.tools`:** `Vec<{ name, state }>`, from the `ToolModel` the server already builds (`space.rs:143`), sent whether or not the message has text.
   - **`Role` is closed** (`space.rs`, no `#[serde(other)]`). So stage 3 timeline events need their own field, as the interaction proposal noted.

### Later: better text

- **Stage 3.** Antialiased, proportional and wider-Unicode text needs a rasterised atlas: TrueType glyphs rasterised at run time into a texture, with signed-distance fields for scale.
- **Not needed first.** The 8 × 8 bitmap is crisp at the sizes above, and every layout here is designed for a monospace grid, so the switch changes the renderer and not the layout.

## 6. Staging

### Stage 1: MVP

Client work, apart from one protocol field.

1. **The recording indicator on the left controller.**
   - Anchored to the grip pose, with the hand and view fallbacks.
   - Shows the timer, partial words or "transcribed when you let go", the release hint and the outcome.
   - A `LIVE` badge for the live microphone.
   - `utterance_limit` in `Welcome`, sent by the server; the warnings appear when it is known.
2. **The right wrist UI.**
   - The gesture, mirrored, with the conflict rules.
   - The styled transcript: speaker colours from the attribution prefix, grouping, collapsed and expanded groups, Markdown-lite, and tool chips from today's tool lines.
   - The pinned request card with one button per choice.
   - The pinned queue with removal and undo.
   - The problem line, following live, the wrist cue, detaching with the right grip, and visitor gating.
3. **The status pill** in place of the panel, with its two rows and an off switch.
4. **The left menu:**
   - the header;
   - the Space section, with mode, speech, mute, pill and a confirmed leave;
   - the participant swatches in People & security;
   - Y pinning the menu, if agreed;
   - the Agent section's place, filled by #72.
5. **The renderer:** the local-space panel renderer, the layout module and the wider `font8x8` tables.
6. **The console:** text renderings of the new views, and the `t`, `s` and `a` commands.

### Stage 2: richer facts and feedback

- **Structured facts.** `Message.speaker` and `Message.tools` from the server; the client stops guessing from the prefix, and shows chips on every agent message.
- **Speaking now.** `ServerMessage::Speech` names the message being spoken (`message`), but the headset only logs it today (`conversation.rs:723`). Its group's bar pulses while it is spoken: captions for free.
- **Replaying a chosen reply.** The right stick click replays the focused reply with `Replay { message: Some(id) }`, which the server already accepts for the replies it keeps (`hub.rs:2290`), or says it is no longer kept.
- **Haptics:** recording start and stop, the 30 s and 10 s warnings, a request arriving, and a choice confirmed.
- **Avatars.** Participant colours are shared with avatars once the headset renders presence.

### Stage 3: hands, text and history

- **Hands.** Poking and pinch-scrolling with hand tracking, and pinch-to-talk on the left hand using the same indicator.
- **Text.** An antialiased, proportional and Unicode font atlas.
- **Timeline.** Events between messages, such as "Ada allowed Bash once" or "Mode → plan (Bob)", which need a protocol field of their own.
- **The queue.** Reordering and merging entries, which need server support (`MoveQueued`).

### Test strategy

Most of the new code is pure functions of poses, time and state, and is tested the way `hand_menu.rs` and `panel_anchor.rs` are.

- **Visibility.**
  - **Hysteresis:** the right palm, from joints and from the grip's −X axis, shows after the hold within 35°, stays to 55°, and does not show for a hand passing through.
  - **Conflicts:** whichever menu shows first wins; the right wrist does not show while the trigger is held, or while the ray highlights a left-menu item.
- **Recording placement.**
  - **Position:** for given grip poses, the indicator is above and in front of the controller, and faces the head.
  - **Fallbacks:** losing the grip pose moves it to the wrist joint, then to the view anchor.
  - **Smoothing** bounds jitter for a trembling pose.
- **Recording content.**
  - **States:** each state shows its rows: answering, offline, sent, failed and too long.
  - **Limits:** with a limit, the warnings come at 30 s and 10 s; without one, the label is unbounded.
- **Transcript view.** With table-driven `SpaceState`s:
  - **Speakers:** the prefix is stripped and matched to a handle and colour, an unknown name is grey, and attribution off gives "Someone".
  - **Grouping:** consecutive messages group, and a tool-only message folds into its turn's chips.
  - **Collapse and streaming:** a collapsed group stops at 6 rows, and a streaming reply shows its start.
  - **Pinning:** the request card and queue are pinned and never scrolled away.
  - **Fit:** every row fits 28 columns, and every row budget holds with a problem, a long standing option and a full queue together.
- **Controls.**
  - **Routing:** while the wrist UI shows, the right stick, A, B and the right grip go to it, and the trigger does nothing.
  - **No accidental decisions:** B never decides, cancels or leaves; A is ignored for 0.6 s after a request appears **and** after opening with one pending; and focus lands on the card's title.
  - **Removal:** removing an entry sends nothing until the undo time ends, and A undoes it.
  - **Visitors** get no choice or remove actions, and send nothing.
  - **Existing tests:** the tests in `design/interaction.md` keep passing against the new view, adjusted only where that document changes.
- **Hand menu:** the header, Space and Agent rows against client states; the leave confirmation; and Y pinning.
- **Renderer:** meshes stay within their vertex bounds and local extents for the longest content, as `caption.rs` tests today. Glyph lookups fall back to `?` for characters outside the tables.
- **Console:** snapshot tests of the `t`, `s` and `a` output.
- **On the headset.** These are the checks that unit tests can't make:
  - legibility at the proposed sizes;
  - the right gesture's false positives while pointing and attracting;
  - arm comfort, and detaching;
  - the indicator's position for different grips;
  - the pill noticed when an approval arrives while facing away.

### Risks

- **Arm fatigue.** Reading on the wrist tires the arm. *Mitigation:* the wrist UI is for glancing and deciding, collapsed groups keep it short, and the right grip detaches it.
- **Missed requests.** The wrist UI is hidden by default, so a request can go unseen. *Mitigation:* the pill's second row, the wrist cue, and stage 2 haptics. The pill can be turned off only knowingly, from the Space section.
- **False shows on the right.** Pointing across the body turns the right palm. *Mitigation:* the hold, the trigger rule, the highlight rule, and a longer hold for the right if needed.
- **The 8 × 8 font at wrist size.** Legibility depends on the headset's pixels per degree. *Mitigation:* a 1.0 mm font pixel is within the hand menu's proven range, and the font pixel is one constant to tune.
- **Guessing speakers from the prefix** fails for duplicate names and for text that happens to begin with a name and a colon. *Mitigation:* a guess colours only a known participant name, and stage 2 makes it structured.
- **Overlap with #72**, which edits `hand_menu.rs` at the same time. *Mitigation:* this proposal takes the Agent section's contents from #72 and only adds sections beside it. Whichever lands second rebases.
- **A large change to `Conversation`'s text paths**, whose tests assert on panel strings. *Mitigation:* the view-model and console text keep the tests' form, and the change can land behind the pill in two steps (wrist UI first, then removing the panel).

## Open questions for you

1. **The floating panel.** Shrink it to the two-row status pill, with the wrist UI detachable for long reading, as recommended? Or keep a full panel as an option?
2. **The right gesture.** Mirror the left (right palm towards your face), as recommended? Or a watch check, with the back of the right wrist towards you?
3. **Y.** Pin the left menu open at Space until Y or B, as recommended? Or keep Y as its own mode list somewhere?
4. **The right grip.** Bind it to detach and reattach the wrist UI?
5. **Colours.** The same colour for a person on everyone's headset, from their handle, as recommended? Or "You" always one fixed colour on your own headset?
6. **"More on the panel."** Brief speech says this when it cuts a reply (`design/speech.md`). Change it to something like "More to read on your wrist", or the neutral "More to read"?
7. **Hand tracking.** Is read-only acceptable for the MVP, with poking in stage 3? Or is controller-free use a priority?

## Design changes this would make

None of these are made by this proposal. If it is agreed:

- **`design/interaction.md`.**
  - Replace "The panel stands in the world" with the status pill and the detachable wrist UI.
  - Move "The queue is always visible" and "Problems stay visible on their own line" to the wrist UI, keeping their rules.
  - Replace "Paging uses the rows that are left" with the transcript's grouping, collapsing and pinning.
  - Update the controls table: the right stick, A, B and the right grip while the wrist UI shows; queue selection and approval choices on the wrist; the left stick freed; Y pinning the menu. The safety rules stay as they are.
- **`design/hand-menu.md`.**
  - Add the header, the **Space** section and the participant swatches.
  - Describe the left controller's recording indicator and `LIVE` badge.
  - Add the right wrist counterpart: either here, as "hand menus" for both hands, or in a new `design/wrist-transcript.md`. The latter is recommended, since the right wrist is a conversation view rather than a menu.
  - The **Agent** section is added by #72.
- **`design/speech.md`:** the "More on the panel." wording, if changed; and that the utterance limit is advertised in `Welcome`.
- **`design/live-audio.md`:** the live microphone's state is shown on the left controller.
- **`design/spaces.md`:** participant colours from the public handle, the same for everyone (stage 1). Messages carrying a structured speaker and tool calls (stage 2).
- **`design/security.md`:** no change. The wrist UI and menus follow its rule that a client hides what a role does not permit.
- **`design/spectator.md`:** no change. The spectator shows no panel, and the wrist UIs are the participant's own.
- **`design/clients.md`:** no change. The new view models are free of rendering code, as the shared scene model is.
