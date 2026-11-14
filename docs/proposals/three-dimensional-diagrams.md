# Proposal: Three-Dimensional Diagrams

Status: **approved by the maintainer**, November 2026, with the decisions below, which this text now follows; **stage 1 built**: static volumes, from a `diagram` block and `show_diagram`, laid out layered or by force, drawn, placed and pointed at, as `design/diagrams.md` now requires, with the departures its rationale names. Stages 2 to 4 are proposals still, and nothing of them is a requirement until each brings it into the design documents it changes (listed at the end), which take precedence. It was written against `origin/main` at `1aa32a71` (#371).

It builds on projection's walls, panels, placement forms, ids, versions and compare-and-swap, revisions and patches, and the limits each client states (`design/projection.md`, #284, #289); chart panels and their summary for older clients (#267); pointing at a list's rows and items (`design/panel-actions.md`, #297, #298); the content scale of a wall (#313); the ray, its unified end and each hand's own ray, trigger and grip (`design/interaction.md`, `design/responders.md`, `docs/proposals/two-handed-pointing.md`, #354); depth ordering, the shared shapes and the faces kept across frames (`design/rendering.md`), and the rendering proposal being written beside this one (`proposal-rendering`: bounds and culling); the glyph atlas (`design/text.md`); Tomori's MCP tools (`design/mcp.md`); and glTF assets (`design/assets.md`). It takes up what agent projection's extension (#82) calls the `tomori-scene` structure format and its `show_diagram` tool, and the graph layout of the shared code toolkit (#147), and the structure views the GitHub and code proposal asks for (`docs/proposals/github-and-code-environments.md`, #152).

## The request

Spoken by the maintainer from the headset, November 2026:

> We've got a lot of good two-dimensional visualisations (images, graphs, text), but we don't have a way for you to talk about the organisation of systems that might be better presented in three dimensions. I wonder if the panel is the right kind of container. A panel is two-dimensional, but it could be three-dimensional and contain geometry. Or you could create geometry like panels with expressed connections between them: here is the renderer module, here is the octree module, and they talk through this mechanism; two spheres in 3D with the connections between them. I don't know where we start: you'd need a mechanism to express that geometry, whether programmatic to start with or a scene description language. The common factor might be WebGPU, but that's pretty low level. I'm open to thoughts.

## The maintainer's decisions

1. **A volume is a kind of panel with depth**, not a separate sibling container (section 2).
2. **Both ways to write it:** a Mermaid-like `diagram` block the agent writes in its replies, and the `show_diagram` and `update_diagram` tools, which report errors in the same turn (section 3). Whether a plain `mermaid` block also becomes a volume is not decided; it stays a code panel.
3. **Default place as panels do today:** onto the wall the asker is looking at if they are near enough, else floating beside them (sections 2 and 8).
4. **Moving nodes by hand: yes**, in stage 3, with two-handed carry; the layout settles around a dragged node and remembers its pin (sections 4, 7 and 10).

The other open questions keep this proposal's recommendation until they are decided (see the end).

## Recommendation in brief

- **A volume is a kind of panel, not a sibling of one.** It is a panel whose face is a box rather than a rectangle: it goes on a wall's slot, floats, is carried, docked, swapped, thrown away, kept with the space, updated by its id and versioned exactly as a text, chart or image panel is, and counts against the same bounds. What is new is its depth, a size of its own, its geometry, and pointing at its parts. A separate container would have to duplicate nearly all of `projection.md`; a panel kind inherits it, as charts and images did.
- **The agent describes structure, never geometry.** Nodes (an id, a label, a shape, a colour, a size by importance, a detail line and an optional reference), edges (from, to, a label, a direction and a style: solid, dashed, thick or a flow that moves), groups (clusters, layers and containers, nested at most three deep) and a layout hint (layered, radial, force or grid). Nothing it writes is a coordinate, a mesh or a shader.
- **Two ways to write it, one model.** A fenced block of language `diagram`, with a panel block's header and a compact body shaped like a Mermaid flowchart, which language models already write fluently; and the tools `show_diagram` and `update_diagram`, which take the same model as JSON, checked against a strict schema and answered at once with any errors. Both become one `tomori_protocol::diagram::Diagram`.
- **The server lays it out, once, deterministically, and keeps the layout.** Layered for pipelines and dependency graphs, radial for a hub and its spokes, force-directed for everything else, each in 3D and seeded from the nodes' ids rather than a random number, and each started from the previous version's positions, so that a node added settles in beside its neighbours and nothing else moves more than it must. Clients draw positions they are sent and lay nothing out, so every participant sees the same diagram by construction.
- **Drawn from a handful of instanced solids, ribbons and billboarded labels**, all from shared primitives and the shared atlas, a volume's mesh built once for its version and moved by one matrix, within a triangle and instance budget, culled by its bounds.
- **Pointing names a node or an edge to the agent**, as a list's row is named today: `[Pointing at node "Octree" (id octree) in diagram "Renderer" …]`. The trigger shows a node's detail and its neighbours for the participant alone; on a collapsed group it opens it for everyone. The agent highlights nodes and edges as it explains, by linking to them in what it says, each highlight shown as the sentence holding it is spoken.
- **Placed as panels are:** onto the wall someone is looking at if they are near enough, else floating beside them.
- **Staged:** (1) static volumes from a `diagram` block and `show_diagram`, layered and force layouts, carried and placed as panels, pointing that names nodes and edges; (2) updates by id that keep the layout stable, groups that collapse and open, radial layout, highlighting while speaking, and `update_diagram`; (3) a volume's own size, scaled with two hands and walked into, its text handle, and single nodes moved by hand with two-handed carry, the layout settling around a dragged node and remembering its pin; (4) glTF models and 3D charts inside volumes, and environments' stands and volumes of their own.

## What exists today

| Part | Today | Where |
| --- | --- | --- |
| Containers | Text, code, chart, list and image panels, each a `Panel` with an optional field for what is not text (`code`, `chart`, `rows`, `image`) and a body every older client can draw | `projection.md` |
| Placement | Walls' slots, floating beside or in front of someone, at an anchor, in a named slot; `place:` in a block's header or `place=` after a code block's language | `projection.md`, **Where the agent's panels go** |
| Updating | `id:` in a block's header, `update_panel`, versions as revisions, compare-and-swap for what edits part of a panel | `projection.md`, `mcp.md` |
| Wire | `Projection` whole, then `ProjectionPatch` ops, a panel replaced whole; live data apart from the version (`chart_append`, `chart_replace`) | `projection.md`, **Revisions and patches** |
| Per-client bounds | `Limits` in `Hello` and `Welcome`: frames, walls, slots, floating, panels | `projection.md`, **Limits stated in Hello** |
| Pointing at parts | A list item by its line, an environment's row by its id, a chart's time by the ray's position, the server describing each from its own copy | `projection.md`, `panel-actions.md` |
| Drawing | Panel faces as meshes of glyphs and shapes in the panel's plane, kept per version; opaque bodies first (the swarm, workers, controllers), then translucent surfaces back to front, each backing writing depth | `rendering.md` |
| Shapes | `tomori_scene::shapes`: rules, boxes, discs, rings, strokes, dots and quads, all in a plane, from the atlas's distance fields | `rendering.md` |
| 3D models | glTF binaries by digest, skinned on the GPU, an environment's own; the asset studio on this machine makes GLBs | `assets.md` |
| Diagrams | None in 3D. A Mermaid block is a code panel of its text; a render is an image panel of a few hundred pixels | `projection.md`, **Code panels**, **Image panels** |

The last row is the gap. Everything above it is what a volume reuses.

## 1. Why not something simpler, or lower

**Why not an image panel of a 3D render.** The agent can already do this: render a scene in Blender or the asset studio and show it with `show_image`. It is the right answer for a picture, and the wrong one for a structure:

- **It is flat.** A render has no stereo depth and no parallax as the head moves, which are the only reasons to draw a structure in three dimensions in a headset rather than on a laptop.
- **Its parts cannot be pointed at.** Pointing at an image panel names the panel; "this one" cannot mean the octree. The agent is told nothing it did not already draw.
- **Its text cannot be read.** `show_image` already tells the agent that a slot shows an image about 200 pixels tall, so labels must be a twentieth of its height; a diagram of twenty labelled nodes does not survive that.
- **Every change is a new render.** Growing the diagram as the conversation goes, or highlighting a node while explaining it, would need an image per step, made on the server's GPU, a second or more each.

**Why not raw WebGPU, shaders or arbitrary geometry.** The common factor across the headset, the desktop window and the spectator is wgpu, but exposing it would be the wrong interface:

- **It is unsafe.** What the agent writes is untrusted (`security.md`): it reads web pages, issues and files others wrote. Shaders from it would run on every participant's GPU, where a pathological one can hang or reset the device, and vertex and texture data from it could exhaust memory. Validating WGSL with naga proves it well formed, not cheap.
- **It cannot be bounded.** The headset is CPU-bound at 72 Hz (`rendering.md`); a projection's every part is bounded so that one at every limit still makes its frame. Arbitrary geometry has no such bound short of counting triangles after the fact.
- **It has no meaning.** Pointing works because the server knows what each thing is. A mesh of triangles has no "node", so nothing pointed at could be named to the agent, and nothing could be highlighted, collapsed or described to an older client.
- **Language models are poor at coordinates.** They write structure well and geometry badly: overlapping nodes, labels inside spheres, edges through boxes. Layout is a solved problem worth solving once, in code, rather than in each reply.

A programmatic form, bounded instances of the same few solids with transforms, as #82's `place` tool sketches, is kept as a later escape hatch (section 11), to be added only if the declarative one proves too limited, and never with shaders.

**Why declarative.** It is what the agent writes best, it is small on the wire (a 40-node diagram is a few kilobytes), it is checked in one place on the server, it carries meaning that pointing, highlighting and descriptions use, it degrades to a readable text outline for a client that cannot draw it, and its block stays readable in the transcript, where it reads as a list of what talks to what.

**Why not Mermaid, Graphviz or #82's SVG first.** #82's stage 1 renders a Mermaid block to SVG on a panel. That is still flat, and Mermaid's reference renderer is JavaScript; Graphviz's DOT is a C library and lays out in two dimensions. Both are worth borrowing from, not running: the body below is shaped like a Mermaid flowchart, so that the agent's habits carry over and much of what it writes is Mermaid as well, and layered layout follows the Sugiyama method Graphviz's `dot` uses, in 3D. This proposal recommends going straight to volumes for diagrams rather than through SVG panels.

## 2. The container: a volume is a panel

The maintainer asked whether the panel is the right container. It is, extended by one dimension, rather than replaced by a new one.

**A panel with a box for a face.** A volume is a `Panel` with a `diagram`, as a chart panel is a `Panel` with a `chart`. It has a title, metadata, an age, an id, a version and a place, and it is laid out on walls, floated, carried, docked, swapped, pinned, thrown away, kept across restarts, moved by `move_panel`, listed by `panels` and bounded as every panel is. So nothing of the placement rules, the full wall's room-making, the floating bound, the per-client limits, the environment's publishing or the older client's fallback has to be designed again.

**What a volume adds:**

- **Depth.** Its extent is a box: width, height and depth in its own frame, upright, its front facing as a panel's face does. Its content stands inside the box; it has no backing over its whole face, so what is behind it shows between its nodes.
- **Its own size.** A volume floating is as large as its diagram needs, from 0.4 m to 1.2 m on its longest side at its own scale of 1, as a floating text panel fits its text. Stage 3 gives it a shared scale, from a quarter to six times, so that it can be shrunk to a desk model or grown to a room to stand inside (section 8).
- **Geometry**, drawn as section 6 describes.
- **Parts that are pointed at**: its nodes, edges and groups (section 7).

**On a wall: a relief in its slot.** In a slot it stands on the slot's plane, its back there and its front at most half the slot's height in front of it, its width and height the slot's, its diagram scaled to fit, the slot drawn with a dark backing plate behind it so that the slot reads as a shallow display case. Since walls are read from in front, a relief keeps its depth readable as the head moves and stays clear of the wall's handles. A slot is small for a diagram, about 60 by 36 cm on a flat wall, so in a slot only the labels of its most important nodes show (section 8); a big wall, or a slot of a larger scale, shows more. Pulled off, it grows to its floating size, as a text panel grows to fit its text, and put back it shrinks into the slot again.

**Floating: a model on a table.** By default a volume goes where a panel goes (section 8): onto the wall its asker is looking at if they are near enough, else floating. Floating, it stands where `beside` places a panel, but nearer, 1.3 m from the eyes, and lower, its middle 25 cm below them, so that one looks a little down into it, as at a model on a table, which shows its depth better than one level with the eyes. A faint round tray beneath it, as wide as it, grounds it and is where the grip takes it when the ray meets nothing else of it. Its title stands on a small label above it, facing the viewer.

**Older clients and other surfaces.** A volume's body is a text outline of its diagram, which the server writes, as a chart panel's is a summary: its groups as headings, its nodes as list items with their details, and its edges as `Renderer → Octree: queries`. A client that knows no diagrams, the console, and a server's own clients stating no volumes (section 5) draw that, so nothing older breaks and nothing is bumped.

**Why not a sibling container.** The recommended direction proposed a "volume" as a 3D sibling of a panel. Reading the code argues against that: walls, slots, floating, the anchors' clusters, the placement forms, ids, versions, patches, the per-client limits, persistence, an environment's panels and the attention resources are all written in terms of `Panel`, and charts (#267), lists (#297) and images each joined as a field of it, with a body for older clients. A sibling would need each of those again, and the first divergence between two copies of "where the agent's panels go" would be a bug that only shows in a full wall. The cost of the panel kind is that some of a panel's assumptions, a face in one plane and text in cells, do not hold for a volume, and the stages below name each place where that matters.

## 3. The description

### The model

| Part | Fields | Bounds |
| --- | --- | --- |
| Diagram | title, id, place, metadata (as a panel's), `layout`, its direction or hub, nodes, edges, groups | at most 64 nodes in stage 1, 128 once measured; 128 edges, 192 later; 12 groups |
| Node | `id`; `label`; `shape`: `box`, `sphere`, `cylinder`, `prism` (hexagonal), `diamond` or `card`; `colour`, of the chart palette (teal, amber, blue, violet, green, red, yellow, grey, white); `size`, 1 to 5, by importance; `detail`; `ref`; `group` | id 1 to 32 ASCII letters, digits, `-`, `_` and `.`; label 32 characters; detail 240; ref 256 bytes |
| Edge | `from`, `to`; `label`; `direction`: forward, back, both or none; `style`: `solid`, `dashed`, `dotted`, `thick` or `flow` (pulses moving from its source to its target); `colour`; `weight`, 1 to 3 | label 24 characters; at most 4 edges between one pair of nodes |
| Group | `id`; `label`; `kind`: `cluster` (a tray its members stand on), `layer` (a floor, its groups stacked one above another) or `container` (a box drawn around its members); `collapsed`; `layout` within it; its members | nested at most 3 deep; a node in at most one group |
| Layout | `layered` with `down`, `up`, `right`, `left` or `away`; `radial`, about a hub node; `force`; `grid` | |

Everything is cleaned on one line as a panel's title is, without control or format characters, drawn as text and never as HTML. A `card` node is a small flat panel standing in the volume, facing the viewer, its label as its title and its detail as up to three lines below, for the "geometry like panels with expressed connections between them" the maintainer described; every other shape is a lit solid with its label beside it. A node's `ref` is the agent's own pointer back to what the node stands for, such as a crate's path or an environment's object id, which the agent is told when the node is pointed at and may `inspect`; Tomori never follows it.

Icons on nodes (a database, a queue, a person, a server, a file) are left to a later stage, drawn as shapes on the node's label rather than as new geometry.

### The block

A fenced block of language `diagram` takes a panel block's header, `title`, `id`, `place` and up to four keys of metadata, with `layout` added, ended by `---`, and then its body, a line each:

| Line | Means |
| --- | --- |
| `octree((Octree))` | a node, its shape from its brackets, as Mermaid's: `[ ]` box, `(( ))` sphere, `[( )]` cylinder, `{{ }}` prism, `{ }` diamond, `[[ ]]` card |
| `octree@{ size: 3, colour: teal, detail: "…", ref: "…" }` | a node's attributes, after it on its own line or its first mention, in the form of Mermaid's newer `@{ … }` node metadata |
| `a --> b`, `a --- b`, `a <--> b` | an edge, directed, undirected, both ways |
| `a -.-> b`, `a ==> b`, `a ~~> b` | dashed, thick, and a flow that moves (Tomori's own) |
| `a --> b : label`, `a -->|label| b`, `a -- label --> b` | an edge's label, in any of the three forms models write |
| `subgraph scene [Scene] … end` | a group, `cluster` unless its first line inside says `kind: layer` or `kind: container`; `direction TB` or `LR` inside it lays it out as Mermaid's does |
| `classDef hub size: 4, colour: amber` and `hub:::hub` or `class a,b hub` | shared attributes |
| `%% …` | a comment |

A node mentioned only in an edge is a box labelled by its id, as in Mermaid, so that a block's lines can be written in any order. The body is Mermaid-shaped, not Mermaid: a block of only boxes, plain edges and subgraphs reads the same in both, but Tomori's shapes, attributes and `~~>` are its own.

### The tools

`show_diagram` takes `title`, `id`, `place`, `meta`, `layout`, `nodes`, `edges` and `groups`, as JSON objects with the fields above, every object with `"additionalProperties": false` as every Tomori tool's is, or else `source`, a block's body as text, for an agent that prefers to write it once. It answers with the panel's identity, id, version, where it went, the layout's size, and every warning (below). `update_diagram` takes a `diagram`, by id, identity or title, an optional `version`, compare-and-swap as `update_panel`'s, and any of `add`, `change` and `remove` (nodes, edges and groups, by id), `layout`, `relayout`, to lay it out afresh rather than from where it stands, `collapse` and `expand`, by group, `highlight` (section 7), `title` and `place`. `panels`, naming a volume, reads it back as a block's body, so that the agent edits what is shown, not what it remembers writing.

### Worked examples

**Tomori's renderer modules**, layered from the client's frame loop down to what draws:

````text
```diagram
title: How a frame is drawn
id: renderer
layout: layered down
---
xr[client-quest: xr.rs]@{ colour: violet, detail: "Locates both eyes once a frame and submits one projection layer" }
desktop[client-desktop]@{ colour: violet }
subgraph scene [tomori-scene]
  step((Frame step))@{ size: 4, colour: teal, detail: "Pointing, carrying and the projector, once a frame" }
  faces[projection::faces]@{ detail: "Lays a panel out once for each version" }
  shapes[canvas and shapes]
end
subgraph text [tomori-text]
  atlas[(Glyph atlas)]@{ colour: amber, detail: "One distance-field atlas, 2,048 wide, up to 8,192 rows" }
end
subgraph render [tomori-render]
  renderer((Renderer))@{ size: 4, colour: teal }
  panels[ui_panel]
  occlusion[occlusion]@{ detail: "Writes each backing's depth where it is 60% opaque" }
  scenery{{Scenery}}@{ detail: "An environment's own drawing, such as the farmyard" }
end
xr --> step : eyes and input
desktop --> step
step --> faces : panels changed
faces --> shapes
shapes -.-> atlas : glyphs
step --> renderer : faces and models
renderer --> panels
renderer --> occlusion
renderer --> scenery : animate, prepare
scenery -.-> atlas : SharedGlyphs
renderer ~~> xr : both eyes' images
```
````

Laid out layered downwards, the clients stand on the top rank, the frame step below them, then `projection::faces` and the renderer side by side, and the atlas at the bottom, with the three crates as trays. The edge back up to `xr` is reversed for ranking and drawn as a flow, so the loop reads without a cycle in the ranks.

**The server, hub, MCP and OpenCode**, radial about the hub, as the same thing written through the tool:

```json
{
  "place": "beside",
  "title": "From a question to an answer",
  "id": "flow",
  "layout": { "kind": "radial", "hub": "hub" },
  "nodes": [
    { "id": "headset", "label": "Headset", "colour": "violet" },
    { "id": "listener", "label": "Client listener", "detail": "TLS; frames up to 32 MiB" },
    { "id": "hub", "label": "Hub", "shape": "sphere", "size": 5, "colour": "teal", "detail": "Spaces and connections under one lock" },
    { "id": "stage", "label": "Stage", "detail": "Walls, panels, revisions and patches" },
    { "id": "speech", "label": "Speech", "detail": "A sentence at a time" },
    { "id": "backend", "label": "OpenCode backend" },
    { "id": "opencode", "label": "OpenCode", "shape": "prism", "colour": "amber" },
    { "id": "session", "label": "Agent session", "shape": "sphere", "colour": "amber" },
    { "id": "mcp", "label": "MCP endpoint", "detail": "POST /mcp on loopback, with a bearer token" }
  ],
  "edges": [
    { "from": "headset", "to": "listener", "direction": "both", "label": "utterances" },
    { "from": "listener", "to": "hub" },
    { "from": "hub", "to": "stage" },
    { "from": "stage", "to": "listener", "style": "flow", "label": "patches" },
    { "from": "hub", "to": "backend", "label": "prompt" },
    { "from": "backend", "to": "opencode", "label": "HTTP API" },
    { "from": "backend", "to": "opencode", "style": "dashed", "label": "registers tools" },
    { "from": "opencode", "to": "backend", "style": "flow", "label": "events" },
    { "from": "opencode", "to": "session" },
    { "from": "session", "to": "mcp", "label": "tools/call" },
    { "from": "mcp", "to": "hub", "label": "acts on the space" },
    { "from": "hub", "to": "speech", "label": "reply" },
    { "from": "speech", "to": "listener", "style": "flow", "label": "agent speech" }
  ]
}
```

The hub stands in the middle; its neighbours on a ring about it, level; those two steps away on a second ring, raised a little, so that the path from a question out to OpenCode and back through MCP reads as a loop around the hub rather than a tangle on a page. Two edges between the backend and OpenCode are drawn side by side, curved apart.

**The farmyard's two sides**, layered from left to right, Tomori's programs either side of the environment's crates:

````text
```diagram
title: The farmyard's two sides
id: farmyard
layout: layered right
---
server[tomori-server]@{ colour: teal }
subgraph common [tomori-environment-farmyard]
  registration[[Registration]]@{ detail: "ID and VERSION, walls, anchors and the status wall" }
  side((Server side))@{ size: 4, colour: green, detail: "Owns the hens and the stones once let go" }
  flock[Flock and behaviour]
  beach[Beach stones]
end
physics[tomori-physics]@{ colour: blue }
subgraph client [tomori-environment-farmyard-client]
  clientside((Client side))@{ size: 4, colour: green }
  scenery{{Scenery}}
  stones[Stones drawn]
end
hens[(Hen model .glb)]@{ colour: amber }
headset[Headset]@{ colour: violet }
server --> side : runs it, with presence
side --> flock
side --> beach
beach --> physics : simulated once let go
side ~~> clientside : state and events
clientside -.-> side : intents
headset --> clientside
clientside --> scenery
scenery --> hens
scenery --> stones
registration -.-> server
registration -.-> headset
```
````

The `card` node for the registration is a small panel standing in the volume, with its detail on it, since it is a declaration rather than a running part.

### Validation and errors

The server reads a block or a tool's arguments into the model and checks it whole, before laying anything out:

- **Lenient by the line, strict by the bound.** A line of a block that does not parse is skipped and named; an edge to a node never declared makes that node, as Mermaid does; a second declaration of a node merges its attributes, the first of each kept. A diagram past a bound keeps what fits, in the order written, and drops the rest, saying how many: so a slightly wrong diagram is still shown, and its errors still reach the agent. A diagram with no nodes, or over 16 KiB of block or arguments, is not projected.
- **What the agent is told.** The tool answers with its warnings, each with its line where it came from a block, cut at eight and counted beyond: `line 7: "octre" is not a node; made a box called octre (did you mean octree?)`, `edges 129 to 140 left out: at most 128`, `group "gpu" nested 4 deep: its members put in "render"`. A block in a reply is projected when the reply completes, as a panel block is, and its warnings, where it has any, reach the agent as a note of Tomori's at the start of its next turn, as a timer's firing does, in square brackets and marked as data: `[Tomori: the diagram "renderer" was shown, with 2 warnings: …]`. The panel's metadata says `2 warnings` while it has any, so that participants know it may be incomplete.
- **Unknown values.** A shape, colour, style or layout Tomori does not know is the default (a box, grey, solid, layered down) with a warning, never a refusal, since a newer agent's habits should degrade, not fail.

Why the tool is the better path for a large diagram, and the block for a quick one: the tool's errors come back in the same turn, so the agent can fix them before saying anything, while a block's come back a turn later.

## 4. Layout

**On the server, once, and kept.** The server lays a diagram out as it is projected or changed, off the hub's lock, on a thread of its own, as code panels' highlighting is, within 50 ms, after which it falls back to a grid in the order written, saying so. The positions are sent with the diagram, so clients lay nothing out and every participant sees exactly the same positions, the spectator included. They are kept with the panel in the space's record, so that a restart, or a newer server with a better algorithm, does not rearrange a diagram someone has learned.

**Deterministic.** Each algorithm runs in `f64`, single-threaded, for a fixed number of iterations, over nodes in the order written, with no hash map's order and no random number: where a start is needed, each node's comes from a hash of its id. The same description at the same previous layout always gives the same positions, which the tests check against golden layouts.

**Units.** Positions are in the volume's own units, a node of size 1 being 6 cm across at a volume's scale of 1, nodes at least two of their widths apart; the volume's box is the layout's bounds with a margin, and is scaled to fit its slot or its floating size.

**The algorithms:**

- **Layered**, for pipelines, dependency graphs and anything with a direction. Sugiyama's method, as Graphviz's `dot`: cycles broken by reversing the fewest edges found by a depth-first walk in the order written; nodes ranked by longest path, then balanced; long edges given waypoints on the ranks they cross; crossings reduced by barycentre sweeps. Each rank is then a plane across the main axis rather than a line, which is where 3D helps: a rank of more than five nodes folds into two or three rows in depth, rather than spreading wide, so a broad dependency graph stays within reach and fewer edges cross as seen from in front. `away` ranks along depth, for a pipeline read from front to back.
- **Radial**, for a hub and its spokes, such as the hub above. The hub at the middle; nodes on rings by their distance from it in the graph, each ring a little higher than the last, so the rings do not hide one another from a viewer looking down; each node's angle the mean of its neighbours' on the ring within, so related spokes stay together.
- **Force**, for graphs with no main direction. Stress majorisation, which places nodes so that their distances in space follow their distances in the graph, in 3D, started from classical multidimensional scaling of those distances rather than a random start, so it is deterministic and converges in a few dozen iterations, then separated so that no two nodes' bounds overlap.
- **Grid**, for a set of things with few connections, such as a crate's modules: rows and columns in the order written, groups as blocks.

**Groups are laid out from the inside out.** Each group is laid out with its own layout, or its parent's, and then stands at the next level as one large node of its box's size, so that groups never overlap and their members stay together. A `layer` group's members are laid out in a level plane and its sibling layers stacked above one another in the order written, as the floors of a building, for a layered architecture. Edges between groups are routed between their boxes.

**Stable across small edits.** A diagram changed by its id is laid out from where it stands:

- every node already placed starts where it was, and is held there by a spring as strong as the rest of the layout's forces together, so that it moves only as far as the change needs;
- a node added starts at the mean of its neighbours' positions, or beside its group, and settles from there;
- layered layout keeps each rank's previous order as its tie-break and adds new nodes where the barycentre puts them;
- a change that would move any node more than a third of the volume's width says so in its warnings, with `relayout` to lay it out afresh where the agent wants that;
- a node a participant has dragged (stage 3) is pinned where it was let go, kept with the panel, and held there through every later layout until it is unpinned or `relayout` is asked for.

Clients ease each node from its old position to its new over 0.4 seconds, so that a change reads as growth rather than a jump. Two participants or the agent changing a diagram at once are ordered as any panel's changes are (section 5).

## 5. The protocol

- **`Panel::diagram`**, a `tomori_protocol::diagram::Laid`: the diagram's nodes, edges and groups as described, each node with its position, `[i16; 3]` in millimetres of the volume at scale 1, its size and its group; each edge with up to four waypoints, which a client curves through; each group with its box; and the volume's extent. Every type is non-exhaustive, with constructors, an unknown shape read as a box, an unknown style as solid and an unknown layout as nothing, since positions come with it. A diagram at every bound is under 32 KiB of JSON, and a typical one of 30 nodes about 5 KiB.
- **Versions.** A volume's version changes with its description, its layout, its collapsed groups and its place, as any panel's does, so that `update_diagram` with a version read before a participant opened a group is refused and read again.
- **Highlights are live data.** What the agent highlights changes often while it speaks, and must refuse nobody's edit, so it is not the version: it is the volume's live data, at its data seq, as a chart's points are (#289), sent to a client that applies data ops as `diagram_emphasis { panel, seq, nodes, edges, note }`, and whole otherwise.
- **Opening and closing a group** is a participant's message, `ClientMessage::SetGroupOpen { panel, group, open }`, which says the state wanted and no version, last writer wins, as `LockWall` does; the server lays the diagram out again and sends the panel whole. Visitors send none.
- **`Hello::diagrams`** says a client draws volumes. A client that does not say so is sent the volume as a text panel of its outline, and never `Panel::diagram`, so its frames carry nothing it cannot draw.
- **`Limits`** gains `volumes`, the most volumes a projection sends it, and `nodes`, their nodes together. The server states 32 and 2,048; Tomori's headset states 16 and 1,024 until measured. Volumes beyond a client's are sent to it as their outlines, the least recently placed first, as code panels are sent short. A volume counts as one panel against walls' slots, the floating bound and the panels in all, so no other bound changes.
- **An environment's volumes**, in stage 4, are published as its panels are, a `tomori_environment::Panel::diagram`, within its 32 panels.

## 6. Rendering

The rendering proposal (`proposal-rendering`) is being written beside this one; volumes are meant to fit it rather than add a path of their own:

- **A handful of solids, instanced.** Each shape is one unit mesh, a sphere of 80 triangles (320 near), a box, a cylinder, a hexagonal prism, a diamond and an arrowhead's cone, each drawn in one instanced draw for every volume in view, each instance a model matrix and a colour, lit simply from above with a rim, so that shape and depth read without shadows. Nodes are opaque bodies, drawn first with the swarm and the workers, writing depth (`rendering.md`).
- **Edges as ribbons from the shared stroke.** An edge is a strip along its curve, turned about its own axis to face the eye, sampling one column of the atlas's wedge across it as `Shape::Stroke` does, so that it is antialiased as a chart's line is and costs two triangles a segment; dashes and dots as a dashed chart line's; a flow as bright dots moving along it, placed in the vertex shader from a time, so that it costs the CPU nothing a frame. Arrowheads are the cone.
- **Labels from the shared atlas**, billboarded about the vertical to face each viewer, as workers' signs are, on a backing that writes depth where it is opaque, drawn among the translucent surfaces back to front. A `card` node is a small panel face, laid out once.
- **Built once for a version.** A volume's instances, ribbons and labels are built when its version or its data seq changes and kept, as a panel's face is; carrying, turning or scaling it moves one matrix; a highlight rewrites its instances' colours, as pointing recolours a panel's frame. Clients' easing between layouts rebuilds instances for its 0.4 seconds only.
- **Within a budget.** At the headset's limits, 16 volumes of 64 nodes, the solids are at most about 100,000 triangles and the labels about 40,000 vertices; a render test measures both on lavapipe and the frame log counts volumes drawn. Each volume has a bounding box in its own frame, which the rendering proposal's culling tests against each eye, and a level of detail by distance: beyond 4 m, or 4 m times its scale, its details and edge labels are not drawn and its spheres take the coarse mesh; beyond 10 m it is its title alone.
- **Nearer is in front.** Since nodes are opaque and labels write depth as other backings do, a hand, the menu or the pill in front of a volume covers it and a volume covers what is behind it, in both eyes, as every surface must.

## 7. Interaction and attention

**Pointing.** The ray meets a volume's nodes, by each one's bounds, its edges, within 1.5 cm or 0.5° of their curve, whichever is larger, its group labels and its tray, nearest first, as it meets any surface, and ends there. What it meets is highlighted and ticks, as a panel does; the label on the ray names it: a node's label, shape and detail, how many edges it has; an edge's ends and label; a group's label and how many it holds.

**What the agent is told.** As X is pressed, the client names what the ray is on with the volume's identity, as `Resource::panel` does, and the node, edge or group by its id, `{"kind": "panel", "panel": "panel-7", "node": "octree"}`, and the server describes it from its own copy, never the client's:

```text
Ada: [Pointing at node "projection::faces" (id faces) in diagram "How a frame is drawn" (id renderer, floating beside Ada, projected 4 minutes ago; box, in group "tomori-scene"; edges: from Frame step "panels changed", to canvas and shapes; detail: "Lays a panel out once for each version"; ref: crates/scene/src/projection/faces.rs)] when does this lay a panel out again?
```

An edge is named by its ends and label, `edge "Frame step → projection::faces: panels changed"`, and by its index among the diagram's edges, which the agent may name in `update_diagram`. With two rays shown, both are named, as two-handed pointing lists them. An older server ignores `node`, and describes the volume as a panel.

**The trigger and A.** On a node, they show its **focus**, for the participant alone, nothing of it sent: its neighbours and the edges between them stay bright, everything else dims to a fifth, and its detail stands on a small card beside it, so that someone can trace one module's connections through a crowded diagram. Again on the same node, or B, ends it. On a collapsed group, they open it for everyone (`SetGroupOpen`); on an open group's label, they close it. A participant's own focus is like their scroll: it never moves what anyone else sees.

**The grip** carries a volume as it carries a panel, by any of its nodes, edges or its tray; the right stick pushes, pulls and turns it, its click squares it, B throws it away, and it docks in a slot by nearness, as `projection.md` says. Moving a single node by hand comes in stage 3, by the maintainer's decision: with two-handed carry, one grip holds the volume and the other takes a node, the layout settling around the dragged node as it moves and pinning it where it is let go (section 4).

**The agent highlights while it speaks.** A highlight brightens the nodes and edges named, rings them, dims the rest to half and shows an optional note, for everyone. The agent makes one by linking to a node in what it says, as Markdown it already writes:

```text
The [frame step](diagram:renderer#step) asks the [projector](diagram:renderer#faces) for what changed, and only then hands the [renderer](diagram:renderer#renderer) its faces.
```

Speech reads the link's text, the transcript shows it, and Tomori takes each `diagram:` link out of the reply and attaches its highlight to the sentence that holds it, since speech is made a sentence at a time (`speech.md`): as that sentence begins playing on a client, its highlights show, and they give way to the next sentence's, or fade 4 seconds after the last. `#a,b` names several, and `#a->b` an edge. Without speech, as in brief mode or for someone who skipped it, a reply's last highlight is shown as it completes. `update_diagram`'s `highlight` sets one directly, and `highlight: []` clears it. Highlights are the space's, as speech is, so everyone hearing the sentence sees the same one; a client that is behind, replaying, shows the highlight of what it plays.

## 8. Placement and size

**The `place:` forms are a panel's.** `near`, `looking`, `beside`, `front`, `wall WALL`, `wall WALL slot N` and `anchor NAME`, in a block's header or the tools' `place`, work as `projection.md` says, with a panel's default, by the maintainer's decision: onto the wall whoever asked is looking at if they are near enough, as a relief in its slot, else floating beside and a little below them, as a model on a table (section 2). An `anchor` places it in the anchor's cluster, so that an environment's workbench can hold the diagram being discussed there.

**Its own size.** Stage 3 gives a volume a scale of its own, `Panel::scale`, the space's, from a quarter to six times, kept with it, changed last-writer-wins as placing is, and by the agent's `update_diagram` `scale`, or `size: desk`, `table` or `room`, 0.5, 1 and 5. Two hands on a volume (#354's stage 3) scale it by the spread between the grips, as they size a wall's slots, in steps of 5%, with a readout of its width. Grown to a room, it can be walked through: the floating bound holds it within 20 m of the space's middle, its labels grow with it, and its nodes become things to stand beside, which is the GitHub proposal's "stand in the crate graph" experiment. The floating panel scaling #354 left open is then answered for volumes; text panels can follow or not.

**Accessibility and reading distance.** A label's font pixel is 1.9 mm, a panel's, times the volume's scale and its content scale, so that from 1.3 m it is about one image pixel of the headset's, comfortable (`text.md`); its content scale is the wall's in a slot (#313), and its own floating, which `update_diagram` sets and stage 3's text handle drags. A label is never drawn smaller than half a standard slot's text, as seen from where the volume is read (`layout::LEAST_TEXT`): where labels at that size would overlap, the labels of the larger nodes are kept and the rest shown only as they are pointed at, so a volume shrunk into a slot stays legible rather than crowded. Colour is never the only signal: shape, label and line style each say what colour says, the palette is the charts', and a highlight is a ring and brightness as well as a colour. Flow's pulses are slow, about a node's spacing a second; whether a per-device setting stills them is an open question.

## 9. Bounds and costs

| Bound | Value | Why |
| --- | --- | --- |
| A block, or a tool's arguments | 16 KiB | twice a panel block's, for a diagram of 64 nodes with details |
| Diagrams from one reply | 2, among its four panels | a reply with more is a lecture |
| Nodes, edges, groups | 64, 128, 12 in stage 1 | legible in a floating volume at 1.3 m; raised once measured on a Quest 3 |
| Volumes shown in an environment | 32 | among its panels, within every bound panels have |
| Volumes sent to a client | its `Limits::volumes`, 16 on the headset | the triangle and label budget above |
| Layout time | 50 ms on the server's thread, then a grid | never under the hub's lock |
| Laid diagram on the wire | under 32 KiB | a 30-node diagram about 5 KiB; a highlight a few hundred bytes |

The costs are mostly new code rather than run time: a parser and a model in `tomori-protocol`, the layouts in a crate of their own, `tomori-layout`, on the server only, which #147's code toolkit shares, a pipeline of instanced solids and one of ribbons in `tomori-render`, and pointing at a volume's parts in `tomori-scene`.

## 10. Staged plan

Each stage keeps every test passing, changes nothing for a space without volumes, and is reviewable alone.

1. **Static volumes.** The model, the `diagram` block and `show_diagram`, with their warnings; layered and force layouts, deterministic, with golden tests; `Panel::diagram`, `Hello::diagrams`, `Limits::volumes` and `nodes`, and the outline for older clients; the solids, ribbons and labels, built once for a version, with the budget's render test; floating and in a slot as a relief; carried, docked, thrown away and kept as panels are; pointing that names nodes and edges to the agent; focus with the trigger. The agent's instructions gain a paragraph saying when a diagram helps (organisation, flows, dependencies), that it describes structure and Tomori lays it out, and to keep it under about thirty nodes.
2. **Diagrams that grow.** Updates by id laid out from where they stand, with easing; `update_diagram` and reading a volume back through `panels`; groups that collapse and open, and `layer` groups as floors; radial and grid layouts; highlights as live data, set by the tool and by `diagram:` links timed to speech.
3. **Size and hands.** A volume's own scale, desk to room, two-handed scaling with #354's stage 3, walking into a large one, the level of detail by distance, and the text handle for a floating volume's content scale; single nodes moved by hand with two-handed carry, the layout settling around a dragged node and remembering its pin.
4. **Richer content.** glTF models as nodes, by digest, from the asset studio's GLBs or an environment's assets, loaded under `assets.md`'s limits; 3D charts in a volume, bar towers over two categories and scatter in three dimensions, as a `chart3d` beside `diagram`; environments publishing volumes onto their walls and declaring **stands**, a workbench or a plinth on which volumes rest as panels rest on a notice board; and, only if the declarative form proves too limited, a programmatic one (section 11).

## 11. Later: a programmatic form

If structures turn up that a graph does not describe, such as a floor plan, a timeline in depth or a physical layout of racks, the escape hatch is a bounded list of instances of the same solids, each with a position, rotation, size, colour, label and id, in the volume's units, which the agent places itself: still no meshes and no shaders, still pointed at by id, still bounded by the same counts. It is deliberately left out until a real request needs it, since every diagram written that way loses layout, stability and the outline older clients read.

## 12. Costs and risks

- **Legibility in a headset is the real risk.** A diagram that reads well on a page can be a cloud of spheres in a headset, its labels overlapping as the head moves. Stage 1 should be tried on the Quest 3 with the three examples above before its bounds are raised, and the label rules (section 8) tuned there.
- **3D can be worse than 2D.** Depth hides nodes behind nodes, and a force layout in three dimensions has no natural viewpoint. Layered and radial layouts are recommended first for that reason, each with a front; the GitHub proposal's question, "is the 3D layout better than a 2D picture of the same subset?", is exactly what stage 1 is for.
- **A panel kind stretches some panel assumptions.** A face in one plane, text in cells and a mesh in `CARD_CAPACITY` do not describe a volume; the projector, faces and pointing need a branch for it. That is smaller than a second container, but it is not nothing.
- **Layout stability is easy to promise and hard to keep.** Holding old positions with springs can leave a grown diagram tangled; `relayout` is the agent's escape, and the warning tells it when one is due.
- **Timing highlights to speech** couples projection to the speech pipeline's segments, which no panel does today.

## Open questions for the maintainer

Decided (see [the maintainer's decisions](#the-maintainers-decisions)):

- **A panel kind or a sibling?** **Decided: a panel kind**, a panel with depth.
- **The syntax.** **Decided: both**, the Mermaid-shaped `diagram` block and the `show_diagram` and `update_diagram` tools, which report errors in the same turn. A plain `mermaid` block stays a code panel for now.
- **Default place.** **Decided: as panels go today**, onto the wall the asker is looking at if near enough, else floating beside them.
- **Who changes a diagram.** **Decided: participants move single nodes by hand** in stage 3, with two-handed carry, the layout settling around a dragged node and remembering its pin.

Still open, each keeping this proposal's recommendation until decided:

- **Focus and highlight.** Is a participant's focus right as their own, unseen by others, and the agent's highlight as everyone's?
- **Highlights in speech.** Are `diagram:` links in the agent's prose the right way to point while speaking, or should highlights come only from `update_diagram`?
- **Motion.** Should flow edges' pulses be stilled by a per-device setting, as the Feedback setting is kept on the device?
- **Bounds.** Are 64 nodes and 128 edges a sensible first limit, given that a diagram past thirty nodes is rarely a good explanation?
- **First example.** Which diagram should stage 1 be judged by on the headset: the renderer's modules, the server's flow, the farmyard's sides, or a crate graph from `cargo metadata` for the client split?

## Design documents this would change

`projection.md` (volumes as a panel kind: depth, size, a relief in a slot, floating as a model, the outline for older clients, `Panel::diagram`, `Panel::scale`, highlights as live data, `SetGroupOpen`, `Hello::diagrams` and `Limits`), `rendering.md` (instanced solids, ribbons, billboarded labels, a volume's mesh kept per version, its budget, bounds and level of detail), `interaction.md` and `responders.md` (pointing at a volume's parts, focus with the trigger, opening groups), `mcp.md` (`show_diagram`, `update_diagram`, `panels` reading volumes back, the agent told of nodes and edges, the instructions' paragraph), `speech.md` (highlights carried with the sentence that holds them), `text.md` (labels' least size in a volume), `spaces.md` (the diagram block's warnings as a note of Tomori's), `environments.md` (stage 4's published volumes and stands), `assets.md` (stage 4's models in volumes), and a new `design/diagrams.md` for the description, its bounds and the layouts.
