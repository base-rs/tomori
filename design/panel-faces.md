# Panel Faces

**PROPOSED, not built.** This proposes how an environment gives a panel it publishes a look and controls of its own, a **face** its client crate draws, while the panel stays Tomori's: its frame, its slot on a wall, carrying, resizing, dismissal and the projection. Nothing below is a requirement until the maintainer accepts it, increment by increment, as `responders.md` and `panel-actions.md` were.

## Requirement

None yet. Every statement under **Proposal** is direction, written as it would be required once accepted; the sections **What exists today** describe the code as it is at `8585f66`.

## Status

Asked for in November 2026 by the maintainer, in the headset, of a downstream environment, Observe, whose own 3D panels, an attention list with a scrolling highlight, an inspector with bars and toggles and a paged system wall, were moved onto Tomori's walls as published panels and lost their look and their controls: "With regards to these walls, how do we make them more dynamic? … I understand the panel is a totally different implementation. But is there some way in which the application can subclass panels and have more complex rendering behaviour or something?" The direction the maintainer approved: Tomori keeps owning the panel's frame, its wall slot, grabbing, moving, resizing, dismissal and the projection; the environment's client side draws what is inside the panel and is told where the ray points within it; the agent reads only what the environment describes in text, so each such panel needs a text fallback, which clients without the environment's client code show. Richer built-in list rows, tones, value bars, a hover and a highlight bar (`panel-actions.md`, increment 4), are being built beside this; this proposal is for what built-ins cannot cover.

## Recommendation

**A face is a presentation of a list panel's rows, drawn by the environment's client crate as a 2D drawing in Tomori's own shapes and text, and never a second source of data.** The server side publishes a list panel exactly as today, `Panel::list(id, title, rows)`, and names a face, `with_face(Face::new("attention"))`. The body the server writes from the rows stays the fallback, what the agent reads and what every client without the drawer shows. A client whose build registered a drawer for that key draws the face instead: a pure function from the panel as published to shapes, text and **regions**, each region one of the panel's rows. Pointing at a region is pointing at that row, and choosing it is choosing that row, by the messages and rungs rows already have. So the face adds a look, and no new input path, intent, forgery surface or thing the agent cannot read.

Of the three ways to draw, this is (b) below in the form the code already has, a `Canvas` display list rather than a texture, fed by (c)'s declarative data. Arbitrary 3D inside a panel, (a), is not recommended.

| Increment | What | What it gives Observe |
| --- | --- | --- |
| 0, in progress elsewhere | Built-in richer rows: tones, value bars, hover and highlight bars | The attention list's highlight and bars on every client, without any client code |
| 1 | Faces over list panels: drawn, bounded, contained; regions pointed at and chosen as rows; hover outlined by Tomori | The attention list's own layout and look, choosing a row to open it in the inspector; the inspector's bars, and its toggles as choosable rows drawn as switches; the system wall's pager as choosable rows, shared by the space |
| 2 | Each participant's scroll of a face, by rows or pages, with Tomori's scroll bar and `Stick ↕  scroll` | The highlight kept in view as the list scrolls; paging the system wall for oneself |
| 3 | The client side told the row pointed at; a region that takes the right stick | The pod of the row pointed at lit on the globe; a value adjusted with the stick |
| Later | Animation; faces on text and chart panels; the spectator | Each the maintainer's call |

Where increment 0's rows are enough, an environment should use them and no face: they work on every client, need no code, and are legible and agent-readable by construction. A face is for layout the vocabulary cannot say: columns and badges, a sparkline in each row, gauges, a heat map, a topology.

## What exists today

