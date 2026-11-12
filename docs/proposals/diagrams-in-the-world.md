# Proposal: Diagrams in the World

Status: **proposed**, November 2026, for the maintainer's review. Nothing here is implemented, and nothing of it is a requirement until a stage brings it into the design documents it changes (listed at the end), which take precedence. It was written against `origin/main` at `350239cb` (#377), with stage 1 of the three-dimensional diagrams (#386, `diagrams-stage-1`) read as it stands to be merged.

It follows the three-dimensional diagrams proposal (`docs/proposals/three-dimensional-diagrams.md`, approved: a volume is a panel with depth, a Mermaid-like `diagram` block and the `show_diagram` and `update_diagram` tools, placed as panels are, nodes dragged by hand in a later stage) and its first stage (`design/diagrams.md` on #386: the block's syntax, `id[Label]`, shapes by brackets, `@{ colour, size, detail, ref }`, `subgraph … end`, edges `-->`, `==>`, `-.->` and `~~>` with `: label`, server layout, layered or force, and pointing that names nodes and edges). It builds on what the world already names: environments' objects and their references, `inspect` and `Object::at` (`design/environments.md`); cluster anchors, declared and placed, and `presence` (`design/environments.md`, `design/mcp.md`); walls and panels by identity, id or title (`design/projection.md`); workers (`design/mcp.md`); participants' handles (`design/mcp.md`); environments' client sides and `pick` (`tomori-scene`'s `client_side.rs`); and the physics of loose objects, streamed from the server side and sending nothing at rest (`design/physics.md`).

## The request

Spoken by the maintainer from the headset, November 2026:

> As part of this layout language, is it worth exploring how you could connect it to existing anchors, panels or geometry in the world? A practical example: if we were talking about marshmallows, you'd show a flow chart of how to cook them, and step one, "find the fire", has an arrow that points to the fire; step two, "find the stick", points to one of the sticks. The server would be responsible for figuring out how to visualise that. It might be tricky, since we're incorporating arbitrary things into a diagram, but the real world is messy, and it's something we should explore.

And then:

> Hooke's law, springs, to make diagrams look natural in 3D space.

## Recommendation in brief

- **A diagram binds short names to things in the world, then uses them as ordinary edge ends.** One new line form, `fire := object farmyard:campsite/bonfire`, declares a **world reference**, of one of seven kinds: `object`, `anchor`, `wall`, `panel`, `worker`, `person` and `agent`, each named as the agent's tools already name it. After that, `step1 --> fire : here` is an edge like any other. Nothing else in the block changes, and the JSON tool gains a `world` list of the same.
- **The server resolves which thing; each client tracks where it is, frame by frame.** References are resolved on the server, against the tables the agent's tools already read, within the space and its current environment only, and every unknown reference is reported in the same turn with a suggestion, as stage 1 reports its warnings. Clients follow what moves (a hen, a stick in someone's hand, a carried panel, a person) from what they already draw, and fall back to the server's positions where they cannot.
- **A world edge is a leader line**, curved out of the volume from its node to the thing, ending in a mark on the target (a ring on the ground, a halo, a panel's frame lit, the object itself outlined where its environment can draw that), labelled at the target's end. It fades with distance and dims unless its step is in focus, and a target out of view gets a pointer at the edge of view. Later stages route it around heads and panels.
- **Springs make it look natural.** Stage 1's force layout is already springs, solved once: stress majorisation is Hooke's law between every pair of nodes. World references add springs from each referring node towards its target, weaker for far targets; the volume gets a spring to its placement; edges get rest lengths by importance; heads and panels push back. Run live on the server, damped, at a low rate, the springs let a diagram drift gently as its targets move or someone drags a node, then settle and sleep. Poses are streamed as physics streams bodies, and nothing is sent at rest.
- **Placement near what it points at:** `place: near references` stands the volume between its asker and the things it refers to, or beside the things if they are far off.
- **Interaction reuses diagrams' own:** pointing at a world edge names both ends; the trigger's focus on a step lights its targets; the agent's highlights while speaking light a step's targets with it.
- **Callouts are the same thing without a diagram**, and are worth their own small feature: a `world:` link in what the agent says points at the thing while that sentence plays, and a callout panel pins a label on a thing until it is dismissed.
- **Staged:** (1) references to anchors and objects (walls and panels too, since they cost nothing more), resolved when shown, with static leader lines and the springs used once at layout; (2) live tracking of moving targets, people, workers and panels, highlights drawn on the targets themselves, and the springs run live; (3) callouts without a diagram; (4) routing around heads and panels, edge-of-view pointers and the rest of the clutter rules.

## What exists today

| Thing | How the agent names it today | Where it is known | Moves? |
| --- | --- | --- | --- |
| An environment's object | its id, `ENV:REFERENCE`, as `inspect` takes it and pointing tells it: `farmyard:campsite/bonfire`, `farmyard:campsite/stick-0` to `stick-3`, `farmyard:hen/biscuit` | the server side describes it about once a second (the farmyard's `DESCRIBE`), with `Object::at` where it says, carried into the space; a client's client side draws it and `pick`s it by its **identity**, which the agent is never told | hens wander; sticks are carried, thrown and float; a held stick has no `at`, only its holder's hand |
| A cluster anchor | its name in `presence`: `home`, `campsite`, `pool-lookout`, `station-1` | `SpaceState::anchors.clusters`, pose and extent, on every client | declared anchors are fixed; placed ones move as the side places them |
| A wall | its identity or name, as `set_wall` takes | the shared slot layout, `tomori_protocol` | when someone moves or edits it |
| A panel | its id, identity or title, as `panels` and `move_panel` take | the projection, on every client | when carried, docked or moved |
| A worker | its title or session, as the worker tools take | `SpaceState::clusters` and each client's placement | as it is seated or placed |
| A participant | their handle, `Samuel#11`, as `presence` and `beside NAME` take | presence, as each client last reported; others' clients draw them | always |
| The agent | itself | `SpaceState`, the swarm or avatar | when it follows or is sent |

