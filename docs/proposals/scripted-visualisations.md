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

- **A visualisation is a Rust value implementing one trait, `Visualisation`**, in a new core crate with no Tomori dependency, proposed as `base-visualisation`, with Tomori as one host through an adapter crate (section 2). It declares the sources it reads and the parameters it takes; `update` reads sources and changes its own state, returning **when it wants to run next**; `draw` builds a scene from that state, purely; `event` receives pointing, grabbing, releasing and choosing, and returns the same. Tomori runs it on the server, diffs successive scenes and sends only what changed.
- **Kinds are registered; instances are made by the agent.** Tomori and environments register visualisation kinds (`observe:faster-pods`, `tomori:crates`), each with a typed parameter list like an environment action's. In phase 1 the agent's interface is `show_visualisation { kind, id, params, place }`: it chooses and parameterises compiled code. Later, a script or a loaded module is just another implementation of the same trait.
- **Inputs that are also outputs are channels**: named, typed, observable values in the space (`time`, `region`, `selection`, `hovered_pod`). A kind declares the channels it subscribes to and publishes; values are scoped space, wall and panel, with inheritance, which **is** today's context model, generalised rather than duplicated: a context parameter becomes a shared channel, a chip shows a channel's binding at a scope, and the linked time cursor becomes a built-in participant-local channel. Changes settle once per tick in dependency order, in one direction, with cycles refused and fast channels kept on the client. The wiring is listed to the agent and drawn as cables on panels' backs.
- **Sources are a registry of named, typed, read-only capabilities.** The server's builder and environments register them (`prometheus:observe`, which is Observe's shared cached client; `cargo:workspace`); a visualisation reaches them by name and type through its context; every answer carries its **provenance**.
- **Outputs are a scene description** (instanced points, bars, lines and tubes, meshes, labels, axes, legends, groups, and a graph primitive that is #534's interactive diagram), plus **declared behaviours** (springs, drag, easing, hover) that clients run natively at frame rate. Clients never run visualisation code; the headset, the spectator and the browser all draw the same description.
- **Every panel gets a back**, flipped by a chip, by turning it over in the hand, or by the agent, showing its source, query, window, refresh, provenance and errors, and its jacks; cables show which panels feed which.
- **The web renders the same description** (SVG for 2D, WebGL2 for 3D); **reports** compose narrative and frozen or live visualisations at a signed-in URL, with share links and a static export.
- **Crates of their own.** The trait, channels, source traits and the scene description are a core crate with no Tomori dependency, proposed as `base-visualisation` (a future `base-rs/visualisation` repository), beside a viewer runtime and separate renderers; Tomori is one host, through an adapter crate that maps channels onto the context model and scenes onto panels. CI checks that the core never depends on Tomori, and that kinds depend only on the core and sources.
- **Phase 1, in Observe's space:** (a) the back of a panel for Observe's charts, about 5 days; (b1) the core, the runtime, the Tomori adapter and the scene panel, about 13 days, and (b2) channels, with the context model and the linked cursor moved onto them, about 8 days; (c) two compiled visualisations written only against the core and sources: Observe's faster-pods 3D scatter, following the shared `time` and publishing `hovered_pod` to light that pod's rod on Observe's region disc, about 4 days, and Tomori's crate graph with grab-and-pull springs reusing #534, about 3 days on top of #534's phases (a) and (b). Faster turnaround, the web, reports and hand wiring come later; the choice between dynamic Rust, an interpreter and WebAssembly is deferred until the interface has been used.

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

In the core crate, `base-visualisation` (section 2), which depends on nothing of Tomori:

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
    Every(Duration),         // a cadence, at least a second
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

Events run on the server, so a reaction arrives about a tenth of a second later. That suits "show this pod's trace"; anything that must follow the hand at frame rate is a declared behaviour (section 3) or a per-participant channel evaluated on the client (below).

### Channels: inputs that are also outputs

A **channel** is a named, typed, observable value in a space. Visualisations, environments' sides, Tomori's own panels, participants and the agent read and set channels; nobody reads anyone else's internals. This is KVO's idea with its scope made explicit: what is observable is declared, typed and listed, never any property of any component.

**Declaring.** A channel is declared by whoever owns its meaning, with a key typed at compile time:

```rust
pub const TIME: ChannelKey<TimeBinding> = ChannelKey::tomori("time");            // Follows, Live { minutes }, Window { from_ms, to_ms }
pub const CURSOR_TIME: ChannelKey<Option<Instant>> = ChannelKey::tomori("cursor.time");  // per participant, expiring
pub const SELECTION: ChannelKey<Keys> = ChannelKey::tomori("selection");

// In Observe's common crate:
pub const REGION: ChannelKey<Text> = ChannelKey::environment("region");            // with its choices, as a context parameter today
pub const HOVERED_POD: ChannelKey<Keys> = ChannelKey::environment("hovered_pod"); // per participant, expiring
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
pub const CURSOR_TIME: ChannelKey<Option<Instant>> = ChannelKey::tomori("cursor.time")
    .per_participant()                // a value for each person
    .scoped(Scope::Wall)              // an Observe terminal's three charts share one; another wall has its own
    .expiring(Duration::from_secs(1)) // cleared if its publisher stops: pointer lost, headset asleep, connection gone
    .relayed(Rate::per_second(10));   // what others need goes through the server, at most 10 a second

pub const TIME: ChannelKey<TimeBinding> = ChannelKey::tomori("time")
    .shared()                         // one value for everyone
    .scoped(Scope::Inherited)         // panel, else wall, else space: today's context settings
    .kept()                           // kept with the space; never expires
    .relayed(Rate::per_second(2));    // a chip pressed or the agent's set_channel, coalesced
```

**Publishing.** The chart pointed at publishes `Some(t)` as the ray moves along its plot, the instant under the ray, and `None` as the ray leaves it. Publishing is a value at the publisher's scope: the wall the chart stands on.

**Notification.** Every panel on that wall subscribing to `cursor.time` is told each transition, `None → Some(t)`, `Some(t) → Some(t′)` and `Some(t) → None`, once per frame at most, after the frame's writes have settled. Its response is its own: a chart draws its readout at `t` (its line down the plot there, a dot on each series and their values, fainter than the pointed chart's own) or hides it; a chart whose window does not reach `t` within half a point draws nothing; a scene with a time axis draws a plane at `t`.

