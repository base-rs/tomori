# Proposal: The Agent Follows the Speaker

Status: **a proposal for discussion**, November 2026. Nothing here is a requirement. The design documents it would change (listed at the end) take precedence until it is agreed.

It builds on the homes and anchors of `design/environments.md` (#231, #271), the summons and the entourage that travels with it (#231, #261), the workers' arc (#264, #270, #271), environment walls and their locks (#262), the float-beside placement of `design/projection.md` (#269), shared attention and `presence` (`design/interaction.md`, `design/mcp.md`, #223), and held speech and the speaking pill (`design/speech.md`, #244).

## The request

Spoken by the maintainer from the headset, November 2026, after experimenting with a data-centre visualisation environment on their work laptop, a scene dense with racks and information in which it was hard to fit everything sensibly:

- **Remove the Tomori wall as the agent's fixture.** Environments designate a status wall, or area, for what the Tomori wall holds today: status notices, workers finishing, pairing and so on. Presenting content stays as it is: panels go on the looked-at wall or float beside the asker (#269), and the agent can still place content.
- **The agent follows the speaker.** When someone asks something while looking or pointing at something, the agent comes over to answer, as a person would. Its voice then comes from near the listener, which helps spatial audio. It should orient sensibly relative to what they point or look at.
- **Replace the trigger-to-summon** (#231's 0.5 s hold) as the main mechanism with this automatic behaviour, and decide whether the manual summons stays as an override.
- **Return home after the conversation ends:** after a minute or two with no follow-up, while the agent is not waiting for input, it glides back to its home, wherever the environment puts it.
- **Keep a home:** the ring beneath the swarm can be its visualised home.
- **Workers need a place.** The arc around the user "does work quite well", but in the data centre it smashes into everything. Explore the options.
- **MCP control:** the agent may steer some of this (stay here, go home, come to someone, don't follow for now), within sensible rules.
- **The aim:** a more organic, collaborative relationship, with the agent as a dynamic part of the environment.

## Recommendation in brief

- **The server moves the agent to whoever it is talking with.** As a participant's words begin, the server, which already captures what they point at and look at and knows every head, wall, floating panel and the environment's solids, chooses a **conversation spot**: about **1.5 m** from their eyes (1.2 to 1.8 m), **off to the left** of their line of sight to what they attend to, so that it never blocks it, at about their eye height, facing them. The agent turns towards them at once and glides there if it is not already near enough. It uses the same `SpaceState::summoned` every client already glides the agent to.
- **Hysteresis and turn-taking.** It moves only when the speaker is beyond conversational range or it blocks their view; it serves a group from one spot when several nearby people are talking; and when the agent is busy with one person's request, a second person's words move it only when their turn is answered, in queue order. Visitors never move it.
- **It goes home after 90 seconds** (tunable, 30 s to 10 min, or never) of an idle conversation: nobody recording, nothing queued, no question or approval waiting, no reply playing or held, and the agent's session idle. Any new words cancel the return.
- **The trigger's summons stays, as a pin:** a 0.5 s hold now means *come here and stay*: following and the return timer are suspended until someone sends it home, summons it again or asks it to follow.
- **The Tomori wall becomes the space's status wall.** It keeps its identity, panels and edits, but stops being the agent's fixture: it never travels with the agent, and environments say where it stands and how large it is with a new `Environment::with_status_wall`, which generalises `with_stage`. Without one it stands where it stands today, beneath the agent's home. Answers keep going where the asker looks or beside them; Tomori's own notices go to the status wall.
- **Workers stay at home.** They no longer travel with the agent. They stand about the home on today's arc, fitted to an environment-declared **worker zone** where one is given, with a **compact** arrangement as fallback. While the agent is away, a few small **satellites** orbiting it show how many workers are running and whether any has finished.
- **`move_agent` gains `stay`, `follow` and `object ID`,** `presence` says the agent's mode, and the server limits how often the agent may move itself.
- **Staged:** (1) following and the return home, behind a space setting; (2) the status wall; (3) workers at home, zones and compaction; (4) the MCP controls; (5) tuning in dense environments, then following by default.

## What the system does today

| Part | Today | Where |
| --- | --- | --- |
| The agent's home | The environment's `agent` anchor, or 2.3 m ahead of the arrival, 1.4 m up. It never moves of its own accord | `environments.md`, homes |
| Summoning | Trigger held 0.5 s on nothing: the agent stands at least 2.3 m from the summoner's head, 0.3 m above their eyes, never below 1.4 m, kept as `SpaceState::summoned` for the run and environment, gliding at up to 0.85 m/s | `environments.md`, `entourage::summoned` |
| The entourage | The Tomori wall folds, travels and unfolds beneath the summoned agent, after a search for a clear spot (`stage/travel.rs`). Workers glide to an arc through the agent about the summoner (`SpaceState::summoner`) | #261, #264, #270, #271 |
| Why 2.3 m | The workers' arc passes through the agent, and its columns and metre-wide signs need that radius. The wall beneath needs the 1.4 m height | `entourage::SUMMONED`, `LOWEST_AGENT` |
| Sending it home | `move_agent home`, the hand menu's **Send Tomori home**, another summons, a change of environment, a restart. It never drifts home | `mcp.md`, `hand-menu.md` |
| The Tomori wall | One per space, its panels and edits (grid, shape, tilt, lock) kept with the space in every environment. By default beneath the agent's home, or at the environment's `stage`. It takes Tomori's notices (`Stage::project_notice` to `Destination::Tomori`), panels projected without anyone asking, and is every placement's last fallback | `projection.md` |
| Answers | Onto the wall the asker looked at as their words began, within 4 m, or floating beside them, 1.2 m away, 30° right, then 30° left, and so on (#269) | `projection.md` |
| Attention | Captured as the words begin: what the ray points at, sent by the client. Where each head is and what it looks at comes from presence, which the server resolves against its own walls and panels | `interaction.md`, `mcp.md` |
| Solids | An environment gives boxes of its scenery, `Environment::with_solids`, which the wall's travel spot and the beside placement avoid | `environments.md`, `projection.md` |
| Agent voice | From the avatar's voice points, wherever it stands. Unity at 2 m, at most 6 dB quieter far away, never louder within 0.5 m | `spatial-audio.md` |
| The swarm's size | Up to 1.35 m across, 0.52 m tall and 0.42 m deep (`entourage::SWARM`) | `avatars.md` |

So the parts this needs exist: the server already owns the agent's pose for everyone, captures attention when words begin, knows heads, walls, panels and solids, and searches for clear spots for walls and panels. What changes is **who decides to move the agent** (the conversation, not only a trigger), **where it stands** (closer, beside, without an entourage), and **what no longer moves with it** (the wall and the workers).

## 1. The behaviour model

### Who is being talked with

The agent has one **conversation partner** at a time: the participant whose words it is answering now, or else whose words it last answered. In a group, the partner can be a **huddle** (below). The partner is the server's to decide, from what it already has: the queue, the delivered input's speaker, and presence.

- **When the agent is free**, with nothing queued and nobody's words being answered, the participant who presses X becomes the partner as their words begin.
- **When the agent is busy** with someone else's words, a new speaker's press **turns the agent's halo towards them for a moment**, an acknowledgement, without moving it. They become the partner, and the agent moves, when their words are **delivered**, which is when the agent begins answering them, in queue order. "The one addressed" is therefore whoever the agent is answering. Words in Tomori are always addressed to the agent, so there is no need to guess from names.
- **Visitors never become the partner.** They cannot speak to the agent, and their trigger draws the swarm only while held, as today.
- **Words that are not a question**, such as renaming the space (titling) or a recording cancelled or with no speech heard, never move the agent. A move already under way stops where it is, and the return timer starts from there.

### When it moves

1. **On press, it turns.** As X goes down, the agent turns to face the speaker at once (a yaw, not a move), as a person looks up when spoken to. This costs nothing in comfort and tells the speaker they are heard.
2. **After 0.4 s of holding, it commits.** A tap or a press let go at once, which push-to-talk often is when it is a mistake, moves nothing. After 0.4 s the server chooses the conversation spot from the attention and presence captured **as the words began**, the same moment `projection.md` places answers from, and the agent starts gliding. Moving while the person is still talking, rather than after transcription, matters: a glide of a few metres takes several seconds, and that time overlaps their speaking and the agent's thinking, so it usually arrives before the reply plays.
3. **It only moves when it needs to.** It stays where it is, turning only, when it already stands within **3 m** of the speaker's eyes, within **50°** of the way their head faces, seen clearly from their eyes, and not in their way (below). So a follow-up question from the same place moves nothing, and someone turning slightly to point at another rack does not drag it about.
4. **It doesn't jitter between speakers.** Once it arrives somewhere it **stays for at least 10 s** before moving for another partner, unless that partner is more than 6 m away or cannot see it. When the new partner is within 3 m of the old one, it doesn't leave one for the other; it serves both, as a huddle.
5. **Huddles.** When two or more participants have spoken to it within the last 60 s and stand within 4 m of each other, the conversation spot serves the group: about 1.5 m from the nearest of them, on the side of the group facing most of them, facing the middle of their heads, clear of every line of sight to what any of them attends to. A group around a rack gets one agent standing at the edge of the circle, not one that hops between them.

### Where it stands

The server chooses the spot when the agent commits, from the speaker's **eyes** E and their **focus** F: what they pointed at as the words began, else the wall, panel or person they looked at, as presence resolves it, else 2 m along the way their head faces. A worker or an environment object pointed at counts too, where the server knows where it stands (for an object, that needs the side to give a position; see section 4).

- **Distance:** 1.5 m from E horizontally, a comfortable conversational distance between a person's 1.2 m social bubble and the 1.8 m at which a group forms. Candidates are tried at 1.5, 1.8, then 1.2 m.
- **Bearing:** 45° to the speaker's **left** of the line from E to F. That is out of the line of sight, so it never blocks what they look at, and away from where `beside` puts the first floating panel, 30° to their right, so that a reply's panel and the agent don't stand in each other's way. The candidates are then 45° right, 70° left, 70° right, 25° left and 25° right. A candidate is refused when the swarm's reach (`entourage::SWARM`) falls within **12°** of the line from E to F, as seen from E.
- **Height:** the swarm's middle at the speaker's eye height plus 0.1 m, as a face across from them, so it reads alike for someone sitting or standing. It is never below 1.1 m nor above 2.4 m above the floor beneath it. Without a wall beneath the agent, today's 1.4 m least height (`LOWEST_AGENT`) no longer applies.
- **Facing:** towards E. While it speaks about something pointed at, the halo may glance towards F and back, a later refinement that every avatar could take as a hint.
- **Clear of everything:** the box of the swarm's reach, 0.1 m larger on every side, must be clear of the environment's solids, of every wall shown (its slots 5 cm thicker, as the beside placement takes them), of every floating panel within 3 m, and at least **0.8 m** from every participant's head, so that it never ends in anyone's face. The line from E to its middle must also pass through no solid or wall. This is the test `stage/travel.rs` already makes for the wall's travel spot, and the beside placement for panels, shared rather than written a third time.
- **Only where the environment lets it stand:** if the environment declares **open space** (section 4), the spot must be inside it. This is how a dense environment stops the agent standing inside a rack row whose solids it never gave.
- **Fallback:** with every candidate refused, it tries the same bearings at 2.3 m, today's summons distance. With none clear, it stays where it is and only turns. It never stands somewhere it would be hidden.

The agent's spot then counts as **taken** for the beside placement, as a floating panel does, so that panels in the reply that follows fan out around it rather than through it. The agent is placed first, as words begin, and panels after, as the reply completes, so the order is natural.

**The swarm's size up close.** The swarm reaches up to 1.35 m across, about 48° of view from 1.5 m, so up close it would fill the speaker's view. **Recommendation:** an avatar is told how far it stands from whoever it faces, and the swarm draws in to about 0.7 m across within 2 m of them, as a flock gathering to listen, and spreads out again at home. The clear-space test uses the size it will have there.

### How it moves

- **A glide with ease.** It accelerates over 0.4 s, cruises at up to **0.85 m/s**, today's summons speed, and settles over 0.5 s. The listener is never moved, so this is an object moving in the scene, not artificial locomotion, and `locomotion.md`'s rules against smooth motion of the rig do not apply. Still, a large object sweeping past close by is unsettling, so the path keeps 0.8 m from heads.
- **Long distances stream.** Beyond **5 m**, rather than a glide of many seconds, the swarm's agents **stream** there as a flock, at the particles' bounded 6 m/s, the way workers already fly out and back, and regather at the spot within about 2 s. The agent's voice and pose follow the flock's middle. Long distances are common in a large data centre, and this reads as organic rather than as a slow chase.
- **Never through people or walls.** The server checks the straight path against heads (0.8 m), solids and walls shown. Where it is blocked, it tries one waypoint, raised to pass 0.5 m above the tallest head along the way, or offset 1 m to either side, and sends it as an optional `via` with the spot, which clients glide through. Where none is clear, the agent streams (above), which needs no path.
- **Interrupted moves turn round smoothly**, as a summons sent back on the way does today.

### When it goes home

The conversation is **open** from the partner's words until it is idle. The agent returns home once **all** of these have held for the **return delay**, by default **90 s**, a space setting from 30 s to 10 minutes, or never:

- nobody in the space is recording words for it, and no words are on their way or queued;
- no approval or question of the agent's is waiting;
- the agent's session is idle;
- no agent speech is playing for anyone, as the server knows from what it sends, and none is held for anyone (`speech.md`'s held speech and its 600 ms release pause);
- it is not pinned (below).

Any participant's words cancel the return, as does the agent's own speech starting, for instance with a worker's report. A worker still running is **not** a reason to stay: workers take minutes, and their reports are for the space, not for whoever happened to ask last (see Workers). When the partner **leaves** the space, or their engagement goes **away** (`spaces.md`), the delay shortens to 15 s.

Going home is the same glide or stream, to the environment's agent home, facing as the home faces. It is silent: no cue and no notice. The **ring on the floor** beneath the home always shows, faint while the agent is away, as the visible "this is where it lives" the maintainer asked for, and brightens as it arrives.

### The trigger's summons, kept as a pin

**Recommendation: keep it, as an override, with a new meaning.** A pull held 0.5 s and let go now means **come here and stay**: the agent goes where it was drawn (at the conversation distance, not the 2.3 m it needed for the arc, since the entourage no longer comes), and is **pinned**. While pinned, it doesn't follow the speaker and doesn't return home on its own. It still turns to whoever speaks. A pin ends with **Send Tomori home**, `move_agent home` or `follow`, another summons, a change of environment, a restart, or after **30 minutes** with nobody in the space.

The case it serves is "stand there by that rack while we work through this", which following would undo every time someone stepped away to look at something. It costs nothing to keep, people already know it, and the trigger's order of rungs (`interaction.md`) is unchanged. A shorter tug still only draws the swarm while held.

### A space setting

**Agent movement**, an owner's space setting, kept with the space as other settings are:

- **Follow the speaker**: everything above;
- **Summon only**: today's behaviour, the agent moving only when summoned or asked (without the entourage, from stage 2);
- **Stay home**: never moves, summons included, for a presentation or a cramped environment.

In stage 1 the default is **Summon only**, so nothing changes until an owner turns following on. Stage 5 decides whether **Follow the speaker** becomes the default.

## 2. The status wall

### What it is

The Tomori wall stops being the agent's fixture and becomes the space's **status wall**: the place for what the space and Tomori say about themselves rather than in answer to someone. It is the **same wall**, the same identity (`tomori_protocol::TOMORI_WALL`), the same kept panels and edits, so nothing a space keeps is lost or moved. What changes is where it stands, its name and how big it may be.

- **It never travels.** It stays at its home whatever the agent does: no fold, travel or unfold, and `stage/travel.rs` retires. That was what made the dense environment hard: a wall that had to find room beneath the agent wherever it went.
- **Its home is the environment's.** `Environment::with_status_wall(EnvironmentWall)`, in the environment's own coordinates and carried as every anchor is, gives where it stands **and** its grid, slot size and shape, which `with_stage` could not. A dense environment can give a compact strip by the door, a sparse one a board in the yard. It may say the wall stands **on its scenery**, as a notice board does, and is then locked by default, as an environment's own walls are, while today's default board stays unlocked.
- **`with_stage` stays** as the same thing with today's grid, so that existing registrations build unchanged.
- **Without either, it stands where it stands today,** beneath the agent's home, as `entourage::wall_beneath` puts it. That is the backward-compatible fallback for Sky, the grid, the GitHub prototype and the farmyard until they choose otherwise. It stays tied to the home, not the agent, so it no longer follows the agent anywhere.
- **Bounds:** within 20 m of home, upright and finite, as every wall is. Its grid may be up to the general wall bounds, but **at least two slots**, so that two notices can stand together. Each environment's sightline tests check that it reads from the arrival (4 m, or further in proportion for a big wall) and stands clear of solids, as the Tomori wall's home is checked today.

**Why not let the environment name one of its own walls as the status wall?** Its own walls belong to the environment and their panels are kept per environment, while the status wall's panels belong to the space and stay as the space changes environment. Pointing at an environment wall would mean moving the space's status panels in and out of that environment's storage on every change. Giving the space's one status wall a home and a size in each environment keeps the storage as it is. An environment that wants it on its console draws the console where it puts the status wall.

### What goes where

| Content | Where | Change |
| --- | --- | --- |
| An answer to someone's words | The wall they looked at, within reach, else floating beside them (#269) | None. The agent's spot is now taken for the beside fan |
| `place:` forms | As today | `wall status wall` added; `wall Tomori wall` kept as an alias |
| Tomori's own notices: workers finishing or removed, pairing decided, walls changed, panels moved, layouts | The status wall (`project_notice`) | None in code; it is the same destination |
| Panels the agent projects without anyone asking, as after a restart | The status wall | None |
| A worker's report spoken for the space | Spoken from wherever the agent is; its notice on the status wall | The agent doesn't move for it (open question) |
| A pairing waiting for an owner | The status wall, and the owners' pill and hand menu, as today | None |
| Every placement's last fallback | The status wall | None |

### Migrating from the Tomori wall

- **Kept edits.** The space's kept grid, shape, tilt and lock of the Tomori wall stand over the status wall's home in every environment, as they do today. Where an environment's `with_status_wall` gives a grid, it is the default those edits stand over. An edit the new home cannot take, such as six slots where the environment allows three, is kept but laid out as the layout already does: panels in columns it no longer has go into the nearest free slot or float.
- **Kept panels** stay in their slots, pinned and arranged, since the wall is the same. Nothing on the old wall is dismissed.
- **Names.** `presence` and `panels` call it `status wall (the Tomori wall)` for a release, then `status wall`. The `place:` and wall-tool parsers take both names, with or without `the`. The board's label becomes `STATUS`, or stays `TOMORI` (an open question).
- **Instructions.** `tomori-tools` says that the status wall is for notices and things said to the whole space, that answers go where the asker looks or beside them, that the agent moves to whoever it is answering on its own, and when to use `move_agent` (section 5).
- **The agent's home without a wall beneath it.** An environment's agent home no longer needs to stand 1.4 m up so that a wall fits beneath. The sightline test that the wall stands beneath the agent applies only where the status wall uses the default home.
- **Older clients** draw the status wall where the projection says, as they draw the Tomori wall, so they need nothing. Its label is drawn by the client, so an older one still says `TOMORI`.

## 3. Workers

### The problem

Today's arc stands about whoever the agent faces, through the agent, five columns each side at 2.3 m, rows 0.85 m apart, signs a metre wide, and it travels with the agent. Every summons in a dense scene puts eighteen metre-wide signs somewhere the environment never planned for, and the arc's radius is what fixes the summons at 2.3 m. In the farmyard, an open yard, the same arc is clear and the maintainer likes it: they face the agent and turn their head left and right to see the workers, "pretty much how I work every day".

### The options

| Option | Sparse (farmyard) | Dense (data centre) | Cost |
| --- | --- | --- | --- |
| **A. Today's arc, following the agent** | Works well | Smashes into racks wherever it goes, and keeps the agent at 2.3 m | None |
| **B. A bigger arc** | Signs far away, but readable since they grow with distance | Collides more, not less | Small |
| **C. The arc at home, not following** | As today at home; nothing to see while the agent is away | Collides only at home, which the environment can plan for | Small: drop the summoner's gathering |
| **D. An environment-declared worker zone** | The yard's corner, or as today | The environment puts them in an aisle, over a rack row, on a gantry | A registration hook and a fitting rule |
| **E. Collision-aware placement against geometry** | Unneeded | Works anywhere, but signs pushed aside by racks scatter, the "all over the place" of #271's review | Clients need the solids; placement becomes a search; hard to keep stable |
| **F. Stacking or compacting** | Unneeded until many workers | Fits a small area: smaller signs, tighter columns, more rows | A second layout, chosen by fit |
| **G. Miniature workers following the agent** | Charming, glanceable | Small enough to fit anywhere | A new small form; too small to read or point at comfortably |
| **H. Workers stay home while the agent roams** | Nothing lost: home is in view of the yard | Home planned by the environment | Same as C |

### Recommendation: C, D and F, with G as satellites

- **Workers stay at home** (C, H), in every mode, summons included. The arrangement stands about the home's audience, the eyes at the arrival, 1.6 m above it, as today's home arrangement does. It doesn't move when the agent moves. That keeps the arrangement people learn their way around in one place, which #231's rationale valued, and frees the agent to stand at a conversational distance. `SpaceState::summoner` stops gathering workers.
- **An environment may declare a worker zone** (D): `Environment::with_worker_zone(WorkerZone)`, a pose, the way it faces (towards whoever watches it, by the one facing convention) and an extent, width, height and depth, carried and bounded as anchors are and sent with the anchors. It replaces the `workers` anchor, which becomes a zone of today's arc's size. The data centre can put its workers in a clear aisle end or above the racks, facing the floor.
- **The default arrangement fits the zone** (F). It tries today's arc first. Where that doesn't fit the zone's extent, it fits a **compact** arrangement: signs at 0.7 of their size, columns as close as their signs allow, more rows up to the zone's height, and, beyond what fits, a single **"+N more"** sign that lists the rest when pointed at. The same guarantees hold as today: a worker's place never moves when titles, states or membership change; signs stack within 30° of their worker; picking grows with distance. Without a zone, today's arc stands, so Sky, the grid and the farmyard look exactly as they do now.
- **A sightline-style test for zones.** `tomori_environment` gains a check, as `sightlines` checks walls, that the arrangement for one to eighteen workers fits a zone clear of the environment's solids and reads from the arrival. Environments test their zone once, at build time. This replaces collision-aware placement at run time (E), which the review of #271 showed scatters signs. The client's `WorkerPlacement` hook stays for environments that want their own algorithm, such as ringing a selected pod.
- **Satellites while the agent is away** (G, small). While the agent is away from home, up to eight small motes orbit it, one for each running worker, in its state's colour. A finished worker's mote pulses once and fades. They are not pointed at and have no signs. They say "three are still working, one just finished" to whoever the agent is with, without bringing the arc. Pointing at the workers themselves happens at home, where they are readable from afar, as their signs grow with distance.

**Should workers follow at all?** No, under this proposal. The agent going to someone is a conversation. The workers are the space's ongoing work, which belongs to everyone and to a place. The satellites carry the one thing a conversation needs from them.

## 4. What an environment declares

Gathered, since dense environments need all of it. Each is optional, bounded and carried as today's anchors are:

| Hook | What it gives | Default |
| --- | --- | --- |
| `with_agent` (exists) | The agent's home | 2.3 m ahead of the arrival, 1.4 m up |
| `with_status_wall` (new; generalises `with_stage`) | Where the status wall stands, its grid, slots and whether it is on the scenery | Beneath the agent's home |
| `with_worker_zone` (new; generalises `with_workers`) | Where workers stand, and how much room they have | Today's arc about home |
| `with_solids` (exists) | Boxes the agent, its path, the status wall and beside-panels keep clear of | None |
| `with_open_space` (new) | Boxes the agent may stand in when following, such as aisles and the yard. Outside them it doesn't stand | Anywhere clear of solids within 20 m of home |
| `Object::at` (new, for a side's described objects) | Where an object a participant points at stands, so that the agent can stand off its line of sight, or go to it when asked | None: the focus falls back to gaze |

Solids today are a handful of boxes, so dense environments need more of them: **raise the limit to 256 boxes**, checked in the server against the swarm's box and the path, which costs microseconds per move. A rack row is one box, not forty. A data centre whose racks change at run time, as its topology does, publishes them from its **server side** with `Space::occupy(boxes)`, bounded as `describe` is, replacing its static solids for that binding. That is a later step, listed in stage 5.

## 5. MCP controls

### `move_agent`, extended

`move_agent`'s `place` gains forms, and keeps those it has:

- `home`: home, as today. It also ends a pin and stops following for this conversation, until someone speaks again.
- `near NAME`, `beside NAME`: to that person's **conversation spot**, as following would choose it, rather than 2.3 m in front. `front NAME` keeps today's 2.3 m in front of them, facing them.
- `stay`: pin where it stands now, as a summons does: no following and no return home, until released.
- `follow`: end a pin and follow the speaker again. It is refused, saying so, where the space's setting is Summon only or Stay home, since that is an owner's choice.
- `object ID`: to stand beside an object a side describes with a position (`Object::at`), such as `datacentre:rack/a12`, at the conversation distance from whoever asked, off their line of sight to it. It is refused for an object without a position.

Its result says where the agent is, its **mode** (`home`, `following`, `pinned`) and, while it follows, when it will go home if nothing more is said. `presence` reports the same under `agent`, with the conversation partner's name.

### How the agent decides

`tomori-tools` would say, in a few sentences: you move to whoever you are answering on your own, so don't call `move_agent` for that. Call it when someone asks you to stay, to go home, to follow again or to come to someone, or when someone asks you to show them something an object stands by ("take me to the overloaded server"). Say where you are going. Don't move yourself to fill silence, to look busy or between turns of the same conversation.

### Limits

- **The agent's own moves**, those not asked for in the words it is answering (which the server tells from `requested_by`, as for its other gated tools), are limited to **one every 20 s** and **six in ten minutes**. Beyond that they are refused, saying so, with the agent told to stay where it is.
- **Moves asked for** are limited only as a participant's summons is.
- **A participant always wins:** a summons or **Send Tomori home** after an agent's `stay` or `follow` overrides it at once.
- **Every move is logged** at info with its reason (follow, summons, tool, home), which stage 5 tunes from.

### Participants' controls

The hand menu's Space section keeps **Send Tomori home**, shown while it is away, and gains **Tomori: stay here** / **Tomori: follow again**, the same as `stay` and `follow`. An owner sets **Agent movement** and **Return home after** in the space settings. Whether someone can say "don't come to me" for themselves alone is an open question.

## 6. Spatial audio

- **Nearer and clearer.** At 1.5 m the agent's voice is a few decibels louder for the speaker than at today's 2.3 m (by an inverse-distance law, about 3.7 dB, or 2.5 dB above the unity it has at 2 m), well within the existing cap of 12 dB above unity, and clearly placed off their left shoulder. Its direction now tells them, and everyone else, who it is talking with.
- **Others hear the conversation from where it is.** Someone across the room hears the agent at the distance law's floor, never more than 6 dB below unity, from the direction of the conversation, which is what a person there would hear. No change to the law is needed. If it proves too quiet for onlookers, the agent's floor could be raised to 4 dB.
- **Gliding while speaking.** The voice moves with the swarm's voice points, and the panner's 30 ms smoothing already takes continuous motion without clicks. There is no Doppler, which would be wrong for a voice. Streaming over a long distance moves the voice at up to 6 m/s for about 2 s, which smoothing takes too, though it should be heard on the headset (stage 1's checks).
- **Held speech is unchanged.** A reply that starts while the speaker records is held for them and released 600 ms after X is let go. The agent is usually still gliding then, and settles within a second or two, while the reply already plays from it. Waiting for the agent to arrive before releasing held speech is not recommended: arrival is not guaranteed (a blocked path, a far partner), and the pause would grow unpredictable.
- **Echo.** A nearer, louder agent puts a little more of its voice into an open live microphone. Push-to-talk silences playback while recording, so only live-microphone users are affected, and `spatial-audio.md`'s echo measurement covers them.

## 7. Interaction with existing features

- **Placement (#269).** The looked-at wall and float-beside are unchanged. The agent's spot is chosen first and counts as taken for the beside fan, and the agent avoids the line of sight to the looked-at wall, so the two complement each other: the answer is where you look, the agent just to your left.
- **The speaking pill.** Unchanged. It already follows playback, not the agent's position. The pill could say `coming over` while the agent glides to you, but the turn on press says it better, so no.
- **Pointing at workers.** Workers stay at home and are pointed at there. Signs and picking grow with distance, so a worker across the yard is pointed at as one 3 m away. The satellites aren't pointed at.
- **Summoning (#231).** Kept as a pin (section 1). The summoned distance becomes the conversation distance, and the 2.3 m and 1.4 m constraints that came from the entourage go.
- **The entourage (#261).** Retired: the wall doesn't travel and the workers don't follow. `SpaceState::summoner` stays in the protocol for older clients but no longer gathers workers in newer ones.
- **Locked walls (#262).** The status wall takes the lock as the Tomori wall does, unlocked by default, or locked when it stands on the environment's scenery. Following never moves a wall.
- **Live chart panels (#267) and environment walls.** Unaffected. A console wall is a wall the agent keeps clear of and out of the line of sight to.
- **Restart and change of environment.** The agent starts at home, as today. Following is never kept.
- **The spectator** follows a participant and sees the agent where it stands. It needs nothing.
- **Older clients.** They glide the agent to `summoned` as today, so they see it follow. Without knowing that workers stay home, they still gather workers about `summoner` while the agent is away. The server could leave `summoner` out for them, which today means "about whoever the agent faces, 2.3 m before it", no better. **Recommendation:** accept it as a known limit for older clients, which update with the app.

## 8. A staged plan

1. **Follow the speaker, and go home** behind the **Agent movement** setting, default **Summon only**. The server chooses the conversation spot, with the turn on press, the 0.4 s commit, hysteresis, huddles, the queue rule and the return delay, from existing presence, attention, walls, panels and solids. Clients glide to `summoned` as today, adding the `via` waypoint and streaming. In **Follow** mode the Tomori wall stays home and workers stay at home. The pin replaces summons there. Moves are logged with their reasons. Tests: the spot's distance, bearing, height and clearance, refusal within 12° of the line of sight, hysteresis and the 10 s dwell, the queue rule, the timer's conditions and cancellation, visitors.
2. **The status wall.** `with_status_wall`, with `with_stage` as its compatible form, the rename and aliases, the retired travel, its sightline tests, the instructions, and the swarm drawing in up close. The entourage stops travelling in every mode.
3. **Workers at home.** `with_worker_zone`, the compact arrangement and its "+N more" sign, the zone's fit test, and the satellites.
4. **MCP controls.** `stay`, `follow`, `object ID`, the conversation spot for `near` and `beside`, the mode in `presence`, the rate limits, the hand menu's stay and follow, and `Object::at`.
5. **Tuning in dense environments.** The data-centre environment declares solids, open space, a status wall and a worker zone. Then: raise the solids limit, add `Space::occupy` for run-time geometry, tune the distances, the dwell and the return delay from the logs and the headset, and decide whether **Follow the speaker** becomes the default.

Stages 1 and 2 can swap if the data centre's clutter is the pressing problem, since the status wall alone frees the agent's surroundings.

## Risks

- **Motion as distraction.** An agent that moves at every question becomes the thing everyone watches. The 0.4 s commit, the 3 m and 50° "near enough" rule and the 10 s dwell are there to make it move rarely. The logs will show how often it does.
- **Motion comfort.** A large, close, moving object can unsettle some people even when they aren't moved themselves. The 0.8 m clearance, the ease, the 0.85 m/s cap and streaming for long distances are meant to help, and the headset is the judge.
- **Multi-user etiquette.** One person dominating the agent's position, or the agent leaving someone mid-thought for another, is the risk of any shared assistant. Queue order, huddles and the pin are the answers proposed. An owner's **Summon only** setting remains.
- **Losing the "home base" feel.** The maintainer's daily pattern, the agent ahead with workers either side, partly goes away when the agent walks off. Workers staying put and the agent returning after 90 s are what keep it.
- **Wrong focus.** Gaze from the head is coarse, and a pointed environment object has no position until sides give one, so the agent may stand where something is. The 12° line-of-sight test uses whatever focus is known, and a participant can always move it with the trigger.
- **Dense scenes without geometry.** An environment that declares no solids gets an agent that can stand inside a rack. Stage 5's test, that the arrival's region is covered by solids or open space, is what catches it.

## Decisions requested

1. Following, as modelled: on press it turns; after 0.4 s it commits from what was captured as the words began; it moves only beyond 3 m and 50° or when in the way; it stays at least 10 s; in a queue, it moves to whoever it answers. **Recommended.**
2. Where it stands: 1.5 m, 45° to the speaker's left of their focus, eye height plus 0.1 m, facing them. **Recommended**, left because float-beside goes right first.
3. Return home after 90 s idle, as a space setting from 30 s to never, with a running worker not holding it. **Recommended.**
4. The trigger's summons kept as a pin ("come here and stay"). **Recommended.**
5. The Tomori wall becomes the space's status wall, never travelling, its home and size given by `with_status_wall`, falling back to beneath the agent's home. **Recommended.**
6. Workers stay at home, fitted to an environment's worker zone, compacted where needed, with satellites on the agent while away. **Recommended.**
7. `move_agent` gains `stay`, `follow` and `object ID`, with self-initiated moves limited to one every 20 s. **Recommended.**
8. Stage 1 behind a setting, default Summon only, before any default changes. **Recommended.**

## Open questions for you

- Should the board be labelled `STATUS`, or keep `TOMORI` as the space's own wall?
- Should the agent go to whoever asked for a worker when that worker reports, or report from wherever it is? (Recommended: wherever it is, with the notice on the status wall.)
- Should someone be able to say "don't come to me" for themselves, as a device setting, for example while presenting?
- Should the swarm draw in up close (recommended), or should the conversation distance grow to suit its full size?
- Left or right by default, or the side with more open space each time?
- For the data centre: does its scene have aisles it can declare as open space, and a natural place for a status strip and a worker zone?
- Should a summons pin survive the agent's own `follow`, or should only a participant release it?

## Design changes this would make

- `design/environments.md`: the agent's home and its return; following and the pin replacing "it never moves of its own accord"; `with_status_wall`, `with_worker_zone`, `with_open_space`, `Object::at` and the solids' limit; workers at home; the entourage retired; the rationale.
- `design/projection.md`: the Tomori wall as the status wall, its home and grid, no travel, its names in `place:`; the agent's spot taken in the beside fan.
- `design/interaction.md`: the turn on press; the trigger's summons as a pin; workers at home; the satellites.
- `design/mcp.md`: `move_agent`'s new forms and limits; `presence`'s agent mode; `tomori-tools`.
- `design/spatial-audio.md`: a note on the agent's nearer voice and on movement while speaking.
- `design/hand-menu.md`: stay and follow; the space settings for agent movement and the return delay.
- `design/avatars.md`: the avatar told its distance to whoever it faces; the swarm drawing in.
- `design/farmyard.md`: whether it declares a status wall, a worker zone and open space, or keeps today's defaults.