- **A panel's face is already a 2D drawing, not a texture.** `tomori_scene::projection::face` returns a `tomori_scene::canvas::Canvas`: rows of `ui_text` cells, `Shape`s (`Rule`, `Rect`, `Bar`, `Fill`, `Disc`, `Ring`, `Check`, `Quad`, `Stroke`, `Dot`) and `Label`s placed in font pixels, which `Canvas::mesh` turns into quads of the shared glyph atlas, with no GPU. `projection::faces` lays each face out once and keeps it, the renderer keeps its mesh in a buffer of `CARD_CAPACITY`, 12,000 vertices, and a paced projector lays out at most two changed panels a frame (`faces::LAYOUTS`) and eight arriving (`faces::ARRIVALS`) (`rendering.md`). A curved wall's slots are each flat, so its curve is in the panels' models, never in a face.
- **The chart panel is a custom face Tomori draws.** `projection::chart::face` puts Tomori's title, status row and rule over a `chart::Drawing`, shapes and labels in an `Area`; its body is a text summary for a client that cannot draw it (`projection.md`). A face generalises this: the drawing comes from the environment's crate.
- **An environment's client side draws in 3D.** Its `ClientSide::update` returns a `Drawn` that its `Scenery::animate` reads, and the scenery draws with its own pipelines in `draw` and `draw_translucent`, the latter after the scene and before the panels, building text from the `Target`'s `SharedGlyphs`. It is told `Input::pointed`, `Input::select` and `Input::scroll` for what its `pick` returns as `Picked`, `selectable()` or `handles(Actions::SCROLL)`, only after every rung of Tomori's (`environments.md`, `responders.md`).
- **The ray takes a panel before the client side**, so a client side cannot pick anything on a panel (`interaction.md`). The projector finds the panel pointed at and where (`projection::Output::pointed`), and `item_in` turns that point into a body line through the panel's model.
- **Rows already have identity, pointing, inspection and choice.** `Row::new(id, text)`, `with_reference`, `choosable`, `Panel::highlighting`; the projection's `Panel::rows` of `ListRow`; `Resource::panel_row`; `ClientMessage::ChoosePanelRow`; `Intent::choice` and `Chosen`; `precedence::Trigger::Row` and the row's place in A's order; `panels` and `inspect` for the agent (`panel-actions.md`).
- **The spectator runs no client side** (`environments.md`), and, as far as reading `tomori-client-spectator` shows, draws no projected panels at all: it never applies the frame step's sheets. See Open questions.

## Proposal

### The model

- **Published as a list, with a face named.** `tomori_environment::Panel::with_face(Face)` on a list panel; `Face::new(key)`, the key 1 to 32 ASCII letters, digits, `-` and `_` (`valid_face_key`), as a panel's id, and optionally `Face::with_state(json)`, at most 4 KiB of JSON (`FACE_STATE_LIMIT`, as an event's), for what the drawing needs that is not a row: which columns to show, a sparkline's points. Everything else is unchanged: its id, title, rows, highlight, references, wall and slot, its place among the environment's 32, its rates and its resting. The server still writes the body from the rows, and refuses a face on a panel that is not a list, a key it does not take, and a state too large or not JSON, saying why.
- **On the wire.** A projection's `Panel` gains `face`, a `tomori_protocol::Face` of the key and the state, part of what the panel's version covers, sent only to a client whose `Hello` sets `faces` and lists the environment at the version the space is in. An older client is never sent it and draws the body.
- **Drawn by a registered drawer.** An environment's client crate registers, beside its scenery and client side, `Sceneries::set_face(ID, "attention", drawer)`, a `tomori_scene::face::Drawer`: `fn draw(&self, input: &face::Input) -> face::Drawing`. It is in `tomori-scene`, free of GPU code, so every client that draws panels can run it.
- **What it is given**, `face::Input`: the panel's title; its rows, each with its id, its text as the server cleaned it, whether it is choosable and highlighted, and its tone and value as increment 0 sends them; the face's state; and its area, the columns and rows of cells below Tomori's title, metadata and rule, in font pixels, at the size the panel is drawn (`Fit`). Nothing else: no clock, no ray, no buttons, no environment state.
- **What it returns**, `face::Drawing`: `ui_text::Row`s on the cell grid, `Shape`s and `Label`s in font pixels from its area's top-left, as `chart::Drawing` has, and `face::Region`s, each a rectangle and the id of one of the panel's rows, at most one a row.
- **Kept, not redrawn.** `projection::faces` keeps the drawing as it keeps any face, by the panel's version and size. The drawer runs when the server changes the panel, in its turn among `faces::LAYOUTS`' two a frame, and when the panel is sized anew; a frame that changes nothing calls no drawer and allocates nothing.
- **Why the panel alone, and not the environment's shared state.** A drawer could read the state `Space::share` keeps instead. It would then draw from two sources that arrive apart, the projection's patch and the state channel, so a face could show what its fallback does not; it would need the stateful `ClientSide`, which only the joined participant's device has; and it could not be kept by the panel's version. One publish carries the rows, the body and the state together.

### The alternatives, compared