**Per person.** The channel holds one value for each participant on the wall. Each subscriber chooses what it shows: **the viewer's own** (`Show::Mine`, a chart's default, which is today's behaviour) or **everyone's** (`Show::Everyone`), each participant's readout in their own colour with their name, the viewer's strongest, for a presenter walking a room through an incident.

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

**Scopes and inheritance, unified with the context model.** A shared channel has a value at up to three scopes: the **space** (what its declarer publishes; an environment's selection today), each **wall** and each **panel**. A panel's effective value is its own setting, else its wall's, else the space's, exactly the inheritance `panel-context.md` requires of parameters. A setting at a scope is one of: **follows** (inherit), **fixed** (a value), or, new, **linked** (follow a named channel that another panel publishes, `Setting::Linked { panel, channel }`), which is how one chart's brushed window drives another without a shared name. Chips draw a panel's or wall's binding of each channel it subscribes to, as they draw parameters today; instances (a copy shown with a different fixed value) and pinning by pulling off a wall work unchanged, since they are defined on settings.

A per-participant channel has no inheritance: its scope is where its publisher stands, a wall, the space, or the panel alone, and its subscribers at that scope hear it, as the worked example shows.

**Migration of what exists.** No wire change is needed at first; the server's model changes underneath:

- **Context parameters become shared channels.** `Space::set_context(Vec<ContextParameter>)` declares shared `Text` channels at space scope, with the side's selection as their space value; `Panel::depends_on` subscribes a published panel to them; `Intent::context()` is the notification of a change in a panel's effective values; `ContextChip` and `ContextSetting` show a channel's binding; `contexts` and `set_context` stay as the agent's names for `channels` and `set_channel` limited to these. `time` is the built-in `TimeBinding` channel. The bounds (3 parameters and time, 16 choices, 8 instances) stay until the model has been used.
- **The linked time cursor becomes `cursor.time`**, as the worked example above maps, rule for rule.
- **Highlighting a series across charts**, the maintainer's example, is then `hovered_series`, a per-participant, wall-scoped, expiring `Option<Keys>` channel, the cursor's twin: a chart's readout publishes the series pointed at by its key, and every chart subscribing brightens that series and dims the rest.

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

The visualisation decides when it runs: `Next::then` is its own schedule, so a chart of a 30-second-scrape metric asks for `Every(30 s)`, a clock asks `At(next minute)`, a crate graph asks `Every(10 s)` and reads again only when `Cargo.lock`'s digest changed, and a fixed window asks `Idle` once read. Tomori bounds it rather than overrides it: a panel's refresh policy, where #509 is accepted and someone has set one, caps the cadence (`manual` turns `Every` into `Idle`), the live-panel budget counts any visualisation woken more often than once a minute, and the server never wakes one more than once a second.

### Lifecycle

1. **Create.** The agent calls `show_visualisation`; the server checks the kind's parameters and the space's grants, calls `make`, then `update(Created)` and `draw`, and answers the tool call with the scene's counts, the provenance of every read, warnings and the log's first lines, or the problem in the kind's own words. A visualisation whose first update fails is not projected.
2. **Run.** Updates run on a pool of visualisation threads, never under the hub's lock, since a source read may block for seconds; `draw` runs after any call returning `redraw`, and the hub takes the diff. A call taking over 2 s is logged on the back; compiled code cannot be preempted, which is acceptable in phase 1 (section 10).
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

### The layout

| Crate | What it holds | Depends on | Built by |
| --- | --- | --- | --- |
| **`base-visualisation`** (the core) | The `Visualisation` trait, `Kind`, `Context`, `Wake`, `Next`, `Event`, `Problem`; channels: `ChannelKey<T>`, the value types, holding, scopes, lifetimes, rates, and the **channel store** with its settle (topological order, echo suppression, cycle refusal, expiry); the `Source` trait, `Sourced<T>`, provenance and the registry; the **scene description** (primitives, palettes and materials, groups and transforms, declared behaviours, items' keys, channel bindings), its builder, scales and diffing; and a test harness running a visualisation against fake sources with golden scenes | `serde`, `serde_json`, `glam`; nothing of Tomori, no `wgpu`, no protocol, no async runtime | Servers and clients (it holds no credentials) |
| **`base-visualisation-runtime`** | What a viewer runs: applying a scene's diffs, easing, the declared behaviours (springs, drag, hover, orbit), picking items with a ray, and per-participant channels settled locally each frame with expiry and relay hooks; no GPU, it yields instances, lines and labels for a renderer to draw | The core; #534's spring solver (below) | Clients, the web viewer (as WebAssembly) |
| Renderers | Draw what the runtime yields | The runtime and a graphics stack | Each its own |
| **`tomori-visualisation`** (the adapter, server side) | Tomori as a host: scopes ↔ walls and panels; channels ↔ the context model (parameters, `time`, chips, `Intent::context()`, `contexts` and `set_context`); scenes ↔ panels (`Panel::scene`, live-data diffs, limits); events from pointing and grab intents; registering kinds and sources from `ServerBuilder` and environments; the agent's tools' logic | The core; `tomori-protocol`, `tomori-environment` | `tomori-server` |
| Tomori's client side of the adapter | In `tomori-scene` (`projection::scene`): a scene panel's runtime, drawn through `tomori-render`'s solids, tubes and the glyph atlas; local channels (`cursor.time`) and relaying them with presence | The runtime | Headset, desktop, spectator |
| **Sources** | `tomori-prometheus` implements `Source` behind a `visualisation` feature (it is a leaf crate with no Tomori dependency, so the core's rule holds); `cargo metadata` and files in `tomori-visualisation-kinds` for now | The core, and their own clients | Servers only (the existing rule that no client builds `tomori-prometheus`) |
| **Kinds** | Tomori's built-in visualisations (`tomori:crates`) in `tomori-visualisation-kinds`; Observe's (`observe:faster-pods`) in a module of Observe's server crate | The core and the sources they read, **never Tomori's internals** | Servers |

**Renderers, in order.** Phase 1 draws scenes through Tomori's own pipelines, via `tomori-scene` and `tomori-render`, so nothing is drawn twice. Later, as each is needed: `base-visualisation-svg` (2D scenes and charts to SVG, pure Rust, for the web pages, reports and static export); `base-visualisation-web` (the runtime built to WebAssembly with a WebGL2 or WebGPU drawing of its output, served as the web viewer); and `base-visualisation-wgpu`, a standalone renderer for hosts other than Tomori, extracted from `tomori-render`'s instanced solids when someone outside Tomori needs it, not before.

**The springs.** #534's phase (c) proposes a shared `tomori-diagram` crate for the spring solver, depending on nothing beyond `glam`. It has no Tomori dependency either, so the runtime's `springs` behaviour should be that solver, not a second one: either the runtime depends on it, or the solver moves into the runtime and `tomori-diagram` keeps the layouts. To be agreed with #534.

**How the core reaches a host.** The core never names walls or panels. A host gives it a tree of scopes (each scope's parent: panel → wall → space in Tomori, and whatever a dashboard has elsewhere), the participants who may hold per-participant values, a clock, and a place to send scene diffs; the core's channel store resolves inheritance over that tree, and its `Context` hands a visualisation its sources and channels. The adapter is where Tomori's names meet the core's.

### Rules

- **The core and the runtime depend on no `tomori-*` crate.** `bake/tests/dependencies.rs` checks it in CI, as it already checks that no client builds `tomori-prometheus`, and checks that kinds depend only on the core and sources.
- **Sources are server-only**; the core and runtime are allowed on clients, since they hold no credentials.
- **Kinds see only the core.** A kind written against `base-visualisation` and a source can be lifted into another host unchanged; that is the test of the interface.
- **`tomori-protocol` carries the core's scene description** as `Panel::scene`, with the description's own format number, so a client can tell a description it cannot read from one it can; the protocol depends on the core, never the reverse.

### Name, home and versioning

- **Name:** `base-visualisation`, following the `base-rs` organisation's `base-*` crates (`base-http-server`, `base-http-client`), with `-runtime`, `-svg`, `-web` and `-wgpu` beside it. It is named so from the start, so that moving it changes no imports.
- **Home:** under `crates/visualisation*` in Tomori's workspace while the interface is being found, released with the workspace's one version (`0.x`, breaking changes accepted); then, once phase 2's turnaround work has used it from run-time code and it has stopped changing every week, its own repository, `base-rs/visualisation`, with its own semantic versions, which Tomori pins.
- **The scene description's format** is versioned apart from the crate: a number bumped on any change to what a viewer must understand, so that a headset, a browser and a server of different releases can tell whether they agree.

## 3. The scene and interaction API

A scene is a tree of items in the panel's own unit box, x, y and z each from −0.5 to 0.5, scaled to the panel as a volume is (`diagrams.md`). A scene of zero depth is **flat**, drawn on the face as a chart is; any other is **deep**, standing in front of its plate as a volume does. The description and its builder are Rust in the core, `base-visualisation`; `tomori-protocol` carries the description as `Panel::scene`.

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
| `scene.graph(id, diagram)` | **#534's interactive diagram**: a `tomori_protocol::diagram::Diagram` with kinds and groups, drawn by the volume code with its visual encoding and springs | Dependency graphs, topologies |
| `scene.extension(key, payload, fallback)` | An environment's face drawer (#306), else the fallback items | Shading or density a scene cannot draw |
| `scene.group(id, transform, …)` | A node of the tree | Moving, focusing or hiding items together |

Scales (`linear`, `log`, `time`, `band`) and helpers (`extent`, `bin`, `group_by`, `quantile`) live beside the builder, so a visualisation's code reads like a D3 chart's.

### Colour and materials

Colours come from palettes, never free RGB, so every scene stays legible and colour-blind safe: categorical (the eight Okabe–Ito-based `Colour::SERIES`, then shapes and dashes beyond eight), sequential and diverging ramps tested under simulated colour blindness, and named tones (`good`, `warn`, `bad`, `muted`). Colour-blind mode (#516) swaps them per participant. Two materials: **solid**, lit from above and opaque, as diagram solids; **glow**, unlit, for highlights. No translucency, for the reasons #530 and #534 give.

### Animation

An item whose id persists between scenes eases to its new position, size and colour over 0.3 s on the client; newcomers fade in and leavers out; nothing extra is sent. Motion over time (a sweep, a pulse) is a behaviour, never code run per frame.

### Behaviours: declared on the server, run on the client

A behaviour is a named, parameterised piece of Tomori's own client code acting on items the scene names:

- `springs { nodes, edges, rest, stiffness, repulsion, damping, home }`: #534's spring layout and fan, the same solver. A `scene.graph` gets it by default.
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
- **Extensions.** `scene.extension` places an environment's face drawer (#306, tier 1 or tier 3) inside a scene where that environment's client crate is present, with the visualisation's own fallback items elsewhere.

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

- **Quotas** in phase 1 are the sources' own (the client's slots, cache and limits) and a count per space of source requests a minute, logged rather than enforced; they become limits with the security boundary (section 10).
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

A participant's flip is **private**, as the trigger's focus is; the agent's is shared until someone flips it back.

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

Each phase is tried in Observe's space on a Quest 3 before the next starts. Estimates are for one agent working with the maintainer's review, tests included.

### Phase 1 (a): the back of a panel, for Observe's charts (about 5 days, and 1 in Observe)

- **`tomori-environment`:** `Back` (rows, links, actions) and `Panel::with_back`.
- **`tomori-protocol`:** `Panel::back`, `Hello::backs`, `ClientMessage::FlipPanel`.
- **`tomori-server`:** keeping and bounding backs (30 rows, as a list's); an automatic back for every panel (identity, origin, age, version); `flip_panel`; `panels` and `inspect` listing backs.
- **`tomori-scene`:** the back's canvas from rows, the ⟲ chip, the flip animation (private by default), the reverse face of a floating panel, the wrist roll while carrying; read-only jacks for the context parameters and time a panel depends on, with their bindings, ahead of channels.
- **Observe** (`environment-observe-server`, `environment-observe`): its agent charts publish a back with the source's name, each series' label and full PromQL, window and step, refresh cadence, when last read and how long it took, series kept of how many, whether partial; actions Refresh, Freeze and Clear.
- **Judged by:** can the maintainer, on the bridge, flip a chart and know what it queries and how fresh it is without asking.

### Phase 1 (b1): the core, the adapter and the scene panel (about 13 days)

- **`base-visualisation`**, new, under `crates/visualisation` (about 5 days): `Visualisation`, `Kind`, `Context`, `Wake`, `Next`, `Event`, `Problem`; the `Source` trait, `Sourced`, provenance and the registry with grants; the scene description, its builder with scales and palettes, and diffing; the scope tree a host supplies; the test harness with fake sources and golden scenes. No `tomori-*` dependency.
- **`base-visualisation-runtime`**, new (about 2 days): applying diffs, easing, picking, level of detail, yielding instances, lines and labels.
- **`bake`** (half a day): the dependency rules above in `bake/tests/dependencies.rs`.
- **`tomori-visualisation`**, the adapter, new (about 3 days): kinds and sources registered from `ServerBuilder::visualisation` and `::source` and from environments; the visualisation thread pool and scheduler honouring `Next`; scenes ↔ panels, diffs as live data; events from intents; instances, saved state and last scenes kept across restarts; the tools' logic (`visualisations`, `show_visualisation`, `update_visualisation`, `visualisation_logs`, `clear_visualisation`); pointing notes from `detail`; backs from `Visualisation::back`.
- **`tomori-protocol`** (about 1 day): `Panel::scene` carrying the core's description with its format number, `Hello::scenes`, `Limits::scenes` and `Limits::items`; older peers sent the summary.
- **`tomori-server`** and **`tomori-environment`** (half a day): wiring the adapter into the hub and stage, the MCP tools, `Environment::with_visualisation` and `with_source`.
- **`tomori-prometheus`** (half a day): `Source` behind the `visualisation` feature, and `promql::guard`, moved from Observe.
- **`tomori-scene`** (about 1 day): `projection::scene`, a scene panel's runtime drawn through the volume's solids, tubes and labels, with trigger focus and grab and release sent as intents. `tomori-render`: no new pipeline expected.

### Phase 1 (b2): channels (about 8 days)

- **`base-visualisation`** (about 2 days): `ChannelKey<T>`, the value types with `Option`, holding, scopes, lifetimes and rates; the channel store's settle in topological order, echo suppression, the 8-tick guard, cycle refusal, expiry; `Wake::Channels`.
- **`base-visualisation-runtime`** (about 1 day): per-participant channels settled locally each frame, expiry, `Show::Mine` and `Show::Everyone`, relay hooks; `publish_on_hover` and `emphasise_when`.
- **`tomori-visualisation`** (about 2 days): **the context model moved onto channels**, with no change to what clients or sides see, the existing tests of `panel-context.md` passing unchanged as the proof; `Setting::Linked`; `channels` and `set_channel`, with `contexts` and `set_context` kept.
- **`tomori-protocol`** and **`tomori-server`** (about 1 day): relaying per-participant values with presence, at their declared rates, to those who can see the scope; `Setting::Linked` on the wire.
- **`tomori-environment`** (half a day): a side declares channels (`Space::declare_channel`, of which `set_context` becomes a form); its client side reads and sets per-participant channels (`ClientContext::channel`, `publish`).
- **`tomori-scene`** (about 1.5 days): **the linked time cursor moved onto `cursor.time`**, the existing tests of `projection.md` passing unchanged; `hovered_series` from a chart's readout; the back's jacks from channels, and cables to the wall's strip and to linked panels.
- **Judged by:** nothing visible changes for existing panels; a scene joins the cursor and the hover with a line of declaration each; and a third host could use the channel store without Tomori (a unit test drives it with a scope tree of its own).

### Phase 1 (c): two compiled visualisations (about 4 and 3 days)

**`observe:faster-pods`**, a module of Observe's server crate (`environment-observe-server`, its pure parts in `environment-observe`) that imports only `base-visualisation` and `tomori-prometheus`'s source, registered through `Environment::with_visualisation`:

- **Sources:** `prometheus:observe`. **Parameters:** `top` (pods, 300 by default).
- **Subscribes:** `time` (shared), inherited from the wall it stands on, so that when the wall is set to the last 6 hours, or a window around an incident, the scatter and every chart beside it change together; `region` (shared, Observe's existing parameter), following the bridge's open region; and `hovered_pod` (participant), to emphasise a pod hovered anywhere.
- **Publishes:** `hovered_pod` (participant), by `publish_on_hover` on its points keyed by pod; and `selection` (shared, `Keys`) of the pods grabbed, so another panel can follow what is held.
- **Update:** on `Wake::Channels` for `time` or `region`, and on its schedule: two `range_all` reads at a step chosen from the window (p95 request queuing by pod and revision, and requests a second by pod and revision), joined on pod and time; `Every(1 min)` while viewed and the window is live, else idle; a fixed window read once.
- **Draw:** about 3,600 points, requests a second across, p95 up on a log scale, time in depth, coloured by deployment revision; axes, a legend of revisions, and a regression line per revision so the claim "faster pods get more traffic" is drawn, not only seen; a plane at `cursor.time` when someone points at a time on a chart on its wall.
- **Events:** pointing tells the agent the pod's figures; the trigger focuses one revision; **grab** a point draws that pod's trajectory through the window, lifts its label and adds it to `selection`; **both** hands on two pods draw both trajectories; release clears.
- **Observe's client side** (about 1 day of the 4) subscribes to `hovered_pod`: hovering a point in the scatter lights **that pod's rod on the region disc** (Observe's "plate" of every instance) at frame rate, on the hoverer's own client only; and it publishes `hovered_pod` when its own rods are pointed at, so pointing at a rod brightens the pod's points in the scatter. Neither knows the other exists; both know only the channel.
- **Fixture:** invented pods for the tests and the spectator's golden render.

**`tomori:crates`**, in `tomori-visualisation-kinds` (depending only on the core and its `cargo metadata` source; registered by `tomori-server`):

- **Sources:** `cargo:workspace`. **Parameters:** `depth`. **Publishes and subscribes:** `crate` (shared, `Keys`), the focused crate, so a second panel, such as a list of that crate's design documents, can follow it, and the agent can set it with `set_channel` while explaining.
- **Update:** `metadata()`; `Every(10 s)`, drawing again only when `Cargo.lock`'s digest or a manifest changed, so the graph follows the code as it is edited.
- **Draw:** one `scene.graph`: crates as nodes, kinds from the crate's role (client, server, environment, protocol, leaf), groups for the workspace's families, edges for dependencies; **#534's visual encoding, springs, fan, pins and two-node paths come with the graph primitive**, not from this visualisation.
- **Events:** **Choose** (A) on a crate publishes it as `crate`, collapsing everything beyond `depth` (its own write does not wake it twice); pointing tells the agent its dependents and dependencies with counts; grab and pull is #534's.
- **Coordination with #534:** this needs #534's phases (a) and (b) (about 6 days, its own estimate), and its phase (c), the shared `tomori-diagram` solver, for pulling to be shared rather than local. This proposal's `scene.graph` is how a visualisation reaches #534's work; it adds no second spring solver or encoding. If #534 has not landed, `tomori:crates` draws today's static volume and gains the springs when #534 does.

### Later phases

| Phase | What | Crates | Effort |
| --- | --- | --- | --- |
| 2 | Faster turnaround (section 9), once phase 1's interface has been used | `base-visualisation` (a run-time `Visualisation`), a new host crate per route | 5 to 15 days by route |
| 3 | The web view: scene pages, SVG, the WebGL2 viewer, the feed, events from signed-in participants | `base-visualisation-svg` and `base-visualisation-web`, new; `tomori-server` (`pages`) | about 8 days |
| 4 | Reports: `create_report`, `update_report`, `export_report`, share links, CSP | `tomori-server`, `tomori-security` | about 6 days |
| 5 | Wiring by hand: dragging cables between jacks, `Setting::Linked` from a participant, cables routed between walls, a wall's "show wiring" chip | `tomori-protocol`, `tomori-server`, `tomori-scene` | about 4 days |
| 6 | Snapshots and export: freezing scenes, `export_data` | `tomori-visualisation`, `base-visualisation` | about 3 days |
| 7 | More behaviours (drag, orbit) and extensions over #306's drawers | `tomori-scene`, #306's crates | about 3 days each |
| 8 | More sources: files, git, the agent's data, environments' own | `tomori-visualisation-kinds`, `tomori-visualisation` | about 2 days each |

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

## 10. Security, deferred but bounded

Phase 1 visualisations are server code, written or reviewed like any other, with the same trust as an environment's server side; the agent can already change the server's code. So phase 1 adds no boundary beyond what exists, with three rules that cost nothing now and keep the later boundary possible:

- **Credentials stay in sources.** A visualisation sees `Sourced<T>` answers through typed handles, filtered by the source's label filter; no source gives out an endpoint or a token. A later sandboxed visualisation, in another process, can then be given handles that proxy over a pipe, holding no secrets.
- **Clients never run visualisation code.** Only Tomori's behaviours run on devices, so no later route changes what a headset or browser trusts.
- **Visitors cause nothing.** Their events never reach a visualisation, they write no channel, and share links are frozen by default.
- **What a person hovers stays theirs unless shown.** A per-participant value leaves its client only when a subscriber at its scope shows everyone's, and then only to people who can see that wall, never logged and never kept; otherwise only in a pointing note when they speak.

The boundary to define with phase 2, once the interface is known: who may create code-at-run-time visualisations (the agent only, acting for owners and collaborators, is the obvious start); a separate, credential-free worker process for run-time code whatever the route; limits on memory, CPU per call and source requests per space; what a persisted visualisation may do unattended; and what live report panels may read for someone outside the space.

## 11. Risks and open questions

### Risks

- **The interface fits two examples and nothing else.** Mitigation: before phase 2, write a third kind of a different shape (a heat map of a fleet, or a flame graph from a file) against it, and change the trait where it strains.
- **Phase 1 tempts template thinking.** Compiled kinds with parameters are templates. They are a means to find the interface, and phase 2 is what returns "any code" to the agent; the plan should not stop at phase 1.
- **Two ways to draw a graph.** Diagrams and visualisations overlap. `scene.graph` makes them one drawing path; the rule is that a diagram is structure the agent describes, a visualisation is data code computes.
- **Blocking reads and slow code.** Compiled updates cannot be preempted. The thread pool keeps them off the hub's lock; a slow one is visible on its back.
- **Channels becoming KVO after all.** Declared, typed, settled channels are easier to reason about than KVO, but a space with dozens of channels and links can still surprise. Mitigations: the wiring is always visible (backs, cables, `channels`); a change's cause is logged with the settle that carried it ("`time` set by Ada on wall Charts; woke 6 panels"); and the first version keeps the context model's small bounds.
- **A reusable core shaped by one host.** Tomori's walls, chips and intents could leak into the core. Mitigations: the core knows only a scope tree, participants and a clock; kinds are checked to import nothing of Tomori; and a unit test drives the core with a host of its own.
- **Moving the context model underneath.** It is a large, well-tested requirement; the migration is judged by its existing tests passing unchanged, and is done before anything new depends on channels.
- **Handler latency.** About 100 ms on a local network; anything tracking the hand must be a behaviour, and the vocabulary will need to grow from use.
- **Web posture.** Serving Tomori's own viewer script changes the pages' threat model; the CSP and server-made SVG keep it small.

### Open questions for the maintainer

1. **Where do kinds live?** Proposed: Tomori's in `tomori-visualisation-kinds`, Observe's in a module of its server crate, both depending only on the core and sources. Should an application be able to register kinds without an environment?
2. **Is `update` / `draw` / `event` the right split,** or should a visualisation build its scene directly in `update` and `event` (simpler, but no purity for tests and restarts)?
3. **Who controls refresh** when a visualisation's own schedule and a participant's #509 policy disagree? Proposed: the policy caps the schedule. Should refresh itself be a channel, so a wall's policy is inherited as `time` is? #509 already models it beside the context with the same inheritance, so it would fit.
4. **Channels' names and reach.** Proposed: Tomori's built-ins unprefixed (`time`, `cursor.time`, `selection`), an environment's its own (`region`, `hovered_pod`), all within one space. Should a channel ever cross spaces, such as one incident window followed by several spaces?
5. **Who turns on everyone's cursors?** Proposed: a subscriber's choice (`Show::Everyone`), set per panel by a chip or the agent. Should it instead be the publisher's consent, so a participant chooses whether their pointer is shown to others?
6. **Implicit by name, or explicit links only?** Proposed: panels follow a channel by name through the scopes, as context does today, with explicit links for the exceptions. Explicit-only would make every cable deliberate, at the cost of wiring every panel.
7. **Private or shared flip?** Proposed: a participant's private, the agent's shared.
8. **Grants:** should Observe's Prometheus source be granted to every space by default, as proposed, or only to spaces an owner names?
9. **Does Observe's `chart` action become a visualisation kind** in phase 1 (a good third example), or stay as it is?
10. **Reports' share links:** frozen-only and 7 days by default? Owners only, or collaborators too?
11. **The web viewer:** hand-written WebGL2 first, as proposed, or straight to `tomori-render` in WebAssembly?
12. **Portals:** is a snapshot standing in front of its plate enough, or is a parallax window wanted enough to revisit #529?
13. **Phase 2's first route:** loaded Rust, as the nearest step from phase 1, or an interpreter, as the nearest to how the agent wrote Python?
14. **The core's name and home:** `base-visualisation`, in Tomori's workspace until it settles and then `base-rs/visualisation`, as proposed? And should #534's spring solver live in the runtime?

## Design documents this would change

A new `design/visualisations.md` (the trait, kinds, context, wakes, events, scheduling, channels and their propagation, the scene builder, tools, and later the run-time routes and their boundary); `design/projection.md` (the scene panel kind, `Hello::scenes`, `Limits::scenes` and `items`, the back of every panel, `FlipPanel`, scene diffs as live data); `design/data-sources.md` (the source registry, grants, `promql::guard`, provenance); `design/diagrams.md` (`scene.graph` as a way to reach a volume, with #534); `design/environments.md` (`Panel::with_back`, `with_visualisation`, `with_source`); `design/panel-context.md` (parameters and time as shared channels, scopes and inheritance, `Setting::Linked`, `channels` and `set_channel`), `design/projection.md`'s linked time cursor (as the participant channel `cursor.time`, and `hovered_series`); `design/mcp.md` (the visualisation tools, `channels` and `set_channel`, `flip_panel`, the report tools, `export_data`); `design/interaction.md` and `design/responders.md` (flipping, grabbing items in scenes); `design/rendering.md` (scenes in the volumes' budget, cables); `design/security.md` (report pages, share links, CSP, Tomori's own script in the web pages, and later the run-time boundary); `design/spectator.md` (drawing scenes); `design/clients.md` (the web as a client of scene descriptions); and `design/crates.md` (`base-visualisation`, `base-visualisation-runtime`, the renderers, `tomori-visualisation` and `tomori-visualisation-kinds`, and their dependency rules).