What none of it has is a way to draw a connection from something the agent shows to one of these things. The nearest is a node's `ref`, which stage 1 keeps as an opaque string the agent is told and Tomori never follows. That is deliberately left as it is (section 1).

## 1. The syntax

### A world reference is a binding, not a node

```text
fire := object farmyard:campsite/bonfire [The bonfire]
```

A line of the form `ID := KIND NAME`, optionally followed by `[Label]` and `@{ … }`, declares a world reference: a short id, in the same namespace and with the same rule as a node's id, bound to a thing of the given kind and name. After that, the id is used as an edge's end, either end, exactly as a node's is, and the rest of the block is unchanged.

Why this form, rather than the alternatives considered:

- **Not a node with a special shape.** A world thing is not drawn in the volume and has no place in its layout, and every bracket a model might reach for is already a shape. A reference written as `fire>>bonfire<<` would also be easy to mistake for a typo of an existing shape, and would be read by stage 1's parser as a box called `fire`, silently.
- **Not the `ref:` attribute.** Stage 1's `ref` is the agent's own pointer that Tomori never follows, and keeping it so keeps one rule simple: Tomori acts on `:=` lines and on nothing else. A node that both stands for a crate and points at a stick keeps its `ref` and gets an edge.
- **A binding is written once and used by a short name.** A long id like `farmyard:campsite/stick-2` is written once, so a typo is fixed in one place and reported once, and the edges stay as short as the agent's other edges, which is what keeps the agent writing them correctly.
- **Mermaid has no `:=`**, so no Mermaid line the agent writes out of habit can read as a reference, and stage 1's parser already skips such a line and names it. An older server shows the diagram without its world edges, with a warning naming the line, and nothing breaks.
- **The kind is explicit**, because names collide across kinds: `campsite` is an anchor, and a panel could be titled "Campsite". To be lenient by the line, as stage 1 is, a reference without a kind is still taken: a name with a `:` is an object, and any other name is tried as an anchor, a wall, a panel and a person, in that order, with a warning saying which it took.

| Kind | Name | Example |
| --- | --- | --- |
| `object` | an object's id, as `inspect` takes it | `stick := object farmyard:campsite/stick-2` |
| `anchor` | a cluster anchor's name, as `presence` lists it | `lookout := anchor pool-lookout` |
| `wall` | a wall's identity or name | `board := wall notice-board` |
| `panel` | a panel's id, identity or title | `notes := panel trial-checklist` |
| `worker` | a worker's title or session | `builder := worker "Build the client"` |
| `person` | a participant's handle or, where unique, name | `samuel := person Samuel#11` |
| `agent` | nothing: the agent itself | `me := agent` |

A name runs to the end of the line, or to ` [` or ` @{`. One holding either of those, or starting with a quote, is written in double quotes. A label is what the target's end of the line is labelled with when the edge has no label of its own; without one, the server's short name for the thing is used: an object's description cut at its first comma, an anchor's label, a panel's title, a person's name.

