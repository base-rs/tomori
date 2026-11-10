# Proposal: The Agent Follows the Speaker

Status: **approved by the maintainer**, November 2026, with the amendments below (cluster anchors and placing workers, the trigger's summons removed, and the farmyard's notice board as its status wall), to be implemented in the stages of section 9. Stage 1, following the speaker, going home, removing the trigger's summons and **Call Tomori here**, stage 2, the status wall, the retired entourage and the swarm drawing in up close, stage 3, the shared cluster rule, the home anchor's cluster, its fit test and the satellites, and stage 4, named anchors declared and placed by a server side within their limits, anchors in `presence`, the server's seating of every cluster with hysteresis, "+N more" naming its workers, `place_worker`, `place: anchor NAME`, `move_agent anchor NAME`, removal and its notice, and placements kept by anchor name with their 60 s grace, are now in the design documents. Nothing here is a requirement until each stage brings it into the design documents it changes (listed at the end), which take precedence until then.

It builds on the homes and anchors of `design/environments.md` (#231, #271), the summons and the entourage that travels with it (#231, #261), the workers' arc (#264, #270, #271), environment walls and their locks (#262), environments' server sides and the limits they keep within (#275), the float-beside placement of `design/projection.md` (#269), shared attention and `presence` (`design/interaction.md`, `design/mcp.md`, #223), and held speech and the speaking pill (`design/speech.md`, #244). Its cluster anchors are what #259, participants carrying workers as they carry panels, would build on.

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

Then, reading the first draft, from the headset:

- **Workers in relevant places.** The agent should be able to place its workers, not only itself: a worker investigating a rack stands by that rack.
- **Named anchors that things cluster around.** An environment declares named anchors, a point and some room. Anything placed at one clusters around it: one item at the point, several in a small arc or grid fitted to the room, with the compact fallback.
- **Anchors that come and go.** Environments create and remove anchors as they run, from their server side, as their state changes: a rack alerting, a new area opening.
- **Remove the trigger's summons.** Rather than keeping #231's 0.5 s hold as "come here and stay", summoning happens by voice, "Hey Tomori, come here and help me investigate this", which following and `move_agent stay` handle. That frees the trigger for pointing and selecting, which the maintainer thinks much more useful.
- **The farmyard's notice board becomes its status wall**, rather than the Tomori wall.

The maintainer then approved the proposal with these amendments, to be implemented in stages.

## Recommendation in brief

- **The server moves the agent to whoever it is talking with.** As a participant's words begin, the server, which already captures what they point at and look at and knows every head, wall, floating panel and the environment's solids, chooses a **conversation spot**: about **1.5 m** from their eyes (1.2 to 1.8 m), **off to the left** of their line of sight to what they attend to, so that it never blocks it, at about their eye height, facing them. The agent turns towards them at once and glides there if it is not already near enough. It uses the same `SpaceState::summoned` every client already glides the agent to.
- **Hysteresis and turn-taking.** It moves only when the speaker is beyond conversational range or it blocks their view; it serves a group from one spot when several nearby people are talking; and when the agent is busy with one person's request, a second person's words move it only when their turn is answered, in queue order. Visitors never move it.
- **It goes home after 90 seconds** (tunable, 30 s to 10 min, or never) of an idle conversation: nobody recording, nothing queued, no question or approval waiting, no reply playing or held, and the agent's session idle. Any new words cancel the return.
- **The trigger's summons goes.** Summoning is by voice: "come here" is following, and "stay here" is `move_agent stay`, a **pin** that suspends following and the return timer until someone sends it home or asks it to follow. Without speech, or while the agent is busy, the hand menu's **Call Tomori here**, beside **Send Tomori home**, brings it and pins it. The trigger is freed for pointing and selecting. A server that stops keeping summons already tells older clients to send none.
- **The Tomori wall becomes the space's status wall.** It keeps its identity, panels and edits, but stops being the agent's fixture: it never travels with the agent, and environments say where it is with a new `Environment::with_status_wall`, which generalises `with_stage`: either a wall of the space's own that they place and size, or **one of their own walls**, as the farmyard's **notice board becomes its status wall**, its lock and the environment's own panels there coming first. Without one it stands where it stands today, beneath the agent's home. Answers keep going where the asker looks or beside them; Tomori's own notices go to the status wall.
- **Workers stay at home unless placed.** They no longer travel with the agent. By default they stand about the home on today's arc, fitted to the room the environment gives it, with a **compact** arrangement as fallback. While the agent is away, a few small **satellites** orbiting it show how many workers are running and whether any has finished.
- **Cluster anchors put work where it belongs.** An environment names places, each a pose, whose point things gather at, and an extent, with an optional kind and label: statically with `Environment::with_anchor`, or at run time from its server side with `Space::place_anchor` and `Space::remove_anchor`, within limits that `tomori-environment` exports (#275). `presence` lists them. One shared rule clusters whatever stands at one, workers, panels or the agent: one at the point, then an arc, then a grid, then compact. The home's worker arrangement is simply the `home` anchor's cluster.
- **The agent places its workers** with a new `place_worker` tool, at an anchor, an object, a wall or a person, rate-limited so that workers don't drift; panels take `place: anchor NAME`, and the agent `move_agent anchor NAME`. A worker with no relevant place stays home. When an anchor goes, its workers go home and its panels stay floating where they are, with a notice. Placements are kept with the space by anchor name.
- **`move_agent` gains `stay`, `follow`, `object ID` and `anchor NAME`,** `presence` says the agent's mode, and the server limits how often the agent may move itself.
- **Staged:** (1) following and the return home, behind a space setting, with the trigger's summons replaced by voice and **Call Tomori here**; (2) the status wall; (3) workers at home, as the home anchor's cluster, with the shared cluster rule and compaction; (4) cluster anchors, static then dynamic, and placing workers; (5) the agent's own MCP controls; (6) tuning in dense environments, then following by default.

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
| Anchors | A fixed set, each a pose in the registration: `arrival`, `agent`, `stage` and `workers`. None is named by the environment, none comes or goes at run time, and a worker has no resource, so it cannot stand by what it investigates | `environments.md` |
| Solids | An environment gives boxes of its scenery, `Environment::with_solids`, which the wall's travel spot and the beside placement avoid | `environments.md`, `projection.md` |
| Agent voice | From the avatar's voice points, wherever it stands. Unity at 2 m, at most 6 dB quieter far away, never louder within 0.5 m | `spatial-audio.md` |
| The swarm's size | Up to 1.35 m across, 0.52 m tall and 0.42 m deep (`entourage::SWARM`) | `avatars.md` |

So the parts this needs exist: the server already owns the agent's pose for everyone, captures attention when words begin, knows heads, walls, panels and solids, and searches for clear spots for walls and panels. What changes is **who decides to move the agent** (the conversation, not only a trigger), **where it stands** (closer, beside, without an entourage), and **what no longer moves with it** (the wall and the workers).

## 1. The behaviour model

### Who is being talked with

The agent has one **conversation partner** at a time: the participant whose words it is answering now, or else whose words it last answered. In a group, the partner can be a **huddle** (below). The partner is the server's to decide, from what it already has: the queue, the delivered input's speaker, and presence.

- **When the agent is free**, with nothing queued and nobody's words being answered, the participant who presses X becomes the partner as their words begin.
- **When the agent is busy** with someone else's words, a new speaker's press **turns the agent's halo towards them for a moment**, an acknowledgement, without moving it. They become the partner, and the agent moves, when their words are **delivered**, which is when the agent begins answering them, in queue order. "The one addressed" is therefore whoever the agent is answering. Words in Tomori are always addressed to the agent, so there is no need to guess from names.
- **Visitors never become the partner.** They cannot speak to the agent, and their trigger moves nothing.
- **Words that are not a question**, such as renaming the space (titling) or a recording cancelled or with no speech heard, never move the agent. A move already under way stops where it is, and the return timer starts from there.

### When it moves

1. **On press, it turns.** As X goes down, the agent turns to face the speaker at once (a yaw, not a move), as a person looks up when spoken to. This costs nothing in comfort and tells the speaker they are heard.
2. **After 0.4 s of holding, it commits.** A tap or a press let go at once, which push-to-talk often is when it is a mistake, moves nothing. After 0.4 s the server chooses the conversation spot from the attention and presence captured **as the words began**, the same moment `projection.md` places answers from, and the agent starts gliding. Moving while the person is still talking, rather than after transcription, matters: a glide of a few metres takes several seconds, and that time overlaps their speaking and the agent's thinking, so it usually arrives before the reply plays.
3. **It only moves when it needs to.** It stays where it is, turning only, when it already stands within **3 m** of the speaker's eyes, within **50°** of the way their head faces, seen clearly from their eyes, and not in their way (below). So a follow-up question from the same place moves nothing, and someone turning slightly to point at another rack does not drag it about.
4. **It doesn't jitter between speakers.** Once it arrives somewhere it **stays for at least 10 s** before moving for another partner, unless that partner is more than 6 m away or cannot see it. When the new partner is within 3 m of the old one, it doesn't leave one for the other; it serves both, as a huddle.
5. **Huddles.** When two or more participants have spoken to it within the last 60 s and stand within 4 m of each other, the conversation spot serves the group: about 1.5 m from the nearest of them, on the side of the group facing most of them, facing the middle of their heads, clear of every line of sight to what any of them attends to. A group around a rack gets one agent standing at the edge of the circle, not one that hops between them.

### Where it stands

The server chooses the spot when the agent commits, from the speaker's **eyes** E and their **focus** F: what they pointed at as the words began, else the wall, panel or person they looked at, as presence resolves it, else 2 m along the way their head faces. A worker or an environment object pointed at counts too, where the server knows where it stands (for an object, that needs the side to give a position; see section 5).

- **Distance:** 1.5 m from E horizontally, a comfortable conversational distance between a person's 1.2 m social bubble and the 1.8 m at which a group forms. Candidates are tried at 1.5, 1.8, then 1.2 m.
- **Bearing:** 45° to the speaker's **left** of the line from E to F. That is out of the line of sight, so it never blocks what they look at, and away from where `beside` puts the first floating panel, 30° to their right, so that a reply's panel and the agent don't stand in each other's way. The candidates are then 45° right, 70° left, 70° right, 25° left and 25° right. A candidate is refused when the swarm's reach (`entourage::SWARM`) falls within **12°** of the line from E to F, as seen from E.
- **Height:** the swarm's middle at the speaker's eye height plus 0.1 m, as a face across from them, so it reads alike for someone sitting or standing. It is never below 1.1 m nor above 2.4 m above the floor beneath it. Without a wall beneath the agent, today's 1.4 m least height (`LOWEST_AGENT`) no longer applies.
- **Facing:** towards E. While it speaks about something pointed at, the halo may glance towards F and back, a later refinement that every avatar could take as a hint.
- **Clear of everything:** the box of the swarm's reach, 0.1 m larger on every side, must be clear of the environment's solids, of every wall shown (its slots 5 cm thicker, as the beside placement takes them), of every floating panel within 3 m, and at least **0.8 m** from every participant's head, so that it never ends in anyone's face. The line from E to its middle must also pass through no solid or wall. This is the test `stage/travel.rs` already makes for the wall's travel spot, and the beside placement for panels, shared rather than written a third time.
- **Only where the environment lets it stand:** if the environment declares **open space** (section 5), the spot must be inside it. This is how a dense environment stops the agent standing inside a rack row whose solids it never gave.
- **Fallback:** with every candidate refused, it tries the same bearings at 2.3 m, today's summons distance. With none clear, it stays where it is and only turns. It never stands somewhere it would be hidden.

Whatever stands in a cluster at an anchor (section 4) counts as solid for this test, as the server lays clusters out by the same rule as clients. The agent's spot then counts as **taken** for the beside placement, as a floating panel does, so that panels in the reply that follows fan out around it rather than through it. The agent is placed first, as words begin, and panels after, as the reply completes, so the order is natural.

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

### Summoning by voice, and the trigger's summons removed

**Recommendation, as the maintainer decided: remove the trigger's summons** (#231's pull held 0.5 s on nothing). Summoning is spoken instead, and following does most of it:

- **"Hey Tomori, come here and help me investigate this"** moves the agent because its words are a question to it: it comes to the speaker's conversation spot, as for any words, and the return timer runs as usual.
- **"Stay here", "wait by this rack"** pins it: the agent calls `move_agent stay` (section 6). While **pinned**, it doesn't follow the speaker and doesn't return home on its own. It still turns to whoever speaks. A pin ends with **Send Tomori home**, `move_agent home` or `follow`, a change of environment, a restart, or after **30 minutes** with nobody in the space.
- **Without a voice**, as when someone's speech is off, their microphone is muted or the agent is busy with someone else's words, the hand menu's Space section offers **Call Tomori here**, beside **Send Tomori home**: the server brings the agent to their conversation spot at once and pins it there, as the summons did, without asking the agent, so that it works with any backend, while the agent is busy, and in the **Only when asked** setting (below). It is shown while the agent is not already pinned near them. **Stay home** hides it.

The case the pin serves is "stand there by that rack while we work through this", which following would undo every time someone stepped away to look at something. Voice already carries it, with what they point at, and needs no gesture to learn.

**The trigger, freed.** A pull on nothing of Tomori's no longer draws the swarm or sends a summons, so the trigger becomes the hand's way of **pointing and selecting**: environment objects (the `Input::select` a client side already has, extended beyond objects marked selectable), workers, a worker's place or an anchor's cluster (section 4), and list items on panels. What each selection does, and how it relates to A, which already chooses and ticks, is left as an open question; this proposal only frees it. The trigger's order of rungs in `interaction.md` loses its swarm rung.

**Older clients.** An older client still sends `ClientMessage::PlaceAgent` after a 0.5 s pull, but only to a server whose space state says `SpaceState::summons`; a client already sends nothing to one that does not. So the server stops setting `summons`, and an older client's pull only draws its own swarm while held, then lets it go back, as a visitor's does, changing nothing for anyone else. A `PlaceAgent` that arrives anyway, as from a client that ignores the flag, is dropped and logged at debug. Older clients lack **Call Tomori here** until they update, but can still say "come here".

### A space setting

**Agent movement**, an owner's space setting, kept with the space as other settings are:

- **Follow the speaker**: everything above;
- **Only when asked**: the agent moves only when someone asks it by voice, through `move_agent`, or with **Call Tomori here**, and where it goes it stays until sent home, as a summons did (without the entourage, from stage 2);
- **Stay home**: never moves, **Call Tomori here** and `move_agent` included, for a presentation or a cramped environment.

In stage 1 the default is **Only when asked**, so the agent moves on its own only once an owner turns following on. Stage 6 decides whether **Follow the speaker** becomes the default.

## 2. The status wall

### What it is

The Tomori wall stops being the agent's fixture and becomes the space's **status wall**: the place for what the space and Tomori say about themselves rather than in answer to someone. It is the **same wall**, the same identity (`tomori_protocol::TOMORI_WALL`), the same kept panels and edits, so nothing a space keeps is lost or moved. What changes is where it stands, its name and how big it may be.

- **It never travels.** It stays at its home whatever the agent does: no fold, travel or unfold, and `stage/travel.rs` retires. That was what made the dense environment hard: a wall that had to find room beneath the agent wherever it went.
- **Where it stands is the environment's choice**, with `Environment::with_status_wall(StatusWall)`, in one of two forms:
  - **One of its own walls**, `StatusWall::own(N)`, naming a wall it declares with `with_walls` by its place in their order, from 0, as a side's `Panel::on_wall` names one. That wall *is* the status wall in that environment: a notice board or console the place already has, its pose, grid, lock and scenery all the environment's. The Tomori wall is not shown there at all.
  - **A wall of its own**, `StatusWall::at(EnvironmentWall)`, in the environment's own coordinates and carried as every anchor is: where the space's status wall stands **and** its grid, slot size and shape, which `with_stage` could not give, such as a compact strip by a data centre's door.
- **`with_stage` stays** as `StatusWall::at` with today's grid, so that existing registrations build unchanged.
- **Without either, it stands where it stands today,** beneath the agent's home, as `entourage::wall_beneath` puts it. That is the backward-compatible fallback for Sky, the grid and the GitHub prototype until they choose otherwise. It stays tied to the home, not the agent, so it no longer follows the agent anywhere.
- **Bounds:** within 20 m of home, upright and finite, as every wall is. Its grid may be up to the general wall bounds, but **at least two slots**, so that two notices can stand together; `StatusWall::own` naming a wall of one slot, or a wall the environment doesn't declare, is left out with a warning, and the default stands. Each environment's sightline tests check that it stands clear of solids and is readable: from the arrival (4 m, or further in proportion for a big wall) for `StatusWall::at` and the default, as the Tomori wall's home is checked today; and, for one of its own walls, from the ground in front of it, which participants can reach, and seen, though not necessarily readable, from the arrival.

### The worked example: the farmyard's notice board

The farmyard registers `with_status_wall(StatusWall::own(0))`: its **notice board** on the fence, two slots wide and one high, becomes its status wall, and the Tomori wall beneath the agent no longer stands in the yard. A farm's notices go on its notice board, and the yard loses the second board that stood beneath the agent.

- **Its lock is unchanged (#262).** The board stays an environment wall, locked by default, which locks its pose, grid and shape, not its content: notices still go onto it, as panels go onto any locked wall. An owner may unlock it, move or reshape it for the space in the farmyard, and put it back with `↺`, as today; the status role moves with the wall wherever it stands. It is never removed.
- **Its edits are the board's.** What the space kept of the board in the farmyard, its grid, slots, tilt, lock and, unlocked, its pose, stands as today. The Tomori wall's own kept edits do not apply to it: they are kept, and stand wherever the status wall is the space's own wall, in other environments.
- **Two kinds of panel share its slots.** The board holds what it holds today, the environment's and participants' panels, kept for the space in the farmyard, and also the space's **status panels**: Tomori's notices, the panels it projects unasked, `place: wall status wall`, and those the space kept on the Tomori wall, which belong to the space and travel with it. A panel's kind follows how it arrived: whatever came through the status wall's role is a status panel and leaves with the space when it changes environment, to the next environment's status wall; whatever a participant or the agent puts on the board by hand, or the side publishes there, is the board's, kept for the farmyard. Moving one between the board's slots keeps its kind; pulling one off and putting it back makes it the board's. `panels` and `presence` say which, as `status: true`.
- **When it is full.** A notice takes the board's free slot; with none, it gives up the oldest **status panel** not arranged, as a full wall gives up its oldest today; only then one of the board's own not arranged; and with every slot holding an environment's published panel or one a participant arranged, it floats just in front of the board's right end, facing as the board faces, so that a notice is never lost. **An environment's own panels come first:** a side publishing onto a board that is full of status panels gives up the oldest of them rather than being refused, since the place is the environment's and it chose to share its board. A live chart there keeps its slot and its own data path (`projection.md`); notices never displace it.
- **Migrating its panels.** On first opening in the farmyard after this change, the space's status panels, up to six kept on the Tomori wall, go into the board's free slots, those a participant arranged first, then the newest. Those that do not fit are kept, not shown, listed by `panels` as waiting for a slot, and appear as slots free, newest first, or on the status wall of an environment with room. Nothing is dismissed.
- **Readable where people go.** The board stands beyond reading distance from the arrival, facing into the yard rather than towards it, as `farmyard.md`'s tests require, so a notice there is read by walking, or teleporting, up to it. That suits notices, which are rarely urgent: what matters is said aloud, the newest panel is framed in blue, and a pairing waiting is also on the owners' pill and hand menu. Whether a distant status wall should give a quiet cue when a notice arrives is an open question.
- **No new farmyard version.** The board already stands at version 3; which wall holds the status panels is the projection's, which every client draws as sent, so an older client sees notices on the board too.

**Why one status wall in every environment, but sometimes an environment's own?** The space's status panels belong to the space and must follow it, which is why the status wall is not simply one more environment wall. But a place that already has a notice board or console should not grow a second board beside it. Letting the status role rest on an environment's own wall, while each panel keeps its own kind and storage, gives both: the farmyard's notices are on its board, and a space that moves on takes its notices with it.

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

- **Kept edits.** The space's kept grid, shape, tilt and lock of the Tomori wall stand over the status wall's home wherever the status wall is the space's own wall, the default or `StatusWall::at`, as they do today. Where `StatusWall::at` gives a grid, it is the default those edits stand over. An edit the new home cannot take, such as six slots where the environment allows three, is kept but laid out as the layout already does: panels in columns it no longer has go into the nearest free slot or float. Where the status wall is an environment's own, as the farmyard's board, the board's edits stand instead, and the Tomori wall's are kept for elsewhere (above).
- **Kept panels** stay in their slots, pinned and arranged, where the wall is the same; on an environment's own wall they move into its free slots, as the farmyard's example says. Nothing on the old wall is dismissed.
- **Names.** `presence` and `panels` call it `status wall (the Tomori wall)` for a release, then `status wall`; an environment's own wall in the role is named as both, `environment wall 1 (the status wall)`. The `place:` and wall-tool parsers take every name, with or without `the`. The default board's label becomes `STATUS`, or stays `TOMORI` (an open question); an environment's own wall keeps whatever its scenery draws.
- **Instructions.** `tomori-tools` says that the status wall is for notices and things said to the whole space, that answers go where the asker looks or beside them, that the agent moves to whoever it is answering on its own, and when to use `move_agent` and `place_worker` (section 6).
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

- **Workers stay at home** (C, H), in every mode, unless the agent or a participant places one somewhere relevant (section 4). The arrangement stands about the home's audience, the eyes at the arrival, 1.6 m above it, as today's home arrangement does. It doesn't move when the agent moves. That keeps the arrangement people learn their way around in one place, which #231's rationale valued, and frees the agent to stand at a conversational distance. `SpaceState::summoner` stops gathering workers.
- **An environment may say where home's workers stand, and how much room they have** (D): the **`home` cluster anchor**, `Environment::with_anchor(Anchor::new("home", pose).with_extent(…))`, a pose, the way it faces (towards whoever watches it, by the one facing convention) and an extent, width, height and depth, carried and bounded as anchors are. An earlier draft called this a worker zone, with a hook of its own; it is now simply the home anchor of section 4, so that one rule and one test serve home and every other place. It replaces the `workers` anchor, which `with_workers` keeps declaring, as a `home` anchor of today's arc's size. The data centre can put its workers in a clear aisle end or above the racks, facing the floor.
- **The default arrangement fits the home anchor's room** (F), by the cluster rule of section 4. It tries today's arc first. Where that doesn't fit the extent, it fits a **compact** arrangement: signs at 0.7 of their size, columns as close as their signs allow, more rows up to the zone's height, and, beyond what fits, a single **"+N more"** sign that lists the rest when pointed at. The same guarantees hold as today: a worker's place never moves when titles, states or membership change; signs stack within 30° of their worker; picking grows with distance. Without a home anchor, today's arc stands, so Sky, the grid and the farmyard look exactly as they do now.
- **A sightline-style test for anchors.** `tomori_environment` gains a check, as `sightlines` checks walls, that the cluster for one to eighteen members fits an anchor's extent clear of the environment's solids and reads from the arrival, or from the anchor's audience. Environments test their static anchors once, at build time, `home` included. This replaces collision-aware placement at run time (E), which the review of #271 showed scatters signs. The client's `WorkerPlacement` hook stays for the home cluster, for environments that want their own algorithm there, such as ringing a selected pod.
- **Satellites while the agent is away** (G, small). While the agent is away from home, up to eight small motes orbit it, one for each running worker, wherever it stands, in its state's colour. A finished worker's mote pulses once and fades. They are not pointed at and have no signs. They say "three are still working, one just finished" to whoever the agent is with, without bringing the arc. Pointing at the workers themselves happens at home, where they are readable from afar, as their signs grow with distance.

**Should workers follow at all?** No, under this proposal. The agent going to someone is a conversation. The workers are the space's ongoing work, which belongs to everyone and to a place: home by default, or the place their work is about, where the agent or a participant puts them (section 4). The satellites carry the one thing a conversation needs from them.

## 4. Cluster anchors: work where it belongs

Workers at home are the right default, but a worker investigating rack A12 is better by rack A12: whoever looks at the rack sees that something is working on it, and pointing at the worker is pointing at the work. The maintainer asked for that, and for environments to name the places things gather, statically and as their state changes.

### What an anchor is

A **cluster anchor** is a named place in an environment that workers, panels and the agent can be put at:

- a **name**, 1 to 64 bytes of ASCII letters, digits, `-`, `_`, `.` and `/`, the same rule as an object's reference (`valid_reference`), such as `rack/a12` or `aisle-3`, by which the agent names it and the space keeps what stands there;
- a **pose**, in the environment's own coordinates, carried into the space's as every anchor is, facing along +Z towards whoever watches it, by the one facing convention: its **point**, where a single item stands;
- an **extent**, width, height and depth, each 0.3 to 12 m, a box centred on the point: the room its cluster may take;
- optionally a **kind**, a slug of at most 32 bytes such as `rack`, `area` or `console`, and a **label**, one line of at most 80 characters, cleaned as an object's description is, such as `Rack A12, alerting: CPU throttled`, which the agent is told;
- optionally the **object** it stands by, the reference of an object the side describes, so that `object datacentre:rack/a12` and `anchor rack/a12` lead to the same cluster.

**Declared statically**, in the registration, beside walls and solids: `Environment::with_anchor(Anchor::new("aisle-3", pose).with_extent(4.0, 2.5, 1.5).with_kind("area").with_label("Aisle 3"))`. The anchor named **`home`** is the home's worker arrangement (section 3), declared only this way.

**Created and removed at run time** by the environment's server side, through its `Space`, beside `share`, `describe` and `publish`: `Space::place_anchor(Anchor)` creates an anchor, or, given a name the side placed already, moves, resizes or relabels it in place, its cluster gliding with it; `Space::remove_anchor(name)` removes it (below). A data centre places `rack/a12` as the rack starts alerting and removes it as it recovers, or places `hall-2` as a new hall opens. Dynamic anchors belong to the binding, as everything a side keeps does: forgotten when the space is bound to another environment, closes or is bound again, as after a restart, so a side places its anchors again as it starts.

### Limits and identifiers

The limits are defined once in `tomori-environment`, exported and enforced by the server as the same constants, as #275 made every limit a side keeps within:

- **`ANCHOR_LIMIT`, 64 anchors** in a binding, static ones counted first. Placing beyond it is refused with a message the side may show as its error, as a refused state is.
- **`ANCHOR_NAME_BYTES`**, equal to `REFERENCE_BYTES`; **`ANCHOR_LABEL_LIMIT`, 80 characters**, cut rather than refused, as descriptions are.
- **Bounds**, as for every anchor: finite, within 20 m of home on each axis, with an orientation, made upright; an extent within 0.3 to 12 m. A static anchor that isn't is left out with a warning; a dynamic one is refused, saying why.
- **Rate**, as for a side's panels: about one change a second for each anchor, and 4 changes a second in a space after a burst of 8, the latest winning while one waits, a removal cancelling a placement still waiting. Every change lays a cluster out again and moves what stands there, so a flapping alert must not shake workers about; a side that debounces its own state is the better fix, and the limit is the backstop.
- **Stable names.** A side chooses names that stay the same for as long as the place is the same: a rack that alerts, recovers and alerts again is `rack/a12` each time, so that what the space keeps by name finds it again.
- **Collisions.** Placing a dynamic name again updates that anchor; there are never two of one name. Two static anchors of one name keep the first and leave out the rest, with a warning as the server starts, and the environment's tests catch it. A dynamic anchor named as a static one, or `home`, is refused, saying so: the place is the environment's.
- **Names belong to an environment.** The agent names an anchor by its name alone, in its space's current environment, and the space keeps placements by environment and name, so another environment's `home` or `aisle-3` is never confused with this one's.

### Carried to clients, and reported

- **To clients**, with the space's state, `SpaceState::anchors.clusters`, carried into the space's coordinates: each anchor's name, pose, extent, kind and label, at most about 16 KiB for 64, sent again on each change, which the rate limit bounds. Where each worker or panel stands is sent with it, as the anchor it is at (below). An older client ignores the field and keeps every worker at home, as it does now.
- **To the agent**, in `presence`, under `anchors`: each one's name, kind, label, position, facing yaw, extent, whether the environment placed it at run time, the object it stands by, and what stands there now, the agent, workers by session and title, and panels by identity. So the agent finds "the rack that's alerting" by its kind and label, not by coordinates. What someone looks or points at names the anchor too, where it is a member of a cluster, as `worker "Investigate A12" (at rack/a12)`. Sixty-four anchors add a few kilobytes to `presence`; a separate `anchors` tool is an open question, should it grow too long.

### Clustering: one rule for everything at an anchor

One rule, `tomori_protocol::cluster`, shared by the server and every client, as `wall` and `entourage` are, lays out whatever stands at an anchor, workers, floating panels and the agent alike, each by the footprint it draws: a worker with its sign about 1 m across, a floating panel 0.85 by 0.6 m, the swarm drawn in to 0.7 m.

1. **One item stands at the point**, facing along the anchor's +Z.
2. **Several form an arc through the point**, curving about the anchor's **audience**, a point 2.3 m before it along +Z, or, for `home`, the eyes at the arrival, 1.6 m above it, as today: alternately right and left of the point, as seen from the audience, a footprint apart, as many as fit across the extent's width, then a row above, 0.85 m higher, as its height allows.
3. **Where an arc doesn't fit**, as in an extent deeper than it is wide, such as an aisle seen from its end, a **grid** in the extent's face, columns across and rows up, then further back along its depth.
4. **Then compact**: footprints at 0.7, columns as close as their signs allow, and beyond what fits, one **"+N more"** sign at the cluster's end, listing the rest when pointed at.

- **Order within a cluster is order of arrival.** The first to arrive stands at the point, the next to its right, then left, and so on. A member keeps its slot while it stays, so that nothing moves when another arrives, leaves, or changes state or title; a gap left by one leaving is filled by the next to arrive. Only a change to the anchor itself, moved, resized or with the geometry about it changed, lays the cluster out again, in slot order, each member gliding to its new slot. At `home` the point is the agent's own, kept for it while it is away, so that workers stand either side of it as today.
- **Clear of the environment's solids.** A static anchor's room is the environment's promise, checked at build time (section 3's test). A dynamic one cannot be, so the server and clients skip any slot whose footprint, 0.1 m larger, meets a solid or a wall shown, and take the next, never pushing a member aside. Skipping depends only on the anchor and the environment's geometry, so it is stable, and every client skips the same slots. A side that knows its scene places anchors in clear space; a slot skipped is logged at debug, so that a side placing anchors inside racks finds out.
- **Members are known to the server.** Since it lays clusters out by the same rule, the server knows where each member stands, so `presence` reports it, the agent's conversation spot (section 1) and the beside fan treat members as taken, and a member pointed at is resolved.
- **The worker zone is simply the home anchor's cluster.** Recommended: the rule above, with the audience at the arrival's eyes and the agent at the point, *is* today's arc, so `home` needs nothing of its own. Unifying them means one layout, one fit test and one compaction for home and every other place, and that #259's carrying (below) puts a worker back home exactly as it puts one by a rack.

**Defaults stay.** A worker nobody placed stands at home. A worker's own workers stand with it, wherever it is, since they work on its task, unless placed themselves.

### When an anchor goes

When the side removes an anchor:

- **Its workers go home.** Each glides back to the home cluster, taking the next free slot there, and its placement is forgotten: it does not return if the anchor comes back, since a worker that follows a flapping alert back and forth is exactly the drift the limits are there to prevent. The agent may place it again.
- **Its panels stay where they stand**, floating, facing as they faced, as any floating panel does, pinned only if a participant had pinned them, within the 120 that float.
- **The agent**, if it stood there, stays where it is, and the following and return rules apply as anywhere.
- **One notice** on the status wall, titled "Places", from Tomori, says what went and what moved, such as "The environment closed rack/a12: 2 workers went home, 1 panel was left floating.", replacing the last such notice unless a participant put it somewhere, as Tomori's other notices do. An anchor removed with nothing at it says nothing.

A binding ending, as the space changing environment or the server restarting, is not a removal: nothing is said, and placements are kept (below).

### Across restarts and changes of environment

- **Placements are kept with the space**, for each environment, as its panels are: for each worker placed, its session and the anchor's name, or the object's id, the wall's identity, or a pose for one placed beside a person or left by a participant; for each panel at an anchor, the anchor's name with what else it keeps of the panel. At most 64 worker placements are kept for each environment, and a worker's is forgotten when the worker is removed.
- **Static anchors are there at once**, so their clusters stand as the space opens.
- **Dynamic anchors come back with the side.** A worker kept at an anchor not yet placed again waits at home, for up to **60 s** after the side starts, and goes there as the anchor appears; after that its placement is forgotten, silently, since the place may simply be gone. A panel kept at one floats where it last stood meanwhile, and joins the cluster as it appears.
- **Changing environment**, workers stand at the new environment's home, or where that environment's kept placements put them; coming back, those kept here are restored where their anchors exist.
- **The agent's own place** at an anchor is not kept, as following isn't: it starts at home.

### Participants carrying workers (#259)

#259 proposes that participants take workers with the grip, carry them and leave them where they like, as they do panels. That builds on this placement, not beside it: a worker let go near an anchor's cluster joins it, its slot outlined while carried as a wall's slot is; let go anywhere else, it stands where it was left, a placement at a pose. **A participant's placement wins over the agent's, as with panels:** nothing automatic moves a worker a participant placed, and `place_worker` moves one only when someone's words asked (section 6), the worker staying pinned where it goes, with a notice saying what moved. Who placed each worker is kept with its placement. Grip precedence over panels, walls and cards is #259's to settle, by `interaction.md`'s order.

## 5. What an environment declares

Gathered, since dense environments need all of it. Each is optional, bounded and carried as today's anchors are:

| Hook | What it gives | Default |
| --- | --- | --- |
| `with_agent` (exists) | The agent's home | 2.3 m ahead of the arrival, 1.4 m up |
| `with_status_wall` (new; generalises `with_stage`) | Which wall is the status wall: one of the environment's own (`StatusWall::own`), or one it places and sizes (`StatusWall::at`) | Beneath the agent's home |
| `with_anchor` (new; generalises `with_workers`) | Named cluster anchors, each a pose, an extent, a kind and a label, `home` among them for where workers stand by default | Today's arc about home, and no others |
| `Space::place_anchor`, `Space::remove_anchor` (new, for a server side) | Anchors that come and go with the environment's state, within `ANCHOR_LIMIT` and its rate | None |
| `with_solids` (exists) | Boxes the agent, its path, the status wall, clusters and beside-panels keep clear of | None |
| `with_open_space` (new) | Boxes the agent may stand in when following, such as aisles and the yard. Outside them it doesn't stand | Anywhere clear of solids within 20 m of home |
| `Object::at` (new, for a side's described objects) | Where an object a participant points at stands, so that the agent can stand off its line of sight, go to it when asked, or put a worker by it | None: the focus falls back to gaze |

Solids today are a handful of boxes, so dense environments need more of them: **raise the limit to 256 boxes**, checked in the server against the swarm's box and the path, which costs microseconds per move. A rack row is one box, not forty. A data centre whose racks change at run time, as its topology does, publishes them from its **server side** with `Space::occupy(boxes)`, bounded as `describe` is, replacing its static solids for that binding. That is a later step, listed in stage 6.

## 6. MCP controls

### `move_agent`, extended

`move_agent`'s `place` gains forms, and keeps those it has:

- `home`: home, as today. It also ends a pin and stops following for this conversation, until someone speaks again.
- `near NAME`, `beside NAME`: to that person's **conversation spot**, as following would choose it, rather than 2.3 m in front. `front NAME` keeps today's 2.3 m in front of them, facing them. This is what "come over here" calls in the **Only when asked** setting, where following doesn't bring it.
- `stay`: pin where it stands now, as **Call Tomori here** does: no following and no return home, until released. This is what "stay here" calls.
- `follow`: end a pin and follow the speaker again. It is refused, saying so, where the space's setting is Only when asked or Stay home, since that is an owner's choice.
- `object ID`: to stand beside an object a side describes with a position (`Object::at`), such as `datacentre:rack/a12`, at the conversation distance from whoever asked, off their line of sight to it. It is refused for an object without a position.
- `anchor NAME`: to that anchor's cluster, at its point if free, else its next free slot, as any member arrives (section 4), facing the anchor's audience. The mode is unchanged: a following agent may be moved on by the next words that need it, and `stay` pins it there.

Its result says where the agent is, its **mode** (`home`, `following`, `pinned`), the anchor it stands at, if any, and, while it follows, when it will go home if nothing more is said. `presence` reports the same under `agent`, with the conversation partner's name.

### `place_worker`, new

**Recommendation: a new tool, `place_worker`, rather than a `move_agent` form taking a worker's session.** A worker is another subject with other rules: `stay`, `follow` and the conversation spot mean nothing for it, its limits differ, and a participant's placement of it must be respected as a panel's is. It reads as `move_panel` does, which the agent already knows.

`place_worker`, with a `worker`, its session identifier or title as `workers` lists it, of at most 1024 bytes, which must name one of this space's workers, and a `place`, of at most 1024 bytes:

- `anchor NAME`: in that anchor's cluster;
- `object ID`: in the cluster of the anchor that object stands by, or else in a small cluster of its own, 1.5 m wide, 2 m tall and 1 m deep, 1 m before the object towards whoever asked, from its `Object::at`;
- `wall WALL`: just beyond the end of that wall nearer whoever asked, at its slots' height, facing as it faces, in a small cluster of the same size, so that a worker stands by the chart it is reading;
- `near NAME` or `beside NAME`: 1.5 m to that person's right, as the agent stands to their left, placed from their head once and never chasing it;
- `home`: back home, forgetting its placement.

It says where the worker stands now, at which anchor, and asks the agent to say where it put it. It refuses, saying why: a worker not this space's, as `remove_workers` checks; a finished worker, which is removed rather than placed; one a participant is carrying; an anchor, object, wall or person not here, naming the anchors nearest in name; and one a participant placed, unless `requested_by` names whoever's words asked, checked as for the gated tools, when it moves, staying pinned, with a "Moved" notice on the status wall, as for panels. A placement is logged at info, as `The agent of space ID placed worker SESSION at rack/a12.`. `workers` and `presence` give each worker's placement and who made it.

### Panels at anchors

`place: anchor NAME`, in a panel block's header, a code block's `place=` and every tool that takes a `place`, `move_panel`, `update_panel` and `show_image`, floats the panel in that anchor's cluster, facing along the anchor's +Z, and keeps it a member, so that it glides with the anchor if the environment moves it. A participant pulling it off takes it out of the cluster. An anchor not here is ignored, as a wall not here is, and the panel goes where it would by default.

### How the agent decides

`tomori-tools` would say, in a few sentences: you move to whoever you are answering on your own, so don't call `move_agent` for that. Call it when someone asks you to stay, to go home, to follow again or to come to someone, or when someone asks you to show them something an object stands by ("take me to the overloaded server"). Say where you are going. Don't move yourself to fill silence, to look busy or between turns of the same conversation.

And, for workers: when you start a worker on something that has a place in the space, such as a rack, an area or a console, put it there with `place_worker`, at an anchor `presence` lists or by the object's id. When nothing relevant has a place, leave it at home. Place each worker once, as it starts, and move it again only when its work moves or someone asks. Never move a worker a participant placed unless they ask. Say where you put it.

### Limits

- **The agent's own moves**, those not asked for in the words it is answering (which the server tells from `requested_by`, as for its other gated tools), are limited to **one every 20 s** and **six in ten minutes**. Beyond that they are refused, saying so, with the agent told to stay where it is.
- **Placing workers, so that they don't drift**: the agent places each worker at most **once a minute** and **three times in ten minutes**, and the space's workers **twelve times a minute** together. A worker's first placement, as it starts, never counts against its own limit. Beyond that, `place_worker` is refused, saying to leave the worker where it is. Placements asked for, with `requested_by`, are limited only as a participant's carrying is. A worker moved because its anchor moved or went is the environment's doing, and counts against nothing.
- **Moves asked for** are limited only as a participant's own controls are.
- **A participant always wins:** **Call Tomori here** or **Send Tomori home** after an agent's `stay` or `follow` overrides it at once, and a worker a participant placed stays theirs.
- **Every move is logged** at info with its reason (follow, call, tool, home, anchor), which stage 6 tunes from.

### Participants' controls

The hand menu's Space section keeps **Send Tomori home**, shown while it is away, and gains **Call Tomori here**, beside it (section 1), and **Tomori: stay here** / **Tomori: follow again**, the same as `stay` and `follow`. An owner sets **Agent movement** and **Return home after** in the space settings. Whether someone can say "don't come to me" for themselves alone is an open question.

## 7. Spatial audio

- **Nearer and clearer.** At 1.5 m the agent's voice is a few decibels louder for the speaker than at today's 2.3 m (by an inverse-distance law, about 3.7 dB, or 2.5 dB above the unity it has at 2 m), well within the existing cap of 12 dB above unity, and clearly placed off their left shoulder. Its direction now tells them, and everyone else, who it is talking with.
- **Others hear the conversation from where it is.** Someone across the room hears the agent at the distance law's floor, never more than 6 dB below unity, from the direction of the conversation, which is what a person there would hear. No change to the law is needed. If it proves too quiet for onlookers, the agent's floor could be raised to 4 dB.
- **Gliding while speaking.** The voice moves with the swarm's voice points, and the panner's 30 ms smoothing already takes continuous motion without clicks. There is no Doppler, which would be wrong for a voice. Streaming over a long distance moves the voice at up to 6 m/s for about 2 s, which smoothing takes too, though it should be heard on the headset (stage 1's checks).
- **Held speech is unchanged.** A reply that starts while the speaker records is held for them and released 600 ms after X is let go. The agent is usually still gliding then, and settles within a second or two, while the reply already plays from it. Waiting for the agent to arrive before releasing held speech is not recommended: arrival is not guaranteed (a blocked path, a far partner), and the pause would grow unpredictable.
- **Echo.** A nearer, louder agent puts a little more of its voice into an open live microphone. Push-to-talk silences playback while recording, so only live-microphone users are affected, and `spatial-audio.md`'s echo measurement covers them.

## 8. Interaction with existing features

- **Placement (#269).** The looked-at wall and float-beside are unchanged. The agent's spot is chosen first and counts as taken for the beside fan, and the agent avoids the line of sight to the looked-at wall, so the two complement each other: the answer is where you look, the agent just to your left. Clusters at anchors count as taken too.
- **The speaking pill.** Unchanged. It already follows playback, not the agent's position. The pill could say `coming over` while the agent glides to you, but the turn on press says it better, so no.
- **Pointing at workers.** Workers at home are pointed at there, and placed workers where they stand. Signs and picking grow with distance, so a worker across the yard is pointed at as one 3 m away, and what someone points at names its anchor. The satellites aren't pointed at.
- **Summoning (#231).** The trigger's summons is removed (section 1): summoning is by voice, `move_agent stay` and **Call Tomori here**. The 2.3 m and 1.4 m constraints that came from the entourage go. The trigger's swarm rung leaves `interaction.md`'s order, freeing the trigger for pointing and selecting, whose uses are an open question.
- **The entourage (#261).** Retired: the wall doesn't travel and the workers don't follow. `SpaceState::summoner` stays in the protocol for older clients but no longer gathers workers in newer ones.
- **Workers carried (#259).** Builds on worker placement (section 4): a participant's placement is a placement at an anchor or a pose, and wins over the agent's.
- **Locked walls (#262).** The status wall takes the lock as the Tomori wall does, unlocked by default; an environment's own wall in the role, as the farmyard's board, keeps its own lock, locked by default. A lock never keeps notices off a wall. Following never moves a wall.
- **Live chart panels (#267) and environment walls.** A console wall is a wall the agent keeps clear of and out of the line of sight to. Where an environment's own wall is the status wall, its published panels come before Tomori's notices for its slots (section 2).
- **Environments' server sides (#275).** Gain `place_anchor` and `remove_anchor`, with `ANCHOR_LIMIT`, `ANCHOR_NAME_BYTES` and `ANCHOR_LABEL_LIMIT` exported and enforced, and their rate limited as their panels' is.
- **Restart and change of environment.** The agent starts at home, as today. Following is never kept. Worker placements and panels at anchors are kept by anchor name, for each environment (section 4).
- **The spectator** follows a participant and sees the agent and clusters where they stand. It lays clusters out by the shared rule, and needs nothing else.
- **Older clients.** They glide the agent to `summoned` as today, so they see it follow. Without knowing that workers stay home, they still gather workers about `summoner` while the agent is away. The server could leave `summoner` out for them, which today means "about whoever the agent faces, 2.3 m before it", no better. They ignore anchors and keep placed workers at home, and send no summons, since the server no longer says it keeps them. **Recommendation:** accept these as known limits for older clients, which update with the app.

## 9. A staged plan

The maintainer approved this proposal, with its amendments, to be implemented in these stages:

1. **Follow the speaker, and go home** behind the **Agent movement** setting, default **Only when asked**. The server chooses the conversation spot, with the turn on press, the 0.4 s commit, hysteresis, huddles, the queue rule and the return delay, from existing presence, attention, walls, panels and solids. Clients glide to `summoned` as today, adding the `via` waypoint and streaming. **The trigger's summons is removed**: the server stops keeping summons, the client's swarm rung goes, the hand menu gains **Call Tomori here**, and `move_agent stay` pins. In **Follow** mode the Tomori wall stays home and workers stay at home. Moves are logged with their reasons. Tests: the spot's distance, bearing, height and clearance, refusal within 12° of the line of sight, hysteresis and the 10 s dwell, the queue rule, the timer's conditions and cancellation, visitors, **Call Tomori here** pinning, and an older client's `PlaceAgent` dropped.
2. **The status wall.** `with_status_wall`, in both forms, with `with_stage` as its compatible form; the farmyard's notice board as its status wall, with status panels and the board's own sharing its slots, the full-wall order and the migration of kept panels; the rename and aliases, the retired travel, the sightline tests, the instructions, and the swarm drawing in up close. The entourage stops travelling in every mode.
3. **Workers at home, as the home anchor's cluster.** The shared cluster rule, `tomori_protocol::cluster`, with its arc, grid and compact steps and the "+N more" sign; `with_anchor` for `home`, with `with_workers` as its compatible form; the fit test; and the satellites.
4. **Cluster anchors and placing workers.** Static anchors with `with_anchor`; then dynamic ones, `Space::place_anchor` and `remove_anchor`, with their limits exported from `tomori-environment` and enforced; anchors in the space state and `presence`; `place_worker`, `place: anchor NAME` and `move_agent anchor NAME`; removal, its notice, and placements kept by name with the 60 s grace for dynamic anchors; the rate limits and the instructions. Tests: the cluster's slots for one to many members and their stability as members come and go, skipped slots against solids, name collisions, a side over its limits, removal sending workers home and leaving panels, a restart restoring placements, and a participant's placement refusing the agent's. It needs only stage 3, so it can follow it directly, and it is where #259's carrying would begin.
5. **The agent's own MCP controls.** `stay`'s instructions, `follow`, `object ID`, the conversation spot for `near` and `beside`, the mode in `presence`, the rate limits, the hand menu's stay and follow, and `Object::at`.
6. **Tuning in dense environments.** The data-centre environment declares solids, open space, a status wall and a home anchor, and places an anchor for each rack that alerts. Then: raise the solids limit, add `Space::occupy` for run-time geometry, tune the distances, the dwell, the return delay and the worker limits from the logs and the headset, and decide whether **Follow the speaker** becomes the default.

Stages 1 and 2 can swap if the data centre's clutter is the pressing problem, since the status wall alone frees the agent's surroundings.

## Risks

- **Motion as distraction.** An agent that moves at every question becomes the thing everyone watches. The 0.4 s commit, the 3 m and 50° "near enough" rule and the 10 s dwell are there to make it move rarely. The logs will show how often it does.
- **Motion comfort.** A large, close, moving object can unsettle some people even when they aren't moved themselves. The 0.8 m clearance, the ease, the 0.85 m/s cap and streaming for long distances are meant to help, and the headset is the judge.
- **Multi-user etiquette.** One person dominating the agent's position, or the agent leaving someone mid-thought for another, is the risk of any shared assistant. Queue order, huddles and the pin are the answers proposed. An owner's **Only when asked** setting remains.
- **Losing the "home base" feel.** The maintainer's daily pattern, the agent ahead with workers either side, partly goes away when the agent walks off. Workers staying put and the agent returning after 90 s are what keep it.
- **Losing a gesture.** People who learned the trigger's summons lose it. Voice is the natural replacement, and **Call Tomori here** covers speech being off or the agent busy, but it is two steps through a menu rather than one pull.
- **Wrong focus.** Gaze from the head is coarse, and a pointed environment object has no position until sides give one, so the agent may stand where something is. The 12° line-of-sight test uses whatever focus is known, and a participant can always call it with **Call Tomori here** or ask it to move.
- **Dense scenes without geometry.** An environment that declares no solids gets an agent that can stand inside a rack. Stage 6's test, that the arrival's region is covered by solids or open space, is what catches it.
- **Clutter from placed workers.** Workers spread across a dense scene, each with a metre-wide sign, could bring back the clutter this proposal set out to remove, now everywhere at once. The instruction to place only where there is a relevant anchor, the limits, compact footprints when a cluster fills and the 64 anchors at most bound it; whether placed workers' signs should shrink or show only when looked at is for stage 6.
- **Pointing at distant workers, and losing track of them.** A worker by a rack 15 m away, behind other racks, is hard to point at or even see from home. Picking grows with distance, but not through racks. `presence` and `workers` name each worker's place, so "how is the A12 worker doing?" works by voice, the satellites count placed workers too, and a worker's finishing notice on the status wall says where it is.
- **Anchors that flap.** A side whose alerts flap could move clusters about, and its workers home. The rate limit, workers not returning on their own, and the advice to debounce are the answers; the logs will show whether they are enough.
- **Notices out of the way.** The farmyard's board is not readable from the arrival, so a notice there may go unread for a while. What matters is said aloud, and a quiet cue is an open question.

## Decisions

The maintainer approved the proposal with these amendments, November 2026:

1. Following, as modelled: on press it turns; after 0.4 s it commits from what was captured as the words began; it moves only beyond 3 m and 50° or when in the way; it stays at least 10 s; in a queue, it moves to whoever it answers. **Approved.**
2. Where it stands: 1.5 m, 45° to the speaker's left of their focus, eye height plus 0.1 m, facing them, left because float-beside goes right first. **Approved.**
3. Return home after 90 s idle, as a space setting from 30 s to never, with a running worker not holding it. **Approved.**
4. The trigger's summons removed: summoning by voice, `move_agent stay` to pin, and **Call Tomori here** in the hand menu as the fallback without speech; the trigger freed for pointing and selecting. **Approved**, as the maintainer's amendment.
5. The Tomori wall becomes the space's status wall, never travelling, given by `with_status_wall` as one of the environment's own walls or one it places, falling back to beneath the agent's home; in the farmyard, its notice board. **Approved.**
6. Workers stay at home unless placed, home being the `home` anchor's cluster, compacted where needed, with satellites on the agent while away. **Approved.**
7. Cluster anchors, static and from a server side, within exported limits, with one cluster rule for workers, panels and the agent; workers going home and panels staying floating when an anchor goes; placements kept by name. **Approved.**
8. `place_worker` for workers, `place: anchor NAME` for panels, and `move_agent` gaining `stay`, `follow`, `object ID` and `anchor NAME`, with the agent's own moves and placements rate-limited. **Approved.**
9. Stage 1 behind a setting, default Only when asked, before any default changes. **Approved.**

## Open questions

Left to the stages that meet them:

- Should the default board be labelled `STATUS`, or keep `TOMORI` as the space's own wall?
- Should the agent go to whoever asked for a worker when that worker reports, or report from wherever it is? (Recommended: wherever it is, with the notice on the status wall.)
- Should someone be able to say "don't come to me" for themselves, as a device setting, for example while presenting?
- Should the swarm draw in up close (recommended), or should the conversation distance grow to suit its full size?
- Left or right by default, or the side with more open space each time?
- For the data centre: does its scene have aisles it can declare as open space, a natural place for a status strip, and room for a home anchor?
- Should a pin from **Call Tomori here** survive the agent's own `follow`, or should only a participant release it?
- What the freed trigger does: selecting environment objects beyond those marked selectable, selecting a worker to see what it is doing, selecting an anchor's cluster, and selecting list items; and how each relates to A, which already chooses and ticks.
- Should a status wall out of reading distance, as the farmyard's board, give a quiet cue when a notice arrives?
- Should the agent place a worker automatically when the words that started it pointed at an object with an anchor, or always decide itself (recommended)?
- Should a worker sent home by a removed anchor return if the anchor comes back soon? (Recommended: no, to avoid drift.)
- Do large environments need anchors beyond 20 m of home, as a big data hall might?
- Should anchors have a tool of their own, if `presence` grows too long with 64 of them?

## Design changes this would make

- `design/environments.md`: the agent's home and its return; following and the pin replacing "it never moves of its own accord"; the trigger's summons removed; `with_status_wall` in both forms, `with_anchor`, `Space::place_anchor` and `remove_anchor` with their limits, `with_open_space`, `Object::at` and the solids' limit; the cluster rule; workers at home and placed; the entourage retired; the rationale.
- `design/projection.md`: the Tomori wall as the status wall, its home and grid, an environment's own wall in the role, status panels sharing its slots and the full-wall order; no travel; its names in `place:`, and `place: anchor NAME`; the agent's spot and clusters taken in the beside fan.
- `design/interaction.md`: the turn on press; the trigger's summons and swarm rung removed; workers at home and placed; the satellites.
- `design/mcp.md`: `move_agent`'s new forms and limits; `place_worker`; `presence`'s agent mode, anchors and worker placements; `tomori-tools`.
- `design/spatial-audio.md`: a note on the agent's nearer voice and on movement while speaking.
- `design/hand-menu.md`: **Call Tomori here**, stay and follow; the space settings for agent movement and the return delay.
- `design/avatars.md`: the avatar told its distance to whoever it faces; the swarm drawing in.
- `design/farmyard.md`: its notice board as its status wall, and whether it declares a home anchor and open space, or keeps today's defaults.