|  | (a) The scenery draws in 3D in the panel's rectangle | (b) A 2D texture mapped onto the panel | (b′) A 2D drawing in `Canvas` shapes and text, **recommended** | (c) A declarative vocabulary Tomori draws |
| --- | --- | --- | --- | --- |
| Clipping | Not guaranteed. A scissor is screen-aligned and differs for each eye; the eyes' depth is `Depth32Float`, without a stencil; clip planes are an optional GPU feature; else the environment's shader is trusted to discard | Free | By construction: what leaves the area is left out | Free |
| Depth and order | `draw_translucent` runs before the panels, so a face would lie under its panel's 94% backing; needs a new hook for each panel, in each eye, and its own depth writes (`rendering.md`) | Free, as an image panel's quad | Free: the panel's own mesh, one draw | Free |
| Curved walls | Each slot's model handed to the scenery every frame | Free | Free | Free |
| Text | The environment's own, at any size | Blurred: pixels, not distance fields; needs mipmaps and anisotropy, as `panel_images` | The atlas's, sharp, at the panel's one size | The same |
| Cost | Pipelines and draws of its own, for each face, each eye | About 2.5 MiB for a 1024 by 640 face, 3.3 MiB with mipmaps, 32 of them most of `panel_images::TEXTURE_BYTES`' 128 MiB; an upload at each change; and Tomori has no 2D rasteriser, so the environment brings one | A chart's: about 0.2 to 1.3 ms to lay out and mesh on a desktop, once for each change (`rendering.md`); no texture | The least |
| Legibility checked | No | No: Tomori sees pixels | Yes: text has no size to choose | Yes |
| Tested without a GPU | No | Partly | Yes | Yes |
| Clients without the code | Fallback | Fallback | Fallback | Every client, where the vocabulary is drawn; the body elsewhere |
| What it can show | Anything | Any image | Anything the shapes draw: lines, bars, boxes, dots, areas, text | What the vocabulary names |

(c) beats custom drawing whenever it can say what is wanted, which is why it comes first and why (b′) takes its data from it: rows, tones, values, toggles and pagers stay declarative, on every client and readable to the agent, and only the drawing is the environment's. Tomori's own list face could become the default drawer, so that both share regions, hover and scrolling; that is left to whoever builds increment 0.

### Input

- **The panel takes the ray, as any panel.** The projector finds the point where the ray meets the panel and carries it through the panel's model into the face's area, as `item_in` does for a body line. The first region containing it names its row, which becomes the row pointed at, `Conversation::pointed_row`, exactly as a list's line under the ray does today. Off every region, the panel alone is pointed at.
- **Select is the row's.** A region whose row the server marked choosable is a choosable row pointed at: the trigger takes it at `precedence::Trigger::Row`, after the hand menu, the transcript and a wall carried, and A at the row's place in the conversation's meanings, `precedence::A::Conversation`, after everything that decides and before a teleport pad, with `Answer the request before choosing.` while a request waits. Neither order changes, and no rung is added. The drawer cannot make a row choosable: the server checks its own copy.
- **Hover is Tomori's.** Tomori outlines the region under the ray in its highlight colour, by rewriting vertices in the mesh kept, as a frame is recoloured (`Canvas::frame_mesh`), so that hovering calls no drawer, looks the same on every face, and cannot be hidden by one: the participant always sees what a press would choose. The ray's label ends `A  choose`, then `✓  chosen`, as for any row.
- **Scroll** (increment 2). A face with more rows than it shows scrolls for each participant, at `precedence::Right::Scroll`, as a panel does: the drawer is given the first row to show and says how many it showed, and Tomori draws the scroll bar and the label's hint. A region that itself takes the stick (increment 3) would be a rung between `Transcript` and `Scroll`, latched as every push is.
- **What stays Tomori's.** The grip pulls the panel off and carries it, the right stick and its click move a panel carried, B throws it away, holding B leaves; the frame, the title, metadata and rule, the scroll bar and the ray's label are drawn by Tomori; a wall's board, bar and handles are untouched. A face is never told a button, a stick or the ray, so it has nothing to capture them with.
- **Hands, the desktop, visitors.** Hand tracking points and hovers; it chooses nothing until a pinch is the trigger (`environments.md`). In the desktop window the mouse points, the left button and Enter choose, the wheel and the paging keys scroll; the console prints the body. A visitor sees faces and points at rows, and chooses nothing.

