# Proposal: Visualisations as Code, Interface First

Status: **proposed**, November 2026, for the maintainer's review. Nothing here is a requirement until a phase brings it into the design documents it changes (listed at the end), which take precedence. No implementation yet. It was written against `origin/main` at `916c45ca` (#531).

It builds on panels and their kinds, versions, live data and limits (`design/projection.md`); volumes, their layouts, pointing and focus (`design/diagrams.md`); the panel context model, chips, instances, pinning and freezing (`design/panel-context.md`); environments' server and client sides (`design/environments.md`) and their actions (`design/environment-actions.md`); the shared, cached Prometheus client (`design/data-sources.md`, `tomori-prometheus`); opaque faces and the frame's budgets (`design/rendering.md`); the platform-neutral client (`design/clients.md`); the spectator (`design/spectator.md`); the server's web pages and trust boundary (`design/security.md`); and four open proposals: panel faces (#306), refresh policies (#509), compositor layers, parked (#529), and interactive diagrams (#534). It also draws on Observe (Shopify's internal downstream environment, its `design/observe.md`), whose `chart` action, PromQL guardrails and comparisons are the nearest thing to what this proposes.

## The request

The maintainer, in the headset, November 2026, over a long conversation:

- Tomori has three ways to show data, and only one of them works for anything. Diagrams (`show_diagram`) are a fixed format drawn as a static picture. Observe's `chart` action draws live Prometheus lines. And the agent writes Python, renders a PNG and shows it with `show_image`, as it did for "faster pods get more traffic": latency against requests a second, coloured by deployment window, which is really a 3D dataset flattened into an image. **The Python route works because the agent can write any code to make any output.** The diagrams disappoint because they cannot, and nothing in them can be grabbed.
- Don't design a universal dataset schema (it never covers every case and grows an endless list of options), and don't think in templates. **Think in code**: code that queries data (a Prometheus series, the Cargo crate graph) and builds an interactive 3D scene, with axes, points, bars, nodes, edges, labels and colours, and decides what happens when something is pointed at or grabbed (pull a node and the graph rearranges on springs, #534).
- **Data sources are separate from rendering and from environments.** Observe's Prometheus access should be usable in any space, by capability.
- **The back of a panel**, like the rack in the Reason audio app: flip a panel to see how it is configured (source, query, time window, refresh, links) and how it is wired to other panels, including panels on walls with no walkway behind them.
- **Snapshot panels**: a panel as a window onto frozen 3D geometry.
- **Extension renderers** in an environment's compiled code where needed (#306, tier 3).
- **The web**: not everyone has a headset. Render the same visualisations in a browser, and let the agent make **shareable reports**: a discussion's question, data, charts, conclusion and next steps, as a page to send to colleagues.
- Prototype it in the Observe space.

### The maintainer's direction: interface first, in Rust, compiled

After a first draft that led with choosing an interpreter, the maintainer redirected:

> Whatever interface scripts get must be exposed from Rust anyway; an interpreter only changes *when* code is evaluated, at run time instead of compile time.

1. **Define the visualisation interface as a Rust API**: its **inputs** (named data sources offered by an environment or by Tomori), its **outputs** (a panel or scene, 2D, 3D, a graph, anything), its **events** (point, grab, release, choose) and **its own update control** (it decides when it refreshes or queries again, and how it reacts to interaction). A fully encapsulated unit, evaluated by the server.
2. **Prototype with hard-coded visualisations**, Rust modules compiled into the server, in Tomori or in the Observe environment for Observe's data. The turnaround is a rebuild and a restart, which is fine: the point is to discover the interface and the interactivity, not to invest in an interpreter.
3. **Only then improve the turnaround**: dynamically loaded Rust, an interpreter or WebAssembly. Compare them, but defer the decision. Security and speed are not the first concern, since the agent can already rewrite the server; the security boundary is defined once the interface is understood.

And then a further point for the interface:

> Inputs can also be outputs, and shared.

A time range many panels follow is shared configuration (today's context model, #393 to #485); pointing at a series on one chart and seeing it on the others is linked interaction (today's linked time cursor, #379). The maintainer recalled Objective-C's **key-value observation (KVO)**: components declare interest in another's value and are told when it changes. It made complex interactions possible that would be hard to build by hand, but it was hard to use in practice. So the interface should model shared values explicitly, as typed, scoped, observable channels, and avoid KVO's pitfalls.

This proposal is organised around that order.

## Recommendation in brief

- **A visualisation is a Rust value implementing one trait, `Visualisation`**, in a core crate with no Tomori dependency, `tomori-visual-core` (section 2). It declares the sources it reads, the channels it follows and sets, and the parameters it takes; `update` reads sources and changes its own state, returning **when it wants to run next**; `draw` builds a scene from that state, purely; `event` receives pointing, grabbing, moving, releasing and choosing. Tomori runs it on the server, diffs successive scenes and sends only what changed.
- **Kinds are registered; instances are made by the agent.** Tomori and environments register visualisation kinds (`observe:faster-pods`, `tomori:crates`), each with typed parameters like an environment action's. In phase 1 the agent's interface is `show_visualisation { kind, id, params, place }`: it chooses and parameterises compiled code. Later, a script or a loaded module is just another implementation of the same trait.
- **Sources are a registry of named, typed, read-only capabilities** (`prometheus:observe`, Observe's shared cached client; `cargo:workspace`), reached by name and type through the visualisation's context; every answer carries its **provenance**.
- **Inputs that are also outputs are channels**: named, typed, observable values (`time`, `region`, `cursor.time`, `hovered_pod`), shared or held per person, scoped to a panel, a wall or the space, kept or expiring, settled once per tick in dependency order with cycles refused, and evaluated on the viewer's own client first where they are fast. The linked time cursor (#379) is the worked example. In phase 1 the context model's `time` and parameters reach visualisations through an **adapter bridge**; moving the context model itself onto channels is a later phase of its own.
- **Outputs are a scene description** of the core's own types (instanced points, bars, lines and tubes, meshes, labels, axes, legends, groups, a graph, colours by role, an opaque extension hook) plus **declared behaviours** that clients run at frame rate. Clients never run visualisation code; the headset, the spectator and later the browser draw the same description.
- **Crates of their own, extraction-ready.** `tomori-visual-core`, `tomori-visual-runtime` and `tomori-visual-adapter` start in Tomori's workspace under its naming and shared-version rules; CI checks that the core and runtime depend on no other Tomori crate, so the core can later move to its own `base-rs` repository under a `base-*` name. `tomori-protocol` keeps its own scene wire format, converted in the adapter.
- **Every panel gets a back**, flipped by a chip, by turning it over in the hand, or by the agent, showing its source, query, window, refresh, provenance, errors and channels; cables show which panels feed which.
- **Web and reports later**: the same description as SVG or WebGL2, and reports at a signed-in URL with share links and a static export.
- **Phase 1, each step usable in the headset, about 60 days in all:** (a) the back of a panel for Observe's charts, about 7 days; (b) the core, the adapter, the scene panel and the faster-pods scatter following `time` through the bridge, about 27 days; (c) channels proper, with the linked cursor and `hovered_pod` lighting a pod's rod on Observe's region disc, about 14 days; (d) Tomori's crate graph with grab-and-pull, about 12 days including #534's prerequisite phases. The choice between dynamic Rust, an interpreter and WebAssembly is deferred until the interface has been used; the leading candidate for the agent's route is a **purpose-built declarative workbook language**, decided in outline by the maintainer: channels as `input`, `output` and `shared`, immutable `let` definitions in any order, strong types, method chaining with keyword arguments, embedded `promql!`/`sql!` blocks bound as AST nodes, imported view modules, and its own sandboxed Rust interpreter (section 10).

## What exists today

| Way to show data | What it can do | What it cannot |
| --- | --- | --- |
| Text, list and chart panels (`projection.md`) | Live lines and bars, 8 series of 256 points, readouts, context chips, freezing | Any other shape of chart; anything in depth; any behaviour |
| Volumes (`diagrams.md`) | Nodes, edges and groups the server lays out, pointed at and focused | Data-driven geometry; scales, axes or points; grabbing (proposed in #534) |
| Observe's `chart` action (`query_chart`) | Any PromQL the agent writes, guarded, read through the shared cached client, live or over a fixed window | Anything but a time-series line chart, and only in Observe's space |
| `show_image` of a PNG the agent rendered | Any picture at all | Interaction, liveness, depth: "static, ugly and slow", as the maintainer said of charts before the `chart` action |
| Face panels (#306, proposed) | An environment's compiled client code drawing a panel's face | Reading sources; anything in depth; being run on the server |
| An environment's server side (`environments.md`) | Arbitrary compiled code on its own thread, reading sources, publishing panels and handling choices | Being reused outside its environment; drawing anything but text, lists and charts |

The last row matters: an environment's server side is already "compiled code, evaluated by the server, with inputs, outputs and events". A visualisation is that idea cut down to one panel and freed from its environment. The server already serves plain HTML pages, sign-in and an owner's status page, without scripts (`crates/server/src/pages.rs`, `security.md`). `data-sources.md` leaves open how sources are configured in the server and shared among environments (#83, #73), and whether the agent may use them (#158).

## 1. The visualisation interface

### The trait

In the core crate, `tomori-visual-core` (section 2), which depends on nothing of Tomori:

```rust
/// A panel's worth of code: it reads sources, keeps its own state, draws a scene from it and reacts to people.
pub trait Visualisation: Send {
    /// Read sources and update state. Called when the visualisation is created, when it asked to be woken,
    /// when its parameters, time context or inputs change, and when someone presses refresh.
    fn update(&mut self, cx: &mut Context<'_>, why: Wake) -> Result<Next, Problem>;

    /// Build the scene from state. Pure: no sources, no clock but `view.now`, the same state gives the same scene.
    fn draw(&self, view: &View<'_>, scene: &mut Scene);

    /// React to someone pointing, grabbing, releasing or choosing an item. Nothing happens by default.
    fn event(&mut self, cx: &mut Context<'_>, event: &Event) -> Result<Next, Problem> { Ok(Next::idle()) }

    /// What the agent, a client that draws no scenes and a report's text read. Required.
    fn summary(&self) -> String;

    /// Rows for the back of the panel beyond what Tomori writes itself (section 6).
    fn back(&self) -> Back { Back::default() }

    /// State to keep across a restart, and what `Kind::restore` is given. None keeps nothing.
    fn save(&self) -> Option<serde_json::Value> { None }
}

/// Why `update` was called.
#[non_exhaustive]
pub enum Wake { Created, Restored, Due, Params, Channels(Vec<ChannelName>), Viewed, Refresh }

/// What a call asks for next: whether to draw again, and when to be woken.
pub struct Next { pub redraw: bool, pub then: Then }

#[non_exhaustive]
pub enum Then {
    Idle,                    // only when something changes or someone presses refresh
    At(SystemTime),          // once, at a time: the next minute, the end of a window
    Every(Duration),         // a cadence, at least 5 seconds
    WhileViewed(Duration),   // a cadence while someone can read the panel, otherwise idle (#509's `on view`)
}
```

A **kind** is registered once and makes instances:

```rust
pub struct Kind {
    pub id: KindId,                       // "observe:faster-pods": the registrant's prefix, then 1 to 32 letters, digits, `-`
    pub title: &'static str,
    pub description: &'static str,        // for the agent: what it shows and when to use it
    pub params: Vec<Param>,               // as environment actions' parameters: text, number, choice, each bounded
    pub sources: Vec<SourceSpec>,         // the sources it reads, by name and type (section 5)
    pub subscribes: Vec<ChannelSpec>,     // channels it follows: its inputs (below)
    pub publishes: Vec<ChannelSpec>,      // channels it sets: its outputs (below)
    pub make: fn(&Params) -> Result<Box<dyn Visualisation>, Problem>,
    pub restore: Option<fn(&Params, serde_json::Value) -> Result<Box<dyn Visualisation>, Problem>>,
}
```

### Inputs: the context

`Context` is everything a visualisation may reach, and nothing else:

- `cx.source::<Prometheus>("observe")` and `cx.source::<CargoMetadata>("workspace")`: a typed handle to a registered source (section 5), refused with a `Problem` if the kind did not declare it or the space is not granted it.
- `cx.params()`: its own parameters, which belong to this panel alone, as the agent last set them.
- `cx.channel(&TIME)`, `cx.channel(&REGION)`: the effective value of a channel it subscribes to, resolved at this panel's scope (below). Time is a channel like any other, so a visualisation follows its wall's window and can be frozen as a chart is.
- `cx.publish(&SELECTION, value)`: sets a channel it publishes.
- `cx.viewed()`: whether anyone can read the panel now, from the server's own geometry as #509 proposes.
- `cx.now()`: the instant of this call, the same for every source read in it.
- `cx.log()`: lines for the back and for `visualisation_logs`.

Every source answer is a `Sourced<T>`, the value with its provenance; the context records each one for the call, so a visualisation never has to.

### Outputs: the scene

`draw` fills a `Scene` (section 3): instanced points and bars, lines, tubes, meshes, labels, axes, legends, groups, a graph, and declared behaviours. Items carry ids the visualisation chooses; Tomori diffs one scene against the last by those ids and sends the difference as live data, so the visualisation never deals with the wire.

### Events

`Event` is `{ kind, item, group, hand, participant, at }`, where `kind` is `Point` (words begun while pointing, which also tells the agent the item's `detail`), `Trigger`, `Grab`, `Move` (the held item moving, at most 10 a second), `Release { inside }`, `Both` (two hands each holding an item) or `Choose` (A on a choosable item, as a list's row is chosen). Visitors' events never reach a visualisation. Without an `event` implementation the defaults are Tomori's: the trigger gives a private focus, as on a diagram; the grip carries the whole panel.

**Who owns a grab.** A grab has two owners at two speeds, and neither guesses:

- **The client owns the motion.** Where the scene declares a behaviour on an item, `drag` or the springs of a `graph`, the holder's client runs it at frame rate: the item follows the hand, the springs settle around it, with no round trip. An item with no such behaviour does not move.
- **The visualisation owns the meaning.** It receives `Grab` when the hold begins, `Move` at most 10 times a second while it lasts (the item's position, not every frame), and `Release` with where it ended and whether inside its box. On release it may accept the position, move the item elsewhere, pin it, or change anything else in its state and draw again; its next scene is the truth, and the client eases to it.
- **Without a handler**, the client's behaviour alone applies and nothing reaches the visualisation; without a behaviour or handler, the grip carries the whole panel, as today.
- **First grab wins, per item.** Holding is an intent with an epoch, as #534 and physics already define: the server grants the first that arrives; a second participant's client sees the item in the first one's hand and lets go of its own. Different people may hold different items.
- **Shared motion.** While an item is held, other participants see it where the holder's presence says, interpolated; for a graph, the server runs #534's solver for the shared layout and streams positions as live data, as #534's phase (c) proposes.


Events run on the server, so a reaction arrives about a tenth of a second later. That suits "show this pod's trace"; anything that must follow the hand at frame rate is a declared behaviour (section 3) or a per-participant channel evaluated on the client (below).

### Channels: inputs that are also outputs

A **channel** is a named, typed, observable value in a space. Visualisations, environments' sides, Tomori's own panels, participants and the agent read and set channels; nobody reads anyone else's internals. This is KVO's idea with its scope made explicit: what is observable is declared, typed and listed, never any property of any component.

**Declaring.** A channel is declared by whoever owns its meaning, with a key typed at compile time:

```rust
pub const TIME: ChannelKey<TimeBinding> = ChannelKey::builtin("time");            // Follows, Live { minutes }, Window { from_ms, to_ms }
pub const CURSOR_TIME: ChannelKey<Option<Instant>> = ChannelKey::builtin("cursor.time");  // per participant, expiring
pub const SELECTION: ChannelKey<Keys> = ChannelKey::builtin("selection");

// In Observe's common crate:
pub const REGION: ChannelKey<Text> = ChannelKey::namespaced("observe", "region");  // bridged to its context parameter in phase 1
pub const HOVERED_POD: ChannelKey<Keys> = ChannelKey::namespaced("observe", "hovered_pod"); // per participant, expiring
```

| Property | Choices | Notes |
| --- | --- | --- |
| **Type** | `TimeBinding`, `Instant`, `Text` (with optional choices), `Number` (with a range), `Keys` (a set of up to 64 short strings: pods, crates, series), `Flag`, and `Option` of any of them, where `None` means "nothing now" | A closed set, so that every channel can be drawn as a chip, typed as a cable's colour, shown in `channels` and checked when linked. Two declarations of one name with different types are refused at registration. |
| **Holding** | **Shared**: one value, held by the server, the same for everyone. **Per participant**: one value for each participant in the space. | `time`, `region`, `selection`: shared. `cursor.time`, `hovered_pod`, `hovered_series`: per participant. |
| **Scope** | Where a value applies: the **space**, a **wall** (the panels on it) or one **panel**; for a shared channel, with inheritance panel → wall → space (below) | `time` at every scope; `cursor.time` at the wall of the chart pointed at |
| **Lifetime** | **Kept** with the space across restarts, or **expiring**: cleared to `None` if its publisher does not refresh it within a timeout | `time`: kept. `cursor.time`: expires after 1 s |
| **Propagation** | **Local first**: a participant's own value is settled on their own client every frame. **Relayed**: values others need go through the server, at most at a declared rate, coalesced, last value winning | `time` is written at most twice a second and settled on the server at most 10 times a second; `cursor.time` is local at frame rate and relayed at most 10 times a second, and only where someone subscribes to everyone's |
| **Writers** | Who may set it: its declarer, publishers, participants (by chips and gestures), the agent | Visitors never write. A shared change by a participant or the agent is announced, as `set_context`'s are. |

These are properties of one mechanism, not two systems: a long-lived shared setting and a fleeting per-person pointer differ only in their declaration, as the worked example shows.

### Worked example: the linked time cursor

Today's linked time cursor (#379, `projection.md`) is the model the maintainer agreed for every channel. Expressed as a channel:

```rust
pub const CURSOR_TIME: ChannelKey<Option<Instant>> = ChannelKey::builtin("cursor.time")
    .per_participant()                // a value for each person
    .scopes([Scope::Group, Scope::Space]) // the publisher's axis picks: a live chart its wall (a group), a fixed window the space
    .expiring(Duration::from_secs(1)) // cleared if its publisher stops: pointer lost, headset asleep, connection gone
    .relayed(Rate::per_second(10));   // what others need goes through the server, at most 10 a second

pub const TIME: ChannelKey<TimeBinding> = ChannelKey::builtin("time")
    .shared()                         // one value for everyone
    .scoped(Scope::Inherited)         // panel, else wall, else space: today's context settings
    .kept()                           // kept with the space; never expires
    .relayed(Rate::per_second(2));    // a chip pressed or the agent's set_channel, coalesced
```

The core names the levels of a host's scope tree `Panel`, `Group` and `Space`, never walls; Tomori's adapter maps a wall to a `Group`.

**Publishing.** The chart pointed at publishes `Some(t)` as the ray moves along its plot, the instant under the ray, and `None` as the ray leaves it. Publishing is a value at the publisher's scope, which its time axis decides, as #379 does: a **live** chart publishes at its **wall** (an Observe terminal's three charts share one value, another wall has its own); a **fixed-window** chart publishes at the **space**, since an absolute instant means the same wherever the chart stands; a subscriber listens at the scope its own axis implies, so live and fixed never meet. The maintainer's "scoped to a wall" is the live case, the common one.

**Notification.** Every panel on that wall subscribing to `cursor.time` is told each transition, `None → Some(t)`, `Some(t) → Some(t′)` and `Some(t) → None`, once per frame at most, after the frame's writes have settled. Its response is its own: a chart draws its readout at `t` (its line down the plot there, a dot on each series and their values, fainter than the pointed chart's own) or hides it; a chart whose window does not reach `t` within half a point draws nothing; a scene with a time axis draws a plane at `t`.

**Per person.** The channel holds one value for each participant at each scope. Each subscriber chooses what it shows: **the viewer's own** (`Show::Mine`, a chart's default, which is today's behaviour) or **everyone's** (`Show::Everyone`), each participant's readout in their own colour with their name, the viewer's strongest, for a presenter walking a room through an incident.

**Local first.** The viewer's own cursor never waits for the server: it is published and settled on their own client every frame, so their readouts move with their hand. Only when some subscriber in the scope shows everyone's does the client relay its value, with presence, which already reaches the server up to 20 times a second, at most 10 times a second and only when it changed; the server passes it to the clients of participants who can see that wall. A relayed value for a live window is carried as how long before the server's receipt it points, so that two devices' clocks never misplace it.

**Expiry.** A value not refreshed within a second clears to `None`, on the publisher's own client and on every client it was relayed to, so a readout never sticks when a pointer is lost, a headset sleeps or a connection drops; a participant leaving clears theirs at once.

**The same mechanism as the time range.** `time`, set by the context chips on a wall or panel, or by the agent, is declared with the other choice of each property: shared, inherited from panel to wall to space, kept, and changed a few times a minute rather than sixty times a second. Its subscribers are told its transitions as `cursor.time`'s are, and redraw over the new window. A chart typically subscribes to both: `time` for what window to read, `cursor.time` for where to draw a readout.

**How #379 maps onto it.**

| Today (#379, `projection.md`) | As a channel |
| --- | --- |
| A readout follows the ray on the chart pointed at | The chart publishes `Some(t)` to `cursor.time`, and `None` as the ray leaves |
| Every other chart that shares its time shows a linked readout, fainter | Every subscriber at the same scope is notified and draws its readout; the publisher draws its own at full strength |
| Live charts (`TimeAxis::Live`) link only to live charts on the same wall, by how long before now | A live chart publishes and subscribes at its **wall's** scope; the instant is now less how long before now |
| Fixed windows (`TimeAxis::Fixed`) link to every overlapping fixed window in the space, by the instant | A fixed-window chart publishes and subscribes at the **space's** scope, so live and fixed never meet, as today |
| A live chart floating or carried links to nothing | It has no wall, so its scope is the panel itself and nothing else subscribes there |
| Charts link only by their structured time axes, never by labels | The channel's type is an `Instant`; a chart without a time axis neither publishes nor subscribes |
| A chart out of range by more than half a point shows nothing | The subscriber's own response to `Some(t)` |
| Linked readouts are per participant and unsent | `Show::Mine` and local-first propagation: with no subscriber showing everyone's, nothing is relayed, exactly as now |
| The spectator draws no readouts | The spectator has no participant value and subscribes to no one's |

Nothing a participant sees changes; what changes is that new subscribers (a scene's time plane, an environment's own 3D timeline) and new choices (everyone's cursors) come at the cost of a declaration.

**Scopes and inheritance: bridged in phase 1, unified later.** A shared channel has a value at up to three scopes: the **space**, each **wall** and each **panel**, a panel's effective value being its own setting, else its wall's, else the space's. That is exactly the inheritance `panel-context.md` requires of context parameters and time, but `panel-context.md` requires much more besides: settings of parameters a side no longer declares are kept; instances under `~` ids, at most 8 an environment; held windows, accepting only content declared for a fixed window; pinning by pulling a panel off its wall; freezing a copy of a chart; resting panels coming back with their settings; and the chips and messages on the wire. Moving all of that onto a new store is a project, not a step.

So phase 1 uses an **adapter bridge**. The core asks its host for a channel's value through a trait (`ChannelHost`); for a channel backed by the context model, Tomori's adapter answers from the existing model:

- **`time`** reads the panel's effective time binding as `panel-context.md` resolves it today (its own setting, else its wall's, else following), so a visualisation on a wall fixed to an incident's window reads that window, a pinned copy keeps its own, and a frozen one keeps what it showed. A visualisation that publishes `time` (a brush) writes a panel setting through the same path `set_context` uses, refused where the model refuses it.
- **An environment's parameters** (Observe's `region`) read the panel's binding of that parameter, or the side's selection where it follows; a visualisation subscribes to them by declaring it depends on them, which the adapter turns into `Panel::depends_on`.
- **Changes** reach the visualisation as `Wake::Channels`, from the moments the server already tells a side `Intent::context()`.
- **Chips, instances, pinning, freezing and the agent's `contexts` and `set_context`** keep working unchanged, because nothing under them changed.

New channels with no counterpart in the context model (`cursor.time`, `hovered_pod`, `selection`, `crate`) live in the core's own channel store from phase 1(c). **Migrating the context model itself onto the store** is a later phase of its own, with its existing tests as the proof, estimated at about 15 days (section 8). Until then the two meet only in the adapter.

A per-participant channel has no inheritance: its scope is where its publisher stands, a wall, the space, or the panel alone, and its subscribers at that scope hear it, as the worked example shows.

**What stays and what moves, later.** When the migration phase comes: `Space::set_context` declares shared `Text` channels; `Panel::depends_on` subscribes; `Intent::context()` is the change notification; chips show bindings; `contexts` and `set_context` become views of `channels` and `set_channel`. The linked time cursor moves onto `cursor.time` in phase 1(c) already, since it is client-side, per participant and has no persistence to migrate; `hovered_series` is its twin, a per-participant, expiring `Option<Keys>`: a chart's readout publishes the series pointed at, and charts subscribing brighten it and dim the rest.


**Propagation, avoiding KVO's pitfalls.**

| KVO's pitfall | The rule here |
| --- | --- |
| Any property observable, so nobody knows what depends on what | Only declared channels; every subscriber and publisher is declared by its kind and listed by `channels` and on panels' backs |
| Notifications fire synchronously, re-entering half-updated observers | Writes are collected and **settled once per tick**: on the server, each hub sweep for shared channels; on a client, each frame for participant channels |
| Glitches: an observer sees one input changed and another not yet | Within a settle, subscribers are updated **in topological order** of the declared graph (publisher before subscriber), each at most once, with every input already settled: `Wake::Channels` names all that changed |
| Cascades and loops | **One direction per update**: a value published while settling reaches only subscribers downstream of the publisher in this settle; anything else waits for the next tick. **Cycles are refused** when declared or linked (a link that would close a cycle among shared channels is refused, saying which), except that a visualisation may subscribe to and publish the same channel (a brush that follows and sets `time`), and its own write never wakes it (echo suppression). A run of ticks in which a value keeps changing without a new outside write is stopped after 8 and logged. |
| Observers outliving what they observe | Subscriptions belong to panels and kinds; they end with the panel |
| High-frequency changes swamping everything | **Local first**, then relayed at a declared rate, coalesced, last value winning; a hover at frame rate stays on its client unless someone shows everyone's, while a time range never needs frame rate. **Expiry** clears a value whose publisher went quiet, so nothing is left dangling |
| Stringly-typed key paths and typos | `ChannelKey<T>` is checked at compile time and declarations at registration; the agent's `set_channel` refuses an unknown name with suggestions within two edits |

**Participant channels and server-evaluated visualisations.** A visualisation runs on the server, but a hover must react at frame rate on the participant's own client. So it binds participant channels **declaratively**, in its scene, rather than in code: items carry keys (`points.key(|r| &r.pod)`), and the scene declares `publish_on_hover(&HOVERED_POD)` (the key of the item under the ray becomes the channel's value) and `emphasise_when(&HOVERED_POD)` (items whose key is in the value brighten, the rest dim, as a trigger focus does). The client runs both with no round trip. Environments' client sides read and set participant channels directly (`ClientContext::channel` and `publish` in `tomori-environment`), since they already run on the client. A participant's value reaches the server only when relayed, because some subscriber shows everyone's, or in a pointing note when words begin; server-evaluated code never subscribes to a per-participant channel's stream, only to what its scene declares.

**The agent's tools.**

- **`channels`**: every channel in the space, its type, holding, scope, lifetime and declarer; its space value; every wall and panel that sets it, with `follows`, `fixed` or `linked`; its publishers and subscribers; and the links. Participant channels are listed without values.
- **`set_channel`**: a channel at `space` (where the declarer allows), a `wall` or a `panel`, to a value, `follow`, or `link` to another panel's published channel; refused as the stage refuses, saying why; announced on the status wall as `set_context` is today.
- `contexts` and `set_context` remain, as the parameter-shaped view of the same.

### Wiring is visible

The declared graph is the wiring: each subscription and publication is a **jack** on the panel's back, and each link, or each panel following its wall's setting, a **cable** (section 6). Nothing propagates through a channel without a jack and a cable that the back of the panel would show.

### Update control

The visualisation decides when it runs: `Next::then` is its own schedule, so a chart of a 30-second-scrape metric asks for `Every(30 s)`, a clock asks `At(next minute)`, a crate graph asks `Every(10 s)` and reads again only when `Cargo.lock`'s digest changed, and a fixed window asks `Idle` once read. Tomori bounds it rather than overrides it: a panel's refresh policy, where #509 is accepted and someone has set one, caps the cadence (`manual` turns `Every` into `Idle`), the live-panel budget counts any visualisation woken more often than once a minute, and the server never wakes one more often than every 5 seconds, nor one reading Prometheus more often than its cache's bucket (15 s), since a faster cadence would only re-read cached answers. Anything faster than that is a behaviour or a per-participant channel on the client, not an update.

### Lifecycle

1. **Create.** The agent calls `show_visualisation`; the server checks the kind's parameters and the space's grants, calls `make`, then `update(Created)` and `draw`, and answers the tool call with the scene's counts, the provenance of every read, warnings and the log's first lines, or the problem in the kind's own words. A visualisation whose first update fails is not projected.
2. **Run.** Updates run on a pool of visualisation threads, never under the hub's lock, since a source read may block for seconds; `draw` runs after any call returning `redraw`, and the hub takes the diff. A call taking over 2 s is logged on the back; compiled code cannot be preempted, which is acceptable in phase 1 (section 11).
3. **Problems.** A failed update keeps the last good scene, marks the panel stale on its face, and puts the problem on its back; three in a row stop automatic wakes until a refresh or a change of parameters. A panic is caught, the instance dropped and its last scene kept, frozen, with one warning.
4. **Teardown.** Throwing the panel away, closing the space or `clear_visualisation` drops the instance. Its kind, parameters, saved state and last scene are kept with the space; after a restart the last scene is shown, marked as of when it was made, and `restore` (or `make`) and `update(Restored)` run.

### Registration and the agent's tools

- **Registering.** `ServerBuilder::visualisation(kind)` for Tomori's own and an application's; `Environment::with_visualisation(kind)` for an environment's, which needs the environment's server crate but not its space: a kind from Observe can be shown in any space whose server runs Observe's server crate and grants its sources.
- **Tools:** `visualisations` lists the kinds available in this space with their descriptions and parameters; `show_visualisation { kind, id, params, place, title }` projects or replaces one; `update_visualisation { id, params }`; `visualisation_logs { id }` gives the log, problems, timings and provenance; `export_data { id, format }` writes what it last read (section 5); `clear_visualisation`. `panels` lists a scene panel as kind `visualisation` with its kind and summary.
- Tomori's layer of the agent's instructions says to prefer a visualisation to rendering an image when a kind fits, and to say in a sentence what it shows.

### Why a trait and not a description format

A trait is the "any code" the maintainer asked for, at compile time: a visualisation can join two queries, compute a regression, lay out a graph or keep a ring of history in ordinary Rust, and nothing about its data needs a schema. It is also exactly what an interpreter or a loaded module will have to implement: `ScriptedVisualisation`, holding an interpreter, implements `Visualisation` by calling the script's `update`, `draw` and `event`, so phase 1's work is not thrown away when turnaround improves (section 9).

## 2. Crates

The interface is useful beyond a headset: a dashboard, a report generator, a desktop tool or another project's server could run the same visualisations against the same sources. So, at the maintainer's request, it lives in crates of its own with a well-defined interface, and Tomori is one host of it.

### Naming and placement

`design/crates.md` requires every workspace crate to be named `tomori-*`, live under `crates/` without the prefix, and share the workspace's one version; `base-*` crates (`base-http`, `base-http-server`, `base-scheduler`) live in their own `base-rs` repositories. So the crates **start in Tomori's workspace as `tomori-visual-*`**, under those rules, while the interface is found, with the core kept **extraction-ready**; once it stabilises (after phase 2 has used it from run-time code), it **moves to its own repository, `base-rs/visual`, as `base-visual`**, with its own semantic versions, which Tomori then pins. Being extraction-ready means: no dependency on any other Tomori crate, no Tomori names in its API, its own tests and documentation, and nothing in it that only Tomori could host.

### The layout

| Crate (folder) | What it holds | Depends on | Built by |
| --- | --- | --- | --- |
| **`tomori-visual-core`** (`crates/visual-core`) | The `Visualisation` trait, `Kind`, `Context`, `Wake`, `Next`, `Event`, `Problem`; channels: `ChannelKey<T>`, value types, holding, scopes, lifetimes, rates, the `ChannelHost` trait a host implements, and the channel store with its settle (topological order, echo suppression, cycle refusal, expiry); the `Source` trait, `Sourced<T>`, provenance and the registry; the **scene description** in its own types: primitives, `Graph`, `Shape`, `Colour` by role, materials, groups and transforms, declared behaviours, items' keys, channel bindings, the opaque `Extension`; its builder, scales and diffing; a test harness with fake sources and golden scenes | `serde`, `serde_json`, `glam`; **no `tomori-*` crate**, no `wgpu`, no protocol, no async runtime | Servers and clients (it holds no credentials) |
| **`tomori-visual-runtime`** (`crates/visual-runtime`) | What a viewer runs: applying a scene's diffs, easing, `drag`, `hover` and `orbit`, picking items with a ray, level of detail, per-participant channels settled locally each frame with expiry and relay hooks; no GPU: it yields instances, lines and labels for a renderer | `tomori-visual-core` only | Clients; later the web viewer as WebAssembly |
| **`tomori-visual-adapter`** (`crates/visual-adapter`) | Tomori as a host: the scope tree (panel → wall → space); the context bridge (`time` and parameters ↔ the context model); scenes ↔ panels, and **converting the core's scene description into `tomori-protocol`'s scene wire types** and back: `Graph` into a volume's `Diagram`, `Colour` roles into the chart palette, `Extension` keys into face drawers (#306); events from pointing and grab intents; registering kinds and sources from `ServerBuilder` and environments; the tools' logic. Its server half is behind a `server` feature; clients build the conversion only | The core and runtime; `tomori-protocol`, `tomori-environment` | `tomori-server` (with `server`), `tomori-scene` |
| **`tomori-visual-kinds`** (`crates/visual-kinds`) | Tomori's built-in visualisations (`tomori:crates`) and the `cargo metadata` source | The core only | Servers |
| Observe's kinds | `observe:faster-pods`, a module of Observe's server crate | The core and `tomori-prometheus`'s source, never Tomori's internals | Observe's server |
| Sources | `tomori-prometheus` implements the core's `Source` behind a `visual` feature; it is already a leaf with no Tomori dependency | The core and its own client | Servers only (the existing rule that no client builds `tomori-prometheus`) |
| Renderers | Phase 1 draws through Tomori's own `tomori-scene` and `tomori-render` (solids, tubes, the glyph atlas), via the adapter's conversion, so nothing is drawn twice. Later, as each is needed: `tomori-visual-svg` (2D to SVG, pure Rust, for web pages and reports), `tomori-visual-web` (the runtime in WebAssembly with a WebGL2 or WebGPU drawing), and a standalone `tomori-visual-wgpu` for other hosts, extracted from `tomori-render`'s solids only when someone outside Tomori needs it | The runtime and a graphics stack | Each its own |

**The wire format is Tomori's, not the core's.** `tomori-protocol` gets its own `scene` module, owned and versioned with the protocol as every other panel kind is, and **does not depend on the core**. The adapter converts between the two, so a change to the core's description changes the adapter, not the protocol, and the protocol changes only when what crosses the wire changes. The cost is one conversion each way, which is small beside meshing.

**The graph and the springs.** The core's `Graph` is its own type; the adapter turns it into `tomori_protocol::diagram::Diagram`, so Tomori draws it with #534's encoding and spring solver, and the core never sees either. #534's phase (c) proposes moving the solver into a shared `tomori-diagram` crate; if that crate depends on nothing of Tomori beyond `glam`, a generic `springs` behaviour in the runtime could later reuse it, which the dependency rule below would then have to allow by name. Until then the runtime has no spring solver, and springs come only through `Graph` in Tomori.

**How the core reaches a host.** The core never names walls, panels or participants' handles. A host supplies a scope tree (each scope's parent), participants as opaque ids, a clock, a `ChannelHost` for channels it backs itself, and a sink for scene diffs and problems. The adapter is where Tomori's names meet the core's.

### Rules, checked in CI

- **`tomori-visual-core` and `tomori-visual-runtime` depend on no `tomori-*` crate other than each other** (the runtime on the core). `bake/tests/dependencies.rs` checks it on the resolved dependency graph, beside its existing rule that no client builds `tomori-prometheus`, and checks that `tomori-visual-kinds` depends only on the core. With the graph, colour and extension types above, nothing in the core or runtime needs a Tomori crate, so the rule passes from the first commit.
- **Kinds see only the core and sources**: Observe's faster-pods module and `tomori-visual-kinds` import no Tomori internals, so a kind can be lifted into another host unchanged; that is the test of the interface.
- **`tomori-protocol` never depends on the core**; the adapter converts.
- **Sources stay server-only**; the core and runtime are allowed on clients, since they hold no credentials.

### Versioning

In the workspace, the crates share Tomori's one version and release (`0.x`, breaking changes accepted), as `design/crates.md` requires. The protocol's scene format changes with the protocol, under its own compatibility rules (non-exhaustive types, `Hello::scenes`, older peers sent the summary). On extraction, `base-visual` gets its own versions and changelog, and `tomori-visual-adapter` pins it.

## 3. The scene and interaction API

A scene is a tree of items in the panel's own unit box, x, y and z each from −0.5 to 0.5, scaled to the panel as a volume is (`diagrams.md`). A scene of zero depth is **flat**, drawn on the face as a chart is; any other is **deep**, standing in front of its plate as a volume does. The description and its builder are Rust in the core, `tomori-visual-core`; `tomori-protocol` carries the description as `Panel::scene`.

### Primitives

| Primitive | Drawn as | For |
| --- | --- | --- |
| `scene.points(id, items)` with position, colour, size, shape, detail | Instanced solids of `tomori_render::solids` | Scatter plots in 2D and 3D |
| `scene.bars(id, items)` with base, height, footprint | Instanced boxes | Histograms, 3D bars, heat maps as boxes |
| `scene.mesh(id, vertices, indices, colours)` | One opaque mesh, at most 16,384 vertices | Surfaces |
| `scene.line(id, points)` and `scene.tube(…)`, solid, dashed or dotted | Hexagonal tubes, as diagram edges | Series, trajectories |
| `scene.label(text, at)` with size, anchor, billboard | The shared glyph atlas | Text, always drawn as text |
| `scene.axes(x, y, z)`, `scene.grid(…)` | Rules, ticks and labels from scales | Any plot |
| `scene.legend(…)` | On the face below the title, as #534's | What colour and shape mean |
| `scene.graph(id, graph)` | The core's own `Graph`: nodes (key, label, `Shape`, colour by role, size, group, detail), edges (from, to, label, direction, weight) and groups, with a layout hint. In Tomori, the adapter converts it to a volume's diagram, so #534's encoding and springs draw it | Dependency graphs, topologies |
| `scene.extension(key, payload, fallback)` | An opaque, keyed escape hatch: the core carries a key and bytes it never reads, with fallback items; **its meaning lives in the host**. Tomori's adapter maps a key to an environment's face drawer (#306); any other host draws the fallback | Shading or density a scene cannot draw |
| `scene.group(id, transform, …)` | A node of the tree | Moving, focusing or hiding items together |

Scales (`linear`, `log`, `time`, `band`) and helpers (`extent`, `bin`, `group_by`, `quantile`) live beside the builder, so a visualisation's code reads like a D3 chart's.

### Colour and materials

The core has its own colour type, by **role**, never by RGB or by any host's palette names, so every scene stays legible and colour-blind safe on any host:

```rust
#[non_exhaustive]
pub enum Colour {
    Category(u8),                 // the nth of a categorical set: the host draws its own distinct, colour-blind-safe colours, then shapes or dashes past its count
    Sequential(f32),              // 0..1 along a perceptually even ramp
    Diverging(f32),               // -1..1 about a neutral middle
    Tone(Tone),                   // Good, Warn, Bad, Muted, Neutral, Accent
}
```

A host maps roles to colours. Tomori's adapter maps `Category` to the eight Okabe–Ito-based series colours (`Colour::SERIES` in `tomori-protocol`), the ramps to tested ramps, and tones to its own; colour-blind mode (#516) swaps them per participant without the visualisation knowing. Two materials: **solid**, lit and opaque, as diagram solids; **glow**, unlit, for highlights. No translucency, for the reasons #530 and #534 give.


### Animation

An item whose id persists between scenes eases to its new position, size and colour over 0.3 s on the client; newcomers fade in and leavers out; nothing extra is sent. Motion over time (a sweep, a pulse) is a behaviour, never code run per frame.

### Behaviours: declared on the server, run on the client

A behaviour is a named, parameterised piece of Tomori's own client code acting on items the scene names:

- `springs { nodes, edges, rest, stiffness, repulsion, damping, home }`: a spring layout. In phase 1 it is offered only through `scene.graph`, which Tomori's adapter draws with #534's solver; a generic springs behaviour in `tomori-visual-runtime` waits until that solver can live in a crate with no other Tomori dependency (section 2).
- `drag { items, along: free | plane | axis, snap, bounds }`: an item follows the hand within a constraint; `Release` tells the visualisation where it ended.
- `hover { items, grow, label }`: what the ray's touch does, privately.
- `publish_on_hover(channel)` and `emphasise_when(channel)`: the declarative ends of participant channels (section 1).
- `orbit` and `pulse`: motion over time.

The holder's client draws a held item in the hand with no lag; the server runs the same solver in `f64` for the shared result and streams positions as live data while anything moves, as #534's phase (c) and physics do (about 7 KB/s per awake scene, nothing at rest). The vocabulary grows with Tomori's releases, which keeps visualisation code off the client.

### Picking

The ray meets an item within its solid's bounds, or a line within 1.5 cm or half a degree, as for a diagram's node or edge. Words begun pointing at an item tell the agent its `detail`, as `[Pointing at point "web-7f9c" in visualisation "Faster pods get more traffic" (id pods, …; 41.2 req/s, p95 18 ms, revision r812)]`.

### Level of detail and budgets

- **Per scene:** 16,384 instanced items, 64 lines of up to 2,048 points, 4 meshes, 256 labels, one graph within the diagram bounds (64 nodes, 128 edges), groups at most 4 deep; past a bound, items are left out in order and counted.
- **Per frame:** scenes share the volumes' budget, 100,000 triangles and 16,384 solids (`rendering.md`); the last in the frame's order draw only their faces and labels.
- **By distance:** past 3 m only axes, the legend and the largest labels; past 8 m only the largest items; drawn as a title at `faces::FAR`, nothing.
- **On the wire:** positions as `i16` in the unit box, colours as palette indices: about 10 bytes an item, about 160 KiB for the largest scene, within a frame's 32 MiB. Diffs follow published panels' pacing, about one a second.

### Snapshots and extensions

- **Snapshots.** Freezing a scene panel (its time chip, a pinned copy, or the agent) keeps its scene description and its data with the panel and drops the instance: a window onto frozen geometry that every client and the web draw with no code running, as a frozen chart is (`panel-context.md`). A true portal, the geometry seen through the panel's rectangle with parallax, needs a render to texture per eye and waits on the measurements that parked #529.
- **Extensions.** `scene.extension` is opaque to the core; Tomori's adapter turns a key it knows into an environment's face drawer (#306, tier 1 or tier 3) where that environment's client crate is present, and every other client or host draws the fallback items.

## 4. Where visualisations run

| | Server only | Client only | **Hybrid (recommended)** |
| --- | --- | --- | --- |
| Credentials | Stay on the server | Must reach clients | Stay on the server |
| Evaluations | One for everyone | One per client | One for everyone |
| Viewers agree | By construction | Only if every client computes alike | By construction |
| Frame-rate interaction | No | Yes | Yes, through behaviours |
| Code on the Quest | None | Every visualisation | None but Tomori's behaviours |
| Web and spectator | Draw the description | Must run the code | Draw the description |

**Hybrid:** visualisations run in the server; their scenes are diffed and sent as live data (a new `data_seq`, not a version, as chart points are, so a refresh never refuses a participant's or the agent's edit); clients draw scenes and run behaviours. A client that does not state `Hello::scenes` is sent the summary as a text panel, as older clients are sent charts and volumes. The spectator draws scenes as it draws volumes, without hover. The web draws the same description (section 7).

**Determinism.** One evaluation means clients need not agree. `draw` is pure so that tests can check scenes against golden files and so that a restart redraws the same picture from saved state; `cx.now()` is one instant per call, bucketed as the source's cache buckets it.

## 5. Data sources

### The registry

A **source** is a named, typed, read-only capability, answering `data-sources.md`'s open question of how sources are configured and shared:

```rust
pub trait Source: Send + Sync + 'static {
    fn kind(&self) -> &'static str;           // "prometheus", "cargo", "files"
    fn describe(&self) -> SourceInfo;         // for the back, the agent and reports: never an endpoint or token
}

impl ServerBuilder { pub fn source(self, name: &str, source: impl Source) -> Self }
impl Environment   { pub fn with_source(self, name: &str, source: impl Source) -> Self }
```

`Context::source::<T>(name)` returns the source downcast to its concrete type, so each source keeps a typed API of its own rather than one lowest-common-denominator query method.

| Source | Registered by | API | Bounds |
| --- | --- | --- | --- |
| `prometheus:NAME` (`tomori_prometheus::Source`) | The application or an environment, wrapping a shared `Client` with its cache and label filter. **Observe registers its own client as `prometheus:observe`.** | `instant`, `range`, `range_all` (one batch, concurrent), each with a call's label filter | The client's limits; every expression through guardrails moved from Observe's `query_chart::guard` into `tomori_prometheus::promql::guard` (concrete metric names, ranges up to a day, offsets up to a week, no `@`); background priority, so a visualisation never delays an environment's own panels |
| `cargo:workspace` | Tomori | `metadata()`: the crate graph of the session's workspace, by `cargo metadata --offline` | Cached by `Cargo.lock`'s digest |
| `files` | Tomori, later | Read and parse CSV, JSON or Parquet under the session's directory | 16 MiB a file |
| `git` | Tomori, later | Log and per-file change counts of the session's repository | Fixed arguments, 10 s |
| `agent` | The agent, later | JSON it passes with `show_visualisation` | 1 MiB |

A space is **granted** sources by the operator's configuration: Tomori's own to every space by default; an application's or environment's to the spaces it names (Observe: `prometheus:observe` to every space on its server). A kind's declared sources must be within the grant before it is made. Sources and channels are different things on purpose: a source is a capability to read an external system, held by the server, with credentials; a channel is a value in the space that anyone permitted may read or set.

### Quotas, provenance and export

- **Quotas** in phase 1 are the sources' own (the client's slots, cache and limits) and a count per space of source requests a minute, logged rather than enforced; they become limits with the security boundary (section 11).
- **Provenance:** every `Sourced<T>` records the source's name, the query, the instant or range and step, when it was read, how long it took, how many series or rows, whether it was partial, and warnings. It is shown on the back, given by `visualisation_logs`, told to the agent on `inspect` (as `tomori_prometheus::Metric`'s JSON), and printed beneath each figure in reports. A scene drawn from partial data says so on its face.
- **Export:** `export_data` writes what the visualisation last read, through its optional `fn data(&self) -> Option<Table>`, to `exports/` under the session's directory as CSV or JSON, Parquet later, with a provenance file beside it, when someone asks.

## 6. The back of a panel

### What it shows

Every panel has a back, Tomori's own canvas:

- **Identity:** title, id, kind, who made it (the agent, an environment, a participant's copy), version and age.
- **Source:** the source's name, the queries, window and step, the refresh policy, when it last refreshed and when it next will.
- **Channels:** each channel it follows or sets, with its binding (`follows`, `fixed`, `inherited` or `linked`) and value, as jacks.
- **Provenance:** the last update's reads with their timings, series counts and any partial answer.
- **Code:** for a visualisation, its kind, its parameters, its schedule (`Every 30 s`), its last problem and its log's last lines.
- **Jacks** and their cables.
- **Actions:** refresh now, freeze, export, and "ask Tomori about this", which tells the agent the panel and its back.

An environment's published panel gets one through `Panel::with_back(Back)`: rows of label and value, links, and environment actions shown as buttons. A visualisation's comes from `Visualisation::back` plus what Tomori knows.

### How flipping works

- **The ⟲ chip** on the panel's chrome turns it about its vertical axis over 0.4 s. For a panel on a wall, with no walkway behind it, this is the way to see the back.
- **Turning it over in the hand:** rolling the wrist past 120° while carrying a panel shows its back.
- **Walking round:** a floating panel's back is drawn on its reverse face. Its chip may also offer "walk round", moving the participant behind it by the existing locomotion where the scenery allows.
- **The agent's `flip_panel`** flips a panel for everyone while explaining it.

**Decided by the maintainer, November 2026: flips are private by default, with a way to flip for everyone.** A participant's flip shows the back to them alone, as the trigger's focus is private. To show others, the ⟲ chip has a second action, **flip for everyone** (holding the chip, or its menu), and the agent's `flip_panel` flips for everyone when someone asks; a shared flip is announced on the status wall and lasts until anyone flips the panel back for everyone. A private flip and a shared one combine: anyone may still flip a panel back to its front for themselves.

### Wiring and cables

The jacks are the channels a panel subscribes to (inputs, on the left) and publishes (outputs, on the right), each labelled with its name, type and current binding (`time · inherited · last 1 h`). A wall has a jack strip of its own on its frame for the channels it sets. **Cables** are sagging tubes between jacks, coloured by the channel's type, drawn only while an end is flipped or a wall's "show wiring" chip is on: from a panel's input to its wall's strip where it inherits, to another panel's output where it is linked, and to the declarer (the environment's object, or the Tomori wall) where it follows the space. Participant channels are drawn dashed, since they carry each person's own value. Between walls, cables route along the floor. A participant links by dragging from an output jack to an input jack on flipped panels, or unlinks by pulling a cable out; the agent uses `set_channel` with `link`. Links are kept with the space; one that would close a cycle is refused.

## 7. Web rendering and reports

### The same description in a browser

- **Flat scenes and charts** become SVG on the server, in Rust, from the same description: no script in the page, printable, and safe to mail.
- **Deep scenes** are drawn in WebGL2 by a viewer Tomori serves as a static asset, reading the description and its diffs from a read-only feed (server-sent events) while the page is open: orbit, zoom, hover for `detail`, click as `Trigger`. A small hand-written WebGL2 viewer is quickest, since the primitives are few; building `tomori-scene` and `tomori-render` to WebAssembly with `wgpu`'s WebGL2 backend would draw identically to the headset at the cost of megabytes and a build target. Hand-written first, WebAssembly once the description settles.
- **Events from the web** reach visualisations as from a headset, from signed-in owners and collaborators only; a share link's viewer can orbit and hover but not act.
- The back of a panel is a toggle on its card.

### Reports

A **report** is an artefact the agent composes from a conversation with `create_report`: title, the question, narrative sections in Markdown, panels by id (each **frozen** as a snapshot when composed, by default, or **live**), data tables, conclusion and next steps; kept with the space, versioned, edited with `update_report`.

- **URL:** `https://SERVER/reports/ID`, with an unguessable 128-bit id.
- **Access:** signed-in owners and collaborators by default. A **share link** is minted by an owner, read-only, expiring after 7 days unless set otherwise, listed and revocable on the status page, and frozen-only unless the owner allows live panels, since a live panel lets its viewer cause reads under the space's grants.
- **Rendering:** Markdown to a fixed set of elements on the server, raw HTML refused, every string escaped; `Content-Security-Policy: default-src 'none'` with only the viewer's own script allowed, by hash.
- **Static export**, `export_report`: one self-contained HTML file in the session's directory with the narrative, SVG figures, frozen 3D scenes with the viewer inlined, the data as JSON and CSV, and every figure's provenance; it opens offline.

This changes `security.md`'s rule that the web pages are "plain HTML ... without scripts": the viewer is Tomori's own script, never the agent's. It needs the maintainer's agreement.

## 8. Phasing

Each phase ends with something the maintainer uses in the headset, in Observe's space on a Quest 3, and is judged there before the next starts. Estimates are for one agent working with the maintainer's review, tests and headset tuning included, and are meant to be realistic rather than hopeful: about 60 days for phase 1.

| Phase | What | Days |
| --- | --- | --- |
| 1(a) | The back of a panel, for Observe's charts | about 7 |
| 1(b) | The core, the adapter, the scene panel, and the faster-pods scatter following `time` through the bridge | about 27 |
| 1(c) | Channels proper: `cursor.time`, `hovered_series` and `hovered_pod`, jacks and cables on backs | about 14 |
| 1(d) | Tomori's crate graph with grab and pull, including #534's prerequisite phases | about 12 |

### Phase 1(a): the back of a panel, for Observe's charts (about 7 days)

- **`tomori-environment`:** `Back` (rows, links, actions) and `Panel::with_back`.
- **`tomori-protocol`:** `Panel::back`, `Hello::backs`, `ClientMessage::FlipPanel`.
- **`tomori-server`** (about 2 days): keeping and bounding backs (30 rows, as a list's); an automatic back for every panel (identity, origin, age, version, and its context bindings read-only); the shared flip state of each panel and its announcement; `flip_panel`, for everyone, used when someone asks; `panels` and `inspect` listing backs.
- **`tomori-scene`** (about 3 days): the back's canvas from rows; the ⟲ chip with its two actions, flip for me (the default) and flip for everyone; the flip animation; the reverse face of a floating panel; the wrist roll while carrying (private). Flips are **private by default, with a way to flip for everyone**, as the maintainer decided (question 7): a private flip is client-only view state and sends nothing; a shared flip is a `ClientMessage::FlipPanel { shared: true }` or the agent's `flip_panel`, kept by the server and sent to everyone.
- **Observe** (`environment-observe-server`, `environment-observe`; about 1 day): its agent charts publish a back with the source's name, each series' label and full PromQL, window and step, refresh cadence, when last read and how long it took, series kept of how many, whether partial; actions Refresh, Freeze and Clear.
- **Judged by:** on the bridge, the maintainer flips one of a terminal's charts and knows what it queries, over what window and how fresh it is, without asking; flipping back is as easy; the walls stay readable.

### Phase 1(b): the core, the adapter, the scene panel and the faster-pods scatter (about 27 days)

- **`tomori-visual-core`** (about 6 days): the trait and its types, sources and provenance, the scene description in the core's own types (`Graph`, `Shape`, `Colour` by role, `Extension`), the builder with scales, diffing, the `ChannelHost` trait (only host-backed channels in this phase), the test harness.
- **`tomori-visual-runtime`** (about 3 days): applying diffs, easing within its budget, `hover`, picking, level of detail.
- **`bake`** (half a day): the dependency rules in `bake/tests/dependencies.rs`.
- **`tomori-protocol`** (about 2 days): its own `scene` module, `Panel::scene`, `Hello::scenes`, `Limits::scenes` and `Limits::items`; older peers sent the summary.
- **`tomori-visual-adapter`** (about 5 days): kinds and sources registered from `ServerBuilder` and environments; the evaluation pool and scheduler honouring `Next` within the caps (section 12); scenes ↔ panels, converted to the protocol's types, diffs as live data; **the context bridge for `time` and parameters**; instances, saved state and last scenes across restarts; events from intents; the tools (`visualisations`, `show_visualisation`, `update_visualisation`, `visualisation_logs`, `clear_visualisation`); pointing notes from `detail`; backs from `Visualisation::back`.
- **`tomori-server`**, **`tomori-environment`** (about 1 day): wiring the adapter into the hub and stage, the MCP tools, `Environment::with_visualisation` and `with_source`.
- **`tomori-prometheus`** (about 1 day): `Source` behind the `visual` feature, and `promql::guard`, moved from Observe with its tests.
- **`tomori-scene`** and **`tomori-render`** (about 5 days): the scene panel: a runtime per panel drawn through the instanced solids, tubes and glyph atlas, axes and ticks, the legend, flat scenes on the face and deep ones in front of the plate, slot and floating sizes, trigger focus, grab and release sent as intents, the frame budget shared with volumes; lavapipe renders.
- **`observe:faster-pods`** in Observe (about 3 days): as below, with a fixture source and golden scenes.
- **Headset measurement and tuning** (about half a day): the checklist in section 12.
- **Judged by:** the agent shows the faster-pods scatter from a sentence; the maintainer reads "faster pods get more traffic" from it at a glance, turns it round, points at a pod and hears its figures, grabs one to see its trajectory; setting the wall's time chip to the last 6 hours changes the scatter and the charts beside it together; frame time stays within budget.

**`observe:faster-pods`**, a module of Observe's server crate (`environment-observe-server`, its pure parts in `environment-observe`) that imports only `tomori-visual-core` and `tomori-prometheus`'s source, registered through `Environment::with_visualisation`:

- **Sources:** `prometheus:observe`. **Parameters:** `top` (pods, 300 by default).
- **Channels (bridged):** follows `time`, inherited from the wall it stands on, and Observe's `region`, following the bridge's open region.
- **Update:** on `Wake::Channels` and on its schedule: two `range_all` reads at a step chosen from the window (p95 request queuing by pod and revision, and requests a second by pod and revision), joined on pod and time; `Every(1 min)` while viewed and the window is live, else idle; a fixed window read once.
- **Draw:** about 3,600 points, requests a second across, p95 up on a log scale, time in depth, coloured by deployment revision (`Colour::Category`); axes, a legend, and a regression line per revision.
- **Events:** pointing tells the agent the pod's figures; the trigger focuses one revision; **grab** draws that pod's trajectory through the window and lifts its label (no behaviour moves the point; the visualisation owns the meaning, section 1); **both** hands on two pods draw both; release clears.

### Phase 1(c): channels proper (about 14 days)

- **`tomori-visual-core`** (about 3 days): the channel store for channels the host does not back: shared and per-participant holding, scopes, lifetimes, rates, settle in topological order, echo suppression, the 8-tick guard, cycle refusal, expiry; `Show::Mine` and `Show::Everyone`.
- **`tomori-visual-runtime`** (about 2 days): per-participant channels settled locally each frame, expiry, relay hooks; `publish_on_hover` and `emphasise_when`.
- **`tomori-visual-adapter`**, **`tomori-protocol`**, **`tomori-server`** (about 3 days): relaying per-participant values with presence at their declared rates to those who can see the scope; `channels` and `set_channel` (bridged channels read and set through the context model; `contexts` and `set_context` unchanged).
- **`tomori-scene`** (about 3 days): **the linked time cursor on `cursor.time`**, live charts at their wall's scope and fixed windows at the space's, its existing tests in `projection.md`'s validation passing unchanged; `hovered_series`; the back's jacks from channels, with cables to the wall and to the declarer.
- **`tomori-environment`** and **Observe** (about 3 days): a client side reads and sets per-participant channels (`ClientContext::channel`, `publish`); the scatter publishes `hovered_pod` from its points and emphasises it; Observe's client side lights **that pod's rod on the region disc** when it is hovered in the scatter, and publishes `hovered_pod` when its own rods are pointed at; the scatter draws a plane at `cursor.time`.
- **Judged by:** pointing at a time on one of a terminal's charts shows readouts on its other two, exactly as today, and a plane in the scatter on the same wall; hovering a pod in the scatter lights its rod on the disc, and pointing at a rod brightens its points, both at frame rate; flipping a panel shows its channels and cables.

### Phase 1(d): Tomori's crate graph (about 12 days)

- **#534's phases (a) and (b)** (about 6 days, its own estimate): the visual encoding, and grab and pull on a single client. This proposal adds no second encoding or solver.
- **`tomori-visual-core`** and **`tomori-visual-adapter`** (about 2 days): `Graph` converted into a volume's `Diagram` with kinds and groups; grab, move and release on a graph's nodes sent to the visualisation as well as driving #534's springs (section 1).
- **`tomori-visual-kinds`** (about 3 days): the `cargo:workspace` source and `tomori:crates`: crates as nodes, kinds by role, groups by family, dependencies as edges; `Every(10 s)`, drawing again only when `Cargo.lock` or a manifest changed; **Choose** (A) on a crate publishes `crate` (a shared channel from 1(c)) and collapses everything beyond `depth`; on release inside the box the visualisation pins the node, outside it lets #534's home springs return it.
- **Headset tuning** (about 1 day).
- **Judged by:** the maintainer pulls `tomori-scene` towards themselves and sees its inputs fan left and outputs right; the graph follows a dependency added in the code within ten seconds; another participant sees the pull (once #534's phase (c) shares it; until then, it is local, as #534 phase (b) is).

### Later phases

| Phase | What | Crates | Days |
| --- | --- | --- | --- |
| 2 | Faster turnaround (sections 9 and 10), once phase 1's interface has been used: the workbook language's spike (5 to 10 days) and, only if it feels good in the headset, its growth (45 to 60 days more, section 10), or another route | the core (a run-time `Visualisation`), a new host crate per route | 8 to 20 by route |
| 3 | **Migrating the context model onto channels**: undeclared settings kept, instances, held windows, pin and freeze, resting, chips and messages on the wire, the agent's tools, all on the core's store, with every existing test passing unchanged | `tomori-visual-adapter`, `tomori-server`, `tomori-environment` | about 15 |
| 4 | The web view: scene pages, SVG, the WebGL2 viewer, the feed, events from signed-in participants | `tomori-visual-svg`, `tomori-visual-web`, `tomori-server` (`pages`) | about 10 |
| 5 | Reports: `create_report`, `update_report`, `export_report`, share links, CSP | `tomori-server`, `tomori-security` | about 8 |
| 6 | Wiring by hand: dragging cables between jacks, `Setting::Linked`, cables between walls, "show wiring" | `tomori-protocol`, `tomori-visual-adapter`, `tomori-scene` | about 5 |
| 7 | Snapshots and export: freezing scenes, `export_data` | `tomori-visual-adapter`, the core | about 4 |
| 8 | More behaviours (drag, orbit, generic springs) and extensions over #306's drawers | `tomori-visual-runtime`, `tomori-scene`, #306's crates | about 4 each |
| 9 | More sources: files, git, the agent's data, environments' own | `tomori-visual-kinds`, `tomori-visual-adapter` | about 2 each |
| 10 | Extracting the core to `base-rs/visual` as `base-visual` | the core, its own repository | about 3 |

## 9. Faster turnaround, later

Once the trait has been used by the two prototypes and a few more kinds, the agent should be able to make a visualisation without a rebuild. Each route below implements `Visualisation` for code that arrives at run time, so the interface does not change. The decision is deferred; this section records the comparison to make then.

| | Dynamically loaded Rust (`libloading`, `abi_stable`) | Interpreter: JavaScript on QuickJS (`rquickjs`) | Interpreter: Luau (`mlua`) | Interpreter: Rhai | WebAssembly (`wasmtime`) |
| --- | --- | --- | --- | --- | --- |
| Turnaround | A crate build (seconds to a minute), then load | Instant | Instant | Instant | A build to `wasm32`, then load |
| How well the agent writes it | Well; the same Rust as phase 1, the compiler's errors are excellent | Best: the most visualisation code in its training (D3, Three.js) | Good; 1-based arrays trip it | Weak: little training data | As the source language (Rust, AssemblyScript) |
| Interface binding | Free, but needs a stable ABI: `abi_stable`'s types, or a C-shaped facade | Bindings for the scene builder and sources; JSON-shaped data is natural | Bindings, as for JS | Bindings, simplest to write | A WIT interface (component model), generated bindings |
| Unloading and crashes | Unloading is unsafe in practice; a panic or fault takes the server down unless in a process of its own | Clean | Clean | Clean | Clean, trapped |
| Sandboxing, when wanted | None in-process; a separate process needed | Memory limit, interrupt handler, no I/O; C engine | Memory limit, interrupts, sandbox mode; C++ engine | Operation and size limits; safe Rust | The strongest: fuel or epochs, bounded memory, no ambient I/O |
| Speed | Native | A fast interpreter | The fastest interpreter | The slowest | Near native |
| Runs in a browser too | No | Yes: the same engine builds for `wasm32-unknown-unknown` | Yes, a WebAssembly build | Yes | Natively |
| Size in the server | None | About 1 MB | About 2 MB | About 1 to 2 MB | 10 MB or more with Cranelift |
| Licence | MIT / Apache-2.0 | MIT | MIT | MIT / Apache-2.0 | Apache-2.0 with LLVM exception |

Sizes are rough and to be measured. A first reading, not a decision: **dynamically loaded Rust** keeps one language and the compiler's checking, and suits a maintainer happy with a rebuild of one crate rather than the server; **JavaScript on QuickJS** is the strongest interpreter for an agent writing code mid-conversation, and its browser build would let a static report keep its interactivity offline; **WebAssembly** is the natural boundary once security matters, since its interface (WIT) can be generated from the same trait. These could coexist: loaded Rust for the maintainer's own kinds, an interpreter for the agent's throwaway ones.

**A purpose-built language** is a further option the maintainer raised: a small declarative workbook language with its own Rust interpreter, rather than any general-purpose language above. Section 10 develops it; if accepted, it replaces the interpreter columns of this table as phase 2's route for the agent.

## 10. A purpose-built visualisation language, later

The maintainer asked to consider a language made for this, rather than JavaScript, Lua, Python or Ruby, which bring a general-purpose language's baggage: a standard library to fence off, modules, ambient clocks and randomness, unbounded loops, and an object model the sandbox has to understand. What he wants is mostly declarative and "fourth-generation", SQL-like in spirit, organised as a **workbook** (as in MATLAB, Jupyter or Observable), where a set of data definitions (get this series, get another, join them, transform them) feeds **several visualisations**, run by a **simple interpreter written in Rust** whose priority is sandboxing, not speed. He considered WebAssembly as the target for scripts and found it slower in practice and more than this needs.

This section records the language as the maintainer decided it in a long design conversation in November 2026, with the recommendations of a research note on prior art, *Prior art for a visualisation workbook language* (`vis-language-research.md`, November 2026, kept with the Observe prototype's notes; cited below as "the research note"), and the maintainer's decisions on an independent review of this section. **The order does not change**, as the maintainer decided: the Rust interface and the hand-written faster-pods visualisation come first (phase 1); then a **language spike** of one to two weeks, a parser and interpreter for just the lead example's features, driving the same visualisation through the same interface; and the language grows only if it feels good in the headset.

### Why a language of its own fits here

- **The domain is narrow.** Nearly every visualisation asked for so far is: read a few series or tables from declared sources, reshape them, map columns onto marks, react to a few gestures. That is a query pipeline plus a grammar of graphics, not a general program, and a language containing only those things has nothing to fence off.
- **Sandboxing by construction.** No `while`, no recursion, no I/O, no clock, no randomness, no reflection, no exceptions. The only effects are reading declared sources and, in handlers, writing channels. Every workbook terminates, and fuel and memory are counted by the interpreter itself, identically on the server and in the browser.
- **The dataflow is visible.** Definitions name what they use, so Tomori knows what to recompute when a source refreshes or a channel changes, what each panel's back should show, what a report must freeze, and what can be cached.
- **The agent can write it, if it looks familiar.** Its shapes come from SQL, PromQL, PRQL, Vega-Lite, Ruby blocks and OpenSCAD's keyword arguments, all of which models write fluently, without their runtimes.
- **Parity without cross-compiling.** The interpreter is plain Rust, so it builds once for the browser as WebAssembly; workbooks are never compiled to WebAssembly or JavaScript.

### Precedents, and what we take

From the research note, which reads each system's documentation:

| Precedent | Take | Avoid |
| --- | --- | --- |
| **Observable** and **marimo** (reactive notebooks) | Any-order single definitions forming a DAG; cycles and duplicate names refused at load, naming the sites; `viewof` as the model of an input (a widget and a value); demand-driven evaluation; invalidation of in-flight reads when an input moves; marimo's cache keyed on code hash and input content | `mutable` and side effects in cells; implicit globals such as `now` that break cache keys |
| **Starlark** (Bazel) | The chain that makes caching safe: top-level single assignment, frozen values, deterministic iteration, no ambient I/O, no `while`, no recursion, no exceptions; static name resolution | Its Python syntax, which invites general-purpose habits |
| **Vega-Lite**, grammar of graphics | Encodings as named arguments with defaults from field types; composition by layering, faceting and concatenation; `resolve` for shared or independent scales; selections that are both a value and a predicate | JSON as the authoring surface; selections defined by DOM event strings |
| **PRQL** | A pipeline step is function application with the subject as an argument: here `a.f(x)` means `f(a, x)` | Space-separated application; raw SQL holes stitched into text |
| **OpenSCAD** | Pure modules with positional and keyword arguments and defaults; caching each call by its code and evaluated arguments | Reassignment's surprising "last value wins"; dynamic `$` variables; unknown keywords silently ignored (here they are errors) |
| **Malloy** | Presentation hints as metadata apart from the computation; `nest` as explicit cardinality; per-run parameters (givens) beside per-instance ones; `$name` only inside expressions | Re-implementing SQL dialects; a relational-only core that does not fit time series |
| **Evidence** | One document that is both an interactive view and a shareable report; named queries composed by name; a typed form of the "optional predicate" | Templating values into SQL text, even with automatic quoting |
| **sqlx** and **F# type providers** | Let the engine describe a query (prepare, `DESCRIBE`, a PromQL parse plus a labels probe); cache descriptions by `hash(block, schema version)`; structural, lazily provided types; `Option` wherever a source cannot promise presence | Requiring a live source to type-check (here: "unchecked, source unreachable"); inferring types from samples without widening |
| **Kusto** and **Grafana** (the cautionary pair) | Kusto's typed `query_parameters`, sent apart from the text | Grafana's and Workbooks' text interpolation, whose long list of formats (`csv`, `regex`, `singlequote`, `glob`…) still leaves injection and multi-value bugs |
| **Racket readers, Raku slangs, Rust macros, heredocs, Markdown fences** | Delegating the end of an embedded block to a small per-language skimmer, with a line-based fallback and a raw delimiter | Delegating to a full parser (one typo loses the rest of the file); Perl's source filters |

### The lead example

The maintainer's example, revised after an independent review of this section and the maintainer's decisions on it (dotted columns, `==`, `own`, interpolated strings, `to_map`):

```
import tomori.charts.{scatter3d, timeline, table, sparkline, callouts, highlight, rules}

input  region: Region
input  time as window: TimeRange
input  notes: Map(Pod, Text)
own    focus: Set(Pod)                 # one value per participant
shared selected: Set(Pod)              # read and written: `each` reads it

let rate = promql! { sum by (pod, deploy) (rate(admitted_total{region=$region}[5m])) }
let lat  = promql! { histogram_quantile(0.5, sum by (pod, le) (rate(latency_bucket{region=$region}[5m]))) }
let deps = sql!    { select at, sha from deploys where region = $region and at >= $window.start }

let data = rate.over(window).join(lat.over(window), on = .pod).latest

let hot       = data.where(.rate > 2 * data.mean(.rate))
let hot_notes = hot.to_map(key = .pod, value = |r| "#{(r.rate / data.mean(.rate)).round(1)}× its peers' traffic")

let scatter = data.scatter3d(x = .lat, y = .rate, colour = .deploy, key = .pod)
  + hot_notes.callouts()
  + notes.callouts()
  + focus.highlight()

scatter.on(choose, |p| selected = selected.toggle(p.pod))
scatter.on(hover) do |p|
  focus = Set(p.pod)
end

let trend = rate.over(window).timeline(by = .deploy)
  + deps.rules(x = .at, label = .sha)

let slowest = data.sort(.lat, descending = true).top(10).table(columns = [.pod, .deploy, .lat, .rate])
  + hot_notes.callouts()
  + focus.highlight()

let each = selected.map(|p| data.where(.pod == p).sparkline(y = .lat))
```

What it does: two PromQL queries and one SQL query, each parsed and bound to the `region` channel as AST nodes (the SQL block's `=` and the PromQL matcher's `=` are those languages' own syntax, untouched by the host's rules); a join on `.pod`, a label both PromQL results are known to carry, checked before anything runs; the pods carrying more than twice the mean traffic, with a note each saying by how much (`"2.3× its peers' traffic"`); four views, each a panel by default (`scatter`, `trend`, `slowest`, and `each`, one small panel per selected pod); annotations layered as data (`hot_notes` derived, `notes` set from outside); `focus` held **per participant**, so each person's hover highlights their own pod in the scatter and the table, never someone else's; and `selected` **shared**, chosen in the scatter by anyone, opening a sparkline panel per pod for everyone.

### The language

**1. Interface: `input`, `output`, `shared`, `own`.** A workbook's whole interface with the world is channels (section 1); there is no other external mechanism, and constants or dashboard-style variables are simply channels that rarely change.

- `input name: Type`: read-only; a dependency, so a change recomputes what depends on it.
- `output name: Type`: written only by handlers, **never read by this workbook's definitions** (its handlers may read it, below); how a workbook offers a selection or focus to others.
- `shared name: Type`: read and written; one value for everyone in the space.
- **`own name: Type`**: read and written; **one value per participant**, a person, the agent, a spectator or an automated observer, the same across all of that participant's panels. A hover focus is `own`; a selection made for the room is `shared`. A definition that depends on an `own` channel is evaluated per participant, so it is limited to cheap layers over values already computed (highlights, callouts, filters of a computed table), cached by the channel's value so participants with the same focus share it, and capped.
- The binding's name defaults to the channel's; `input time as window: TimeRange` aliases it. A channel's name may be qualified with dots (`cursor.time`, `observe.region`); a binding's name is always simple, so a qualified channel needs `as` (`output cursor.time as cursor: Option(Instant)`).
- The checker flags a `shared` or `own` never written (it should be an `input`), an `output` never set, and an `input` never read.
- An `Option` input removes what it parameterises: `input deploy: Option(Sha)` drops the matcher from the query when unset, the typed form of Evidence's optional predicate (the research note).
- **When several views write one output in a tick**, their writes fold in arrival order (below), so the last writer wins; or the declaration states a merge, `output selected: Set(Pod) resolve union` (or `intersect`), as Vega-Lite's `resolve` does for selections.

*A note on the name `own`.* Candidates were `personal`, `private`, `isolated` and `own`. `own` was chosen for now: it is neutral, not human-centric (the agent and observers hold values too), and pairs with `shared`; `private` carries access-control baggage, `isolated` suggests that no other panel sees the value (all of a participant's panels do), and `personal` is human-centric. To be revisited after use.

**2. `let` single definitions.** Immutable; in any order, the workbook being a DAG of names; a name defined twice is an error with both spans; a cycle is refused, naming it. Definitions never write channels; only handlers do. A top-level `view.on(event, …)` is a **handler declaration** attached to a view, not a mutation: the view's value does not change, and each view has at most one handler per event.

**3. `=` and `==`.** `=` binds, and only binds: a `let` definition, a keyword argument (`on = .pod`), or a channel write in a handler (`focus = Set(p.pod)`). **`==` is equality, always** (`where(.pod == p)`), with `!=`, `<`, `<=`, `>`, `>=`. Embedded blocks keep their own languages' syntax.

**4. Columns are dotted; bare names are definitions.** A **dotted name** refers to a column: `.rate` is shorthand for the row function `|row| row.rate`, and an expression containing dotted names becomes a row function (`.rate > 2 * data.mean(.rate)` is `|row| row.rate > 2 * data.mean(.rate)`). Dotted names are allowed only where the parameter's type expects a row function (`where`, `sort`, encodings such as `x =` and `key =`, `columns = [...]`, `on = .pod`, `aggregate`); anywhere else they are a type error. **The nearest table wins**: inside `data.where(.rate > 2 * data.mean(.rate))`, the second `.rate` belongs to `mean`'s argument, `data`, not to the row `where` is testing. A **bare name** is always a definition or a channel, never a column. An explicit closure, `|r| r.pod`, is always allowed.

*The shorthand is generic.* Wherever a parameter's type is a function of one value, an expression containing dotted names becomes that function, and each dot refers to that value: table rows (`data.where(.rate > 10)`), set elements (`selected.map(.region)`), graph nodes (`crates.nodes.where(.in_degree > 5)`), events, tree nodes and series points alike. It is Kotlin's `it`, Swift's `$0` or Scala's `_`, made explicit by the dot. A function of two or more parameters is written as an explicit closure (`|acc, x| …`). **Expressions over dotted names are allowed** wherever the shorthand is (`x = .lat / 10`, `size = .rate / data.mean(.rate)`), with units carried and checked (dividing a rate by a mean rate gives a plain number; adding a `Duration` to a `Number` is an error) and conversions written `.lat.in(ms)`; an axis's label defaults to its expression (`lat / 10`) and is overridden with `label =`. **Whole-table operations** (`mean`, `rank`, `quantile`) are table methods, `data.mean(.rate)`, computed once for the table, not per row, however many rows refer to them.

**5. Types.** Strongly typed, with inference. Type names are capitalised. Generic types are applied like functions, with parentheses and keyword arguments, never angle brackets: `Set(Pod)`, `Map(Pod, Text)`, `Series(Number)`, `Option(Region)`, and one notation for tables, **`Table(key = Pod, lat = Duration, rate = Number)`**. A row function returning `T` has type `Column(T)`. Tables have **width subtyping**: a table with extra columns is accepted wherever fewer are required, so a module asking for `Table(key = Pod, load = Ratio)` accepts any table keyed by `Pod` with a `load: Ratio` column. A type constructs its values (`Set(p.pod)`, `Option(c.key)` for a present value, `None` for an absent one). Enumerations' values are written `Type.value` (`DependencyKind.normal`). Types come from Tomori's built-ins (`Number`, `Text`, `Bool`, `Instant`, `Duration`, `TimeRange`, `Ratio`, `Bytes`, `Colour`…), from environments (Observe declares `Region`, `Pod`, `Workload`), or from the workbook. Results from sources are **structural and lazily provided** (the research note's erased type providers): a PromQL block's type is a table whose columns are its known labels plus its value, named after its binding (`rate`, `lat`); a SQL block's comes from its select list and the source's schema. Columns a source cannot promise are `Option`. **Units** are types from Tomori's built-ins, inferred from the source's metadata where it has them (Prometheus's metadata and the `_seconds`, `_bytes`, `_ratio` naming conventions, so a quantile over `_seconds` buckets is a `Duration`; SQL column types), carried through arithmetic, converted where compatible and refused where not, and used to label axes and format text; `.cast(.load, Ratio)` asserts one where inference cannot, checked when values arrive.

**6. Calls and expressions.** Method chaining is the pipeline: **`a.f(x)` means `f(a, x)`**, so library functions and methods are one thing and there are no classes; a call with no arguments may drop its parentheses (`.latest`, `.keys`). **Keyword arguments are OpenSCAD-style, `name = value`**, with defaults at the definition; an unknown keyword is an error with a suggestion. Closures are `|p| expr` for one-liners and `do |p| … end` for longer blocks. **Braces are reserved for embedded blocks.** The rest, briefly: comments run from `#` to the end of the line; `if c then a else b` is an expression; tuples are `(a, b)` and lists `[a, b]`; `and`, `or` and `!` are logical (`!` is not, and is distinct from a tag's `name!`, which needs a name immediately followed by `!` and a brace, `name! {` or `name!{`); arithmetic is `+ - * /`, with `+` also layering views. There is no dynamic scope: ambient settings (theme, time zone) are inputs.

**7. Strings.** Text is written `"…"` and interpolated Ruby-style, **`"#{expr}"`**. A value formats itself through its type and unit (`"#{lat}"` reads `212 ms`, `"#{share}"` reads `37 %`), with methods for the rest (`"#{x.round(1)}"`); there is no format mini-language. **Interpolated text is for display only**: escaped wherever a panel draws it, and refused as input to any source. Queries take values only through `$` parameters bound as values (item 8), so no string a workbook builds can reach a query.

**8. Embedded sources: `name! { … }`.** `name` is a registered **source** (section 5), which declares its language and dialect (PromQL; PostgreSQL, ClickHouse or SQLite SQL). `promql!` and `sql!` are shorthands for the space's default source of each language; `observe_prod! { … }` names one explicitly.

- **Finding the end.** Each source supplies a small, total, versioned **lexical skimmer** that knows only its language's strings, quoted identifiers, comments and dollar quotes, and counts braces outside them; it finds the matching `}` even when the content is wrong. The same skimmer runs on the server, in the WebAssembly build and in editors, so exactly one function decides where a block ends (the research note's point about smuggling bugs when components disagree). If the skimmer fails (an unterminated string), the block ends at the first line holding only `}` at the block's indentation, the error is reported inside the block, and the rest of the workbook still loads. A **raw delimiter**, `name! r#{ … }#` with any number of `#`, covers anything else; the formatter switches to it when needed.
- **Parsing.** The exact span is parsed by the language's own parser: PromQL by [`promql-parser`](https://github.com/GreptimeTeam/promql-parser) (Apache-2.0, 0.11.1, declared compatible with Prometheus 3.8, used by GreptimeDB and OpenObserve; its error positions and the gateway's dialect to be confirmed in the spike), SQL by `sqlparser` in the source's dialect.
- **Binding, never interpolation.** `$name` and `$name.field` (an input or a `let` of a scalar, set or time type) are host tokens, recognised by the skimmer outside strings and comments and turned into typed placeholders before the dialect parser runs; `$<digit>`, `$$` and `$tag$` are refused at the start of a name, to keep clear of PostgreSQL's own forms. **Each source defines how each type binds in each position**, so the author never chooses a format: in PromQL a `Region` becomes a matcher's literal value, a `Set(Pod)` an `=~` of regex-escaped keys, an unset `Option` removes the matcher, and a `TimeRange` (through `.over(window)`) becomes the range and its step; in SQL every value is a bound parameter of a prepared statement, sent apart from the text. A value containing quotes, braces or keywords is just a value.
- **Checks on the parsed tree.** PromQL: every selector names a metric, ranges and subqueries at most a day, offsets at most a week, no `@`, the printed query within 16 KiB, a cost estimate from the tree and metadata (`promql::guard`, moved from Observe's lexer). SQL: read-only (a single `SELECT` or `WITH … SELECT`), only the tables or views the source allow-lists, only allowed functions, a row limit and a statement timeout added.
- **Validation against the source.** Metric and label names against Prometheus's metadata (labels, series and metadata APIs, read through the shared cached client, bounded and cached; `tomori-prometheus` adds these reads); SQL columns and types from the source's schema or by preparing the statement. Descriptions are cached by `hash(block, schema version)`, sqlx-style, so reloading a workbook checks it without querying, and an unreachable source leaves a block "unchecked" with a warning rather than failing it.
- **Result labels known statically.** `sum by (pod, deploy)` answers with exactly those labels, so `join(…, on = .pod)` and `colour = .deploy` are checked before any query runs.

**9. Imports for everything that renders.** The language core is data: sources, pipelines, joins, types. Every output comes from an **imported module** offered by Tomori or an environment: `import tomori.charts.{scatter3d, timeline, table}`, `import observe.plate`, `import … as name` for a collision, or a fully qualified name (`tomori.charts.scatter3d(…)`). An import brings free functions, callable as methods by the uniform call. Modules declare **typed signatures**, checked when the workbook loads: Observe's plate needs a table keyed by `Pod` with a load that is a `Ratio`. A module is the language-side face of the scene API (section 3) and of compiled Rust visualisations, which is also the escape hatch: anything the language cannot express becomes a module function written in Rust.

**10. Views are values; panels by default.** A view is a value like any other: it can be layered (`+`), passed to a function, inspected (`view.domain(x)`). **Each top-level view becomes a panel** by default; a switcher option instead shows alternatives in one panel, chosen by a chip. **The workbook panel** holds the code and can be tucked away; cables run from it to its view panels (section 6). **Each view panel's back** shows its own view definition, the names it uses, its data's age and its errors, with "show workbook" to bring the workbook panel forward.

**11. Handlers.** Stored with their view, run **only on events**: `choose`, `hover`, `leave`, `activate`, `deactivate`, and grab, move and release (section 1). A handler is a **pure function of the event and the current channel values** that returns new channel values, as in Elm: `selected = selected.toggle(p.pod)` inside a handler is that return value, not an assignment. **Handlers may read any channel's current value**, outputs included, since they run at event time; **definitions may not read outputs**. Handlers read no source and define nothing. **Events within a tick are applied in arrival order, each handler seeing the result of the one before** (a fold), so two people toggling different pods in the same tick both count. The writes take effect for definitions on the **next tick**, so a handler is never re-entered within one and channel feedback between workbooks is one tick of delay, never a loop. Handlers are **never re-run when data changes**. In the first phases **handlers run on the server**; the instant feedback of a hover comes from Tomori's built-in client highlight of the hovered item (section 3), and the server's `own` focus follows within a tick. No interpreter runs on clients until phase L4.

**12. Cardinality, explicit.** Four forms, each a function over views, never an implicit loop:

- **Merge**: one view, the item as an encoding (`colour = .deploy`, `key = .pod`).
- **One panel per item**: `set.map(|p| view)`, as `each` above, with a **hard cap** (12 by default) and a stated overflow ("12 of 50 pods, by rate").
- **Facets**: small multiples in one panel, `view.facet(by = .region, scales = Scales.shared)`, `shared` by default since independent scales across small multiples mislead, `independent` when asked.
- **Chooser**: `set.choose(default = set.first)`, a chip or dropdown on the panel that becomes an `input`; Observable's `viewof`.

**13. Joins and aggregates.** A join states its key (`on = .pod`); where the type says a side is keyed, the checker knows whether the join is one-to-one, many-to-one or many-to-many. **An aggregate across a join that fans out** (summing a column of the "one" side after joining it to the "many" side, which counts it once per match) is **warned of** at check time, naming the join; Malloy's symmetric aggregates are the model if warnings prove not enough. **Sets and maps keep insertion order**, and every built-in is deterministic in order, so iteration, tables and scenes come out the same on every run.

**14. Identity and annotations.** Every mark carries its data's key (`key = .pod`), which is how hover, choice, focus and annotations find it, and how a pod in one view is the same pod in another. **Annotations are data**: key, time or range selectors plus content, drawn as layers (`view + notes.callouts() + focus.highlight() + deps.rules(x = .at)`), and shared across every view showing those keys. **Derived annotations** are `let` definitions and recompute with the data (`hot_notes`); **externally set ones** arrive through channels, written by the agent explaining, a participant, or another workbook (`notes`). **Attention is channels**: `focus` and `notes`, which the agent can set while it talks.

**15. Purity, CSE and caching.** Purity is defined by the language, not by convention: the same expression in the same context gives the same result. That allows **common-subexpression elimination** (`data.mean(.rate)` above is computed once) and **content-hash caching**. A definition's key is the hash of:

- its normalised code (names normalised, so the same expression in two workbooks can share an entry);
- its input values;
- **the identity of each source it reads and the space's grants**, so a result is shared only between workbooks that could read the same data, never across spaces with different grants;
- each source's **data version**: for Prometheus, which has none, the query's evaluation time aligned to the step (a sliding window reuses all but its newest step, the live edge keyed apart); for SQL, a source-declared version (a change counter, `max(updated_at)`) or else a time-to-live;
- and, for a cached failure, the fuel and memory limits it failed under.

**Sources declare determinism**: a query that is not deterministic (PromQL or SQL using `now()` or `random()`, or SQL rows read without `order by`) is neither cached nor shared, and the checker says so on the block. A definition that recomputes to an unchanged hash does not re-run its dependents (early cutoff, as in Salsa, rust-analyzer's incremental engine, which the research note suggests studying). Results are cached per call, as OpenSCAD caches module instances. Deterministic failures (out of fuel or memory) are cached under their limits; wall-clock timeouts never are. The language's cache sits above `tomori-prometheus`'s own, which already aligns ranges and asks only for missing steps.

**16. Sandboxing and termination.** The interpreter counts its own **fuel** (in operations, with built-ins charging per row) and **memory** (bytes in an interpreter-owned arena, source results charged as they enter), with the same limits on the server and in WebAssembly, so a budget is exceeded at the same step everywhere. Parse depth, string and collection sizes and call depth are bounded. There is no `while`, and **no recursion**: a closure cannot refer to itself (a `let` closure naming itself is a cycle, refused), and there is no self-application; **so every workbook terminates**. Iterative algorithms (force layouts, clustering, springs) are built-in or imported native functions with budgets of their own (`layout.force(…)`), never user loops. A host deadline is only a backstop. Errors are fatal for the definition that raised them; its dependents show "upstream failed", and a source failure is a typed result shown on the panel. The interpreter follows Rhai's "don't panic" rule, with its parser and skimmers fuzzed.

**17. Sharing.** A workbook and its `shared` channels are **shared in the space by default**: everyone sees the same views and selection. A personal variant is a copy. Only `own` channels (hover focus, the cursor) and fleeting per-person values differ between participants.

**18. Streams, later.** A live source could be typed as a stream (`Live(T)`), with a head the cache knows moves; for now a source's refresh recomputes its dependents within the caps.

### An Observe example: the plate

Observe's region disc of control rods, offered as a module with a typed signature:

```
# In Observe's module (Rust), declared to the language:
#   module observe.plate
#     fn plate(data: Table(key = Pod, load = Ratio), label: Option(Column(Text)) = None) -> View
#   types Region, Pod

import observe.plate
import tomori.charts.{highlight, callouts}

input region: Region
input notes: Map(Pod, Text)
own   focus: Set(Pod)

let load = promql! { max by (pod) (scaling_pressure{region=$region}) }   # its value column, load, is a Number

let rods = load.latest.cast(.load, Ratio).plate(label = .pod)
  + focus.highlight()
  + notes.callouts()

rods.on(hover) do |p|
  focus = Set(p.pod)
end
```

The plate's signature is checked at load: a table keyed by `Pod` with a `load: Ratio` column (extra columns allowed). Without `.cast(.load, Ratio)` the error says so: `plate needs load: Ratio; load is Number here (add .cast(.load, Ratio))`. Because `focus` is an `own` channel keyed by `Pod`, a participant hovering a rod highlights the same pod in the lead example's scatter and table in their own view.

### Two more examples

**Trial against control: the spread of per-pod intake.**

```
import tomori.charts.{lines, bars, rules}

input  time as window: TimeRange
input  trial: Workload
input  control: Workload
output cursor.time as cursor: Option(Instant)

let arms   = Set(trial, control)
let intake = promql! { sum by (pod, workload) (rate(root_admissions_total{workload=~$arms}[2m])) }

let spread = intake.over(window)
  .group(by = [.workload, .t])
  .aggregate(cv = stddev(.intake) / mean(.intake), pods = count())

let chart = spread.lines(x = .t, y = .cv, colour = .workload, y_from = 0)
  + rules(y = 0.1, label = "target")
chart.on(hover, |p| cursor = Option(p.t))

let now = spread.where(.t == spread.last(.t))
  .bars(x = .workload, y = .cv, colour = .workload, label = |r| "#{r.cv.round(2)}")
```

`$arms`, a `Set(Workload)`, binds as an `=~` of regex-escaped values; `cursor.time` is the per-participant cursor channel of section 1, so hovering the chart moves each person's own cursor on the other charts of the wall.

**Tomori's crate graph from `cargo metadata`.**

```
import tomori.charts.{graph}
import tomori.layout

input  depth: Number
shared crate as focus: Option(Crate)

let crates = cargo! { packages }.where(.workspace)
let deps   = cargo! { dependencies }.where(.kind == DependencyKind.normal)

let role = |c| if c.name.starts_with("tomori-client") then "client"
               else if c.name.starts_with("tomori-environment") then "environment"
               else "module"

let crate_graph = graph(nodes = crates, edges = deps, key = .crate, shape = role, colour = role,
                        layout = layout.force())
  .neighbourhood(of = focus, depth = depth)

crate_graph.on(choose, |c| focus = Option(c.crate))
```

`role` is a row function, so it is accepted where `shape` and `colour` expect one; `focus` is an `Option(Crate)`, written with the chosen node's key; the force layout is a native function with its own step budget. Pulling a node is the client's springs (section 1, who owns a grab); choosing one writes `crate`, which any other panel following it sees.

### Implementation

- **`tomori-visual-lang`**, a crate depending only on `tomori-visual-core`, under the same extraction rule (section 2). A hand-written lexer and recursive-descent parser with spans; a checker for names, single definitions, cycles, types, keywords, channel use and module signatures; a tree-walking interpreter over a small typed IR. Simplicity over speed: the language is glue.
- **Native built-ins do the work**: joins, aggregations, `rate`, windows, resampling, sorting, and every module function. A workbook spends almost all its time in Rust.
- **Sources register their languages**: a skimmer, a parser, a checker that yields the result's type, a binder per type and position, and a cost estimate, behind one trait in the core. PromQL lives with `tomori-prometheus`; SQL with a SQL source; `cargo!` with `tomori-visual-kinds`. The language crate knows no query language.
- **The same `Visualisation`.** A workbook's definitions run in one shared evaluator; **each top-level view is a `Visualisation`** whose `update` reads the evaluator's results, whose `draw` is the view's value, and whose `event` runs its handlers. Sources, channels, scheduling, scenes, backs and the agent's tools are those of the Rust prototypes, so nothing downstream knows whether a panel was compiled or written in a workbook.
- **In the browser**, the same crate builds for `wasm32-unknown-unknown` (phase L4) for offline reports that embed a workbook with frozen source answers, and for running handlers on a viewer's own client where that proves worth it; until then handlers run on the server and the live web view only draws scene descriptions.

### Agent authoring

- **Tools.** `show_workbook { id, source }` checks and evaluates the workbook and answers with each definition's status, type, rows, a few sample rows, time and fuel, and errors with spans underlined in an excerpt, including spans inside embedded blocks; its views appear as panels. `update_workbook { id, name, source }` replaces one definition and re-evaluates what depends on it. `workbook_value { id, name }` previews a value. `workbook_source { id }` returns the source with line numbers. `sources` lists the space's granted sources with their languages and how to discover metrics, tables or packages.
- **Validation results** come back with the first answer: unknown metrics, labels, tables or columns with suggestions; type and signature mismatches; guard refusals with the bound and the estimate; and, once run, each query exactly as sent, printed from its tree with bound values, which the panel's back shows too.
- **A workbook is an artefact**: a text file in the session's directory and kept with the space, its notes and views the skeleton of a report (section 7).

### Risks

- **Scope creep.** Every language grows. Mitigations: a tiny core (definitions, channels, calls, closures, embedded blocks, imports); no loops, recursion, classes or exceptions, ever; new power arrives as module functions in Rust.
- **Learning curve.** Familiar shapes, errors that suggest the fix, per-definition previews, and the lead example first in the instructions.
- **Tooling.** Errors with spans from the first version; the skimmers shared with a `tomori-highlight` grammar so workbooks read well on code panels; a tree-sitter grammar later.
- **Versioning.** A workbook states its version; the interpreter keeps reading every released version; skimmers and binders are versioned with their sources.

### Phasing

| Phase | What | Days |
| --- | --- | --- |
| L1, spike (after phase 1) | Just the lead example's features: `import`, `input`/`own`/`shared`, `let` with any order and single definition, dotted columns and `==`, `promql!` with the skimmer, `promql-parser` and `$region` bound as a matcher, `.over`, `.join`, `.where`, `.latest`, `scatter3d` with keyword encodings, `on(hover)` writing `focus` on the server; **basic type checking** (tables' columns from PromQL label sets, row functions, keyword arguments, channel kinds), the riskiest part; fuel and memory counted. Drives the faster-pods visualisation through the same interface. **Judged by:** its scene equals the hand-written kind's golden scene from the same fixture, type errors point at the right span, and it feels good in the headset when the agent writes and edits it | 5 to 10 |
| L2, only if L1 feels good | The rest of this section: the full type system with units, width subtyping and modules' signatures; `sql!` and `cargo!` with their skimmers and binders; strings; annotations; cardinality; merge rules and join warnings; the workbook panel; caching with early cutoff and determinism; the tools; the metadata reads | 45 to 60 |
| L3 | Workbooks as reports | about 5 |
| L4 | The interpreter in WebAssembly: offline reports, and handlers on viewers' clients if wanted | about 5 |

### Open questions

1. **The name.** It should avoid the names the research note lists as taken in this field (Lens, Loom, Weave, Mosaic, Lumen, Panel, Plot, Facet, Vega, Vista, Prism, Glyph, Atlas, Quill, Workbook, Sight, Scry, Kino, Marimo, Evidence, Malloy, Cube); a name tied to Tomori avoids most collisions.
2. **The file extension**, checked against existing uses (`.vis`, `.wb` and `.lens` are taken).
3. **Editor support**: the highlighting grammar from the skimmers first, a language server later, or only Tomori's own panels?

## 11. Security, deferred but bounded

Phase 1 visualisations are server code, written or reviewed like any other, with the same trust as an environment's server side; the agent can already change the server's code. So phase 1 adds no boundary beyond what exists, with four rules that cost nothing now and keep the later boundary possible:

- **Credentials stay in sources.** A visualisation sees `Sourced<T>` answers through typed handles, filtered by the source's label filter; no source gives out an endpoint or a token. A later sandboxed visualisation, in another process, can then be given handles that proxy over a pipe, holding no secrets.
- **Clients never run visualisation code.** Only Tomori's behaviours run on devices, so no later route changes what a headset or browser trusts.
- **Visitors cause nothing.** Their events never reach a visualisation, they write no channel, and share links are frozen by default.
- **What a person hovers stays theirs unless shown.** A per-participant value leaves its client only when a subscriber at its scope shows everyone's, and then only to people who can see that scope, never logged and never kept; otherwise only in a pointing note when they speak.

**A decision for the maintainer: who may read Prometheus.** A visualisation reading `prometheus:observe` can show production metrics in any space it is granted to, to everyone there, visitors included. The choices: (1) **granted to every space by default**, which is what "usable in any space" asks and suits a server with one operator; (2) **only spaces that opt in**, by the operator's configuration naming them; (3) **owner-approved per space**, an owner granting it from the hand menu or the status page, shown on the space's wall. Recommended: Tomori's builder defaults to (2), so a source reaches no space unless configured, and Observe's server configures (1) for its own deployment, where the operator and the owners are the same people; (3) once spaces are shared more widely.

The boundary to define with phase 2, once the interface is known: who may create visualisations from code arriving at run time (the agent only, acting for owners and collaborators, is the obvious start); a separate, credential-free worker process for run-time code whatever the route; limits on memory, CPU per call and source requests per space; what a persisted visualisation may do unattended; and what live report panels may read for someone outside the space.

## 12. Risks and open questions

### Risks

- **Quest cost.** Easing 16,384 items means rewriting their instances while a change settles: about 512 KiB of instance data a frame at 32 bytes each. Budgets: at most 4,096 items eased a frame on the headset, the rest snapping; only changed items eased, for 0.3 s; one scene easing at a time per frame turn, as faces are laid out two a frame; springs only on graphs, within #534's 64 nodes; the frame budget of volumes (100,000 triangles, 16,384 solids) shared; level of detail by distance (section 3). Measured on a Quest 3 at 2.0× with 2× MSAA before phase 1(b) is judged.
- **Server CPU.** Compiled updates cannot be preempted. Caps in phase 1: at most 16 visualisations a space and 64 a server; an evaluation pool of 4 threads unless configured, never the hub's lock; an update over 2 s logged on its back, one over 30 s abandoned (its thread marked hung, its instance not scheduled again, the pool replacing the thread); `Every` no faster than 5 s, and Prometheus reads no faster than the cache's bucket.
- **Bandwidth per participant.** A first scene at most about 160 KiB; diffs by item id, positions as `i16` and colours as roles, so a refresh of 3,600 moved points is about 36 KiB; at most one diff a second a panel, as published panels are paced; per-participant relays at most 10 a second, a few bytes each, only where someone shows everyone's. Whether the protocol's frames are compressed (deflate or zstd for scene frames over a size) is measured, not assumed.
- **Determinism between server and client.** The server's graph solver runs in `f64` and is the truth; the holder's client predicts in `f32` and is corrected by the server's positions, blended over 150 ms, as #534 proposes; the two are never compared bit for bit. Saved positions are rounded to millimetres, so restarts and tests agree.
- **The interface fits two examples and nothing else.** Before phase 2, a third kind of a different shape (a fleet heat map, a flame graph from a file) is written against it, and the trait changed where it strains.
- **Phase 1 tempts template thinking.** Compiled kinds with parameters are templates; they are the means to find the interface, and phase 2 returns "any code" to the agent.
- **Two ways to draw a graph.** `scene.graph` converts to a volume, so diagrams and visualisations share one drawing path.
- **Channels becoming KVO after all.** The wiring is always visible (backs, cables, `channels`); a change's cause is logged with the settle that carried it ("`time` set by Ada on wall Charts; woke 6 panels").
- **A reusable core shaped by one host.** The core knows only a scope tree, opaque participants and a clock; kinds import nothing of Tomori; a unit test drives the core with a host of its own.
- **Handler latency.** About 100 ms on a local network; anything tracking the hand is a behaviour.
- **Web posture.** Serving Tomori's own viewer script changes the pages' threat model; the CSP and server-made SVG keep it small.

### Test plan

- **The core and runtime, headless:** golden scenes from fixtures; diffs; channel settles (order, echo suppression, cycles, expiry, rates) with a toy host; the runtime's easing, picking and level of detail; the dependency rules.
- **The adapter:** the bridge against `panel-context.md`'s cases (inherited, fixed, instances, pinned and frozen copies, held windows); conversions into the protocol's types and back; scheduling caps; restarts.
- **Rendering, offscreen on lavapipe:** each primitive, flat and deep scenes in a slot and floating, the faster-pods fixture and the crate graph, saved as previews for review, as `TOMORI_VOLUME_PREVIEW` does for volumes; the spectator's render of a scene panel.
- **The linked cursor:** `projection.md`'s existing cursor tests passing unchanged on `cursor.time`.
- **Headset checklist, each phase:** frame time and GPU time in Observe's busy view with a scene at its limits; CPU time to apply a full scene and a refresh; easing while turning the head; readability of axes and labels from 2 and 4 m; pointing and grabbing at 1 m and 4 m; hover reaction at frame rate; a second participant seeing a grab; battery and thermal over 20 minutes.

### Open questions for the maintainer

1. **Where do kinds live?** Proposed: Tomori's in `tomori-visual-kinds`, Observe's in a module of its server crate, both depending only on the core and sources. Should an application be able to register kinds without an environment?
2. **Is `update` / `draw` / `event` the right split,** or should a visualisation build its scene directly in `update` and `event` (simpler, but no purity for tests and restarts)?
3. **Who controls refresh** when a visualisation's own schedule and a participant's #509 policy disagree? Proposed: the policy caps the schedule. Should refresh itself be a channel, so a wall's policy is inherited as `time` is? #509 already models it beside the context with the same inheritance, so it would fit.
4. **Channels' names and reach.** Proposed: Tomori's built-ins unprefixed (`time`, `cursor.time`, `selection`), an environment's its own (`region`, `hovered_pod`), all within one space. Should a channel ever cross spaces, such as one incident window followed by several spaces?
5. **Who turns on everyone's cursors?** Proposed: a subscriber's choice (`Show::Everyone`), set per panel by a chip or the agent. Should it instead be the publisher's consent, so a participant chooses whether their pointer is shown to others?
6. **Implicit by name, or explicit links only?** Proposed: panels follow a channel by name through the scopes, as context does today, with explicit links for the exceptions. Explicit-only would make every cable deliberate, at the cost of wiring every panel.
7. **Private or shared flip?** **Decided:** private by default, with a way to flip for everyone (the chip's second action, or the agent's `flip_panel` on request).
8. **Grants:** should Observe's Prometheus source be granted to every space by default, as proposed, or only to spaces an owner names?
9. **Does Observe's `chart` action become a visualisation kind** in phase 1 (a good third example), or stay as it is?
10. **Reports' share links:** frozen-only and 7 days by default? Owners only, or collaborators too?
11. **The web viewer:** hand-written WebGL2 first, as proposed, or straight to `tomori-render` in WebAssembly?
12. **Portals:** is a snapshot standing in front of its plate enough, or is a parallax window wanted enough to revisit #529?
13. **Phase 2's first route:** loaded Rust, as the nearest step from phase 1, or an interpreter, as the nearest to how the agent wrote Python?
14. **The core's name and home:** `tomori-visual-core` in Tomori's workspace, extraction-ready, then `base-rs/visual` as `base-visual` once it settles, as proposed? And should #534's spring solver live in the runtime?

## Design documents this would change

A new `design/visualisations.md` (the trait, later the workbook language and its embedded source queries, kinds, context, wakes, events, scheduling, channels and their propagation, the scene builder, tools, and later the run-time routes and their boundary); `design/projection.md` (the scene panel kind, `Hello::scenes`, `Limits::scenes` and `items`, the back of every panel, `FlipPanel`, scene diffs as live data); `design/data-sources.md` (the source registry, grants, `promql::guard`, provenance); `design/diagrams.md` (`scene.graph` as a way to reach a volume, with #534); `design/environments.md` (`Panel::with_back`, `with_visualisation`, `with_source`); `design/panel-context.md` (parameters and time as shared channels, scopes and inheritance, `Setting::Linked`, `channels` and `set_channel`), `design/projection.md`'s linked time cursor (as the participant channel `cursor.time`, and `hovered_series`); `design/mcp.md` (the visualisation tools, `channels` and `set_channel`, `flip_panel`, the report tools, `export_data`); `design/interaction.md` and `design/responders.md` (flipping, grabbing items in scenes); `design/rendering.md` (scenes in the volumes' budget, cables); `design/security.md` (report pages, share links, CSP, Tomori's own script in the web pages, and later the run-time boundary); `design/spectator.md` (drawing scenes); `design/clients.md` (the web as a client of scene descriptions); and `design/crates.md` (`tomori-visual-core`, `tomori-visual-runtime`, `tomori-visual-adapter`, `tomori-visual-kinds`, later renderers, their dependency rules, and the core's eventual move to its own `base-rs` repository).

## Decisions for the maintainer

1. **Private or shared panel flips** (question 7): **decided** by the maintainer, November 2026: private by default, with a way to flip for everyone when showing others (the ⟲ chip's "flip for everyone", or the agent's `flip_panel` on request). Phase 1(a) is scoped accordingly.
2. **Prometheus access by default** (question 8, section 11): every space, opt-in spaces, or owner-approved. Recommended: opt-in by default in Tomori's builder; Observe configures every space for its own deployment.
3. **Whether the context migration happens in phase 1.** Recommended: **no**; phase 1 uses the adapter bridge, and the migration is phase 3, about 15 days.
4. **The core's name and home:** `tomori-visual-core` in the workspace now, extraction-ready, moving to `base-rs/visual` as `base-visual` once it settles?
5. **Who turns on everyone's cursors** (question 5): the subscriber's choice, or the publisher's consent?
6. **Implicit by name, or explicit links only** (question 6): follow channels by name through the scopes, with links for exceptions, as proposed?
7. **Grants** (question 8): settled with decision 2.
8. **The workbook language** (section 10), as decided in November 2026: a spike after phase 1, grown only if it feels good in the headset; still open are its name (avoiding the names the research note lists), its file extension and editor support.
