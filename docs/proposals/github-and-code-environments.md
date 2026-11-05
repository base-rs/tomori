# Proposal: A GitHub Environment, and Understanding Code in the Space

Status: **a proposal for discussion**, November 2026. Nothing here is a requirement. It captures a conversation with the maintainer, turns it into experiments, and is tracked by issues to come back to later. The first crates.io release (#73) comes first. Its companion, [A Data Center Environment for Running Code](data-center-environment.md), is about the running code, and hands off to this one when the agent offers to make a pull request.

It builds on the environment framework proposal (PR #90, `context/environments.md`): environment crates (#75), state synchronization (#76), the generic scene and renderer (#77), interaction (#78), anchors (#79), the UI kit (#80), shared attention (#81), agent projection (#82), data sources (#83) and physics (#97). The client split (#132) gives it the `tomori-scene` and `tomori-render` crates to build on.

## Issues

| Issue | What |
|---|---|
| #148 | A GitHub environment: an inbox of issue and pull request cards |
| #149 | Triage boxes that apply labels or types on GitHub, with confirmation |
| #151 | Code panels, span selection and "explain this" |
| #152 | Structure and relationship views from Rust analysis |
| #147 | A shared code visual toolkit |
| #153 | Experiment: the surround view |
| #154 | Experiment: guided tours and the time dial |
| #155 | Experiment: code as a place, and following the flow |
| #146 | Text rendering: a glyph atlas, antialiasing and Unicode, and the hand menu's text bugs |

## The discussion

Spoken by the maintainer from the headset, November 2026, with suggestions from the coordinating agent marked as such.

### 1. A GitHub environment, first

- The maintainer's own workflows centre on GitHub, so **GitHub comes first**, and plain Git later.
- Pull requests and issues get a physical manifestation. An **inbox** is literally a box full of issue and pull request cards, which the maintainer can look through and organise in the space with the pointer.
- The **physics engine** (#97: server-authoritative Rapier, with grabbing and throwing) lets them pick cards up, flick them onto piles and stack them, to help triage and manage issues.
- Each card's face shows the **title, number, author, CI status and age**.
- **Gestures must be able to be real actions.** For example, there are **boxes for Bug, Feature and so on**, and dropping a card in a box actually applies that classification on GitHub.
  - *The coordinator suggested* writing back to GitHub only through a deliberate, confirmed gesture, with plain layout (piles, stacks, positions) saved with the space and never sent to GitHub.
  - *The maintainer wants* actions to be possible. Both views are captured; the recommendation is under [Write-back and confirmation](#write-back-and-confirmation).
- **The server holds GitHub access** and syncs the cards, so the headset never sees credentials.

### 2. Drilling into an issue: understanding, not editing

- From an issue, saying **"show me how that works"** makes the agent pop up source code in panels.
- The maintainer can **highlight** a function, a keyword or a block of code with the ray, and ask **"explain this"** or **"why is it like this?"**. The selected span reaches the agent as attention context: shared attention, #81.
- **Structure views:** "show me how these modules are organised and relate to each other". The agent projects a subset of the relationships (agent projection, #82), using Rust analysis tooling: rust-analyzer, `cargo metadata`, call hierarchy, module trees and crate dependencies.
- **The key principle**, in the maintainer's words: *"I don't want to edit code in VR. I'd rather talk about it and have you do it."*
  - It should not replicate a 2D code editor.
  - AI agents do more, faster. Humans guide the high-level design, and have feelings and ideas about how things should look.
  - The question is what interaction 3D space enables that a laptop or monitor can't. The maintainer doesn't have the answers, and wants to explore and try strategies.

### 3. VR-native ideas, as experiments

Suggested by the coordinator, to be tried rather than built outright.

- **Code as a place.** Crates are districts and modules are buildings, sized by their code, with a heat glow for recent churn or failing tests. It relies on spatial memory ("the auth stuff is over there"), and shows where in the city the agent's changes land.
- **Following the flow.** A request or a datum is a path you walk along, with each function a station in sequence and the agent's voice spatialised from there. For security review, trust boundaries are literal walls and untrusted input is a coloured stream, so you see where it crosses a wall without a check.
- **Everything around you at once.** Focusing a function gathers its callers to the left, its callees to the right, its tests behind, and its history below like tree rings. You glance rather than navigate. Hands pull a piece out, push irrelevant parts away, and gather related bits into a cluster to ask about the group.
- **Guided tours.** For a pull request or a security issue, the agent walks you through stations, pointing with its own ray, and you interrupt with "why?" while pointing. A **time dial** that you twist makes a code region morph through its git history.

### 4. A shared abstraction

The GitHub and code environments might share a crate or visual toolkit for rendering code: highlighted source panels, span selection and graph layout, perhaps as part of the standard UI kit (#80).

### 5. Text rendering

The headset's text rendering needs improving: a proper font atlas, antialiasing and Unicode, which overlaps with Japanese text in #103. Code panels need it. The left-hand menu's text "could be better; some parts of it are buggy". It is not high priority, but it is a prerequisite for code panels. Concrete findings from the current code are under [Text rendering today](#text-rendering-today).

### 6. Process

Not everything at once: capture it all, cut issues, and come back to them later. The first crates.io release comes first.

## Recommendation in brief

- **Two kinds of thing, kept apart.**
  - The **GitHub environment** (`environment-github`) is an environment crate (#75): a place with an inbox, cards, piles and boxes.
  - **Code panels and structure views** are the agent's projections (#82), shown wherever "show me how that works" is said, in the GitHub environment or any other. They are not an environment of their own.
  - The bigger spatial ideas (the city, the flow walk) would become a code environment later, if the experiments earn it.
- **One shared toolkit** for source panels, spans, selection and graph layout (#147), used by both, and part of the UI kit (#80).
- **Layout is free; write-back is deliberate.** Moving, piling and stacking cards changes only the space. Dropping a card in an action box makes a *pending* change shown on the card, which is applied to GitHub only when a participant confirms the box's batch. See [Write-back and confirmation](#write-back-and-confirmation).
- **Credentials stay on the server.** The environment's server half holds the GitHub token as a data source (#83). Clients receive only card data; the agent receives only resource identities and descriptions.
- **Experiments, in order:** the inbox with layout only, then code panels with "explain this", then triage boxes with write-back. The rest follow from what those teach. See [Experiments](#experiments).

## What exists today

From `origin/main` at `064c4e2`.

- **No environment has state.** Sky and the Minimal grid are scenery; an owner can now change a space's environment from the hand menu (`design/environments.md`). Environments are registered through the server builder (`design/builders.md`), but server halves, synchronization and the generic scene (#75, #76, #77) are not built.
- **Pointing at workers** is a prototype of shared attention (`design/interaction.md`): the right ray highlights a worker, and the worker pointed at as X is pressed is sent as the words begin, for the server to describe to the agent before them. Nothing else in a space can be pointed at.
- **Panels.** `ui_panel` draws local-space panels of styled monospaced text (`ui_text`) with depth testing, used by the right wrist's transcript. They are the nearest thing to a code panel today.
- **Safety rules.** `design/interaction.md` says nothing is decided by reflex: choices take **A**, a cancellation waits 3 seconds during which **A** undoes it, and a new request ignores **A** for 0.6 seconds. Write-back to GitHub should follow the same rules.
- **Roles.** Owners and collaborators are participants; visitors only watch (`design/security.md`).
- **Rendering** is wgpu with 4× MSAA where the GPU supports it (`graphics::SAMPLES`), and text, when this was written, was `font8x8` bitmaps drawn as one quad per run of lit pixels. It is now drawn from a signed-distance-field glyph atlas of bundled Noto fonts, a quad per glyph, with Japanese taking two cells (`design/text.md`).

## The GitHub environment

### The inbox and its cards

- **The inbox** is a box in the space filled from a **query**, such as a repository's open issues and pull requests, or a search like `repo:ioquatix/tomori is:open no:label`. An owner sets it when creating the space or later. GitHub notifications and review requests are later sources.
- **A card** is a flat body, about the size of an index card, faced with:
  - the title, wrapped to two lines;
  - the number and kind: issue, pull request or draft;
  - the author;
  - for a pull request, the CI status of its head commit, as a coloured mark (passing, failing, pending or none);
  - the age, such as "3d";
  - its labels, and any pending change (below).
- **Cards are resources (#81).** Each has an identity such as `github://ioquatix/tomori/issues/97`, and a description the server half supplies: title, state, labels, author, a bounded excerpt of the body. Pointing at a card and saying "what's this about?" or "show me how that works" gives the agent the card. The agent fetches more with its own tools.
- **Handling cards.**
  - Before physics: point and pull the trigger to pick up a card, which follows the ray, and release to put it down. Cards snap into piles when released near one.
  - With physics (#97): grab, flick and throw cards; they land, slide and stack. Two participants can't hold the same card.
  - Hands push irrelevant cards away, and gathering cards into a pile makes the pile a selection the agent can be asked about ("what do these have in common?").
- **Layout is the space's own.** Positions, piles and stacks are environment state (#76), kept with the space across restarts. A card that closes on GitHub dims and moves to a *done* tray rather than vanishing, so the layout is not disturbed while someone is looking.
- **Arriving with context.** When another environment hands the space off to this one, such as the data center after "I found why. Would you like me to make a pull request?", its finding stands at the `presentation` anchor, code panels open at the lines it names, and the pull request the agent opens arrives as a card. See the companion proposal's [hand-off](data-center-environment.md#cross-environment-hand-off).
- **Limits.** The inbox shows at most a bounded number of cards (say 200), oldest-updated last, so physics and synchronization stay within their budgets.

### Write-back and confirmation

The maintainer wants gestures to be real actions; the coordinator suggested write-back only through a deliberate, confirmed gesture. **Recommendation: the gesture is the action, and the commit is deliberate.** Dropping a card in a box is how you say what should happen; confirming the box's batch is how it happens.

- **Two kinds of place.**
  - **Piles** are plain: anything on them stays in the space and never reaches GitHub.
  - **Action boxes** are distinct objects, labelled with their exact effect, such as "Label: bug" or "Type: Feature", in the label's colour.
- **Dropping makes a pending change.**
  - A card placed or thrown into an action box sits in the box's tray, marked with the change it will make and a pending ring. A short haptic tick confirms the drop. Nothing is sent to GitHub.
  - Taking it back out removes the pending change.
  - A throw that lands in a box by accident is therefore harmless.
- **Confirming applies the batch.**
  - Each box shows how many changes wait. Choosing **Apply** on the box, or the hand menu's Environment section (#80), opens a short summary ("Label 4 issues bug: #97, #103, …"), confirmed with **A**, as `design/interaction.md` requires.
  - Saying "apply those" asks the agent, which can only open the same summary; a participant still confirms.
  - Applying is visible to the space: each card shows its new label, and the transcript notes "Samuel labelled 4 issues bug".
- **Undo.** For a few minutes after applying, the summary offers **Undo**, which applies the inverse (removing the label, restoring the previous type).
- **Who.** Owners by default; an owner can allow collaborators in a space. Visitors never. The agent can suggest triage by placing cards in boxes as pending changes, but never confirms.
- **What a box can do, at first.** Add or remove a label, and set the issue type where the repository has issue types. GitHub issue types exist only for organisations; a personal repository such as `ioquatix/tomori` has none (its `issueTypes` is null), so its boxes use labels such as `bug` and `enhancement`. Closing, assigning and milestones come later; commenting and merging are never gestures.
- **Conflicts.** Before applying, the server checks each issue is unchanged since it was synced. A change that no longer applies, such as a label already present or an issue closed meanwhile, is skipped and shown on its card; the rest are applied.
- **An experiment variable.** Once the batch works, try an **undo window** for a box instead: each drop applies after a few seconds unless the card is pulled out, like removing a queued entry today. If it proves safe in use, it is a per-box choice; the batch stays the default.

### Credentials and sync

- **GitHub access is a server-side data source** (#83), configured on the server, with a pluggable token provider as #83 has for Prometheus: a fine-grained personal access token, the output of a command such as `gh auth token`, or an environment variable. A GitHub App installation, with short-lived per-repository tokens, is the better long-term choice.
- **Scopes:** read issues, pull requests, checks and metadata; write issues only when an owner enables write-back for the space.
- **Never on clients.** The token lives in server configuration or the state directory, readable only by the server's user. It is never in space state, the protocol, agent instructions or logs. Clients receive card data only.
- **Sync.** The server half queries GitHub while someone is interested (#76), batching issues, pull requests and their check status in one GraphQL query, about once a minute, with conditional requests so that unchanged results cost little of the rate limit. Webhooks need a public endpoint and come later, if ever.
- **Visitors.** Like shared windows in #85, private repositories' cards are not shown to visitors by default.
- **The agent** uses its own GitHub access (such as `gh` in OpenCode) and never receives the server's token. Tomori gives it only identities and descriptions.

## Code in the space

### Code panels and "explain this"

- **Opening.** The agent shows source with a tool, such as `show_code(path, lines)` among Tomori's tools (#82). The server reads the file from the space's directory, within the workspace roots (`design/workspaces.md`), bounded in size, highlights it, and places a panel at the agent stage (#79).
- **Selecting.** The ray selects a span: a line or block by sweeping with the trigger held, an identifier by pointing at it, a function by pointing at its signature. The selection shows on every headset, labelled with who made it.
- **Asking.** While a selection or pointing exists, speech carries it as attention context (#81): the file, the line range, the selected text (bounded), and the enclosing item where known, such as `fn wrap in crates/client-quest/src/ui_text.rs`. "Explain this" and "why is it like this?" need nothing else.
- **No editing.** Panels are read-only. Changes are asked for in speech and made by the agent, which shows the result in the panel as a diff.

### Structure and relationship views

- "Show me how these modules are organised and relate to each other" makes the agent project a **subset** of relationships, chosen for the question, not the whole graph.
- **Sources,** run on the server:
  - `cargo metadata`: packages, features and the crate dependency graph;
  - rust-analyzer as a language server: document symbols, the call hierarchy (incoming and outgoing calls), references, implementations and parent modules;
  - `rust-analyzer scip`, an offline index for whole-workspace questions without a running server;
  - module trees from the source tree, or `cargo-modules`.
- **Shown** as a diagram (#82's `show_diagram`, laid out in 3D layers), whose nodes are resources: pointing at a module and asking about it works like a card. Focusing a node can open its code panel.
- **The agent decides the subset;** Tomori provides the data as tools and the drawing.

### The shared code toolkit

Both environments, and the agent's projections, use one toolkit (#147), part of the UI kit (#80):

| Part | What it does | Where |
|---|---|---|
| Source model | Files, byte and line ranges, spans with identities | Shared |
| Highlighting | Token classes from tree-sitter, styled by a palette | Server |
| Code panel | Highlighted lines with a gutter, wrapping, scrolling, diff marks and highlights | `tomori-render` |
| Span selection | Ray to line, column and token; sweeping; shared selections | Client and #81 |
| Graph layout | Layered (Sugiyama-style) and radial layout in 3D, edge routing | Shared |
| Rust analysis | `cargo metadata`, rust-analyzer and SCIP adapters | Server |
| History | `git log -L` for a range, blame and churn | Server |

## Text rendering today

A code panel needs crisp, small, mostly ASCII text, many characters at once, and wider Unicode for comments and names. Reading `crates/client-quest` at `064c4e2` found these:

**Bugs in the hand menu's text**

1. **Tilted labels draw striped glyphs on a backing that falls short.** The hand menu's labels tilt to face the head (`tilt: true` in `hand_menu.rs`), but `label_into` in `arrival_graphics.rs` builds each quad's height along world down (`- Vec3::Y * (v * h)`) rather than the label's own `up`, while placing rows along `up`. Seen from the head, a label tilted by θ draws each pixel row cos θ tall at full row spacing, leaving gaps between rows: about 30% of each row at 45°. Its backing is shortened by the same factor, so it doesn't reach the text's last rows. Only the hand menu and its cues tilt; arrival's upright labels are unaffected. The fix is to use `up` for the quad's height. Confirm on the headset before and after.
2. **The menu shows `?` for anything outside ASCII.** Its labels use only `font8x8`'s basic table, while the wrist transcript's `ui_text` also uses the Latin, Greek, box, block and symbol tables. A space title, a mode or a person's name such as "José" shows as "Jos?" in the menu, while the wrist transcript draws it correctly.
3. **Three wrappers with different rules.** The menu wraps titles and mode descriptions at 32 columns with its own `wrap`, which silently drops the rest of a word longer than the row. Instruction layers and errors wrap at 40 columns with `caption::wrapped_lines`, which turns non-ASCII into `?`. The transcript uses `ui_text::wrap`. So rows in the same menu differ in width and coverage.
4. **Many menu rows are not bounded at all:** the status line, buttons with names ("Let … in"), the pairing label, the list of people, notices and mode names. A long one runs far past the menu's other rows.

**Limits of `ui_text`, which code panels would inherit**

5. **A tab becomes one space** in code, so tab-indented code loses its indentation; it should advance to the next tab stop.
6. **Control characters take an invisible cell.** `font8x8` has blank glyphs for them, so `shown` keeps them: a carriage return from CRLF text takes a cell, and an ANSI escape shows as `[31m` after a gap.
7. **Combining marks and variation selectors take a cell each,** drawn as `?`: "✓️" shows as "✓?", and a decomposed "é" as "e?". Characters are cells, not graphemes.
8. **A non-breaking space breaks lines,** since `wrap_words` splits on `char::is_whitespace`, which includes U+00A0, though `shown` draws it as a space.
9. **One cell per character,** so CJK (#103) can't be double width.

**Rendering cost and quality**

10. **Glyphs are geometry.** Every run of lit pixels is a quad of six vertices, so a glyph is about 50 to 80 vertices. A code panel of 80 by 40 characters would be about 200,000 vertices, against 6 per glyph with an atlas. The arrival labels cap their buffer at 2¹⁹ vertices and silently drop quads beyond it.
11. **No filtering.** MSAA smooths the quads' edges, but an 8 × 8 bitmap at a millimetre per pixel, half a metre away, shimmers as the head moves, and there is one weight (bold is drawn twice, a pixel apart) and one size.

**Proposal (#146):** fix 1 to 4 now, independently; they are small. Then one text system in `tomori-render`, as #77 already plans: a multi-channel signed-distance-field glyph atlas generated from a font with wide coverage (a monospaced font for code, a proportional one for UI), shaped runs with grapheme clusters and double-width cells, tab stops, control characters stripped, and one wrapper with consistent rules for every panel and label. Japanese (#103) needs a CJK font and line breaking without spaces; the atlas should grow on demand rather than hold every glyph.

Since then, #169 fixed 1 to 4, and the glyph atlas (`design/text.md`) fixed 6 to 11, apart from tab stops (5): graphemes take one cell, CJK two, a non-breaking space joins words, and a glyph is one antialiased quad. Its atlas is single-channel rather than multi-channel, and lives in `tomori-text`, below `tomori-render`, so that `tomori-scene` can lay text out with it.

## Experiments

Each is small, answers one question on the headset, and can be abandoned. They are ordered by value and feasibility.

| # | Experiment | Value | Feasibility | Effort |
|---|---|---|---|---|
| 1 | Inbox cards, layout only | High: the maintainer's own triage | Needs #75, #76, a grab | Medium |
| 2 | Code panel and "explain this" | High: the core principle | Needs a panel tool and #81's capture | Medium |
| 3 | Triage boxes with confirmed write-back | High, once 1 works | Small on top of 1 | Small |
| 4 | Guided tour of a pull request | High for review | Builds on 2 and spatial audio | Medium |
| 5 | Surround view | Medium to high | Builds on 2 and rust-analyzer | Medium |
| 6 | Structure projection | Medium | Needs #82's diagram | Small to medium |
| 7 | Time dial | Medium | Builds on 2 and `git log -L` | Small |
| 8 | Code city | Uncertain | Needs instancing in #77 | Medium |
| 9 | Flow walk and trust boundaries | Uncertain, high if it works | Needs locomotion (#78) and authored paths | Large |

### 1. Inbox cards, layout only (#148)

- **Hypothesis:** sorting a real backlog by hand, as cards in piles, is quicker and clearer than a list in a browser, and the piles are remembered between sessions.
- **Build:** `environment-github`'s server half syncing one repository's open issues and pull requests through a token on the server; cards with the faces above; pick up and put down with the ray; snapping piles; layout kept with the space. No physics, no write-back. It can be the "small test environment with real state and intents" #75 and #76 call for.
- **Evaluate:** triage Tomori's own open issues on the headset, then a comparable set on a laptop. Compare time, decisions made, and how it felt. A day later, find "the pile with the physics issue" without labels.
- **Depends on:** #75, #76, #77 (cards as panels), #78 (grab), #79 (where the inbox stands), #83 (the token as a data source), and #146 for legible titles at card size.
- **Effort:** medium, mostly the framework it exercises.

### 2. Code panel and "explain this" (#151)

- **Hypothesis:** pointing at code and asking is faster and more precise than describing it, and the agent's answers are better grounded.
- **Build:** one tool, `show_code`; one read-only, highlighted panel; line-range selection by sweeping the ray; the selection and pointing captured at the start of speech, as worker pointing already is, and delivered as a bounded context block. The glyph atlas (`design/text.md`) draws code's symbols and any Unicode its fonts cover, a quad per glyph.
- **Evaluate:** ask ten "explain this" and "why is it like this?" questions about Tomori's own code. Count how often the agent addresses the right span, and compare with asking the same by description alone. Note which panel sizes and distances are legible.
- **Depends on:** #81 (capture and context), #82 (the tool and its transport, shared with #100), #147 (the panel), and #146 for anything beyond ASCII.
- **Effort:** medium.

### 3. Triage boxes with confirmed write-back (#149)

- **Hypothesis:** putting the classification into the gesture removes the "now do it on the laptop" step, without labelling anything by accident.
- **Build:** two or three label boxes, pending changes on cards, the batch summary confirmed with **A**, undo, and the conflict check. Then try the undo-window variant.
- **Evaluate:** a real triage session; count wrong labels applied (target: none), how often a pending card was pulled back out, and whether confirming felt like a chore.
- **Depends on:** experiment 1, #80 (the Environment section), and #97 if throwing is in.
- **Effort:** small on top of 1.

### 4. Guided tour of a pull request (#154)

- **Hypothesis:** being walked through a change by the agent, which points with its own ray and speaks from each station, gives understanding of a pull request faster than reading the diff.
- **Build:** the agent produces a tour, an ordered list of stations (a file range and a sentence each), through a tool; stations stand in an arc of code panels; agent speech is placed at the current station; "next", "back" and "why?" while pointing interrupt it. The agent's ray is a highlight drawn from the coordinator to the span.
- **Evaluate:** review two of Tomori's recent pull requests by tour and two on GitHub. Plant a mistake in one branch and see whether it is found.
- **Depends on:** experiment 2, #82, #79, and spatial audio placing speech at a point (today agent speech sounds from the swarm, `design/spatial-audio.md`).
- **Effort:** medium.

### 5. Surround view (#153)

- **Hypothesis:** seeing a function's callers, callees, tests and history at once, by glancing, answers "is it safe to change this?" faster than navigating to each.
- **Build:** focusing a function in a code panel asks rust-analyzer for incoming and outgoing calls and references from tests, and `git log -L` for its last few changes. Callers stand to the left, callees to the right, tests behind and history below as rings, each a small card that opens a panel when pulled out. Push a card away to dismiss it; gather cards to ask about the group.
- **Evaluate:** five "what would changing this break?" questions, on the headset and on a laptop with an editor's call hierarchy. Compare time and confidence.
- **Depends on:** experiment 2, #152 (rust-analyzer on the server), #81 (multiple selections), and #97 or #78 for pulling and pushing.
- **Effort:** medium.

### 6. Structure projection (#152)

- **Hypothesis:** a 3D layered view of crates and modules, showing only what a question needs, helps reason about organisation, such as the client split (#132).
- **Build:** tools that return `cargo metadata`'s crate graph and a module tree to the agent, which chooses a subset and draws it with #82's diagram.
- **Evaluate:** stand in the crate graph and talk through #132's split with the agent; ask it to highlight what a change touches. Is the 3D layout better than a 2D picture of the same subset?
- **Depends on:** #82 and #77; experiment 2 to open code from a node.
- **Effort:** small to medium.

### 7. Time dial (#154)

- **Hypothesis:** twisting a dial to watch a region change through its history makes "why is it like this?" answerable at a glance, alongside the agent's account.
- **Build:** a dial on a code panel, turned by twisting the controller while gripping it; each step is a commit from `git log -L` for the range, with lines that change morphing between versions and the commit's subject shown.
- **Evaluate:** answer "when and why did this change?" for three regions with the dial and with `git log` on a laptop.
- **Depends on:** experiment 2.
- **Effort:** small.

### 8. Code city (#155)

- **Hypothesis:** a stable spatial layout of the codebase builds memory ("the auth stuff is over there") and makes it obvious where the agent's changes land while it works.
- **Build:** districts from crates and buildings from modules, sized by lines of code and glowing with recent churn or failing tests; buildings pulse when the agent writes to their files during a turn. The layout must be stable as code changes.
- **Evaluate:** over three sessions, point to where things live without labels; during a real agent task, is watching changes land useful or noise? The controlled experiment with CodeCity (Wettel, Lanza and Robbes, ICSE 2011) is a reference point for what to measure.
- **Depends on:** #77 (instanced shapes), #79, and #152 for the data.
- **Effort:** medium.

### 9. Flow walk and trust boundaries (#155)

- **Hypothesis:** walking a request's path, station by station, with trust boundaries as walls and untrusted input as a coloured stream, makes a missing check visible.
- **Build:** the agent authors a path from the call hierarchy and its reading of the code; stations on the floor with code panels; walls where the agent marks trust boundaries; teleport between stations. Start with Tomori's own path from `POST /connect` through the authenticators to the hub.
- **Evaluate:** remove a check in a branch and see whether the walk reveals it, and whether the agent's account from each station is clearer than a written review.
- **Depends on:** #78 (teleport), #79, #82, experiment 2, and spatial audio at a point.
- **Effort:** large.

### Recommended first

1. **Inbox cards, layout only.** It is the maintainer's own workflow, it is useful on the first day, and it gives #75 and #76 a real environment to be built against.
2. **Code panel and "explain this".** It tests the central principle, talking about code rather than editing it, and every later code experiment builds on it.
3. **Triage boxes with confirmed write-back,** as soon as the inbox works, to settle how gestures become actions.

Fixing the hand menu's text bugs (1 to 4 above) can happen at any time, and should, since they are small. The atlas should come before code panels go beyond ASCII.

## Open questions

- **Which inbox?** A repository's open issues, a saved search, review requests or notifications? One inbox per space, or several?
- **Where does a card go when it changes on GitHub** while it sits in a pile: stay put and update, or move?
- **Batch or undo window** as the default for write-back, and should the undo window ever apply to anything but labels?
- **Who may write back:** owners only, or collaborators when an owner allows it? Is GitHub's view of who did it (the server's token) enough, or should Tomori add a trailer, comment or audit log?
- **GitHub App or personal token,** and per-server or per-owner access, if several owners share a server?
- **Visitors and private code:** are code panels and cards hidden from visitors by default, like shared windows in #85?
- **Persistence of agent content:** do code panels and diagrams survive a restart (#82's open question)?
- **Code environment or projections only:** do the city and the flow walk earn an environment of their own, or stay as projections in any environment?
- **Languages beyond Rust:** the toolkit should not assume Rust, but the analysis adapters start with it.
- **Plain Git** after GitHub: local branches, commits and stashes as objects, without GitHub?

## Changes to existing documents

None yet. If experiments become requirements:

- `design/environments.md`: the GitHub environment and its write-back rules.
- `design/interaction.md`: action boxes and their confirmation under "No control has a hidden destructive meaning".
- `design/security.md`: GitHub credentials as server-side data sources, and code and cards for visitors.
- `design/hand-menu.md`: the Environment section's Apply and Undo.