### Actions reach the environment's server side

By the server-made intent rows already have, `Intent::choice`, whose `Chosen` names the side's own panel, the row and whether the participant saw the current version, and not by the client side's `EnvironmentIntent`. A client-side intent's payload is whatever a client wrote, so a side must treat it as forgeable; a choice is attested by the server, which takes it only from a participant, for a row its own copy marks choosable, on a panel its environment publishes now, within the participant's allowance of 8 at once and 4 a second (`panel-actions.md`). It also works from a client that has no drawer, on the fallback list, so a toggle chosen on an older headset and one chosen on a face are the same choice. A toggle is a choosable row whose side flips it and publishes again; a pager is two. The client side's intents remain for what its own 3D objects need.

A side's changes to one panel are held to about one a second (`environments.md`), so a switch drawn from the server's state may lag the press; the label's `✓  chosen` is the immediate answer. Whether a choice should allow its panel one prompt update is an open question.

### The agent

- **The fallback is the source, so it cannot drift.** The face has no data but the panel: the rows the server wrote the body from. What a participant points at on a face is a row, described from the server's own copy by its number, its id and its text, with its reference to `inspect`, as today. `panels` gives the rows as today and, new, the face's key and its state, cleaned as an inspection's strings are, so the agent knows a custom presentation is shown and can read everything it was drawn from.
- **What can still go wrong** is an environment putting a fact only in the state, or a drawer that ignores its rows. Both are the operator's code. The state is small and visible to the agent; and `tomori-scene` gives a test helper, `face::test_support::covers`, that draws a face and fails unless every row that fits has a region and its text is among what is drawn.

### Clients without the drawer

A client draws a face only where its build registered a drawer for the key in that environment and it presents the environment at the version the space is in, the rule a client side already follows; otherwise it draws the body. A key's meaning and its state's format are the environment's own, agreed by the environment's protocol version: a change an older drawer cannot ignore raises it (`environments.md`). `Hello`'s `faces` only spares a client bytes it would not draw. No environment's version and no existing client changes for a panel without a face.

### Limits and safety

- **Bounded in.** The key and the 4 KiB of state, counted in the space's change budget with the panel.
- **Bounded out.** A drawing has at most 1,024 shapes, 1,024 cells of text and 30 regions, every coordinate finite, and with Tomori's header must mesh within `CARD_CAPACITY`. A shape, label or region not wholly inside the area is left out whole. A drawing past a count or the capacity is refused whole and the body shown, the client saying so once in its log for that key.
- **Legible.** Text is cells and labels at the panel's font pixel, 1.9 mm on a standard wall and scaled with it (`projection.md`); a drawer has no smaller size to choose.
- **Contained.** A drawer that panics is caught, as an avatar and a worker placement are (`avatars.md`, `environments.md`), never called again while the client runs, and its panels show the body, with one warning. A drawer cannot be interrupted, so one that is slow costs its frame: the client logs a draw longer than 4 ms, and drops a drawer after three.
- **What an environment cannot do.** Draw outside its area, over the frame, the title or another panel; draw over anything of Tomori's, since the face is in its panel's mesh at its panel's depth; put a face on the Tomori wall or another environment's, where it publishes nothing; take a control from Tomori; or make a press mean anything but the choice of a row it published. It can draw something that looks like a button, as a text panel can write `[Approve]`: Tomori never asks for a decision on a wall panel, a request takes A before any row, and the title row and the ray's label, which Tomori draws, say whose panel it is.

### Testing

A drawer is a pure function, so an environment tests it in its own crate with no GPU and no server: build a `face::Input` from a fixture panel, compare the `Drawing`, and check `covers`. `tomori-scene` tests the face through `projection::faces` with the count of layouts it already keeps: nothing drawn across frames that change nothing, one draw for a new version, the body for an unknown key, a refused drawing and a panic. A property test draws each fixture at every slot shape, floating, and from 0 to 30 rows, and checks the bounds. `tomori-render`'s `ui_panel::tests` draw a fixture face on lavapipe, flat, on a curved wall's slots and 4 m away, with a PNG preview, as charts are.

### Increment 1, and its size