A reference's attributes are few: `mark`, which is `ring`, `halo`, `outline` or `none` and otherwise chosen by its kind (section 3), and `colour`, the chart palette's, and otherwise the edge's. Unknown ones are a warning, as stage 1's are.

**World edges.** An edge with a world reference at one end is a **world edge**. It takes every style and direction an edge takes: `-->` points at the thing, `<--` from it, `~~>` flows towards it, `-.->` is a tentative "maybe this one". An edge between two world references, `stick --> fire : over this`, is drawn in the world alone, between the two things, and is left for stage 4, since it is the case most likely to clutter. A world reference used by no edge is a warning, and is drawn as nothing.

**In the JSON tool** the same is a list beside `nodes`:

```json
"world": [
  { "id": "fire", "kind": "object", "name": "farmyard:campsite/bonfire", "label": "The bonfire" },
  { "id": "samuel", "kind": "person", "name": "Samuel#11" }
]
```

and edges name `fire` in `from` or `to`, as any node. `update_diagram` adds, changes and removes world references by id, as it does nodes.

### The marshmallow example, in full

````text
```diagram
title: Toasting a marshmallow
id: marshmallow
layout: layered down
place: near references
---
fire := object farmyard:campsite/bonfire [The bonfire]
stick := object farmyard:campsite/stick-2 [A marshmallow stick]
seat := anchor pool-lookout [The lookout's seat]
samuel := person Samuel#11

find_fire[1. Find the fire]@{ colour: amber, detail: "At the campsite, beside the pool lookout" }
take_stick[2. Take a stick]@{ detail: "Any of the four leaning on the far log; grip to take it" }
hold[3. Hold it over the embers]@{ detail: "A hand's breadth above the embers, not in the flames" }
turn[4. Turn it slowly]@{ detail: "About 16 s to golden at the hottest; black soon after" }
eat((5. Let it cool and eat))@{ colour: green }
sit[Sit while it cools]@{ size: 1 }

find_fire --> take_stick --> hold --> turn --> eat
eat -.-> sit
find_fire --> fire : here
take_stick --> stick : this one
hold ~~> fire : over the embers
sit -.-> seat
eat --> samuel : your turn
```
````

Laid out downwards, the five steps stand in a column, with the aside to sit beside the last. Placed `near references`, the volume stands a little to one side of the line from Samuel to the campsite (section 4), so that looking at the steps and at the fire is one turn of the head. From step 1 a leader curves down out of the volume's side to the bonfire, ending in an amber ring about the fire pit labelled "here". From step 2 one curves to stick 2, wherever it is, leaning on its log or in someone's hand, ending in a halo about it, and, in stage 2, the stick itself glows. Step 3's flow runs beads towards the fire. The aside points at the lookout's seat with a dashed line and a ring on the ground. The last step's line ends in a halo above Samuel's head, labelled "your turn", which Samuel's own client draws as a short line coming towards them that ends before their eyes. Read by an older client, the outline adds a line for each world edge: `1. Find the fire → The bonfire (farmyard:campsite/bonfire): here`.

## 2. Resolution on the server

**What resolves, and against what.** As a diagram is shown or changed, the server resolves each reference against the tables the agent's own tools read, in the agent's space only, as `inspect` and `presence` are space tools: objects among those its current environment's server side describes, by reference; anchors among `presence`'s; walls and panels as `set_wall` and `panels` find them, in the current environment; workers among the space's; people among the participants in the space reporting presence; and the agent itself. A reference to another environment's object, as `sky:…` in the farmyard, is unknown, as are panels kept for an environment the space is not in.

**What the agent is told, in the same turn.** `show_diagram` and `update_diagram` answer with each reference's resolution, so that the agent knows what it pointed at before it says so:

```text
world:
  fire → the campsite's bonfire (farmyard:campsite/bonfire), at the campsite, 3.1 m from Samuel
  stick → a marshmallow on a stick, golden, held by someone (farmyard:campsite/stick-2)
  seat → anchor pool-lookout "The pool lookout" (viewpoint), 2.4 m from Samuel
  samuel → Samuel (Samuel#11)
warnings:
  line 9: "stik" is not a reference or a node; made a box called stik (did you mean stick?)
```

