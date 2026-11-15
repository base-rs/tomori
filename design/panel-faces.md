# Panel Faces

**PROPOSED, not built.** This proposes **face panels**: panels an environment's server side publishes like any other, but whose inside its client crate draws, with Tomori's shapes and text and, where those are not enough, its own shaders. Tomori keeps everything else: the frame and title, the wall slot, carrying, resizing, pinning, copying, dismissal, the level of detail, the projection and the limits. Nothing below is a requirement until the maintainer accepts it, phase by phase, as was done for `responders.md` and `panel-actions.md`.

## Requirement

None yet. Everything under **Proposal** is direction, written as it would read once required. **What exists today** describes `origin/main` at `916c45ca` (#531).

## Status

The maintainer first asked in November 2026, in the headset, of Observe, a downstream environment whose own 3D panels had lost their look when they moved onto Tomori's walls: "is there some way in which the application can subclass panels and have more complex rendering behaviour or something?" The approved direction: Tomori owns the panel; the environment's client side draws what is inside it and is told where the ray points within it; each such panel has a text fallback for the agent and for clients without the code. The first draft of this document (#306, at `8585f66`) answered with faces over list rows, drawn only in Tomori's shapes.

The proposal was parked while rendering was reworked, and is revised here against what has landed since: 128 panels and 64 walls (#494); panels drawn as their titles when too small to read (#506); opaque faces drawn first with depth writes (#530); back faces culled (#531); the rule that no shader passes a uniform struct by value (#521, #525); eyes at 2.0× with 2× MSAA on the Quest 3 (#527); charts of 8 series (#511, #517); richer list rows (tones, values, columns, keys); the panel context model with chips, instances, pinning and freezing (#393 to #485); a spectator that runs client sides and draws panels; and two open proposals, refresh policies (#509) and compositor layers, parked (#529).

**The motivating example**, in the maintainer's words: Observe's **world clocks**. These are three 24-hour dials, lit by day and dark by night, each showing its area's traffic and trend. Today they are scenery, not panels, so adding a clock means writing code. As face panels they could be placed on walls, carried, copied (a clock per region), given context (a region parameter), pinned, and counted within the panel limits like any panel.

## Recommendation

**Add a fourth kind of published panel, `Panel::face`. It carries a key naming a drawing the environment's client crate registers, a small typed payload (bounded and versioned like a chart's live data), a text summary the server keeps as its body, and optionally rows as its pointable items.** The drawing has two tiers, both registered under the key:

1. **A canvas drawing**, required. It is a pure function in `tomori-scene`, without the GPU: the payload in, Tomori's `Shape`s, text and labels out, plus regions naming the items. It is meshed and kept like a chart's face, so its text is the atlas's and stays sharp. It is what tests check, and what every client that has the crate draws, the spectator included.
2. **A GPU layer**, optional. It is the environment's own pipeline, which renders into a texture Tomori allocates for the face, sized to how large the face appears, and renders again only when the payload, the size or the drawer's declared next change says so. Tomori draws that texture as the face's opaque backing, first in each eye with depth writes as #530 draws every face, and draws the canvas drawing's text over it. Clipping, curved walls, carrying and depth then come for free. The environment's shaders run only offscreen, never in Tomori's eye pass.

The fallback is always Tomori's: the title, then the summary as a text panel. This is what a client without the crate draws, and every client draws the title alone at a distance where it could not be read.

Pointing, choosing and inspecting reuse rows (`panel-actions.md`): a region is a row, so a press reaches the side as the server-made `Intent::choice`. No new intent path is added. A per-participant **cursor** overlay, like a chart's readout, lets a face show what is under the ray without being rendered again.

**For the clocks, tier 1 is enough.** A 24-hour ring is 24 `Shape::Quad` arcs, the needle a `Shape::Stroke`, the trend `Shape::Bar`s. The large figures need one addition: labels at 2 to 4 times the font pixel. So the clocks could move onto a wall with phase 1 alone. The GPU layer is phase 3, for faces that need gradients, glows or many thousands of primitives.

| Phase | What | What it gives Observe |
| --- | --- | --- |
| 1 | `Panel::face`: the key, payload and summary; the canvas drawing, kept and paced; the title and summary as fallbacks; scaled labels; regions as rows, with Tomori's hover outline; cursors; the agent told the summary and the item | World clocks on a wall of their own: carried, resized, pinned, copied by area, drawn by the spectator, counted in the 128; the attention list's own look |
| 2 | Face payloads as live data, under the refresh policies (#509); a drawer's declared next change, such as each minute; a glance level of detail | Clocks that tick each minute with no server traffic, and traffic updated each minute without changing the panel's version |
| 3 | The GPU layer: the environment's pipeline into a cached texture, within budgets of time and memory | Dials with a lit gradient of day and night; dense heat maps |
| Later | Faces that animate at a few frames a second; faces on text and chart panels; the client side told the item pointed at | Each the maintainer's call |

Richer list rows (tones, values, columns) remain the first choice where they are enough: they need no client code and are readable everywhere.

## What exists today

- **A face is a canvas.** `tomori_scene::projection::face` returns a `tomori_scene::canvas::Canvas`: rows of cells, `Shape`s (`Rule`, `Rect`, `Bar`, `Fill`, `Disc`, `Ring`, `Check`, `Quad`, `Stroke`, `Dot`), `Label`s placed in font pixels, an optional `CanvasImage` drawn from a texture, and a `Kind` such as `PanelFace`. `projection::faces` lays each face out once for its version and size, and the renderer keeps its mesh in a GPU buffer. A paced projector lays out at most two changed panels a frame (`faces::LAYOUTS`) and eight arriving (`faces::ARRIVALS`), the nearest first (`rendering.md`).
- **Faces are opaque and drawn first.** A canvas whose backing is wholly opaque (`occlusion::opaque`) has that backing drawn first in each eye, flat, unblended and writing depth (`ui_panel::Panel::draw_face`). Its text, shapes, frame and image are drawn later, blended, a few depth steps nearer (`occlusion::OVER`, `Panel::draw_over`). A panel carried (`Sheet::ahead`) is drawn whole in its turn, compared as though `occlusion::TIE` nearer. A wall marks only empty slots. A canvas an environment draws itself with `Panel::draw` is translucent (`rendering.md`, **How**).
- **Far faces are titles.** A face in a slot whose row of text subtends under 0.09° (`faces::FAR`) is drawn as its title alone, 3 times as large (`FAR_SCALE`), without laying anything out, until it subtends 0.108° again (`NEAR`). Up to two faces a frame come back to full, the nearest first (`NEARING`). `PanelLod` turns this off. While a face is a title, nothing of it is pointed at.
- **A chart is the nearest precedent.** `projection::chart::face` places Tomori's title, status row and rule over a `chart::Drawing` of shapes and labels. Its body is a summary for a client that cannot draw it. Its points are live data, versioned by `data_seq` apart from its version and sent as `chart_append` or `chart_replace` patches, so they never refuse an edit (`projection.md`). Its readout follows the ray for one participant, unsent.
- **Images are textures.** `panel_images` keeps decoded images within `TEXTURE_BYTES`, 128 MiB, mipmapped and sampled trilinearly, and evicts the least recently shown first. A canvas's `CanvasImage` is drawn over the canvas.
- **An environment's scenery draws in 3D** with its own pipelines: `Scenery::submit`, `cull`, `draw` (opaque, nearest first) and `draw_translucent` (before the panels). It builds text from `Target`'s `SharedGlyphs`. Its shaders must follow **No shader passes a struct or an array of a uniform by value**; `tomori_render::test_gpu::validate_wgsl`, behind `test-gpu`, checks a shader in an environment's own tests.
- **Rows** carry an id, text, a reference, `choosable`, a tone, a value, cells and a key. On the wire, `ListRow` carries `highlighted`, `tone`, `value`, `cells` and the key's revision. The rest, pointing by row id (`Resource::panel_row`), `ChoosePanelRow`, `Intent::choice`, `precedence::Trigger::Row`, A's row rung, `panels` and `inspect`, is in `panel-actions.md`.
- **Context and instances.** A side declares up to 3 parameters (`CONTEXT_PARAMETERS`) of up to 16 choices (`CONTEXT_CHOICES`). A panel `depends_on` them. Walls and panels hold settings, shown as chips. A copy in another context is an **instance** under a server-minted `~` id, at most 8 an environment (`stage::context::INSTANCES`), counted in the 128. The side is told `Intent::context()` and publishes the instance. Pulling a dependent panel off its wall pins a copy (`panel-context.md`).
- **The spectator** runs the client side registered for its environment and draws what the space projects, through the same panel buffers (`spectator.md`).
- **The Quest 3** draws each eye at 3,360 × 3,520 with 2 samples. In Observe's busy view, the panels' faces and walls' boards cost about 0.61 ms of a 10.75 ms eye time (the compositor-layers proposal, #529).

## Proposal

### The API

**Published.** `tomori_environment::Panel::face(id, title, Face)`, where:

- `Face::new(key)`: the key is 1 to 32 ASCII letters, digits, `-` and `_` (`valid_face_key`), as a panel id is, naming the drawing within the environment.
- `Face::with_data(json)`: the payload, at most 4 KiB of JSON (`FACE_DATA_LIMIT`, as an event's `EVENT_LIMIT`), the environment's own format.
- `Face::with_summary(text)`: what the server writes as the body, cleaned and bounded as any body (30 lines, 1,200 characters).
- `Face::with_items(rows)`: optionally, up to 30 `Row`s (`ROW_LIMIT`) as its pointable items, with ids, references, `choosable`, tones and keys as a list's rows have, but **not** written into the body. The summary says what matters about them.

Everything else a published panel has applies unchanged: `on_wall`, `with_reference`, `depends_on`, `for_window`, the 128, rates, resting, instances and pinning. `Panel::list(…).with_face(Face)` is the special case of a face drawn from a list's rows, its body the list as today. The server refuses, saying why, a key it does not take, a payload too large or not JSON, and a face on a chart or text panel.

**On the wire.** A projection's `Panel` gains `face: Option<tomori_protocol::Face>`, the key and the payload. Items travel as `rows`, as a list's do. A client gets `face` only if its `Hello` sets `faces` and it presents the environment at the space's version. Any other client is sent the panel as a text panel of the summary, as a chart is to a client that knows no charts.

**Registered.** An environment's client crate registers, beside its scenery and client side: `Sceneries::set_face(ID, key, drawer)`, where `drawer` is a `tomori_scene::face::Drawer`; and, for phase 3, `Sceneries::set_face_layer(ID, key, create)`, where `create` makes a `tomori_render::face::Layer` for the renderer's `Target`. A client draws a face only where both its build has the key and it presents the environment at the version the space is in, the rule client sides follow already. The key's meaning and the payload's format are agreed by the environment's protocol version (`environments.md`).

**The drawer** is given `face::Input` and returns `face::Drawing`:

```rust
pub trait Drawer: Send + Sync {
    /// What the face shows, in font pixels from its area's top left: shapes, rows, labels, and regions naming items.
    fn draw(&self, input: &Input<'_>) -> Drawing;
    /// What to show over it where the ray is, for this participant alone: a few shapes and labels. None by default.
    fn cursor(&self, _input: &Input<'_>, _at: Vec2) -> Option<Overlay> { None }
}

#[non_exhaustive]
pub struct Input<'a> {
    pub title: &'a str,
    pub data: &'a serde_json::Value,      // the payload, parsed once
    pub items: &'a [ListRow],             // ids, choosable, highlighted, tone, value, cells
    pub context: &'a [ContextChip],       // its effective settings, as its chips show them
    pub area: Area,                       // columns and rows of cells below Tomori's header, in font pixels
    pub scale: f32,                       // its wall's text scale, for a big wall
    pub detail: Detail,                   // Full, or Glance (phase 2)
    pub now: SystemTime,                  // the client's clock, for a face that shows time
}

#[non_exhaustive]
pub struct Drawing {
    pub rows: Vec<ui_text::Row>,
    pub shapes: Vec<Shape>,
    pub labels: Vec<Label>,           // gains a scale of 1 to 4: large figures, headline numbers
    pub regions: Vec<Region>,         // a rectangle and an item's id, at most one an item
    pub next_change: Option<SystemTime>, // when the drawing changes from time alone (phase 2)
}
```

`Area` is the rectangle of the face below Tomori's title, metadata or chips, and rule, at the size the panel is drawn (`Fit`): the "panel's rect" in the units every face is laid out in. The drawer is given nothing else: no ray, no buttons, no environment state, no GPU.

**The GPU layer** (phase 3) is a `tomori_render::face::Layer`:

```rust
pub trait Layer: Send {
    /// Encode the face into `target`, a texture Tomori owns, cleared to the panel's plate; `input` is the drawer's.
    fn render(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder,
              target: &FaceTarget<'_>, input: &face::Input<'_>);
}
```

`FaceTarget` gives the view, its size in pixels, its format (`Rgba8UnormSrgb`), an optional depth view of its own, the sample count (1 or 4) and the mapping from font pixels to its pixels, so that the layer lines up with the canvas drawn over it. The layer makes its pipelines once, as it is created at startup or on entering the environment, never in a frame. Its WGSL must pass `validate_wgsl` in the environment's tests, which enforces the uniform rule, and it shares the haze functions only if it wants them (a face is hazed as its panel is, by Tomori, after). It never sees the eyes, their cameras or Tomori's passes.

**How it is drawn.** Phases 1 and 2: the canvas drawing becomes the face's canvas, under Tomori's header and frame, with an opaque backing. It is drawn exactly as every face is: the backing first and writing depth, its text and shapes over it at `occlusion::OVER`, culled as panels are, carried with `Sheet::ahead`. Phase 3: the layer's texture **is** the backing. It is drawn in the opaque-first pass as a textured, unblended quad writing depth, in place of the flat colour (`Panel::draw_face` sampling instead of filling), so the GPU still rejects what lies behind the face before shading it. The canvas drawing's text and shapes are then drawn over it, as text is over any backing. A face is **opaque by default and only opaque** in this proposal: a translucent face would give up #530's early depth rejection and need sorting.

**Level of detail.** At `faces::FAR`, the face is its title and neither tier runs, and its texture is let go after a while. Phase 2 adds `Detail::Glance` between `FAR` and three times it, which a drawer may use to draw less: a clock's ring and time without its trend and words. Tomori owns the thresholds and their hysteresis, the same for every face. With `PanelLod` off, every face is drawn full.

### Data flow

- **The payload is part of the panel**, not the environment's shared state (`Space::share`). One publish carries the title, the summary, the items and the payload together, so the face and its fallback cannot disagree on what arrived. A face is kept by the panel's version and size, as any face is. A drawer works on the spectator, which reads the panel but not everything a client side reads. And one panel's data can differ from another's, which an instance needs.
- **Typed, by the environment's common crate.** The payload is JSON on the wire, as state and events are. The common crate gives a type, `serde` both ways, with a test that its largest value fits `FACE_DATA_LIMIT`. Tomori parses it once per version, never per frame, and hands the drawer the `serde_json::Value`, which the crate reads into its type, in the face's cache, not per draw.
- **Updates** (phase 2). The payload and the summary are the face's **live data**, as a chart's points are: a change of them alone takes a new `data_seq`, not a new version, so it never refuses a participant's or the agent's edit, and it reaches a client applying patches as `face_replace`, the payload and summary whole, about the payload's size. The title, items, key and placement are the version. Changes are held to the same pacing as every published panel (about one a second per panel), counted in the space's bytes.
- **Time** is the client's. A drawer that shows time, as a clock does, reads `Input::now` and says `next_change` (the next minute), and Tomori draws it again then, at most once a second a face, in its turn among `faces::LAYOUTS`. No server traffic is needed for the hands to move. The summary, which the agent reads, says the time at the side's last publish; the side publishes again as its own data changes.
- **Refresh policies (#509).** A face panel takes a refresh policy like any published panel: `live`, `every N min`, `on view` or `manual`. Under a policy that is not live, the server holds its live data until a refresh. A clock that ticks by `next_change` and whose traffic is refreshed every minute is `every 1 min` and does not count in the proposal's 32 live panels. A face's chip strip shows the `↻` chip as any panel's.
- **Context.** A face panel that `depends_on(["area"])` is told its binding in `Intent::context()` like any panel, and its side publishes the payload for that context. The drawer is also given its effective settings (`Input::context`), so it may print the area or window without the side repeating it in the payload.

### Interaction

- **The panel takes the ray**, as any panel: the frame, the title, the grip (carry), the right stick and its click while carried, B (throw away), holding B (leave), the wall's board and handles are all Tomori's, unchanged. The projector carries the point the ray meets through the panel's model into the face's area, as `item_in` does for a body line.
- **Items are rows.** The first `Region` containing the point names an item, which becomes the row pointed at (`Conversation::pointed_row`): `Resource::panel_row` names it by its id, the server describes it from its own copy, a choosable item is chosen by the trigger at `precedence::Trigger::Row` and by A at the row's place among the conversation's meanings, `ChoosePanelRow` carries it with its key's revision, and the side is handed `Intent::choice`. No rung or message is added. A face cannot make an item choosable: the server checks its own copy.
- **Hover** is Tomori's. Tomori outlines the region under the ray in its highlight colour by rewriting vertices of the kept mesh, as a frame's colour is recoloured (`Canvas::frame_mesh`), so hovering calls no drawer, renders no texture and cannot be hidden. The ray's label ends `A  choose` and then `✓  chosen`, as for any row.
- **Cursors.** Pointing at a face whose drawer has `cursor` shows, for that participant alone and unsent, an overlay of at most 64 shapes and 4 labels over the face, as a chart's readout is drawn. The drawer is asked again only as the point moves a font pixel. A clock's cursor might show, at the hour pointed at on its ring, that hour in that city and in the participant's own zone. While a cursor shows, the ray's label does not.
- **Pressing X while pointing** tells the server what the participant points at. On an item, it is the row, as now. Elsewhere on a face, it is the panel, with the point across and down in ten-thousandths, `{"kind": "panel", "panel": "panel-3", "at": [6000, 2500]}`, which the server cannot read itself but hands, with the reference, to the side's inspector when the agent inspects (below).
- **Hands, the desktop, the spectator, visitors.** Hand tracking points and hovers, and chooses only once a pinch is the trigger. In the desktop window the mouse points and the left button and Enter choose. The spectator points at nothing, so it shows no hover or cursor. A visitor points and chooses nothing.

### The agent

- **The summary is what the agent reads**, as a chart's is. Pointing at a face panel, the note before the participant's words gives its title, its environment, its id and its summary's first line, cut to 200 characters (`DESCRIPTION_LIMIT`), as an object's description is. `panels` gives its key, its summary, its items as a list's rows are given (`id`, `inspect_id`, `highlighted`, `choosable`), its context and its refresh. `inspect` on the panel's reference asks the side's inspector, as now, and passes the point last pointed at, if any.
- **Keeping it truthful is the common crate's job.** Tomori cannot read a payload. The recommended pattern, which the phase 1 example and test helper follow: the common crate gives `fn summary(&Data) -> String` beside the type, the server side writes the body with it, and a test in the common crate checks that each figure the drawer shows appears in the summary. `face::test_support::covers` checks that every item has a region and that every item's text appears in the drawing.
- **The fallback is always there.** A face never replaces the summary: the agent, older clients and builds without the crate all read the same text.

### Fallbacks

| Client | Draws |
| --- | --- |
| Has the key, at the space's version | The face, tier 1, and tier 3 where registered |
| Has the crate but not tier 3's layer, or the layer failed | Tier 1 alone, on a flat backing |
| Has no drawer for the key, or an older environment version | The title and the summary as a text panel; items not drawn; pointing describes the panel |
| An older client, without `faces` | Sent the panel as a text panel of the summary |
| Any client, at `faces::FAR` | The title alone, 3 times as large |
| The spectator | As a participant's client with the same sceneries; no hover or cursor |

### Limits and cost

- **Bounded in.** A key, 4 KiB of payload, the summary as any body, at most 30 items, all counted in the space's bytes with the panel. A face panel is one of the 128 and, under #509, one of the 32 live ones while live.
- **Bounded out (tier 1).** At most 1,024 shapes, 1,024 cells of text and labels, 30 regions, every number finite, meshed with Tomori's header within the most a panel's buffer grows to (`scene_graphics::CARD_MOST`). What lies outside the area is left out whole. A drawing past a bound is refused whole and the summary shown, logged once for the key.
- **Draw time.** A drawer runs only when its panel's version, data, size, detail or `next_change` says so, in its turn among `faces::LAYOUTS` two a frame and `ARRIVALS` eight. A chart's face takes 0.2 to 1.3 ms to lay out and mesh on a desktop and several times that on a headset (`rendering.md`), so a face is held to a chart's cost: 1 ms on a desktop for the largest drawing. Tomori logs any draw over 4 ms on the headset and drops a drawer after three such draws, showing the summary. A frame that changes nothing calls no drawer and allocates nothing.
- **Legible.** Text is the atlas's, at the panel's font pixel (1.9 mm on a standard wall, scaled with its wall), or at 2 to 4 times that for a scaled label. A drawer has no smaller size, and a layer's own text, rasterised into its texture, is discouraged in favour of the canvas drawing's.
- **GPU cost in the eyes** (all tiers). A face covers the same pixels as the flat backing it replaces. Tier 1 adds its shapes to a mesh drawn in the panel's one draw, as charts do today; at 2.0× with 2×, the panels' share measured in Observe's busy view was 0.61 ms of 10.75 ms, and a dozen faces of a chart's density should stay within the same order. Tier 3 turns a flat backing into a trilinear texture sample per pixel, one more draw a face, at 3,360 × 3,520 with 2 samples; it is to be measured with the `GPU over` line, as #529 measured panels.
- **GPU cost offscreen (tier 3), cached.** A layer renders only when tier 1 would draw again, and at most one layer a frame, the nearest first. Its texture is sized to the face's angular size, in steps of 256, 512 and 1,024 pixels along its longer side, chosen with hysteresis: a standard slot read from 3 m spans about 11°, some 290 of the display's pixels, so 512; carried at arm's length, 1,024. A 1,024 × 640 texture is 2.5 MiB, 3.3 MiB with mipmaps. All face textures together are held to 48 MiB (`FACE_TEXTURE_BYTES`), within the device memory the eyes leave (`rendering.md`), evicting the least recently drawn, and a texture is let go once its face is a title for 900 frames. Each layer's pass is timed by the GPU profiler, as a part of its own (`faces`), and a layer whose pass takes over 2 ms three times is dropped for the session, its face drawn by tier 1.
- **Caching tier 1 in a texture**, too, was considered. It would save little: a kept mesh costs no CPU and a few thousand vertices of GPU, and a texture would blur its text and cost memory. It is not proposed.
- **What an environment cannot do.** Draw outside its face: tier 1 is cut to its area, and tier 3 draws only into its own texture. Draw over Tomori's frame, header, chips, hover, cursor label or anything else of Tomori's, which is drawn over or beside it. Take a control: a face is told no button, stick or ray. Make a press mean anything but the choice of an item it published, which the server checks. Draw something that looks like an approval: Tomori asks for no decision on a wall panel, a request takes A before any row, and the title row, which Tomori draws, says whose panel it is. A drawer or layer that panics is caught, never called again in that client, and its faces fall back, with one warning, as avatars and worker placements are (`avatars.md`, `environments.md`).
- **The one risk not contained.** A layer whose pipeline the driver refuses can lose the device for everything, as #414 did. This is why layers make their pipelines at startup, inside an error scope, and why their shaders must pass `validate_wgsl` in the environment's tests. A pipeline that fails there disables the layer before any frame.

### Worked example: Observe's world clocks

**Today.** `clocks.rs` in Observe's client crate makes each clock from `Block`s, instanced boxes drawn by the scenery's blocks pipeline (the plate, its rim, 24 hour arcs coloured by `hour_colour`, 4 ticks, a needle standing proud and 30 trend bars), and `Label`s for the area, the time (8.6 cm figures), the zone and daylight, the traffic and its detail. They stand at `Placement::OVER_GLOBE`, picked by hit plates (`clocks::pick`). The server side shares `Clocks` with each snapshot, a few hundred bytes. The client derives each `Face` from it and its own clock, again each minute (`ClockView`). The server describes each clock as an object (`clock/europe`) with `describe_clock`, inspected by the same reference. They are scenery because the side then published 31 of the 32 panels it could, and a chart's title is too small to read a time across the room.

**As face panels**, phase 1:

- **Common crate.** `clocks::FaceData { area, minute, step, per_minute, reporting, regions, trend }`, the per-area part of `Clock`, read back within 600 bytes as it is tested today. `describe_clock` moves here, as `clocks::summary(&FaceData, now) -> String`, so that the server's summary and the drawer use one source.
- **Server side.** It declares a context parameter `area` (Americas, Europe, Asia-Pacific). It publishes one panel per area, `Panel::face("clock-europe", "Europe · 13:00", Face::new("world-clock").with_data(…).with_summary(summary(…))).depends_on(["area"]).with_reference("clock/europe")`, on a new declared wall, `Clocks`, three portrait slots over the globe where the band stands (an environment declares its walls' slot shape, curve and name, #296). The title carries the local time, so that the far level of detail still reads `Europe · 13:00`. This costs 3 title updates a minute, within the per-panel pace. Its inspector answers `clock/europe` as now (`inspected_clock`). The three clocks leave `Space::describe`'s objects, which frees 3 of its 32 for pods.
- **Client crate.** `blocks()` and `labels()` become one `Drawer::draw` in the face's area, in font pixels: the plate is the panel's backing; the ring is 24 `Shape::Quad` arcs, each a little short of its hour; the ticks are `Shape::Rule`s; the needle is a `Shape::Stroke` (flat now, no longer standing 12 mm proud); the trend is `Shape::Bar`s on one scale, a gap a stub in `colour::GAP`, muted when stale; the time is a `Label` at 4 times the font pixel in `time_ink`, the zone and daylight, traffic and detail at 1 or 2 times. `next_change` is the next minute. `cursor` shows the hour pointed at on the ring in the area's city and in UTC. `clocks::pick`, `Placement`, the clocks' `Block`s and their box in `Environment::with_solids` go: Tomori picks panels, and walls are kept clear by Tomori already.
- **Tests.** The `clocks/tests.rs` checks of the ring, needle, trend and words move onto `Drawing` with no GPU. The `clocks.png` renders become a face drawn on a wall in the existing scenery tests. `covers` checks the summary against what is drawn.

**What Observe gains.** The clocks are carried, resized, put on any wall, pinned and thrown away by anyone, like any panel. **More clocks without code**: `copy_panel` with `area` set, or a participant pulling one off (which pins a copy), makes another clock as an instance, up to 8. The side may publish one for every production region, within the 128, without a new pipeline or placement. The spectator draws them. Clients without Observe's crate show `Europe · 13:00` and the summary. The agent reads exactly what it read before.

**What Observe gives up.** The ring's relief and the needle standing proud: a face is flat. Phase 3's layer could shade a relief, or the slight depth could be dropped. The band's exact placement and spacing become a wall's slots.

**A clock per region, not per area.** `CONTEXT_CHOICES` is 16 and `INSTANCES` is 8, while Observe has up to 24 production regions. Per-region clocks would be published by the side, as panels of their own, rather than copied by context: see Open questions.

### Phasing

1. **Face panels, tier 1.** `tomori-environment`'s `Face`, `Panel::face` and `with_face`; `tomori-protocol`'s `Panel::face` and `Hello::faces`; `tomori-server`'s `stage/published.rs` keeping and checking it, the body from the summary, and `hub/agent_panels.rs`; `tomori-scene`'s `face` module (`Drawer`, `Input`, `Drawing`, `Region`, `Overlay`, `test_support`), `projection::faces` drawing through it, regions as rows, hover outlines, cursors; `Label`'s scale; `Sceneries::set_face`. A fixture face in the tests' queue environment. This is roughly the size of choosing a row (#298, about 2,200 lines over 33 files), most of it tests.
2. **Live faces.** `data_seq` and a `face_replace` patch op; `next_change`; `Detail::Glance`; the refresh policies of #509, once those are accepted.
3. **GPU layers.** `tomori-render`'s `face::Layer`, `FaceTarget`, the texture cache, the textured opaque backing in `Panel::draw_face`, the profiler's `faces` part, the time and memory budgets. To be measured on a Quest 3 at 2.0× with 2× before its numbers are fixed.
4. **Later**, each the maintainer's call: faces animated at up to 10 frames a second into their texture; faces on text and chart panels; the client side told the item pointed at; a region taking the right stick.

### Risks

- **Device loss from an environment's shader** (phase 3), contained only by tests and startup creation; see Limits.
- **The summary drifting from the face.** Tomori can check items, not payloads; the common-crate pattern mitigates this but cannot enforce it.
- **Draw cost on a crowded wall.** 128 faces at a chart's density, arriving together, ripple in over a fraction of a second as charts do; the far level of detail keeps distant ones free. Phase 1 must measure it with `panel_frames`.
- **A second way to draw environment content.** Faces and scenery overlap; the rule is that what should be placed, carried, copied or counted as a panel is a face, and what belongs to the room is scenery.

### Rejected

- **Drawing in Tomori's eye pass inside the panel's rectangle**, with the environment's pipelines each frame. A scissor is screen-aligned and differs per eye; the depth buffer is `Depth32Float` without stencil; clip planes are optional; and #530's opaque-first order and the text depth bias would have to be opened to each environment, per panel and per eye, at 3,360 × 3,520 with 2 samples. A texture gives all of that for free, at the cost of rendering only when the face changes.
- **Caching every face in a texture**, including tier 1: blurred text and memory, for little saved.
- **Quad layers per face**: parked with all panels in #529, and contrary to **One composition layer**.
- **HTML or a webview**: panels are never HTML (`security.md`), and a browser on the headset is out of proportion.
- **Drawers sent by the server**, as scripts or WebAssembly: environments are compiled into clients by design (`environments.md`).
- **The drawer reading the environment's shared state**: two sources arriving apart, unusable by instances and the spectator.
- **Regions acting through the client side's own intents**: forgeable; rows' server-made choices already exist.

## Rationale

The maintainer asked for panels an application can "subclass". In AppKit a subclass overrides `draw` and inherits the window and its event routing. Here the panel is the window, and the face overrides drawing alone. Tomori already has the surface for it: every face is a canvas, and a chart is a custom face in all but who writes it. Tier 1 reuses that surface, so faces are sharp, cheap, kept, and testable without a GPU.

The motivating clocks need almost nothing beyond it. Their value is in becoming panels, so they can be placed, copied with context, pinned and counted, rather than in new rendering. Tier 3 answers the request for an environment's own shaders while keeping them where they cannot break Tomori's order: offscreen, into a texture that becomes the face's opaque backing as #530 already draws every backing.

Making items rows, rather than targets of the face's own, keeps the costs the maintainer was told of small. The agent reads what the server holds, clients without the drawer still list and choose the items, and every press reaches the side as #298's attested choice, in orders that took many reviews to settle and that this leaves alone.

## Scope

This would cover `tomori-environment`'s panel and limits, `tomori-protocol`'s `Panel`, `Hello` and patch ops, `tomori-server`'s `stage/published.rs`, `hub/agent_panels.rs` and pointing descriptions, `tomori-scene`'s `face`, `canvas` (`Label`'s scale), `projection` and `projection::faces`, `tomori-render`'s `Sceneries`, `ui_panel` and, in phase 3, `face` and the profiler, and the builders. It would change the text of `environments.md`, `projection.md`, `panel-actions.md`, `panel-context.md`, `rendering.md`, `builders.md`, `mcp.md`, `spectator.md` and `security.md`. Phase 1 leaves `precedence`, the server's handling of choices and the renderer's pipelines untouched.

## Validation

For this proposal: check each statement under What exists today against the code it names. For each phase, once accepted:

- a drawer tested offscreen and deterministically, from a fixture `Input` to a `Drawing` compared whole, at every slot shape, floating, carried, at `Glance`, and from 0 to 30 items, against the bounds, with `covers`;
- `projection::faces`' counts: nothing drawn across frames that change nothing, one draw for a new version, data seq or `next_change`, the summary for an unknown key, a refused drawing, a panic;
- `tomori-render`'s `ui_panel::tests` drawing a fixture face on lavapipe, flat, on a curved wall's slots and 4 m away, with a PNG preview, as charts are; and for phase 3 a fixture layer whose texture lines up with its canvas within a font pixel, drawn opaque first, hidden behind a nearer panel and drawn over the wall behind it;
- complete region coverage (`testing.md`);
- on a Quest 3: Observe's clocks legible on their wall from the bridge; carried, copied by area and pinned; the needle moving each minute; the title reading far away; and the frame log's CPU and GPU lines with and without a wall of faces.

## Open questions

- Should a face be drawn below Tomori's title, metadata or chips, and rule, as proposed and as a chart is, or fill the whole frame, which a clock's look might prefer but which gives up the row that always says whose panel it is?
- Should per-region clocks be copies? That would need more than 8 instances (`INSTANCES`) and more than 16 choices (`CONTEXT_CHOICES`) for face panels, or an environment action, "add a clock for Iowa", by which the side publishes one.
- Should the payload be live data from phase 1, rather than waiting for phase 2?
- Are scaled labels of 2 to 4 times enough, or should a face set any size above the font pixel, for a single huge figure?
- Should faces animate (phase 4), and at what rate: a pulsing alert, a needle sweeping seconds?
- Should faces be translucent where an environment asks, giving up early depth rejection for a face that floats over the room?
- Is the agent told the point pointed at on a face outside an item, through the side's inspector as proposed, or not at all?
- Should a drawer's colours be free, or Tomori's palette and tones, so that every environment's faces read alike?
- What are the right budgets (4 KiB, 1,024 shapes and cells, 4 ms and 2 ms, 48 MiB, one layer a frame)? Each phase should measure them on the headset.
- Should the client side be told the item pointed at, to light the matching pod on the globe as a clock's area is pointed at?
