# Panel Faces

This describes **face panels**: panels an environment's server side publishes like any other, whose whole frame its client crate draws, in Tomori's shapes and text, from a payload published with them. Tomori keeps everything else: the frame, the wall slot, carrying, resizing, pinning, copying, dismissal, the level of detail, the projection and the limits. The first phase is a requirement, built; the later phases are a proposal, each its own decision for the maintainer.

## Requirement

**Published.** A side publishes a face panel with `tomori_environment::Panel::face(id, title, Face)`, where `Face::new(key)` names the drawing, `with_data(json)` gives its payload, `with_summary(text)` its summary and `with_items(rows)` its items. Everything else a published panel has applies unchanged: `on_wall`, `with_reference`, `depends_on`, the 128 panels an environment shows, their rates and bytes, resting, copies and pinning (`environments.md`, `panel-context.md`).

- **The key** is 1 to 32 ASCII letters, digits, `-` and `_` (`tomori_environment::valid_face_key`, `FACE_KEY_BYTES`), naming a drawing within its environment.
- **The payload** is the environment's own JSON, at most 4 KiB as written (`FACE_DATA_LIMIT`, `tomori_protocol::FACE_DATA_BYTES`, an event's `MESSAGE_LIMIT`), or none. It counts towards the bytes its side publishes.
- **The summary** is the panel's body, cleaned and bounded as any body (30 lines, 1,200 characters). It is what the agent reads and what a client without the drawing shows. A face's items are never written into its body.
- **The items** are rows, as a list's, at most 30, with ids, references, `choosable`, tones and keys, kept by the server as a list's are.
- The server refuses, saying why, a key it does not take (`A face's key is 1 to 32 ASCII letters, digits, - and _.`), a payload that is not JSON (`A face's data is JSON.`) and a payload past its bound (`A face's data takes at most 4096 bytes of JSON.`).