An unknown reference is a warning with a suggestion, as stage 1's node warnings are: within two edits among things of that kind in the space, `no object farmyard:campsite/stick-7 in the farmyard (there are stick-0 to stick-3)`, or, beyond a few, a pointer to the tool that lists them, `see presence for anchors`. A block's warnings reach the agent before the next words spoken in the space, as stage 1's do. The wording never says whether a thing exists elsewhere: "no person Ada in this space" alike whether Ada is in another space or nowhere.

**Kept by name, resolved again as the world changes.** A reference is kept as written, not as what it resolved to, so a diagram outlives the things it points at changing. Each is resolved again as the tables it was resolved against change, at most once a second for each diagram. A target can be in one of three states:

- **here**: drawn;
- **absent**: a kind of thing that comes and goes and is not here now, such as a participant away or not yet arrived, an anchor the side has not placed yet, or an object described without a position. The line is drawn as a short stub from its node, faint, ending in a small open ring, and the label says why: "Samuel is away";
- **unknown**: never resolved. It is drawn as nothing. A reference that is still unknown 60 s after it was written, the grace placements wait for their anchors (`places::GRACE`), is noted once more to the agent and then left alone.

A thing that resolves later, as an anchor the side places after the space opens or a participant who rejoins, is connected then.

**Privacy and what the agent may not see.** A reference names only what the agent's tools already show it, so a diagram tells the agent nothing it could not learn, and tells participants only what they already see: where people stand is in presence for the agent and drawn as avatars for everyone. Some rules keep it so:

- **Only participants**, as presence is: visitors and the spectator report no presence and cannot be referred to.
- **Never stale.** A participant whose presence is 5 seconds old or more (`PRESENCE_STALE`), or who is away, is absent, never drawn at where they were last seen.
- **Within the space.** Nothing resolves across spaces, and the agent of one space cannot draw a line to anything in another.
- **People may opt out.** A participant may remove the world edges pointing at themselves: pressing B on the halo above them removes them, for everyone, with a notice to the agent. A setting to refuse being pointed at altogether is an open question.

**Limits.** At most 16 world references in a diagram, 24 world edges, and 8 edges to any one reference; 64 world edges across a space's volumes and callouts together, beyond which the least recently shown diagram's world edges are drawn faint and not tracked. Resolving is a lookup of each reference in tables the hub already holds, a few dozen lookups for each diagram, done under the hub's lock as stage 1's layout is.

**On the wire.** `tomori_protocol::world::Reference`, non-exhaustive, is a kind and a name, cleaned and bounded as a node's ref is, at 128 bytes. A laid diagram gains `world`, a list of targets, each with its reference, its label, its state and, where it is here, a `Track` saying how a client follows it, and an edge's end may name a target by index:

| `Track` | A client follows it by | Fallback |
| --- | --- | --- |
| `Object { identity }` | asking its environment's client side where it draws that identity (below) | the server's position, eased |
| `Anchor { name }` | the anchor in `SpaceState` | none needed |
| `Wall { identity }`, `Panel { identity }` | its own layout of walls and panels | none needed |
| `Worker { session }` | where it draws the worker | its seat |
| `Participant { id }` | where it draws that participant's head and body | presence |
| `Agent` | where it draws the agent | `SpaceState` |

So **the server decides which thing, and the client decides where it is this frame.** A held stick is drawn in its holder's hand by every client already, and a line ending where that client draws it never lags. A client side learns of the identity its client already uses for `pick`, never the reference, so nothing new reaches a client that the agent's tools hide.

`tomori-scene`'s `ClientSide` gains, with defaults that do nothing:

- `fn locate(&self, identity: &str) -> Option<Located>`: where it draws that object this frame, in the environment's own coordinates, its middle and radius, the way `pick` reports a distance;
- `Input::emphasised`: the identities to emphasise this frame and how strongly, which a client side may draw as it likes (section 3).

The server's positions for what a client side cannot locate are sent as live data, at the volume's data seq, as stage 2's highlights are, at most twice a second, and only for targets that moved more than 5 cm: `diagram_world { panel, seq, targets }`. Clients ease between them over half a second. An object has no size today. `Object::with_extent(radius)` lets a side say one, 0.05 to 3 m, and without it a target is taken as 0.3 m across.

`Hello::world_edges` says a client draws them. A client that does not say so is sent the diagram without its world references and edges, and its outline says them in text, so nothing older breaks.

## 3. Drawing the connection

