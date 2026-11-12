# Proposal: Physics, Stacking Stones and a Little Dam

Status: **approved by the maintainer**, November 2026, with the decisions below, which this text now follows. Its first two stages are implemented: the beach's stones on `tomori-physics` (#367), and the campsite's sticks moved onto the same model, toasted by the server, their trust in the client closed (see `design/physics.md` and `design/farmyard.md`); stages 3 and 4 are not, but for passing a body from one of a participant's hands to the other, done ahead of stage 3's handovers (see `design/physics.md`), though the beach is now walked down to the water's edge and into its shallows, and a stone resting in the river beside it, its dam reach, stays there, as a dam's would, rather than returning (see `design/farmyard.md`); stage 4's rising water would raise the dam reach's level as its stones close it. It was written against `origin/main` at `f2aad5e1` (#334) and amended after the campsite.

It builds on environments' server sides and the limits they keep within (`design/environments.md`, #275), objects that say where they stand (`Object::at`, #307) and anchors, the state channel with its state, events and intents, the projection's revisions, patches and per-client limits (`design/projection.md`, #282, #284, #289), the grip and the responder chain (`design/interaction.md`, `design/responders.md`), presence and travel (`design/spaces.md`, `design/locomotion.md`), the rate limits by role (`design/security.md`, #290), the farmyard's hens, owned by its server side and animated alike on every client (`design/farmyard.md`, #198), and the river stones of the farmyard's trail (#334).

## The request

Spoken by the maintainer from the headset, on the farmyard's trail, November 2026:

> I wonder if there's some point in the future where we have a physics system, and I can build a little rock sculpture, like a little rock tower, a cairn, or even a little dam in the river.

They expect it may be difficult, and it is for later; what is wanted now is a clear, staged plan. After the campsite's marshmallow sticks, they added:

> I wonder if the marshmallow sticks would follow the same physics model… it would be nice if we had a general model for physics, at least to the extent it makes sense.

## The maintainer's decisions

1. **Where:** a shingle beach by the `riverbank` anchor for cairns, and a **dam walling off a pool at the river's edge**, as children build, rather than a new brook.
2. **Hands:** both grips and both triggers. The live microphone moves from the left grip to the left thumbstick's click (`live-on-stick-click`, #355), freeing both, as the left trigger is bound to nothing; symmetric two-handed rays, triggers and grips are a proposal of their own, `two-handed-pointing.md` (#354), which this follows.
3. **Reaching:** within reach of either hand, and either hand's ray pulling a stone to the hand from a few metres.
4. **No magic stacking:** real rigid-body physics from the first stage.
5. **Taking from another's hand** is allowed, a handover passing control from one holder to the other, unless it proves complicated; a swung stone can knock over a cairn.
6. **Throwing** is allowed, up to 8 m/s; stones thrown into the river come back after a while.
7. **Tidying:** owners, and the agent on someone's words; an abandoned cairn never tidies itself.
8. **Cairns persist** when the space comes back to the farmyard after another environment.
9. **A shared `tomori-physics` crate** from the start, since other environments will want it.
10. **glam:** the workspace moves to glam 0.33 for Rapier.

## Recommendation in brief

- **One general model for loose objects.** Stones, the campsite's sticks and whatever an environment later lets be picked up are bodies of `tomori-physics`; an environment declares each object **physical**, simulated once let go, or only **grabbable**, kinematic, resting where its side puts it (section 7).
- **Authority: the holder while held, the server once let go.** A stone in someone's hand follows that hand on every client, drawn from the presence each client already receives every frame, so holding costs nothing on the wire and has no latency for the holder. Let go, it is simulated once, by the farmyard's server side, which streams the poses of moving stones only, and says once where each comes to rest. A stone at rest costs nothing on the wire. Not server-authoritative throughout, which drags the held stone a round trip behind the hand, and not deterministic lockstep, which buys nothing a few dozen stones need and makes every divergence permanent.
- **Engine: Rapier**, `rapier3d` 0.36 (Apache-2.0, pure Rust, glam-based since 0.32), on the server only, with `parry3d`, its collision library, on clients for queries about the held stone; the workspace moves to glam 0.33 for it. Not Avian, which needs Bevy's ECS, nor Jolt, which is C++ behind community bindings; a small custom solver is the fallback if Rapier cannot hold a cairn still.
- **The core gains a few small, general things**, and no physics: the campsite already gave client sides the right grip and others' right hands, and server sides who is present; this adds the left hand, presence for server sides, a side's own record kept with the space across restarts; and a per-space bound on how often a side announces. The simulation itself lives in a new optional crate, `tomori-physics`, shared from the start, that an environment's common crate builds on.
- **Stones are low-poly convex hulls**, 8 to 25 cm across and flattish, drawn and collided from the same points, with high friction, a little damping and firm sleep thresholds; a stone that has rested is snapped to the pose every client was told, so that what is saved, sent and simulated agree to the bit.
- **The dam is faked**: a wall of stones from the bank out into the shallows and back walls off a pool at the river's edge, whose water rises to the level at which what the stones let through, over their crests and between them, matches what the river feeds it, solved once whenever the stones come to rest, and eased in over half a minute. No fluid simulation, and no new brook.
- **Four stages**, each shippable: (1) `tomori-physics` on Rapier and the beach's stones, grabbed with either hand, carried, thrown and stacked for real; (2) the sticks moved onto the general model, closing the campsite's trust in the client; (3) persistence, handovers, knocking cairns over and tidying; (4) the dam and its pool.
- **The biggest risk is a cairn that will not stand still**: stacking is the hardest case for an iterative solver, and a tower that creeps, jitters or bursts apart when it wakes would spoil the whole idea. Stage 1 opens with a test harness that stacks towers and watches them for minutes before anyone builds one on the headset.

## What the system has today

- **An environment's server side** runs in the server process, on a thread of its own, for each binding of a space to its environment (`environments.md`). It shares a state of at most 64 KiB (`STATE_LIMIT`), announces events of at most 4 KiB (`EVENT_LIMIT`) to everyone the channel reaches, takes participants' intents, at most 4 KiB, 8 at once and 4 a second for each participant, visitors refused, and describes at most 32 objects to the agent (`OBJECT_LIMIT`), each of which may say where it stands (`Object::at`). Since the campsite it reads who is present, `Space::present`, as the hub sweeps about once a second, but not where they are. Nothing it keeps survives a restart, and nothing bounds how often it announces: environments.md lists "per-space limits on the rate of a side's updates, and persistence across restarts" as not yet done.
- **The farmyard's hens** are the worked example: the side owns what each hen does, a `Behaviour` with when it began, and every client computes where she is from it with the same function, `Behaviour::place`, so the server never streams a pose. A clock in every message keeps clients within a message's flight of the side. The side moves the flock on every 50 ms or when an intent arrives.
- **Presence** carries each participant's head, controllers and hands, in the space's coordinates, once for every frame their client draws, up to 120 a second from a headset, and the hub relays each report at once to everyone else in the space (`hub.rs`, `ClientMessage::Presence`). The server keeps the latest as `Seen`. An environment's server side is given nobody's; since the campsite, a client side is given the others' right hands, `Input::hands`, to draw what they hold.
- **The right grip reaches an environment** since the campsite (#346): `precedence::Grab` puts the transcript, anything carried and anything of Tomori's under the ray before the environment; a client side marks objects with `Actions::GRAB` or answers `ClientSide::within_reach`, and is told `Input::grab` with the holding hand each frame. Only the right hand grabs, the left grip being the live microphone's and the left trigger bound to nothing. Hand tracking does not grab or select yet.
- **The campsite's sticks** are held as this proposal's first stage was drafted: drawn from the holder's hand, an epoch per stick, rested straight down by the server side. But the server side has no presence, so it does not check where a grab or release is, and **toasting is the holder's client's say**, capped at the fire's rate; `farmyard.md` records that gap until this proposal's server authority lands.
- **The projection** is sent as revisions and patches, each wall's and panel's version the revision of its last change, with compare-and-swap for edits that read before they write (`projection.md`). Interest management, sending each client only what is near it, is left for later.
- **The river stones** of #334 are scenery: up to 70 instances of one 15-triangle model, 0.3 to 0.85 m across, along the banks, in the shallows and about the pool, each with a box `Solid`. They are boulders, not stones to pick up. Since #345 the river lies in a channel 0.4 to 0.7 m below its banks, and no walkable surface reaches its water. The backdrop's budget is 22,000 triangles, of which 21,437 are used.

## 1. What is needed

### Grabbing environment objects

The campsite built the right grip's rung (`precedence::Grab`, `within_reach`, `Input::grab`); this widens it:

- **Both hands.** Since `live-on-stick-click` (#355) moved the live microphone to the left stick's click, both grips and both triggers are free, and `Input::grab` names either hand, each holding one object, with `Input::hands` giving the others' left hands too. Their order on each hand, and the triggers', is `two-handed-pointing.md`'s (#354) to declare, its second stage alongside these; this asks only that each grip end, as the right does now, with the environment.
- **Within reach**, an object's hull within 5 cm of the controller's grip pose, as the campsite's within reach comes after a panel under the ray.
- **Pulled by the ray**: an object the ray meets within 3 m that takes the grip flies to the hand over a quarter of a second, kinematic, and is held where it arrives, so that a stone at the water's edge can be taken from the beach above it.
- **The desktop window** grabs with G where the mouse points, within 3 m, carrying along the ray as cards are carried, the wheel pushing and pulling, so that every stage is testable through the shared frame step.
- **Hand tracking** grabs in stage 3, when a hand closes about a stone, from the curl of its fingers, the stone held from the palm.

### Carrying

- **Held, a stone is rigidly attached to the hand** at the offset it was taken with. The holder's client draws it there every frame from its own tracking, as it draws the hand.
- **Everyone else draws it from the holder's presence**, which already reaches them every frame: the side announces once that participant N holds stone S at offset O in hand H, and each client side places it from N's hand. So a held stone costs one event to take and one to let go, and nothing while carried. Client sides need other participants' hands for this, which is the second core change.
- **The held stone does not pass through the world as the holder sees it.** Their client, with `parry3d`, keeps the drawn stone out of the ground and of stones at rest, sliding it along what it meets while the hand goes through, with a tick of haptics as it touches (see `audio-and-haptics.md`). The stone is let go where it is drawn, so a stone pressed onto a cairn is let go resting on it rather than inside it.
- **On the server, a held stone is a kinematic body** following the holder's hand from `Space::presence()`, so that a stone swung into a cairn knocks it over, for everyone alike (stage 3). It is the server's copy, a round trip behind the hand, so a cairn falls a moment after the holder's stone reaches it; until stage 3, a held stone touches nothing on the server.

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

- `grab { stone, epoch, hand, offset }` takes a stone at rest or moving, if its epoch is the one sent; the side raises it, marks the stone held by that participant and takes it out of the simulation, and announces it. A grab of a stone someone else holds is a **handover** (stage 3): the side raises the epoch, the stone is the new holder's, and the old holder's client lets go of it with a tick, seeing it in the other's hand. A grab whose epoch has moved on is refused. Should handovers prove complicated, they are refused instead, and a stone is let go before it is passed.
- `release { stone, epoch, … }` is taken only from the holder, at the epoch their grab made.
- **Catching** is taking a stone while it moves, which ends the server's simulation of it.
- **The holder leaves.** A participant who leaves the space, or whose presence has said nothing for 3 seconds, lets go of what they held, at their hand's last pose, as `Space::present` tells the side, as it tells the campsite's today.

**Two people grab the same stone.** Each client takes it at once in its own hand, so that the grip feels immediate. The side takes the first grab to arrive and refuses the second; its announcement that the other holds it makes the loser's client let it go, with a tick, the stone moving over to the other's hand. On a local network that is a few milliseconds of disagreement, which nobody will see.

**Sleeping costs nothing.** While any stone moves, the side announces the poses of the moving stones 20 times a second, each a stone's id, position to the millimetre and rotation as four integers, about 50 bytes in JSON, so a typical throw is a few hundred bytes a second and a cairn falling over a few KiB. When a stone sleeps, it is announced once, at rest, at its exact pose, positions to 0.1 mm and rotations to about 0.01°, and **the server snaps its body to that pose**, so that the state, every client and the simulation agree to the bit. Then nothing is sent for it until it wakes. Clients draw moving stones 100 ms behind the farmyard's clock, interpolating between snapshots, and a stone at rest where it was said to rest.

**Persistence.** A cairn stays where it was built, across restarts and when the space comes back to the farmyard after another environment: the side keeps a record of every stone's resting pose with the space, written a few seconds after everything comes to rest and never while something moves, and is given it back as it starts, restoring every stone asleep where it rested. This needs the core's third change, a side's record kept with the space (section 4).

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
| Fits Tomori | glam, through `glamx`; Rapier is on 0.33, which the workspace moves to | brings an ECS Tomori does not use | an FFI surface and a C++ build in a pure-Rust workspace | no dependency |

- **Rapier** is maintained by Dimforge, released monthly (0.36.1 in October 2026), and since 0.32 speaks glam in its public interface, as Tomori does. Its sleeping islands, contact softness and speculative contacts suit stones; its new quarantine of non-finite bodies means a bad release cannot poison the world. The cost is its size, Rapier, `parry3d`, `nalgebra` internally, `simba` and `wide`, to be measured, and **glam**: Rapier needs glam 0.33, so the workspace moves from 0.30 to 0.33, a mechanical change across the renderer and scene, in its own pull request at the start of stage 1.
- **Avian** is good, and simulates much as a custom XPBD solver would, but it is Bevy's physics: it brings Bevy's ECS, app and scheduling into a codebase that has none.
- **Jolt** stacks better than anything else here and is what many shipped games use, but its Rust bindings are community efforts of varying maturity, and it would put a C++ build into the server and, if clients query it, the APK.
- **A custom solver**: extended position-based dynamics (Müller and others, *Detailed Rigid Body Simulation with Extended Position Based Dynamics*, 2020) for a few dozen convex hulls, with `parry3d`'s contact manifolds, is perhaps 1,500 lines. XPBD with substeps stacks well. But tuning friction, restitution, sleeping and waking until a cairn of ten flat stones stands for an hour is the hard part of physics, and Rapier has done it.

**Recommendation: Rapier on the server, `parry3d` alone on clients.** The Quest never simulates: it interpolates poses and asks `parry3d` whether the held stone touches something. A custom XPBD solver is the fallback should stage 1's harness show Rapier's stacks creeping in ways its settings cannot fix; Jolt, the fallback after that.

Rapier's cost on the server is small: a few dozen convex bodies step in well under a millisecond on any server CPU, and the side steps only while something moves, at 60 Hz with four substeps, waiting for intents otherwise, as the hens' side does every 50 ms. Stage 1 measures it.

## 4. The environment's interface

What the core gains, each general and of use beyond stones:

1. **The grip for client sides**, built by the campsite for the right hand, gains the left, and the ray's pull (section 1).
2. **Other participants for client and server sides.** A client side is given both of the others' hands, not only the right. A server side may read the latest presence of each participant, at most as fresh as 20 times a second, with `Space::presence()`, to check reach and to move held bodies; who has left it already reads from `Space::present`.
3. **A side's record, kept with the space.** `Space::keep(value)`, JSON of at most 16 KiB, written with the space's record, debounced to once every 5 seconds, kept for each environment the space has been bound to, and given back as the side starts, `Start::kept()`, for that environment, so that a cairn is there when the space comes back to the farmyard. A record from an earlier version is offered to the side, which keeps what still fits.
4. **A bound on announcing.** At most 30 events a second for each binding, a burst of 60, the rest refused to the side, as the panels' rates are; stones at 20 snapshots a second keep within it with room for grabs and landings. The rate belongs to the space, not to physics.

And a new crate, **`tomori-physics`**, shared from the start, which builds `rapier3d` and `parry3d` but no renderer, so that an environment's common crate builds it and both halves share its shapes. Nothing in the core depends on it. It holds what any environment with loose objects would need, and nothing about stones or sticks:

- **Bodies**: `Body::convex(id, points)`, from the points of a convex hull, or a compound of hulls, such as a stick, with `mass` or `density`, `friction`, `restitution` and `damping`, and an optional `water` damping and buoyancy; each either `Dynamic`, simulated once let go, or `Kinematic`, grabbable only, rested where its side says.
- **Static colliders**: `Static::heights(grid)`, a heightfield, `Static::mesh(triangles)`, from what the scenery draws, `Static::convex(points)` and `Static::cuboid`, in the environment's coordinates; and `Water::below(height, outline)`, a volume that damps and buoys.
- **Settings**: gravity, the step and its substeps, and the sleep thresholds.
- **The world**: steps only while something moves; takes and lets go of a body by epoch; answers snapshots of moving bodies, rests and wakes, each encoded as JSON for the state channel; snaps a body that sleeps to its encoded pose; saves and restores resting poses for `Space::keep`.
- **Holding**: a held body's pose from the holder's hand, from its presence on the server and the client side's hand on clients, by the offset it was taken at; so a side can compute anything that depends on where a held body is, such as a stick's toasting, from the hand itself.
- **The client's half**: the same encoding read back, interpolation 100 ms behind a clock, and the held body's slide along what it touches.
- **Limits**, defined once and enforced by the world as #275's are by the server: `BODY_LIMIT`, 64 bodies, so that a snapshot of all of them fits one 4 KiB event; `HULL_POINTS`, 32 points for each hull; `STATIC_TRIANGLES`, 16,384 for the static colliders; `SNAPSHOT_RATE`, 20 a second; `HELD_LIMIT`, 2 for each participant, a hand each; `RELEASE_SPEED`, 8 m/s, and `RELEASE_SPIN`, 20 rad/s; and `REACH`, 1.5 m from the head. Bodies falling below the lowest static collider by a metre, or beyond 30 m of home, are returned to where they were placed first; so are bodies an environment marks as lost in water, after a while (section 5).

The farmyard's common crate declares its stones, sticks and colliders through it, and its side runs the world beside the flock. Everything about stones, cairns, sticks and dams stays in the farmyard's crates, as everything about hens does.

**The agent.** The side describes each cairn, not each stone, since stones outnumber `OBJECT_LIMIT`: a group of three or more touching stones at rest is "a cairn of 7 stones, 0.9 m tall, on the shingle by the river", standing where its top stone is, with a reference such as `cairn/1`, so that someone can point at one and ask about it, and the agent can go to it. A dam is described as one, with how high it holds the water.

## 5. The stones

- **Where.** A shingle beach beside the trail where it passes the river, by the `riverbank` viewpoint anchor, sloping down into the channel to the water's edge, walkable as polygons at its height (`locomotion.md`), the one surface that reaches the water, with 24 to 40 stones lying on it. The existing boulders join the static colliders as convex hulls, so a cairn can be built on one.
- **Shapes.** Low-poly convex hulls, generated deterministically from a seed like the river's stones are: 10 to 16 points, 16 to 28 triangles, 8 to 25 cm across, flattish, their height a third to two thirds of their width, with a broad, gently domed top and bottom, as stones chosen for a cairn are. Flat faces stack; round ones roll. The same points draw them and collide them, so a stone never visibly floats or sinks. Forty stones are under 1,100 triangles, beyond what the backdrop's budget has left; the budget would rise by about 1,200, as it did for the trail.
- **Physical properties.** Granite's density, 2,650 kg/m³, so a stone of 20 cm is about 2 kg; masses within a ratio of 1 to 10 of each other, which iterative solvers stack far better than wider ratios; friction 0.9 and restitution 0.05; a little damping, 0.1 linear and 0.5 angular.
- **Stability.** A stone sleeps once it has moved less than 5 mm a second and turned less than 0.03 rad a second for half a second, firmer than Rapier's defaults, so that a cairn stops being simulated as soon as it stands. Since a sleeping island wakes only when something awake touches it, a cairn that stands, stands for as long as nobody touches it: it cannot slowly topple or jitter while asleep, and asleep it is snapped to the pose everyone was told. What stage 1 must prove is that a cairn **settles** before it sleeps, rather than creeping, and that it does not burst when woken by one more stone: four substeps, Rapier's contact softness, and capped correction speed, tested in a harness before the headset.
- **Throwing.** Up to 8 m/s and 20 rad/s, as released. A stone that comes to rest in the river's water, beyond the pool a dam walls off, comes back to the beach a minute later, gliding up from the water's edge to where it began, so that the beach is never emptied into the river.
- **Sounds.** A clack when a stone lands, a splash when it enters the water, from the client side, each at the stone, louder for a harder landing, which the side's rest and landing events say.
- **Tidying.** Stones lost below the world or carried off return to where they began. An owner may **Return the stones** from the hand menu's Space section, and the agent may take an action of the beach's, `tidy`, which is consequential, since it undoes someone's cairn, and so is taken only on someone's words (`environment-actions.md`). Either sends every stone not held back to where it began, gliding there over a second rather than vanishing. Nothing tidies a cairn on its own, however long it is left: an abandoned cairn stands until someone asks.

## 6. The dam

The river's bed is 2.4 m across, 12 cm deep, and its water spreads to about 6 m between its banks (`RIVER_HALF_WIDTH`, `SPREAD`); a dam across it would take every stone on the beach. So, as children do, the dam **walls off a pool at the river's edge**: a curve of stones from the beach's bank out into the shallows and back, enclosing a metre or two of water, which the river feeds from upstream through gaps at the curve's head. The beach's shallows are shaped for it, a shelf a few centimetres deep, its bed and bank static colliders.

**The water level is solved, not simulated.**

- The wall is a line of stations along the shelf's edge, each a column 5 cm wide.
- Whenever the stones come to rest, the side finds, for each column, the **crest**: the highest top of any stone at rest over it, by casting down onto them with `parry3d`, or the shelf where there is none.
- Water flows in at the curve's head in proportion to how much the gaps there let through, and out over each column of the rest as over a weir, q = C · w · (h − crest)^1.5 where the level h is above the crest, and seeps through the stones in proportion to the depth, since a wall of loose stones leaks.
- The pool's level is the h at which what flows out equals what flows in, found by bisection, no higher than the bank nor than the river's own surface upstream, and no lower than the river's surface beside it.
- The side shares the level it is heading for, and clients ease the pool's surface towards it, over 10 to 60 seconds according to its area, so that someone who places the last stone sees the water rise behind it.
- The pool is a flat surface over the shelf inside the wall, drawn by the farmyard's water shader without the river's ripples, and hidden where stones and bank stand above it; a trickle sound plays at the lowest crest.
- It costs nothing while nothing moves, and one small solve whenever something comes to rest. Stones in the pool are damped and buoyed by its water volume, whose height is the pool's level.

## 7. The marshmallow sticks, and a general model

The campsite's sticks become bodies of the same world, so that one model covers everything loose in an environment, as far as it makes sense:

- **Held**, a stick is drawn from the holder's hand on every client, as now, and on the server its pose comes from the holder's presence, `Space::presence()`, by the offset it was taken at.
- **Toasting is the server's.** The side computes it from the held pose's distance to the fire, by the same heat as now, a hand's breadth above the embers hottest and nothing beyond about 0.6 m, and shares how toasted each stick is; the holder's client foresees it by the same rule and eases to what the side says. The client's toast intent goes.
- **At rest**, a stick is simulated by the server once let go, a compound of hulls with a marshmallow at its end, so that it falls, rolls and rests on whatever is beneath it, as a stone does; let go by its log, in the fire or beyond 5 m, it goes back on its log, fresh, as now.
- **Grabs and releases are checked**: within reach of the holder's hand as the server last saw it, or along their ray, by the same rules as stones.
- So the campsite's special case, its own grab, rest and toast intents, is replaced, and **its trust in the client is closed**: a client can no longer take a stick from anywhere, toast it away from the fire, or drop it where it was not.

**For any environment.** An environment declares which of its objects are **physical**, `Dynamic`, simulated once let go, such as stones and sticks, and which are only **grabbable**, `Kinematic`, carried in the hand and rested where its side decides, such as something that should hang on a hook or snap back to a shelf. Both are held, handed over, persisted and described alike, and anything future environments let be picked up is one or the other.

## 8. A staged plan

**Stage 1: real physics for stones, and grabbing with either hand.** Implemented (#367).

- Workspace: glam 0.30 to 0.33, in a pull request of its own first.
- `tomori-physics` on Rapier, shared: bodies, colliders, the world, holding, snapshots, rests and wakes, its limits; opening with the **harness**: programmatically stacked cairns of 3 to 10 stones, run for ten simulated minutes, required to settle and sleep within seconds and never move after; one more stone dropped on, required to wake the cairn without bursting it; and releases with the poses and speeds a hand gives.
- Core: the left grip for client sides, free since #355, following `two-handed-pointing.md`; the ray's pull; both of the others' hands; `Space::presence()` for server sides; the bound on announcing.
- Farmyard: the beach, sloping to the water, its stones and surfaces; grab and release intents checked against presence; the holder's prediction and blend; throwing, and stones returned from the river; clacks and splashes.
- Costs: Rapier and its dependencies in the server; server CPU, small and only while something moves; about 1,200 triangles; a farmyard version bump.
- Risks: **stacking stability**, the biggest of the proposal; the grip's order on the left hand; the visible blend at release over a slow link.

**Stage 2: the sticks on the general model.** Implemented.

- Farmyard: the campsite's sticks as bodies of the world, toasting on the server from the held pose, at rest simulated; the campsite's own grab, rest and toast handling removed, and `farmyard.md`'s note of its trust in the client with it.
- Risks: a stick, long and thin, resting less steadily than a stone; toasting that lags the hand by presence's flight, which the holder's foresight hides.

**Stage 3: persistence, handovers, knocking over and tidying.**

- Core: a side's record kept with the space, `Space::keep` and `Start::kept`, across environments.
- Farmyard: cairns kept and restored asleep; handovers from hand to hand; held bodies kinematic on the server, so that a swung stone knocks a cairn over; cairns described to the agent; **Return the stones** and `tidy`; hand-tracked grabbing.
- Costs: the record's format and its bounds; tests with several clients.
- Risks: a restored cairn that wakes and moves, if what was saved is not exactly what slept, which snapping the body to its encoded pose at rest is for; a handover racing a release, which epochs order, or which is refused if that proves complicated.

**Stage 4: the dam and its pool.**

- Farmyard: the beach's shelf in the shallows, its colliders and water volume; the level's solve; the pool's surface and its easing; the trickle.
- Risks: water that looks fake where the pool meets the stones, since nothing flows between them; a level that jumps when one stone settles into a gap, which the easing hides.

## Risks

- **A cairn that will not stand still.** Discussed above: the harness first, in stage 1, firm sleeping, snapping to the encoded pose, a custom XPBD solver and then Jolt as fallbacks.
- **The core's interface grows.** The grip, other people's presence and a kept record each widen what environments can do and must be designed for any environment, not just stones; each is its own decision.
- **Latency over the internet.** On a local network, the blend at release and the refused grab are invisible; across continents, they are not. The holder's prediction covers the first; the second could keep the loser's stone for a few frames before it moves over.
- **The spectator** runs no client side, so it shows no stones, as it shows no hens. A scenery that draws resting stones from the state would fix both, and is out of scope.
- **Dependencies.** Rapier is large and moves quickly; each release's changelog is a review.

## Open questions

None: the maintainer decided all ten (above). Hand tracking's grab, by closing the hand rather than a pinch, is assumed, and `two-handed-pointing.md` may revisit it.

## Design changes this would make

- `design/environments.md`: the left grip and the ray's pull for client sides; both of the others' hands; `Space::presence()`; a side's kept record; the bound on announcing; and `tomori-physics` as an optional crate environments build on, with physical and grabbable objects.
- `design/interaction.md` and `design/responders.md`: the environment's rung on both grips, with `two-handed-pointing.md`; hand-tracked grabbing.
- `design/farmyard.md`: the beach, its stones and cairns, `tidy`, the sticks on the general model with their trust in the client gone, and in stage 4 the dam and its pool; the backdrop's budget.
- A new `design/physics.md` for `tomori-physics`.
- `design/hand-menu.md`: **Return the stones** for owners.
- `design/security.md`: grabbing and releasing as participants' intents, refused to visitors, with the release's bounds.