`tomori-environment`'s `Face` and `Panel::with_face`; `tomori-protocol`'s `Panel::face` and `Hello`'s `faces`; the server's `stage/published.rs` keeping and checking it, and `hub/agent_panels.rs`; `tomori-scene`'s new `face` module, `projection::faces` drawing through it, the row found by region, and the hover outline; `Sceneries::set_face` and the headset builder; and a fixture face in the tests' queue environment. About the size of choosing a row (#298, some 2,200 lines with tests over 33 files): most of it tests, since the paths for pointing, choosing and the agent are already there.

### Rejected

- **Arbitrary 3D inside a panel**, (a): no guaranteed clipping, a draw order the renderer would have to open for each panel and eye, nothing a client without the scenery, a test or the agent can see. An environment that needs 3D has its scenery and its `pick`; a globe is not a panel.
- **HTML or a webview.** Panels are text and never HTML (`security.md`); a webview is a browser on the headset, drawn to a texture or a compositor layer, against one composition layer (`rendering.md`).
- **A texture the environment rasterises**, (b): blurred text, memory, a rasteriser to bring. Set aside, not forever: a heat map that is truly pixels could become an image in a face's area.
- **Drawers sent by the server**, as a script or WebAssembly, so that every client has them: a sandboxed interpreter on the headset for what compiled-in code already does, when environments are compiled into clients by design (`environments.md`).
- **The drawer reading the environment's shared state**, and **regions selected through client-side intents**: see The model and Actions.

## Rationale

The maintainer asked for panels an application can "subclass" for richer rendering. In AppKit a subclass overrides drawing and inherits the window's frame and event routing; here the panel is the window, and the face overrides only `draw`. Reading the code showed that Tomori already has the drawing surface a subclass needs: every face is a `Canvas`, and the chart panel is a custom face in all but who writes it. Reusing it keeps one pipeline, one draw a panel, sharp text and the kept meshes of #286 and #299, where a texture or a scenery's draw would give each up.

Making regions rows, rather than targets of the face's own, is what keeps the costs the maintainer was told of small. The agent reads the same rows the face draws; a client without the drawer shows them as a list and can still choose them; and a press reaches the side as the attested choice of #298, in an order that took several reviews to settle and that this leaves alone.

## Scope

Would cover `tomori-environment`, `tomori-protocol`'s `Panel` and `Hello`, `tomori-server`'s `stage/published.rs` and `hub/agent_panels.rs`, `tomori-scene`'s `face`, `projection` and `projection::faces`, `tomori-render`'s `Sceneries`, and the builders; with the text of `environments.md`, `projection.md`, `panel-actions.md`, `rendering.md`, `builders.md`, `mcp.md` and `security.md`. The renderer's pipelines and shaders, `precedence`, the server's handling of choices and the spectator are untouched by increment 1.

## Validation

For this proposal: check each statement under What exists today against the code it names. For an increment, once accepted: the tests under Testing, complete region coverage (`testing.md`), and on the headset, in a space whose environment publishes a face, that it is legible from where its wall is read, flat and on a curved wall, that the region under the ray is outlined and chosen with the trigger and A, that the panel is carried, resized and thrown away as any other, and that a headset without the environment's client crate shows the list.

## Open questions

- Whether a face draws below Tomori's title, metadata and rule, as proposed and as a chart does, or the whole inside of the frame, which Observe's look may want and which gives up the one row that always says whose panel it is.
- Whether the state is wanted at all, or every fact should be a field of a row; and whether it is live data, as a chart's points are, changing no version.
- Whether paging is each participant's, as scrolling is, the space's, as a pager's choosable rows make it, or both; and whether a face follows its highlighted row until the participant scrolls.
- Whether a choice should allow its panel one update at once, so that a toggle answers within a frame or two rather than a second.
- Whether faces may animate, by a clock given to the drawer at a few times a second, as a pulsing alert would need, or stay still between changes.
- Whether text and chart panels take faces, which would have no regions, and whether Tomori's own list face becomes the default drawer.
- Whether the client side is told the row pointed at, and a region may take the stick (increment 3), and by what server-made intent an adjustment reaches the side.
- Whether a drawer's colours are free or the chart's palette, and whether a label may be drawn at twice or three times the size, for a headline number.
- The spectator: whether it should draw projected panels, which it appears not to, and then faces, whose drawers it could run from the sceneries it is given.
- The bounds' numbers, 4 KiB, 1,024 shapes and cells, and 4 ms, each to be measured on the headset by the increment that builds it.
