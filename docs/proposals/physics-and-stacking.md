# Proposal: Physics, Stacking Stones and a Little Dam

Status: **a proposal for discussion**, November 2026. Nothing here is a requirement, and nothing is implemented. It is written against `origin/main` at `f2aad5e1` (#334).

It builds on environments' server sides and the limits they keep within (`design/environments.md`, #275), objects that say where they stand (`Object::at`, #307) and anchors, the state channel with its state, events and intents, the projection's revisions, patches and per-client limits (`design/projection.md`, #282, #284, #289), the grip and the responder chain (`design/interaction.md`, `design/responders.md`), presence and travel (`design/spaces.md`, `design/locomotion.md`), the rate limits by role (`design/security.md`, #290), the farmyard's hens, owned by its server side and animated alike on every client (`design/farmyard.md`, #198), and the river stones of the farmyard's trail (#334).

## The request

Spoken by the maintainer from the headset, on the farmyard's trail, November 2026:

> I wonder if there's some point in the future where we have a physics system, and I can build a little rock sculpture, like a little rock tower, a cairn, or even a little dam in the river.

They expect it may be difficult, and it is for later; what is wanted now is a clear, staged plan.

## Recommendation in brief

- **Authority: the holder while held, the server once let go.** A stone in someone's hand follows that hand on every client, drawn from the presence each client already receives every frame, so holding costs nothing on the wire and has no latency for the holder. Let go, it is simulated once, by the farmyard's server side, which streams the poses of moving stones only, and says once where each comes to rest. A stone at rest costs nothing on the wire. Not server-authoritative throughout, which drags the held stone a round trip behind the hand, and not deterministic lockstep, which buys nothing a few dozen stones need and makes every divergence permanent.
- **Engine: Rapier**, `rapier3d` 0.36 (Apache-2.0, pure Rust, glam-based since 0.32), on the server only, with `parry3d`, its collision library, on clients for queries about the held stone. Not Avian, which needs Bevy's ECS, nor Jolt, which is C++ behind community bindings; a small custom solver is the fallback if Rapier cannot hold a cairn still.
- **The core gains four small, general things**, and no physics: the grip reaching an environment's client side, as the trigger, A, B and the sticks already do; other participants' hands for client sides, and presence for server sides; a side's own record kept with the space across restarts; and a per-space bound on how often a side announces. The simulation itself lives in a new optional crate, `tomori-physics`, that an environment's common crate builds on, as the farmyard would.
- **Stones are low-poly convex hulls**, 8 to 25 cm across and flattish, drawn and collided from the same points, with high friction, a little damping and firm sleep thresholds; a stone that has rested is snapped to the pose every client was told, so that what is saved, sent and simulated agree to the bit.
- **The dam is faked**: the water behind it rises to the level at which what the stones let through, over their crests and between them, matches a little brook's flow, solved once whenever the stones come to rest, and eased in over half a minute. No fluid simulation. Damming the river, whose water spreads to about 6 m, would take every stone on the beach and more, so the dam needs a brook, which the farmyard does not have yet.
- **Four stages**, each shippable: (1) grab, carry and drop stones, kinematic, a dropped stone settling straight down onto whatever is below it; (2) rigid bodies and stacking; (3) persistence, several people and tidying; (4) the brook and its dam.
- **The biggest risk is a cairn that will not stand still**: stacking is the hardest case for an iterative solver, and a tower that creeps, jitters or bursts apart when it wakes would spoil the whole idea. Stage 2 opens with a test harness that stacks towers and watches them for minutes before anyone builds one on the headset.

## What the system has today

- **An environment's server side** runs in the server process, on a thread of its own, for each binding of a space to its environment (`environments.md`). It shares a state of at most 64 KiB (`STATE_LIMIT`), announces events of at most 4 KiB (`EVENT_LIMIT`) to everyone the channel reaches, takes participants' intents, at most 4 KiB, 8 at once and 4 a second for each participant, visitors refused, and describes at most 32 objects to the agent (`OBJECT_LIMIT`), each of which may say where it stands (`Object::at`). Nothing it keeps survives a restart, and nothing bounds how often it announces: environments.md lists "per-space limits on the rate of a side's updates, and persistence across restarts" as not yet done.
- **The farmyard's hens** are the worked example: the side owns what each hen does, a `Behaviour` with when it began, and every client computes where she is from it with the same function, `Behaviour::place`, so the server never streams a pose. A clock in every message keeps clients within a message's flight of the side. The side moves the flock on every 50 ms or when an intent arrives.
- **Presence** carries each participant's head, controllers and hands, in the space's coordinates, once for every frame their client draws, up to 120 a second from a headset, and the hub relays each report at once to everyone else in the space (`hub.rs`, `ClientMessage::Presence`). The server keeps the latest as `Seen`. Neither an environment's server side nor its client side is given anyone's presence; a client side is given only its own participant's head and ray.
- **The grip never reaches an environment** (`interaction.md`). The right grip detaches the transcript, takes a card, pulls off a panel or takes hold of a wall, each consumer filtering on the others, with no declared order (`responders.md`, which lists `precedence::Grab` as not yet declared and "Grab for environment objects" among the things environments lack). Hand tracking does not grab or select yet.
- **The projection** is sent as revisions and patches, each wall's and panel's version the revision of its last change, with compare-and-swap for edits that read before they write (`projection.md`). Interest management, sending each client only what is near it, is left for later.
- **The river stones** of #334 are scenery: up to 70 instances of one 15-triangle model, 0.3 to 0.85 m across, along the banks, in the shallows and about the pool, each with a box `Solid`. They are boulders, not stones to pick up, and the trail passes the river along its bank, not at the water's edge. The backdrop's budget is 22,000 triangles, of which 21,437 are used.

## 1. What is needed

### Grabbing environment objects

The grip becomes the environment's when nothing of Tomori's wants it, as the trigger, A and B already are, decided as the gesture begins and held until it ends, by the ownership rule `responders.md` describes:

- **`precedence::Grab` is declared** (responders increment 3) with the environment as its last rung: the transcript while it shows, then a card, a panel or a wall the ray is on, then the environment.
- **Touching beats pointing.** A stone within reach of the hand, its hull within 5 cm of the controller's grip pose, is taken before anything the ray points at further away, since the participant is visibly holding it; the transcript, on the wrist, stays first.
- **A client side says what it lets be held**, with a new `ClientSide::grabbable(hand)` that answers the object nearest the hand and where the hand holds it, as `pick` answers the ray. The frame step tells it `Grab::Taken { object, hand, offset }` and `Grab::Released { object, hand }` in its `Input`, with each hand's pose every frame while it holds; the left grip stays the live microphone's, so on the headset only the right hand grabs in stages 1 and 2. Whether the left grip, or both hands, should grab in the farmyard is an open question.
- **The desktop window** grabs with G where the mouse points, within 3 m, carrying along the ray as cards are carried, the wheel pushing and pulling, so that every stage is testable through the shared frame step.
- **Hand tracking** grabs in stage 3, when a hand closes about a stone, from the curl of its fingers, the stone held from the palm. It needs nothing of the trigger's pinch, which is still undecided.

### Carrying

- **Held, a stone is rigidly attached to the hand** at the offset it was taken with. The holder's client draws it there every frame from its own tracking, as it draws the hand.
- **Everyone else draws it from the holder's presence**, which already reaches them every frame: the side announces once that participant N holds stone S at offset O in hand H, and each client side places it from N's hand. So a held stone costs one event to take and one to let go, and nothing while carried. Client sides need other participants' hands for this, which is the second core change.
- **The held stone does not pass through the world as the holder sees it.** Their client, with `parry3d`, keeps the drawn stone out of the ground and of stones at rest, sliding it along what it meets while the hand goes through, with a tick of haptics as it touches (see `audio-and-haptics.md`). The stone is let go where it is drawn, so a stone pressed onto a cairn is let go resting on it rather than inside it.
- **On the server, a held stone touches nothing** in stage 2: it is out of the simulation until it is let go. A held stone that knocks a cairn over is realistic and fun, but would be the server's copy, a round trip behind the hand, knocking things the holder's own copy had not reached; whether it should, once tried, is an open question.

### Letting go, with velocity

- **The client sends `release { stone, epoch, pose, velocity, spin, clock }`**: where the stone was drawn as it was let go, its linear and angular velocity from OpenXR's own velocities of the controller, or the desktop's movement, smoothed over the last 50 to 100 ms, and the farmyard's clock as it saw it.
- **The side validates it**: the stone is held by that participant at that epoch (below); the pose is within 1.5 m of the participant's head as the server last saw it; the speed is at most 8 m/s and the spin at most 20 rad/s, cut to them otherwise. It then puts the stone into the simulation there, moving so.
- **The holder sees no hitch.** Their client carries the stone on under gravity from where it let go for the moment before the server's poses arrive, and blends into them over 150 ms. Everyone else sees the stone go from the holder's hand to the server's poses, which begin where it was let go.

### Rigid bodies

Gravity, contacts between convex hulls and against the ground and banks, friction, little bounce, **stable stacking**, and **sleeping**: a body that has barely moved for half a second stops being simulated, and an island of sleeping bodies costs nothing until something awake touches it. Rapier has all of these; section 3 compares the alternatives.

### Water

A stone in the water is slowed, with more linear and angular damping below the water's surface, and loses the weight of the water it displaces, about 38% for granite; it splashes as it enters, with a sound from the client side. The river's flow does not push stones. The dam is in section 6.

## 2. Authority and sync

### The options

**A. Server-authoritative throughout.** One simulation, on the server; a held stone is a kinematic body following the holder's hand from their presence, and every client, the holder's included, draws the server's poses.

- One truth, no conflicts, and a held stone that knocks the cairn knocks it the same for everyone.
- The holder's stone trails their hand by the presence's flight, the simulation's step, the snapshot's interval and the interpolation buffer: 60 to 150 ms on a local network, more over the internet. At 72 to 120 Hz, a stone that lags the hand is the first thing anyone notices, and placing the top stone of a cairn is precisely where it matters most.
- Fixing that by drawing the held stone at the hand on the holder's own client is option B.

**B. The holder while held, the server once let go.** The holder's client is the authority for a stone it holds, and everyone else draws it from the holder's hand; let go, the server simulates it; at rest, it is the server's and stays put. This is how most multiplayer VR handles held objects, and is close to the scheme Glenn Fiedler describes in *Networked Physics in Virtual Reality* (2018), for Oculus, of stacking cubes: authority that follows interaction, with resting objects falling back to a default authority.

- No latency for the holder, ever.
- Nothing extra on the wire while carrying, since presence already goes to everyone.
- A thrown or dropped stone is simulated once, so nobody disagrees about where it lands. The holder sees a short blend from their own prediction into the server's poses.
- It needs the server to arbitrate who holds what, and a hand-off rule (below).

**B′. The holder until the stone comes to rest.** The holder's client also simulates what it threw or dropped until it sleeps, then hands it back, as many VR games do with authority that spreads through contacts.

- A dropped stone lands exactly as the holder saw it, with no blend.
- Two people's stones meeting, or one landing on a cairn the server owns, spreads authority across clients, which needs Fiedler's full scheme of authority, ownership and their sequence numbers, and a client on a Quest simulating for everyone. For a few dozen stones on a local network, the blend B has is cheaper than that.

**C. Deterministic lockstep.** Every client runs the same simulation from the same inputs, and only inputs are sent.

- Rapier can be bit-for-bit deterministic across platforms (`enhanced-determinism`, now combinable with its parallel solver), and the Quest's aarch64 and a server's x86-64 are both IEEE 754.
- But every step waits for every participant's input, hands are continuous inputs sent at up to 120 Hz, joining needs the whole state anyway, and the farmyard's clock is only good to a message's flight. One divergence, a different float in one hand's pose, is permanent. Each client would also simulate on the Quest's CPU.
- The hens are deterministic in a different, cheaper way: their behaviour is computed from what the server decided, not simulated. Stones in contact cannot be computed that way.

### Recommendation: B

It fits the projection model's habits: the server keeps the truth, clients are sent what changed, and a version guards every change that reads before it writes. It keeps the Quest out of the simulation, which then needs no particular performance on the headset at all. And it uses what presence already sends.

**Ownership and hand-off.** Each stone has an **epoch**, a counter the side raises with each taking and letting go, kept in the state and named in every intent, as a panel's version is named in an edit:

- `grab { stone, epoch, hand, offset }` takes a stone at rest or moving, if its epoch is the one sent; the side raises it, marks the stone held by that participant and takes it out of the simulation, and announces it. A grab of a stone someone else holds is refused, as is one whose epoch has moved on.
- `release { stone, epoch, … }` is taken only from the holder, at the epoch their grab made.
- **Catching** is taking a stone while it moves, which ends the server's simulation of it.
- **The holder leaves.** A participant who leaves the space, or whose presence has said nothing for 3 seconds, lets go of what they held, at their hand's last pose, at rest. The side needs to be told of departures, which it is not today.

**Two people grab the same stone.** Each client takes it at once in its own hand, so that the grip feels immediate. The side takes the first grab to arrive and refuses the second; its announcement that the other holds it makes the loser's client let it go, with a tick, the stone moving over to the other's hand. On a local network that is a few milliseconds of disagreement, which nobody will see. Whether someone may take a stone straight out of another's hand is an open question; recommended: not, so that a stone is let go before it is passed.

**Sleeping costs nothing.** While any stone moves, the side announces the poses of the moving stones 20 times a second, each a stone's id, position to the millimetre and rotation as four integers, about 50 bytes in JSON, so a typical throw is a few hundred bytes a second and a cairn falling over a few KiB. When a stone sleeps, it is announced once, at rest, at its exact pose, positions to 0.1 mm and rotations to about 0.01°, and **the server snaps its body to that pose**, so that the state, every client and the simulation agree to the bit. Then nothing is sent for it until it wakes. Clients draw moving stones 100 ms behind the farmyard's clock, interpolating between snapshots, and a stone at rest where it was said to rest.

**Persistence.** A cairn stays where it was built: the side keeps a record of every stone's resting pose with the space, written a few seconds after everything comes to rest and never while something moves, and is given it back as it starts, restoring every stone asleep where it rested. This needs the core's third change, a side's record kept with the space (section 4).

**The wire, later.** Snapshots ride the state channel's events today, on the one reliable connection, where a lost packet holds up speech and presence alike. The channels proposal (`channels-and-content.md`, #283) would move them to a live stream where only the latest matters, which suits them better; nothing here depends on it.

**Interest management** is unnecessary for a few dozen stones, since only moving ones are sent. If an environment one day has hundreds of bodies, the snapshot is the natural place to begin it.

## 3. The engine

| | Rapier (`rapier3d` 0.36) | Avian | Jolt, through bindings | Custom |
|---|---|---|---|---|
| Language | Rust | Rust | C++ behind Rust bindings | Rust |
| Licence | Apache-2.0 | MIT or Apache-2.0 | MIT (the engine) | Tomori's |
| Standalone | yes | no: built on Bevy's ECS and app | yes | yes |
| Stacking and sleeping | good, with substeps; islands sleep and wake | good (XPBD with substeps) | excellent, the best of these | what we build |
| Determinism | local by default; cross-platform with `enhanced-determinism` | local; cross-platform with a feature | cross-platform with `JPH_CROSS_PLATFORM_DETERMINISTIC` | ours to keep |
| Building for the Quest | ordinary Rust for aarch64 | ordinary Rust, but all of Bevy's crates | the NDK's C++ toolchain in the APK's build | ordinary Rust |
| Fits Tomori | glam, through `glamx`; Tomori is on glam 0.30, Rapier on 0.33 | brings an ECS Tomori does not use | an FFI surface and a C++ build in a pure-Rust workspace | no dependency |

- **Rapier** is maintained by Dimforge, released monthly (0.36.1 in October 2026), and since 0.32 speaks glam in its public interface, as Tomori does. Its sleeping islands, contact softness and speculative contacts suit stones; its new quarantine of non-finite bodies means a bad release cannot poison the world. The cost is its size, Rapier, `parry3d`, `nalgebra` internally, `simba` and `wide`, to be measured, and **glam**: Rapier needs glam 0.33, so the workspace would either move from 0.30 to 0.33, which is a mechanical change across the renderer and scene, or build two glams for a while.
- **Avian** is good, and simulates much as a custom XPBD solver would, but it is Bevy's physics: it brings Bevy's ECS, app and scheduling into a codebase that has none.
- **Jolt** stacks better than anything else here and is what many shipped games use, but its Rust bindings are community efforts of varying maturity, and it would put a C++ build into the server and, if clients query it, the APK.
- **A custom solver**: extended position-based dynamics (Müller and others, *Detailed Rigid Body Simulation with Extended Position Based Dynamics*, 2020) for a few dozen convex hulls, with `parry3d`'s contact manifolds, is perhaps 1,500 lines. XPBD with substeps stacks well. But tuning friction, restitution, sleeping and waking until a cairn of ten flat stones stands for an hour is the hard part of physics, and Rapier has done it.

**Recommendation: Rapier on the server, `parry3d` alone on clients.** The Quest never simulates: it interpolates poses and asks `parry3d` whether the held stone touches something. Stage 1 needs only `parry3d`, for the drop's straight-down cast. A custom XPBD solver is the fallback should stage 2's harness show Rapier's stacks creeping in ways its settings cannot fix; Jolt, the fallback after that.

Rapier's cost on the server is small: a few dozen convex bodies step in well under a millisecond on any server CPU, and the side steps only while something moves, at 60 Hz with four substeps, waiting for intents otherwise, as the hens' side does every 50 ms. Stage 2 measures it.

## 4. The environment's interface

What the core gains, each general and of use beyond stones:

1. **The grip for client sides**: `ClientSide::grabbable`, `Input::grab`, the holding hand's pose each frame, and `precedence::Grab` declared with the environment as its last rung (section 1).
2. **Other participants for client and server sides.** A client side is given, each frame, every other participant's hands and controllers from presence, in the environment's coordinates, by their number. A server side may read the latest presence of each participant, at most as fresh as 20 times a second, with `Space::presence()`, and is told of each participant who leaves, as an intent only the server makes, `Intent::departed`, as `Intent::choice` and `Intent::action` are.
3. **A side's record, kept with the space.** `Space::keep(value)`, JSON of at most 16 KiB, written with the space's record, debounced to once every 5 seconds, kept for each environment the space has been bound to, and given back as the side starts, `Start::kept()`, for that environment and version, so that a cairn is there when the space comes back to the farmyard. Its bound and what happens to a record of an earlier version are open questions.
4. **A bound on announcing.** At most 30 events a second for each binding, a burst of 60, the rest refused to the side, as the panels' rates are; stones at 20 snapshots a second keep within it with room for grabs and landings. The rate belongs to the space, not to physics.

And a new crate, **`tomori-physics`**, which builds `rapier3d` and `parry3d` but no renderer, so that an environment's common crate builds it and both halves share its shapes. Nothing in the core depends on it. It holds what any environment with loose objects would need, and nothing about stones:

- **Bodies**: `Body::convex(id, points)`, from the points of a convex hull, with `mass` or `density`, `friction`, `restitution` and `damping`, and an optional `water` damping and buoyancy.
- **Static colliders**: `Static::heights(grid)`, a heightfield, `Static::mesh(triangles)`, from what the scenery draws, `Static::convex(points)` and `Static::cuboid`, in the environment's coordinates; and `Water::below(height, outline)`, a volume that damps and buoys.
- **Settings**: gravity, the step and its substeps, and the sleep thresholds.
- **The world**: steps only while something moves; takes and lets go of a body by epoch; answers snapshots of moving bodies, rests and wakes, each encoded as JSON for the state channel; snaps a body that sleeps to its encoded pose; saves and restores resting poses for `Space::keep`.
- **The client's half**: the same encoding read back, interpolation 100 ms behind a clock, the held body's slide along what it touches, and the drop's cast (stage 1).
- **Limits**, defined once and enforced by the world as #275's are by the server: `BODY_LIMIT`, 64 bodies, so that a snapshot of all of them fits one 4 KiB event; `HULL_POINTS`, 32 points for each hull; `STATIC_TRIANGLES`, 16,384 for the static colliders; `SNAPSHOT_RATE`, 20 a second; `HELD_LIMIT`, 2 for each participant, a hand each; `RELEASE_SPEED`, 8 m/s, and `RELEASE_SPIN`, 20 rad/s; and `REACH`, 1.5 m from the head. Bodies falling below the lowest static collider by a metre, or beyond 30 m of home, are returned to where they were placed first.

The farmyard's common crate declares its stones and colliders through it, and its side runs the world beside the flock. Everything about stones, cairns, brooks and dams stays in the farmyard's crates, as everything about hens does.

**The agent.** The side describes each cairn, not each stone, since stones outnumber `OBJECT_LIMIT`: a group of three or more touching stones at rest is "a cairn of 7 stones, 0.9 m tall, on the shingle by the river", standing where its top stone is, with a reference such as `cairn/1`, so that someone can point at one and ask about it, and the agent can go to it. A dam is described as one, with how high it holds the water.

## 5. The stones

- **Where.** A shingle beach beside the trail where it passes the river, by the `riverbank` viewpoint anchor, walkable to the water's edge as a polygon at its height (`locomotion.md`), with 24 to 40 stones lying on it. The existing boulders join the static colliders as convex hulls, so a cairn can be built on one.
- **Shapes.** Low-poly convex hulls, generated deterministically from a seed like the river's stones are: 10 to 16 points, 16 to 28 triangles, 8 to 25 cm across, flattish, their height a third to two thirds of their width, with a broad, gently domed top and bottom, as stones chosen for a cairn are. Flat faces stack; round ones roll. The same points draw them and collide them, so a stone never visibly floats or sinks. Forty stones are under 1,100 triangles, beyond what the backdrop's budget has left; the budget would rise by about 1,200, as it did for the trail.
- **Physical properties.** Granite's density, 2,650 kg/m³, so a stone of 20 cm is about 2 kg; masses within a ratio of 1 to 10 of each other, which iterative solvers stack far better than wider ratios; friction 0.9 and restitution 0.05; a little damping, 0.1 linear and 0.5 angular.
- **Stability.** A stone sleeps once it has moved less than 5 mm a second and turned less than 0.03 rad a second for half a second, firmer than Rapier's defaults, so that a cairn stops being simulated as soon as it stands. Since a sleeping island wakes only when something awake touches it, a cairn that stands, stands for as long as nobody touches it: it cannot slowly topple or jitter while asleep, and asleep it is snapped to the pose everyone was told. What stage 2 must prove is that a cairn **settles** before it sleeps, rather than creeping, and that it does not burst when woken by one more stone: four substeps, Rapier's contact softness, and capped correction speed, tested in a harness before the headset.
- **Sounds.** A clack when a stone lands, a splash when it enters the water, from the client side, each at the stone, louder for a harder landing, which the side's rest and landing events say.
- **Tidying.** Stones lost below the world or carried off return to where they began. An owner may **Return the stones** from the hand menu's Space section, and the agent may take an action of the beach's, `tidy`, which is consequential, since it undoes someone's cairn, and so is taken only on someone's words (`environment-actions.md`). Either sends every stone not held back to where it began, gliding there over a second rather than vanishing.

## 6. The dam

The river's bed is 2.4 m across, 12 cm deep, and its water spreads to about 6 m between its banks (`RIVER_HALF_WIDTH`, `SPREAD`); a dam across it would take more stones than the beach has and leave none for a cairn. Stage 4 adds a **brook**: a little stream 0.5 to 0.8 m wide and 10 to 15 cm deep, running out of the bush across the meadow into the river beside the beach, its bed and banks static colliders, its water drawn by the farmyard's water shader.

**The water level is solved, not simulated.**

- The brook is a line of stations along its course, each a cross-section 16 columns wide.
- Whenever the stones come to rest, the side finds, for each column, the **crest**: the highest top of any stone at rest over it, by casting down onto them with `parry3d`, or the bed where there is none.
- Through a station, water flows over each column as over a weir, q = C · w · (h − crest)^1.5 where the level h is above the crest, and seeps through the stones in proportion to the depth, since a dam of loose stones leaks.
- The brook's flow is fixed. The level behind the most restrictive station is the h at which what passes it equals that flow, found by bisection, no higher than the lower bank, over which the water then spills round. Downstream, the brook runs lower in proportion to what passes.
- The side shares the level it is heading for, and clients ease the pond's surface towards it, filling over 10 to 60 seconds according to the pond's area, so that someone who places the last stone sees the water rise behind it.
- The pond is a flat surface over the brook upstream of the dam, as far as its bed is below the level, drawn by the same water shader and hidden where the banks stand above it; a trickle sound plays at the lowest crest.
- It costs nothing while nothing moves, and one small solve whenever something comes to rest. Stones in the brook are damped and buoyed by its water volume, whose height is the pond's level.

## 7. A staged plan

**Stage 1: grab, carry and drop, kinematic.**

- Core: the grip for client sides, with `precedence::Grab` declared and its equivalence test; other participants' hands for client sides; departures for server sides.
- Farmyard: the beach, its stones and its walkable surface; intents to grab and drop; held stones drawn from presence.
- A dropped stone keeps its turn about the vertical, is laid on its broadest face, and settles **straight down** onto whatever is below it, the ground, a boulder or another stone, by a cast of its hull, gliding there in a quarter of a second. Stones stack, but never fall: a tower stands however it is built. That is magic rather than physics, and says so in the farmyard's design; it may be all some people want.
- Costs: the core's grip and presence changes, the larger part; `parry3d` on both sides; a farmyard version bump; about 1,200 triangles.
- Risks: the grip's precedence among cards, panels and walls; the feel of a stone in a controller's hand; the desktop's carrying being only an approximation of a hand.

**Stage 2: rigid bodies and stacking.**

- `tomori-physics` on Rapier; the server simulates what is let go; the holder's prediction and blend; snapshots, rests and wakes on the wire; the bound on announcing; throwing; clacks and splashes.
- It opens with the harness: programmatically stacked cairns of 3 to 10 stones, run for ten simulated minutes, required to settle and sleep within seconds and never move after; one more stone dropped on, required to wake the cairn without bursting it; and releases with the poses and speeds a hand gives.
- Costs: Rapier, `parry3d` and their dependencies in the server; glam 0.30 to 0.33 across the workspace, or two glams; server CPU, small and only while something moves.
- Risks: **stacking stability**, the biggest risk of the whole proposal; the visible blend at release over a slow link; snapshots held up behind other traffic on the one connection.

**Stage 3: persistence, several people and tidying.**

- Core: a side's record kept with the space, `Space::keep` and `Start::kept`; `Space::presence()` for validating reach.
- Farmyard: epochs, refused grabs and the loser's letting go; letting go on departure; cairns described to the agent; **Return the stones** and `tidy`; hand-tracked grabbing.
- Costs: the record's format and its bounds; tests with several clients.
- Risks: a restored cairn that wakes and moves, if what was saved is not exactly what slept, which snapping the body to its encoded pose at rest is for; a record from an earlier version of the farmyard that no longer fits its stones.

**Stage 4: the brook and its dam.**

- Farmyard: the brook in the backdrop, its colliders and water volume; the level's solve; the pond's surface and its easing; the trickle.
- Costs: the brook's place in a backdrop that keeps views open and plants clear; its triangles; another version bump.
- Risks: water that looks fake where the pond meets the stones, since nothing flows between them; a level that jumps when one stone settles into a gap, which the easing hides.

## Risks

- **A cairn that will not stand still.** Discussed above: the harness first, firm sleeping, snapping to the encoded pose, a custom XPBD solver and then Jolt as fallbacks.
- **The core's interface grows.** The grip, other people's presence and a kept record each widen what environments can do and must be designed for any environment, not just stones; each is its own decision.
- **Latency over the internet.** On a local network, the blend at release and the refused grab are invisible; across continents, they are not. The holder's prediction covers the first; the second could keep the loser's stone for a few frames before it moves over.
- **The spectator** runs no client side, so it shows no stones, as it shows no hens. A scenery that draws resting stones from the state would fix both, and is out of scope.
- **Dependencies.** Rapier is large and moves quickly; each release's changelog is a review.

## Open questions

1. **Where?** The shingle beach by the `riverbank` anchor and a new brook for the dam, or somewhere else, such as stones in the yard? Or, instead of a brook, a little dam walling off a pool at the river's edge, as children build one?
2. **Which hands?** The right grip only, since the left is the live microphone, or both in the farmyard? And should hand tracking grab by closing the hand, or by a pinch?
3. **Reaching.** Only stones within the hand's reach, or the ray too, pulling a stone to the hand from a few metres away?
4. **Stage 1's magic stacking.** Ship it on its own, a tower that never falls, or treat it as a step towards stage 2 only?
5. **Taking from another's hand**, and should a held stone knock over a cairn it is swung into?
6. **Throwing.** Allowed, at up to 8 m/s, or only dropping and placing? Stones thrown into the river: lost until tidied, or returned?
7. **Who tidies?** Owners and the agent on someone's words, as proposed, or any participant? And should an abandoned cairn ever tidy itself?
8. **Persistence across environments.** Should a cairn come back when the space returns to the farmyard after another environment, as proposed?
9. **Where physics lives.** A shared `tomori-physics` crate from the start, or the farmyard's own until a second environment wants it?
10. **glam.** Move the workspace to glam 0.33 for Rapier, or carry two versions for a while?

## Design changes this would make

- `design/environments.md`: the grip for client sides; other participants' presence for client and server sides; `Intent::departed`; a side's kept record; the bound on announcing; and `tomori-physics` as an optional crate environments build on.
- `design/interaction.md` and `design/responders.md`: the right grip's order declared, with the environment last and touching before pointing; hand-tracked grabbing.
- `design/farmyard.md`: the beach, its stones and cairns, `tidy`, and in stage 4 the brook and its dam; the backdrop's budget.
- `design/hand-menu.md`: **Return the stones** for owners.
- `design/security.md`: grabbing and releasing as participants' intents, refused to visitors, with the release's bounds.