**On the wire.** A projection's `Panel` gains `face`, a `tomori_protocol::Face` of the key and the payload, sent only for a face panel, part of what the panel's version covers and of what it shows (`Panel::same_structure`, the patches' `same_content`). Its items travel with it, as `Face::items`, never as `Panel::rows`, found by their places among them by `Panel::row_on` and `Panel::listed`, so that a client that draws no face, an older one included, never takes a line of the summary for one of them. An older client ignores `face` and draws the body, the summary, as a text panel's. A new payload is a new version, as any change of a published panel's content is.

**Drawn.** An environment's client crate registers a drawing for each key it draws, with `tomori-render`'s `Sceneries::set_face(id, key, drawer)`, a `tomori_scene::face::Drawer`, which every client that registers sceneries hands to its conversation (`Conversation::set_faces`): the headset, the desktop window and the spectator. The frame step gives the projector the drawings of the environment drawn (`Projector::set_drawers`), at no allocation on a frame that changes nothing.

- **It fills the whole frame.** The drawer is given `face::Input`: the panel's title, its payload, its items and the size of its whole frame in font pixels, whatever the panel's size. It returns a `face::Drawing`: an optional background colour, shapes, labels and regions, in font pixels from the frame's top-left corner. Tomori draws no title, metadata or chips over it, only its frame. A drawing that wants its title shows it.
- **Its size.** A face panel is a chart's size: in a wall's slot, the slot's cells; floating, 48 columns of 14 rows below a heading (`projection::chart::FLOATING`), drawn or shown as its summary alike, so that it is the same size on every client.
- **Tomori's shapes and text.** A drawing's shapes are the shared primitives (`Shape`); its text is labels from the glyph atlas, which `Label::scaled` draws 1 to 4 times as large as the panel's text (`canvas::LABEL_SCALE`), for large figures. A face is drawn in its panel's own mesh, by the panel pipeline, one draw a panel, as every face is (`rendering.md`).
- **Opaque.** Its background, the plate (`face::PLATE`) where it gives none, is made wholly opaque, so that it is drawn first in each eye, writing depth, its shapes and text over it, as every opaque face is (`rendering.md`, **How**).
- **Within its frame.** A shape, label or region not wholly inside the frame is left out whole, as is one any of whose places or sizes is not a number, every coordinate checked, or any of whose sizes, a width, height, radius or outline, is negative. A drawing of more than 1,024 shapes (`face::SHAPES`), 1,024 cells of text (`face::CELLS`) or 30 regions (`face::REGIONS`) is not drawn, nor is one whose drawer panics, which is caught: the panel shows its summary instead, until it changes.
- **Kept.** The drawer is asked only when the panel's version, its size or what draws it changes, in its turn among the panels a paced projector lays out (`faces::LAYOUTS`, `faces::ARRIVALS`), and its drawing is kept at each of the panel's latest three sizes, as every face is (`projection::faces`). Pointing at it, moving its wall, the head moving and the ray sweeping draw nothing again.
- **Far away, its title.** A face in a wall's slot too small to read is drawn as its title, as every panel is (`rendering.md`, **A panel too small to read is drawn as its title**), and its drawer is not asked.
- **Without its drawing.** A client whose build registers no drawing for the key in the environment drawn, an older client, and one whose drawer failed, draws the panel's title and summary as a text panel's, at the same size. None of the summary's lines is one of its items, nor ticked, whatever Markdown it holds: pointing at one names the panel alone, and A chooses nothing there.
- **Held to a window.** A face panel that depends on time, held to a fixed window it has not been published for, is sent without its face, its items or its rows, its body saying it waits for that window's data, as any panel so held is (`panel-context.md`), so that a fixed window never shows its live payload; published for that window, its face is that window's.

**Its items.** A region names one of the panel's items by its id, at most one region an item, ignoring one that names no item; the server describes an item only by its id, never by a line, which is of the summary. The ray on a region points at that item as a list's line points at its row, by its place among the rows: the server describes it from its own copy, its id given to `inspect` by its reference, and a choosable item is chosen with A or the trigger, its choice reaching the side as `Intent::choice`, as for a list (`panel-actions.md`). The region of the item the ray is on is outlined in Tomori's highlight colour, and the one just chosen filled faintly, by Tomori, over the drawing, without asking the drawer again.

## Rationale

In November 2026 the maintainer asked, in the headset, of a downstream environment whose own 3D panels had lost their look on Tomori's walls, whether "the application can subclass panels and have more complex rendering behaviour". The approved direction was that Tomori owns the panel and the environment's client side draws what is inside it, with a text fallback for the agent and for clients without its code. The motivating example became that environment's world clocks, three 24-hour dials lit by day and dark by night, each with an area's traffic and trend, which were scenery and so could not be placed, carried, copied by region, pinned or counted as panels.

Every face was already a canvas of shared shapes and atlas text, and a chart is a custom face in all but who writes it, so a face panel's drawing is that canvas, kept as a chart's is: sharp, one draw, cached by version and size, and tested without a GPU. The clocks need nothing more but labels larger than the panel's text. Drawing with an environment's own shaders, into a texture that becomes the face's opaque backing, is a later phase, for what shapes cannot draw.

Deciding on the proposal (#306), the maintainer chose that a face fills the whole of its panel's frame, whatever its size, rather than drawing below a heading of Tomori's, as a chart does.

A region names an item, which is a row, so that pointing, choosing and inspecting reuse the paths rows have, the server describing and checking everything from its own copy: a face adds a look, and no new input, intent or forgery surface. The summary is the body, so that the agent, older clients and builds without the environment's crate read the same text.

## Scope

`tomori-environment`'s `Face`, `Panel::face`, `face_spec`, `FACE_KEY_BYTES`, `FACE_DATA_LIMIT` and `valid_face_key`; `tomori-protocol`'s `Panel::face`, `Face`, `FACE_KEY_BYTES`, `FACE_DATA_BYTES` and `valid_face_key`; `tomori-server`'s `stage/published.rs` and `stage.rs`; `tomori-scene`'s `face` module, `canvas::Label::scaled`, `projection` (`fit`, `drawn_face`, `Projector::set_drawers`), `projection::faces`, `Conversation::set_faces` and the frame step; `tomori-render`'s `Sceneries::set_face` and `faces`; and the clients that hand sceneries to their conversation.

## Validation

- `tomori-environment`'s doctests check `Face` and `Panel::face`; `tomori-protocol`'s tests check that a face is sent only where a panel has one, read as none from an older server, and part of its version, and its doctest the keys taken.
- `tomori-server`'s `stage/published` tests check that a face panel reaches clients with its face, its summary as its body and its items with its face, never as rows, that pointing at its item describes it with its reference, a line alone none, and that it is chosen, that publishing the same changes nothing and a new payload updates it, and that a key, a payload that is not JSON and one past its bound are refused, saying so.
- `tomori-scene`'s `face` tests check that a drawing is placed from the frame's corner on an opaque plate, at a standard slot's size, that what lies outside the frame is left out whole, as is every kind of shape with a place that is not a number or a negative size, and labels and regions likewise, that every shape is moved alike, that a drawing past its bounds and a drawer that panics draw nothing, that regions name items by their place among the rows, outlined and filled as marked, and that the world clock fixture (`face::fixtures::Clock`) draws its 24 hours, its time large and its dial's region at three sizes. `projection::faces`' tests check that a face panel fills its frame with its drawing, drawn once and kept across 30 frames, again for a new payload, and as its summary in another environment; that without its drawing, or with a drawer that panics, it shows its title and summary at the same size; and that pointing at its region names its item, elsewhere none, and that hovering outlines it; and that a summary written as a Markdown list, shown or with a drawer that panics, names none of its items on any of its lines. `stage/held` tests check that a face held to a fixed window shows no face until published for it. `canvas`' tests check scaled labels.
- `tomori-render`'s `ui_panel::tests::face` draws the world clock on lavapipe in a standard slot and floating, and checks its day, night, needle and time and its opaque plate; set `TOMORI_FACE_PREVIEW` to a PNG path to see it. Its `scenery` tests check that drawings are registered by identifier and key.
- Region coverage of everything added is complete (`testing.md`).
- On the headset: in a space whose environment publishes a face panel, confirm that it reads on its wall, is carried, resized, copied and thrown away as any panel, shows its title from far away, and that a client without the environment's crate shows its summary.

## Proposal: the later phases

### Phase 2: live faces

- **Live data.** A face's payload and summary become its live data, as a chart's points are: a change of them alone takes a new `data_seq`, not a new version, so that it never refuses a participant's or the agent's edit, and reaches a client applying patches as a `face_replace` op.
- **Time.** A drawing says when it next changes from time alone (`Drawing::next_change`, given the client's clock in `Input::now`), such as a clock's next minute, and Tomori draws it again then, at most once a second a face, in its turn, with no traffic from the server.
- **A glance level of detail**, between the far title and the whole face, at which a drawing may draw less.
- **Refresh policies** (`docs/proposals/panel-refresh.md`, #509), once accepted: a face panel takes a policy as any published panel, and its live budget counts it while live.

### Phase 3: an environment's own shaders

An environment's client crate registers, beside a drawing, a `tomori_render::face::Layer`, its own pipeline, which renders into a texture Tomori owns for the face, sized to how large it appears (256, 512 or 1,024 pixels on its longer side, with hysteresis), only when the drawing would be drawn again, at most one a frame, the nearest first. The texture is the face's opaque backing, drawn first in each eye with depth writes in place of the flat plate, and the drawing's text and shapes are drawn over it, so that clipping, curved walls, carrying and depth come free and the environment's shaders run only offscreen. Its WGSL passes `tomori_render::test_gpu::validate_wgsl` in the environment's tests, which enforces **No shader passes a struct or an array of a uniform by value** (`rendering.md`); its pipelines are made at startup, inside an error scope. Face textures together are held to a budget (48 MiB proposed), evicting the least recently drawn, and a layer whose pass takes too long repeatedly falls back to the drawing alone. Its numbers are to be measured on a Quest 3 at 2.0× with 2× MSAA.

### Later

Each the maintainer's call: a cursor overlay over a face, for one participant, as a chart's readout is drawn; faces that animate at a few frames a second; faces on text and chart panels; the client side told the item pointed at.

### What an environment needs next for its world clocks

- **Common crate:** a payload type for one area's clock (its area, city, minute, traffic, trend and the day's hours), and the summary written from it, which the server's description of a clock already is.
- **Server side:** one face panel for each area, `Panel::face("clock-europe", "Europe · 13:20", Face::new("world-clock")…).depends_on(["area"]).with_reference("clock/europe")`, on a declared wall of three slots where the band of clocks stands, with its local time in its title, so that its far title still reads; its clocks no longer described as objects.
- **Client crate:** its blocks and labels as one `Drawer`, each hour an arc of `Shape::Quad`s, the ticks `Shape::Rule`s, the needle a `Shape::Stroke`, the trend `Shape::Fill`s or `Shape::Bar`s and the time a scaled label, registered with `Sceneries::set_face`; its pick and placement go. Until phase 2, its side publishes the clocks again each minute for the needle to move.
- **Copies:** a clock for another area is a copy with `area` set, counted among the 128 panels, with no bound of its own once copies count only there (#535).

## Open questions

- Whether a face's payload should be live data from the start, rather than in phase 2, so that a clock's minute never changes its version.
- Whether a drawing's colours should be free, as now, or Tomori's palette and tones, so that every environment's faces read alike.
- Whether faces may be translucent where an environment asks, giving up early depth rejection for a face that floats over the room.
- Whether a drawer that panics should be asked again when its panel next changes, as now, or never again while the client runs.
- The budgets of phase 3, each to be measured on the headset.
