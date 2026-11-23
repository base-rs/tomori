# Proposal: Interactive Diagrams

Status: **proposed**, November 2026, for the maintainer's review. Nothing here is a requirement until a stage brings it into the design documents it changes (listed at the end), which take precedence. It was written against `origin/main` at `916c45ca` (#531).

It builds on the diagrams already built (`design/diagrams.md`: volumes, the `diagram` block and `show_diagram`, layered and force layouts on the server, pointing at nodes and edges, the trigger's private focus, and world references), the two proposals they came from (`docs/proposals/three-dimensional-diagrams.md`, #373, whose stage 3 the maintainer approved as "single nodes moved by hand, the layout settling around a dragged node and remembering its pin", and `docs/proposals/diagrams-in-the-world.md`, #389, whose section 5 runs springs live), carrying and both hands (`design/projection.md`, **Both hands**, #478), objects' reach and grips (`design/interaction.md`, #388), physics' authority following the hand (`design/physics.md`), the chart palette (#511, #517), opaque panels and the depth rules (`design/rendering.md`, #530), and the colour-blind mode (#516, open). Label occlusion and `<br>` in labels are being fixed separately (branch `diagram-label-fixes`) and are not covered here.

## The request

The maintainer, in the headset, November 2026, looking at a diagram of about 30 nodes showing Tomori's crates:

- It is hard to understand and looks complicated: a lot of geometry, a lot of grey boxes.
- The grey platforms under subgraphs have no clear meaning.
- Arrows don't quite touch their source or destination.
- It misses the chance to be **interactive**: "grab a node and pull it towards me, and have the graph rearrange itself on springs (Hooke's law), so I can see everything coming into it and going out of it."
- Use **colour and shape** much more to show structure.

## Recommendation in brief

- **Colour means group, shape means kind, and nothing else is drawn by default.** Each top-level group gets a colour from the chart palette and its nodes are tinted in it. Clusters get no platform: only a label in the group's colour above its members, which the layout already keeps together. Layers keep a thin floor and containers a thin frame, both in the group's colour, because those kinds mean something. Ungrouped nodes are a light neutral colour, not dark grey.
- **A small set of kinds, written in Mermaid's own syntax.** `:::store` or `class a,b store` uses one of six built-in kinds (module, store, external, interface, process, decision), each with a shape. `classDef` defines new kinds. A legend on the volume's face lists the groups and kinds used.
- **Edges that touch.** Each edge ends exactly on the surface of each node's actual solid. Direction is shown by a taper from source to target, plus moving pulses on the edges of a focused or held node, instead of small cones. Edges are thinner and neutral, and an edge inside a group takes the group's colour.
- **Grab a node and pull it.** The grip on a node takes that node, by ray or by touch. The grip on the volume's face or heading still carries the whole volume. A live spring layout (Hooke's springs on edges, repulsion between nodes, damping) keeps the held node on the hand. Its incoming neighbours fan out to its left and its outgoing ones to its right, as seen by the person holding it. Everything else falls back and fades. Let it go outside the volume and it springs home. Let it go inside and it stays pinned there, as the maintainer already decided. A on a pinned node unpins it, and a ↺ on the heading resets the layout.
- **Shared by the server, the way physics is.** The server runs the springs while someone holds a node, with the held node pinned to that person's hand from the presence it already gets, and streams node positions as physics streams bodies. Nothing is sent when the layout is at rest. Whoever holds the node sees it in their hand with no lag. Each node can be held by one person at a time (first to arrive wins). Different people can hold different nodes.
- **Prototype in two phases on the Quest 3:** (a) the visual encoding, in about 2 days; (b) grab and pull for a single user, simulated on the client and sharing nothing, in about 4 days. Sharing, agent tools and expanding and collapsing groups come after.

## What exists today, and why it looks complicated

| What the maintainer saw | Cause in the code |
| --- | --- |
| Lots of grey boxes | A node's colour defaults to `Colour::Grey`, drawn as `NODE_GREY` (`tomori_scene::projection::volume::rgb`). An agent that colours nothing gets all grey nodes on a dark plate. |
| Grey platforms | Every group that is not a container is a `TRAY` box underneath its members (`Mesh::new`), dark grey for both `cluster` and `layer`. A 30-node crate graph with five groups draws five slabs that mean only "these belong together", which nearness already shows. |
| Lots of geometry | Edge tubes 4 mm thick (8 mm for weight 3, twice that for thick edges), cones 30 mm long, layered waypoints that bend each long edge up to four times, and trays. With 30 nodes and about 50 edges, edges and trays outweigh the nodes. |
| Arrows don't touch | Each edge is cut back from the node's centre by the node's half-width (`path[0] += start * from.1`), which is the radius of a sphere around it. A box is 0.7 of its width tall and deep, so a vertical edge in `layered down` stops 0.15 of a width (about 1 cm) short of the box. A card is 0.12 deep, so an edge arriving from the front stops 3.5 cm short. A cylinder or prism is 0.7 to 0.8 tall. The cone then sits in that gap. |
| Not interactive | The layout is computed once on the server and drawn from a mesh built once per version. The trigger gives a private focus, and the grip carries the whole volume "by any of its nodes, edges or its tray". |

## 1. Visual encoding

A small, consistent visual language: **colour = group, shape = kind, size = importance, taper and pulse = direction, brightness = attention.** Each property means one thing, and the legend says what.

### Groups as colour, not platforms

- **Each top-level group gets a colour from the palette**, `Colour::SERIES` (the eight Okabe–Ito-based colours from #511, tested to stay distinct under simulated colour blindness), assigned in the order the groups are written. A nested group uses its parent's colour, slightly lighter (+12% L in CIELAB) for each level, so the hierarchy reads as a family of shades. A ninth and later top-level group falls back to neutral with a dashed frame, with a warning (`9 groups: "x" and later are drawn neutral; the palette has 8`).
- **Nodes take their group's colour.** A node's own `colour:` still applies and overrides the group's, and the legend lists it as its own entry. A node without a group is a light neutral (`[0.78, 0.8, 0.84]`), never the current dark grey, so a diagram with no groups reads as calm and light rather than murky.
- **A cluster draws no geometry.** It gets its label, in its colour, above the top of its members. When a member is pointed at, focused or held, a thin outline of the group's rounded bounds appears in its colour (tubes 1.5 mm thick) and fades over 0.3 s once attention leaves. That is when knowing the boundary helps.
- **A layer keeps a floor, and a container keeps a frame**, because those kinds are about where something sits (a tier) or what holds it. They are now thin, 3 mm, in the group's colour at 35% brightness. The agent's instructions say to use `kind: layer` only for real tiers and `kind: container` only for real containment.
- **No translucent hulls.** A soft translucent hull around each group was considered and rejected for now. A Quest 3 frame is 88% fragment shading (#530), and large translucent shapes covering each other are the most expensive thing to draw and the hardest to read in stereo. Tinted nodes cost nothing extra: they are the same opaque instanced solids (`tomori_render::solids`).

### Shapes by kind

The six shapes stay the same. What changes is that the agent writes what a node *is*, and the shape follows:

| Kind | Shape | For |
| --- | --- | --- |
| `module` (default) | box | a crate, module, component or service |
| `store` | cylinder | a database, cache, file or queue at rest |
| `external` | sphere | a person, a client or a third-party system |
| `interface` | card | an API, protocol, schema or config |
| `process` | hexagonal prism | a job, stage, worker or step |
| `decision` | diamond | a branch or check |

**Syntax, kept compatible with Mermaid.** The kinds are classes: `db:::store`, `class api,proto interface`, or a `classDef` that defines a new kind or changes a built-in one, such as `classDef crate shape: box, label: "Crate"`. Mermaid itself accepts a class it has no definition for and draws the node normally, so a Tomori diagram still renders in Mermaid. Brackets (`[( )]` and so on) still set a node's shape directly and win over its kind. A node's kind is sent as `Node::kind`, a short name of up to 16 characters, used only by the legend and the pointing note (`store, cylinder`). `classDef`'s Mermaid-style `fill:` and `stroke:` are read as `colour:` where they name one of the palette's colours, and otherwise ignored with a warning, as `style` lines are now.

### Edges

- **Attached to the real surface.** Each end of an edge is where its last segment, extended, meets the node's actual solid, scaled and turned as drawn: a box, cylinder, prism, octahedron or thin card. This is a ray-against-solid test in `Mesh::new` (one per edge end, done once per layout), instead of cutting back by a bounding sphere.
- **Direction by taper and pulse, not cones.** A directed edge is a tube that narrows from 3 mm at its source to 1.5 mm at its target, and ends in a 1 mm collar flat against the target's surface. That is readable from any side and at any distance, where a 12 mm cone reads poorly. An undirected edge keeps the same width all along. An edge marked `both` is uniform with collars at both ends. When a node is focused, pointed at or held, its edges carry moving beads (as `flow` edges do now: 3 beads at 12 cm/s) towards the target, so direction can be seen without reading anything. Flow edges keep their beads all the time. Arrowhead cones are dropped. One new solid, a tapered tube (a frustum), joins `Solid::ALL`.
- **Colour.** An edge between two nodes of the same group is that group's colour at 60% brightness. An edge between groups is neutral (`[0.6, 0.66, 0.74]` at 70%), so groups read as coloured clusters joined by quiet grey lines. A focused node's edges brighten to their full colour. An explicit edge `colour:` still applies.
- **Fewer bends.** Layered waypoints stay, but an edge between adjacent ranks is straight. A long edge is drawn as one smooth curve through its waypoints (a Catmull–Rom curve, 8 pieces per span) rather than a chain of straight segments with corners.

### Legend and labels

- **A legend is generated** on the volume's face below its title, only when it says something: one row per group used (a swatch in its colour and its label), then one row per kind used other than `module` (a small solid of that shape, drawn as a node is, and the kind's label). At most 8 group rows and 6 kind rows, with "+N" for the rest. The panel's outline for older clients lists it too.
- **Labels** keep the font, sizes and billboarding they have now. Placing and wrapping them is the label-fixes branch's job and is not repeated here. Two changes: a group's label is drawn in the group's colour, and when a node is held, its neighbours' labels are always shown, ahead of the size ranking that drops some labels when there are more than 32.

## 2. Interaction

### Grab and pull

- **What the grip takes.** On a volume, the grip's order (`precedence::Grab`) becomes: a node the ray meets, or a node within `projection::TOUCH` (12 cm) of the hand, **takes that node**; otherwise the volume's face, heading, plate or an edge carries the whole volume, as now. Carrying and both-hands stretching of the whole volume are unchanged. A second grip on another node of the same volume takes that node too (see **Two nodes** below). A second grip on the face while one hand holds a node holds the volume still, as #373's stage 3 described.
- **Where the held node goes.** Taken by ray, the node keeps the distance along the ray where it was taken, and follows the ray point, so pulling the hand in brings it closer. The stick reels it in or out along the ray, as it pushes and pulls a carried panel. Taken by touch, it is held at the offset where the hand closed on it. In both cases the hand's own client draws it in the hand every frame.
- **The springs.** While a node is held, the volume runs a live spring layout in its own millimetres, one fixed step of 1/60 s at a time, decoupled from the frame rate:
  - **Hooke springs along edges**: `F = −k (|d| − L) d̂`, with rest length `L` = 120 mm × the scale of the larger of the two nodes, and `k` per edge proportional to its weight;
  - **repulsion between every pair of nodes**: `F = c / r²`, capped below the sum of the two half-widths so close pairs don't fly apart;
  - **a weak pull towards home**: each node is pulled towards the position the server's layout gave it, at 5% of an edge's stiffness, so the shape of the diagram is still recognisable;
  - **the fan**: in the frame of the person holding the node (their head's right and up), each incoming neighbour is pulled to a point on a 70° arc to the held node's left, and each outgoing neighbour to a matching arc on its right, ordered by their current height, at radius `L`. A neighbour that is both incoming and outgoing goes above. Inputs on the left and outputs on the right read in the same direction as the text;
  - **pushed back**: nodes more than one step from the held node feel a gentle push away from the viewer, out of the fan's plane;
  - **damping**: velocity is multiplied by 0.86 each step; speed is capped at 1.5 m/s in the volume's scale; integration is semi-implicit Euler;
  - **the held node** is fixed in place, with infinite stiffness, at the hand.
- **Attention.** While a node is held, it and its neighbours are drawn at full brightness, with pulses on its edges. Nodes two steps away are at 50%, and everything else at 20% with its labels hidden, as the trigger's focus does now. A held node's detail card is shown beside it.
- **Letting go.**
  - **Outside the volume's box** (the pull-towards-me case): the node is released, its spring home takes over at full strength, and the layout eases back to the server's positions over about a second, then stops. Pulling a node out is for looking at it. It behaves like a rubber band, not a rearrangement.
  - **Inside the box**: the node is **pinned** there, as the maintainer decided for #373. The rest of the layout settles around it and stops. A pinned node gets a small ring at its base, in its colour.
  - **A** on a pinned node unpins it, and it eases back. A small **↺** on the volume's heading (like a wall's reset handle) unpins every node and returns to the server's layout. The agent's `relayout` does the same.
- **Settling and sleeping.** Once no node has moved more than 1 mm for 0.5 s, the simulation stops and costs nothing until someone grabs a node or the diagram changes.
- **Two nodes.** Holding a node in each hand pins both to their hands, without the fan. The shortest path between them (fewest edges, in the order written) is drawn at full brightness with pulses, and everything else dims. "How does this get to that?" becomes a gesture.

### Point to focus, expand, collapse, reset

- **Pointing at a node** brightens its edges and its neighbours' labels slightly (to 70%), only for the person pointing and sending nothing, so the neighbourhood can be previewed before grabbing. The **trigger** keeps its current meaning: a private focus with the detail card.
- **Expand and collapse** (stage 2 of #373, still to come): pointing at a group's label with A collapses its members into one node of the group's colour and its kind's shape, with its edges merged and their count written on them. A again expands it. This changes the diagram for everyone and is sent like a tick, against the panel's version.
- **Reset**: the ↺ on the heading (above), for everyone once the layout is shared.

### What the agent is told

- **Pointing notes** gain the node's kind, its in and out degree, and whether it is held or pinned: `[Pointing at node "tomori-scene" (id scene) in diagram "Tomori's crates" (id crates, floating, projected 4 minutes ago; module, box, in group "client"; in 5: from client-quest, client-desktop, client-spectator, render…; out 4: to protocol, text, world, highlight; held by Samuel)]`. Each list of neighbours is cut at 6 with "…" and counted. The counts come from the server's own copy of the diagram.
- **Words begun while holding a node** name it, as words begun while pointing do. The held node is listed first among what the hands are on, because holding something is the stronger sign of what someone means.
- **Grabs and releases are not narrated.** They would be noise. The agent learns what a participant holds when they speak, and from `presence`, which lists for each hand the node it holds.

### Agent tools

One tool, `diagram_view`, so the agent can direct attention for everyone while it explains:

| Argument | Effect |
| --- | --- |
| `id` | the volume |
| `focus: "node"` | that node and its neighbours bright for everyone, the rest dimmed, with the fan layout, as if the node were held by nobody in particular, from the point of view of whoever asked |
| `path: ["a", "b"]` | the shortest path from a to b highlighted, with pulses |
| `highlight: ["a", "b→c"]` | those nodes and edges highlighted |
| `expand` / `collapse: "group"` | as A on its label |
| `clear: true` | ends the agent's focus and highlight |
| `relayout: true` | unpins every node |

The agent's focus and highlight are shared and last 20 s, or until cleared, replaced, or someone grabs a node. A participant's own trigger focus still takes priority on their own client. The tool answers right away, saying what it highlighted and naming any unknown ids, with suggestions for close matches, as `show_diagram` does. Timing highlights to sentences (`diagram:` links in speech) remains #373's stage 2.

## 3. Shared state

A diagram is part of the space, so once the layout moves for one person it has to move for everyone, or two people would be pointing at different pictures. The physics design has already solved this pattern, and diagrams can use it directly.

- **The server runs the simulation.** It already lays the diagram out deterministically in `f64` (`crate::diagram`). A volume with a node held is **awake**: the hub's sweep steps its springs at 30 Hz (the cost is below), using the held nodes' targets taken from the holders' hands in the presence the server already receives (up to 20 times a second per participant for server sides, each a pose). When it settles it sleeps, and its rest positions and pins are stored with the panel.
- **Holding is an intent, like taking a body.** `HoldNode { panel, version, node, hand, offset, epoch }` and `LetGoNode { panel, node, epoch, at }`, using physics' rules: visitors can't hold nodes; the server checks the node is within reach (`PULL`, 3 m, of the hand, or on its ray within 8 m); each node has an epoch, raised by every hold and every release, so **if two people grab the same node, whoever's intent arrives first holds it**, and the other's client sees it in the winner's hand and lets go of its own. **Two people holding different nodes** is fine: both are pinned and the springs pull between them, with each holder's fan applied in their own frame, which in practice makes a tug between them. A node is let go where the server last saw the hand when its holder leaves or goes quiet for 3 s (`SILENT`).
- **No lag for the holder.** The holder's client draws the held node in their hand every frame. Every other client draws it from the holder's hand in presence, interpolated on the same timeline that draws the holder's own hand (`people.md`), so holding adds nothing to the wire. The rest of the layout is drawn from the server's snapshots, 100 ms behind (the `DELAY` that `mirror::Mirror` uses for bodies), and smoothed in over 150 ms (`BLEND`). The holder sees the graph follow about a tenth of a second behind their hand, which looks like springs.
- **Bandwidth.** A snapshot lists only the nodes that moved more than 1 mm: an index and three `i16` millimetres, 7 bytes each, so at most 448 bytes for 64 nodes, sent at 15 Hz while awake, at most about 7 KB/s per awake volume and nothing at rest. At most 4 volumes in a space are awake at once; a fifth grab is refused with a light tap and the label "Another diagram is moving". Snapshots are live data for a panel, like `chart_append`: they don't change the panel's version, so a tick, update or move read before a grab isn't refused because of it. Pins change the version, because they change what is stored.
- **Determinism.** Only the server simulates for everyone, so clients don't need to agree bit for bit. They draw what they are sent. Determinism matters for what is stored and for tests, and the existing rules already cover it: `f64`, fixed steps, nodes in the order written, no randomness, no hash-map order. Rest positions are rounded to millimetres before storing, so a restart, someone joining and the agent's `panels` all see the same picture.
- **Private versus shared.** The layout, pins and the dimming caused by a held node are shared, because they are what the room sees moving. Pointing and the trigger's focus stay private. The agent's `diagram_view` focus is shared.
- **Older clients.** A client that doesn't announce `Hello::diagram_springs` is sent the volume with its stored positions each time it settles, as a normal update. It sees the diagram jump from one rest to the next rather than move. A server that doesn't understand the intents ignores them, and the client lets go after 0.5 s with no answer.

## 4. Performance

| Item | Estimate on Quest 3 |
| --- | --- |
| Bounds | unchanged: 64 nodes, 128 edges, 12 groups per volume; the headset's 16 volumes and 1,024 nodes |
| One spring step, 64 nodes | 2,016 pairs of repulsion plus 128 springs plus the fan, about 25k floating-point operations: about 15 µs on one of the Quest's cores in `f32` |
| Client simulation (phase b) | 60 steps a second: under 1 ms a second, about 0.1% of a 72 Hz frame budget, only while awake |
| Server simulation (phase c) | 30 steps a second × 4 awake volumes, in `f64`: under 3 ms a second |
| Rebuilding the mesh while moving | nodes' instances move; each edge is rebuilt (at most 128 × 8 curve pieces, plus beads), about 1,200 instances, all submitted as the instance buffer already is each frame; label canvases are kept and only their positions change. About 0.2 ms of CPU per frame for a volume at its limits |
| GPU | the same opaque solids, fewer of them than today (no trays, no cones, thinner edges); no new translucent surfaces except outlines while focused |

- **Layout data separated from geometry.** Phase (b) splits `Mesh` into what is built once (topology, kinds, colours, label canvases) and what each frame computes from the current positions (node and edge instances, label positions, hit spheres). A volume at rest keeps today's cost of building once.
- **Settling** is bounded: if a volume hasn't settled after 8 s, it is damped to stillness over 1 s and goes to sleep, so a tangled graph never jitters forever.
- **Level of detail**: past 3 m, or when a volume's largest side looks smaller than 15°, edges are drawn as untapered tubes without beads and edge labels are hidden; past 8 m, nodes and group labels only. Nothing grabbed is reduced.
- **Ranks.** A layered diagram that is grabbed switches to springs while awake, drawn with straight edges. Its waypoints come back when it eases home. A pinned layered diagram keeps the layered positions of everything except what the pins pulled, and is laid out again only when its springs settle, so the ranks stay readable.

## 5. Accessibility

- **Colour never works alone.** Group is also shown by position (the layout keeps groups together), by the group's coloured label, and by the outline when focused. Kind is shown by shape. Direction is shown by taper and by motion. Attention is shown by brightness and label visibility, not hue.
- **Colour-blind mode (#516)**, when it's built, adds to diagrams without changing the visual language: group outlines always shown, each group's dash pattern of its own (solid, long, short, dotted, and so on), and each group's initial letter on its nodes' labels and in the legend. If the mode picks a palette tuned to one type of colour blindness, groups use it. The palette's separation test (`Colour::SERIES`) already covers what this proposal adds.
- **Motion.** Springs are damped and speed-capped, and they settle and sleep. The beads move slowly (12 cm/s). Whether a setting on each device stills them is still an open question from #373, and the colour-blind mode's settings would be a sensible place for it.
- **Reach.** Nodes can be taken by ray up to 8 m away, as well as by touch, and either hand can grab.

## 6. Phasing and the prototype

Each phase is tried on the Quest 3 against the maintainer's crate diagram (saved with the prototype as `crates/server/src/diagram/crates.diagram`) before the next starts.

### Phase (a): colour, shape, edges that touch (about 2 days)

- **`tomori-protocol`**: built-in kinds as classes (`diagram/parse.rs`, `diagram.rs`), `Node::kind`, `classDef` read as defining a kind, `fill:` read as a colour, the warning for a ninth group, and the outline's legend; tests in `diagram/tests.rs` and `parse/tests.rs`.
- **`tomori-scene`** (`projection/volume.rs`): group colours and their shades, the light neutral for ungrouped nodes, no trays for clusters, thin floors and frames in the group's colour, the outline on focus, ends that meet the real solid, tapered edges with collars, no cones, curved waypoints, colour by group for edges, beads on a focused node's edges, the legend on the face; tests in `volume/tests.rs` (every shape's surface reached within 0.5 mm from every direction, colours, a legend for groups and kinds used).
- **`tomori-render`**: the tapered tube solid in `solids`; the `TOMORI_VOLUME_PREVIEW` render of the crate diagram, before and after, attached to the PR.
- **`tomori-server`**: the agent's instructions (kinds, groups as colour, `layer` and `container` only when they mean it) and kinds in pointing notes.
- **Judged by**: is the crate diagram readable at a glance, do groups read without platforms, and do edges visibly touch.

### Phase (b): grab and pull, single user (about 4 days)

- **`tomori-scene`**: `projection/volume/springs.rs`, the spring simulation in `f32` with fixed steps, the fan, pushing back, home springs, damping, sleeping, and two-node paths; `Mesh` split into a built part and a per-frame part; the grip's new rung for a node (`precedence::Grab`, the projector), reeling with the stick, release inside or outside the box, local pins, A to unpin, the ↺ chip, dimming and the detail card while holding, two nodes in two hands; feedback on the hand (a tap on taking, on pinning and on springing home). **All of this is local to one client and sends nothing.** Other participants keep seeing the server's layout, and a pin lasts only for this session on this client.
- **`tomori-server`**: in and out degree and kind in pointing notes (shared with phase c).
- **`tomori-render`**: no changes expected; a lavapipe preview of the crate diagram with `tomori-scene` held, pulled 60 cm out, fanned.
- **Tests**: the springs converge and sleep within bounds for the crate diagram and at the 64/128 bounds; a held node stays at its target; incoming neighbours end up left and outgoing right in the holder's frame; release outside returns home within 1.5 s and inside stays; the step costs under 50 µs on desktop at the bounds (a guard well above the Quest estimate); the grip on a node takes the node and the grip on the face carries the volume; words begun while holding name the node.
- **Judged by**: does pulling a node towards you make its inputs and outputs obvious, does the motion feel like springs rather than wobble, and is release-to-return right.

### Later phases

- **(c) Shared.** The solver moves to a small shared crate, `tomori-diagram` (no Rapier, no dependencies beyond `glam`), with the server's `force.rs` and `layered.rs` joining it, so the server and the client prototype run the same code. Hold and release intents, epochs, awake volumes in the hub's sweep, snapshots as live data, pins stored with the panel, `Hello::diagram_springs`, `presence` listing held nodes, older clients updated at each rest. About 5 days.
- **(d) The agent and groups.** `diagram_view` (focus, path, highlight, relayout), expand and collapse (#373's stage 2), group outlines that follow the springs. About 4 days.
- **(e) Colour-blind mode hooks** with #516, and level of detail tuned on the headset. About 2 days.

## 7. Open questions

1. **Shared or private pull?** This proposal makes pulling shared (phase c) because everyone should see the same diagram. An alternative is to make pull-to-look a private lens, like the trigger's focus, and share only pins. Which fits how the maintainer presents to others?
2. **Release semantics.** "Outside the box springs home, inside the box pins" is a guess at both wishes (looking versus rearranging). Is the boundary the right signal, or should pinning be a deliberate gesture (for example, releasing while holding the trigger)?
3. **Colour for group or for kind?** This proposal puts group on colour and kind on shape. A diagram without groups could colour by kind instead. Is that a useful exception, or a confusing one?
4. **Should `force` become the default layout** for diagrams that aren't pipelines, since the springs fit it naturally and a layered diagram has to switch to springs when grabbed?
5. **Direction cues.** Is the taper readable on the headset, or should a small arrowhead stay at the target for anyone who is used to arrows?
6. **Two nodes, two people.** Does a tug between two holders feel playful or confusing? An alternative is that the second holder's fan is ignored while someone else holds a node.
7. **What the agent is told about grabs.** Only when words begin (as proposed), or a short note when someone pulls a node and holds it for more than 3 s, so the agent could offer an explanation?
8. **The legend's place**: on the face below the title (as proposed), or a small card that floats beside the volume and can be hidden?

## Design documents this would change

`design/diagrams.md` (kinds, groups as colour, edges' ends, taper and collars, the legend, grabbing, the springs, pins, sharing and snapshots, `diagram_view`, pointing notes' degrees), `design/projection.md` (live layout data for a volume, `Hello::diagram_springs`, the grip on a node versus the volume), `design/interaction.md` and `design/responders.md` (the grip's node rung, A to unpin, the ↺ chip, two nodes in two hands), `design/mcp.md` (`diagram_view`, `presence`'s held nodes), `design/rendering.md` (the tapered tube solid, outlines while focused), `design/physics.md` (the hold and release intents and epochs shared with nodes), `design/crates.md` (`tomori-diagram`, in phase c), and `design/feedback.md` (taps on taking, pinning and springing home).