**A leader line.** A world edge is drawn as stage 1 draws an edge, a thin tube through its waypoints, with its style (dashes, dots, a flow's beads moving towards the arrowhead) and its arrowhead, but in the space rather than in the volume. It leaves its node through the face of the volume's box nearest the target, at a port on that face (section 5 makes the port natural rather than arbitrary), and curves to the target as a cubic: its first tangent straight out of that face, its last coming down onto the target from above at about 30°, so that a line to something on the ground lands on it like a finger pointing, and one to a panel arrives at its frame from in front. Its width keeps its angular size, at least 1.5 mm at a metre and growing with distance as picking does, so a line to a fire 8 m off is still seen. Like the volume's own solids it is opaque and writes depth, so a hand or a panel in front of it covers it.

**The mark on the target**, by its kind unless `mark` says otherwise:

- **An object:** a ring on the ground beneath it, of the object's extent, and a faint halo about it. In stage 2, where its environment's client side emphasises it, the object itself is outlined or lit: the farmyard would brighten the bonfire's flames, draw a stick's rod and marshmallow with a rim glow, and lift a hen's feathers into the edge's colour. That is the environment's own drawing, so it can be in the object's own style. Where it cannot, the halo is all.
- **An anchor:** a ring on the floor about its point, as wide as its extent's footprint up to 3 m, the way the agent's home ring is drawn.
- **A panel or a wall:** its frame lit as pointing lights it, the line ending at the frame's nearest edge, never across its face.
- **A worker:** its sign lit as pointing lights it.
- **A person:** a halo ring 25 cm above their head. Never on their face, and never a line ending at their eyes.
- **The agent:** its rings brightened.

**The label** stands beside the mark, facing the viewer, on a backing as the volume's labels are: the edge's label, or else the reference's, with the step's label before it in small type, "1. Find the fire · here", so that a target read on its own says why it is lit.

**Targets out of view or far away** (refined in stage 4; stage 1 draws the line and nothing more):

- **Behind you.** Where the target is outside the viewer's field of view and the edge is in focus or highlighted, a small chevron stands at the edge of view nearest it, in the edge's colour, with its label and distance, "The bonfire · 6 m". It slides round the border as the head turns, and goes once the target comes into view.
- **Far.** A line longer than 4 m is drawn solid for its first and last metre and fades to a quarter between, so it does not slash across the scene, and beyond 15 m it is the mark and the chevron alone.
- **Hidden.** A target behind a solid of the scenery, such as a hen behind the coop, has its mark drawn through what hides it, faintly, once, so that "it's there, behind" reads, and nothing else is drawn through anything.

**When things move.** A carried volume's ports move with it, and so do its lines, at no cost, since the line's ends come from the volume's matrix and the target's track each frame. A moving target pulls its end along (section 2), and the line's middle is recomputed from its two ends, so nothing has to be sent. A target that becomes absent shrinks its line back to its stub over half a second rather than vanishing.

**Clutter.** Seen alone, a volume's world edges are drawn at half strength and its targets' marks at a third. The edges of a node in focus, by a participant's trigger, for them alone, or highlighted by the agent, for everyone, are drawn at full strength with their marks and labels, and all others drop to a fifth. A volume with three world edges or fewer draws them all at full strength. Labels at targets show only at full strength. Two edges to one target merge into one line for their last metre, with both labels stacked, so a fire four steps refer to has one mark, not four. A client draws at most 32 world edges a frame (`Limits::leaders`), those in focus or highlighted first, then the nearest. Routing around people and panels is in section 5 and stage 4.

## 4. Placing the volume

Panels' places stay the default, by the maintainer's decision: onto the wall the asker is looking at if they are near enough, else floating beside them. A volume on a distant wall can still point at a fire. One new form joins `place`:

**`near references`.** Taking the middle of the targets that are here, weighted by the number of edges to each:

- if it stands within 6 m of the asker, the volume floats between them, on the asker's side, 1.3 m from their eyes, as a floating volume does, turned 25° off the line to the middle, towards the side with fewer targets, so that it never covers what it points at, its front facing the asker;
- further off, it stands by the targets, 1.5 m from their middle on the asker's side, at eye height, facing back along the way they would walk to it, so that a diagram about the campsite waits at the campsite;
- where the targets span more than 120° as seen from the asker, it falls back to `beside`, since no one place sees them all.

It is computed once, when the volume is placed, and is then where the volume's anchor spring holds it (section 5). A participant who moves the volume moves it, as with any panel. The agent's instructions recommend `near references` for a diagram whose steps are done in the world, and the default for one that only mentions things.

## 5. Springs

**Stage 1's force layout is already Hooke's law, solved once.** #386 lays out `force` by stress majorisation (`crates/server/src/diagram/force.rs`): every pair of nodes is joined by a spring whose rest length is their distance in the graph times a unit, and whose stiffness is one over that length squared, so near neighbours are held firmly and distant pairs loosely. Repulsion is not a separate force, as in Eades's or Fruchterman and Reingold's models; it comes from unjoined pairs having long rest lengths, with a separation pass afterwards so that no two nodes' bounds overlap. A node laid out again is held where it stood by one more spring, as strong as all its others together. What it is not is a simulation over time: it minimises the springs' energy in 200 fixed sweeps, each node moved straight to its best place given the others, with no velocity, damping or mass, and it runs only when the diagram changes. Layered layout has no springs at all.

**For world-connected diagrams, the springs are made explicit**, added to the same energy and solved the same way in stage 1:

- **A world spring from each node to each target it refers to.** The target is outside the volume, so the spring pulls the node towards the **port**: the point on the volume's box where the straight line from its middle to the target leaves it. Its stiffness falls with the target's distance, as `k / (1 + (d / 2 m)²)`, so a fire a metre away pulls step 1 firmly to the volume's near side, and a hen 15 m away barely turns it. Several targets average out, and a step that points at nothing feels none. This is what makes the leader lines short and uncrossed: step 1 sits on the fire's side of the volume, and step 2 on the sticks' side.
- **An anchor spring from the volume to its placement**, for a floating volume, so that what pushes it (below) moves it a little and never far: stiff, with a leash of 0.5 m beyond which it is held.
- **Edges by importance.** An edge's rest length is the unit divided by its weight, 1 to 3, and a thick edge (`==>`) is stiffer, so important connections stand closer and straighter. Within a layered layout these act only within a rank's plane and across its depth, so the ranks stay.
- **Repulsion from heads and panels.** Each participant's head is a soft sphere of 0.6 m, and each panel and wall slot a soft slab 0.2 m deep, which push on the volume's nodes, on the volume as a whole and on the middle control points of its leader lines, with a force that is zero beyond them and grows as Hooke's law does within. So a volume never settles into someone's face, and a leader to a fire bends over a head rather than through it. Repulsion uses the heads the server knows from presence; the viewer's own head is handled on their client alone, which fades a line within 0.3 m of their eyes, since that is the one head that moves faster than the server sees.

**Running the springs live.** Solved once, a diagram whose stick is picked up and carried off has a leader that simply stretches. Run live, its step drifts across the volume to follow. So, from stage 2, the server runs a world-connected or dragged volume's springs as a small damped simulation:

- **Owned by the server and deterministic.** Each node and the volume have a position and a velocity, stepped at a fixed 20 Hz by semi-implicit Euler in `f64`, nodes in the order written, from the same positions on every run, with the target positions the server knows (objects' as described, heads' from presence, anchors and panels as laid out). Clients run nothing. They draw poses they are sent, so everyone sees the same diagram, by construction, as stage 1's layout and the physics of loose objects promise.
- **Stable.** Damped just short of critically, so nothing rings; speeds capped at 0.3 m/s for a node and 0.15 m/s for the volume, so a hen running past makes a step glide, not lurch; and a target's movement is taken only beyond 10 cm, with hysteresis, so a hen pecking in place moves nothing.
- **Settling, then sleeping.** Once no node has moved more than a millimetre for a second, the volume sleeps: its rest positions are kept with the panel as stage 1's are, and nothing more is computed or sent. It wakes when a target moves beyond its threshold, the volume is carried or let go, a head comes within its repulsion, a node is dragged, or the diagram changes. A volume without world references, never dragged, never wakes, and is exactly stage 1's.
- **Streamed as physics streams bodies.** While a volume is awake, the server sends at most 10 times a second the nodes that moved more than a millimetre, each `[i16; 3]` in the volume's millimetres as stage 1 sends them, and the volume's pose when it moved, encoded as a body's placement is (`design/physics.md`), as live data at the volume's data seq, not a new version. At rest it sends nothing. Clients interpolate between the last two, a tenth of a second behind, as they do bodies.
- **Dragging, in diagrams' stage 3.** The hand dragging a node is its own spring, the stiffest: the dragger's client draws the node in their hand at once, as a holder's client draws a held stick, and sends where it is a few times a second; the server lets the rest settle around it and streams that to everyone; let go, the node is pinned where it was, as decided.

**The cost.** A step is the stress terms between every pair of nodes, at most 64 × 63, about 4,000 spring evaluations, plus the world springs, ports and repulsion, a few hundred: about 20 µs, and 0.4 ms a second for an awake volume at 20 Hz. At most 8 volumes in a space are awake at once, the rest held asleep until one sleeps, so a space's springs cost at most about 3 ms a second, run in the hub's sweep beside the zones. The wire costs at most about 4 KB a second for each awake volume, and nothing at rest, which is most of the time. The design risk is not the cost but the motion: a diagram that moves while being read is harder to read, which is why the speed caps, the thresholds and sleeping matter more than the springs themselves, and why the headset trial should judge them.

## 6. Interaction

- **Pointing at a world edge** with either ray, along its tube or at its target's mark or label, highlights it and names both ends, as the server describes them:

  ```text
  Samuel: [Pointing at world edge "1. Find the fire → The bonfire: here" in diagram "Toasting a marshmallow" (id marshmallow, floating near the campsite; target farmyard:campsite/bonfire, the campsite's bonfire, 1.2 m from Samuel)] is it hot enough yet?
  ```

  Pointing at the object itself still names the object as today, adding which diagram's steps point at it: `(step 1 of "Toasting a marshmallow" points here)`. The agent can then `inspect` the target by its id, as ever.
- **The trigger on a step** focuses it, for that participant alone, as in stage 1. Its world edges and their targets' marks and labels go to full strength, others dim, and a target out of view gets its edge-of-view chevron. Focus on a world edge's mark focuses the step it comes from.
- **The agent walks through the steps as it speaks.** Diagrams' stage 2 highlights nodes as the sentence that links them plays. A highlighted node's world edges and targets light with it, and the targets' environments emphasise them. So "first [find the fire](diagram:marshmallow#find_fire), then [take a stick](diagram:marshmallow#take_stick)" lights the fire as the first clause is heard and the stick as the second is. A link may also name a reference directly, `diagram:marshmallow#fire`, which lights the target and every edge to it.

## 7. Pointing without a diagram: callouts

What this gives the agent is pointing in the world, and most of the time it will want that without a diagram: "the stick on the left", "the lookout over there". A one-node diagram does it, but at the cost of a volume, a layout and a panel's chrome around a single label. Callouts should be **their own small feature**, sharing the references, their resolution and the drawing with world edges:

- **A `world:` link in what the agent says points while the sentence plays.** `Take [this stick](world:object/farmyard:campsite/stick-2) and hold it over [the fire](world:object/farmyard:campsite/bonfire).` Speech reads the link's text, the transcript shows it, and as the sentence plays the target's mark and label show and a thin line runs to it from the agent itself, the swarm reaching towards it, which is the agent's own pointing gesture. It fades 4 s after the sentence ends. The forms are `world:KIND/NAME`, with a handle's `#` kept as it is (`world:person/Samuel#11`), and resolve as references do. An unknown one is spoken as plain text, with a note to the agent.
- **A callout pins a label on a thing until dismissed.** A line in a block, or the `show_callout` tool, `callout fire : "Start here"` or `{ "target": { "kind": "object", "name": "farmyard:campsite/bonfire" }, "label": "Start here", "id": "start" }`, stands a small label by the thing, joined to it by a short leader, tracking it as a world edge's end does. It is a panel kind, `Panel::callout`, as a volume is: so it has an id, a version and an update, is listed by `panels`, is kept with the space and is thrown away with B, and is bounded among panels, 8 in a space at most. What is new is that its place is its target. A participant may pull it off to float, where its leader still joins it to its target.
- **Inside a diagram**, `callout fire : "Hot!"` labels a reference without a node, for a diagram whose world is part of the explanation but not a step.

## 8. Bounds

| Bound | Value | Why |
| --- | --- | --- |
| World references in a diagram | 16 | a diagram pointing at more is a tour, better walked through with callouts |
| World edges in a diagram, to one reference | 24, 8 | legible from one place; the merge at the target keeps one mark |
| World edges and callouts in a space | 64 edges, 8 callouts | beyond, the oldest diagram's are faint and untracked |
| Leaders drawn by a client | 32 a frame, `Limits::leaders` | tubes and labels within the volumes' budget |
| Re-resolution | once a second per diagram | the rate objects are described at |
| Server positions for untracked targets | 2 Hz, beyond 5 cm | eased on clients |
| Awake volumes, springs | 8 in a space, 20 Hz, streamed at 10 Hz | about 3 ms a second, 4 KB a second each while awake, nothing at rest |

## 9. Staged plan

After diagrams' stage 1 (#386). Each stage keeps every test passing and changes nothing for a diagram without world references.

1. **Anchors and objects, with static leader lines.** The `:=` line and `world` in `show_diagram`; references to objects, anchors, walls and panels, resolved when the diagram is shown or changed, with warnings and suggestions in the same turn; `Hello::world_edges` and the outline's lines for older clients; leader lines as tubes with a ring or a frame's light on the target and labels; the world springs and ports, solved once with stage 1's layout; `place: near references`; pointing that names a world edge's ends; focus that brightens a step's world edges and dims others, with half-strength by default; `Object::with_extent`, given by the farmyard to the bonfire and sticks. Judged on the headset with the marshmallow example at the campsite.
2. **Live tracking and highlights on targets.** `ClientSide::locate` and `Input::emphasised`, implemented by the farmyard for the bonfire, sticks and hens; the server's positions as live data for what cannot be located; people, workers and the agent as targets, with the privacy rules and the opt-out; absent targets as stubs; the springs run live, damped, settling and sleeping, streamed as physics' bodies; highlights while speaking lighting targets, with diagrams' stage 2.
3. **Callouts without a diagram.** `world:` links in speech, with the swarm's reaching line; `Panel::callout`, `show_callout` and the `callout` line.
4. **Routing and clutter.** Repulsion from heads and panels on leaders and volumes; edge-of-view chevrons; fading with distance and marks drawn through what hides them; edges between two world references; merged ends; `Limits::leaders` tuned on the Quest 3.

## 10. Costs and risks

- **The world is messy, as the maintainer said.** Things move, get picked up, float off, leave. The design leans on three choices to keep that tractable: references kept by name and resolved again, so diagrams survive change; the server deciding which and the client deciding where, so motion is drawn without lag and without new traffic; and everything absent drawn as a stub with a reason, never as a line to nowhere.
- **Lines across a scene are visually loud.** Faint by default, full only in focus or while spoken of, fading with distance, and capped. The first trial should check whether half strength is still too much.
- **Environments differ in what they can emphasise.** Outlining is the environment's own drawing, optional, and a halo is the floor. An environment that names little (Sky, the grid) gives the agent only anchors, panels and people to point at, which is an argument for environments describing their landmarks, the farmyard's coop, pool, waterfall and gate, as objects with references and positions, even though nothing about them is interactive.
- **Moving diagrams are harder to read.** The live springs are capped, thresholded and sleep. If the trial finds even that distracting, stage 2 can track the leaders' ends live and leave the nodes where stage 1 put them.
- **Pointing at people is pointing at people.** It is no more than presence shows, but a halo labelled by the agent above someone's head is a social act; the opt-out is a minimum.

## Open questions for the maintainer

- **Syntax.** Is `fire := object farmyard:campsite/bonfire` the right form, against a keyword line such as `world fire object …`, or an attribute on a node? And should a missing kind be inferred, as recommended, or refused?
- **Default placement.** Should a diagram with world references default to `near references` rather than the panel default, or only when the agent asks?
- **People.** Should people be targets at all in stage 2, and is B on one's own halo enough, or should a participant be able to refuse being pointed at in their settings?
- **Callouts.** A panel kind of their own, as recommended, or callouts only as transient `world:` links in speech?
- **The agent's gesture.** Should the swarm reach towards what a `world:` link names, or should the target only light?
- **Live springs.** Live for every world-connected volume, as recommended, or only while a node is dragged, with targets' motion moving only the leaders' ends?
- **Landmarks.** Should environments describe their non-interactive landmarks as objects so they can be pointed at, and should the farmyard be first?

## Design documents this would change

`design/diagrams.md` (world references, their syntax, resolution, warnings, world edges, ports and the springs, live and asleep), `projection.md` (`near references`, `Panel::callout`, `Hello::world_edges`, `Limits::leaders`, live data for targets and springs), `environments.md` (`Object::with_extent`, `ClientSide::locate`, `Input::emphasised`, landmarks), `mcp.md` (`world` in `show_diagram` and `update_diagram`, `show_callout`, the resolutions in their answers, the instructions' paragraph), `interaction.md` and `responders.md` (pointing at world edges and marks, focus lighting targets, B on one's own halo), `speech.md` (`world:` links timed to sentences), `rendering.md` (leader tubes in the space, marks, chevrons, the budget), `security.md` (what references may name, and the privacy rules), `farmyard.md` (extents, locating and emphasising the bonfire, sticks and hens, and its landmarks), and `physics.md` (the encoding shared with streamed volumes).
